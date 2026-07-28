mod view;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::config::Theme;
use crate::messages::{Event, Message};
use crate::prelude::*;
use crate::store::Workspace;

/// The running application.
///
/// eilmeldung's `App` carries the whole RSS session — NewsFlash handle, sync
/// state, login. mitodo's owns a `Workspace` and nothing else, which is why
/// this is a rewrite rather than a port.
pub struct App {
    pub workspace: Workspace,
    pub theme: Theme,
    should_quit: bool,
}

impl App {
    pub fn new(workspace: Workspace) -> Self {
        Self {
            workspace,
            theme: Theme::default(),
            should_quit: false,
        }
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
            // Redraw happens unconditionally after each message, so a resize
            // needs no further handling.
            Message::Event(Event::Resized(..)) => {}
            Message::Event(Event::Mouse(_)) => {}
            Message::Event(Event::WorkspaceReloaded) => {}
        }
    }

    /// Stage A recognises only quit. Stage B replaces this with the ported
    /// keybinding engine (`input/key.rs`, still in `_port`).
    fn handle_key(&mut self, key: KeyEvent) {
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE) => self.should_quit = true,
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => self.should_quit = true,
            _ => {}
        }
    }
}
