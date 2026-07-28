# mitodo Phase 2 — TUI (decomposition)

> **For agentic workers:** each stage below is a separate plan-and-execute cycle. Write the stage's detailed plan (superpowers:writing-plans) before implementing it. Do not attempt the whole phase in one pass.

**Status:** all five stages complete (2026-07-28). 134 tests passing.

| stage | delivered |
|---|---|
| A — a TUI that runs | three-pane frame over the real workspace, quits on `q` |
| B — browse | group/item cursors, vim nav, focus, hide-done, scrolling, detail pane |
| C — query | `acct:` `pri:` `done` `sec:` `has:desc` `text:` with AND/OR/NOT and parens |
| D — mutate | toggle, add sibling/child, edit, description, delete-with-confirm |
| E — live reload | debounced notify watcher, own writes silent, external edits announced |

Still in `src/_port/`: `messages/command` and the keybinding engine (only needed
for a configurable `:` command vocabulary), the three original list panes, and
`chyron` (phase 3). `config/login_configuration.rs` and
`feed_list_content_identfier.rs` are dead and can be deleted.

## Why this is decomposed

Phase 2 is ~14,000 LOC of eilmeldung code in `src/_port/`. A single plan would
produce a long period with no working build, which is the worst possible state
for a port. Each stage below ends with something that runs.

## Measured coupling

Counted with `grep -rc news_flash` per module. This is what makes the port
tractable and dictates the ordering:

| module | LOC | `news_flash` refs | disposition |
|---|---|---|---|
| `input/` | 538 | 0 | move as-is |
| `utils/` | 88 | 0 | move as-is |
| `ui/help_popup/` | 322 | 0 | move as-is |
| `ui/command_confirm/` | 94 | 0 | move as-is |
| `ui/tooltip/` | 61 | 0 | move as-is |
| `ui/view.rs` | 182 | 1 | move, retarget layout |
| `config/` | 2167 | 3 | merge with phase 1 `config/`, drop `login_configuration.rs` |
| `messages/` | 1591 | 3 | move, swap command vocabulary |
| `query/` | 1326 | 3 | move, swap field vocabulary |
| `ui/command_input/` | 861 | 5 | move, swap completion sources |
| `ui/chyron/` | 677 | 14 | phase 3 |
| `ui/mod.rs` | 916 | 37 | **rewrite** — the App struct is RSS-shaped |
| `ui/feeds_list/` | 2315 | — | **rewrite** as `groups_list` |
| `ui/articles_list/` | 1521 | — | **rewrite** as `items_list` |
| `ui/article_content/` | 1167 | — | **rewrite** as `item_detail` |

The correction that matters: the three list panes and `ui/mod.rs` are a
**rewrite against `crate::store`**, not a transliteration. Roughly 8k LOC ports
nearly as-is; roughly 6k is new code informed by the originals.

## Stages

### Stage A — a TUI that runs

Move the zero- and low-coupling modules into the compile path together
(`utils`, `input`, `config` merge, `messages`, `view.rs`, the three modals),
then write a minimal `App` against `crate::store::Workspace`.

These must land as one stage: moved piecemeal they do not compile, because they
reference each other through `prelude`.

**Deliverable:** `mitodo` opens a terminal, renders the three-pane frame with
real group names and item counts from `~/repos/TODO`, and quits on `q`.

### Stage B — browse

`groups_list` and `items_list` panes: tree of groups and sections, flattened
item list with checkboxes and indentation, vim navigation (`j/k/g/G`), focus
switching, scrolling.

**Deliverable:** navigate the whole real workspace read-only.

### Stage C — query

Swap the query field vocabulary (`feed:` → `acct:`, `unread` → `!done`,
`date:` → `due:`) and retarget the filter predicate at `Item`. Wire `/` and `:`.

**Deliverable:** `pri:P0 acct:lefv !done` filters the list — this is what
retires `mcli todos act`.

### Stage D — mutate

`item_detail` pane plus the write keys: `space`/`x` toggle, `a`/`A` add,
`e` edit, `d` delete with confirmation, description editing. All routed through
the phase 1 conflict-aware writer, with `Conflict` surfaced in the status bar.

**Deliverable:** mitodo replaces `mcli run -g todos list` for daily use.

### Stage E — live reload

`store/watch.rs` — `notify` with a 200ms debounce and a file-hash gate so
mitodo's own writes do not loop.

**Deliverable:** edits by Claude, `mcli todos` or `todos-mcp` appear without a
restart.

## Ordering constraint

A → B → C → D → E. Stage A is the only one that cannot be split further; the
rest can each be planned independently once A lands.

## Not in phase 2

Chyron vocabulary swap, the agent subsystem, git sync, README and `NOTICE`,
packaging. All phase 3.
