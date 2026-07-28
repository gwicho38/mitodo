pub mod category_grid;
pub mod status_bar;
pub mod ticker;
pub mod ticker_queue;

use chrono::{DateTime, Utc};
use ratatui::prelude::*;

use crate::prelude::*;

use self::ticker::TickerState;

/// All mutable state for chyron mode, stored as a field on `App`.
pub struct ChyronState {
    pub ticker: TickerState,
    pub last_sync_time: Option<DateTime<Utc>>,
    pub categories: Vec<ticker_queue::CategoryInfo>,
    pub total_unread: i64,
    pub feed_count: usize,
}

impl ChyronState {
    pub fn new(default_speed: u8) -> Self {
        Self {
            ticker: TickerState::new(default_speed),
            last_sync_time: None,
            categories: Vec::new(),
            total_unread: 0,
            feed_count: 0,
        }
    }
}

impl App {
    /// Render the complete chyron layout: status bar, category grid, ticker, help bar.
    pub fn render_chyron(&mut self, area: Rect, buf: &mut Buffer) {
        // 4-zone vertical layout
        let [status_area, grid_area, ticker_area, help_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),  // status bar
                Constraint::Min(0),    // category grid (fills remaining)
                Constraint::Length(2), // ticker
                Constraint::Length(1), // help bar
            ])
            .areas(area);

        // Status bar
        status_bar::render_chyron_status_bar(
            status_area,
            buf,
            &self.config,
            self.chyron_state.feed_count,
            self.chyron_state.total_unread,
            self.news_flash_utils.is_async_operation_running(),
            self.is_offline,
            self.chyron_state.last_sync_time,
            &self.async_operation_throbber,
        );

        // Category grid
        category_grid::render_chyron_category_grid(
            grid_area,
            buf,
            &self.chyron_state.categories,
            &self.config,
        );

        // Ticker
        ticker::render_ticker(
            ticker_area,
            buf,
            &self.chyron_state.ticker,
            &self.config,
        );

        // Help bar
        let help_line = if self.chyron_state.ticker.paused {
            Line::from(Span::styled(
                " ▌▌ Paused │ ←/→ prev/next │ ↵ open │ p resume │ q quit",
                self.config.theme.statusbar(),
            ))
        } else {
            Line::from(Span::styled(
                format!(
                    " ▶ Playing (speed {}) │ +/- speed │ p pause │ s sync │ C reader │ q quit",
                    self.chyron_state.ticker.speed
                ),
                self.config.theme.statusbar(),
            ))
        };

        Block::default()
            .style(self.config.theme.statusbar())
            .render(help_area, buf);
        help_line.render(help_area, buf);
    }
}
