use crate::error::{Error, Result};
use crate::protocol::JsonRpcResponse;
use semstore::SemanticIndex;
use serde_json::json;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Semantic knowledge store for tool-call responses.
///
/// Unlike the exact-match cache, the knowledge store indexes responses by the
/// *meaning* of the request. A future query of "useState hook" can hit a
/// stored entry for "hooks" without calling upstream again.
///
/// Responses are stored in a local SQLite file and expire after `ttl_days`.
pub struct KnowledgeStore {
    index: SemanticIndex,
    ttl_secs: u64,
}

impl KnowledgeStore {
    /// Open or create a knowledge store at `path`.
    ///
    /// `threshold` is the minimum cosine-similarity score `[0.0, 1.0]`
    /// required to accept a semantic hit (default 0.82 is recommended).
    pub fn open(path: &Path, threshold: f32, ttl_days: u64) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Config(format!("cannot create knowledge dir: {e}")))?;
        }
        let index = SemanticIndex::builder()
            .path(path)
            .threshold(threshold)
            .build()
            .map_err(|e| Error::Upstream(format!("knowledge store init: {e}")))?;
        Ok(Self {
            index,
            ttl_secs: ttl_days * 86_400,
        })
    }

    /// Search for a stored response semantically similar to `query_text`.
    ///
    /// Returns `(response, tokens_original, tokens_compressed)` for the first
    /// non-expired result above the configured threshold, or `None`.
    pub fn search(&self, query_text: &str) -> Option<(JsonRpcResponse, usize, usize)> {
        let query_tool = query_text.split(": ").next().unwrap_or(query_text);
        let results = self.index.search(query_text, 5).ok()?;
        let now = now_secs();
        for result in results {
            let meta = &result.metadata;
            // Reject cross-tool hits: tool names must match.
            if let Some(stored_tool) = meta.get("tool_name").and_then(|v| v.as_str())
                && stored_tool != query_tool
            {
                debug!(
                    stored_tool,
                    query_tool, "knowledge hit skipped: tool name mismatch"
                );
                continue;
            }
            let inserted_at = meta.get("inserted_at")?.as_u64()?;
            if now.saturating_sub(inserted_at) >= self.ttl_secs {
                debug!(score = result.score, "knowledge hit expired, skipping");
                continue;
            }
            let response_json = meta.get("response")?;
            let response: JsonRpcResponse = serde_json::from_value(response_json.clone()).ok()?;
            let tokens_original = meta
                .get("tokens_original")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let tokens_compressed = meta
                .get("tokens_compressed")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            debug!(score = result.score, "knowledge store hit");
            return Some((response, tokens_original, tokens_compressed));
        }
        None
    }

    /// Index a response against `query_text` for future semantic lookups.
    ///
    /// `tokens_original` / `tokens_compressed` are the pre/post-compression
    /// token counts, stored so that savings can be reported accurately on hits.
    ///
    /// Evicts expired entries before inserting to prevent unbounded growth.
    pub fn insert(
        &mut self,
        query_text: &str,
        response: &JsonRpcResponse,
        tokens_original: usize,
        tokens_compressed: usize,
    ) -> Result<()> {
        // Evict expired entries before each insert (one SQL query, fast).
        let evicted = self
            .index
            .remove_older_than(self.ttl_secs)
            .map_err(|e| Error::Upstream(format!("knowledge evict: {e}")))?;
        if evicted > 0 {
            debug!(evicted, "evicted expired knowledge entries");
        }

        let response_value = serde_json::to_value(response)
            .map_err(|e| Error::Upstream(format!("serialize response: {e}")))?;
        let tool_name = query_text.split(": ").next().unwrap_or(query_text);
        let metadata = json!({
            "inserted_at": now_secs(),
            "tool_name": tool_name,
            "response": response_value,
            "tokens_original": tokens_original,
            "tokens_compressed": tokens_compressed,
        });
        self.index
            .insert(query_text, metadata)
            .map_err(|e| Error::Upstream(format!("knowledge insert: {e}")))?;
        Ok(())
    }

    /// Remove all entries whose TTL has expired. Returns the count removed.
    ///
    /// Called automatically on each `insert`. Can also be called manually
    /// (e.g. at process shutdown) to compact the store.
    #[allow(dead_code)]
    pub fn evict_expired(&mut self) -> usize {
        self.index.remove_older_than(self.ttl_secs).unwrap_or(0)
    }

    /// Number of entries in the index (including potentially expired ones).
    pub fn len(&self) -> usize {
        self.index.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

/// Build the query text used as the semantic key for a tool call.
///
/// Format: `"<tool_name>: <args_json>"` — concise enough for good embeddings.
pub fn query_text(tool_name: &str, args: &serde_json::Value) -> String {
    format!("{tool_name}: {args}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{JsonRpcResponse, RequestId};
    use serde_json::json;

    fn make_response(text: &str) -> JsonRpcResponse {
        JsonRpcResponse::ok(
            RequestId::Number(1),
            json!({ "content": [{ "type": "text", "text": text }] }),
        )
    }

    #[test]
    fn test_query_text_format() {
        let args = json!({ "topic": "hooks" });
        let qt = query_text("get-library-docs", &args);
        assert!(qt.starts_with("get-library-docs:"));
        assert!(qt.contains("hooks"));
    }

    #[test]
    fn test_insert_and_exact_search() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut store = KnowledgeStore::open(&path, 0.70, 7).unwrap();

        let resp = make_response("React hooks documentation");
        store
            .insert("get-library-docs: react hooks", &resp, 100, 40)
            .unwrap();

        // Exact same query → must hit
        let hit = store.search("get-library-docs: react hooks");
        assert!(hit.is_some());
        let (_, tok_in, tok_out) = hit.unwrap();
        assert_eq!(tok_in, 100);
        assert_eq!(tok_out, 40);
    }

    #[test]
    fn test_no_hit_on_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.db");
        let store = KnowledgeStore::open(&path, 0.82, 7).unwrap();
        assert!(store.search("anything").is_none());
    }

    #[test]
    fn test_expired_entry_not_returned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("expired.db");
        // ttl_days = 0 → expires immediately
        let mut store = KnowledgeStore::open(&path, 0.70, 0).unwrap();

        let resp = make_response("some content");
        store.insert("some query", &resp, 0, 0).unwrap();

        // Even an exact match should be filtered out (TTL = 0 days = 0 secs)
        let hit = store.search("some query");
        assert!(hit.is_none());
    }

    #[test]
    fn test_len_after_insert() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("len.db");
        let mut store = KnowledgeStore::open(&path, 0.82, 7).unwrap();
        assert!(store.is_empty());
        store
            .insert("query", &make_response("response"), 0, 0)
            .unwrap();
        assert_eq!(store.len(), 1);
    }
}
