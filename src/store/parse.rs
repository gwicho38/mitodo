use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

use chrono::NaiveDate;

use crate::config::{DueConfig, PriorityConfig, PrioritySource};

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

/// Map a captured priority marker to a band.
///
/// Accepts digits (`0`–`3`) and letters (`A`–`D`), so one `pattern` setting
/// covers both `## P1 — High` headings and todo.txt-style `(A)` tags.
fn band(marker: &str) -> Priority {
    match marker.trim().to_ascii_uppercase().as_str() {
        "0" | "A" => Priority::P0,
        "1" | "B" => Priority::P1,
        "2" | "C" => Priority::P2,
        "3" | "D" => Priority::P3,
        _ => Priority::None,
    }
}

/// Priority of an item, per the workspace's configuration.
///
/// The same pattern is applied to whichever subject `source` selects: the
/// section heading, or the item's own text.
fn derive_priority(
    matcher: Option<&Regex>,
    source: PrioritySource,
    section: &str,
    text: &str,
) -> Priority {
    let Some(matcher) = matcher else {
        return Priority::None;
    };
    let subject = match source {
        PrioritySource::None => return Priority::None,
        PrioritySource::Heading => section,
        PrioritySource::Tag => text,
    };
    matcher
        .captures(subject)
        .and_then(|caps| caps.get(1))
        .map(|m| band(m.as_str()))
        .unwrap_or(Priority::None)
}

/// Pull a due date out of an item's text, per the configured pattern.
fn derive_due(matcher: Option<&Regex>, text: &str) -> Option<NaiveDate> {
    matcher?
        .captures(text)?
        .get(1)
        .and_then(|m| NaiveDate::parse_from_str(m.as_str(), "%Y-%m-%d").ok())
}

/// Compile a pattern, disabling the feature rather than failing the parse.
fn compile(pattern: &str, what: &str) -> Option<Regex> {
    match Regex::new(pattern) {
        Ok(re) => Some(re),
        Err(err) => {
            log::warn!("{what} pattern {pattern:?} is not a valid regex ({err}); disabled");
            None
        }
    }
}

/// Parse one markdown todo file into items in document order.
///
/// `file_rel` is the workspace-relative path used for id computation, so that
/// identical text in two different files yields different ids.
pub fn parse_todo_file(
    path: &Path,
    file_rel: &str,
    source: &str,
    priority_config: &PriorityConfig,
    due_config: &DueConfig,
) -> Vec<Item> {
    // Compiled once per file. An unusable pattern disables priorities rather
    // than failing the parse, so a bad config never costs you your todo list.
    let matcher = match priority_config.source {
        PrioritySource::None => None,
        _ => compile(&priority_config.pattern, "priority"),
    };
    let due_matcher = due_config
        .enabled
        .then(|| compile(&due_config.pattern, "due"))
        .flatten();

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

            let priority =
                derive_priority(matcher.as_ref(), priority_config.source, &section, &text);
            let due = derive_due(due_matcher.as_ref(), &text);
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
                priority,
                due,
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
    use chrono::NaiveDate;

    use crate::config::{DueConfig, PriorityConfig, PrioritySource};
    use std::path::Path;

    fn heading_config() -> PriorityConfig {
        PriorityConfig {
            source: PrioritySource::Heading,
            pattern: "^P([0-3])".to_string(),
        }
    }

    fn fixture() -> Vec<Item> {
        let source = include_str!("../../tests/fixtures/basic/TODO.md");
        parse_todo_file(
            Path::new("lefv/TODO.md"),
            "lefv/TODO.md",
            source,
            &heading_config(),
            &DueConfig::default(),
        )
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
    fn priority_source_none_disables_priorities() {
        let cfg = PriorityConfig {
            source: PrioritySource::None,
            pattern: "^P([0-3])".to_string(),
        };
        let source = include_str!("../../tests/fixtures/basic/TODO.md");
        let items = parse_todo_file(Path::new("f"), "f", source, &cfg, &DueConfig::default());
        assert!(
            items.iter().all(|i| i.priority == Priority::None),
            "source = none must mean no priorities, whatever the headings say"
        );
    }

    #[test]
    fn priority_source_tag_reads_the_item_text() {
        let cfg = PriorityConfig {
            source: PrioritySource::Tag,
            pattern: r"\(([A-D])\)".to_string(),
        };
        let doc = "## Anything\n\n- [ ] (A) urgent thing\n- [ ] (C) later thing\n- [ ] untagged\n";
        let items = parse_todo_file(Path::new("f"), "f", doc, &cfg, &DueConfig::default());
        assert_eq!(items[0].priority, Priority::P0, "(A) is the top band");
        assert_eq!(items[1].priority, Priority::P2, "(C) is the third band");
        assert_eq!(items[2].priority, Priority::None, "untagged");
    }

    #[test]
    fn a_custom_heading_pattern_is_honoured() {
        let cfg = PriorityConfig {
            source: PrioritySource::Heading,
            pattern: r"prio-([0-3])".to_string(),
        };
        let doc = "## prio-1 things\n\n- [ ] a\n";
        let items = parse_todo_file(Path::new("f"), "f", doc, &cfg, &DueConfig::default());
        assert_eq!(items[0].priority, Priority::P1);
    }

    #[test]
    fn an_invalid_pattern_disables_priorities_rather_than_panicking() {
        let cfg = PriorityConfig {
            source: PrioritySource::Heading,
            pattern: "([unclosed".to_string(),
        };
        let doc = "## P0 — Critical\n\n- [ ] a\n";
        let items = parse_todo_file(Path::new("f"), "f", doc, &cfg, &DueConfig::default());
        assert_eq!(items[0].priority, Priority::None);
    }

    fn due_items(doc: &str, due: &DueConfig) -> Vec<Item> {
        parse_todo_file(Path::new("f"), "f", doc, &PriorityConfig::default(), due)
    }

    #[test]
    fn reads_an_iso_due_date_out_of_the_item_text() {
        let items = due_items(
            "- [ ] file the brief due:2026-08-01\n",
            &DueConfig::default(),
        );
        assert_eq!(
            items[0].due,
            Some(NaiveDate::from_ymd_opt(2026, 8, 1).unwrap())
        );
    }

    #[test]
    fn items_without_a_date_have_none() {
        let items = due_items("- [ ] no deadline here\n", &DueConfig::default());
        assert_eq!(items[0].due, None);
    }

    #[test]
    fn an_unparseable_date_is_ignored_rather_than_guessed() {
        let items = due_items("- [ ] due:2026-13-45\n", &DueConfig::default());
        assert_eq!(items[0].due, None, "month 13 is not a date");
    }

    #[test]
    fn a_custom_due_pattern_is_honoured() {
        let cfg = DueConfig {
            enabled: true,
            pattern: r"\(due (\d{4}-\d{2}-\d{2})\)".to_string(),
        };
        let items = due_items("- [ ] hearing (due 2026-09-15)\n", &cfg);
        assert_eq!(
            items[0].due,
            Some(NaiveDate::from_ymd_opt(2026, 9, 15).unwrap())
        );
    }

    #[test]
    fn due_dates_can_be_turned_off() {
        let cfg = DueConfig {
            enabled: false,
            ..Default::default()
        };
        let items = due_items("- [ ] thing due:2026-08-01\n", &cfg);
        assert_eq!(items[0].due, None);
    }

    #[test]
    fn an_invalid_due_pattern_disables_dates_rather_than_panicking() {
        let cfg = DueConfig {
            enabled: true,
            pattern: "([unclosed".to_string(),
        };
        let items = due_items("- [ ] thing due:2026-08-01\n", &cfg);
        assert_eq!(items[0].due, None);
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
