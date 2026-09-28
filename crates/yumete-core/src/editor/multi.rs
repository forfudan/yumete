//! **多選區的那幾個鍵**（#405，方案在 `docs/development.md §5.13`）。
//!
//! 造出第二段的是 `C`／`A-C`，收回去的是 `,`。語義照 helix
//! （`helix-term/src/commands.rs` 的 `copy_selection_on_line`，基準 commit `079a789e8`），
//! 但**「行」走的是這個倉自己的那一套**：`crate::wrap` 的視覺行，也就是 `j`／`k` 踩的那個
//! 頁面。⚠️ 軟折行開着的時候，一個自然段是好幾行，而使用者看見的「下一行」是折出來的那
//! 一行，不是檔案裏的那一行。竪排同理（§5.13.8 二：「複製到視覺上的下一列」與橫排是同
//! 一句話）。

use super::*;
use crate::selection::Range;

/// **正則那一族在等什麽**（#405 Phase 2）。
///
/// 四個鍵共用搜索那一扇提示行——於是**拼音、簡繁、模糊、正則四個開關一起白拿**，
/// 搜「书斋」選得出「書齋」。helix 沒有這一件，它的 `s` 只認正則。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sift {
    /// `s` — 在每一段選區**裏面**選出所有匹配。
    Select,
    /// `S` — 拿匹配當分隔符，把每一段切開。
    Split,
    /// `A-k` — 只留下匹配的那幾段。
    Keep,
    /// `A-K` — 去掉匹配的那幾段。
    Drop,
}

impl Sift {
    /// 提示行前面寫什麼。
    ///
    /// ⚠️ **寫字，不寫字母。** `s/` `S/` `k/` `K/` 那一套省三格，可是按下去之後屏幕上
    /// 那一行說不出它要做什麼，而這四件事做完的樣子差得很遠（選出、切開、只留、去掉）。
    pub fn prefix(self) -> &'static str {
        match self {
            Sift::Select => "選出/",
            Sift::Split => "切開/",
            Sift::Keep => "只留/",
            Sift::Drop => "去掉/",
        }
    }
}

impl super::Editor {
    /// **往下／往上再加一個選區**（`C`／`A-C`）。
    ///
    /// 每一段各複製 `count` 份：兩端各自記住自己的**列**，然後一行一行往下走，走到哪一
    /// 行兩端的列都還在，就在那一行放一份。
    ///
    /// ⚠️ **太短的行跳過，不是把選區壓到行尾**（helix 的規矩）。一串 md 列表項長短不一，
    /// 壓到行尾等於在每一行的不同位置放一個光標，那不是「同一列」。
    ///
    /// ⚠️ **新長出來的那一段是主選區**：讀者的注意力就在剛長出來的那一個上，而且連按
    /// `C` 要沿着它繼續往下長。
    pub(super) fn copy_selection_on_row(&mut self, down: bool, count: usize) {
        let made = {
            let hide = |line: usize| self.hidden_on_line(line);
            let fold = |line: usize| self.line_is_folded(line);
            let drawn = |line: usize| self.drawn_on_line(line);
            let typed = |line: usize| self.typed_on_line(line);
            let flat = |line: usize| self.table_row_at(line);
            let width = self.wrap_width().unwrap_or(crate::wrap::NO_WRAP);
            let m = crate::wrap::Measure::new(width, &hide)
                .with_indent(self.paragraph_indent())
                .with_folds(&fold)
                .with_drawn(&drawn)
                .with_typed_drawn(&typed)
                .with_unwrapped(&flat)
                .with_version(self.current_buffer().id(), self.current_buffer().revision())
                .with_edit(self.current_buffer().edit())
                .with_open_line(self.open_line())
                .with_caret(Some(self.caret_in_line()));
            let rope = self.current_buffer().rope();
            let step = |pos: usize, goal: usize| match down {
                true => crate::wrap::next_row(rope, pos, m, goal),
                false => crate::wrap::prev_row(rope, pos, m, goal),
            };
            // ⚠️ **主選區那一段的複製件排在最後推**。`push` 把剛推進去的那一段當主選
            // 區，而 `normalize` 是按**值**把主選區認回來的，所以「最後推的是誰」就決定
            // 了新的主選區是誰。helix 同樣把主選區交給它自己那一段的最後一個複製件
            // （`commands.rs:2156`），而不是交給頁面上最下面那一個。
            let primary = self.sel.primary();
            let mut made: Vec<Range> = Vec::new();
            let mut mine: Vec<Range> = Vec::new();
            for one in self.sel.iter() {
                let into = match *one == primary {
                    true => &mut mine,
                    false => &mut made,
                };
                let anchor_goal = crate::wrap::column_of(rope, one.anchor, m);
                let head_goal = crate::wrap::column_of(rope, one.head, m);
                let (mut anchor, mut head) = (one.anchor, one.head);
                let mut placed = 0;
                while placed < count {
                    let (next_anchor, next_head) =
                        (step(anchor, anchor_goal), step(head, head_goal));
                    // 兩端都走不動了就是到頭了。⚠️ 判的是「有沒有動」而不是「在不在檔
                    // 尾」：折行、摺疊、表格都會讓「還有沒有下一行」不等於「行號還夠」。
                    if next_anchor == anchor && next_head == head {
                        break;
                    }
                    anchor = next_anchor;
                    head = next_head;
                    if crate::wrap::column_of(rope, anchor, m) == anchor_goal
                        && crate::wrap::column_of(rope, head, m) == head_goal
                    {
                        // ⚠️ 複製件記下**它自己那一列**：它就是照這一列放下去的，
                        // 而接着按 `j` 要瞄準的正是這一列。
                        into.push(Range { anchor, head, goal: Some(head_goal), goal_slot: None });
                        placed += 1;
                    }
                }
            }
            made.extend(mine);
            made
        };
        // ⚠️ **這一支造出來的重疊不報。** 兩段相鄰的選區各往下複製一份，下面那一份必然
        // 落在上面那一段原來的位置上，於是每按一次 `C` 都會併掉幾段——那是機制，不是意
        // 外。[`Self::say_the_merge`] 要說的是別的：移動或編輯把兩段撞到一起。
        let before = self.sel.len();
        for one in made {
            self.sel.push(one);
        }
        if self.sel.len() == before {
            self.status = say!("selection.no-room");
        }
    }

    /// **把「作用在主選區上的一件事」逐段各做一次**（#405 Phase 1）。
    ///
    /// 辦法是把每一段輪流擺成唯一的那一段，跑一次 `what`，取回結果。⚠️ **這樣三百多處
    /// 讀寫主選區的代碼一行都不用改**——它們本來就只管主選區，而這一支保證它們每次看見
    /// 的都是一段真的、當下該管的選區。
    ///
    /// ⚠️ **只給移動用，不給編輯用。** 編輯會挪動別的選區的下標，那要走 Phase 1 第五步
    /// 的 `edit_each`（從後往前做，一個撤銷點）。這一支假定 `what` 不改文本。
    ///
    /// ⚠️ **目標列不在這裏管。** 它跟着每一段自己走（`Range::goal`），所以連按 `j` 跨過
    /// 一行短行之後，每一段記得住的還是它自己原來那一列。2026-09-28 之前它是 `Editor`
    /// 上的一個 `goal_column`，那時 N 段會一起瞄準主選區那一列。
    pub(super) fn each_selection(&mut self, what: impl Fn(&mut Self)) {
        if !self.sel.is_plural() {
            what(self);
            return;
        }
        let was = self.sel.clone();
        let primary = was.primary();
        let mut out: Vec<Range> = Vec::with_capacity(was.len());
        let mut which = 0;
        // ⚠️ **一次性的那幾格要每一段都看得見。** 第一段跑完就把 `pending` 吃掉了
        // （`f` 補上字符之後 `Pending::Find` 就沒了），後面幾段於是把那個字符當成一個
        // 普通的鍵——`f丙` 只有第一段走得動，別的原地不動。`count` 同理（`3w`）。
        // 每一段開跑之前擺回去，跑完之後留最後一段的那一份。
        let pending = self.pending.clone();
        let count = self.count;
        // ⚠️ **`"` 指的那個寄存器也是一次性的**（2026-09-28）：`take_register` 是
        // **take**，第一趟就把它拿走了，後面幾段於是去讀無名的那一個，`"#p` 只有一段
        // 貼得上號。同 `pending` 和 `count`，每一段開跑之前擺回去。
        let named = self.pending_register;
        for (nth, one) in was.iter().enumerate() {
            self.sel = crate::selection::Selections::one(*one);
            self.pending = pending.clone();
            self.count = count;
            self.pending_register = named;
            what(self);
            if *one == primary {
                which = nth;
            }
            out.push(self.sel.primary());
        }
        let merged = self.sel.rebuild(out, which);
        self.say_the_merge(merged);
    }

    /// **逐段各編輯一次：從後往前做，只留一個撤銷點**（#405 Phase 1 第五步）。
    ///
    /// 和 [`Self::each_selection`] 是同一個形狀，多兩件事。
    ///
    /// ⚠️ **一、從後往前。** 一次編輯會把它後面所有的下標都挪掉，所以先做下標最大的那一
    /// 段：輪到前面那幾段的時候，它們記着的下標還是對的。
    ///
    /// ⚠️ **二、已經算完的結果要跟着挪。** 從後往前保住的是**輸入**，不是輸出：做完第
    /// 五段再去做第三段，第三段那一刀會把第四、第五段的新位置一起推走。所以每做完一段
    /// 就量一次文本長度的差，把手上收着的那幾段各挪一次。
    ///
    /// ⚠️ **三、一個撤銷點。** `what` 自己會叫 `snapshot`，N 段就是 N 個撤銷點，按一次
    /// `u` 只退回去一段。這裏先自己報一個點，然後開一個 undo group 把裏面那 N 次
    /// `snapshot` 全堵掉（`buffer.rs` 的 `begin_undo_group`），做完再放開。
    pub(super) fn edit_each(&mut self, what: impl Fn(&mut Self)) {
        self.edit_each_from(true, what);
    }

    /// 同上，`point` 說要不要自己報一個撤銷點。
    ///
    /// ⚠️ **插入模式下不報。** 進插入的那一下已經報過一個了，而這個倉的規矩是「一次插入
    /// 是一次撤銷」（§5.12.3）。每敲一鍵報一次，`u` 就只退一個字。
    pub(super) fn edit_each_from(&mut self, point: bool, what: impl Fn(&mut Self)) {
        if !self.sel.is_plural() {
            what(self);
            return;
        }
        if point {
            self.snapshot();
        }
        let grouping = self.current_buffer_mut().begin_undo_group();
        let was = self.sel.clone();
        let primary = was.primary();
        let pending = self.pending.clone();
        let count = self.count;
        // ⚠️ **`"` 指的那個寄存器也是一次性的**（2026-09-28）：`take_register` 是
        // **take**，第一趟就把它拿走了，後面幾段於是去讀無名的那一個，`"#p` 只有一段
        // 貼得上號。同 `pending` 和 `count`，每一段開跑之前擺回去。
        let named = self.pending_register;
        let mut out: Vec<Range> = Vec::with_capacity(was.len());
        let mut which = 0;
        let ranges: Vec<Range> = was.iter().copied().collect();
        for (nth, one) in ranges.iter().enumerate().rev() {
            let before = self.current_buffer().rope().len_chars();
            self.sel = crate::selection::Selections::one(*one);
            self.pending = pending.clone();
            self.count = count;
            self.pending_register = named;
            // ⚠️ **文檔次序，不是執行次序**：這一趟從後往前跑，而讀者數的是從上往下
            // 第幾個。`#` 寄存器讀它。
            self.edit_nth = Some(nth);
            what(self);
            self.edit_nth = None;
            let after = self.current_buffer().rope().len_chars();
            let moved = after as isize - before as isize;
            if moved != 0 {
                let top = after;
                for done in out.iter_mut() {
                    let shift = |at: usize| {
                        ((at as isize + moved).max(0) as usize).min(top)
                    };
                    done.anchor = shift(done.anchor);
                    done.head = shift(done.head);
                }
            }
            if *one == primary {
                which = nth;
            }
            out.push(self.sel.primary());
        }
        // 收的時候是從後往前收的，擺回去。⚠️ `which` 記的是**原來那一組**裏的下標，
        // 反過來之後纔對得上。
        out.reverse();
        self.current_buffer_mut().end_undo_group(grouping);
        let merged = self.sel.rebuild(out, which);
        self.clamp_cursor();
        self.refresh_goal_column();
        self.say_the_merge(merged);
    }

    /// **一段選區在屏幕上蓋住的是哪一截**（字符下標，左閉右開）。
    ///
    /// ⚠️ **`Range` 的 head 是包含在內的**（光標站的那個字素在選區裏，helix 的模型），
    /// 而「切開」「選出」這一族算的是半開區間。兩套下標混在一起是這一族最容易錯的地
    /// 方，所以進出各走一支。
    pub(super) fn drawn(&self, one: Range) -> (usize, usize) {
        let rope = self.current_buffer().rope();
        let (from, to) = one.span();
        (from, crate::motion::next_grapheme(rope, to))
    }

    /// 上一支的反面：半開區間變回一段選區。
    pub(super) fn from_drawn(&self, from: usize, to: usize) -> Range {
        let rope = self.current_buffer().rope();
        let head = crate::motion::prev_grapheme(rope, to).max(from);
        Range { anchor: from, head, goal: None, goal_slot: None }
    }

    /// **把每一段選區按行切開**（`A-s`，helix 的 `split_selection_on_newline`）。
    ///
    /// ⚠️ **一行選區不會被切成零段。** 選區只佔一行的時候這一支什麼都不改——那正是
    /// 「按行切」在只有一行上的答案，不是失敗。
    pub(super) fn split_on_newline(&mut self) {
        let rope = self.current_buffer().rope().clone();
        let mut out: Vec<Range> = Vec::new();
        let mut which = 0;
        let primary = self.sel.primary();
        for one in self.sel.iter() {
            if *one == primary {
                which = out.len();
            }
            let (from, to) = self.drawn(*one);
            let first = rope.char_to_line(from);
            let last = rope.char_to_line(to.saturating_sub(1).max(from));
            for line in first..=last {
                let head = rope.line_to_char(line);
                let a = head.max(from);
                let b = (head + crate::zong::line_chars(&rope, line).len()).min(to);
                if b > a {
                    out.push(self.from_drawn(a, b));
                }
            }
        }
        if out.is_empty() {
            self.status = say!("selection.nothing-to-split");
            return;
        }
        let merged = self.sel.rebuild(out, which);
        self.say_the_merge(merged);
    }

    /// 開那一扇提示行，並且記下 Enter 按下去要做哪一件。
    pub(super) fn open_sift(&mut self, what: Sift) {
        self.sift = Some(what);
        self.command_line.clear();
        self.command_caret = 0;
        self.mode = crate::input::Mode::Search;
    }

    /// **正則那一族**（`s`／`S`／`A-k`／`A-K`，#405 Phase 2）。
    ///
    /// 匹配器是搜索那一支（[`Look`]），所以拼音、簡繁、模糊、正則四個開關一起管用。
    ///
    /// ⚠️ **主選區留在離原來那一段最近的地方。** helix 這三個命令一律把 primary 重置成
    /// 0（`selection.rs` 三處都留着同一句 `// TODO: figure out a new primary index`），
    /// 於是在第八十行選出二十處之後，屏幕當場跳回檔首。§5.13.11 的坑 6。
    ///
    /// ⚠️ **一個都不剩就什麽都不做**，並且說一句。把選區清空是沒有這個狀態的
    /// （`Selections` 永遠至少一段），而靜靜地留在原地會讓人以為是鍵沒按上。
    pub(super) fn sift(&mut self, what: Sift) {
        let Some(look) = self.looker() else {
            self.status = say!("selection.sift-needs-a-pattern");
            return;
        };
        let rope = self.current_buffer().rope().clone();
        let was = self.sel.primary().span().0;
        let mut out: Vec<Range> = Vec::new();
        for one in self.sel.iter() {
            let (from, to) = self.drawn(*one);
            let text: String = rope.slice(from..to).chars().collect();
            let hits = look.spans(&text);
            match what {
                Sift::Select => {
                    for (a, b) in hits {
                        if b > a {
                            out.push(self.from_drawn(from + a, from + b));
                        }
                    }
                }
                Sift::Split => {
                    let mut cut = 0usize;
                    for (a, b) in hits {
                        if a > cut {
                            out.push(self.from_drawn(from + cut, from + a));
                        }
                        cut = b.max(cut);
                    }
                    let len = text.chars().count();
                    if cut < len {
                        out.push(self.from_drawn(from + cut, to));
                    }
                }
                Sift::Keep if !hits.is_empty() => out.push(*one),
                Sift::Drop if hits.is_empty() => out.push(*one),
                _ => {}
            }
        }
        if out.is_empty() {
            self.status = say!("selection.sift-found-nothing");
            return;
        }
        // 離原來的主選區最近的那一段。
        let which = out
            .iter()
            .enumerate()
            .min_by_key(|(_, r)| r.span().0.abs_diff(was))
            .map(|(nth, _)| nth)
            .unwrap_or(0);
        let count = out.len();
        let merged = self.sel.rebuild(out, which);
        self.clamp_cursor();
        self.refresh_goal_column();
        self.status = say!("selection.sift-done", count.to_string());
        self.say_the_merge(merged);
    }

    /// **換一個選區當主選區**（`)` 往後、`(` 往前，#405 Phase 3）。
    ///
    /// ⚠️ **一段都不動**，動的只是「哪一段是主的」。主選區是那些只能有一個的東西要挑
    /// 的那一段：終端的硬件光標、跟着光標跑的候選面板、頁面滾動跟誰。選了二十處之後要
    /// 一處一處看過去，靠的就是它。
    pub(super) fn rotate_primary(&mut self, forward: bool) {
        if !self.sel.is_plural() {
            self.status = say!("selection.already-one");
            return;
        }
        self.sel.turn(forward);
        self.clamp_cursor();
        self.refresh_goal_column();
        self.status = say!(
            "selection.which-one",
            (self.sel.primary_index() + 1).to_string(),
            self.sel.len().to_string()
        );
    }

    /// **把每一段兩端的空白去掉**（`_`，helix 的 `trim_selections`）。
    ///
    /// ⚠️ **整段都是空白的那些會被丟掉。** `A-s` 按行切開之後空行就是這一種，而留着它們
    /// 等於在空行上放一個光標——接着打字會在空行上寫東西。⚠️ 全丟光了就什麼都不做：
    /// 選區不能為空。
    pub(super) fn trim_selections(&mut self) {
        let rope = self.current_buffer().rope().clone();
        let mut out: Vec<Range> = Vec::new();
        let was = self.sel.primary().span().0;
        for one in self.sel.iter() {
            let (mut a, mut b) = self.drawn(*one);
            while a < b && rope.char(a).is_whitespace() {
                a += 1;
            }
            while b > a && rope.char(b - 1).is_whitespace() {
                b -= 1;
            }
            if b > a {
                out.push(self.from_drawn(a, b));
            }
        }
        if out.is_empty() {
            self.status = say!("selection.all-blank");
            return;
        }
        let which = out
            .iter()
            .enumerate()
            .min_by_key(|(_, r)| r.span().0.abs_diff(was))
            .map(|(nth, _)| nth)
            .unwrap_or(0);
        let merged = self.sel.rebuild(out, which);
        self.clamp_cursor();
        self.refresh_goal_column();
        self.say_the_merge(merged);
    }

    /// **把每一段選區的開頭對齊到同一列**（`&`，helix 的 `align_selections`）。
    ///
    /// 在最靠右的那一段之前的每一段前面補空格，補到大家的開頭在同一格上。
    ///
    /// ⚠️ **算的是顯示寬度，不是字數。** 一個漢字兩格，所以「三個字」和「三個字母」
    /// 對不齊——這一族在中文稿子裏用不對齊就等於沒用。
    ///
    /// ⚠️ **一行只認一段。** 同一行上有兩段的時候，補在前一段前面的空格會把後一段推
    /// 走，「對齊」就成了一句沒有意義的話。helix 同樣只處理每行第一段。
    ///
    /// ⚠️ **從後往前插**，並且把已經處理過的那幾段跟着挪——同 [`Self::edit_each`]，
    /// 理由也一樣。一個撤銷點。
    pub(super) fn align_selections(&mut self) {
        if !self.sel.is_plural() {
            self.status = say!("selection.already-one");
            return;
        }
        let rope = self.current_buffer().rope().clone();
        let mut out: Vec<Range> = self.sel.iter().copied().collect();
        let mut seen: Vec<usize> = Vec::new();
        let mut want = 0usize;
        // 每一段開頭在第幾格，以及要補幾格。
        let mut pads: Vec<Option<usize>> = Vec::with_capacity(out.len());
        for one in &out {
            let from = self.drawn(*one).0;
            let line = rope.char_to_line(from);
            if seen.contains(&line) {
                pads.push(None);
                continue;
            }
            seen.push(line);
            let head = rope.line_to_char(line);
            let col = yumete_cjk::str_width(&rope.slice(head..from).to_string());
            pads.push(Some(col));
            want = want.max(col);
        }
        if pads.iter().flatten().all(|&col| col == want) {
            self.status = say!("selection.already-aligned");
            return;
        }
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        for nth in (0..out.len()).rev() {
            let Some(col) = pads[nth] else { continue };
            let pad = want - col;
            if pad == 0 {
                continue;
            }
            let from = self.drawn(out[nth]).0;
            if self.edit_insert(from, &" ".repeat(pad)) {
                // 這一段自己、以及下標比它大的那幾段，都往後挪。
                for one in out.iter_mut().skip(nth) {
                    one.anchor += pad;
                    one.head += pad;
                }
            }
        }
        self.current_buffer_mut().end_undo_group(grouping);
        let which = self.sel.primary_index().min(out.len() - 1);
        let merged = self.sel.rebuild(out, which);
        self.clamp_cursor();
        self.refresh_goal_column();
        self.say_the_merge(merged);
    }

    /// **把每一段選區的文字輪轉一格**（`A-)` 往後、`A-(` 往前，helix 的
    /// `rotate_selection_contents_*`）。
    ///
    /// ⚠️ **邊界不動，動的是裝在裏面的字。** `(`／`)` 是換「哪一段是主的」，這一對是
    /// 把甲段的字搬到乙段去。表格裏換兩欄、對話裏換兩個人說的話，都是這一件。
    ///
    /// ⚠️ **各段長短不一，所以要從後往前換，並且把後面幾段跟着挪**——同
    /// [`Self::edit_each`]。一個撤銷點。
    pub(super) fn rotate_contents(&mut self, forward: bool) {
        if !self.sel.is_plural() {
            self.status = say!("selection.already-one");
            return;
        }
        let rope = self.current_buffer().rope().clone();
        let mut out: Vec<Range> = self.sel.iter().copied().collect();
        let count = out.len();
        let texts: Vec<String> = out
            .iter()
            .map(|one| {
                let (a, b) = self.drawn(*one);
                rope.slice(a..b).chars().collect()
            })
            .collect();
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        for nth in (0..count).rev() {
            // 往後輪：第 n 段拿的是第 n−1 段的字。
            let from = match forward {
                true => (nth + count - 1) % count,
                false => (nth + 1) % count,
            };
            let text = &texts[from];
            let (a, b) = self.drawn(out[nth]);
            let was = b - a;
            let now = text.chars().count();
            if !self.edit_remove(a..b) || !self.edit_insert(a, text) {
                break;
            }
            out[nth] = self.from_drawn(a, a + now);
            let moved = now as isize - was as isize;
            if moved != 0 {
                for one in out.iter_mut().skip(nth + 1) {
                    one.anchor = (one.anchor as isize + moved).max(0) as usize;
                    one.head = (one.head as isize + moved).max(0) as usize;
                }
            }
        }
        self.current_buffer_mut().end_undo_group(grouping);
        let which = self.sel.primary_index().min(out.len().saturating_sub(1));
        let merged = self.sel.rebuild(out, which);
        self.clamp_cursor();
        self.refresh_goal_column();
        self.say_the_merge(merged);
    }

    /// **只留主選區**（`,`）。
    pub(super) fn keep_primary_selection(&mut self) {
        match self.sel.keep_primary() {
            0 => self.status = say!("selection.already-one"),
            gone => self.murmur(say!("selection.kept-one", gone.to_string())),
        }
    }

    /// **合併是靜默地少掉一段，所以要說一句**（2026-09-28 定）。
    ///
    /// helix 不說。中文更常撞上：打一個字要按好幾下，相鄰的兩個光標很容易在中途撞到一
    /// 起，而那一刻屏幕上少一個光標、沒有任何提示。走 murmur 而不是 status，因為它不該
    /// 擋住鍵位提示。
    pub(super) fn say_the_merge(&mut self, merged: usize) {
        if merged > 0 {
            let left = self.sel.len().to_string();
            self.murmur(say!("selection.merged", merged.to_string(), left));
        }
    }
}

/// **哪些鍵要逐段各做一次**（#405 Phase 1 第四步）。
///
/// ⚠️ **這是一張明寫的表，不是規則。** 反過來寫（除了這幾個以外全都逐段做）試過在腦子裏
/// 推一遍就知道不行：`:`、空格選單、`u`、`/`、面板那一族、進插入的那幾個，每一個都是
/// 「整個編輯器做一次」的事，漏一個就是一次很難查的怪象。明寫的表漏掉一個鍵，症狀是那個
/// 鍵只動主選區，看得見、好查。
///
/// ⚠️ **不收會改文本的鍵。** 編輯會挪動後面每一段的下標，那要走第五步的 `edit_each`
/// （從後往前做，一個撤銷點）。
///
/// ⚠️ **不收要再等一個鍵的**（`f` `t` `g` `m` `[` `]` `空格` 這些前綴）：真正該逐段做的
/// 是**補上那個字符的時候**，不是按下前綴的時候。那一半在 `answer_with_char` 與
/// `handle_goto` 裏各包一次，見它們自己的註釋。
fn moves_every_selection(key: crate::input::Key) -> bool {
    use crate::input::Key;
    matches!(
        key,
        Key::Char('h' | 'l' | 'j' | 'k')
            | Key::Left
            | Key::Right
            | Key::Up
            | Key::Down
            | Key::Char('w' | 'b' | 'e' | 'W' | 'B' | 'E')
            | Key::Char(';')
            | Key::Alt(';')
            | Key::Char('x' | 'X')
    )
}

/// **這一鍵該不該逐段各做一次**——問的是鍵，也是**手上還等着什麽**。
///
/// ⚠️ **等着一個字符的時候，鍵本身說明不了問題。** 按 `f` 的那一下只是把 `Pending::Find`
/// 立起來，真正的移動發生在補上那個字符的時候，而那個字符可以是任何字——包括 `x`，而 `x`
/// 自己在上面那張表裏。所以先看 `pending`，再看鍵。
pub(super) fn each_selection_key(pending: &super::Pending, key: crate::input::Key) -> bool {
    match pending {
        // `f` `F` `t` `T` 補上的那一個字符：逐段各找各的。
        super::Pending::Find(_) => true,
        // ⚠️ **`g` 那一層只有幾個是移動。** `gf` 開檔、`gd` 看定義、`gw` 撒標籤，每一個
        // 都是「整個編輯器做一次」。⚠️ `gg`／`ge` 也不逐段做：它們是「到檔首／檔尾」，
        // N 段一起去同一個地方，`normalize` 會把它們併成一段——那不是使用者要的。
        // 逐段做的是**行內**的那三個：到行首、到行首第一個字、到行尾。
        super::Pending::Goto => {
            matches!(key, crate::input::Key::Char('h' | 's' | 'l'))
        }
        super::Pending::None => moves_every_selection(key),
        _ => false,
    }
}

/// **哪些鍵要逐段各編輯一次**（#405 Phase 1 第五步）。
///
/// ⚠️ **進插入模式的那幾個也在裏面**（`i` `a` `I` `A` `o` `O` `c` `A-c`）。它們有的不改
/// 文本（`i` 只是把光標挪到選區開頭），走同一支不虧：文本沒動的時候那一趟挪位是零。收進
/// 來的理由是它們要**逐段各進各的插入點**，然後插入模式那一支
/// （[`types_at_every_selection`]）接着把每一個鍵送到 N 處。
///
/// ⚠️ **`y` 不在。** 複製不改文本，可是 N 段複製出來在寄存器裏怎麽擺（helix 是各存一格、
/// 貼的時候一段對一段）是寄存器那一族的事，不是這一步的。
pub(super) fn edits_every_selection(pending: &super::Pending, key: crate::input::Key) -> bool {
    use crate::input::Key;
    match pending {
        // `r` 補上的那一個字符：逐段各換各的。
        super::Pending::Replace => true,
        super::Pending::None => matches!(
            key,
            Key::Char('d') | Key::Alt('d') | Key::Char('p' | 'P') | Key::Char('>' | '<')
        ) || matches!(
            key,
            Key::Char('i' | 'a' | 'I' | 'A' | 'o' | 'O' | 'c') | Key::Alt('c')
        // `C-a`／`C-x` 給每一段各加各的（#405 Phase 3）。
        ) || matches!(key, Key::Ctrl('a') | Key::Ctrl('x')),
        _ => false,
    }
}

/// **插入模式下，這一鍵要不要在每一段各做一次**（#405 Phase 1 第六步）。
///
/// 會在稿子上留下字的那幾個都要：打字、換行、退格、刪除、`Tab`。
///
/// ⚠️ **剩下的一次就夠**：`Esc` 是「離開插入模式」，補全單子那一族（`C-n`、`Tab` 在單子
/// 開着的時候）是一張浮在上面的單子，`C-g u` 是撤銷斷點。這些都是整個編輯器做一次的事。
///
/// ⚠️ **輸入法的 preedit 不在這裏。** 中文碼串還在 IME 手裏的時候一個鍵都不進
/// `on_insert_key`（`prompt.rs:26` 記着這個模型），上屏的那一下纔進來，那時它就是一串
/// 字符，和打拉丁字母走同一條路。**N 個光標同時畫出自己的拼音串**是另一件事，排在
/// Phase 4（§5.13.8 一）。
pub(super) fn types_at_every_selection(key: crate::input::Key) -> bool {
    use crate::input::Key;
    matches!(
        key,
        Key::Char(_) | Key::Enter | Key::Backspace | Key::Delete | Key::Tab
    )
}
