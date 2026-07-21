//! `yumete-core` — the editor core for **yumete** (the Yuhao IME text editor).
//!
//! This crate holds the pure, UI-independent editing logic, mirroring Helix's
//! separation of a rope/selection/transaction core from the TUI. It has no
//! terminal or IME dependencies.
//!
//! Feature #1 (open file / new buffer) lives here:
//!
//! - [`Buffer`] wraps the text store for a single document.
//! - [`Editor`] owns the list of open buffers and the active one.
//! - [`command`] parses the `:open` / `:new` command line.
//!
//! All text access goes through the [`TextStore`] trait so the storage backend
//! (currently a [`ropey`] rope) can be swapped later without touching call sites.

pub mod buffer;
pub mod command;
pub mod editor;
pub mod text_store;

pub use buffer::Buffer;
pub use command::{Command, CommandError};
pub use editor::{CommandOutcome, Editor, EditorError};
pub use text_store::TextStore;
