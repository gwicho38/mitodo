# Model Services and Agent Management Popup — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let mitodo choose between several model-service CLIs (claude, codex, ollama) from a picker in the UI, and drive item management — add, complete, update, archive — through a free-form agent popup whose output goes through the existing review pane.

**Architecture:** `[[services]]` in config replaces the single `[agent].command`, with a `schema_mode` enum covering the three ways these CLIs accept a JSON schema (inline string, file path, prompt text). A new `Verb::Manage` reuses the change-set pipeline that `scan` already uses; `ChangeAction::Archive` is added and delegates to a new `store::archive::archive_items`, extracted from `archive_done`. `agent/` still never writes files — it returns a `ChangeSet` and only `changeset::apply` touches disk, after per-change review.

**Tech Stack:** Rust 2024 edition (rustc 1.85+), ratatui 0.30, serde + toml, chrono, tempfile (promoted from dev-dependency), std threads and `std::process::Command`. No async runtime involvement in the agent path.

## Global Constraints

- Spec: [docs/superpowers/specs/2026-08-11-model-services-and-agent-popup-design.md](../specs/2026-08-11-model-services-and-agent-popup-design.md). Where this plan and the spec differ, two corrections are recorded in Task 5 and Task 8 — the spec's signatures there were wrong.
- Branch: `feat/model-services-and-agent-popup`. Never commit to `main`.
- Comments: minimum possible, one line each, stating a hidden constraint only. Never restate what the code does. Never reference this plan, this branch, or the change that introduced the line.
- Test names are sentences describing behaviour (`archive_rows_start_unticked`), matching the existing suite. No `#[ignore]`, no skipped tests.
- `cargo clippy --all-targets` must be warning-free at every commit; `cargo test` must be green at every commit.
- Existing `archive_done` tests must pass **unmodified**. Editing them means the Task 5 refactor changed behaviour.
- Existing configs with `[agent].command` and no `[[services]]` must keep working with zero edits and no behaviour change.
- `agent/` must not gain any filesystem write outside the schema temp file.
- One new dependency only: `tempfile` moves from `[dev-dependencies]` to `[dependencies]`. Add nothing else.
- Commit after every task with a Conventional Commits subject. No AI attribution or co-author trailers.

---

### Task 1: Service config, schema modes, and legacy synthesis

**Files:**
- Modify: `src/config/mod.rs:22-53` (`Config`, `UiConfig`), `src/config/mod.rs:121-145` (`AgentConfig`)
- Test: `src/config/mod.rs` `mod tests` (same file, existing pattern)

**Interfaces:**
- Consumes: nothing.
- Produces: `SchemaMode { Flag, File, Prompt }`; `ServiceConfig { name: String, command: Vec<String>, schema_mode: SchemaMode, schema_flag: Option<String>, timeout_secs: u64 }`; `Config::services(&self) -> Vec<ServiceConfig>`; `ActiveService { service: Option<ServiceConfig>, notice: Option<String> }`; `Config::active_service(&self) -> ActiveService`; `UiConfig.service: Option<String>`. `UiConfig` **loses `Copy`** — it now holds a `String`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/config/mod.rs`:

```rust
    const THREE_SERVICES: &str = r#"
[workspace]
root      = "/tmp/w"
group_by  = "directory"
todo_glob = "*/TODO.md"

[[services]]
name         = "claude"
command      = ["claude", "--print"]
schema_mode  = "flag"
schema_flag  = "--json-schema"
timeout_secs = 600

[[services]]
name         = "codex"
command      = ["codex", "exec", "--json"]
schema_mode  = "file"
schema_flag  = "--output-schema"

[[services]]
name        = "ollama"
command     = ["ollama", "run", "qwen2.5:3b", "--format", "json"]
schema_mode = "prompt"

[ui]
service = "codex"
"#;

    #[test]
    fn three_services_parse_in_config_order() {
        let cfg: Config = toml::from_str(THREE_SERVICES).unwrap();
        let names: Vec<&str> = cfg.services().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["claude", "codex", "ollama"]);
        assert_eq!(cfg.services()[1].schema_mode, SchemaMode::File);
        assert_eq!(cfg.services()[2].schema_flag, None);
    }

    #[test]
    fn a_service_without_a_timeout_gets_the_default() {
        let cfg: Config = toml::from_str(THREE_SERVICES).unwrap();
        assert_eq!(cfg.services()[1].timeout_secs, default_timeout());
    }

    #[test]
    fn ui_service_selects_which_one_is_active() {
        let cfg: Config = toml::from_str(THREE_SERVICES).unwrap();
        let active = cfg.active_service();
        assert_eq!(active.service.unwrap().name, "codex");
        assert_eq!(active.notice, None);
    }

    // A config shared between machines can name a service the other one lacks;
    // opening the workspace must not depend on the agent being resolvable.
    #[test]
    fn an_unknown_ui_service_falls_back_to_the_first_with_a_notice() {
        let text = THREE_SERVICES.replace(r#"service = "codex""#, r#"service = "gpt5""#);
        let cfg: Config = toml::from_str(&text).unwrap();
        let active = cfg.active_service();
        assert_eq!(active.service.unwrap().name, "claude");
        let notice = active.notice.expect("a notice explains the fallback");
        assert!(notice.contains("gpt5") && notice.contains("claude"), "{notice}");
    }

    #[test]
    fn no_ui_service_means_the_first_one() {
        let text = THREE_SERVICES.replace("[ui]\nservice = \"codex\"\n", "");
        let cfg: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg.active_service().service.unwrap().name, "claude");
    }

    #[test]
    fn a_legacy_agent_section_becomes_one_service_named_default() {
        let legacy = format!(
            "{SAMPLE}\n[agent]\ncommand = [\"claude\", \"--print\"]\nschema_flag = \"--json-schema\"\ntimeout_secs = 42\n"
        );
        let cfg: Config = toml::from_str(&legacy).unwrap();
        let services = cfg.services();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "default");
        assert_eq!(services[0].command, vec!["claude", "--print"]);
        assert_eq!(services[0].schema_flag.as_deref(), Some("--json-schema"));
        assert_eq!(services[0].schema_mode, SchemaMode::Flag);
        assert_eq!(services[0].timeout_secs, 42);
        assert_eq!(cfg.active_service().service.unwrap().name, "default");
    }

    #[test]
    fn services_win_over_a_legacy_agent_section() {
        let both = format!("{THREE_SERVICES}\n[agent]\ncommand = [\"old\"]\n");
        let cfg: Config = toml::from_str(&both).unwrap();
        assert_eq!(cfg.services().len(), 3);
    }

    #[test]
    fn no_services_and_no_agent_means_none_active() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        assert!(cfg.services().is_empty());
        assert!(cfg.active_service().service.is_none());
    }

    #[test]
    fn a_service_list_round_trips_through_toml() {
        let cfg: Config = toml::from_str(THREE_SERVICES).unwrap();
        let rendered = toml::to_string_pretty(&cfg).unwrap();
        let again: Config = toml::from_str(&rendered).unwrap();
        assert_eq!(cfg, again);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet config 2>&1 | tail -20`
Expected: compile errors — `SchemaMode` not found, no method `services`, no field `service` on `UiConfig`.

- [ ] **Step 3: Implement the config types**

In `src/config/mod.rs`, add after `PrioritySource` (around line 119):

```rust
/// How a service is handed the JSON schema it must answer in.
///
/// claude takes it inline, codex takes a file path, ollama takes neither and
/// needs it stated in the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SchemaMode {
    #[default]
    Flag,
    File,
    Prompt,
}

/// One model service: a CLI that takes a prompt and emits JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub schema_mode: SchemaMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_flag: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

/// The service in force, plus anything the user should be told about resolving it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActiveService {
    pub service: Option<ServiceConfig>,
    pub notice: Option<String>,
}
```

Add the field to `Config` (after `agent`, around line 32):

```rust
    #[serde(default, rename = "services", skip_serializing_if = "Vec::is_empty")]
    pub service_list: Vec<ServiceConfig>,
```

Add to `UiConfig` (after `wrap`, around line 52) and drop `Copy` from its derive list:

```rust
    /// Which service the picker last selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
```

Add `service: None` to `UiConfig::default()`.

Add the resolution methods in the `impl Config` block:

```rust
    /// Every configured service, in config order.
    ///
    /// A config predating `[[services]]` still has `[agent]`; it reads as one
    /// service so an existing setup keeps working unedited.
    pub fn services(&self) -> Vec<ServiceConfig> {
        if !self.service_list.is_empty() {
            return self.service_list.clone();
        }
        if self.agent.is_disabled() {
            return Vec::new();
        }
        vec![ServiceConfig {
            name: "default".to_string(),
            command: self.agent.command.clone(),
            schema_mode: SchemaMode::Flag,
            schema_flag: self.agent.schema_flag.clone(),
            timeout_secs: self.agent.timeout_secs,
        }]
    }

    /// The service `ui.service` names, else the first one.
    pub fn active_service(&self) -> ActiveService {
        let services = self.services();
        let Some(first) = services.first() else {
            return ActiveService::default();
        };
        match &self.ui.service {
            None => ActiveService { service: Some(first.clone()), notice: None },
            Some(wanted) => match services.iter().find(|s| &s.name == wanted) {
                Some(found) => ActiveService { service: Some(found.clone()), notice: None },
                None => ActiveService {
                    service: Some(first.clone()),
                    notice: Some(format!(
                        "service {:?} not in config — using {}",
                        wanted, first.name
                    )),
                },
            },
        }
    }
```

- [ ] **Step 4: Fix the `Copy` fallout**

`UiConfig` is no longer `Copy`. Two call sites break:

`src/ui/mod.rs:452-462` — clone the preserved fields instead of copying the struct:

```rust
        let current = crate::config::UiConfig {
            hide_done: self.hide_done,
            ticker: self.ticker.is_some(),
            // Not view toggles; preserve whatever the user configured.
            mouse: self.config.ui.mouse,
            wrap: self.wrap,
            service: self.config.ui.service.clone(),
        };
```

`src/main.rs:209` already reads `config.ui.mouse` (a `bool`), so it needs no change. Run `cargo build` and fix any other move errors the compiler names.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --quiet config 2>&1 | tail -8 && cargo clippy --all-targets 2>&1 | tail -3`
Expected: all config tests pass, clippy silent.

- [ ] **Step 6: Commit**

```bash
git add src/config/mod.rs src/ui/mod.rs
git commit -m "feat(config): a named list of model services, with legacy [agent] as one"
```

---

### Task 2: `agent::run` takes a service and honours all three schema modes

**Files:**
- Modify: `src/agent/mod.rs:150-210` (`run`), `src/agent/mod.rs:22-34` (`AgentError`)
- Modify: `Cargo.toml` (promote `tempfile`)
- Test: `src/agent/mod.rs` `mod tests`

**Interfaces:**
- Consumes: `ServiceConfig`, `SchemaMode` from Task 1.
- Produces: `agent::run(service: &ServiceConfig, schema: &str, prompt: &str, cwd: &Path, cancel: &Arc<AtomicBool>) -> Result<String, AgentError>`; `AgentError::Cancelled`. The old five-argument form is gone.

- [ ] **Step 1: Promote `tempfile` to a dependency**

In `Cargo.toml`, move the line out of `[dev-dependencies]` into `[dependencies]`:

```toml
tempfile = "3.14.0"
```

Leave it out of `[dev-dependencies]` — a regular dependency is visible to tests already.

- [ ] **Step 2: Write the failing tests**

Replace the existing `runs_a_command_and_returns_stdout`, `passes_the_schema_behind_the_configured_flag`, `an_empty_command_is_not_configured`, `a_missing_program_is_reported`, `a_nonzero_exit_is_reported_with_stderr`, `a_wedged_agent_is_killed_rather_than_waited_on_forever` and `a_prompt_config_is_still_honoured_within_the_timeout` tests in `src/agent/mod.rs` with service-based equivalents, and add the new mode tests:

```rust
    use crate::config::{SchemaMode, ServiceConfig};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn service(command: &[&str], mode: SchemaMode, flag: Option<&str>) -> ServiceConfig {
        ServiceConfig {
            name: "test".to_string(),
            command: command.iter().map(|s| s.to_string()).collect(),
            schema_mode: mode,
            schema_flag: flag.map(|f| f.to_string()),
            timeout_secs: 30,
        }
    }

    fn go(service: &ServiceConfig, prompt: &str) -> Result<String, AgentError> {
        let dir = tempfile::tempdir().unwrap();
        run(service, "{\"type\":\"object\"}", prompt, dir.path(), &Arc::new(AtomicBool::new(false)))
    }

    #[test]
    fn runs_a_command_and_returns_stdout() {
        let out = go(&service(&["echo"], SchemaMode::Prompt, None), "hello").unwrap();
        assert!(out.contains("hello"));
    }

    #[test]
    fn flag_mode_passes_the_schema_inline_behind_the_flag() {
        let svc = service(&["echo"], SchemaMode::Flag, Some("--json-schema"));
        let out = go(&svc, "PROMPT").unwrap();
        assert!(out.contains("--json-schema"), "got {out:?}");
        assert!(out.contains("\"type\":\"object\""), "got {out:?}");
        assert!(out.trim().ends_with("PROMPT"), "prompt comes last: {out:?}");
    }

    // codex takes `--output-schema <FILE>`, so the schema reaches it as a path.
    #[test]
    fn file_mode_passes_a_readable_path_behind_the_flag() {
        let svc = service(&["sh", "-c", r#"printf '%s' "$(cat "$2")""#, "sh"], SchemaMode::File, Some("--x"));
        let out = go(&svc, "ignored").unwrap();
        assert!(out.contains("\"type\":\"object\""), "the file held the schema: {out:?}");
    }

    #[test]
    fn the_schema_file_is_gone_once_the_call_returns() {
        let svc = service(&["sh", "-c", r#"printf '%s' "$2""#, "sh"], SchemaMode::File, Some("--x"));
        let printed = go(&svc, "ignored").unwrap();
        let path = std::path::PathBuf::from(printed.trim());
        assert!(!path.exists(), "temp schema outlived the call: {}", path.display());
    }

    #[test]
    fn prompt_mode_appends_the_schema_to_the_prompt_and_passes_no_flag() {
        let svc = service(&["echo"], SchemaMode::Prompt, Some("--ignored"));
        let out = go(&svc, "PROMPT").unwrap();
        assert!(!out.contains("--ignored"), "prompt mode sends no flag: {out:?}");
        assert!(out.contains("PROMPT"));
        assert!(out.contains("\"type\":\"object\""), "schema is in the prompt: {out:?}");
    }

    // Nested inside a Claude Code session, the child sees itself running inside
    // itself unless this is stripped.
    #[test]
    fn claudecode_is_stripped_from_the_child_environment() {
        unsafe { std::env::set_var("CLAUDECODE", "1") };
        let svc = service(&["sh", "-c", "env", "sh"], SchemaMode::Prompt, None);
        let out = go(&svc, "ignored").unwrap();
        unsafe { std::env::remove_var("CLAUDECODE") };
        assert!(!out.contains("CLAUDECODE"), "leaked into the child: {out:?}");
    }

    #[test]
    fn an_empty_command_is_not_configured() {
        let svc = service(&[], SchemaMode::Flag, None);
        assert!(matches!(go(&svc, "p"), Err(AgentError::NotConfigured)));
    }

    #[test]
    fn a_missing_program_is_reported() {
        let svc = service(&["definitely-not-a-real-program"], SchemaMode::Prompt, None);
        assert!(matches!(go(&svc, "p"), Err(AgentError::Spawn(..))));
    }

    #[test]
    fn a_nonzero_exit_is_reported_with_stderr() {
        let svc = service(&["sh", "-c", "echo boom >&2; exit 2"], SchemaMode::Prompt, None);
        match go(&svc, "ignored") {
            Err(AgentError::Failed(_, stderr)) => assert!(stderr.contains("boom")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_wedged_agent_is_killed_rather_than_waited_on_forever() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(&["sh", "-c", "sleep 60"], SchemaMode::Prompt, None);
        svc.timeout_secs = 1;
        let started = std::time::Instant::now();
        let result = run(&svc, "{}", "ignored", dir.path(), &Arc::new(AtomicBool::new(false)));
        assert!(matches!(result, Err(AgentError::TimedOut(1))), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(10), "gave up promptly");
    }

    #[test]
    fn a_cancelled_call_kills_the_child_promptly() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(&["sh", "-c", "sleep 60"], SchemaMode::Prompt, None);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let result = run(&svc, "{}", "ignored", dir.path(), &cancel);
        assert!(matches!(result, Err(AgentError::Cancelled)), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "cancel is not a timeout wait");
    }
```

Note on the `file_mode` test: `sh -c '<script>' sh` makes the shell's `$0` the literal `sh`, so the appended argv lands on `$1` (the flag) and `$2` (the schema path or the prompt). That is why the scripts read `$2`.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --quiet agent 2>&1 | tail -20`
Expected: compile errors — `run` takes five positional args of the old shape, `AgentError::Cancelled` does not exist.

- [ ] **Step 4: Implement**

In `src/agent/mod.rs`, add the imports and the error variant:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{SchemaMode, ServiceConfig};
```

```rust
    #[error("cancelled")]
    Cancelled,
```

Replace `run` wholesale:

```rust
/// Run a service and return its raw stdout.
///
/// Blocking: callers put this on a dedicated thread so the UI keeps drawing.
/// `cancel` is polled on the same tick as the child, so setting it kills the
/// child rather than waiting out `timeout_secs`.
pub fn run(
    service: &ServiceConfig,
    schema: &str,
    prompt: &str,
    cwd: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<String, AgentError> {
    let Some((program, leading)) = service.command.split_first() else {
        return Err(AgentError::NotConfigured);
    };

    // Held for the whole call: dropping it removes the file, on every exit path.
    let mut schema_file: Option<tempfile::NamedTempFile> = None;

    let mut cmd = Command::new(program);
    cmd.args(leading);
    let prompt = match service.schema_mode {
        SchemaMode::Flag => {
            if let Some(flag) = &service.schema_flag {
                cmd.arg(flag).arg(schema);
            }
            prompt.to_string()
        }
        SchemaMode::File => {
            let mut file = tempfile::Builder::new()
                .prefix("mitodo-schema-")
                .suffix(".json")
                .tempfile()
                .map_err(|err| AgentError::Spawn(program.clone(), err))?;
            std::io::Write::write_all(&mut file, schema.as_bytes())
                .map_err(|err| AgentError::Spawn(program.clone(), err))?;
            if let Some(flag) = &service.schema_flag {
                cmd.arg(flag).arg(file.path());
            }
            schema_file = Some(file);
            prompt.to_string()
        }
        // `--format json` buys valid JSON, not the right shape, so say the shape.
        SchemaMode::Prompt => {
            format!("{prompt}\n\nReply with JSON matching this schema:\n{schema}")
        }
    };

    cmd.arg(&prompt)
        .current_dir(cwd)
        // Nested inside an agent session this makes the child see itself.
        .env_remove("CLAUDECODE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|err| AgentError::Spawn(program.clone(), err))?;

    // Poll rather than block, so a wedged or unwanted agent is killed instead
    // of leaving the UI waiting on it. The replies are small JSON documents, so
    // the pipe buffer will not fill while we wait.
    let deadline = Instant::now() + Duration::from_secs(service.timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if cancel.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(AgentError::Cancelled);
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(AgentError::TimedOut(service.timeout_secs));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => return Err(AgentError::Spawn(program.clone(), err)),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|err| AgentError::Spawn(program.clone(), err))?;
    drop(schema_file);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AgentError::Failed(
            output.status.to_string(),
            stderr.lines().next().unwrap_or_default().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

`drop(schema_file)` is explicit only to document the lifetime; the binding would drop at end of scope anyway. Keep the explicit drop — it is the difference between "held long enough" being intentional and being luck.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --quiet agent 2>&1 | tail -8`
Expected: all agent tests pass, including the three mode tests, the temp-file lifetime test, the env-strip test and the cancel test.

Note: `src/ui/mod.rs` will not compile yet — `spawn_agent` still calls the old signature. That is Task 6's job. Run `cargo test --quiet --lib agent` if the build blocks; otherwise apply the minimal `spawn_agent` fix now and let Task 6 finish it.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/agent/mod.rs src/ui/mod.rs
git commit -m "feat(agent): run a configured service, with three schema-delivery modes"
```

---

### Task 3: The `manage` verb and a four-action change schema

**Files:**
- Modify: `src/agent/mod.rs:38-134` (`Verb`)
- Modify: `src/agent/changeset.rs:14` (`SCAN_SCHEMA`)
- Test: `src/agent/mod.rs` `mod tests`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `Verb::Manage` with `label() == "manage"`, `writes() == true`, `schema() == changeset::CHANGE_SCHEMA`; `changeset::CHANGE_SCHEMA` (renamed from `SCAN_SCHEMA`, now with `"archive"` in the action enum).

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/agent/mod.rs`:

```rust
    #[test]
    fn manage_writes_and_so_needs_review() {
        assert!(Verb::Manage.writes());
        assert_eq!(Verb::Manage.label(), "manage");
    }

    #[test]
    fn manage_is_sent_both_the_instruction_and_the_files() {
        let prompt = Verb::Manage.default_prompt();
        assert!(prompt.contains("{input}"), "what the user asked for");
        assert!(prompt.contains("{files}"), "a change names its file");
        assert!(!prompt.contains("{items}"), "the rendered view is not enough");
    }

    #[test]
    fn manage_and_scan_share_the_change_schema() {
        assert_eq!(Verb::Manage.schema(), Verb::Scan.schema());
    }

    #[test]
    fn the_change_schema_offers_archive_as_an_action() {
        let schema: serde_json::Value = serde_json::from_str(Verb::Manage.schema()).unwrap();
        let actions = schema["properties"]["changes"]["items"]["properties"]["action"]["enum"]
            .as_array()
            .expect("the action property is an enum");
        let names: Vec<&str> = actions.iter().filter_map(|a| a.as_str()).collect();
        assert_eq!(names, vec!["add", "complete", "update", "archive"]);
    }

    #[test]
    fn every_verb_has_valid_json_schema() {
        for verb in [
            Verb::Query,
            Verb::Summarize,
            Verb::Explain,
            Verb::Act,
            Verb::Breakdown,
            Verb::Scan,
            Verb::Manage,
        ] {
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(verb.schema());
            assert!(parsed.is_ok(), "{} schema is valid JSON", verb.label());
        }
    }
```

Delete the old `every_verb_has_valid_json_schema` test — the version above replaces it and covers all seven verbs.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet agent 2>&1 | tail -12`
Expected: `no variant named Manage found for enum Verb`.

- [ ] **Step 3: Rename the schema and widen its action enum**

In `src/agent/changeset.rs`, rename the constant and add `"archive"`:

```rust
/// Schema handed to the agent for the change-set verbs.
pub const CHANGE_SCHEMA: &str = r#"{"type":"object","properties":{"changes":{"type":"array","items":{"type":"object","properties":{"file":{"type":"string"},"action":{"type":"string","enum":["add","complete","update","archive"]},"section":{"type":"string"},"heading":{"type":"string"},"content":{"type":"string"},"reason":{"type":"string"}},"required":["file","action","content","reason"]}},"summary":{"type":"string"}},"required":["changes","summary"]}"#;
```

Update the two references: `changeset::SCAN_SCHEMA` in `Verb::schema` becomes `changeset::CHANGE_SCHEMA`, and the `pub use` line if it names it.

- [ ] **Step 4: Add the verb**

In `src/agent/mod.rs`, add to `enum Verb` after `Scan`:

```rust
    /// Carry out an instruction across the workspace as a change-set. Writes,
    /// after review.
    Manage,
```

Add the three match arms:

```rust
            Verb::Manage => "manage",
```

```rust
    pub fn writes(self) -> bool {
        matches!(self, Verb::Breakdown | Verb::Scan | Verb::Manage)
    }
```

```rust
            Verb::Scan | Verb::Manage => changeset::CHANGE_SCHEMA,
```

And the prompt:

```rust
            Verb::Manage => {
                "Carry out the instruction below against the todo files, as a change-set.\n\
                 Use \"add\" for new items, \"complete\" for ones that are finished, \
                 \"update\" to reword one, and \"archive\" to move one out of the working \
                 file. Prefer \"archive\" over \"complete\" when the item should stop \
                 appearing at all. Change nothing the instruction did not ask for.\n\n\
                 Reply with JSON only. Each change names the file it belongs to, using the \
                 workspace-relative path exactly as shown below, and gives a one-line \
                 reason.\n\n{files}\n\nInstruction: {input}"
            }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --quiet agent 2>&1 | tail -8 && cargo clippy --all-targets 2>&1 | tail -3`
Expected: pass, clippy silent. `src/ui/mod.rs`'s `interpret` may warn about a non-exhaustive match on `Verb`; add `Verb::Manage` alongside `Verb::Scan` there now (both parse a `ChangeSet`).

- [ ] **Step 6: Commit**

```bash
git add src/agent/mod.rs src/agent/changeset.rs src/ui/mod.rs
git commit -m "feat(agent): a manage verb that turns an instruction into a change-set"
```

---

### Task 4: `archive_items`, extracted from `archive_done`

**Files:**
- Modify: `src/store/archive.rs:64-116` (`archive_done`)
- Test: `src/store/archive.rs` `mod tests`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `store::archive::archive_items(todo_file: &Path, archive_dir: &Path, targets: &[&Item], date: &str) -> Result<ArchiveReport, WriteError>`. `archive_done` keeps its signature and behaviour exactly.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/store/archive.rs`. Do **not** modify any existing test in this file.

```rust
    // The agent path archives one named item, whatever its state — unlike `X`,
    // which only sweeps finished ones.
    #[test]
    fn one_named_open_item_is_moved_and_its_siblings_are_left() {
        let (_dir, todo, archive, items) = workspace("## P0\n\n- [ ] alpha\n- [ ] beta\n");
        let target: Vec<&Item> = items.iter().filter(|i| i.text == "alpha").collect();
        assert_eq!(target.len(), 1, "fixture has one alpha");

        let report = archive_items(&todo, &archive, &target, "2026-08-11").unwrap();
        assert_eq!(report.archived, 1);

        let left = std::fs::read_to_string(&todo).unwrap();
        assert!(!left.contains("alpha"), "moved out: {left}");
        assert!(left.contains("beta"), "sibling untouched: {left}");

        let moved = std::fs::read_to_string(archive.join("TODO.md")).unwrap();
        assert!(moved.contains("## Archived 2026-08-11"));
        assert!(moved.contains("- [ ] alpha"), "verbatim: {moved}");
    }

    #[test]
    fn an_items_subtree_travels_with_it() {
        let body = "## P0\n\n- [ ] parent\n  > why it matters\n  - [ ] child\n- [ ] other\n";
        let (_dir, todo, archive, items) = workspace(body);
        let target: Vec<&Item> = items.iter().filter(|i| i.text == "parent").collect();

        archive_items(&todo, &archive, &target, "2026-08-11").unwrap();

        let left = std::fs::read_to_string(&todo).unwrap();
        assert!(!left.contains("child") && !left.contains("why it matters"), "{left}");
        assert!(left.contains("other"));
        let moved = std::fs::read_to_string(archive.join("TODO.md")).unwrap();
        assert!(moved.contains("child") && moved.contains("why it matters"), "{moved}");
    }

    #[test]
    fn an_item_whose_line_moved_on_disk_is_skipped_not_guessed_at() {
        let (_dir, todo, archive, items) = workspace("## P0\n\n- [ ] alpha\n- [ ] beta\n");
        let target: Vec<&Item> = items.iter().filter(|i| i.text == "alpha").collect();
        std::fs::write(&todo, "## P0\n\n- [ ] something else entirely\n").unwrap();

        let report = archive_items(&todo, &archive, &target, "2026-08-11").unwrap();
        assert_eq!(report.archived, 0);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].contains("file changed on disk"), "{:?}", report.skipped);
    }

    #[test]
    fn archiving_nothing_writes_nothing() {
        let (_dir, todo, archive, _items) = workspace("## P0\n\n- [ ] alpha\n");
        let before = std::fs::read_to_string(&todo).unwrap();
        let report = archive_items(&todo, &archive, &[], "2026-08-11").unwrap();
        assert_eq!(report.archived, 0);
        assert_eq!(std::fs::read_to_string(&todo).unwrap(), before);
        assert!(!archive.join("TODO.md").exists(), "no empty archive file");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet archive 2>&1 | tail -12`
Expected: `cannot find function archive_items in this scope`.

- [ ] **Step 3: Extract the mover**

In `src/store/archive.rs`, replace the body of `archive_done` and add `archive_items`:

```rust
/// Move finished items from `todo_file` into `archive_dir/TODO.md`.
///
/// A done item whose subtree still contains open work is left alone and
/// reported, since archiving it would hide those open items.
pub fn archive_done(
    todo_file: &Path,
    archive_dir: &Path,
    items: &[Item],
    date: &str,
) -> Result<ArchiveReport, WriteError> {
    let (lines, _, _) = read_lines(todo_file)?;
    let mut report = ArchiveReport::default();

    let mut candidates: Vec<&Item> = items
        .iter()
        .filter(|i| i.file == todo_file && i.done && i.parent.is_none())
        .collect();
    candidates.sort_by_key(|i| i.line);

    let mut targets: Vec<&Item> = Vec::new();
    for item in candidates {
        // A moved line fails `verify` inside archive_items; this guard only
        // needs the lines it can still trust.
        if item.line < lines.len() && !wholly_done(&lines, item.line..subtree_end(&lines, item.line))
        {
            report
                .skipped
                .push(format!("{:?}: has open sub-items", item.text));
            continue;
        }
        targets.push(item);
    }

    let moved = archive_items(todo_file, archive_dir, &targets, date)?;
    report.archived = moved.archived;
    report.skipped.extend(moved.skipped);
    Ok(report)
}

/// Move each named item, and everything under it, into `archive_dir/TODO.md`.
///
/// Unconditional on state: the caller decides what deserves archiving. An item
/// whose line no longer matches the file is skipped and reported rather than
/// guessed at.
pub fn archive_items(
    todo_file: &Path,
    archive_dir: &Path,
    targets: &[&Item],
    date: &str,
) -> Result<ArchiveReport, WriteError> {
    let (mut lines, ending, trailing) = read_lines(todo_file)?;
    let mut report = ArchiveReport::default();

    let mut ordered: Vec<&&Item> = targets.iter().collect();
    ordered.sort_by_key(|i| i.line);

    let mut moved: Vec<(usize, usize)> = Vec::new();
    let mut block: Vec<String> = Vec::new();

    for item in ordered {
        if verify(todo_file, &lines, item.line, &item.raw).is_err() {
            report
                .skipped
                .push(format!("{:?}: file changed on disk", item.text));
            continue;
        }
        let end = subtree_end(&lines, item.line);
        block.extend_from_slice(&lines[item.line..end]);
        moved.push((item.line, end));
        report.archived += 1;
    }

    if moved.is_empty() {
        return Ok(report);
    }

    append_to_archive(archive_dir, &block, date, ending.as_str())?;

    // Remove bottom-up so earlier ranges keep their indices.
    for (start, end) in moved.into_iter().rev() {
        lines.drain(start..end);
    }
    write_lines(todo_file, &lines, ending, trailing)?;

    Ok(report)
}
```

- [ ] **Step 4: Run the whole archive suite**

Run: `cargo test --quiet archive 2>&1 | tail -10`
Expected: the four new tests pass **and** every pre-existing `archive_done` test passes with no edit. If any existing test needed changing, the extraction changed behaviour — revert and redo.

- [ ] **Step 5: Confirm the existing tests were not touched**

Run: `git diff --stat src/store/archive.rs && git diff src/store/archive.rs | grep -c '^-.*fn archive'`
Expected: only `archive_done`'s body appears as removed lines; no existing `#[test]` body appears in the deletions.

- [ ] **Step 6: Commit**

```bash
git add src/store/archive.rs
git commit -m "refactor(store): archive_items moves named items; archive_done picks them"
```

---

### Task 5: `ChangeAction::Archive` and the archive branch in `apply`

**Files:**
- Modify: `src/agent/changeset.rs:16-32` (`ChangeAction`), `:104-146` (`apply`, `apply_add`)
- Test: `src/agent/changeset.rs` `mod tests`

**Interfaces:**
- Consumes: `archive_items` from Task 4.
- Produces: `ChangeAction::Archive` with `glyph() == "→ archive"`; `changeset::apply(root: &Path, archive_dirs: &HashMap<PathBuf, PathBuf>, today: &str, items: &[Item], set: &ChangeSet) -> ApplyReport`; `ChangeSet::open_sub_items(&self, index: usize, items: &[Item]) -> usize`.

**Spec correction.** The spec's § 6 gives `apply(root, archive_dir: Option<&Path>, today, items, set)`. That is wrong: `archive_dir` lives on `Group` (`src/store/model.rs:122`), so it is **per group**, and one change-set can span groups. The parameter is therefore a map from todo-file path to that group's archive directory. A file missing from the map has no archive directory configured.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/agent/changeset.rs`:

```rust
    use std::collections::HashMap;

    fn archive_map(todo: &Path, dir: &Path) -> HashMap<PathBuf, PathBuf> {
        HashMap::from([(todo.to_path_buf(), dir.to_path_buf())])
    }

    #[test]
    fn archive_parses_as_an_action() {
        let parsed = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"alpha","reason":"closed"}]}"#);
        assert_eq!(parsed.changes[0].action, ChangeAction::Archive);
        assert_eq!(ChangeAction::Archive.glyph(), "→ archive");
    }

    #[test]
    fn applying_an_archive_change_moves_the_item_out_of_the_file() {
        let (dir, path, items) = workspace(DOC);
        let archive = dir.path().join("_archive");
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"alpha","reason":"closed"}]}"#);

        let report = apply(
            dir.path(),
            &archive_map(&path, &archive),
            "2026-08-11",
            &items,
            &changes,
        );
        assert_eq!(report.applied, 1, "{:?}", report.skipped);

        let left = std::fs::read_to_string(&path).unwrap();
        assert!(!left.contains("alpha"), "{left}");
        assert!(left.contains("beta"), "sibling untouched: {left}");
        let moved = std::fs::read_to_string(archive.join("TODO.md")).unwrap();
        assert!(moved.contains("alpha") && moved.contains("2026-08-11"), "{moved}");
    }

    #[test]
    fn an_archive_change_naming_no_item_is_skipped() {
        let (dir, path, items) = workspace(DOC);
        let archive = dir.path().join("_archive");
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"nothing like this","reason":"r"}]}"#);

        let report = apply(dir.path(), &archive_map(&path, &archive), "2026-08-11", &items, &changes);
        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].contains("no matching item"), "{:?}", report.skipped);
    }

    // The rest of a change-set is still worth applying when archiving cannot run.
    #[test]
    fn without_an_archive_dir_the_archive_is_skipped_and_the_rest_applies() {
        let (dir, path, items) = workspace(DOC);
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"alpha","reason":"r"},
            {"file":"lefv.md","action":"complete","content":"alpha","reason":"r"}]}"#);

        let report = apply(dir.path(), &HashMap::new(), "2026-08-11", &items, &changes);
        assert_eq!(report.applied, 1, "the complete still ran");
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].contains("no archive_dir configured"), "{:?}", report.skipped);
        let _ = path;
    }

    #[test]
    fn an_archive_row_reports_how_much_open_work_it_carries() {
        let body = "## P0\n\n- [ ] parent\n  - [ ] one\n  - [x] two\n";
        let (_dir, _path, items) = workspace(body);
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"parent","reason":"r"}]}"#);
        assert_eq!(changes.open_sub_items(0, &items), 1);
    }

    #[test]
    fn a_non_archive_row_carries_no_open_sub_item_count() {
        let (_dir, _path, items) = workspace(DOC);
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"complete","content":"alpha","reason":"r"}]}"#);
        assert_eq!(changes.open_sub_items(0, &items), 0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet changeset 2>&1 | tail -12`
Expected: `no variant named Archive`, and `apply` takes three arguments.

- [ ] **Step 3: Implement**

In `src/agent/changeset.rs`, add the variant and glyph:

```rust
pub enum ChangeAction {
    Add,
    Complete,
    Update,
    Archive,
}
```

```rust
            ChangeAction::Archive => "→ archive",
```

Add the open-sub-item count on `impl ChangeSet`:

```rust
    /// How many unfinished sub-items an archive change would carry out of the
    /// working file. Zero for every other action.
    pub fn open_sub_items(&self, index: usize, items: &[Item]) -> usize {
        let Some(change) = self.changes.get(index) else {
            return 0;
        };
        if change.action != ChangeAction::Archive {
            return 0;
        }
        let Some(parent) = items
            .iter()
            .find(|i| i.text.trim().eq_ignore_ascii_case(change.content.trim()))
        else {
            return 0;
        };
        items
            .iter()
            .filter(|i| i.parent == Some(parent.line) && !i.done)
            .count()
    }
```

Replace `apply` and add `apply_archive`:

```rust
/// Apply a reviewed change-set.
///
/// `complete`, `update` and `archive` locate their target by matching item text
/// within the named file, so a change-set stays valid even if line numbers moved
/// since the agent read the workspace. Anything that cannot be located is
/// skipped and reported rather than guessed at.
pub fn apply(
    root: &Path,
    archive_dirs: &HashMap<PathBuf, PathBuf>,
    today: &str,
    items: &[Item],
    set: &ChangeSet,
) -> ApplyReport {
    let mut report = ApplyReport::default();

    for change in &set.changes {
        let path: PathBuf = root.join(&change.file);
        let result = match change.action {
            ChangeAction::Add => apply_add(&path, items, change),
            ChangeAction::Complete => apply_complete(&path, items, change),
            ChangeAction::Update => apply_update(&path, items, change),
            ChangeAction::Archive => apply_archive(&path, archive_dirs, today, items, change),
        };
        match result {
            Ok(()) => report.applied += 1,
            Err(reason) => report.skipped.push(reason),
        }
    }
    report
}

fn apply_archive(
    path: &Path,
    archive_dirs: &HashMap<PathBuf, PathBuf>,
    today: &str,
    items: &[Item],
    change: &Change,
) -> Result<(), String> {
    let Some(item) = find(items, path, &change.content) else {
        return Err(format!("archive {:?}: no matching item", change.content));
    };
    let Some(archive_dir) = archive_dirs.get(path) else {
        return Err(format!(
            "archive {:?}: no archive_dir configured for {}",
            item.text,
            path.display()
        ));
    };
    let report = store::archive_items(path, archive_dir, &[item], today)
        .map_err(|e| describe(e, "archive", &item.text))?;
    match report.skipped.first() {
        Some(reason) => Err(format!("archive {:?}: {reason}", item.text)),
        None => Ok(()),
    }
}
```

Add `use std::collections::HashMap;` at the top of the file, and export `archive_items` from `src/store/mod.rs` alongside `archive_done` if it is not already re-exported.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --quiet changeset 2>&1 | tail -8 && cargo clippy --all-targets 2>&1 | tail -3`
Expected: pass. `src/ui/mod.rs:1137` still calls the old three-argument `apply`; fix it in Task 7. If the build blocks, pass `&HashMap::new()` and today's date there as a placeholder and complete it in Task 7.

- [ ] **Step 5: Commit**

```bash
git add src/agent/changeset.rs src/store/mod.rs src/ui/mod.rs
git commit -m "feat(agent): an archive action that moves an item out of the working file"
```

---

### Task 6: The service picker

**Files:**
- Modify: `src/ui/mod.rs:145-169` (`Mode`), `:333-341` (`App` fields), `:1266-1350` (`handle_normal_key`), `:1960-1999` (`spawn_agent`), `:444-466` (`persist_ui_state`)
- Modify: `src/ui/view.rs:606-700` (top bar, menus)
- Test: `src/ui/mod.rs` `mod tests`

**Interfaces:**
- Consumes: `Config::services`, `Config::active_service`, `ActiveService` (Task 1); `agent::run(service, …)` (Task 2).
- Produces: `Mode::ServiceMenu`; `App.service_cursor: usize`; `App.service: Option<ServiceConfig>`; `App::select_service(&mut self, index: usize)`; `view::service_tab_label(app) -> String`; `view::service_tab_rect`, `view::service_menu_rect` for mouse hit-testing.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/ui/mod.rs`, following the existing helper pattern in that module for building an `App`:

```rust
    fn app_with_services() -> App {
        let mut config = Config::default();
        config.service_list = vec![
            crate::config::ServiceConfig {
                name: "claude".to_string(),
                command: vec!["echo".to_string()],
                schema_mode: crate::config::SchemaMode::Flag,
                schema_flag: Some("--json-schema".to_string()),
                timeout_secs: 600,
            },
            crate::config::ServiceConfig {
                name: "ollama".to_string(),
                command: vec!["echo".to_string()],
                schema_mode: crate::config::SchemaMode::Prompt,
                schema_flag: None,
                timeout_secs: 300,
            },
        ];
        App::new(
            Workspace {
                root: PathBuf::from("/w"),
                groups: vec![group("a")],
                items: vec![item("a", "a-open", false)],
            },
            config,
        )
    }

    #[test]
    fn m_opens_the_service_picker_on_the_active_service() {
        let mut app = app_with_services();
        app.config.ui.service = Some("ollama".to_string());
        app.service = app.config.active_service().service;
        press(&mut app, KeyCode::Char('m'));
        assert_eq!(app.mode, Mode::ServiceMenu);
        assert_eq!(app.service_cursor, 1, "the cursor starts on the active one");
    }

    #[test]
    fn selecting_in_the_picker_switches_the_active_service() {
        let mut app = app_with_services();
        press(&mut app, KeyCode::Char('m'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.service.as_ref().unwrap().name, "ollama");
        assert_eq!(app.config.ui.service.as_deref(), Some("ollama"));
    }

    #[test]
    fn esc_leaves_the_picker_without_switching() {
        let mut app = app_with_services();
        press(&mut app, KeyCode::Char('m'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.service.as_ref().unwrap().name, "claude");
    }

    #[test]
    fn the_picker_says_so_when_no_service_is_configured() {
        let mut app = app();
        press(&mut app, KeyCode::Char('m'));
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.notice.as_deref().unwrap_or_default().contains("no agent configured"));
    }

    #[test]
    fn the_top_bar_names_the_active_service() {
        let app = app_with_services();
        assert!(crate::ui::view::service_tab_label(&app).contains("claude"));
    }

    #[test]
    fn an_unknown_configured_service_is_reported_once_at_startup() {
        let mut app = app_with_services();
        app.config.ui.service = Some("gpt5".to_string());
        let active = app.config.active_service();
        app.service = active.service;
        app.notice = active.notice;
        assert_eq!(app.service.as_ref().unwrap().name, "claude");
        assert!(app.notice.as_deref().unwrap().contains("gpt5"));
    }
```

The helpers `app()`, `press(&mut app, code)`, `item()` and `group()` already exist in this module (`src/ui/mod.rs:2181-2225`). Use them; add no new helpers beyond `app_with_services`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet ui:: 2>&1 | tail -15`
Expected: `no variant named ServiceMenu`, no field `service_cursor`, no function `service_tab_label`.

- [ ] **Step 3: Add the mode, the state, and the key**

`Mode`, after `ViewMenu`:

```rust
    /// The service picker is open.
    ServiceMenu,
```

`App` fields, beside `busy`:

```rust
    pub service: Option<ServiceConfig>,
    pub service_cursor: usize,
```

Initialise in the constructor: `service: config.active_service().service`, `service_cursor: 0`. Where the constructor already sets `notice`, seed it from `config.active_service().notice` so an unknown name is reported once.

Key in `handle_normal_key`, next to the `v` arm:

```rust
            (K::Char('m'), KeyModifiers::NONE) => self.open_service_menu(),
```

Route the mode in `handle_key`:

```rust
            Mode::ServiceMenu => self.handle_service_key(key),
```

Handlers:

```rust
    fn open_service_menu(&mut self) {
        let services = self.config.services();
        if services.is_empty() {
            self.notice = Some("no agent configured (set [[services]])".to_string());
            return;
        }
        self.service_cursor = self
            .service
            .as_ref()
            .and_then(|active| services.iter().position(|s| s.name == active.name))
            .unwrap_or(0);
        self.mode = Mode::ServiceMenu;
    }

    fn handle_service_key(&mut self, key: KeyEvent) {
        use KeyCode as K;
        let last = self.config.services().len().saturating_sub(1);
        match key.code {
            K::Esc | K::Char('q') | K::Char('m') => self.mode = Mode::Normal,
            K::Char('j') | K::Down => self.service_cursor = (self.service_cursor + 1).min(last),
            K::Char('k') | K::Up => self.service_cursor = self.service_cursor.saturating_sub(1),
            K::Enter | K::Char(' ') => self.select_service(self.service_cursor),
            _ => self.mode = Mode::Normal,
        }
    }

    pub fn select_service(&mut self, index: usize) {
        self.mode = Mode::Normal;
        let Some(chosen) = self.config.services().get(index).cloned() else {
            return;
        };
        self.notice = Some(format!("service: {}", chosen.name));
        self.config.ui.service = Some(chosen.name.clone());
        self.service = Some(chosen);
    }
```

- [ ] **Step 4: Persist the choice and use it when spawning**

In `persist_ui_state`, carry the selected name:

```rust
            service: self.config.ui.service.clone(),
```

In `spawn_agent`, replace the `config.agent` reads with the active service:

```rust
        let Some(service) = self.service.clone() else {
            self.notice = Some("no agent configured (set [[services]])".to_string());
            return;
        };
```

and the thread body:

```rust
            let result = agent::run(&service, verb.schema(), &prompt, &root, &cancel);
            let event = match result {
                Err(err) => Event::TaskFinished {
                    title: format!("{} failed", verb.label()),
                    body: format!("{}: {err}", service.name),
                },
                Ok(json) => interpret(verb, &json),
            };
```

`cancel` is added in Task 8; for now pass `&Arc::new(AtomicBool::new(false))`.

- [ ] **Step 5: Draw the tab and the dropdown**

In `src/ui/view.rs`, beside `view_tab_label` / `view_tab_rect` / `view_menu_rect` / `render_view_menu`, add the service equivalents:

```rust
pub fn service_tab_label(app: &App) -> String {
    match &app.service {
        Some(service) => format!(" {} ▾ ", service.name),
        None => " no model ▾ ".to_string(),
    }
}

/// Where the service tab was drawn, for mouse hit-testing.
pub fn service_tab_rect(top_bar: Rect) -> Rect {
    let view = view_tab_rect(top_bar);
    let width = 14u16.min(top_bar.width);
    Rect {
        x: view.x.saturating_sub(width),
        y: top_bar.y,
        width,
        height: 1,
    }
}

/// Where the service menu is drawn, so clicks can be routed to its entries.
pub fn service_menu_rect(app: &App, top_bar: Rect) -> Rect {
    let tab = service_tab_rect(top_bar);
    Rect {
        x: tab.x,
        y: top_bar.y + 1,
        width: 30u16.min(top_bar.width),
        height: app.config.services().len() as u16 + 2,
    }
}
```

Render the label in `render_top_bar` immediately before the `view ▾` span, highlighted when `app.mode == Mode::ServiceMenu`, using the same `theme.selected(&theme.statusbar())` / `theme.header()` pair the view tab uses. Add a `render_service_menu` modelled line-for-line on `render_view_menu`, listing `app.config.services()` by `name` with `▸` on `app.service_cursor`, and call it from the same place `render_view_menu` is called, guarded by `Mode::ServiceMenu`.

Route the mouse: wherever `view_tab_rect` is hit-tested in `src/ui/mod.rs`, add the same treatment for `service_tab_rect` (open the menu) and `service_menu_rect` (`select_service(row)`).

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --quiet 2>&1 | tail -8 && cargo clippy --all-targets 2>&1 | tail -3`
Expected: whole suite green, clippy silent.

- [ ] **Step 7: Commit**

```bash
git add src/ui/mod.rs src/ui/view.rs
git commit -m "feat(ui): a picker for the active model service, remembered between runs"
```

---

### Task 7: The manage popup, and archive rows in the review pane

**Files:**
- Modify: `src/ui/mod.rs:996-1005` (`begin_review`), `:1122-1141` (`apply_review`), `handle_normal_key`
- Modify: `src/ui/view.rs` (review row rendering)
- Test: `src/ui/mod.rs` `mod tests`

**Interfaces:**
- Consumes: `Verb::Manage` (Task 3); `ChangeAction::Archive`, `apply(root, archive_dirs, today, items, set)`, `ChangeSet::open_sub_items` (Task 5).
- Produces: `App::archive_dirs(&self) -> HashMap<PathBuf, PathBuf>`; review selection defaults that exclude archive rows.

- [ ] **Step 1: Write the failing tests**

```rust
    fn archive_change_set() -> crate::agent::ChangeSet {
        crate::agent::ChangeSet::parse(
            r#"{"summary":"s","changes":[
                {"file":"lefv.md","action":"add","content":"new","reason":"r"},
                {"file":"lefv.md","action":"archive","content":"alpha","reason":"r"},
                {"file":"lefv.md","action":"complete","content":"beta","reason":"r"}]}"#,
        )
        .unwrap()
    }

    // A move out of the working file is opted into, never opted out of.
    #[test]
    fn archive_rows_start_unticked_and_the_rest_start_ticked() {
        let mut app = app();
        app.begin_review(Pending::Changes(archive_change_set()));
        assert_eq!(app.review_selected, vec![true, false, true]);
    }

    #[test]
    fn applying_a_review_with_only_archive_rows_left_says_nothing_was_selected() {
        let mut app = app();
        let only_archive = crate::agent::ChangeSet::parse(
            r#"{"summary":"s","changes":[
                {"file":"lefv.md","action":"archive","content":"alpha","reason":"r"}]}"#,
        )
        .unwrap();
        app.begin_review(Pending::Changes(only_archive));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.notice.as_deref().unwrap_or_default().contains("nothing selected"));
    }

    #[test]
    fn space_can_still_tick_an_archive_row() {
        let mut app = app();
        app.begin_review(Pending::Changes(archive_change_set()));
        app.review_cursor = 1;
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(app.review_selected[1], true);
    }

    #[test]
    fn m_uppercase_opens_the_manage_prompt() {
        let mut app = app_with_services();
        press(&mut app, KeyCode::Char('M'));
        assert_eq!(app.mode, Mode::AskingAgent(Verb::Manage));
    }

    #[test]
    fn the_manage_prompt_needs_a_configured_service() {
        let mut app = app();
        press(&mut app, KeyCode::Char('M'));
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.notice.as_deref().unwrap_or_default().contains("no agent configured"));
    }

    #[test]
    fn archive_dirs_maps_each_groups_todo_file_to_its_archive() {
        let app = app();
        let map = app.archive_dirs();
        for group in &app.workspace.groups {
            match &group.archive_dir {
                Some(dir) => assert_eq!(map.get(&group.todo_file), Some(dir)),
                None => assert!(!map.contains_key(&group.todo_file)),
            }
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet ui:: 2>&1 | tail -15`
Expected: `review_selected` is all `true`; no method `archive_dirs`; `M` does nothing.

- [ ] **Step 3: Default archive rows to unticked**

Replace `begin_review`:

```rust
    /// Open the review list over whatever was proposed.
    fn begin_review(&mut self, pending: Pending) {
        // Everything starts picked — unpicking is easier than picking from
        // nothing — except a move out of the working file, which is opted into.
        self.review_selected = (0..pending.len())
            .map(|index| !pending.is_archive(index))
            .collect();
        self.review_cursor = 0;
        self.review_scroll = 0;
        self.pending = Some(pending);
        self.mode = Mode::ReviewingChangeSet;
    }
```

Add to `impl Pending`, beside its existing `len` / `summary` / `row` / `reason` arms:

```rust
    fn is_archive(&self, index: usize) -> bool {
        match self {
            Pending::Changes(set) => set
                .changes
                .get(index)
                .is_some_and(|c| c.action == ChangeAction::Archive),
            Pending::SubItems { .. } => false,
        }
    }
```

- [ ] **Step 4: Wire the key and the apply call**

In `handle_normal_key`, next to the `R` arm:

```rust
            (K::Char('M'), _) => self.begin_ask(Verb::Manage),
```

`begin_ask` already refuses when no service is configured if it checks `self.service`; if it does not, add the same guard `spawn_agent` uses so `M` on an unconfigured mitodo reports rather than opening a prompt that cannot be sent.

Add the archive-directory map and use it in `apply_review`:

```rust
    /// Each group's todo file mapped to its archive directory, for change-sets
    /// that span groups.
    pub fn archive_dirs(&self) -> std::collections::HashMap<PathBuf, PathBuf> {
        self.workspace
            .groups
            .iter()
            .filter_map(|g| g.archive_dir.clone().map(|dir| (g.todo_file.clone(), dir)))
            .collect()
    }
```

```rust
            Pending::Changes(set) => {
                let picked = set.selected(&self.review_selected);
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                let report = agent::changeset::apply(
                    &self.workspace.root,
                    &self.archive_dirs(),
                    &today,
                    &self.workspace.items,
                    &picked,
                );
                (report.applied, report.skipped)
            }
```

- [ ] **Step 5: Label open sub-items on archive rows**

Wherever the review pane renders a row (the call to `pending.row(index)` in `src/ui/view.rs`), append the count when it is non-zero:

```rust
    let mut text = pending.row(index);
    if let Pending::Changes(set) = pending {
        let open = set.open_sub_items(index, &app.workspace.items);
        if open > 0 {
            text.push_str(&format!("  ({open} open sub-items)"));
        }
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --quiet 2>&1 | tail -8 && cargo clippy --all-targets 2>&1 | tail -3`
Expected: whole suite green, clippy silent.

- [ ] **Step 7: Commit**

```bash
git add src/ui/mod.rs src/ui/view.rs
git commit -m "feat(ui): a manage prompt, with archive rows opted into at review"
```

---

### Task 8: Cancelling a running call with `esc`

**Files:**
- Modify: `src/ui/mod.rs` (`App` fields, `spawn_agent`, `handle_normal_key`)
- Test: `src/ui/mod.rs` `mod tests`

**Interfaces:**
- Consumes: `agent::run(…, cancel: &Arc<AtomicBool>)`, `AgentError::Cancelled` (Task 2).
- Produces: `App.cancel: Arc<AtomicBool>`; `App::cancel_agent(&mut self)`.

**Spec correction.** The spec's § 7 says the kill handle "lives in `App` behind a mutex". A `std::process::Child` cannot be shared that way without fighting `wait_with_output`, which consumes it. The polling loop in `run` already owns the child and already wakes every 100 ms, so an `Arc<AtomicBool>` it checks on each tick achieves the same outcome with no shared ownership. Cancellation latency is one poll interval.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn esc_while_busy_requests_cancellation_and_clears_the_spinner() {
        let mut app = app_with_services();
        app.busy = Some(Busy::new("manage"));
        press(&mut app, KeyCode::Esc);
        assert!(app.busy.is_none(), "the spinner goes away at once");
        assert!(app.cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert!(app.notice.as_deref().unwrap_or_default().contains("cancelled"));
    }

    // Without a fresh flag per call, one cancel would kill every later call.
    #[test]
    fn a_new_call_gets_an_uncancelled_flag() {
        let mut app = app_with_services();
        app.busy = Some(Busy::new("manage"));
        press(&mut app, KeyCode::Esc);
        app.spawn_agent(Verb::Summarize, String::new());
        assert!(!app.cancel.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn esc_when_not_busy_still_clears_the_query() {
        let mut app = app_with_services();
        app.set_query("pri:P0").unwrap();
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.query_input, "");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --quiet ui:: 2>&1 | tail -12`
Expected: no field `cancel` on `App`.

- [ ] **Step 3: Implement**

`App` field, beside `busy`:

```rust
    pub cancel: Arc<AtomicBool>,
```

Initialise with `cancel: Arc::new(AtomicBool::new(false))` and add `use std::sync::Arc; use std::sync::atomic::{AtomicBool, Ordering};` to the imports.

In `spawn_agent`, hand each call its own flag:

```rust
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
```

and pass `&cancel` to `agent::run` inside the thread.

Change the `Esc` arm of `handle_normal_key`:

```rust
            (K::Esc, _) => {
                if self.busy.is_some() {
                    self.cancel_agent();
                } else {
                    self.clear_query();
                }
            }
```

```rust
    fn cancel_agent(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let label = self.busy.take().map(|b| b.label).unwrap_or_default();
        self.notice = Some(format!("{label} cancelled"));
    }
```

The spinner clears immediately rather than waiting for the thread, so the UI never looks stuck. The thread's late `Cancelled` result must not reopen a modal: in the `Event::TaskFinished` handler, ignore a result whose body ends in `cancelled` when `self.busy.is_none()`. Implement that as a guard in the handler, not by dropping the event.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --quiet 2>&1 | tail -8 && cargo clippy --all-targets 2>&1 | tail -3`
Expected: whole suite green.

- [ ] **Step 5: Commit**

```bash
git add src/ui/mod.rs
git commit -m "feat(ui): esc cancels a running agent call"
```

---

### Task 9: Documentation, help, and the full gate

**Files:**
- Modify: `README.md` (agent config section, keys table)
- Modify: `src/ui/mod.rs` (`help_lines`)

**Interfaces:**
- Consumes: everything above.
- Produces: no code interfaces.

- [ ] **Step 1: Add the new keys to the in-app help**

In `help_lines()`, beside the existing agent verbs:

```rust
        "m  pick model service        M  manage items with the agent",
        "esc  cancel a running agent call",
```

Match the existing formatting in that function exactly rather than introducing a new column layout.

- [ ] **Step 2: Update the README's agent section**

Replace the `[agent]` config block with the `[[services]]` form, keeping the existing prose voice. Include all three services as shown in the spec's § 4, and state the back-compat rule in one sentence: a config with `[agent]` and no `[[services]]` keeps working as a single service named `default`.

Add to the keys table:

| `m` | pick model service | `M` | manage items with the agent |

Document the archive action in the change-set paragraph: an agent may propose `archive`, which moves the item into `<archive_dir>/TODO.md` exactly as `X` does, and those rows arrive unticked in the review pane.

- [ ] **Step 3: Run the full gate**

Run: `cargo test 2>&1 | grep -E '^test result' && cargo clippy --all-targets 2>&1 | tail -3 && cargo fmt --check && echo FMT-OK`
Expected: one `test result: ok` line with 0 failed and 0 ignored, clippy silent, formatting clean.

- [ ] **Step 4: Verify the comment budget**

Run:
```bash
git diff main --stat | tail -1
comments=$(git diff main -U0 | grep -cE '^\+\s*//')
added=$(git diff main -U0 | grep -cE '^\+')
echo "$comments comment lines of $added added"
```
Expected: under ~10%. Over that, re-read every added comment and delete anything that is narrative rather than a hidden constraint.

- [ ] **Step 5: Smoke-test against a real service**

```bash
cargo install --path .
mitodo   # press m — the picker lists claude, codex, ollama
         # select one, press M, type: add a P2 to test the manage popup
         # confirm the review pane opens, the add row is ticked, y applies it
```
Expected: the item appears in the workspace file. Undo it by hand afterwards, or point `workspace.root` at a scratch directory for the test.

- [ ] **Step 6: Commit**

```bash
git add README.md src/ui/mod.rs
git commit -m "docs: [[services]] config, the m and M keys, and the archive action"
```

---

## Self-review

**Spec coverage.** Every section maps to a task: § 3 architecture → Tasks 1–8; § 4 config and back-compat → Task 1; § 4 schema modes → Task 2; § 5 picker → Task 6; § 5 manage popup and review defaults → Task 7; § 6 archive action and store refactor → Tasks 4–5; § 7 error handling → Tasks 1 (fallback notice), 2 (env strip, temp file, cancelled), 6 (service-prefixed failure), 8 (cancel); § 8 testing → the test steps of every task; § 10 files touched → all nine tasks, plus README in Task 9.

**Two spec corrections**, both recorded at the head of their task: `apply` takes a per-group `HashMap` rather than a single `archive_dir` (Task 5), and cancellation uses an `AtomicBool` rather than a shared `Child` behind a mutex (Task 8).

**Type consistency.** `ServiceConfig` fields are named identically in Tasks 1, 2 and 6. `agent::run`'s five parameters are in the same order everywhere. `CHANGE_SCHEMA` replaces `SCAN_SCHEMA` in Task 3 and is never referenced by the old name afterwards. `apply`'s five parameters match between Task 5's definition and Task 7's call. `open_sub_items(index, items)` has the same signature in Tasks 5 and 7. `archive_items(todo_file, archive_dir, targets, date)` matches between Tasks 4 and 5.

**One known ordering hazard.** Tasks 2, 3 and 5 each change a signature that `src/ui/mod.rs` calls, so the build is briefly red between tasks if `src/ui/mod.rs` is not patched in the same commit. Each of those tasks says so and names the minimal placeholder to keep the build green; Tasks 6–8 replace the placeholders with the real wiring. Do not skip a task's `cargo test` step on the grounds that "Task 6 will fix it".
