//! **What a language server says is wrong** — LSP, first step (#53／#54,
//! 2026-09-20).
//!
//! 2026-09-19：「yumete 太好了，如果能編程就更好」——說這話的人寫 go 和 rust。
//! The four steps are in `development.md` §5.4; this is the first,
//! and it is the one that is useful on its own: **a server's complaints, on
//! the page**. No jumping, no hovering, no completion yet.
//!
//! ## Why the types live here and the socket does not
//!
//! This crate is the editor: it holds what is true about the document. A
//! language server is a *process*, and processes belong to the front end,
//! which already owns the event loop and the one place a thread may talk to
//! it. So the shape of a complaint lives here and the JSON-RPC lives there —
//! the same division the IME and the reading table already use (a trait here,
//! the data over there).
//!
//! ⚠️ **A complaint is not a file's truth, it is a server's opinion**, and it
//! goes stale the moment a key is pressed. Everything here is keyed by the
//! path it arrived for, so a stale list belongs to a file nobody is looking at
//! rather than to the wrong lines of the file in front of you.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How loud a complaint is — LSP's four, in LSP's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// 4 — 「you might like to know」.
    Hint,
    /// 3 — information.
    Note,
    /// 2 — a warning.
    Warn,
    /// 1 — an error. **The loudest wins a line**, which is why this is last.
    Error,
}

impl Severity {
    /// From LSP's number. Anything else is a hint: a server that invents a
    /// severity is not a reason to drop what it said.
    pub fn from_lsp(n: i64) -> Severity {
        match n {
            1 => Severity::Error,
            2 => Severity::Warn,
            3 => Severity::Note,
            _ => Severity::Hint,
        }
    }
}

/// One thing a server said about one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Where it starts, in **characters** — never LSP's UTF-16 units. The
    /// front end converts on the way in; nothing downstream should ever see
    /// the other coordinate system. ⚠️ Getting this wrong puts every mark in a
    /// Chinese file in the wrong column, and silently.
    pub line: usize,
    pub column: usize,
    pub severity: Severity,
    pub message: String,
    /// Which server said it (`rust-analyzer`, `gopls`), when it says.
    pub source: Option<String>,
}

/// Every server's complaints, by the file they are about.
#[derive(Debug, Default)]
pub struct Problems {
    by_file: HashMap<PathBuf, Vec<Problem>>,
}

impl Problems {
    /// Replace everything said about `path`.
    ///
    /// ⚠️ **Replace, never merge.** `publishDiagnostics` is the whole truth
    /// about a file each time it arrives; merging would leave a fixed error on
    /// the page for as long as the session lasted.
    pub fn set(&mut self, path: PathBuf, mut said: Vec<Problem>) {
        said.sort_by_key(|p| (p.line, p.column));
        match said.is_empty() {
            true => {
                self.by_file.remove(&path);
            }
            false => {
                self.by_file.insert(path, said);
            }
        }
    }

    /// What was said about `path`.
    pub fn of(&self, path: &Path) -> &[Problem] {
        self.by_file.get(path).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The loudest complaint on one line of `path`, for the gutter.
    pub fn worst_on(&self, path: &Path, line: usize) -> Option<Severity> {
        self.of(path).iter().filter(|p| p.line == line).map(|p| p.severity).max()
    }

    /// Every file that has something to say, and how much — for `:check-code`
    /// when the cursor is not in any of them.
    pub fn files(&self) -> Vec<(&Path, usize)> {
        let mut out: Vec<(&Path, usize)> =
            self.by_file.iter().map(|(p, said)| (p.as_path(), said.len())).collect();
        out.sort_by_key(|(p, _)| p.to_path_buf());
        out
    }

    /// How many complaints there are altogether.
    pub fn count(&self) -> usize {
        self.by_file.values().map(Vec::len).sum()
    }

    /// Forget a file — what a server sends when it stops watching one, and
    /// what happens when the last server for a language goes away.
    pub fn forget(&mut self, path: &Path) {
        self.by_file.remove(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: usize, severity: Severity) -> Problem {
        Problem { line, column: 0, severity, message: "…".into(), source: None }
    }

    #[test]
    fn the_loudest_complaint_wins_a_line() {
        let mut all = Problems::default();
        let file = PathBuf::from("a.rs");
        all.set(file.clone(), vec![at(3, Severity::Warn), at(3, Severity::Error), at(9, Severity::Hint)]);
        assert_eq!(all.worst_on(&file, 3), Some(Severity::Error));
        assert_eq!(all.worst_on(&file, 9), Some(Severity::Hint));
        assert_eq!(all.worst_on(&file, 4), None);
        assert_eq!(all.count(), 3);
    }

    /// ⚠️ **Each list replaces the last.** A server that has nothing more to
    /// say says so with an empty list, and a fixed error must leave the page.
    #[test]
    fn an_empty_list_clears_the_file() {
        let mut all = Problems::default();
        let file = PathBuf::from("a.rs");
        all.set(file.clone(), vec![at(1, Severity::Error)]);
        assert_eq!(all.of(&file).len(), 1);
        all.set(file.clone(), Vec::new());
        assert!(all.of(&file).is_empty(), "a fixed error leaves the page");
        assert_eq!(all.files().len(), 0);
    }

    #[test]
    fn complaints_come_back_in_reading_order() {
        let mut all = Problems::default();
        let file = PathBuf::from("a.rs");
        all.set(
            file.clone(),
            vec![
                Problem { line: 9, column: 2, ..at(9, Severity::Warn) },
                Problem { line: 1, column: 7, ..at(1, Severity::Warn) },
                Problem { line: 1, column: 2, ..at(1, Severity::Warn) },
            ],
        );
        let places: Vec<(usize, usize)> = all.of(&file).iter().map(|p| (p.line, p.column)).collect();
        assert_eq!(places, [(1, 2), (1, 7), (9, 2)]);
    }
}
