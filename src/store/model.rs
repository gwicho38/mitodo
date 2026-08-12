use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// Derived priority of an item. `None` means the workspace has no priority
/// source configured, or the item's section did not match one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Priority {
    P0,
    P1,
    P2,
    P3,
    #[default]
    None,
}

impl Priority {
    /// Parse a leading `P0`–`P3` out of a section heading such as
    /// `"P1 — High Priority"`. Anything else is `Priority::None`.
    pub fn from_heading(heading: &str) -> Self {
        let mut chars = heading.trim_start().chars();
        if chars.next() != Some('P') {
            return Priority::None;
        }
        match chars.next() {
            Some('0') => Priority::P0,
            Some('1') => Priority::P1,
            Some('2') => Priority::P2,
            Some('3') => Priority::P3,
            _ => Priority::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::P0 => "P0",
            Priority::P1 => "P1",
            Priority::P2 => "P2",
            Priority::P3 => "P3",
            Priority::None => "-",
        }
    }
}

/// Content-addressed item identifier.
///
/// Derived from content rather than position, so a text edit yields a new id and
/// a moved item keeps its own. `occurrence` counts identical items earlier in the
/// same file: without it, two items alike in text, section, heading and indent
/// share an id, and anything addressing items by id can write to the wrong one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(String);

impl ItemId {
    pub fn compute(
        file_rel: &str,
        section: &str,
        heading: &str,
        indent: usize,
        text: &str,
        occurrence: usize,
    ) -> Self {
        let mut hasher = Sha256::new();
        // Unit separator between fields so that concatenation is unambiguous.
        for field in [file_rel, section, heading] {
            hasher.update(field.as_bytes());
            hasher.update([0x1f]);
        }
        hasher.update(indent.to_string().as_bytes());
        hasher.update([0x1f]);
        hasher.update(text.as_bytes());
        hasher.update([0x1f]);
        hasher.update(occurrence.to_string().as_bytes());
        let digest = hasher.finalize();
        ItemId(hex_12(&digest))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn hex_12(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

/// A single checkbox line, plus the description block beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: ItemId,
    pub file: PathBuf,
    /// 0-based line index of the checkbox line within `file`.
    pub line: usize,
    /// Leading whitespace width of the checkbox line, in characters.
    pub indent: usize,
    pub done: bool,
    pub text: String,
    /// The checkbox line exactly as it appears in the file.
    ///
    /// Every write verifies this against the file before touching it, so it
    /// must stay byte-identical to the source rather than reconstructed.
    pub raw: String,
    /// Blockquote lines directly beneath the item, joined with newlines.
    pub description: String,
    /// The `## ` heading in force above this item.
    pub section: String,
    /// The `### ` heading in force above this item.
    pub heading: String,
    pub priority: Priority,
    /// Deadline parsed out of the item's text, if it carries one.
    pub due: Option<chrono::NaiveDate>,
    pub parent: Option<ItemId>,
    pub children: Vec<ItemId>,
}

/// One account, project, or context — a top-level node in the UI's left pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub todo_file: PathBuf,
    pub notes_file: Option<PathBuf>,
    pub archive_dir: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_parses_from_section_heading() {
        assert_eq!(
            Priority::from_heading("P0 — Critical / Time-Sensitive"),
            Priority::P0
        );
        assert_eq!(Priority::from_heading("P1 — High Priority"), Priority::P1);
        assert_eq!(Priority::from_heading("P3 — Someday"), Priority::P3);
        assert_eq!(Priority::from_heading("Notes"), Priority::None);
        assert_eq!(Priority::from_heading(""), Priority::None);
    }

    #[test]
    fn item_id_is_stable_for_identical_content() {
        let a = ItemId::compute(
            "lefv/TODO.md",
            "P0",
            "Prefecture",
            0,
            "Check convocation",
            0,
        );
        let b = ItemId::compute(
            "lefv/TODO.md",
            "P0",
            "Prefecture",
            0,
            "Check convocation",
            0,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn item_id_changes_when_text_changes() {
        let a = ItemId::compute(
            "lefv/TODO.md",
            "P0",
            "Prefecture",
            0,
            "Check convocation",
            0,
        );
        let b = ItemId::compute(
            "lefv/TODO.md",
            "P0",
            "Prefecture",
            0,
            "Check convocation now",
            0,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn item_id_distinguishes_same_text_in_different_files() {
        let a = ItemId::compute("lefv/TODO.md", "P0", "H", 0, "same text", 0);
        let b = ItemId::compute("jzlaw/TODO.md", "P0", "H", 0, "same text", 0);
        assert_ne!(a, b);
    }

    // Two items can be identical in text, section, heading and indent. Without
    // the occurrence index they shared an id, and a tool addressing items by id
    // would write to whichever one it found first.
    #[test]
    fn item_id_distinguishes_repeated_occurrences() {
        let first = ItemId::compute("lefv/TODO.md", "P0", "H", 0, "call the bank", 0);
        let second = ItemId::compute("lefv/TODO.md", "P0", "H", 0, "call the bank", 1);
        assert_ne!(first, second);
    }

    #[test]
    fn duplicate_items_in_one_file_all_get_distinct_ids() {
        use crate::config::{DueConfig, PriorityConfig};
        use crate::store::parse_todo_file;
        let body = concat!(
            "## P0 — Critical\n\n",
            "- [ ] call the bank\n",
            "- [ ] call the bank\n",
            "- [ ] alpha\n",
            "  - [ ] ping\n",
            "- [ ] beta\n",
            "  - [ ] ping\n",
        );
        let items = parse_todo_file(
            std::path::Path::new("/w/lefv/TODO.md"),
            "lefv/TODO.md",
            body,
            &PriorityConfig::default(),
            &DueConfig::default(),
        );
        assert_eq!(items.len(), 6);
        let distinct: std::collections::HashSet<&str> =
            items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(distinct.len(), items.len(), "every id is unique");
    }

    #[test]
    fn item_id_is_twelve_hex_chars() {
        let id = ItemId::compute("f", "s", "h", 0, "t", 0);
        assert_eq!(id.as_str().len(), 12);
        assert!(id.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }
}
