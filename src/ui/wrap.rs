//! Word wrapping for the item list.
//!
//! ratatui's `Paragraph` can wrap, but it does the work inside the widget, so
//! the caller cannot tell how many rows a line became. The item list needs
//! that number to keep the cursor visible and to map a click back to an item,
//! so wrapping happens here instead.

/// Split `text` into chunks no wider than `width` display columns.
///
/// Breaks on whitespace where it can and mid-word where it must, so a long URL
/// still fits rather than overflowing the pane.
pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    if text.chars().count() <= width {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for word in text.split_whitespace() {
        let word_width = word.chars().count();

        // A word too long for any line is split across lines.
        if word_width > width {
            if current_width > 0 {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
            for chunk in chunks(word, width) {
                if chunk.chars().count() == width {
                    lines.push(chunk);
                } else {
                    current = chunk;
                    current_width = current.chars().count();
                }
            }
            continue;
        }

        let needed = if current_width == 0 {
            word_width
        } else {
            word_width + 1
        };
        if current_width + needed > width {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        if current_width > 0 {
            current.push(' ');
            current_width += 1;
        }
        current.push_str(word);
        current_width += word_width;
    }

    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// Hard-split a word into `width`-wide pieces.
fn chunks(word: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    chars
        .chunks(width)
        .map(|c| c.iter().collect::<String>())
        .collect()
}

/// Truncate to `width` columns, for when wrapping is off.
pub fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    text.chars().take(width).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_one_line() {
        assert_eq!(wrap_text("hello", 20), vec!["hello"]);
    }

    #[test]
    fn text_breaks_on_whitespace() {
        assert_eq!(
            wrap_text("the quick brown fox", 10),
            vec!["the quick", "brown fox"]
        );
    }

    #[test]
    fn no_line_exceeds_the_width() {
        let text = "Respond to opposing counsel regarding the outstanding discovery requests";
        for width in [8usize, 20, 33, 60] {
            for line in wrap_text(text, width) {
                assert!(
                    line.chars().count() <= width,
                    "width {width}: {line:?} is too wide"
                );
            }
        }
    }

    #[test]
    fn a_word_longer_than_the_line_is_split_rather_than_overflowing() {
        let lines = wrap_text("https://example.com/a/very/long/path/that/never/breaks", 12);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| l.chars().count() <= 12));
    }

    #[test]
    fn wrapping_preserves_every_word() {
        let text = "file the 83(b) election before the deadline";
        let joined = wrap_text(text, 11).join(" ");
        for word in text.split_whitespace() {
            assert!(joined.contains(word), "lost {word:?}");
        }
    }

    #[test]
    fn a_zero_width_yields_one_empty_line_rather_than_looping() {
        assert_eq!(wrap_text("anything", 0), vec![""]);
    }

    #[test]
    fn empty_text_is_still_one_line() {
        assert_eq!(wrap_text("", 10), vec![""]);
    }

    #[test]
    fn truncate_cuts_at_the_width() {
        assert_eq!(truncate("hello world", 5), "hello");
        assert_eq!(truncate("hi", 5), "hi");
    }

    #[test]
    fn wrapping_handles_multibyte_text() {
        let lines = wrap_text("café déjà vu naïve", 6);
        assert!(lines.iter().all(|l| l.chars().count() <= 6), "{lines:?}");
        assert!(lines.join(" ").contains("déjà"));
    }
}
