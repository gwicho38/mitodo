# `mitodo mcp-server` — Design

**Date:** 2026-08-12
**Status:** Approved for planning
**Sub-project:** 1 of 4 toward a conversational agent (see §10)
**Builds on:** [2026-08-11-model-services-and-agent-popup-design.md](2026-08-11-model-services-and-agent-popup-design.md), [2026-08-12-command-palette-design.md](2026-08-12-command-palette-design.md)

---

## 1. What this is

A new subcommand, `mitodo mcp-server`, that speaks the Model Context Protocol over
stdio and exposes mitodo's operations as tools. Point Claude Code or Codex at it
and the agent can read and manage your todos itself:

```
claude --strict-mcp-config \
  --mcp-config '{"mcpServers":{"mitodo":{"command":"mitodo","args":["mcp-server"]}}}' \
  -p 'archive the everlongtech items that closed, and add a P1 to chase Sam'
```

It ships value on its own, with no changes to the TUI. It is also the foundation
for sub-project 2, where a chat popup inside mitodo spawns exactly that command.

**Why a server rather than a tool loop inside mitodo.** `claude` and `codex` are
both MCP clients with their own reasoning loop, permission model and turn
management. Handing them a tool catalogue means mitodo does not reimplement any
of that. The agent's loop is the agent's problem; mitodo's job is to expose
operations and keep the files correct.

---

## 2. Decisions

| Decision | Chosen | Alternative rejected |
|---|---|---|
| Transport | MCP over stdio, line-delimited JSON-RPC 2.0 | A hand-rolled prompt-and-parse loop inside mitodo (reimplements the agent's loop, loses its permission model) |
| Protocol version | **Echo whatever the client sends** | Assert a fixed version (the live client sends `2025-11-25` while the published spec has moved to a stateless `2026-07-28`; asserting either breaks the other) |
| Writes | Apply immediately; git is the undo | Per-call approval round-tripped to the TUI (hardest piece in the system); batching into the review pane (agent cannot see its own effects mid-conversation) |
| Item addressing | `ItemId` hex, returned by every read | Group + text substring (ambiguous across groups); group + line number (brittle across edits) |
| Tool surface | 15 tools: 4 reads, 10 writes, `git_sync` | Adding `set_query` (it changes TUI state in another process — belongs in sub-project 2); adding UI-control tools (same reason) |
| Write path | Reuse `store::write` verbatim | Tools writing markdown directly (would break the byte-preservation guarantee) |
| Shared state | None. Files on disk are the only channel | IPC to a running TUI (the TUI's `notify` watcher already refreshes on file change) |

---

## 3. The wire protocol, as captured from a live client

Not taken from the specification. `claude 2.1.228` was pointed at a stub server
that logged its stdin; the exchange is saved verbatim at
[resources/mcp-client-handshake.log](../../../resources/mcp-client-handshake.log)
and is the fixture the protocol tests replay.

```
→ {"method":"initialize","params":{"protocolVersion":"2025-11-25",
     "capabilities":{"roots":{"listChanged":true},"elicitation":{}},
     "clientInfo":{"name":"claude-code","title":"Claude Code","version":"2.1.228",…}},
   "jsonrpc":"2.0","id":0}
← {"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-11-25",
     "capabilities":{"tools":{}},
     "serverInfo":{"name":"mitodo","version":"<crate version>"}}}

→ {"method":"notifications/initialized","jsonrpc":"2.0"}        no id → no reply

→ {"method":"tools/list","jsonrpc":"2.0","id":1}
← {"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":…,"description":…,
                                             "inputSchema":{…}},…]}}

→ {"method":"tools/call","params":{"name":"ping","arguments":{},
     "_meta":{"claudecode/toolUseId":"toolu_…","progressToken":2}},"jsonrpc":"2.0","id":2}
← {"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"…"}]}}
```

Four consequences, each one a thing that would otherwise be guessed wrong:

- **The installed client uses the classic handshake at `2025-11-25`.** The
  published `2026-07-28` spec retires `initialize`/`initialized` entirely in
  favour of per-request `_meta`. Implementing only the current spec would produce
  a server no installed client can talk to. Echoing the client's version, and
  never *requiring* `initialize` before serving `tools/list`, satisfies both.
- **Line-delimited JSON**, one object per line. No `Content-Length` framing —
  that belongs to the HTTP transport.
- **`notifications/initialized` has no `id`** and must receive no response.
  Replying to a notification is a protocol violation.
- **`tools/call` carries `_meta`** with a progress token; a minimal server
  ignores it.

### stdout is protocol-only

Every diagnostic goes to stderr. A stray `println!` corrupts the stream and
surfaces as an unexplained client-side parse failure — the easiest way to break
an MCP server, and worth a lint-level habit rather than a comment.

---

## 4. Architecture

```
  mitodo mcp-server                    a subcommand: no TUI, no terminal setup
  ┌──────────────────────────────────────────────────────────────────────┐
  │  stdin  ── line-delimited JSON-RPC 2.0 ──▶  dispatch                 │
  │                                               │                      │
  │   initialize              ──▶ echo protocolVersion, {"tools":{}}      │
  │   notifications/initialized ──▶ ignore                                │
  │   tools/list              ──▶ 15 schemas from one static table        │
  │   tools/call              ──▶ run it; _meta ignored                   │
  │   other, with an id       ──▶ −32601                                  │
  │   other, without one      ──▶ ignore                                  │
  │                                               │                      │
  │  stdout ◀── one JSON object per line ─────────┘                      │
  │  stderr ◀── logs only, never protocol                                │
  └──────────────────────────────────────────────────────────────────────┘
                                    │ every tool
                                    ▼
                     store::{parse_todo_file, write, archive}  ← unchanged
                     conflict-aware, byte-preserving
```

New module layout, keeping files focused:

| File | Responsibility |
|---|---|
| `src/mcp/mod.rs` | the stdio loop: read a line, dispatch, write a line |
| `src/mcp/protocol.rs` | JSON-RPC envelope types, error codes, response builders |
| `src/mcp/tools.rs` | the static tool table: name, description, `inputSchema` |
| `src/mcp/exec.rs` | argument parsing and each tool's call into `store` |

`src/cli.rs` gains `Command::McpServer`; `src/main.rs` dispatches to
`mcp::serve(config)` before any terminal setup, since this process has no UI.

**No shared state with the TUI.** Each call re-reads the workspace, resolves the
item, and writes through `verify`. A TUI open on the same workspace refreshes via
its existing `notify` watcher. Two processes, one source of truth on disk.

---

## 5. The tools

Every read returns `ItemId` hex strings; every write takes them. `ItemId::compute`
hashes `(file, section, heading, indent, text)`, so an id survives completing,
archiving and note edits, and **changes when the text changes** —
`edit_item_text` therefore returns the new id.

```
 reads                                                    maps to
 ──────────────────────────────────────────────────────────────────────────────
 list_items{query?, group?}   → [{id, text, done, priority, group, section,
                                  due, has_notes, parent}]
                                              query::Query + store parse
 get_item{id}                 → {…, notes, children:[{id, text, done}]}
 list_groups{}                → [{name, todo_file, open, total, has_notes,
                                  archive_dir?}]
 read_group_notes{group}      → {text}                     notes sidecar

 writes                                                   maps to
 ──────────────────────────────────────────────────────────────────────────────
 add_item{group, text, section?, notes?, children?} → {id} store::create_item
 add_child{parent_id, text}   → {id}                       store::add_item
 complete_item{id}            → {}                         store::toggle(true)
 uncomplete_item{id}          → {}                         store::toggle(false)
 edit_item_text{id, text}     → {id}   ← the new id        store::edit_text
 set_item_notes{id, notes}    → {}                         store::set_description
 delete_item{id}              → {}                         store::delete_item
 archive_item{id}             → {}                         store::archive_items
 archive_finished{group}      → {archived, skipped[]}      store::archive_done
 create_group{name}           → {todo_file}                mkdir + seeded TODO.md

 session                                                   maps to
 ──────────────────────────────────────────────────────────────────────────────
 git_sync{}                   → {ok, transcript}           crate::git
```

### Details that are otherwise a coin toss

- **`add_child` indents by two beyond its parent** and anchors to the parent's
  line, via `store::add_item(path, parent.line, parent.raw, parent.indent + 2,
  text)`. That is what the TUI's own `A` key does.
- **`list_items{query}` parses the string with `query::Query::parse`.** A
  malformed query is an `isError` result carrying the parser's message, not a
  protocol error — the agent can then fix its own query.
- **`get_item{id}.children` are direct children only**, not the whole subtree.
  The agent walks deeper by calling `get_item` on a child id.
- **`archive_item` and `archive_finished` need the group's `archive_dir`.**
  Absent, they return `isError` with
  `no archive_dir configured for <group>` — the same rule `changeset::apply`
  already follows.
- **`git_sync` requires `[git] enabled = true`.** Otherwise it returns `isError`
  with `git sync is disabled in config`, mirroring the TUI's `s`.

Three notes:

- **`create_group`** is the only tool that creates a directory. It writes
  `<root>/<name>/TODO.md`, seeding it with the `## ` headings **copied from the
  first existing group's file**, so a new group matches the convention already in
  use. With no group to copy from, it seeds `## P0` … `## P3` when
  `[priority] source = "heading"`, and an empty file otherwise. Seeding matters:
  without a section the next `add_item` would hit the empty-file dead end fixed
  in `c5e27be`.
- **`archive_finished` keeps its guard**: it refuses an item whose subtree still
  holds open work and lists it in `skipped`, exactly as `X` does.
  `archive_item` is single and deliberate, and does not.
- **`set_query` is deliberately absent.** It would change what *your screen*
  shows — TUI state in another process. It belongs in sub-project 2, where the
  popup runs inside the TUI and can set it directly.

---

## 6. Errors

| Failure | Channel | Example |
|---|---|---|
| malformed JSON on stdin | `error` **−32700** | a truncated line |
| request with an `id` naming an unknown method | **−32601** | `tools/foo` |
| `tools/call` missing `name`, or a bad argument type | **−32602** | `complete_item{}` |
| **the tool ran and failed** | `result` with **`isError: true`** | `no item with id a1b2c3d4e5f6` |
| a notification we do not handle | nothing | `notifications/cancelled` |

A tool failure is a *result*, not an error. The agent reads `isError` text and can
react — "that id is stale, list again" — whereas a JSON-RPC error is a
transport-level fault that may abort its turn. So `file changed on disk`,
`no such group` and `name already exists` are all `isError` results.

Shutdown follows the stdio rule: stdin closes → EOF → exit 0. The client owns our
lifetime, so no heartbeat is needed.

---

## 7. The containment boundary

**No tool accepts a path.** Tools take a *group name*, and the server joins it
onto the workspace root itself. A name is rejected when it contains `/` or `\`,
equals `..`, or begins with `.`. The agent therefore cannot address anything
outside the workspace even by mistake.

There is no shell in the server. `git_sync` runs only the argv list already in
`[git] sync` in the user's config.

Requests are handled one at a time, sequentially. stdio is a single stream and
the workspaces are small (seven groups, a few hundred items), so re-reading per
call costs less than the complexity of a cache that could go stale against a TUI
editing the same files.

---

## 8. Testing

No model and no network in the suite.

```
 protocol      the recorded transcript replays clean:
                 initialize (2025-11-25) → response echoes that version
                 notifications/initialized → stdout stays empty
                 tools/list → 15 tools, each inputSchema valid JSON Schema
                 tools/call → result with content[0].type == "text"
               malformed line → −32700;  unknown method with id → −32601
               unknown tool name → isError result, not a protocol error
               tools/list before any initialize → still works

 tools         table-driven, a temp workspace each:
                 every write tool's file effect asserted byte-for-byte
                 every read tool's JSON shape asserted
                 a stale id → isError "no item with id …"
                 a file changed between read and write →
                   isError "file changed on disk", file untouched
                 archive_finished still refuses a subtree with open work
                 edit_item_text returns an id that resolves afterwards
                 add_child lands two spaces deeper than its parent
                 a malformed query → isError carrying the parser's message
                 archive with no archive_dir → isError, file untouched
                 git_sync with [git] enabled=false → isError
                 create_group copies the headings from an existing group

 containment   group names "../evil", "/etc/passwd", ".git", "a/b" rejected,
               and no file is created anywhere

 formatting    the project's core guarantee, re-pinned at this layer:
                 add_item into a real fixture leaves every other line
                 byte-identical, including line endings and final newline
```

**Not in CI:** a live run driving real `claude --mcp-config` against the built
binary. It needs network and a model, so it is a documented manual smoke step;
the plan spells out the exact command and what to look for.

---

## 9. Out of scope

- The chat popup inside the TUI — sub-project 2
- Retiring the seven verbs and deleting the review layer — sub-project 3
- Ollama tool-calling over its HTTP API — sub-project 4
- `set_query` and any tool that drives the interface
- MCP resources, prompts, sampling, elicitation, or progress notifications
- The stateless `2026-07-28` request shape, beyond not requiring `initialize`
- Serving MCP over HTTP or SSE
- Concurrent request handling

---

## 10. Where this sits

| # | Sub-project | Status |
|---|---|---|
| **1** | `mitodo mcp-server` — this spec | designing |
| 2 | Chat popup driving claude/codex over MCP | after 1 |
| 3 | Retire the seven verbs, delete the review layer (~700 lines) | after 2 |
| 4 | Ollama tool-calling over its HTTP API | after 2 |

Decisions already taken that shape 2–4: writes apply immediately with git as the
undo; all seven verb keys retire in favour of one chat key; the review pane is
deleted once nothing calls it; ollama gets a hand-rolled loop against
`127.0.0.1:11434` because its CLI cannot be given our tools.

---

## 11. Files touched

| File | Change |
|---|---|
| `src/mcp/mod.rs` | New: the stdio serve loop |
| `src/mcp/protocol.rs` | New: JSON-RPC envelopes, error codes, response builders |
| `src/mcp/tools.rs` | New: the 15-tool static table with `inputSchema`s |
| `src/mcp/exec.rs` | New: argument parsing and calls into `store` |
| `src/cli.rs` | `Command::McpServer` |
| `src/main.rs` | dispatch before terminal setup |
| `src/store/mod.rs` | re-export anything the tools need that is currently private |
| `resources/mcp-client-handshake.log` | the captured transcript, as a test fixture |
| `README.md` | how to point Claude Code or Codex at it |

No new dependencies: `serde_json` and `serde` are already present, and the loop
uses `std::io` only.
