use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::prelude::*;
use super::ticker_queue::CategoryInfo;

/// Render a responsive grid of category summary cells.
///
/// Column count adapts to terminal width: 4 columns at 120+, 3 at 80+, 2 at narrow.
/// Categories are pre-sorted by unread count descending.
pub fn render_chyron_category_grid(
    area: Rect,
    buf: &mut Buffer,
    categories: &[CategoryInfo],
    _config: &Config,
) {
    if categories.is_empty() {
        let msg = Paragraph::new("No categories found. Press s to sync.")
            .alignment(Alignment::Center);
        msg.render(area, buf);
        return;
    }

    let col_count = if area.width >= 120 {
        4
    } else if area.width >= 80 {
        3
    } else {
        2
    };

    let row_count = categories.len().div_ceil(col_count);
    let cell_width = area.width / col_count as u16;
    let cell_height = if row_count > 0 {
        (area.height / row_count as u16).max(3)
    } else {
        3
    };

    for (idx, cat) in categories.iter().enumerate() {
        let col = idx % col_count;
        let row = idx / col_count;

        let x = area.x + (col as u16) * cell_width;
        let y = area.y + (row as u16) * cell_height;

        // Skip if we'd render off-screen
        if y + cell_height > area.y + area.height {
            break;
        }

        let cell_area = Rect::new(
            x,
            y,
            cell_width.min(area.x + area.width - x),
            cell_height.min(area.y + area.height - y),
        );

        render_category_cell(cell_area, buf, cat);
    }
}

fn render_category_cell(
    area: Rect,
    buf: &mut Buffer,
    category: &CategoryInfo,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(category.color))
        .title(Span::styled(
            truncate_str(&category.name, area.width.saturating_sub(2) as usize),
            Style::default().fg(category.color).bold(),
        ));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Line 1: unread count
    let count_text = format!("{} unread", category.unread_count);
    let count_line = Line::from(Span::styled(
        count_text,
        Style::default().fg(Color::White),
    ));
    if inner.height >= 1 {
        count_line.render(Rect::new(inner.x, inner.y, inner.width, 1), buf);
    }

    // Line 2: most recent headline (truncated)
    if inner.height >= 2 && let Some(headline) = &category.latest_headline {
        let truncated = truncate_str(headline, inner.width as usize);
        let headline_line = Line::from(Span::styled(
            truncated,
            Style::default().fg(Color::DarkGray),
        ));
        headline_line.render(Rect::new(inner.x, inner.y + 1, inner.width, 1), buf);
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else if max_len > 1 {
        let truncated: String = s.chars().take(max_len - 1).collect();
        format!("{truncated}…")
    } else {
        String::new()
    }
}
