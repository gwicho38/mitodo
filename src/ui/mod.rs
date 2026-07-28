mod view;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::config::Theme;
use crate::messages::{Event, Message};
use crate::prelude::*;
use crate::store::Workspace;
use crate::store::model::{Group, Item};

/// Which pane takes keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Groups,
    Items,
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
    should_quit: bool,
}

impl App {
    pub fn new(workspace: Workspace) -> Self {
        Self {
            workspace,
            theme: Theme::default(),
            focus: Focus::Items,
            group_cursor: 0,
            item_cursor: 0,
            hide_done: false,
            should_quit: false,
        }
    }

    /// The selected group, or `None` when "all" is selected.
    pub fn selected_group(&self) -> Option<&Group> {
        if self.group_cursor == 0 {
            None
        } else {
            self.workspace.groups.get(self.group_cursor - 1)
        }
    }

    /// Items shown in the items pane, after group and hide-done filtering.
    pub fn visible_items(&self) -> Vec<&Item> {
        let group_file = self.selected_group().map(|g| &g.todo_file);
        self.workspace
            .items
            .iter()
            .filter(|item| group_file.is_none_or(|f| &item.file == f))
            .filter(|item| !(self.hide_done && item.done))
            .collect()
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
            Message::Event(Event::WorkspaceReloaded) => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        use KeyCode as K;
        match (key.code, key.modifiers) {
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

    fn toggle_hide_done(&mut self) {
        self.hide_done = !self.hide_done;
        // The visible list just changed length underneath the cursor.
        let len = self.visible_items().len();
        self.item_cursor = self.item_cursor.min(len.saturating_sub(1));
    }
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
            description: String::new(),
            section: "P0".to_string(),
            heading: "H".to_string(),
            priority: Priority::P0,
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
        App::new(Workspace {
            root: PathBuf::from("/w"),
            groups: vec![group("a"), group("b")],
            items: vec![
                item("a", "a-open", false),
                item("a", "a-done", true),
                item("b", "b-open", false),
            ],
        })
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

    #[test]
    fn an_empty_workspace_does_not_panic_on_navigation() {
        let mut app = App::new(Workspace::default());
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.item_cursor, 0);
        assert!(app.selected_item().is_none());
    }
}
