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
                        into.push(Range { anchor, head });
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
    /// ⚠️ **`goal_column` 還是一個**（`editor.rs` 上的欄位，不在 `Range` 裏）。所以每一
    /// 段跑之前先按它自己的位置重算一次，否則 N 段會一起瞄準主選區那一列。代價是**連按
    /// `j` 跨過一行短行之後，目標列記不住了**——單段的時候記得住。真要修就是把
    /// `goal_column` 搬進 `Range`（§5.13.9 說了「等它們真的需要各有一份的時候再搬」，
    /// 這就是那個時候，只是不在這一步）。
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
        for (nth, one) in was.iter().enumerate() {
            self.sel = crate::selection::Selections::one(*one);
            self.pending = pending.clone();
            self.count = count;
            self.refresh_goal_column();
            what(self);
            if *one == primary {
                which = nth;
            }
            out.push(self.sel.primary());
        }
        let merged = self.sel.rebuild(out, which);
        self.refresh_goal_column();
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
        if !self.sel.is_plural() {
            what(self);
            return;
        }
        self.snapshot();
        let grouping = self.current_buffer_mut().begin_undo_group();
        let was = self.sel.clone();
        let primary = was.primary();
        let pending = self.pending.clone();
        let count = self.count;
        let mut out: Vec<Range> = Vec::with_capacity(was.len());
        let mut which = 0;
        let ranges: Vec<Range> = was.iter().copied().collect();
        for (nth, one) in ranges.iter().enumerate().rev() {
            let before = self.current_buffer().rope().len_chars();
            self.sel = crate::selection::Selections::one(*one);
            self.pending = pending.clone();
            self.count = count;
            what(self);
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
/// ⚠️ **`c`／`A-c` 不在裏面，而它們是編輯。** 它們刪完就進插入模式，而插入模式下 N 個
/// 光標一起打字是另一件事（每按一鍵要在 N 處各寫一次，還要和輸入法的 preedit 對上）。
/// 刪了那一半、打字只落在主選區上，是比「只作用在主選區」更難看懂的狀態。所以整個
/// `c` 族先只動主選區，等插入模式那一步做完再收進來。
///
/// ⚠️ **`y` 也不在。** 複製不改文本，可是 N 段複製出來要在寄存器裏怎麽擺（helix 是各存
/// 一格、貼的時候一段對一段）是寄存器那一族的事，不是這一步的。
pub(super) fn edits_every_selection(pending: &super::Pending, key: crate::input::Key) -> bool {
    use crate::input::Key;
    match pending {
        // `r` 補上的那一個字符：逐段各換各的。
        super::Pending::Replace => true,
        super::Pending::None => matches!(
            key,
            Key::Char('d') | Key::Alt('d') | Key::Char('p' | 'P') | Key::Char('>' | '<')
        ),
        _ => false,
    }
}
