use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use throbber_widgets_tui::ThrobberState;

use crate::prelude::*;

/// Render the single-line chyron status bar.
///
/// Content: `░ EILMELDUNG CHYRON ░  {feed_count} feeds │ {unread_count} unread │ {●/○} │ synced {timestamp}`
#[allow(clippy::too_many_arguments)]
pub fn render_chyron_status_bar(
    area: Rect,
    buf: &mut Buffer,
    config: &Config,
    feed_count: usize,
    unread_count: i64,
    is_syncing: bool,
    is_offline: bool,
    last_sync_time: Option<DateTime<Utc>>,
    _throbber_state: &ThrobberState,
) {
    // Fill background
    Block::default()
        .style(config.theme.statusbar())
        .render(area, buf);

    let connection_indicator = if is_offline { "○" } else { "●" };

    let sync_text = if is_syncing {
        " syncing...".to_string()
    } else {
        match last_sync_time {
            Some(time) => format!(" synced {}", time.format("%H:%M")),
            None => " not synced".to_string(),
        }
    };

    let status_text = format!(
        " ░ EILMELDUNG CHYRON ░  {} feeds │ {} unread │ {} │{}",
        feed_count, unread_count, connection_indicator, sync_text
    );

    let line = Line::from(Span::styled(status_text, config.theme.statusbar()));
    line.render(area, buf);
}
