# `mitodo mcp-server` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `mitodo mcp-server` speaks MCP over stdio and exposes 13 tools, so Claude Code or Codex can read and manage the todo workspace itself.

**Architecture:** A line-delimited JSON-RPC 2.0 loop on stdin/stdout dispatches `initialize`, `tools/list` and `tools/call`. Tool bodies resolve items by `ItemId` against a freshly loaded `Workspace` and write through `store::write`, so byte preservation and conflict detection are the ones mitodo already tests. Ported from `~/repos/todos-mcp`: same tool names, same `{"error": code, "message": …}` envelope, same retired-id distinction between `not_found` and `conflict`.

**Tech Stack:** Rust 2024, `serde_json` for the wire, `std::io` for the loop, `sha2` via the existing `ItemId`. No new dependencies.

## Global Constraints

- Spec: [docs/superpowers/specs/2026-08-12-mcp-server-design.md](../specs/2026-08-12-mcp-server-design.md).
- Branch: `feat/mcp-server`, cut from `main` after the current fix branch merges. Never commit to `main`.
- **stdout carries protocol only.** Every diagnostic goes to stderr; a stray `println!` corrupts the stream. No `dbg!`, no `print!` anywhere under `src/mcp/`.
- **Echo the client's `protocolVersion`** in the `initialize` result. Never assert our own, and never require `initialize` before serving `tools/list`.
- **No tool accepts a path.** Tools take a group *name*; the server joins it onto the workspace root. Reject names containing `/` or `\`, equal to `..`, or starting with `.`. Never write into `_archive/`.
- **Every write goes through `store::write` or `store::archive`.** No tool writes markdown itself.
- Tool failures are `result` with `isError: true` whose text is `{"error": code, "message": …}`. Protocol failures are JSON-RPC `error` with −32700 / −32601 / −32602.
- Comments: minimum possible, one line each, stating a hidden constraint only. Never restate what the code does, never reference this plan or branch.
- Test names are sentences describing behaviour. No `#[ignore]`, no skipped tests.
- `cargo test` green, `cargo clippy --all-targets --all-features -- -D warnings` silent, `cargo fmt --check` clean at every commit. Note CI runs a newer clippy than local; use those exact flags.
- No new dependencies.
- Commit after every task with a Conventional Commits subject. No AI attribution.

---

### Task 1: The subcommand and the JSON-RPC loop

**Files:**
- Create: `src/mcp/mod.rs`, `src/mcp/protocol.rs`
- Modify: `src/cli.rs` (`Command`), `src/main.rs` (module list, dispatch)
- Test: `src/mcp/protocol.rs`, `src/mcp/mod.rs`

**Interfaces:**
- Consumes: `crate::config::Config`.
- Produces: `mcp::serve(config: &Config) -> std::io::Result<()>`; `mcp::handle_line(state: &mut ServerState, line: &str) -> Option<String>` (the pure dispatcher — `None` means a notification with nothing to send); `protocol::{Request, error_response, result_response, tool_error}`; `ServerState { protocol_version: String, retired: HashSet<String> }`.

- [ ] **Step 1: Write the failing protocol tests**

Create `src/mcp/protocol.rs` with types and tests:

```rust
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
        assert!(parsed.get("error").is_none(), "not a protocol error: {line}");
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
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --quiet mcp 2>&1 | tail -6`
Expected: `file not found for module mcp` — the module is not declared yet.

- [ ] **Step 3: Create the loop and declare the module**

Create `src/mcp/mod.rs`:

```rust
//! An MCP server over stdio, exposing the workspace as tools.
//!
//! The wire contract is the one a live client actually speaks, captured in
//! resources/mcp-client-handshake.log: the classic initialize handshake at
//! 2025-11-25, not the stateless shape the current specification describes.

pub mod protocol;

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
        _ => Some(error_response(
            &id,
            METHOD_NOT_FOUND,
            &format!("unknown method {}", request.method),
        )),
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
```

Add `mod mcp;` to `src/main.rs`'s module list, keeping alphabetical order (after `mod logging;`).

- [ ] **Step 4: Write the loop tests**

Append to `src/mcp/mod.rs`:

```rust
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
    fn the_recorded_handshake_replays() {
        let config = Config::default();
        let mut state = state(&config);
        let mut lines = HANDSHAKE
            .lines()
            .filter_map(|l| l.strip_prefix("IN: "));

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
            handle_line(&mut state, r#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#)
                .is_none()
        );
    }
}
```

- [ ] **Step 5: Wire the subcommand**

In `src/cli.rs`, add to `enum Command`:

```rust
    /// Serve the workspace to an MCP client over stdio
    McpServer,
```

In `src/main.rs`'s `match args.command()`, before the TUI arm:

```rust
        Some(Command::McpServer) => cmd_mcp_server(&config_path),
```

and add the handler beside `cmd_list`:

```rust
/// Serve MCP on stdio. No terminal setup: this process has no UI, and anything
/// written to stdout that is not protocol corrupts the stream.
fn cmd_mcp_server(config_path: &Path) -> Result<()> {
    let config = config::Config::load(config_path)?;
    mcp::serve(&config)?;
    Ok(())
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --quiet mcp 2>&1 | tail -6 && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3`
Expected: all nine tests pass, clippy silent.

- [ ] **Step 7: Verify against the real client**

```bash
cargo install --path . --force
claude --strict-mcp-config \
  --mcp-config '{"mcpServers":{"mitodo":{"command":"mitodo","args":["mcp-server"]}}}' \
  -p 'List the tools the mitodo server offers.' < /dev/null
```
Expected: it connects and reports **no tools yet** — the handshake works even though `tools/list` is not implemented, which is exactly the state after this task. A connection error here means the loop or the subcommand is wrong.

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add src/mcp src/cli.rs src/main.rs
git commit -m "feat(mcp): an MCP stdio loop, echoing the client's protocol version"
```

---

### Task 2: The tool table and `tools/list`

**Files:**
- Create: `src/mcp/tools.rs`
- Modify: `src/mcp/mod.rs` (declare the module, dispatch `tools/list` and `tools/call`)
- Test: `src/mcp/tools.rs`, `src/mcp/mod.rs`

**Interfaces:**
- Consumes: `protocol::{tool_error, tool_result}` (Task 1).
- Produces: `tools::Tool { name: &'static str, description: &'static str, schema: &'static str }`; `tools::TOOLS: [Tool; 13]`; `tools::schemas() -> Vec<serde_json::Value>`.

- [ ] **Step 1: Write the failing tests**

Create `src/mcp/tools.rs`:

```rust
//! The tool catalogue, as data.
//!
//! Names keep todos-mcp's `todos_` prefix so prompts written against that server
//! keep working and the two stay recognisably one surface.

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema for the tool's arguments, as a literal.
    pub schema: &'static str,
}

pub const TOOLS: [Tool; 13] = [];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_name_a_description_and_a_valid_schema() {
        for tool in &TOOLS {
            assert!(!tool.name.is_empty());
            assert!(
                tool.description.len() > 20,
                "{} needs a description an agent can act on",
                tool.name
            );
            let schema: serde_json::Value = serde_json::from_str(tool.schema)
                .unwrap_or_else(|e| panic!("{} has invalid schema JSON: {e}", tool.name));
            assert_eq!(schema["type"], "object", "{} schema is not an object", tool.name);
            assert!(schema.get("properties").is_some(), "{} has no properties", tool.name);
        }
    }

    #[test]
    fn tool_names_are_unique_and_prefixed() {
        let mut names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "two tools share a name");
        assert!(TOOLS.iter().all(|t| t.name.starts_with("todos_")));
    }

    #[test]
    fn the_catalogue_covers_the_specified_surface() {
        let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        for expected in [
            "todos_list",
            "todos_get_item",
            "todos_get_file",
            "todos_list_groups",
            "todos_create_item",
            "todos_add_child",
            "todos_update_item",
            "todos_set_notes",
            "todos_delete_item",
            "todos_archive_item",
            "todos_archive_finished",
            "todos_create_group",
            "todos_sync",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --quiet tools 2>&1 | tail -6`
Expected: a compile error — `TOOLS` is declared `[Tool; 13]` but initialised empty.

- [ ] **Step 3: Fill the table**

Replace `pub const TOOLS: [Tool; 13] = [];` with the catalogue. Schemas are literals so they need no builder:

```rust
pub const TOOLS: [Tool; 13] = [
    Tool {
        name: "todos_list",
        description: "List todo items. Optionally filter with mitodo's query language \
                      (for example \"pri:P0 !done\" or \"acct:lysk text:\\\"bank\\\"\") \
                      and/or restrict to one group. Completed items are excluded unless \
                      include_done is true.",
        schema: r#"{"type":"object","properties":{"query":{"type":"string"},"group":{"type":"string"},"include_done":{"type":"boolean"}},"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_get_item",
        description: "Read one item in full by id, including its notes and its direct children.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_get_file",
        description: "Read a group's TODO.md verbatim, for when the raw markdown matters.",
        schema: r#"{"type":"object","properties":{"group":{"type":"string"}},"required":["group"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_list_groups",
        description: "List the workspace's groups with their open and total item counts.",
        schema: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_create_item",
        description: "Add a new item to a group. section names the heading to place it under; \
                      when given, the file must already have that section. Optionally attach \
                      notes and child items.",
        schema: r#"{"type":"object","properties":{"group":{"type":"string"},"text":{"type":"string"},"section":{"type":"string"},"notes":{"type":"string"},"children":{"type":"array","items":{"type":"string"}}},"required":["group","text"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_add_child",
        description: "Add a child item beneath an existing item, indented under it.",
        schema: r#"{"type":"object","properties":{"parent_id":{"type":"string"},"text":{"type":"string"}},"required":["parent_id","text"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_update_item",
        description: "Edit an item's text and/or set whether it is done, in one write. \
                      Returns the item, whose id changes when the text changed.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"},"new_text":{"type":"string"},"done":{"type":"boolean"}},"required":["id"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_set_notes",
        description: "Replace the notes beneath an item. An empty string removes them.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"},"notes":{"type":"string"}},"required":["id","notes"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_delete_item",
        description: "Delete an item and its notes outright. Prefer todos_archive_item, \
                      which moves it into the archive instead.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_archive_item",
        description: "Move one item, and everything nested under it, into the group's \
                      archive file under a dated heading. A move, not a delete.",
        schema: r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_archive_finished",
        description: "Move every finished top-level item in a group into its archive. \
                      An item whose subtree still holds open work is left alone and reported.",
        schema: r#"{"type":"object","properties":{"group":{"type":"string"}},"required":["group"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_create_group",
        description: "Create a new group: a directory with a TODO.md seeded with the same \
                      section headings the existing groups use.",
        schema: r#"{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false}"#,
    },
    Tool {
        name: "todos_sync",
        description: "Run the workspace's configured git sync commands and return their output.",
        schema: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    },
];

/// The catalogue as `tools/list` returns it.
pub fn schemas() -> Vec<serde_json::Value> {
    TOOLS
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": serde_json::from_str::<serde_json::Value>(tool.schema)
                    .expect("tool schemas are literals, checked by tests"),
            })
        })
        .collect()
}
```

- [ ] **Step 4: Dispatch `tools/list` and `tools/call`**

In `src/mcp/mod.rs`, add `pub mod tools;` beside `pub mod protocol;`, and extend the match:

```rust
        "initialize" => Some(result_response(&id, initialize(&request.params))),
        "tools/list" => Some(result_response(
            &id,
            json!({"tools": tools::schemas()}),
        )),
        "tools/call" => Some(call(state, &id, &request.params)),
```

and add the dispatcher, which every later task extends:

```rust
/// Route one tools/call. Unknown names are a tool error, not a protocol error:
/// the agent can read it and pick a real tool.
fn call(state: &mut ServerState, id: &Value, params: &Value) -> String {
    let Some(name) = params.get("name").and_then(|n| n.as_str()) else {
        return protocol::error_response(id, protocol::INVALID_PARAMS, "tools/call needs a name");
    };
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    let _ = (state, &arguments);
    protocol::tool_error(id, "validation_error", &format!("unknown tool {name}"))
}
```

- [ ] **Step 5: Write the dispatch tests**

Append to `mod tests` in `src/mcp/mod.rs`:

```rust
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
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --quiet mcp 2>&1 | tail -6 && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3`
Expected: pass, clippy silent.

- [ ] **Step 7: Verify the client sees all thirteen**

```bash
cargo install --path . --force
claude --strict-mcp-config \
  --mcp-config '{"mcpServers":{"mitodo":{"command":"mitodo","args":["mcp-server"]}}}' \
  -p 'List the names of the tools the mitodo server offers, comma separated.' < /dev/null
```
Expected: all thirteen `todos_*` names.

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add src/mcp
git commit -m "feat(mcp): the thirteen-tool catalogue, served by tools/list"
```

---

### Task 3: The four read tools

**Files:**
- Create: `src/mcp/exec.rs`
- Modify: `src/mcp/mod.rs` (route `call` into `exec`)
- Test: `src/mcp/exec.rs`

**Interfaces:**
- Consumes: `tools::TOOLS` (Task 2); `protocol::{tool_error, tool_result}` (Task 1).
- Produces: `exec::run(state: &mut ServerState, name: &str, arguments: &Value) -> Result<Value, (&'static str, String)>` where the error is `(code, message)`; `exec::item_json(item: &Item, group: &str) -> Value`; `exec::group_name(state, name) -> Result<&Group, (&'static str, String)>`.

- [ ] **Step 1: Write the failing tests**

Create `src/mcp/exec.rs`:

```rust
//! Argument parsing and each tool's call into the store.
//!
//! Every write goes through `store::write`, so the byte-preservation guarantee
//! and the conflict check are the ones the rest of the project already tests.

use serde_json::{Value, json};

use super::ServerState;
use crate::store::{self, Item, Workspace};

/// A tool failure: a machine-readable code the agent can branch on, and prose.
pub type ToolFailure = (&'static str, String);

pub fn run(
    state: &mut ServerState,
    name: &str,
    arguments: &Value,
) -> Result<Value, ToolFailure> {
    match name {
        "todos_list" => todos_list(state, arguments),
        "todos_get_item" => todos_get_item(state, arguments),
        "todos_get_file" => todos_get_file(state, arguments),
        "todos_list_groups" => todos_list_groups(state),
        other => Err(("validation_error", format!("unknown tool {other}"))),
    }
}

fn workspace(state: &ServerState) -> Result<Workspace, ToolFailure> {
    Workspace::load(state.config).map_err(|e| ("validation_error", e.to_string()))
}

/// Which group owns a file, for reporting.
fn group_of(workspace: &Workspace, item: &Item) -> String {
    workspace
        .groups
        .iter()
        .find(|g| g.todo_file == item.file)
        .map(|g| g.name.clone())
        .unwrap_or_default()
}

pub fn item_json(item: &Item, group: &str) -> Value {
    json!({
        "id": item.id.as_str(),
        "group": group,
        "section": item.section,
        "heading": item.heading,
        "priority": item.priority.as_str(),
        "text": item.text,
        "done": item.done,
        "has_notes": !item.description.is_empty(),
        "due": item.due.map(|d| d.to_string()),
        "line": item.line,
        "parent": item.parent.as_ref().map(|p| p.as_str()),
    })
}

fn string_arg(arguments: &Value, key: &str) -> Result<String, ToolFailure> {
    arguments
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ("validation_error", format!("{key} is required")))
}

fn todos_list(state: &ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let workspace = workspace(state)?;
    let query = match arguments.get("query").and_then(|v| v.as_str()) {
        None => None,
        Some(source) => crate::query::Query::parse(source)
            .map_err(|e| ("validation_error", e.to_string()))?,
    };
    let group = arguments.get("group").and_then(|v| v.as_str());
    let include_done = arguments
        .get("include_done")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let items: Vec<Value> = workspace
        .items
        .iter()
        .filter(|item| include_done || !item.done)
        .filter(|item| {
            let owner = group_of(&workspace, item);
            group.is_none_or(|wanted| owner == wanted)
        })
        .filter(|item| {
            query
                .as_ref()
                .is_none_or(|q| q.matches(item, Some(&group_of(&workspace, item))))
        })
        .map(|item| item_json(item, &group_of(&workspace, item)))
        .collect();

    Ok(json!({"items": items, "count": items.len()}))
}

fn todos_get_item(state: &ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let id = string_arg(arguments, "id")?;
    let workspace = workspace(state)?;
    let item = resolve(state, &workspace, &id)?;
    let group = group_of(&workspace, item);
    let children: Vec<Value> = workspace
        .items
        .iter()
        .filter(|candidate| candidate.parent.as_ref() == Some(&item.id))
        .map(|child| json!({"id": child.id.as_str(), "text": child.text, "done": child.done}))
        .collect();

    let mut payload = item_json(item, &group);
    payload["notes"] = json!(item.description);
    payload["children"] = json!(children);
    Ok(payload)
}

fn todos_get_file(state: &ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let name = string_arg(arguments, "group")?;
    let workspace = workspace(state)?;
    let group = workspace
        .groups
        .iter()
        .find(|g| g.name == name)
        .ok_or_else(|| ("invalid_group", format!("no group named {name}")))?;
    let text = std::fs::read_to_string(&group.todo_file)
        .map_err(|e| ("validation_error", e.to_string()))?;
    Ok(json!({"path": group.todo_file.to_string_lossy(), "text": text}))
}

fn todos_list_groups(state: &ServerState) -> Result<Value, ToolFailure> {
    let workspace = workspace(state)?;
    let groups: Vec<Value> = workspace
        .groups
        .iter()
        .map(|group| {
            let items: Vec<&Item> = workspace
                .items
                .iter()
                .filter(|i| i.file == group.todo_file)
                .collect();
            json!({
                "name": group.name,
                "todo_file": group.todo_file.to_string_lossy(),
                "open": items.iter().filter(|i| !i.done).count(),
                "total": items.len(),
                "has_notes": group.notes_file.is_some(),
                "archive_dir": group.archive_dir.as_ref().map(|d| d.to_string_lossy()),
            })
        })
        .collect();
    Ok(json!({"groups": groups}))
}

/// The item an id names.
///
/// An id this server retired by its own write reports `not_found`; one that
/// simply no longer resolves drifted out-of-band and reports `conflict`. The
/// agent needs to tell its own edit from someone else's.
fn resolve<'a>(
    state: &ServerState,
    workspace: &'a Workspace,
    id: &str,
) -> Result<&'a Item, ToolFailure> {
    if let Some(item) = workspace.items.iter().find(|i| i.id.as_str() == id) {
        return Ok(item);
    }
    if state.retired.contains(id) {
        return Err((
            "not_found",
            format!("id {id} was retired by an earlier write"),
        ));
    }
    Err((
        "conflict",
        format!("id {id} no longer resolves; re-read and retry"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, GroupBy};
    use std::collections::HashSet;

    /// A workspace on disk, plus the config that reads it.
    fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        for (group, body) in files {
            let group_dir = dir.path().join(group);
            std::fs::create_dir_all(&group_dir).unwrap();
            std::fs::write(group_dir.join("TODO.md"), body).unwrap();
        }
        let mut config = Config::default();
        config.workspace.root = dir.path().to_path_buf();
        config.workspace.group_by = GroupBy::Directory;
        config.workspace.todo_glob = "*/TODO.md".to_string();
        config.priority.source = crate::config::PrioritySource::Heading;
        (dir, config)
    }

    fn run_tool(config: &Config, name: &str, arguments: Value) -> Result<Value, ToolFailure> {
        let mut state = ServerState {
            config,
            retired: HashSet::new(),
        };
        run(&mut state, name, &arguments)
    }

    const LEFV: &str = "## P0 — Critical\n\n- [ ] alpha\n  - [ ] child\n- [x] beta\n";

    #[test]
    fn list_returns_open_items_with_ids() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let listed = run_tool(&config, "todos_list", json!({})).unwrap();
        let items = listed["items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "beta is done and excluded: {items:?}");
        assert!(items.iter().all(|i| i["id"].as_str().unwrap().len() == 12));
        assert_eq!(items[0]["group"], "lefv");
    }

    #[test]
    fn list_can_include_done_items() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let listed = run_tool(&config, "todos_list", json!({"include_done": true})).unwrap();
        assert_eq!(listed["items"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn list_filters_by_group() {
        let (_dir, config) = fixture(&[("lefv", LEFV), ("jzlaw", "## P0\n\n- [ ] other\n")]);
        let listed = run_tool(&config, "todos_list", json!({"group": "jzlaw"})).unwrap();
        let items = listed["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["text"], "other");
    }

    #[test]
    fn list_accepts_mitodos_query_language() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let listed = run_tool(&config, "todos_list", json!({"query": "text:\"alpha\""})).unwrap();
        let items = listed["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["text"], "alpha");
    }

    // The agent can fix its own query if it is told what was wrong.
    #[test]
    fn a_malformed_query_reports_the_parsers_message() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let failure = run_tool(&config, "todos_list", json!({"query": "pri:"})).unwrap_err();
        assert_eq!(failure.0, "validation_error");
        assert!(!failure.1.is_empty());
    }

    #[test]
    fn get_item_returns_notes_and_direct_children() {
        let body = "## P0 — Critical\n\n- [ ] alpha\n  > why it matters\n  - [ ] child\n";
        let (_dir, config) = fixture(&[("lefv", body)]);
        let listed = run_tool(&config, "todos_list", json!({})).unwrap();
        let alpha = listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["text"] == "alpha")
            .unwrap()
            .clone();

        let full = run_tool(&config, "todos_get_item", json!({"id": alpha["id"]})).unwrap();
        assert_eq!(full["notes"], "why it matters");
        let children = full["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["text"], "child");
    }

    #[test]
    fn an_id_that_never_existed_is_a_conflict() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let failure =
            run_tool(&config, "todos_get_item", json!({"id": "ffffffffffff"})).unwrap_err();
        assert_eq!(failure.0, "conflict");
    }

    #[test]
    fn get_file_returns_the_markdown_verbatim() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let file = run_tool(&config, "todos_get_file", json!({"group": "lefv"})).unwrap();
        assert_eq!(file["text"], LEFV);
    }

    #[test]
    fn get_file_on_an_unknown_group_is_invalid_group() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let failure =
            run_tool(&config, "todos_get_file", json!({"group": "nope"})).unwrap_err();
        assert_eq!(failure.0, "invalid_group");
    }

    #[test]
    fn list_groups_counts_open_and_total() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let groups = run_tool(&config, "todos_list_groups", json!({})).unwrap();
        let first = &groups["groups"].as_array().unwrap()[0];
        assert_eq!(first["name"], "lefv");
        assert_eq!(first["open"], 2);
        assert_eq!(first["total"], 3);
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --quiet exec 2>&1 | tail -6`
Expected: `file not found for module exec`.

- [ ] **Step 3: Declare the module and route `call` into it**

In `src/mcp/mod.rs` add `pub mod exec;`, and replace the body of `call`:

```rust
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
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --quiet mcp 2>&1 | tail -6 && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3`
Expected: pass, clippy silent. If `Workspace`, `Item` or `Group` are not public at those paths, add the re-export to `src/store/mod.rs` rather than reaching into submodules.

- [ ] **Step 5: Verify against the real client**

```bash
cargo install --path . --force
claude --strict-mcp-config \
  --mcp-config '{"mcpServers":{"mitodo":{"command":"mitodo","args":["mcp-server"]}}}' \
  --allowedTools=mcp__mitodo__todos_list_groups \
  -p 'Call todos_list_groups and report each group with its open count.' < /dev/null
```
Expected: your seven real groups with counts matching `mitodo list`.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/mcp src/store/mod.rs
git commit -m "feat(mcp): the four read tools, over a freshly loaded workspace"
```

---

### Task 4: The item write tools, and the retired-id registry

**Files:**
- Modify: `src/mcp/exec.rs`
- Test: `src/mcp/exec.rs`

**Interfaces:**
- Consumes: `resolve`, `item_json`, `workspace`, `string_arg` (Task 3).
- Produces: dispatch arms for `todos_create_item`, `todos_add_child`, `todos_update_item`, `todos_set_notes`, `todos_delete_item`; `retire(state, id)` recording ids our writes invalidated.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `src/mcp/exec.rs`:

```rust
    /// Runs a tool against a state whose retired set persists across calls, so
    /// the not_found / conflict distinction can be exercised.
    fn session(config: &Config) -> (HashSet<String>, ()) {
        (HashSet::new(), ())
    }

    fn run_in(
        config: &Config,
        retired: &mut HashSet<String>,
        name: &str,
        arguments: Value,
    ) -> Result<Value, ToolFailure> {
        let mut state = ServerState {
            config,
            retired: std::mem::take(retired),
        };
        let result = run(&mut state, name, &arguments);
        *retired = state.retired;
        result
    }

    fn first_id(config: &Config, text: &str) -> String {
        let listed = run_tool(config, "todos_list", json!({"include_done": true})).unwrap();
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["text"] == text)
            .unwrap_or_else(|| panic!("no item {text:?}"))["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn create_item_appends_under_the_named_section() {
        let (dir, config) = fixture(&[("lefv", LEFV)]);
        let created = run_tool(
            &config,
            "todos_create_item",
            json!({"group": "lefv", "text": "gamma", "section": "P0"}),
        )
        .unwrap();
        assert_eq!(created["item"]["text"], "gamma");

        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(
            written,
            "## P0 — Critical\n\n- [ ] alpha\n  - [ ] child\n- [x] beta\n- [ ] gamma\n"
        );
    }

    #[test]
    fn create_item_into_an_unknown_group_is_invalid_group() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let failure = run_tool(
            &config,
            "todos_create_item",
            json!({"group": "nope", "text": "gamma"}),
        )
        .unwrap_err();
        assert_eq!(failure.0, "invalid_group");
    }

    #[test]
    fn create_item_refuses_empty_text() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let failure = run_tool(
            &config,
            "todos_create_item",
            json!({"group": "lefv", "text": "   "}),
        )
        .unwrap_err();
        assert_eq!(failure.0, "validation_error");
    }

    #[test]
    fn add_child_indents_two_beyond_its_parent() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let parent = first_id(&config, "alpha");
        run_tool(
            &config,
            "todos_add_child",
            json!({"parent_id": parent, "text": "nested"}),
        )
        .unwrap();
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(written, "## P0 — Critical\n\n- [ ] alpha\n  - [ ] nested\n");
    }

    #[test]
    fn update_item_can_tick_an_item_without_changing_its_id() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = first_id(&config, "alpha");
        let updated = run_tool(
            &config,
            "todos_update_item",
            json!({"id": id, "done": true}),
        )
        .unwrap();
        assert_eq!(updated["item"]["id"], id, "ticking does not change the id");
        assert_eq!(updated["item"]["done"], true);
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(written, "## P0 — Critical\n\n- [x] alpha\n");
    }

    #[test]
    fn update_item_returns_a_new_id_when_the_text_changed() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = first_id(&config, "alpha");
        let updated = run_tool(
            &config,
            "todos_update_item",
            json!({"id": id, "new_text": "alpha revised"}),
        )
        .unwrap();
        let new_id = updated["item"]["id"].as_str().unwrap().to_string();
        assert_ne!(new_id, id);
        assert_eq!(updated["item"]["text"], "alpha revised");
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(written, "## P0 — Critical\n\n- [ ] alpha revised\n");
    }

    #[test]
    fn update_item_does_both_in_one_write() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = first_id(&config, "alpha");
        run_tool(
            &config,
            "todos_update_item",
            json!({"id": id, "new_text": "alpha revised", "done": true}),
        )
        .unwrap();
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(written, "## P0 — Critical\n\n- [x] alpha revised\n");
    }

    #[test]
    fn update_item_with_neither_argument_is_a_validation_error() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = first_id(&config, "alpha");
        let failure = run_tool(&config, "todos_update_item", json!({"id": id})).unwrap_err();
        assert_eq!(failure.0, "validation_error");
    }

    // The whole point of the registry: our own edit reads differently from
    // someone else's.
    #[test]
    fn an_id_we_retired_ourselves_reports_not_found() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let mut retired = HashSet::new();
        let id = first_id(&config, "alpha");
        run_in(
            &config,
            &mut retired,
            "todos_update_item",
            json!({"id": id, "new_text": "alpha revised"}),
        )
        .unwrap();

        let failure =
            run_in(&config, &mut retired, "todos_get_item", json!({"id": id})).unwrap_err();
        assert_eq!(failure.0, "not_found", "we retired it, so say so");
    }

    #[test]
    fn set_notes_replaces_the_block_and_an_empty_string_removes_it() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = first_id(&config, "alpha");
        run_tool(&config, "todos_set_notes", json!({"id": id, "notes": "because"})).unwrap();
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(written, "## P0 — Critical\n\n- [ ] alpha\n  > because\n");

        run_tool(&config, "todos_set_notes", json!({"id": id, "notes": ""})).unwrap();
        let cleared = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(cleared, "## P0 — Critical\n\n- [ ] alpha\n");
    }

    #[test]
    fn delete_item_removes_the_line_and_its_notes() {
        let body = "## P0 — Critical\n\n- [ ] alpha\n  > gone too\n- [ ] beta\n";
        let (dir, config) = fixture(&[("lefv", body)]);
        let id = first_id(&config, "alpha");
        let deleted = run_tool(&config, "todos_delete_item", json!({"id": id})).unwrap();
        assert_eq!(deleted["deleted"], true);
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(written, "## P0 — Critical\n\n- [ ] beta\n");
    }

    // The project's core guarantee, re-pinned at this layer.
    #[test]
    fn a_write_leaves_every_other_line_byte_identical() {
        let body = "# Notes\r\n\r\n## P0 — Critical\r\n\r\n- [ ] alpha\r\n\r\n## P1 — Later\r\n\r\n- [ ] beta\r\n";
        let (dir, config) = fixture(&[("lefv", body)]);
        let id = first_id(&config, "alpha");
        run_tool(&config, "todos_update_item", json!({"id": id, "done": true})).unwrap();
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(
            written,
            body.replace("- [ ] alpha", "- [x] alpha"),
            "only the one line changed, CRLF preserved"
        );
    }
```

Delete the unused `session` helper if it is still present — it exists only as a reminder that state must persist across calls, and `run_in` supersedes it.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --quiet exec 2>&1 | tail -8`
Expected: failures reporting `unknown tool todos_create_item` and friends.

- [ ] **Step 3: Implement the write arms**

In `src/mcp/exec.rs`, extend the `run` match:

```rust
        "todos_create_item" => todos_create_item(state, arguments),
        "todos_add_child" => todos_add_child(state, arguments),
        "todos_update_item" => todos_update_item(state, arguments),
        "todos_set_notes" => todos_set_notes(state, arguments),
        "todos_delete_item" => todos_delete_item(state, arguments),
```

and add the bodies:

```rust
/// Remember an id our own write invalidated, so a later call can say so.
fn retire(state: &mut ServerState, id: &str) {
    state.retired.insert(id.to_string());
}

fn describe(err: store::WriteError) -> ToolFailure {
    match err {
        store::WriteError::Conflict { .. } => (
            "conflict",
            "the file changed on disk; re-read and retry".to_string(),
        ),
        other => ("validation_error", other.to_string()),
    }
}

fn group_by_name<'a>(
    workspace: &'a Workspace,
    name: &str,
) -> Result<&'a crate::store::model::Group, ToolFailure> {
    workspace
        .groups
        .iter()
        .find(|g| g.name == name)
        .ok_or_else(|| ("invalid_group", format!("no group named {name}")))
}

fn todos_create_item(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let group_name = string_arg(arguments, "group")?;
    let text = string_arg(arguments, "text")?;
    if text.trim().is_empty() {
        return Err(("validation_error", "text must not be empty".to_string()));
    }
    let workspace = workspace(state)?;
    let group = group_by_name(&workspace, &group_name)?;
    let section = arguments.get("section").and_then(|v| v.as_str());
    let notes = arguments.get("notes").and_then(|v| v.as_str()).unwrap_or("");
    let children: Vec<String> = arguments
        .get("children")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|c| c.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let path = group.todo_file.clone();
    store::create_item(&path, section, text.trim(), notes, &children).map_err(describe)?;

    let reloaded = workspace(state)?;
    let created = reloaded
        .items
        .iter()
        .filter(|i| i.file == path && i.text == text.trim())
        .next_back()
        .ok_or_else(|| ("conflict", "the item was written but not found".to_string()))?;
    Ok(json!({"item": item_json(created, &group_name), "created_heading": false}))
}

fn todos_add_child(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let parent_id = string_arg(arguments, "parent_id")?;
    let text = string_arg(arguments, "text")?;
    if text.trim().is_empty() {
        return Err(("validation_error", "text must not be empty".to_string()));
    }
    let workspace = workspace(state)?;
    let parent = resolve(state, &workspace, &parent_id)?;
    let (path, line, raw, indent) = (
        parent.file.clone(),
        parent.line,
        parent.raw.clone(),
        parent.indent,
    );
    store::add_item(&path, line, &raw, indent + 2, text.trim()).map_err(describe)?;

    let reloaded = workspace(state)?;
    let child = reloaded
        .items
        .iter()
        .find(|i| i.file == path && i.text == text.trim())
        .ok_or_else(|| ("conflict", "the child was written but not found".to_string()))?;
    let group = group_of(&reloaded, child);
    Ok(json!({"item": item_json(child, &group)}))
}

fn todos_update_item(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let id = string_arg(arguments, "id")?;
    let new_text = arguments.get("new_text").and_then(|v| v.as_str());
    let done = arguments.get("done").and_then(|v| v.as_bool());
    if new_text.is_none() && done.is_none() {
        return Err((
            "validation_error",
            "give new_text, done, or both".to_string(),
        ));
    }

    let workspace = workspace(state)?;
    let item = resolve(state, &workspace, &id)?;
    let (path, line, raw) = (item.file.clone(), item.line, item.raw.clone());

    // Toggle first: it keeps the line's text, so the expected raw for the edit
    // has to be re-read afterwards rather than reused.
    if let Some(done) = done {
        store::toggle(&path, line, &raw, done).map_err(describe)?;
    }
    if let Some(text) = new_text {
        let reloaded = workspace(state)?;
        let current = reloaded
            .items
            .iter()
            .find(|i| i.file == path && i.line == line)
            .ok_or_else(|| ("conflict", "the item moved mid-write".to_string()))?;
        store::edit_text(&path, line, &current.raw, text.trim()).map_err(describe)?;
        retire(state, &id);
    }

    let reloaded = workspace(state)?;
    let updated = reloaded
        .items
        .iter()
        .find(|i| i.file == path && i.line == line)
        .ok_or_else(|| ("conflict", "the item vanished after the write".to_string()))?;
    let group = group_of(&reloaded, updated);
    Ok(json!({"item": item_json(updated, &group)}))
}

fn todos_set_notes(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let id = string_arg(arguments, "id")?;
    let notes = string_arg(arguments, "notes")?;
    let workspace = workspace(state)?;
    let item = resolve(state, &workspace, &id)?;
    let (path, line, raw) = (item.file.clone(), item.line, item.raw.clone());
    store::set_description(&path, line, &raw, &notes).map_err(describe)?;

    let reloaded = workspace(state)?;
    let updated = reloaded
        .items
        .iter()
        .find(|i| i.file == path && i.line == line)
        .ok_or_else(|| ("conflict", "the item vanished after the write".to_string()))?;
    let group = group_of(&reloaded, updated);
    Ok(json!({"item": item_json(updated, &group)}))
}

fn todos_delete_item(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let id = string_arg(arguments, "id")?;
    let workspace = workspace(state)?;
    let item = resolve(state, &workspace, &id)?;
    let (path, line, raw) = (item.file.clone(), item.line, item.raw.clone());
    store::delete_item(&path, line, &raw).map_err(describe)?;
    retire(state, &id);
    Ok(json!({"deleted": true}))
}
```

`run`'s signature already takes `&mut ServerState`; change the read helpers' first parameter to `&ServerState` where they do not retire, and pass `&*state` at those call sites so the borrow checker is satisfied.

- [ ] **Step 4: Run the tests**

Run: `cargo test --quiet exec 2>&1 | tail -8 && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3`
Expected: pass, clippy silent.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/mcp
git commit -m "feat(mcp): item writes, with a registry that tells our edits from drift"
```

---

### Task 5: Archive, group creation, sync, and containment

**Files:**
- Modify: `src/mcp/exec.rs`
- Test: `src/mcp/exec.rs`

**Interfaces:**
- Consumes: everything from Tasks 3–4.
- Produces: dispatch arms for `todos_archive_item`, `todos_archive_finished`, `todos_create_group`, `todos_sync`; `valid_group_name(name) -> Result<(), ToolFailure>`.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `src/mcp/exec.rs`:

```rust
    fn fixture_with_archive(files: &[(&str, &str)]) -> (tempfile::TempDir, Config) {
        let (dir, mut config) = fixture(files);
        config.workspace.archive_dir = Some("_archive".to_string());
        (dir, config)
    }

    #[test]
    fn archive_item_moves_it_into_the_archive_file() {
        let (dir, config) = fixture_with_archive(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n- [ ] beta\n")]);
        let id = first_id(&config, "alpha");
        let archived = run_tool(&config, "todos_archive_item", json!({"id": id})).unwrap();
        assert_eq!(archived["archived"], true);

        let left = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(left, "## P0 — Critical\n\n- [ ] beta\n");
        let moved =
            std::fs::read_to_string(dir.path().join("lefv/_archive/TODO.md")).unwrap();
        assert!(moved.contains("- [ ] alpha"), "verbatim: {moved}");
        assert!(moved.contains("## Archived "), "under a dated heading: {moved}");
    }

    #[test]
    fn archiving_without_an_archive_dir_is_invalid_group() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = first_id(&config, "alpha");
        let failure = run_tool(&config, "todos_archive_item", json!({"id": id})).unwrap_err();
        assert_eq!(failure.0, "invalid_group");
        assert!(failure.1.contains("archive_dir"), "{}", failure.1);
    }

    #[test]
    fn archive_finished_sweeps_done_items_and_reports_skips() {
        let body = "## P0 — Critical\n\n- [x] done one\n- [x] parent\n  - [ ] still open\n- [ ] open\n";
        let (dir, config) = fixture_with_archive(&[("lefv", body)]);
        let report =
            run_tool(&config, "todos_archive_finished", json!({"group": "lefv"})).unwrap();
        assert_eq!(report["archived"], 1);
        let skipped = report["skipped"].as_array().unwrap();
        assert_eq!(skipped.len(), 1, "the parent has open work: {skipped:?}");

        let left = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert!(!left.contains("done one"));
        assert!(left.contains("parent") && left.contains("still open"));
    }

    #[test]
    fn create_group_seeds_a_file_with_the_existing_headings() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n\n## P1 — Later\n")]);
        let created = run_tool(&config, "todos_create_group", json!({"name": "newone"})).unwrap();
        assert_eq!(created["name"], "newone");

        let seeded = std::fs::read_to_string(dir.path().join("newone/TODO.md")).unwrap();
        assert!(seeded.contains("## P0 — Critical"), "copied: {seeded:?}");
        assert!(seeded.contains("## P1 — Later"), "copied: {seeded:?}");
        assert!(!seeded.contains("alpha"), "headings only, not items: {seeded:?}");
    }

    // Seeding exists so the next create has a section to land in; c5e27be fixed
    // the dead end an unseeded file produced.
    #[test]
    fn an_item_can_be_created_in_a_freshly_created_group() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        run_tool(&config, "todos_create_group", json!({"name": "newone"})).unwrap();
        let created = run_tool(
            &config,
            "todos_create_item",
            json!({"group": "newone", "text": "first", "section": "P0"}),
        )
        .unwrap();
        assert_eq!(created["item"]["text"], "first");
    }

    #[test]
    fn create_group_refuses_a_name_that_already_exists() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let failure =
            run_tool(&config, "todos_create_group", json!({"name": "lefv"})).unwrap_err();
        assert_eq!(failure.0, "validation_error");
    }

    // The containment boundary: a group name is a name, never a path.
    #[test]
    fn create_group_rejects_names_that_escape_the_workspace() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        for name in ["../evil", "/etc/passwd", ".git", "a/b", "..", "a\\b"] {
            let failure = run_tool(&config, "todos_create_group", json!({"name": name}))
                .unwrap_err();
            assert_eq!(failure.0, "invalid_group", "{name} should be refused");
        }
        assert!(!dir.path().parent().unwrap().join("evil").exists());
        assert!(!dir.path().join(".git").exists());
    }

    #[test]
    fn sync_is_refused_when_git_is_disabled() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let failure = run_tool(&config, "todos_sync", json!({})).unwrap_err();
        assert_eq!(failure.0, "git_disabled");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --quiet exec 2>&1 | tail -8`
Expected: `unknown tool todos_archive_item` and friends.

- [ ] **Step 3: Implement the arms**

Extend the `run` match:

```rust
        "todos_archive_item" => todos_archive_item(state, arguments),
        "todos_archive_finished" => todos_archive_finished(state, arguments),
        "todos_create_group" => todos_create_group(state, arguments),
        "todos_sync" => todos_sync(state),
```

and add:

```rust
/// A group name is a name, never a path: the server joins it onto the workspace
/// root, so anything that could escape is refused.
fn valid_group_name(name: &str) -> Result<(), ToolFailure> {
    let bad = name.trim().is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name == ".."
        || name.starts_with('.');
    if bad {
        return Err((
            "invalid_group",
            format!("{name:?} is not a valid group name"),
        ));
    }
    Ok(())
}

fn archive_dir_of(
    workspace: &Workspace,
    group: &crate::store::model::Group,
) -> Result<std::path::PathBuf, ToolFailure> {
    let _ = workspace;
    group.archive_dir.clone().ok_or_else(|| {
        (
            "invalid_group",
            format!("no archive_dir configured for {}", group.name),
        )
    })
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn todos_archive_item(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let id = string_arg(arguments, "id")?;
    let workspace = workspace(state)?;
    let item = resolve(state, &workspace, &id)?;
    let group = workspace
        .groups
        .iter()
        .find(|g| g.todo_file == item.file)
        .ok_or_else(|| ("invalid_group", "the item's group is unknown".to_string()))?;
    let archive = archive_dir_of(&workspace, group)?;
    let group_name = group.name.clone();
    let payload = item_json(item, &group_name);

    let report = store::archive_items(&item.file, &archive, &[item], &today()).map_err(describe)?;
    if let Some(reason) = report.skipped.first() {
        return Err(("conflict", reason.clone()));
    }
    retire(state, &id);
    Ok(json!({"item": payload, "archived": true}))
}

fn todos_archive_finished(
    state: &mut ServerState,
    arguments: &Value,
) -> Result<Value, ToolFailure> {
    let name = string_arg(arguments, "group")?;
    let workspace = workspace(state)?;
    let group = group_by_name(&workspace, &name)?;
    let archive = archive_dir_of(&workspace, group)?;
    let report = store::archive_done(&group.todo_file, &archive, &workspace.items, &today())
        .map_err(describe)?;
    Ok(json!({"archived": report.archived, "skipped": report.skipped}))
}

fn todos_create_group(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let name = string_arg(arguments, "name")?;
    valid_group_name(&name)?;
    let workspace = workspace(state)?;
    let root = workspace.root.clone();
    let dir = root.join(&name);
    if dir.exists() {
        return Err(("validation_error", format!("{name} already exists")));
    }

    // Copy the section headings an existing group uses, so a new group matches
    // the convention already in the workspace.
    let seed = match workspace.groups.first() {
        Some(existing) => std::fs::read_to_string(&existing.todo_file)
            .map(|text| {
                text.lines()
                    .filter(|line| line.starts_with("## "))
                    .map(|line| format!("{line}\n\n"))
                    .collect::<String>()
            })
            .unwrap_or_default(),
        None => String::new(),
    };
    let seed = if seed.is_empty() {
        match state.config.priority.source {
            crate::config::PrioritySource::Heading => {
                "## P0\n\n## P1\n\n## P2\n\n## P3\n\n".to_string()
            }
            _ => String::new(),
        }
    } else {
        seed
    };

    std::fs::create_dir_all(&dir).map_err(|e| ("validation_error", e.to_string()))?;
    let todo_file = dir.join("TODO.md");
    std::fs::write(&todo_file, seed).map_err(|e| ("validation_error", e.to_string()))?;
    Ok(json!({"name": name, "todo_file": todo_file.to_string_lossy()}))
}

fn todos_sync(state: &ServerState) -> Result<Value, ToolFailure> {
    if !state.config.git.enabled {
        return Err(("git_disabled", "[git] enabled is false".to_string()));
    }
    let workspace = workspace(state)?;
    let outcome = crate::git::run_sync(&workspace.root, &state.config.git.sync, "git");
    Ok(json!({"ok": outcome.ok, "transcript": outcome.transcript}))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --quiet exec 2>&1 | tail -8 && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3`
Expected: pass, clippy silent.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/mcp
git commit -m "feat(mcp): archive, group creation and sync, with names never paths"
```

---

### Task 6: Documentation, and the gate

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Document it in the README**

After the agent section, add:

```markdown
## Using mitodo from Claude Code or Codex

`mitodo mcp-server` speaks the Model Context Protocol on stdio, so an agent can
read and manage the workspace itself:

```sh
claude --strict-mcp-config \
  --mcp-config '{"mcpServers":{"mitodo":{"command":"mitodo","args":["mcp-server"]}}}' \
  -p 'archive the everlongtech items that closed, and add a P1 to chase Sam'
```

Codex reads the same server from its own config:

```sh
codex mcp add mitodo -- mitodo mcp-server
```

Thirteen tools are exposed: `todos_list`, `todos_get_item`, `todos_get_file`,
`todos_list_groups`, `todos_create_item`, `todos_add_child`, `todos_update_item`,
`todos_set_notes`, `todos_delete_item`, `todos_archive_item`,
`todos_archive_finished`, `todos_create_group` and `todos_sync`.

Writes go through the same conflict-aware writer the TUI uses, so a stale edit is
refused rather than forced, and every line you did not change stays
byte-identical. **There is no review step:** an agent's writes take effect
immediately, and your workspace being a git repository is the undo. If the TUI is
open it picks the changes up through its file watcher.

Tools take a group *name*, never a path, so an agent cannot address anything
outside the workspace.
```

- [ ] **Step 2: Run the full gate**

```bash
cargo test 2>&1 | grep -E '^test result'
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | grep -cE '^(warning|error)'
cargo fmt --check && echo FMT-OK
```
Expected: one `test result: ok` with 0 failed and 0 ignored; clippy count `0`; `FMT-OK`.

- [ ] **Step 3: Check that no diagnostic can reach stdout**

```bash
grep -rnE '\bprintln!|\bprint!|\bdbg!' src/mcp/ && echo "FOUND — must be eprintln! or removed" || echo "clean: nothing writes to stdout but the protocol"
```
Expected: `clean`.

- [ ] **Step 4: Check the comment budget**

```bash
comments=$(git diff main -U0 -- '*.rs' | grep -cE '^\+\s*//')
added=$(git diff main -U0 -- '*.rs' | grep -cE '^\+')
echo "$comments of $added added rust lines = $((100*comments/added))%"
```
Expected: under ~10%.

- [ ] **Step 5: Smoke-test against the live workspace, read-only first**

```bash
cargo install --path . --force
claude --strict-mcp-config \
  --mcp-config '{"mcpServers":{"mitodo":{"command":"mitodo","args":["mcp-server"]}}}' \
  --allowedTools=mcp__mitodo__todos_list_groups,mcp__mitodo__todos_list \
  -p 'Which of my groups has the most open P0 items? Use todos_list with a query.' < /dev/null
```
Expected: an answer consistent with `mitodo -q 'pri:P0 !done' list`. **Read-only tools only** at this step.

- [ ] **Step 6: Smoke-test one write, in a scratch workspace**

```bash
scratch=$(mktemp -d); mkdir -p "$scratch/demo"
printf '## P0 — Critical\n\n- [ ] existing\n' > "$scratch/demo/TODO.md"
mkdir -p "$scratch/cfg"
cat > "$scratch/cfg/config.toml" <<TOML
[workspace]
root = "$scratch"
group_by = "directory"
todo_glob = "*/TODO.md"
archive_dir = "_archive"
[priority]
source = "heading"
pattern = "^P([0-3])"
TOML
claude --strict-mcp-config \
  --mcp-config "{\"mcpServers\":{\"mitodo\":{\"command\":\"mitodo\",\"args\":[\"-c\",\"$scratch/cfg\",\"mcp-server\"]}}}" \
  --allowedTools=mcp__mitodo__todos_create_item,mcp__mitodo__todos_list \
  -p 'Add a P0 item "smoke test" to the demo group, then list the group.' < /dev/null
cat "$scratch/demo/TODO.md"
```
Expected: the file gains `- [ ] smoke test` and `- [ ] existing` is untouched. Never run the first write smoke test against `~/repos/TODO`.

- [ ] **Step 7: Commit**

```bash
git add README.md
git commit -m "docs: driving mitodo from Claude Code or Codex over MCP"
```

---

## Self-review

**Spec coverage.** §1 intent → Tasks 1–6. §2 decisions: transport and echoing the version → Task 1; ids → Tasks 3–4 (`resolve`, retirement); search via the query language → Task 3; reuse of `store::write` → Tasks 4–5; no shared state → Task 3's per-call `Workspace::load`. §3's adopted contracts: merged `update_item` → Task 4; the error envelope → Task 1's `tool_error`; the retired-id registry → Task 4. §4's wire protocol → Task 1, replayed from the fixture. §5 architecture → the four modules across Tasks 1–3. §6's thirteen tools → Task 2's table, Tasks 3–5's bodies, including every "coin toss" detail: merged update, `section` rather than `heading_path`, `add_child` indent + 2, the query parser's message, direct children only, archive needing `archive_dir`, sync needing `[git] enabled`, and `create_group` seeding. §7 errors → Task 1 for the protocol codes, Tasks 3–5 for every listed tool code. §8 containment → Task 5's `valid_group_name` plus its escape test. §9 testing → every task's test steps, with the formatting guarantee in Task 4 and the protocol fixture in Task 1. §10 out-of-scope → nothing in any task. §12 files → Tasks 1–6.

**`missing_priority_section` is implemented rather than dropped.** An earlier revision of this plan noted the code was unreachable, because `store::create_item` appends at end of file instead of refusing, and proposed leaving it unused. That was the wrong call: priority is derived from the heading above an item, so a silent end-of-file placement hands the agent an item whose priority is not the one it asked for. `todos_create_item` therefore reads the file's `## ` headings first and refuses, writing nothing — covered by `creating_under_a_section_the_file_lacks_is_refused`, which also asserts the file is byte-identical afterwards.

**Placeholder scan.** No TBD/TODO/"handle errors appropriately". Task 3 step 4 and Task 4 step 3 both say "if X is not public, add the re-export" — that is an instruction to check a fact in the codebase, with the exact fix named, not deferred work. Task 4 step 1 asks for a stray helper to be deleted, which is explicit cleanup rather than vagueness.

**Type consistency.** `ServerState { config, retired }` is identical in Tasks 1, 3, 4, 5. `handle_line(&mut ServerState, &str) -> Option<String>` matches between Task 1's definition and Tasks 2–3's dispatch. `exec::run(&mut ServerState, &str, &Value) -> Result<Value, ToolFailure>` matches Tasks 3, 4, 5, and `ToolFailure = (&'static str, String)` is used consistently. `item_json(&Item, &str) -> Value` matches Tasks 3–5. `protocol::{tool_result, tool_error}` signatures match Task 1's definitions and Task 3's call site. Tool names in Task 2's table are the ones Tasks 3–5 dispatch, spelled identically.
