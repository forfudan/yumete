//! The search panel — Feature #419.
//!
//! `:grep` printed a buffer of lines and `:replace` rewrote whatever that
//! buffer had found. Both are gone. What replaces them is a **panel**: a form
//! you type a pattern into and a list of what it found, in a slot down the
//! side of the page, the way VSCode's search is.
//!
//! **A command here only ever names a place**, never a pattern — `:search 卵`
//! is 「a folder called 卵」, and what to look for is typed in the box. That is
//! how the ambiguity is kept out: there is one place a pattern can be.
//!
//! This module is the panel's *state*. Running the search is the editor's
//! (`editor/search.rs`), drawing it is the front end's.

/// How much case matters — Feature #419.
///
/// **Three, not two.** Two could not spell 「the same rule the rest of the
/// editor uses」: the page's own `/` is smart-cased (a pattern with no capital
/// ignores case, #301), and a panel that offered only on/off would search
/// differently from the key beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Case {
    /// A capital in the pattern is how you ask for case to matter — the rule
    /// `/` already follows.
    #[default]
    Smart,
    /// Case matters, capital or not.
    Sensitive,
    /// Case never matters.
    Insensitive,
}

impl Case {
    /// The next one, cycling — what `Enter` on the switch does.
    pub fn next(self) -> Case {
        match self {
            Case::Smart => Case::Sensitive,
            Case::Sensitive => Case::Insensitive,
            Case::Insensitive => Case::Smart,
        }
    }
}

/// **Where to look** — Feature #419.
///
/// A command names one of these and nothing else: `:search 卵` is 「a folder
/// called 卵」, never 「look for 卵」. That is the whole of how the ambiguity
/// is kept out — what to look for has exactly one home, the box.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Where {
    /// The file being written. **The only one that is searched as you type**:
    /// it is in memory, and a pass over it costs nothing worth counting.
    #[default]
    Buffer,
    /// **Every open buffer**, the unnamed drafts included — in memory, like
    /// [`Where::Buffer`].
    ///
    /// vim has it (`:bufdo`) and telescope has it (`grep_open_files`); helix's
    /// search does not, though its `空格 b` picker is exactly this list.
    Buffers,
    /// The working directory, and everything under it — where `ye` was typed,
    /// moved by `:cd`.
    Working,
    /// **The project** — the nearest ancestor of the working directory holding
    /// `.yumete` (or `.yumete.toml`); failing that, the nearest holding `.git`;
    /// failing both, the working directory itself.
    ///
    /// Warning: **`.jj` 和 `.svn` 不算**：[`crate::editor::book_root`] 只找那兩樣。
    /// 兩道是分開找的，所以
    /// 上面某一層的 `.yumete` 贏過更近的一層 `.git`——書根是寫稿的人標的，版本
    /// 庫是工具標的。
    ///
    /// Warning: **Reckoned from the working directory, never from the file being
    /// edited**. The two part company exactly when `gd` has
    /// jumped somewhere else — into homebrew, say — and a scope that followed
    /// the cursor would then search homebrew rather than the project the
    /// reader is writing. helix's one cursor-following root is the no-LSP
    /// `空格 S`, and its comment says that is for two projects open in a
    /// split, which is not a thing here.
    Project,
    /// A folder named outright: `:search ../稿`.
    ///
    /// Warning: **Not in the `0` cycle, and not editable in the panel.** A typed
    /// path is not something 「next」 can reach, and a cycle through it would
    /// wipe it on the way past.
    Named(std::path::PathBuf),
}

impl Where {
    /// **下一個範圍**——`0` 在面板裏按一下走一格，範圍不必打路徑就換得了。
    ///
    /// 本文件 → 緩衝區 → 工作路徑 → 項目路徑 → 回到本文件。Warning: **指名道姓那一種
    /// 不在圈裏**：它是使用者自己打的一個路徑，輪到它就回本文件——一個「下一個」
    /// 走不到、也走不出的值不該卡在環上。
    pub fn next(&self) -> Where {
        match self {
            Where::Buffer => Where::Buffers,
            Where::Buffers => Where::Working,
            Where::Working => Where::Project,
            Where::Project => Where::Buffer,
            // **指定文件夾只能用命令進來**。它要是圈上的一檔兼輸入框，`0` 繞
            // 一圈就會把打好的路徑清掉。
            Where::Named(_) => Where::Buffer,
        }
    }

    /// Whether this is searched again on every keystroke.
    ///
    /// Warning: **Only what is already in memory is.** The disk scopes walk files,
    /// and a hundred chapters per letter typed is not a thing to do — those
    /// wait for `Enter`. The panel says which it is, because one panel
    /// behaving two ways with nothing on the screen to tell them apart is the
    /// trap.
    pub fn live(&self) -> bool {
        matches!(self, Where::Buffer | Where::Buffers)
    }

    /// **Whether this scope walks the disk** — and so whether 包含, 排除 and
    /// 搜索隱藏和忽略 mean anything.
    ///
    /// 本文件 and 緩衝區 are a list already in front of the reader; narrowing
    /// it by a path glob is not a thing anyone wants, and a hidden file you
    /// have open is on the list by definition. Neither reference implementation
    /// offers the combination either: telescope's `grep_open_files` silently
    /// drops `search_dirs` (`__files.lua:172-180`), and vim's `:bufdo` takes no
    /// file argument.
    pub fn walks_the_disk(&self) -> bool {
        matches!(self, Where::Working | Where::Project | Where::Named(_))
    }
}

/// Which cell of the form the keys are in.
///
/// In screen order, which is also `Tab`'s order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Field {
    /// **哪裏找** —— 四選一：本文件、緩衝區、工作路徑、項目路徑。第五檔「指定
    /// 文件夾」只能 `:search 某目錄` 進來，不在輪替上。
    ///
    /// Warning: **畫在開關那一列的末尾，不畫在查詢框上面**（畫在上面，那個鍵在
    /// 屏幕上一個字都没有）：它和包含/排除、不搜合成「搜哪裏、搜哪些」一組，
    /// 號碼是 `6`。
    Scope,
    /// The pattern.
    #[default]
    Query,
    /// **中文匹配**——簡繁異體與拼音合成一行。
    ///
    /// 四態：`[繁簡+拼音]` → `[繁簡]` → `[拼音]` → `[ ]`。兩個說的是同一件事——
    /// **字面不同的寫法算不算同一個**——所以合在一行。
    ///
    /// Warning: **合得成，是因為兩者都「永遠有效」。** 繁簡在正則和模糊底下也折
    /// （`glyphs::widen_pattern` 與 `nearby::spans` 的字形折疊），兩者都没有
    /// 例外；有一個失效的話，合成一行就有一半是假的。
    ///
    /// 拼音只在查詢全是 ASCII 字母的時候纔真的跑（[`crate::pinyin::as_query`]），
    /// 所以搜 `hello` 一點不受它影響。
    Chinese,
    /// What to put in its place — only there when the panel is replacing.
    Replace,
    /// 大小寫, three ways.
    Case,
    /// **匹配模式**——字面/正則/模糊，三選一。
    ///
    /// 三個互斥的答案回答同一個問題：**這串字怎麼讀**。不做成兩個獨立的勾：那樣
    /// 「兩個都關」纔是默認——那一檔没有名字，只能從兩個空方框去推。
    Matching,
    /// 西文整詞匹配 — ASCII `\b` on both ends.
    ///
    /// Warning: **不併進 [`Field::Matching`]**：它和 正則 疊得起來（`\b<式子>\b`），
    /// 只和 模糊 互斥——模糊底下畫灰。
    Whole,
    /// **替換 —— 一行三態**，按一下轉一格。
    ///
    /// 關 → 字面替換 → 智能大小寫。一行而不是兩個勾（替換，加一個只在勾上之後
    /// 纔畫得出來的「跟原文的大小寫」）：省一行，而且没勾替換的時候没有一個灰着
    /// 的號碼。
    ///
    /// 「字面替換」就是出廠那一檔：打什麼就寫什麼。「智能大小寫」是換上去的那一
    /// 段跟着原文走——VS Code 把它畫成替換行上的 `AB`，和搜索行上的 `Aa` 分開。
    ///
    /// `:search` 進來的人想改一個詞，不必退出去重按 `:replace`。它是個開關而不是
    /// 另一扇面板：面板是同一扇，開上說的
    /// 是「這些我要改」，不是「剛纔找到的不算了」。
    Replacing,
    /// **只搜哪些文件** —— a glob, or several separated by commas
    /// (VS Code's 「files to include」).
    ///
    /// Warning: **Only means anything for a scope that walks the disk.** 本文件 and
    /// 緩衝區 are a list already in front of the reader; see
    /// [`Where::walks_the_disk`].
    Include,
    /// **哪些文件不搜** —— the same, the other way round.
    Exclude,
    /// **不搜哪些**——隱藏文件、`.gitignore` 裏的，兩件事分開撥。
    ///
    /// 按 `7` 轉一格，四態：`[隱藏+忽略]`（出廠）→`[隱藏]`→`[忽略]`→`[無]`。
    /// 方框裏列的是**不搜哪些**，所以 `[隱藏]` 是「點文件仍然跳過，`.gitignore`
    /// 裏的那些現在搜」——那一步排在第一，因為想找回來的多半是被忽略的目錄，
    /// 不是點文件。
    ///
    /// Warning: **兩件事不併成一個勾。** 在一個工作區根上搜，`.gitignore` 裏寫着
    /// `/yu/`，整個子倉就搜不到，而一個合併的開關名叫「隱藏」，誰也想不到去撥它。
    ///
    /// 出廠兩個都關：跳過隱藏文件、尊重 `.gitignore`，同 helix 與 telescope 的默認。
    /// vim 的默認 `grepprg` 是 `rg --vimgrep -uu`，故意關掉這兩道閘來跟傳統
    /// grep 對齊——那是它要兼容的歷史，不是我們的。
    ///
    /// Warning: **號碼排在 保留大小寫 後面**，和它一樣是「畫得出來纔算數」那一族的
    /// 尾巴，所以上面那七個的號碼一個都不動。
    Hidden,
    /// The list of what was found. Not a cell to type in; `Tab` reaches it so
    /// that walking the form ends up where the answers are.
    Results,
}

impl Field {
    /// Every cell, in `Tab`'s order.
    /// Warning: **這個次序就是畫出來的次序**（`draw_search`）。走的和看的不是一回事
    /// 的時候，`Tab` 會在一張看不見的表上跳，而那是没人能學會的。
    ///
    /// 大小寫排在頭一個：它是三態的那一個，擺在最上面，讀者第
    /// 一眼看見的就是「這一格裏寫着狀態」，下面三個 `[x]`/`[ ]` 自然照這個讀法。
    pub const ALL: [Field; 12] = [
        Field::Query,
        Field::Replace,
        Field::Case,
        Field::Chinese,
        Field::Matching,
        Field::Whole,
        Field::Replacing,
        // **「搜哪裏、搜哪些」四格攢在最下面**，位置在包含和排除上方。位置說的
        // 是範圍，和底下三格是一夥的，所以四格一組，號碼接着往下排。
        Field::Scope,
        Field::Include,
        Field::Exclude,
        Field::Hidden,
        Field::Results,
    ];

    /// **The switches, top to bottom as they are drawn** — #419.
    ///
    /// They are pressed by number (`1`–`7`) and the cursor never stops on
    /// them, so this order is the whole of what the numbers mean. It is the
    /// screen's order, so a reader counts rows rather than learning a list.
    ///
    /// Warning: **Every one is always drawn**, 西文整詞匹配 included — it goes quiet
    /// under 模糊 rather than disappearing, so the numbers below it do not
    /// shift under the reader's eye.
    pub const SWITCHES: [Field; 7] = [
        Field::Case,
        Field::Chinese,
        Field::Matching,
        Field::Whole,
        Field::Replacing,
        // **位置也按號碼到**（`6`）。它不是一個勾，是
        // 四選一，可按法和上面幾個一樣：不必先把光標走上去。
        Field::Scope,
        Field::Hidden,
    ];

    /// A tick or a state rather than something to type in or a list to walk.
    pub fn is_switch(self) -> bool {
        Field::SWITCHES.contains(&self)
    }

    /// **`jk` 走不上去的那幾格**——按號碼到，不是走過去。
    ///
    /// Warning: **位置那一格停不住。** 它是四選一，按號碼換一檔，和開關那一列裏
    /// 別的幾格是同一種東西；指定文件夾只能 `:search 某目錄` 進來，進來之後面板
    /// 裏改不了，所以那一格没有任何可以打字的狀態。
    ///
    /// Warning: **包含/排除看範圍走不走磁碟。** 選到 本文件 或 緩衝區 的時候它們整行
    /// 不畫，自然也停不住。
    pub fn walked_past(self, on_disk: bool) -> bool {
        match self {
            Field::Include | Field::Exclude => !on_disk,
            other => other.is_switch(),
        }
    }

    /// Whether this cell is typed into at all (so `i` and the IME belong here).
    ///
    /// Warning: **包含/排除是有條件的**，問 [`Search::takes_text`] 纔算數：範圍不走
    /// 磁碟的時候那兩格畫灰，打不了字。位置那一格永遠打不了。
    pub fn takes_text(self) -> bool {
        matches!(self, Field::Query | Field::Replace | Field::Include | Field::Exclude)
    }

    /// The next cell in that direction, wrapping — **the boxes and the list,
    /// never a switch**, and the replace box only while the panel is replacing.
    ///
    /// Warning: **The switches are walked past, not walked into**, so `jk` moves
    /// between the boxes and the results without spending keys on the rows of
    /// settings in between. They are pressed by number;
    /// what the cursor walks is 範圍 → 找什麼 →（換成什麼）→ 結果, which is the
    /// path anybody actually takes through this form.
    ///
    /// Warning: **模糊 and replacing never show together.** A loose
    /// match covers characters nobody typed, so 「replace them all」 would hand
    /// the manuscript to a range the writer cannot predict. 模糊 is for
    /// finding; when it has found the place, `Esc` and change it there.
    /// Warning: **從一個走不上去的格子出發也要對。** 按了 `6` 或者 `3`，鍵就落在
    /// 那一格上了，而它不在可走的名單裏——拿「名單第 0 格」頂替的話，按完號碼
    /// 再按 `k` 就會跳到名單最底下的結果。所以走的是**畫出來的那張全表**，一格
    /// 一格往那個方向找，碰到第一個停得住的就停。
    pub fn step(self, back: bool, replacing: bool, on_disk: bool, any_hits: bool) -> Field {
        let stops = |f: Field| match f {
            Field::Replace => replacing,
            // **一行都没有的名單不是一個落腳點**。空名單什麼都不畫，鍵走進去
            // 就**看不見了**，按 `j` 之後不知道光標去了哪裏。往回繞也撞得上：從「搜」按 `k` 繞過隊尾那個空名單纔到
            // 「排除」，看着就是「消失一下再出現在別處」。
            Field::Results => any_hits,
            f => !f.walked_past(on_disk),
        };
        let n = Field::ALL.len();
        let mut at = Field::ALL.iter().position(|&f| f == self).unwrap_or(0);
        for _ in 0..n {
            at = match back {
                true => (at + n - 1) % n,
                false => (at + 1) % n,
            };
            if stops(Field::ALL[at]) {
                return Field::ALL[at];
            }
        }
        self
    }
}

/// **這張名單是照着哪一版正文算出來的。**
///
/// 三個數各答一件事，合起來纔說得出「改的是誰」：
///
/// | | 變了說明 |
/// | --- | --- |
/// | `buffer` | 換了一份稿子在寫——每一處命中身上的檔名都要重寫 |
/// | `revision` | 正在寫的這一份改過了 |
/// | `every` | 某一份改過了，不一定是正在寫的這一份 |
///
/// Warning: **`every` 一個數不夠。** 它是所有緩衝的改動次數之和，看得出「有人動過」，
/// 看不出動的是誰；而「只有正在寫的這一份動過」正是只重搜一份的前提。兩個數
/// 一減就答得上來：`every` 的增量等於 `revision` 的增量，就是只有它動過。
///
/// 和數當指紋成立，是因為改動次數只增不減：兩次改動抵消不掉。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    pub buffer: u64,
    pub revision: u64,
    pub every: u64,
}

/// One place the pattern was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The file it is in — `None` for the one being written.
    ///
    /// `None` rather than the buffer's own path because a hit in *this* file
    /// is reached by moving the cursor, and one in another file is reached by
    /// opening it; the two are different acts and the type says so.
    pub file: Option<std::path::PathBuf>,
    /// Which line of the buffer, counting from zero.
    pub line: usize,
    /// **第幾個字**，從這一行的行首數起、從零起。
    ///
    /// Warning: [`Self::mark`] 答不了這個：它是命中落在**摘錄**裏的位置，而摘錄是命中
    /// 前後各六十個字——命中靠後的時候前面還會多一個 `…`。管道那一邊要印的是
    /// `檔:行:列:文字`，而那個列是行裏的列。
    pub column: usize,
    /// Where the match starts and ends, as character offsets into the buffer.
    pub at: usize,
    pub end: usize,
    /// **A few characters either side of the match**, not the whole line.
    ///
    /// A line of a novel is a paragraph — a few thousand characters — so
    /// 「the line it is on」 is not a unit anybody can read in a column.
    pub excerpt: String,
    /// Where in `excerpt` the match itself sits, in characters.
    pub mark: std::ops::Range<usize>,
    /// **Which match on its line this is**, counting from zero.
    ///
    /// How one hit is picked out again when the time comes to change it:
    /// the offsets above were counted when the file was read, and a buffer
    /// opened since may have moved everything after the first edit.
    pub nth: usize,
    /// **Which buffer it is in**, when the answer is a buffer rather than a
    /// file — the 緩衝區 scope.
    ///
    /// Warning: **An unnamed draft has no path to reach it by**, and two of them
    /// have the same `[scratch]` label, so a path cannot say which. Only the
    /// 緩衝區 scope fills this in; every other scope reaches a hit by opening
    /// the file named in [`Hit::file`].
    pub buffer: Option<u64>,
}

/// **How many hits the list holds.** Every one is counted; this many are kept.
///
/// Warning: **問 [`Search::most`]，別直接用這個常量。** 管道那一邊要的是全部
/// （`ye --grep`），而它是靠 [`Search::uncapped`] 說的。
///
/// 「的」 in a novel is twenty thousand places, and the count is the useful
/// half of that answer — the list is for walking, and nobody walks twenty
/// thousand rows.
pub const MOST: usize = 500;

/// How many characters of context an excerpt carries on each side.
///
/// **Cut for the command row, not for the panel column.** The column is
/// thirty-odd cells wide and clips whatever will not fit; the row under the
/// text is the width of the window, and it is the row a reader is actually
/// reading while `j k` walks the hits. Sixty each way fills a 250-column
/// terminal, which is wider than anybody's.
///
/// Warning: **A hit in another file has nothing else to offer.** The buffer is not
/// open, so this excerpt — taken when the search ran — is the only context
/// that row will ever have. Hence the number is set by the row, and the
/// column is left to clip.
pub const AROUND: usize = 60;

/// A row of the results, as drawn — Feature #419.
///
/// Flat while everything is in the file being written; a tree of file headers
/// and their hits as soon as it is not. The rows are worked out from the hits
/// every time they are wanted rather than stored beside them, so folding a
/// file away cannot leave the two disagreeing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A file, with how many hits are in it and whether they are folded away.
    ///
    /// Warning: **名字不夠認出一個檔。** 緩衝區那一檔裏 `path` 放的是**給人看的
    /// 名字**，而没有名字的草稿一律叫 `[scratch]`——只認名字，兩份草稿就併成
    /// 一行，標題寫着「2 處」，`r` 按下去只換得掉第一份裏的那一處。
    /// 所以認一個檔要名字加號碼這一對，和 [`Hit`] 一樣。
    File {
        path: std::path::PathBuf,
        buffer: Option<u64>,
        hits: usize,
        folded: bool,
    },
    /// One hit, by its index into [`Search::hits`].
    Hit(usize),
}

/// The search panel's state — Feature #419.
#[derive(Debug, Clone, Default)]
pub struct Search {
    /// What is being looked for, as typed.
    pub query: String,
    /// What to put in its place.
    pub replace: String,
    /// 換上去的那一段跟不跟原文的大小寫——替換那一行的第三檔，見
    /// [`Field::Replacing`]。
    pub preserve_case: bool,
    /// Whether the replace row is showing — what `:replace` opens with.
    ///
    /// A row rather than a mode: the panel is the same panel, and turning it
    /// on is 「I am going to change these」, not 「forget what I found」.
    pub replacing: bool,
    /// Whether this panel may change how many cells a row has — what the `!`
    /// of `:replace!` buys, and nothing else sets.
    ///
    /// The same permission `:s` spells `t`. It lives on the panel rather than
    /// on the one call because the panel is reopened by the command and then
    /// used many times: the writer said 「I mean it」 once, about this hunt.
    pub reshape: bool,
    /// Where to look.
    pub scope: Where,
    /// **那一格裏寫着的字** —— [`Field::Scope`] 的正文。
    ///
    /// 與 [`Self::scope`] 分開存，因為它們不是同一個東西：這是**打了一半的**，
    /// 那是**已經在找的**。Enter 纔把這個變成那個（`Editor::take_scope`）。
    pub scope_text: String,
    /// **只搜哪些文件** —— 逗號隔開的幾條 glob，見 [`Field::Include`]。
    pub include: String,
    /// **哪些文件不搜** —— 同上，見 [`Field::Exclude`]。
    pub exclude: String,
    /// 連隱藏文件一起搜。出廠關着，見 [`Field::Hidden`]。
    pub hidden: bool,
    /// 連 `.gitignore` 裏的一起搜。出廠關着，見 [`Field::Hidden`]。
    pub ignored: bool,
    /// **名單不封頂**。
    ///
    /// [`MOST`] 是給**面板**定的：「的」在一本小說裏是兩萬處，而名單是拿來走的，
    /// 没人走兩萬行。管道那一邊不是拿來走的——`ye --grep` 印給別的程序看，少印
    /// 一條就是錯一條。出廠是封頂的，編輯器裏的名單就是封頂的那一種。
    pub uncapped: bool,
    /// **包含或排除裏有一條寫錯了** —— 這一趟根本没跑。
    ///
    /// Warning: **數目那一格不許因此說「無結果」。** 零那一格最像「真的没有」，而這一
    /// 次連找都没找——狀態欄說了，標題也不能反着說。同 [`Search::cut`] 那一條。
    pub bad_glob: bool,
    /// **這一趟没走完就停了** —— 數目那一格要說出來。
    ///
    /// 見 `editor::WALK_GRACE`：走查有一個地板加一隻錶，碰到頭就交出半截的答
    /// 案。不說的話，一張短清單看着像是全部。
    pub cut: bool,
    /// **一個檔没搜，因為它就是這一趟輸出要去的地方**，名字在這裏。
    /// 同 [`Search::cut`]：漏掉一個檔，呼叫方有義務說出來。`ye --grep` 說在
    /// stderr 上；編輯器裏開不出這一種（它的輸出不是檔）。
    pub own_output: Option<String>,
    /// The files whose hits are folded away，按（名字, 號）這一對記，見 [`Row::File`]。
    pub folded: std::collections::BTreeSet<(std::path::PathBuf, Option<u64>)>,
    /// What the paths in [`Hit::file`] are relative to, so opening one can
    /// put it back together.
    pub root: Option<std::path::PathBuf>,
    /// **Whether the box has changed since a walk of the disk last ran.**
    ///
    /// Only ever true for a scope that is not live: it is what the panel says
    /// 「press Enter」 for, so that a stale list is never mistaken for the
    /// answer to what is in the box now.
    pub stale: bool,
    /// **哪一版正文跑出來的這張名單**——`(當前緩衝區的號, 所有緩衝區的改動次數
    /// 之和)`，没跑過就是 `None`。改過緩衝區再回到搜索按 `Enter`，名單要刷新。
    ///
    /// 對不上就是過期，`Editor::search_is_stale` 由它算出來。Warning: **不存「過期」這
    /// 個結論，存的是那一版的指紋**：結論要有人在正文改完的那一刻去改它，而改正文
    /// 的路有幾十條，漏一條就是一張看着新鮮的舊名單。
    ///
    /// 和數之和當指紋成立，是因為**改動次數只增不減**：兩次改動抵消不掉。
    /// 當前緩衝區的號也要記——本文件那一檔換了檔案，名單說的就是別人的事了。
    /// Warning: **搜文件夾也要看所有緩衝區**：開着的檔是從內存讀的，不是從磁盤
    /// （見 `search_now` 裏「An open file is read from its buffer」那一段）。
    pub looked_at: Option<Mark>,

    // 只重搜一份是按**檔名**在名單上找那一段（`Editor::the_run_of`），所以哪一份
    // 都換得了，也不必記着開搜索那一刻寫着的是哪一份。
    /// Where the caret is in it, in characters.
    pub caret: usize,
    /// **記着的那個詞，畫成灰字**——`空格 /` 什麼都没選中的時候留下的。
    ///
    /// 框**是空的**，這一串只是寫在那裏給人看：接着打字它自己就没了，按 `Tab`
    /// 把它收進框裏。和 `/` 那一行一個樣（`modes.rs` 的 `prompt_ghost`）。
    ///
    /// Warning: **不是框裏真的站着一段選中的文字。** 整條反白在這個倉裏別處是
    /// 「這一整段被選上了」，而這一刻要說的是「上次搜的是這個」——同一個樣子
    /// 說兩件事，讀的人會看不懂。
    pub ghost: String,
    /// Which cell has the keys.
    pub field: Field,
    /// **簡繁異字形**：「書齋」找得到「书斋」（見 [`crate::glyphs`]）。
    ///
    /// Warning: **出廠開着**，所以 `Search` 要走 [`Search::new`] 而不是 `default()`——
    /// `derive(Default)` 給不出「這一項是 true」。`default()` 留給測試。
    ///
    /// Warning: **正則底下也折。** 整串改寫確實會把使用者寫的 `.`、`*`、`[` 一起吃掉，所以
    /// 正則那一路走 [`crate::glyphs::widen_pattern`]：先把式子解析一遍，只動「原
    /// 樣打出來的那些字」。見 `Editor::search_pattern`。
    pub glyphs: bool,
    /// **拼音**：`shuzhai` 找得到「書齋」。出廠開着，見 [`Field::Pinyin`]。
    pub pinyin: bool,
    /// **`/` 那一行的「繁簡拼音通配」**（`A-j` 翻它）。
    ///
    /// 開着的時候 `/` 的式子折一道字形、再掛上讀音那一路——與這張面板上那兩個
    /// 勾同一套機制，只是面板分得開而那一行只有一個鍵。
    ///
    /// Warning: **出廠是關的，而面板上那兩個勾出廠是開的。**
    /// 兩個數不一樣不是忘了對齊：`/` 是最常按的那一個鍵，而拼音通配會讓
    /// `/de`、`/shi` 這種英文短詞連帶配上一堆漢字。那一行要的是「按一下才通配」。
    pub wild: bool,
    /// Read the pattern as a regular expression.
    pub regex: bool,
    /// How much case matters.
    pub case: Case,
    /// ASCII `\b` on both ends. Warning: **A no-op between 漢字** — there is no word
    /// boundary there — so it only ever bites on the Western words in a
    /// manuscript. Said in the manual rather than hidden.
    pub whole: bool,
    /// 「差不多是這幾個字」 — [`crate::nearby`] instead of a pattern.
    ///
    /// Warning: **It stands in place of 正則 and 完整匹配**, which are about a
    /// pattern and are drawn quiet while this is on; 大小寫 still applies.
    /// Never on while the panel is replacing (see [`Field::step`]).
    pub fuzzy: bool,
    /// What was found, at most [`MOST`] of them.
    pub hits: Vec<Hit>,
    /// How many there are altogether, however many are listed.
    pub total: usize,
    /// **每一個有命中的檔，一個不落**——`(檔名, 緩衝區號)`，和 [`Hit`] 同一對。
    ///
    /// Warning: **`R` 要的是這一張，不是 `hits`。** `hits` 封頂 [`MOST`] 條，而
    /// `total` 不封頂；拿 `hits` 推要動哪幾個檔的話，一本大書裏**排在第 500 處
    /// 之後的那幾個檔一個都不動**，而問句照着 `total` 問「把這 602 處全部換掉？」，
    /// 換完說「共替換 600 處」。一聲不吭地換了一半，正是 `R` 最不該犯的錯。
    ///
    /// 這張表按**檔**算，不按處算，所以它的長度是這棵樹裏有命中的檔數——封頂没有
    /// 意義，也不會大到哪裏去。
    pub files: Vec<(Option<std::path::PathBuf>, Option<u64>)>,
    /// Which hit the highlight is on.
    pub selected: usize,
    /// The pattern does not compile.
    ///
    /// Warning: The hits are **kept** when this is true, and drawn quiet: typing a
    /// regular expression walks through `[`, `(` and every other unfinished
    /// state, and emptying the list on each of them flickers. Quiet says
    /// 「not the answer to what is in the box」, which is the truth; blank
    /// would say 「nothing found」, which is not.
    pub broken: bool,
}

impl Search {
    /// 出廠的樣子——**簡繁異字形開着**，別的照 `default()`。
    ///
    /// 只多說一項，往後加字段不會漏；手寫一整個 `Default` 纔會。
    pub fn new() -> Search {
        Search { glyphs: true, pinyin: true, ..Search::default() }
    }

    /// Whether anything has been asked for yet.
    ///
    /// Warning: Not the same as 「found nothing」: an empty box has not been asked,
    /// and a panel that answered `0 處` to a question nobody put would be the
    /// `⟨缺⟩`-versus-blank mistake all over again.
    /// **這一趟留幾條。** 面板留 [`MOST`]，管道全留。
    pub fn most(&self) -> usize {
        match self.uncapped {
            true => usize::MAX,
            false => MOST,
        }
    }

    pub fn asked(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// **The rows to draw**, worked out from the hits.
    ///
    /// Flat when every hit is in the file being written — a header saying
    /// 「this file」 above the file you are looking at says nothing. A tree
    /// otherwise, in the order the files were walked.
    pub fn rows(&self) -> Vec<Row> {
        if self.hits.iter().all(|h| h.file.is_none()) {
            return (0..self.hits.len()).map(Row::Hit).collect();
        }
        let mut rows = Vec::new();
        let mut at: Option<(std::path::PathBuf, Option<u64>)> = None;
        for (i, hit) in self.hits.iter().enumerate() {
            let one = (
                hit.file.clone().unwrap_or_default(),
                hit.buffer,
            );
            if at.as_ref() != Some(&one) {
                at = Some(one.clone());
                let folded = self.folded.contains(&one);
                rows.push(Row::File {
                    hits: self
                        .hits
                        .iter()
                        .filter(|h| {
                            h.file.clone().unwrap_or_default() == one.0 && h.buffer == one.1
                        })
                        .count(),
                    path: one.0.clone(),
                    buffer: one.1,
                    folded,
                });
            }
            if !self.folded.contains(&one) {
                rows.push(Row::Hit(i));
            }
        }
        rows
    }

    /// The row the highlight is on.
    pub fn row(&self) -> Option<Row> {
        let rows = self.rows();
        rows.get(self.selected.min(rows.len().saturating_sub(1))).cloned()
    }

    /// The hit the highlight is on, if it is on one.
    pub fn here(&self) -> Option<&Hit> {
        match self.row()? {
            Row::Hit(i) => self.hits.get(i),
            Row::File { .. } => None,
        }
    }

    /// **站着的是第幾處**，從 1 數起；没站在命中上就是 `None`。
    ///
    /// 2026-09-27 報的：表頭只寫「11 處」，走到第幾條一個字都不說。跨檔的時候
    /// 名單裏夾着檔名那幾行，所以數的是**命中**，不是行。
    pub fn nth_hit(&self) -> Option<usize> {
        match self.row()? {
            Row::Hit(i) => Some(i + 1),
            Row::File { .. } => None,
        }
    }

    /// Move the highlight, stopping at the ends.
    pub fn step(&mut self, down: bool) {
        let last = self.rows().len().saturating_sub(1);
        self.selected = match down {
            true => self.selected.saturating_add(1).min(last),
            false => self.selected.saturating_sub(1),
        };
    }

    /// Fold the file the highlight is in, or open it again (`h`/`l`).
    ///
    /// On a hit rather than a header, `h` folds the file it belongs to and
    /// takes the highlight up to it — the same 「less of this」 the tree and
    /// the outline already mean by that key.
    pub fn fold(&mut self, away: bool) -> bool {
        let rows = self.rows();
        let Some(row) = rows.get(self.selected.min(rows.len().saturating_sub(1))) else {
            return false;
        };
        let one = match row {
            Row::File { path, buffer, .. } => (path.clone(), *buffer),
            Row::Hit(i) => match self.hits.get(*i) {
                Some(hit) => match hit.file.clone() {
                    Some(path) => (path, hit.buffer),
                    None => return false,
                },
                None => return false,
            },
        };
        let changed = match away {
            true => self.folded.insert(one.clone()),
            false => self.folded.remove(&one),
        };
        // Folding takes rows away; the highlight goes to the header rather
        // than sliding onto whatever filled the gap.
        if changed && away {
            if let Some(at) = self
                .rows()
                .iter()
                .position(|r| matches!(r, Row::File { path: p, buffer: b, .. } if (p.clone(), *b) == one))
            {
                self.selected = at;
            }
        }
        changed
    }

    /// Whichever box the keys are in.
    fn box_here(&mut self) -> &mut String {
        match self.field {
            Field::Replace => &mut self.replace,
            Field::Include => &mut self.include,
            Field::Exclude => &mut self.exclude,
            _ => &mut self.query,
        }
    }

    /// Type a character into the box, replacing all of it if it is selected.
    pub fn type_char(&mut self, ch: char) {
        let at = self.byte_at(self.caret);
        self.box_here().insert(at, ch);
        self.caret += 1;
    }

    /// The same for a whole committed string — what the IME hands over.
    pub fn type_text(&mut self, text: &str) {
        let at = self.byte_at(self.caret);
        self.box_here().insert_str(at, text);
        self.caret += text.chars().count();
    }

    /// Backspace: 光標前面那一個字。
    pub fn backspace(&mut self) {
        if self.caret == 0 {
            return;
        }
        let to = self.byte_at(self.caret - 1);
        let from = self.byte_at(self.caret);
        self.box_here().replace_range(to..from, "");
        self.caret -= 1;
    }

    /// **刪掉光標壓着的那一個字**——框裏的 `d`。
    ///
    /// 光標停在末尾（没壓着字）就什麼都不做：那裏没有東西可刪，而「往回刪一個」
    /// 是 `Backspace` 說的另一件事。
    pub fn delete_here(&mut self) {
        // Warning: **光標停在末尾那個空位上的時候，刪的是它前面那一個**
        // （2026-09-27 兩個試用的人都報「`d` 按了什麼都不發生」）。框裏的光標走
        // 得到文字後面那一格（打字要從那裏接着打），而 `Esc` 出來之後它多半就停
        // 在那裏——於是「刪光標壓着的那一個」壓着的是空氣，鍵位行上明明寫着
        // 「d 刪」。按的人想刪的是看得見的最後那個字。
        if self.caret >= self.box_here().chars().count() {
            let Some(back) = self.caret.checked_sub(1) else {
                return;
            };
            let from = self.byte_at(back);
            let to = self.byte_at(self.caret);
            self.box_here().replace_range(from..to, "");
            self.caret = back;
            return;
        }
        let from = self.byte_at(self.caret);
        let to = self.byte_at(self.caret + 1);
        if from == to {
            return;
        }
        self.box_here().replace_range(from..to, "");
    }

    /// The word before the caret and the spaces after it (`C-w`), as on the
    /// `:` line.
    pub fn delete_word_before_caret(&mut self) {
        let caret = self.caret;
        let head: String = self.box_here().chars().take(caret).collect();
        let cut = crate::editor::word_cut(&head);
        let (from, to) = (self.byte_at(cut), self.byte_at(self.caret));
        self.box_here().replace_range(from..to, "");
        self.caret = cut;
    }

    /// **從光標刪到行尾**——框裏的 `D`。
    ///
    /// Warning: **末尾那個空位上照樣刪得掉最後一個字**（2026-10-02 補，同
    /// [`Search::delete_here`] 2026-09-27 那條）。從前這一支在那裏是 `truncate(len)`
    /// ＝什麼都不做，而同一個位置上 `d` 刪得掉——**兩個鍵對同一個光標位置給出
    /// 兩種答案**，而屏幕上那一行寫着「dD 刪除」。
    ///
    /// 道理是 `d` 那一條的原話：框裏的光標走得到文字後面那一格（打字要從那裏接
    /// 着打），`Esc` 出來多半就停在那裏，而按的人想刪的是看得見的最後那個字。
    /// vi 的 Normal 態根本没有「末尾後面那一格」，`D` 停在最後一個字上就是刪掉它。
    pub fn delete_to_end(&mut self) {
        let last = self.box_here().chars().count();
        let at = match self.caret >= last {
            true => self.caret.saturating_sub(1),
            false => self.caret,
        };
        self.caret = at;
        let from = self.byte_at(at);
        self.box_here().truncate(from);
    }

    /// What is in the box the keys are in.
    /// **這一格現在打得了字嗎。**
    ///
    /// 「搜」和「換」永遠打得了。「包含」「排除」只在範圍走磁碟的時候打得了。
    /// 「位置」永遠打不了：它是四選一，按號碼換一檔，而指定
    /// 文件夾只能 `:search 某目錄` 進來。
    pub fn takes_text(&self) -> bool {
        match self.field {
            Field::Include | Field::Exclude => self.on_disk(),
            other => other.takes_text(),
        }
    }

    /// **現在這一檔走不走磁碟** —— 包含/排除/搜索隱藏和忽略 算不算數。
    pub fn on_disk(&self) -> bool {
        self.scope.walks_the_disk()
    }

    /// **位置那一格現在是不是一條打出來的路徑。**
    ///
    /// 只剩畫的時候用得上（那一行要印出路徑而不是一個檔名），面板裏改不了它。
    pub fn naming(&self) -> bool {
        matches!(self.scope, Where::Named(_))
    }

    pub fn typed(&self) -> &str {
        match self.field {
            Field::Replace => &self.replace,
            Field::Include => &self.include,
            Field::Exclude => &self.exclude,
            _ => &self.query,
        }
    }

    /// Move the caret.
    pub fn move_caret(&mut self, to: usize) {
        self.caret = to.min(self.typed().chars().count());
    }

    /// **Walk to another cell, and park the cursor at the end of it.**
    ///
    /// Warning: **One caret serves every box**, so walking off 搜 (caret at 3) on to
    /// an empty 換 would leave the block cursor sitting three cells past the
    /// end of a box with nothing in it. The caret is the *current* box's, and
    /// changing which box that is has to move it (2026-09-25, when `h`/`l`
    /// started meaning 「走字」 and the caret became something a reader can see).
    pub fn stand_on(&mut self, field: Field) {
        self.field = field;
        self.caret = self.typed().chars().count();
    }

    /// **走一格** —— `jk`/`Tab`/`↑↓` 都問這一支。
    ///
    /// 四件事決定哪幾格停得住（換框在不在、走不走磁碟、名單空不空、是不是開關），
    /// 包在這裏一處，免得每個按鍵自己拼一遍參數——拼漏一個就是一條走法和別人不同。
    pub fn walk(&self, back: bool) -> Field {
        self.field.step(back, self.replacing, self.on_disk(), !self.hits.is_empty())
    }

    /// **開一扇空框，記着的那個詞寫成灰字** —— `空格 /` 什麼都没選中的時候。
    pub fn guess(&mut self, remembered: String) {
        self.query.clear();
        self.caret = 0;
        self.ghost = remembered;
        self.field = Field::Query;
    }

    /// **把剛選中的那段擺進框裏** —— `空格 /` 選着一段的時候。
    ///
    /// Warning: **選中的照填，記着的變灰**。你剛選中的那一段是你
    /// 此刻做的動作，直接進框、光標在末尾；上次搜過的詞只是一個猜測，所以是灰
    /// 的。`/` 那一行的規矩也是這一條——它没有「選中」這個來源，所以只剩灰字。
    pub fn ask(&mut self, query: String) {
        self.caret = query.chars().count();
        self.query = query;
        self.ghost.clear();
        self.field = Field::Query;
    }

    /// 把灰字收進框裏——`Tab`。回 `true` ＝ 真收了一個。
    pub fn adopt_ghost(&mut self) -> bool {
        if self.ghost.is_empty() || !self.query.is_empty() || self.field != Field::Query {
            return false;
        }
        self.query = std::mem::take(&mut self.ghost);
        self.caret = self.query.chars().count();
        true
    }

    /// 灰字現在畫不畫得出來。
    ///
    /// Warning: **一打字它就没了**，所以問的是「框還空着嗎」，不是「存了没有」——
    /// 同 `prompt_ghost`（那一支問的是 `command_line.is_empty()`）。
    pub fn ghost_now(&self) -> &str {
        match self.field == Field::Query && self.query.is_empty() {
            true => &self.ghost,
            false => "",
        }
    }

    /// Where character `at` starts, in bytes.
    fn byte_at(&self, at: usize) -> usize {
        let text = self.typed();
        text.char_indices()
            .nth(at)
            .map(|(i, _)| i)
            .unwrap_or(text.len())
    }
}
