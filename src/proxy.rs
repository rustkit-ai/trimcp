#![allow(dead_code)]

use crate::error::{Error, Result};
use crate::protocol::{IncomingMessage, JsonRpcResponse};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tracing::debug;

/// Spawns and communicates with an upstream MCP server process.
pub struct Proxy {
    child: Child,
    writer: BufWriter<ChildStdin>,
    reader: BufReader<ChildStdout>,
}

impl Proxy {
    /// Spawn the upstream MCP server.
    pub fn spawn(command: &str, args: &[String]) -> Result<Self> {
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
        })
    }

    /// Forward a message to the upstream and return its response.
    ///
    /// Returns `None` if the upstream closed its stdout (EOF).
    /// Intercepts `tools/call` responses for compression (see `compress` module).
    pub async fn forward(&mut self, msg: &IncomingMessage) -> Result<Option<JsonRpcResponse>> {
        self.send(msg).await?;

        match msg {
            IncomingMessage::Notification(_) => Ok(None),
            IncomingMessage::Request(req) => {
                let response = self.recv().await?;
                if req.is_tools_call() {
                    return Ok(response.map(|resp| self.intercept_tools_call(resp)));
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

    /// Hook for TICKET-005: intercept tools/call responses for compression.
    /// Currently a no-op pass-through.
    fn intercept_tools_call(&self, resp: JsonRpcResponse) -> JsonRpcResponse {
        resp
    }

    /// Kill the upstream process.
    pub async fn shutdown(&mut self) -> Result<()> {
        self.child
            .kill()
            .await
            .map_err(|e| Error::Upstream(e.to_string()))
    }
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
