# mitodo Foundation Implementation Plan (Phase 1 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the forked `eilmeldung` RSS reader into a crate named `mitodo` that can parse, autodetect, and safely write a real markdown TODO workspace from the command line — with no TUI involved.

**Architecture:** Strip the RSS layer first so builds are fast, moving the not-yet-ported TUI code to a `src/_port/` staging directory that is not compiled. Then build the store bottom-up: types, parser, config, detection, writer. Every write re-reads the file and verifies the target line before touching it, and preserves every untouched byte.

**Tech Stack:** Rust 2024 edition, clap (CLI), serde + toml + config (configuration), sha2 (content-hash item ids), thiserror + color-eyre (errors), rstest (tests).

## Global Constraints

- Crate name `mitodo`, version `0.1.0`, edition `2024`, licence `GPL-3.0-or-later`.
- Item line numbers are **0-based** everywhere in Rust code. (`todos.py` used 1-based; do not copy that.)
- The writer's contract: after any mutation, **every line the user did not edit is byte-identical**, including line endings and the presence or absence of a trailing newline.
- No SQLite, no index. Parse into memory.
- Nothing in `src/_port/` is compiled during this phase. Do not add `mod` declarations for it.
- Phase 1 ships a working CLI. Phases 2 (TUI) and 3 (agent, git, chyron) follow in separate plans.

---

### Task 1: Rename crate and strip the RSS layer

**Files:**
- Modify: `Cargo.toml`
- Create: `src/_port/README.md`
- Move: `src/ui/`, `src/query/`, `src/messages/`, `src/config/`, `src/input/`, `src/utils/` → `src/_port/`
- Delete: `src/newsflash_utils.rs`, `src/login.rs`, `src/connectivity.rs`
- Modify: `src/main.rs`, `src/cli.rs`, `src/prelude.rs`

**Interfaces:**
- Consumes: nothing (first task).
- Produces: a crate that builds in seconds with no `news-flash` or `openssl`; `CliArgs` with `config_dir: Option<PathBuf>`, `log_file`, `log_level`; binary prints `mitodo 0.1.0` for `--version`.

- [ ] **Step 1: Move the not-yet-ported TUI code out of the compile path**

```bash
cd /Users/home/repos/mitodo
mkdir -p src/_port
git mv src/ui src/query src/messages src/config src/input src/utils src/_port/
git rm -q src/newsflash_utils.rs src/login.rs src/connectivity.rs
```

- [ ] **Step 2: Explain the staging directory**

Create `src/_port/README.md`:

```markdown
# Staging area for un-ported eilmeldung code

This directory holds the original [eilmeldung](https://github.com/christo-auer/eilmeldung)
TUI modules. Nothing here is compiled — there are no `mod` declarations pointing
at it.

Phase 2 of the mitodo port moves modules out of here one at a time, rewriting
their RSS-specific parts against `crate::store`. When this directory is empty,
the port is done.

| module | becomes | phase |
|---|---|---|
| `ui/feeds_list` | `ui/groups_list` | 2 |
| `ui/articles_list` | `ui/items_list` | 2 |
| `ui/article_content` | `ui/item_detail` | 2 |
| `ui/chyron` | `ui/chyron` (vocabulary swap) | 3 |
| `query` | `query` (vocabulary swap) | 2 |
| `config`, `input`, `messages`, `utils` | same names | 2 |
```

- [ ] **Step 3: Rewrite `Cargo.toml`**

Replace the `[package]` and `[dependencies]` sections with:

```toml
[package]
name = "mitodo"
version = "0.1.0"
edition = "2024"
description = "a TUI todo tracker over plain markdown checklists"
homepage = "https://github.com/gwicho38/mitodo"
repository = "https://github.com/gwicho38/mitodo"
license = "GPL-3.0-or-later"
keywords = [ "todo", "tui", "markdown", "tasks" ]
categories = [ "command-line-utilities" ]
publish = true

[dependencies]
chrono = "0.4.42"
clap = { version = "4.5.53", features = ["derive"] }
color-eyre = "0.6.5"
config = "0.15.17"
directories = "6.0.0"
env_logger = "0.11.8"
getset = "0.1.6"
indexmap = { version = "2.12.1", features = ["serde"] }
itertools = "0.14.0"
log = "0.4.28"
once_cell = "1.21.3"
regex = "1.12.2"
serde = { version = "1.0.226", features = ["derive"] }
sha2 = "0.10.9"
strum = { version = "0.27.2", features = ["derive", "strum_macros"] }
thiserror = "2.0.17"
toml = "0.9.10"

[dev-dependencies]
claims = "0.8.0"
rstest = "0.26.1"
tempfile = "3.14.0"
```

Leave `[profile.*]` and any other existing sections untouched.

- [ ] **Step 4: Reduce `src/prelude.rs`**

```rust
pub use color_eyre::eyre::{Result, eyre};
pub use log::{debug, error, info, warn};
```

- [ ] **Step 5: Rewrite `src/cli.rs`**

```rust
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use getset::Getters;
use log::LevelFilter;

#[derive(Parser, Debug, Getters)]
#[command(name = "mitodo", version, about = "a TUI todo tracker over plain markdown checklists")]
#[getset(get = "pub")]
pub struct CliArgs {
    /// Log file (must be writable)
    #[arg(long)]
    log_file: Option<PathBuf>,

    /// Log level (OFF, ERROR, WARN, INFO, DEBUG, TRACE)
    #[arg(long)]
    log_level: Option<LevelFilter>,

    /// Directory holding config.toml
    #[arg(short, long)]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Detect the layout of a todo workspace and write a config file
    Init {
        /// Workspace root directory
        root: PathBuf,
        /// Overwrite an existing config file
        #[arg(long)]
        force: bool,
    },
    /// Print the workspace to stdout
    List,
}
```

- [ ] **Step 6: Rewrite `src/main.rs`**

```rust
mod cli;
mod logging;
mod prelude;

use clap::Parser;

use crate::cli::CliArgs;
use crate::prelude::*;

fn main() -> Result<()> {
    color_eyre::install()?;
    let args = CliArgs::parse();
    logging::init(args.log_file().as_deref(), *args.log_level());
    info!("mitodo starting");
    Ok(())
}
```

- [ ] **Step 7: Adjust `src/logging.rs` to the new signature**

Read the existing file first. Replace its public entry point with:

```rust
use std::path::Path;

use log::LevelFilter;

pub fn init(log_file: Option<&Path>, level: Option<LevelFilter>) {
    let level = level.unwrap_or(LevelFilter::Warn);
    let mut builder = env_logger::Builder::new();
    builder.filter_level(level);
    if let Some(path) = log_file
        && let Ok(file) = std::fs::File::create(path)
    {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    let _ = builder.try_init();
}
```

- [ ] **Step 8: Verify the build is green and fast**

Run: `cargo build 2>&1 | tail -20`
Expected: `Finished` with no errors. No `openssl-sys` or `news-flash` in the compile output.

Run: `cargo run -- --version`
Expected: `mitodo 0.1.0`

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor: strip RSS layer, rename crate to mitodo

Moves un-ported eilmeldung TUI modules to src/_port/ (not compiled).
Deletes newsflash_utils.rs, login.rs, connectivity.rs. Drops news-flash,
openssl, reqwest, image and related dependencies, cutting build time to
seconds."
```

---

### Task 2: Core types

**Files:**
- Create: `src/store/mod.rs`
- Create: `src/store/model.rs`
- Modify: `src/main.rs` (add `mod store;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `Priority` enum: `P0 | P1 | P2 | P3 | None`, with `Priority::from_heading(&str) -> Priority`.
  - `ItemId(String)` with `ItemId::compute(file_rel: &str, section: &str, heading: &str, indent: usize, text: &str) -> ItemId` and `fn as_str(&self) -> &str`.
  - `Item` struct with public fields as listed below.
  - `Group` struct with `name: String`, `todo_file: PathBuf`, `notes_file: Option<PathBuf>`, `archive_dir: Option<PathBuf>`.

- [ ] **Step 1: Write the failing test**

Create `src/store/model.rs` containing only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_parses_from_section_heading() {
        assert_eq!(Priority::from_heading("P0 — Critical / Time-Sensitive"), Priority::P0);
        assert_eq!(Priority::from_heading("P1 — High Priority"), Priority::P1);
        assert_eq!(Priority::from_heading("P3 — Someday"), Priority::P3);
        assert_eq!(Priority::from_heading("Notes"), Priority::None);
        assert_eq!(Priority::from_heading(""), Priority::None);
    }

    #[test]
    fn item_id_is_stable_for_identical_content() {
        let a = ItemId::compute("lefv/TODO.md", "P0", "Prefecture", 0, "Check convocation");
        let b = ItemId::compute("lefv/TODO.md", "P0", "Prefecture", 0, "Check convocation");
        assert_eq!(a, b);
    }

    #[test]
    fn item_id_changes_when_text_changes() {
        let a = ItemId::compute("lefv/TODO.md", "P0", "Prefecture", 0, "Check convocation");
        let b = ItemId::compute("lefv/TODO.md", "P0", "Prefecture", 0, "Check convocation now");
        assert_ne!(a, b);
    }

    #[test]
    fn item_id_distinguishes_same_text_in_different_files() {
        let a = ItemId::compute("lefv/TODO.md", "P0", "H", 0, "same text");
        let b = ItemId::compute("jzlaw/TODO.md", "P0", "H", 0, "same text");
        assert_ne!(a, b);
    }

    #[test]
    fn item_id_is_twelve_hex_chars() {
        let id = ItemId::compute("f", "s", "h", 0, "t");
        assert_eq!(id.as_str().len(), 12);
        assert!(id.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `mod store;` to `src/main.rs` above `mod cli;`, and create `src/store/mod.rs`:

```rust
pub mod model;

pub use model::{Group, Item, ItemId, Priority};
```

Run: `cargo test --lib model 2>&1 | tail -20`
Expected: compile failure — `cannot find type Priority in this scope`.

- [ ] **Step 3: Write the implementation**

Prepend to `src/store/model.rs`, above the test module:

```rust
use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// Derived priority of an item. `None` means the workspace has no priority
/// source configured, or the item's section did not match one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Priority {
    P0,
    P1,
    P2,
    P3,
    #[default]
    None,
}

impl Priority {
    /// Parse a leading `P0`–`P3` out of a section heading such as
    /// `"P1 — High Priority"`. Anything else is `Priority::None`.
    pub fn from_heading(heading: &str) -> Self {
        let mut chars = heading.trim_start().chars();
        if chars.next() != Some('P') {
            return Priority::None;
        }
        match chars.next() {
            Some('0') => Priority::P0,
            Some('1') => Priority::P1,
            Some('2') => Priority::P2,
            Some('3') => Priority::P3,
            _ => Priority::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::P0 => "P0",
            Priority::P1 => "P1",
            Priority::P2 => "P2",
            Priority::P3 => "P3",
            Priority::None => "-",
        }
    }
}

/// Content-addressed item identifier.
///
/// Deliberately derived from content rather than position: a text edit yields a
/// new id, which matches the scheme used by the `todos-mcp` server so both
/// tools agree on item identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(String);

impl ItemId {
    pub fn compute(
        file_rel: &str,
        section: &str,
        heading: &str,
        indent: usize,
        text: &str,
    ) -> Self {
        let mut hasher = Sha256::new();
        // Unit separator between fields so that concatenation is unambiguous.
        for field in [file_rel, section, heading] {
            hasher.update(field.as_bytes());
            hasher.update([0x1f]);
        }
        hasher.update(indent.to_string().as_bytes());
        hasher.update([0x1f]);
        hasher.update(text.as_bytes());
        let digest = hasher.finalize();
        ItemId(hex_12(&digest))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn hex_12(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

/// A single checkbox line, plus the description block beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: ItemId,
    pub file: PathBuf,
    /// 0-based line index of the checkbox line within `file`.
    pub line: usize,
    /// Leading whitespace width of the checkbox line, in characters.
    pub indent: usize,
    pub done: bool,
    pub text: String,
    /// Blockquote lines directly beneath the item, joined with newlines.
    pub description: String,
    /// The `## ` heading in force above this item.
    pub section: String,
    /// The `### ` heading in force above this item.
    pub heading: String,
    pub priority: Priority,
    pub parent: Option<ItemId>,
    pub children: Vec<ItemId>,
}

/// One account, project, or context — a top-level node in the UI's left pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub todo_file: PathBuf,
    pub notes_file: Option<PathBuf>,
    pub archive_dir: Option<PathBuf>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib model 2>&1 | tail -20`
Expected: `test result: ok. 5 passed`

- [ ] **Step 5: Commit**

```bash
git add src/store src/main.rs
git commit -m "feat(store): add Item, Group, Priority and content-hash ItemId"
```

---

### Task 3: Markdown parser

**Files:**
- Create: `src/store/parse.rs`
- Create: `tests/fixtures/basic/TODO.md`
- Modify: `src/store/mod.rs`

**Interfaces:**
- Consumes: `Item`, `ItemId`, `Priority` from Task 2.
- Produces: `parse_todo_file(path: &Path, file_rel: &str, source: &str) -> Vec<Item>` — takes already-read source text so tests need no filesystem, and returns items in document order with parent/child links populated.

Parser rules, ported from `todos.py` with three corrections noted inline:

| input | behaviour |
|---|---|
| `---` fenced YAML block at the very start of the file | skipped entirely (**correction:** `todos.py` did not handle frontmatter) |
| `## Heading` | sets section, clears heading, clears the parent stack |
| `### Heading` | sets heading, clears the parent stack |
| `- [ ] text` / `- [x] text` | an item; `[X]` counts as done |
| leading whitespace on a checkbox | nesting depth, resolved against an indent stack (**correction:** `todos.py` supported only one level) |
| `  > note` directly beneath an item | appended to that item's description |
| anything else | ignored, but still consumes a line number |

- [ ] **Step 1: Write the fixture**

Create `tests/fixtures/basic/TODO.md` — mirrors the real `lefv/TODO.md` shape:

```markdown
---
tags: ["lefv", "lefv/todo"]
---

# LEFV Law — TODOs

**Account:** luis@lefv.io

---

## P0 — Critical / Time-Sensitive

### Prefecture Appointment

- [x] Check convocation for exact date/time
- [ ] Attend PREF92 appointment
  > do NOT miss — government immigration
  > bring the original convocation letter

## P1 — High Priority

### Holon Law

- [ ] Respond to Shani Phillips
  - [ ] draft the reply
  - [x] look up her timezone
- [ ] Confirm the 10am call
```

- [ ] **Step 2: Write the failing test**

Create `src/store/parse.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture() -> Vec<Item> {
        let source = include_str!("../../tests/fixtures/basic/TODO.md");
        parse_todo_file(Path::new("lefv/TODO.md"), "lefv/TODO.md", source)
    }

    #[test]
    fn frontmatter_is_not_parsed_as_content() {
        let items = fixture();
        assert!(items.iter().all(|i| !i.text.contains("tags")));
    }

    #[test]
    fn finds_every_checkbox() {
        assert_eq!(fixture().len(), 6);
    }

    #[test]
    fn records_done_state() {
        let items = fixture();
        assert!(items[0].done, "first item is [x]");
        assert!(!items[1].done, "second item is [ ]");
    }

    #[test]
    fn assigns_section_and_heading() {
        let items = fixture();
        assert_eq!(items[0].section, "P0 — Critical / Time-Sensitive");
        assert_eq!(items[0].heading, "Prefecture Appointment");
        assert_eq!(items[2].section, "P1 — High Priority");
        assert_eq!(items[2].heading, "Holon Law");
    }

    #[test]
    fn derives_priority_from_section() {
        let items = fixture();
        assert_eq!(items[0].priority, Priority::P0);
        assert_eq!(items[2].priority, Priority::P1);
    }

    #[test]
    fn collects_multi_line_descriptions() {
        let items = fixture();
        assert_eq!(
            items[1].description,
            "do NOT miss — government immigration\nbring the original convocation letter"
        );
    }

    #[test]
    fn items_without_a_description_have_an_empty_one() {
        assert_eq!(fixture()[0].description, "");
    }

    #[test]
    fn nests_indented_items_under_their_parent() {
        let items = fixture();
        let parent = &items[2];
        assert_eq!(parent.text, "Respond to Shani Phillips");
        assert_eq!(parent.children.len(), 2);
        assert_eq!(items[3].parent.as_ref(), Some(&parent.id));
        assert_eq!(items[4].parent.as_ref(), Some(&parent.id));
    }

    #[test]
    fn top_level_items_have_no_parent() {
        assert!(fixture()[0].parent.is_none());
    }

    #[test]
    fn records_zero_based_line_numbers() {
        let source = include_str!("../../tests/fixtures/basic/TODO.md");
        let items = fixture();
        let lines: Vec<&str> = source.lines().collect();
        for item in &items {
            assert!(
                lines[item.line].contains(&item.text),
                "line {} should contain {:?}",
                item.line,
                item.text
            );
        }
    }

    #[test]
    fn a_new_section_resets_nesting() {
        let items = fixture();
        // items[2] opens a new section; it must not be adopted by the
        // parent that was open at the end of the previous section.
        assert!(items[2].parent.is_none());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Add to `src/store/mod.rs`:

```rust
pub mod model;
pub mod parse;

pub use model::{Group, Item, ItemId, Priority};
pub use parse::parse_todo_file;
```

Run: `cargo test --lib parse 2>&1 | tail -20`
Expected: compile failure — `cannot find function parse_todo_file`.

- [ ] **Step 4: Write the implementation**

Prepend to `src/store/parse.rs`:

```rust
use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

use super::model::{Item, ItemId, Priority};

static CHECKBOX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\s*)- \[([ xX])\] (.+)$").expect("valid checkbox regex"));
static HEADING2_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^## (.+)$").expect("valid h2 regex"));
static HEADING3_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^### (.+)$").expect("valid h3 regex"));
static DESC_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s+>\s?(.*)$").expect("valid description regex"));

/// Number of leading lines occupied by a YAML frontmatter block, or 0 if the
/// file does not open with one.
fn frontmatter_len(lines: &[&str]) -> usize {
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return 0;
    }
    match lines.iter().skip(1).position(|l| l.trim_end() == "---") {
        // +2 accounts for the opening delimiter and the closing one.
        Some(offset) => offset + 2,
        None => 0,
    }
}

/// Parse one markdown todo file into items in document order.
///
/// `file_rel` is the workspace-relative path used for id computation, so that
/// identical text in two different files yields different ids.
pub fn parse_todo_file(path: &Path, file_rel: &str, source: &str) -> Vec<Item> {
    let lines: Vec<&str> = source.lines().collect();
    let start = frontmatter_len(&lines);

    let mut items: Vec<Item> = Vec::new();
    let mut section = String::new();
    let mut heading = String::new();
    // (indent width, index into `items`) for each open ancestor.
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut last: Option<usize> = None;

    for (line_no, raw) in lines.iter().enumerate().skip(start) {
        if let Some(caps) = HEADING2_RE.captures(raw) {
            section = caps[1].trim().to_string();
            heading.clear();
            stack.clear();
            last = None;
            continue;
        }

        if let Some(caps) = HEADING3_RE.captures(raw) {
            heading = caps[1].trim().to_string();
            stack.clear();
            last = None;
            continue;
        }

        if let Some(caps) = CHECKBOX_RE.captures(raw) {
            let indent = caps[1].chars().count();
            let done = caps[2].eq_ignore_ascii_case("x");
            let text = caps[3].trim().to_string();

            // Close any ancestors at or below this indent level.
            while stack.last().is_some_and(|(width, _)| *width >= indent) {
                stack.pop();
            }
            let parent_idx = stack.last().map(|(_, idx)| *idx);

            let id = ItemId::compute(file_rel, &section, &heading, indent, &text);
            let item = Item {
                id: id.clone(),
                file: path.to_path_buf(),
                line: line_no,
                indent,
                done,
                text,
                description: String::new(),
                section: section.clone(),
                heading: heading.clone(),
                priority: Priority::from_heading(&section),
                parent: parent_idx.map(|idx| items[idx].id.clone()),
                children: Vec::new(),
            };

            let idx = items.len();
            items.push(item);
            if let Some(pidx) = parent_idx {
                items[pidx].children.push(id);
            }
            stack.push((indent, idx));
            last = Some(idx);
            continue;
        }

        if let Some(caps) = DESC_RE.captures(raw)
            && let Some(idx) = last
        {
            let note = caps[1].trim_end();
            let description = &mut items[idx].description;
            if description.is_empty() {
                description.push_str(note);
            } else {
                description.push('\n');
                description.push_str(note);
            }
        }
    }

    items
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib parse 2>&1 | tail -20`
Expected: `test result: ok. 11 passed`

- [ ] **Step 6: Prove it against the real workspace**

Run:

```bash
cargo test --lib 2>&1 | tail -5
```

Expected: all tests pass (16 total across model and parse).

- [ ] **Step 7: Commit**

```bash
git add src/store/parse.rs src/store/mod.rs tests/fixtures
git commit -m "feat(store): parse markdown todo files into items

Ports the todos.py parser with three corrections: YAML frontmatter is
skipped, nesting uses an indent stack rather than a single parent slot,
and line numbers are 0-based."
```

---

### Task 4: Configuration

**Files:**
- Create: `src/config/mod.rs`
- Modify: `src/main.rs` (add `mod config;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `Config { workspace: WorkspaceConfig, priority: PriorityConfig, git: GitConfig }`, all `serde::Deserialize + Serialize + Default`.
  - `WorkspaceConfig { root: PathBuf, group_by: GroupBy, todo_glob: String, notes_glob: Option<String>, archive_dir: Option<String> }`.
  - `GroupBy::{Directory, Heading}`, serialised snake_case.
  - `PriorityConfig { source: PrioritySource, pattern: String }`, `PrioritySource::{Heading, Tag, None}`.
  - `GitConfig { enabled: bool, sync: Vec<Vec<String>> }`.
  - `Config::load(path: &Path) -> Result<Config, ConfigError>` and `Config::save(&self, path: &Path) -> Result<(), ConfigError>`.
  - `default_config_path() -> PathBuf` — `~/.config/mitodo/config.toml` via `directories`.

- [ ] **Step 1: Write the failing test**

Create `src/config/mod.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Deliberately not a `~` path: `Config::load` expands tildes, so a tilde
    // here would break the save/load round-trip test. Expansion is covered by
    // its own test below.
    const SAMPLE: &str = r#"
[workspace]
root        = "/tmp/todo-workspace"
group_by    = "directory"
todo_glob   = "*/TODO.md"
notes_glob  = "*/notes.md"
archive_dir = "_archive"

[priority]
source  = "heading"
pattern = "^P([0-3])"

[git]
enabled = true
sync    = [["add", "-A"], ["commit", "-m", "mitodo: sync"]]
"#;

    #[test]
    fn parses_a_full_config() {
        let cfg: Config = toml::from_str(SAMPLE).expect("sample config parses");
        assert_eq!(cfg.workspace.group_by, GroupBy::Directory);
        assert_eq!(cfg.workspace.todo_glob, "*/TODO.md");
        assert_eq!(cfg.workspace.archive_dir.as_deref(), Some("_archive"));
        assert_eq!(cfg.priority.source, PrioritySource::Heading);
        assert!(cfg.git.enabled);
        assert_eq!(cfg.git.sync.len(), 2);
        assert_eq!(cfg.git.sync[0], vec!["add", "-A"]);
    }

    #[test]
    fn round_trips_through_toml() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        let rendered = toml::to_string_pretty(&cfg).unwrap();
        let again: Config = toml::from_str(&rendered).unwrap();
        assert_eq!(cfg, again);
    }

    #[test]
    fn missing_optional_sections_fall_back_to_defaults() {
        let minimal = r#"
[workspace]
root      = "/tmp/w"
group_by  = "heading"
todo_glob = "TODO.md"
"#;
        let cfg: Config = toml::from_str(minimal).expect("minimal config parses");
        assert_eq!(cfg.priority.source, PrioritySource::None);
        assert!(!cfg.git.enabled);
        assert!(cfg.workspace.notes_glob.is_none());
    }

    #[test]
    fn saves_and_loads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        cfg.save(&path).expect("save succeeds");
        let loaded = Config::load(&path).expect("load succeeds");
        assert_eq!(cfg, loaded);
    }

    #[test]
    fn loading_a_missing_file_is_an_error() {
        let err = Config::load(std::path::Path::new("/nonexistent/config.toml"));
        assert!(err.is_err());
    }

    #[test]
    fn tilde_in_root_is_expanded_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[workspace]\nroot = \"~/repos/TODO\"\ngroup_by = \"directory\"\ntodo_glob = \"*/TODO.md\"\n",
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        assert!(!cfg.workspace.root.to_string_lossy().starts_with('~'));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `mod config;` to `src/main.rs`.

Run: `cargo test --lib config 2>&1 | tail -20`
Expected: compile failure — `cannot find type Config in this scope`.

- [ ] **Step 3: Write the implementation**

Prepend to `src/config/mod.rs`:

```rust
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("config file could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("config file is not valid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("config could not be serialised: {0}")]
    Serialise(#[from] toml::ser::Error),
    #[error("no home directory could be determined")]
    NoHomeDir,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Config {
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub priority: PriorityConfig,
    #[serde(default)]
    pub git: GitConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceConfig {
    pub root: PathBuf,
    pub group_by: GroupBy,
    pub todo_glob: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_glob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    /// One group per subdirectory, each owning its own todo file.
    #[default]
    Directory,
    /// One group per `## ` heading inside a single todo file.
    Heading,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorityConfig {
    pub source: PrioritySource,
    pub pattern: String,
}

impl Default for PriorityConfig {
    fn default() -> Self {
        Self {
            source: PrioritySource::None,
            pattern: "^P([0-3])".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrioritySource {
    /// Derived from the `## ` section heading.
    Heading,
    /// Derived from an inline marker on the item itself.
    Tag,
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub sync: Vec<Vec<String>>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&text)?;
        config.workspace.root = expand_tilde(&config.workspace.root)?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Expand a leading `~` to the user's home directory. Paths without one are
/// returned unchanged.
fn expand_tilde(path: &Path) -> Result<PathBuf, ConfigError> {
    let text = path.to_string_lossy();
    let Some(rest) = text.strip_prefix('~') else {
        return Ok(path.to_path_buf());
    };
    let home = directories::BaseDirs::new()
        .ok_or(ConfigError::NoHomeDir)?
        .home_dir()
        .to_path_buf();
    Ok(home.join(rest.trim_start_matches('/')))
}

/// `~/.config/mitodo/config.toml` on Linux and macOS.
pub fn default_config_path() -> Result<PathBuf, ConfigError> {
    let dirs = directories::ProjectDirs::from("", "", "mitodo").ok_or(ConfigError::NoHomeDir)?;
    Ok(dirs.config_dir().join("config.toml"))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib config 2>&1 | tail -20`
Expected: `test result: ok. 6 passed`

- [ ] **Step 5: Commit**

```bash
git add src/config src/main.rs
git commit -m "feat(config): add workspace, priority and git configuration"
```

---

### Task 5: Workspace detection and `mitodo init`

**Files:**
- Create: `src/store/detect.rs`
- Modify: `src/store/mod.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `Config`, `WorkspaceConfig`, `GroupBy`, `PriorityConfig`, `PrioritySource`, `GitConfig` from Task 4.
- Produces: `detect(root: &Path) -> Result<Detection, DetectError>` where `Detection { config: Config, notes: Vec<String> }` — `notes` holds the human-readable findings printed by `init`.

- [ ] **Step 1: Write the failing test**

Create `src/store/detect.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn dir_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for group in ["lefv", "jzlaw"] {
            let g = dir.path().join(group);
            fs::create_dir_all(g.join("_archive")).unwrap();
            fs::write(g.join("TODO.md"), "## P0 — Critical\n\n- [ ] a\n").unwrap();
            fs::write(g.join("notes.md"), "notes\n").unwrap();
        }
        dir
    }

    #[test]
    fn detects_directory_grouping() {
        let dir = dir_workspace();
        let found = detect(dir.path()).unwrap();
        assert_eq!(found.config.workspace.group_by, GroupBy::Directory);
        assert_eq!(found.config.workspace.todo_glob, "*/TODO.md");
    }

    #[test]
    fn detects_heading_priorities() {
        let dir = dir_workspace();
        let found = detect(dir.path()).unwrap();
        assert_eq!(found.config.priority.source, PrioritySource::Heading);
    }

    #[test]
    fn detects_sidecars() {
        let dir = dir_workspace();
        let found = detect(dir.path()).unwrap();
        assert_eq!(found.config.workspace.notes_glob.as_deref(), Some("*/notes.md"));
        assert_eq!(found.config.workspace.archive_dir.as_deref(), Some("_archive"));
    }

    #[test]
    fn detects_single_file_workspace_as_heading_grouped() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("TODO.md"), "## Work\n\n- [ ] a\n").unwrap();
        let found = detect(dir.path()).unwrap();
        assert_eq!(found.config.workspace.group_by, GroupBy::Heading);
        assert_eq!(found.config.workspace.todo_glob, "TODO.md");
    }

    #[test]
    fn priority_source_is_none_when_headings_do_not_match() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("TODO.md"), "## Shopping\n\n- [ ] milk\n").unwrap();
        let found = detect(dir.path()).unwrap();
        assert_eq!(found.config.priority.source, PrioritySource::None);
    }

    #[test]
    fn git_is_enabled_when_the_root_is_a_repository() {
        let dir = dir_workspace();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let found = detect(dir.path()).unwrap();
        assert!(found.config.git.enabled);
        assert_eq!(found.config.git.sync[0], vec!["add", "-A"]);
    }

    #[test]
    fn git_is_disabled_without_a_repository() {
        let dir = dir_workspace();
        assert!(!detect(dir.path()).unwrap().config.git.enabled);
    }

    #[test]
    fn an_empty_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect(dir.path()).is_err());
    }

    #[test]
    fn reports_findings_for_display() {
        let dir = dir_workspace();
        let found = detect(dir.path()).unwrap();
        assert!(found.notes.iter().any(|n| n.contains("2 group")));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod detect;` to `src/store/mod.rs` and re-export `pub use detect::{Detection, detect};`.

Run: `cargo test --lib detect 2>&1 | tail -20`
Expected: compile failure — `cannot find function detect`.

- [ ] **Step 3: Write the implementation**

Prepend to `src/store/detect.rs`:

```rust
use std::path::Path;

use crate::config::{
    Config, GitConfig, GroupBy, PriorityConfig, PrioritySource, WorkspaceConfig,
};

use super::model::Priority;

#[derive(thiserror::Error, Debug)]
pub enum DetectError {
    #[error("workspace could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("no TODO.md files found under {0}")]
    NoTodoFiles(String),
}

/// A detected workspace layout, plus human-readable findings for `init` to print.
#[derive(Debug, Clone)]
pub struct Detection {
    pub config: Config,
    pub notes: Vec<String>,
}

const DEFAULT_SYNC: [&[&str]; 4] = [
    &["add", "-A"],
    &["commit", "-m", "mitodo: sync"],
    &["pull", "--rebase"],
    &["push"],
];

pub fn detect(root: &Path) -> Result<Detection, DetectError> {
    let mut notes = Vec::new();

    // Group directories are subdirectories containing a TODO.md.
    let mut group_dirs: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("TODO.md").is_file() {
            group_dirs.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    group_dirs.sort();

    let root_todo = root.join("TODO.md");
    let (group_by, todo_glob) = if !group_dirs.is_empty() {
        notes.push(format!(
            "{} group directories, pattern */TODO.md",
            group_dirs.len()
        ));
        (GroupBy::Directory, "*/TODO.md".to_string())
    } else if root_todo.is_file() {
        notes.push("single TODO.md at the root, grouping by ## heading".to_string());
        (GroupBy::Heading, "TODO.md".to_string())
    } else {
        return Err(DetectError::NoTodoFiles(root.display().to_string()));
    };

    // Sample every discovered todo file's section headings.
    let sample_files: Vec<std::path::PathBuf> = if group_by == GroupBy::Directory {
        group_dirs.iter().map(|g| root.join(g).join("TODO.md")).collect()
    } else {
        vec![root_todo.clone()]
    };

    let mut heading_priorities = 0usize;
    for file in &sample_files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("## ")
                && Priority::from_heading(rest) != Priority::None
            {
                heading_priorities += 1;
            }
        }
    }

    let priority = if heading_priorities > 0 {
        notes.push(format!(
            "priorities from \"## \" headings ({heading_priorities} matched P0–P3)"
        ));
        PriorityConfig {
            source: PrioritySource::Heading,
            pattern: "^P([0-3])".to_string(),
        }
    } else {
        notes.push("no priority headings found; priorities disabled".to_string());
        PriorityConfig {
            source: PrioritySource::None,
            ..Default::default()
        }
    };

    // Sidecars are only meaningful for directory-grouped workspaces.
    let (notes_glob, archive_dir) = if group_by == GroupBy::Directory {
        let has_notes = group_dirs
            .iter()
            .any(|g| root.join(g).join("notes.md").is_file());
        let has_archive = group_dirs
            .iter()
            .any(|g| root.join(g).join("_archive").is_dir());
        if has_notes {
            notes.push("notes.md sidecars".to_string());
        }
        if has_archive {
            notes.push("_archive/ directories".to_string());
        }
        (
            has_notes.then(|| "*/notes.md".to_string()),
            has_archive.then(|| "_archive".to_string()),
        )
    } else {
        (None, None)
    };

    let git_enabled = root.join(".git").exists();
    if git_enabled {
        notes.push("git repository, sync enabled".to_string());
    }
    let git = GitConfig {
        enabled: git_enabled,
        sync: if git_enabled {
            DEFAULT_SYNC
                .iter()
                .map(|argv| argv.iter().map(|s| s.to_string()).collect())
                .collect()
        } else {
            Vec::new()
        },
    };

    Ok(Detection {
        config: Config {
            workspace: WorkspaceConfig {
                root: root.to_path_buf(),
                group_by,
                todo_glob,
                notes_glob,
                archive_dir,
            },
            priority,
            git,
        },
        notes,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib detect 2>&1 | tail -20`
Expected: `test result: ok. 9 passed`

- [ ] **Step 5: Wire up the `init` subcommand**

Replace `main` in `src/main.rs`:

```rust
mod cli;
mod config;
mod logging;
mod prelude;
mod store;

use std::path::PathBuf;

use clap::Parser;

use crate::cli::{CliArgs, Command};
use crate::prelude::*;

fn main() -> Result<()> {
    color_eyre::install()?;
    let args = CliArgs::parse();
    logging::init(args.log_file().as_deref(), *args.log_level());

    let config_path = match args.config_dir() {
        Some(dir) => dir.join("config.toml"),
        None => config::default_config_path()?,
    };

    match args.command() {
        Some(Command::Init { root, force }) => cmd_init(root, *force, &config_path),
        Some(Command::List) => Err(eyre!("`list` arrives in Task 8")),
        None => Err(eyre!("no subcommand given; try `mitodo --help`")),
    }
}

fn cmd_init(root: &PathBuf, force: bool, config_path: &std::path::Path) -> Result<()> {
    if config_path.exists() && !force {
        return Err(eyre!(
            "{} already exists; pass --force to overwrite",
            config_path.display()
        ));
    }

    let found = store::detect(root)?;
    println!("scanning {}...", root.display());
    for note in &found.notes {
        println!("  ✓ {note}");
    }
    found.config.save(config_path)?;
    println!("wrote {}", config_path.display());
    Ok(())
}
```

Add to `src/cli.rs`, inside the `CliArgs` getters block, nothing new — `command()` is generated by `getset`. Confirm the field is `command: Option<Command>` as written in Task 1.

- [ ] **Step 6: Run it against the real workspace**

Run:

```bash
cargo run -- --config-dir /tmp/mitodo-test init ~/repos/TODO
cat /tmp/mitodo-test/config.toml
```

Expected: reports 6 group directories, heading priorities, `notes.md` sidecars, `_archive/` directories, and git enabled. The written config matches the spec's example.

- [ ] **Step 7: Commit**

```bash
git add src/store/detect.rs src/store/mod.rs src/main.rs src/cli.rs
git commit -m "feat(store): autodetect workspace layout and add mitodo init"
```

---

### Task 6: Conflict-aware writer — toggle

**Files:**
- Create: `src/store/write.rs`
- Modify: `src/store/mod.rs`

**Interfaces:**
- Consumes: `Item` from Task 2.
- Produces:
  - `WriteError::{Io, Conflict, LineOutOfRange}`.
  - `toggle(path: &Path, line: usize, expected: &str, done: bool) -> Result<(), WriteError>` — `expected` is the full raw line as parsed; a mismatch is a `Conflict`.
  - Private helper `read_lines(path) -> Result<(Vec<String>, LineEnding, bool), WriteError>` and `write_lines(path, &[String], LineEnding, bool)`, preserving CRLF and trailing-newline presence.

- [ ] **Step 1: Write the failing test**

Create `src/store/write.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_temp(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TODO.md");
        fs::write(&path, contents).unwrap();
        (dir, path)
    }

    const DOC: &str = "## P0\n\n- [ ] first\n- [x] second\n";

    #[test]
    fn marks_an_item_done() {
        let (_d, path) = write_temp(DOC);
        toggle(&path, 2, "- [ ] first", true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "## P0\n\n- [x] first\n- [x] second\n");
    }

    #[test]
    fn marks_an_item_not_done() {
        let (_d, path) = write_temp(DOC);
        toggle(&path, 3, "- [x] second", false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "## P0\n\n- [ ] first\n- [ ] second\n");
    }

    #[test]
    fn leaves_every_other_byte_untouched() {
        let messy = "---\ntags: [a]\n---\n\n#  Title   \n\n\n- [ ] first\t\n\n- [x] second  \n";
        let (_d, path) = write_temp(messy);
        toggle(&path, 7, "- [ ] first\t", true).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        let expected = messy.replace("- [ ] first\t", "- [x] first\t");
        assert_eq!(after, expected, "only the toggled line may change");
    }

    #[test]
    fn preserves_a_missing_trailing_newline() {
        let (_d, path) = write_temp("- [ ] only");
        toggle(&path, 0, "- [ ] only", true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "- [x] only");
    }

    #[test]
    fn preserves_crlf_line_endings() {
        let (_d, path) = write_temp("## P0\r\n\r\n- [ ] first\r\n");
        toggle(&path, 2, "- [ ] first", true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "## P0\r\n\r\n- [x] first\r\n");
    }

    #[test]
    fn a_changed_line_is_a_conflict() {
        let (_d, path) = write_temp(DOC);
        // Another writer edited the line since we parsed it.
        fs::write(&path, "## P0\n\n- [ ] first, amended\n- [x] second\n").unwrap();
        let err = toggle(&path, 2, "- [ ] first", true).unwrap_err();
        assert!(matches!(err, WriteError::Conflict { .. }));
    }

    #[test]
    fn a_conflict_leaves_the_file_untouched() {
        let (_d, path) = write_temp(DOC);
        let amended = "## P0\n\n- [ ] first, amended\n- [x] second\n";
        fs::write(&path, amended).unwrap();
        let _ = toggle(&path, 2, "- [ ] first", true);
        assert_eq!(fs::read_to_string(&path).unwrap(), amended);
    }

    #[test]
    fn a_line_past_the_end_is_out_of_range() {
        let (_d, path) = write_temp(DOC);
        let err = toggle(&path, 99, "- [ ] first", true).unwrap_err();
        assert!(matches!(err, WriteError::LineOutOfRange { .. }));
    }

    #[test]
    fn toggling_an_already_done_item_is_a_no_op() {
        let (_d, path) = write_temp(DOC);
        toggle(&path, 3, "- [x] second", true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), DOC);
    }

    #[test]
    fn preserves_indentation_of_nested_items() {
        let (_d, path) = write_temp("- [ ] parent\n  - [ ] child\n");
        toggle(&path, 1, "  - [ ] child", true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "- [ ] parent\n  - [x] child\n");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod write;` to `src/store/mod.rs` and re-export `pub use write::{WriteError, toggle};`.

Run: `cargo test --lib write 2>&1 | tail -20`
Expected: compile failure — `cannot find function toggle`.

- [ ] **Step 3: Write the implementation**

Prepend to `src/store/write.rs`:

```rust
use std::path::Path;

#[derive(thiserror::Error, Debug)]
pub enum WriteError {
    #[error("file could not be read or written: {0}")]
    Io(#[from] std::io::Error),
    #[error("line {line} of {file} changed on disk (expected {expected:?}, found {found:?})")]
    Conflict {
        file: String,
        line: usize,
        expected: String,
        found: String,
    },
    #[error("line {line} is past the end of {file} ({len} lines)")]
    LineOutOfRange { file: String, line: usize, len: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }
}

/// Read a file into lines, remembering how to put it back together verbatim.
///
/// Returns the lines without their terminators, the dominant line ending, and
/// whether the file ended with a terminator.
fn read_lines(path: &Path) -> Result<(Vec<String>, LineEnding, bool), WriteError> {
    let text = std::fs::read_to_string(path)?;
    let ending = if text.contains("\r\n") {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    };
    let trailing = text.ends_with('\n');
    let body = if trailing {
        text.strip_suffix('\n').unwrap_or(&text)
    } else {
        &text
    };
    let body = body.strip_suffix('\r').unwrap_or(body);
    let lines: Vec<String> = if body.is_empty() && trailing {
        vec![String::new()]
    } else {
        body.split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
            .collect()
    };
    Ok((lines, ending, trailing))
}

/// Write lines back atomically: temp file in the same directory, then rename.
fn write_lines(
    path: &Path,
    lines: &[String],
    ending: LineEnding,
    trailing: bool,
) -> Result<(), WriteError> {
    let mut out = lines.join(ending.as_str());
    if trailing {
        out.push_str(ending.as_str());
    }

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".{}.mitodo.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Fetch a line, verifying it still holds what the caller parsed.
///
/// This is the guard that makes concurrent editing by other tools safe.
fn verify(
    path: &Path,
    lines: &[String],
    line: usize,
    expected: &str,
) -> Result<(), WriteError> {
    let found = lines.get(line).ok_or_else(|| WriteError::LineOutOfRange {
        file: path.display().to_string(),
        line,
        len: lines.len(),
    })?;
    if found != expected {
        return Err(WriteError::Conflict {
            file: path.display().to_string(),
            line,
            expected: expected.to_string(),
            found: found.clone(),
        });
    }
    Ok(())
}

/// Set the checkbox on `line` to `done`, leaving every other byte alone.
pub fn toggle(path: &Path, line: usize, expected: &str, done: bool) -> Result<(), WriteError> {
    let (mut lines, ending, trailing) = read_lines(path)?;
    verify(path, &lines, line, expected)?;

    let current = &lines[line];
    let replaced = if done {
        current.replacen("- [ ]", "- [x]", 1)
    } else {
        let once = current.replacen("- [x]", "- [ ]", 1);
        if once == *current {
            current.replacen("- [X]", "- [ ]", 1)
        } else {
            once
        }
    };

    if replaced == *current {
        return Ok(());
    }
    lines[line] = replaced;
    write_lines(path, &lines, ending, trailing)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib write 2>&1 | tail -20`
Expected: `test result: ok. 10 passed`

- [ ] **Step 5: Commit**

```bash
git add src/store/write.rs src/store/mod.rs
git commit -m "feat(store): conflict-aware toggle with byte-exact preservation

Re-reads and verifies the target line before writing, so concurrent edits
by mcli, todos-mcp or Claude surface as Conflict rather than being
clobbered. Preserves CRLF and trailing-newline state."
```

---

### Task 7: Writer — edit, add, delete, description

**Files:**
- Modify: `src/store/write.rs`, `src/store/mod.rs`

**Interfaces:**
- Consumes: `read_lines`, `write_lines`, `verify`, `WriteError` from Task 6.
- Produces, all `-> Result<(), WriteError>` and all taking `expected: &str` for the verify guard:
  - `edit_text(path, line, expected, new_text)`
  - `add_item(path, after_line, expected_after, indent, text)` — inserts below any description block belonging to `after_line`
  - `delete_item(path, line, expected)` — removes the item and its description block
  - `set_description(path, line, expected, description)` — replaces the block; an empty description removes it

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `src/store/write.rs`:

```rust
    const NESTED: &str = "## P0\n\n- [ ] parent\n  > note one\n  > note two\n- [x] sibling\n";

    #[test]
    fn edits_item_text_and_keeps_its_state() {
        let (_d, path) = write_temp(DOC);
        edit_text(&path, 3, "- [x] second", "second, revised").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n- [x] second, revised\n"
        );
    }

    #[test]
    fn edits_preserve_indentation() {
        let (_d, path) = write_temp("- [ ] parent\n  - [ ] child\n");
        edit_text(&path, 1, "  - [ ] child", "renamed").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "- [ ] parent\n  - [ ] renamed\n");
    }

    #[test]
    fn editing_a_drifted_line_is_a_conflict() {
        let (_d, path) = write_temp(DOC);
        let err = edit_text(&path, 2, "- [ ] not what is there", "x").unwrap_err();
        assert!(matches!(err, WriteError::Conflict { .. }));
    }

    #[test]
    fn adds_an_item_below_the_target() {
        let (_d, path) = write_temp(DOC);
        add_item(&path, 2, "- [ ] first", 0, "inserted").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n- [ ] inserted\n- [x] second\n"
        );
    }

    #[test]
    fn adds_below_an_existing_description_block() {
        let (_d, path) = write_temp(NESTED);
        add_item(&path, 2, "- [ ] parent", 0, "inserted").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] parent\n  > note one\n  > note two\n- [ ] inserted\n- [x] sibling\n"
        );
    }

    #[test]
    fn adds_a_nested_child_with_indentation() {
        let (_d, path) = write_temp(DOC);
        add_item(&path, 2, "- [ ] first", 2, "child").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n  - [ ] child\n- [x] second\n"
        );
    }

    #[test]
    fn deletes_an_item() {
        let (_d, path) = write_temp(DOC);
        delete_item(&path, 2, "- [ ] first").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "## P0\n\n- [x] second\n");
    }

    #[test]
    fn deleting_takes_the_description_block_with_it() {
        let (_d, path) = write_temp(NESTED);
        delete_item(&path, 2, "- [ ] parent").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "## P0\n\n- [x] sibling\n");
    }

    #[test]
    fn sets_a_description_where_there_was_none() {
        let (_d, path) = write_temp(DOC);
        set_description(&path, 2, "- [ ] first", "a note").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n  > a note\n- [x] second\n"
        );
    }

    #[test]
    fn replaces_an_existing_description() {
        let (_d, path) = write_temp(NESTED);
        set_description(&path, 2, "- [ ] parent", "only note").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] parent\n  > only note\n- [x] sibling\n"
        );
    }

    #[test]
    fn an_empty_description_removes_the_block() {
        let (_d, path) = write_temp(NESTED);
        set_description(&path, 2, "- [ ] parent", "").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "## P0\n\n- [ ] parent\n- [x] sibling\n");
    }

    #[test]
    fn a_multi_line_description_becomes_multiple_blockquote_lines() {
        let (_d, path) = write_temp(DOC);
        set_description(&path, 2, "- [ ] first", "one\ntwo").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n  > one\n  > two\n- [x] second\n"
        );
    }

    #[test]
    fn descriptions_indent_relative_to_their_item() {
        let (_d, path) = write_temp("- [ ] parent\n  - [ ] child\n");
        set_description(&path, 1, "  - [ ] child", "note").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "- [ ] parent\n  - [ ] child\n    > note\n"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib write 2>&1 | tail -20`
Expected: compile failure — `cannot find function edit_text`.

- [ ] **Step 3: Write the implementation**

Append to the implementation half of `src/store/write.rs`, above the test module:

```rust
/// True if `line` is a description blockquote line, e.g. `"  > note"`.
fn is_description(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('>') && line.len() > trimmed.len()
}

/// Index one past the last description line belonging to the item on `line`.
fn description_end(lines: &[String], line: usize) -> usize {
    let mut end = line + 1;
    while end < lines.len() && is_description(&lines[end]) {
        end += 1;
    }
    end
}

/// Leading whitespace of a line, as a string slice.
fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Replace the text of a checkbox line, preserving indentation and done state.
pub fn edit_text(
    path: &Path,
    line: usize,
    expected: &str,
    new_text: &str,
) -> Result<(), WriteError> {
    let (mut lines, ending, trailing) = read_lines(path)?;
    verify(path, &lines, line, expected)?;

    // Everything up to and including the "] " marker — indentation, bullet and
    // done state — is preserved verbatim; only the text after it changes.
    let marker_end = match lines[line].find("] ") {
        Some(idx) => idx + 2,
        None => return Ok(()),
    };
    let mut replaced = lines[line][..marker_end].to_string();
    replaced.push_str(new_text);
    lines[line] = replaced;

    write_lines(path, &lines, ending, trailing)
}

/// Insert a new unchecked item after the item on `after_line`, below any
/// description block that item owns.
pub fn add_item(
    path: &Path,
    after_line: usize,
    expected_after: &str,
    indent: usize,
    text: &str,
) -> Result<(), WriteError> {
    let (mut lines, ending, trailing) = read_lines(path)?;
    verify(path, &lines, after_line, expected_after)?;

    let insert_at = description_end(&lines, after_line);
    lines.insert(insert_at, format!("{}- [ ] {}", " ".repeat(indent), text));
    write_lines(path, &lines, ending, trailing)
}

/// Remove a checkbox line and any description block beneath it.
pub fn delete_item(path: &Path, line: usize, expected: &str) -> Result<(), WriteError> {
    let (mut lines, ending, trailing) = read_lines(path)?;
    verify(path, &lines, line, expected)?;

    let end = description_end(&lines, line);
    lines.drain(line..end);
    write_lines(path, &lines, ending, trailing)
}

/// Replace the description block beneath an item. An empty or blank
/// description removes the block entirely.
pub fn set_description(
    path: &Path,
    line: usize,
    expected: &str,
    description: &str,
) -> Result<(), WriteError> {
    let (mut lines, ending, trailing) = read_lines(path)?;
    verify(path, &lines, line, expected)?;

    let end = description_end(&lines, line);
    lines.drain(line + 1..end);

    let description = description.trim();
    if !description.is_empty() {
        let prefix = format!("{}  > ", leading_whitespace(&lines[line]));
        for (offset, note) in description.lines().enumerate() {
            lines.insert(line + 1 + offset, format!("{prefix}{note}"));
        }
    }

    write_lines(path, &lines, ending, trailing)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib write 2>&1 | tail -20`
Expected: `test result: ok. 23 passed`

- [ ] **Step 5: Update the module re-exports**

In `src/store/mod.rs`:

```rust
pub use write::{WriteError, add_item, delete_item, edit_text, set_description, toggle};
```

- [ ] **Step 6: Commit**

```bash
git add src/store/write.rs src/store/mod.rs
git commit -m "feat(store): add edit, insert, delete and description writes"
```

---

### Task 8: Workspace loading and `mitodo list`

**Files:**
- Create: `src/store/workspace.rs`
- Modify: `src/store/mod.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `Config`, `GroupBy` (Task 4); `parse_todo_file` (Task 3); `Group`, `Item` (Task 2).
- Produces:
  - `Workspace { root: PathBuf, groups: Vec<Group>, items: Vec<Item> }`.
  - `Workspace::load(config: &Config) -> Result<Workspace, LoadError>`.
  - `Workspace::items_for_group(&self, group: &str) -> Vec<&Item>`.
  - `Workspace::open_count(&self) -> usize`.

- [ ] **Step 1: Write the failing test**

Create `src/store/workspace.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, GroupBy, WorkspaceConfig};
    use std::fs;

    fn workspace_fixture() -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        for (group, body) in [
            ("lefv", "## P0 — Critical\n\n- [ ] alpha\n- [x] beta\n"),
            ("jzlaw", "## P1 — High\n\n- [ ] gamma\n"),
        ] {
            let g = dir.path().join(group);
            fs::create_dir_all(&g).unwrap();
            fs::write(g.join("TODO.md"), body).unwrap();
        }
        let config = Config {
            workspace: WorkspaceConfig {
                root: dir.path().to_path_buf(),
                group_by: GroupBy::Directory,
                todo_glob: "*/TODO.md".to_string(),
                notes_glob: None,
                archive_dir: None,
            },
            ..Default::default()
        };
        (dir, config)
    }

    #[test]
    fn loads_every_group() {
        let (_d, config) = workspace_fixture();
        let ws = Workspace::load(&config).unwrap();
        let names: Vec<&str> = ws.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["jzlaw", "lefv"], "groups are sorted by name");
    }

    #[test]
    fn loads_items_from_every_group() {
        let (_d, config) = workspace_fixture();
        assert_eq!(Workspace::load(&config).unwrap().items.len(), 3);
    }

    #[test]
    fn ids_are_unique_across_groups() {
        let (_d, config) = workspace_fixture();
        let ws = Workspace::load(&config).unwrap();
        let mut ids: Vec<&str> = ws.items.iter().map(|i| i.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "no duplicate item ids");
    }

    #[test]
    fn filters_items_by_group() {
        let (_d, config) = workspace_fixture();
        let ws = Workspace::load(&config).unwrap();
        assert_eq!(ws.items_for_group("lefv").len(), 2);
        assert_eq!(ws.items_for_group("jzlaw").len(), 1);
    }

    #[test]
    fn counts_only_open_items() {
        let (_d, config) = workspace_fixture();
        assert_eq!(Workspace::load(&config).unwrap().open_count(), 2);
    }

    #[test]
    fn a_missing_root_is_an_error() {
        let (_d, mut config) = workspace_fixture();
        config.workspace.root = "/nonexistent/workspace".into();
        assert!(Workspace::load(&config).is_err());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod workspace;` plus `pub use workspace::Workspace;` to `src/store/mod.rs`.

Run: `cargo test --lib workspace 2>&1 | tail -20`
Expected: compile failure — `cannot find type Workspace`.

- [ ] **Step 3: Write the implementation**

Prepend to `src/store/workspace.rs`:

```rust
use std::path::{Path, PathBuf};

use crate::config::{Config, GroupBy};

use super::model::{Group, Item};
use super::parse::parse_todo_file;

#[derive(thiserror::Error, Debug)]
pub enum LoadError {
    #[error("workspace root {0} could not be read")]
    Root(String),
    #[error("todo file could not be read: {0}")]
    Io(#[from] std::io::Error),
}

/// Every group and item currently on disk. Rebuilt from scratch on reload —
/// parsing a few hundred items costs microseconds, so there is no index.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    pub root: PathBuf,
    pub groups: Vec<Group>,
    pub items: Vec<Item>,
}

impl Workspace {
    pub fn load(config: &Config) -> Result<Workspace, LoadError> {
        let root = &config.workspace.root;
        if !root.is_dir() {
            return Err(LoadError::Root(root.display().to_string()));
        }

        let groups = match config.workspace.group_by {
            GroupBy::Directory => directory_groups(root, config)?,
            GroupBy::Heading => vec![Group {
                name: root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "todo".to_string()),
                todo_file: root.join("TODO.md"),
                notes_file: None,
                archive_dir: None,
            }],
        };

        let mut items = Vec::new();
        for group in &groups {
            let Ok(source) = std::fs::read_to_string(&group.todo_file) else {
                continue;
            };
            let file_rel = group
                .todo_file
                .strip_prefix(root)
                .unwrap_or(&group.todo_file)
                .to_string_lossy()
                .to_string();
            items.extend(parse_todo_file(&group.todo_file, &file_rel, &source));
        }

        Ok(Workspace {
            root: root.clone(),
            groups,
            items,
        })
    }

    /// Items whose file belongs to the named group.
    pub fn items_for_group(&self, group: &str) -> Vec<&Item> {
        let Some(g) = self.groups.iter().find(|g| g.name == group) else {
            return Vec::new();
        };
        self.items
            .iter()
            .filter(|i| i.file == g.todo_file)
            .collect()
    }

    pub fn open_count(&self) -> usize {
        self.items.iter().filter(|i| !i.done).count()
    }
}

/// One group per subdirectory holding a TODO.md, sorted by name.
fn directory_groups(root: &Path, config: &Config) -> Result<Vec<Group>, LoadError> {
    let mut groups = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        let todo_file = dir.join("TODO.md");
        if !todo_file.is_file() {
            continue;
        }

        let notes_file = config
            .workspace
            .notes_glob
            .as_ref()
            .map(|_| dir.join("notes.md"))
            .filter(|p| p.is_file());
        let archive_dir = config
            .workspace
            .archive_dir
            .as_ref()
            .map(|name| dir.join(name))
            .filter(|p| p.is_dir());

        groups.push(Group {
            name: entry.file_name().to_string_lossy().to_string(),
            todo_file,
            notes_file,
            archive_dir,
        });
    }
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(groups)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib workspace 2>&1 | tail -20`
Expected: `test result: ok. 6 passed`

- [ ] **Step 5: Wire up the `list` subcommand**

Replace the `Command::List` arm in `src/main.rs`:

```rust
        Some(Command::List) => cmd_list(&config_path),
```

And add:

```rust
fn cmd_list(config_path: &std::path::Path) -> Result<()> {
    let config = config::Config::load(config_path)?;
    let workspace = store::Workspace::load(&config)?;

    for group in &workspace.groups {
        let items = workspace.items_for_group(&group.name);
        let open = items.iter().filter(|i| !i.done).count();
        println!("\n{} ({} open / {} total)", group.name, open, items.len());
        for item in items {
            let box_ = if item.done { "x" } else { " " };
            let indent = " ".repeat(item.indent);
            println!(
                "  {indent}[{box_}] {:<3} {}",
                item.priority.as_str(),
                item.text
            );
        }
    }
    println!(
        "\n{} open across {} groups",
        workspace.open_count(),
        workspace.groups.len()
    );
    Ok(())
}
```

- [ ] **Step 6: Run it against the real workspace**

Run:

```bash
cargo run -- --config-dir /tmp/mitodo-test list | head -40
```

Expected: every group from `~/repos/TODO` with its open counts, priorities, and correctly indented nested items.

- [ ] **Step 7: Run the whole suite**

Run: `cargo test 2>&1 | tail -10`
Expected: all tests pass — 5 model, 11 parse, 6 config, 9 detect, 23 write, 6 workspace = 60.

- [ ] **Step 8: Commit**

```bash
git add src/store/workspace.rs src/store/mod.rs src/main.rs
git commit -m "feat(store): load a whole workspace and add mitodo list

Completes phase 1: mitodo now detects, parses, lists and safely writes a
real markdown todo workspace from the command line."
```

---

## What phase 1 delivers

A `mitodo` binary that:

- detects a workspace layout and writes an explicit config (`mitodo init <root>`)
- loads every group and item from that workspace (`mitodo list`)
- parses frontmatter, sections, headings, nesting, and description blockquotes
- writes toggles, edits, insertions, deletions and descriptions back to markdown without disturbing a single untouched byte, and refuses to write when another tool has changed the line

## Phases 2 and 3

- **Phase 2 — TUI.** Move `query/`, `config/`, `input/`, `messages/`, `utils/` and the three list panes out of `src/_port/`, retargeting them at `crate::store`. Delivers the three-pane UI, vim keybindings, the query vocabulary swap, and live reload via `store/watch.rs`.
- **Phase 3 — agent, git, polish.** `agent/` with the four verbs and the review-diff modal, `git.rs` sync, the chyron vocabulary swap, README, `NOTICE`, and packaging.

Each gets its own plan written after the preceding phase lands.
