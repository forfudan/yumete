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
                let mut text = self.current_buffer().rope().to_string();
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

    /// What opencc said, patched and put in the buffer.
    pub fn provide_conversion(&mut self, output: &str) {
        let patched = match self.convert_patch.take() {
            Some(side) => crate::convert::respell(output, side),
            None => output.to_string(),
        };
        // opencc answers an empty file with an empty file and exit 0, so
        // 「nothing came back」 is only a finding when something went in.
        if patched.is_empty() && self.current_buffer().char_count() > 0 {
            self.status = say!("convert.nothing-came-back");
            return;
        }
        self.rewrite_with_conversion(&patched);
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
