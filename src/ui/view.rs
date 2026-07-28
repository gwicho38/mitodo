use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::App;

/// The four horizontal bands of the screen, top to bottom.
///
/// Mirrors eilmeldung's layout so later stages can drop the command line and
/// popups in without moving anything: top bar, main area, command line, status.
pub struct Frames {
    pub top_bar: Rect,
    pub groups: Rect,
    pub items: Rect,
    pub detail: Rect,
    pub status: Rect,
}

pub fn split(area: Rect) -> Frames {
    let [top_bar, middle, status] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(area);

    let [groups, right] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(0)])
        .areas(middle);

    let [items, detail] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Min(0)])
        .areas(right);

    Frames {
        top_bar,
        groups,
        items,
        detail,
        status,
    }
}

/// Render the whole screen. Kept free of I/O so it can be exercised headlessly
/// against a `TestBackend`.
pub fn render(app: &App, frame: &mut Frame) {
    let theme = &app.theme;
    let f = split(frame.area());

    // Top bar: workspace, open count, active query.
    let open = app.workspace.open_count();
    let total = app.workspace.items.len();
    let top = Line::from(vec![
        Span::styled("  mitodo  ", theme.statusbar()),
        Span::styled(
            format!(
                " {} · {open} open / {total} total ",
                app.workspace.root.display()
            ),
            theme.header(),
        ),
    ]);
    frame.render_widget(Paragraph::new(top).style(theme.statusbar()), f.top_bar);

    // Groups pane.
    let group_lines: Vec<Line> = app
        .workspace
        .groups
        .iter()
        .map(|g| {
            let items = app.workspace.items_for_group(&g.name);
            let open = items.iter().filter(|i| !i.done).count();
            Line::from(vec![
                Span::styled(format!(" {:<12}", g.name), theme.feed()),
                Span::styled(format!("{open:>3}"), theme.unread_count()),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(group_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.eff_border(true))
                .title("groups"),
        ),
        f.groups,
    );

    // Items pane — every item, flattened with indentation. Stage B adds
    // selection, scrolling and folding.
    let item_lines: Vec<Line> = app
        .workspace
        .items
        .iter()
        .take(f.items.height.saturating_sub(2) as usize)
        .map(|item| {
            let mark = if item.done { "x" } else { " " };
            let style: Style = if item.done {
                theme.read(&Style::default())
            } else {
                theme.unread(&Style::default())
            };
            Line::from(Span::styled(
                format!(
                    "{}[{mark}] {:<3} {}",
                    " ".repeat(item.indent),
                    item.priority.as_str(),
                    item.text
                ),
                style,
            ))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(item_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.eff_border(false))
                .title("items"),
        ),
        f.items,
    );

    // Detail pane — Stage D makes this the editable description view.
    frame.render_widget(
        Paragraph::new("").block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.eff_border(false))
                .title("detail"),
        ),
        f.detail,
    );

    // Status bar.
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {} groups · q to quit ", app.workspace.groups.len()),
            theme.statusbar(),
        )))
        .style(theme.statusbar()),
        f.status,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Workspace;
    use crate::store::model::{Group, Item, ItemId, Priority};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn item(text: &str, done: bool, indent: usize) -> Item {
        Item {
            id: ItemId::compute("lefv/TODO.md", "P0", "H", indent, text),
            file: PathBuf::from("/w/lefv/TODO.md"),
            line: 0,
            indent,
            done,
            text: text.to_string(),
            description: String::new(),
            section: "P0 — Critical".to_string(),
            heading: "H".to_string(),
            priority: Priority::P0,
            parent: None,
            children: Vec::new(),
        }
    }

    fn app() -> App {
        let workspace = Workspace {
            root: PathBuf::from("/w"),
            groups: vec![Group {
                name: "lefv".to_string(),
                todo_file: PathBuf::from("/w/lefv/TODO.md"),
                notes_file: None,
                archive_dir: None,
            }],
            items: vec![
                item("file the 83(b)", false, 0),
                item("done thing", true, 0),
            ],
        };
        App::new(workspace)
    }

    /// Flatten the rendered buffer into one string per row.
    fn draw(width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let app = app();
        terminal.draw(|frame| render(&app, frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn renders_the_group_name_and_open_count() {
        let rows = draw(80, 24).join("\n");
        assert!(rows.contains("lefv"), "group pane shows the group name");
        assert!(rows.contains("groups"), "group pane is titled");
    }

    #[test]
    fn renders_item_text_with_checkbox_state() {
        let rows = draw(80, 24).join("\n");
        assert!(
            rows.contains("[ ] P0  file the 83(b)"),
            "open item rendered"
        );
        assert!(rows.contains("[x] P0  done thing"), "done item rendered");
    }

    #[test]
    fn top_bar_reports_open_and_total_counts() {
        let rows = draw(80, 24);
        assert!(rows[0].contains("mitodo"), "top bar is branded");
        assert!(rows[0].contains("1 open / 2 total"), "counts in top bar");
    }

    #[test]
    fn status_bar_reports_group_count_and_quit_hint() {
        let rows = draw(80, 24);
        let last = rows.last().unwrap();
        assert!(last.contains("1 groups"), "status shows group count");
        assert!(last.contains("q to quit"), "status shows the quit hint");
    }

    #[test]
    fn all_four_panes_are_present() {
        let rows = draw(80, 24).join("\n");
        for title in ["groups", "items", "detail"] {
            assert!(rows.contains(title), "{title} pane is rendered");
        }
    }

    #[test]
    fn renders_without_panicking_in_a_tiny_terminal() {
        // Layout maths must survive a terminal smaller than the pane minimums.
        draw(20, 5);
    }
}
