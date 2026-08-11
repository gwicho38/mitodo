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
    #[serde(default, rename = "services", skip_serializing_if = "Vec::is_empty")]
    pub service_list: Vec<ServiceConfig>,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub due: DueConfig,
}

/// View state that survives a restart, the way `mcli todos` remembered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub hide_done: bool,
    #[serde(default)]
    pub ticker: bool,
    /// Capture the mouse for click, scroll and drag. Turning this off hands
    /// selection and scrollback back to the terminal emulator.
    #[serde(default = "yes")]
    pub mouse: bool,
    /// Wrap long item text across several rows instead of truncating it.
    #[serde(default)]
    pub wrap: bool,
    /// Which service the picker last selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
}

fn yes() -> bool {
    true
}

fn default_timeout() -> u64 {
    600
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            hide_done: false,
            ticker: false,
            mouse: true,
            wrap: false,
            service: None,
        }
    }
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
    /// Give up on an agent that has not answered in this long.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
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

/// How a due date is written inside an item's text.
///
/// Kept as a pattern rather than a fixed syntax so an existing convention —
/// `due:2026-08-01`, `(due 2026-08-01)`, `📅 2026-08-01` — keeps working.
/// Capture group 1 must yield an ISO `YYYY-MM-DD` date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DueConfig {
    pub enabled: bool,
    pub pattern: String,
}

impl Default for DueConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            pattern: r"due:(\d{4}-\d{2}-\d{2})".to_string(),
        }
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
        config.agent.prompts = config
            .agent
            .prompts
            .iter()
            .map(|(verb, path)| Ok((verb.clone(), expand_tilde(path)?)))
            .collect::<Result<_, ConfigError>>()?;
        Ok(config)
    }

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
            None => ActiveService {
                service: Some(first.clone()),
                notice: None,
            },
            Some(wanted) => match services.iter().find(|s| &s.name == wanted) {
                Some(found) => ActiveService {
                    service: Some(found.clone()),
                    notice: None,
                },
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

/// `$XDG_CONFIG_HOME/mitodo/config.toml`, else `~/.config/mitodo/config.toml`.
///
/// Deliberately XDG on every platform rather than the OS-native location.
/// `directories` would put this in `~/Library/Application Support` on macOS,
/// which is not where anyone looks for a terminal tool's config — helix, yazi
/// and starship all use `~/.config` there too.
pub fn default_config_path() -> Result<PathBuf, ConfigError> {
    Ok(config_root()?.join("mitodo").join("config.toml"))
}

fn config_root() -> Result<PathBuf, ConfigError> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.trim().is_empty()
    {
        return Ok(PathBuf::from(xdg));
    }
    let home = directories::BaseDirs::new()
        .ok_or(ConfigError::NoHomeDir)?
        .home_dir()
        .to_path_buf();
    Ok(home.join(".config"))
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
        let services = cfg.services();
        let names: Vec<&str> = services.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["claude", "codex", "ollama"]);
        assert_eq!(services[1].schema_mode, SchemaMode::File);
        assert_eq!(services[2].schema_flag, None);
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

    #[test]
    fn the_config_path_is_xdg_on_every_platform() {
        // Not `~/Library/Application Support` on macOS: a terminal tool's
        // config belongs where people look for it.
        let path = default_config_path().unwrap();
        let text = path.to_string_lossy();
        assert!(
            text.ends_with("mitodo/config.toml"),
            "unexpected path: {text}"
        );
        assert!(
            !text.contains("Application Support"),
            "should not use the macOS native location: {text}"
        );
    }

    #[test]
    fn ui_state_defaults_to_off_and_round_trips() {
        let cfg: Config = toml::from_str(SAMPLE).unwrap();
        assert!(!cfg.ui.hide_done);
        assert!(!cfg.ui.ticker);

        let with_ui = format!("{SAMPLE}\n[ui]\nhide_done = true\nticker = true\n");
        let cfg: Config = toml::from_str(&with_ui).unwrap();
        assert!(cfg.ui.hide_done);
        assert!(cfg.ui.ticker);

        let rendered = toml::to_string_pretty(&cfg).unwrap();
        let again: Config = toml::from_str(&rendered).unwrap();
        assert_eq!(cfg.ui, again.ui);
    }

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

    // A literal `~` here reads as missing and silently falls back to the
    // built-in prompt, so one config shared across machines would behave
    // differently on each.
    #[test]
    fn tilde_in_a_prompt_path_is_expanded_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[workspace]\nroot = \"/tmp/w\"\ngroup_by = \"directory\"\ntodo_glob = \"*/TODO.md\"\n\n[agent]\ncommand = [\"claude\"]\n\n[agent.prompts]\nscan = \"~/.config/mitodo/prompts/scan.md\"\n",
        )
        .unwrap();
        let cfg = Config::load(&path).unwrap();
        let scan = cfg.agent.prompts.get("scan").unwrap().to_string_lossy();
        assert!(!scan.starts_with('~'), "unexpanded prompt path: {scan}");
        assert!(scan.ends_with("/.config/mitodo/prompts/scan.md"));
    }
}

pub mod theme;
pub use theme::Theme;
