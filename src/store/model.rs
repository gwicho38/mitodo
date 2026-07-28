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
/// Deliberately derived from content rather than position: a text edit yields a
/// new id, which matches the scheme used by the `todos-mcp` server so both
/// tools agree on item identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(String);

impl ItemId {
    pub fn compute(
        file_rel: &str,
        section: &str,
        heading: &str,
        indent: usize,
        text: &str,
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
    /// Blockquote lines directly beneath the item, joined with newlines.
    pub description: String,
    /// The `## ` heading in force above this item.
    pub section: String,
    /// The `### ` heading in force above this item.
    pub heading: String,
    pub priority: Priority,
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
        assert_eq!(Priority::from_heading("P0 — Critical / Time-Sensitive"), Priority::P0);
        assert_eq!(Priority::from_heading("P1 — High Priority"), Priority::P1);
        assert_eq!(Priority::from_heading("P3 — Someday"), Priority::P3);
        assert_eq!(Priority::from_heading("Notes"), Priority::None);
        assert_eq!(Priority::from_heading(""), Priority::None);
    }

    #[test]
    fn item_id_is_stable_for_identical_content() {
        let a = ItemId::compute("lefv/TODO.md", "P0", "Prefecture", 0, "Check convocation");
        let b = ItemId::compute("lefv/TODO.md", "P0", "Prefecture", 0, "Check convocation");
        assert_eq!(a, b);
    }

    #[test]
    fn item_id_changes_when_text_changes() {
        let a = ItemId::compute("lefv/TODO.md", "P0", "Prefecture", 0, "Check convocation");
        let b = ItemId::compute("lefv/TODO.md", "P0", "Prefecture", 0, "Check convocation now");
        assert_ne!(a, b);
    }

    #[test]
    fn item_id_distinguishes_same_text_in_different_files() {
        let a = ItemId::compute("lefv/TODO.md", "P0", "H", 0, "same text");
        let b = ItemId::compute("jzlaw/TODO.md", "P0", "H", 0, "same text");
        assert_ne!(a, b);
    }

    #[test]
    fn item_id_is_twelve_hex_chars() {
        let id = ItemId::compute("f", "s", "h", 0, "t");
        assert_eq!(id.as_str().len(), 12);
        assert!(id.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }
}
