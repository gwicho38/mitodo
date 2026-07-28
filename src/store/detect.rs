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
