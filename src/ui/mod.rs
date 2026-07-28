pub mod chyron;
mod view;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
    /// Label of a background task in flight.
    pub busy: Option<String>,
    /// Scrolling ticker, when enabled.
    pub ticker: Option<TickerState>,
    sender: Option<UnboundedSender<Msg>>,
    config: Config,
    /// Where to write remembered view state on exit.
    config_path: Option<PathBuf>,
    should_quit: bool,
}

impl App {
    pub fn new(workspace: Workspace, config: Config) -> Self {
        let hide_done = config.ui.hide_done;
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
            busy: None,
            ticker,
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
        terminal.draw(|frame| view::render(&self, frame))?;

        while let Some(message) = messages.recv().await {
            self.handle(message);
            if self.should_quit {
                info!("quit requested");
                self.persist_ui_state();
                break;
            }
            terminal.draw(|frame| view::render(&self, frame))?;
        }

        Ok(())
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Event(Event::Key(key)) => self.handle_key(key),
            Message::Event(Event::Quit) => self.should_quit = true,
            // A redraw follows every message, so a resize needs no handling.
            Message::Event(Event::Resized(..)) => {}
            Message::Event(Event::Mouse(_)) => {}
            Message::Event(Event::Tick) => {
                if let Some(mut ticker) = self.ticker.take() {
                    ticker.advance();
                    self.ticker = Some(ticker);
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
        }
    }

    fn handle_review_key(&mut self, key: KeyEvent) {
        self.mode = Mode::Normal;
        let Some(set) = self.pending.take() else {
            return;
        };
        if !matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
            self.notice = Some("change-set discarded".to_string());
            return;
        }
        let report = agent::changeset::apply(&self.workspace.root, &self.workspace.items, &set);
        self.reload();
        let mut body = vec![format!("applied {} change(s)", report.applied)];
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
            (K::Char('G'), KeyModifiers::SHIFT) => self.cursor_to_end(),

            (K::Tab, _) | (K::Char('l'), KeyModifiers::NONE) | (K::Right, _) => {
                self.focus = Focus::Items
            }
            (K::BackTab, _) | (K::Left, _) => self.focus = Focus::Groups,

            (K::Char('h'), KeyModifiers::NONE) => self.toggle_hide_done(),

            (K::Char(' '), _) | (K::Char('x'), KeyModifiers::NONE) => self.toggle_selected(),
            (K::Char('a'), KeyModifiers::NONE) => self.begin_edit(EditKind::AddSibling, false),
            (K::Char('A'), KeyModifiers::SHIFT) => self.begin_edit(EditKind::AddChild, false),
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
            (K::Char('N'), KeyModifiers::SHIFT) => self.show_notes(),
            (K::Char('X'), KeyModifiers::SHIFT) => self.begin_archive(),
            (K::Char('n'), KeyModifiers::NONE) => self.begin_ask(Verb::Query),
            (K::Char('S'), KeyModifiers::SHIFT) => self.spawn_agent(Verb::Summarize, String::new()),
            (K::Char('b'), KeyModifiers::NONE) => match self.selected_item() {
                Some(item) => {
                    let text = item.text.clone();
                    self.spawn_agent(Verb::Breakdown, text)
                }
                None => self.notice = Some("no item selected".to_string()),
            },
            (K::Char('R'), KeyModifiers::SHIFT) => self.spawn_agent(Verb::Scan, String::new()),
            // Guarded rather than nested: deleting nothing is not a mode.
            (K::Char('d'), KeyModifiers::NONE) if self.selected_item().is_some() => {
                self.mode = Mode::ConfirmingDelete;
            }
            _ => {}
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
        }
    }

    fn cursor_to_start(&mut self) {
        match self.focus {
            Focus::Groups => {
                self.group_cursor = 0;
                self.item_cursor = 0;
            }
            Focus::Items => self.item_cursor = 0,
        }
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
        self.busy = Some("git sync".to_string());
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
        let prompt =
            agent::render_prompt(&self.prompt_template(verb), &input, &self.items_context());
        let command = self.config.agent.command.clone();
        let schema_flag = self.config.agent.schema_flag.clone();
        let root = self.workspace.root.clone();

        self.busy = Some(verb.label().to_string());
        std::thread::spawn(move || {
            let result = agent::run(
                &command,
                schema_flag.as_deref(),
                verb.schema(),
                &prompt,
                &root,
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
        "  ?  this help          q  quit",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
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
        app.busy = Some("git sync".to_string());
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
