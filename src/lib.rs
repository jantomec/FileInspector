//! fileinspector: an interactive, system-wide disk usage explorer.
//!
//! The crate is split into three layers:
//! - [`tree`]: the shared arena-based directory tree model and scan events;
//! - `scan`: the filesystem walker that produces a [`tree::Tree`] (owned by the scanner);
//! - [`tui`]: the interactive terminal UI that consumes scan events.

pub mod tree;
pub mod tui;
