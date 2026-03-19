use crate::cache::{Cache, make_cache_key};
#[cfg(feature = "semtree")]
use crate::code_context::{CodeContext, format_code_context};
use crate::compress::{Pipeline, tokens_saved};
use crate::error::{Error, Result};
use crate::knowledge::{KnowledgeStore, query_text};
use crate::metrics::Metrics;
use crate::protocol::{IncomingMessage, JsonRpcRequest, JsonRpcResponse, ResponsePayload};
#[cfg(feature = "semtree")]
use semtree_core::Chunk;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tracing::debug;

/// Spawns and communicates with an upstream MCP server process.
pub struct Proxy {
    child: Child,
    writer: BufWriter<ChildStdin>,
    reader: BufReader<ChildStdout>,
    pipeline: Pipeline,
    metrics: Arc<Metrics>,
    cache: Option<Cache>,
    cache_path: Option<PathBuf>,
    knowledge: Option<KnowledgeStore>,
    #[cfg(feature = "semtree")]
    code_context: Option<CodeContext>,
}

impl Proxy {
    /// Spawn the upstream MCP server.
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        metrics: Arc<Metrics>,
    ) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Upstream("failed to open upstream stdin".to_string()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Upstream("failed to open upstream stdout".to_string()))?;

        Ok(Self {
            child,
            writer: BufWriter::new(stdin),
            reader: BufReader::new(stdout),
            pipeline: Pipeline::default_pipeline(),
            metrics,
            cache: None,
            cache_path: None,
            knowledge: None,
            #[cfg(feature = "semtree")]
            code_context: None,
        })
    }

    /// Enable TTL caching for `tools/call` results, loaded from and saved to `path`.
    pub fn with_cache(mut self, ttl_secs: u64, path: PathBuf) -> Self {
        self.cache = Some(Cache::load(&path, ttl_secs));
        self.cache_path = Some(path);
        self
    }

    /// Enable the semantic knowledge store for `tools/call` results.
    ///
    /// When enabled, responses are indexed by the meaning of the request.
    /// Future semantically similar queries can hit locally without upstream.
    pub fn with_knowledge_store(mut self, store: KnowledgeStore) -> Self {
        self.knowledge = Some(store);
        self
    }

    /// Enable semtree code-context injection for `tools/call` responses.
    ///
    /// When enabled, every tool response is enriched with the most relevant
    /// code chunks from the indexed codebase before being returned to the LLM.
    #[cfg(feature = "semtree")]
    pub fn with_code_context(mut self, ctx: CodeContext) -> Self {
        self.code_context = Some(ctx);
        self
    }

    /// Forward a message to the upstream and return its response.
    ///
    /// Returns `None` if the upstream closed its stdout (EOF).
    /// For `tools/call`: checks the cache first, compresses the response,
    /// then stores it in the cache for future identical calls.
    pub async fn forward(&mut self, msg: &IncomingMessage) -> Result<Option<JsonRpcResponse>> {
        match msg {
            IncomingMessage::Notification(_) => {
                self.send(msg).await?;
                Ok(None)
            }
            IncomingMessage::Request(req) => {
                if req.is_tools_call() {
                    // Layer 1: exact TTL cache.
                    if let Some((cached, tok_in, tok_out)) = self.cache_get(req) {
                        debug!("cache hit for tools/call");
                        self.metrics.record_cache_hit_with_tokens(tok_in, tok_out);
                        return Ok(Some(cached));
                    }

                    // Layer 2: semantic knowledge store.
                    if let Some((mut resp, tok_in, tok_out)) = self.knowledge_search(req) {
                        resp.id = req.id.clone();
                        debug!("knowledge store hit for tools/call");
                        self.metrics
                            .record_knowledge_hit_with_tokens(tok_in, tok_out);
                        return Ok(Some(resp));
                    }
                }

                self.send(msg).await?;
                let response = self.recv().await?;

                if req.is_tools_call() {
                    let tok_in_before = self.metrics.tokens_in();
                    let tok_out_before = self.metrics.tokens_out();
                    let result = response.map(|resp| self.intercept_tools_call(resp));
                    #[cfg(feature = "semtree")]
                    let result = if let Some(resp) = result {
                        Some(self.enrich_code_context(resp, req).await)
                    } else {
                        None
                    };
                    let tok_in = self.metrics.tokens_in() - tok_in_before;
                    let tok_out = self.metrics.tokens_out() - tok_out_before;
                    if let Some(ref resp) = result {
                        self.cache_insert(req, resp.clone(), tok_in, tok_out);
                        self.knowledge_insert(req, resp, tok_in, tok_out);
                    }
                    return Ok(result);
                }

                Ok(response)
            }
        }
    }

    /// Write a message to the upstream stdin.
    async fn send(&mut self, msg: &IncomingMessage) -> Result<()> {
        let json = serde_json::to_string(msg)?;
        debug!(message = %json, "→ upstream");
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Read one response line from the upstream stdout.
    async fn recv(&mut self) -> Result<Option<JsonRpcResponse>> {
        let mut line = String::new();
        let bytes = self.reader.read_line(&mut line).await?;

        if bytes == 0 {
            return Ok(None);
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        debug!(message = %trimmed, "← upstream");
        let resp = serde_json::from_str(trimmed)?;
        Ok(Some(resp))
    }

    /// Intercept tools/call responses and compress text content.
    fn intercept_tools_call(&self, resp: JsonRpcResponse) -> JsonRpcResponse {
        let ResponsePayload::Result(ref value) = resp.payload else {
            return resp;
        };

        let Some(original) = extract_text_content(value) else {
            self.metrics.record_call();
            return resp;
        };

        let compressed = self.pipeline.compress(&original);
        let saved = tokens_saved(&original, &compressed);

        self.metrics.record(&original, &compressed);

        if saved == 0 {
            return resp;
        }

        debug!(saved, "compressed tools/call output");

        let mut new_value = value.clone();
        if let Some(Some(items)) = new_value.get_mut("content").map(|c| c.as_array_mut()) {
            for item in items.iter_mut() {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item["text"] = serde_json::Value::String(compressed.clone());
                }
            }
        }

        JsonRpcResponse {
            payload: ResponsePayload::Result(new_value),
            ..resp
        }
    }

    /// Return a cached response and its token sizes for this tools/call request.
    fn cache_get(&self, req: &JsonRpcRequest) -> Option<(JsonRpcResponse, usize, usize)> {
        let key = tools_call_cache_key(req)?;
        let cache = self.cache.as_ref()?;
        let mut resp = cache.get(key)?.clone();
        resp.id = req.id.clone();
        let (tok_in, tok_out) = cache.get_tokens(key).unwrap_or((0, 0));
        Some((resp, tok_in, tok_out))
    }

    /// Store a tools/call response in the cache with its token sizes.
    fn cache_insert(
        &mut self,
        req: &JsonRpcRequest,
        resp: JsonRpcResponse,
        tokens_original: usize,
        tokens_compressed: usize,
    ) {
        if let (Some(cache), Some(key)) = (&mut self.cache, tools_call_cache_key(req)) {
            cache.evict_expired();
            cache.insert(key, resp, tokens_original, tokens_compressed);
            debug!(entries = cache.len(), "cache updated");
        }
    }

    /// Search the knowledge store for a semantically similar past response.
    fn knowledge_search(&self, req: &JsonRpcRequest) -> Option<(JsonRpcResponse, usize, usize)> {
        let qt = tools_call_query_text(req)?;
        self.knowledge.as_ref()?.search(&qt)
    }

    /// Index this response in the knowledge store for future semantic hits.
    fn knowledge_insert(
        &mut self,
        req: &JsonRpcRequest,
        resp: &JsonRpcResponse,
        tokens_original: usize,
        tokens_compressed: usize,
    ) {
        let Some(qt) = tools_call_query_text(req) else {
            return;
        };
        if let Some(store) = &mut self.knowledge {
            if let Err(e) = store.insert(&qt, resp, tokens_original, tokens_compressed) {
                debug!(err = %e, "knowledge insert failed");
            } else {
                debug!(entries = store.len(), "knowledge store updated");
            }
        }
    }

    /// Enrich a `tools/call` response with relevant code chunks from semtree.
    ///
    /// If no `CodeContext` is configured, or no relevant chunks are found,
    /// the response is returned unchanged.
    #[cfg(feature = "semtree")]
    async fn enrich_code_context(
        &self,
        resp: JsonRpcResponse,
        req: &JsonRpcRequest,
    ) -> JsonRpcResponse {
        let Some(ctx) = &self.code_context else {
            return resp;
        };
        let Some(query) = tools_call_query_text(req) else {
            return resp;
        };
        let chunks = ctx.search(&query).await;
        if chunks.is_empty() {
            return resp;
        }
        debug!(chunks = chunks.len(), "injecting semtree code context");
        inject_code_context_into_response(resp, &chunks)
    }

    /// Kill the upstream process and persist the cache to disk.
    pub async fn shutdown(&mut self) -> Result<()> {
        if let (Some(cache), Some(path)) = (&mut self.cache, &self.cache_path) {
            debug!(entries = cache.len(), "saving cache at shutdown");
            let _ = cache.save(path);
        }
        self.child
            .kill()
            .await
            .map_err(|e| Error::Upstream(e.to_string()))
    }
}

/// Compute a cache key from a tools/call request's tool name and arguments.
fn tools_call_cache_key(req: &JsonRpcRequest) -> Option<u64> {
    let params = req.params.as_ref()?;
    let name = params.get("name")?.as_str()?;
    let args = params.get("arguments").unwrap_or(&Value::Null);
    Some(make_cache_key(name, args))
}

/// Build the query text used as the semantic key for a tools/call request.
fn tools_call_query_text(req: &JsonRpcRequest) -> Option<String> {
    let params = req.params.as_ref()?;
    let name = params.get("name")?.as_str()?;
    let args = params.get("arguments").unwrap_or(&Value::Null);
    Some(query_text(name, args))
}

/// Parse a raw tools/call response content into a string for compression.
pub fn extract_text_content(value: &Value) -> Option<String> {
    let content = value.get("content")?;
    let items = content.as_array()?;

    let texts: Vec<&str> = items
        .iter()
        .filter_map(|item| {
            if item.get("type")?.as_str()? == "text" {
                item.get("text")?.as_str()
            } else {
                None
            }
        })
        .collect();

    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

/// Prepend a `[Code Context]` block (built from `chunks`) to the first text
/// item of a `tools/call` response, returning the enriched response.
#[cfg(feature = "semtree")]
fn inject_code_context_into_response(
    mut resp: JsonRpcResponse,
    chunks: &[Chunk],
) -> JsonRpcResponse {
    let ResponsePayload::Result(ref mut value) = resp.payload else {
        return resp;
    };
    let context_block = format_code_context(chunks);
    if let Some(Some(items)) = value.get_mut("content").map(|c| c.as_array_mut()) {
        for item in items.iter_mut() {
            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(original) = item
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(str::to_string)
                {
                    item["text"] = Value::String(format!("{context_block}\n\n{original}"));
                    break;
                }
            }
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_text_content_single_item() {
        let value = json!({
            "content": [
                { "type": "text", "text": "hello world" }
            ]
        });
        assert_eq!(
            extract_text_content(&value),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn test_extract_text_content_multiple_items() {
        let value = json!({
            "content": [
                { "type": "text", "text": "line one" },
                { "type": "text", "text": "line two" }
            ]
        });
        assert_eq!(
            extract_text_content(&value),
            Some("line one\nline two".to_string())
        );
    }

    #[test]
    fn test_extract_text_content_ignores_non_text() {
        let value = json!({
            "content": [
                { "type": "image", "data": "base64..." },
                { "type": "text", "text": "only this" }
            ]
        });
        assert_eq!(extract_text_content(&value), Some("only this".to_string()));
    }

    #[test]
    fn test_extract_text_content_empty_content() {
        let value = json!({ "content": [] });
        assert_eq!(extract_text_content(&value), None);
    }

    #[test]
    fn test_extract_text_content_missing_content_key() {
        let value = json!({ "result": "ok" });
        assert_eq!(extract_text_content(&value), None);
    }

    #[test]
    fn test_method_tools_call_constant() {
        use crate::protocol::method;
        assert_eq!(method::TOOLS_CALL, "tools/call");
    }
}
