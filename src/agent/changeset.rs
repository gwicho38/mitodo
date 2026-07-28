//! The change-set an agent proposes, and how it is applied.
//!
//! Deliberately the same shape as the private `mcli todos scan` schema so an
//! existing prompt keeps working, but every change is reviewed before it
//! touches a file, and applying goes through the conflict-aware writer.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::store::{self, Item, WriteError};

/// Schema handed to the agent for the `scan` verb.
pub const SCAN_SCHEMA: &str = r#"{"type":"object","properties":{"changes":{"type":"array","items":{"type":"object","properties":{"file":{"type":"string"},"action":{"type":"string","enum":["add","complete","update"]},"section":{"type":"string"},"heading":{"type":"string"},"content":{"type":"string"},"reason":{"type":"string"}},"required":["file","action","content","reason"]}},"summary":{"type":"string"}},"required":["changes","summary"]}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeAction {
    Add,
    Complete,
    Update,
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
    pub fn parse(json: &str) -> Result<ChangeSet, serde_json::Error> {
        serde_json::from_str(json.trim())
    }

    /// One display line per change, for the review modal.
    pub fn review_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.summary.is_empty() {
            lines.push(self.summary.clone());
            lines.push(String::new());
        }
        for (index, change) in self.changes.iter().enumerate() {
            let verb = match change.action {
                ChangeAction::Add => "+ add     ",
                ChangeAction::Complete => "✓ complete",
                ChangeAction::Update => "~ update  ",
            };
            lines.push(format!("{:>2}. {verb} {}", index + 1, change.file));
            lines.push(format!("     {}", change.content));
            lines.push(format!("     why: {}", change.reason));
        }
        if self.changes.is_empty() {
            lines.push("no changes proposed".to_string());
        }
        lines
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub applied: usize,
    pub skipped: Vec<String>,
}

/// Apply a reviewed change-set.
///
/// `complete` and `update` locate their target by matching item text within the
/// named file, so a change-set stays valid even if line numbers moved since the
/// agent read the workspace. Anything that cannot be located is skipped and
/// reported rather than guessed at.
pub fn apply(root: &Path, items: &[Item], set: &ChangeSet) -> ApplyReport {
    let mut report = ApplyReport::default();

    for change in &set.changes {
        let path: PathBuf = root.join(&change.file);
        let result = match change.action {
            ChangeAction::Add => apply_add(&path, items, change),
            ChangeAction::Complete => apply_complete(&path, items, change),
            ChangeAction::Update => apply_update(&path, items, change),
        };
        match result {
            Ok(()) => report.applied += 1,
            Err(reason) => report.skipped.push(reason),
        }
    }
    report
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

    // Anchor to the last item in the requested section, else the last in the
    // file. Without an anchor there is no safe place to insert.
    let anchor = items
        .iter()
        .rfind(|i| i.file == path && (change.section.is_empty() || i.section == change.section))
        .or_else(|| items.iter().rfind(|i| i.file == path));

    let Some(anchor) = anchor else {
        return Err(format!(
            "add {:?}: no existing item in {} to anchor to",
            text,
            path.display()
        ));
    };

    store::add_item(path, anchor.line, &anchor.raw, anchor.indent, text)
        .map_err(|e| describe(e, "add", text))
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
    use crate::store::parse_todo_file;

    const DOC: &str = "## P0 — Critical\n\n- [ ] alpha\n- [x] beta\n";

    fn workspace(body: &str) -> (tempfile::TempDir, PathBuf, Vec<Item>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lefv.md");
        std::fs::write(&path, body).unwrap();
        let items = parse_todo_file(&path, "lefv.md", body);
        (dir, path, items)
    }

    fn set(json: &str) -> ChangeSet {
        ChangeSet::parse(json).expect("change-set parses")
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
    fn section_and_heading_are_optional() {
        let parsed = set(r#"{"summary":"","changes":[{"file":"f","action":"complete",
                "content":"x","reason":"done"}]}"#);
        assert_eq!(parsed.changes[0].section, "");
    }

    #[test]
    fn review_lines_describe_every_change() {
        let parsed = set(r#"{"summary":"S","changes":[
                {"file":"lefv.md","action":"add","content":"new","reason":"because"}]}"#);
        let text = parsed.review_lines().join("\n");
        assert!(text.contains("S"));
        assert!(text.contains("add"));
        assert!(text.contains("lefv.md"));
        assert!(text.contains("because"));
    }

    #[test]
    fn an_empty_change_set_says_so() {
        let parsed = ChangeSet::default();
        assert!(parsed.review_lines().join("\n").contains("no changes"));
    }

    #[test]
    fn add_appends_after_the_last_item() {
        let (dir, path, items) = workspace(DOC);
        let changes = set(
            r#"{"summary":"","changes":[{"file":"lefv.md","action":"add",
                "content":"- [ ] gamma","reason":"r"}]}"#,
        );
        let report = apply(dir.path(), &items, &changes);
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
        assert_eq!(apply(dir.path(), &items, &changes).applied, 1);
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
        let report = apply(dir.path(), &items, &changes);
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
        assert_eq!(apply(dir.path(), &items, &changes).applied, 1);
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
        let report = apply(dir.path(), &items, &changes);
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
        let report = apply(dir.path(), &items, &changes);
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

    #[test]
    fn adding_to_a_file_with_no_items_is_skipped_not_guessed() {
        let (dir, _path, items) = workspace("## P0 — Critical\n");
        let changes = set(
            r#"{"summary":"","changes":[{"file":"lefv.md","action":"add",
                "content":"first ever","reason":"r"}]}"#,
        );
        let report = apply(dir.path(), &items, &changes);
        assert_eq!(report.applied, 0);
        assert!(report.skipped[0].contains("no existing item"));
    }

    #[test]
    fn several_changes_apply_independently() {
        let (dir, path, items) = workspace(DOC);
        let changes = set(r#"{"summary":"","changes":[
                {"file":"lefv.md","action":"complete","content":"alpha","reason":"r"},
                {"file":"lefv.md","action":"complete","content":"missing","reason":"r"}]}"#);
        let report = apply(dir.path(), &items, &changes);
        assert_eq!(report.applied, 1, "the good one still applies");
        assert_eq!(report.skipped.len(), 1);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("- [x] alpha")
        );
    }
}
