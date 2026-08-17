# `mitodo self mcp` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `mitodo self mcp setup` registers mitodo's MCP server with every supported client on the machine, so nobody hand-writes `--mcp-config` JSON; `status` and `remove` complete the trio.

**Architecture:** Where a client ships its own `mcp add/get/remove` (claude, codex) mitodo shells out to it, so the client owns its config format. Claude Desktop has no CLI, so it gets one merged key written through a temp file and an atomic rename. Registration is always the absolute `current_exe()` path plus `["mcp-server"]`.

**Tech Stack:** Rust 2024, `std::process::Command` for delegation, `serde_json` for the one JSON merge, `tempfile` for the atomic write. All three already in the tree.

## Global Constraints

- Spec: [docs/superpowers/specs/2026-08-17-self-mcp-bootstrap-design.md](../specs/2026-08-17-self-mcp-bootstrap-design.md).
- Branch: `feat/self-mcp-bootstrap`, cut from `main`. Never commit to `main`.
- **Register the absolute path**, never bare `mitodo`: Claude Desktop is launched by the GUI and inherits no shell `PATH`, and `~/.cargo/bin` is not on the default `PATH` anyway.
- **Never parse a config a client CLI can write.** claude and codex are driven through their own subcommands; only Claude Desktop, which has no CLI, is edited as a file.
- **The Claude Desktop merge touches exactly one key** (`mcpServers.mitodo`) and preserves every other byte: read → modify → temp file in the same directory → atomic rename. Refuse to write anything we could not parse.
- **One failing target never aborts another.** Exit `0` all good, `1` a found target failed, `2` no supported client found at all.
- **`--dry-run` performs no write and spawns no subprocess.**
- Comments: minimum possible, one line each, stating a hidden constraint only. Never restate what the code does.
- Test names are sentences describing behaviour. No `#[ignore]`, no skipped tests. No test may invoke a real client CLI or touch a real client config.
- `cargo test` green, `cargo clippy --all-targets --all-features -- -D warnings` silent, `cargo fmt --check` clean at every commit. CI runs a newer clippy than local; use those exact flags.
- No new dependencies.
- Commit after every task with a Conventional Commits subject. No AI attribution.

### The registration protocol, established by probing the real CLIs

Both were tested with a throwaway server name. They disagree, and both exit `0` either way, so **exit codes cannot be used to detect state**:

| | `add` on a name that already exists |
|---|---|
| `claude` | refuses — `MCP server zzprobe already exists in user config` — and **keeps the old value** |
| `codex` | **overwrites** with the new value |

One uniform strategy therefore covers both, with no per-client special casing:

```
 get succeeds, command == ours   → no-op          "already current"
 get succeeds, command != ours   → remove, add    "re-pointed from <old>"
 get fails (exit non-zero)       → add            "registered"
```

`get`'s output differs in case and layout between the two (`Command: uv` versus
`command: /abs/path`), so the parse is case-insensitive on a `command:` prefix.
`claude` needs `--scope user`; its default `local` registers only for the current
directory.

---

### Task 1: Targets and detection

**Files:**
- Create: `src/selfcfg/target.rs`
- Create: `src/selfcfg/mod.rs` (module declarations only in this task)
- Modify: `src/main.rs` (module list)

**Interfaces:**
- Consumes: nothing.
- Produces: `Kind { Delegated { cli: &'static str, scope: Option<&'static str> }, DesktopJson { path: PathBuf } }`; `Target { name: &'static str, kind: Kind }`; `detect() -> Vec<Target>`; `detect_in(has_cli: &dyn Fn(&str) -> bool, desktop: Option<PathBuf>) -> Vec<Target>`; `unsupported() -> Vec<(&'static str, &'static str)>`; `desktop_config_path() -> Option<PathBuf>`.

- [ ] **Step 1: Write the failing tests**

Create `src/selfcfg/target.rs`:

```rust
//! What counts as a client mitodo can register itself with.
//!
//! A client that ships its own `mcp add` is driven through it, so the client
//! owns its config format. Only a client with no CLI is edited as a file.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Drive the client's own `mcp` subcommands.
    Delegated {
        cli: &'static str,
        /// `claude` defaults to a per-directory scope; a todo server is not
        /// directory-scoped.
        scope: Option<&'static str>,
    },
    /// No CLI exists: merge one key into this JSON file.
    DesktopJson { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub name: &'static str,
    pub kind: Kind,
}

/// Clients seen on this machine that mitodo deliberately does not touch.
pub fn unsupported() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "opencode",
            "`opencode mcp add` takes no arguments, and there is no remove",
        ),
        (
            "zed",
            "settings.json is JSONC; rewriting it would delete your comments",
        ),
    ]
}

/// Where Claude Desktop keeps its config, when the app is installed.
pub fn desktop_config_path() -> Option<PathBuf> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    let path = home
        .join("Library")
        .join("Application Support")
        .join("Claude")
        .join("claude_desktop_config.json");
    path.parent().filter(|dir| dir.is_dir()).map(|_| path)
}

pub fn detect() -> Vec<Target> {
    detect_in(&|cli| which(cli), desktop_config_path())
}

/// Detection with its two lookups injected, so tests never depend on what is
/// installed on the machine running them.
pub fn detect_in(has_cli: &dyn Fn(&str) -> bool, desktop: Option<PathBuf>) -> Vec<Target> {
    let mut found = Vec::new();
    if has_cli("claude") {
        found.push(Target {
            name: "claude",
            kind: Kind::Delegated {
                cli: "claude",
                scope: Some("user"),
            },
        });
    }
    if has_cli("codex") {
        found.push(Target {
            name: "codex",
            kind: Kind::Delegated {
                cli: "codex",
                scope: None,
            },
        });
    }
    if let Some(path) = desktop {
        found.push(Target {
            name: "claude-desktop",
            kind: Kind::DesktopJson { path },
        });
    }
    found
}

/// Whether a command resolves on PATH.
fn which(cli: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(cli);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cli_on_path_becomes_a_delegated_target() {
        let found = detect_in(&|cli| cli == "claude", None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "claude");
        assert_eq!(
            found[0].kind,
            Kind::Delegated {
                cli: "claude",
                scope: Some("user")
            },
            "claude's default scope is per-directory, so user is passed explicitly"
        );
    }

    #[test]
    fn codex_is_delegated_without_a_scope_flag() {
        let found = detect_in(&|cli| cli == "codex", None);
        assert_eq!(
            found[0].kind,
            Kind::Delegated {
                cli: "codex",
                scope: None
            }
        );
    }

    #[test]
    fn an_absent_cli_is_not_a_target() {
        assert!(detect_in(&|_| false, None).is_empty());
    }

    #[test]
    fn a_present_desktop_config_becomes_a_file_target() {
        let found = detect_in(&|_| false, Some(PathBuf::from("/tmp/x.json")));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "claude-desktop");
        assert_eq!(
            found[0].kind,
            Kind::DesktopJson {
                path: PathBuf::from("/tmp/x.json")
            }
        );
    }

    #[test]
    fn everything_present_yields_every_target() {
        let found = detect_in(&|_| true, Some(PathBuf::from("/tmp/x.json")));
        let names: Vec<&str> = found.iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["claude", "codex", "claude-desktop"]);
    }

    // Silence would read as "not installed"; these are seen and skipped.
    #[test]
    fn the_unsupported_clients_are_named_with_reasons() {
        let skipped = unsupported();
        let names: Vec<&str> = skipped.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"opencode"));
        assert!(names.contains(&"zed"));
        assert!(
            skipped.iter().all(|(_, why)| why.len() > 20),
            "a reason a person can act on"
        );
    }
}
```

Create `src/selfcfg/mod.rs` containing only:

```rust
//! Registering mitodo with the MCP clients on this machine.

pub mod target;
```

Add `mod selfcfg;` to `src/main.rs`'s module list, after `mod query;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet selfcfg 2>&1 | tail -6`
Expected: `file not found for module selfcfg`, then compile errors once declared.

- [ ] **Step 3: Make them pass**

The implementation is in step 1's file. If `-D warnings` rejects unused items,
do **not** add a module-wide `#![allow(dead_code)]` — add
`#[allow(dead_code)]` to the single item that is not yet consumed, so it goes
stale visibly when Task 3 consumes it.

- [ ] **Step 4: Run the tests**

Run: `cargo test --quiet selfcfg 2>&1 | tail -4 && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3`
Expected: six tests pass, clippy silent.

- [ ] **Step 5: Sanity-check detection against this machine**

Run: `cargo run --quiet -- self mcp status 2>&1 | head` — this will fail until
Task 4 wires the subcommand. Instead confirm the detection logic sees what the
survey found:

```bash
cargo test --quiet selfcfg 2>&1 | tail -3
ls "$HOME/Library/Application Support/Claude/claude_desktop_config.json"
command -v claude codex
```
Expected: the desktop config exists and both CLIs resolve, so `detect()` on this
machine will return all three targets.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/selfcfg src/main.rs
git commit -m "feat(selfcfg): detect the MCP clients worth registering with"
```

---

### Task 2: The Claude Desktop JSON merge

**Files:**
- Create: `src/selfcfg/desktop.rs`
- Modify: `src/selfcfg/mod.rs` (declare it)

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `desktop::Entry { command: String, args: Vec<String> }`; `desktop::read_entry(path, name) -> Result<Option<Entry>, String>`; `desktop::merge(path, name, entry) -> Result<(), String>`; `desktop::remove(path, name) -> Result<bool, String>`.

- [ ] **Step 1: Write the failing tests**

Create `src/selfcfg/desktop.rs`:

```rust
//! The one client with no CLI, so the one config mitodo edits itself.
//!
//! Exactly one key is touched and every other byte is preserved: read, modify,
//! write a temp file in the same directory, rename over the original. The same
//! discipline `store::write` applies to markdown, for the same reason — it is
//! someone else's file.

use std::path::Path;

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub command: String,
    pub args: Vec<String>,
}

fn load(path: &Path) -> Result<Value, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&raw).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))
}

fn servers_of(root: &Value) -> Result<Map<String, Value>, String> {
    match root.get("mcpServers") {
        None => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => Err("mcpServers is not an object; refusing to rewrite it".to_string()),
    }
}

/// What `name` is registered as, if it is.
pub fn read_entry(path: &Path, name: &str) -> Result<Option<Entry>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let servers = servers_of(&load(path)?)?;
    Ok(servers.get(name).map(|entry| Entry {
        command: entry
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string(),
        args: entry
            .get("args")
            .and_then(|a| a.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|a| a.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    }))
}

/// Write `name` into the file's `mcpServers`, leaving everything else alone.
pub fn merge(path: &Path, name: &str, entry: &Entry) -> Result<(), String> {
    let mut root = if path.is_file() { load(path)? } else { json!({}) };
    let mut servers = servers_of(&root)?;
    servers.insert(
        name.to_string(),
        json!({"command": entry.command, "args": entry.args}),
    );
    root
        .as_object_mut()
        .ok_or_else(|| format!("{} is not a JSON object", path.display()))?
        .insert("mcpServers".to_string(), Value::Object(servers));
    write_atomically(path, &root)
}

/// Drop `name`. `Ok(false)` means it was not there.
pub fn remove(path: &Path, name: &str) -> Result<bool, String> {
    if !path.is_file() {
        return Ok(false);
    }
    let mut root = load(path)?;
    let mut servers = servers_of(&root)?;
    if servers.remove(name).is_none() {
        return Ok(false);
    }
    root
        .as_object_mut()
        .ok_or_else(|| format!("{} is not a JSON object", path.display()))?
        .insert("mcpServers".to_string(), Value::Object(servers));
    write_atomically(path, &root)?;
    Ok(true)
}

/// The temp file goes in the same directory, so the rename stays on one
/// filesystem and is therefore atomic.
fn write_atomically(path: &Path, root: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;

    let mut text = serde_json::to_string_pretty(root).map_err(|e| e.to_string())?;
    text.push('\n');

    let temp = tempfile::Builder::new()
        .prefix(".mitodo-")
        .suffix(".json")
        .tempfile_in(parent)
        .map_err(|e| format!("{}: {e}", parent.display()))?;
    std::fs::write(temp.path(), text).map_err(|e| format!("{}: {e}", temp.path().display()))?;
    temp.persist(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours() -> Entry {
        Entry {
            command: "/abs/mitodo".to_string(),
            args: vec!["mcp-server".to_string()],
        }
    }

    fn config(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        std::fs::write(&path, body).unwrap();
        (dir, path)
    }

    const EXISTING: &str = r#"{
  "mcpServers": {
    "gmail-multi": {
      "command": "/opt/gmail",
      "args": []
    }
  },
  "preferences": {
    "theme": "dark"
  }
}
"#;

    #[test]
    fn merging_leaves_the_other_servers_and_keys_intact() {
        let (_dir, path) = config(EXISTING);
        merge(&path, "mitodo", &ours()).unwrap();

        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["mcpServers"]["gmail-multi"]["command"], "/opt/gmail");
        assert_eq!(root["preferences"]["theme"], "dark", "unrelated keys survive");
        assert_eq!(root["mcpServers"]["mitodo"]["command"], "/abs/mitodo");
        assert_eq!(root["mcpServers"]["mitodo"]["args"][0], "mcp-server");
    }

    #[test]
    fn merging_twice_changes_nothing_the_second_time() {
        let (_dir, path) = config(EXISTING);
        merge(&path, "mitodo", &ours()).unwrap();
        let once = std::fs::read_to_string(&path).unwrap();
        merge(&path, "mitodo", &ours()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once);
    }

    #[test]
    fn a_stale_command_is_replaced() {
        let (_dir, path) = config(EXISTING);
        merge(
            &path,
            "mitodo",
            &Entry {
                command: "/old/mitodo".to_string(),
                args: vec!["mcp-server".to_string()],
            },
        )
        .unwrap();
        merge(&path, "mitodo", &ours()).unwrap();
        assert_eq!(
            read_entry(&path, "mitodo").unwrap().unwrap().command,
            "/abs/mitodo"
        );
    }

    #[test]
    fn read_entry_reports_absence_and_presence() {
        let (_dir, path) = config(EXISTING);
        assert!(read_entry(&path, "mitodo").unwrap().is_none());
        merge(&path, "mitodo", &ours()).unwrap();
        assert_eq!(read_entry(&path, "mitodo").unwrap(), Some(ours()));
    }

    #[test]
    fn removing_drops_only_our_entry() {
        let (_dir, path) = config(EXISTING);
        merge(&path, "mitodo", &ours()).unwrap();
        assert!(remove(&path, "mitodo").unwrap());

        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(root["mcpServers"].get("mitodo").is_none());
        assert_eq!(root["mcpServers"]["gmail-multi"]["command"], "/opt/gmail");
    }

    #[test]
    fn removing_something_absent_reports_false_and_writes_nothing() {
        let (_dir, path) = config(EXISTING);
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(!remove(&path, "mitodo").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    // Never overwrite a file we could not read.
    #[test]
    fn unparseable_json_is_refused_and_the_file_is_untouched() {
        let (_dir, path) = config("{ this is not json");
        let before = std::fs::read_to_string(&path).unwrap();
        let failure = merge(&path, "mitodo", &ours()).unwrap_err();
        assert!(failure.contains("not valid JSON"), "{failure}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn a_non_object_mcpservers_is_refused() {
        let (_dir, path) = config(r#"{"mcpServers": ["not", "an", "object"]}"#);
        let failure = merge(&path, "mitodo", &ours()).unwrap_err();
        assert!(failure.contains("not an object"), "{failure}");
    }

    #[test]
    fn a_missing_mcpservers_key_is_created() {
        let (_dir, path) = config(r#"{"preferences": {"theme": "light"}}"#);
        merge(&path, "mitodo", &ours()).unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["mcpServers"]["mitodo"]["command"], "/abs/mitodo");
        assert_eq!(root["preferences"]["theme"], "light");
    }

    #[test]
    fn an_empty_file_is_treated_as_an_empty_config() {
        let (_dir, path) = config("");
        merge(&path, "mitodo", &ours()).unwrap();
        assert!(read_entry(&path, "mitodo").unwrap().is_some());
    }

    #[test]
    fn a_file_that_does_not_exist_yet_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        merge(&path, "mitodo", &ours()).unwrap();
        assert!(path.is_file());
        assert_eq!(read_entry(&path, "mitodo").unwrap(), Some(ours()));
    }
}
```

Add `pub mod desktop;` to `src/selfcfg/mod.rs`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet desktop 2>&1 | tail -6`
Expected: `file not found for module desktop` before it is declared.

- [ ] **Step 3: Run them after declaring the module**

Run: `cargo test --quiet desktop 2>&1 | tail -4 && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3`
Expected: eleven tests pass, clippy silent.

- [ ] **Step 4: Prove the atomicity claim is not just a comment**

The rename is the only mutation, so a failure before it leaves the original
intact — which the `unparseable_json_is_refused_and_the_file_is_untouched` test
already demonstrates end to end. Confirm no code path writes `path` directly:

```bash
grep -n 'fs::write' src/selfcfg/desktop.rs
```
Expected: exactly one hit, and its target is `temp.path()`, never `path`.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/selfcfg
git commit -m "feat(selfcfg): merge one key into Claude Desktop's config, atomically"
```

---

### Task 3: The three verbs

**Files:**
- Modify: `src/selfcfg/mod.rs`

**Interfaces:**
- Consumes: `target::{Target, Kind, detect, unsupported}` (Task 1); `desktop::{Entry, read_entry, merge, remove}` (Task 2).
- Produces: `Outcome { target: String, state: State, detail: String }`; `State { Registered, Repointed, Current, Removed, Nothing, Failed, Unsupported, Missing }`; `plan_entry() -> Result<desktop::Entry, String>`; `setup(dry_run: bool) -> Vec<Outcome>`; `status() -> Vec<Outcome>`; `remove_all(dry_run: bool) -> Vec<Outcome>`; `report(outcomes: &[Outcome]) -> String`; `exit_code(outcomes: &[Outcome]) -> i32`; `parse_command(stdout: &str) -> Option<String>`.

- [ ] **Step 1: Write the failing tests**

Replace `src/selfcfg/mod.rs` with the module plus its tests:

```rust
//! Registering mitodo with the MCP clients on this machine.
//!
//! A client that ships its own `mcp add` is driven through it; only a client
//! with no CLI has its config edited here.

pub mod desktop;
pub mod target;

use std::process::Command;

use target::{Kind, Target};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Registered,
    Repointed,
    Current,
    Removed,
    Nothing,
    Failed,
    Unsupported,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub target: String,
    pub state: State,
    pub detail: String,
}

/// The name every client registers mitodo under.
pub const SERVER_NAME: &str = "mitodo";

/// What to register: this binary, by absolute path, serving MCP.
///
/// Bare `mitodo` would resolve for a shell-launched client and fail for the
/// desktop app, which inherits no PATH.
pub fn plan_entry() -> Result<desktop::Entry, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("cannot determine this binary's path: {e}"))?;
    Ok(desktop::Entry {
        command: exe.to_string_lossy().to_string(),
        args: vec!["mcp-server".to_string()],
    })
}

/// The command a client reports for a registered server.
///
/// `claude` prints `Command: uv` and `codex` prints `command: /abs/path`, so the
/// match is case-insensitive and takes the remainder of the line.
pub fn parse_command(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        let (head, rest) = trimmed.split_once(':')?;
        if head.trim().eq_ignore_ascii_case("command") {
            let value = rest.trim();
            (!value.is_empty()).then(|| value.to_string())
        } else {
            None
        }
    })
}

pub fn setup(dry_run: bool) -> Vec<Outcome> {
    let entry = match plan_entry() {
        Ok(entry) => entry,
        Err(why) => {
            return vec![Outcome {
                target: "mitodo".to_string(),
                state: State::Failed,
                detail: why,
            }];
        }
    };
    let mut outcomes: Vec<Outcome> = target::detect()
        .iter()
        .map(|t| register(t, &entry, dry_run))
        .collect();
    outcomes.extend(unsupported_outcomes());
    outcomes
}

pub fn status() -> Vec<Outcome> {
    let wanted = plan_entry().ok();
    let mut outcomes: Vec<Outcome> = target::detect()
        .iter()
        .map(|t| inspect(t, wanted.as_ref()))
        .collect();
    outcomes.extend(unsupported_outcomes());
    outcomes
}

pub fn remove_all(dry_run: bool) -> Vec<Outcome> {
    target::detect().iter().map(|t| unregister(t, dry_run)).collect()
}

fn unsupported_outcomes() -> Vec<Outcome> {
    target::unsupported()
        .into_iter()
        .map(|(name, why)| Outcome {
            target: name.to_string(),
            state: State::Unsupported,
            detail: why.to_string(),
        })
        .collect()
}

/// What a client currently has registered, if anything.
fn current(kind: &Kind) -> Result<Option<String>, String> {
    match kind {
        Kind::Delegated { cli, .. } => {
            let output = Command::new(cli)
                .args(["mcp", "get", SERVER_NAME])
                .output()
                .map_err(|e| format!("could not run {cli}: {e}"))?;
            if !output.status.success() {
                return Ok(None);
            }
            Ok(parse_command(&String::from_utf8_lossy(&output.stdout)))
        }
        Kind::DesktopJson { path } => {
            Ok(desktop::read_entry(path, SERVER_NAME)?.map(|entry| entry.command))
        }
    }
}

fn register(target: &Target, entry: &desktop::Entry, dry_run: bool) -> Outcome {
    let existing = match current(&target.kind) {
        Ok(existing) => existing,
        Err(why) => return failed(target, why),
    };

    match existing {
        Some(command) if command == entry.command => Outcome {
            target: target.name.to_string(),
            state: State::Current,
            detail: entry.command.clone(),
        },
        existing => {
            if dry_run {
                return Outcome {
                    target: target.name.to_string(),
                    state: match existing {
                        Some(_) => State::Repointed,
                        None => State::Registered,
                    },
                    detail: format!("would register {}", entry.command),
                };
            }
            // `claude mcp add` refuses an existing name and keeps the old value,
            // while `codex` overwrites: removing first makes both re-point.
            if existing.is_some()
                && let Err(why) = do_remove(&target.kind)
            {
                return failed(target, why);
            }
            match do_add(&target.kind, entry) {
                Err(why) => failed(target, why),
                Ok(()) => Outcome {
                    target: target.name.to_string(),
                    state: match existing {
                        Some(old) => {
                            return Outcome {
                                target: target.name.to_string(),
                                state: State::Repointed,
                                detail: format!("was {old}"),
                            };
                        }
                        None => State::Registered,
                    },
                    detail: entry.command.clone(),
                },
            }
        }
    }
}

fn inspect(target: &Target, wanted: Option<&desktop::Entry>) -> Outcome {
    match current(&target.kind) {
        Err(why) => failed(target, why),
        Ok(None) => Outcome {
            target: target.name.to_string(),
            state: State::Nothing,
            detail: "not registered".to_string(),
        },
        Ok(Some(command)) => {
            let resolves = std::path::Path::new(&command).is_file();
            let matches = wanted.is_some_and(|w| w.command == command);
            Outcome {
                target: target.name.to_string(),
                state: if resolves && matches {
                    State::Current
                } else if resolves {
                    State::Registered
                } else {
                    State::Missing
                },
                detail: command,
            }
        }
    }
}

fn unregister(target: &Target, dry_run: bool) -> Outcome {
    match current(&target.kind) {
        Err(why) => failed(target, why),
        Ok(None) => Outcome {
            target: target.name.to_string(),
            state: State::Nothing,
            detail: "nothing to remove".to_string(),
        },
        Ok(Some(command)) => {
            if dry_run {
                return Outcome {
                    target: target.name.to_string(),
                    state: State::Removed,
                    detail: format!("would remove {command}"),
                };
            }
            match do_remove(&target.kind) {
                Err(why) => failed(target, why),
                Ok(()) => Outcome {
                    target: target.name.to_string(),
                    state: State::Removed,
                    detail: command,
                },
            }
        }
    }
}

fn do_add(kind: &Kind, entry: &desktop::Entry) -> Result<(), String> {
    match kind {
        Kind::Delegated { cli, scope } => {
            let mut command = Command::new(cli);
            command.args(["mcp", "add"]);
            if let Some(scope) = scope {
                command.args(["--scope", scope]);
            }
            command.arg(SERVER_NAME).arg("--").arg(&entry.command);
            command.args(&entry.args);
            run(command, cli)
        }
        Kind::DesktopJson { path } => desktop::merge(path, SERVER_NAME, entry),
    }
}

fn do_remove(kind: &Kind) -> Result<(), String> {
    match kind {
        Kind::Delegated { cli, scope } => {
            let mut command = Command::new(cli);
            command.args(["mcp", "remove"]);
            if let Some(scope) = scope {
                command.args(["--scope", scope]);
            }
            command.arg(SERVER_NAME);
            run(command, cli)
        }
        Kind::DesktopJson { path } => desktop::remove(path, SERVER_NAME).map(|_| ()),
    }
}

fn run(mut command: Command, cli: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|e| format!("could not run {cli}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(stderr
        .lines()
        .next()
        .unwrap_or("the command failed")
        .to_string())
}

fn failed(target: &Target, detail: String) -> Outcome {
    Outcome {
        target: target.name.to_string(),
        state: State::Failed,
        detail,
    }
}

/// One renderer, so the three verbs cannot drift in wording.
pub fn report(outcomes: &[Outcome]) -> String {
    outcomes
        .iter()
        .map(|outcome| {
            let (mark, what) = match outcome.state {
                State::Registered => ("+", "registered"),
                State::Repointed => ("~", "re-pointed"),
                State::Current => ("=", "already current"),
                State::Removed => ("-", "removed"),
                State::Nothing => (" ", "nothing to do"),
                State::Failed => ("!", "failed"),
                State::Unsupported => (" ", "unsupported"),
                State::Missing => ("!", "path no longer exists"),
            };
            format!(
                "{mark} {:<16} {what} · {}",
                outcome.target, outcome.detail
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 0 all good · 1 a found target failed · 2 no supported client at all.
pub fn exit_code(outcomes: &[Outcome]) -> i32 {
    if outcomes.iter().any(|o| o.state == State::Failed) {
        return 1;
    }
    let supported = outcomes
        .iter()
        .filter(|o| o.state != State::Unsupported)
        .count();
    if supported == 0 { 2 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registered_command_is_this_binary_by_absolute_path() {
        let entry = plan_entry().unwrap();
        assert!(
            std::path::Path::new(&entry.command).is_absolute(),
            "a bare name fails for a GUI-launched client: {}",
            entry.command
        );
        assert_eq!(entry.args, vec!["mcp-server".to_string()]);
    }

    // claude prints "Command:" and codex prints "command:".
    #[test]
    fn a_reported_command_is_parsed_whatever_its_case() {
        assert_eq!(
            parse_command("name:\n  Scope: User\n  Command: /abs/mitodo\n  Args: mcp-server"),
            Some("/abs/mitodo".to_string())
        );
        assert_eq!(
            parse_command("name\n  command: /abs/mitodo\n  args: mcp-server"),
            Some("/abs/mitodo".to_string())
        );
    }

    #[test]
    fn output_with_no_command_line_parses_to_nothing() {
        assert_eq!(parse_command("No MCP server found"), None);
        assert_eq!(parse_command("command:"), None, "an empty value is nothing");
    }

    #[test]
    fn a_failure_anywhere_makes_the_exit_code_one() {
        let outcomes = vec![
            Outcome {
                target: "claude".to_string(),
                state: State::Registered,
                detail: String::new(),
            },
            Outcome {
                target: "codex".to_string(),
                state: State::Failed,
                detail: "boom".to_string(),
            },
        ];
        assert_eq!(exit_code(&outcomes), 1);
    }

    // "nothing to do" and "everything worked" must not look alike.
    #[test]
    fn finding_no_supported_client_is_exit_code_two() {
        let only_unsupported = vec![Outcome {
            target: "zed".to_string(),
            state: State::Unsupported,
            detail: "JSONC".to_string(),
        }];
        assert_eq!(exit_code(&only_unsupported), 2);
        assert_eq!(exit_code(&[]), 2);
    }

    #[test]
    fn a_clean_run_is_exit_code_zero() {
        let outcomes = vec![Outcome {
            target: "claude".to_string(),
            state: State::Current,
            detail: "/abs/mitodo".to_string(),
        }];
        assert_eq!(exit_code(&outcomes), 0);
    }

    #[test]
    fn every_state_renders_a_line_naming_its_target() {
        for state in [
            State::Registered,
            State::Repointed,
            State::Current,
            State::Removed,
            State::Nothing,
            State::Failed,
            State::Unsupported,
            State::Missing,
        ] {
            let line = report(&[Outcome {
                target: "claude".to_string(),
                state,
                detail: "detail".to_string(),
            }]);
            assert!(line.contains("claude"), "{state:?} omits the target");
            assert!(line.contains("detail"), "{state:?} omits the detail");
        }
    }

    #[test]
    fn the_unsupported_clients_appear_in_status() {
        let listed = unsupported_outcomes();
        assert!(listed.iter().all(|o| o.state == State::Unsupported));
        assert!(listed.iter().any(|o| o.target == "opencode"));
        assert!(listed.iter().any(|o| o.target == "zed"));
    }

    // A stub client, so the argv mitodo builds is asserted without needing a
    // real CLI installed.
    fn stub_cli(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn registering_a_delegated_client_passes_the_path_after_a_double_dash() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("argv.log");
        // `mcp get` must fail so the target reads as unregistered.
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\n[ \"$2\" = get ] && exit 1\nexit 0\n",
            log.display()
        );
        let cli = stub_cli(dir.path(), "fakeclient", &script);

        let target = Target {
            name: "fake",
            kind: Kind::Delegated {
                cli: Box::leak(cli.to_string_lossy().to_string().into_boxed_str()),
                scope: Some("user"),
            },
        };
        let entry = desktop::Entry {
            command: "/abs/mitodo".to_string(),
            args: vec!["mcp-server".to_string()],
        };
        let outcome = register(&target, &entry, false);
        assert_eq!(outcome.state, State::Registered, "{outcome:?}");

        let argv = std::fs::read_to_string(&log).unwrap();
        assert!(argv.contains("mcp get mitodo"), "{argv}");
        assert!(
            argv.contains("mcp add --scope user mitodo -- /abs/mitodo mcp-server"),
            "{argv}"
        );
    }

    #[test]
    fn a_delegated_client_that_fails_is_reported_not_panicked() {
        let dir = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\n[ \"$2\" = get ] && exit 1\necho 'it went wrong' >&2\nexit 3\n";
        let cli = stub_cli(dir.path(), "failclient", script);
        let target = Target {
            name: "fake",
            kind: Kind::Delegated {
                cli: Box::leak(cli.to_string_lossy().to_string().into_boxed_str()),
                scope: None,
            },
        };
        let outcome = register(&target, &plan_entry().unwrap(), false);
        assert_eq!(outcome.state, State::Failed);
        assert_eq!(outcome.detail, "it went wrong");
    }

    #[test]
    fn a_dry_run_spawns_nothing_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("argv.log");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\n[ \"$2\" = get ] && exit 1\nexit 0\n",
            log.display()
        );
        let cli = stub_cli(dir.path(), "dryclient", &script);
        let target = Target {
            name: "fake",
            kind: Kind::Delegated {
                cli: Box::leak(cli.to_string_lossy().to_string().into_boxed_str()),
                scope: None,
            },
        };
        let outcome = register(&target, &plan_entry().unwrap(), true);
        assert_eq!(outcome.state, State::Registered);
        assert!(outcome.detail.starts_with("would register"));
        // `get` still runs to decide what would happen; `add` must not.
        let argv = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(!argv.contains("mcp add"), "a dry run added something: {argv}");
    }

    #[test]
    fn a_desktop_target_with_a_stale_path_is_repointed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        std::fs::write(&path, r#"{"mcpServers":{}}"#).unwrap();
        desktop::merge(
            &path,
            SERVER_NAME,
            &desktop::Entry {
                command: "/old/mitodo".to_string(),
                args: vec!["mcp-server".to_string()],
            },
        )
        .unwrap();

        let target = Target {
            name: "claude-desktop",
            kind: Kind::DesktopJson { path: path.clone() },
        };
        let entry = plan_entry().unwrap();
        let outcome = register(&target, &entry, false);
        assert_eq!(outcome.state, State::Repointed);
        assert!(outcome.detail.contains("/old/mitodo"), "{}", outcome.detail);
        assert_eq!(
            desktop::read_entry(&path, SERVER_NAME).unwrap().unwrap().command,
            entry.command
        );
    }

    #[test]
    fn a_desktop_target_already_current_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        let entry = plan_entry().unwrap();
        desktop::merge(&path, SERVER_NAME, &entry).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        let target = Target {
            name: "claude-desktop",
            kind: Kind::DesktopJson { path: path.clone() },
        };
        assert_eq!(register(&target, &entry, false).state, State::Current);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn status_flags_a_registered_path_that_no_longer_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        desktop::merge(
            &path,
            SERVER_NAME,
            &desktop::Entry {
                command: "/nonexistent/mitodo".to_string(),
                args: vec!["mcp-server".to_string()],
            },
        )
        .unwrap();
        let target = Target {
            name: "claude-desktop",
            kind: Kind::DesktopJson { path },
        };
        let outcome = inspect(&target, plan_entry().ok().as_ref());
        assert_eq!(outcome.state, State::Missing);
    }

    #[test]
    fn removing_a_desktop_target_reports_what_it_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        let entry = plan_entry().unwrap();
        desktop::merge(&path, SERVER_NAME, &entry).unwrap();
        let target = Target {
            name: "claude-desktop",
            kind: Kind::DesktopJson { path: path.clone() },
        };
        assert_eq!(unregister(&target, false).state, State::Removed);
        assert!(desktop::read_entry(&path, SERVER_NAME).unwrap().is_none());

        assert_eq!(
            unregister(&target, false).state,
            State::Nothing,
            "removing twice is not an error"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet selfcfg 2>&1 | tail -8`
Expected: compile errors for the newly referenced items before the module body
above replaces the stub.

- [ ] **Step 3: Simplify `register`'s nested return**

The `register` body in step 1 contains a `return` inside a `match` arm that
builds an `Outcome` — that compiles but reads badly. Replace the `Ok(())` arm
with the flat form:

```rust
            match do_add(&target.kind, entry) {
                Err(why) => failed(target, why),
                Ok(()) => match existing {
                    Some(old) => Outcome {
                        target: target.name.to_string(),
                        state: State::Repointed,
                        detail: format!("was {old}"),
                    },
                    None => Outcome {
                        target: target.name.to_string(),
                        state: State::Registered,
                        detail: entry.command.clone(),
                    },
                },
            }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --quiet selfcfg 2>&1 | tail -4 && cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3`
Expected: pass, clippy silent. `Box::leak` in the stub tests is deliberate — `cli`
is a `&'static str` in `Kind`, and leaking a few bytes in a test is cheaper than
making the field owned for production code that only ever uses literals.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/selfcfg
git commit -m "feat(selfcfg): setup, status and remove over every detected client"
```

---

### Task 4: Wire the subcommand, document it, and gate

**Files:**
- Modify: `src/cli.rs`, `src/main.rs`, `README.md`

**Interfaces:**
- Consumes: `selfcfg::{setup, status, remove_all, report, exit_code}` (Task 3).
- Produces: `Command::Selfie { action: SelfAction }`; `SelfAction::Mcp { action: McpAction }`; `McpAction::{Setup { dry_run }, Status, Remove { dry_run }}`.

- [ ] **Step 1: Add the subcommand**

In `src/cli.rs`, extend `enum Command`:

```rust
    /// Manage this installation of mitodo
    #[command(name = "self")]
    Selfie {
        #[command(subcommand)]
        action: SelfAction,
    },
```

and add below it — `Self` is a reserved word, hence `Selfie` with clap spelling
the subcommand `self`:

```rust
#[derive(Subcommand, Debug, Clone)]
pub enum SelfAction {
    /// Register mitodo's MCP server with the clients on this machine
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum McpAction {
    /// Register with every supported client found
    Setup {
        /// Print what would change, and change nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Show where mitodo is registered, and whether its path still resolves
    Status,
    /// Unregister from every client that has it
    Remove {
        #[arg(long)]
        dry_run: bool,
    },
}
```

- [ ] **Step 2: Dispatch it**

In `src/main.rs`, add `mod selfcfg;` if Task 1 did not, import the new types
alongside `Command`, and add the arm before the TUI arm:

```rust
        Some(Command::Selfie { action }) => cmd_self(action),
```

with the handler beside `cmd_mcp_server`:

```rust
/// Registering with other tools writes outside mitodo's own files, so the
/// outcome of every target is printed and the exit code says whether any failed.
fn cmd_self(action: &cli::SelfAction) -> Result<()> {
    let cli::SelfAction::Mcp { action } = action;
    let outcomes = match action {
        cli::McpAction::Setup { dry_run } => selfcfg::setup(*dry_run),
        cli::McpAction::Status => selfcfg::status(),
        cli::McpAction::Remove { dry_run } => selfcfg::remove_all(*dry_run),
    };
    println!("{}", selfcfg::report(&outcomes));
    let code = selfcfg::exit_code(&outcomes);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
```

- [ ] **Step 3: Verify it runs, without changing anything**

```bash
cargo run --quiet -- self mcp status
cargo run --quiet -- self mcp setup --dry-run
```
Expected: `status` lists `claude`, `codex`, `claude-desktop` as not registered,
plus `opencode` and `zed` as unsupported with reasons. `--dry-run` says it would
register three targets. **Neither writes anything** — confirm with
`claude mcp get mitodo; echo $?` still exiting 1.

- [ ] **Step 4: Replace the hand-written incantation in the README**

The MCP section currently opens with a `--mcp-config` command line. Put the
bootstrap first, and keep the manual form as the fallback:

```markdown
## Using mitodo from Claude Code or Codex

Register it once:

```sh
mitodo self mcp setup
```

That writes mitodo's absolute path into every supported client it finds —
`claude` and `codex` through their own `mcp add`, and Claude Desktop by merging a
single key into its config. Re-run it after any reinstall: it reports
`already current`, or re-points a path that has moved.

```sh
mitodo self mcp status     # where it is registered, and whether the path resolves
mitodo self mcp remove     # unregister everywhere
```

`opencode` and Zed are detected but skipped, and `status` says why: `opencode mcp
add` takes no arguments to script, and Zed's settings are JSONC, so rewriting
them would delete your comments.

Nothing stops you registering by hand instead:

```sh
claude --strict-mcp-config \
  --mcp-config '{"mcpServers":{"mitodo":{"command":"mitodo","args":["mcp-server"]}}}' \
  -p 'archive the everlongtech items that closed'
```
```

Leave the existing paragraphs about the thirteen tools, the absent review step
and group-name containment exactly as they are.

- [ ] **Step 5: Run the full gate**

```bash
cargo test 2>&1 | grep -E '^test result'
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | grep -cE '^(warning|error)'
cargo fmt --check && echo FMT-OK
```
Expected: one `test result: ok` with 0 failed and 0 ignored; clippy count `0`;
`FMT-OK`.

- [ ] **Step 6: Check the comment budget**

```bash
comments=$(git diff main -U0 -- '*.rs' | grep -cE '^\+\s*//')
added=$(git diff main -U0 -- '*.rs' | grep -cE '^\+')
echo "$comments of $added added rust lines = $((100*comments/added))%"
```
Expected: under ~10%.

- [ ] **Step 7: Register for real, and prove it round-trips**

This is the step CI cannot do, because it writes outside the repository:

```bash
cargo install --path . --force
mitodo self mcp setup
mitodo self mcp status
claude mcp get mitodo | grep -iE 'command|args'
python3 -c "import json,os; d=json.load(open(os.path.expanduser('~/Library/Application Support/Claude/claude_desktop_config.json'))); print(d['mcpServers']['mitodo'])"
mitodo self mcp setup            # second run must say "already current"
```
Expected: `claude mcp get mitodo` shows the absolute `~/.cargo/bin/mitodo` path
with `mcp-server`; the desktop config holds the same; the second `setup` changes
nothing. Then confirm the other servers in the desktop config are untouched:

```bash
python3 -c "import json,os; d=json.load(open(os.path.expanduser('~/Library/Application Support/Claude/claude_desktop_config.json'))); print(sorted(d['mcpServers']))"
```
Expected: `gmail-multi` and `repowise` still present.

- [ ] **Step 8: Commit**

```bash
git add src/cli.rs src/main.rs README.md
git commit -m "feat(cli): self mcp setup, status and remove"
```

---

## Self-review

**Spec coverage.** §1's three verbs → Task 3, wired in Task 4. §2's client survey → Task 1's `detect_in` and `unsupported`, with `opencode` and Zed excluded by name. §3's registration triple → Task 3's `plan_entry`, asserted absolute by `the_registered_command_is_this_binary_by_absolute_path`; `--scope user` → Task 1's `Kind::Delegated { scope }` and Task 3's `do_add`. §4's architecture → the three modules across Tasks 1–3, with delegation and file-merge kept apart by `Kind`. §5's per-verb behaviour table → Task 3's `register`, `inspect`, `unregister`, each state covered by a test; exit codes → `exit_code` with all three cases tested. §6's error table → `load`'s parse refusal, `servers_of`'s non-object refusal, `run`'s stderr capture, `current_exe`'s failure aborting before any write. §7's test list → Tasks 1–3. §8 out-of-scope → nothing in any task. §9's files → Tasks 1–4.

**One spec item deliberately not implemented:** `setup --config-dir <dir>` and `--name`, mentioned in §3 as the way to pin a second registration. No task adds them, because the default registration follows the config file wherever it points and a second named registration has no user yet. Flagged here rather than silently dropped; §8 should gain them as out-of-scope in a follow-up edit if they stay unbuilt.

**Placeholder scan.** No TBD/TODO/"handle errors appropriately". Task 1 step 3 says "if `-D warnings` rejects unused items, annotate the single item" — a conditional instruction with the exact remedy named, and it explicitly forbids the module-wide allow that went stale in the MCP work. Task 3 step 3 rewrites code step 1 deliberately introduced, which is unusual but honest: the flat form is clearer and the plan says so rather than pretending the first draft was right.

**Type consistency.** `desktop::Entry { command: String, args: Vec<String> }` is identical in Tasks 2 and 3. `Kind::Delegated { cli: &'static str, scope: Option<&'static str> }` matches between Task 1's definition and Task 3's `do_add`/`do_remove`/`current`. `read_entry(&Path, &str) -> Result<Option<Entry>, String>`, `merge(&Path, &str, &Entry) -> Result<(), String>` and `remove(&Path, &str) -> Result<bool, String>` match between Task 2 and Task 3. `Outcome`/`State` match between Task 3's definition and Task 4's dispatch. `SERVER_NAME` is used everywhere rather than the literal `"mitodo"` being repeated.
