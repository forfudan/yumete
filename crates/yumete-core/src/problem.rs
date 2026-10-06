//! **What a language server says is wrong** — LSP, first step (#53/#54,
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
//! Warning: **A complaint is not a file's truth, it is a server's opinion**, and it
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
    /// Warning: **The name is the whole point.** LSP counts in UTF-16, which for
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

/// **A character offset into `line`, as a UTF-16 offset** — the other way.
///
/// What [`char_column`] undoes. Asking a server about a position means saying
/// it in the server's units, and the cursor is in characters.
pub fn utf16_column(line: &str, chars: usize) -> usize {
    line.chars().take(chars).map(char::len_utf16).sum()
}

/// Every server's complaints, by the file they are about.
#[derive(Debug, Default)]
pub struct Problems {
    /// **每一個服務器各存各的**，鍵是（檔，哪一個服務器）——#425，2026-09-30。
    ///
    /// Warning: **`publishDiagnostics` 是「這個檔此刻的全部真相，由我說」。**
    /// 「由我說」那一半從前沒記：一個檔只存一份，於是一種語言跑兩個服務器的時
    /// 候，ruff 推一次抹掉 pylsp 說的，pylsp 推一次抹掉 ruff 說的，屏幕上永遠
    /// 只剩最後推的那一個。
    ///
    /// Warning: **不是 [`Problem::source`]。** 那一格是服務器自己說這一條是誰發
    /// 現的（`rustc`/`clippy`），一個服務器報得出好幾種；這裏的鍵是**哪一個服
    /// 務器**，而「整份替換」要按它來。
    by_source: HashMap<(PathBuf, String), Vec<Problem>>,
    /// 併起來的那一份，畫的時候讀它。由 `by_source` 算出來，沒有人單獨改它。
    by_file: HashMap<PathBuf, Vec<Problem>>,
}

impl Problems {
    /// Replace everything `whose` said about `path`.
    ///
    /// Warning: **Replace, never merge — but only that server's share.**
    /// `publishDiagnostics` is the whole truth about a file each time it
    /// arrives; merging would leave a fixed error on the page for as long as
    /// the session lasted. Warning: 而**別家說的一個字都不動**，見 `by_source`。
    pub fn set(&mut self, path: PathBuf, whose: String, said: Vec<Problem>) {
        match said.is_empty() {
            true => {
                self.by_source.remove(&(path.clone(), whose));
            }
            false => {
                self.by_source.insert((path.clone(), whose), said);
            }
        }
        self.gather(&path);
    }

    /// 把這個檔上各家說的併成一份——畫的時候讀的就是它。
    fn gather(&mut self, path: &Path) {
        let mut all: Vec<Problem> = self
            .by_source
            .iter()
            .filter(|((file, _), _)| file == path)
            .flat_map(|(_, said)| said.iter().cloned())
            .collect();
        all.sort_by_key(|p| (p.line, p.utf16_column));
        match all.is_empty() {
            true => {
                self.by_file.remove(path);
            }
            false => {
                self.by_file.insert(path.to_path_buf(), all);
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

    /// Every file that has something to say, and how much — for `:diagnostics-all`
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
        self.by_source.retain(|(file, _), _| file != path);
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
        all.set(file.clone(), "test".into(), vec![at(3, Severity::Warn), at(3, Severity::Error), at(9, Severity::Hint)]);
        assert_eq!(all.worst_on(&file, 3), Some(Severity::Error));
        assert_eq!(all.worst_on(&file, 9), Some(Severity::Hint));
        assert_eq!(all.worst_on(&file, 4), None);
        assert_eq!(all.count(), 3);
    }

    /// Warning: **Each list replaces the last.** A server that has nothing more to
    /// say says so with an empty list, and a fixed error must leave the page.
    #[test]
    fn an_empty_list_clears_the_file() {
        let mut all = Problems::default();
        let file = PathBuf::from("a.rs");
        all.set(file.clone(), "test".into(), vec![at(1, Severity::Error)]);
        assert_eq!(all.of(&file).len(), 1);
        all.set(file.clone(), "test".into(), Vec::new());
        assert!(all.of(&file).is_empty(), "a fixed error leaves the page");
        assert_eq!(all.files().len(), 0);
    }

    /// Warning: **UTF-16 is not characters**, and the difference only shows on
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

    /// Warning: 兩個方向要對得上，否則問出去的位置和畫回來的位置差一截。
    #[test]
    fn the_two_coordinate_systems_are_each_others_undoing() {
        for line in ["let x = 1;", "第一章：開始", "𠀀甲乙", "a𠀀b"] {
            for chars in 0..=line.chars().count() {
                assert_eq!(
                    char_column(line, utf16_column(line, chars)),
                    chars,
                    "{line:?} 的第 {chars} 個字"
                );
            }
        }
    }

    /// **兩個服務器說的話不互相抹掉**（#425，2026-09-30）。
    ///
    /// Warning: **`publishDiagnostics` 是「這個檔此刻的全部真相，由我說」。**
    /// 「由我說」那一半從前沒記——一個檔只存一份，於是 ruff 推一次抹掉 pylsp 說
    /// 的，pylsp 推一次抹掉 ruff 說的，屏幕上永遠只剩最後推的那一個。這一條就
    /// 是釘那個。
    #[test]
    fn two_servers_do_not_erase_each_other() {
        let mut all = Problems::default();
        let file = PathBuf::from("a.py");
        all.set(file.clone(), "ruff".into(), vec![at(1, Severity::Warn)]);
        all.set(file.clone(), "pylsp".into(), vec![at(5, Severity::Error)]);
        assert_eq!(all.of(&file).len(), 2, "兩家的都在：{:?}", all.of(&file));
        assert_eq!(all.of(&file)[0].line, 1);
        assert_eq!(all.of(&file)[1].line, 5, "併起來還按行排");

        // ruff 那一行改好了：它推一份空的，**只**抹掉自己說的。
        all.set(file.clone(), "ruff".into(), Vec::new());
        assert_eq!(all.of(&file).len(), 1, "pylsp 說的還在");
        assert_eq!(all.of(&file)[0].line, 5);

        // 同一個服務器再推一次，是整份替換，不是加上去。
        all.set(file.clone(), "pylsp".into(), vec![at(7, Severity::Error)]);
        assert_eq!(all.of(&file).len(), 1, "替換，不是累加");
        assert_eq!(all.of(&file)[0].line, 7);

        // 兩家都空了，這個檔就從單子上下去。
        all.set(file.clone(), "pylsp".into(), Vec::new());
        assert!(all.of(&file).is_empty());
        assert_eq!(all.count(), 0, "連計數也歸零");

        // 關掉一個檔，兩家存的都清乾淨——不然換個檔再開回來，舊的又冒出來。
        all.set(file.clone(), "ruff".into(), vec![at(1, Severity::Warn)]);
        all.set(file.clone(), "pylsp".into(), vec![at(2, Severity::Warn)]);
        all.forget(&file);
        assert!(all.of(&file).is_empty());
        all.set(file.clone(), "ruff".into(), Vec::new());
        assert!(all.of(&file).is_empty(), "忘乾淨了，不會借屍還魂");
    }

    #[test]
    fn complaints_come_back_in_reading_order() {
        let mut all = Problems::default();
        let file = PathBuf::from("a.rs");
        all.set(
            file.clone(),
            "test".into(),
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
