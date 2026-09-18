//! The picker behind `Space f` and `Space b` — Feature #90.
//!
//! A novel is a hundred files. Cycling through them with `gn` is not a way to
//! reach chapter 63; typing its whole path is not either. Helix's answer is a
//! picker: a list, a line to narrow it with, and Enter. This is that, with the
//! matching kept deliberately plain — a **subsequence** match, scored so that
//! letters found together and letters at the start of a word count for more.
//! Nobody needs a better algorithm to find `ch63` among a hundred chapters, and
//! a scoring function nobody can predict is worse than one that is merely
//! adequate.

/// What a picker offers, and what choosing it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A file to open, by path.
    File(String),
    /// One of the open buffers, by index.
    Buffer(usize, String),
    /// A line of the file being written, by index — what a table's jump offers
    /// when a cell names several rows and only a person can say which.
    Row(usize, String),
    /// Something to paste: which row of the editor's paste menu, and the line
    /// shown for it. `None` is the system clipboard, which only the front end
    /// can read.
    Paste(Option<usize>, String),
}

impl Item {
    /// The text shown in the list, and matched against.
    pub fn label(&self) -> &str {
        match self {
            Item::File(path) => path,
            Item::Buffer(_, name) | Item::Row(_, name) | Item::Paste(_, name) => name,
        }
    }
}

/// An open picker.
#[derive(Debug, Clone)]
pub struct Picker {
    /// What it is picking, for the prompt.
    ///
    /// A `String`, not a `&'static str`: the title is a message like everything
    /// else on the screen, so it arrives already translated.
    pub title: String,
    /// Everything it could offer, in the order it was gathered.
    items: Vec<Item>,
    /// What has been typed to narrow it.
    query: String,
    /// Which of the *matching* items is highlighted.
    selected: usize,
    /// How far into the query the caret is, in characters.
    caret: usize,
    /// **Whether the keys are in the query or in the list** (2026-09-17).
    ///
    /// A picker where typing narrows the list cannot also spend `j` and `k` on
    /// moving through it — `j` is a letter of a file name. So it has two
    /// layers, and **it opens in the list**: 「通過 jklh 什麽的可以在文件樹裏
    /// 移動，也能通過 `/` 搜索文件」 — `jk` walk from the first keystroke, and
    /// `/` (or `i`) is what puts the keys in the query, where `Esc` hands them
    /// back to the list.
    typing: bool,
    /// **What to put near the top before anything is typed**, one number per
    /// item (2026-09-18).
    ///
    /// Alphabetical is chapter order, which is the right answer for a book and
    /// the wrong one for 「the file I was in five minutes ago」 — 137 names and
    /// the one being written is somewhere in the middle. So the caller says
    /// what it knows: an open buffer, the file last opened, the one before it.
    /// A **tie-breaker, not an override** — the numbers are small beside a
    /// match's own score, so typing still decides what matches best, and this
    /// decides which of two equally good matches is offered first.
    bonus: Vec<i64>,
}

/// Where a caret is being asked to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caret {
    Left,
    Right,
    Start,
    End,
}

impl Picker {
    /// Open a picker over `items`.
    pub fn new(title: &str, items: Vec<Item>) -> Picker {
        let count = items.len();
        Picker {
            title: title.to_string(),
            items,
            query: String::new(),
            selected: 0,
            caret: 0,
            typing: false,
            bonus: vec![0; count],
        }
    }

    /// Say what to prefer: one number per item, bigger first.
    ///
    /// Longer or shorter than the items is not a caller mistake worth a panic
    /// — the list is gathered in one place and weighted in another — so it is
    /// padded and truncated to fit.
    pub fn prefer(&mut self, bonus: Vec<i64>) {
        self.bonus = bonus;
        self.bonus.resize(self.items.len(), 0);
    }

    /// What has been typed so far.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// How many items there are in all.
    pub fn total(&self) -> usize {
        self.items.len()
    }

    /// The items matching the query, best first.
    ///
    /// Recomputed on each call rather than cached: a picker holds a few hundred
    /// paths, and being always right about what is on screen is worth more here
    /// than saving a scan.
    pub fn matches(&self) -> Vec<&Item> {
        let mut scored: Vec<(i64, usize, &Item)> = match self.query.is_empty() {
            true => self
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| (self.bonus.get(i).copied().unwrap_or(0), i, item))
                .collect(),
            false => self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    matched(item.label(), &self.query)
                        .map(|(s, _)| (s + self.bonus.get(i).copied().unwrap_or(0), i, item))
                })
                .collect(),
        };
        // Best score first; ties keep the order they were gathered in, which for
        // files is alphabetical and so is chapter order.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, _, item)| item).collect()
    }

    /// **Which characters of `label` the query is standing on**, so the front
    /// end can light them up — the one thing that tells a reader why this name
    /// is on the list at all when the letters are scattered through it.
    ///
    /// Character positions, not bytes: the caller is measuring cells.
    pub fn hits(&self, label: &str) -> Vec<usize> {
        match self.query.is_empty() {
            true => Vec::new(),
            false => matched(label, &self.query).map(|(_, at)| at).unwrap_or_default(),
        }
    }

    /// The first or the last match.
    pub fn go(&mut self, last: bool) {
        self.selected = match last {
            true => self.matches().len().saturating_sub(1),
            false => 0,
        };
    }

    /// Ten at a time.
    pub fn page(&mut self, down: bool) {
        for _ in 0..10 {
            self.step(down);
        }
    }

    /// Whether the keys are in the query rather than in the list.
    pub fn typing(&self) -> bool {
        self.typing
    }

    /// Put the keys in the list (`Esc`), or back in the query (`/`).
    pub fn type_here(&mut self, typing: bool) {
        self.typing = typing;
    }

    /// Which match is highlighted, clamped to what there is.
    pub fn selected(&self) -> usize {
        let count = self.matches().len();
        self.selected.min(count.saturating_sub(1))
    }

    /// The highlighted item, if the query matched anything.
    pub fn chosen(&self) -> Option<Item> {
        let matches = self.matches();
        matches.get(self.selected()).map(|&item| item.clone())
    }

    /// How far into the query the caret is, in characters.
    ///
    /// A query is typed text, and typed text is edited in the middle: this had
    /// `push` and `backspace` and nothing else, so a typo four characters back
    /// meant deleting everything after it.
    pub fn caret(&self) -> usize {
        self.caret.min(self.query.chars().count())
    }

    /// The query up to the caret — what the front end measures to put the
    /// terminal's cursor in the right cell.
    pub fn before_caret(&self) -> String {
        self.query.chars().take(self.caret()).collect()
    }

    /// Add a character at the caret. The highlight goes back to the top,
    /// because the list under it is a different list.
    pub fn push(&mut self, c: char) {
        let at = self.byte(self.caret());
        self.query.insert(at, c);
        self.caret = self.caret() + 1;
        self.selected = 0;
    }

    /// Remove the character before the caret, returning `false` when there was
    /// none — which is how Backspace on an empty query closes the picker.
    pub fn backspace(&mut self) -> bool {
        self.selected = 0;
        let caret = self.caret();
        if caret == 0 {
            return !self.query.is_empty();
        }
        let (from, to) = (self.byte(caret - 1), self.byte(caret));
        self.query.replace_range(from..to, "");
        self.caret = caret - 1;
        true
    }

    /// Remove the character *under* the caret; the caret stays where it is.
    pub fn delete(&mut self) {
        let caret = self.caret();
        if caret < self.query.chars().count() {
            let (from, to) = (self.byte(caret), self.byte(caret + 1));
            self.query.replace_range(from..to, "");
            self.selected = 0;
        }
    }

    /// Move the caret: `Left`, `Right`, `Home`/`C-a`, `End`/`C-e`.
    pub fn move_caret(&mut self, to: Caret) {
        let len = self.query.chars().count();
        self.caret = match to {
            Caret::Left => self.caret().saturating_sub(1),
            Caret::Right => (self.caret() + 1).min(len),
            Caret::Start => 0,
            Caret::End => len,
        };
    }

    /// Everything from the caret back to the start, gone (`C-u`).
    pub fn clear_before_caret(&mut self) {
        let at = self.byte(self.caret());
        self.query = self.query[at..].to_string();
        self.caret = 0;
        self.selected = 0;
    }

    /// Where the `at`-th character begins, in bytes.
    fn byte(&self, at: usize) -> usize {
        self.query
            .char_indices()
            .nth(at)
            .map(|(i, _)| i)
            .unwrap_or(self.query.len())
    }

    /// Move the highlight, wrapping at both ends.
    pub fn step(&mut self, down: bool) {
        let count = self.matches().len();
        if count == 0 {
            return;
        }
        let at = self.selected();
        self.selected = if down {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
    }
}

/// **How well `label` matches `query`, and where** — `None` when it does not.
///
/// A subsequence match: every character of the query must appear in the label,
/// in order. What the score rewards is what a reader's eye rewards, and what
/// every fuzzy finder from fzf onwards rewards too:
///
/// - letters found **next to each other** rather than scattered,
/// - letters at the **start of a path segment** or a word,
/// - letters in the **file's own name** rather than in the folders above it —
///   `ch63` typed at a novel means the chapter, not the folder it sits in,
/// - a **short** label over a long one holding the same letters.
///
/// ⚠️ **Two passes, and the second is backwards.** A single greedy pass takes
/// the *first* place each character fits, which for `ch6` in
/// `chapters/ch6.md` marks the `ch` of 「chapters」 and then the `6` far
/// away — a scatter, scored as one, and lit up in the wrong place. So the
/// forward pass only finds **where a match can end**, and a backward pass from
/// there takes the last place each character fits, which is the tightest match
/// ending at that point. fzf's v1 algorithm does the same thing for the same
/// reason.
fn matched(label: &str, query: &str) -> Option<(i64, Vec<usize>)> {
    // ⚠️ **One lowercase character per character.** `to_lowercase` may hand
    // back several (İ), and the positions this returns are indices into the
    // label the caller will be drawing — a mapping that is not one to one
    // would light up the wrong cell.
    let low = |c: char| c.to_lowercase().next().unwrap_or(c);
    let haystack: Vec<char> = label.chars().map(low).collect();
    let needle: Vec<char> = query.chars().map(low).collect();
    if needle.is_empty() {
        return Some((0, Vec::new()));
    }
    let mut at = 0usize;
    let mut end = 0usize;
    for &want in &needle {
        let found = (at..haystack.len()).find(|&i| haystack[i] == want)?;
        at = found + 1;
        end = found;
    }
    let mut positions = Vec::with_capacity(needle.len());
    let mut upto = end as isize;
    for &want in needle.iter().rev() {
        let found = (0..=upto).rev().find(|&i| haystack[i as usize] == want)?;
        positions.push(found as usize);
        upto = found - 1;
    }
    positions.reverse();
    // Where the name itself begins: everything before the last separator is
    // the folders, which are not what was typed at.
    let name_at = haystack
        .iter()
        .rposition(|&c| c == '/' || c == '\\')
        .map_or(0, |i| i + 1);
    let mut score = 0i64;
    for (n, &pos) in positions.iter().enumerate() {
        score += 1;
        if n > 0 && positions[n - 1] + 1 == pos {
            // Adjacent to the last match: the query is a run, not a scatter.
            score += 8;
        }
        let starts_segment = pos == 0
            || matches!(
                haystack[pos - 1],
                '/' | '\\' | '_' | '-' | '.' | ' ' | '\u{3000}'
            );
        if starts_segment {
            score += 4;
        }
        if pos >= name_at {
            score += 2;
        }
    }
    // A short label containing the query is a better answer than a long one.
    Some((score * 100 - haystack.len() as i64, positions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> Picker {
        Picker::new(
            "檔案",
            paths.iter().map(|p| Item::File(p.to_string())).collect(),
        )
    }

    #[test]
    fn an_empty_query_offers_everything_in_order() {
        let picker = files(&["ch01.md", "ch02.md"]);
        assert_eq!(picker.matches().len(), 2);
        assert_eq!(picker.chosen(), Some(Item::File("ch01.md".to_string())));
    }

    #[test]
    fn a_run_of_letters_beats_a_scatter() {
        let mut picker = files(&["卷二/ch63.md", "chapters/six/three.md"]);
        for c in "ch63".chars() {
            picker.push(c);
        }
        assert_eq!(
            picker.chosen(),
            Some(Item::File("卷二/ch63.md".to_string())),
            "the run should win"
        );
    }

    #[test]
    fn a_query_that_is_not_a_subsequence_matches_nothing() {
        let mut picker = files(&["ch01.md"]);
        for c in "zz".chars() {
            picker.push(c);
        }
        assert!(picker.matches().is_empty());
        assert_eq!(picker.chosen(), None);
    }

    #[test]
    fn the_highlight_wraps_and_survives_a_narrowing_query() {
        let mut picker = files(&["a.md", "b.md", "c.md"]);
        picker.step(true);
        assert_eq!(picker.selected(), 1);
        picker.step(false);
        picker.step(false);
        assert_eq!(picker.selected(), 2, "wraps at the top");

        // Typing narrows the list, and the highlight goes back to its head —
        // the item that was under it is not the item that is there now.
        picker.push('a');
        assert_eq!(picker.selected(), 0);
        assert_eq!(picker.chosen(), Some(Item::File("a.md".to_string())));
    }

    #[test]
    fn backspace_says_when_the_query_was_already_empty() {
        let mut picker = files(&["a.md"]);
        picker.push('a');
        assert!(picker.backspace());
        assert!(!picker.backspace(), "nothing left to delete");
    }

    #[test]
    fn the_name_beats_the_folder_it_is_in() {
        // 2026-09-18: 「ch63」 typed at a novel means the chapter, not the
        // folder the chapters are in.
        let mut picker = files(&["ch63/notes.md", "卷二/ch63.md"]);
        for c in "ch63".chars() {
            picker.push(c);
        }
        assert_eq!(picker.chosen(), Some(Item::File("卷二/ch63.md".to_string())));
    }

    #[test]
    fn the_hits_are_the_tightest_match_not_the_first_one() {
        // A single greedy pass marks the `ch` of 「chapters」 and then the `6`
        // eleven characters later — a scatter, and lit up in the wrong place.
        let mut picker = files(&["chapters/ch6.md"]);
        for c in "ch6".chars() {
            picker.push(c);
        }
        assert_eq!(picker.hits("chapters/ch6.md"), vec![9, 10, 11]);
        // Nothing is lit up before anything is typed.
        let quiet = files(&["chapters/ch6.md"]);
        assert!(quiet.hits("chapters/ch6.md").is_empty());
    }

    #[test]
    fn what_was_preferred_comes_first_until_something_is_typed() {
        let mut picker = files(&["a.md", "b.md", "c.md"]);
        picker.prefer(vec![0, 400, 0]);
        assert_eq!(picker.chosen(), Some(Item::File("b.md".to_string())));
        // …and it is a tie-breaker: typing decides, and `a` does not match
        // 「b.md」 at all.
        picker.push('a');
        assert_eq!(picker.chosen(), Some(Item::File("a.md".to_string())));
    }

    #[test]
    fn a_preference_shorter_than_the_list_is_padded_not_a_panic() {
        let mut picker = files(&["a.md", "b.md", "c.md"]);
        picker.prefer(vec![7]);
        assert_eq!(picker.matches().len(), 3);
        assert_eq!(picker.chosen(), Some(Item::File("a.md".to_string())));
    }

    #[test]
    fn the_keys_start_in_the_list_and_slash_takes_them_to_the_query() {
        let mut picker = files(&["a.md"]);
        assert!(!picker.typing(), "jk walk from the first keystroke");
        picker.type_here(true);
        assert!(picker.typing());
    }

    #[test]
    fn cjk_paths_match_by_their_own_characters() {
        let mut picker = files(&["卷一/初雪.md", "卷二/驚蟄.md"]);
        picker.push('驚');
        assert_eq!(
            picker.chosen(),
            Some(Item::File("卷二/驚蟄.md".to_string()))
        );
    }
}
