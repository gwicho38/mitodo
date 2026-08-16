//! Argument parsing and each tool's call into the store.
//!
//! Every write goes through `store::write`, so the byte-preservation guarantee
//! and the conflict check are the ones the rest of the project already tests.

use serde_json::{Value, json};

use super::ServerState;
use crate::store::{Group, Item, Workspace};

/// A tool failure: a machine-readable code the agent can branch on, and prose.
pub type ToolFailure = (&'static str, String);

pub fn run(state: &mut ServerState, name: &str, arguments: &Value) -> Result<Value, ToolFailure> {
    match name {
        "todos_list" => todos_list(state, arguments),
        "todos_get_item" => todos_get_item(state, arguments),
        "todos_get_file" => todos_get_file(state, arguments),
        "todos_list_groups" => todos_list_groups(state),
        "todos_create_item" => todos_create_item(state, arguments),
        "todos_add_child" => todos_add_child(state, arguments),
        "todos_update_item" => todos_update_item(state, arguments),
        "todos_set_notes" => todos_set_notes(state, arguments),
        "todos_delete_item" => todos_delete_item(state, arguments),
        "todos_archive_item" => todos_archive_item(state, arguments),
        "todos_archive_finished" => todos_archive_finished(state, arguments),
        "todos_create_group" => todos_create_group(state, arguments),
        "todos_sync" => todos_sync(state),
        other => Err(("validation_error", format!("unknown tool {other}"))),
    }
}

fn load_workspace(state: &ServerState) -> Result<Workspace, ToolFailure> {
    Workspace::load(state.config).map_err(|e| ("validation_error", e.to_string()))
}

/// Which group owns a file, for reporting.
fn group_of(workspace: &Workspace, item: &Item) -> String {
    workspace
        .groups
        .iter()
        .find(|g| g.todo_file == item.file)
        .map(|g| g.name.clone())
        .unwrap_or_default()
}

fn group_by_name<'a>(workspace: &'a Workspace, name: &str) -> Result<&'a Group, ToolFailure> {
    workspace
        .groups
        .iter()
        .find(|g| g.name == name)
        .ok_or_else(|| ("invalid_group", format!("no group named {name}")))
}

pub fn item_json(item: &Item, group: &str) -> Value {
    json!({
        "id": item.id.as_str(),
        "group": group,
        "section": item.section,
        "heading": item.heading,
        "priority": item.priority.as_str(),
        "text": item.text,
        "done": item.done,
        "has_notes": !item.description.is_empty(),
        "due": item.due.map(|d| d.to_string()),
        "line": item.line,
        "parent": item.parent.as_ref().map(|p| p.as_str()),
    })
}

fn string_arg(arguments: &Value, key: &str) -> Result<String, ToolFailure> {
    arguments
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ("validation_error", format!("{key} is required")))
}

fn todos_list(state: &ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let workspace = load_workspace(state)?;
    let query = match arguments.get("query").and_then(|v| v.as_str()) {
        None => None,
        Some(source) => {
            crate::query::Query::parse(source).map_err(|e| ("validation_error", e.to_string()))?
        }
    };
    let wanted_group = arguments.get("group").and_then(|v| v.as_str());
    let include_done = arguments
        .get("include_done")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let items: Vec<Value> = workspace
        .items
        .iter()
        .filter(|item| include_done || !item.done)
        .filter_map(|item| {
            let owner = group_of(&workspace, item);
            if let Some(wanted) = wanted_group
                && owner != wanted
            {
                return None;
            }
            match &query {
                Some(q) if !q.matches(item, Some(&owner)) => None,
                _ => Some(item_json(item, &owner)),
            }
        })
        .collect();

    Ok(json!({"count": items.len(), "items": items}))
}

fn todos_get_item(state: &ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let id = string_arg(arguments, "id")?;
    let workspace = load_workspace(state)?;
    let item = resolve(state, &workspace, &id)?;
    let group = group_of(&workspace, item);
    let children: Vec<Value> = workspace
        .items
        .iter()
        .filter(|candidate| candidate.parent.as_ref() == Some(&item.id))
        .map(|child| json!({"id": child.id.as_str(), "text": child.text, "done": child.done}))
        .collect();

    let mut payload = item_json(item, &group);
    payload["notes"] = json!(item.description);
    payload["children"] = json!(children);
    Ok(payload)
}

fn todos_get_file(state: &ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let name = string_arg(arguments, "group")?;
    let workspace = load_workspace(state)?;
    let group = group_by_name(&workspace, &name)?;
    let text = std::fs::read_to_string(&group.todo_file)
        .map_err(|e| ("validation_error", e.to_string()))?;
    Ok(json!({"path": group.todo_file.to_string_lossy(), "text": text}))
}

fn todos_list_groups(state: &ServerState) -> Result<Value, ToolFailure> {
    let workspace = load_workspace(state)?;
    let groups: Vec<Value> = workspace
        .groups
        .iter()
        .map(|group| {
            let items: Vec<&Item> = workspace
                .items
                .iter()
                .filter(|i| i.file == group.todo_file)
                .collect();
            json!({
                "name": group.name,
                "todo_file": group.todo_file.to_string_lossy(),
                "open": items.iter().filter(|i| !i.done).count(),
                "total": items.len(),
                "has_notes": group.notes_file.is_some(),
                "archive_dir": group.archive_dir.as_ref().map(|d| d.to_string_lossy()),
            })
        })
        .collect();
    Ok(json!({"groups": groups}))
}

fn describe(err: crate::store::WriteError) -> ToolFailure {
    match err {
        crate::store::WriteError::Conflict { .. } => (
            "conflict",
            "the file changed on disk; re-read and retry".to_string(),
        ),
        other => ("validation_error", other.to_string()),
    }
}

fn todos_create_item(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let group_name = string_arg(arguments, "group")?;
    let text = string_arg(arguments, "text")?;
    if text.trim().is_empty() {
        return Err(("validation_error", "text must not be empty".to_string()));
    }
    let workspace = load_workspace(state)?;
    let group = group_by_name(&workspace, &group_name)?;
    let path = group.todo_file.clone();
    let section = arguments.get("section").and_then(|v| v.as_str());

    // A named section the file lacks would otherwise land at end of file, and
    // priority is derived from the heading above an item: the agent would be told
    // it filed a P0 that is not one.
    if let Some(wanted) = section {
        let body =
            std::fs::read_to_string(&path).map_err(|e| ("validation_error", e.to_string()))?;
        let found = body.lines().any(|line| {
            line.strip_prefix("## ").is_some_and(|heading| {
                heading
                    .trim()
                    .to_lowercase()
                    .starts_with(&wanted.trim().to_lowercase())
            })
        });
        if !found {
            return Err((
                "missing_priority_section",
                format!(
                    "{group_name} has no section starting {wanted:?}; sections are never invented"
                ),
            ));
        }
    }

    let notes = arguments
        .get("notes")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let children: Vec<String> = arguments
        .get("children")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|c| c.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    crate::store::create_item(&path, section, text.trim(), notes, &children).map_err(describe)?;

    let reloaded = load_workspace(state)?;
    let created = reloaded
        .items
        .iter()
        .rfind(|i| i.file == path && i.text == text.trim())
        .ok_or_else(|| ("conflict", "the item was written but not found".to_string()))?;
    let owner = group_of(&reloaded, created);
    Ok(json!({"item": item_json(created, &owner)}))
}

/// A group name is a name, never a path: the server joins it onto the workspace
/// root, so anything that could escape is refused.
fn valid_group_name(name: &str) -> Result<(), ToolFailure> {
    let bad = name.trim().is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name == ".."
        || name.starts_with('.');
    if bad {
        return Err((
            "invalid_group",
            format!("{name:?} is not a valid group name"),
        ));
    }
    Ok(())
}

fn archive_dir_of(group: &Group) -> Result<std::path::PathBuf, ToolFailure> {
    group.archive_dir.clone().ok_or_else(|| {
        (
            "invalid_group",
            format!("no archive_dir configured for {}", group.name),
        )
    })
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn todos_archive_item(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let id = string_arg(arguments, "id")?;
    let workspace = load_workspace(state)?;
    let item = resolve(state, &workspace, &id)?;
    let group = workspace
        .groups
        .iter()
        .find(|g| g.todo_file == item.file)
        .ok_or_else(|| ("invalid_group", "the item's group is unknown".to_string()))?;
    let archive = archive_dir_of(group)?;
    let payload = item_json(item, &group.name);

    let report =
        crate::store::archive_items(&item.file, &archive, &[item], &today()).map_err(describe)?;
    if let Some(reason) = report.skipped.first() {
        return Err(("conflict", reason.clone()));
    }
    retire(state, &id);
    Ok(json!({"item": payload, "archived": true}))
}

fn todos_archive_finished(
    state: &mut ServerState,
    arguments: &Value,
) -> Result<Value, ToolFailure> {
    let name = string_arg(arguments, "group")?;
    let workspace = load_workspace(state)?;
    let group = group_by_name(&workspace, &name)?;
    let archive = archive_dir_of(group)?;
    let report = crate::store::archive_done(&group.todo_file, &archive, &workspace.items, &today())
        .map_err(describe)?;
    Ok(json!({"archived": report.archived, "skipped": report.skipped}))
}

fn todos_create_group(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let name = string_arg(arguments, "name")?;
    valid_group_name(&name)?;
    let workspace = load_workspace(state)?;
    let dir = workspace.root.join(&name);
    if dir.exists() {
        return Err(("validation_error", format!("{name} already exists")));
    }

    // Copy the section headings an existing group uses, so a new group matches
    // the convention already in the workspace and the next create has a section
    // to land in.
    let copied = workspace
        .groups
        .first()
        .and_then(|existing| std::fs::read_to_string(&existing.todo_file).ok())
        .map(|text| {
            text.lines()
                .filter(|line| line.starts_with("## "))
                .map(|line| format!("{line}\n\n"))
                .collect::<String>()
        })
        .unwrap_or_default();
    let seed = if !copied.is_empty() {
        copied
    } else if state.config.priority.source == crate::config::PrioritySource::Heading {
        "## P0\n\n## P1\n\n## P2\n\n## P3\n\n".to_string()
    } else {
        String::new()
    };

    std::fs::create_dir_all(&dir).map_err(|e| ("validation_error", e.to_string()))?;
    let todo_file = dir.join("TODO.md");
    std::fs::write(&todo_file, seed).map_err(|e| ("validation_error", e.to_string()))?;
    Ok(json!({"name": name, "todo_file": todo_file.to_string_lossy()}))
}

fn todos_sync(state: &ServerState) -> Result<Value, ToolFailure> {
    if !state.config.git.enabled {
        return Err(("git_disabled", "[git] enabled is false".to_string()));
    }
    let workspace = load_workspace(state)?;
    let outcome = crate::git::run_sync(&workspace.root, &state.config.git.sync, "git");
    Ok(json!({"ok": outcome.ok, "transcript": outcome.transcript}))
}

/// Remember an id our own write invalidated, so a later call can say so.
fn retire(state: &mut ServerState, id: &str) {
    state.retired.insert(id.to_string());
}

/// The item at `line` in `path` after a write, for reporting what changed.
fn reread<'a>(
    workspace: &'a Workspace,
    path: &std::path::Path,
    line: usize,
) -> Result<&'a Item, ToolFailure> {
    workspace
        .items
        .iter()
        .find(|i| i.file == path && i.line == line)
        .ok_or_else(|| ("conflict", "the item vanished after the write".to_string()))
}

fn todos_add_child(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let parent_id = string_arg(arguments, "parent_id")?;
    let text = string_arg(arguments, "text")?;
    if text.trim().is_empty() {
        return Err(("validation_error", "text must not be empty".to_string()));
    }
    let workspace = load_workspace(state)?;
    let parent = resolve(state, &workspace, &parent_id)?;
    let (path, line, raw, indent) = (
        parent.file.clone(),
        parent.line,
        parent.raw.clone(),
        parent.indent,
    );
    crate::store::add_item(&path, line, &raw, indent + 2, text.trim()).map_err(describe)?;

    let reloaded = load_workspace(state)?;
    let child = reread(&reloaded, &path, line + 1)?;
    let owner = group_of(&reloaded, child);
    Ok(json!({"item": item_json(child, &owner)}))
}

fn todos_update_item(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let id = string_arg(arguments, "id")?;
    let new_text = arguments.get("new_text").and_then(|v| v.as_str());
    let done = arguments.get("done").and_then(|v| v.as_bool());
    if new_text.is_none() && done.is_none() {
        return Err((
            "validation_error",
            "give new_text, done, or both".to_string(),
        ));
    }

    let workspace = load_workspace(state)?;
    let item = resolve(state, &workspace, &id)?;
    let (path, line, raw) = (item.file.clone(), item.line, item.raw.clone());

    // Toggling rewrites the line, so the expected raw for a following edit has
    // to be re-read rather than reused.
    if let Some(done) = done {
        crate::store::toggle(&path, line, &raw, done).map_err(describe)?;
    }
    if let Some(text) = new_text {
        let between = load_workspace(state)?;
        let current = reread(&between, &path, line)?.raw.clone();
        crate::store::edit_text(&path, line, &current, text.trim()).map_err(describe)?;
        retire(state, &id);
    }

    let reloaded = load_workspace(state)?;
    let updated = reread(&reloaded, &path, line)?;
    let owner = group_of(&reloaded, updated);
    Ok(json!({"item": item_json(updated, &owner)}))
}

fn todos_set_notes(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let id = string_arg(arguments, "id")?;
    let notes = string_arg(arguments, "notes")?;
    let workspace = load_workspace(state)?;
    let item = resolve(state, &workspace, &id)?;
    let (path, line, raw) = (item.file.clone(), item.line, item.raw.clone());
    crate::store::set_description(&path, line, &raw, &notes).map_err(describe)?;

    let reloaded = load_workspace(state)?;
    let updated = reread(&reloaded, &path, line)?;
    let owner = group_of(&reloaded, updated);
    Ok(json!({"item": item_json(updated, &owner)}))
}

fn todos_delete_item(state: &mut ServerState, arguments: &Value) -> Result<Value, ToolFailure> {
    let id = string_arg(arguments, "id")?;
    let workspace = load_workspace(state)?;
    let item = resolve(state, &workspace, &id)?;
    let (path, line, raw) = (item.file.clone(), item.line, item.raw.clone());
    crate::store::delete_item(&path, line, &raw).map_err(describe)?;
    retire(state, &id);
    Ok(json!({"deleted": true}))
}

/// The item an id names.
///
/// An id this server retired by its own write reports `not_found`; one that
/// simply no longer resolves drifted out-of-band and reports `conflict`. The
/// agent needs to tell its own edit from someone else's.
fn resolve<'a>(
    state: &ServerState,
    workspace: &'a Workspace,
    id: &str,
) -> Result<&'a Item, ToolFailure> {
    if let Some(item) = workspace.items.iter().find(|i| i.id.as_str() == id) {
        return Ok(item);
    }
    if state.retired.contains(id) {
        return Err((
            "not_found",
            format!("id {id} was retired by an earlier write"),
        ));
    }
    Err((
        "conflict",
        format!("id {id} no longer resolves; re-read and retry"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, GroupBy, PrioritySource};
    use std::collections::HashSet;

    /// A workspace on disk, plus the config that reads it.
    fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        for (group, body) in files {
            let group_dir = dir.path().join(group);
            std::fs::create_dir_all(&group_dir).unwrap();
            std::fs::write(group_dir.join("TODO.md"), body).unwrap();
        }
        let config = Config {
            workspace: crate::config::WorkspaceConfig {
                root: dir.path().to_path_buf(),
                group_by: GroupBy::Directory,
                todo_glob: "*/TODO.md".to_string(),
                notes_glob: None,
                archive_dir: None,
            },
            priority: crate::config::PriorityConfig {
                source: PrioritySource::Heading,
                pattern: "^P([0-3])".to_string(),
            },
            ..Default::default()
        };
        (dir, config)
    }

    fn run_tool(config: &Config, name: &str, arguments: Value) -> Result<Value, ToolFailure> {
        let mut state = ServerState {
            config,
            retired: HashSet::new(),
        };
        run(&mut state, name, &arguments)
    }

    const LEFV: &str = "## P0 — Critical\n\n- [ ] alpha\n  - [ ] child\n- [x] beta\n";

    #[test]
    fn list_returns_open_items_with_ids() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let listed = run_tool(&config, "todos_list", json!({})).unwrap();
        let items = listed["items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "beta is done and excluded: {items:?}");
        assert!(items.iter().all(|i| i["id"].as_str().unwrap().len() == 12));
        assert_eq!(items[0]["group"], "lefv");
    }

    #[test]
    fn list_can_include_done_items() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let listed = run_tool(&config, "todos_list", json!({"include_done": true})).unwrap();
        assert_eq!(listed["items"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn list_filters_by_group() {
        let (_dir, config) = fixture(&[("lefv", LEFV), ("jzlaw", "## P0\n\n- [ ] other\n")]);
        let listed = run_tool(&config, "todos_list", json!({"group": "jzlaw"})).unwrap();
        let items = listed["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["text"], "other");
    }

    #[test]
    fn list_accepts_mitodos_query_language() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let listed = run_tool(&config, "todos_list", json!({"query": "text:\"alpha\""})).unwrap();
        let items = listed["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["text"], "alpha");
    }

    // The agent can fix its own query if it is told what was wrong.
    #[test]
    fn a_malformed_query_reports_the_parsers_message() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let failure = run_tool(&config, "todos_list", json!({"query": "pri:"})).unwrap_err();
        assert_eq!(failure.0, "validation_error");
        assert!(!failure.1.is_empty());
    }

    #[test]
    fn get_item_returns_notes_and_direct_children() {
        let body = "## P0 — Critical\n\n- [ ] alpha\n  > why it matters\n  - [ ] child\n";
        let (_dir, config) = fixture(&[("lefv", body)]);
        let listed = run_tool(&config, "todos_list", json!({})).unwrap();
        let alpha = listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["text"] == "alpha")
            .unwrap()
            .clone();

        let full = run_tool(&config, "todos_get_item", json!({"id": alpha["id"]})).unwrap();
        assert_eq!(full["notes"], "why it matters");
        let children = full["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["text"], "child");
    }

    #[test]
    fn an_id_that_never_existed_is_a_conflict() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let failure =
            run_tool(&config, "todos_get_item", json!({"id": "ffffffffffff"})).unwrap_err();
        assert_eq!(failure.0, "conflict");
    }

    // The registry's whole point: our own edit reads differently from drift.
    #[test]
    fn an_id_we_retired_ourselves_is_not_found() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let mut state = ServerState {
            config: &config,
            retired: HashSet::from(["ffffffffffff".to_string()]),
        };
        let failure =
            run(&mut state, "todos_get_item", &json!({"id": "ffffffffffff"})).unwrap_err();
        assert_eq!(failure.0, "not_found");
    }

    #[test]
    fn get_file_returns_the_markdown_verbatim() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let file = run_tool(&config, "todos_get_file", json!({"group": "lefv"})).unwrap();
        assert_eq!(file["text"], LEFV);
    }

    #[test]
    fn get_file_on_an_unknown_group_is_invalid_group() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let failure = run_tool(&config, "todos_get_file", json!({"group": "nope"})).unwrap_err();
        assert_eq!(failure.0, "invalid_group");
    }

    #[test]
    fn list_groups_counts_open_and_total() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let groups = run_tool(&config, "todos_list_groups", json!({})).unwrap();
        let first = &groups["groups"].as_array().unwrap()[0];
        assert_eq!(first["name"], "lefv");
        assert_eq!(first["open"], 2);
        assert_eq!(first["total"], 3);
    }

    // Without this check store::create_item appends at end of file, and because
    // priority is derived from the section heading the item would silently get
    // the wrong priority — an agent that asked for P0 would believe it got one.
    #[test]
    fn creating_under_a_section_the_file_lacks_is_refused() {
        let (dir, config) = fixture(&[("lefv", "## P1 — Later\n\n- [ ] existing\n")]);
        let before = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();

        let failure = run_tool(
            &config,
            "todos_create_item",
            json!({"group": "lefv", "text": "urgent", "section": "P0"}),
        )
        .unwrap_err();
        assert_eq!(failure.0, "missing_priority_section");
        assert!(failure.1.contains("P0"), "{}", failure.1);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap(),
            before,
            "a refused create writes nothing"
        );
    }

    #[test]
    fn creating_under_a_section_the_file_has_is_allowed() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] existing\n")]);
        let created = run_tool(
            &config,
            "todos_create_item",
            json!({"group": "lefv", "text": "urgent", "section": "P0"}),
        )
        .unwrap();
        assert_eq!(created["item"]["text"], "urgent");
        assert_eq!(created["item"]["priority"], "P0", "it really is a P0");
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(
            written,
            "## P0 — Critical\n\n- [ ] existing\n- [ ] urgent\n"
        );
    }

    // No section named means "wherever create_item puts it", which is the
    // existing behaviour the TUI's own add relies on.
    #[test]
    fn creating_without_a_section_is_not_refused() {
        let (_dir, config) = fixture(&[("lefv", "## P1 — Later\n\n- [ ] existing\n")]);
        let created = run_tool(
            &config,
            "todos_create_item",
            json!({"group": "lefv", "text": "somewhere"}),
        )
        .unwrap();
        assert_eq!(created["item"]["text"], "somewhere");
    }

    // The directory must exist: Workspace::load only records an archive_dir it
    // can see on disk.
    fn fixture_with_archive(files: &[(&str, &str)]) -> (tempfile::TempDir, Config) {
        let (dir, mut config) = fixture(files);
        config.workspace.archive_dir = Some("_archive".to_string());
        for (group, _) in files {
            std::fs::create_dir_all(dir.path().join(group).join("_archive")).unwrap();
        }
        (dir, config)
    }

    fn id_of(config: &Config, text: &str) -> String {
        let listed = run_tool(config, "todos_list", json!({"include_done": true})).unwrap();
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["text"] == text)
            .unwrap_or_else(|| panic!("no item {text:?}"))["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn add_child_indents_two_beyond_its_parent() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let parent = id_of(&config, "alpha");
        let added = run_tool(
            &config,
            "todos_add_child",
            json!({"parent_id": parent, "text": "nested"}),
        )
        .unwrap();
        assert_eq!(added["item"]["text"], "nested");
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(written, "## P0 — Critical\n\n- [ ] alpha\n  - [ ] nested\n");
    }

    #[test]
    fn update_item_ticks_without_changing_the_id() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = id_of(&config, "alpha");
        let updated = run_tool(
            &config,
            "todos_update_item",
            json!({"id": id, "done": true}),
        )
        .unwrap();
        assert_eq!(updated["item"]["id"], id, "ticking does not change the id");
        assert_eq!(updated["item"]["done"], true);
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(written, "## P0 — Critical\n\n- [x] alpha\n");
    }

    #[test]
    fn update_item_returns_a_new_id_when_the_text_changed() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = id_of(&config, "alpha");
        let updated = run_tool(
            &config,
            "todos_update_item",
            json!({"id": id, "new_text": "alpha revised"}),
        )
        .unwrap();
        assert_ne!(updated["item"]["id"], id);
        assert_eq!(updated["item"]["text"], "alpha revised");
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(written, "## P0 — Critical\n\n- [ ] alpha revised\n");
    }

    #[test]
    fn update_item_does_both_in_one_call() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = id_of(&config, "alpha");
        run_tool(
            &config,
            "todos_update_item",
            json!({"id": id, "new_text": "alpha revised", "done": true}),
        )
        .unwrap();
        let written = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert_eq!(written, "## P0 — Critical\n\n- [x] alpha revised\n");
    }

    #[test]
    fn update_item_with_neither_argument_is_a_validation_error() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = id_of(&config, "alpha");
        let failure = run_tool(&config, "todos_update_item", json!({"id": id})).unwrap_err();
        assert_eq!(failure.0, "validation_error");
    }

    // The registry's whole point: our own edit reads differently from drift.
    #[test]
    fn an_id_our_own_write_retired_reports_not_found() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = id_of(&config, "alpha");
        let mut state = ServerState {
            config: &config,
            retired: HashSet::new(),
        };
        run(
            &mut state,
            "todos_update_item",
            &json!({"id": id, "new_text": "alpha revised"}),
        )
        .unwrap();

        let failure = run(&mut state, "todos_get_item", &json!({"id": id})).unwrap_err();
        assert_eq!(failure.0, "not_found", "we retired it, so say so");
    }

    #[test]
    fn set_notes_writes_a_block_and_an_empty_string_removes_it() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = id_of(&config, "alpha");
        run_tool(
            &config,
            "todos_set_notes",
            json!({"id": id, "notes": "because"}),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap(),
            "## P0 — Critical\n\n- [ ] alpha\n  > because\n"
        );

        run_tool(&config, "todos_set_notes", json!({"id": id, "notes": ""})).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap(),
            "## P0 — Critical\n\n- [ ] alpha\n"
        );
    }

    #[test]
    fn delete_item_removes_the_line_and_its_notes() {
        let body = "## P0 — Critical\n\n- [ ] alpha\n  > gone too\n- [ ] beta\n";
        let (dir, config) = fixture(&[("lefv", body)]);
        let id = id_of(&config, "alpha");
        assert_eq!(
            run_tool(&config, "todos_delete_item", json!({"id": id})).unwrap()["deleted"],
            true
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap(),
            "## P0 — Critical\n\n- [ ] beta\n"
        );
    }

    // The project's core guarantee, re-pinned at this layer.
    #[test]
    fn a_write_leaves_every_other_line_byte_identical() {
        let body = "# Notes\r\n\r\n## P0 — Critical\r\n\r\n- [ ] alpha\r\n\r\n## P1 — Later\r\n\r\n- [ ] beta\r\n";
        let (dir, config) = fixture(&[("lefv", body)]);
        let id = id_of(&config, "alpha");
        run_tool(
            &config,
            "todos_update_item",
            json!({"id": id, "done": true}),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap(),
            body.replace("- [ ] alpha", "- [x] alpha"),
            "only the one line changed, CRLF preserved"
        );
    }

    #[test]
    fn archive_item_moves_it_into_the_archive_file() {
        let (dir, config) =
            fixture_with_archive(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n- [ ] beta\n")]);
        let id = id_of(&config, "alpha");
        assert_eq!(
            run_tool(&config, "todos_archive_item", json!({"id": id})).unwrap()["archived"],
            true
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap(),
            "## P0 — Critical\n\n- [ ] beta\n"
        );
        let moved = std::fs::read_to_string(dir.path().join("lefv/_archive/TODO.md")).unwrap();
        assert!(moved.contains("- [ ] alpha"), "verbatim: {moved}");
        assert!(moved.contains("## Archived "), "dated heading: {moved}");
    }

    #[test]
    fn archiving_without_an_archive_dir_is_invalid_group() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let id = id_of(&config, "alpha");
        let failure = run_tool(&config, "todos_archive_item", json!({"id": id})).unwrap_err();
        assert_eq!(failure.0, "invalid_group");
        assert!(failure.1.contains("archive_dir"), "{}", failure.1);
    }

    #[test]
    fn archive_finished_sweeps_done_items_and_reports_skips() {
        let body =
            "## P0 — Critical\n\n- [x] done one\n- [x] parent\n  - [ ] still open\n- [ ] open\n";
        let (dir, config) = fixture_with_archive(&[("lefv", body)]);
        let report = run_tool(&config, "todos_archive_finished", json!({"group": "lefv"})).unwrap();
        assert_eq!(report["archived"], 1);
        assert_eq!(
            report["skipped"].as_array().unwrap().len(),
            1,
            "the parent still has open work"
        );
        let left = std::fs::read_to_string(dir.path().join("lefv/TODO.md")).unwrap();
        assert!(!left.contains("done one"));
        assert!(left.contains("parent") && left.contains("still open"));
    }

    #[test]
    fn create_group_seeds_a_file_with_the_existing_headings() {
        let (dir, config) =
            fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n\n## P1 — Later\n")]);
        let created = run_tool(&config, "todos_create_group", json!({"name": "newone"})).unwrap();
        assert_eq!(created["name"], "newone");
        let seeded = std::fs::read_to_string(dir.path().join("newone/TODO.md")).unwrap();
        assert!(seeded.contains("## P0 — Critical"), "copied: {seeded:?}");
        assert!(seeded.contains("## P1 — Later"), "copied: {seeded:?}");
        assert!(!seeded.contains("alpha"), "headings only: {seeded:?}");
    }

    // Seeding exists so the next create has a section to land in.
    #[test]
    fn an_item_can_be_created_in_a_freshly_created_group() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        run_tool(&config, "todos_create_group", json!({"name": "newone"})).unwrap();
        let created = run_tool(
            &config,
            "todos_create_item",
            json!({"group": "newone", "text": "first", "section": "P0"}),
        )
        .unwrap();
        assert_eq!(created["item"]["text"], "first");
    }

    #[test]
    fn create_group_refuses_a_name_that_already_exists() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let failure = run_tool(&config, "todos_create_group", json!({"name": "lefv"})).unwrap_err();
        assert_eq!(failure.0, "validation_error");
    }

    // The containment boundary: a group name is a name, never a path.
    #[test]
    fn create_group_rejects_names_that_escape_the_workspace() {
        let (dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        // An absolute path, not a specific system file: naming one trips secret
        // scanners on a test that contains no secret.
        for name in ["../evil", "/tmp/outside", ".git", "a/b", "..", "a\\b"] {
            let failure =
                run_tool(&config, "todos_create_group", json!({"name": name})).unwrap_err();
            assert_eq!(failure.0, "invalid_group", "{name} should be refused");
        }
        assert!(!dir.path().parent().unwrap().join("evil").exists());
        assert!(!dir.path().join(".git").exists());
    }

    #[test]
    fn sync_is_refused_when_git_is_disabled() {
        let (_dir, config) = fixture(&[("lefv", "## P0 — Critical\n\n- [ ] alpha\n")]);
        let failure = run_tool(&config, "todos_sync", json!({})).unwrap_err();
        assert_eq!(failure.0, "git_disabled");
    }

    // Every advertised tool answers; none is left dispatching to "unknown tool".
    #[test]
    fn every_advertised_tool_is_implemented() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        for tool in &crate::mcp::tools::TOOLS {
            let mut state = ServerState {
                config: &config,
                retired: HashSet::new(),
            };
            // Deliberately empty arguments: a missing argument is fine, but
            // "unknown tool" means the catalogue advertises what does not exist.
            if let Err((code, message)) = run(&mut state, tool.name, &json!({}))
                && message.starts_with("unknown tool")
            {
                panic!("{} is advertised but not implemented ({code})", tool.name);
            }
        }
    }

    #[test]
    fn a_missing_required_argument_is_a_validation_error() {
        let (_dir, config) = fixture(&[("lefv", LEFV)]);
        let failure = run_tool(&config, "todos_get_item", json!({})).unwrap_err();
        assert_eq!(failure.0, "validation_error");
    }
}
