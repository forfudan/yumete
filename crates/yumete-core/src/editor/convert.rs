//! 簡繁, by the built-in OpenCC (#241).

use super::*;

impl Editor {
    // ---- 簡繁, by the built-in OpenCC (Feature #241) ----------------------

    /// `:convert` — every shape of it.
    pub(super) fn convert(&mut self, ask: crate::command::ConvertAsk) {
        use crate::command::ConvertAsk;
        match ask {
            ConvertAsk::Explain => self.explain_convert(),
            ConvertAsk::Run { from, to, force } => self.run_conversion(from, to, force, None),
        }
    }

    /// Convert the whole book (`range` is `None`) or one stretch of it, here
    /// and now: OpenCC is linked in, so there is nothing to wait for and the
    /// answer goes straight into the buffer as one undo step.
    fn run_conversion(
        &mut self,
        from: crate::convert::Side,
        to: crate::convert::Side,
        force: bool,
        range: Option<std::ops::Range<usize>>,
    ) {
        use crate::convert::{self, Snag};
        let plan = match convert::plan(from, to, force) {
            Ok(plan) => plan,
            Err(Snag::Same) => {
                self.status = say!("convert.same-side", from.word(), to.word());
                return;
            }
            Err(Snag::NoRoute) => {
                self.status = say!("convert.no-route", from.word(), to.word());
                return;
            }
            Err(Snag::NoWords) => {
                self.status = say!("convert.no-force", from.word(), to.word());
                return;
            }
        };
        let text = match &range {
            Some(range) => self.current_buffer().rope().slice(range.clone()).to_string(),
            None => self.current_buffer().rope().to_string(),
        };
        let converted = match convert::apply(&plan, &text) {
            Ok(converted) => converted,
            Err(config) => {
                self.status = say!("convert.config-failed", config);
                return;
            }
        };
        match range {
            Some(range) => self.rewrite_range_with_conversion(range, &converted),
            None => self.rewrite_with_conversion(&converted),
        }
    }

    /// What `:convert` on its own answers: what it can do.
    fn explain_convert(&mut self) {
        use crate::convert::{self, Side};
        let mut listing = String::new();
        for side in Side::ALL {
            listing.push_str(&match side {
                Side::S => say!("convert.side-s"),
                Side::T => say!("convert.side-t"),
                Side::Tw => say!("convert.side-tw"),
                Side::Hk => say!("convert.side-hk"),
                Side::Jp => say!("convert.side-jp"),
                Side::C => say!("convert.side-c"),
                Side::G => say!("convert.side-g"),
            });
            listing.push('\n');
        }
        listing.push('\n');
        for side in Side::ALL {
            let there = convert::destinations(side);
            if there.is_empty() {
                continue;
            }
            let words: Vec<String> = there
                .iter()
                .map(|(to, force)| match force {
                    true => format!("{} [force]", to.word()),
                    false => to.word().to_string(),
                })
                .collect();
            // The tags are one and two letters wide, so the arrows line up
            // only if the short ones are padded — a column that wobbles by one
            // cell reads as a mistake in a page whose whole job is a table.
            listing.push_str(&say!(
                "convert.can-go",
                format!("{:<2}", side.word()),
                words.join("  ")
            ));
            listing.push('\n');
        }
        listing.push('\n');
        listing.push_str(&say!("convert.force-means"));
        listing.push('\n');
        listing.push_str(&say!("convert.undo-with-u"));
        listing.push('\n');
        self.show_listing(listing, say!("convert.what-it-can-do"));
        self.status = say!("convert.pick-two");
    }

    /// **`` ` `` 那一組：把選區換一種寫法**。
    ///
    /// `:convert` 換的是整本書；這個換選區。Warning: **收在 `` ` `` 底下而不是另起一個
    /// 前綴**：那一組本來就是「把選區裏的字換一種寫法」（`` `l `` 轉小寫、
    /// `` `u `` 轉大寫），簡繁與大小寫是同一類事，多一個前綴就多一份要記的東西。
    ///
    /// Warning: **一鍵一檔，第二個字母各不相同**：`` `w ``/`` `h `` 而不是
    /// `` `tw ``/`` `hk ``——後者裏 `` `t `` 既是完整命令又是 `` `tw `` 的前綴，
    /// 只能靠超時去猜，而那正是 vim 裏最招人煩的一類行為。
    ///
    /// 没有選區就換光標底下那一個字，同 [`Self::map_selection`]。
    pub(super) fn convert_selection(&mut self, from: crate::convert::Side, to: crate::convert::Side) {
        let (start, end) = self.selection();
        let end = match end == start {
            true => crate::motion::right(self.current_buffer().rope(), start).max(start + 1),
            false => end,
        };
        let end = end.min(self.current_buffer().char_count());
        if start >= end {
            self.status = say!("convert.nothing-picked");
            return;
        }
        self.run_conversion(from, to, false, Some(start..end));
    }

    /// **換掉選區那一段**，其餘一個字不動。
    ///
    /// 與整本那一支同一條規矩：一次 `snapshot`，所以 `u` 一下全回來——反着換
    /// **不是**回頭路（`s2tw` 再 `tw2s`，「頭髮」回不去）。
    fn rewrite_range_with_conversion(&mut self, range: std::ops::Range<usize>, text: &str) {
        let long = self.current_buffer().char_count();
        let range = range.start.min(long)..range.end.min(long);
        if range.is_empty() {
            return;
        }
        let before = self.current_buffer().rope().slice(range.clone()).to_string();
        if before == text {
            self.status = say!("convert.not-a-character");
            return;
        }
        self.snapshot();
        let done = self
            .without_cell_guard(|e| e.current_buffer_mut().replace(range.clone(), text));
        if !self.applied(done) {
            return;
        }
        // 換完站在換出來的那一段頭上，選區收起來——那一段已經不是原來那些字了。
        self.set_cursor(range.start.min(self.current_buffer().char_count()));
        self.sel.set_anchor(self.sel.head());
        self.status = say!("convert.done-here", text.chars().count());
    }

    /// Put a converted manuscript in place of the one that was there.
    ///
    /// One `snapshot`, so one `u` takes the whole book back. That matters more
    /// here than in any other command: the reverse direction is **not** the way
    /// back — `s2tw` then `tw2s` returns 头发 for 頭髮 and 里 for both 裡 and
    /// 裏 — so undo is the only thing that undoes this.
    fn rewrite_with_conversion(&mut self, text: &str) {
        let before = self.current_buffer().rope().to_string();
        if before == text {
            self.status = say!("convert.not-a-character");
            return;
        }
        // **The same check `:s` and `:replace` make** (#350). A whole-document
        // rewrite lifts the cell guard, so the one thing table mode promises —
        // that a row never gains or loses a cell — is only kept if whoever
        // lifts the guard asks. `:s` asked, `:replace` asked, and the two
        // commands that rewrite the *entire* book did not.
        if let Some(why) = self.substitution_breaks_the_grid(text) {
            self.status = why;
            return;
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, text));
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
        self.forget_the_text();
        let (was, now) = (before.chars().count(), text.chars().count());
        self.status = match was == now {
            // The ordinary case, and the one worth a number: every 簡繁 step
            // but `force` is one character for one, so 「how many changed」 is
            // exactly how much of the book moved.
            true => say!(
                "convert.characters-changed",
                before
                    .chars()
                    .zip(text.chars())
                    .filter(|(a, b)| a != b)
                    .count()
            ),
            false => say!("convert.length-changed", was, now),
        };
    }
}
