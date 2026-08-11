//! Moving finished items out of the working file.
//!
//! `archive_dir` is detected and recorded per group; this is what it is for.
//! Archiving is a move, not a delete: the lines are appended verbatim to
//! `<archive_dir>/TODO.md` under a dated heading before being removed, so
//! nothing is lost and the text can be pasted back.

use std::path::Path;

use super::model::Item;
use super::write::{WriteError, read_lines, verify, write_lines};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ArchiveReport {
    pub archived: usize,
    pub skipped: Vec<String>,
}

/// Indentation width of a line, in characters.
fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

fn is_checkbox(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- [") && trimmed.len() > 4
}

fn is_description(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('>') && line.len() > trimmed.len()
}

/// Index one past the last line belonging to the item on `start`: its
/// description block, plus every nested child and their descriptions.
fn subtree_end(lines: &[String], start: usize) -> usize {
    let base = indent_of(&lines[start]);
    let mut end = start + 1;
    while end < lines.len() {
        let line = &lines[end];
        // Both a nested checkbox and a description belong to this item; only
        // the indentation decides, so the two cases share a condition.
        let belongs = (is_description(line) || is_checkbox(line)) && indent_of(line) > base;
        if !belongs {
            break;
        }
        end += 1;
    }
    end
}

/// True if every checkbox in `range` is ticked.
fn wholly_done(lines: &[String], range: std::ops::Range<usize>) -> bool {
    lines[range]
        .iter()
        .filter(|l| is_checkbox(l))
        .all(|l| l.trim_start().starts_with("- [x]") || l.trim_start().starts_with("- [X]"))
}

/// Move finished items from `todo_file` into `archive_dir/TODO.md`.
///
/// A done item whose subtree still contains open work is left alone and
/// reported, since archiving it would hide those open items.
pub fn archive_done(
    todo_file: &Path,
    archive_dir: &Path,
    items: &[Item],
    date: &str,
) -> Result<ArchiveReport, WriteError> {
    let (lines, _, _) = read_lines(todo_file)?;
    let mut report = ArchiveReport::default();

    // Candidates are top-level done items in this file, deepest last so that
    // removing them does not disturb the lines above.
    let mut candidates: Vec<&Item> = items
        .iter()
        .filter(|i| i.file == todo_file && i.done && i.parent.is_none())
        .collect();
    candidates.sort_by_key(|i| i.line);

    let mut targets: Vec<&Item> = Vec::new();
    for item in candidates {
        // A line that moved fails `verify` inside archive_items; this guard only
        // needs the lines it can still trust.
        if item.line < lines.len()
            && !wholly_done(&lines, item.line..subtree_end(&lines, item.line))
        {
            report
                .skipped
                .push(format!("{:?}: has open sub-items", item.text));
            continue;
        }
        targets.push(item);
    }

    let moved = archive_items(todo_file, archive_dir, &targets, date)?;
    report.archived = moved.archived;
    report.skipped.extend(moved.skipped);
    Ok(report)
}

/// Move each named item, and everything under it, into `archive_dir/TODO.md`.
///
/// Unconditional on state: the caller decides what deserves archiving. An item
/// whose line no longer matches the file is skipped and reported rather than
/// guessed at.
pub fn archive_items(
    todo_file: &Path,
    archive_dir: &Path,
    targets: &[&Item],
    date: &str,
) -> Result<ArchiveReport, WriteError> {
    let (mut lines, ending, trailing) = read_lines(todo_file)?;
    let mut report = ArchiveReport::default();

    let mut ordered: Vec<&&Item> = targets.iter().collect();
    ordered.sort_by_key(|i| i.line);

    let mut moved: Vec<(usize, usize)> = Vec::new();
    let mut block: Vec<String> = Vec::new();

    for item in ordered {
        if verify(todo_file, &lines, item.line, &item.raw).is_err() {
            report
                .skipped
                .push(format!("{:?}: file changed on disk", item.text));
            continue;
        }
        let end = subtree_end(&lines, item.line);
        block.extend_from_slice(&lines[item.line..end]);
        moved.push((item.line, end));
        report.archived += 1;
    }

    if moved.is_empty() {
        return Ok(report);
    }

    append_to_archive(archive_dir, &block, date, ending.as_str())?;

    // Remove bottom-up so earlier ranges keep their indices.
    for (start, end) in moved.into_iter().rev() {
        lines.drain(start..end);
    }
    write_lines(todo_file, &lines, ending, trailing)?;

    Ok(report)
}

fn append_to_archive(
    archive_dir: &Path,
    block: &[String],
    date: &str,
    ending: &str,
) -> Result<(), WriteError> {
    std::fs::create_dir_all(archive_dir)?;
    let path = archive_dir.join("TODO.md");

    let mut out = String::new();
    if path.exists() {
        let existing = std::fs::read_to_string(&path)?;
        out.push_str(&existing);
        if !existing.is_empty() && !existing.ends_with('\n') {
            out.push_str(ending);
        }
        out.push_str(ending);
    }
    out.push_str(&format!("## Archived {date}{ending}{ending}"));
    for line in block {
        out.push_str(line);
        out.push_str(ending);
    }
    std::fs::write(&path, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DueConfig, PriorityConfig};
    use crate::store::parse_todo_file;
    use std::path::PathBuf;

    fn workspace(body: &str) -> (tempfile::TempDir, PathBuf, PathBuf, Vec<Item>) {
        let dir = tempfile::tempdir().unwrap();
        let todo = dir.path().join("TODO.md");
        let archive = dir.path().join("_archive");
        std::fs::write(&todo, body).unwrap();
        let items = parse_todo_file(
            &todo,
            "TODO.md",
            body,
            &PriorityConfig::default(),
            &DueConfig::default(),
        );
        (dir, todo, archive, items)
    }

    fn run(body: &str) -> (tempfile::TempDir, PathBuf, PathBuf, ArchiveReport) {
        let (dir, todo, archive, items) = workspace(body);
        let report = archive_done(&todo, &archive, &items, "2026-07-28").unwrap();
        (dir, todo, archive, report)
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    // The agent path archives one named item, whatever its state — unlike `X`,
    // which only sweeps finished ones.
    #[test]
    fn one_named_open_item_is_moved_and_its_siblings_are_left() {
        let (_dir, todo, archive, items) = workspace("## P0\n\n- [ ] alpha\n- [ ] beta\n");
        let target: Vec<&Item> = items.iter().filter(|i| i.text == "alpha").collect();
        assert_eq!(target.len(), 1, "fixture has one alpha");

        let report = archive_items(&todo, &archive, &target, "2026-08-11").unwrap();
        assert_eq!(report.archived, 1);

        let left = read(&todo);
        assert!(!left.contains("alpha"), "moved out: {left}");
        assert!(left.contains("beta"), "sibling untouched: {left}");

        let moved = read(&archive.join("TODO.md"));
        assert!(moved.contains("## Archived 2026-08-11"));
        assert!(moved.contains("- [ ] alpha"), "verbatim: {moved}");
    }

    #[test]
    fn an_items_subtree_travels_with_it() {
        let body = "## P0\n\n- [ ] parent\n  > why it matters\n  - [ ] child\n- [ ] other\n";
        let (_dir, todo, archive, items) = workspace(body);
        let target: Vec<&Item> = items.iter().filter(|i| i.text == "parent").collect();

        archive_items(&todo, &archive, &target, "2026-08-11").unwrap();

        let left = read(&todo);
        assert!(
            !left.contains("child") && !left.contains("why it matters"),
            "{left}"
        );
        assert!(left.contains("other"));
        let moved = read(&archive.join("TODO.md"));
        assert!(
            moved.contains("child") && moved.contains("why it matters"),
            "{moved}"
        );
    }

    #[test]
    fn an_item_whose_line_moved_on_disk_is_skipped_not_guessed_at() {
        let (_dir, todo, archive, items) = workspace("## P0\n\n- [ ] alpha\n- [ ] beta\n");
        let target: Vec<&Item> = items.iter().filter(|i| i.text == "alpha").collect();
        std::fs::write(&todo, "## P0\n\n- [ ] something else entirely\n").unwrap();

        let report = archive_items(&todo, &archive, &target, "2026-08-11").unwrap();
        assert_eq!(report.archived, 0);
        assert_eq!(report.skipped.len(), 1);
        assert!(
            report.skipped[0].contains("file changed on disk"),
            "{:?}",
            report.skipped
        );
    }

    #[test]
    fn archiving_nothing_writes_nothing() {
        let (_dir, todo, archive, _items) = workspace("## P0\n\n- [ ] alpha\n");
        let before = read(&todo);
        let report = archive_items(&todo, &archive, &[], "2026-08-11").unwrap();
        assert_eq!(report.archived, 0);
        assert_eq!(read(&todo), before);
        assert!(!archive.join("TODO.md").exists(), "no empty archive file");
    }

    #[test]
    fn moves_done_items_out_of_the_working_file() {
        let (_d, todo, _a, report) = run("## P0\n\n- [ ] open\n- [x] finished\n");
        assert_eq!(report.archived, 1);
        assert_eq!(read(&todo), "## P0\n\n- [ ] open\n");
    }

    #[test]
    fn writes_them_to_the_archive_under_a_dated_heading() {
        let (_d, _t, archive, _r) = run("## P0\n\n- [x] finished\n");
        let text = read(&archive.join("TODO.md"));
        assert!(text.contains("## Archived 2026-07-28"));
        assert!(text.contains("- [x] finished"));
    }

    #[test]
    fn nothing_to_archive_leaves_both_files_alone() {
        let body = "## P0\n\n- [ ] open\n";
        let (_d, todo, archive, report) = run(body);
        assert_eq!(report.archived, 0);
        assert_eq!(read(&todo), body);
        assert!(
            !archive.join("TODO.md").exists(),
            "no empty archive created"
        );
    }

    #[test]
    fn a_description_travels_with_its_item() {
        let (_d, todo, archive, _r) =
            run("## P0\n\n- [x] finished\n  > why it mattered\n- [ ] open\n");
        assert_eq!(read(&todo), "## P0\n\n- [ ] open\n");
        assert!(read(&archive.join("TODO.md")).contains("> why it mattered"));
    }

    #[test]
    fn a_fully_done_subtree_moves_as_one() {
        let (_d, todo, archive, report) =
            run("## P0\n\n- [x] parent\n  - [x] child\n  - [x] other\n- [ ] open\n");
        assert_eq!(report.archived, 1);
        assert_eq!(read(&todo), "## P0\n\n- [ ] open\n");
        let archived = read(&archive.join("TODO.md"));
        assert!(archived.contains("  - [x] child"), "indentation preserved");
        assert!(archived.contains("  - [x] other"));
    }

    #[test]
    fn a_done_parent_with_open_children_is_left_alone() {
        let body = "## P0\n\n- [x] parent\n  - [ ] still open\n";
        let (_d, todo, _a, report) = run(body);
        assert_eq!(report.archived, 0);
        assert_eq!(read(&todo), body, "archiving it would hide open work");
        assert!(report.skipped[0].contains("open sub-items"));
    }

    #[test]
    fn several_items_archive_in_one_pass() {
        let (_d, todo, _a, report) =
            run("## P0\n\n- [x] one\n- [ ] keep\n- [x] two\n- [x] three\n");
        assert_eq!(report.archived, 3);
        assert_eq!(read(&todo), "## P0\n\n- [ ] keep\n");
    }

    #[test]
    fn archiving_twice_appends_rather_than_overwriting() {
        let (dir, todo, archive, _r) = run("## P0\n\n- [x] first\n");
        std::fs::write(&todo, "## P0\n\n- [x] second\n").unwrap();
        let items = parse_todo_file(
            &todo,
            "TODO.md",
            &read(&todo),
            &PriorityConfig::default(),
            &DueConfig::default(),
        );
        archive_done(&todo, &archive, &items, "2026-07-29").unwrap();

        let text = read(&archive.join("TODO.md"));
        assert!(text.contains("- [x] first"), "earlier archive survives");
        assert!(text.contains("- [x] second"));
        assert!(text.contains("## Archived 2026-07-28"));
        assert!(text.contains("## Archived 2026-07-29"));
        drop(dir);
    }

    #[test]
    fn everything_the_item_did_not_own_is_byte_identical() {
        let body = "---\ntags: [a]\n---\n\n# Title\n\n## P0\n\n- [ ] keep me\t\n- [x] go\n\n## P1\n\n- [ ] also keep\n";
        let (_d, todo, _a, _r) = run(body);
        assert_eq!(
            read(&todo),
            body.replace("- [x] go\n", ""),
            "only the archived line is removed"
        );
    }

    #[test]
    fn a_stale_item_is_skipped_rather_than_clobbering() {
        let (dir, todo, archive, items) = workspace("## P0\n\n- [x] finished\n");
        // Another writer rewrites the line after it was parsed.
        std::fs::write(&todo, "## P0\n\n- [x] finished, amended\n").unwrap();

        let report = archive_done(&todo, &archive, &items, "2026-07-28").unwrap();
        assert_eq!(report.archived, 0);
        assert!(report.skipped[0].contains("changed on disk"));
        assert!(read(&todo).contains("amended"), "other writer survives");
        drop(dir);
    }

    #[test]
    fn crlf_files_keep_their_line_endings() {
        let (_d, todo, archive, _r) = run("## P0\r\n\r\n- [x] done\r\n- [ ] open\r\n");
        assert_eq!(read(&todo), "## P0\r\n\r\n- [ ] open\r\n");
        assert!(read(&archive.join("TODO.md")).contains("\r\n"));
    }
}
