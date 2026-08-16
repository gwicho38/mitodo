//! An MCP server over stdio, exposing the workspace as tools.
//!
//! The wire contract is the one a live client actually speaks, captured in
//! resources/mcp-client-handshake.log: the classic initialize handshake at
//! 2025-11-25, not the stateless shape the current specification describes.

pub mod exec;
pub mod protocol;
pub mod tools;

use std::collections::HashSet;
use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::config::Config;
use protocol::{METHOD_NOT_FOUND, PARSE_ERROR, Request, error_response, result_response};

pub struct ServerState<'a> {
    pub config: &'a Config,
    /// Ids this server retired by its own writes, so a stale id reports
    /// not_found while one that drifted out-of-band reports conflict.
    pub retired: HashSet<String>,
}

/// Read a line, answer it, repeat until stdin closes.
pub fn serve(config: &Config) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut state = ServerState {
        config,
        retired: HashSet::new(),
    };

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle_line(&mut state, &line) {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Answer one request. `None` means a notification, which gets no reply.
pub fn handle_line(state: &mut ServerState, line: &str) -> Option<String> {
    let request: Request = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            return Some(error_response(&json!(null), PARSE_ERROR, &err.to_string()));
        }
    };
    let id = request.id.clone()?;

    match request.method.as_str() {
        "initialize" => Some(result_response(&id, initialize(&request.params))),
        "tools/list" => Some(result_response(&id, json!({"tools": tools::schemas()}))),
        "tools/call" => Some(call(state, &id, &request.params)),
        _ => Some(error_response(
            &id,
            METHOD_NOT_FOUND,
            &format!("unknown method {}", request.method),
        )),
    }
}

/// Route one tools/call. Unknown names are a tool error, not a protocol error:
/// the agent can read it and pick a real tool.
fn call(state: &mut ServerState, id: &Value, params: &Value) -> String {
    let Some(name) = params.get("name").and_then(|n| n.as_str()) else {
        return protocol::error_response(id, protocol::INVALID_PARAMS, "tools/call needs a name");
    };
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    match exec::run(state, name, &arguments) {
        Ok(payload) => protocol::tool_result(id, payload),
        Err((code, message)) => protocol::tool_error(id, code, &message),
    }
}

/// Echo the client's protocol version rather than asserting our own: the
/// installed client speaks 2025-11-25 while the published spec has moved on.
fn initialize(params: &Value) -> Value {
    let version = params
        .get("protocolVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("2025-11-25");
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "mitodo", "version": env!("CARGO_PKG_VERSION")},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(config: &Config) -> ServerState<'_> {
        ServerState {
            config,
            retired: HashSet::new(),
        }
    }

    /// The exact bytes a live client sent, so a client-shape change fails here
    /// rather than in a mystery at runtime.
    const HANDSHAKE: &str = include_str!("../../resources/mcp-client-handshake.log");

    #[test]
    fn tools_list_returns_the_whole_catalogue() {
        let config = Config::default();
        let mut state = state(&config);
        let reply = handle_line(
            &mut state,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        let listed = parsed["result"]["tools"].as_array().unwrap();
        assert_eq!(listed.len(), tools::TOOLS.len());
        assert!(listed.iter().all(|t| t["inputSchema"]["type"] == "object"));
    }

    // Serving the catalogue without a handshake keeps a newer, stateless client
    // working; the specification retired the handshake entirely.
    #[test]
    fn tools_list_works_before_any_initialize() {
        let config = Config::default();
        let mut state = state(&config);
        let reply = handle_line(
            &mut state,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .unwrap();
        assert!(serde_json::from_str::<Value>(&reply).unwrap()["result"]["tools"].is_array());
    }

    #[test]
    fn an_unknown_tool_is_a_tool_error_not_a_protocol_error() {
        let config = Config::default();
        let mut state = state(&config);
        let reply = handle_line(
            &mut state,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#,
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        assert!(parsed.get("error").is_none());
        assert_eq!(parsed["result"]["isError"], true);
    }

    #[test]
    fn a_call_without_a_name_is_invalid_params() {
        let config = Config::default();
        let mut state = state(&config);
        let reply = handle_line(
            &mut state,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{}}"#,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reply).unwrap()["error"]["code"],
            -32602
        );
    }

    #[test]
    fn the_recorded_handshake_replays() {
        let config = Config::default();
        let mut state = state(&config);
        let mut lines = HANDSHAKE.lines().filter_map(|l| l.strip_prefix("IN: "));

        let initialize = lines.next().expect("the log starts with initialize");
        let reply = handle_line(&mut state, initialize).expect("initialize is answered");
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(parsed["result"]["serverInfo"]["name"], "mitodo");
        assert!(parsed["result"]["capabilities"]["tools"].is_object());

        let initialized = lines.next().expect("then notifications/initialized");
        assert!(
            handle_line(&mut state, initialized).is_none(),
            "a notification gets no reply"
        );
    }

    #[test]
    fn the_protocol_version_is_echoed_not_asserted() {
        let config = Config::default();
        let mut state = state(&config);
        let reply = handle_line(
            &mut state,
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#,
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["result"]["protocolVersion"], "2099-01-01");
    }

    #[test]
    fn a_malformed_line_is_a_parse_error() {
        let config = Config::default();
        let mut state = state(&config);
        let reply = handle_line(&mut state, "{not json").unwrap();
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["error"]["code"], -32700);
    }

    #[test]
    fn an_unknown_method_with_an_id_is_method_not_found() {
        let config = Config::default();
        let mut state = state(&config);
        let reply = handle_line(
            &mut state,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/foo"}"#,
        )
        .unwrap();
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["error"]["code"], -32601);
    }

    #[test]
    fn an_unknown_notification_is_ignored() {
        let config = Config::default();
        let mut state = state(&config);
        assert!(
            handle_line(
                &mut state,
                r#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#
            )
            .is_none()
        );
    }
}
