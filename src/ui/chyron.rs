//! Bloomberg-style scrolling ticker of urgent items.
//!
//! Ported in spirit from eilmeldung's chyron: the scroll state machine is the
//! same, but a headline is a todo item rather than an article, so `TickerItem`
//! drops `ArticleID`/`feed_name` and gains the group and priority.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::config::Theme;
use crate::store::model::{Item, Priority};

/// A single headline in the ticker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickerItem {
    pub group: String,
    pub priority: Priority,
    pub text: String,
    pub overdue: bool,
}

impl TickerItem {
    fn render(&self) -> String {
        let flag = if self.overdue { "OVERDUE " } else { "" };
        format!(
            "{flag}[{}] {} · {}",
            self.priority.as_str(),
            self.group,
            self.text
        )
    }
}

/// Scroll state for the ticker.
#[derive(Debug, Clone)]
pub struct TickerState {
    pub items: Vec<TickerItem>,
    pub offset: usize,
    pub speed: u8,
    pub paused: bool,
}

/// Separator between headlines, matching the newswire look.
const SEPARATOR: &str = "   ◆   ";

impl TickerState {
    pub fn new(speed: u8) -> Self {
        Self {
            items: Vec::new(),
            offset: 0,
            speed: speed.clamp(1, 10),
            paused: false,
        }
    }

    /// Build the queue from a workspace view: open items, most urgent first.
    pub fn fill<'a>(&mut self, items: impl Iterator<Item = (&'a Item, Option<&'a str>)>) {
        let today = chrono::Local::now().date_naive();
        let mut queue: Vec<TickerItem> = items
            .filter(|(item, _)| !item.done)
            .map(|(item, group)| TickerItem {
                group: group.unwrap_or("—").to_string(),
                priority: item.priority,
                text: item.text.clone(),
                overdue: item.due.is_some_and(|d| d < today),
            })
            .collect();
        // Overdue work leads, then priority. `Priority` orders P0 first and
        // `None` last, which is the order the ticker should cycle in.
        queue.sort_by_key(|t| (!t.overdue, t.priority));
        self.items = queue;
        self.offset = 0;
    }

    /// The full marquee string, repeated headlines separated by a diamond.
    pub fn banner(&self) -> String {
        if self.items.is_empty() {
            return "no open items".to_string();
        }
        self.items
            .iter()
            .map(|i| i.render())
            .collect::<Vec<_>>()
            .join(SEPARATOR)
    }

    /// Advance the scroll, wrapping at the end of the banner.
    pub fn advance(&mut self) {
        if self.paused || self.items.is_empty() {
            return;
        }
        let width = self.banner().chars().count() + SEPARATOR.chars().count();
        if width == 0 {
            return;
        }
        self.offset = (self.offset + self.speed as usize) % width;
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    pub fn speed_up(&mut self) {
        self.speed = (self.speed + 1).min(10);
    }

    pub fn speed_down(&mut self) {
        self.speed = self.speed.saturating_sub(1).max(1);
    }

    /// The visible window of the banner, `width` characters wide.
    ///
    /// Cycles the banner rather than slicing a doubled copy, so the tail wraps
    /// seamlessly into the head and the window is filled at any width — a
    /// terminal wider than the banner included.
    pub fn window(&self, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        let banner = format!("{}{SEPARATOR}", self.banner());
        let start = self.offset % banner.chars().count().max(1);
        banner.chars().cycle().skip(start).take(width).collect()
    }
}

pub fn render_ticker(frame: &mut Frame, area: Rect, state: &TickerState, theme: &Theme) {
    let text = state.window(area.width as usize);
    let style = if state.paused {
        theme.inactive()
    } else {
        theme.statusbar()
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, style))).style(style),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::ItemId;
    use std::path::PathBuf;

    fn item(text: &str, done: bool, priority: Priority) -> Item {
        Item {
            id: ItemId::compute("f", "s", "h", 0, text, 0),
            file: PathBuf::from("f"),
            line: 0,
            indent: 0,
            done,
            text: text.to_string(),
            raw: format!("- [{}] {text}", if done { "x" } else { " " }),
            description: String::new(),
            section: "s".to_string(),
            heading: "h".to_string(),
            priority,
            due: None,
            parent: None,
            children: Vec::new(),
        }
    }

    fn filled() -> TickerState {
        let items = [
            item("low thing", false, Priority::P3),
            item("urgent thing", false, Priority::P0),
            item("finished thing", true, Priority::P0),
        ];
        let mut state = TickerState::new(2);
        let refs: Vec<(&Item, Option<&str>)> = items.iter().map(|i| (i, Some("lefv"))).collect();
        state.fill(refs.into_iter());
        state
    }

    #[test]
    fn done_items_are_excluded() {
        let state = filled();
        assert_eq!(state.items.len(), 2);
        assert!(!state.banner().contains("finished thing"));
    }

    #[test]
    fn overdue_work_leads_the_ticker() {
        let mut late = item("late thing", false, Priority::P3);
        late.due = Some(chrono::Local::now().date_naive() - chrono::Duration::days(1));
        let items = [item("urgent but on time", false, Priority::P0), late];

        let mut state = TickerState::new(1);
        let refs: Vec<(&Item, Option<&str>)> = items.iter().map(|i| (i, Some("g"))).collect();
        state.fill(refs.into_iter());

        assert!(
            state.items[0].overdue,
            "a missed deadline outranks priority"
        );
        assert!(state.banner().starts_with("OVERDUE"));
    }

    #[test]
    fn most_urgent_scrolls_first() {
        let state = filled();
        assert_eq!(state.items[0].text, "urgent thing");
        assert!(state.banner().starts_with("[P0] lefv · urgent thing"));
    }

    #[test]
    fn an_empty_queue_says_so() {
        let state = TickerState::new(1);
        assert_eq!(state.banner(), "no open items");
        assert!(state.window(20).contains("no open items"));
    }

    #[test]
    fn advance_moves_by_the_speed() {
        let mut state = filled();
        state.advance();
        assert_eq!(state.offset, 2);
        state.advance();
        assert_eq!(state.offset, 4);
    }

    #[test]
    fn pausing_stops_the_scroll() {
        let mut state = filled();
        state.toggle_pause();
        state.advance();
        assert_eq!(state.offset, 0, "paused ticker does not move");
        state.toggle_pause();
        state.advance();
        assert_eq!(state.offset, 2);
    }

    #[test]
    fn the_offset_wraps_rather_than_growing_without_bound() {
        let mut state = filled();
        let width = state.banner().chars().count() + SEPARATOR.chars().count();
        for _ in 0..(width * 2) {
            state.advance();
        }
        assert!(state.offset < width, "offset stays inside one cycle");
    }

    #[test]
    fn the_window_is_exactly_the_requested_width() {
        let state = filled();
        for width in [1usize, 10, 40, 200] {
            assert_eq!(
                state.window(width).chars().count(),
                width,
                "width {width} is filled by wrapping"
            );
        }
    }

    #[test]
    fn a_zero_width_window_is_empty() {
        assert_eq!(filled().window(0), "");
    }

    #[test]
    fn the_window_wraps_seamlessly_at_the_end_of_a_cycle() {
        let mut state = filled();
        let banner_len = state.banner().chars().count() + SEPARATOR.chars().count();
        // Park the offset one character before the wrap point.
        state.offset = banner_len - 1;
        let window = state.window(30);
        assert_eq!(window.chars().count(), 30, "no gap at the wrap");
    }

    #[test]
    fn speed_is_clamped_at_both_ends() {
        let mut state = TickerState::new(200);
        assert_eq!(state.speed, 10, "constructor clamps");
        state.speed_up();
        assert_eq!(state.speed, 10);
        for _ in 0..20 {
            state.speed_down();
        }
        assert_eq!(state.speed, 1, "never reaches zero, which would freeze it");
    }

    #[test]
    fn items_without_a_group_render_a_placeholder() {
        let items = [item("orphan", false, Priority::P1)];
        let mut state = TickerState::new(1);
        let refs: Vec<(&Item, Option<&str>)> = items.iter().map(|i| (i, None)).collect();
        state.fill(refs.into_iter());
        assert!(state.banner().contains("—"));
    }
}
