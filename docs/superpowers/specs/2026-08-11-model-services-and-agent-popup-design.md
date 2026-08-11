# Model services and the agent management popup — Design

**Date:** 2026-08-11
**Status:** Approved for planning
**Builds on:** [2026-07-28-mitodo-design.md](2026-07-28-mitodo-design.md) §7 (agent subsystem)

---

## 1. What this is

Two changes that ride the same pipeline.

**A configurable list of model services.** Today `[agent].command` names one CLI.
It becomes `[[services]]` — a list of named CLIs, one active at a time, chosen
from a picker in the UI and remembered between runs. Claude, Codex and Ollama
are the three that motivate it; the mechanism admits any CLI that takes a prompt
and emits JSON.

**A popup that manages items through the active service.** You type an
instruction in plain language; the agent returns a change-set of
add / complete / update / archive; the existing review pane opens; nothing
touches a file until you approve it per change.

**Why now.** `src/agent/mod.rs` was written provider-agnostic on purpose — its
own doc comment says "the command is any binary that takes a prompt and emits
JSON, so no provider is baked in." That property means multi-provider is a
config-and-selection problem, not a rewrite. The write path already exists too:
`scan` proposes a change-set and the review pane applies it. `manage` is the
same machinery with the user supplying the instruction instead of a fixed
prompt.

**Reference.** `~/repos/prview` runs `claude` as a cancellable subprocess and
selects providers/models by environment; two of its details are adopted here
(env hygiene, a retained kill handle) and one is not (env-var provider
selection — mitodo's config is a better home).

---

## 2. Decisions

Recorded because each closed off a real alternative.

| Decision | Chosen | Alternative rejected |
|---|---|---|
| Transport | All three services as CLI subprocesses | HTTP APIs — would put API keys, retries and rate limits inside a todo tracker |
| Popup shape | One free-form box → reviewable change-set | Structured add/remove/manage sub-modes; a multi-turn chat pane |
| Service selection | In-app picker, persisted to `[ui]` | Config-only (needs a restart to switch); liveness probes (subprocess stall on menu open) |
| Verb routing | One active service for all seven verbs | Per-verb overrides; ask-at-invoke-time |
| "Remove" semantics | Archive — move to `<archive_dir>/TODO.md` | Hard delete (git-only recovery); both actions (hands the destructive choice to the model) |
| Config shape | Per-service argv + a `schema_mode` enum | Placeholder templating (a missing placeholder fails silently); a hardcoded provider enum (breaks provider-agnosticism) |

### Why `schema_mode` exists

The three CLIs take a JSON schema three different ways. This is the single
finding that shapes the design, and today's `run()` implements only the first:

| CLI | flag | takes |
|---|---|---|
| `claude` | `--json-schema <schema>` | the schema inline, as a string |
| `codex` | `--output-schema <FILE>` | a path to a file holding the schema |
| `ollama` | `--format json` | the literal word `json` — the schema must be stated in the prompt |

Verified against `--help` on both machines at authoring time.

---

## 3. Architecture

```
  config/mod.rs                agent/                        ui/
  ┌────────────────────┐      ┌──────────────────────┐     ┌───────────────────────┐
  │ ServiceConfig      │      │ run(service, schema, │     │ AskingAgent(Manage)   │
  │  name              │─────▶│     prompt, cwd)     │◀────│  reuses the ask box   │
  │  command[]         │      │  3 schema modes      │     ├───────────────────────┤
  │  schema_mode       │      ├──────────────────────┤     │ ServiceMenu  (new)    │
  │  schema_flag       │      │ Verb::Manage  (new)  │     │  j/k/enter/esc        │
  │  timeout_secs      │      ├──────────────────────┤     └───────────┬───────────┘
  ├────────────────────┤      │ changeset.rs         │                 │ writes
  │ services: Vec<..>  │      │  + Archive action    │                 ▼
  │ legacy [agent] ────┼─────▶│                      │     ┌───────────────────────┐
  │   → "default"      │      └──────────┬───────────┘     │ ui.service (persisted)│
  └────────────────────┘                 │ ChangeSet       └───────────┬───────────┘
                                         ▼                             │ selects
                            ┌──────────────────────────┐               │
                            │ ReviewingChangeSet       │◀──────────────┘
                            │  space toggles · y apply │
                            └──────────┬───────────────┘
                                       │ apply
                                       ▼
                            store/archive.rs
                            ┌──────────────────────────┐
                            │ archive_items(...) (new) │◀── archive_done delegates
                            └──────────────────────────┘
```

**The load-bearing invariant: `agent/` never writes files.** It returns a
`ChangeSet`; only `changeset::apply` touches disk, and only after the review
pane. That is what makes a destructive action safe to add, and it is already how
`scan` works — `manage` inherits it.

Units, one sentence each:

- **`ServiceConfig`** — one named CLI and how it takes a schema. Pure data.
- **`agent::run`** — takes `&ServiceConfig` in place of the three loose
  `command` / `schema_flag` / `timeout` arguments, and branches on
  `schema_mode`. Still blocking, still called on a thread.
- **`Verb::Manage`** — free-form instruction → the change-set schema. Reuses
  `render_prompt` with `{input}` and `{files}`.
- **`Mode::ServiceMenu`** — a list picker modelled on the existing `ViewMenu`.
- **`store::archive::archive_items`** — moves named items' subtrees verbatim
  into the archive file; `archive_done` becomes a caller.

---

## 4. Config and back-compat

```toml
[[services]]
name         = "claude"
command      = ["claude", "--print", "--dangerously-skip-permissions"]
schema_mode  = "flag"
schema_flag  = "--json-schema"
timeout_secs = 600

[[services]]
name         = "codex"
command      = ["codex", "exec", "--json"]
schema_mode  = "file"
schema_flag  = "--output-schema"
timeout_secs = 600

[[services]]
name         = "ollama"
command      = ["ollama", "run", "qwen2.5:3b", "--format", "json"]
schema_mode  = "prompt"
timeout_secs = 300

[agent.prompts]
scan = "~/.config/mitodo/prompts/scan.md"

[ui]
service = "claude"
```

Resolution, in order:

1. `[[services]]` present → those are the services; `ui.service` names the
   active one.
2. `[[services]]` absent and `[agent].command` present → synthesise one service
   named `default` from `[agent]`'s `command` / `schema_flag` / `timeout_secs`,
   with `schema_mode = "flag"`. Existing configs keep working unedited, with no
   behaviour change.
3. `ui.service` names a service not in the list → fall back to the first and
   put a notice on the status line. Never a hard error; the workspace still
   opens. This is the case a config shared between machines hits.
4. Neither present → `AgentError::NotConfigured`, as today. Agent keys report
   it; everything else works.

`[agent.prompts]` stays where it is. A prompt is per *verb*, not per *service* —
the `scan` instruction is the same whichever CLI executes it.

`schema_mode = "prompt"` appends `\n\nReply with JSON matching this
schema:\n<schema>` to the rendered prompt. Ollama's `--format json` guarantees
valid JSON, not the requested shape, so the shape has to be stated in words;
`extract_json` already tolerates the surrounding prose.

`schema_mode = "file"` writes the schema to a `tempfile::NamedTempFile` for the
duration of the call. This promotes `tempfile` from a dev-dependency to a
dependency — the only dependency change in the feature.

---

## 5. UI

### Service picker

A clickable tab in the top bar showing the active service, opened by `m` or a
click, closed by `esc`. Same pattern as the existing `view ▾` tab, including
mouse hit-testing.

```
  mitodo   all · 41 open / 96 shown  /pri:P0 !done      claude ▾   view ▾
┌groups──────────────┐┌items 3/41──────────────┌──────────────────────────┐
│  all         41    ││  [x] P0  File the 83(b)│▸ claude                  │
│▸ work        12    ││    [ ] P0  pull the sig│  codex                   │
│  home         9    ││▸ [ ] P0  Respond to opp│  ollama · qwen2.5:3b     │
│  side         3    │└────────────────────────└──────────────────────────┘
```

`j` / `k` move, `enter` selects, `esc` closes. Selecting writes `ui.service`
and shows `service: codex` on the status line. Persisted on exit alongside
`hide_done` and `ticker`. The tab label is also the standing answer to "which
model just replied to me?"

Each row is the service's configured `name`, verbatim and unparsed — the
`ollama · qwen2.5:3b` row above is a `name` the user typed, not a model
discovered at runtime. Nothing in the picker inspects PATH or queries a service.

### Manage popup

`M` opens the command line every other verb uses, prefix `manage: `:

```
 manage: archive the everlongtech items that closed, add a P1 to chase Sam█
```

`enter` sends, `esc` aborts. The reply lands in the existing review pane, which
gains a fourth glyph:

```
┌review — 4 changes · space toggles · a all · y apply · esc discard────────┐
│▸ [x] + add      lysk/TODO.md · Chase Sam for the exhibit list           │
│  [x] ✓ done     everlongtech/TODO.md · File the 83(b) election          │
│  [x] → archive  everlongtech/TODO.md · Phase A — Demand for Engagement  │
│  [ ] ~ edit     lysk/TODO.md · Draft the LLC agreement                  │
├─────────────────────────────────────────────────────────────────────────┤
│ reason: closed per Rishab's Mar 16 waiver; nothing left to do           │
└─────────────────────────────────────────────────────────────────────────┘
```

Mechanics unchanged: `space` toggles one, `a` toggles all, `y` applies the
selected subset, `esc` discards. **Archive rows arrive unticked; add, complete
and update arrive ticked.** You opt into a move rather than opting out of one.

### Keys

`m` and `M` are both free against the 34 keys already bound. `M` sits beside
`R` (scan) deliberately: `R` is "you decide what needs changing", `M` is "I'll
tell you what to change" — same schema, same review pane, different prompt.

`esc` during a running call cancels it (§7).

---

## 6. The archive action and the store refactor

```
 ChangeAction  ::=  Add | Complete | Update | Archive     # "archive" in the JSON enum
 glyph(Archive) = "→ archive"

 apply(root, archive_dir: Option<&Path>, today: &str, items, set)   # two new params
   Add      → write::add_item                 (unchanged)
   Complete → write::toggle                   (unchanged)
   Update   → write::edit_text                (unchanged)
   Archive  → archive::archive_items(file, archive_dir, [target], today)
```

`archive_dir` is `None` when the config omits it. `today` is passed in by the
caller rather than read inside `apply`, matching `archive_done`'s existing
signature — it keeps `apply` a pure function of its arguments and lets a test
pin a fixed date.

The refactor, with no behaviour change:

```
  BEFORE                                AFTER
  archive_done(file, dir, items, date)  archive_done(file, dir, items, date)
    ├─ read_lines                         └─ picks targets: done ∧ top-level
    ├─ pick done ∧ top-level                   │
    ├─ verify each vs disk                     ▼
    ├─ wholly_done guard                  archive_items(file, dir, targets, date)
    ├─ collect subtree blocks               ├─ read_lines
    ├─ append to archive under date         ├─ verify each vs disk
    └─ remove moved ranges                  ├─ collect subtree blocks
                                            ├─ append to archive under date
                                            └─ remove moved ranges
```

`X` behaves identically because it still supplies the same target list; the
agent path supplies a one-item list. Batching, the disk-changed `verify` and the
verbatim-append property stay in one place.

Three rules inside this section:

**Archiving is not gated on `done`.** `X` archives only ticked items because its
job is clearing out what is finished. The agent's job is "this should not be in
my working file any more" — typically an item reality closed while the file
still shows it open. Requiring `done` first would cost two round-trips.

**`wholly_done` becomes informational, not a veto.** `X` refuses to archive an
item whose subtree still has open work, correctly — it operates in bulk with no
per-item consent. The review pane *is* per-item consent, so the row states the
cost and the user decides:

```
│  [ ] → archive  lysk/TODO.md · Draft the LLC agreement  (2 open sub-items)  │
```

Unticked by default, like every archive row. This is the one place the agent
path deliberately differs from `X`.

**No `archive_dir` configured → archive changes are skipped**, listed in the
existing `ApplyReport.skipped` as `archive "…": no archive_dir configured`.
Every other change in the set still applies.

---

## 7. Error handling

| Condition | Behaviour | Surfaced as |
|---|---|---|
| `ui.service` names a missing service | fall back to the first, keep working | `service "codex" not in config — using claude` |
| service binary not on PATH | `AgentError::Spawn`, prefixed with the service name | `codex failed: could not run codex: …` |
| no services and no `[agent]` | `NotConfigured`, as today | `no agent configured (set [[services]])` |
| `esc` during a call | new `AgentError::Cancelled`, child killed | spinner clears, `manage cancelled` |
| timeout | existing `TimedOut`, per-service value | `agent did not finish within 300s` |
| valid JSON, wrong shape | existing `extract_json` + `field` fallbacks | readable text, never raw JSON |
| change-set unparseable | error modal with the first line of the reply | `manage failed: …` |

Two robustness items adopted from prview, both real gaps today:

**Strip `CLAUDECODE` from the child environment.** prview does this deliberately
in `_claude_env`; mitodo passes the parent environment through. When mitodo is
launched from inside a Claude Code session that variable is set, and the nested
`claude --print` sees itself running inside itself. The same hazard applies to
`codex`.

**Cancellation holds the child's kill handle.** `spawn_agent` currently detaches
its thread, so a 600-second call cannot be stopped. The handle lives in `App`
behind a mutex; `esc` during `Busy` kills the child. This matters most for a
slow local Ollama model.

**The schema temp file is RAII.** `NamedTempFile` drops on every exit path —
success, non-zero exit, timeout-kill, cancel, panic. A manual `remove_file`
after `wait_with_output` would leak the file on exactly the paths that matter.

---

## 8. Testing

Fake services throughout — `echo` and `sh -c`, the pattern the existing agent
tests already use. No live model calls, so the suite stays deterministic and
offline.

```
 config       legacy [agent] with no [[services]] → one service named "default"
              ui.service naming an absent service → first service + notice
              three [[services]] round-trip through save/load unchanged

 run()        schema_mode=flag    → argv contains "--json-schema <schema>"
              schema_mode=file    → argv contains "--output-schema <path>",
                                    the path exists during the call,
                                    and is gone after it returns
              schema_mode=prompt  → no flag; the prompt ends with the schema text
              CLAUDECODE is absent from the child environment
              esc-cancel kills a `sleep 60` child in under a second

 changeset    "archive" parses into ChangeAction::Archive
              archive rows default to unticked, add/complete/update to ticked
              apply with no archive_dir skips archive and applies the rest
              apply archives one named item and leaves its siblings alone
              an archived item with open sub-items still moves, and the row says so

 archive      every existing archive_done test passes unmodified  ← the refactor's pin
              archive_items moves a subtree verbatim and removes the source range
```

The last line is the important one. The extraction is only safe if `X`'s
existing tests pass without being touched; editing them would mean the refactor
changed behaviour.

---

## 9. Out of scope

- HTTP transports and API-key handling
- Per-verb service routing, and choosing a service at invoke time
- Hard delete as an agent action — archive is the only removal
- Streaming or token-by-token output
- Multi-turn conversation with the agent
- Automatic model discovery (`ollama list`) and PATH liveness probes in the
  picker

---

## 10. Files touched

| File | Change |
|---|---|
| `src/config/mod.rs` | `ServiceConfig`, `SchemaMode`, `services`, `ui.service`, legacy synthesis |
| `src/agent/mod.rs` | `run` takes `&ServiceConfig`; three schema modes; `Verb::Manage`; `Cancelled`; env strip |
| `src/agent/changeset.rs` | `ChangeAction::Archive`, schema enum, glyph, `apply` archive branch and `archive_dir` parameter |
| `src/store/archive.rs` | extract `archive_items`; `archive_done` delegates |
| `src/ui/mod.rs` | `Mode::ServiceMenu`, `m` / `M` keys, kill handle, per-service spawn, review defaults |
| `src/ui/view.rs` | service tab, picker dropdown, archive glyph and open-sub-item label |
| `Cargo.toml` | `tempfile` dev-dependency → dependency |
| `README.md` | `[[services]]` config, `m` / `M` keys, archive action |

Roughly 250 lines of implementation and 200 of tests.
