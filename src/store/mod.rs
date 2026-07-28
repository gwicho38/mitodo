// Phase 1 builds the store ahead of the TUI that consumes it, so parts of this
// API have no caller yet. Remove once the UI lands in phase 2.
#![allow(dead_code, unused_imports)]

pub mod model;

pub use model::{Group, Item, ItemId, Priority};
