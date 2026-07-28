// `Config::load` gains its caller when the `list` subcommand lands; until then
// only the tests exercise it. Drop this once the TUI consumes the full API.
#![allow(dead_code)]

use std::collections::HashMap;
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
    /// Omitted from a written config when unconfigured, so `init` does not
    /// leave an empty section that a later append would duplicate.
    #[serde(default, skip_serializing_if = "AgentConfig::is_disabled")]
    pub agent: AgentConfig,
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

/// How to invoke an external agent. Any binary that takes a prompt and emits
/// JSON works; nothing about a particular provider is baked in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    /// Argv prefix, e.g. `["claude", "--print"]`. Empty disables the feature.
    #[serde(default)]
    pub command: Vec<String>,
    /// Flag the schema is passed behind, if the tool supports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_flag: Option<String>,
    /// Paths to prompt templates, keyed by verb. Missing verbs use the
    /// built-in prompt, so a personal template (naming your own sources) stays
    /// local rather than shipping in the repository.
    #[serde(default)]
    pub prompts: HashMap<String, PathBuf>,
}

impl AgentConfig {
    pub fn is_disabled(&self) -> bool {
        self.command.is_empty()
    }
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
    fn an_unconfigured_agent_is_not_written_out() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        let rendered = toml::to_string_pretty(&cfg).unwrap();
        assert!(
            !rendered.contains("[agent"),
            "an empty agent section would make a later append invalid TOML:\n{rendered}"
        );
    }

    #[test]
    fn agent_defaults_to_disabled() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        assert!(cfg.agent.command.is_empty(), "no agent unless configured");
    }

    #[test]
    fn parses_an_agent_section() {
        let with_agent = format!(
            "{SAMPLE}\n[agent]\ncommand = [\"claude\", \"--print\"]\nschema_flag = \"--json-schema\"\n\n[agent.prompts]\nscan = \"/tmp/scan.md\"\n"
        );
        let cfg: Config = toml::from_str(&with_agent).expect("agent config parses");
        assert_eq!(cfg.agent.command, vec!["claude", "--print"]);
        assert_eq!(cfg.agent.schema_flag.as_deref(), Some("--json-schema"));
        assert_eq!(
            cfg.agent
                .prompts
                .get("scan")
                .map(|p| p.to_string_lossy().to_string()),
            Some("/tmp/scan.md".to_string())
        );
    }

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

pub mod theme;
pub use theme::Theme;
