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

    /// Name of the group an item belongs to, resolved by its source file.
    ///
    /// The query language's `acct:` needs this, and `Item` deliberately stores
    /// a path rather than a group name so the store stays independent of how
    /// groups happen to be configured.
    pub fn group_name_for(&self, item: &Item) -> Option<&str> {
        self.groups
            .iter()
            .find(|g| g.todo_file == item.file)
            .map(|g| g.name.as_str())
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
