use std::path::Path;

#[derive(thiserror::Error, Debug)]
pub enum WriteError {
    #[error("file could not be read or written: {0}")]
    Io(#[from] std::io::Error),
    #[error("line {line} of {file} changed on disk (expected {expected:?}, found {found:?})")]
    Conflict {
        file: String,
        line: usize,
        expected: String,
        found: String,
    },
    #[error("line {line} is past the end of {file} ({len} lines)")]
    LineOutOfRange {
        file: String,
        line: usize,
        len: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }
}

/// Read a file into lines, remembering how to put it back together verbatim.
///
/// Returns the lines without their terminators, the dominant line ending, and
/// whether the file ended with a terminator.
pub(super) fn read_lines(path: &Path) -> Result<(Vec<String>, LineEnding, bool), WriteError> {
    let text = std::fs::read_to_string(path)?;
    let ending = if text.contains("\r\n") {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    };
    let trailing = text.ends_with('\n');
    let body = if trailing {
        text.strip_suffix('\n').unwrap_or(&text)
    } else {
        &text
    };
    let body = body.strip_suffix('\r').unwrap_or(body);
    let lines: Vec<String> = if body.is_empty() && trailing {
        vec![String::new()]
    } else {
        body.split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
            .collect()
    };
    Ok((lines, ending, trailing))
}

/// Write lines back atomically: temp file in the same directory, then rename.
pub(super) fn write_lines(
    path: &Path,
    lines: &[String],
    ending: LineEnding,
    trailing: bool,
) -> Result<(), WriteError> {
    let mut out = lines.join(ending.as_str());
    if trailing {
        out.push_str(ending.as_str());
    }

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".{}.mitodo.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Fetch a line, verifying it still holds what the caller parsed.
///
/// This is the guard that makes concurrent editing by other tools safe.
pub(super) fn verify(
    path: &Path,
    lines: &[String],
    line: usize,
    expected: &str,
) -> Result<(), WriteError> {
    let found = lines.get(line).ok_or_else(|| WriteError::LineOutOfRange {
        file: path.display().to_string(),
        line,
        len: lines.len(),
    })?;
    if found != expected {
        return Err(WriteError::Conflict {
            file: path.display().to_string(),
            line,
            expected: expected.to_string(),
            found: found.clone(),
        });
    }
    Ok(())
}

/// Set the checkbox on `line` to `done`, leaving every other byte alone.
pub fn toggle(path: &Path, line: usize, expected: &str, done: bool) -> Result<(), WriteError> {
    let (mut lines, ending, trailing) = read_lines(path)?;
    verify(path, &lines, line, expected)?;

    let current = &lines[line];
    let replaced = if done {
        current.replacen("- [ ]", "- [x]", 1)
    } else {
        let once = current.replacen("- [x]", "- [ ]", 1);
        if once == *current {
            current.replacen("- [X]", "- [ ]", 1)
        } else {
            once
        }
    };

    if replaced == *current {
        return Ok(());
    }
    lines[line] = replaced;
    write_lines(path, &lines, ending, trailing)
}

/// True if `line` is a description blockquote line, e.g. `"  > note"`.
fn is_description(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('>') && line.len() > trimmed.len()
}

/// Index one past the last description line belonging to the item on `line`.
fn description_end(lines: &[String], line: usize) -> usize {
    let mut end = line + 1;
    while end < lines.len() && is_description(&lines[end]) {
        end += 1;
    }
    end
}

/// Leading whitespace of a line, as a string slice.
fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Replace the text of a checkbox line, preserving indentation and done state.
pub fn edit_text(
    path: &Path,
    line: usize,
    expected: &str,
    new_text: &str,
) -> Result<(), WriteError> {
    let (mut lines, ending, trailing) = read_lines(path)?;
    verify(path, &lines, line, expected)?;

    // Everything up to and including the "] " marker — indentation, bullet and
    // done state — is preserved verbatim; only the text after it changes.
    let marker_end = match lines[line].find("] ") {
        Some(idx) => idx + 2,
        None => return Ok(()),
    };
    let mut replaced = lines[line][..marker_end].to_string();
    replaced.push_str(new_text);
    lines[line] = replaced;

    write_lines(path, &lines, ending, trailing)
}

/// Insert a new unchecked item after the item on `after_line`, below any
/// description block that item owns.
pub fn add_item(
    path: &Path,
    after_line: usize,
    expected_after: &str,
    indent: usize,
    text: &str,
) -> Result<(), WriteError> {
    let (mut lines, ending, trailing) = read_lines(path)?;
    verify(path, &lines, after_line, expected_after)?;

    let insert_at = description_end(&lines, after_line);
    lines.insert(insert_at, format!("{}- [ ] {}", " ".repeat(indent), text));
    write_lines(path, &lines, ending, trailing)
}

/// Remove a checkbox line and any description block beneath it.
pub fn delete_item(path: &Path, line: usize, expected: &str) -> Result<(), WriteError> {
    let (mut lines, ending, trailing) = read_lines(path)?;
    verify(path, &lines, line, expected)?;

    let end = description_end(&lines, line);
    lines.drain(line..end);
    write_lines(path, &lines, ending, trailing)
}

/// Replace the description block beneath an item. An empty or blank
/// description removes the block entirely.
pub fn set_description(
    path: &Path,
    line: usize,
    expected: &str,
    description: &str,
) -> Result<(), WriteError> {
    let (mut lines, ending, trailing) = read_lines(path)?;
    verify(path, &lines, line, expected)?;

    let end = description_end(&lines, line);
    lines.drain(line + 1..end);

    let description = description.trim();
    if !description.is_empty() {
        let prefix = format!("{}  > ", leading_whitespace(&lines[line]));
        for (offset, note) in description.lines().enumerate() {
            lines.insert(line + 1 + offset, format!("{prefix}{note}"));
        }
    }

    write_lines(path, &lines, ending, trailing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_temp(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TODO.md");
        fs::write(&path, contents).unwrap();
        (dir, path)
    }

    const DOC: &str = "## P0\n\n- [ ] first\n- [x] second\n";
    const NESTED: &str = "## P0\n\n- [ ] parent\n  > note one\n  > note two\n- [x] sibling\n";

    #[test]
    fn marks_an_item_done() {
        let (_d, path) = write_temp(DOC);
        toggle(&path, 2, "- [ ] first", true).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [x] first\n- [x] second\n"
        );
    }

    #[test]
    fn marks_an_item_not_done() {
        let (_d, path) = write_temp(DOC);
        toggle(&path, 3, "- [x] second", false).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n- [ ] second\n"
        );
    }

    #[test]
    fn leaves_every_other_byte_untouched() {
        let messy = "---\ntags: [a]\n---\n\n#  Title   \n\n\n- [ ] first\t\n\n- [x] second  \n";
        let (_d, path) = write_temp(messy);
        toggle(&path, 7, "- [ ] first\t", true).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        let expected = messy.replace("- [ ] first\t", "- [x] first\t");
        assert_eq!(after, expected, "only the toggled line may change");
    }

    #[test]
    fn preserves_a_missing_trailing_newline() {
        let (_d, path) = write_temp("- [ ] only");
        toggle(&path, 0, "- [ ] only", true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "- [x] only");
    }

    #[test]
    fn preserves_crlf_line_endings() {
        let (_d, path) = write_temp("## P0\r\n\r\n- [ ] first\r\n");
        toggle(&path, 2, "- [ ] first", true).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\r\n\r\n- [x] first\r\n"
        );
    }

    #[test]
    fn a_changed_line_is_a_conflict() {
        let (_d, path) = write_temp(DOC);
        // Another writer edited the line since we parsed it.
        fs::write(&path, "## P0\n\n- [ ] first, amended\n- [x] second\n").unwrap();
        let err = toggle(&path, 2, "- [ ] first", true).unwrap_err();
        assert!(matches!(err, WriteError::Conflict { .. }));
    }

    #[test]
    fn a_conflict_leaves_the_file_untouched() {
        let (_d, path) = write_temp(DOC);
        let amended = "## P0\n\n- [ ] first, amended\n- [x] second\n";
        fs::write(&path, amended).unwrap();
        let _ = toggle(&path, 2, "- [ ] first", true);
        assert_eq!(fs::read_to_string(&path).unwrap(), amended);
    }

    #[test]
    fn a_line_past_the_end_is_out_of_range() {
        let (_d, path) = write_temp(DOC);
        let err = toggle(&path, 99, "- [ ] first", true).unwrap_err();
        assert!(matches!(err, WriteError::LineOutOfRange { .. }));
    }

    #[test]
    fn toggling_an_already_done_item_is_a_no_op() {
        let (_d, path) = write_temp(DOC);
        toggle(&path, 3, "- [x] second", true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), DOC);
    }

    #[test]
    fn preserves_indentation_of_nested_items() {
        let (_d, path) = write_temp("- [ ] parent\n  - [ ] child\n");
        toggle(&path, 1, "  - [ ] child", true).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "- [ ] parent\n  - [x] child\n"
        );
    }

    #[test]
    fn edits_item_text_and_keeps_its_state() {
        let (_d, path) = write_temp(DOC);
        edit_text(&path, 3, "- [x] second", "second, revised").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n- [x] second, revised\n"
        );
    }

    #[test]
    fn edits_preserve_indentation() {
        let (_d, path) = write_temp("- [ ] parent\n  - [ ] child\n");
        edit_text(&path, 1, "  - [ ] child", "renamed").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "- [ ] parent\n  - [ ] renamed\n"
        );
    }

    #[test]
    fn editing_a_drifted_line_is_a_conflict() {
        let (_d, path) = write_temp(DOC);
        let err = edit_text(&path, 2, "- [ ] not what is there", "x").unwrap_err();
        assert!(matches!(err, WriteError::Conflict { .. }));
    }

    #[test]
    fn adds_an_item_below_the_target() {
        let (_d, path) = write_temp(DOC);
        add_item(&path, 2, "- [ ] first", 0, "inserted").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n- [ ] inserted\n- [x] second\n"
        );
    }

    #[test]
    fn adds_below_an_existing_description_block() {
        let (_d, path) = write_temp(NESTED);
        add_item(&path, 2, "- [ ] parent", 0, "inserted").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] parent\n  > note one\n  > note two\n- [ ] inserted\n- [x] sibling\n"
        );
    }

    #[test]
    fn adds_a_nested_child_with_indentation() {
        let (_d, path) = write_temp(DOC);
        add_item(&path, 2, "- [ ] first", 2, "child").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n  - [ ] child\n- [x] second\n"
        );
    }

    #[test]
    fn deletes_an_item() {
        let (_d, path) = write_temp(DOC);
        delete_item(&path, 2, "- [ ] first").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [x] second\n"
        );
    }

    #[test]
    fn deleting_takes_the_description_block_with_it() {
        let (_d, path) = write_temp(NESTED);
        delete_item(&path, 2, "- [ ] parent").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [x] sibling\n"
        );
    }

    #[test]
    fn sets_a_description_where_there_was_none() {
        let (_d, path) = write_temp(DOC);
        set_description(&path, 2, "- [ ] first", "a note").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n  > a note\n- [x] second\n"
        );
    }

    #[test]
    fn replaces_an_existing_description() {
        let (_d, path) = write_temp(NESTED);
        set_description(&path, 2, "- [ ] parent", "only note").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] parent\n  > only note\n- [x] sibling\n"
        );
    }

    #[test]
    fn an_empty_description_removes_the_block() {
        let (_d, path) = write_temp(NESTED);
        set_description(&path, 2, "- [ ] parent", "").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] parent\n- [x] sibling\n"
        );
    }

    #[test]
    fn a_multi_line_description_becomes_multiple_blockquote_lines() {
        let (_d, path) = write_temp(DOC);
        set_description(&path, 2, "- [ ] first", "one\ntwo").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "## P0\n\n- [ ] first\n  > one\n  > two\n- [x] second\n"
        );
    }

    #[test]
    fn descriptions_indent_relative_to_their_item() {
        let (_d, path) = write_temp("- [ ] parent\n  - [ ] child\n");
        set_description(&path, 1, "  - [ ] child", "note").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "- [ ] parent\n  - [ ] child\n    > note\n"
        );
    }
}
