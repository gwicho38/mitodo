// Phase 1 builds the store ahead of the TUI that consumes it, so parts of this
// API have no caller yet. Remove once the UI lands in phase 2.
#![allow(dead_code, unused_imports)]

pub mod model;
pub mod parse;

pub use model::{Group, Item, ItemId, Priority};
pub use parse::parse_todo_file;
