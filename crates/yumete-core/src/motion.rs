//! Cursor motions over a [`ropey::Rope`] — Features #6 (h/j/k/l) and #7
//! (0/$/^/gg/G).
//!
//! Every function takes a rope and a cursor position as a **character index**
//! and returns a new character index. Horizontal motion is grapheme-aware and
//! vertical motion preserves the cursor's **visual column** (summed display
//! width), so wide CJK glyphs, combining marks, and IVS behave correctly. The
//! width and grapheme primitives come from [`yumete_cjk`].
//!
//! Motions operate on one line at a time, materializing that line's text (prose
//! lines are short). A very long single line could later use an incremental
//! grapheme cursor over the rope's chunks; that optimization is deferred.

use ropey::Rope;
use yumete_cjk::{
    grapheme_width, graphemes, next_grapheme_boundary, prev_grapheme_boundary, Segmenter,
};

/// The line's text without its trailing line break.
pub(crate) fn line_text(rope: &Rope, line: usize) -> String {
    let mut s = rope.line(line).to_string();
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
    s
}

/// Byte offset of the `char_col`-th character in `s` (or `s.len()` at the end).
fn byte_of_col(s: &str, char_col: usize) -> usize {
    s.char_indices()
        .nth(char_col)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// Number of characters in `s` up to (not including) byte offset `byte`.
fn col_of_byte(s: &str, byte: usize) -> usize {
    s[..byte].chars().count()
}

/// The number of characters on `line` (excluding the trailing line break).
///
/// Warning: **不抄整行**（2026-10-08）。從前寫的是
/// `line_text(rope, line).chars().count()`——為了數一個數字，把一行一千兩百萬字
/// 的行抄成一個 37 MB 的 `String`，而 `line_end` 問的就是這個數，於是每一下詞動
/// 作都抄一次。`ropey` 的行切片自己報得出字數，剩下的只是把尾巴上那一兩格扣掉，
/// 扣的與 `line_text` 一樣（先 `\n`，再它前面的 `\r`），所以答案逐位相同。
pub(crate) fn line_char_len(rope: &Rope, line: usize) -> usize {
    let slice = rope.line(line);
    let mut n = slice.len_chars();
    if n > 0 && slice.char(n - 1) == '\n' {
        n -= 1;
        if n > 0 && slice.char(n - 1) == '\r' {
            n -= 1;
        }
    }
    n
}

/// The first character **after** this line's break — where the next line starts.
///
/// ⚠ **A line break is not always one character.** [`line_end`] is one past
/// the line's last *character*, and [`line_text`] strips `\r` as well as `\n`
/// — so on a CRLF file `line_end` stands on the `\r`, and `line_end + 1`
/// stands between the two halves of one break. Two joins did exactly that
/// (2026-10-07, 丟字第二輪): `J` and `gJ` deleted the `\r` and left the `\n`,
/// so a CRLF manuscript came back with one line in LF and the two lines not
/// joined at all. Ask for this instead of adding one.
///
/// On the last line, where there is no break, this is the end of the rope.
pub fn past_the_break(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    match line + 1 < rope.len_lines() {
        true => rope.line_to_char(line + 1),
        false => rope.len_chars(),
    }
}

/// The last line the cursor may sit on.
///
/// `ropey` reports a trailing empty line when the text ends with a newline;
/// that phantom line is excluded here so the cursor can't fall past the content.
pub fn last_line(rope: &Rope) -> usize {
    let lines = rope.len_lines();
    let n = rope.len_chars();
    if n > 0 && rope.char(n - 1) == '\n' {
        lines.saturating_sub(2)
    } else {
        lines.saturating_sub(1)
    }
}

/// One grapheme to the left, staying within the current line.
pub fn left(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    let ls = rope.line_to_char(line);
    let col = pos - ls;
    if col == 0 {
        return pos;
    }
    let text = line_text(rope, line);
    let byte = byte_of_col(&text, col);
    let prev = prev_grapheme_boundary(&text, byte);
    ls + col_of_byte(&text, prev)
}

/// One grapheme to the right, staying within the current line.
pub fn right(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    let ls = rope.line_to_char(line);
    let text = line_text(rope, line);
    let col = pos - ls;
    if col >= text.chars().count() {
        return pos;
    }
    let byte = byte_of_col(&text, col);
    let next = next_grapheme_boundary(&text, byte);
    ls + col_of_byte(&text, next)
}

/// One grapheme to the right **in the buffer**, stepping over a line break
/// onto the next line.
///
/// [`right`] stops at the end of a line, because a line's own measurements —
/// where a cell ends, how far a row reaches — must not walk off it. Everything
/// a *reader* moves with is this one: `l`, the right arrow, and the selection,
/// whose grapheme under the cursor has its far edge on the next line when the
/// cursor is on the last character of this one.
pub fn next_grapheme(rope: &Rope, pos: usize) -> usize {
    let stepped = right(rope, pos);
    if stepped != pos {
        return stepped;
    }
    let line = rope.char_to_line(pos);
    if line + 1 < rope.len_lines() {
        rope.line_to_char(line + 1)
    } else {
        rope.len_chars().max(pos)
    }
}

/// One grapheme to the left **in the buffer**, stepping back over a line break.
///
/// Lands on the *start* of the break, so a CRLF pair is treated as the one
/// grapheme it is rather than being split down the middle.
pub fn prev_grapheme(rope: &Rope, pos: usize) -> usize {
    let stepped = left(rope, pos);
    if stepped != pos {
        return stepped;
    }
    let line = rope.char_to_line(pos);
    if line == 0 {
        return pos;
    }
    let previous = line - 1;
    rope.line_to_char(previous) + line_text(rope, previous).chars().count()
}

/// The visual column (summed display width) of `pos` within its line.
pub fn visual_column(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    let ls = rope.line_to_char(line);
    let col = pos - ls;
    // Warning: **只拿光標前面那一截**（2026-10-08）。下面那個迴圈 `chars >= col`
    // 就斷，所以後面的字一個也用不著；可 `line_text` 把**整行**物化成 `String`，
    // 而這一支是狀態欄與光標每幀都問的——一行 150 萬字的檔上就是每幀一次
    // 4.5 MB 的拷貝（`sample` 指的就是這裏）。
    //
    // 多拿 64 個字是給**跨過光標那一個字簇**留的余地：迴圈收的是「起點在 `col`
    // 之前」的字簇，而一個字簇要整個在手裏才量得出寬度。真正的文字裏一個字簇
    // 兩三個碼位就到頂，64 是十幾倍的餘量。
    let end = rope.line_to_char(line) + rope.line(line).len_chars();
    let text: String = rope.slice(ls..end.min(ls + col + 64)).to_string();
    let mut width = 0;
    let mut chars = 0;
    for g in graphemes(&text) {
        if chars >= col {
            break;
        }
        chars += g.chars().count();
        width += grapheme_width(g);
    }
    width
}

/// The character index on `line` whose starting visual column is the greatest
/// one not exceeding `goal` (used to preserve the column on vertical motion,
/// and by vim's `|`).
pub fn pos_at_visual_column(rope: &Rope, line: usize, goal: usize) -> usize {
    let ls = rope.line_to_char(line);
    let text = line_text(rope, line);
    let mut width = 0;
    let mut chars = 0;
    for g in graphemes(&text) {
        let gw = grapheme_width(g);
        if width + gw > goal {
            break;
        }
        width += gw;
        chars += g.chars().count();
    }
    ls + chars
}

/// Up one line, keeping the visual column `goal`.
pub fn up(rope: &Rope, pos: usize, goal: usize) -> usize {
    let line = rope.char_to_line(pos);
    if line == 0 {
        return pos;
    }
    pos_at_visual_column(rope, line - 1, goal)
}

/// Down one line, keeping the visual column `goal`.
pub fn down(rope: &Rope, pos: usize, goal: usize) -> usize {
    let line = rope.char_to_line(pos);
    if line >= last_line(rope) {
        return pos;
    }
    pos_at_visual_column(rope, line + 1, goal)
}

/// The first character of the cursor's line (`0`).
pub fn line_start(rope: &Rope, pos: usize) -> usize {
    rope.line_to_char(rope.char_to_line(pos))
}

/// The first non-blank character of the cursor's line (`^`), or the line start
/// if the line is all blanks or empty.
///
/// Warning: **全是空白的那一行回行首，不是行尾**（2026-10-02 查出來的）。上面那句話
/// 一直是這麼寫的，而代碼走完所有空白就停——在一行只有空白的行上，那個位置是
/// **換行符**。站在換行符上，`selection()` 會把下一個字素也算進來（那是下一行的
/// 第一個字），於是 `x` 選中兩行，`xd` 把下面那一行一起刪了。又是 #382 那個形狀。
///
/// helix 的 `goto_first_nonwhitespace_impl` 在 `first_non_whitespace_char()` 回
/// `None` 的時候**原地不動**。
///
/// Warning: **全角空格也是空白**（同日）。從前只認 ASCII 的空格和製表符，而這是一個
/// 寫中文稿子的編輯器——`　` 縮進的那一行，`gs` 跳到第 1 欄，ASCII 縮進的跳到第 3
/// 欄，同一個鍵兩種答案。照 helix 用 `char::is_whitespace`（它收 U+3000、U+00A0、
/// U+2000–U+200A 那一族）。
pub fn line_first_non_blank(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    let ls = rope.line_to_char(line);
    let text = line_text(rope, line);
    let mut chars = 0;
    for g in graphemes(&text) {
        if g.chars().all(|c| c.is_whitespace() && c != '\n' && c != '\r') {
            chars += g.chars().count();
        } else {
            return ls + chars;
        }
    }
    // 走到頭都沒碰上一個實字：這一行沒有「第一個非空白」，所以不動。
    ls
}

/// The end of the cursor's line (`$`) — one position past the last character.
pub fn line_end(rope: &Rope, pos: usize) -> usize {
    let line = rope.char_to_line(pos);
    rope.line_to_char(line) + line_char_len(rope, line)
}

/// Where the caret **stands** at the end of a line (`gl`, `End`).
///
/// [`line_end`] is the *insert* point — one position past the last character,
/// which is where `A` belongs and where no character is. A Normal-mode caret
/// left on it is standing on the newline, and then every verb aims at the line
/// break instead of at the writing: `a` opened on the next line, `d` ate the
/// break and welded two lines into one (#382). So a motion that leaves the
/// caret somewhere asks for this, and only the ones that open Insert ask for
/// [`line_end`].
///
/// An empty line has no last character; there the two are the same place.
pub fn line_last(rope: &Rope, pos: usize) -> usize {
    let start = line_start(rope, pos);
    let end = line_end(rope, pos);
    match end > start {
        // `.max(start)` because `prev_grapheme` walks to the previous line when
        // it cannot step within this one.
        true => prev_grapheme(rope, end).max(start),
        false => start,
    }
}

/// The start of the buffer (`gg`).
pub fn buffer_start(_rope: &Rope, _pos: usize) -> usize {
    0
}

/// The start of the last line of the buffer (`G`).
pub fn buffer_end(rope: &Rope, _pos: usize) -> usize {
    rope.line_to_char(last_line(rope))
}

/// The word ranges of one line, as absolute character indices.
///
/// **One line, not the buffer.** Word boundaries never cross a line break —
/// a newline separates words for the whitespace rule and breaks a run of 漢字
/// for the dictionary — so a motion only ever needs the line it is on and, at
/// worst, the next one. Segmenting the whole document to find the next word
/// costs a full pass over the text for every press of `w`, which on a novel is
/// most of a second and leaves the key queue running long after the key is let
/// go.
///
/// Three grains, because three keys mean three different things (#304):
///
/// | | what a word is | who asks |
/// | --- | --- | --- |
/// | [`Grain::Big`] | a run of non-whitespace | `W` `B` `E` |
/// | [`Grain::Word`] | whatever `seg` says | `w` `b` |
/// | [`Grain::Coarse`] | a run of one category, 漢字 as letters | `e` |
///
/// `Coarse` is not a third opinion for its own sake. Chinese has no spaces, so
/// a `w` and an `e` that both consult the dictionary do nearly the same thing;
/// left coarse, `e` runs to the next punctuation. **`w` takes a word, `e` takes
/// a clause.** It is also what `w` itself falls back to when the dictionary is
/// switched off ([`WordLevel::Off`](yumete_cjk::WordLevel::Off)) — one grain,
/// two callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grain {
    /// `W` `B` `E`: split on whitespace and nothing else.
    Big,
    /// `w` `b`: whatever the segmenter says.
    Word,
    /// `e`: one category at a time, with a 漢字 counting as a letter.
    Coarse,
}

/// Warning: **詞動作不問這一支，問 [`near_words`]**（2026-10-08）。整行一份答案
/// 在一行一千兩百萬字的檔上就是整份文件一份答案，而一下 `e` 只用得上其中一條。
/// 畫面上的東西（跳轉標籤、`miw`、`*` 取光標下那個詞）還是問這一支：它們要的本
/// 來就是一整行。
pub fn line_words(rope: &Rope, line: usize, grain: Grain, seg: &dyn Segmenter) -> Vec<(usize, usize)> {
    let start = rope.line_to_char(line);
    let text = line_text(rope, line);
    segment_text(&text, grain, seg)
        .into_iter()
        .map(|(a, b)| (start + a, start + b))
        .collect()
}

/// 一段文本按某個粒度切出來的詞範圍（相對這段文本）。
fn segment_text(text: &str, grain: Grain, seg: &dyn Segmenter) -> Vec<(usize, usize)> {
    match grain {
        Grain::Big => yumete_cjk::word_ranges_big(text),
        Grain::Coarse => yumete_cjk::word_ranges_coarse(text),
        Grain::Word => seg.segment(text),
    }
}

// ---- 長行上的詞動作：只切光標附近那一段（2026-10-08） ----------------------

/// 一次詞動作往兩邊各看多少個字元。
///
/// **量出來的病**（2026-10-08，§5.126 ② 留下的那一條）：一行 12,419,022 字的檔
/// 上，`e` 每一下 287 ms、`w` 272 ms、`b` 379 ms，而 `l` 只要 5.7 ms——
/// [`line_words`] 把整行抄成一個 37 MB 的 `String`，分完一千兩百萬字的詞，然後
/// 用掉其中一條。helix 那一邊是 `slice.chars_at(head)` 往前走、碰上第一個邊界就
/// `break`（`helix-core/src/movement.rs:255` 與 `:468-480`），代價跟着**走了多遠**
/// 算，不跟着行長算。這一節要的是同一件事，差別只在漢語的分詞器要的是一段上下
/// 文，不是一個遊標。
///
/// 4096 的來由：這是**一下按出去走多遠**的量，不是上下文的量（那是
/// [`WORD_SEAM`]）。一扇窗（8192 字）走一趟 Viterbi 是十微秒的量級，按住鍵也追得
/// 上一幀；而一下走過四千字的動作不是人在稿子裏按的——[`near_words`] 推窗那條路是
/// 給病態文本留的，不是給正文留的。
const WORD_REACH: usize = 4096;

/// 推窗的時候兩扇窗重疊多少——**一條邊界要多少上下文纔算得準**。
///
/// 窗邊上那條邊界不可信，所以下一扇窗要把它罩進裏面去（[`Window`] 的註釋）；重疊
/// 多少就是那條邊界在新窗裏有多少左文。512 比詞典裏最長的詞（`max_len`，幾個字）
/// 多兩個數量級，而逐字掃的那兩種粒度（`Big`/`Coarse`）只看相鄰兩個字。
///
/// Warning: **別把這個數和 [`WORD_REACH`] 合成一個。** 重疊取 `WORD_REACH` 的時候
/// 推一次只前進半扇窗，於是「一行裏一個空白都沒有」那種檔上 `W` 要把整行分兩遍
/// （量出來和整行分詞一樣慢，49.8 ms 一鍵，白推）。分開之後推一次前進
/// `2 * WORD_REACH - WORD_SEAM`，整行總共只多分 6%。
const WORD_SEAM: usize = 512;

/// 比這麼長的行纔走窗口；到這個長度為止照舊整行分詞。
///
/// `2 * WORD_REACH` 以下的行，窗口本來就是整行，所以這個門檻只是在說「從哪裏開
/// 始算長行」。取四倍半徑：到了這個長度窗口已經比整行便宜一半，而一行一萬六千
/// 字的「段落」不是稿子裏有的東西（那是幾十頁書擠成一段）——**正文走的永遠是從
/// 前那條路，一次整行，答案與開銷逐位相同**。
const WORD_WINDOW_LINE: usize = 4 * WORD_REACH;

/// 一扇窗裏的詞範圍，絕對字元索引。
///
/// Warning: **兩頭那個邊界是窗切出來的，不算。** 窗邊很可能落在一個詞的中間，於
/// 是第一條的**起點**與最後一條的**終點**都可能是假的，而一個可能是錯的邊界一次
/// 都不許交出去——所以問的人拿到的是 [`Window::starts`] 與 [`Window::ended`]，它
/// 們各自把那一頭摘掉。中間那些是真的：`word_ranges_big`/`word_ranges_coarse` 是
/// 逐字掃出來的連續段（窗口改不了它們的內部邊界），詞典那一支的最大概率路徑在幾個
/// 字之內就重新對上——「分词是非贪婪的。只需要看某个区间就行」，那是這一整節的前提。
/// 被丟掉的那條邊界不會就此失傳：[`near_words`] 推下一扇窗的時候重疊 [`WORD_SEAM`]，
/// 於是它在新窗裏離窗邊有五百個字。
struct Window {
    /// 窗裏切出來的詞，絕對索引。
    words: Vec<(usize, usize)>,
    /// 窗的兩頭。
    lo: usize,
    hi: usize,
    /// 行的兩頭（不含換行）。窗的那一頭正好落在這上面，那一頭的邊界就是真的。
    line: (usize, usize),
    /// **窗左邊外頭最後一條詞的終點**，不知道就是 `None`。
    ///
    /// 窗往前推的時候，被丟掉那一扇裏「終點不超過新 `lo`」的那幾條還是真的，
    /// 最靠右的那一條就是這一格——[`Window::leading`] 要的正是它。
    prior: Option<usize>,
}

impl Window {
    /// 整行就是一扇窗——短行走的這一條，兩頭都是真邊界。
    fn whole(words: Vec<(usize, usize)>, line: (usize, usize)) -> Window {
        Window { words, lo: line.0, hi: line.1, line, prior: None }
    }

    /// 只切 `[lo, hi]` 這一段。
    fn cut(
        rope: &Rope,
        line: (usize, usize),
        lo: usize,
        hi: usize,
        grain: Grain,
        seg: &dyn Segmenter,
        prior: Option<usize>,
    ) -> Window {
        let text = rope.slice(lo..hi).to_string();
        let words = segment_text(&text, grain, seg)
            .into_iter()
            .map(|(a, b)| (lo + a, lo + b))
            .collect();
        Window { words, lo, hi, line, prior }
    }

    /// 這扇窗罩着 `pos` 嗎——也就是「`pos` 在不在詞裏」這種問題它答不答得出。
    fn holds(&self, pos: usize) -> bool {
        (self.lo..=self.hi).contains(&pos)
    }

    /// **起點可信**的那幾條的起點。
    fn starts(&self) -> impl DoubleEndedIterator<Item = usize> + '_ {
        let cut = self.lo > self.line.0 && self.words.first().is_some_and(|&(s, _)| s == self.lo);
        self.words[usize::from(cut)..].iter().map(|&(s, _)| s)
    }

    /// **終點可信**的那幾條，整條交出去——`e` 還要問它前面那一條的終點。
    fn ended(&self) -> &[(usize, usize)] {
        let cut = self.hi < self.line.1 && self.words.last().is_some_and(|&(_, e)| e == self.hi);
        &self.words[..self.words.len() - usize::from(cut)]
    }

    /// **第 `k` 條詞前面那條詞的終點** —— `e` 要的那半個邊界（見 [`next_word_end`]）。
    ///
    /// Warning: **`k == 0` 的時候不許拿行首頂替**（2026-10-09）。窗是往前推出來的
    /// 時候，第一條詞前面那條詞落在窗外，而它的終點**可能在 `pos` 後面**：光標正站
    /// 在一個詞的末字上、下一個詞的尾巴又遠在這扇窗之外，那條邊界就是 `pos + 1`。
    /// 拿行首頂替會讓 `e` 把光標那個字一起收進去，`de` 就多吃一個字。被丟掉的那一
    /// 扇窗替我們記住了它（[`Window::prior`]）；真的誰都不知道的時候，那條邊界必定
    /// 在 `pos` 前面（不然推窗之前就問出來了），行首與它同一個答案。
    fn leading(&self, k: usize) -> usize {
        match k {
            0 => self.prior.unwrap_or(self.line.0),
            _ => self.words[k - 1].1,
        }
    }
}

/// **在 `pos` 附近一段一段地問，問到有答案為止**（2026-10-08）。
///
/// 第一扇窗是 `[pos - WORD_REACH, pos + WORD_REACH]`，夾在這一行裏；窗裏沒有答
/// 案就往 `forward` 的方向再推一扇，推到行的那一頭為止。
///
/// Warning: **推，不是停下來。** 一行裏一個空白都沒有的時候，`E` 真要走到行尾，
/// 而那段路一扇窗裝不下；窗裏找不着邊界就當成「沒有」交出去，光標會卡在原地。
/// 兩扇窗重疊 [`WORD_SEAM`]：上一扇窗邊上那條不可信的邊界，在新窗裏離窗邊有五百
/// 個字，是中間那些可信的邊界之一。
///
/// Warning: **短行只問一次，窗就是整行**，於是答案與開銷都和從前整行分詞那一版
/// 逐位相同——稿子走的是這一條。
fn near_words<T>(
    rope: &Rope,
    line: usize,
    pos: usize,
    grain: Grain,
    seg: &dyn Segmenter,
    forward: bool,
    mut pick: impl FnMut(&Window) -> Option<T>,
) -> Option<T> {
    let ls = rope.line_to_char(line);
    let le = ls + line_char_len(rope, line);
    if le - ls <= WORD_WINDOW_LINE {
        return pick(&Window::whole(line_words(rope, line, grain, seg), (ls, le)));
    }
    // 問的人可能站在別的行上（`b` 往回翻行的時候），那就從這一行的那一頭問起。
    let pos = pos.clamp(ls, le);
    let mut lo = pos.saturating_sub(WORD_REACH).max(ls);
    let mut hi = (pos + WORD_REACH).min(le);
    let mut prior = None;
    loop {
        let window = Window::cut(rope, (ls, le), lo, hi, grain, seg, prior);
        if let Some(found) = pick(&window) {
            return Some(found);
        }
        match forward {
            true if hi < le => {
                lo = hi - WORD_SEAM;
                hi = (lo + 2 * WORD_REACH).min(le);
                // 丟掉這一扇之前，把它左半邊那條真邊界留下來（`Window::prior`）。
                prior = window.words.iter().map(|&(_, e)| e).filter(|&e| e <= lo).max().or(prior);
            }
            false if lo > ls => {
                hi = lo + WORD_SEAM;
                lo = hi.saturating_sub(2 * WORD_REACH).max(ls);
                // 往回推的時候窗的左邊往外挪，上一扇記下的那條邊界就不在窗外了。
                prior = None;
            }
            // 推到行頭了：這一行真的沒有。
            _ => return None,
        }
    }
}

/// The line `pos` sits on, clamped into the buffer.
fn line_of(rope: &Rope, pos: usize) -> usize {
    rope.char_to_line(pos.min(rope.len_chars()))
}

/// The start of the next word after `pos` (`w` / `W`).
///
/// Warning: **這一支是故意跨行的**，見 `word_motions_still_cross_lines`。vim 的 `dw`
/// 直接用它（`[pos, next_word_start)`），而「行末的 `dw` 不許吃掉換行」那一條
/// （§5.11 B3）是在 vim 那一邊擋的。helix 那一邊的規矩在 [`unit_forward`] 裏。
pub fn next_word_start(rope: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> usize {
    for line in line_of(rope, pos)..rope.len_lines() {
        if let Some(start) =
            near_words(rope, line, pos, grain, seg, true, |w| w.starts().find(|&start| start > pos))
        {
            return start;
        }
    }
    rope.len_chars()
}

/// **What a motion asks the page for** — the first of the grammar layer
/// (B1, 2026-09-20).
///
/// A motion used to *move the caret itself*, which is why 「the same motion」
/// could not be shared between a selection-first grammar (helix: the motion
/// makes a selection, a verb eats it) and a caret-first one (vim: a verb
/// waits, the motion hands it a range). Made a **value**, one motion serves
/// both — and the difference between the two editors shrinks to who reads
/// which part of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    /// The span a motion covers, both ends inclusive of `head`.
    Over { anchor: usize, head: usize },
    /// The motion found nothing. Warning: **Not an empty span**: a verb must be able
    /// to tell 「nothing there」 from 「a span of one」 and do nothing at all —
    /// that distinction is what `wdiw` at the end of a buffer turned on.
    Missed,
}

/// **`w`, as a span** — the rule helix's word motion has always followed,
/// lifted out of the editor so that both grammars can read it (B1).
///
/// The rule, and it is subtler than 「select to the next word's start」:
///
/// > **A `w` never hands back a span of just the cell you are already on.**
/// > If 「to the next word's start」 would do that, it takes the *next* word
/// > instead.
///
/// One rule, two surfaces, and the difference is that **Chinese has no
/// spaces** (measured 2026-09-20):
///
/// | 文 | 光標在 | `w` 選到 |
/// | --- | --- | --- |
/// | `alpha beta` | `a`（詞首） | `alpha␣` —— 本詞連同後面的空隙 |
/// | `alpha beta` | 第二個 `a`（詞的最後一個字） | `a␣` —— 仍是本詞詞尾 |
/// | `alpha beta` | 空隙 | `beta␣` —— **下一個詞** |
/// | `今天天氣` | 今（詞首） | `今天` |
/// | `今天天氣` | 天（詞的最後一個字） | `天氣` —— **下一個詞** |
///
/// 西文裏「再選就只剩自己這一格」發生在空隙上；中文没有空隙，所以它發生在詞的
/// 最後一個字上。同一條規則。
///
/// Warning: **vim does not use this.** Its `w` is the bare primitive
/// ([`next_word_start`]): the caret goes there, and `dw` operates on
/// `[pos, next_word_start)`. That is the whole of why the two feel different,
/// and why a translation table could never say it.
pub fn word_forward(rope: &Rope, from: usize, grain: Grain, seg: &dyn Segmenter) -> Span {
    // Warning: **一次 `w` 不跨換行**（2026-10-02 照 helix 比出來的，會焊行）。
    //
    // 從前行末按 `w` 選中的是「本行剩下的字 ＋ 換行 ＋ 下一行的縮進」，一個 `d`
    // 就把兩行焊成一行。§5.11 B3 記過這個症狀（「`dw` 在行末吃掉換行**和下一行的
    // 縮進**」），可那一次只改了 vim 那一套文法，helix 這一套留着。
    //
    // helix 的 `range_to_target` 把換行當成一個**目標**（`movement.rs:470`）：`w`
    // 停在行末，再按一下纔過去；真跨過去的時候 `anchor = head`，所以選中的裏面
    // 永遠沒有那個換行。實測（`helix-core` 079a789e8，`alpha beta gamma` 那一行）：
    // 第 14 格按 `w` 選的是 `ma`，第 15 格按 `w` 選的是下一行的縮進 `  `。
    //
    // Warning: **這兩條只給 `w` 用。** [`next_word_start`] 是 vim 的 `dw` 直接用的
    // 原件，故意跨行（`word_motions_still_cross_lines`）；[`unit_forward`] 還服務
    // 段落和句子，而那兩種本來就該跨行。
    let stop_at_line_end = |r: &Rope, p: usize| -> usize {
        let line = line_of(r, p);
        if let Some(start) =
            near_words(r, line, p, grain, seg, true, |w| w.starts().find(|&start| start > p))
        {
            return start;
        }
        let end = line_end(r, r.line_to_char(line));
        match end > p {
            true => end,
            // 已經在行末了：這一下纔走到下一行。
            false => next_word_start(r, p, grain, seg),
        }
    };
    match unit_forward(rope, from, stop_at_line_end) {
        Span::Over { anchor, head } if rope.char_to_line(anchor) != rope.char_to_line(head) => {
            Span::Over { anchor: line_start(rope, head), head }
        }
        other => other,
    }
}

/// **The forward rule itself**, over whatever unit `next` counts (B1).
///
/// Warning: **It was written three times** — for words, for paragraphs (`}`) and for
/// sentences (`L`) — three copies of the same nine lines. They are one rule,
/// and [`word_forward`]'s doc comment is where it is spelled out: *a forward
/// motion never hands back a span of just the cell you are already on.*
///
/// `next` is 「where does the unit after `p` begin」; everything else here is
/// the rule.
pub fn unit_forward(rope: &Rope, from: usize, next: impl Fn(&Rope, usize) -> usize) -> Span {
    let bound = next(rope, from);
    let head = prev_grapheme(rope, bound);
    if head > from {
        return Span::Over { anchor: from, head };
    }
    if bound > from {
        let after = next(rope, bound);
        return Span::Over { anchor: bound, head: prev_grapheme(rope, after).max(bound) };
    }
    Span::Missed
}

/// The same, backwards: the span from where the caret is to where the unit
/// behind it begins.
///
/// Warning: **動不了就是 `Missed`**（2026-10-02 改，原話「Never `Missed` — at the
/// top of the buffer it collapses」）。往前那一支（[`unit_forward`]）動不了的時候
/// 交的是 `Missed`，往回這一支交一格——而 `Span::Missed` 自己的註釋說得很清楚，
/// 那個分別「是 `wdiw` 在檔尾打開的」：動詞要分得出「那裏什麼都沒有」和「那裏有
/// 一格」。
///
/// 少了這個信號，兩套文法只好各自用位置去猜，而且**同一天我把同一條規矩打了兩個
/// 補丁**：`word_back` 自己判一次（檔首按 `b` 不許選一格），vim 那一邊又在
/// `keys.rs` 判一次（檔首按 `db` 不許刪一個字）。兩處都是這一行欠的。
///
/// Warning: **往前和往回本來就不對稱，這不是抄錯。** 往前動不了照樣蓋住你站的那
/// 一格——`dl` 停在行末，`l` 動不了而 vim 照樣刪掉那個字；往回動不了就是真的沒有
/// 東西可拿。所以兩支各說各的方向的規矩，而不是一條。
pub fn unit_back(rope: &Rope, from: usize, prev: impl Fn(&Rope, usize) -> usize) -> Span {
    let head = prev(rope, from);
    match head < from {
        true => Span::Over { anchor: from, head },
        false => Span::Missed,
    }
}

/// **A motion, as a value** — the second half of the grammar layer (B1,
/// 2026-09-20).
///
/// A key no longer *does* a motion; it names one. What a motion covers is
/// [`Span`], and who reads which part of that span is the grammar's business:
/// helix takes the whole of it as a selection, vim (B3) takes one end as a
/// caret target or hands the whole of it to a waiting verb.
///
/// Warning: **The grain rides along.** `w` and `W` are the same motion at two
/// grains — the dictionary's word, or a run between blanks — and `e`/`b` are
/// deliberately [`Grain::Coarse`] in this editor (「`w` takes a word, `e`
/// takes a clause」, #304). A key that spells out its grain is a key the
/// keymap can rebind without the editor knowing which key it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// `w` / `W` — see [`word_forward`].
    WordForward(Grain),
    /// `e` / `E` — the end of the run ahead, both ends set.
    WordEnd(Grain),
    /// `ge` / `gE` — **往回到上一個詞的末尾**（vim；2026-10-02 定，照參考實現）。
    ///
    /// Warning: **helix 沒有這一個**，而 `ge` 在 helix 鍵位裏是「到檔尾」。所以這
    /// 一支只有 vim 文法問得到，兩邊的 `ge` 各是各的。
    WordEndBack(Grain),
    /// **vim 的 `cw`，而 `cw` 不是 `ce`** — 光標**所在**那一段的末尾。
    ///
    /// Warning: `:h cw` 說 `cw` 不吃詞後面的空白，照字面抄就寫成 `ce`，而
    /// `e` 站在詞的最後一格上會跳到**下一**個詞的末尾。2026-10-02 拿 nvim 逐欄
    /// 量：`alpha beta` 的第 5 格（`alpha` 的 `a`）按 `cw`，nvim 只換那一格，而
    /// `ce` 會換掉 `a beta`。所以這是自己的一支，不是 `e`。
    WordEndHere(Grain),
    /// `b` / `B` — backwards to the start of the run behind.
    WordBack(Grain),
    /// `f` / `F` / `t` / `T` — to a character, **on this line only**.
    ///
    /// `till` is vim's `t`: the same search, stopping one short of what it
    /// found. helix has no such key (`t` opens the table group), so this field
    /// is only ever set by the vim grammar.
    Find { forward: bool, target: char, till: bool },
    /// `gg` — the first character of the buffer.
    FileStart,
    /// `ge` — the last.
    FileEnd,
    /// `gh` — the first column of this line.
    LineStart,
    /// `gl` — its last character (not the newline).
    LineEnd,
    /// `gs` — the first thing on it that is not blank.
    LineFirstNonBlank,
    /// `}` / `{` — a paragraph, which in a manuscript is a line of the file.
    Paragraph { forward: bool },
    /// `L` / `H` — a sentence: 。！？ and the closing mark after one.
    Sentence { forward: bool },
    /// `h` / `l` — **one character, and never off this line** (B5,
    /// 2026-09-21).
    ///
    /// The keys themselves walk the page; this is the reading an operator
    /// wants, and it exists because `dl`、`d3l`、`yl`、`c2h` are everyday vim
    /// and a table of motions without `h`/`l` simply drops them on the floor.
    ///
    /// Warning: **Stopping at the line's ends is the whole of it.** vim's `l` will
    /// not carry the caret onto the next line, so `dl` on the last character
    /// takes that character and not the newline — which is the difference
    /// between 「delete a letter」 and 「weld two lines together」.
    Char { forward: bool },
    /// `j` / `k` — **a line of the file**, which is what vim counts.
    ///
    /// Warning: Only ever asked for by an operator (`dj`), and only linewise, so it
    /// answers with the *start* of that line: which column the caret would
    /// keep is the screen's question, and a verb that takes whole lines never
    /// asks it. The keys themselves still walk the screen's rows.
    Line { down: bool },
    /// `H` / `M` / `L` — **the top, middle or bottom of what is on the screen**
    /// (vim; 2026-10-06).
    ///
    /// Warning: **The one motion that has to ask the front end.** Every other is a
    /// function of the rope alone; this one needs to know which slice is drawn,
    /// and the front end hands that over每一幀 (`set_page_span`). Before any
    /// frame has been drawn it misses, so an operator takes nothing rather than
    /// guessing at the whole file.
    ///
    /// A count is 「the nth row in from that edge」, as vim has it: `3H` is the
    /// third row down from the top, `3L` the third up from the bottom, and `M`
    /// pays a count no attention.
    Screen { which: char },
    /// `mi w`, `ma (` — and vim's `ciw`, `di(`, which press the same door.
    ///
    /// Warning: **An object knows both its ends**, which is why it is not two
    /// motions: `i(` is not 「forward to `)`」, it is 「the thing this caret is
    /// inside of」, and from anywhere in it the answer is the same.
    Object { what: Object, around: bool },
}

/// **What a verb does to a span** — the other half of the grammar layer
/// (B2, 2026-09-20).
///
/// A verb used to read the selection itself, which is why a second grammar
/// could not reach it: vim's `dw` has no selection to read, it has a range the
/// motion just handed over. Made a value, the verb takes the range from
/// whoever has one — helix passes its selection, vim (B3) passes the span its
/// motion returned — and **there is one knife** either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    /// `d` — take it out; the register gets it, as it does in helix.
    Delete,
    /// `D` — take it out **deliberately**, into the register.
    Cut,
    /// `c` / `C` — take it out and start typing in its place.
    Change { cut: bool },
    /// `y` — copy it, change nothing.
    Yank,
}

/// **Which reading of a motion is wanted** (B3, 2026-09-20).
///
/// 2026-09-20：「vim 的 `w` 獨立的時候是跳轉，在命令中是選詞。helix 就是
/// 將跳轉和選擇兩個 `w` 合一了。」 That is this enum: one motion, and the two
/// things an editor can ask of it.
///
/// | | `Caret` | `Selection` |
/// | --- | --- | --- |
/// | `w` on `alpha beta` | the `b` —— the primitive | `alpha␣` —— the word and its gap |
/// | `gg` | the first character | the same, collapsed |
///
/// They coincide wherever a motion is *only* a goto, and part company wherever
/// helix wraps the primitive in a rule of its own ([`word_forward`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// What helix asks: the span to select, rule and all.
    Selection,
    /// What vim asks: where the caret lands, and nothing painted.
    Caret,
}

/// What a text object names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    /// A word — or, standing on blanks, that run of blanks (vim's rule, and
    /// the one that makes `wdiw` the handiest press in this editor).
    ///
    /// Warning: **`coarse` 說用不用分詞器**（2026-09-28）。`iw` 走分詞器，和 `w`/`b` 同一個
    /// 答案；`iW` 一律粗粒度，也就是 vim 的「一串非空白」。從前這裏沒有這一格，`iw` 寫
    /// 死了粗粒度，於是「今天天氣很好」整串是一個「詞」，而同一個編輯器的 `w` 走三步。
    /// 一個編輯器對「詞」只能有一個答案。
    ///
    /// Warning: 詞典關着的時候兩個一樣——`word_grain()` 那時本來就回 `Coarse`。
    Word { coarse: bool },
    /// A pair of delimiters, already resolved to its two characters.
    Pair { open: char, close: char },
    /// **`mi m`/`ma m` — 光標所在的那一段 Markdown 標記**（2026-09-28）。
    ///
    /// `**粗**`、`*斜*`、`~~刪~~`、`==標==`、`` `碼` ``、`[文字](地址)`、`[[雙鏈]]`、
    /// `%%批注%%`、腳註。`i` 取裏面的文字，`a` 連標記一起。套着的時候取最裏面那一層。
    ///
    /// Warning: **一個鍵，不是一個標記一個鍵**（2026-09-28 定）。`**` `~~` `==` 是**兩個**
    /// 字符，寫不進 `Object::Pair` 那張一對一的表；而且這一族有九種，一種一個鍵就把
    /// `m` 那一層佔掉一半，而這個倉的規矩是「鍵位很貴」。`md`（去掉最裏面那一對括號）
    /// 早就是同一個思路。
    ///
    /// Warning: **跟着語言走**：靠的是這個倉自己手寫的 Markdown 解析器（`crate::markdown`），
    /// 不是 tree-sitter——tree-sitter 在這個倉裏只管代碼檔與代碼圍欄。`:syntax text`
    /// 的檔交空，Typst 交 Typst 自己那一份。
    Markup,
    /// **`mi s`/`ma s` — 光標所在的那一句**（2026-09-28）。
    ///
    /// Warning: **這一條是這個編輯器該有而 helix 和 vim 都不太有的**：句在這裏本來就是一個
    /// 單位——`H`/`L` 按句走、`:view-sentence` 一句一縱、`:check-punct` 按句查——可是
    /// 「改寫這一句」從前做不到，只能 `H` 再 `L` 再猜邊界。邊界走的是
    /// [`sentence_starts`]，和那三處同一支：兩個答案就意味着光標停在版面不斷行的地方。
    ///
    /// Warning: **`as` 在中文裏會退化成 `is`**，同 `aw`：它多取句末那一段空白，而中文句子之間
    /// 沒有空白。
    Sentence,
    /// `ip` / `ap` — a paragraph (B5, 2026-09-21).
    ///
    /// Warning: **Whole lines, not a run of characters**, which is what makes it an
    /// object and not two `}` motions: from anywhere inside a paragraph the
    /// answer is the same block. `ap` takes the blank lines under it as well
    /// — vim's rule, and the one that makes `dap` remove a paragraph rather
    /// than leave a hole where it was.
    Paragraph,
}

impl Span {
    /// Where the motion ended up — the caret reading of a span.
    ///
    /// Warning: **`Missed` has no head.** The one thing a verb may not do with a
    /// motion that found nothing is act as though it landed somewhere.
    pub fn head(self) -> Option<usize> {
        match self {
            Span::Over { head, .. } => Some(head),
            Span::Missed => None,
        }
    }
}

/// **`f` / `F`, as a span** (B1) — the character on this line, and nothing
/// off it.
///
/// Warning: **A line, not the buffer**: `f` that ran on would be a search, and this
/// editor has one (`/`). Not found is [`Span::Missed`] — the editor says so on
/// the status line, which is its business and not this function's.
pub fn find_char(
    rope: &Rope,
    pos: usize,
    forward: bool,
    target: char,
    till: bool,
    nth: usize,
) -> Span {
    let line = line_of(rope, pos);
    let line_start = rope.line_to_char(line);
    let mut text = rope.line(line).to_string();
    while text.ends_with('\n') || text.ends_with('\r') {
        text.pop();
    }
    let chars: Vec<char> = text.chars().collect();
    let col = pos - line_start;
    // Warning: **數目是「第 n 個」，而且不夠就整個不動**（2026-10-02 拿這臺機器上的
    // nvim 量出來的）。從前數目是在外面**重複**這一支，於是每一趟都從上一個落點
    // 重新下錨：`a,b,c,d,e` 上 `2f,` 選的是「第一個逗號到第二個」，而 vim 和 helix
    // 選的都是「光標到第二個」。數目超出的時候更糟——`9f,` 只有四個逗號，vim 整個
    // 動作失敗、一格不動，而從前它走到最後一個，`d9f,` **默默吃掉三十二個字**。
    //
    // nvim 量出來的（`a,b,c,d,e`，逗號在第 2、4、6、8 欄）：`2f,`→4、`3f,`→6、
    // `9f,`→1（不動）、`2t,`→3、`3t,`→5。
    let nth = nth.max(1);
    let found = match forward {
        true => (col + 1..chars.len()).filter(|&i| chars[i] == target).nth(nth - 1),
        false => (0..col.min(chars.len()))
            .rev()
            .filter(|&i| chars[i] == target)
            .nth(nth - 1),
    };
    let Some(at) = found else { return Span::Missed };
    let head = line_start + at;
    // Warning: **`t` stops one short, and one short is a *grapheme*** — pulling the
    // index back by one would cut a 漢字 in half.
    let head = match (till, forward) {
        (false, _) => head,
        (true, true) => prev_grapheme(rope, head),
        (true, false) => next_grapheme(rope, head),
    };
    Span::Over { anchor: pos, head }
}

/// **`e`, as a span** (B1) — and it sets **both** ends.
///
/// Warning: `select_to` would leave the anchor where the caret was, so standing on a
/// word's last character gave 「that character ＋ the next word」, the
/// punctuation between them riding along. The editor has set both ends since
/// #304; this is that, as a value.
///
/// The grain is the caller's: the editor passes [`Grain::Coarse`] for `e`, so
/// it runs to the next 標點 — `w` takes a word, `e` takes a clause.
pub fn word_end(rope: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> Span {
    let (anchor, head) = next_word_end(rope, pos, grain, seg);
    Span::Over { anchor, head }
}

/// **`b`, as a span** (B1) — backwards, so the anchor is where the caret was
/// and the head is behind it.
///
/// A span whose `head` is before its `anchor` is a backwards selection, which
/// is what this editor has always made of `b`: it selects what it crosses.
///
/// Warning: **站在詞首的時候，你站的那一格不算在內**（2026-10-02 照 helix 比出來的）。
/// 從前錨點一律是 `pos`，於是詞首按 `b` 多拿一個字：`beta` 的 `b` 上按 `b`，選的是
/// `alpha␣b` 而不是 `alpha␣`，`bd` 於是把 `beta` 咬掉一個頭。而 `b` 正常就是停在
/// 詞首按的——「把前面那個詞拿過來」——所以分歧落在最常見的那一下。
///
/// helix 的 `range_to_target` 在第一步就已經跨過邊界的時候 `anchor = head`
/// （`movement.rs:477`）。神諭量出來的（`helix-core` 079a789e8，`alpha beta gamma`）：
///
/// | 光標在 | helix 選中 |
/// | --- | --- |
/// | 3（`alpha` 的 `h`，詞中間） | `alph` ——**帶上**你站的那一格 |
/// | 6（`beta` 的 `b`，詞首） | `alpha␣` ——**不帶** |
/// | 11（`gamma` 的 `g`，詞首） | `beta␣` ——**不帶** |
///
/// Warning: **也不把換行圈進去。** 站在一行的開頭往回走，走的是上一行的最後一個詞，
/// 而那個換行不在選中的裏面（helix 第 17 格給的是 `gamma`，不是 `␊gamma`）。
///
/// Warning: **這是模型層，兩套鍵位一起改。** vim 的 `db` 刪的也是
/// `[prev_word_start, cursor)`——同樣不含光標那一格——所以兩邊要的是同一個答案。
pub fn word_back(rope: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> Span {
    // Warning: **往回也停在行首**，和 `w` 停在行末是同一條（`word_forward`）。沒有
    // 這一條，行首縮進裏按 `b` 會一路退到上一行的詞上，把中間那個換行圈進來。
    // helix 第 18 格（縮進的第二個空格）選的是 `␣␣`，不是 `gamma␊␣`。
    let stop_at_line_start = |r: &Rope, p: usize| -> usize {
        let line = line_of(r, p);
        if let Some(start) =
            near_words(r, line, p, grain, seg, false, |w| w.starts().rfind(|&start| start < p))
        {
            return start;
        }
        let start = line_start(r, p);
        match start < p {
            true => start,
            // 已經在行首了：這一下纔退到上一行。
            false => prev_word_start(r, p, grain, seg),
        }
    };
    let Span::Over { anchor, head } = unit_back(rope, pos, stop_at_line_start) else {
        return Span::Missed;
    };
    // Warning: **後面沒東西了就什麼都不做**，不是「選中一格」。`Span::Missed` 自己的
    // 註釋說的正是這個分別。helix 在檔首按 `b` 原地不動，而從前這裏交出一格，於是
    // `2bd` 在檔首刪掉一個字。
    if head >= anchor {
        return Span::Missed;
    }
    // 站的那一格要不要算：詞首不算，行首和行末那一格也不算（它們的前一格是換行）。
    let line = line_of(rope, pos);
    // 一扇窗就夠，不用推：窗是罩着 `pos` 畫的，兩頭那兩個可能是假的邊界離 `pos`
    // 有四千字（或者那一頭本來就是行的那一頭），落不到 `pos` 這一格上。
    let starts_a_word = near_words(rope, line, pos, grain, seg, true, |w| {
        Some(w.starts().any(|s| s == pos))
    })
    .unwrap_or(false);
    let at_edge = pos == line_start(rope, pos) || pos >= line_end(rope, pos);
    let mut anchor = match starts_a_word || at_edge {
        true => prev_grapheme(rope, anchor),
        false => anchor,
    };
    // 換行不進選區：錨點退到它前面那個字上。`char_to_line` 認不出這一步——換行符
    // 算在**它自己那一行**裏，所以比行號是比不出來的，要看那一格是不是換行。
    while anchor > head && rope.char(anchor.min(rope.len_chars().saturating_sub(1))) == '\n' {
        anchor = prev_grapheme(rope, anchor);
    }
    // Warning: **錨點退到和 head 同一格，那也是一段**（2026-10-04 修）。倒着走的
    // `Span::Over` 是閉區間，所以 `anchor == head` 說的是「選中一個字」，不是
    // 「什麼都沒選中」。從前這裏寫 `anchor > head`，於是**上一個詞只有一個字符的
    // 時候交出 `Missed`**——而 `Missed` 的意思是「哪兒也不去」，光標就卡死了：
    //
    // ```
    // alpha beta gamma      光標在 beta 的 b  →  退到 alpha   ✓
    // alpha `beta` gamma    光標在 beta 的 b  →  一動不動     ✗
    // ```
    //
    // 反引號、逗號、括號、「」——緊挨着詞的標點都是一個字符的詞，所以這一條按得
    // 出來的次數比看上去多。神諭量過（`helix-core`，`aa `bb` cc`，光標在第 4 格）：
    // `anchor=4 head=3 span=[3,4) text="`"`，也就是**走到第 3 格、選中那個反引號**。
    //
    // 真正的「哪兒也不去」在上面那道閘（`head >= anchor`，退錨點之前問的），
    // 那一條管的是檔首——它照舊。
    match anchor >= head {
        true => Span::Over { anchor, head },
        false => Span::Missed,
    }
}

/// The end (last character) of the next word after `pos` (`e` / `E`).
pub fn next_word_end(rope: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> (usize, usize) {
    for line in line_of(rope, pos)..rope.len_lines() {
        // Warning: **那半個邊界要跟着窗走**（2026-10-09 改）。從前這裏論證過「窗把
        // 前一條切掉也不要緊」，論證是錯的：`pos + 1` 這條邊界只在**罩着 `pos` 那一
        // 扇**窗的正中間，而窗一往前推，`pos` 就落在窗外了——光標站在一個詞的末字
        // 上、下一個詞的尾巴又遠在一扇窗之外（`ab` ＋ 兩萬個 `x`，光標在 `b`），推
        // 過去那一扇裏 `k == 0`，拿行首頂替就把 `b` 一起收進了選區，`de` 多吃一個
        // 字。現在那條邊界由被丟掉的那一扇記下來，見 [`Window::leading`]。
        let found = near_words(rope, line, pos, grain, seg, true, |w| {
            let words = w.ended();
            let (k, &(_, end)) =
                words.iter().enumerate().find(|(_, &(_, end))| end.saturating_sub(1) > pos)?;
            // **A word owns the whitespace in front of it** — that is the half
            // of the boundary `e` takes, and `w` takes the other (#304). So the
            // selection starts at the end of the word before, not at the start
            // of this one, and `類` + `e` gives `␠你也是人類` rather than
            // dropping the space on the floor.
            let leading = w.leading(k);
            // …but never *behind* the caret: when the caret is already inside
            // this word, `e` takes the rest of it and nothing before.
            Some((pos.max(leading), end.saturating_sub(1)))
        });
        if let Some(found) = found {
            return found;
        }
    }
    (pos, pos)
}

/// **The end of the run the caret is standing in** — vim's `cw`.
///
/// Off a word (on whitespace) there is no run to end, so it falls back to
/// [`next_word_end`]; the vim grammar never asks in that case, but a motion
/// that answers 「nowhere」 would be a worse model than one that answers 「the
/// next one」.
pub fn word_end_here(rope: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> usize {
    let line = line_of(rope, pos);
    // Warning: **窗裏問的是「`pos` 後面第一個可信的詞尾」**，不是「罩着 `pos` 那一條
    // 的詞尾」（2026-10-08）。兩者在整行上是同一個答案——詞不重疊，`pos` 前面那幾條
    // 的終點都不超過 `pos`——而光標那一條的尾巴落在窗外的時候只有前者還答得出：推
    // 一扇窗之後 `pos` 已經不在窗裏了（一行全是漢字沒有標點的時候，`E` 的「本詞」就
    // 是整行）。
    let end = near_words(rope, line, pos, grain, seg, true, |w| {
        // 「光標在不在詞裏」只有罩着 `pos` 那一扇答得出，而那正是第一扇。空白上沒有
        // 「本詞」可言，`Some(None)` 就是這個回答，外面那一層照老路退回下一個詞的尾。
        if w.holds(pos) && !w.words.iter().any(|&(start, end)| (start..end).contains(&pos)) {
            return Some(None);
        }
        w.ended().iter().map(|&(_, end)| end).find(|&end| end > pos).map(Some)
    });
    match end.flatten() {
        Some(end) => prev_grapheme(rope, end),
        None => next_word_end(rope, pos, grain, seg).1,
    }
}

/// **The end of the last word that ends before `pos`** — vim's `ge`.
///
/// Crosses lines, the way `b` does. Off the front of the buffer it answers
/// with the buffer's start; the caller is the one that knows 「did not move」
/// means the motion failed (nvim's `ge` in column 1 does nothing).
pub fn prev_word_end(rope: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> usize {
    let mut line = line_of(rope, pos);
    loop {
        if let Some(end) = near_words(rope, line, pos, grain, seg, false, |w| {
            w.ended()
                .iter()
                .rev()
                // Warning: **起點在 `pos` 之後的那些詞，先用整數比掉**（2026-10-08）。
                // 一個詞的最後一個字素起在它自己裏面（`start <= prev_grapheme(end)`），
                // 所以那些詞一條都過不了下面那道閘——答案與逐條問 `prev_grapheme` 的
                // 從前完全相同，而少問的那幾下很貴：`prev_grapheme` 在長行上是 O(行長)
                // （`prev_grapheme_boundary` 從行尾往回數字素），於是 `ge` 從前在光標
                // 後面每有一個詞就掃一遍整行。這一跳之後最多問兩次。
                .filter(|&&(start, _)| start < pos)
                .map(|&(_, end)| prev_grapheme(rope, end))
                .find(|&last| last < pos)
        }) {
            return end;
        }
        if line == 0 {
            return 0;
        }
        line -= 1;
    }
}

/// The start of the previous word before `pos` (`b` / `B`).
pub fn prev_word_start(rope: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> usize {
    let mut line = line_of(rope, pos);
    loop {
        if let Some(start) =
            near_words(rope, line, pos, grain, seg, false, |w| w.starts().rfind(|&start| start < pos))
        {
            return start;
        }
        if line == 0 {
            return 0;
        }
        line -= 1;
    }
}


// ---- Paragraphs and sentences (Feature #143) -----------------------------

/// Whether a line holds nothing but blanks.
fn blank(rope: &Rope, line: usize) -> bool {
    line_text(rope, line).trim().is_empty()
}

/// The start of the next paragraph.
///
/// **A paragraph is a logical line**, which is not a definition this module
/// invents: it is the one the rest of the editor already works in — `wrap.rs`
/// calls a logical line a paragraph, the segmentation cache is keyed by one,
/// and a Chinese manuscript is written with one paragraph to a line and no
/// blank line between them.
///
/// That is also why the motion is needed at all. With soft wrap on, `j` and `k`
/// move by *visual row*, and a paragraph is twenty of them — so the key that
/// used to mean "the next paragraph" no longer does, and nothing replaced it.
/// Blank lines are skipped: they separate sections, and no one wants to stop
/// on one.
pub fn next_paragraph(rope: &Rope, pos: usize) -> usize {
    let last = last_line(rope);
    let mut line = line_of(rope, pos);
    while line < last {
        line += 1;
        if !blank(rope, line) {
            return rope.line_to_char(line);
        }
    }
    // No next paragraph: the end of the writing, which is where `}` means to
    // go and where the next paragraph would be written.
    document_end(rope)
}

/// One past the last character of the last line — where writing continues.
fn document_end(rope: &Rope) -> usize {
    line_end(rope, rope.line_to_char(last_line(rope)))
}

/// The start of this paragraph, or of the one before it when already there.
///
/// The same two-step rule `(` follows for a sentence, and what makes `{` usable
/// for backing up: the first press takes you to the top of what you are in.
pub fn prev_paragraph(rope: &Rope, pos: usize) -> usize {
    let mut line = line_of(rope, pos);
    let start = rope.line_to_char(line);
    if pos > start && !blank(rope, line) {
        return start;
    }
    while line > 0 {
        line -= 1;
        if !blank(rope, line) {
            return rope.line_to_char(line);
        }
    }
    0
}

/// Whether the character at `i` closes a sentence.
///
/// The Chinese marks, and a full stop only when a space or a line end follows
/// it — otherwise every `3.14` and every `Mr.` in a manuscript would be the end
/// of a sentence.
fn ends_sentence(chars: &[char], i: usize) -> bool {
    match chars[i] {
        '。' | '！' | '？' | '．' | '…' | '!' | '?' => true,
        '.' => chars.get(i + 1).is_none_or(|c| c.is_whitespace()),
        _ => false,
    }
}

/// Whether the character closes something and so belongs to the sentence that
/// just ended: 「這樣。」 ends after the 」, not before it.
fn closes_a_quote(c: char) -> bool {
    matches!(
        c,
        '」' | '』' | '）' | '》' | '〉' | '】' | '〕' | '｝' | '”' | '’' | '"' | '\'' | ')' | ']' | '}'
    )
}

/// Where every sentence of a line begins.
///
/// `pub(crate)` because `:view-sentence` (Feature #237) lays a 縱書 page out one 句
/// to a 縱, and the boundaries it breaks at have to be the same ones `(` and `)`
/// jump between — two answers would mean the cursor walks to a place the page
/// does not break at.
pub(crate) fn sentence_starts(chars: &[char]) -> Vec<usize> {
    let mut starts = vec![chars
        .iter()
        .position(|c| !c.is_whitespace())
        .unwrap_or(0)];
    let mut i = 0;
    while i < chars.len() {
        if ends_sentence(chars, i) {
            let mut j = i + 1;
            while j < chars.len() && (closes_a_quote(chars[j]) || ends_sentence(chars, j)) {
                j += 1;
            }
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && Some(&j) != starts.last() {
                starts.push(j);
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    starts
}

/// The start of the next sentence, crossing into the next paragraph if this
/// one has no more.
pub fn next_sentence(rope: &Rope, pos: usize) -> usize {
    let last = last_line(rope);
    let mut line = line_of(rope, pos);
    let mut from = Some(pos - rope.line_to_char(line));
    loop {
        let chars: Vec<char> = line_text(rope, line).chars().collect();
        let start = rope.line_to_char(line);
        let after = from.unwrap_or(0);
        if let Some(&at) = sentence_starts(&chars)
            .iter()
            .find(|&&at| at > after || from.is_none())
        {
            return start + at;
        }
        if line >= last {
            return document_end(rope);
        }
        line += 1;
        from = None;
        if blank(rope, line) {
            from = Some(0);
        }
    }
}

/// The start of this sentence, or of the one before it when already there.
pub fn prev_sentence(rope: &Rope, pos: usize) -> usize {
    let mut line = line_of(rope, pos);
    let mut here = Some(pos - rope.line_to_char(line));
    loop {
        let chars: Vec<char> = line_text(rope, line).chars().collect();
        let start = rope.line_to_char(line);
        let starts = sentence_starts(&chars);
        let before = match here {
            Some(col) => starts.iter().rev().find(|&&at| at < col).copied(),
            None => starts.last().copied(),
        };
        if let Some(at) = before {
            return start + at;
        }
        if line == 0 {
            return 0;
        }
        line -= 1;
        here = blank(rope, line).then_some(0);
    }
}

#[cfg(test)]
mod paragraph_and_sentence_tests {
    use super::*;

    #[test]
    fn a_paragraph_is_a_logical_line() {
        // One paragraph to a line and no blank line between them — how a
        // Chinese manuscript is written, and the case `}` exists for: with
        // soft wrap on, `j` moves one of this paragraph's twenty rows.
        let rope = Rope::from_str("第一段很長很長。\n第二段。\n\n第三段。\n");
        assert_eq!(next_paragraph(&rope, 0), 9, "the next line");
        assert_eq!(next_paragraph(&rope, 9), 15, "blank lines are skipped");
        assert_eq!(
            next_paragraph(&rope, 15),
            19,
            "and the last one goes to the end of the writing"
        );
        // Backwards: first to the top of this paragraph, then out of it.
        assert_eq!(prev_paragraph(&rope, 12), 9);
        assert_eq!(prev_paragraph(&rope, 9), 0);
        assert_eq!(prev_paragraph(&rope, 0), 0);
    }

    #[test]
    fn a_sentence_ends_after_its_closing_mark() {
        // 「這樣。」 ends after the 」, not before it — which is the whole
        // reason a sentence motion cannot just search for 。.
        let text = "他說：「不。」她走了。第三句。\n";
        let rope = Rope::from_str(text);
        let starts: Vec<usize> = {
            let chars: Vec<char> = text.trim_end().chars().collect();
            sentence_starts(&chars)
        };
        assert_eq!(starts, vec![0, 7, 11]);
        assert_eq!(next_sentence(&rope, 0), 7);
        assert_eq!(next_sentence(&rope, 7), 11);
        assert_eq!(prev_sentence(&rope, 9), 7);
        assert_eq!(prev_sentence(&rope, 7), 0);
    }

    #[test]
    fn a_full_stop_inside_a_number_is_not_a_sentence() {
        let chars: Vec<char> = "圓周率是 3.14 而已。下一句".chars().collect();
        assert_eq!(sentence_starts(&chars), vec![0, 13]);
    }

    #[test]
    fn a_sentence_motion_crosses_into_the_next_paragraph() {
        let rope = Rope::from_str("只有一句。\n下一段的第一句。\n");
        assert_eq!(next_sentence(&rope, 0), 6, "the next paragraph's first");
        assert_eq!(prev_sentence(&rope, 6), 0);
    }

    #[test]
    fn an_ellipsis_run_is_one_ending() {
        let chars: Vec<char> = "他不說話……她也是。".chars().collect();
        assert_eq!(sentence_starts(&chars), vec![0, 6]);
    }
}

// Warning: **測試模組一律擺在檔尾。** `yumete-core/tests/messages.rs` 那張「每個標籤都
// 有條目」的網把源碼切在**第一個**頂格的 `#[cfg(test)]\nmod ` 處——擺在檔案中間，
// 它後面的生產代碼就整段從網裏消失，於是那裏加一則文案，面板上直接印標籤而測試
// 全綠。這一支從前擺在中間，後面壓着 223 行（2026-09-24 審出來的）。
#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;
    use yumete_cjk::CategorySegmenter;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    /// A word motion must look at the text *near* the cursor, not all of it.
    ///
    /// Timing would make this flaky, so it counts characters instead: a
    /// segmenter that records how much it was handed. Segmenting the whole
    /// buffer for one `w` is what made holding the key run on for seconds after
    /// it was released.
    /// A stand-in dictionary that cuts every two characters — enough to say
    /// 今天｜天氣｜很好, which `CategorySegmenter` cannot: it cuts by category,
    /// and every 漢字 is its own category run.
    struct TwoByTwo;

    impl Segmenter for TwoByTwo {
        fn segment(&self, s: &str) -> Vec<(usize, usize)> {
            let n = s.chars().count();
            (0..n).step_by(2).map(|i| (i, (i + 2).min(n))).collect()
        }
    }

    #[derive(Default)]
    struct Counting(std::cell::Cell<usize>);

    impl Segmenter for Counting {
        fn segment(&self, s: &str) -> Vec<(usize, usize)> {
            self.0.set(self.0.get() + s.chars().count());
            CategorySegmenter.segment(s)
        }
    }

    #[test]
    fn a_word_motion_reads_only_the_lines_it_needs() {
        // A hundred paragraphs; the cursor sits in the first.
        let text = (0..100)
            .map(|_| "那年冬天雪下得比往常都早")
            .collect::<Vec<_>>()
            .join("\n");
        let r = rope(&text);
        let seg = Counting::default();

        next_word_start(&r, 0, Grain::Word, &seg);
        let read = seg.0.get();
        assert!(read > 0, "it has to read something");
        assert!(
            read <= 24,
            "read {read} characters for one `w`; the line is 12"
        );

        // Backwards, from the far end, is bounded the same way.
        let seg = Counting::default();
        prev_word_start(&r, r.len_chars() - 1, Grain::Word, &seg);
        assert!(seg.0.get() <= 24, "read {} going back", seg.0.get());
    }

    // ---- 一行就是整份文件的時候（2026-10-08） ----------------------------
    //
    // 下面這幾支守的是同一件事：`near_words` 那扇窗交出來的答案，**和整行分詞那
    // 一版逐位相同**。所以參照實現就擺在這裏，一字不改地照搬改之前的代碼——它們
    // 是「從前的答案」這句話的本體，別為了「更好寫」去改它們。

    /// 從前的 `next_word_start`。
    fn whole_next_word_start(r: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> usize {
        for line in line_of(r, pos)..r.len_lines() {
            if let Some(start) = line_words(r, line, grain, seg)
                .into_iter()
                .map(|(start, _)| start)
                .find(|&start| start > pos)
            {
                return start;
            }
        }
        r.len_chars()
    }

    /// 從前的 `prev_word_start`。
    fn whole_prev_word_start(r: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> usize {
        let mut line = line_of(r, pos);
        loop {
            if let Some(start) = line_words(r, line, grain, seg)
                .into_iter()
                .rev()
                .map(|(start, _)| start)
                .find(|&start| start < pos)
            {
                return start;
            }
            if line == 0 {
                return 0;
            }
            line -= 1;
        }
    }

    /// 從前的 `next_word_end`。
    fn whole_next_word_end(
        r: &Rope,
        pos: usize,
        grain: Grain,
        seg: &dyn Segmenter,
    ) -> (usize, usize) {
        for line in line_of(r, pos)..r.len_lines() {
            let words = line_words(r, line, grain, seg);
            for (k, &(_, end)) in words.iter().enumerate() {
                let last = end.saturating_sub(1);
                if last <= pos {
                    continue;
                }
                let leading = match k {
                    0 => r.line_to_char(line),
                    _ => words[k - 1].1,
                };
                return (pos.max(leading), last);
            }
        }
        (pos, pos)
    }

    /// 從前的 `prev_word_end`。
    ///
    /// Warning: **這一支帶着那道 `start < pos` 的整數閘**，和改完的那一版一樣——
    /// 不帶的話參照實現自己是行長的平方（`prev_grapheme` 在長行上是 O(行長)），一
    /// 個兩萬字的靶行要跑一分多鐘。它比的仍舊是**整行**的詞，而這幾支要驗的正是窗
    /// 口交出來的答案和整行的一樣。
    fn whole_prev_word_end(r: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> usize {
        let mut line = line_of(r, pos);
        loop {
            if let Some(end) = line_words(r, line, grain, seg)
                .into_iter()
                .rev()
                .filter(|&(start, _)| start < pos)
                .map(|(_, end)| prev_grapheme(r, end))
                .find(|&last| last < pos)
            {
                return end;
            }
            if line == 0 {
                return 0;
            }
            line -= 1;
        }
    }

    /// 從前的 `word_end_here`。
    fn whole_word_end_here(r: &Rope, pos: usize, grain: Grain, seg: &dyn Segmenter) -> usize {
        line_words(r, line_of(r, pos), grain, seg)
            .into_iter()
            .find(|&(start, end)| (start..end).contains(&pos))
            .map(|(_, end)| prev_grapheme(r, end))
            .unwrap_or_else(|| whole_next_word_end(r, pos, grain, seg).1)
    }

    /// 從前的 `word_forward` 停在哪：`stop_at_line_end` 餵給 [`unit_forward`]。
    fn whole_word_forward(r: &Rope, from: usize, grain: Grain, seg: &dyn Segmenter) -> Span {
        let stop = |r: &Rope, p: usize| -> usize {
            let line = line_of(r, p);
            if let Some(start) = line_words(r, line, grain, seg)
                .into_iter()
                .map(|(start, _)| start)
                .find(|&start| start > p)
            {
                return start;
            }
            let end = line_end(r, r.line_to_char(line));
            match end > p {
                true => end,
                false => whole_next_word_start(r, p, grain, seg),
            }
        };
        match unit_forward(r, from, stop) {
            Span::Over { anchor, head } if r.char_to_line(anchor) != r.char_to_line(head) => {
                Span::Over { anchor: line_start(r, head), head }
            }
            other => other,
        }
    }

    /// 一行很長的中文，長到走窗口（`WORD_WINDOW_LINE` 以上）。
    ///
    /// 寫成一段一段不一樣長的，好讓窗邊落在詞裏、落在標點上、落在空隙上都撞得到。
    fn one_long_line() -> String {
        let bits = [
            "那年冬天雪下得比往常都早",
            "，",
            "她說",
            "「不用等我了」",
            "。",
            "abc def",
            "山上的雲很低",
            "；",
            "孩子們在院子裏玩",
            " ",
        ];
        let mut s = String::new();
        let mut i = 0;
        while s.chars().count() <= WORD_WINDOW_LINE + 4000 {
            s.push_str(bits[i % bits.len()]);
            i += 1;
        }
        s.push('\n');
        s
    }

    /// **一行就是整份文件的時候，一下詞動作只讀光標附近那一段**（2026-10-08）。
    ///
    /// 計時會讓測試飄，所以數的是字數：一個把交給它的字數記下來的分詞器。實測的
    /// 病是一行 12,419,022 字的檔上 `e` 每一下 181 ms、`w` 298 ms——整行抄成一個
    /// 37 MB 的 `String`，分完一千兩百萬字，用掉其中一條。
    #[test]
    fn a_word_motion_on_one_huge_line_reads_only_a_window() {
        let text = one_long_line();
        let len = text.chars().count() - 1;
        let r = rope(&text);
        assert!(len > WORD_WINDOW_LINE, "靶行要長到走窗口：{len}");

        // 一扇窗是 `2 * WORD_REACH`；`w` 要兩次（`unit_forward` 問兩回），所以放到
        // 三扇的量——要攔的是「讀了整行」那一種，而整行比這個多得多。
        let budget = 6 * WORD_REACH;
        for &at in &[len / 2, len / 3, len - 1] {
            let seg = Counting::default();
            next_word_start(&r, at, Grain::Word, &seg);
            assert!(seg.0.get() > 0, "總得讀點什麽");
            assert!(seg.0.get() <= budget, "`w` 在第 {at} 格讀了 {} 字", seg.0.get());

            let seg = Counting::default();
            word_forward(&r, at, Grain::Word, &seg);
            assert!(seg.0.get() <= budget, "`w` 的 span 在第 {at} 格讀了 {} 字", seg.0.get());

            let seg = Counting::default();
            prev_word_start(&r, at, Grain::Word, &seg);
            assert!(seg.0.get() <= budget, "`b` 在第 {at} 格讀了 {} 字", seg.0.get());

            let seg = Counting::default();
            word_back(&r, at, Grain::Word, &seg);
            assert!(seg.0.get() <= budget, "`b` 的 span 在第 {at} 格讀了 {} 字", seg.0.get());
        }
    }

    /// **窗裏沒有邊界就推一扇，不是停在原地**（2026-10-08）。
    ///
    /// 一行幾萬個同一個漢字，粗粒度看就是**一個詞**——`e` 真要走到行尾，而那段路
    /// 一扇窗（`2 * WORD_REACH`）裝不下。從前整行分詞答得出，所以這一條守的是
    /// 「窗口那條路不許因為看不見邊界就少走」。
    #[test]
    fn the_window_slides_when_a_run_is_longer_than_it_is() {
        let n = 5 * WORD_REACH;
        let text: String = std::iter::repeat_n('啊', n).chain(['\n']).collect();
        let r = rope(&text);
        let seg = CategorySegmenter;

        for grain in [Grain::Coarse, Grain::Big] {
            // 行首按 `e`：走到行尾那個字，和整行分詞那一版同一格。
            assert_eq!(
                next_word_end(&r, 0, grain, &seg),
                whole_next_word_end(&r, 0, grain, &seg),
                "{grain:?}：`e` 要走過好幾扇窗"
            );
            assert_eq!(next_word_end(&r, 0, grain, &seg), (0, n - 1), "{grain:?}");
            // 行尾按 `b`：退回行首。
            assert_eq!(
                prev_word_start(&r, n - 1, grain, &seg),
                whole_prev_word_start(&r, n - 1, grain, &seg),
                "{grain:?}：`b` 要往回走過好幾扇窗"
            );
            assert_eq!(prev_word_start(&r, n - 1, grain, &seg), 0, "{grain:?}");
            // `ge` 與 `cw` 同理：一個往回推，一個往前推。
            assert_eq!(
                prev_word_end(&r, n / 2, grain, &seg),
                whole_prev_word_end(&r, n / 2, grain, &seg),
                "{grain:?}：`ge`"
            );
            assert_eq!(
                word_end_here(&r, n / 2, grain, &seg),
                whole_word_end_here(&r, n / 2, grain, &seg),
                "{grain:?}：`cw` 的本詞詞尾"
            );
            assert_eq!(word_end_here(&r, n / 2, grain, &seg), n - 1, "{grain:?}");
            // 整行一個詞，所以 `w` 停在行末（`word_forward` 自己那一條）。
            assert_eq!(
                word_forward(&r, 0, grain, &seg),
                whole_word_forward(&r, 0, grain, &seg),
                "{grain:?}：`w`"
            );
        }
    }

    /// **窗口交出來的答案，和整行分詞那一版逐位相同**（2026-10-08）。
    ///
    /// 這是「行為一個字都不許變」那一條的本體：同一行文字，兩套實現，幾十個位置
    /// 逐個對。位置取得密是故意的——窗邊落在詞裏、落在標點上、落在空隙上，切出來
    /// 的那半個詞要被丟掉，而丟錯一條就是一個走偏一格的動作。
    ///
    /// Warning: **粒度只取這三種。** `Big` 與 `Coarse` 是逐字掃出來的連續段，窗口
    /// 改不了它們的內部邊界；`Word` 這裏配的是 `CategorySegmenter`，它也是按字類
    /// 切的。真詞典那一支（Viterbi）在理論上看得見整段，靠的是「分词是非贪婪的，
    /// 只需要看某个区间就行」——那是設計上的前提，不是這一支能證的事。
    #[test]
    fn the_window_answers_what_the_whole_line_answers() {
        let text = one_long_line();
        let len = text.chars().count() - 1;
        let r = rope(&text);
        let seg = CategorySegmenter;

        let mut spots: Vec<usize> = (0..len).step_by(409).collect();
        spots.extend(0..24);
        spots.extend(len.saturating_sub(24)..=len);
        for grain in [Grain::Coarse, Grain::Big, Grain::Word] {
            for &at in &spots {
                assert_eq!(
                    next_word_start(&r, at, grain, &seg),
                    whole_next_word_start(&r, at, grain, &seg),
                    "{grain:?} 第 {at} 格的 `w`"
                );
                assert_eq!(
                    prev_word_start(&r, at, grain, &seg),
                    whole_prev_word_start(&r, at, grain, &seg),
                    "{grain:?} 第 {at} 格的 `b`"
                );
                assert_eq!(
                    next_word_end(&r, at, grain, &seg),
                    whole_next_word_end(&r, at, grain, &seg),
                    "{grain:?} 第 {at} 格的 `e`"
                );
                assert_eq!(
                    prev_word_end(&r, at, grain, &seg),
                    whole_prev_word_end(&r, at, grain, &seg),
                    "{grain:?} 第 {at} 格的 `ge`"
                );
                assert_eq!(
                    word_end_here(&r, at, grain, &seg),
                    whole_word_end_here(&r, at, grain, &seg),
                    "{grain:?} 第 {at} 格的 `cw`"
                );
                assert_eq!(
                    word_forward(&r, at, grain, &seg),
                    whole_word_forward(&r, at, grain, &seg),
                    "{grain:?} 第 {at} 格的 `w`（整段）"
                );
                // `b` 的那一段：頭是「行內上一個詞首」，錨點退不退看「站的那一格是
                // 不是詞首」——兩件事都是窗答的，所以兩件事都對一遍。
                let Span::Over { anchor, head } = word_back(&r, at, grain, &seg) else {
                    assert_eq!(at, 0, "只有檔首纔是 Missed");
                    continue;
                };
                let back = whole_prev_word_start(&r, at, grain, &seg);
                assert_eq!(head, back, "{grain:?} 第 {at} 格的 `b` 落點");
                let starts = line_words(&r, 0, grain, &seg).iter().any(|&(s, _)| s == at);
                let drop = starts || at == 0 || at >= len;
                assert_eq!(
                    anchor,
                    match drop {
                        true => prev_grapheme(&r, at),
                        false => at,
                    },
                    "{grain:?} 第 {at} 格的 `b` 錨點（詞首要不要退一格）"
                );
            }
        }
    }

    /// **一個詞長過一扇窗的時候，`e` 的錨點還是對的**（#315，2026-10-09）。
    ///
    /// 靶造得極小：`ab` ＋ 一個空格 ＋ 兩萬個 `x`，光標按在 `b` 上。`b` 是前一個詞
    /// 的末字，而下一個詞的尾巴在兩萬格之外——窗要往前推四趟纔問得出來，而 `pos`
    /// 在第一趟之後就落在窗外了。從前推過去那一扇用行首頂替那條邊界，於是 `e` 從
    /// `b` 起選（`de` 把 `b` 一起吃掉）；對的答案是從那個空格起選。
    ///
    /// 上面那一支逐位對照的測試看不見這個：[`one_long_line`] 裏最長的「詞」不過十
    /// 幾個字，窗邊永遠落在一條真邊界附近，推一扇窗從來不會跨過一整個詞。
    #[test]
    fn a_word_longer_than_a_window_still_leaves_the_caret_out_of_the_span() {
        let mut text = String::from("ab ");
        text.push_str(&"x".repeat(5 * WORD_REACH));
        text.push('\n');
        let len = text.chars().count() - 1;
        assert!(len > WORD_WINDOW_LINE, "靶行要長到走窗口：{len}");
        let r = rope(&text);
        let seg = CategorySegmenter;
        for grain in [Grain::Coarse, Grain::Big, Grain::Word] {
            assert_eq!(
                next_word_end(&r, 1, grain, &seg),
                whole_next_word_end(&r, 1, grain, &seg),
                "{grain:?}：`b` 上的 `e`"
            );
            assert_eq!(next_word_end(&r, 1, grain, &seg), (2, len - 1), "{grain:?}");
        }
    }

    /// **The rule `w` has always followed, written down at last** (B1,
    /// 2026-09-20). Derived by measuring the editor key by key, then found to
    /// be exactly what the code said — which is why it is pinned here before
    /// a second grammar starts reading it.
    ///
    /// > A `w` never hands back a span of just the cell you are already on.
    /// > If 「to the next word's start」 would do that, it takes the next word.
    #[test]
    fn w_never_selects_only_the_cell_you_are_on() {
        let seg = CategorySegmenter;
        let over = |r: &Rope, at: usize, grain| match word_forward(r, at, grain, &seg) {
            Span::Over { anchor, head } => r.slice(anchor..=head).to_string(),
            Span::Missed => "—".to_string(),
        };

        // 西文：詞首與詞中都選到本詞詞尾（連同後面那個空格）…
        let r = rope("alpha beta gamma");
        assert_eq!(over(&r, 0, Grain::Coarse), "alpha ");
        assert_eq!(over(&r, 2, Grain::Coarse), "pha ");
        // …詞的最後一個字也還是本詞，因爲空格還在前頭…
        assert_eq!(over(&r, 4, Grain::Coarse), "a ");
        // …而站在那個空格上，「到下一個詞首」只剩自己這一格，於是取下一個詞。
        assert_eq!(over(&r, 5, Grain::Coarse), "beta ");

        // 中文没有空格，所以那個分界落在**詞的最後一個字**上。
        // Warning: 用一個「兩字一詞」的替身詞典：`CategorySegmenter` 按字類切，
        // 中文在它眼裏一個字就是一個詞，那樣測不出詞的邊界這回事。
        let two = TwoByTwo;
        let over2 = |r: &Rope, at: usize| match word_forward(r, at, Grain::Word, &two) {
            Span::Over { anchor, head } => r.slice(anchor..=head).to_string(),
            Span::Missed => "—".to_string(),
        };
        let r = rope("今天天氣很好");
        assert_eq!(over2(&r, 0), "今天");
        assert_eq!(over2(&r, 1), "天氣", "站在 今天 的末字，取下一個詞");

        // 到了緩衝區的盡頭是 `Missed`，不是一個空的 span —— 動詞要分得出
        // 「没東西」和「一格」，那正是 `wdiw` 當初栽的地方。
        assert_eq!(word_forward(&r, r.len_chars(), Grain::Word, &two), Span::Missed);
    }

    /// `f` stays on its line, and 「not there」 is `Missed` — the editor turns
    /// that into a sentence, this only reports it.
    #[test]
    fn find_stays_on_its_line() {
        let r = rope("alpha, beta\ngamma, delta");
        // Forward to the comma on this line.
        assert_eq!(find_char(&r, 0, true, ',', false, 1), Span::Over { anchor: 0, head: 5 });
        // The comma on the *next* line is not this line's business.
        assert_eq!(find_char(&r, 7, true, ',', false, 1), Span::Missed);
        // Backwards, and never onto the character the caret is already on.
        assert_eq!(find_char(&r, 8, false, ',', false, 1), Span::Over { anchor: 8, head: 5 });
        assert_eq!(find_char(&r, 5, false, ',', false, 1), Span::Missed);
        // `t` is `f` one short — a **grapheme** short, not an index short.
        assert_eq!(find_char(&r, 0, true, ',', true, 1), Span::Over { anchor: 0, head: 4 });
        let r = rope("他說，她笑");
        assert_eq!(find_char(&r, 0, true, '，', false, 1), Span::Over { anchor: 0, head: 2 });
        assert_eq!(find_char(&r, 0, true, '，', true, 1), Span::Over { anchor: 0, head: 1 });
    }

    /// vim's `w` is the **bare primitive**, and that is the whole difference:
    /// the caret goes to the next word's start, and `dw` operates on
    /// `[pos, next_word_start)`. Standing in a gap, vim takes the gap; helix
    /// takes the next word (above).
    #[test]
    fn vims_w_is_the_primitive_helix_wraps() {
        let seg = CategorySegmenter;
        let r = rope("alpha beta gamma");
        // 詞首：兩者算出同一段 —— helix 選 [0,5]，vim 刪 [0,6)，都是 `alpha `。
        assert_eq!(next_word_start(&r, 0, Grain::Coarse, &seg), 6);
        assert_eq!(word_forward(&r, 0, Grain::Coarse, &seg), Span::Over { anchor: 0, head: 5 });
        // 空隙上：vim 只拿那一格，helix 取下一個詞。分歧就這一處。
        assert_eq!(next_word_start(&r, 5, Grain::Coarse, &seg), 6);
        assert_eq!(word_forward(&r, 5, Grain::Coarse, &seg), Span::Over { anchor: 6, head: 10 });
    }

    /// **一次 `w` 不跨換行**（2026-10-02 照 helix 的 `helix-core` 079a789e8 比出來的）。
    ///
    /// 行末按 `w` 從前選中「本行剩下的字 ＋ 換行 ＋ 下一行的縮進」，一個 `d` 就把
    /// 兩行焊成一行。§5.11 B3 記過這個症狀，可那一次只改了 vim 那一套文法。
    ///
    /// 底下三行是拿 helix 自己的 `move_next_word_start` 量出來的答案。
    #[test]
    fn w_stops_at_the_line_ending_the_way_helix_does() {
        let seg = CategorySegmenter;
        let r = rope("alpha beta gamma\n  indented word here\n");

        // 第 14 格（`gamma` 的 `m`）：選 `ma`，停在行末。
        assert_eq!(
            word_forward(&r, 14, Grain::Coarse, &seg),
            Span::Over { anchor: 14, head: 15 }
        );
        // 第 10 格（`gamma` 前面那個空格）：選整個 `gamma`，照樣不帶換行。
        assert_eq!(
            word_forward(&r, 10, Grain::Coarse, &seg),
            Span::Over { anchor: 11, head: 15 }
        );
        // 第 15 格（`gamma` 的最後一個字）：這一下纔過去，而錨點落在下一行的開頭
        // ——選中的裏面沒有那個換行。
        assert_eq!(
            word_forward(&r, 15, Grain::Coarse, &seg),
            Span::Over { anchor: 17, head: 18 }
        );

        // 整行一個詞的時候，頭停在最後一個字上，不是換行符上。
        let r = rope("甲乙丙丁\n戊己\n");
        assert_eq!(
            word_forward(&r, 0, Grain::Big, &seg),
            Span::Over { anchor: 0, head: 3 }
        );
    }

    /// **`b` 在詞首不帶上你站的那一格，也不圈進換行**（2026-10-02 照 helix 比出來的）。
    ///
    /// 底下每一行都是拿 helix 自己的 `move_prev_word_start` 量出來的（`helix-core`
    /// 079a789e8）。`b` 正常就是停在詞首按的——「把前面那個詞拿過來」——所以那一條
    /// 落在最常見的一下上。
    #[test]
    fn b_drops_the_cell_it_stands_on_at_a_word_start() {
        let seg = CategorySegmenter;
        let r = rope("alpha beta gamma\n  indented word here\n");

        // 詞中間：帶上你站的那一格。
        assert_eq!(
            word_back(&r, 3, Grain::Coarse, &seg),
            Span::Over { anchor: 3, head: 0 }
        );
        // 詞首：不帶。
        assert_eq!(
            word_back(&r, 6, Grain::Coarse, &seg),
            Span::Over { anchor: 5, head: 0 }
        );
        assert_eq!(
            word_back(&r, 11, Grain::Coarse, &seg),
            Span::Over { anchor: 10, head: 6 }
        );
        // 行首往回：走上一行的最後一個詞，而那個換行不在裏面。
        assert_eq!(
            word_back(&r, 17, Grain::Coarse, &seg),
            Span::Over { anchor: 15, head: 11 }
        );
        // 縮進裏往回：停在行首，不退到上一行去。
        assert_eq!(
            word_back(&r, 18, Grain::Coarse, &seg),
            Span::Over { anchor: 18, head: 17 }
        );
        // 檔首：什麼都不做，不是「選中一格」。
        assert_eq!(word_back(&r, 0, Grain::Coarse, &seg), Span::Missed);
    }

    /// **上一個詞只有一個字符的時候，`b` 照樣走得掉**（2026-10-04 修的回歸）。
    ///
    /// 報上來的：站在 `` `mam` `` 的 `m` 上按 `b`，光標一動不動，按幾次都不動。
    /// 跟中文、跟 markdown 都無關，`` aa `bb` cc `` 一樣：
    ///
    /// ```text
    /// alpha beta gamma      光標在 beta 的 b  →  退到 alpha   ✓
    /// alpha `beta` gamma    光標在 beta 的 b  →  一動不動     ✗
    /// ```
    ///
    /// 根子是末尾那道閘寫成了 `anchor > head`。倒着走的 `Span::Over` 是**閉區間**，
    /// 所以 `anchor == head` 說的是「選中一個字」；前一個詞只有一個字符（反引號、
    /// 逗號、括號、「」都是）的時候，詞首那一下的錨點正好退到和 head 同一格，於是
    /// 它被當成「沒地方可去」交了 `Missed`——而 `Missed` 的意思是光標不動。
    ///
    /// 神諭量過（`helix-core`，`` aa `bb` cc ``，光標在第 4 格）：
    /// `anchor=4 head=3 span=[3,4) text="`"`——走到第 3 格，選中那個反引號。
    #[test]
    fn b_gets_out_of_a_word_that_sits_against_a_one_character_word() {
        let seg = CategorySegmenter;
        let r = rope("aa `bb` cc\n");
        // 位：0a 1a 2␣ 3` 4b 5b 6` 7␣ 8c 9c
        //
        // 詞首、而前一個詞只有一個字符：選中那一個字符，光標落在它上面。
        assert_eq!(
            word_back(&r, 4, Grain::Coarse, &seg),
            Span::Over { anchor: 3, head: 3 },
            "`bb` 的第一個 b 上按 b，要走到那個反引號"
        );
        assert_eq!(
            word_back(&r, 8, Grain::Coarse, &seg),
            Span::Over { anchor: 7, head: 6 },
            "cc 的第一個 c 上按 b，走到那個反引號（中間還有個空格）"
        );
        // 同一串裏不受這一條影響的那幾格，照舊。
        assert_eq!(
            word_back(&r, 5, Grain::Coarse, &seg),
            Span::Over { anchor: 5, head: 4 },
            "詞中間：帶上你站的那一格"
        );
        assert_eq!(
            word_back(&r, 3, Grain::Coarse, &seg),
            Span::Over { anchor: 2, head: 0 },
            "反引號自己也是一個詞，從它往回走是 aa"
        );
        // Warning: **檔首那一條不許被這個修順走。** `anchor >= head` 放行的是退錨點
        // **之後**那一步；真正的「哪兒也不去」在前面那道閘上，它管的是這一格。
        assert_eq!(word_back(&r, 0, Grain::Coarse, &seg), Span::Missed);

        // 漢字那一邊同形：句讀是一個字符的詞。
        let r = rope("過「碼」來\n");
        assert_eq!(
            word_back(&r, 2, Grain::Coarse, &seg),
            Span::Over { anchor: 1, head: 1 },
            "「碼」的碼上按 b，要走到那個開引號"
        );
    }

    #[test]
    fn word_motions_still_cross_lines() {
        let r = rope("春江\n潮水");
        let seg = CategorySegmenter;
        // Off the end of the first line, onto the start of the second.
        assert_eq!(next_word_start(&r, 1, Grain::Word, &seg), 3);
        assert_eq!(prev_word_start(&r, 3, Grain::Word, &seg), 1);
    }

    #[test]
    fn horizontal_motion_is_grapheme_aware() {
        // "e" + combining acute is one grapheme; "中" is one wide grapheme.
        let r = rope("e\u{0301}中x");
        // Chars: e(0) ´(1) 中(2) x(3). Graphemes: "é"(0..2 chars), "中", "x".
        assert_eq!(right(&r, 0), 2); // skip the whole "é"
        assert_eq!(right(&r, 2), 3); // over "中"
        assert_eq!(left(&r, 3), 2); // back over "中"
        assert_eq!(left(&r, 2), 0); // back over "é"
        assert_eq!(left(&r, 0), 0); // clamped at line start
    }

    #[test]
    fn horizontal_motion_stays_within_line() {
        let r = rope("ab\ncd");
        // End of first line (after "b", char idx 2): right should not cross "\n".
        assert_eq!(right(&r, 2), 2);
        // Start of second line (char idx 3): left should not cross back.
        assert_eq!(left(&r, 3), 3);
    }

    #[test]
    fn vertical_motion_preserves_visual_column() {
        // Line 0 has a wide glyph; column 2 (after "中") should map to column 2
        // on line 1, which is after "ab".
        let r = rope("中x\nabc");
        let goal = visual_column(&r, 1); // pos 1 = after "中" = width 2
        assert_eq!(goal, 2);
        let down_pos = down(&r, 1, goal);
        // On "abc", visual column 2 is after "ab" → char index 3 + 2 = 5.
        assert_eq!(down_pos, 5);
        assert_eq!(visual_column(&r, down_pos), 2);
    }

    #[test]
    fn line_and_buffer_motions() {
        let r = rope("  hello\nworld\n");
        // ^ on line 0 skips the two leading spaces.
        assert_eq!(line_first_non_blank(&r, 4), 2);
        // 0 goes to the very start of the line.
        assert_eq!(line_start(&r, 4), 0);
        // $ goes to end of "  hello" (7 chars).
        assert_eq!(line_end(&r, 0), 7);
        // G goes to the start of the last (navigable) line, "world".
        assert_eq!(buffer_end(&r, 0), 8);
        assert_eq!(buffer_start(&r, 8), 0);
    }

    #[test]
    fn down_does_not_fall_onto_the_phantom_trailing_line() {
        let r = rope("a\n");
        // Only one navigable line; down from it stays put.
        assert_eq!(last_line(&r), 0);
        assert_eq!(down(&r, 0, 0), 0);
    }

    #[test]
    fn word_motions_step_by_word_and_cjk_character() {
        let seg = yumete_cjk::CategorySegmenter;
        let r = rope("foo bar 你好");
        // Chars: f0 o1 o2 ' '3 b4 a5 r6 ' '7 你8 好9.
        assert_eq!(next_word_start(&r, 0, Grain::Word, &seg), 4); // → "bar"
        assert_eq!(next_word_start(&r, 4, Grain::Word, &seg), 8); // → "你"
        assert_eq!(next_word_start(&r, 8, Grain::Word, &seg), 9); // → "好" (each CJK is a word)
        // `e` answers with both ends now (#304): where the selection starts
        // and where the caret lands. Inside a word it starts where the caret
        // is; stepping out of one it starts at the whitespace before the next.
        assert_eq!(next_word_end(&r, 0, Grain::Coarse, &seg), (0, 2)); // "foo"
        assert_eq!(next_word_end(&r, 2, Grain::Coarse, &seg), (3, 6)); // " bar"
        assert_eq!(prev_word_start(&r, 9, Grain::Word, &seg), 8); // back to "你"
        assert_eq!(prev_word_start(&r, 6, Grain::Word, &seg), 4); // back to start of "bar"
    }

    #[test]
    fn big_word_motions_ignore_punctuation_boundaries() {
        let seg = yumete_cjk::CategorySegmenter;
        let r = rope("a.b cd");
        // Small `w` stops at the punctuation; big `W` skips to "cd".
        assert_eq!(next_word_start(&r, 0, Grain::Word, &seg), 1); // "." is its own word
        assert_eq!(next_word_start(&r, 0, Grain::Big, &seg), 4); // WORD → "cd"
    }

    #[test]
    fn word_motions_use_the_dictionary_when_supplied() {
        // With 你好 in the dictionary, `w` steps over the whole word, not each
        // character; the default category segmenter would stop after 你.
        let seg = yumete_cjk::DictionarySegmenter::new([("你好".to_string(), 100)], 1);
        let r = rope("foo 你好 bar");
        // Chars: f0 o1 o2 ' '3 你4 好5 ' '6 b7 a8 r9.
        assert_eq!(next_word_start(&r, 0, Grain::Word, &seg), 4); // → 你好
        assert_eq!(next_word_start(&r, 4, Grain::Word, &seg), 7); // → "bar" (skips 好)
        // Warning: `e` does **not** consult the dictionary (#304): coarse, 你好 is
        // one run of letters either way, and the caret on the space before it
        // takes the space with it.
        assert_eq!(next_word_end(&r, 3, Grain::Coarse, &seg), (3, 5));
    }
}
