//! The hint row (#122).
//!
//! What the keys mean from where the cursor is standing, one line of it.

use super::*;

impl Editor {
    /// What the row above the status line should say.
    ///
    /// The two rows answer two different questions and that is the whole
    /// design: the bottom one is **where am I** — mode, file, position — and
    /// never changes shape, so the eye always finds the same thing in the same
    /// place; this one is **what just happened, and what can I press**, and is
    /// blank when there is neither.
    ///
    /// In priority order, because only one of them can be the answer: a message
    /// about the thing that just happened, then the keys that would finish a
    /// sequence already begun, then the keys of the pane or mode holding the
    /// keyboard. A key sequence a reader has begun and cannot finish is the
    /// worst of the three to be left alone with, but a message about what just
    /// happened is rarer and more urgent, so it wins.
    pub fn hint(&self) -> Hint {
        if !self.status.is_empty() {
            return Hint::Says(self.status.clone());
        }
        if let Some(keys) = self.pending_keys() {
            return keys;
        }
        if self.sidebar_focus && self.sidebar.is_some() {
            return Hint::Keys(say!("hint.sidebar"), vec![
                    ("j k", say!("hint.move")),
                    ("l", say!("hint.enter")),
                    ("h", say!("hint.sidebar.collapse")),
                    ("Tab", say!("hint.sidebar.other-view")),
                    ("w", say!("hint.sidebar.width")),
                    ("R", say!("hint.sidebar.re-read")),
                    ("C-w", say!("hint.sidebar.back-to-text")),
                    ("q", say!("hint.close")),
                ]);
        }
        // Standing on a footnote reference, the key that shows the note is
        // worth saying: it is the one place `gd` has an answer that the reader
        // could not guess from the page.
        if self.mode == Mode::Normal && self.note_tag_at_cursor().is_some() {
            return Hint::Keys(say!("hint.footnote"), vec![
                ("gd", say!("hint.footnote.show-or-write")),
                ("g/ g?", say!("hint.word-elsewhere")),
            ]);
        }
        match self.mode {
            Mode::Ruby if self.ruby_target.is_some() => {
                Hint::Keys(say!("hint.reading"), vec![("Enter", say!("hint.keep-it")), ("Esc", say!("hint.cancel"))])
            }
            // The one key worth saying inside a cell — without it a person
            // types a value, presses Esc, walks right and types the next.
            Mode::Insert if self.insert_bounds().is_some() => Hint::Keys(say!("hint.table.in-a-cell"), vec![("Tab", say!("hint.table.next-cell")), ("S-Tab", say!("hint.table.previous-cell")), ("Esc", say!("hint.back-to-normal"))]),
            Mode::Normal if self.table_here() => {
                let grain = self.table.as_ref().map(|v| v.grain).unwrap_or(Grain::Cell);
                let markdown = self.md_region().is_some();
                match grain {
                    Grain::Cell if markdown => Hint::Keys(say!("label.table"), vec![
                            ("hjkl", say!("hint.table.by-cell")),
                            ("c d", say!("hint.table.change-or-clear-cell")),
                            ("y Y", say!("hint.table.yank-cell-or-row")),
                            ("p", say!("hint.paste")),
                            ("t", say!("hint.table.operations")),
                            ("t/ t?", say!("hint.table.who-uses-this")),
                            ("Tab", say!("hint.table.by-character-instead")),
                        ]),
                    Grain::Cell => Hint::Keys(say!("label.table"), vec![
                            ("hjkl", say!("hint.table.by-cell")),
                            ("c d", say!("hint.table.change-or-clear-cell")),
                            ("y Y", say!("hint.table.yank-cell-or-row")),
                            ("p", say!("hint.paste")),
                            ("t", say!("hint.table.operations")),
                            ("t/ t?", say!("hint.table.who-uses-this")),
                            ("gd gw", say!("hint.table.row-this-is-about")),
                            ("Tab", say!("hint.table.by-character-instead")),
                        ]),
                    Grain::Char => Hint::Keys(say!("hint.table.character-mode"), vec![
                            ("hjkl", say!("hint.table.by-character")),
                            ("t/ t?", say!("hint.table.who-uses-this-character")),
                            ("gd gw", say!("hint.table.row-this-character-is-about")),
                            ("Tab", say!("hint.table.by-cell-instead")),
                        ]),
                }
            }
            _ => Hint::Quiet,
        }
    }

    /// The keys that would finish the sequence already begun.
    ///
    /// This is the row's most valuable use: a reader who has pressed `m` and
    /// does not remember what follows it currently has nowhere to look but the
    /// manual, and the editor is sitting there knowing the answer.
    fn pending_keys(&self) -> Option<Hint> {
        let keys = match self.pending {
            Pending::None => {
                // A count on its own is a sequence too — `3` is waiting for the
                // motion it multiplies.
                return self
                    .operator_count
                    .map(|n| Hint::Says(say!("hint.count-pending", n)));
            }
            Pending::Space => (
                say!("hint.space.title"),
                Self::SPACE_KEYS
                    .iter()
                    .map(|(key, what)| {
                        // Leaked once each, at most a dozen: the panel wants
                        // `&'static str` keys like every other row here, and a
                        // `char` is not one.
                        let key: &'static str = Box::leak(key.to_string().into_boxed_str());
                        (key, crate::messages::say(what, &[]))
                    })
                    .collect(),
            ),
            Pending::Goto => (say!("hint.goto.title"), Self::said(Self::GOTO_KEYS.iter().copied())),
            Pending::Find(_) => (say!("hint.find"), vec![("", say!("hint.type-a-character"))]),
            Pending::Replace => (say!("hint.overwrite"), vec![("", say!("hint.type-a-character-to-overwrite"))]),
            Pending::Case => (say!("hint.case.title"), Self::said(Self::CASE_KEYS.iter().copied())),
            Pending::Register => (say!("hint.register.title"), vec![("a–z", say!("hint.register.which-one"))]),
            Pending::Match => (say!("hint.match.title"), Self::said(Self::MATCH_KEYS.iter().copied())),
            Pending::MatchPair { .. } => (say!("hint.bracket"), vec![("", say!("hint.type-a-bracket-or-quote"))]),
            Pending::Surround => (say!("hint.match.surround"), vec![("", say!("hint.type-a-bracket"))]),
            Pending::SurroundFrom => (say!("hint.match.take-off"), vec![("", say!("hint.type-the-one-to-take-off"))]),
            Pending::SurroundTo(_) => (say!("hint.change-to"), vec![("", say!("hint.type-the-one-to-change-to"))]),
            Pending::Hop { forward } => (
                match forward {
                    true => say!("hint.hop.next"),
                    false => say!("hint.hop.previous"),
                },
                Self::said(Self::HOP_KEYS.iter().copied()),
            ),
            Pending::Conflict => (
                say!("hint.conflict.title"),
                Self::said(Self::CONFLICT_KEYS.iter().copied()),
            ),
            Pending::Mark => (say!("hint.mark.set-here"), vec![("a–z", say!("hint.mark.name-it"))]),
            Pending::Recall => (say!("hint.mark.go-back"), vec![("a–z", say!("hint.register.which-one"))]),
            // **Which list is a question about the cursor, not the mode.** It
            // used to be `md_region().is_none()`, which is *also* true of a
            // Markdown table nobody has opened yet — so standing in one of
            // 手冊's own tables offered the delimited file's keys.
            Pending::Table => {
                let inside = match self.table.as_ref().map(|v| v.bounds) {
                    Some(Bounds::Block) if self.block_region().is_some() => Some(Bounds::Block),
                    Some(Bounds::Md) if self.md_region().is_some() => Some(Bounds::Md),
                    Some(Bounds::WholeFile) => Some(Bounds::WholeFile),
                    _ => None,
                };
                (say!("hint.table.title"), Self::said(Self::table_keys(inside)))
            }
        };
        Some(Hint::Keys(keys.0, keys.1))
    }

    /// **What the half-pressed key can be finished with** — the which-key
    /// panel's whole content: a title, and each key with what it does.
    ///
    /// The same answer the hint row has always had; it is a panel now because a
    /// row holds four of these and `空格` has fourteen.
    pub fn pending_menu(&self) -> Option<(String, Vec<(&'static str, String)>)> {
        match self.pending_keys()? {
            Hint::Keys(title, keys) => Some((title, keys)),
            _ => None,
        }
    }
}
