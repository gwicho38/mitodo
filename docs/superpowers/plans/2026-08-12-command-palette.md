# Command Palette Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `:` or `ctrl-k` opens a fuzzy-filtered list of every action in mitodo; type to narrow, `enter` runs the highlighted one, `esc` closes.

**Architecture:** A new `src/ui/palette.rs` holds a static `ACTIONS` table (label, display keys, the `KeyEvent` the entry stands for, category), a pure fuzzy `score`, a pure `filter` that ranks entries and appends one per configured model service, and a `help_lines` generated from the same table. `src/ui/mod.rs` gains `Mode::Palette`, three state fields and a key handler that — on `enter` — closes the palette and then dispatches the entry's `KeyEvent` through the existing `handle_normal_key`. The palette never implements an action.

**Tech Stack:** Rust 2024 edition, ratatui 0.30 (`Clear`, `Block`, `Paragraph`, `Line`, `Span`), `ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers}`. No new dependencies.

## Global Constraints

- Spec: [docs/superpowers/specs/2026-08-12-command-palette-design.md](../specs/2026-08-12-command-palette-design.md).
- Branch: `feat/command-palette`. Never commit to `main`.
- Comments: minimum possible, one line each, stating a hidden constraint only. Never restate what the code does. Never reference this plan, this branch, or the change that introduced the line.
- Test names are sentences describing behaviour, matching the existing suite. No `#[ignore]`, no skipped tests.
- `cargo clippy --all-targets` warning-free and `cargo test` green at every commit. `cargo fmt` before every commit.
- The palette must not duplicate any action's behaviour: every static entry dispatches a `KeyEvent` through `handle_normal_key`.
- `ACTIONS` is the single source of truth for both the palette and the `?` help screen. After Task 3 no hand-written key list may remain in `src/ui/mod.rs`.
- No new dependencies.
- Commit after every task with a Conventional Commits subject. No AI attribution or co-author trailers.
- Existing helpers to reuse rather than reinvent: `view::clamp_scroll(scroll, len, height)`, `viewport_start(cursor, len, height)`, `within(rect, x, y)`, `row_index(rect, y)`. Theme styles available: `theme.header()`, `theme.paragraph()`, `theme.inactive()`, `theme.statusbar()`, `theme.command_input()`, `theme.query()`, plus `theme.selected(&Style)` and `theme.eff_border(bool)`.

---

### Task 1: The action table and the fuzzy scorer

**Files:**
- Create: `src/ui/palette.rs`
- Modify: `src/ui/mod.rs:1-4` (module declarations)

**Interfaces:**
- Consumes: nothing.
- Produces: `Category` (with `label(&self) -> &'static str` and `ALL: [Category; 7]`); `Action { label: &'static str, keys: &'static str, key: KeyEvent, category: Category }`; `ACTIONS: [Action; 41]`; `score(needle: &str, haystack: &str) -> Option<u32>`.

- [ ] **Step 1: Create the module with the table and a stub scorer**

Create `src/ui/palette.rs`:

```rust
//! Every action in the app, as data.
//!
//! One table drives the command palette and the `?` help screen, so a new
//! binding cannot appear in one and be forgotten in the other. An entry carries
//! the key it stands for rather than a behaviour: running it presses that key.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Navigation,
    Items,
    Query,
    Agent,
    Groups,
    View,
    Session,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Navigation,
        Category::Items,
        Category::Query,
        Category::Agent,
        Category::Groups,
        Category::View,
        Category::Session,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Navigation => "Navigation",
            Category::Items => "Items",
            Category::Query => "Query",
            Category::Agent => "Agent",
            Category::Groups => "Groups",
            Category::View => "View",
            Category::Session => "Session",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Action {
    pub label: &'static str,
    /// What the palette and the help screen print for this binding.
    pub keys: &'static str,
    /// The event dispatch presses. Uppercase letters are matched with `_`
    /// modifiers by the key handler, so SHIFT here is cosmetic.
    pub key: KeyEvent,
    pub category: Category,
}

const fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;

pub const ACTIONS: [Action; 41] = [
    Action { label: "move down", keys: "↓", key: key(KeyCode::Down, NONE), category: Category::Navigation },
    Action { label: "move up", keys: "↑", key: key(KeyCode::Up, NONE), category: Category::Navigation },
    Action { label: "open a node and step into it", keys: "→", key: key(KeyCode::Right, NONE), category: Category::Navigation },
    Action { label: "close a node and step out", keys: "←", key: key(KeyCode::Left, NONE), category: Category::Navigation },
    Action { label: "jump to first item", keys: "g", key: key(KeyCode::Char('g'), NONE), category: Category::Navigation },
    Action { label: "jump to last item", keys: "G", key: key(KeyCode::Char('G'), SHIFT), category: Category::Navigation },
    Action { label: "move focus left, to the groups pane", keys: "h", key: key(KeyCode::Char('h'), NONE), category: Category::Navigation },
    Action { label: "move focus right, back to the item list", keys: "l", key: key(KeyCode::Char('l'), NONE), category: Category::Navigation },
    Action { label: "move focus down, to the detail pane", keys: "j", key: key(KeyCode::Char('j'), NONE), category: Category::Navigation },
    Action { label: "move focus up, to the item list", keys: "k", key: key(KeyCode::Char('k'), NONE), category: Category::Navigation },
    Action { label: "jump focus to the item list", keys: "tab", key: key(KeyCode::Tab, NONE), category: Category::Navigation },
    Action { label: "jump focus to the groups list", keys: "shift-tab", key: key(KeyCode::BackTab, SHIFT), category: Category::Navigation },
    Action { label: "fold this node", keys: "z", key: key(KeyCode::Char('z'), NONE), category: Category::Navigation },
    Action { label: "fold or unfold everything", keys: "Z", key: key(KeyCode::Char('Z'), SHIFT), category: Category::Navigation },

    Action { label: "toggle done", keys: "space / x", key: key(KeyCode::Char(' '), NONE), category: Category::Items },
    Action { label: "new item (dialog)", keys: "a", key: key(KeyCode::Char('a'), NONE), category: Category::Items },
    Action { label: "quick add sibling", keys: "o", key: key(KeyCode::Char('o'), NONE), category: Category::Items },
    Action { label: "add child item", keys: "A", key: key(KeyCode::Char('A'), SHIFT), category: Category::Items },
    Action { label: "edit item text", keys: "e", key: key(KeyCode::Char('e'), NONE), category: Category::Items },
    Action { label: "edit notes in the detail pane", keys: "i", key: key(KeyCode::Char('i'), NONE), category: Category::Items },
    Action { label: "delete item (asks first)", keys: "d", key: key(KeyCode::Char('d'), NONE), category: Category::Items },

    Action { label: "edit the query", keys: "/", key: key(KeyCode::Char('/'), NONE), category: Category::Query },
    Action { label: "clear the query, or cancel a running agent", keys: "esc", key: key(KeyCode::Esc, NONE), category: Category::Query },
    Action { label: "hide or show done items", keys: "H", key: key(KeyCode::Char('H'), SHIFT), category: Category::Query },

    Action { label: "describe a filter in words, get a query", keys: "n", key: key(KeyCode::Char('n'), NONE), category: Category::Agent },
    Action { label: "summarise what's on screen", keys: "S", key: key(KeyCode::Char('S'), SHIFT), category: Category::Agent },
    Action { label: "explain this item", keys: "E", key: key(KeyCode::Char('E'), SHIFT), category: Category::Agent },
    Action { label: "break this item into sub-items", keys: "b", key: key(KeyCode::Char('b'), NONE), category: Category::Agent },
    Action { label: "act on this item with an agent", keys: "!", key: key(KeyCode::Char('!'), SHIFT), category: Category::Agent },
    Action { label: "scan the workspace for changes", keys: "R", key: key(KeyCode::Char('R'), SHIFT), category: Category::Agent },
    Action { label: "manage items with the agent", keys: "M", key: key(KeyCode::Char('M'), SHIFT), category: Category::Agent },
    Action { label: "pick the model service", keys: "m", key: key(KeyCode::Char('m'), NONE), category: Category::Agent },

    Action { label: "read this group's notes", keys: "N", key: key(KeyCode::Char('N'), SHIFT), category: Category::Groups },
    Action { label: "archive finished items", keys: "X", key: key(KeyCode::Char('X'), SHIFT), category: Category::Groups },

    Action { label: "view settings menu", keys: "v", key: key(KeyCode::Char('v'), NONE), category: Category::View },
    Action { label: "scrolling ticker on or off", keys: "c", key: key(KeyCode::Char('c'), NONE), category: Category::View },
    Action { label: "pause the ticker", keys: "p", key: key(KeyCode::Char('p'), NONE), category: Category::View },
    Action { label: "ticker faster", keys: "+", key: key(KeyCode::Char('+'), SHIFT), category: Category::View },
    Action { label: "ticker slower", keys: "-", key: key(KeyCode::Char('-'), NONE), category: Category::View },

    Action { label: "keyboard help", keys: "?", key: key(KeyCode::Char('?'), SHIFT), category: Category::Session },
    Action { label: "quit", keys: "q", key: key(KeyCode::Char('q'), NONE), category: Category::Session },
];

/// How well `needle` matches `haystack`, or `None` if it does not.
pub fn score(_needle: &str, _haystack: &str) -> Option<u32> {
    None
}
```

Add the module in `src/ui/mod.rs`, keeping the existing alphabetical-ish order:

```rust
pub mod chyron;
pub mod edit;
pub mod palette;
mod view;
pub mod wrap;
```

- [ ] **Step 2: Write the failing scorer tests**

Append to `src/ui/palette.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subsequence_matches_and_a_non_subsequence_does_not() {
        assert!(score("arf", "archive finished items").is_some());
        assert!(score("zzz", "archive finished items").is_none());
    }

    #[test]
    fn characters_must_appear_in_order() {
        assert!(score("ab", "abc").is_some());
        assert!(score("ba", "abc").is_none());
    }

    #[test]
    fn matching_ignores_case_in_both_directions() {
        assert!(score("ARF", "archive finished items").is_some());
        assert!(score("arf", "ARCHIVE FINISHED ITEMS").is_some());
    }

    #[test]
    fn an_empty_needle_matches_everything_equally() {
        assert_eq!(score("", "anything"), Some(0));
        assert_eq!(score("", "something else"), Some(0));
    }

    #[test]
    fn a_word_start_outranks_a_mid_word_hit() {
        let at_start = score("f", "finished items").unwrap();
        let mid_word = score("f", "off screen").unwrap();
        assert!(at_start > mid_word, "{at_start} should beat {mid_word}");
    }

    #[test]
    fn a_contiguous_run_outranks_a_scattered_one() {
        let contiguous = score("arch", "archive items").unwrap();
        let scattered = score("arch", "a rather cheap thing").unwrap();
        assert!(contiguous > scattered, "{contiguous} should beat {scattered}");
    }

    #[test]
    fn an_early_match_outranks_a_late_one() {
        let early = score("item", "items list").unwrap();
        let late = score("item", "delete an item").unwrap();
        assert!(early > late, "{early} should beat {late}");
    }

    #[test]
    fn every_action_has_a_label_and_a_key_display() {
        for action in ACTIONS {
            assert!(!action.label.is_empty(), "a label is missing");
            assert!(!action.keys.is_empty(), "{} has no key display", action.label);
        }
    }

    #[test]
    fn action_labels_are_unique() {
        let mut labels: Vec<&str> = ACTIONS.iter().map(|a| a.label).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "two actions share a label");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --quiet palette 2>&1 | tail -20`
Expected: the seven `score` tests fail (the stub returns `None`); the two table tests pass.

- [ ] **Step 4: Implement the scorer**

Replace the `score` stub in `src/ui/palette.rs`:

```rust
/// How well `needle` matches `haystack`, or `None` if it does not.
///
/// A greedy left-to-right subsequence walk: not the optimal alignment fzf finds
/// by dynamic programming, but indistinguishable across 41 short labels.
pub fn score(needle: &str, haystack: &str) -> Option<u32> {
    if needle.trim().is_empty() {
        return Some(0);
    }
    let needle: Vec<char> = needle.to_lowercase().chars().collect();
    let hay: Vec<char> = haystack.to_lowercase().chars().collect();

    let mut points: u32 = 0;
    let mut next = 0usize;
    let mut previous_match: Option<usize> = None;

    for wanted in needle {
        let found = hay[next..].iter().position(|c| *c == wanted)? + next;
        let at_word_start = found == 0
            || matches!(hay.get(found - 1), Some(' ') | Some('-') | Some(':'));
        points += 1;
        if at_word_start {
            points += 8;
        }
        // Above the word-start bonus, so a contiguous run beats a needle whose
        // characters happen to land on several word starts.
        if previous_match == Some(found.saturating_sub(1)) {
            points += 10;
        }
        if previous_match.is_none() {
            points = points.saturating_sub(found as u32);
        }
        previous_match = Some(found);
        next = found + 1;
    }
    Some(points)
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --quiet palette 2>&1 | tail -6 && cargo clippy --all-targets 2>&1 | tail -3`
Expected: all nine tests pass, clippy silent.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/ui/palette.rs src/ui/mod.rs
git commit -m "feat(ui): every action as data, plus a fuzzy scorer"
```

---

### Task 2: Ranked entries, including one per model service

**Files:**
- Modify: `src/ui/palette.rs`

**Interfaces:**
- Consumes: `ACTIONS`, `score` (Task 1).
- Produces: `Entry` enum with `Key(&'static Action)` and `Service { name: String, index: usize }`, each with `label(&self) -> String`, `keys(&self) -> &str`, `category(&self) -> &'static str`; `filter(needle: &str, services: &[String]) -> Vec<Entry>`.

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `src/ui/palette.rs`:

```rust
    fn labels(entries: &[Entry]) -> Vec<String> {
        entries.iter().map(|e| e.label()).collect()
    }

    #[test]
    fn an_empty_needle_returns_every_entry_in_table_order() {
        let entries = filter("", &[]);
        assert_eq!(entries.len(), ACTIONS.len());
        assert_eq!(entries[0].label(), ACTIONS[0].label);
        assert_eq!(
            entries[ACTIONS.len() - 1].label(),
            ACTIONS[ACTIONS.len() - 1].label
        );
    }

    #[test]
    fn one_entry_is_appended_per_configured_service() {
        let services = vec!["claude".to_string(), "ollama".to_string()];
        let entries = filter("", &services);
        assert_eq!(entries.len(), ACTIONS.len() + 2);
        let found = labels(&entries);
        assert!(found.contains(&"use model service: claude".to_string()));
        assert!(found.contains(&"use model service: ollama".to_string()));
    }

    #[test]
    fn a_service_entry_carries_its_index() {
        let services = vec!["claude".to_string(), "ollama".to_string()];
        let entries = filter("ollama", &services);
        match entries.first() {
            Some(Entry::Service { name, index }) => {
                assert_eq!(name, "ollama");
                assert_eq!(*index, 1, "the index selects it in config order");
            }
            other => panic!("expected a service entry, got {other:?}"),
        }
    }

    #[test]
    fn the_worked_examples_rank_as_designed() {
        for (needle, expected) in [
            ("arf", "archive finished items"),
            ("mod", "pick the model service"),
            ("manage", "manage items with the agent"),
            ("sum", "summarise what's on screen"),
        ] {
            let entries = filter(needle, &[]);
            let top = entries.first().map(|e| e.label()).unwrap_or_default();
            assert_eq!(top, expected, "{needle:?} should rank {expected:?} first");
        }
    }

    #[test]
    fn the_category_is_searchable_too() {
        let entries = filter("agent scan", &[]);
        assert_eq!(
            entries.first().map(|e| e.label()).unwrap_or_default(),
            "scan the workspace for changes"
        );
    }

    #[test]
    fn a_needle_matching_nothing_returns_no_entries() {
        assert!(filter("qqzzxx", &[]).is_empty());
    }

    #[test]
    fn a_key_entry_reports_its_display_keys_and_a_service_entry_does_not() {
        let entries = filter("archive finished", &[]);
        assert_eq!(entries[0].keys(), "X");
        assert_eq!(entries[0].category(), "Groups");

        let services = vec!["codex".to_string()];
        let service = filter("codex", &services);
        assert_eq!(service[0].keys(), "");
        assert_eq!(service[0].category(), "Model");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet palette 2>&1 | tail -12`
Expected: compile errors — `Entry` and `filter` do not exist.

- [ ] **Step 3: Implement `Entry` and `filter`**

Add to `src/ui/palette.rs`, above the `score` function:

```rust
/// One row of the palette.
///
/// A service is not a keypress — picking one calls into the service list by
/// index — so the two cases stay distinct rather than being forced into a key.
#[derive(Debug, Clone)]
pub enum Entry {
    Key(&'static Action),
    Service { name: String, index: usize },
}

impl Entry {
    pub fn label(&self) -> String {
        match self {
            Entry::Key(action) => action.label.to_string(),
            Entry::Service { name, .. } => format!("use model service: {name}"),
        }
    }

    pub fn keys(&self) -> &str {
        match self {
            Entry::Key(action) => action.keys,
            Entry::Service { .. } => "",
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Entry::Key(action) => action.category.label(),
            Entry::Service { .. } => "Model",
        }
    }
}

/// The entries matching `needle`, best first.
///
/// `services` arrives as names rather than being read from config, so this stays
/// a pure function of its arguments.
pub fn filter(needle: &str, services: &[String]) -> Vec<Entry> {
    let all = ACTIONS
        .iter()
        .map(Entry::Key)
        .chain(services.iter().enumerate().map(|(index, name)| Entry::Service {
            name: name.clone(),
            index,
        }));

    let mut scored: Vec<(u32, Entry)> = all
        .filter_map(|entry| {
            let haystack = format!("{} {}", entry.label(), entry.category());
            score(needle, &haystack).map(|points| (points, entry))
        })
        .collect();

    // Stable, so an empty needle keeps the table's grouping.
    scored.sort_by(|left, right| right.0.cmp(&left.0));
    scored.into_iter().map(|(_, entry)| entry).collect()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --quiet palette 2>&1 | tail -8 && cargo clippy --all-targets 2>&1 | tail -3`
Expected: all pass, clippy silent. If a worked example ranks differently, adjust the label in `ACTIONS` rather than weakening the test — the labels exist to be searchable.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/ui/palette.rs
git commit -m "feat(ui): rank palette entries, with one per model service"
```

---

### Task 3: Generate the help screen from the table

**Files:**
- Modify: `src/ui/palette.rs`
- Modify: `src/ui/mod.rs` (delete `help_lines`, call the generated one)

**Interfaces:**
- Consumes: `ACTIONS`, `Category::ALL` (Task 1).
- Produces: `palette::help_lines() -> Vec<String>`. The free function `help_lines()` in `src/ui/mod.rs` is gone; its one caller uses `palette::help_lines()`.

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `src/ui/palette.rs`:

```rust
    #[test]
    fn the_help_screen_lists_every_action() {
        let text = help_lines().join("\n");
        for action in ACTIONS {
            assert!(text.contains(action.label), "help omits {:?}", action.label);
            assert!(text.contains(action.keys), "help omits key {:?}", action.keys);
        }
    }

    #[test]
    fn the_help_screen_is_grouped_by_category() {
        let text = help_lines().join("\n");
        for category in Category::ALL {
            assert!(
                text.contains(category.label()),
                "help omits the {} heading",
                category.label()
            );
        }
    }

    #[test]
    fn the_help_screen_mentions_the_palette_itself() {
        let text = help_lines().join("\n");
        assert!(text.contains(':'), "the palette key is worth finding: {text}");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet palette 2>&1 | tail -8`
Expected: `cannot find function help_lines in this scope`.

- [ ] **Step 3: Implement the generator**

Add to `src/ui/palette.rs`:

```rust
/// The `?` screen, grouped by category.
///
/// Generated from `ACTIONS` so a new binding cannot be added to the palette and
/// forgotten here.
pub fn help_lines() -> Vec<String> {
    let mut lines = vec![
        "  :  or  ctrl-k   command palette — type to filter, enter to run".to_string(),
        String::new(),
    ];
    for category in Category::ALL {
        lines.push(category.label().to_string());
        for action in ACTIONS.iter().filter(|a| a.category == category) {
            lines.push(format!("  {:<10} {}", action.keys, action.label));
        }
        lines.push(String::new());
    }
    lines
}
```

- [ ] **Step 4: Delete the hand-written help and repoint its caller**

In `src/ui/mod.rs`, delete the whole `fn help_lines() -> Vec<String> { ... }` free function, and change its single caller inside `handle_normal_key`:

```rust
            (K::Char('?'), _) => self.open_modal("keys", palette::help_lines()),
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --quiet 2>&1 | grep -E '^test result|FAILED'; cargo clippy --all-targets 2>&1 | grep -E '^(warning|error)' -A 4 | head`
Expected: whole suite green, clippy silent. Any test asserting on the old help text needs its expectation updated to the generated wording — check `cargo test --quiet ui:: 2>&1 | grep -i help`.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/ui/palette.rs src/ui/mod.rs
git commit -m "feat(ui): generate the help screen from the action table"
```

---

### Task 4: The palette mode, its keys, and the dead-entry pin

**Files:**
- Modify: `src/ui/mod.rs` (`Mode`, `App` fields, `App::new`, `handle_key` router, `handle_normal_key`)
- Modify: `src/ui/view.rs` (status-line match arm)

**Interfaces:**
- Consumes: `palette::{filter, Entry}` (Task 2).
- Produces: `Mode::Palette`; `App.palette_input: String`, `App.palette_cursor: usize`, `App.palette_scroll: usize`; `App::palette_entries(&self) -> Vec<Entry>`; `App::open_palette(&mut self)`; `App::run_palette_entry(&mut self)`; test-only `App.unhandled_key: bool`.

- [ ] **Step 1: Write the failing tests**

Add inside `mod tests` in `src/ui/mod.rs`, using the existing `app()`, `app_with_services()` and `press()` helpers:

```rust
    #[test]
    fn colon_and_ctrl_k_both_open_the_palette() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        assert_eq!(app.mode, Mode::Palette);

        app.mode = Mode::Normal;
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::Palette);
    }

    #[test]
    fn esc_closes_the_palette_and_runs_nothing() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.modal.is_none(), "nothing ran");
    }

    #[test]
    fn typing_filters_the_entries() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        let all = app.palette_entries().len();
        for c in "archive".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.palette_input, "archive");
        let narrowed = app.palette_entries().len();
        assert!(narrowed < all, "{narrowed} should be fewer than {all}");
        assert_eq!(
            app.palette_entries()[0].label(),
            "archive finished items"
        );
    }

    #[test]
    fn backspace_widens_the_filter_again() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Char('r'));
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.palette_input, "a");
    }

    // A stale cursor would leave whatever slid under it selected.
    #[test]
    fn each_keystroke_puts_the_cursor_back_on_the_top_match() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.palette_cursor, 2);
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.palette_cursor, 0);
    }

    #[test]
    fn ctrl_n_and_ctrl_p_move_like_the_arrows() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(app.palette_cursor, 1);
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert_eq!(app.palette_cursor, 0);
    }

    // The palette closes first, so the action's own mode survives.
    #[test]
    fn running_an_action_that_opens_a_mode_lands_in_that_mode() {
        let mut app = app_with_services();
        press(&mut app, KeyCode::Char(':'));
        for c in "manage".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::AskingAgent(Verb::Manage));
    }

    #[test]
    fn running_an_action_that_stays_in_normal_mode_closes_the_palette() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        for c in "hide or show".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let before = app.hide_done;
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Normal);
        assert_ne!(app.hide_done, before, "the action ran");
    }

    #[test]
    fn enter_with_no_matches_does_nothing() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        for c in "qqzzxx".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert!(app.palette_entries().is_empty());
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.modal.is_none());
    }

    #[test]
    fn running_a_service_entry_switches_the_active_service() {
        let mut app = app_with_services();
        press(&mut app, KeyCode::Char(':'));
        for c in "ollama".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.service.as_ref().unwrap().name, "ollama");
    }

    // The dead-entry pin: dispatch is by KeyEvent, so a typo in the table would
    // silently produce an entry that presses a key nothing handles.
    #[test]
    fn every_action_in_the_table_reaches_a_real_key_handler() {
        for action in crate::ui::palette::ACTIONS {
            let mut app = app();
            app.unhandled_key = false;
            app.handle_key(action.key);
            assert!(
                !app.unhandled_key,
                "{:?} presses {:?}, which no arm handles",
                action.label, action.keys
            );
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet ui:: 2>&1 | tail -12`
Expected: `no variant named Palette`, no field `palette_input`, no field `unhandled_key`.

- [ ] **Step 3: Add the mode, the state and the router arm**

`Mode`, after `ServiceMenu`:

```rust
    /// The command palette is open.
    Palette,
```

`App` fields, after `service_cursor`:

```rust
    /// Command palette: what has been typed, and where the cursor sits.
    pub palette_input: String,
    pub palette_cursor: usize,
    pub palette_scroll: usize,
    /// Set by the key handler's catch-all arm, so a table entry pressing an
    /// unbound key fails a test rather than doing nothing in silence.
    #[cfg(test)]
    pub unhandled_key: bool,
```

Initialise in `App::new`, beside `service_cursor: 0`:

```rust
            palette_input: String::new(),
            palette_cursor: 0,
            palette_scroll: 0,
            #[cfg(test)]
            unhandled_key: false,
```

Route the mode in `handle_key`, beside the `Mode::ServiceMenu` arm:

```rust
            Mode::Palette => self.handle_palette_key(key),
```

- [ ] **Step 4: Bind the opening keys and record unhandled ones**

In `handle_normal_key`, add beside the `m` arm:

```rust
            (K::Char(':'), _) => self.open_palette(),
            (K::Char('k'), KeyModifiers::CONTROL) => self.open_palette(),
```

and replace the final catch-all arm:

```rust
            _ => {
                #[cfg(test)]
                {
                    self.unhandled_key = true;
                }
            }
```

- [ ] **Step 5: Implement the handler**

Add to `impl App`, beside `handle_service_key`:

```rust
    fn open_palette(&mut self) {
        self.palette_input.clear();
        self.palette_cursor = 0;
        self.palette_scroll = 0;
        self.mode = Mode::Palette;
    }

    /// The entries the palette is currently showing.
    pub fn palette_entries(&self) -> Vec<palette::Entry> {
        let services: Vec<String> = self
            .config
            .services()
            .into_iter()
            .map(|service| service.name)
            .collect();
        palette::filter(&self.palette_input, &services)
    }

    fn handle_palette_key(&mut self, key: KeyEvent) {
        use KeyCode as K;
        let last = self.palette_entries().len().saturating_sub(1);
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            K::Esc => self.mode = Mode::Normal,
            K::Enter => self.run_palette_entry(),
            K::Down => self.palette_cursor = (self.palette_cursor + 1).min(last),
            K::Up => self.palette_cursor = self.palette_cursor.saturating_sub(1),
            K::Char('n') if control => {
                self.palette_cursor = (self.palette_cursor + 1).min(last)
            }
            K::Char('p') if control => {
                self.palette_cursor = self.palette_cursor.saturating_sub(1)
            }
            K::Backspace => {
                self.palette_input.pop();
                self.palette_cursor = 0;
                self.palette_scroll = 0;
            }
            K::Char(c) => {
                self.palette_input.push(c);
                self.palette_cursor = 0;
                self.palette_scroll = 0;
            }
            _ => {}
        }
    }

    /// Close the palette, then do what the highlighted entry stands for.
    ///
    /// Closing first is what lets an action open its own mode: dispatching while
    /// still in `Mode::Palette` would have that mode overwritten on the way out.
    fn run_palette_entry(&mut self) {
        let entries = self.palette_entries();
        let Some(entry) = entries.get(self.palette_cursor).cloned() else {
            self.mode = Mode::Normal;
            return;
        };
        self.mode = Mode::Normal;
        match entry {
            palette::Entry::Key(action) => self.handle_normal_key(action.key),
            palette::Entry::Service { index, .. } => self.select_service(index),
        }
    }
```

- [ ] **Step 6: Add the status-line arm**

`src/ui/view.rs`'s `render_status` matches on `app.mode` exhaustively, so add beside the `Mode::ServiceMenu` arm:

```rust
        (None, None, Mode::Palette) => Line::from(Span::styled(
            " type to filter · ↑↓ move · enter run · esc close ".to_string(),
            theme.statusbar(),
        )),
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --quiet 2>&1 | grep -E '^test result|FAILED'; cargo clippy --all-targets 2>&1 | grep -E '^(warning|error)' -A 4 | head`
Expected: whole suite green, clippy silent.

If `every_action_in_the_table_reaches_a_real_key_handler` fails, the named entry's `KeyEvent` does not match any arm — fix the table entry (usually a modifier: lowercase letters are bound with `KeyModifiers::NONE`, so `SHIFT` on one will miss).

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add src/ui/mod.rs src/ui/view.rs
git commit -m "feat(ui): a command palette that presses the key it names"
```

---

### Task 5: Draw it, and route clicks

**Files:**
- Modify: `src/ui/view.rs` (`render`, new `palette_rect`, `palette_visible_rows`, `render_palette`)
- Modify: `src/ui/mod.rs` (mouse routing in `handle_mouse` and a `click_palette`)

**Interfaces:**
- Consumes: `App.palette_input`, `App.palette_cursor`, `App.palette_scroll`, `App::palette_entries` (Task 4).
- Produces: `view::palette_rect(area: Rect) -> Rect`; `view::palette_visible_rows(area: Rect) -> usize`; `App::click_palette(&mut self, x: u16, y: u16)`.

- [ ] **Step 1: Write the failing tests**

Add inside `mod tests` in `src/ui/mod.rs`:

```rust
    #[test]
    fn the_palette_rect_is_centred_and_fits_the_frame() {
        let frame = Rect { x: 0, y: 0, width: 100, height: 40 };
        let rect = crate::ui::view::palette_rect(frame);
        assert!(rect.width < frame.width && rect.height < frame.height);
        assert_eq!(rect.x + rect.width / 2, frame.width / 2, "centred");
    }

    // `with_layout` draws a real frame headlessly and adopts its layout, so the
    // click lands where a live terminal would put it.
    #[test]
    fn a_click_on_a_palette_row_runs_that_entry() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        with_layout(&mut app);
        let rect = crate::ui::view::palette_rect(app.layout.whole);
        // The input line and the blank beneath it sit above row 0 of the list.
        app.click_palette(rect.x + 2, rect.y + 3);
        assert_eq!(app.mode, Mode::Normal, "the palette closed");
    }

    #[test]
    fn a_click_outside_the_palette_closes_it_without_running_anything() {
        let mut app = app();
        press(&mut app, KeyCode::Char(':'));
        with_layout(&mut app);
        app.click_palette(0, 0);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.modal.is_none());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet ui:: 2>&1 | tail -10`
Expected: `cannot find function palette_rect`, no method `click_palette`.

- [ ] **Step 3: Implement the geometry and the renderer**

Add to `src/ui/view.rs`, beside `review_rect`:

```rust
/// Where the palette is drawn, so clicks can reach its rows.
pub fn palette_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(10).clamp(30, 76);
    let height = area.height.saturating_sub(6).clamp(8, 18);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// How many entry rows the palette can show at once.
///
/// It spends rows on its border, the input line, the blank beneath it and the
/// footer hint.
pub fn palette_visible_rows(area: Rect) -> usize {
    palette_rect(area).height.saturating_sub(5) as usize
}

/// The command palette: an input line over a ranked list.
fn render_palette(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let rect = palette_rect(area);
    let width = rect.width.saturating_sub(2) as usize;
    let height = palette_visible_rows(area);

    let entries = app.palette_entries();
    let start = clamp_scroll(app.palette_scroll, entries.len(), height);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(" > ", theme.query()),
            Span::styled(app.palette_input.clone(), theme.command_input()),
            Span::styled("\u{2588}", theme.command_input()),
        ]),
        Line::from(""),
    ];

    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            " no matching command".to_string(),
            theme.inactive(),
        )));
    }

    for index in start..(start + height).min(entries.len()) {
        let entry = &entries[index];
        let selected = index == app.palette_cursor;
        let style = if selected {
            theme.selected(&theme.paragraph())
        } else {
            theme.paragraph()
        };
        let marker = if selected { "\u{25b8} " } else { "  " };
        let tail = format!("{}  {}", entry.category(), entry.keys());
        let label = entry.label();
        let room = width
            .saturating_sub(marker.chars().count())
            .saturating_sub(tail.chars().count() + 2);
        lines.push(Line::from(vec![
            Span::styled(format!("{marker}{}", truncate(&label, room)), style),
            Span::styled(
                format!(
                    "{}{tail}",
                    " ".repeat(
                        width
                            .saturating_sub(marker.chars().count())
                            .saturating_sub(truncate(&label, room).chars().count())
                            .saturating_sub(tail.chars().count())
                    )
                ),
                theme.inactive(),
            ),
        ]));
    }

    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.eff_border(true))
                .title(format!("palette — {}/{}", entries.len(), ACTIONS_TOTAL)),
        ),
        rect,
    );
}
```

Add the count constant near the top of `src/ui/view.rs`:

```rust
use super::palette::ACTIONS;

const ACTIONS_TOTAL: usize = ACTIONS.len();
```

Call it from `render`, beside the service-menu call:

```rust
    if app.mode == Mode::Palette {
        render_palette(app, frame, f.whole);
    }
```

- [ ] **Step 4: Route clicks**

Add to `impl App` in `src/ui/mod.rs`, beside `click_service_menu`:

```rust
    /// A click while the palette is open: run the row, or dismiss.
    pub fn click_palette(&mut self, x: u16, y: u16) {
        let rect = view::palette_rect(self.layout.whole);
        if !within(rect, x, y) {
            self.mode = Mode::Normal;
            return;
        }
        // The input line and the blank beneath it sit above row 0 of the list.
        let list_top = rect.y + 3;
        if y < list_top {
            return;
        }
        let row = (y - list_top) as usize + self.palette_scroll;
        if row < self.palette_entries().len() {
            self.palette_cursor = row;
            self.run_palette_entry();
        }
    }
```

and route it in `handle_mouse`, beside the `Mode::ServiceMenu` block:

```rust
        if self.mode == Mode::Palette {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.click_palette(x, y);
            }
            return;
        }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --quiet 2>&1 | grep -E '^test result|FAILED'; cargo clippy --all-targets 2>&1 | grep -E '^(warning|error)' -A 4 | head`
Expected: whole suite green, clippy silent.

- [ ] **Step 6: Check it renders against a real backend**

`src/ui/mod.rs` already has a `with_layout(app)` test helper (around line 3330) that draws a real frame through ratatui's `TestBackend` at 100x24 and adopts the resulting layout. Reuse it — a panic in the palette's layout maths surfaces as a failing `with_layout` call:

```rust
    #[test]
    fn drawing_the_palette_does_not_panic_at_any_filter_width() {
        let mut app = app_with_services();
        press(&mut app, KeyCode::Char(':'));
        with_layout(&mut app);

        for c in "archive finished items and then some overflowing text".chars() {
            press(&mut app, KeyCode::Char(c));
            with_layout(&mut app);
        }
        assert!(app.palette_entries().is_empty(), "the filter narrowed to nothing");
    }
```

Typing through a long needle redraws at every width, which is where truncation maths tends to panic. At 100x24 `palette_rect` yields 76x18, so the frame has room; do not raise the `TestBackend` size to make a failure go away.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/ui/view.rs src/ui/mod.rs
git commit -m "feat(ui): draw the palette and route clicks to its rows"
```

---

### Task 6: Document it, and the full gate

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: everything above.
- Produces: no code interfaces.

- [ ] **Step 1: Add the palette to the keys table**

In `README.md`, in the keys table that currently ends with the `?` / `q` row, add as the first row of the table so it reads as the entry point:

```markdown
| `:` `ctrl-k` | command palette | `?` | help |
```

and remove the now-duplicated `| `?` | help | `q` | quit |` row's `?` cell only if it leaves the table ragged — otherwise leave both.

- [ ] **Step 2: Add a short section after the keys table**

```markdown
## The command palette

`:` or `ctrl-k` opens a palette over every action in the app. Type to filter —
`arf` finds "archive finished items", `gs` finds "git sync" — `↑`/`↓` or
`ctrl-p`/`ctrl-n` to move, `enter` to run, `esc` to close.

Each row shows its key on the right, so using the palette teaches you the
binding and you stop needing it. Configured model services appear as entries
too, so `:ollama` switches model in three keystrokes.

The palette does not implement anything: an entry presses the key it names, so
it behaves exactly as the key does. `?` prints the same table as a static
reference, generated from the same source.
```

- [ ] **Step 3: Run the full gate**

Run:
```bash
cargo test 2>&1 | grep -E '^test result'
cargo clippy --all-targets 2>&1 | grep -cE '^(warning|error)'
cargo fmt --check && echo FMT-OK
```
Expected: one `test result: ok` line with 0 failed and 0 ignored; clippy count `0`; `FMT-OK`.

- [ ] **Step 4: Check the comment budget**

```bash
comments=$(git diff main -U0 -- '*.rs' | grep -cE '^\+\s*//')
added=$(git diff main -U0 -- '*.rs' | grep -cE '^\+')
echo "$comments of $added added rust lines = $((100*comments/added))%"
```
Expected: under ~10%. Over that, re-read every added comment and delete what is narrative rather than a hidden constraint.

- [ ] **Step 5: Smoke-test the real binary**

```bash
cargo install --path .
mitodo
```
Press `:` — the palette opens listing every action. Type `arf` — "archive finished items" ranks first. Press `esc`. Press `:`, type `manage`, press `enter` — the `manage:` prompt opens rather than dropping back to Normal. Press `esc`. Press `?` — the help screen now lists `o`, `A`, `p`, `+`, `-`, `z`, `Z` and `tab`, which it previously omitted.

- [ ] **Step 6: Commit**

```bash
git add README.md
git commit -m "docs: the command palette, and its keys"
```

---

## Self-review

**Spec coverage.** §1 intent → Tasks 1–5; §2 decisions: open key → Task 4 step 4, entries → Task 1 table, matching → Task 1 step 4, dispatch → Task 4 step 5; §3 architecture → Tasks 1–5, one module per the split; §4 action table → Task 1 (all 41 rows, counted: Navigation 14, Items 7, Query 3, Agent 8, Groups 2, View 5, Session 2); §4 `Entry` enum → Task 2; §5 on-screen behaviour → Task 5 (render), Task 4 (cursor reset, no-match inertness, run-after-close ordering); §6 matching and ranking → Task 1 step 4 plus Task 2's worked-example test; §7 testing → the test steps of every task, with the `unhandled_key` pin in Task 4; §8 out-of-scope → nothing in any task; §9 files touched → Tasks 1–6.

**Placeholder scan.** No TBD/TODO, no "add error handling", no "similar to Task N". Task 5 step 6 and Task 6 step 1 both say "follow the existing pattern if it differs" — those are instructions to check a fact in the codebase, not deferred work, and each names the exact `grep` to run.

**Verified against the codebase while writing.** `KeyEvent::new` is `pub const fn` in crossterm (`event.rs:757`), so the `const ACTIONS` array compiles; `with_layout` exists in `src/ui/mod.rs` and is what the click and render tests use.

**Type consistency.** `score(&str, &str) -> Option<u32>` identical in Tasks 1 and 2. `filter(&str, &[String]) -> Vec<Entry>` identical in Tasks 2, 4. `Entry::{Key, Service{name,index}}` with `label()/keys()/category()` consistent between Task 2's definition and Task 5's renderer. `palette_rect(Rect) -> Rect` matches between Task 5's definition and its tests. `App.palette_{input,cursor,scroll}` and `unhandled_key` named identically in Tasks 4 and 5. `Category::ALL` used in Task 3 as defined in Task 1.

**One ordering hazard.** Task 3 deletes `help_lines` from `src/ui/mod.rs`; if any existing UI test asserts on the old help wording it will fail there, and Task 3 step 5 names the `grep` to find it. Do not skip that step on the assumption that no test reads the help text.
