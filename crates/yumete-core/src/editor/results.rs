//! **`]q` `[q` `]Q` `[Q` — walking the last listing from anywhere**, Neovim's
//! default meaning (`:cnext`, `:cprevious`, `:clast`, `:crewind`).
//!
//! A listing (`:check-names`, `:diagnostics-all`, `:git-diff`…) is a buffer of
//! rows `gf` can follow. Fixing what it found means going to a row, following
//! it, fixing, coming back for the next one; these keys are that round trip in
//! one press. The listing's own cursor is where the walk is, so going into the
//! listing and moving the cursor there moves the walk too, and the step itself
//! is `gf` on the row: whatever `gf` knows how to follow, the walk follows.
//!
//! A child module of `editor`, so `Editor`'s private fields are reachable here.

use super::*;

/// The listing `]q` walks, and what `gf` needs to follow its rows.
#[derive(Debug, Clone)]
pub(super) struct LastListing {
    pub(super) buffer: u64,
    pub(super) root: Option<PathBuf>,
    pub(super) jumps: Option<(PathBuf, Vec<usize>)>,
    /// Whether a row has been followed yet. Until one has, the row the cursor
    /// rests on is the next result rather than the current one: a listing
    /// opens with its cursor on its first line, and on a listing whose first
    /// line is a result `]q` would otherwise skip it.
    pub(super) walked: bool,
}

/// Which result to go to, with the count it was given.
#[derive(Debug, Clone, Copy)]
pub(super) enum ResultStep {
    Next(usize),
    Prev(usize),
    /// `[Q`: the first, or the `n`th.
    First(Option<usize>),
    /// `]Q`: the last, or the `n`th.
    Last(Option<usize>),
}

impl Editor {
    /// Remember the listing just added as the one `]q` walks.
    pub(super) fn keep_as_last_listing(&mut self, buffer: u64) {
        self.last_listing = Some(LastListing {
            buffer,
            root: self.listing_root.clone(),
            jumps: self.listing_jumps.clone(),
            walked: false,
        });
    }

    /// Take the slots again, for a listing that set them after it was shown
    /// (`:diagnostics-all`'s root, `:git-diff`'s jumps).
    pub(super) fn keep_listing_slots(&mut self) {
        let (root, jumps) = (self.listing_root.clone(), self.listing_jumps.clone());
        if let Some(list) = &mut self.last_listing {
            list.root = root;
            list.jumps = jumps;
        }
    }

    /// `gf` from the last listing's own buffer makes that row the current
    /// result, as `Enter` in Neovim's quickfix window does.
    pub(super) fn note_listing_followed(&mut self) {
        let here = self.current_buffer().id();
        if let Some(list) = self.last_listing.as_mut().filter(|l| l.buffer == here) {
            list.walked = true;
        }
    }

    /// `]q` `[q` `]Q` `[Q`.
    ///
    /// Warning: **no wrapping**: at the end it stops and says so, as Neovim's
    /// `No more items`. A walk that wraps comes back to the first result
    /// without a word, and fixing it a second time is how that is noticed.
    /// A count larger than what is left goes as far as there is (Vim's
    /// `get_nth_valid_entry`).
    pub(super) fn go_to_result(&mut self, step: ResultStep) {
        let Some(list) = self.last_listing.clone() else {
            self.status = say!("listing.none");
            return;
        };
        let Some(index) = self.buffers.iter().position(|b| b.id() == list.buffer) else {
            self.status = say!("listing.closed");
            return;
        };
        let rows = self.result_rows(index, &list);
        if rows.is_empty() {
            self.status = say!("listing.no-results");
            return;
        }
        let at = match index == self.current {
            true => self.sel.head(),
            false => self.buffers[index].saved_cursor(),
        };
        let here = self.buffers[index].rope().char_to_line(at);
        let nth = |from: &[usize], n: usize| from[(n.max(1) - 1).min(from.len() - 1)];
        let to = match step {
            ResultStep::Next(n) => {
                let ahead: Vec<usize> = rows
                    .iter()
                    .copied()
                    .filter(|&r| r > here || (r == here && !list.walked))
                    .collect();
                if ahead.is_empty() {
                    self.status = say!("listing.no-more");
                    return;
                }
                nth(&ahead, n)
            }
            ResultStep::Prev(n) => {
                let behind: Vec<usize> = rows.iter().rev().copied().filter(|&r| r < here).collect();
                if behind.is_empty() {
                    self.status = say!("listing.no-more");
                    return;
                }
                nth(&behind, n)
            }
            ResultStep::First(n) => nth(&rows, n.unwrap_or(1)),
            ResultStep::Last(Some(n)) => nth(&rows, n),
            ResultStep::Last(None) => rows[rows.len() - 1],
        };
        let place = rows.iter().position(|&r| r == to).unwrap_or(0) + 1;
        let came_from = self.current_buffer().id();
        let was_last = self.last_file;
        self.show_buffer(index);
        let rope = self.current_buffer().rope();
        let indent = rope.line(to).chars().take_while(|c| *c == ' ' || *c == '\t').count();
        let start = rope.line_to_char(to) + indent;
        self.set_cursor(start);
        // The slots as they were for this listing: a report shown since may
        // have written its own root into them.
        self.listing_root = list.root;
        self.listing_jumps = list.jumps;
        self.goto_file_under_cursor();
        if self.current_buffer().id() == list.buffer {
            // `gf` could not open it, and said why.
            return;
        }
        // **`ga` goes back to the file the walk came from**, not to the
        // listing it passed through on the way.
        self.last_file = match self.current_buffer().id() == came_from {
            true => was_last,
            false => Some(came_from),
        };
        self.status = say!("listing.at", place, rows.len());
    }

    /// The rows of a listing `gf` can follow, top to bottom.
    ///
    /// A row the listing says where it goes (`listing_jumps`), or a
    /// `path:line` naming a file that is there. Headings, blank lines and
    /// a server's line at the top are passed over, as Vim passes over a
    /// quickfix line that names no file.
    fn result_rows(&self, index: usize, list: &LastListing) -> Vec<usize> {
        let rope = self.buffers[index].rope();
        let told = |row: usize| {
            list.jumps
                .as_ref()
                .and_then(|(_, jumps)| jumps.get(row))
                .is_some_and(|&at| at > 0)
        };
        let named = |row: usize| {
            let line = rope.line(row).to_string();
            let Some((path, rest)) = line.trim().split_once(':') else {
                return false;
            };
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            let ends = rest[digits..].chars().next().is_none_or(|c| c == ':' || c.is_whitespace());
            let path = Path::new(path.trim());
            if digits == 0 || !ends || path.as_os_str().is_empty() {
                return false;
            }
            match (&list.root, path.is_absolute()) {
                (Some(root), false) => root.join(path).is_file(),
                _ => path.is_file(),
            }
        };
        (0..rope.len_lines()).filter(|&row| told(row) || named(row)).collect()
    }
}
