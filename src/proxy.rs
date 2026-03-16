use crate::cache::{Cache, make_cache_key};
use crate::compress::{Pipeline, tokens_saved};
use crate::error::{Error, Result};
use crate::metrics::Metrics;
use crate::protocol::{IncomingMessage, JsonRpcRequest, JsonRpcResponse, ResponsePayload};
use serde_json::Value;
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
}

impl Proxy {
    /// Spawn the upstream MCP server.
    pub fn spawn(command: &str, args: &[String], metrics: Arc<Metrics>) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
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
        })
    }

    /// Enable TTL caching for `tools/call` results.
    pub fn with_cache(mut self, ttl_secs: u64) -> Self {
        self.cache = Some(Cache::new(ttl_secs));
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
                    if let Some(cached) = self.cache_get(req) {
                        debug!("cache hit for tools/call");
                        return Ok(Some(cached));
                    }
                }

                self.send(msg).await?;
                let response = self.recv().await?;

                if req.is_tools_call() {
                    let result = response.map(|resp| self.intercept_tools_call(resp));
                    if let Some(ref resp) = result {
                        self.cache_insert(req, resp.clone());
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

    /// Return a cached response for this tools/call request, updating its id to match.
    fn cache_get(&self, req: &JsonRpcRequest) -> Option<JsonRpcResponse> {
        let key = tools_call_cache_key(req)?;
        let mut resp = self.cache.as_ref()?.get(key)?.clone();
        resp.id = req.id.clone();
        Some(resp)
    }

    /// Store a tools/call response in the cache, evicting stale entries first.
    fn cache_insert(&mut self, req: &JsonRpcRequest, resp: JsonRpcResponse) {
        if let (Some(cache), Some(key)) = (&mut self.cache, tools_call_cache_key(req)) {
            cache.evict_expired();
            cache.insert(key, resp);
            debug!(entries = cache.len(), "cache updated");
        }
    }

    /// Kill the upstream process.
    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(cache) = &self.cache {
            if !cache.is_empty() {
                debug!(entries = cache.len(), "cache entries at shutdown");
            }
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
