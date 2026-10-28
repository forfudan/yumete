//! Words and paths offered while writing (2026-10-28).
//!
//! Two requests from writers: 「德語、英語的單詞太長了，最好打兩個字母，就會在補全
//! 裏跳出當前文件裏之前打過的單詞」, and 「Markdown 和代碼中的 `[]()` 語法中，如果
//! 是本地路徑，最好也能補齊」. Both are what helix does out of the box (its word
//! completion and path completion), and both reuse the list code completion
//! already has: the float under the caret, Tab, `C-n`/`C-p`, Esc.
//!
//! - **Words** come from every open buffer. Latin words as helix indexes them
//!   (3 to 50 letters): those that start with what was typed first, in the
//!   case typed, then those that hold its letters in order; 漢字 words as
//!   the segmenter cuts them, offered only when longer than what was typed
//!   and starting with it, so what comes up is a name or a phrase of the
//!   project, not one more character.
//! - **When**: after `[editor] word_completion` Latin letters (4 by default,
//!   `0` off), or after ⌈N/2⌉ 漢字 — the rule as set on 2026-10-28: N = 3 or 4
//!   gives 2, N = 1 or 2 gives 1. `C-n` / `C-x` asks at any length.
//! - **Paths**: a word with a `/` in it, or anything right after `](`, read
//!   relative to the file's own folder — helix's rule, plus the Markdown link.
//!
//! 漢字 only reach here once committed: while the IME is composing, keys are
//! the IME's, and this list never shares the screen with its candidates.

use super::*;

/// **`word` as a completion of `typed`, in the case typed** (2026-10-28), or
/// `None` when it does not start with `typed` (case aside).
///
/// vim's `'infercase'`: a lowercase letter typed makes the rest lowercase, so
/// `w` gives `when` even when the buffers only hold `When`; two or more
/// letters typed with no lowercase among them make it uppercase, `WH` →
/// `WHEN`; a lone `W` gives `When`. One change to vim's rule, asked for on the
/// same day: **a word with a capital after its first letter is a name and is
/// offered as written** — `java` gives `JavaScript`, `ht` gives `HTML`.
fn in_the_case_typed(word: &str, typed: &str) -> Option<String> {
    let mut letters = word.chars();
    for want in typed.chars() {
        let have = letters.next()?;
        if !have.to_lowercase().eq(want.to_lowercase()) {
            return None;
        }
    }
    let rest: String = letters.collect();
    if word.chars().skip(1).any(char::is_uppercase) {
        return Some(word.to_string());
    }
    let shouting = typed.chars().count() >= 2
        && typed.chars().any(char::is_uppercase)
        && !typed.chars().any(char::is_lowercase);
    Some(match shouting {
        true => format!("{typed}{}", rest.to_uppercase()),
        false => format!("{typed}{rest}"),
    })
}

/// Words shorter than this are not worth offering (helix's `MIN_WORD_GRAPHEMES`).
const SHORTEST: usize = 3;
/// Nor longer than this (helix's `MAX_WORD_LEN`): a run that long is a hash or
/// a URL, not a word anybody types twice.
const LONGEST: usize = 50;
/// How many the list holds.
const MOST: usize = 50;
/// How far back on the line a word or a path is looked for.
const LOOK_BACK: usize = 200;

/// One buffer's words, kept up to date a line at a time: which lines it has
/// (by hash, with how many of each), and the words those lines hold.
#[derive(Default)]
pub(crate) struct WordIndex {
    revision: Option<u64>,
    lines: HashMap<u64, u32>,
    latin: HashMap<String, u32>,
    han: HashMap<String, u32>,
}

/// The words of one line: Latin, then 漢字. Kept by the line's hash, so an
/// unchanged line is never cut twice, in whichever buffer it is.
pub(crate) type LineWords = (Vec<String>, Vec<String>);

fn latin_letter(c: char) -> bool {
    (c.is_alphanumeric() || c == '_') && !yumete_cjk::is_han(c)
}

impl Editor {
    /// How many Latin letters before the list opens by itself; `0` never.
    pub fn word_completion(&self) -> usize {
        self.word_completion
    }

    /// Set it (`[editor] word_completion`).
    pub fn set_word_completion(&mut self, letters: usize) {
        self.word_completion = letters;
    }

    /// **Offer words or a path at the caret**, if there is anything to offer.
    /// Whether an offering now stands. `by_hand` is `C-n` / `C-x`: any length,
    /// and even with the setting at `0`.
    pub(super) fn offer_from_the_text(&mut self, by_hand: bool) -> bool {
        if self.mode != Mode::Insert {
            return false;
        }
        let items = match self.path_offers() {
            Some(items) => items,
            None if self.writes_code() => return false,
            None => match self.word_offers(by_hand) {
                Some(items) => items,
                None => return false,
            },
        };
        self.offering = Some(Offering {
            buffer: self.current_buffer().id(),
            at: self.sel.head(),
            items,
            picked: 0,
            server: None,
        });
        true
    }

    /// The line up to the caret, at most [`LOOK_BACK`] characters of it, and
    /// the line's number.
    fn before_the_caret(&self) -> (usize, Vec<char>) {
        let rope = self.current_buffer().rope();
        let head = self.sel.head().min(rope.len_chars());
        let line = rope.char_to_line(head);
        let from = rope.line_to_char(line).max(head.saturating_sub(LOOK_BACK));
        (line, rope.slice(from..head).chars().collect())
    }

    /// An offer that replaces the last `typed` characters before the caret.
    fn offer_over(&self, line: usize, typed: usize, label: String) -> crate::lsp::Offer {
        let rope = self.current_buffer().rope();
        let head = self.sel.head().min(rope.len_chars());
        let text = crate::motion::line_text(rope, line);
        let to = head - rope.line_to_char(line);
        crate::lsp::Offer {
            insert: label.clone(),
            label,
            kind: 0,
            detail: None,
            filter: None,
            replacing: Some(crate::lsp::Replacing {
                line,
                from: crate::problem::utf16_column(&text, to - typed),
                to: crate::problem::utf16_column(&text, to),
            }),
        }
    }

    /// The files and folders a path before the caret can go on to.
    fn path_offers(&self) -> Option<Vec<crate::lsp::Offer>> {
        let (line, before) = self.before_the_caret();
        let stops = |c: char| c.is_whitespace() || "\"'`()[]<>{}=,;|".contains(c);
        let start = before.iter().rposition(|&c| stops(c)).map_or(0, |i| i + 1);
        let token: String = before[start..].iter().collect();
        let in_a_link = start >= 2 && before[start - 2..start] == [']', '('];
        if token.contains("://") || token.starts_with('#') {
            return None;
        }
        if !token.contains('/') && !(in_a_link && !token.is_empty()) {
            return None;
        }
        let (dir, base) = match token.rfind('/') {
            Some(i) => (&token[..=i], &token[i + 1..]),
            None => ("", token.as_str()),
        };
        // Relative to the file's own folder, as a link in it is read; a file
        // with no name yet has the working directory.
        let here = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| self.working_dir());
        let folder = match dir {
            "" => here,
            dir if dir.starts_with('~') || dir.starts_with('/') => Self::expand_tilde(dir),
            dir => here.join(dir),
        };
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .ok()?
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let shown = base.starts_with('.') || !name.starts_with('.');
                (shown && name.starts_with(base) && name != base).then(|| {
                    let slash = if entry.path().is_dir() { "/" } else { "" };
                    format!("{name}{slash}")
                })
            })
            .collect();
        names.sort();
        names.truncate(MOST);
        let typed = base.chars().count();
        let items: Vec<_> = names.into_iter().map(|n| self.offer_over(line, typed, n)).collect();
        (!items.is_empty()).then_some(items)
    }

    /// The words of the open buffers the word before the caret can become.
    fn word_offers(&mut self, by_hand: bool) -> Option<Vec<crate::lsp::Offer>> {
        let n = self.word_completion;
        if n == 0 && !by_hand {
            return None;
        }
        let (line, before) = self.before_the_caret();
        let last = *before.last()?;
        // What follows the caret on the line, a few characters of it: a word
        // the caret is in the middle of is not one to be offered to itself.
        let after: Vec<char> = {
            let rope = self.current_buffer().rope();
            let head = self.sel.head().min(rope.len_chars());
            rope.chars_at(head).take(LONGEST).take_while(|&c| c != '\n').collect()
        };
        if latin_letter(last) {
            if !by_hand && after.first().is_some_and(|&c| latin_letter(c)) {
                return None;
            }
            let typed: String = {
                let from = before.iter().rposition(|&c| !latin_letter(c)).map_or(0, |i| i + 1);
                before[from..].iter().collect()
            };
            let length = typed.chars().count();
            if length == 0 || (!by_hand && length < n) {
                return None;
            }
            self.read_the_open_buffers();
            // **Completion first, fuzzy after** (2026-10-28). A word that starts
            // with what was typed is offered in the case typed (see
            // [`in_the_case_typed`]), so `When` and `when` are one row; then the
            // words that only hold the letters in order, as written.
            // Shown → (fuzzy?, score, how often).
            let mut found: HashMap<String, (bool, i64, u32)> = HashMap::new();
            for index in self.word_index.values() {
                for (word, &count) in &index.latin {
                    let (shown, fuzzy, score) = match in_the_case_typed(word, &typed) {
                        Some(shown) => (shown, false, 0),
                        None => match super::modes::in_order(word, &typed) {
                            Some(score) => (word.clone(), true, score),
                            None => continue,
                        },
                    };
                    if shown == typed {
                        continue;
                    }
                    let row = found.entry(shown).or_insert((fuzzy, score, 0));
                    // The same word in two buffers is one word, counted once
                    // per buffer; a completion beats a fuzzy reading of it.
                    if row.0 && !fuzzy {
                        *row = (false, 0, 0);
                    }
                    row.2 += count;
                }
            }
            let mut found: Vec<(String, (bool, i64, u32))> = found.into_iter().collect();
            found.sort_by(|(a, x), (b, y)| {
                x.0.cmp(&y.0)
                    .then(y.1.cmp(&x.1))
                    .then(y.2.cmp(&x.2))
                    .then(a.len().cmp(&b.len()))
                    .then(a.cmp(b))
            });
            let words: Vec<String> = found.into_iter().take(MOST).map(|(w, _)| w).collect();
            let items: Vec<_> = words.into_iter().map(|w| self.offer_over(line, length, w)).collect();
            return (!items.is_empty()).then_some(items);
        }
        if yumete_cjk::is_han(last) {
            // ⌈N/2⌉ 漢字, at least one.
            let k = n.div_ceil(2).max(1);
            let run = before.iter().rev().take_while(|&&c| yumete_cjk::is_han(c)).count();
            let k = match by_hand {
                true => k.min(run),
                false => k,
            };
            if run < k {
                return None;
            }
            let typed: String = before[before.len() - k..].iter().collect();
            self.read_the_open_buffers();
            let mut found: HashMap<&String, u32> = HashMap::new();
            for index in self.word_index.values() {
                for (word, &count) in &index.han {
                    let rest: Vec<char> = word.chars().skip(k).collect();
                    let written = after.len() >= rest.len() && after[..rest.len()] == rest[..];
                    if word.chars().count() > k && word.starts_with(&typed) && !written {
                        *found.entry(word).or_default() += count;
                    }
                }
            }
            let mut found: Vec<(&String, u32)> = found.into_iter().collect();
            found.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.len().cmp(&b.0.len())).then(a.0.cmp(b.0)));
            let words: Vec<String> = found.into_iter().take(MOST).map(|(w, _)| w.clone()).collect();
            let items: Vec<_> = words.into_iter().map(|w| self.offer_over(line, k, w)).collect();
            return (!items.is_empty()).then_some(items);
        }
        None
    }

    /// Bring every open buffer's words up to date. **Only the lines that
    /// changed are read again**: a buffer's lines are hashed (a novel's in a
    /// millisecond or two), and a line whose hash is new is cut once and kept,
    /// so typing into a long book costs the line being typed, not the book.
    fn read_the_open_buffers(&mut self) {
        let open: Vec<u64> = self.buffers.iter().map(|b| b.id()).collect();
        self.word_index.retain(|id, _| open.contains(id));
        for i in 0..self.buffers.len() {
            let (id, revision) = (self.buffers[i].id(), self.buffers[i].revision());
            if self.word_index.get(&id).is_some_and(|index| index.revision == Some(revision)) {
                continue;
            }
            let mut now: HashMap<u64, u32> = HashMap::new();
            for line in self.buffers[i].rope().lines() {
                let text = line.to_string();
                let stamp = super::memo::stamp(&text);
                if !self.line_words.contains_key(&stamp) {
                    let cut = self.words_of_a_line(&text);
                    self.line_words.insert(stamp, cut);
                }
                *now.entry(stamp).or_default() += 1;
            }
            let mut index = self.word_index.remove(&id).unwrap_or_default();
            // What each line hash gained or lost since the last reading.
            let mut changes: Vec<(u64, i64)> = Vec::new();
            for (&stamp, &count) in &now {
                let was = index.lines.get(&stamp).copied().unwrap_or(0);
                if count != was {
                    changes.push((stamp, count as i64 - was as i64));
                }
            }
            for (&stamp, &was) in &index.lines {
                if !now.contains_key(&stamp) {
                    changes.push((stamp, -(was as i64)));
                }
            }
            // A line that left the cache cannot be taken back out: read the
            // buffer whole instead.
            if changes.iter().any(|(stamp, _)| !self.line_words.contains_key(stamp)) {
                index = WordIndex::default();
                changes = now.iter().map(|(&stamp, &count)| (stamp, count as i64)).collect();
            }
            for (stamp, by) in changes {
                let (latin, han) = &self.line_words[&stamp];
                for (words, into) in [(latin, &mut index.latin), (han, &mut index.han)] {
                    for word in words {
                        let count = into.entry(word.clone()).or_default();
                        *count = (*count as i64 + by).max(0) as u32;
                        if *count == 0 {
                            into.remove(word);
                        }
                    }
                }
            }
            index.lines = now;
            index.revision = Some(revision);
            self.word_index.insert(id, index);
        }
        // The cache of cut lines only grows: dropped whole once it is far past
        // what the open buffers hold (the next reading starts over).
        if self.line_words.len() > 400_000 {
            self.line_words.clear();
            self.word_index.clear();
        }
    }

    /// Cut one line into its Latin words and its 漢字 words.
    fn words_of_a_line(&self, text: &str) -> LineWords {
        let chars: Vec<char> = text.chars().collect();
        let mut latin = Vec::new();
        let mut at = 0;
        while at < chars.len() {
            if !latin_letter(chars[at]) {
                at += 1;
                continue;
            }
            let from = at;
            while at < chars.len() && latin_letter(chars[at]) {
                at += 1;
            }
            let length = at - from;
            // A word, not a number.
            if (SHORTEST..=LONGEST).contains(&length) && chars[from..at].iter().any(|c| c.is_alphabetic()) {
                latin.push(chars[from..at].iter().collect());
            }
        }
        let han = match chars.iter().any(|&c| yumete_cjk::is_han(c)) {
            false => Vec::new(),
            true => self
                .segmenter
                .segment(text)
                .into_iter()
                .filter(|&(a, b)| b - a >= 2 && b - a <= LONGEST && chars[a..b].iter().all(|&c| yumete_cjk::is_han(c)))
                .map(|(a, b)| chars[a..b].iter().collect())
                .collect(),
        };
        (latin, han)
    }
}
