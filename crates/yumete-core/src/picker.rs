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
    /// **A wiki entry, by name** — `:wiki 朱宇浩` (2026-09-25).
    ///
    /// The second string is the **blurb**: the head of what the entry says, so
    /// a list of names is a list you can read. ⚠️ **It is drawn and never
    /// matched** — typing 「冬天」 should find the entry *called* 冬天, not
    /// every entry that mentions winter.
    ///
    /// The name, not an index: the wiki is re-read whenever one of its files
    /// is saved, and an index would go stale between opening this list and
    /// choosing from it. The name is the feature's own key (`Wiki::by_name`).
    Wiki(String, String),
}

impl Item {
    /// The text shown in the list, and matched against.
    pub fn label(&self) -> &str {
        match self {
            Item::File(path) => path,
            Item::Buffer(_, name) | Item::Row(_, name) | Item::Paste(_, name) => name,
            Item::Wiki(name, _) => name,
        }
    }

    /// **What is drawn after the label and never matched.** Empty for
    /// everything but a wiki entry, whose row is 「名字　它說的頭一句…」.
    pub fn blurb(&self) -> &str {
        match self {
            Item::Wiki(_, blurb) => blurb,
            _ => "",
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
    // **In order first, and if that fails, the same letters in any order**
    // (2026-09-25). 原話：「我覺得改變順序應該很常見的，比如 a red apple 和 a
    // apple red 的相近程度其實很高」，而在此之前一個顛倒的查詢不是排在後面，是
    // **整條被篩掉、根本不出現**。
    //
    // ⚠️ **兩檔之間差着 [`IN_ORDER`] 分**，所以順序對的永遠在上面，顛倒的墊在
    // 底下——放寬不會把本來就對的那一批攪亂。門檻沒有：詞條、檔名這些單子本來就
    // 不長（作者定：「寧可多列」）。
    // **簡繁異體照樣算同一個字**（2026-09-26 作者提：「中文搜索的匹配繁简体和匹配
    // 拼音对于 picker, buffer wiki 窗口中的搜索也应该有效」）。和高級搜索那一扇用
    // 的是同一張表。
    //
    // ⚠️ **放寬的是查詢那一邊，不是名字那一邊**，而這個方向就是那張表值錢的地方：
    // `class(发) = 发發髮`，所以打「头发」找得到「頭髮」；`class(發) = 發发`，所以
    // 打「發」不會誤中「髮」（見 [`crate::glyphs`]）。反過來折就把這個性質毀了。
    let same = |want: char, have: char| {
        want == have || crate::glyphs::shapes(want).contains(have)
    };
    let (positions, in_order) = match forwards(&haystack, &needle, same) {
        Some(found) => (found, true),
        None => match anyhow(&haystack, &needle, same) {
            Some(found) => (found, false),
            // **字面找不着，就問它念作什麽**（2026-09-26 作者提）。`tianmen` 找得
            // 到「天門」。
            //
            // ⚠️ **只認全拼，和高級搜索一條規矩**（作者定：「Option 2 更符合目前
            // 的设计哲学——不一下子提供太多东西直到真有人要」）。所以 `tm` 不中。
            //
            // ⚠️ **排在字面之後**：查詢全是字母的時候，`md` 既是一個後綴也是一串
            // 讀音，而讀者打 `md` 十有八九在找 `.md`。字面接得住就不必問讀音。
            None => {
                let said = crate::pinyin::as_query(query)?;
                let (from, to) = *crate::pinyin::spans(label, &said).first()?;
                ((from..to).collect(), true)
            }
        },
    };
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
    let score = score * 100 - haystack.len() as i64;
    Some((score + if in_order { IN_ORDER } else { 0 }, positions))
}

/// How much better an in-order match is than the same letters jumbled.
///
/// Bigger than any score a single label can earn: the longest name worth
/// matching is a few dozen characters, each worth at most 15 before the ×100,
/// so a few tens of thousands covers it with room to spare. The point is that
/// the two kinds never interleave — 「順序對的」 is a category, not a nudge.
const IN_ORDER: i64 = 1_000_000;

/// Every needle character, **in order**, earliest and then tightest.
///
/// Two passes, and the second is backwards — the same shape [`crate::nearby`]
/// uses, for the same reason. A forward walk alone takes the *first* place
/// each character fits: 「他説」 in 「他。他説」 would be marked from the first
/// 他, and the reader sees a range with a full stop in the middle of it.
fn forwards(
    haystack: &[char],
    needle: &[char],
    same: impl Fn(char, char) -> bool,
) -> Option<Vec<usize>> {
    let mut at = 0usize;
    let mut end = 0usize;
    for &want in needle {
        let found = (at..haystack.len()).find(|&i| same(want, haystack[i]))?;
        at = found + 1;
        end = found;
    }
    let mut positions = Vec::with_capacity(needle.len());
    let mut upto = end as isize;
    for &want in needle.iter().rev() {
        let found = (0..=upto).rev().find(|&i| same(want, haystack[i as usize]))?;
        positions.push(found as usize);
        upto = found - 1;
    }
    positions.reverse();
    Some(positions)
}

/// **Every needle character is in there somewhere, order be damned** — 朱浩宇
/// finding 朱宇浩 (2026-09-25).
///
/// One haystack character per needle character (so 「朱朱」 needs two 朱), each
/// taken as early as it can be. The positions come back **sorted**, because
/// they are about to be drawn: a highlight has to run left to right whatever
/// order the query was typed in.
fn anyhow(
    haystack: &[char],
    needle: &[char],
    same: impl Fn(char, char) -> bool,
) -> Option<Vec<usize>> {
    let mut taken = vec![false; haystack.len()];
    let mut positions = Vec::with_capacity(needle.len());
    for &want in needle {
        let found = (0..haystack.len()).find(|&i| !taken[i] && same(want, haystack[i]))?;
        taken[found] = true;
        positions.push(found);
    }
    positions.sort_unstable();
    Some(positions)
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

    /// **簡繁異體在這扇窗裏也算同一個字**（2026-09-26 作者提：「中文搜索的匹配繁
    /// 简体和匹配拼音对于 picker, buffer wiki 窗口中的搜索也应该有效」）。
    #[test]
    fn the_two_ways_of_writing_a_character_find_one_file() {
        let mut picker = files(&["卷一/天門真境.md", "卷二/別的.md"]);
        for c in "天门".chars() {
            picker.push(c);
        }
        assert_eq!(picker.chosen(), Some(Item::File("卷一/天門真境.md".to_string())));

        // ⚠️ **放寬的是查詢那一邊**：`发` 含混，兩邊都中；`發` 說得清，不碰「髮」。
        let mut picker = files(&["頭髮.md", "發現.md"]);
        picker.push('發');
        assert_eq!(picker.matches().len(), 1, "「發」不該誤中「髮」");
        assert_eq!(picker.chosen(), Some(Item::File("發現.md".to_string())));
    }

    /// **拼音也找得到**（同日）。⚠️ **只認全拼**，和高級搜索一條規矩。
    #[test]
    fn the_letters_a_name_is_read_as_find_it_too() {
        let mut picker = files(&["卷一/天門真境.md", "notes.md"]);
        for c in "tianmen".chars() {
            picker.push(c);
        }
        assert_eq!(picker.chosen(), Some(Item::File("卷一/天門真境.md".to_string())));
        assert_eq!(picker.matches().len(), 1);

        // 首字母那一路不算——作者 2026-09-26 定，同高級搜索。
        let mut picker = files(&["卷一/天門真境.md"]);
        for c in "tm".chars() {
            picker.push(c);
        }
        assert!(picker.matches().is_empty(), "只認全拼，`tm` 不算");

        // ⚠️ **字面先來**：`md` 是一串讀音，可讀者打它十有八九在找 `.md`。
        let mut picker = files(&["notes.md", "馬達.txt"]);
        for c in "md".chars() {
            picker.push(c);
        }
        assert_eq!(picker.chosen(), Some(Item::File("notes.md".to_string())));
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

    /// **順序反了也找得到，而順序對的排在前面**（2026-09-25 定）。
    ///
    /// 原話：「我覺得改變順序應該很常見的，比如 a red apple 和 a apple red 的
    /// 相近程度其實很高」。從前顛倒的查詢不是排在後面，是**整條篩掉**。
    #[test]
    fn letters_out_of_order_still_match_and_rank_below_the_ones_in_order() {
        let mut picker = files(&["朱宇浩.md"]);
        for c in "朱浩宇".chars() {
            picker.push(c);
        }
        assert_eq!(picker.matches().len(), 1, "顛倒的名字也找得到");

        // 兩條都有這幾個字，只有一條的次序對——對的那條在上面。
        let mut picker = files(&["朱宇浩.md", "朱浩宇.md"]);
        for c in "朱浩宇".chars() {
            picker.push(c);
        }
        let order: Vec<&str> = picker.matches().iter().map(|i| i.label()).collect();
        assert_eq!(order, ["朱浩宇.md", "朱宇浩.md"], "順序對的先出");

        // ⚠️ **缺一個字就不算**——放寬的是次序，不是「有幾個算幾個」。
        let mut picker = files(&["朱宇浩.md"]);
        for c in "朱浩甲".chars() {
            picker.push(c);
        }
        // ⚠️ `total()` 是「一共幾條」，不是「配上幾條」——問的是 `matches()`。
        assert_eq!(picker.matches().len(), 0, "甲 不在裏面");
    }

    /// 詞條那一行後半截只畫不比：打「冬天」找的是**叫**冬天的那一條。
    #[test]
    fn a_wiki_blurb_is_drawn_and_never_matched() {
        let mut picker = Picker::new(
            "詞條",
            vec![
                Item::Wiki("阿寧".into(), "女主角，冬天住在石階盡頭".into()),
                Item::Wiki("冬天".into(), "一年裏最冷的那一段".into()),
            ],
        );
        for c in "冬天".chars() {
            picker.push(c);
        }
        let order: Vec<&str> = picker.matches().iter().map(|i| i.label()).collect();
        assert_eq!(order, ["冬天"], "阿寧 的正文裏有「冬天」，可它不叫冬天");
    }
}
