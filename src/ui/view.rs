use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{App, Focus, Mode, viewport_start};

/// The horizontal bands of the screen, top to bottom.
///
/// Mirrors eilmeldung's layout so later stages can drop the command line and
/// popups in without moving anything.
pub struct Frames {
    pub top_bar: Rect,
    pub groups: Rect,
    pub items: Rect,
    pub detail: Rect,
    pub command_line: Rect,
    pub status: Rect,
}

pub fn split(area: Rect, command_line_visible: bool) -> Frames {
    let [top_bar, middle, command_line, status] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(if command_line_visible { 1 } else { 0 }),
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
        command_line,
        status,
    }
}

/// Render the whole screen. Kept free of I/O so it can be exercised headlessly
/// against a `TestBackend`.
pub fn render(app: &App, frame: &mut Frame) {
    let editing = app.mode == Mode::EditingQuery;
    let f = split(frame.area(), editing);
    render_top_bar(app, frame, f.top_bar);
    render_groups(app, frame, f.groups);
    render_items(app, frame, f.items);
    render_detail(app, frame, f.detail);
    if editing {
        render_command_line(app, frame, f.command_line);
    }
    render_status(app, frame, f.status);
}

fn render_command_line(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("/", theme.command_input()),
            Span::styled(app.query_input.clone(), theme.command_input()),
            // A block cursor, since the terminal cursor stays hidden.
            Span::styled("\u{2588}", theme.command_input()),
        ]))
        .style(theme.command_input()),
        area,
    );
}

fn render_top_bar(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let visible = app.visible_items();
    let open = visible.iter().filter(|i| !i.done).count();
    let scope = match app.selected_group() {
        Some(g) => g.name.clone(),
        None => "all".to_string(),
    };

    let mut spans = vec![
        Span::styled("  mitodo  ", theme.statusbar()),
        Span::styled(
            format!(" {scope} · {open} open / {} shown ", visible.len()),
            theme.header(),
        ),
    ];
    if app.hide_done {
        spans.push(Span::styled(" !done ", theme.query()));
    }
    if !app.query_input.is_empty() && app.query.is_some() {
        spans.push(Span::styled(
            format!(" /{} ", app.query_input),
            theme.query(),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.statusbar()),
        area,
    );
}

fn render_groups(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let focused = app.focus == Focus::Groups;
    let height = area.height.saturating_sub(2) as usize;

    // Row 0 is the synthetic "all"; the rest are real groups.
    let mut rows: Vec<(String, usize)> = vec![(
        "all".to_string(),
        app.workspace.items.iter().filter(|i| !i.done).count(),
    )];
    rows.extend(app.workspace.groups.iter().map(|g| {
        let items = app.workspace.items_for_group(&g.name);
        (g.name.clone(), items.iter().filter(|i| !i.done).count())
    }));

    let start = viewport_start(app.group_cursor, rows.len(), height);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, (name, open))| {
            let selected = index == app.group_cursor;
            let base = if selected {
                theme.selected(&Style::default())
            } else {
                theme.feed()
            };
            Line::from(vec![
                Span::styled(if selected { "▸ " } else { "  " }, base),
                Span::styled(format!("{name:<11}"), base),
                Span::styled(format!("{open:>3}"), theme.unread_count()),
            ])
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.eff_border(focused))
                .title("groups"),
        ),
        area,
    );
}

fn render_items(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let focused = app.focus == Focus::Items;
    let height = area.height.saturating_sub(2) as usize;
    let items = app.visible_items();
    let start = viewport_start(app.item_cursor, items.len(), height);

    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, item)| {
            let selected = index == app.item_cursor;
            let mut style: Style = if item.done {
                theme.read(&Style::default())
            } else {
                theme.unread(&Style::default())
            };
            if selected {
                style = theme.selected(&style);
            }
            let mark = if item.done { "x" } else { " " };
            Line::from(Span::styled(
                format!(
                    "{}{}[{mark}] {:<3} {}",
                    if selected { "▸ " } else { "  " },
                    " ".repeat(item.indent),
                    item.priority.as_str(),
                    item.text
                ),
                style,
            ))
        })
        .collect();

    let title = if items.is_empty() {
        "items (none)".to_string()
    } else {
        format!("items {}/{}", app.item_cursor + 1, items.len())
    };

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.eff_border(focused))
                .title(title),
        ),
        area,
    );
}

fn render_detail(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let lines: Vec<Line> = match app.selected_item() {
        Some(item) => {
            let mut lines = vec![
                Line::from(Span::styled(item.text.clone(), theme.header())),
                Line::from(""),
                Line::from(Span::styled(
                    format!(
                        "{} · {} · {}",
                        item.priority.as_str(),
                        item.section,
                        item.heading
                    ),
                    theme.paragraph(),
                )),
            ];
            if !item.description.is_empty() {
                lines.push(Line::from(""));
                lines.extend(
                    item.description
                        .lines()
                        .map(|l| Line::from(Span::styled(l.to_string(), theme.paragraph()))),
                );
            }
            lines
        }
        None => vec![Line::from("")],
    };

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.eff_border(false))
                .title("detail"),
        ),
        area,
    );
}

fn render_status(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let line = match (&app.query_error, app.mode) {
        (Some(err), _) => Line::from(Span::styled(
            format!(" query error: {err} "),
            theme.tooltip_error(),
        )),
        (None, Mode::EditingQuery) => Line::from(Span::styled(
            " enter apply · esc cancel ".to_string(),
            theme.statusbar(),
        )),
        (None, Mode::Normal) => Line::from(Span::styled(
            " j/k move · tab focus · / query · esc clear · h hide done · q quit ".to_string(),
            theme.statusbar(),
        )),
    };
    frame.render_widget(Paragraph::new(line).style(theme.statusbar()), area);
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

    fn test_app() -> App {
        App::new(Workspace {
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
        })
    }

    /// Flatten the rendered buffer into one string per row.
    fn draw_app(app: &App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(app, frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn draw(width: u16, height: u16) -> Vec<String> {
        draw_app(&test_app(), width, height)
    }

    #[test]
    fn renders_the_group_name_and_the_all_row() {
        let rows = draw(80, 24).join("\n");
        assert!(rows.contains("all"), "synthetic all row present");
        assert!(rows.contains("lefv"), "group pane shows the group name");
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
    fn marks_the_selected_item_with_a_cursor() {
        let rows = draw(80, 24).join("\n");
        assert!(rows.contains("▸ [ ] P0  file the 83(b)"), "cursor on row 0");
    }

    #[test]
    fn top_bar_reports_scope_and_counts() {
        let rows = draw(80, 24);
        assert!(rows[0].contains("mitodo"), "top bar is branded");
        assert!(rows[0].contains("all"), "scope shown");
        assert!(rows[0].contains("1 open / 2 shown"), "counts in top bar");
    }

    #[test]
    fn top_bar_shows_the_hide_done_flag_when_active() {
        let mut app = test_app();
        app.hide_done = true;
        let rows = draw_app(&app, 80, 24);
        assert!(rows[0].contains("!done"), "hide-done indicator shown");
        assert!(rows[0].contains("1 open / 1 shown"), "done item excluded");
    }

    #[test]
    fn items_title_reports_cursor_position() {
        let rows = draw(80, 24).join("\n");
        assert!(rows.contains("items 1/2"), "position in the pane title");
    }

    #[test]
    fn detail_pane_shows_the_selected_item() {
        let rows = draw(80, 24).join("\n");
        assert!(rows.contains("file the 83(b)"), "detail shows item text");
        assert!(rows.contains("P0 — Critical"), "detail shows the section");
    }

    #[test]
    fn detail_pane_shows_the_description_when_present() {
        let mut app = test_app();
        app.workspace.items[0].description = "needs CPA sign-off".to_string();
        let rows = draw_app(&app, 80, 24).join("\n");
        assert!(rows.contains("needs CPA sign-off"));
    }

    #[test]
    fn status_bar_lists_the_keybindings() {
        let rows = draw(80, 24);
        let last = rows.last().unwrap();
        assert!(last.contains("q quit"));
        assert!(last.contains("tab focus"));
    }

    #[test]
    fn long_lists_scroll_to_keep_the_cursor_visible() {
        let mut app = test_app();
        app.workspace.items = (0..100)
            .map(|n| item(&format!("item-{n:03}"), false, 0))
            .collect();
        app.item_cursor = 99;

        let rows = draw_app(&app, 80, 24).join("\n");
        assert!(rows.contains("item-099"), "cursor row is on screen");
        assert!(!rows.contains("item-000"), "top of the list scrolled away");
    }

    #[test]
    fn an_empty_item_list_renders_without_panicking() {
        let mut app = test_app();
        app.workspace.items.clear();
        let rows = draw_app(&app, 80, 24).join("\n");
        assert!(rows.contains("items (none)"));
    }

    #[test]
    fn renders_without_panicking_in_a_tiny_terminal() {
        // Layout maths must survive a terminal smaller than the pane minimums.
        draw(20, 5);
        draw(4, 3);
    }
}
