//! JSON-RPC 2.0 envelopes for the MCP stdio transport.
//!
//! Line-delimited, one object per line: the `Content-Length` framing in the
//! specification belongs to the HTTP transport, not this one.

use serde::Deserialize;
use serde_json::{Value, json};

pub const PARSE_ERROR: i32 = -32700;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub method: String,
    /// Absent on notifications, which must receive no reply.
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub params: Value,
}

pub fn result_response(id: &Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

pub fn error_response(id: &Value, code: i32, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

/// A tool that ran and failed: a result carrying the port's error envelope, so
/// the agent can branch on `code` rather than parse prose.
pub fn tool_error(id: &Value, code: &str, message: &str) -> String {
    let envelope = json!({"error": code, "message": message}).to_string();
    result_response(
        id,
        json!({"content": [{"type": "text", "text": envelope}], "isError": true}),
    )
}

/// A tool that succeeded, with its JSON rendered as the text content MCP expects.
pub fn tool_result(id: &Value, payload: Value) -> String {
    result_response(
        id,
        json!({"content": [{"type": "text", "text": payload.to_string()}]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notification_has_no_id() {
        let request: Request =
            serde_json::from_str(r#"{"method":"notifications/initialized","jsonrpc":"2.0"}"#)
                .unwrap();
        assert_eq!(request.method, "notifications/initialized");
        assert!(request.id.is_none());
    }

    #[test]
    fn a_call_carries_its_params_and_ignores_meta() {
        let request: Request = serde_json::from_str(
            r#"{"method":"tools/call","params":{"name":"todos_list","arguments":{},
                "_meta":{"progressToken":2}},"jsonrpc":"2.0","id":2}"#,
        )
        .unwrap();
        assert_eq!(request.params["name"], "todos_list");
        assert_eq!(request.id, Some(json!(2)));
    }

    #[test]
    fn a_tool_error_is_a_result_not_a_protocol_error() {
        let line = tool_error(&json!(7), "conflict", "the file moved");
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert!(
            parsed.get("error").is_none(),
            "not a protocol error: {line}"
        );
        assert_eq!(parsed["result"]["isError"], true);
        let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
        let envelope: Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["error"], "conflict");
        assert_eq!(envelope["message"], "the file moved");
    }

    #[test]
    fn a_tool_result_renders_its_payload_as_text() {
        let line = tool_result(&json!(1), json!({"ok": true}));
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["result"]["content"][0]["type"], "text");
        let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(serde_json::from_str::<Value>(text).unwrap()["ok"], true);
    }
}
