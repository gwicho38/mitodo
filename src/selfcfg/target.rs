//! What counts as a client mitodo can register itself with.
//!
//! A client that ships its own `mcp add` is driven through it, so the client
//! owns its config format. Only a client with no CLI is edited as a file.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Drive the client's own `mcp` subcommands.
    Delegated {
        cli: String,
        /// `claude` defaults to a per-directory scope; a todo server is not
        /// directory-scoped.
        scope: Option<&'static str>,
    },
    /// No CLI exists: merge one key into this JSON file.
    DesktopJson { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub name: String,
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
    let dir = home
        .join("Library")
        .join("Application Support")
        .join("Claude");
    dir.is_dir().then(|| dir.join("claude_desktop_config.json"))
}

pub fn detect() -> Vec<Target> {
    detect_in(&which, desktop_config_path())
}

/// Detection with its two lookups injected, so tests never depend on what is
/// installed on the machine running them.
pub fn detect_in(has_cli: &dyn Fn(&str) -> bool, desktop: Option<PathBuf>) -> Vec<Target> {
    let mut found = Vec::new();
    if has_cli("claude") {
        found.push(Target {
            name: "claude".to_string(),
            kind: Kind::Delegated {
                cli: "claude".to_string(),
                scope: Some("user"),
            },
        });
    }
    if has_cli("codex") {
        found.push(Target {
            name: "codex".to_string(),
            kind: Kind::Delegated {
                cli: "codex".to_string(),
                scope: None,
            },
        });
    }
    if let Some(path) = desktop {
        found.push(Target {
            name: "claude-desktop".to_string(),
            kind: Kind::DesktopJson { path },
        });
    }
    found
}

/// Whether a command resolves on PATH.
fn which(cli: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(cli).is_file()))
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
                cli: "claude".to_string(),
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
                cli: "codex".to_string(),
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
        let names: Vec<&str> = found.iter().map(|t| t.name.as_str()).collect();
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
