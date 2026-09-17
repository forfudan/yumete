//! Where the fences are, and their code in colour (#420).
//!
//! The block scan already says which lines are code; what it does not say is
//! where one fence ends and the next begins, or what language each is in. This
//! reads that off the fence lines themselves, once per revision, and parses each
//! fence's body once per *content* — an edit in chapter three does not reparse
//! the snippet in chapter one, only find it again.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use super::*;
use crate::code::Language;
use crate::markdown::{Block, Span};

/// A fence longer than this is drawn in the fence's one colour. A snippet in a
/// manuscript is dozens of lines; a pasted data file is not something anyone
/// reads by its colours, and parsing it on every edit is not free.
const LONGEST: usize = 5_000;

/// Enough parsed fences to cover several open chapters without growing for as
/// long as the session runs.
const KEPT: usize = 256;

/// One fence: the line that opens it, the line that closes it if anything
/// does, and the language its info string names.
#[derive(Debug, Clone, Copy)]
pub(super) struct Fence {
    open: usize,
    close: Option<usize>,
    language: Option<Language>,
}

type Lines = Rc<Vec<Vec<Span>>>;

#[derive(Default)]
pub(super) struct CodeCache {
    /// Buffer, revision and syntax the fences were read for.
    key: Option<(u64, u64, u8)>,
    fences: Rc<Vec<Fence>>,
    /// This revision's answers, by opening line.
    by_open: HashMap<usize, Lines>,
    /// Every revision's answers, by what the fence says.
    by_text: HashMap<u64, Lines>,
}

impl Editor {
    /// Whether fenced code is coloured by its grammar (`:view-code`).
    pub fn code_colours(&self) -> bool {
        self.code_colours
    }

    /// Colour fenced code by its grammar, or draw it in the fence's one colour.
    pub fn set_code_colours(&mut self, on: bool) {
        self.code_colours = on;
    }

    /// The coloured runs of code on `line`, if it is a line inside a fence
    /// whose language this build can parse. Empty otherwise — including on the
    /// fence lines themselves, which are Markdown's, not the code's.
    pub(super) fn code_line(&self, line: usize) -> Vec<Span> {
        if !self.code_colours || !self.markup_visible() {
            return Vec::new();
        }
        let fences = self.fences();
        let at = fences.partition_point(|f| f.open < line);
        let Some(fence) = at.checked_sub(1).map(|i| fences[i]) else {
            return Vec::new();
        };
        if fence.close.is_some_and(|close| line >= close) {
            return Vec::new();
        }
        let Some(language) = fence.language else {
            return Vec::new();
        };
        let end = fence.close.unwrap_or(self.current_buffer().rope().len_lines());
        let body = self.parsed(fence.open, fence.open + 1..end, language);
        body.get(line - fence.open - 1).cloned().unwrap_or_default()
    }

    /// The coloured runs of `line` in a file that is code from top to bottom.
    pub(super) fn code_file_line(&self, line: usize, language: Language) -> Vec<Span> {
        if !self.code_colours {
            return Vec::new();
        }
        // Read for the key's sake: a new revision clears the per-revision
        // answers, and the whole file is one of them.
        self.fences();
        let lines = self.current_buffer().rope().len_lines();
        let body = self.parsed(usize::MAX, 0..lines, language);
        body.get(line).cloned().unwrap_or_default()
    }

    /// The parsed runs of `range`, from this revision's cache (under `key`),
    /// the content cache, or the parser — in that order.
    fn parsed(&self, key: usize, range: std::ops::Range<usize>, language: Language) -> Lines {
        if let Some(found) = self.code_cache.borrow().by_open.get(&key) {
            return found.clone();
        }
        let rope = self.current_buffer().rope();
        let lines: Vec<String> = range
            .map(|l| {
                let mut text = rope.line(l).to_string();
                while text.ends_with('\n') || text.ends_with('\r') {
                    text.pop();
                }
                text
            })
            .collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (language, &lines).hash(&mut hasher);
        let text = hasher.finish();

        let mut cache = self.code_cache.borrow_mut();
        let body = match cache.by_text.get(&text) {
            Some(found) => found.clone(),
            None => {
                let body: Lines = Rc::new(match lines.len() > LONGEST {
                    true => vec![Vec::new(); lines.len()],
                    false => crate::code::highlight(language, &lines),
                });
                if cache.by_text.len() >= KEPT {
                    cache.by_text.clear();
                }
                cache.by_text.insert(text, body.clone());
                body
            }
        };
        cache.by_open.insert(key, body.clone());
        body
    }

    /// Every fence in the buffer, in order — read once per revision.
    fn fences(&self) -> Rc<Vec<Fence>> {
        let buffer = self.current_buffer();
        let key = (buffer.id(), buffer.revision(), buffer.syntax().tag());
        if self.code_cache.borrow().key == Some(key) {
            return self.code_cache.borrow().fences.clone();
        }
        let rope = buffer.rope();
        let mut fences: Vec<Fence> = Vec::new();
        // The character that opened the fence being walked, if one is.
        let mut open: Option<char> = None;
        for line in 0..rope.len_lines() {
            if !matches!(self.block_of(line), Block::Code { .. }) {
                // A fence the scan says ended — at a conflict, say — ended.
                open = None;
                continue;
            }
            let head: String = rope.line(line).chars().take(crate::markdown::PREFIX).collect();
            let trimmed = head.trim_start();
            let mark = ['`', '~']
                .into_iter()
                .find(|&c| trimmed.starts_with(&c.to_string().repeat(3)));
            match (open, mark) {
                // ⚠️ **Only a fence opens a fence.** Typst's scan calls a
                // `#show` body code too, and reading its first line as an info
                // string would parse `json("a.json")` as JSON.
                (None, Some(mark)) => {
                    let info = trimmed.trim_start_matches(mark);
                    fences.push(Fence {
                        open: line,
                        close: None,
                        language: Language::from_info(info),
                    });
                    open = Some(mark);
                }
                (Some(opened), Some(closing)) if opened == closing => {
                    if let Some(last) = fences.last_mut() {
                        last.close = Some(line);
                    }
                    open = None;
                }
                _ => {}
            }
        }
        let mut cache = self.code_cache.borrow_mut();
        cache.key = Some(key);
        cache.fences = Rc::new(fences);
        cache.by_open.clear();
        cache.fences.clone()
    }
}
