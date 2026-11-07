//! **Where vim's `zt` `zz` `zb` leave the cursor** when scrolloff is at
//! least half the window (nvim 0.12, measured 2026-11-07 with
//! `scripts/oracle/view_sweep.py`).
//!
//! With that much scrolloff there is no room for it above and below at
//! once, and the answer near either end of the file comes out of how vim's
//! scrolling functions count, not out of a rule short enough to state:
//! `zt` near the end keeps scrolloff lines above (the lines below it are
//! not there to fit), `zz` there is corrected for scrolloff only while the
//! page is full, and so on, a line or two either way. So this is
//! `scroll_cursor_top`, `scroll_cursor_halfway`, `scroll_cursor_bot` and
//! the `update_topline` that follows them, from vim's `move.c`, with a line
//! a row, no folds and 'scrolljump' 1. Held against nvim over windows of
//! three to sixteen rows, scrolloff 0 to 9 and every line of short and long
//! files, it agrees everywhere; the editor asks it only where its own,
//! row-wise answer did not.

use super::Aim;

/// The window over the file, in vim's terms: lines count from 1, `top` is
/// the first line shown and `cursor` the cursor's.
struct View {
    lines: usize,
    height: usize,
    so: usize,
    cursor: usize,
    top: usize,
}

/// More than any window is tall: what vim counts a line past the end as.
const PAST_THE_END: isize = isize::MAX / 2;

impl View {
    /// The first line below the window, one past the file's last when the
    /// window reaches it (`w_botline`).
    fn botline(&self) -> usize {
        (self.top + self.height).min(self.lines + 1)
    }

    /// The rows past the file's end (`w_empty_rows`).
    fn empty_rows(&self) -> usize {
        self.height - (self.botline() - self.top)
    }

    /// `scroll_cursor_halfway`: lines added below and above the cursor in
    /// turns until the window is full; past the end, when `at_end`, a row
    /// counts as a line.
    fn halfway(&mut self, at_end: bool, prefer_above: bool) {
        let (mut above, mut below, mut used) = (0, 0, 1);
        let (mut up, mut down) = (self.cursor, self.cursor);
        let mut top = self.cursor;
        'fill: while top > 1 {
            for round in 1..=2 {
                let add_below = match prefer_above {
                    true => round == 2 && below < above,
                    false => round == 1 && below <= above,
                };
                if add_below {
                    if down < self.lines {
                        down += 1;
                        used += 1;
                        if used > self.height {
                            break 'fill;
                        }
                    } else if at_end {
                        used += 1;
                    }
                    below += 1;
                }
                let add_above = match prefer_above {
                    true => round == 1 && below >= above,
                    false => round == 1 && below > above,
                };
                if add_above {
                    up -= 1;
                    used += 1;
                    if used > self.height {
                        break 'fill;
                    }
                    above += 1;
                    top = up;
                }
            }
        }
        self.top = top;
    }

    /// `scroll_cursor_top`: as many lines above the cursor as scrolloff
    /// asks, counting the lines below it scrolloff would want too; when
    /// they do not fit, `halfway`.
    fn scroll_top(&mut self, min_scroll: usize, always: bool) {
        let (mut used, mut extra) = (1, 0);
        let mut scrolled = if self.cursor < self.top { used } else { 0 };
        let (mut top, mut bot) = (self.cursor - 1, self.cursor + 1);
        let mut new_top = self.cursor;
        while top > 0 {
            if top < self.top {
                scrolled += 1;
            }
            if (new_top >= self.top || scrolled > min_scroll) && extra >= self.so {
                break;
            }
            used += 1;
            // vim's own bound: the file's last line is not counted here.
            if extra < self.so && bot < self.lines {
                used += 1;
            }
            if used > self.height {
                break;
            }
            extra += 1;
            new_top = top;
            top -= 1;
            bot += 1;
        }
        if used > self.height {
            self.halfway(false, false);
        } else {
            if new_top < self.top || always {
                self.top = new_top;
            }
            self.top = self.top.min(self.cursor);
        }
    }

    /// `scroll_cursor_bot`: the window ending at the cursor line first when
    /// `set_top` (`zb`), then scrolled on until scrolloff lines are below
    /// it, or centred when that is more than a window.
    fn scroll_bottom(&mut self, min_scroll: usize, set_top: bool) {
        let (lines, height, so, cursor) = (self.lines, self.height, self.so, self.cursor);
        let (botline, empty) = match set_top {
            true => {
                self.top = cursor.saturating_sub(height - 1).max(1);
                (cursor + 1, height - (cursor + 1 - self.top))
            }
            false => (self.botline(), self.empty_rows()),
        };
        let (empty, min) = (empty as isize, min_scroll as isize);
        let mut used = 1;
        let mut scrolled: isize = 0;
        if cursor >= botline {
            scrolled = 1;
            if cursor == botline {
                scrolled -= empty;
            }
        }
        let (mut up, mut down, mut extra) = (cursor, cursor, 0);
        while up > 1 {
            if (((scrolled <= 0 || scrolled >= min) && extra >= so) || down + 1 > lines) && up <= botline {
                break;
            }
            let before = up;
            up -= 1;
            used += 1;
            if used > height {
                break;
            }
            if up >= botline {
                scrolled += 1;
                if up == botline && before > botline {
                    scrolled -= empty;
                }
            }
            if down < lines {
                let was = down;
                down += 1;
                used += 1;
                if used > height {
                    break;
                }
                if extra < so || scrolled < min {
                    extra += 1;
                    if down >= botline {
                        scrolled += 1;
                        if was < botline {
                            scrolled -= empty;
                        }
                    }
                }
            }
        }
        let line_count = if scrolled <= 0 {
            0
        } else if used > height {
            used
        } else {
            let (mut count, mut seen, mut line) = (0, 0, self.top - 1);
            while seen < scrolled && line < botline {
                line += 1;
                seen += if line <= lines { 1 } else { PAST_THE_END };
                count += 1;
            }
            if seen < scrolled { 9999 } else { count }
        };
        if line_count >= height && line_count > min_scroll {
            self.halfway(false, true);
        } else if line_count > 0 {
            self.top = (self.top + line_count).min(lines);
        }
    }

    /// `update_topline`: scrolloff kept above the cursor unless the file's
    /// first line is shown, and below it unless its last is.
    fn settle(&mut self) {
        let (lines, height, so, cursor) = (self.lines, self.height, self.so, self.cursor);
        let mut below_too = true;
        if self.top > 1 && cursor < self.top + so {
            let half = (height / 2).saturating_sub(1).max(2);
            if self.top + so - cursor >= half {
                self.halfway(false, false);
                below_too = false;
            } else {
                self.scroll_top(1, false);
            }
        }
        let botline = self.botline();
        if !below_too || botline > lines {
            return;
        }
        let short = match cursor < botline {
            true => cursor + so >= botline && botline - 1 - cursor < so,
            false => true,
        };
        if short {
            match cursor + 1 + so - botline <= height + 1 {
                true => self.scroll_bottom(1, false),
                false => self.halfway(false, false),
            }
        }
    }
}

/// The row (from 0) vim's `zt` (`Aim::Top`), `zz` (`Aim::Middle`) or `zb`
/// (`Aim::Bottom`) leaves the cursor on, it being on line `line` (from 0)
/// of `lines`, in a window `height` rows tall with scrolloff `so`.
pub(super) fn z_row(aim: Aim, line: usize, lines: usize, height: usize, so: usize) -> usize {
    let mut view = View { lines: lines.max(1), height: height.max(1), so, cursor: line + 1, top: 1 };
    view.cursor = view.cursor.min(view.lines);
    match aim {
        Aim::Top => view.scroll_top(0, true),
        Aim::Bottom => view.scroll_bottom(0, true),
        _ => view.halfway(true, false),
    }
    // nvim corrects `zz` for scrolloff only while the window is full of
    // the file (measured: on the last lines it stays centred).
    if !(matches!(aim, Aim::Middle) && view.empty_rows() > 0) {
        view.settle();
    }
    view.cursor - view.top
}
