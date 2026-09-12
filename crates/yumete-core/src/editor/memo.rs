//! One answer per line, kept against the one thing that decides whether it is
//! still true (#348).
//!
//! Five things on the page are worked out a line at a time and asked for again
//! every frame: the words in it, its 平仄, the marks it got wrong, its Markdown
//! runs, and the readings laid over it. Each one used to keep its own map, and
//! each map decided for itself what went in the key, how big it was allowed to
//! get and when it was thrown away — so three of them said 「line 3」 without
//! saying which file's line 3, and two of them had no bound at all and grew an
//! entry per line for as long as the reader kept scrolling.
//!
//! This is that decision made once. What a caller still says for itself is the
//! **stamp**: the one value that changes when the answer would. Two are in use
//! and both are right for what they cover —
//!
//! * a hash of the line's own text, when reading the line is cheap beside
//!   working the answer out (an edit then costs only the line being typed
//!   into, not the forty on screen); and
//! * the buffer's revision, when reading the line is *not* cheap (a chapter
//!   written as one 500 000-character paragraph was hashed once per 縱 of every
//!   frame, #315) — plus whatever else the answer was read under, such as the
//!   syntax or the dialects.
//!
//! What is deliberately **not** here: the caches that are not per line.
//! [`crate::editor::FoldMap`] and the block map are one answer for the whole
//! document, the padding is one table at a time, and `crate::wrap` keeps eight
//! paragraphs by text and width in a list it moves to the front — an LRU, not a
//! line map, and a paragraph is not a line once it wraps.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// How many lines' answers one memo keeps.
///
/// A page is tens of lines; the bound only exists so that scrolling a long
/// document does not end up holding an entry per line in it. Full means
/// emptied — an eviction order would have to be kept up to date on every read,
/// and the next frame asks about the lines it is drawing anyway.
pub(crate) const MEMO_LINES: usize = 512;

/// The one value a caller says its answer was worked out under.
pub(crate) fn stamp(of: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    of.hash(&mut hasher);
    hasher.finish()
}

/// Answers already worked out, one per line of one buffer.
#[derive(Debug)]
pub(crate) struct LineMemo<T> {
    answers: RefCell<HashMap<(u64, usize), (u64, T)>>,
}

impl<T> Default for LineMemo<T> {
    fn default() -> Self {
        Self {
            answers: RefCell::new(HashMap::new()),
        }
    }
}

impl<T: Clone> LineMemo<T> {
    /// The answer for `line` of `buffer` under `stamp`, working it out if what
    /// is kept was worked out under something else.
    ///
    /// **The memo is not borrowed while `work` runs.** It is looked at, let go
    /// of, and only borrowed again to keep what came back — so a `work` that
    /// asks this same memo something gets a wrong-but-live answer rather than a
    /// panic in the middle of a frame.
    pub(crate) fn or_work_out(
        &self,
        buffer: u64,
        line: usize,
        stamp: u64,
        work: impl FnOnce() -> T,
    ) -> T {
        if let Some((kept, answer)) = self.answers.borrow().get(&(buffer, line)) {
            if *kept == stamp {
                return answer.clone();
            }
        }
        let answer = work();
        let mut answers = self.answers.borrow_mut();
        if answers.len() >= MEMO_LINES {
            answers.clear();
        }
        answers.insert((buffer, line), (stamp, answer.clone()));
        answer
    }

    /// Throw everything away — for when what changed is not in any stamp.
    pub(crate) fn forget(&self) {
        self.answers.borrow_mut().clear();
    }

    /// How many lines are remembered. For the tests that hold this to its bound.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.answers.borrow().len()
    }
}
