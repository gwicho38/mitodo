use std::time::Duration;

use log::{info, trace};
use ratatui::crossterm::event::{self, MouseEventKind};
use tokio::sync::mpsc::UnboundedSender;

use crate::messages::{Event, Message};

/// Blocking loop that turns crossterm input into messages.
///
/// Ported from eilmeldung, which runs this on `spawn_blocking` so terminal
/// polling never stalls the async application loop. Exits once the receiving
/// end is dropped.
pub fn input_reader(message_sender: UnboundedSender<Message>) -> color_eyre::Result<()> {
    info!("starting input reader loop");
    loop {
        if !event::poll(Duration::from_millis(100))? {
            if message_sender.is_closed() {
                return Ok(());
            }
            continue;
        }

        match event::read()? {
            event::Event::Key(key_event) => {
                trace!("key event: {key_event:?}");
                message_sender.send(Message::Event(Event::Key(key_event)))?;
            }
            event::Event::Resize(width, height) => {
                trace!("resized to {width} {height}");
                message_sender.send(Message::Event(Event::Resized(width, height)))?;
            }
            event::Event::Mouse(mouse_event) => {
                // Moved events flood the queue and nothing consumes them.
                if !matches!(mouse_event.kind, MouseEventKind::Moved) {
                    trace!(
                        "mouse {:?} at ({}, {})",
                        mouse_event.kind, mouse_event.column, mouse_event.row
                    );
                    message_sender.send(Message::Event(Event::Mouse(mouse_event)))?;
                }
            }
            other => trace!("ignoring event {other:?}"),
        }
    }
}
