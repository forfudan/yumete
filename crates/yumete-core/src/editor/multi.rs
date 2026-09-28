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
