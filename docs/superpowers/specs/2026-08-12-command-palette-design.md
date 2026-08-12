# Command palette — Design

**Date:** 2026-08-12
**Status:** Approved for planning
**Builds on:** [2026-07-28-mitodo-design.md](2026-07-28-mitodo-design.md), [2026-08-11-model-services-and-agent-popup-design.md](2026-08-11-model-services-and-agent-popup-design.md)

---

## 1. What this is

A fuzzy command palette: `:` or `ctrl-k` opens a filter line over a ranked list of
every action in mitodo. Type to narrow, `enter` to run, `esc` to close.

It is a **discovery layer, not new behaviour**. Every entry resolves to a
`KeyEvent` and dispatches it through the existing `handle_normal_key`, so the
palette is a synonym for the keypress by construction rather than by test.

**Why now.** mitodo binds 43 keys in normal mode, eight of which appear nowhere
in the help screen, and the recent model-services work added three more (`m`,
`M`, and the service picker). `help_lines()` is hand-written prose that had to be
edited by hand alongside the keymap and has already drifted. A palette needs a
structured action table, and that same table can generate the help text — so the
feature closes the drift rather than adding a third place to forget.

---

## 2. Decisions

| Decision | Chosen | Alternative rejected |
|---|---|---|
| Open key | `:` **and** `ctrl-k` | `:` alone; `ctrl-k` alone; `ctrl-p` (some terminals bind it to history) |
| Entries | Every bound key, motions included, plus one per configured service | Actions-only (would omit motions, but then the table cannot generate complete help); actions plus dynamic jump targets (a different, larger feature) |
| Matching | Fuzzy subsequence, ranked | Case-insensitive substring; word-prefix matching |
| Dispatch | Entry carries a `KeyEvent`; running it calls `handle_normal_key` | Extract an `Action` enum from all 43 arms (large diff through a working 5,000-line file); a `fn(&mut App)` per entry (needs ~15 inline match-arm bodies extracted first, leaving two routes to each behaviour) |

The dispatch choice has one weakness — a typo'd `KeyEvent` yields a silently
dead entry, which the compiler cannot catch. §7 closes that with a test that
walks the table.

---

## 3. Architecture

`src/ui/mod.rs` is 5,093 lines, so the palette goes in a new module. The split
follows a real boundary: everything except dispatch is pure.

```
  src/ui/palette.rs   (new, ~180 lines + ~150 of tests)
  ┌────────────────────────────────────────────────────────┐
  │ Category   Navigation | Items | Query | Agent | Groups  │
  │            | View | Session                            │
  │                                                        │
  │ Action { label, keys, key: KeyEvent, category }         │
  │ ACTIONS: [Action; 41]      ← the single source of truth │
  │                                                        │
  │ score(needle, haystack) -> Option<u32>    pure          │
  │ filter(needle, services)  -> Vec<Entry>   pure          │
  │ help_lines()              -> Vec<String>  pure          │
  └────────────────────────────────────────────────────────┘
                    │                    │
        entries +   │                    │  generated help text
        ranking     ▼                    ▼
  src/ui/mod.rs                    the `?` modal
  ┌────────────────────────────────────────────────────────┐
  │ Mode::Palette                                          │
  │ palette_input: String   palette_cursor: usize          │
  │ palette_scroll: usize                                  │
  │ handle_palette_key()  →  on Enter:                     │
  │       mode = Normal; then dispatch the entry           │
  └────────────────────────────────────────────────────────┘
                    │
                    ▼
  src/ui/view.rs — render_palette(), centred overlay
```

Units, one sentence each:

- **`ACTIONS`** — a static table of every normal-mode binding: label, display
  string, the `KeyEvent` it stands for, category. Data only.
- **`score`** — fuzzy subsequence scorer, `(needle, haystack) -> Option<u32>`,
  `None` for no match. No `App`, no ratatui.
- **`filter`** — needle plus the configured service names in, ranked entries out.
  Services are passed in rather than read from config, keeping it pure.
- **`handle_palette_key`** — the only impure part: edits `palette_input`, moves
  `palette_cursor`, and on `enter` closes the palette and dispatches.

**The load-bearing property: the palette never implements an action.** It
resolves a label to a `KeyEvent` and presses it.

`help_lines()` moves into this module and is generated from `ACTIONS`, grouped by
category. Adding a row updates the palette and the help screen together.

---

## 4. The action table

One entry per binding. Two foldings: `space` and `x` are one action with an
alias, and `ctrl-c` folds into `q`. That gives 41 static entries plus one per
configured service.

```rust
Action { label: &'static str, keys: &'static str, key: KeyEvent, category: Category }
```

`keys` is what the palette displays (`"space / x"`, `"shift-tab"`); `key` is the
single event dispatch presses.

| Category | Label | Key |
|---|---|---|
| Navigation | move down · move up | `↓` `↑` |
| Navigation | open a node and step into it · close a node and step out | `→` `←` |
| Navigation | jump to first item · jump to last item | `g` `G` |
| Navigation | focus groups pane · focus items pane · focus detail pane · focus back | `h` `j` `k` `l` |
| Navigation | focus the item list · focus the groups list | `tab` `shift-tab` |
| Navigation | fold this node · fold or unfold everything | `z` `Z` |
| Items | toggle done | `space / x` |
| Items | new item (dialog) · quick add sibling · add child item | `a` `o` `A` |
| Items | edit item text · edit notes in the detail pane | `e` `i` |
| Items | delete item (asks first) | `d` |
| Query | edit the query · clear the query, or cancel a running agent | `/` `esc` |
| Query | hide or show done items | `H` |
| Agent | describe a filter in words, get a query | `n` |
| Agent | summarise what's on screen · explain this item | `S` `E` |
| Agent | break this item into sub-items | `b` |
| Agent | act on this item with an agent | `!` |
| Agent | scan the workspace for changes | `R` |
| Agent | manage items with the agent | `M` |
| Agent | pick the model service | `m` |
| Groups | read this group's notes · archive finished items | `N` `X` |
| View | view settings menu | `v` |
| View | scrolling ticker on/off · pause ticker · ticker faster · ticker slower | `c` `p` `+` `-` |
| Session | keyboard help · quit | `?` `q` |

Appended at runtime from `config.services()`:

```
  use model service: claude
  use model service: codex
  use model service: ollama
```

Those are the one case where an entry is not a bare keypress — they call
`select_service(index)`. Hence:

```rust
enum Entry {
    Key(&'static Action),
    Service { name: String, index: usize },
}
```

Two label decisions:

- **`esc`** is two actions depending on state: clears the query, or cancels a
  running agent. One entry, labelled for both, since the key does the right thing
  either way.
- **`d`** is a guarded arm (`if self.selected_item().is_some()`). From the
  palette with nothing selected it does nothing silently, exactly as the key
  does. Adding a notice would make the palette *not* a synonym for the key.

---

## 5. On screen

A centred overlay, sized and hit-tested like the review pane and the new-item
dialog.

```
┌palette — 4/44 ───────────────────────────────────────────────┐
│ > arc█                                                       │
│                                                              │
│▸ archive finished items                             Groups  X│
│  act on this item with an agent                      Agent  !│
│  scan the workspace for changes                      Agent  R│
│  use model service: claude                           Model   │
│                                                              │
│ ↑↓ move · enter run · esc close                              │
└──────────────────────────────────────────────────────────────┘
```

Label left; category and key right-aligned, so the key column teaches the
binding and using the palette makes you stop needing it.

| Key | Does |
|---|---|
| any character | appends to the filter, re-ranks, cursor to the top match |
| `backspace` | deletes one character |
| `↑` `↓` | move the cursor (`j`/`k` cannot — they are typable) |
| `ctrl-p` `ctrl-n` | same as `↑`/`↓` |
| `enter` | close, then run the highlighted entry |
| `esc` | close, run nothing |
| click a row | run it |
| click outside | close |

Behaviour, each a deliberate call:

- **An empty filter lists all 44 in table order**, grouped as in §4, so a
  first-time `:` is a browsable index rather than an arbitrary ranking.
- **The cursor resets to 0 on every keystroke.** Typing narrows toward the
  target; a stale cursor would leave whatever slid under it selected.
- **No matches** renders `no matching command`, and `enter` does nothing.
- **The title count** (`4/44`) shows whether the filter is too narrow without
  scrolling.
- **Scrolling** reuses the existing `clamp_scroll` / `viewport_start` helpers.
- **Runs after closing, not before.** `enter` sets `mode = Normal` and *then*
  dispatches, so actions that open their own mode (`a`, `/`, `M`, `v`, `m`) land
  in that mode instead of being overwritten by the palette's own exit. `:` →
  `manage` → `enter` must leave the `manage:` prompt open; the ordering is what
  makes that work, and §7 tests it.

---

## 6. Matching and ranking

```
score(needle, haystack) -> Option<u32>        pure, ~35 lines

  both lowercased; an empty needle scores Some(0)
  greedy left-to-right subsequence walk; any unmatched needle character → None

  per matched character:
      +8   at a word start (index 0, or the previous character is space, - or :)
      +4   contiguous with the previous match
      +1   base
      -1   per character skipped before the first match
```

Haystack is label plus category, so `agent scan` finds the scan verb. Ties break
by table order through a stable sort, which keeps the unfiltered list in §4's
grouping.

| Type | Top match | Why |
|---|---|---|
| `arf` | archive finished items | `a` and `f` at word starts |
| `gs` | git sync | both at word starts |
| `mod` | pick the model service | contiguous inside "model" |
| `manage` | manage items with the agent | contiguous run at index 0 |
| `sum` | summarise what's on screen | contiguous at a word start |
| `ollama` | use model service: ollama | contiguous, dynamic entry |

**Known limitation.** Greedy leftmost matching is not optimal — fzf runs
dynamic programming to find the best alignment; this takes the first one. A
needle whose ideal alignment needs backtracking scores lower than it deserves.
Across 44 short labels this is invisible, and 35 obvious lines beat 120 lines of
DP for a list this size. If it ever feels wrong, the fix is local to `score`.

---

## 7. Testing

Everything except dispatch is pure, so most of this needs no `App`.

```
 score       a subsequence matches; a non-subsequence does not
             order matters: "ba" does not match "abc"
             case-insensitive in both directions
             word-start beats mid-word
             contiguous beats scattered
             an empty needle matches everything, with equal scores

 filter      an empty needle returns all entries in table order
             each worked example in §6 ranks as stated
             one service entry per configured service
             no matches returns empty

 table       ← the dead-entry pin
             every ACTIONS entry's key reaches a real arm of handle_normal_key:
             dispatch it and assert the catch-all arm was not taken

 dispatch    ":" opens the palette, and so does ctrl-k
             esc closes it and runs nothing
             typing filters, and each keystroke resets the cursor to 0
             enter on "manage items with the agent" leaves mode ==
                 AskingAgent(Manage), not Normal      ← the ordering test
             enter with no matches does nothing
             enter on "use model service: ollama" sets the active service

 help        help_lines() contains every ACTIONS label
             and every category heading
```

The table test earns its keep: it closes the one hole the dispatch choice opens.
It needs a way to see that a key was handled, and "did any state change?" will
not do — several actions are legitimately inert in a given state (`esc` with no
query, `d` with nothing selected, `p` with no ticker), so an inertness check
would need an allow-list large enough to hide the very typo it exists to catch.

Instead, `handle_normal_key`'s existing catch-all arm records that it was taken:

```rust
// src/ui/mod.rs
#[cfg(test)]
pub(crate) unhandled_key: bool,      // App field

            _ => {
                #[cfg(test)]
                {
                    self.unhandled_key = true;
                }
            }
```

The test clears the flag, dispatches the entry's key, and asserts the flag is
still false. That is precise (it fails only when a key reaches no arm), robust
against formatting and refactoring, costs one field and three lines, and compiles
out of release builds.

---

## 8. Out of scope

- Dynamic jump targets (go-to-group, saved filters) — a different feature
- A vim-style typed command line (`:q`, `:sync` as text commands)
- Recently-used ordering or frecency
- Rebindable keys, or a palette over a user-defined command set
- Optimal (DP) fuzzy alignment
- Palette access to actions that only exist inside another mode (review-pane
  keys, dialog fields)

---

## 9. Files touched

| File | Change |
|---|---|
| `src/ui/palette.rs` | New: `Category`, `Action`, `ACTIONS`, `Entry`, `score`, `filter`, `help_lines` |
| `src/ui/mod.rs` | `Mode::Palette`, palette state, `handle_palette_key`, `:` and `ctrl-k` bindings, mouse routing, a test-only `unhandled_key` flag on the catch-all arm, and `help_lines` moved out to `palette.rs` |
| `src/ui/view.rs` | `render_palette`, `palette_rect`, status-line hint |
| `README.md` | A keys-table row and a short palette section |

Roughly 310 lines of implementation and 150 of tests. No new dependencies.
