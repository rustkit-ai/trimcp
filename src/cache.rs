use crate::protocol::JsonRpcResponse;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    response: JsonRpcResponse,
    expires_at: u64, // Unix timestamp (seconds)
    #[serde(default)]
    tokens_original: usize,
    #[serde(default)]
    tokens_compressed: usize,
}

impl CacheEntry {
    fn is_expired(&self) -> bool {
        now_secs() >= self.expires_at
    }
}

/// TTL-based cache for tool call results, persistent across sessions.
#[derive(Serialize, Deserialize)]
pub struct Cache {
    entries: HashMap<u64, CacheEntry>,
    ttl_secs: u64,
}

impl Cache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: HashMap::new(),
            ttl_secs,
        }
    }

    /// Load from disk, falling back to an empty cache if absent or corrupt.
    pub fn load(path: &Path, ttl_secs: u64) -> Self {
        if path.exists()
            && let Ok(raw) = std::fs::read_to_string(path)
            && let Ok(mut cache) = serde_json::from_str::<Self>(&raw)
        {
            cache.ttl_secs = ttl_secs; // honour current config
            return cache;
        }
        Self::new(ttl_secs)
    }

    /// Persist to disk (expired entries are evicted first).
    pub fn save(&mut self, path: &Path) -> anyhow::Result<()> {
        self.evict_expired();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string(self)?)?;
        Ok(())
    }

    /// Look up a cached response. Returns `None` if missing or expired.
    pub fn get(&self, key: u64) -> Option<&JsonRpcResponse> {
        let entry = self.entries.get(&key)?;
        if !entry.is_expired() {
            Some(&entry.response)
        } else {
            None
        }
    }

    /// Return the token sizes stored with a cache entry.
    pub fn get_tokens(&self, key: u64) -> Option<(usize, usize)> {
        let entry = self.entries.get(&key)?;
        if !entry.is_expired() {
            Some((entry.tokens_original, entry.tokens_compressed))
        } else {
            None
        }
    }

    /// Insert a response into the cache with its pre/post-compression token sizes.
    pub fn insert(
        &mut self,
        key: u64,
        response: JsonRpcResponse,
        tokens_original: usize,
        tokens_compressed: usize,
    ) {
        self.entries.insert(
            key,
            CacheEntry {
                response,
                expires_at: now_secs() + self.ttl_secs,
                tokens_original,
                tokens_compressed,
            },
        );
    }

    /// Remove all expired entries.
    pub fn evict_expired(&mut self) {
        self.entries.retain(|_, e| !e.is_expired());
    }

    /// Number of live (non-expired) entries.
    pub fn len(&self) -> usize {
        self.entries.values().filter(|e| !e.is_expired()).count()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Compute a stable cache key from the tool name and its JSON arguments.
pub fn make_cache_key(tool_name: &str, args: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    tool_name.hash(&mut hasher);
    args.to_string().hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{JsonRpcResponse, RequestId};
    use serde_json::json;

    fn make_response(id: i64) -> JsonRpcResponse {
        JsonRpcResponse::ok(RequestId::Number(id), json!({"content": []}))
    }

    #[test]
    fn test_insert_and_get_returns_response() {
        let mut cache = Cache::new(60);
        let resp = make_response(1);
        cache.insert(42, resp, 0, 0);
        assert!(cache.get(42).is_some());
    }

    #[test]
    fn test_get_missing_key_returns_none() {
        let cache = Cache::new(60);
        assert!(cache.get(999).is_none());
    }

    #[test]
    fn test_expired_entry_returns_none() {
        let mut cache = Cache::new(0); // 0s TTL — expires immediately
        cache.insert(1, make_response(1), 0, 0);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(cache.get(1).is_none());
    }

    #[test]
    fn test_len_counts_live_entries() {
        let mut cache = Cache::new(60);
        cache.insert(1, make_response(1), 0, 0);
        cache.insert(2, make_response(2), 0, 0);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_evict_expired_removes_stale_entries() {
        let mut cache = Cache::new(0);
        cache.insert(1, make_response(1), 0, 0);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        cache.evict_expired();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");

        let mut cache = Cache::new(60);
        cache.insert(1, make_response(1), 0, 0);
        cache.save(&path).unwrap();

        let loaded = Cache::load(&path, 60);
        assert!(loaded.get(1).is_some());
    }

    #[test]
    fn test_load_absent_file_returns_empty() {
        let cache = Cache::load(Path::new("/nonexistent/cache.json"), 60);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_make_cache_key_same_input_same_key() {
        let a = make_cache_key("tools/call", &json!({"name": "read_file", "path": "/tmp"}));
        let b = make_cache_key("tools/call", &json!({"name": "read_file", "path": "/tmp"}));
        assert_eq!(a, b);
    }

    #[test]
    fn test_make_cache_key_different_args_different_key() {
        let a = make_cache_key("tools/call", &json!({"path": "/tmp/a"}));
        let b = make_cache_key("tools/call", &json!({"path": "/tmp/b"}));
        assert_ne!(a, b);
    }

    #[test]
    fn test_make_cache_key_different_tool_different_key() {
        let a = make_cache_key("read_file", &json!({}));
        let b = make_cache_key("write_file", &json!({}));
        assert_ne!(a, b);
    }
}
