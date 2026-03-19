#![allow(dead_code)]

use crate::error::Result;
use crate::protocol::IncomingMessage;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

/// Reads newline-delimited JSON-RPC messages from stdin.
pub struct StdinReader {
    inner: BufReader<tokio::io::Stdin>,
}

/// Writes newline-delimited JSON-RPC messages to stdout.
pub struct StdoutWriter {
    inner: BufWriter<tokio::io::Stdout>,
}

impl StdinReader {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            inner: BufReader::new(tokio::io::stdin()),
        }
    }

    /// Read the next message. Returns `None` on EOF.
    pub async fn read(&mut self) -> Result<Option<IncomingMessage>> {
        let mut line = String::new();
        let bytes_read = self.inner.read_line(&mut line).await?;

        if bytes_read == 0 {
            return Ok(None);
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let msg = serde_json::from_str(trimmed)?;
        Ok(Some(msg))
    }
}

impl StdoutWriter {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            inner: BufWriter::new(tokio::io::stdout()),
        }
    }

    /// Write a value as a newline-delimited JSON message to stdout.
    pub async fn write<T: serde::Serialize>(&mut self, value: &T) -> Result<()> {
        let json = serde_json::to_string(value)?;
        self.inner.write_all(json.as_bytes()).await?;
        self.inner.write_all(b"\n").await?;
        self.inner.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::{IncomingMessage, JsonRpcRequest, RequestId};
    use serde_json::json;

    fn make_request(method: &str) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            method: method.to_string(),
            params: None,
        }
    }

    #[test]
    fn test_request_serializes_to_single_line() {
        let req = make_request("tools/list");
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains('\n'), "must be single line");
    }

    #[test]
    fn test_incoming_message_request_roundtrip() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#;
        let msg: IncomingMessage = serde_json::from_str(raw).unwrap();
        let re_serialized = serde_json::to_string(&msg).unwrap();
        let val: serde_json::Value = serde_json::from_str(&re_serialized).unwrap();
        assert_eq!(val["method"], "tools/call");
    }

    #[test]
    fn test_incoming_message_notification_roundtrip() {
        let raw = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let msg: IncomingMessage = serde_json::from_str(raw).unwrap();
        let re_serialized = serde_json::to_string(&msg).unwrap();
        let val: serde_json::Value = serde_json::from_str(&re_serialized).unwrap();
        assert_eq!(val["method"], "notifications/initialized");
    }

    #[test]
    fn test_response_serializes_to_valid_json() {
        use crate::protocol::{JsonRpcResponse, RequestId};
        let resp = JsonRpcResponse::ok(RequestId::Number(42), json!({"tools": []}));
        let json = serde_json::to_string(&resp).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["id"], 42);
        assert_eq!(val["result"]["tools"], json!([]));
    }
}
