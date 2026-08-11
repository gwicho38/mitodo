// Phase 1 builds the store ahead of the TUI that consumes it, so parts of this
// API have no caller yet. Remove once the UI lands in phase 2.
#![allow(dead_code, unused_imports)]

pub mod archive;
pub mod detect;
pub mod model;
pub mod parse;
pub mod watch;
pub mod workspace;
pub mod write;

pub use archive::{ArchiveReport, archive_done, archive_items};
pub use detect::{Detection, detect};
pub use model::{Group, Item, ItemId, Priority};
pub use parse::parse_todo_file;
pub use workspace::Workspace;
pub use write::{
    WriteError, add_item, create_item, delete_item, edit_text, set_description, toggle,
};
