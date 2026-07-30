//! A small multi-line text editor, for editing notes in place.
//!
//! The description used to be typed into the one-line command field, which is
//! a poor fit for notes that run to several lines. This holds the buffer and
//! the caret so the detail pane can be edited where it is shown.

/// Text being edited, and where the caret sits within it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    lines: Vec<String>,
    /// Caret line, and column measured in characters.
    row: usize,
    col: usize,
}

impl Default for Editor {
    /// One empty line, never zero: every operation indexes the caret's line.
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            col: 0,
        }
    }
}

impl Editor {
    pub fn new(text: &str) -> Self {
        let lines: Vec<String> = if text.is_empty() {
            vec![String::new()]
        } else {
            text.lines().map(|l| l.to_string()).collect()
        };
        let row = lines.len().saturating_sub(1);
        let col = lines[row].chars().count();
        Self { lines, row, col }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map_or(0, |l| l.chars().count())
    }

    /// Byte offset of character `col` on the caret's line.
    fn byte_at(&self, row: usize, col: usize) -> usize {
        self.lines[row]
            .char_indices()
            .nth(col)
            .map_or(self.lines[row].len(), |(i, _)| i)
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte_at(self.row, self.col);
        self.lines[self.row].insert(at, c);
        self.col += 1;
    }

    pub fn newline(&mut self) {
        let at = self.byte_at(self.row, self.col);
        let rest = self.lines[self.row].split_off(at);
        self.lines.insert(self.row + 1, rest);
        self.row += 1;
        self.col = 0;
    }

    /// Delete backwards, joining onto the previous line at the margin.
    pub fn backspace(&mut self) {
        if self.col > 0 {
            let at = self.byte_at(self.row, self.col - 1);
            self.lines[self.row].remove(at);
            self.col -= 1;
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.line_len(self.row);
            self.lines[self.row].push_str(&line);
        }
    }

    /// Delete forwards, pulling the next line up at the end of a line.
    pub fn delete(&mut self) {
        if self.col < self.line_len(self.row) {
            let at = self.byte_at(self.row, self.col);
            self.lines[self.row].remove(at);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    pub fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.line_len(self.row);
        }
    }

    pub fn right(&mut self) {
        if self.col < self.line_len(self.row) {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    pub fn up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.col = self.col.min(self.line_len(self.row));
        }
    }

    pub fn down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = self.col.min(self.line_len(self.row));
        }
    }

    /// Put the caret at a row and column, clamped to the text.
    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.row = row.min(self.lines.len().saturating_sub(1));
        self.col = col.min(self.line_len(self.row));
    }

    pub fn home(&mut self) {
        self.col = 0;
    }

    pub fn end(&mut self) {
        self.col = self.line_len(self.row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_editor_starts_at_the_end_of_the_text() {
        let editor = Editor::new("hello");
        assert_eq!(editor.cursor(), (0, 5), "ready to keep typing");
        assert_eq!(editor.text(), "hello");
    }

    #[test]
    fn the_default_editor_is_usable_immediately() {
        // Regression: a default with no lines panicked on the first keystroke.
        let mut editor = Editor::default();
        editor.insert('a');
        assert_eq!(editor.text(), "a");
    }

    #[test]
    fn an_empty_editor_has_one_empty_line() {
        let editor = Editor::new("");
        assert_eq!(editor.lines(), &[String::new()]);
        assert_eq!(editor.cursor(), (0, 0));
    }

    #[test]
    fn existing_multi_line_notes_are_loaded() {
        let editor = Editor::new("one\ntwo\nthree");
        assert_eq!(editor.lines().len(), 3);
        assert_eq!(editor.cursor(), (2, 5), "at the end of the last line");
    }

    #[test]
    fn typing_inserts_at_the_caret() {
        let mut editor = Editor::new("ac");
        editor.left();
        editor.insert('b');
        assert_eq!(editor.text(), "abc");
        assert_eq!(editor.cursor(), (0, 2));
    }

    #[test]
    fn enter_splits_the_line() {
        let mut editor = Editor::new("abcd");
        editor.left();
        editor.left();
        editor.newline();
        assert_eq!(editor.text(), "ab\ncd");
        assert_eq!(editor.cursor(), (1, 0));
    }

    #[test]
    fn backspace_joins_onto_the_previous_line() {
        let mut editor = Editor::new("ab\ncd");
        editor.down();
        editor.home();
        editor.backspace();
        assert_eq!(editor.text(), "abcd");
        assert_eq!(editor.cursor(), (0, 2), "caret at the seam");
    }

    #[test]
    fn backspace_at_the_very_start_does_nothing() {
        let mut editor = Editor::new("abc");
        editor.home();
        editor.backspace();
        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn delete_pulls_the_next_line_up() {
        let mut editor = Editor::new("ab\ncd");
        editor.up();
        editor.end();
        editor.delete();
        assert_eq!(editor.text(), "abcd");
    }

    #[test]
    fn delete_at_the_very_end_does_nothing() {
        let mut editor = Editor::new("abc");
        editor.delete();
        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn the_caret_wraps_between_lines() {
        let mut editor = Editor::new("ab\ncd");
        editor.up();
        editor.end();
        editor.right();
        assert_eq!(editor.cursor(), (1, 0), "right at end of line goes down");
        editor.left();
        assert_eq!(editor.cursor(), (0, 2), "and left comes back");
    }

    #[test]
    fn moving_down_a_short_line_clamps_the_column() {
        let mut editor = Editor::new("longer line\nab");
        editor.up();
        editor.end();
        editor.down();
        assert_eq!(editor.cursor(), (1, 2), "clamped to the shorter line");
    }

    #[test]
    fn the_caret_can_be_placed_and_is_clamped() {
        let mut editor = Editor::new("hello\nab");
        editor.set_cursor(0, 3);
        assert_eq!(editor.cursor(), (0, 3));

        editor.set_cursor(1, 99);
        assert_eq!(editor.cursor(), (1, 2), "clamped to the line");

        editor.set_cursor(99, 0);
        assert_eq!(editor.cursor(), (1, 0), "clamped to the last line");
    }

    #[test]
    fn the_caret_stops_at_both_ends() {
        let mut editor = Editor::new("ab");
        editor.home();
        editor.left();
        assert_eq!(editor.cursor(), (0, 0));
        editor.end();
        editor.right();
        assert_eq!(editor.cursor(), (0, 2));
    }

    #[test]
    fn multibyte_text_is_edited_by_character_not_byte() {
        let mut editor = Editor::new("café");
        editor.backspace();
        assert_eq!(editor.text(), "caf");

        let mut editor = Editor::new("déjà vu");
        editor.home();
        editor.right();
        editor.insert('X');
        assert_eq!(editor.text(), "dXéjà vu");
    }

    #[test]
    fn typing_a_whole_note_round_trips() {
        let mut editor = Editor::new("");
        for c in "due Friday".chars() {
            editor.insert(c);
        }
        editor.newline();
        for c in "ask Sam".chars() {
            editor.insert(c);
        }
        assert_eq!(editor.text(), "due Friday\nask Sam");
    }
}
