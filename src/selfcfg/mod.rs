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
    let exe =
        std::env::current_exe().map_err(|e| format!("cannot determine this binary's path: {e}"))?;
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
        let (head, rest) = line.trim().split_once(':')?;
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
                target: SERVER_NAME.to_string(),
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
    target::detect()
        .iter()
        .map(|t| unregister(t, dry_run))
        .collect()
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

    if existing.as_deref() == Some(entry.command.as_str()) {
        return Outcome {
            target: target.name.clone(),
            state: State::Current,
            detail: entry.command.clone(),
        };
    }

    if dry_run {
        return Outcome {
            target: target.name.clone(),
            state: match existing {
                Some(_) => State::Repointed,
                None => State::Registered,
            },
            detail: format!("would register {}", entry.command),
        };
    }

    // `claude mcp add` refuses an existing name and keeps the old value, while
    // `codex` overwrites: removing first makes both re-point.
    if existing.is_some()
        && let Err(why) = do_remove(&target.kind)
    {
        return failed(target, why);
    }

    match do_add(&target.kind, entry) {
        Err(why) => failed(target, why),
        Ok(()) => match existing {
            Some(old) => Outcome {
                target: target.name.clone(),
                state: State::Repointed,
                detail: format!("was {old}"),
            },
            None => Outcome {
                target: target.name.clone(),
                state: State::Registered,
                detail: entry.command.clone(),
            },
        },
    }
}

fn inspect(target: &Target, wanted: Option<&desktop::Entry>) -> Outcome {
    match current(&target.kind) {
        Err(why) => failed(target, why),
        Ok(None) => Outcome {
            target: target.name.clone(),
            state: State::Nothing,
            detail: "not registered".to_string(),
        },
        Ok(Some(command)) => {
            let resolves = std::path::Path::new(&command).is_file();
            let matches = wanted.is_some_and(|w| w.command == command);
            Outcome {
                target: target.name.clone(),
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
            target: target.name.clone(),
            state: State::Nothing,
            detail: "nothing to remove".to_string(),
        },
        Ok(Some(command)) => {
            if dry_run {
                return Outcome {
                    target: target.name.clone(),
                    state: State::Removed,
                    detail: format!("would remove {command}"),
                };
            }
            match do_remove(&target.kind) {
                Err(why) => failed(target, why),
                Ok(()) => Outcome {
                    target: target.name.clone(),
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
        target: target.name.clone(),
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
            format!("{mark} {:<16} {what} · {}", outcome.target, outcome.detail)
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

    fn delegated(cli: &std::path::Path, scope: Option<&'static str>) -> Target {
        Target {
            name: "fake".to_string(),
            kind: Kind::Delegated {
                cli: cli.to_string_lossy().to_string(),
                scope,
            },
        }
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
        let entry = desktop::Entry {
            command: "/abs/mitodo".to_string(),
            args: vec!["mcp-server".to_string()],
        };
        let outcome = register(&delegated(&cli, Some("user")), &entry, false);
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
        let outcome = register(&delegated(&cli, None), &plan_entry().unwrap(), false);
        assert_eq!(outcome.state, State::Failed);
        assert_eq!(outcome.detail, "it went wrong");
    }

    #[test]
    fn a_dry_run_inspects_but_never_adds() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("argv.log");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\n[ \"$2\" = get ] && exit 1\nexit 0\n",
            log.display()
        );
        let cli = stub_cli(dir.path(), "dryclient", &script);
        let outcome = register(&delegated(&cli, None), &plan_entry().unwrap(), true);
        assert_eq!(outcome.state, State::Registered);
        assert!(outcome.detail.starts_with("would register"));
        let argv = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            !argv.contains("mcp add"),
            "a dry run added something: {argv}"
        );
    }

    fn desktop_target(path: &std::path::Path) -> Target {
        Target {
            name: "claude-desktop".to_string(),
            kind: Kind::DesktopJson {
                path: path.to_path_buf(),
            },
        }
    }

    #[test]
    fn a_desktop_target_with_a_stale_path_is_repointed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        desktop::merge(
            &path,
            SERVER_NAME,
            &desktop::Entry {
                command: "/old/mitodo".to_string(),
                args: vec!["mcp-server".to_string()],
            },
        )
        .unwrap();

        let entry = plan_entry().unwrap();
        let outcome = register(&desktop_target(&path), &entry, false);
        assert_eq!(outcome.state, State::Repointed);
        assert!(outcome.detail.contains("/old/mitodo"), "{}", outcome.detail);
        assert_eq!(
            desktop::read_entry(&path, SERVER_NAME)
                .unwrap()
                .unwrap()
                .command,
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

        assert_eq!(
            register(&desktop_target(&path), &entry, false).state,
            State::Current
        );
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
        let outcome = inspect(&desktop_target(&path), plan_entry().ok().as_ref());
        assert_eq!(outcome.state, State::Missing);
    }

    #[test]
    fn removing_a_desktop_target_reports_what_it_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        let entry = plan_entry().unwrap();
        desktop::merge(&path, SERVER_NAME, &entry).unwrap();
        assert_eq!(
            unregister(&desktop_target(&path), false).state,
            State::Removed
        );
        assert!(desktop::read_entry(&path, SERVER_NAME).unwrap().is_none());
        assert_eq!(
            unregister(&desktop_target(&path), false).state,
            State::Nothing,
            "removing twice is not an error"
        );
    }
}
