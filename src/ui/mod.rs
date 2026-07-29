pub mod chyron;
mod view;
pub mod wrap;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::agent::{self, ChangeSet, Verb};
use crate::config::Config;
use crate::config::Theme;
use crate::git;
use crate::messages::Message as Msg;
use crate::messages::{Event, Message};
use crate::prelude::*;
use crate::query::Query;
use crate::store::Workspace;
use crate::store::model::{Group, Item};
use crate::store::{self, WriteError};
use chyron::TickerState;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::UnboundedSender;

/// A background task the user is waiting on.
///
/// Carries enough to prove it is still alive: a frame that advances on every
/// tick, and when it started. A scan over a real inbox runs for minutes, and a
/// motionless banner cannot be told apart from a wedged process.
#[derive(Debug, Clone)]
pub struct Busy {
    pub label: String,
    pub started: std::time::Instant,
    frame: usize,
}

impl Busy {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            started: std::time::Instant::now(),
            frame: 0,
        }
    }

    fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// The current spinner glyph.
    pub fn spinner(&self) -> char {
        const FRAMES: [char; 8] = ['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];
        FRAMES[(self.frame / 2) % FRAMES.len()]
    }

    /// How long it has been running, as "8s" or "2m 04s".
    pub fn elapsed(&self) -> String {
        let secs = self.started.elapsed().as_secs();
        if secs < 60 {
            format!("{secs}s")
        } else {
            format!("{}m {:02}s", secs / 60, secs % 60)
        }
    }
}

/// A draggable boundary between panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    /// Between the item list and the detail pane.
    ItemsDetail,
    /// Between the groups pane and everything right of it.
    GroupsMain,
}

/// Which pane takes keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Groups,
    Items,
}

/// What keyboard input currently means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing into the query line.
    EditingQuery,
    /// Typing replacement or new text for an item.
    Editing(EditKind),
    /// Waiting for y/n on a destructive action.
    ConfirmingDelete,
    /// Waiting for y/n before archiving finished items.
    ConfirmingArchive,
    /// Showing scrollable output; any key dismisses.
    Modal,
    /// Showing a proposed change-set; y applies, anything else discards.
    ReviewingChangeSet,
    /// Typing the input for an agent verb.
    AskingAgent(Verb),
    /// The view-settings menu is open.
    ViewMenu,
}

/// A toggle in the view menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewSetting {
    Wrap,
    HideDone,
    Ticker,
}

impl ViewSetting {
    pub const ALL: [ViewSetting; 3] = [
        ViewSetting::Wrap,
        ViewSetting::HideDone,
        ViewSetting::Ticker,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ViewSetting::Wrap => "wrap long text",
            ViewSetting::HideDone => "hide finished items",
            ViewSetting::Ticker => "scrolling ticker",
        }
    }
}

/// What the text currently being typed will become.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    AddSibling,
    AddChild,
    EditText,
    Description,
}

impl EditKind {
    fn prompt(self) -> &'static str {
        match self {
            EditKind::AddSibling => "new item",
            EditKind::AddChild => "new sub-item",
            EditKind::EditText => "edit",
            EditKind::Description => "description",
        }
    }
}

/// The running application.
///
/// eilmeldung's `App` carries the whole RSS session — NewsFlash handle, sync
/// state, login. mitodo's owns a `Workspace` and the cursor state, which is why
/// this is a rewrite rather than a port.
pub struct App {
    pub workspace: Workspace,
    pub theme: Theme,
    pub focus: Focus,
    /// 0 selects the synthetic "all" group; 1.. index into `workspace.groups`.
    pub group_cursor: usize,
    pub item_cursor: usize,
    pub hide_done: bool,
    pub mode: Mode,
    /// Text currently being typed into the query line.
    pub query_input: String,
    /// The applied query, if any.
    pub query: Option<Query>,
    /// Parse failure from the last attempted query, shown in the status bar.
    pub query_error: Option<String>,
    /// Text being typed for an item edit.
    pub edit_buffer: String,
    /// Outcome of the last write, shown in the status bar.
    pub notice: Option<String>,
    /// Title and body of the modal, when one is open.
    pub modal: Option<(String, Vec<String>)>,
    /// Change-set awaiting review.
    pub pending: Option<ChangeSet>,
    /// Which proposed changes are picked, and where the cursor is in the list.
    pub review_selected: Vec<bool>,
    pub review_cursor: usize,
    pub review_scroll: usize,
    /// The background task in flight, if any.
    pub busy: Option<Busy>,
    /// Scrolling ticker, when enabled.
    pub ticker: Option<TickerState>,
    /// Wrap long item text instead of truncating it.
    pub wrap: bool,
    /// Cursor position within the view-settings menu.
    pub view_cursor: usize,
    /// First visible row of the item list. Owned by the view, not derived
    /// from the cursor, so the wheel can scroll without moving the selection.
    pub item_scroll: usize,
    /// First visible row of the groups list.
    pub group_scroll: usize,
    /// Height of the items pane when the divider has been dragged.
    pub items_height: Option<u16>,
    /// Width of the groups pane when its divider has been dragged.
    pub groups_width: Option<u16>,
    /// Layout of the last frame drawn, for hit-testing mouse events.
    pub(crate) layout: view::Frames,
    /// Which item each row of the item pane showed, so a click on a wrapped
    /// item's second row still selects that item.
    item_rows: Vec<usize>,
    /// Row each item starts at, and the total, for scroll maths.
    item_starts: Vec<usize>,
    item_total_rows: usize,
    /// Which divider, if any, is being dragged.
    drag: Option<Divider>,

    sender: Option<UnboundedSender<Msg>>,
    config: Config,
    /// Where to write remembered view state on exit.
    config_path: Option<PathBuf>,
    should_quit: bool,
}

impl App {
    pub fn new(workspace: Workspace, config: Config) -> Self {
        let hide_done = config.ui.hide_done;
        let config_wrap = config.ui.wrap;
        let ticker = config.ui.ticker.then(|| TickerState::new(2));
        Self {
            workspace,
            config,
            config_path: None,
            theme: Theme::default(),
            focus: Focus::Items,
            group_cursor: 0,
            item_cursor: 0,
            hide_done,
            mode: Mode::Normal,
            query_input: String::new(),
            query: None,
            query_error: None,
            edit_buffer: String::new(),
            notice: None,
            modal: None,
            pending: None,
            review_selected: Vec::new(),
            review_cursor: 0,
            review_scroll: 0,
            busy: None,
            ticker,
            wrap: config_wrap,
            view_cursor: 0,
            item_scroll: 0,
            group_scroll: 0,
            items_height: None,
            groups_width: None,
            layout: view::Frames::default(),
            item_rows: Vec::new(),
            item_starts: Vec::new(),
            item_total_rows: 0,
            drag: None,
            sender: None,
            should_quit: false,
        }
    }

    /// Remember where the config lives so view state can be written on exit.
    pub fn with_config_path(mut self, path: &Path) -> Self {
        self.config_path = Some(path.to_path_buf());
        // The ticker needs filling once the workspace is known.
        if let Some(mut ticker) = self.ticker.take() {
            self.refill_ticker(&mut ticker);
            self.ticker = Some(ticker);
        }
        self
    }

    /// Write remembered view state back to the config file.
    ///
    /// Only touched when something actually changed, so a config the user is
    /// editing by hand is left alone during an ordinary session.
    fn persist_ui_state(&mut self) {
        let Some(path) = self.config_path.clone() else {
            return;
        };
        let current = crate::config::UiConfig {
            hide_done: self.hide_done,
            ticker: self.ticker.is_some(),
            // Not a view toggle; preserve whatever the user configured.
            mouse: self.config.ui.mouse,
            wrap: self.wrap,
        };
        if current == self.config.ui {
            return;
        }
        self.config.ui = current;
        if let Err(err) = self.config.save(&path) {
            warn!("could not save view state: {err}");
        }
    }

    /// Apply a query supplied on the command line.
    pub fn set_query(&mut self, text: &str) -> Result<(), crate::query::QueryError> {
        self.query = Query::parse(text)?;
        self.query_input = text.to_string();
        let len = self.visible_items().len();
        self.item_cursor = self.item_cursor.min(len.saturating_sub(1));
        Ok(())
    }

    /// Give the app a channel so background tasks can report back.
    pub fn with_sender(mut self, sender: UnboundedSender<Msg>) -> Self {
        self.sender = Some(sender);
        self
    }

    /// The selected group, or `None` when "all" is selected.
    pub fn selected_group(&self) -> Option<&Group> {
        if self.group_cursor == 0 {
            None
        } else {
            self.workspace.groups.get(self.group_cursor - 1)
        }
    }

    /// Items shown in the items pane, after group and hide-done filtering,
    /// in the order the query asks for.
    pub fn visible_items(&self) -> Vec<&Item> {
        let group_file = self.selected_group().map(|g| &g.todo_file);
        let mut items: Vec<(&Item, Option<&str>)> = self
            .workspace
            .items
            .iter()
            .filter(|item| group_file.is_none_or(|f| &item.file == f))
            .filter(|item| !(self.hide_done && item.done))
            .map(|item| (item, self.workspace.group_name_for(item)))
            .filter(|(item, group)| match &self.query {
                Some(query) => query.matches(item, *group),
                None => true,
            })
            .collect();

        if let Some(query) = &self.query {
            query.sort_items(&mut items);
        }
        items.into_iter().map(|(item, _)| item).collect()
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.visible_items().get(self.item_cursor).copied()
    }

    /// Main loop: draw, wait for a message, handle it, repeat.
    pub async fn run(
        mut self,
        mut messages: UnboundedReceiver<Message>,
        mut terminal: DefaultTerminal,
    ) -> Result<()> {
        let mut drawn = None;
        terminal.draw(|frame| drawn = Some(view::render(&self, frame)))?;
        if let Some(frames) = drawn {
            self.adopt(frames);
        }

        while let Some(message) = messages.recv().await {
            self.handle(message);
            if self.should_quit {
                info!("quit requested");
                self.persist_ui_state();
                break;
            }
            let mut drawn = None;
            terminal.draw(|frame| drawn = Some(view::render(&self, frame)))?;
            if let Some(frames) = drawn {
                self.adopt(frames);
            }
        }

        Ok(())
    }

    /// Take the measurements of the frame just drawn.
    fn adopt(&mut self, rendered: view::Rendered) {
        self.layout = rendered.frames;
        self.item_rows = rendered.item_rows;
        self.item_starts = rendered.item_starts;
        self.item_total_rows = rendered.item_total_rows;
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Event(Event::Key(key)) => self.handle_key(key),
            Message::Event(Event::Quit) => self.should_quit = true,
            // A redraw follows every message, so a resize needs no handling.
            Message::Event(Event::Resized(..)) => {}
            Message::Event(Event::Mouse(mouse)) => self.handle_mouse(mouse),
            Message::Event(Event::Tick) => {
                if let Some(mut ticker) = self.ticker.take() {
                    ticker.advance();
                    self.ticker = Some(ticker);
                }
                if let Some(busy) = &mut self.busy {
                    busy.tick();
                }
            }
            Message::Event(Event::TaskFinished { title, body }) => {
                self.busy = None;
                self.open_modal(&title, body.lines().map(|l| l.to_string()).collect());
            }
            Message::Event(Event::QueryProposed(text)) => {
                self.busy = None;
                self.query_input = text;
                self.apply_query();
            }
            Message::Event(Event::ChangeSetProposed(set)) => {
                self.busy = None;
                // Everything starts picked: the common case is accepting the
                // lot, and unpicking is easier than picking from nothing.
                self.review_selected = vec![true; set.changes.len()];
                self.review_cursor = 0;
                self.review_scroll = 0;
                self.pending = Some(set);
                self.mode = Mode::ReviewingChangeSet;
            }
            Message::Event(Event::WorkspaceReloaded) => {
                // Don't stomp on a half-typed edit; the reload lands when the
                // user finishes. Only announce a change that is actually
                // someone else's — mitodo's own writes reload eagerly and so
                // leave the fingerprint already up to date.
                if self.mode == Mode::Normal {
                    let before = self.workspace.fingerprint();
                    self.reload();
                    if self.workspace.fingerprint() != before {
                        self.notice = Some("workspace changed on disk, reloaded".to_string());
                    }
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // A notice reports what just happened; the next thing you do dismisses
        // it. Without this it sits over the keybinding hints forever.
        self.notice = None;
        let key = normalise(key);
        match self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::EditingQuery => self.handle_query_key(key),
            Mode::Editing(kind) => self.handle_edit_key(key, kind),
            Mode::ConfirmingDelete => self.handle_confirm_key(key),
            Mode::ConfirmingArchive => {
                self.mode = Mode::Normal;
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    self.archive_done();
                }
            }
            Mode::Modal => {
                self.mode = Mode::Normal;
                self.modal = None;
            }
            Mode::ReviewingChangeSet => self.handle_review_key(key),
            Mode::AskingAgent(verb) => self.handle_ask_key(key, verb),
            Mode::ViewMenu => self.handle_view_key(key),
        }
    }

    /// Whether a view setting is currently on.
    pub fn view_setting(&self, setting: ViewSetting) -> bool {
        match setting {
            ViewSetting::Wrap => self.wrap,
            ViewSetting::HideDone => self.hide_done,
            ViewSetting::Ticker => self.ticker.is_some(),
        }
    }

    fn toggle_view_setting(&mut self, setting: ViewSetting) {
        match setting {
            ViewSetting::Wrap => self.wrap = !self.wrap,
            ViewSetting::HideDone => self.toggle_hide_done(),
            ViewSetting::Ticker => self.toggle_ticker(),
        }
    }

    fn handle_view_key(&mut self, key: KeyEvent) {
        use KeyCode as K;
        let last = ViewSetting::ALL.len() - 1;
        match key.code {
            K::Esc | K::Char('q') | K::Char('v') => self.mode = Mode::Normal,
            K::Char('j') | K::Down => self.view_cursor = (self.view_cursor + 1).min(last),
            K::Char('k') | K::Up => self.view_cursor = self.view_cursor.saturating_sub(1),
            K::Char(' ') | K::Enter => self.toggle_view_setting(ViewSetting::ALL[self.view_cursor]),
            _ => {}
        }
    }

    fn handle_review_key(&mut self, key: KeyEvent) {
        use KeyCode as K;
        let last = self
            .pending
            .as_ref()
            .map_or(0, |s| s.changes.len())
            .saturating_sub(1);
        match key.code {
            K::Esc | K::Char('q') | K::Char('n') => {
                self.mode = Mode::Normal;
                self.pending = None;
                self.notice = Some("change-set discarded".to_string());
            }
            K::Char('j') | K::Down => {
                self.review_cursor = (self.review_cursor + 1).min(last);
                self.follow_review_cursor();
            }
            K::Char('k') | K::Up => {
                self.review_cursor = self.review_cursor.saturating_sub(1);
                self.follow_review_cursor();
            }
            K::Char('g') => {
                self.review_cursor = 0;
                self.follow_review_cursor();
            }
            K::Char('G') => {
                self.review_cursor = last;
                self.follow_review_cursor();
            }
            K::Char(' ') => self.toggle_review_at(self.review_cursor),
            K::Char('a') | K::Char('A') => {
                // All or nothing, whichever is the bigger change.
                let all_on = self.review_selected.iter().all(|s| *s);
                self.review_selected.iter_mut().for_each(|s| *s = !all_on);
            }
            K::Enter | K::Char('y') | K::Char('Y') => self.apply_review(),
            _ => {}
        }
    }

    /// Clicks and scrolling inside the review list.
    fn handle_review_mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) {
        let total = self.pending.as_ref().map_or(0, |s| s.changes.len());
        let height = view::review_visible_rows(self.layout.whole);
        match kind {
            MouseEventKind::ScrollDown => {
                self.review_scroll = scroll_by(self.review_scroll, SCROLL_STEP, total, height);
            }
            MouseEventKind::ScrollUp => {
                self.review_scroll = scroll_by(self.review_scroll, -SCROLL_STEP, total, height);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let chosen = self.review_selected.iter().filter(|s| **s).count();
                let whole = self.layout.whole;
                if within(view::apply_button_rect(whole, chosen), x, y) {
                    self.apply_review();
                } else if within(view::cancel_button_rect(whole, chosen), x, y) {
                    self.mode = Mode::Normal;
                    self.pending = None;
                    self.notice = Some("change-set discarded".to_string());
                } else if let Some(index) = self.review_row_at(x, y) {
                    self.toggle_review_at(index);
                }
            }
            _ => {}
        }
    }

    /// Which proposed change sits under this point, if any.
    fn review_row_at(&self, x: u16, y: u16) -> Option<usize> {
        let popup = view::review_rect(self.layout.whole);
        if !within(popup, x, y) {
            return None;
        }
        let first = view::review_first_row(self.layout.whole);
        let height = view::review_visible_rows(self.layout.whole);
        if y < first || y >= first + height as u16 {
            return None;
        }
        let total = self.pending.as_ref().map_or(0, |s| s.changes.len());
        let index = self.review_scroll + (y - first) as usize;
        (index < total).then_some(index)
    }

    fn toggle_review_at(&mut self, index: usize) {
        if let Some(slot) = self.review_selected.get_mut(index) {
            *slot = !*slot;
            self.review_cursor = index;
        }
    }

    fn follow_review_cursor(&mut self) {
        let height = view::review_visible_rows(self.layout.whole);
        if height == 0 {
            return;
        }
        if self.review_cursor < self.review_scroll {
            self.review_scroll = self.review_cursor;
        } else if self.review_cursor >= self.review_scroll + height {
            self.review_scroll = self.review_cursor + 1 - height;
        }
    }

    /// Apply the picked changes and report what happened.
    fn apply_review(&mut self) {
        self.mode = Mode::Normal;
        let Some(set) = self.pending.take() else {
            return;
        };
        let picked = set.selected(&self.review_selected);
        if picked.changes.is_empty() {
            self.notice = Some("nothing selected; no changes applied".to_string());
            return;
        }

        let report = agent::changeset::apply(&self.workspace.root, &self.workspace.items, &picked);
        self.reload();
        let mut body = vec![format!(
            "applied {} of {} proposed change(s)",
            report.applied,
            set.changes.len()
        )];
        body.extend(report.skipped.iter().map(|s| format!("skipped: {s}")));
        self.open_modal("apply", body);
    }

    fn handle_ask_key(&mut self, key: KeyEvent, verb: Verb) {
        use KeyCode as K;
        match key.code {
            K::Esc => {
                self.mode = Mode::Normal;
                self.edit_buffer.clear();
            }
            K::Enter => {
                let input = std::mem::take(&mut self.edit_buffer);
                self.mode = Mode::Normal;
                self.spawn_agent(verb, input);
            }
            K::Backspace => {
                self.edit_buffer.pop();
            }
            K::Char(c) => self.edit_buffer.push(c),
            _ => {}
        }
    }

    fn handle_edit_key(&mut self, key: KeyEvent, kind: EditKind) {
        use KeyCode as K;
        match key.code {
            K::Esc => {
                self.mode = Mode::Normal;
                self.edit_buffer.clear();
            }
            K::Enter => self.commit_edit(kind),
            K::Backspace => {
                self.edit_buffer.pop();
            }
            K::Char(c) => self.edit_buffer.push(c),
            _ => {}
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        use KeyCode as K;
        match key.code {
            K::Char('y') | K::Char('Y') => {
                self.mode = Mode::Normal;
                self.delete_selected();
            }
            _ => self.mode = Mode::Normal,
        }
    }

    /// Keys while typing a query. Enter applies, Esc abandons the edit and
    /// leaves whatever query was previously applied in place.
    fn handle_query_key(&mut self, key: KeyEvent) {
        use KeyCode as K;
        match key.code {
            K::Esc => {
                self.mode = Mode::Normal;
                self.query_error = None;
            }
            K::Enter => self.apply_query(),
            K::Backspace => {
                self.query_input.pop();
            }
            K::Char(c) => self.query_input.push(c),
            _ => {}
        }
    }

    /// Parse `query_input` and adopt it, or report why it failed.
    fn apply_query(&mut self) {
        match Query::parse(&self.query_input) {
            Ok(query) => {
                self.query = query;
                self.query_error = None;
                self.mode = Mode::Normal;
                // The list just changed length underneath the cursor.
                let len = self.visible_items().len();
                self.item_cursor = self.item_cursor.min(len.saturating_sub(1));
            }
            Err(err) => self.query_error = Some(err.to_string()),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        use KeyCode as K;
        match (key.code, key.modifiers) {
            (K::Char('/'), KeyModifiers::NONE) => {
                self.mode = Mode::EditingQuery;
                self.query_error = None;
            }
            (K::Esc, _) => self.clear_query(),
            (K::Char('q'), KeyModifiers::NONE) => self.should_quit = true,
            (K::Char('c'), KeyModifiers::CONTROL) => self.should_quit = true,

            (K::Char('j'), KeyModifiers::NONE) | (K::Down, _) => self.move_cursor(1),
            (K::Char('k'), KeyModifiers::NONE) | (K::Up, _) => self.move_cursor(-1),
            (K::Char('g'), KeyModifiers::NONE) => self.cursor_to_start(),
            (K::Char('G'), _) => self.cursor_to_end(),

            (K::Tab, _) | (K::Char('l'), KeyModifiers::NONE) | (K::Right, _) => {
                self.focus = Focus::Items
            }
            (K::BackTab, _) | (K::Left, _) => self.focus = Focus::Groups,

            (K::Char('h'), KeyModifiers::NONE) => self.toggle_hide_done(),

            (K::Char(' '), _) | (K::Char('x'), KeyModifiers::NONE) => self.toggle_selected(),
            (K::Char('a'), KeyModifiers::NONE) => self.begin_edit(EditKind::AddSibling, false),
            (K::Char('A'), _) => self.begin_edit(EditKind::AddChild, false),
            (K::Char('e'), KeyModifiers::NONE) => self.begin_edit(EditKind::EditText, true),
            (K::Char('i'), KeyModifiers::NONE) => self.begin_edit(EditKind::Description, true),
            (K::Char('s'), KeyModifiers::NONE) => self.spawn_git_sync(),
            (K::Char('c'), KeyModifiers::NONE) => self.toggle_ticker(),
            (K::Char('p'), KeyModifiers::NONE) => {
                if let Some(ticker) = &mut self.ticker {
                    ticker.toggle_pause();
                }
            }
            (K::Char('+'), _) => {
                if let Some(ticker) = &mut self.ticker {
                    ticker.speed_up();
                }
            }
            (K::Char('-'), _) => {
                if let Some(ticker) = &mut self.ticker {
                    ticker.speed_down();
                }
            }
            (K::Char('?'), _) => self.open_modal("keys", help_lines()),
            (K::Char('v'), KeyModifiers::NONE) => {
                self.view_cursor = 0;
                self.mode = Mode::ViewMenu;
            }
            (K::Char('N'), _) => self.show_notes(),
            (K::Char('X'), _) => self.begin_archive(),
            (K::Char('n'), KeyModifiers::NONE) => self.begin_ask(Verb::Query),
            (K::Char('S'), _) => self.spawn_agent(Verb::Summarize, String::new()),
            (K::Char('b'), KeyModifiers::NONE) => match self.selected_item() {
                Some(item) => {
                    let text = item.text.clone();
                    self.spawn_agent(Verb::Breakdown, text)
                }
                None => self.notice = Some("no item selected".to_string()),
            },
            (K::Char('R'), _) => self.spawn_agent(Verb::Scan, String::new()),
            // Guarded rather than nested: deleting nothing is not a mode.
            (K::Char('d'), KeyModifiers::NONE) if self.selected_item().is_some() => {
                self.mode = Mode::ConfirmingDelete;
            }
            _ => {}
        }
    }

    /// Route a mouse event to whichever pane it landed in.
    ///
    /// Hit-tested against the layout of the frame actually on screen, so a
    /// dragged divider or a visible command line does not throw the maths off.
    fn handle_mouse(&mut self, mouse: MouseEvent) {
        let (x, y) = (mouse.column, mouse.row);
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            self.notice = None;
        }

        // The view menu is one of two overlays that take clicks.
        if self.mode == Mode::ViewMenu {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.click_view_menu(x, y);
            }
            return;
        }

        // The review list is the other: pick changes with the mouse.
        if self.mode == Mode::ReviewingChangeSet {
            self.handle_review_mouse(mouse.kind, x, y);
            return;
        }

        // Other modal and prompt states own the screen; ignore clicks behind them.
        if self.mode != Mode::Normal {
            return;
        }

        match mouse.kind {
            MouseEventKind::ScrollDown => self.scroll_at(x, y, SCROLL_STEP),
            MouseEventKind::ScrollUp => self.scroll_at(x, y, -SCROLL_STEP),
            MouseEventKind::Down(MouseButton::Left) => match self.divider_at(x, y) {
                Some(divider) => self.drag = Some(divider),
                None => self.click_at(x, y),
            },
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(divider) = self.drag {
                    self.drag_to(divider, x, y);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.drag = None,
            _ => {}
        }
    }

    /// Which divider, if any, sits under this point.
    fn divider_at(&self, x: u16, y: u16) -> Option<Divider> {
        let items = self.layout.items;
        let groups = self.layout.groups;

        // The items/detail divider is the items pane's bottom border.
        if items.height > 0 && y == items.y + items.height - 1 && within_x(items, x) {
            return Some(Divider::ItemsDetail);
        }
        // The groups divider is that pane's right border.
        if groups.width > 0
            && x == groups.x + groups.width - 1
            && y >= groups.y
            && y < groups.y + groups.height
        {
            return Some(Divider::GroupsMain);
        }
        None
    }

    fn drag_to(&mut self, divider: Divider, x: u16, y: u16) {
        // Every pane keeps a border pair plus a row of content.
        const MIN: u16 = 3;
        match divider {
            Divider::ItemsDetail => {
                let items = self.layout.items;
                let bottom = self.layout.detail.y + self.layout.detail.height;
                let height = y.saturating_sub(items.y).saturating_add(1);
                self.items_height = Some(height.clamp(MIN, bottom.saturating_sub(items.y + MIN)));
            }
            Divider::GroupsMain => {
                let groups = self.layout.groups;
                let right = self.layout.items.x + self.layout.items.width;
                let width = x.saturating_sub(groups.x).saturating_add(1);
                // Twice the minimum on the right so the item list stays usable.
                self.groups_width =
                    Some(width.clamp(MIN, right.saturating_sub(groups.x + MIN * 2)));
            }
        }
    }

    /// Bring the item cursor back into view after it moves.
    fn follow_item_cursor(&mut self) {
        let height = content_height(self.layout.items);
        if height == 0 {
            return;
        }
        let Some(&start) = self.item_starts.get(self.item_cursor) else {
            return;
        };
        // Where the selected item ends, so a wrapped one is shown whole.
        let end = self
            .item_starts
            .get(self.item_cursor + 1)
            .copied()
            .unwrap_or(self.item_total_rows);

        if start < self.item_scroll {
            self.item_scroll = start;
        } else if end > self.item_scroll + height {
            self.item_scroll = end.saturating_sub(height);
        }
    }

    fn follow_group_cursor(&mut self) {
        let height = content_height(self.layout.groups);
        if height == 0 {
            return;
        }
        if self.group_cursor < self.group_scroll {
            self.group_scroll = self.group_cursor;
        } else if self.group_cursor >= self.group_scroll + height {
            self.group_scroll = self.group_cursor + 1 - height;
        }
    }

    /// Scroll the pane under the pointer, leaving the selection alone.
    ///
    /// This is the ordinary scrollwheel contract: the view moves, what you had
    /// selected stays selected. Moving the cursor is what j/k and clicks do.
    fn scroll_at(&mut self, x: u16, y: u16, delta: isize) {
        if within(self.layout.groups, x, y) {
            let rows = self.workspace.groups.len() + 1;
            let height = content_height(self.layout.groups);
            self.group_scroll = scroll_by(self.group_scroll, delta, rows, height);
        } else if within(self.layout.items, x, y) {
            let height = content_height(self.layout.items);
            self.item_scroll = scroll_by(self.item_scroll, delta, self.item_total_rows, height);
        }
    }

    /// A click while the view menu is open: toggle an entry, or dismiss.
    fn click_view_menu(&mut self, x: u16, y: u16) {
        let tab = view::view_tab_rect(self.layout.top_bar);
        if within(tab, x, y) {
            self.mode = Mode::Normal;
            return;
        }
        let menu = view::view_menu_rect(self.layout.top_bar);
        if !within(menu, x, y) {
            self.mode = Mode::Normal;
            return;
        }
        if let Some(row) = row_index(menu, y)
            && let Some(setting) = ViewSetting::ALL.get(row).copied()
        {
            self.view_cursor = row;
            self.toggle_view_setting(setting);
        }
    }

    fn click_at(&mut self, x: u16, y: u16) {
        // The view tab lives in the top bar and opens its menu.
        if within(view::view_tab_rect(self.layout.top_bar), x, y) {
            self.view_cursor = 0;
            self.mode = Mode::ViewMenu;
            return;
        }
        if within(self.layout.groups, x, y) {
            if let Some(index) = row_index(self.layout.groups, y) {
                let rows = self.workspace.groups.len() + 1;
                let start =
                    viewport_start(self.group_cursor, rows, content_height(self.layout.groups));
                if start + index < rows {
                    self.focus = Focus::Groups;
                    self.group_cursor = start + index;
                    self.item_cursor = 0;
                }
            }
        } else if within(self.layout.items, x, y)
            && let Some(row) = row_index(self.layout.items, y)
            && let Some(index) = self.item_rows.get(row).copied()
        {
            self.focus = Focus::Items;
            self.item_cursor = index;
        }
    }

    /// Move the focused pane's cursor, saturating at both ends.
    fn move_cursor(&mut self, delta: isize) {
        let (cursor, len) = match self.focus {
            Focus::Groups => (&mut self.group_cursor, self.workspace.groups.len() + 1),
            Focus::Items => {
                let len = self.visible_items().len();
                (&mut self.item_cursor, len)
            }
        };
        if len == 0 {
            *cursor = 0;
            return;
        }
        let last = len - 1;
        *cursor = cursor.saturating_add_signed(delta).min(last);

        // Changing group invalidates the item cursor.
        if self.focus == Focus::Groups {
            self.item_cursor = 0;
            self.item_scroll = 0;
        }
        self.follow_cursor();
    }

    /// Keep whichever cursor just moved inside its pane.
    fn follow_cursor(&mut self) {
        self.follow_item_cursor();
        self.follow_group_cursor();
    }

    fn cursor_to_start(&mut self) {
        match self.focus {
            Focus::Groups => {
                self.group_cursor = 0;
                self.item_cursor = 0;
            }
            Focus::Items => self.item_cursor = 0,
        }
        self.follow_cursor();
    }

    fn cursor_to_end(&mut self) {
        match self.focus {
            Focus::Groups => {
                self.group_cursor = self.workspace.groups.len();
                self.item_cursor = 0;
            }
            Focus::Items => {
                self.item_cursor = self.visible_items().len().saturating_sub(1);
            }
        }
        self.follow_cursor();
    }

    /// Open the text editor, optionally seeded with the item's current value.
    fn begin_edit(&mut self, kind: EditKind, seed_from_item: bool) {
        let Some(item) = self.selected_item() else {
            // Adding to an empty list has no anchor line to insert after.
            self.notice = Some("no item selected".to_string());
            return;
        };
        self.edit_buffer = if seed_from_item {
            match kind {
                EditKind::EditText => item.text.clone(),
                EditKind::Description => item.description.clone(),
                _ => String::new(),
            }
        } else {
            String::new()
        };
        self.mode = Mode::Editing(kind);
    }

    fn commit_edit(&mut self, kind: EditKind) {
        let Some(item) = self.selected_item() else {
            self.mode = Mode::Normal;
            return;
        };
        // Copy what the write needs before borrowing self mutably.
        let (file, line, raw, indent) =
            (item.file.clone(), item.line, item.raw.clone(), item.indent);
        let text = self.edit_buffer.clone();

        let result = match kind {
            EditKind::AddSibling => store::add_item(&file, line, &raw, indent, &text),
            EditKind::AddChild => store::add_item(&file, line, &raw, indent + 2, &text),
            EditKind::EditText => store::edit_text(&file, line, &raw, &text),
            EditKind::Description => store::set_description(&file, line, &raw, &text),
        };

        self.mode = Mode::Normal;
        self.edit_buffer.clear();
        self.after_write(result, kind.prompt());
    }

    fn toggle_selected(&mut self) {
        let Some(item) = self.selected_item() else {
            return;
        };
        let (file, line, raw, done) = (item.file.clone(), item.line, item.raw.clone(), item.done);
        let result = store::toggle(&file, line, &raw, !done);
        self.after_write(result, "toggle");
    }

    fn delete_selected(&mut self) {
        let Some(item) = self.selected_item() else {
            return;
        };
        let (file, line, raw) = (item.file.clone(), item.line, item.raw.clone());
        let result = store::delete_item(&file, line, &raw);
        self.after_write(result, "delete");
    }

    /// Report the outcome and re-read the workspace.
    ///
    /// A `Conflict` means another writer changed the line first; reloading is
    /// both the recovery and the way the user sees what actually happened.
    fn after_write(&mut self, result: Result<(), WriteError>, what: &str) {
        match result {
            Ok(()) => {
                self.notice = None;
                self.reload();
            }
            Err(WriteError::Conflict { .. }) => {
                self.notice = Some(format!("{what} failed: file changed on disk, reloaded"));
                self.reload();
            }
            Err(err) => self.notice = Some(format!("{what} failed: {err}")),
        }
    }

    /// Re-read the workspace from disk, keeping the cursor on the same item
    /// where possible. Item ids are content hashes, so an edit deliberately
    /// moves the cursor to whatever now occupies the row.
    pub fn reload(&mut self) {
        let previous = self.selected_item().map(|i| i.id.clone());
        match Workspace::load(&self.config) {
            Ok(workspace) => self.workspace = workspace,
            Err(err) => {
                self.notice = Some(format!("reload failed: {err}"));
                return;
            }
        }
        if let Some(id) = previous
            && let Some(index) = self.visible_items().iter().position(|i| i.id == id)
        {
            self.item_cursor = index;
        }
        let len = self.visible_items().len();
        self.item_cursor = self.item_cursor.min(len.saturating_sub(1));

        // The ticker mirrors the workspace, so it follows every reload.
        if let Some(mut ticker) = self.ticker.take() {
            let offset = ticker.offset;
            self.refill_ticker(&mut ticker);
            // Keep the scroll position so a background reload does not make
            // the ticker visibly jump back to the start.
            ticker.offset = offset;
            self.ticker = Some(ticker);
        }
    }

    /// Turn the ticker on or off, seeding it from the current view.
    fn toggle_ticker(&mut self) {
        if self.ticker.is_some() {
            self.ticker = None;
            return;
        }
        let mut ticker = TickerState::new(2);
        self.refill_ticker(&mut ticker);
        self.ticker = Some(ticker);
    }

    fn refill_ticker(&self, ticker: &mut TickerState) {
        let entries: Vec<_> = self
            .visible_items()
            .into_iter()
            .map(|item| (item, self.workspace.group_name_for(item)))
            .collect();
        ticker.fill(entries.into_iter());
    }

    /// Ask before archiving, since it rewrites two files at once.
    fn begin_archive(&mut self) {
        match self.archive_target() {
            Some(_) => self.mode = Mode::ConfirmingArchive,
            None => {
                self.notice =
                    Some("select a group with an archive directory configured".to_string())
            }
        }
    }

    /// The selected group's todo and archive paths, if archiving is possible.
    fn archive_target(&self) -> Option<(PathBuf, PathBuf)> {
        let group = self.selected_group()?;
        let archive = group.archive_dir.clone()?;
        Some((group.todo_file.clone(), archive))
    }

    fn archive_done(&mut self) {
        let Some((todo_file, archive_dir)) = self.archive_target() else {
            return;
        };
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let items = self.workspace.items.clone();

        match store::archive_done(&todo_file, &archive_dir, &items, &date) {
            Ok(report) => {
                self.reload();
                let mut body = vec![format!("archived {} item(s)", report.archived)];
                body.extend(report.skipped.iter().map(|s| format!("skipped: {s}")));
                self.open_modal("archive", body);
            }
            Err(err) => self.notice = Some(format!("archive failed: {err}")),
        }
    }

    /// Show the selected group's notes sidecar.
    ///
    /// `notes_glob` is detected and recorded per group; this is what makes it
    /// worth recording.
    fn show_notes(&mut self) {
        let Some(group) = self.selected_group() else {
            self.notice = Some("select a group to read its notes".to_string());
            return;
        };
        let name = group.name.clone();
        let Some(path) = group.notes_file.clone() else {
            self.notice = Some(format!("{name} has no notes file"));
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let body: Vec<String> = text.lines().map(|l| l.to_string()).collect();
                self.open_modal(&format!("{name} notes"), body);
            }
            Err(err) => self.notice = Some(format!("could not read {}: {err}", path.display())),
        }
    }

    fn open_modal(&mut self, title: &str, body: Vec<String>) {
        self.modal = Some((title.to_string(), body));
        self.mode = Mode::Modal;
    }

    fn begin_ask(&mut self, verb: Verb) {
        if self.config.agent.command.is_empty() {
            self.notice = Some("no agent configured (set [agent] command)".to_string());
            return;
        }
        self.edit_buffer.clear();
        self.mode = Mode::AskingAgent(verb);
    }

    /// Everything the agent needs about the current view, as plain text.
    /// Every todo file, with its workspace-relative path and contents.
    ///
    /// This is what `scan` needs: a proposed change names the file it belongs
    /// to, so the agent has to have seen the paths.
    fn files_context(&self) -> String {
        self.workspace
            .groups
            .iter()
            .filter_map(|group| {
                let rel = group
                    .todo_file
                    .strip_prefix(&self.workspace.root)
                    .unwrap_or(&group.todo_file)
                    .to_string_lossy()
                    .to_string();
                std::fs::read_to_string(&group.todo_file)
                    .ok()
                    .map(|text| format!("### {rel}\n```markdown\n{text}\n```"))
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn items_context(&self) -> String {
        self.visible_items()
            .iter()
            .map(|i| {
                format!(
                    "- [{}] {} ({})",
                    if i.done { "x" } else { " " },
                    i.text,
                    i.priority.as_str()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Prompt template for a verb: the configured file if present and readable,
    /// otherwise the built-in.
    fn prompt_template(&self, verb: Verb) -> String {
        self.config
            .agent
            .prompts
            .get(verb.label())
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_else(|| verb.default_prompt().to_string())
    }

    fn spawn_git_sync(&mut self) {
        if !self.config.git.enabled {
            self.notice = Some("git sync is disabled in config".to_string());
            return;
        }
        let (Some(sender), root, commands) = (
            self.sender.clone(),
            self.workspace.root.clone(),
            self.config.git.sync.clone(),
        ) else {
            return;
        };
        self.busy = Some(Busy::new("git sync"));
        std::thread::spawn(move || {
            let outcome = git::run_sync(&root, &commands, "git");
            let _ = sender.send(Msg::Event(Event::TaskFinished {
                title: format!("git sync ({})", if outcome.ok { "ok" } else { "failed" }),
                body: outcome.transcript,
            }));
        });
    }

    fn spawn_agent(&mut self, verb: Verb, input: String) {
        if self.config.agent.command.is_empty() {
            self.notice = Some("no agent configured (set [agent] command)".to_string());
            return;
        }
        let Some(sender) = self.sender.clone() else {
            return;
        };
        let prompt = agent::render_prompt(
            &self.prompt_template(verb),
            &input,
            &self.items_context(),
            &self.files_context(),
        );
        let command = self.config.agent.command.clone();
        let schema_flag = self.config.agent.schema_flag.clone();
        let timeout = self.config.agent.timeout_secs;
        let root = self.workspace.root.clone();

        self.busy = Some(Busy::new(verb.label()));
        std::thread::spawn(move || {
            let result = agent::run(
                &command,
                schema_flag.as_deref(),
                verb.schema(),
                &prompt,
                &root,
                timeout,
            );
            let event = match result {
                Err(err) => Event::TaskFinished {
                    title: format!("{} failed", verb.label()),
                    body: err.to_string(),
                },
                Ok(json) => interpret(verb, &json),
            };
            let _ = sender.send(Msg::Event(event));
        });
    }

    fn clear_query(&mut self) {
        self.query = None;
        self.query_input.clear();
        self.query_error = None;
        let len = self.visible_items().len();
        self.item_cursor = self.item_cursor.min(len.saturating_sub(1));
    }

    fn toggle_hide_done(&mut self) {
        self.hide_done = !self.hide_done;
        // The visible list just changed length underneath the cursor.
        let len = self.visible_items().len();
        self.item_cursor = self.item_cursor.min(len.saturating_sub(1));
    }
}

/// Turn an agent's JSON reply into the event that acts on it.
fn interpret(verb: Verb, json: &str) -> Event {
    match verb {
        Verb::Query => match agent::field(json, "query") {
            Ok(query) => Event::QueryProposed(query),
            Err(err) => fail(verb, err.to_string()),
        },
        Verb::Summarize => match agent::field(json, "brief") {
            Ok(brief) => Event::TaskFinished {
                title: "summary".to_string(),
                body: brief,
            },
            Err(err) => fail(verb, err.to_string()),
        },
        Verb::Breakdown => match agent::string_list(json, "sub_items") {
            Ok(items) => Event::TaskFinished {
                title: "proposed sub-items".to_string(),
                body: items.join("\n"),
            },
            Err(err) => fail(verb, err.to_string()),
        },
        Verb::Scan => match ChangeSet::parse(json) {
            Ok(set) => Event::ChangeSetProposed(set),
            Err(err) => fail(verb, err.to_string()),
        },
    }
}

fn fail(verb: Verb, body: String) -> Event {
    Event::TaskFinished {
        title: format!("{} failed", verb.label()),
        body,
    }
}

fn help_lines() -> Vec<String> {
    [
        "navigation",
        "  j/k  down/up          g/G  first/last",
        "  tab  focus items      shift-tab  focus groups",
        "",
        "items",
        "  space/x  toggle done  a/A  add sibling/child",
        "  e  edit text          i  edit description",
        "  d  delete             h  hide done",
        "",
        "query",
        "  /  edit query         esc  clear query",
        "",
        "agent and sync",
        "  n  natural language to query",
        "  S  summarise view     b  break down item",
        "  R  scan for changes   s  git sync",
        "",
        "chyron",
        "  c  toggle ticker      p  pause",
        "",
        "groups",
        "  N  read the selected group's notes.md",
        "  X  archive finished items into _archive/",
        "  +/-  faster/slower",
        "",
        "view",
        "  v  view settings (wrap, hide done, ticker)",
        "",
        "  ?  this help          q  quit",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn within(rect: Rect, x: u16, y: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && x >= rect.x
        && x < rect.x + rect.width
        && y >= rect.y
        && y < rect.y + rect.height
}

fn within_x(rect: Rect, x: u16) -> bool {
    rect.width > 0 && x >= rect.x && x < rect.x + rect.width
}

/// Rows a bordered pane can show.
fn content_height(rect: Rect) -> usize {
    rect.height.saturating_sub(2) as usize
}

/// Which content row a click landed on, ignoring the border rows.
fn row_index(rect: Rect, y: u16) -> Option<usize> {
    let first = rect.y + 1;
    let last = rect.y + rect.height.saturating_sub(1);
    (y >= first && y < last).then(|| (y - first) as usize)
}

/// Fold the terminal's shift reporting into one shape.
///
/// Terminals disagree about shifted letters: some send `Char('R')` with SHIFT,
/// some `Char('R')` with no modifier, and some `Char('r')` with SHIFT. Binding
/// on an exact modifier set therefore works in one terminal and silently does
/// nothing in another, which is how `R` and `S` came to be dead keys.
fn normalise(key: KeyEvent) -> KeyEvent {
    match key.code {
        KeyCode::Char(c)
            if c.is_ascii_lowercase() && key.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            KeyEvent {
                code: KeyCode::Char(c.to_ascii_uppercase()),
                ..key
            }
        }
        _ => key,
    }
}

/// Rows moved per wheel notch.
const SCROLL_STEP: isize = 3;

/// Move a scroll offset, clamped to the list.
pub fn scroll_by(scroll: usize, delta: isize, len: usize, height: usize) -> usize {
    if height == 0 || len <= height {
        return 0;
    }
    let max = len - height;
    scroll.saturating_add_signed(delta).min(max)
}

/// First visible row so that `cursor` sits inside a window of `height` rows.
///
/// Keeps the cursor pinned inside the viewport rather than recentring, which is
/// what vim-style navigation expects.
pub fn viewport_start(cursor: usize, len: usize, height: usize) -> usize {
    if height == 0 || len <= height {
        return 0;
    }
    let max_start = len - height;
    // Show the cursor at the bottom edge once it passes the fold.
    cursor.saturating_sub(height - 1).min(max_start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::store::model::{ItemId, Priority};
    use std::path::PathBuf;

    fn item(file: &str, text: &str, done: bool) -> Item {
        Item {
            id: ItemId::compute(file, "P0", "H", 0, text),
            file: PathBuf::from(file),
            line: 0,
            indent: 0,
            done,
            text: text.to_string(),
            raw: format!("- [{}] {}", if done { "x" } else { " " }, text),
            description: String::new(),
            section: "P0".to_string(),
            heading: "H".to_string(),
            priority: Priority::P0,
            due: None,
            parent: None,
            children: Vec::new(),
        }
    }

    fn group(name: &str) -> Group {
        Group {
            name: name.to_string(),
            todo_file: PathBuf::from(name),
            notes_file: None,
            archive_dir: None,
        }
    }

    fn app() -> App {
        App::new(
            Workspace {
                root: PathBuf::from("/w"),
                groups: vec![group("a"), group("b")],
                items: vec![
                    item("a", "a-open", false),
                    item("a", "a-done", true),
                    item("b", "b-open", false),
                ],
            },
            Config::default(),
        )
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn all_group_is_selected_first_and_shows_everything() {
        let app = app();
        assert!(app.selected_group().is_none(), "row 0 is the synthetic all");
        assert_eq!(app.visible_items().len(), 3);
    }

    #[test]
    fn selecting_a_group_filters_the_item_list() {
        let mut app = app();
        app.focus = Focus::Groups;
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_group().unwrap().name, "a");
        assert_eq!(app.visible_items().len(), 2);
    }

    #[test]
    fn hide_done_removes_completed_items() {
        let mut app = app();
        press(&mut app, KeyCode::Char('h'));
        assert!(app.hide_done);
        assert_eq!(app.visible_items().len(), 2);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.visible_items().len(), 3);
    }

    #[test]
    fn cursor_saturates_at_both_ends() {
        let mut app = app();
        for _ in 0..10 {
            press(&mut app, KeyCode::Char('j'));
        }
        assert_eq!(app.item_cursor, 2, "stops at the last item");
        for _ in 0..10 {
            press(&mut app, KeyCode::Char('k'));
        }
        assert_eq!(app.item_cursor, 0, "stops at the first item");
    }

    #[test]
    fn g_and_shift_g_jump_to_the_ends() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        assert_eq!(app.item_cursor, 2);
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.item_cursor, 0);
    }

    #[test]
    fn changing_group_resets_the_item_cursor() {
        let mut app = app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.item_cursor, 2);

        app.focus = Focus::Groups;
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.item_cursor, 0, "cursor must not dangle past the filter");
    }

    #[test]
    fn hiding_done_clamps_a_cursor_that_is_now_out_of_range() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        assert_eq!(app.item_cursor, 2);
        press(&mut app, KeyCode::Char('h'));
        assert!(
            app.item_cursor < app.visible_items().len(),
            "cursor stays inside the shorter list"
        );
    }

    #[test]
    fn tab_and_shift_tab_move_focus() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Groups);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Items);
    }

    #[test]
    fn selected_item_tracks_the_cursor() {
        let mut app = app();
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_item().unwrap().text, "a-done");
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let mut quit_with_q = app();
        press(&mut quit_with_q, KeyCode::Char('q'));
        assert!(quit_with_q.should_quit);

        let mut quit_with_ctrl_c = app();
        quit_with_ctrl_c.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(quit_with_ctrl_c.should_quit);
    }

    #[test]
    fn viewport_keeps_the_cursor_visible() {
        // Short list: never scrolls.
        assert_eq!(viewport_start(0, 3, 10), 0);
        assert_eq!(viewport_start(2, 3, 10), 0);

        // Long list: cursor pinned to the bottom edge once past the fold.
        assert_eq!(viewport_start(0, 100, 10), 0);
        assert_eq!(viewport_start(9, 100, 10), 0);
        assert_eq!(viewport_start(10, 100, 10), 1);
        assert_eq!(viewport_start(99, 100, 10), 90, "clamps at the last page");
    }

    #[test]
    fn viewport_handles_degenerate_sizes() {
        assert_eq!(viewport_start(0, 0, 0), 0);
        assert_eq!(viewport_start(5, 3, 0), 0);
    }

    fn type_query(app: &mut App, text: &str) {
        press(app, KeyCode::Char('/'));
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn slash_enters_query_mode_and_enter_applies() {
        let mut app = app();
        type_query(&mut app, "!done");
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.query.is_some());
        assert_eq!(app.visible_items().len(), 2, "only open items");
    }

    #[test]
    fn query_filters_by_group_name() {
        let mut app = app();
        type_query(&mut app, "acct:b");
        assert_eq!(app.visible_items().len(), 1);
        assert_eq!(app.visible_items()[0].text, "b-open");
    }

    #[test]
    fn a_bad_query_reports_an_error_and_stays_in_edit_mode() {
        let mut app = app();
        type_query(&mut app, "bogus:x");
        assert_eq!(app.mode, Mode::EditingQuery, "stays open to be corrected");
        assert!(app.query_error.is_some());
        assert!(app.query.is_none(), "no query applied");
        assert_eq!(app.visible_items().len(), 3, "list unchanged");
    }

    #[test]
    fn esc_abandons_the_edit_without_applying() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('x'));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.query.is_none());
    }

    #[test]
    fn esc_in_normal_mode_clears_an_applied_query() {
        let mut app = app();
        type_query(&mut app, "acct:b");
        assert_eq!(app.visible_items().len(), 1);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.query.is_none());
        assert_eq!(app.visible_items().len(), 3);
    }

    #[test]
    fn backspace_edits_the_query_buffer() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        for c in "acct:bx".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.visible_items().len(),
            1,
            "acct:b applied after backspace"
        );
    }

    #[test]
    fn normal_keys_do_not_act_while_typing_a_query() {
        let mut app = app();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit, "q is text, not quit, while editing");
        assert_eq!(app.query_input, "q");
    }

    #[test]
    fn applying_a_query_clamps_a_now_out_of_range_cursor() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
        assert_eq!(app.item_cursor, 2);
        type_query(&mut app, "acct:b");
        assert!(app.item_cursor < app.visible_items().len());
    }

    #[test]
    fn query_and_group_selection_compose() {
        let mut app = app();
        type_query(&mut app, "!done");
        app.focus = Focus::Groups;
        press(&mut app, KeyCode::Char('j')); // group "a"
        assert_eq!(app.visible_items().len(), 1, "open items in group a only");
        assert_eq!(app.visible_items()[0].text, "a-open");
    }

    // --- stage D: mutations against a real temp workspace ---

    /// A workspace on disk plus an App pointed at it.
    fn disk_app(body: &str) -> (tempfile::TempDir, App) {
        use crate::config::{GroupBy, WorkspaceConfig};
        let dir = tempfile::tempdir().unwrap();
        let g = dir.path().join("lefv");
        std::fs::create_dir_all(&g).unwrap();
        std::fs::write(g.join("TODO.md"), body).unwrap();

        let config = Config {
            workspace: WorkspaceConfig {
                root: dir.path().to_path_buf(),
                group_by: GroupBy::Directory,
                todo_glob: "*/TODO.md".to_string(),
                notes_glob: None,
                archive_dir: None,
            },
            ..Default::default()
        };
        let workspace = Workspace::load(&config).unwrap();
        let app = App::new(workspace, config);
        (dir, app)
    }

    fn file_of(app: &App) -> std::path::PathBuf {
        app.workspace.groups[0].todo_file.clone()
    }

    fn read(app: &App) -> String {
        std::fs::read_to_string(file_of(app)).unwrap()
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }

    const DOC: &str = "## P0 — Critical\n\n- [ ] alpha\n- [x] beta\n";

    #[test]
    fn space_toggles_the_selected_item_on_disk() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(read(&app), "## P0 — Critical\n\n- [x] alpha\n- [x] beta\n");
        assert!(
            app.workspace.items[0].done,
            "reloaded state reflects the write"
        );
    }

    #[test]
    fn toggling_twice_returns_the_file_to_its_original_bytes() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(read(&app), DOC);
    }

    #[test]
    fn e_edits_the_selected_item_text() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.edit_buffer, "alpha", "seeded with the current text");
        app.edit_buffer.clear();
        type_text(&mut app, "alpha revised");
        assert_eq!(
            read(&app),
            "## P0 — Critical\n\n- [ ] alpha revised\n- [x] beta\n"
        );
    }

    #[test]
    fn a_adds_a_sibling_below_the_selection() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('a'));
        type_text(&mut app, "inserted");
        assert_eq!(
            read(&app),
            "## P0 — Critical\n\n- [ ] alpha\n- [ ] inserted\n- [x] beta\n"
        );
    }

    #[test]
    fn shift_a_adds_an_indented_child() {
        let (_d, mut app) = disk_app(DOC);
        app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
        type_text(&mut app, "child");
        assert_eq!(
            read(&app),
            "## P0 — Critical\n\n- [ ] alpha\n  - [ ] child\n- [x] beta\n"
        );
    }

    #[test]
    fn i_sets_the_description() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('i'));
        type_text(&mut app, "needs CPA sign-off");
        assert_eq!(
            read(&app),
            "## P0 — Critical\n\n- [ ] alpha\n  > needs CPA sign-off\n- [x] beta\n"
        );
        assert_eq!(app.workspace.items[0].description, "needs CPA sign-off");
    }

    #[test]
    fn d_asks_before_deleting_and_y_confirms() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.mode, Mode::ConfirmingDelete);
        assert_eq!(read(&app), DOC, "nothing written before confirmation");

        press(&mut app, KeyCode::Char('y'));
        assert_eq!(read(&app), "## P0 — Critical\n\n- [x] beta\n");
    }

    #[test]
    fn any_other_key_cancels_the_delete() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(read(&app), DOC, "file untouched");
    }

    #[test]
    fn esc_abandons_an_edit_without_writing() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Char('z'));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(read(&app), DOC);
    }

    #[test]
    fn a_concurrent_edit_surfaces_as_a_conflict_and_reloads() {
        let (_d, mut app) = disk_app(DOC);
        // Another writer changes the line after mitodo parsed it.
        std::fs::write(
            file_of(&app),
            "## P0 — Critical\n\n- [ ] alpha, amended elsewhere\n- [x] beta\n",
        )
        .unwrap();

        press(&mut app, KeyCode::Char(' '));

        let notice = app.notice.as_deref().unwrap_or_default();
        assert!(
            notice.contains("changed on disk"),
            "conflict reported: {notice}"
        );
        assert_eq!(
            app.workspace.items[0].text, "alpha, amended elsewhere",
            "reloaded to the other writer's version"
        );
        assert!(
            read(&app).contains("- [ ] alpha, amended elsewhere"),
            "the other writer's change was not clobbered"
        );
    }

    #[test]
    fn hide_done_is_seeded_from_the_config() {
        let (dir, _app) = disk_app(DOC);
        drop(dir);

        let (_d, mut app) = disk_app(DOC);
        app.config.ui.hide_done = true;
        let seeded = App::new(app.workspace.clone(), app.config.clone());
        assert!(seeded.hide_done, "remembered from last session");
        assert_eq!(seeded.visible_items().len(), 1);
    }

    #[test]
    fn view_state_is_written_back_on_quit() {
        let (dir, mut app) = disk_app(DOC);
        let config_path = dir.path().join("config.toml");
        app.config.save(&config_path).unwrap();
        app = app.with_config_path(&config_path);

        press(&mut app, KeyCode::Char('h')); // hide done
        press(&mut app, KeyCode::Char('c')); // ticker on
        app.persist_ui_state();

        let reloaded = Config::load(&config_path).unwrap();
        assert!(reloaded.ui.hide_done, "hide_done remembered");
        assert!(reloaded.ui.ticker, "ticker remembered");
    }

    #[test]
    fn an_unchanged_view_leaves_the_config_file_untouched() {
        let (dir, mut app) = disk_app(DOC);
        let config_path = dir.path().join("config.toml");
        app.config.save(&config_path).unwrap();
        let before = std::fs::read_to_string(&config_path).unwrap();

        app = app.with_config_path(&config_path);
        app.persist_ui_state();

        assert_eq!(
            std::fs::read_to_string(&config_path).unwrap(),
            before,
            "a config being hand-edited is not rewritten for nothing"
        );
    }

    #[test]
    fn shift_x_archives_finished_items_after_confirmation() {
        let (dir, mut app) = disk_app(DOC);
        app.config.workspace.archive_dir = Some("_archive".to_string());
        std::fs::create_dir_all(dir.path().join("lefv/_archive")).unwrap();
        app.reload();

        app.focus = Focus::Groups;
        press(&mut app, KeyCode::Char('j')); // select "lefv"
        app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT));
        assert_eq!(app.mode, Mode::ConfirmingArchive);
        assert_eq!(read(&app), DOC, "nothing moved before confirmation");

        press(&mut app, KeyCode::Char('y'));
        assert_eq!(
            read(&app),
            "## P0 — Critical\n\n- [ ] alpha\n",
            "beta moved out"
        );
        let archived = std::fs::read_to_string(dir.path().join("lefv/_archive/TODO.md")).unwrap();
        assert!(archived.contains("- [x] beta"), "and moved in");
    }

    #[test]
    fn declining_the_archive_prompt_changes_nothing() {
        let (dir, mut app) = disk_app(DOC);
        app.config.workspace.archive_dir = Some("_archive".to_string());
        std::fs::create_dir_all(dir.path().join("lefv/_archive")).unwrap();
        app.reload();

        app.focus = Focus::Groups;
        press(&mut app, KeyCode::Char('j'));
        app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(read(&app), DOC);
    }

    #[test]
    fn archiving_needs_a_group_and_a_configured_archive_dir() {
        let (_d, mut app) = disk_app(DOC);
        // "all" is selected, and no archive_dir is configured.
        app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT));
        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.notice
                .as_deref()
                .unwrap_or_default()
                .contains("archive directory"),
            "got {:?}",
            app.notice
        );
    }

    #[test]
    fn shift_n_reads_the_group_notes_sidecar() {
        let (dir, mut app) = disk_app(DOC);
        std::fs::write(dir.path().join("lefv/notes.md"), "background\nreading\n").unwrap();
        // Re-detect so the sidecar is picked up.
        app.config.workspace.notes_glob = Some("*/notes.md".to_string());
        app.reload();

        app.focus = Focus::Groups;
        press(&mut app, KeyCode::Char('j')); // select "lefv"
        app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));

        let (title, body) = app.modal.clone().expect("notes modal opened");
        assert!(title.contains("lefv"));
        assert!(body.contains(&"background".to_string()));
    }

    #[test]
    fn shift_n_explains_when_a_group_has_no_notes() {
        let (_d, mut app) = disk_app(DOC);
        app.focus = Focus::Groups;
        press(&mut app, KeyCode::Char('j'));
        app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
        assert!(app.modal.is_none());
        assert!(
            app.notice
                .as_deref()
                .unwrap_or_default()
                .contains("no notes file"),
            "got {:?}",
            app.notice
        );
    }

    #[test]
    fn shift_n_on_the_all_row_asks_for_a_group() {
        let (_d, mut app) = disk_app(DOC);
        assert!(app.selected_group().is_none(), "all is selected");
        app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
        assert!(
            app.notice
                .as_deref()
                .unwrap_or_default()
                .contains("select a group")
        );
    }

    // --- view menu ---

    #[test]
    fn a_notice_is_dismissed_by_the_next_keypress() {
        // Regression: "workspace changed on disk, reloaded" used to sit over
        // the keybinding hints for the rest of the session.
        let (_d, mut app) = disk_app(DOC);
        app.notice = Some("workspace changed on disk, reloaded".to_string());

        press(&mut app, KeyCode::Char('j'));
        assert!(app.notice.is_none(), "the next thing you do clears it");
    }

    #[test]
    fn a_notice_is_dismissed_by_a_click() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        app.notice = Some("something happened".to_string());

        let items = app.layout.items;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            items.x + 5,
            items.y + 1,
        ));
        assert!(app.notice.is_none());
    }

    #[test]
    fn an_action_can_still_report_its_own_outcome() {
        // Clearing on keypress must not swallow the message that keypress makes.
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('s')); // git sync, disabled here
        assert!(
            app.notice
                .as_deref()
                .unwrap_or_default()
                .contains("disabled"),
            "got {:?}",
            app.notice
        );
    }

    #[test]
    fn v_opens_and_closes_the_view_menu() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.mode, Mode::ViewMenu);
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn esc_closes_the_view_menu() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('v'));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn space_toggles_wrap_from_the_menu() {
        let (_d, mut app) = disk_app(DOC);
        assert!(!app.wrap);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char(' '));
        assert!(app.wrap, "first entry is wrap");
        press(&mut app, KeyCode::Char(' '));
        assert!(!app.wrap);
    }

    #[test]
    fn the_menu_reaches_every_setting() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('v'));

        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char(' '));
        assert!(app.hide_done, "second entry is hide done");

        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char(' '));
        assert!(app.ticker.is_some(), "third entry is the ticker");
    }

    #[test]
    fn the_menu_cursor_saturates() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('v'));
        for _ in 0..10 {
            press(&mut app, KeyCode::Char('j'));
        }
        assert_eq!(app.view_cursor, ViewSetting::ALL.len() - 1);
        for _ in 0..10 {
            press(&mut app, KeyCode::Char('k'));
        }
        assert_eq!(app.view_cursor, 0);
    }

    #[test]
    fn the_menu_reports_the_current_state() {
        let (_d, mut app) = disk_app(DOC);
        assert!(!app.view_setting(ViewSetting::Wrap));
        app.wrap = true;
        assert!(app.view_setting(ViewSetting::Wrap));
        assert!(!app.view_setting(ViewSetting::HideDone));
    }

    #[test]
    fn normal_keys_do_not_act_while_the_menu_is_open() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.mode, Mode::ViewMenu, "d did not open the delete prompt");
        assert_eq!(read(&app), DOC);
    }

    #[test]
    fn wrap_survives_a_restart() {
        let (dir, mut app) = disk_app(DOC);
        let config_path = dir.path().join("config.toml");
        app.config.save(&config_path).unwrap();
        app = app.with_config_path(&config_path);

        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char(' '));
        app.persist_ui_state();

        let reloaded = Config::load(&config_path).unwrap();
        assert!(reloaded.ui.wrap, "wrap remembered");
        assert!(
            App::new(app.workspace.clone(), reloaded).wrap,
            "and restored"
        );
    }

    // --- mouse ---

    fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// Draw a real frame headlessly, so layout and row mapping match what the
    /// mouse handler would see in a live terminal.
    fn with_layout(app: &mut App) {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        let mut drawn = None;
        terminal
            .draw(|frame| drawn = Some(view::render(app, frame)))
            .unwrap();
        app.adopt(drawn.unwrap());
    }

    #[test]
    fn hit_testing_respects_pane_bounds() {
        let r = Rect::new(10, 5, 20, 8);
        assert!(within(r, 10, 5), "top-left corner is inside");
        assert!(within(r, 29, 12), "bottom-right corner is inside");
        assert!(!within(r, 30, 12), "one past the right edge is outside");
        assert!(!within(r, 29, 13), "one past the bottom is outside");
        assert!(
            !within(Rect::new(0, 0, 0, 0), 0, 0),
            "an empty rect hits nothing"
        );
    }

    #[test]
    fn a_click_maps_to_the_row_under_it_ignoring_borders() {
        let r = Rect::new(0, 0, 20, 6);
        assert_eq!(row_index(r, 0), None, "top border");
        assert_eq!(row_index(r, 1), Some(0), "first content row");
        assert_eq!(row_index(r, 4), Some(3), "last content row");
        assert_eq!(row_index(r, 5), None, "bottom border");
    }

    #[test]
    fn clicking_an_item_selects_it() {
        let (_d, mut app) = disk_app("## P0\n\n- [ ] one\n- [ ] two\n- [ ] three\n");
        with_layout(&mut app);
        let items = app.layout.items;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            items.x + 5,
            items.y + 3,
        ));
        assert_eq!(app.focus, Focus::Items);
        assert_eq!(app.item_cursor, 2, "third content row");
        assert_eq!(app.selected_item().unwrap().text, "three");
    }

    #[test]
    fn clicking_a_group_selects_it_and_resets_the_item_cursor() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        app.item_cursor = 1;
        let groups = app.layout.groups;

        // Row 0 is the synthetic "all"; row 1 is the first real group.
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            groups.x + 3,
            groups.y + 2,
        ));
        assert_eq!(app.focus, Focus::Groups);
        assert_eq!(app.selected_group().unwrap().name, "lefv");
        assert_eq!(app.item_cursor, 0);
    }

    #[test]
    fn clicking_past_the_last_row_selects_nothing() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        let items = app.layout.items;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            items.x + 5,
            items.y + 10,
        ));
        assert_eq!(app.item_cursor, 0, "empty space below the list is inert");
    }

    /// A workspace with more items than fit on screen.
    fn long_app() -> (tempfile::TempDir, App) {
        let mut body = String::from("## P0 — Critical\n\n");
        for n in 0..60 {
            body.push_str(&format!("- [ ] item-{n:03}\n"));
        }
        let (dir, mut app) = disk_app(&body);
        with_layout(&mut app);
        (dir, app)
    }

    #[test]
    fn the_wheel_scrolls_the_view_and_leaves_the_selection_alone() {
        // The ordinary scrollwheel contract: looking around is not selecting.
        let (_d, mut app) = long_app();
        let items = app.layout.items;

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, items.x + 5, items.y + 2));
        assert_eq!(app.item_scroll, SCROLL_STEP as usize, "the view moved");
        assert_eq!(app.item_cursor, 0, "the selection did not");

        app.handle_mouse(mouse(MouseEventKind::ScrollUp, items.x + 5, items.y + 2));
        assert_eq!(app.item_scroll, 0);
        assert_eq!(app.item_cursor, 0);
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let (_d, mut app) = long_app();
        let items = app.layout.items;
        let height = content_height(items);

        for _ in 0..50 {
            app.handle_mouse(mouse(MouseEventKind::ScrollDown, items.x + 5, items.y + 2));
        }
        assert_eq!(
            app.item_scroll,
            app.item_total_rows - height,
            "the last row stays on screen"
        );

        for _ in 0..50 {
            app.handle_mouse(mouse(MouseEventKind::ScrollUp, items.x + 5, items.y + 2));
        }
        assert_eq!(app.item_scroll, 0);
    }

    #[test]
    fn a_short_list_does_not_scroll_at_all() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        let items = app.layout.items;
        app.handle_mouse(mouse(MouseEventKind::ScrollDown, items.x + 5, items.y + 2));
        assert_eq!(app.item_scroll, 0, "nothing to scroll to");
    }

    #[test]
    fn scrolling_does_not_steal_focus() {
        let (_d, mut app) = long_app();
        app.focus = Focus::Groups;
        let items = app.layout.items;

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, items.x + 5, items.y + 2));
        assert_eq!(app.focus, Focus::Groups, "reading is not selecting");
    }

    #[test]
    fn moving_the_cursor_pulls_the_view_along() {
        let (_d, mut app) = long_app();
        let height = content_height(app.layout.items);

        // Walk the cursor past the bottom of the viewport.
        for _ in 0..(height + 2) {
            press(&mut app, KeyCode::Char('j'));
        }
        assert!(app.item_scroll > 0, "the view followed the cursor down");
        assert!(
            app.item_cursor >= app.item_scroll,
            "cursor is inside the viewport"
        );

        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.item_scroll, 0, "and back to the top");
    }

    #[test]
    fn scrolling_away_then_moving_the_cursor_brings_it_back() {
        let (_d, mut app) = long_app();
        let items = app.layout.items;

        for _ in 0..10 {
            app.handle_mouse(mouse(MouseEventKind::ScrollDown, items.x + 5, items.y + 2));
        }
        assert!(app.item_scroll > 0);
        assert_eq!(app.item_cursor, 0, "selection untouched by scrolling");

        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.item_scroll, 1, "the cursor's row is shown again");
    }

    #[test]
    fn the_groups_divider_can_be_dragged() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        let groups = app.layout.groups;
        let edge = groups.x + groups.width - 1;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            edge,
            groups.y + 2,
        ));
        assert_eq!(app.drag, Some(Divider::GroupsMain));

        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            edge + 8,
            groups.y + 2,
        ));
        assert_eq!(app.groups_width, Some(groups.width + 8));

        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            edge + 8,
            groups.y + 2,
        ));
        assert!(app.drag.is_none());
    }

    #[test]
    fn the_groups_divider_cannot_swallow_the_item_list() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        let groups = app.layout.groups;
        let right = app.layout.items.x + app.layout.items.width;
        app.drag = Some(Divider::GroupsMain);

        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            right + 50,
            groups.y + 2,
        ));
        assert!(
            groups.x + app.groups_width.unwrap() <= right - 3,
            "the item list keeps room"
        );

        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            0,
            groups.y + 2,
        ));
        assert!(
            app.groups_width.unwrap() >= 3,
            "the groups pane stays usable"
        );
    }

    #[test]
    fn the_wheel_outside_any_pane_does_nothing() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        let before = app.item_cursor;
        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 0, 0)); // top bar
        assert_eq!(app.item_cursor, before);
    }

    #[test]
    fn dragging_the_divider_resizes_the_split() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        let items = app.layout.items;
        let divider = items.y + items.height - 1;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            items.x + 5,
            divider,
        ));
        assert_eq!(app.drag, Some(Divider::ItemsDetail));
        assert_eq!(app.item_cursor, 0, "grabbing the divider is not a click");

        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            items.x + 5,
            divider + 4,
        ));
        assert_eq!(app.items_height, Some(items.height + 4));

        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            items.x + 5,
            divider + 4,
        ));
        assert!(app.drag.is_none());
    }

    #[test]
    fn the_divider_cannot_be_dragged_to_collapse_a_pane() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        let items = app.layout.items;
        app.drag = Some(Divider::ItemsDetail);

        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            items.x + 5,
            0,
        ));
        assert!(app.items_height.unwrap() >= 3, "items pane stays usable");

        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            items.x + 5,
            200,
        ));
        let bottom = app.layout.detail.y + app.layout.detail.height;
        assert!(
            items.y + app.items_height.unwrap() <= bottom - 3,
            "detail pane stays usable"
        );
    }

    #[test]
    fn the_mouse_is_ignored_while_a_modal_is_open() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        press(&mut app, KeyCode::Char('?')); // opens the help modal
        let items = app.layout.items;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            items.x + 5,
            items.y + 2,
        ));
        assert_eq!(app.mode, Mode::Modal, "the click did not fall through");
        assert_eq!(app.item_cursor, 0);
    }

    #[test]
    fn clicking_the_view_tab_opens_the_menu() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        let tab = view::view_tab_rect(app.layout.top_bar);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            tab.x + 1,
            tab.y,
        ));
        assert_eq!(app.mode, Mode::ViewMenu);
    }

    #[test]
    fn clicking_a_menu_entry_toggles_it() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        press(&mut app, KeyCode::Char('v'));
        with_layout(&mut app);
        let menu = view::view_menu_rect(app.layout.top_bar);

        // First content row is "wrap long text".
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu.x + 4,
            menu.y + 1,
        ));
        assert!(app.wrap);
    }

    #[test]
    fn clicking_away_closes_the_menu() {
        let (_d, mut app) = disk_app(DOC);
        with_layout(&mut app);
        press(&mut app, KeyCode::Char('v'));
        with_layout(&mut app);

        let items = app.layout.items;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            items.x + 5,
            items.y + items.height - 2,
        ));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn clicking_a_wrapped_items_second_row_still_selects_that_item() {
        let long = "- [ ] ".to_string()
            + &"a very long item that will certainly need to be wrapped across rows ".repeat(3)
            + "\n- [ ] second\n";
        let (_d, mut app) = disk_app(&format!("## P0 — Critical\n\n{long}"));
        app.wrap = true;
        with_layout(&mut app);

        // The first item occupies several rows; row 1 is its continuation.
        assert!(app.item_rows.len() > 2);
        assert_eq!(app.item_rows[0], 0);
        assert_eq!(app.item_rows[1], 0, "continuation row belongs to item 0");

        let items = app.layout.items;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            items.x + 5,
            items.y + 2,
        ));
        assert_eq!(
            app.item_cursor, 0,
            "clicking a continuation row selects its item"
        );
    }

    #[test]
    fn c_toggles_the_ticker() {
        let (_d, mut app) = disk_app(DOC);
        assert!(app.ticker.is_none());
        press(&mut app, KeyCode::Char('c'));
        assert!(app.ticker.is_some(), "ticker on");
        press(&mut app, KeyCode::Char('c'));
        assert!(app.ticker.is_none(), "ticker off");
    }

    #[test]
    fn the_ticker_is_seeded_with_open_items_only() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('c'));
        let ticker = app.ticker.as_ref().unwrap();
        assert_eq!(ticker.items.len(), 1, "beta is done and excluded");
        assert_eq!(ticker.items[0].text, "alpha");
        assert_eq!(ticker.items[0].group, "lefv");
    }

    #[test]
    fn ticks_advance_only_when_the_ticker_is_on() {
        let (_d, mut app) = disk_app(DOC);
        app.handle(Message::Event(Event::Tick));
        assert!(app.ticker.is_none(), "a tick with no ticker is harmless");

        press(&mut app, KeyCode::Char('c'));
        app.handle(Message::Event(Event::Tick));
        assert!(app.ticker.as_ref().unwrap().offset > 0, "scrolled");
    }

    #[test]
    fn p_pauses_the_ticker() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('c'));
        press(&mut app, KeyCode::Char('p'));
        app.handle(Message::Event(Event::Tick));
        assert_eq!(app.ticker.as_ref().unwrap().offset, 0, "paused");
    }

    #[test]
    fn a_write_refills_the_ticker() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.ticker.as_ref().unwrap().items.len(), 1);

        // Completing the only open item empties the queue.
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(
            app.ticker.as_ref().unwrap().items.len(),
            0,
            "ticker follows the workspace"
        );
    }

    #[test]
    fn the_spinner_advances_on_every_tick() {
        // A motionless banner cannot be told apart from a wedged process.
        let (_d, mut app) = disk_app(DOC);
        app.busy = Some(Busy::new("scan"));

        let first = app.busy.as_ref().unwrap().spinner();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..16 {
            app.handle(Message::Event(Event::Tick));
            seen.insert(app.busy.as_ref().unwrap().spinner());
        }
        assert!(seen.len() > 1, "the spinner moved");
        assert!(seen.contains(&first), "and cycles back round");
    }

    #[test]
    fn ticks_are_harmless_when_nothing_is_running() {
        let (_d, mut app) = disk_app(DOC);
        assert!(app.busy.is_none());
        app.handle(Message::Event(Event::Tick));
        assert!(app.busy.is_none());
    }

    #[test]
    fn elapsed_reads_as_seconds_then_minutes() {
        let mut busy = Busy::new("scan");
        assert_eq!(busy.elapsed(), "0s");
        busy.started = std::time::Instant::now() - std::time::Duration::from_secs(75);
        assert_eq!(busy.elapsed(), "1m 15s");
        busy.started = std::time::Instant::now() - std::time::Duration::from_secs(600);
        assert_eq!(busy.elapsed(), "10m 00s");
    }

    #[test]
    fn a_finished_task_clears_the_busy_state() {
        let (_d, mut app) = disk_app(DOC);
        app.busy = Some(Busy::new("scan"));
        app.handle(Message::Event(Event::TaskFinished {
            title: "scan".to_string(),
            body: "done".to_string(),
        }));
        assert!(app.busy.is_none(), "the spinner stops when the work does");
    }

    #[test]
    fn shifted_keys_work_however_the_terminal_reports_them() {
        // Regression: R and S were dead keys in terminals that report shift
        // differently from tmux.
        for (code, modifiers) in [
            (KeyCode::Char('G'), KeyModifiers::SHIFT), // tmux
            (KeyCode::Char('G'), KeyModifiers::NONE),  // no modifier reported
            (KeyCode::Char('g'), KeyModifiers::SHIFT), // base key reported
        ] {
            let (_d, mut app) = disk_app("## P0\n\n- [ ] a\n- [ ] b\n- [ ] c\n");
            app.handle_key(KeyEvent::new(code, modifiers));
            assert_eq!(
                app.item_cursor, 2,
                "G should jump to the end for {code:?} + {modifiers:?}"
            );
        }
    }

    #[test]
    fn lowercase_keys_are_unaffected_by_normalisation() {
        let (_d, mut app) = disk_app("## P0\n\n- [ ] a\n- [ ] b\n- [ ] c\n");
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.item_cursor, 0, "plain g is still go-to-top");
    }

    #[test]
    fn the_scan_key_reaches_the_agent_however_shift_is_reported() {
        for modifiers in [KeyModifiers::SHIFT, KeyModifiers::NONE] {
            let (_d, mut app) = disk_app(DOC);
            app.handle_key(KeyEvent::new(KeyCode::Char('R'), modifiers));
            // No agent configured here, so it reports that rather than nothing.
            assert!(
                app.notice
                    .as_deref()
                    .unwrap_or_default()
                    .contains("no agent"),
                "R was a dead key for {modifiers:?}"
            );
        }
    }

    #[test]
    fn the_file_context_carries_paths_and_contents() {
        // Without the workspace-relative path, a scan change-set cannot say
        // which file it belongs to.
        let (_d, app) = disk_app(DOC);
        let context = app.files_context();
        assert!(
            context.contains("### lefv/TODO.md"),
            "path shown: {context}"
        );
        assert!(context.contains("- [ ] alpha"), "contents included");
        assert!(context.contains("```markdown"), "fenced for the agent");
    }

    #[test]
    fn the_view_context_is_the_rendered_list() {
        let (_d, app) = disk_app(DOC);
        let context = app.items_context();
        assert!(
            context.contains("- [ ] alpha"),
            "the rendered line, priority as configured: {context}"
        );
        assert!(
            !context.contains("TODO.md"),
            "no paths; that is what files is for"
        );
    }

    #[test]
    fn agent_keys_are_inert_without_configuration() {
        let (_d, mut app) = disk_app(DOC);
        for key in ['n', 'b'] {
            press(&mut app, KeyCode::Char(key));
            assert_eq!(app.mode, Mode::Normal, "{key} does not open an editor");
            assert!(
                app.notice
                    .as_deref()
                    .unwrap_or_default()
                    .contains("no agent"),
                "{key} explains why nothing happened"
            );
        }
    }

    #[test]
    fn sync_is_inert_when_git_is_disabled() {
        let (_d, mut app) = disk_app(DOC);
        assert!(!app.config.git.enabled);
        press(&mut app, KeyCode::Char('s'));
        assert!(
            app.notice
                .as_deref()
                .unwrap_or_default()
                .contains("disabled"),
            "explains that git sync is off"
        );
    }

    #[test]
    fn question_mark_opens_the_help_modal() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('?'));
        assert_eq!(app.mode, Mode::Modal);
        let (title, body) = app.modal.clone().unwrap();
        assert_eq!(title, "keys");
        assert!(body.join("\n").contains("space/x"));
    }

    #[test]
    fn any_key_dismisses_a_modal() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char('z'));
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.modal.is_none());
    }

    #[test]
    fn a_finished_task_opens_a_modal_and_clears_busy() {
        let (_d, mut app) = disk_app(DOC);
        app.busy = Some(Busy::new("git sync"));
        app.handle(Message::Event(Event::TaskFinished {
            title: "git sync (ok)".to_string(),
            body: "$ git push\nsync complete".to_string(),
        }));
        assert!(app.busy.is_none());
        assert_eq!(app.mode, Mode::Modal);
        assert!(app.modal.unwrap().1.contains(&"sync complete".to_string()));
    }

    #[test]
    fn a_proposed_query_is_applied() {
        let (_d, mut app) = disk_app(DOC);
        app.handle(Message::Event(Event::QueryProposed("!done".to_string())));
        assert!(app.query.is_some());
        assert_eq!(app.visible_items().len(), 1, "only alpha is open");
    }

    #[test]
    fn a_proposed_change_set_waits_for_review() {
        let (_d, mut app) = disk_app(DOC);
        let set = ChangeSet::parse(
            r#"{"summary":"s","changes":[{"file":"lefv/TODO.md","action":"complete",
                "content":"alpha","reason":"r"}]}"#,
        )
        .unwrap();
        app.handle(Message::Event(Event::ChangeSetProposed(set)));
        assert_eq!(app.mode, Mode::ReviewingChangeSet);
        assert_eq!(read(&app), DOC, "nothing written before review");
    }

    fn proposed(app: &mut App, n: usize) {
        let changes: Vec<String> = (0..n)
            .map(|i| {
                format!(
                    r#"{{"file":"lefv/TODO.md","action":"add","content":"proposal {i}","reason":"r{i}"}}"#
                )
            })
            .collect();
        let json = format!(
            r#"{{"summary":"found {n}","changes":[{}]}}"#,
            changes.join(",")
        );
        let set = ChangeSet::parse(&json).unwrap();
        app.handle(Message::Event(Event::ChangeSetProposed(set)));
    }

    #[test]
    fn everything_starts_picked() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 3);
        assert_eq!(app.review_selected, vec![true, true, true]);
        assert_eq!(app.review_cursor, 0);
    }

    #[test]
    fn space_unpicks_a_single_change() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 3);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(app.review_selected, vec![true, false, true]);
    }

    #[test]
    fn a_toggles_everything_at_once() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 3);
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.review_selected, vec![false, false, false]);
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.review_selected, vec![true, true, true]);
    }

    #[test]
    fn only_the_picked_changes_are_applied() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 3);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char(' ')); // unpick the second
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let text = read(&app);
        assert!(text.contains("proposal 0"), "picked one applied");
        assert!(!text.contains("proposal 1"), "unpicked one skipped");
        assert!(text.contains("proposal 2"), "picked one applied");
    }

    #[test]
    fn the_report_counts_against_what_was_proposed() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 3);
        press(&mut app, KeyCode::Char(' ')); // unpick the first
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let (_title, body) = app.modal.clone().unwrap();
        assert!(
            body[0].contains("2 of 3"),
            "says what it did and what was offered: {body:?}"
        );
    }

    #[test]
    fn picking_nothing_applies_nothing() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 2);
        press(&mut app, KeyCode::Char('a')); // none
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(read(&app), DOC, "file untouched");
        assert!(
            app.notice
                .as_deref()
                .unwrap_or_default()
                .contains("nothing selected")
        );
    }

    #[test]
    fn clicking_a_row_toggles_that_change() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 3);
        with_layout(&mut app);

        let first = view::review_first_row(app.layout.whole);
        let popup = view::review_rect(app.layout.whole);
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            popup.x + 5,
            first + 1,
        ));
        assert_eq!(
            app.review_selected,
            vec![true, false, true],
            "clicked the second"
        );
        assert_eq!(app.review_cursor, 1, "and highlighted it");
    }

    #[test]
    fn clicking_apply_applies_the_selection() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 3);
        with_layout(&mut app);
        press(&mut app, KeyCode::Char(' ')); // unpick the first
        with_layout(&mut app);

        let button = view::apply_button_rect(app.layout.whole, 2);
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            button.x + 2,
            button.y,
        ));

        assert_eq!(app.mode, Mode::Modal, "the report is shown");
        let text = read(&app);
        assert!(!text.contains("proposal 0"), "the unpicked one stayed out");
        assert!(text.contains("proposal 1") && text.contains("proposal 2"));
    }

    #[test]
    fn clicking_cancel_discards_without_writing() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 3);
        with_layout(&mut app);

        let button = view::cancel_button_rect(app.layout.whole, 3);
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            button.x + 2,
            button.y,
        ));

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(read(&app), DOC, "file untouched");
        assert!(
            app.notice
                .as_deref()
                .unwrap_or_default()
                .contains("discarded")
        );
    }

    #[test]
    fn the_buttons_do_not_overlap_the_change_rows() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 3);
        with_layout(&mut app);

        let whole = app.layout.whole;
        let apply = view::apply_button_rect(whole, 3);
        let first = view::review_first_row(whole);
        let last_row = first + view::review_visible_rows(whole) as u16;
        assert!(apply.y >= last_row, "buttons sit below the list");
        assert!(
            app.review_row_at(apply.x + 1, apply.y).is_none(),
            "a click on Apply is not also a row toggle"
        );
    }

    #[test]
    fn clicking_outside_the_rows_changes_nothing() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 2);
        with_layout(&mut app);

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0, 0));
        assert_eq!(app.review_selected, vec![true, true]);
        assert_eq!(app.mode, Mode::ReviewingChangeSet, "and does not discard");
    }

    #[test]
    fn a_long_list_scrolls_in_the_review() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 60);
        with_layout(&mut app);
        let popup = view::review_rect(app.layout.whole);

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, popup.x + 5, popup.y + 3));
        assert_eq!(app.review_scroll, SCROLL_STEP as usize);
    }

    #[test]
    fn esc_discards_without_writing() {
        let (_d, mut app) = disk_app(DOC);
        proposed(&mut app, 2);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(read(&app), DOC);
        assert!(
            app.notice
                .as_deref()
                .unwrap_or_default()
                .contains("discarded")
        );
    }

    #[test]
    fn y_applies_a_reviewed_change_set() {
        let (_d, mut app) = disk_app(DOC);
        let set = ChangeSet::parse(
            r#"{"summary":"s","changes":[{"file":"lefv/TODO.md","action":"complete",
                "content":"alpha","reason":"r"}]}"#,
        )
        .unwrap();
        app.handle(Message::Event(Event::ChangeSetProposed(set)));
        press(&mut app, KeyCode::Char('y'));
        assert!(read(&app).contains("- [x] alpha"), "change applied");
        assert_eq!(app.mode, Mode::Modal, "report shown");
    }

    #[test]
    fn n_discards_a_reviewed_change_set() {
        let (_d, mut app) = disk_app(DOC);
        let set = ChangeSet::parse(
            r#"{"summary":"s","changes":[{"file":"lefv/TODO.md","action":"complete",
                "content":"alpha","reason":"r"}]}"#,
        )
        .unwrap();
        app.handle(Message::Event(Event::ChangeSetProposed(set)));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(read(&app), DOC, "file untouched");
        assert!(
            app.notice
                .as_deref()
                .unwrap_or_default()
                .contains("discarded")
        );
    }

    #[test]
    fn an_own_write_does_not_announce_an_external_change() {
        // Regression: the watcher's digest snapshot lives on its own thread and
        // cannot tell whose write it saw, so a reload triggered right after
        // mitodo's own toggle used to report "changed on disk".
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char(' '));
        assert!(app.notice.is_none(), "own write is silent");

        app.handle(Message::Event(Event::WorkspaceReloaded));
        assert!(
            app.notice.is_none(),
            "a watcher event for our own write stays silent, got {:?}",
            app.notice
        );
    }

    #[test]
    fn an_external_change_is_announced_on_reload() {
        let (_d, mut app) = disk_app(DOC);
        std::fs::write(
            file_of(&app),
            "## P0 — Critical\n\n- [ ] alpha\n- [x] beta\n- [ ] gamma\n",
        )
        .unwrap();

        app.handle(Message::Event(Event::WorkspaceReloaded));
        assert_eq!(app.workspace.items.len(), 3, "picked up the new item");
        assert!(
            app.notice
                .as_deref()
                .unwrap_or_default()
                .contains("changed on disk"),
            "external change announced"
        );
    }

    #[test]
    fn a_reload_while_editing_is_deferred() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('e'));
        std::fs::write(
            file_of(&app),
            "## P0 — Critical\n\n- [ ] alpha\n- [x] beta\n- [ ] gamma\n",
        )
        .unwrap();

        app.handle(Message::Event(Event::WorkspaceReloaded));
        assert_eq!(
            app.workspace.items.len(),
            2,
            "a half-typed edit is not stomped on"
        );
    }

    #[test]
    fn normal_keys_do_not_act_while_typing_an_edit() {
        let (_d, mut app) = disk_app(DOC);
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit, "q is text while editing");
    }

    #[test]
    fn mutations_on_an_empty_list_are_a_no_op_with_a_notice() {
        let (_d, mut app) = disk_app("## P0 — Critical\n");
        assert!(app.selected_item().is_none());
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.mode, Mode::Normal, "no editor opened without an anchor");
        assert!(app.notice.is_some());
    }

    #[test]
    fn an_empty_workspace_does_not_panic_on_navigation() {
        let mut app = App::new(Workspace::default(), Config::default());
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.item_cursor, 0);
        assert!(app.selected_item().is_none());
    }
}
