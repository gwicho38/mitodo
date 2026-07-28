use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use chrono::Local;

use super::{App, Focus, Mode, chyron, viewport_start};

/// Short deadline marker, and whether it is already past.
fn due_label(item: &crate::store::model::Item) -> Option<(String, bool)> {
    let due = item.due?;
    let today = Local::now().date_naive();
    let overdue = due < today && !item.done;
    let label = match (due - today).num_days() {
        0 => "today".to_string(),
        1 => "tmrw".to_string(),
        d if d < 0 => format!("{}d ago", -d),
        d if d <= 99 => format!("{d}d"),
        _ => due.format("%m-%d").to_string(),
    };
    Some((label, overdue))
}

/// The horizontal bands of the screen, top to bottom.
///
/// Mirrors eilmeldung's layout so later stages can drop the command line and
/// popups in without moving anything.
#[derive(Debug, Clone, Copy, Default)]
pub struct Frames {
    pub top_bar: Rect,
    pub groups: Rect,
    pub items: Rect,
    pub detail: Rect,
    pub command_line: Rect,
    pub status: Rect,
}

/// `items_height` overrides the items/detail split, as set by dragging the
/// divider between them.
pub fn split_with(area: Rect, command_line_visible: bool, items_height: Option<u16>) -> Frames {
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
        .constraints(match items_height {
            Some(h) => [Constraint::Length(h), Constraint::Min(0)],
            None => [Constraint::Percentage(65), Constraint::Min(0)],
        })
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
///
/// Returns the layout it used, so mouse events can be hit-tested against the
/// panes actually on screen rather than a guess.
pub fn render(app: &App, frame: &mut Frame) -> Frames {
    let editing = matches!(
        app.mode,
        Mode::EditingQuery | Mode::Editing(_) | Mode::AskingAgent(_)
    );
    let f = split_with(frame.area(), editing, app.items_height);
    render_top_bar(app, frame, f.top_bar);
    render_groups(app, frame, f.groups);
    render_items(app, frame, f.items);
    render_detail(app, frame, f.detail);
    if editing {
        render_command_line(app, frame, f.command_line);
    }
    render_status(app, frame, f.status);
    render_overlay(app, frame, frame.area());
    f
}

/// Modal and change-set review share one centred overlay.
fn render_overlay(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let (title, body) = match (&app.modal, &app.pending, app.mode) {
        (_, Some(set), Mode::ReviewingChangeSet) => {
            ("review change-set".to_string(), set.review_lines())
        }
        (Some((title, body)), _, Mode::Modal) => (title.clone(), body.clone()),
        _ => return,
    };

    let width = area.width.saturating_sub(8).clamp(20, 100);
    let height = (body.len() as u16 + 2)
        .min(area.height.saturating_sub(4))
        .max(3);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(
            body.iter()
                .map(|l| Line::from(Span::styled(l.clone(), theme.paragraph())))
                .collect::<Vec<_>>(),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.eff_border(true))
                .title(title),
        ),
        popup,
    );
}

fn render_command_line(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.theme;
    let (prefix, text) = match app.mode {
        Mode::Editing(kind) => (format!("{}: ", kind.prompt()), app.edit_buffer.clone()),
        Mode::AskingAgent(verb) => (format!("{}: ", verb.label()), app.edit_buffer.clone()),
        _ => ("/".to_string(), app.query_input.clone()),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(prefix, theme.command_input()),
            Span::styled(text, theme.command_input()),
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
    if let Some(busy) = &app.busy {
        spans.push(Span::styled(format!(" ⣟ {busy}… "), theme.tooltip_info()));
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
            let mut spans = vec![Span::styled(
                format!(
                    "{}{}[{mark}] {:<3} ",
                    if selected { "▸ " } else { "  " },
                    " ".repeat(item.indent),
                    item.priority.as_str(),
                ),
                style,
            )];
            if let Some((label, overdue)) = due_label(item) {
                let due_style = if overdue {
                    theme.flagged(&style)
                } else {
                    theme.highlighted(&style)
                };
                spans.push(Span::styled(format!("{label:<7} "), due_style));
            }
            spans.push(Span::styled(item.text.clone(), style));
            Line::from(spans)
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
                    match item.due {
                        Some(due) => format!(
                            "{} · due {} · {} · {}",
                            item.priority.as_str(),
                            due.format("%Y-%m-%d"),
                            item.section,
                            item.heading
                        ),
                        None => format!(
                            "{} · {} · {}",
                            item.priority.as_str(),
                            item.section,
                            item.heading
                        ),
                    },
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
    // The ticker takes over the status row when enabled, as in eilmeldung.
    if let Some(ticker) = &app.ticker
        && app.mode == Mode::Normal
        && app.notice.is_none()
        && app.query_error.is_none()
    {
        chyron::render_ticker(frame, area, ticker, theme);
        return;
    }
    let line = match (&app.query_error, &app.notice, app.mode) {
        (Some(err), _, _) => Line::from(Span::styled(
            format!(" query error: {err} "),
            theme.tooltip_error(),
        )),
        (None, Some(notice), _) => {
            Line::from(Span::styled(format!(" {notice} "), theme.tooltip_warning()))
        }
        (None, None, Mode::EditingQuery | Mode::Editing(_)) => Line::from(Span::styled(
            " enter apply · esc cancel ".to_string(),
            theme.statusbar(),
        )),
        (None, None, Mode::ConfirmingDelete) => Line::from(Span::styled(
            " delete this item? y / n ".to_string(),
            theme.tooltip_warning(),
        )),
        (None, None, Mode::ConfirmingArchive) => Line::from(Span::styled(
            " move finished items into _archive/? y / n ".to_string(),
            theme.tooltip_warning(),
        )),
        (None, None, Mode::ReviewingChangeSet) => Line::from(Span::styled(
            " apply this change-set? y / n ".to_string(),
            theme.tooltip_warning(),
        )),
        (None, None, Mode::Modal) => Line::from(Span::styled(
            " any key to dismiss ".to_string(),
            theme.statusbar(),
        )),
        (None, None, Mode::AskingAgent(_)) => Line::from(Span::styled(
            " enter run · esc cancel ".to_string(),
            theme.statusbar(),
        )),
        (None, None, Mode::Normal) => Line::from(Span::styled(
            " space toggle · a add · e edit · d del · / query · s sync · ? keys · q quit "
                .to_string(),
            theme.statusbar(),
        )),
    };
    frame.render_widget(Paragraph::new(line).style(theme.statusbar()), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
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
            raw: format!("- [{}] {}", if done { "x" } else { " " }, text),
            description: String::new(),
            section: "P0 — Critical".to_string(),
            heading: "H".to_string(),
            priority: Priority::P0,
            due: None,
            parent: None,
            children: Vec::new(),
        }
    }

    fn test_app() -> App {
        App::new(
            Workspace {
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
            },
            Config::default(),
        )
    }

    /// Flatten the rendered buffer into one string per row.
    fn draw_app(app: &App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render(app, frame);
            })
            .unwrap();
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

    fn with_due(app: &mut App, index: usize, offset_days: i64) {
        app.workspace.items[index].due =
            Some(Local::now().date_naive() + chrono::Duration::days(offset_days));
    }

    #[test]
    fn a_deadline_is_shown_beside_the_item() {
        let mut app = test_app();
        with_due(&mut app, 0, 0);
        assert!(draw_app(&app, 90, 24).join("\n").contains("today"));

        let mut app = test_app();
        with_due(&mut app, 0, 1);
        assert!(draw_app(&app, 90, 24).join("\n").contains("tmrw"));

        let mut app = test_app();
        with_due(&mut app, 0, 5);
        assert!(draw_app(&app, 90, 24).join("\n").contains("5d"));
    }

    #[test]
    fn a_missed_deadline_reads_as_overdue() {
        let mut app = test_app();
        with_due(&mut app, 0, -3);
        assert!(
            draw_app(&app, 90, 24).join("\n").contains("3d ago"),
            "how late it is, not just that it is late"
        );
    }

    #[test]
    fn items_without_a_deadline_get_no_column() {
        let app = test_app();
        let rows = draw_app(&app, 90, 24).join("\n");
        assert!(
            rows.contains("[ ] P0  file the 83(b)"),
            "text follows priority"
        );
    }

    #[test]
    fn the_detail_pane_spells_the_deadline_out_in_full() {
        let mut app = test_app();
        with_due(&mut app, 0, 0);
        let expected = Local::now().date_naive().format("%Y-%m-%d").to_string();
        assert!(
            draw_app(&app, 90, 24)
                .join("\n")
                .contains(&format!("due {expected}"))
        );
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
        assert!(last.contains("space toggle"));
        assert!(last.contains("/ query"));
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
