use ratatui::crossterm::event::{KeyEvent, MouseEvent};

/// Something that happened, from the terminal or from the filesystem.
#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resized(u16, u16),
    /// The workspace changed on disk and has been re-read.
    WorkspaceReloaded,
    /// The user asked to leave.
    Quit,
}
