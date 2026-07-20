//! A [`Buffer`]: a single editable document — its text plus the file it is
//! associated with. This is the heart of Feature #1 (open file / new buffer).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ropey::Rope;

use crate::text_store::TextStore;

/// A single editable document.
///
/// The text is held in a [`ropey`] rope (behind [`TextStore`]); `path` records
/// the file the buffer is bound to (if any); `modified` tracks unsaved changes
/// for the future dirty-check on quit (Feature #3).
pub struct Buffer {
    rope: Rope,
    path: Option<PathBuf>,
    modified: bool,
}

impl Buffer {
    /// Create a new, empty, unnamed buffer (a "scratch" buffer).
    pub fn scratch() -> Self {
        Buffer {
            rope: Rope::new(),
            path: None,
            modified: false,
        }
    }

    /// Create a buffer holding `text`, not yet associated with any file.
    pub fn from_text(text: &str) -> Self {
        Buffer {
            rope: Rope::from_str(text),
            path: None,
            modified: false,
        }
    }

    /// Open `path` into a new buffer.
    ///
    /// If the file exists, its contents are read into the buffer. If it does
    /// **not** exist, an empty buffer *bound to* `path` is returned, so a later
    /// save creates it — this matches what writers expect from `yumete newfile`.
    /// Any other I/O error (permissions, a directory, invalid UTF-8) is returned.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref();
        let rope = match fs::read_to_string(path) {
            Ok(text) => Rope::from_str(&text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Rope::new(),
            Err(err) => return Err(err),
        };
        Ok(Buffer {
            rope,
            path: Some(path.to_path_buf()),
            modified: false,
        })
    }

    /// The file this buffer is bound to, if any.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Bind this buffer to `path` (used by save-as, Feature #2).
    pub fn set_path<P: Into<PathBuf>>(&mut self, path: P) {
        self.path = Some(path.into());
    }

    /// Whether the buffer has unsaved modifications.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// A short, human-readable name for status lines: the file name, or
    /// `[scratch]` for an unnamed buffer.
    pub fn display_name(&self) -> String {
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned()),
            None => "[scratch]".to_string(),
        }
    }

    /// Read-only access to the underlying rope, for the editing layers to come.
    pub fn rope(&self) -> &Rope {
        &self.rope
    }
}

impl TextStore for Buffer {
    fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    fn char_count(&self) -> usize {
        self.rope.len_chars()
    }

    fn line(&self, index: usize) -> Option<String> {
        if index >= self.rope.len_lines() {
            return None;
        }
        let mut s = self.rope.line(index).to_string();
        // Strip a single trailing line break ("\n" or "\r\n") for display.
        if s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') {
                s.pop();
            }
        }
        Some(s)
    }

    fn text(&self) -> String {
        self.rope.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_buffer_is_empty_and_unnamed() {
        let b = Buffer::scratch();
        assert_eq!(b.path(), None);
        assert_eq!(b.char_count(), 0);
        // An empty document still reports one (empty) line.
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0).as_deref(), Some(""));
        assert_eq!(b.display_name(), "[scratch]");
        assert!(!b.is_modified());
    }

    #[test]
    fn from_text_splits_into_lines_without_trailing_newline() {
        let b = Buffer::from_text("第一行\n第二行\n");
        assert_eq!(b.line(0).as_deref(), Some("第一行"));
        assert_eq!(b.line(1).as_deref(), Some("第二行"));
        assert_eq!(b.text(), "第一行\n第二行\n");
    }
}
