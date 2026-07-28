// Stage A constructs only key events; the mouse payload, resize payload,
// WorkspaceReloaded and Quit gain consumers in stages B, D and E.
#![allow(dead_code)]

pub mod event;

pub use event::Event;

/// Everything that reaches the application loop arrives as a `Message`.
///
/// eilmeldung splits this into `Command` (user intent, from the keybinding
/// engine) and `Event` (something that happened). mitodo keeps the same shape
/// so the command vocabulary can land in a later stage without reworking the
/// loop, but phase 2 stage A only needs events.
#[derive(Debug)]
pub enum Message {
    Event(Event),
}
