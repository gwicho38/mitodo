//! The change-set an agent proposes, and how it is applied.
//!
//! Deliberately the same shape as the private `mcli todos scan` schema so an
//! existing prompt keeps working, but every change is reviewed before it
//! touches a file, and applying goes through the conflict-aware writer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::store::{self, Item, WriteError};

/// Schema handed to the agent for the change-set verbs.
pub const CHANGE_SCHEMA: &str = r#"{"type":"object","properties":{"changes":{"type":"array","items":{"type":"object","properties":{"file":{"type":"string"},"action":{"type":"string","enum":["add","complete","update","archive"]},"section":{"type":"string"},"heading":{"type":"string"},"content":{"type":"string"},"reason":{"type":"string"}},"required":["file","action","content","reason"]}},"summary":{"type":"string"}},"required":["changes","summary"]}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeAction {
    Add,
    Complete,
    Update,
    Archive,
}

impl ChangeAction {
    pub fn glyph(self) -> &'static str {
        match self {
            ChangeAction::Add => "+ add",
            ChangeAction::Complete => "✓ done",
            ChangeAction::Update => "~ edit",
            ChangeAction::Archive => "→ archive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Change {
    pub file: String,
    pub action: ChangeAction,
    #[serde(default)]
    pub section: String,
    #[serde(default)]
    pub heading: String,
    pub content: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct ChangeSet {
    pub changes: Vec<Change>,
    #[serde(default)]
    pub summary: String,
}

impl ChangeSet {
    /// Parse a reply, tolerating the wrappers models put around JSON.
    pub fn parse(json: &str) -> Result<ChangeSet, serde_json::Error> {
        match super::extract_json(json) {
            Some(value) => serde_json::from_value(value),
            None => serde_json::from_str(super::sanitize(json).trim()),
        }
    }

    /// A one-line summary of a change, for the review list.
    pub fn row(&self, index: usize) -> String {
        match self.changes.get(index) {
            None => String::new(),
            Some(change) => format!(
                "{} {} · {}",
                change.action.glyph(),
                change.file,
                change.content.trim().trim_start_matches("- [ ]").trim()
            ),
        }
    }

    /// Why the agent proposed the change at `index`.
    pub fn reason(&self, index: usize) -> String {
        self.changes
            .get(index)
            .map(|c| c.reason.clone())
            .unwrap_or_default()
    }

    /// How many unfinished items an archive change would carry out of the
    /// working file, anywhere in the subtree. Zero for every other action.
    ///
    /// Resolves the change through `find`, exactly as `apply` does, so the row
    /// always describes the item that would actually be archived.
    pub fn open_sub_items(&self, index: usize, root: &Path, items: &[Item]) -> usize {
        let Some(change) = self.changes.get(index) else {
            return 0;
        };
        if change.action != ChangeAction::Archive {
            return 0;
        }
        let path = root.join(&change.file);
        let Some(target) = find(items, &path, &change.content) else {
            return 0;
        };
        descendants(items, target)
            .into_iter()
            .filter(|i| !i.done)
            .count()
    }

    /// A copy carrying only the changes at the given positions.
    pub fn selected(&self, keep: &[bool]) -> ChangeSet {
        ChangeSet {
            changes: self
                .changes
                .iter()
                .enumerate()
                .filter(|(index, _)| keep.get(*index).copied().unwrap_or(false))
                .map(|(_, change)| change.clone())
                .collect(),
            summary: self.summary.clone(),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub applied: usize,
    pub skipped: Vec<String>,
}

/// Apply a reviewed change-set.
///
/// `complete`, `update` and `archive` locate their target by matching item text
/// within the named file, so a change-set stays valid even if line numbers moved
/// since the agent read the workspace. Anything that cannot be located is
/// skipped and reported rather than guessed at.
///
/// `archive_dirs` maps a todo file to its group's archive directory; a file
/// missing from it has none configured.
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

/// Everything below `root_item`, at any depth.
///
/// Archiving moves the whole subtree, so a count of what travels with an item
/// cannot stop at its direct children.
fn descendants<'a>(items: &'a [Item], root_item: &Item) -> Vec<&'a Item> {
    let mut found = Vec::new();
    let mut frontier = vec![root_item.id.clone()];
    while let Some(parent) = frontier.pop() {
        for child in items.iter().filter(|i| i.parent.as_ref() == Some(&parent)) {
            frontier.push(child.id.clone());
            found.push(child);
        }
    }
    found
}

/// The item in `path` whose text best matches `needle`.
fn find<'a>(items: &'a [Item], path: &Path, needle: &str) -> Option<&'a Item> {
    let needle = needle.trim().to_lowercase();
    items.iter().filter(|i| i.file == path).find(|i| {
        i.text.to_lowercase().contains(&needle) || needle.contains(&i.text.to_lowercase())
    })
}

fn apply_add(path: &Path, items: &[Item], change: &Change) -> Result<(), String> {
    let text = change.content.trim().trim_start_matches("- [ ]").trim();

    // Creating the group is a separate matter; say which file is missing rather
    // than reporting an absent anchor.
    if !path.exists() {
        return Err(format!("add {:?}: {} does not exist", text, path.display()));
    }

    // Anchor to the last item in the requested section, else the last in the
    // file, so the file keeps its shape.
    let anchor = items
        .iter()
        .rfind(|i| i.file == path && (change.section.is_empty() || i.section == change.section))
        .or_else(|| items.iter().rfind(|i| i.file == path));

    match anchor {
        Some(anchor) => store::add_item(path, anchor.line, &anchor.raw, anchor.indent, text)
            .map_err(|e| describe(e, "add", text)),
        // A group with no items yet has no anchor, but its section headings
        // still give a position.
        None => {
            let section = (!change.section.is_empty()).then_some(change.section.as_str());
            store::create_item(path, section, text, "", &[]).map_err(|e| describe(e, "add", text))
        }
    }
}

fn apply_complete(path: &Path, items: &[Item], change: &Change) -> Result<(), String> {
    let Some(item) = find(items, path, &change.content) else {
        return Err(format!("complete {:?}: no matching item", change.content));
    };
    if item.done {
        return Ok(());
    }
    store::toggle(path, item.line, &item.raw, true).map_err(|e| describe(e, "complete", &item.text))
}

fn apply_update(path: &Path, items: &[Item], change: &Change) -> Result<(), String> {
    let Some(item) = find(items, path, &change.content) else {
        return Err(format!("update {:?}: no matching item", change.content));
    };
    let text = change.content.trim().trim_start_matches("- [ ]").trim();
    store::edit_text(path, item.line, &item.raw, text).map_err(|e| describe(e, "update", text))
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

fn describe(err: WriteError, what: &str, text: &str) -> String {
    match err {
        WriteError::Conflict { .. } => {
            format!("{what} {text:?}: file changed on disk")
        }
        other => format!("{what} {text:?}: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DueConfig, PriorityConfig};
    use crate::store::parse_todo_file;

    const DOC: &str = "## P0 — Critical\n\n- [ ] alpha\n- [x] beta\n";

    fn workspace(body: &str) -> (tempfile::TempDir, PathBuf, Vec<Item>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lefv.md");
        std::fs::write(&path, body).unwrap();
        let items = parse_todo_file(
            &path,
            "lefv.md",
            body,
            &PriorityConfig::default(),
            &DueConfig::default(),
        );
        (dir, path, items)
    }

    fn set(json: &str) -> ChangeSet {
        ChangeSet::parse(json).expect("change-set parses")
    }

    fn archive_map(todo: &Path, dir: &Path) -> HashMap<PathBuf, PathBuf> {
        HashMap::from([(todo.to_path_buf(), dir.to_path_buf())])
    }

    #[test]
    fn archive_parses_as_an_action() {
        let parsed = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"alpha","reason":"closed"}]}"#);
        assert_eq!(parsed.changes[0].action, ChangeAction::Archive);
        assert_eq!(ChangeAction::Archive.glyph(), "\u{2192} archive");
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
        assert!(
            moved.contains("alpha") && moved.contains("2026-08-11"),
            "{moved}"
        );
    }

    #[test]
    fn an_archive_change_naming_no_item_is_skipped() {
        let (dir, path, items) = workspace(DOC);
        let archive = dir.path().join("_archive");
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"nothing like this","reason":"r"}]}"#);

        let report = apply(
            dir.path(),
            &archive_map(&path, &archive),
            "2026-08-11",
            &items,
            &changes,
        );
        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped.len(), 1);
        assert!(
            report.skipped[0].contains("no matching item"),
            "{:?}",
            report.skipped
        );
    }

    // The rest of a change-set is still worth applying when archiving cannot run.
    #[test]
    fn without_an_archive_dir_the_archive_is_skipped_and_the_rest_applies() {
        let (dir, _path, items) = workspace(DOC);
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"alpha","reason":"r"},
            {"file":"lefv.md","action":"complete","content":"alpha","reason":"r"}]}"#);

        let report = apply(dir.path(), &HashMap::new(), "2026-08-11", &items, &changes);
        assert_eq!(report.applied, 1, "the complete still ran");
        assert_eq!(report.skipped.len(), 1);
        assert!(
            report.skipped[0].contains("no archive_dir configured"),
            "{:?}",
            report.skipped
        );
    }

    #[test]
    fn an_archive_row_reports_how_much_open_work_it_carries() {
        let body = "## P0\n\n- [ ] parent\n  - [ ] one\n  - [x] two\n";
        let (dir, _path, items) = workspace(body);
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"parent","reason":"r"}]}"#);
        assert_eq!(changes.open_sub_items(0, dir.path(), &items), 1);
    }

    // Archiving takes the whole subtree, so anything deeper counts too.
    #[test]
    fn open_sub_items_counts_the_whole_subtree_not_just_direct_children() {
        let body =
            "## P0\n\n- [ ] parent\n  - [ ] child\n    - [ ] grandchild\n    - [x] done one\n";
        let (dir, _path, items) = workspace(body);
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"parent","reason":"r"}]}"#);
        assert_eq!(changes.open_sub_items(0, dir.path(), &items), 2);
    }

    // Two groups can hold the same title; the count must come from the file the
    // change names, or it describes an item that is not being archived.
    #[test]
    fn open_sub_items_counts_within_the_file_the_change_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut items = Vec::new();
        // other.md first: a search that ignores the named file finds this copy,
        // and reports its two sub-items against lefv.md's childless one.
        for (name, body) in [
            (
                "other.md",
                "## P0\n\n- [ ] Review contract\n  - [ ] one\n  - [ ] two\n",
            ),
            ("lefv.md", "## P0\n\n- [ ] Review contract\n"),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, body).unwrap();
            items.extend(parse_todo_file(
                &path,
                name,
                body,
                &PriorityConfig::default(),
                &DueConfig::default(),
            ));
        }
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"Review contract","reason":"r"}]}"#);
        assert_eq!(
            changes.open_sub_items(0, dir.path(), &items),
            0,
            "lefv.md's copy has no sub-items; other.md's two must not be counted"
        );
    }

    // The row must describe the item `apply` would archive, so both resolve the
    // change the same way — an exact-equality count would report zero here.
    #[test]
    fn open_sub_items_resolves_the_change_the_way_apply_does() {
        let body = "## P0\n\n- [ ] File the 83(b) election\n  - [ ] pull the signature page\n";
        let (dir, path, items) = workspace(body);
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"archive","content":"the 83(b) election","reason":"r"}]}"#);

        assert_eq!(
            changes.open_sub_items(0, dir.path(), &items),
            1,
            "a partial title still resolves, as it does in apply"
        );

        let archive = dir.path().join("_archive");
        let report = apply(
            dir.path(),
            &archive_map(&path, &archive),
            "2026-08-11",
            &items,
            &changes,
        );
        assert_eq!(report.applied, 1, "and apply archives that same item");
    }

    #[test]
    fn a_non_archive_row_carries_no_open_sub_item_count() {
        let (dir, _path, items) = workspace(DOC);
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"complete","content":"alpha","reason":"r"}]}"#);
        assert_eq!(changes.open_sub_items(0, dir.path(), &items), 0);
    }

    // A group whose file exists but holds no items yet had no anchor to insert
    // after, so every add into it was skipped.
    #[test]
    fn an_add_into_an_empty_group_file_lands_under_its_heading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("newgroup.md");
        std::fs::write(&path, "## P0 — Critical\n\n## P1 — Later\n\n").unwrap();

        let changes = set(r#"{"summary":"s","changes":[
            {"file":"newgroup.md","action":"add","section":"P0","content":"first task","reason":"r"}]}"#);
        let report = apply(dir.path(), &HashMap::new(), "2026-08-12", &[], &changes);
        assert_eq!(report.applied, 1, "{:?}", report.skipped);

        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written, "## P0 — Critical\n\n- [ ] first task\n## P1 — Later\n\n",
            "inside the named section, and the other heading is untouched"
        );
        let p0 = written.find("## P0").unwrap();
        let task = written.find("first task").unwrap();
        let p1 = written.find("## P1").unwrap();
        assert!(p0 < task && task < p1, "it sits within P0: {written:?}");
    }

    #[test]
    fn an_add_into_an_empty_file_with_no_matching_section_is_appended() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("newgroup.md");
        std::fs::write(&path, "## P0 — Critical\n\n").unwrap();

        let changes = set(r#"{"summary":"s","changes":[
            {"file":"newgroup.md","action":"add","section":"P9 — Nope","content":"orphan","reason":"r"}]}"#);
        let report = apply(dir.path(), &HashMap::new(), "2026-08-12", &[], &changes);
        assert_eq!(report.applied, 1, "{:?}", report.skipped);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("- [ ] orphan")
        );
    }

    // Creating the group itself is a separate action; say so rather than
    // reporting a missing anchor.
    #[test]
    fn an_add_naming_a_file_that_does_not_exist_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"nosuch.md","action":"add","content":"a task","reason":"r"}]}"#);
        let report = apply(dir.path(), &HashMap::new(), "2026-08-12", &[], &changes);
        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped.len(), 1);
        assert!(
            report.skipped[0].contains("nosuch.md") && report.skipped[0].contains("does not exist"),
            "{:?}",
            report.skipped
        );
    }

    #[test]
    fn an_add_into_a_group_that_has_items_still_anchors_to_the_last_one() {
        let (dir, path, items) = workspace(DOC);
        let changes = set(r#"{"summary":"s","changes":[
            {"file":"lefv.md","action":"add","section":"P0","content":"another","reason":"r"}]}"#);
        let report = apply(dir.path(), &HashMap::new(), "2026-08-12", &items, &changes);
        assert_eq!(report.applied, 1, "{:?}", report.skipped);
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            "## P0 — Critical\n\n- [ ] alpha\n- [x] beta\n- [ ] another\n"
        );
    }

    #[test]
    fn parses_the_mcli_scan_shape() {
        let parsed = set(r#"{"summary":"two things","changes":[
                {"file":"lefv.md","action":"add","section":"P0","heading":"H",
                 "content":"- [ ] new thing","reason":"seen in email"}]}"#);
        assert_eq!(parsed.summary, "two things");
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].action, ChangeAction::Add);
    }

    #[test]
    fn a_fenced_change_set_is_parsed() {
        let fenced = "```json\n{\"summary\":\"s\",\"changes\":[{\"file\":\"f\",\"action\":\"add\",\"content\":\"c\",\"reason\":\"r\"}]}\n```";
        let parsed = ChangeSet::parse(fenced).expect("fenced JSON parses");
        assert_eq!(parsed.changes.len(), 1);
    }

    #[test]
    fn section_and_heading_are_optional() {
        let parsed = set(r#"{"summary":"","changes":[{"file":"f","action":"complete",
                "content":"x","reason":"done"}]}"#);
        assert_eq!(parsed.changes[0].section, "");
    }

    #[test]
    fn a_row_describes_its_change() {
        let parsed = set(r#"{"summary":"S","changes":[
                {"file":"lefv.md","action":"add","content":"- [ ] new thing","reason":"because"}]}"#);
        let row = parsed.row(0);
        assert!(row.contains("add"), "the action: {row}");
        assert!(row.contains("lefv.md"), "the file");
        assert!(row.contains("new thing"), "the text, without the checkbox");
        assert_eq!(parsed.reason(0), "because");
    }

    #[test]
    fn selecting_keeps_only_the_marked_changes() {
        let parsed = set(r#"{"summary":"S","changes":[
                {"file":"a.md","action":"add","content":"one","reason":"r"},
                {"file":"b.md","action":"add","content":"two","reason":"r"},
                {"file":"c.md","action":"add","content":"three","reason":"r"}]}"#);
        let kept = parsed.selected(&[true, false, true]);
        assert_eq!(kept.changes.len(), 2);
        assert_eq!(kept.changes[0].file, "a.md");
        assert_eq!(kept.changes[1].file, "c.md");
    }

    #[test]
    fn selecting_nothing_yields_an_empty_change_set() {
        let parsed = set(r#"{"summary":"","changes":[
                {"file":"a.md","action":"add","content":"one","reason":"r"}]}"#);
        assert!(parsed.selected(&[false]).changes.is_empty());
    }

    #[test]
    fn a_row_past_the_end_is_empty_rather_than_panicking() {
        assert_eq!(ChangeSet::default().row(3), "");
        assert_eq!(ChangeSet::default().reason(3), "");
    }

    #[test]
    fn add_appends_after_the_last_item() {
        let (dir, path, items) = workspace(DOC);
        let changes = set(
            r#"{"summary":"","changes":[{"file":"lefv.md","action":"add",
                "content":"- [ ] gamma","reason":"r"}]}"#,
        );
        let report = apply(dir.path(), &HashMap::new(), "2026-08-11", &items, &changes);
        assert_eq!(report.applied, 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "## P0 — Critical\n\n- [ ] alpha\n- [x] beta\n- [ ] gamma\n"
        );
    }

    #[test]
    fn complete_marks_a_matching_item_done() {
        let (dir, path, items) = workspace(DOC);
        let changes = set(
            r#"{"summary":"","changes":[{"file":"lefv.md","action":"complete",
                "content":"alpha","reason":"r"}]}"#,
        );
        assert_eq!(
            apply(dir.path(), &HashMap::new(), "2026-08-11", &items, &changes).applied,
            1
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("- [x] alpha")
        );
    }

    #[test]
    fn completing_an_already_done_item_is_a_no_op() {
        let (dir, _path, items) = workspace(DOC);
        let changes = set(
            r#"{"summary":"","changes":[{"file":"lefv.md","action":"complete",
                "content":"beta","reason":"r"}]}"#,
        );
        let report = apply(dir.path(), &HashMap::new(), "2026-08-11", &items, &changes);
        assert_eq!(report.applied, 1);
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn update_replaces_the_matching_item_text() {
        let (dir, path, items) = workspace(DOC);
        let changes = set(
            r#"{"summary":"","changes":[{"file":"lefv.md","action":"update",
                "content":"alpha, revised","reason":"r"}]}"#,
        );
        // "alpha, revised" contains "alpha", so it locates the existing item.
        assert_eq!(
            apply(dir.path(), &HashMap::new(), "2026-08-11", &items, &changes).applied,
            1
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("- [ ] alpha, revised")
        );
    }

    #[test]
    fn an_unlocatable_target_is_skipped_with_a_reason() {
        let (dir, path, items) = workspace(DOC);
        let before = std::fs::read_to_string(&path).unwrap();
        let changes = set(
            r#"{"summary":"","changes":[{"file":"lefv.md","action":"complete",
                "content":"nothing like this","reason":"r"}]}"#,
        );
        let report = apply(dir.path(), &HashMap::new(), "2026-08-11", &items, &changes);
        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].contains("no matching item"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "file untouched"
        );
    }

    #[test]
    fn a_stale_change_set_conflicts_rather_than_clobbering() {
        let (dir, path, items) = workspace(DOC);
        // Another writer edits the line after the agent read the workspace.
        std::fs::write(
            &path,
            "## P0 — Critical\n\n- [ ] alpha amended\n- [x] beta\n",
        )
        .unwrap();

        let changes = set(
            r#"{"summary":"","changes":[{"file":"lefv.md","action":"complete",
                "content":"alpha","reason":"r"}]}"#,
        );
        let report = apply(dir.path(), &HashMap::new(), "2026-08-11", &items, &changes);
        assert_eq!(report.applied, 0);
        assert!(
            report.skipped[0].contains("changed on disk"),
            "got {:?}",
            report.skipped
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("alpha amended"),
            "the other writer's change survives"
        );
    }

    // This once refused, on the grounds that a file with no items offered no safe
    // insertion point. Section headings do offer one, and the new-item dialog was
    // already using it, so the first item in a group is placed rather than
    // rejected. Refusal is kept only for a file that does not exist.
    #[test]
    fn adding_the_first_item_to_a_file_places_it_under_its_heading() {
        let (dir, path, items) = workspace("## P0 — Critical\n");
        let changes = set(
            r#"{"summary":"","changes":[{"file":"lefv.md","action":"add",
                "content":"first ever","reason":"r"}]}"#,
        );
        let report = apply(dir.path(), &HashMap::new(), "2026-08-11", &items, &changes);
        assert_eq!(report.applied, 1, "{:?}", report.skipped);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "## P0 — Critical\n- [ ] first ever\n"
        );
    }

    #[test]
    fn several_changes_apply_independently() {
        let (dir, path, items) = workspace(DOC);
        let changes = set(r#"{"summary":"","changes":[
                {"file":"lefv.md","action":"complete","content":"alpha","reason":"r"},
                {"file":"lefv.md","action":"complete","content":"missing","reason":"r"}]}"#);
        let report = apply(dir.path(), &HashMap::new(), "2026-08-11", &items, &changes);
        assert_eq!(report.applied, 1, "the good one still applies");
        assert_eq!(report.skipped.len(), 1);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("- [x] alpha")
        );
    }
}
