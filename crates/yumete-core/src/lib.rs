//! `yumete-core` — the editor core for **yumete** (Yume + TE, the terminal
//! editor with the Yume IME in it).
//!
//! This crate holds the pure, UI-independent editing logic, mirroring Helix's
//! separation of a rope/selection/transaction core from the TUI. It has no
//! terminal or IME dependencies.
//!
//! Feature #1 (open file / new buffer) lives here, alongside the modal editing
//! core (Feature #5):
//!
//! - [`Buffer`] wraps the text store for a single document.
//! - [`Editor`] owns the open buffers, the cursor, the mode, and the command
//!   line; [`Editor::on_key`] drives the Normal / Insert / Command state machine.
//! - [`command`] parses the `:` command line; [`motion`] holds the grapheme- and
//!   width-aware cursor motions.
//!
//! All text access goes through the [`TextStore`] trait so the storage backend
//! (currently a [`ropey`] rope) can be swapped later without touching call sites.

pub mod buffer;
pub mod clock;
pub mod code;
pub mod command;
pub mod comment;
pub mod conflict;
pub mod convert;
pub mod diag;
pub mod diff;
pub mod discover;
pub mod drawn;
pub mod editor;
pub mod export;
pub mod input;
pub mod markdown;
pub mod lookfor;
pub mod messages;
pub mod mdtable;
pub mod meter;
pub mod motion;
pub mod picker;
pub mod progress;
pub mod punct;
pub mod ruby;
pub mod search_panel;
pub mod sidebar;
pub mod table;
pub mod syntax;
pub mod text_store;
pub mod tutor;
pub mod usage;
pub mod words;
pub mod wrap;
pub mod zong;

pub use buffer::Buffer;
pub use command::{Command, CommandError};
pub use editor::{CommandOutcome, Editor, EditorError, KeyOutcome, ShotJob};
pub use input::{Key, Mode};
pub use text_store::TextStore;

// `Buffer::rope` already hands one out, so the type belongs in the public API
// rather than making every caller depend on `ropey` directly.
pub use ropey::Rope;

// Re-exported so binaries can install a segmenter without a direct dependency
// on `yumete-cjk`.
pub use yumete_cjk::{set_ambiguous_wide, CategorySegmenter, DictionarySegmenter, Segmenter};
