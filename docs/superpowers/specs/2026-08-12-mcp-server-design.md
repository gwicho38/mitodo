# `mitodo mcp-server` — Design

**Date:** 2026-08-12 (rewritten as a port)
**Status:** Approved for planning
**Sub-project:** 1 of 4 toward a conversational agent (see §11)
**Ports:** [`~/repos/todos-mcp`](https://github.com/gwicho38/todos-mcp) — a working
Python MCP server over the same workspace. Its tool surface, semantics and error
codes are adopted; its SQLite/FTS5 index is not.
**Builds on:** [2026-08-11-model-services-and-agent-popup-design.md](2026-08-11-model-services-and-agent-popup-design.md), [2026-08-12-command-palette-design.md](2026-08-12-command-palette-design.md)

---

## 1. What this is

A new subcommand, `mitodo mcp-server`, speaking the Model Context Protocol over
stdio and exposing mitodo's operations as tools. Point Claude Code or Codex at it
and the agent manages your todos itself:

```
claude --strict-mcp-config \
  --mcp-config '{"mcpServers":{"mitodo":{"command":"mitodo","args":["mcp-server"]}}}' \
  -p 'archive the everlongtech items that closed, and add a P1 to chase Sam'
```

It ships value on its own with no TUI changes, and is the foundation for
sub-project 2, where a chat popup inside mitodo spawns exactly that command.

**Why port rather than build fresh.** `todos-mcp` already solved this problem
against this workspace, and its contracts are better than the ones first drafted
here: one write tool that toggles *and* edits in a single conflict-aware pass,
machine-readable error codes, and a distinction between "you already changed that"
and "someone else did".

**Why port rather than depend on it.** A Rust port is one binary with no Python
runtime, reuses `store::write` so the byte-preservation guarantee is the one
mitodo already tests, and leaves a single writer on the workspace.

---

## 2. Decisions

| Decision | Chosen | Alternative rejected |
|---|---|---|
| Implementation | Rust, in-repo, ported from todos-mcp | Depend on todos-mcp (adds a Python runtime to the chat feature); invent a fresh surface (worse contracts, per §3) |
| Transport | MCP over stdio, line-delimited JSON-RPC 2.0 | A hand-rolled prompt-and-parse loop — reimplements the agent's loop, loses its permission model |
| Protocol version | **Echo whatever the client sends** | Assert a fixed one: the live client sends `2025-11-25`, the published spec has moved to a stateless `2026-07-28` |
| Writes | Apply immediately; git is the undo | Per-call approval across a process boundary; batching into the review pane |
| Item ids | mitodo's `ItemId`, now carrying an occurrence index (`3961186`) | todos-mcp's blake2b/16 scheme — compatibility with a server this replaces |
| Search | mitodo's existing query language | Port FTS5/BM25 (needs rusqlite and an index that can drift); fuzzy scoring (a scorer tuned for 41 short labels) |
| Write path | Reuse `store::write` verbatim | Tools writing markdown directly, breaking byte preservation |
| Shared state | None — files on disk are the only channel | IPC to a running TUI; its `notify` watcher already refreshes on change |

---

## 3. What the port adopts, and what it leaves behind

Read from `~/repos/todos-mcp/src/todos_mcp/{server,writer,parser}.py`.

**Adopted, because it beats the first draft written here:**

| todos-mcp | first draft | why theirs wins |
|---|---|---|
| `update_item{id, new_text?, done?}` → item with its **new id** | `complete_item` + `uncomplete_item` + `edit_item_text` | toggle and edit in one conflict-aware write; no half-applied pair |
| envelope `{"error": code, "message": …}`, codes `conflict`, `not_found`, `ambiguous_match`, `invalid_priority`, `invalid_account`, `validation_error` | free-text `isError` | the agent branches on a code instead of parsing prose |
| re-locate the target by identity tuple on re-read, then write | resolve once, then write | drift between read and write is detected, not assumed away |
| a retired-id registry: an id this writer invalidated reports `not_found`, one that drifted out-of-band reports `conflict` | one `conflict` for both | tells the agent "you already changed that" apart from "someone else did" |

**Considered and deferred:** todos-mcp's `heading_path: [string]`, matched at any
heading level, is better than a two-level `section` — it handles `###` and deeper,
which mitodo's parser flattens. Adopting it means teaching `store::parse` to carry
a path rather than `section` + `heading`, which is a change to a core type for a
gain no current workspace needs. `todos_create_item` therefore takes `section`,
and §10 records the deferral.

**Left behind:**

- **`todos_search`, `todos_status`, `todos_reindex`** — all three serve the
  SQLite/FTS5 index. Search becomes `todos_list{query}` over mitodo's query
  language, already implemented, tested and documented.
- **The index and its watcher.** Each call re-reads the workspace. Seven groups
  and a few hundred items cost less than a cache that can drift against a TUI
  editing the same files.
- **`normalized_text` in the id.** mitodo hashes raw text; changing that would
  rewrite every id for no gain here.
- **Note-context records** (`notes.md` parsed into heading/prose/pointer/bullet
  kinds). `todos_get_file` returns a group's notes verbatim instead.

**Two claims corrected while reading the source:**

1. An earlier draft of this design said the two id schemes matched, quoting
   mitodo's own doc comment. They never did — sha256/12 over
   `(file, section, heading, indent, text)` here versus blake2b/16 over
   `(file, heading_path, normalized_text, occurrence_index)` there;
   `b5795463985e` versus `bb0f022283041cab` for the same item. Corrected in
   `3961186`, which also added the occurrence index that stops two identical
   items sharing an id.
2. An intermediate note claimed todos-mcp creates groups implicitly, so
   `create_group` was unnecessary. It does not. `created_group` there means an
   absent `###` *leaf heading* was created; the account's `TODO.md` must already
   exist (`invalid_account`), and a missing `## Pn` section is refused rather than
   invented (`missing_priority_section`). Directory-backed group creation is
   genuinely absent, hence `todos_create_group` in §6.

---

## 4. The wire protocol, captured from a live client

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

→ {"method":"tools/call","params":{"name":"…","arguments":{…},
     "_meta":{"claudecode/toolUseId":"toolu_…","progressToken":2}},"jsonrpc":"2.0","id":2}
← {"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"…"}]}}
```

Four consequences that would otherwise be guessed wrong:

- **The installed client uses the classic handshake at `2025-11-25`.** The
  published `2026-07-28` spec retires `initialize`/`initialized` in favour of
  per-request `_meta`. Building only to the current spec yields a server no
  installed client can talk to. Echoing the client's version, and never
  *requiring* `initialize` before serving `tools/list`, satisfies both.
- **Line-delimited JSON**, one object per line. No `Content-Length` framing —
  that is the HTTP transport.
- **`notifications/initialized` has no `id`** and must receive no response.
- **`tools/call` carries `_meta`** with a progress token; ignore it.

### stdout is protocol-only

Every diagnostic goes to stderr. A stray `println!` corrupts the stream and
surfaces as an unexplained client-side parse failure.

---

## 5. Architecture

```
  mitodo mcp-server                    a subcommand: no TUI, no terminal setup
  ┌──────────────────────────────────────────────────────────────────────┐
  │  stdin  ── line-delimited JSON-RPC 2.0 ──▶  dispatch                 │
  │                                               │                      │
  │   initialize              ──▶ echo protocolVersion, {"tools":{}}      │
  │   notifications/initialized ──▶ ignore                                │
  │   tools/list              ──▶ 13 schemas from one static table        │
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

| File | Responsibility |
|---|---|
| `src/mcp/mod.rs` | the stdio loop: read a line, dispatch, write a line |
| `src/mcp/protocol.rs` | JSON-RPC envelopes, error codes, response builders |
| `src/mcp/tools.rs` | the static tool table: name, description, `inputSchema` |
| `src/mcp/exec.rs` | argument parsing, the retired-id registry, calls into `store` |

`src/cli.rs` gains `Command::McpServer`; `src/main.rs` dispatches to
`mcp::serve(config)` before any terminal setup.

**No shared state with the TUI.** Each call re-reads the workspace, resolves the
item, writes through `verify`. A TUI open on the same workspace refreshes through
its existing `notify` watcher.

---

## 6. The tools

Names keep the `todos_` prefix, so prompts written against todos-mcp keep working
and the two servers stay recognisably one surface. Thirteen in total — four reads,
eight writes and `todos_sync` — against todos-mcp's ten.

```
 reads                                                       maps to
 ──────────────────────────────────────────────────────────────────────────────
 todos_list{query?, group?, include_done=false}
     → [{id, group, section, priority, text, done, has_notes, due,
         line, occurrence, parent}]
                                    query::Query::parse + Workspace::load
 todos_get_item{id}
     → {…as above…, notes, children:[{id, text, done}]}
 todos_get_file{group}      → {path, text}     the group's TODO.md verbatim
 todos_list_groups{}        → [{name, todo_file, open, total, has_notes,
                                archive_dir?}]

 writes                                                      maps to
 ──────────────────────────────────────────────────────────────────────────────
 todos_create_item{group, text, priority?, section?, notes?, children?}
     → {item, created_heading}                  store::create_item
 todos_add_child{parent_id, text}     → {item}  store::add_item
 todos_update_item{id, new_text?, done?}
     → {item}   ← the item's NEW id when text changed
                                                store::edit_text, store::toggle
 todos_set_notes{id, notes}           → {item}  store::set_description
 todos_delete_item{id}                → {deleted:true}        store::delete_item
 todos_archive_item{id}               → {item, archived:true} store::archive_items
 todos_archive_finished{group}        → {archived, skipped[]}  store::archive_done
 todos_create_group{name}             → {name, todo_file}      mkdir + seeded file
 todos_sync{}                         → {ok, transcript}       crate::git
```

### Details that are otherwise a coin toss

- **`todos_update_item` merges toggle and edit**, as todos-mcp does: `new_text`
  alone edits, `done` alone toggles, both do both in one write, and the returned
  item carries the new id when the text changed. Neither argument given is a
  `validation_error`.
- **`todos_create_item` never invents a `## Pn` section.** With `section` given
  and no `## ` heading in the file starting with it, the tool fails
  `missing_priority_section` and writes nothing. todos-mcp's rule, kept for a
  sharper reason than file formatting: `store::create_item` would otherwise append
  at end of file, and because priority is *derived from the heading above an
  item*, the agent would be told it filed a P0 that is not one. The check reads
  the file's `## ` headings before writing; matching is case-insensitive on the
  prefix, so `"P0"` matches `## P0 — Critical`.
- **`section` rather than `heading_path` here.** mitodo's parser carries a
  two-level `section` + `heading`, and `store::create_item` already places by
  section. Accepting a full path would mean either flattening it silently or
  reworking the parser — out of scope for this sub-project, and recorded in §10.
- **`todos_add_child` indents two beyond its parent** and anchors to the parent's
  line: `store::add_item(path, parent.line, parent.raw, parent.indent + 2, text)`,
  which is what the TUI's `A` key does.
- **`todos_list{query}` parses with `query::Query::parse`.** A malformed query
  returns the envelope carrying the parser's message, so the agent can fix it.
- **`todos_get_item.children` are direct children only.** The agent walks deeper
  by id.
- **Both archive tools need the group's `archive_dir`.** Absent, they fail with
  `no archive_dir configured for <group>`.
- **`todos_sync` requires `[git] enabled = true`** and runs only the argv list
  already in `[git] sync`.
- **`todos_create_group{name}`** is the one tool that creates a directory. It
  writes `<root>/<name>/TODO.md` seeded with the `## ` headings copied from the
  first existing group, so a new group matches the convention in use; with no
  group to copy from it seeds `## P0` … `## P3` when
  `[priority] source = "heading"`, and an empty file otherwise. Seeding matters:
  without a section the next create would hit the empty-file dead end fixed in
  `c5e27be`.

---

## 7. Errors

Two channels. Conflating them is a common MCP bug.

| Failure | Channel | Example |
|---|---|---|
| malformed JSON on stdin | `error` **−32700** | a truncated line |
| a request with an `id` naming an unknown method | **−32601** | `tools/foo` |
| `tools/call` missing `name`, or a bad argument type | **−32602** | no `name` field |
| **the tool ran and failed** | `result`, `isError: true`, text is the envelope | `{"error":"conflict","message":"…"}` |
| a notification we do not handle | nothing | `notifications/cancelled` |

Tool failures are *results* carrying todos-mcp's envelope
`{"error": code, "message": …}` as their text. The agent branches on `code`:

| code | meaning |
|---|---|
| `not_found` | the id was retired by a write this server made — you already changed it |
| `conflict` | the item drifted out-of-band; re-read and retry |
| `ambiguous_match` | the id resolved to more than one line |
| `invalid_group` | no such group, or not a valid group name |
| `invalid_priority` | not one of `P0`–`P3` |
| `missing_priority_section` | the file carries no section with that priority token |
| `validation_error` | empty text, or another argument the tool refuses |
| `git_disabled` | `[git] enabled = false` |

The `not_found` / `conflict` split needs the **retired-id registry**: an
in-process set of ids this server invalidated by its own writes. Without it both
cases look identical and the agent cannot tell its own edit from someone else's.
The registry lives for the process's lifetime and is not persisted.

Shutdown follows the stdio rule: stdin closes → EOF → exit 0.

---

## 8. The containment boundary

**No tool accepts a path.** Tools take a *group name*; the server joins it onto
the workspace root. A name is rejected when it contains `/` or `\`, equals `..`,
or begins with `.`. Writes never target `_archive/` — todos-mcp's
`_reject_if_archive` rule, kept.

There is no shell in the server. `todos_sync` runs only the argv list already in
the user's config. Requests are handled one at a time, sequentially.

---

## 9. Testing

No model and no network in the suite.

```
 protocol      the recorded transcript replays clean:
                 initialize (2025-11-25) → response echoes that version
                 notifications/initialized → stdout stays empty
                 tools/list → 13 tools, each inputSchema valid JSON Schema
                 tools/call → result with content[0].type == "text"
               malformed line → −32700;  unknown method with id → −32601
               unknown tool name → isError result, not a protocol error
               tools/list before any initialize → still works

 tools         table-driven, a temp workspace each:
                 every write tool's file effect asserted byte-for-byte
                 every read tool's JSON shape asserted
                 update_item with new_text returns an id that resolves after
                 update_item with done only keeps the id
                 update_item with neither → validation_error
                 create_item with a priority absent from the file →
                   missing_priority_section, file untouched
                 add_child lands two spaces deeper than its parent
                 a malformed query → envelope carrying the parser's message
                 archive with no archive_dir → invalid_group, file untouched
                 sync with [git] enabled=false → git_disabled
                 create_group copies headings from an existing group

 identity      an id retired by our own write → not_found
               an item changed on disk behind us → conflict
               the two are distinguishable — the registry's whole point

 containment   group names "../evil", "/etc/passwd", ".git", "a/b" rejected,
               no file created anywhere; a write aimed at _archive/ refused

 formatting    the project's core guarantee, re-pinned at this layer:
                 create_item into a real fixture leaves every other line
                 byte-identical, including line endings and final newline
```

**Not in CI:** a live run driving real `claude --mcp-config` against the built
binary. It needs network and a model, so it is a documented manual smoke step.

---

## 10. Out of scope

- The chat popup inside the TUI — sub-project 2
- Retiring the seven verbs and deleting the review layer — sub-project 3
- Ollama tool-calling over its HTTP API — sub-project 4
- An FTS5 index, and `todos_search` / `todos_status` / `todos_reindex`
- Arbitrary-depth `heading_path` placement, which would need mitodo's parser to
  carry a path rather than `section` + `heading`
- A tool that changes what the TUI displays (`set_query`) or drives its cursor
- MCP resources, prompts, sampling, elicitation, progress notifications
- Serving MCP over HTTP or SSE, and concurrent request handling
- Note-context records parsed out of `notes.md`

---

## 11. Where this sits

| # | Sub-project | Status |
|---|---|---|
| **1** | `mitodo mcp-server`, ported from todos-mcp — this spec | designing |
| 2 | Chat popup driving claude/codex over MCP | after 1 |
| 3 | Retire the seven verbs, delete the review layer (~700 lines) | after 2 |
| 4 | Ollama tool-calling over its HTTP API | after 2 |

---

## 12. Files touched

| File | Change |
|---|---|
| `src/mcp/mod.rs` | New: the stdio serve loop |
| `src/mcp/protocol.rs` | New: JSON-RPC envelopes, error codes, builders |
| `src/mcp/tools.rs` | New: the 13-tool static table with `inputSchema`s |
| `src/mcp/exec.rs` | New: argument parsing, retired-id registry, calls into `store` |
| `src/cli.rs` | `Command::McpServer` |
| `src/main.rs` | dispatch before terminal setup |
| `src/store/mod.rs` | re-export anything the tools need that is currently private |
| `resources/mcp-client-handshake.log` | the captured transcript, as a test fixture |
| `README.md` | how to point Claude Code or Codex at it |

No new dependencies: `serde`, `serde_json` and `sha2` are already present; the
loop uses `std::io` only.
