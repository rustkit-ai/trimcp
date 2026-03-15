#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── JSON-RPC 2.0 base types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(flatten)]
    pub payload: ResponsePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponsePayload {
    Result(Value),
    Error(JsonRpcError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}

// ── Incoming message (request or notification) ────────────────────────────────

/// Any message received from the client or upstream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IncomingMessage {
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
}

// ── MCP method names ──────────────────────────────────────────────────────────

pub mod method {
    pub const INITIALIZE: &str = "initialize";
    pub const TOOLS_LIST: &str = "tools/list";
    pub const TOOLS_CALL: &str = "tools/call";
    pub const RESOURCES_LIST: &str = "resources/list";
    pub const RESOURCES_READ: &str = "resources/read";
    pub const PROMPTS_LIST: &str = "prompts/list";
    pub const PROMPTS_GET: &str = "prompts/get";
}

// ── Helpers ───────────────────────────────────────────────────────────────────

impl JsonRpcRequest {
    pub fn is_tools_call(&self) -> bool {
        self.method == method::TOOLS_CALL
    }
}

impl JsonRpcResponse {
    pub fn ok(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            payload: ResponsePayload::Result(result),
        }
    }

    pub fn err(id: RequestId, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            payload: ResponsePayload::Error(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_request_deserializes() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":null}"#;
        let req: JsonRpcRequest = serde_json::from_str(raw).unwrap();
        assert_eq!(req.method, "tools/list");
        assert!(matches!(req.id, RequestId::Number(1)));
    }

    #[test]
    fn test_request_with_string_id_deserializes() {
        let raw = r#"{"jsonrpc":"2.0","id":"abc","method":"initialize"}"#;
        let req: JsonRpcRequest = serde_json::from_str(raw).unwrap();
        assert!(matches!(req.id, RequestId::String(_)));
    }

    #[test]
    fn test_response_ok_serializes() {
        let resp = JsonRpcResponse::ok(RequestId::Number(1), json!({"tools": []}));
        let raw = serde_json::to_string(&resp).unwrap();
        let val: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(val["jsonrpc"], "2.0");
        assert_eq!(val["result"]["tools"], json!([]));
    }

    #[test]
    fn test_response_err_serializes() {
        let resp = JsonRpcResponse::err(
            RequestId::Number(1),
            JsonRpcError::METHOD_NOT_FOUND,
            "method not found",
        );
        let raw = serde_json::to_string(&resp).unwrap();
        let val: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(val["error"]["code"], JsonRpcError::METHOD_NOT_FOUND);
        assert_eq!(val["error"]["message"], "method not found");
    }

    #[test]
    fn test_notification_deserializes() {
        let raw = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let notif: JsonRpcNotification = serde_json::from_str(raw).unwrap();
        assert_eq!(notif.method, "notifications/initialized");
        assert!(notif.params.is_none());
    }

    #[test]
    fn test_incoming_message_dispatches_request() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#;
        let msg: IncomingMessage = serde_json::from_str(raw).unwrap();
        assert!(matches!(msg, IncomingMessage::Request(_)));
    }

    #[test]
    fn test_incoming_message_dispatches_notification() {
        let raw = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let msg: IncomingMessage = serde_json::from_str(raw).unwrap();
        assert!(matches!(msg, IncomingMessage::Notification(_)));
    }

    #[test]
    fn test_is_tools_call() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(raw).unwrap();
        assert!(req.is_tools_call());
    }

    #[test]
    fn test_request_roundtrip() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(42),
            method: method::TOOLS_LIST.to_string(),
            params: Some(json!({"cursor": null})),
        };
        let serialized = serde_json::to_string(&req).unwrap();
        let deserialized: JsonRpcRequest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.method, req.method);
    }
}
