//! 簡繁, handed to opencc (#241).

use super::*;

impl Editor {
    // ---- 簡繁, handed to opencc (Feature #241) ----------------------------

    /// `:convert` — every shape of it.
    pub(super) fn convert(&mut self, ask: crate::command::ConvertAsk) {
        use crate::command::ConvertAsk;
        use crate::convert::{self, Snag};
        match ask {
            ConvertAsk::Explain => self.explain_convert(),
            ConvertAsk::Opencc { update } => {
                let line = match update {
                    true => convert::install_line().replace("install", "upgrade"),
                    false => convert::install_line().to_string(),
                };
                // Only where one command does it without a password. Elsewhere
                // the line is *said*, because a full-screen terminal
                // application is the wrong place to be asked for a sudo
                // password — and because which package manager this machine
                // has is not something the editor should guess at and then run.
                if !convert::install_runs_here() {
                    self.status = say!("convert.install-it-yourself", line);
                    return;
                }
                self.status = say!("convert.installing", line);
                self.shell_request = Some(Shell {
                    line,
                    how: How::Terminal,
                });
            }
            ConvertAsk::Run { from, to, force } => {
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
                // The way out of 通規 happens here rather than in the child:
                // opencc's configs cannot read those 字形, so what it is handed
                // has to be 字形 it knows.
                // ⚠️ **换哪一段，在這裏就定下來。** 整本是 `None`；`` ` `` 那一
                // 組先把選區放進 `convert_range`，這裏照它取字。
                let whole = self.convert_range.is_none();
                let mut text = match &self.convert_range {
                    Some(range) => self.current_buffer().rope().slice(range.clone()).to_string(),
                    None => self.current_buffer().rope().to_string(),
                };
                let _ = whole;
                if let Some(side) = plan.unpatch {
                    text = convert::unrespell(&text, side);
                }
                let Some(config) = plan.config else {
                    // 繁體 to 繁體: 字形 tables and nothing else, so there is
                    // no program to be missing and no process to spawn.
                    if let Some(side) = plan.patch {
                        text = convert::respell(&text, side);
                    }
                    self.rewrite_with_conversion(&text);
                    return;
                };
                let Some(program) = convert::opencc() else {
                    self.status = say!("convert.opencc-needed", convert::install_line());
                    return;
                };
                self.convert_patch = plan.patch;
                self.shell_request = Some(Shell {
                    line: convert::command_line(&program, config),
                    how: How::Convert(text),
                });
            }
        }
    }

    /// What `:convert` on its own answers: what it can do, and whether the
    /// program it needs is here.
    fn explain_convert(&mut self) {
        use crate::convert::{self, Side};
        let mut listing = String::new();
        listing.push_str(&match convert::opencc() {
            Some(path) => say!("convert.opencc-at", path.display()),
            None => say!("convert.opencc-needed", convert::install_line()),
        });
        listing.push_str("\n\n");
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

    /// **`` ` `` 那一組：把選區換一種寫法**（2026-09-22 提的）。
    ///
    /// `:convert` 換的是整本書；這個換選區。⚠️ **收在 `` ` `` 底下而不是另起一個
    /// 前綴**：那一組本來就是「把選區裏的字換一種寫法」（`` `l `` 轉小寫、
    /// `` `u `` 轉大寫），簡繁與大小寫是同一類事，多一個前綴就多一份要記的東西。
    ///
    /// ⚠️ **一鍵一檔，第二個字母各不相同**：`` `w ``／`` `h `` 而不是
    /// `` `tw ``／`` `hk ``——後者裏 `` `t `` 既是完整命令又是 `` `tw `` 的前綴，
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
        self.convert_range = Some(start..end);
        self.convert(crate::command::ConvertAsk::Run { from, to, force: false });
    }

    /// What opencc said, patched and put in the buffer.
    pub fn provide_conversion(&mut self, output: &str) {
        let patched = match self.convert_patch.take() {
            Some(side) => crate::convert::respell(output, side),
            None => output.to_string(),
        };
        // opencc answers an empty file with an empty file and exit 0, so
        // 「nothing came back」 is only a finding when something went in.
        if patched.is_empty() && self.current_buffer().char_count() > 0 {
            self.convert_range = None;
            self.status = say!("convert.nothing-came-back");
            return;
        }
        match self.convert_range.take() {
            Some(range) => self.rewrite_range_with_conversion(range, &patched),
            None => self.rewrite_with_conversion(&patched),
        }
    }

    /// **換掉選區那一段**（2026-09-22），其餘一個字不動。
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
        self.anchor = self.cursor;
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
