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
    /// a list of names is a list you can read. Warning: **It is drawn and never
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
    /// **列表那一層，鍵落在搜索框那一行**（2026-10-01 定，原話：「当然是移动到
    /// 搜索行点 i 啊。。。这就是个模态编辑啊」）。
    ///
    /// `k` 從第一條走上來就落在它身上，`j` 再走下去回第一條；`i` 在它身上纔進打
    /// 字。從前 `/` 在列表的任何一條上都進得去，而那不是模態的走法——同日去掉。
    on_query: bool,
    /// **檔名那一列是相對哪個目錄算的**（2026-10-02）。
    ///
    /// Warning: **從前它和列表緩衝區共用 `Editor::listing_root` 一個槽。** 那個槽的本
    /// 業是「`檔名:行號:` 這種列表的根」，給 `gf` 用；開一次 `空格 f` 就把它蓋
    /// 掉，於是 `:check` 出一張單子、中間按一下 `空格 f`，再回去 `gf` 就解到挑
    /// 選器的根底下去了。兩件不相干的事，各存各的。
    pub root: Option<std::path::PathBuf>,
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
    /// **收不收「跳着配」和「亂序」**。缺省**收**，`空格 f` 那一扇一格沒變。
    ///
    /// Warning: **只有 `ye --files` 把它關掉**（2026-10-04 定）。那一邊把全部印出來，
    /// 長尾就是噪音；`空格 f` 是一張排過序的單子，只看前十條，長尾不要錢——而跳着
    /// 配（`rlcrd` 找 `release_card.py`）在那裏是真有用的。原話：「cli --files 更
    /// 精确（五个选项不全开），只有当加了 open 之后进了 tui 才五个选项都开。」
    ///
    /// 關掉的是兩檔，不是五個：**大小寫、繁簡、拼音照舊**。所以 `ye --files` 比
    /// `ye --grep` 還多一條——後者的大小寫是智能的，這裏一律不分。
    loose: bool,
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
            // **一開在列表那一層**，`/` 或 `i` 纔進查詢。
            //
            // Warning: **2026-10-01 試過改成「一開就能打字」**（helix、fzf、telescope
            // 都是那樣），當天撤回。代價是那兩樣不想要的：關掉挑選器變成要按
            // 兩次 `Esc`（一次回列表、一次出門），而且開門第一下退格就換層。
            // 原話：「那就改成默认 normal 状态吧。」
            typing: false,
            on_query: false,
            root: None,
            bonus: vec![0; count],
            loose: true,
        }
    }

    /// 只收連在一起的那一段 —— `ye --files` 開門就撥它。
    pub fn tighten(&mut self) {
        self.loose = false;
        self.selected = 0;
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
                    matched(item.label(), &self.query, self.loose)
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
            false => matched(label, &self.query, self.loose).map(|(_, at)| at).unwrap_or_default(),
        }
    }

    /// The first or the last match.
    pub fn go(&mut self, last: bool) {
        self.selected = match last {
            true => self.matches().len().saturating_sub(1),
            false => 0,
        };
    }

    /// Ten at a time, **stopping at the ends**.
    ///
    /// Warning: 從前它是十次 [`Self::step`]，而 `step` 是繞回去的——於是一張**短過二十
    /// 條**的名單上，`PageDown` 往回走：十二條的名單站在第六條上按下去，
    /// `(5 + 10) % 12 = 3`，高亮退到第四條（2026-10-07 審出來的）。
    ///
    /// `j`/`k` 繞回去是對的，一張名單走到底再回頭是挑選器的老規矩；可「翻一頁」
    /// 從來不是「走十步」——翻到頭就是頭。
    pub fn page(&mut self, down: bool) {
        let count = self.matches().len();
        if count == 0 {
            return;
        }
        let at = self.selected();
        self.selected = match down {
            true => (at + 10).min(count - 1),
            false => at.saturating_sub(10),
        };
    }

    /// Whether the keys are in the query rather than in the list.
    pub fn typing(&self) -> bool {
        self.typing
    }

    /// Put the keys in the list (`Esc`), or back in the query (`/`).
    pub fn type_here(&mut self, typing: bool) {
        self.typing = typing;
        // 進了框，列表那一層的鍵就該停在框那一行上——`Esc` 出來纔落回原處。
        if typing {
            self.on_query = true;
        }
    }

    /// 列表那一層此刻站在搜索框那一行上嗎。
    pub fn on_query(&self) -> bool {
        self.on_query
    }

    /// **把列表那一層的鍵放到搜索框那一行上**，不進打字態。
    ///
    /// Warning: **`d`/`D` 也要叫它**（2026-10-01）：那兩個鍵在任何一行上都改得了查詢
    /// 詞，而站在一條檔名上按 `d`、查詢框裏悄悄少一個字，是看不見的事。改完把
    /// 鍵放到那一行上，改了什麼就在眼前。
    pub fn stand_on_query(&mut self) {
        self.on_query = true;
    }

    /// 把列表那一層的鍵從搜索框那一行帶回單子上。
    ///
    /// Warning: **`g`/`G`/翻頁要叫它**（2026-10-02 審出來的）：那四個走的是單子，
    /// 從前它們動了高亮卻把鍵留在框上——屏幕上兩個光標，而接着那一下 `j` 只夠
    /// 用來離開搜索行。
    pub fn leave_query(&mut self) {
        self.on_query = false;
    }

    /// **列表那一層走一步**——搜索框是最上面那一「行」，走得上去。
    ///
    /// Warning: **不包着走。** 從框往上沒有地方可去，從最後一條往下也不繞回框——
    /// 繞回去的話一路按 `j` 會在列表和框之間打轉，而那一行不是一條候選。
    pub fn step_in_list(&mut self, down: bool) {
        if self.on_query {
            // Warning: **單子空着就沒有地方可去**（2026-10-02 審出來的）：下去是站在
            // 「沒有符合的」那一行上，`Enter` 只能說一句狀態，`k` 是唯一的回路。
            if down && !self.matches().is_empty() {
                self.on_query = false;
            }
            return;
        }
        if !down && self.selected() == 0 {
            self.on_query = true;
            return;
        }
        self.step(down);
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

    /// **從光標刪到末尾**——框裏的 `D`（2026-10-01）。
    ///
    /// Warning: **末尾那一格上刪掉看得見的最後一個字**（2026-10-02 改，和搜索面板同一
    /// 條規矩）。那一條是 2026-09-27 定的，原話是兩個試用的人都報「`d` 按了什麼都
    /// 不發生」：框裏的光標走得到文字後面那一格，`Esc` 出來多半就停在那裏，而按
    /// 的人想刪的是看得見的最後那個字。
    ///
    /// 同日早些時候這裏改成了「末尾就什麼都別動」，理由是「刪了個空還把單子撥回
    /// 第一條，位置白丟」——那個理由現在不成立了：真的刪掉一個字，單子本來就該
    /// 重篩。兩扇面板一個鍵，不許有兩種答案。
    pub fn delete_to_end(&mut self) {
        let last = self.query.chars().count();
        let at = self.byte(self.caret().min(last.saturating_sub(1)));
        if self.query.is_empty() {
            return;
        }
        self.query.truncate(at);
        self.selected = 0;
    }

    /// Remove the character *under* the caret; the caret stays where it is.
    ///
    /// Warning: **末尾那一格上刪的是它前面那一個**——同 [`Picker::delete_to_end`]，
    /// 同搜索面板的 `Search::delete_here`。
    pub fn delete(&mut self) {
        let last = self.query.chars().count();
        let caret = match self.caret() >= last {
            true => match last.checked_sub(1) {
                Some(back) => back,
                None => return,
            },
            false => self.caret(),
        };
        let (from, to) = (self.byte(caret), self.byte(caret + 1));
        self.query.replace_range(from..to, "");
        self.caret = self.caret.min(self.query.chars().count());
        self.selected = 0;
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
/// Warning: **Two passes, and the second is backwards.** A single greedy pass takes
/// the *first* place each character fits, which for `ch6` in
/// `chapters/ch6.md` marks the `ch` of 「chapters」 and then the `6` far
/// away — a scatter, scored as one, and lit up in the wrong place. So the
/// forward pass only finds **where a match can end**, and a backward pass from
/// there takes the last place each character fits, which is the tightest match
/// ending at that point. fzf's v1 algorithm does the same thing for the same
/// reason.
fn matched(label: &str, query: &str, loose: bool) -> Option<(i64, Vec<usize>)> {
    // **原樣收着，大小寫交給 [`crate::nearby::alike`] 折**（2026-10-04）。
    //
    // Warning: **從前這裏自己先折一遍**，而且只能取 `to_lowercase()` 的第一個字
    // （İ 折出來是兩個），理由是「交回去的下標是要拿來高亮的，一對多就會點亮錯
    // 的格子」。共用那一支比的是**兩個字**、不改下標，所以那個將就沒有了。
    let haystack: Vec<char> = label.chars().collect();
    let needle: Vec<char> = query.chars().collect();
    if needle.is_empty() {
        return Some((0, Vec::new()));
    }
    // **In order first, and if that fails, the same letters in any order**
    // (2026-09-25). 原話：「我覺得改變順序應該很常見的，比如 a red apple 和 a
    // apple red 的相近程度其實很高」，而在此之前一個顛倒的查詢不是排在後面，是
    // **整條被篩掉、根本不出現**。
    //
    // Warning: **兩檔之間差着 [`IN_ORDER`] 分**，所以順序對的永遠在上面，顛倒的墊在
    // 底下——放寬不會把本來就對的那一批攪亂。門檻沒有：詞條、檔名這些單子本來就
    // 不長（定的，原話：「寧可多列」）。
    // **簡繁異體照樣算同一個字**（2026-09-26 原話：「中文搜索的匹配繁简体和匹配
    // 拼音对于 picker, buffer wiki 窗口中的搜索也应该有效」）。和高級搜索那一扇用
    // 的是同一張表。
    //
    // Warning: **放寬的是查詢那一邊，不是名字那一邊**，而這個方向就是那張表值錢的地方：
    // `class(发) = 发發髮`，所以打「头发」找得到「頭髮」；`class(發) = 發发`，所以
    // 打「發」不會誤中「髮」（見 [`crate::glyphs`]）。反過來折就把這個性質毀了。

    // **跨多遠算一處命中** —— 和搜索面板那一扇同一條規矩（`nearby::window`，
    // 2026-10-04 合的）。從前這裏不限：`release` 七個字母散在
    // `crates/yumete-core/src/lib.rs` 裏也算配上，於是 `ye --files release`
    // 在本倉吐了四兆。面板限它的理由寫在 `nearby` 的檔頭上——「差不多是這幾個字」
    // 說的是**一個詞組**，不是一行；一條路徑也不是一個詞組。
    let reach = Some(crate::nearby::window(needle.len()));
    // 大小寫折、繁簡折——兩條都是 `alike` 的事，面板問的是同一支。
    let (fold_case, glyphs) = (true, true);
    // **字面找不着，就問它念作什麽**（2026-09-26 提的）。`shuzhai` 找得到「書齋」。
    //
    // Warning: **只認全拼，和高級搜索一條規矩**（定的，原話：「Option 2 更符合目前的设计
    // 哲学——不一下子提供太多东西直到真有人要」）。所以 `sz` 不中。
    //
    // Warning: **排在字面之後**：查詢全是字母的時候，`md` 既是一個後綴也是一串讀音，而
    // 讀者打 `md` 十有八九在找 `.md`。字面接得住就不必問讀音。
    //
    // 兩檔共用它：讀音配上的那一段本來就是連着的，所以收緊也不影響。
    let said_instead = || -> Option<(Vec<usize>, bool)> {
        let said = crate::pinyin::atoms(query)?;
        let (from, to) = *crate::pinyin::spans_of(label, &said, true).first()?;
        Some(((from..to).collect(), true))
    };
    let (positions, in_order) = match loose {
        true => match crate::nearby::one(&haystack, &needle, 0, reach, fold_case, glyphs) {
            Some(found) => (found, true),
            None => match anyhow(&haystack, &needle, reach, fold_case, glyphs) {
                Some(found) => (found, false),
                None => said_instead()?,
            },
        },
        // **收緊：只收連在一起的那一段。** 跳着配和亂序兩檔都不走。
        false => match run_of(&haystack, &needle, fold_case, glyphs) {
            Some(found) => (found, true),
            None => said_instead()?,
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

/// **連在一起的那一段**，從左往右第一處 —— 收緊那一檔（2026-10-04）。
///
/// 和 [`crate::nearby::one`] 的分別就是「跳不跳」：這一支要求每一個字緊挨着上一個。
/// 折疊規矩還是 [`crate::nearby::alike`]，所以簡繁與大小寫照舊不分。
fn run_of(
    haystack: &[char],
    needle: &[char],
    fold_case: bool,
    glyphs: bool,
) -> Option<Vec<usize>> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|&at| {
            needle.iter().enumerate().all(|(k, &want)| {
                crate::nearby::alike(want, haystack[at + k], fold_case, glyphs)
            })
        })
        .map(|at| (at..at + needle.len()).collect())
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
    reach: Option<usize>,
    fold_case: bool,
    glyphs: bool,
) -> Option<Vec<usize>> {
    let same = |query: char, text: char| crate::nearby::alike(query, text, fold_case, glyphs);
    let mut taken = vec![false; haystack.len()];
    let mut positions = Vec::with_capacity(needle.len());
    for &want in needle {
        let found = (0..haystack.len()).find(|&i| !taken[i] && same(want, haystack[i]))?;
        taken[found] = true;
        positions.push(found);
    }
    positions.sort_unstable();
    // Warning: **這一檔也要限窗**（2026-10-04）。順序那一檔限了而它沒限，放水的就
    // 全走它這一條——而它本來就是最鬆的一檔。
    let (first, last) = (positions[0], positions[positions.len() - 1]);
    reach.is_none_or(|reach| last - first < reach).then_some(positions)
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

    /// **簡繁異體在這扇窗裏也算同一個字**（2026-09-26 原話：「中文搜索的匹配繁
    /// 简体和匹配拼音对于 picker, buffer wiki 窗口中的搜索也应该有效」）。
    #[test]
    fn the_two_ways_of_writing_a_character_find_one_file() {
        let mut picker = files(&["卷一/岳陽樓記.md", "卷二/別的.md"]);
        for c in "岳阳楼记".chars() {
            picker.push(c);
        }
        assert_eq!(picker.chosen(), Some(Item::File("卷一/岳陽樓記.md".to_string())));

        // Warning: **放寬的是查詢那一邊**：`发` 含混，兩邊都中；`發` 說得清，不碰「髮」。
        let mut picker = files(&["頭髮.md", "發現.md"]);
        picker.push('發');
        assert_eq!(picker.matches().len(), 1, "「發」不該誤中「髮」");
        assert_eq!(picker.chosen(), Some(Item::File("發現.md".to_string())));
    }

    /// **拼音也找得到**（同日）。Warning: **只認全拼**，和高級搜索一條規矩。
    #[test]
    fn the_letters_a_name_is_read_as_find_it_too() {
        let mut picker = files(&["卷一/洞庭湖.md", "notes.md"]);
        for c in "dongtinghu".chars() {
            picker.push(c);
        }
        assert_eq!(picker.chosen(), Some(Item::File("卷一/洞庭湖.md".to_string())));
        assert_eq!(picker.matches().len(), 1);

        // 首字母那一路不算——2026-09-26 定，同高級搜索。
        let mut picker = files(&["卷一/洞庭湖.md"]);
        for c in "dth".chars() {
            picker.push(c);
        }
        assert!(picker.matches().is_empty(), "只認全拼，`dth` 不算");

        // Warning: **字面先來**：`md` 是一串讀音，可讀者打它十有八九在找 `.md`。
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

    /// **審閱 2026-10-02 抓到的三件，都沒有測試守着。**
    #[test]
    fn the_query_row_does_not_swallow_the_list() {
        // ① 單子空着的時候 `j` 不許從搜索行掉下去——下面只有「沒有符合的」。
        let mut picker = files(&["a.md"]);
        picker.type_here(true);
        picker.push('z');
        picker.push('q');
        assert!(picker.matches().is_empty(), "什麼都配不上");
        picker.type_here(false);
        assert!(picker.on_query(), "離開打字態，鍵落在搜索行上");
        picker.step_in_list(true);
        assert!(picker.on_query(), "單子空着，j 留在原地");

        // ② `D` 在末尾什麼都刪不掉，那就別把單子撥回第一條。
        let mut picker = files(&["a.md", "b.md", "c.md"]);
        picker.step(true);
        assert_eq!(picker.selected(), 1);
        picker.delete_to_end();
        assert_eq!(picker.selected(), 1, "刪了個空，位置不許丟");
        picker.push('.');
        picker.push('m');
        picker.step(true);
        let at = picker.selected();
        picker.move_caret(Caret::Start);
        picker.delete_to_end();
        assert_eq!(picker.query(), "", "真的刪了");
        assert_eq!(picker.selected(), 0, "真刪了纔撥回去，{at}");

        // ③ `leave_query` 把鍵從搜索行帶回單子上。
        let mut picker = files(&["a.md", "b.md"]);
        picker.step_in_list(false);
        assert!(picker.on_query(), "k 從第一條走上搜索行");
        picker.leave_query();
        assert!(!picker.on_query());
    }

    /// **一開在列表那一層**——`jk` 第一下就走得動，`/` 或 `i` 纔進查詢。
    ///
    /// Warning: **2026-10-01 試過反過來，當天撤回**：那樣關掉挑選器要按兩次 `Esc`。
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

        // Warning: **缺一個字就不算**——放寬的是次序，不是「有幾個算幾個」。
        let mut picker = files(&["朱宇浩.md"]);
        for c in "朱浩甲".chars() {
            picker.push(c);
        }
        // Warning: `total()` 是「一共幾條」，不是「配上幾條」——問的是 `matches()`。
        assert_eq!(picker.matches().len(), 0, "甲 不在裏面");
    }

    /// 詞條那一行後半截只畫不比：打「冬天」找的是**叫**冬天的那一條。
    /// **管道那一邊收緊：跳着配和亂序都不收**（2026-10-04 定）。
    ///
    /// 原話：「cli --files 更精确（五个选项不全开），只有当加了 open 之后进了 tui
    /// 才五个选项都开。」`空格 f` 是一張排過序的單子，只看前十條，長尾不要錢；
    /// `ye --files` 把全部印出來，長尾就是噪音。
    ///
    /// 量出來的（本倉）：`ye --files lib` 從 13 行剩 **5 行**——正好那五個 `lib.rs`，
    /// 而 `build.rs`/`table.rs`/`labels.rs` 那幾條（`l`…`i`…`b` 跳着湊的）沒了。
    /// 同一個查詢在 `空格 f` 裏仍是 13 條。
    #[test]
    fn the_pipe_takes_a_name_literally() {
        // 跳着配：鬆的收，緊的不收。
        assert!(matched("scripts/release_card.py", "rlcrd", true).is_some());
        assert!(matched("scripts/release_card.py", "rlcrd", false).is_none());
        // 亂序同理。
        assert!(matched("scripts/release_card.py", "saeler", true).is_some());
        assert!(matched("scripts/release_card.py", "saeler", false).is_none());
        // `l`…`i`…`b` 跳着湊出來的那一族，緊的一條都不收。
        assert!(matched("crates/yumete/build.rs", "lib", true).is_some());
        assert!(matched("crates/yumete/build.rs", "lib", false).is_none());
        // 連在一起的照舊中。
        assert!(matched("crates/yumete-core/src/lib.rs", "lib", false).is_some());

        // Warning: **關掉的是兩檔，不是五個。** 大小寫、繁簡、拼音在緊的這一檔裏
        // 一格沒動——讀音配上的那一段本來就是連着的。
        assert!(matched("README.md", "readme", false).is_some(), "大小寫不分");
        assert!(matched("雜記/書齋夜話.md", "书斋", false).is_some(), "繁簡");
        assert!(matched("朱宇浩的手稿.md", "zhuyuhao", false).is_some(), "拼音");
        assert!(matched("卷01/第120章.md", "di120", false).is_some(), "字母與漢字混寫");
    }

    /// **一處命中跨不過那個窗口** —— 和搜索面板同一條規矩（2026-10-04 合的）。
    ///
    /// 從前這裏不限：子序列跨多遠都算配上，於是 `release` 七個字母散在
    /// `crates/yumete-core/src/lib.rs` 裏也中。路徑長、重複字母多，結果是幾乎
    /// 什麽都中——`ye --files release` 在本倉吐了**四兆**（2026-10-04 報的：「噪
    /// 音太多太多」）。
    ///
    /// 面板早就限了（`nearby::window` ＝ 查詢長度×2＋4），理由寫在 `nearby` 的檔頭
    /// 上：「差不多是這幾個字」說的是**一個詞組**，不是一行。一條路徑也不是一個
    /// 詞組，所以同一條規矩在這裏一樣對。
    #[test]
    fn a_match_cannot_reach_across_a_whole_path() {
        // 七個字母，窗口 18 個字；這條路徑 29 個字，散着夠不着。
        assert!(matched("crates/yumete-core/src/lib.rs", "release", true).is_none());
        // 同樣七個字母，連在一起的那一條照舊中。
        assert!(matched("scripts/release_card.py", "release", true).is_some());
        // 跳着配沒有被取消，只是不許跳得太遠：`rlcrd` 在 `release_card` 裏跨 11 個
        // 字，窗口是 14。
        assert!(matched("scripts/release_card.py", "rlcrd", true).is_some());
        // 亂序那一檔也限窗——它本來就是最鬆的一檔，不限就全走它那一條。
        assert!(matched("crates/yumete-core/src/lib.rs", "esaeler", true).is_none());
        assert!(matched("scripts/release_card.py", "saeler", true).is_some(), "亂序、可是挨着");

        // Warning: **中文那幾路一格都沒動**，窗口是按字算的，而它們本來就是連着的。
        assert!(matched("朱宇浩的手稿.md", "zhuyuhao", true).is_some(), "拼音");
        assert!(matched("雜記/書齋夜話.md", "shuzhai", true).is_some(), "拼音，跨目錄");
        assert!(matched("雜記/書齋夜話.md", "书斋", true).is_some(), "繁簡");
        assert!(matched("卷01/第120章.md", "di120", true).is_some(), "字母與漢字混寫");
        assert!(matched("卷01/第120章.md", "juan01/di120", true).is_some(), "混寫，跨目錄");
    }

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


    /// **翻一頁不是走十步**（2026-10-07 審出來的）。
    ///
    /// `page` 從前是十次 `step`，而 `step` 繞回去——於是短過二十條的名單上
    /// `PageDown` 往回走：十二條，站在第六條上按下去，`(5 + 10) % 12 = 3`。
    #[test]
    fn paging_down_a_short_list_never_goes_backwards() {
        let names: Vec<String> = (1..=12).map(|n| format!("ch{n:02}.md")).collect();
        let mut picker = files(&names.iter().map(String::as_str).collect::<Vec<_>>());
        for _ in 0..5 {
            picker.step(true);
        }
        assert_eq!(picker.selected(), 5, "站在第六條上");
        picker.page(true);
        assert_eq!(picker.selected(), 11, "翻到底就是底，不是繞回第四條");
        picker.page(true);
        assert_eq!(picker.selected(), 11, "到底了就不動");
        picker.page(false);
        assert_eq!(picker.selected(), 1, "往回也是十步");
        picker.page(false);
        assert_eq!(picker.selected(), 0, "翻到頂就是頂");
    }

    /// 長名單照舊是十步，而 `j`/`k` 照舊繞回去——那是挑選器的老規矩，沒動。
    #[test]
    fn a_long_list_still_moves_ten_and_the_arrows_still_wrap() {
        let names: Vec<String> = (1..=40).map(|n| format!("ch{n:02}.md")).collect();
        let mut picker = files(&names.iter().map(String::as_str).collect::<Vec<_>>());
        picker.page(true);
        assert_eq!(picker.selected(), 10);
        picker.step(false);
        assert_eq!(picker.selected(), 9);
        // 頂上再往上，繞到底。
        picker.page(false);
        assert_eq!(picker.selected(), 0);
        picker.step(false);
        assert_eq!(picker.selected(), 39, "`k` 照舊繞回去");
    }
}
