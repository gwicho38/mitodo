use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

use super::model::{Item, ItemId, Priority};

static CHECKBOX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\s*)- \[([ xX])\] (.+)$").expect("valid checkbox regex"));
static HEADING2_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^## (.+)$").expect("valid h2 regex"));
static HEADING3_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^### (.+)$").expect("valid h3 regex"));
static DESC_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s+>\s?(.*)$").expect("valid description regex"));

/// Number of leading lines occupied by a YAML frontmatter block, or 0 if the
/// file does not open with one.
fn frontmatter_len(lines: &[&str]) -> usize {
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return 0;
    }
    match lines.iter().skip(1).position(|l| l.trim_end() == "---") {
        // +2 accounts for the opening delimiter and the closing one.
        Some(offset) => offset + 2,
        None => 0,
    }
}

/// Parse one markdown todo file into items in document order.
///
/// `file_rel` is the workspace-relative path used for id computation, so that
/// identical text in two different files yields different ids.
pub fn parse_todo_file(path: &Path, file_rel: &str, source: &str) -> Vec<Item> {
    let lines: Vec<&str> = source.lines().collect();
    let start = frontmatter_len(&lines);

    let mut items: Vec<Item> = Vec::new();
    let mut section = String::new();
    let mut heading = String::new();
    // (indent width, index into `items`) for each open ancestor.
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut last: Option<usize> = None;

    for (line_no, raw) in lines.iter().enumerate().skip(start) {
        if let Some(caps) = HEADING2_RE.captures(raw) {
            section = caps[1].trim().to_string();
            heading.clear();
            stack.clear();
            last = None;
            continue;
        }

        if let Some(caps) = HEADING3_RE.captures(raw) {
            heading = caps[1].trim().to_string();
            stack.clear();
            last = None;
            continue;
        }

        if let Some(caps) = CHECKBOX_RE.captures(raw) {
            let indent = caps[1].chars().count();
            let done = caps[2].eq_ignore_ascii_case("x");
            let text = caps[3].trim().to_string();

            // Close any ancestors at or below this indent level.
            while stack.last().is_some_and(|(width, _)| *width >= indent) {
                stack.pop();
            }
            let parent_idx = stack.last().map(|(_, idx)| *idx);

            let id = ItemId::compute(file_rel, &section, &heading, indent, &text);
            let item = Item {
                id: id.clone(),
                file: path.to_path_buf(),
                line: line_no,
                indent,
                done,
                text,
                raw: (*raw).to_string(),
                description: String::new(),
                section: section.clone(),
                heading: heading.clone(),
                priority: Priority::from_heading(&section),
                parent: parent_idx.map(|idx| items[idx].id.clone()),
                children: Vec::new(),
            };

            let idx = items.len();
            items.push(item);
            if let Some(pidx) = parent_idx {
                items[pidx].children.push(id);
            }
            stack.push((indent, idx));
            last = Some(idx);
            continue;
        }

        if let Some(caps) = DESC_RE.captures(raw)
            && let Some(idx) = last
        {
            let note = caps[1].trim_end();
            let description = &mut items[idx].description;
            if description.is_empty() {
                description.push_str(note);
            } else {
                description.push('\n');
                description.push_str(note);
            }
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture() -> Vec<Item> {
        let source = include_str!("../../tests/fixtures/basic/TODO.md");
        parse_todo_file(Path::new("lefv/TODO.md"), "lefv/TODO.md", source)
    }

    #[test]
    fn records_the_raw_line_verbatim() {
        let source = include_str!("../../tests/fixtures/basic/TODO.md");
        let lines: Vec<&str> = source.lines().collect();
        for item in fixture() {
            assert_eq!(
                item.raw, lines[item.line],
                "raw must match the source line byte for byte"
            );
        }
    }

    #[test]
    fn frontmatter_is_not_parsed_as_content() {
        let items = fixture();
        assert!(items.iter().all(|i| !i.text.contains("tags")));
    }

    #[test]
    fn finds_every_checkbox() {
        assert_eq!(fixture().len(), 6);
    }

    #[test]
    fn records_done_state() {
        let items = fixture();
        assert!(items[0].done, "first item is [x]");
        assert!(!items[1].done, "second item is [ ]");
    }

    #[test]
    fn assigns_section_and_heading() {
        let items = fixture();
        assert_eq!(items[0].section, "P0 — Critical / Time-Sensitive");
        assert_eq!(items[0].heading, "Prefecture Appointment");
        assert_eq!(items[2].section, "P1 — High Priority");
        assert_eq!(items[2].heading, "Holon Law");
    }

    #[test]
    fn derives_priority_from_section() {
        let items = fixture();
        assert_eq!(items[0].priority, Priority::P0);
        assert_eq!(items[2].priority, Priority::P1);
    }

    #[test]
    fn collects_multi_line_descriptions() {
        let items = fixture();
        assert_eq!(
            items[1].description,
            "do NOT miss — government immigration\nbring the original convocation letter"
        );
    }

    #[test]
    fn items_without_a_description_have_an_empty_one() {
        assert_eq!(fixture()[0].description, "");
    }

    #[test]
    fn nests_indented_items_under_their_parent() {
        let items = fixture();
        let parent = &items[2];
        assert_eq!(parent.text, "Respond to Shani Phillips");
        assert_eq!(parent.children.len(), 2);
        assert_eq!(items[3].parent.as_ref(), Some(&parent.id));
        assert_eq!(items[4].parent.as_ref(), Some(&parent.id));
    }

    #[test]
    fn top_level_items_have_no_parent() {
        assert!(fixture()[0].parent.is_none());
    }

    #[test]
    fn records_zero_based_line_numbers() {
        let source = include_str!("../../tests/fixtures/basic/TODO.md");
        let items = fixture();
        let lines: Vec<&str> = source.lines().collect();
        for item in &items {
            assert!(
                lines[item.line].contains(&item.text),
                "line {} should contain {:?}",
                item.line,
                item.text
            );
        }
    }

    #[test]
    fn a_new_section_resets_nesting() {
        let items = fixture();
        // items[2] opens a new section; it must not be adopted by the
        // parent that was open at the end of the previous section.
        assert!(items[2].parent.is_none());
    }
}
