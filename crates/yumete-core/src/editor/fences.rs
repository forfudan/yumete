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
///
/// Warning: **Counted in lines as well as in entries** ([`HELD`]). This cache was
/// designed for *fences* — a snippet in a manuscript, a dozen lines — and 256
/// of those is nothing. Then whole code files started coming through the same
/// door (#420), where one entry can be five thousand lines of spans; 256 of
/// **those** is not a cache, it is a leak with a lid on it.
const KEPT: usize = 256;

/// …and the lines those entries may add up to, whichever fills first.
///
/// A line's runs are a `Vec<Span>` — a handful of spans at 32 bytes each — so
/// fifty thousand lines is some ten megabytes, and it holds ten of the biggest
/// files the cap above ([`LONGEST`]) lets in.
const HELD: usize = 50_000;

/// One fence: the line that opens it, the line that closes it if anything
/// does, and the language its info string names.
#[derive(Debug, Clone, Copy)]
pub(super) struct Fence {
    open: usize,
    close: Option<usize>,
    language: Option<Language>,
}

type Lines = Rc<Vec<Vec<Span>>>;

/// **一份整個是代碼的檔，解析出來的那棵樹**（#423，2026-09-30）。
///
/// Warning: **樹要留着，這是整條的前提。** 從前 `code::highlight` 每叫一次解析
/// 一次，於是滾一屏也付一次全份解析——量過 20,368 行的 `lib.rs` 是 50 毫秒。
/// 留着它，滾屏只剩一次窗口查詢（0.06 毫秒），改一個字只剩一次增量解析
/// （0.7 毫秒）。
struct Held {
    buffer: u64,
    revision: u64,
    language: Language,
    /// 解析用的那一份正文（`lines` 接起來，每行帶一個換行）。
    source: String,
    lines: Vec<String>,
    tree: tree_sitter::Tree,
}

/// 一次畫多少行。
///
/// 一屏撐死幾十行，所以一塊蓋得住一屏；而塊越大，跨過邊界重查的次數越少。量過
/// 五十行一窗是 0.06 毫秒，一百二十八行也還在零點幾毫秒。
const CHUNK: usize = 128;

#[derive(Default)]
pub(super) struct CodeCache {
    /// Buffer, revision and syntax the fences were read for.
    key: Option<(u64, u64, u8)>,
    fences: Rc<Vec<Fence>>,
    /// This revision's answers, by opening line.
    by_open: HashMap<usize, Lines>,
    /// Every revision's answers, by what the fence says.
    by_text: HashMap<u64, Lines>,
    /// How many lines of spans `by_text` is holding.
    held: usize,
    /// 整份代碼檔那棵樹，和它是哪一版的（#423）。
    whole: Option<Held>,
    /// 這一版已經畫過的那幾塊，按塊號（#423）。換一版就清。
    by_chunk: HashMap<usize, Lines>,
}

impl Editor {
    /// Whether fenced code is coloured by its grammar (`:view-code`).
    pub fn code_colours(&self) -> bool {
        self.code_colours
    }

    /// **一行長過這個就不上色**（2026-10-08 定，10-09 從 3000 擡到這個數）。
    ///
    /// 從前取的是 vim `synmaxcol` 的 3000。那個數不對：vim 那一道閘管的是**代碼上
    /// 的語法著色**，而這一道還連着記號的藏與不藏、腳註的上標、分詞的著色——正是散
    /// 文靠的那幾樣。3000 字在這個編輯器裏不是壓縮過的 `.js`，是一段寫得長的散文，
    /// 於是出廠狀態下一段 3371 字的中文就靜靜地不再上色、`**粗**` 也不再收起來。
    ///
    /// 這個數是量出來的。扣掉啓動基線，每幀多花的時間：
    ///
    /// | 一行多少字 | 每幀多花 |
    /// | --- | --- |
    /// | 4 千 | 2 ms |
    /// | 2 萬 | 4 ms |
    /// | 5 萬 | 7 ms |
    /// | 10 萬 | 12 ms |
    /// | 50 萬 | 52 ms |
    /// | 150 萬 | 152 ms |
    ///
    /// 五萬字是「幾十頁擠成一段」，[`crate::motion`] 那頭的 `WORD_WINDOW_LINE` 用同
    /// 一句話說過它不是稿子裏有的東西；到這裏為止每幀七毫秒，而這道閘本來要攔的那
    /// 一種（一行一百五十萬字）照舊攔得住。記號擺得很密的行會比這張表貴（量過一行
    /// 每五十字一對 `**` 的，10 萬字 165 ms），那種行調 `:view-long-line` 就是。
    ///
    /// Warning: **數的是字，不是列。** 問這一句不許造字串（見
    /// [`Self::line_is_too_long`]），而一行的列寬要逐字量，正好是這道閘要省的那筆
    /// 賬。界面上那幾則文案現在說的是「列數」，對不上——那是文案，等作者定。
    pub const LONG_LINE: usize = 50_000;

    /// 那個門槛，`None` ＝沒有上限。
    pub fn long_line(&self) -> Option<usize> {
        self.long_line
    }

    pub fn set_long_line(&mut self, upto: Option<usize>) {
        self.long_line = upto;
        // **走這道閘的備忘，四份一起忘**（2026-10-09 改）。
        //
        // Warning: 從前只忘兩份（記號與代碼），另兩份（分詞與標點）靠的是「那道閘擺
        // 在備忘查詢**之前**」——按今天的代碼確實不會過期，可那就成了「兩份靠順序、
        // 兩份靠清空」，而讀代碼的人看見清空這一句，自然以為把閘擺在查詢後面也行。
        // 門槛是一個設置，動一次就這麼一下，四份全忘最便宜也最說得清。
        self.markup_memo.forget();
        self.segment_memo.forget();
        self.note_memo.forget();
        self.code_cache.borrow_mut().by_chunk.clear();
    }

    /// **這一行長得超過門槛了嗎**——超了就一個記號都不算。
    ///
    /// Warning: **一個字一個字數，不造字串**：這一句每幀每行要問一次，而它存在的
    /// 理由就是「那一行太長」——為了問這一句把整行拷一份是自己把自己抵消掉。
    /// `rope.line(n)` 是一個切片，`len_chars` 是 O(log n)。
    pub(super) fn line_is_too_long(&self, line: usize) -> bool {
        let Some(upto) = self.long_line else {
            return false;
        };
        let rope = self.current_buffer().rope();
        line < rope.len_lines() && rope.line(line).len_chars() > upto
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
    ///
    /// Warning: **這一支走的是另一條路**（#423，2026-09-30）。圍欄裏那幾行是幾
    /// 十行，整份解析一次就完了；一份兩萬行的源碼不是——所以這一條**留着樹**、
    /// **只查看得見的那一塊**、**改一個字走增量**。三件湊起來把一次按鍵從 75 毫
    /// 秒壓到 0.8 毫秒，那個五千行的閘也就不必再有了。
    pub(super) fn code_file_line(&self, line: usize, language: Language) -> Vec<Span> {
        if !self.code_colours {
            return Vec::new();
        }
        // Warning: **先確認手上那棵樹就是這個檔這一版的**（2026-10-01 報的：
        // 「open a rust file first and then open a python file via picker,
        // the coloring of the python file is incorrect」）。`by_chunk` 只按
        // 「第幾塊」記，**不記是哪個檔**——所以從 `build.rs` 切到 `sc2tc.py`，
        // 第 0 塊早就在裏頭了，直接命中的是**上一個檔的顏色**，而
        // `hold_the_tree` 連叫都沒叫到，那一格於是永遠不清。
        //
        // 放在前面不貴：同一個檔同一版的時候 `hold_the_tree` 就是三個比較。
        self.hold_the_tree(language);
        let chunk = line / CHUNK;
        if let Some(found) = self.code_cache.borrow().by_chunk.get(&chunk) {
            return found.get(line - chunk * CHUNK).cloned().unwrap_or_default();
        }
        let mut cache = self.code_cache.borrow_mut();
        let Some(held) = cache.whole.as_ref().filter(|h| h.language == language) else {
            return Vec::new();
        };
        let rows = chunk * CHUNK..(chunk + 1) * CHUNK;
        let painted: Lines = Rc::new(crate::code::paint(
            language,
            &held.source,
            &held.tree,
            &held.lines,
            rows,
        ));
        cache.by_chunk.insert(chunk, painted.clone());
        painted.get(line - chunk * CHUNK).cloned().unwrap_or_default()
    }

    /// **這一份裏每一個定義**，位置是**字符**（`]f`/`]c`/`mi f`，2026-10-06）。
    ///
    /// 只在「整份是代碼」的檔上回得出東西：markdown 裏的圍欄各有各的樹，而一段
    /// 稿子裏的 `def` 不是這本書的結構。空的回空——按鍵那一頭照這個說「這裏沒有」。
    pub(super) fn definitions_here(&self) -> Vec<(usize, usize, crate::code::Define)> {
        let crate::syntax::Syntax::Code(language) = self.current_buffer().syntax() else {
            return Vec::new();
        };
        if !self.code_colours {
            return Vec::new();
        }
        self.hold_the_tree(language);
        let cache = self.code_cache.borrow();
        let Some(held) = cache.whole.as_ref().filter(|h| h.language == language) else {
            return Vec::new();
        };
        let found = crate::code::definitions(language, &held.source, &held.tree);
        // 字節 → 字符。`source` 就是解析用的那一份，所以兩邊數的是同一串。
        let at = |byte: usize| held.source[..byte.min(held.source.len())].chars().count();
        found.into_iter().map(|(from, to, kind)| (at(from), at(to), kind)).collect()
    }

    /// **這一份裏每一個參數/註釋**，位置是**字符**（`mi a`/`mi c`，2026-10-06）。
    pub(super) fn objects_here(&self, want: crate::code::Object) -> Vec<(usize, usize)> {
        let crate::syntax::Syntax::Code(language) = self.current_buffer().syntax() else {
            return Vec::new();
        };
        if !self.code_colours {
            return Vec::new();
        }
        self.hold_the_tree(language);
        let cache = self.code_cache.borrow();
        let Some(held) = cache.whole.as_ref().filter(|h| h.language == language) else {
            return Vec::new();
        };
        let found = crate::code::objects(language, &held.source, &held.tree, want);
        let at = |byte: usize| held.source[..byte.min(held.source.len())].chars().count();
        found.into_iter().map(|(from, to)| (at(from), at(to))).collect()
    }

    /// **新開的那一行，比這一行多縮幾級**（2026-10-08，tree-sitter 縮進第一期）。
    ///
    /// 回 `0` ＝ 和這一行一樣（照抄空白，從前的行為）。`closing` 是新那一行開頭已
    /// 經打出來的那個字——`}` 之類會把這個數減一。
    ///
    /// **照 helix 的 `Hybrid`**：樹只說「差幾級」，**基準是上一行真實的縮進**。它
    /// 自己的註釋寫着理由——「incomplete queries, incomplete source code &
    /// differing indentation styles」——而這三樣每一樣這裏都有。所以源碼打到一半
    /// 解析不出來的時候（實測：`fn f() {` 自己一份檔，tree-sitter 給的是
    /// `(source_file (ERROR …))`，一個 `block` 都沒有），兩頭都答 0，差也是 0，
    /// 於是照抄上一行——和 2026-10-08 早些時候做的 `autoindent` 一字不差。
    pub(super) fn levels_to_open(&self, closing: Option<char>) -> isize {
        let crate::syntax::Syntax::Code(language) = self.current_buffer().syntax() else {
            return 0;
        };
        self.hold_the_tree(language);
        let cache = self.code_cache.borrow();
        let Some(held) = cache
            .whole
            .as_ref()
            .filter(|h| h.buffer == self.current_buffer().id() && h.language == language)
        else {
            return 0;
        };
        // 樹是按**字節**數的，而這一頭數字符。
        let rope = self.current_buffer().rope();
        let at = self.sel.head().min(rope.len_chars());
        let byte = rope.char_to_byte(at).min(held.source.len());
        let line = rope.char_to_line(at);
        // 算的是**新開的那一行**，所以門檻是光標這一行 ＋ 1。
        let here =
            crate::code::open_levels(language, &held.source, &held.tree, byte, line + 1, closing);
        // 上一行的第一個非空白——基準那一行自己開着幾級（門檻是它自己）。
        let start = rope.line_to_char(line);
        let text = crate::motion::line_text(rope, line);
        let first = text.chars().take_while(|c| c.is_whitespace()).count();
        let base_byte = rope.char_to_byte((start + first).min(rope.len_chars())).min(held.source.len());
        let base =
            crate::code::open_levels(language, &held.source, &held.tree, base_byte, line, None);
        here as isize - base as isize
    }

    /// **把這一版的樹備好**——已經是這一版就什麽都不做。
    ///
    /// Warning: **改過就走增量。** 上一版的正文還在手上，掐頭去尾就看得出改了
    /// 哪一段（`code::what_changed`），`Tree::edit` 吃下去再解析一遍是 0.7 毫
    /// 秒；從頭解析要 50 毫秒，而那是**每按一個鍵**付一次。
    fn hold_the_tree(&self, language: Language) {
        let buffer = self.current_buffer();
        let (id, revision) = (buffer.id(), buffer.revision());
        {
            let cache = self.code_cache.borrow();
            if cache.whole.as_ref().is_some_and(|h| {
                h.buffer == id && h.revision == revision && h.language == language
            }) {
                return;
            }
        }
        let rope = buffer.rope();
        let lines: Vec<String> = (0..rope.len_lines())
            .map(|l| {
                let mut text = rope.line(l).to_string();
                while text.ends_with('\n') || text.ends_with('\r') {
                    text.pop();
                }
                text
            })
            .collect();
        let source = crate::code::joined(&lines);
        let mut cache = self.code_cache.borrow_mut();
        // 同一個檔、同一種語言，只是版本新了：拿上一棵樹走增量。
        let was = cache.whole.take().filter(|h| h.buffer == id && h.language == language);
        // **手上那一棵已經不算數了，畫過的那幾塊也就不算數**——擺在這裏而不是
        // 擺在最後，因為下面那一句解析失敗會直接 `return`，而那時候
        // `cache.whole` 已經被 `take` 走了：留着舊的 `by_chunk` 就是一張沒有樹
        // 的顏色表，下一次查還會命中它。
        cache.by_chunk.clear();
        let tree = match was {
            Some(held) => match crate::code::what_changed(&held.source, &source) {
                Some(edit) => {
                    let mut edited = held.tree;
                    edited.edit(&edit);
                    crate::code::parse(language, &source, Some(&edited))
                }
                // 一個字都沒改（換了 revision 卻同一份正文——撤銷回原處就是）。
                None => Some(held.tree),
            },
            None => crate::code::parse(language, &source, None),
        };
        let Some(tree) = tree else { return };
        cache.whole = Some(Held { buffer: id, revision, language, source, lines, tree });
    }

    /// **How long the code under the cursor is, when that is why it has no
    /// colours** — `(lines, the cap)`, and `None` when nothing is wrong.
    ///
    /// Warning: The cap exists (parsing runs on every edit, so a pasted data file
    /// would be re-parsed per keystroke), but a cap that says nothing reads as
    /// a broken feature: 「顏色到這裏就沒了」. `:view-code` asks this, so the
    /// question 「why is this not coloured」 has an answer where a reader would
    /// look for it.
    pub(super) fn code_too_long_here(&self) -> Option<(usize, usize)> {
        if !self.code_colours {
            return None;
        }
        let line = self.cursor_line();
        let rope = self.current_buffer().rope();
        let long = |n: usize| (n > LONGEST).then_some((n, LONGEST));
        // Warning: **整份是代碼的檔沒有這個閘了**（#423，2026-09-30）：它走留樹
        // ＋窗口查詢＋增量解析那一條，多長都染得起。閘只剩給稿子裏貼的那一段
        // ——那一種是整份解析的，而一份貼進來的數據檔沒人靠顏色讀。
        if matches!(self.current_buffer().syntax(), crate::syntax::Syntax::Code(_)) {
            return None;
        }
        let fences = self.fences();
        let at = fences.partition_point(|f| f.open < line);
        let fence = at.checked_sub(1).map(|i| fences[i])?;
        if fence.close.is_some_and(|close| line >= close) || fence.language.is_none() {
            return None;
        }
        let end = fence.close.unwrap_or(rope.len_lines());
        long(end.saturating_sub(fence.open + 1))
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
                if cache.by_text.len() >= KEPT || cache.held >= HELD {
                    cache.by_text.clear();
                    cache.held = 0;
                }
                cache.held += body.len();
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
                // Warning: **Only a fence opens a fence.** Typst's scan calls a
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
