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
    /// Which line, 0 起算 — the one number both sides count the same way.
    pub line: usize,
    /// Where it starts **in UTF-16 code units**, exactly as the server said
    /// it.
    ///
    /// ⚠️ **The name is the whole point.** LSP counts in UTF-16, which for
    /// ASCII agrees with bytes and with characters — so a field called
    /// `column` would be right in every English test and wrong down the whole
    /// length of a Chinese line, silently. Turning it into a character offset
    /// needs **that line's text** ([`char_column`]), and the text is exactly
    /// what a complaint about an unopened file does not come with. So it is
    /// carried as what it is, and converted where there is something to
    /// convert it against.
    pub utf16_column: usize,
    pub severity: Severity,
    pub message: String,
    /// Which server said it (`rust-analyzer`, `gopls`), when it says.
    pub source: Option<String>,
}

/// **A UTF-16 offset into `line`, as a character offset.**
///
/// This is the one place the two coordinate systems meet, and it needs the
/// text because that is the only thing that knows which characters are wide:
/// every character outside the Basic Multilingual Plane — 𝄞, an emoji, a rare
/// 漢字 like 𠀀 — counts as **two** in UTF-16 and one here.
///
/// A number past the end of the line comes back as the end of the line: a
/// server counting against a version of the file we have already changed is an
/// everyday event, not an error.
pub fn char_column(line: &str, utf16: usize) -> usize {
    let mut seen = 0;
    for (chars, c) in line.chars().enumerate() {
        if seen >= utf16 {
            return chars;
        }
        seen += c.len_utf16();
    }
    line.chars().count()
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
        said.sort_by_key(|p| (p.line, p.utf16_column));
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
        Problem { line, utf16_column: 0, severity, message: "…".into(), source: None }
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

    /// ⚠️ **UTF-16 is not characters**, and the difference only shows on
    /// exactly the text this editor is for.
    #[test]
    fn a_utf16_offset_becomes_a_character_offset_against_the_line() {
        // 漢字 are one UTF-16 unit each (BMP), so these agree…
        assert_eq!(char_column("第一章：開始", 3), 3);
        // …but a character outside the BMP is two units, and after it every
        // number is one out. 𠀀 is U+20000.
        assert_eq!(char_column("𠀀甲乙", 0), 0);
        assert_eq!(char_column("𠀀甲乙", 2), 1, "the 甲 is the second character");
        assert_eq!(char_column("𠀀甲乙", 3), 2);
        // Plain English is the case where the bug hides.
        assert_eq!(char_column("let x = 1;", 4), 4);
        // Past the end is the end: the server counted against an older file.
        assert_eq!(char_column("ab", 99), 2);
    }

    #[test]
    fn complaints_come_back_in_reading_order() {
        let mut all = Problems::default();
        let file = PathBuf::from("a.rs");
        all.set(
            file.clone(),
            vec![
                Problem { line: 9, utf16_column: 2, ..at(9, Severity::Warn) },
                Problem { line: 1, utf16_column: 7, ..at(1, Severity::Warn) },
                Problem { line: 1, utf16_column: 2, ..at(1, Severity::Warn) },
            ],
        );
        let places: Vec<(usize, usize)> = all.of(&file).iter().map(|p| (p.line, p.utf16_column)).collect();
        assert_eq!(places, [(1, 2), (1, 7), (9, 2)]);
    }
}
