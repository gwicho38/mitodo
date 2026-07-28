use std::path::Path;
use std::process::Command;

/// Outcome of running a configured command list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncOutcome {
    pub transcript: String,
    pub ok: bool,
}

/// Run each configured argv in `root`, stopping at the first failure.
///
/// Shells out to `git` rather than linking libgit2: the command list is
/// user-configurable, so whatever `git` they have is the one that should run.
pub fn run_sync(root: &Path, commands: &[Vec<String>], program: &str) -> SyncOutcome {
    let mut transcript = String::new();
    if commands.is_empty() {
        return SyncOutcome {
            transcript: "no sync commands configured".to_string(),
            ok: false,
        };
    }

    for argv in commands {
        if argv.is_empty() {
            continue;
        }
        transcript.push_str(&format!("$ {program} {}\n", argv.join(" ")));

        let output = Command::new(program).args(argv).current_dir(root).output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                if !stdout.trim().is_empty() {
                    transcript.push_str(stdout.trim_end());
                    transcript.push('\n');
                }
                if !stderr.trim().is_empty() {
                    transcript.push_str(stderr.trim_end());
                    transcript.push('\n');
                }
                if !out.status.success() {
                    transcript.push_str(&format!("failed with {}\n", out.status));
                    return SyncOutcome {
                        transcript,
                        ok: false,
                    };
                }
            }
            Err(err) => {
                transcript.push_str(&format!("could not run {program}: {err}\n"));
                return SyncOutcome {
                    transcript,
                    ok: false,
                };
            }
        }
    }

    transcript.push_str("sync complete\n");
    SyncOutcome {
        transcript,
        ok: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_every_command_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let commands = vec![vec!["first".to_string()], vec!["second".to_string()]];
        let outcome = run_sync(dir.path(), &commands, "echo");
        assert!(outcome.ok);
        assert!(outcome.transcript.contains("first"));
        assert!(outcome.transcript.contains("second"));
        assert!(outcome.transcript.contains("sync complete"));
    }

    #[test]
    fn echoes_each_command_before_running_it() {
        let dir = tempfile::tempdir().unwrap();
        let commands = vec![vec!["hello".to_string()]];
        let outcome = run_sync(dir.path(), &commands, "echo");
        assert!(outcome.transcript.contains("$ echo hello"));
    }

    #[test]
    fn stops_at_the_first_failure() {
        let dir = tempfile::tempdir().unwrap();
        let commands = vec![
            vec!["1".to_string()],
            vec!["-c".to_string(), "exit 3".to_string()],
            vec!["never".to_string()],
        ];
        // `sh -c 'exit 3'` fails; the third command must not run.
        let outcome = run_sync(dir.path(), &commands, "sh");
        assert!(!outcome.ok);
        assert!(!outcome.transcript.contains("never"));
    }

    #[test]
    fn a_missing_program_is_reported_not_panicked() {
        let dir = tempfile::tempdir().unwrap();
        let commands = vec![vec!["x".to_string()]];
        let outcome = run_sync(dir.path(), &commands, "definitely-not-a-real-program");
        assert!(!outcome.ok);
        assert!(outcome.transcript.contains("could not run"));
    }

    #[test]
    fn an_empty_command_list_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = run_sync(dir.path(), &[], "git");
        assert!(!outcome.ok);
        assert!(outcome.transcript.contains("no sync commands"));
    }

    #[test]
    fn runs_in_the_given_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();
        let commands = vec![vec![]];
        // `ls` with no args lists the cwd.
        let outcome = run_sync(dir.path(), &commands, "ls");
        // An empty argv is skipped, so use a real one.
        assert!(outcome.ok, "empty argv is skipped: {}", outcome.transcript);

        let outcome = run_sync(dir.path(), &[vec![".".to_string()]], "ls");
        assert!(outcome.transcript.contains("marker.txt"));
    }
}
