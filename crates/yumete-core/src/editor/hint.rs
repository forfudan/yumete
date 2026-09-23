//! The command row's standing content (#122, #302).
//!
//! What the keys mean from where the cursor is standing, one line of it.

use super::*;

impl Editor {
    /// What the row below the status line should say when nothing is being
    /// typed into it.
    ///
    /// The two rows answer two different questions and that is the whole
    /// design: the upper one is **where am I** — mode, file, position — and
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
        // 光標放上去的那一種：没什麽可走進去、也没什麽要收起來，所以這一行只說
        // 它真有的那兩個鍵（#293）。
        if let Some(side) = self.panel_focus() {
            if let Some(kind) = self.transient(side) {
                // Named for what it is holding: 字典 and 詳情 are two panels
                // with the same two keys, and the row that says only 「keys」
                // would leave a reader unsure which one has them.
                let what = crate::messages::say(crate::sidebar::Panel::from(kind).tag(), &[]);
                return Hint::Keys(what, vec![
                        ("j k", say!("hint.sidebar.move")),
                        ("C-w／空格 s", say!("hint.sidebar.back-to-text")),
                    ]);
            }
        }
        // **In the box, before anything about the panel around it** (#419):
        // the keys are in a field, and the one that is not obvious is that
        // `Enter` walks the hits without giving them back.
        if self.mode == Mode::Field {
            return Hint::Keys(say!("label.panel.search"), vec![
                    ("Enter", say!("hint.search.next-hit")),
                    ("Tab", say!("hint.search.next-cell")),
                    ("Esc", say!("hint.search.out-of-the-box")),
                ]);
        }
        // **Walking the hits: the row says what is around this one** (#419).
        // The panel is a column and a line of a novel is a paragraph, so the
        // excerpt there is a few characters; this row is the width of the
        // window. It displaces the panel's keys, and that is the right trade
        // while a reader is choosing which hit to go to.
        if let Some(line) = self.hit_in_context() {
            return Hint::Says(line);
        }
        // The search panel is a form, not a list: none of the tree's keys
        // mean anything in it (#419).
        if let Some(side) = self.panel_focus() {
            if self.transient(side).is_none()
                && self.panel(side).map(|p| p.view()) == Some(crate::sidebar::View::Search)
            {
                let mut keys = vec![
                    ("i", say!("hint.search.type")),
                    ("Tab", say!("hint.search.next-cell")),
                    ("Enter", say!("hint.search.use-it")),
                ];
                // Only when they do something: `r`/`R` are live on the
                // replacing panel and nowhere else.
                if self.search().replacing {
                    keys.push(("r R", say!("hint.search.replace")));
                }
                keys.push(("C-w／空格 s", say!("hint.sidebar.back-to-text")));
                keys.push(("q", say!("hint.close")));
                return Hint::Keys(say!("label.panel.search"), keys);
            }
        }
        if let Some(side) = self.panel_focus() {
            // **百科是一段文章，`j`／`k` 滾它**（2026-09-22 報的：那一頁翻不動）。
            // 別的視圖是行的列表，`j` 走下一行——同一個鍵兩件事，所以這一行得說
            // 對是哪一件。
            let walking = match self.panel(side).map(|p| p.view()) {
                Some(crate::sidebar::View::Wiki) => say!("hint.sidebar.scroll"),
                _ => say!("hint.sidebar.move"),
            };
            return Hint::Keys(say!("hint.sidebar"), vec![
                    ("j k", walking),
                    ("Tab", say!("hint.sidebar.other-view")),
                    ("w", say!("hint.sidebar.width")),
                    ("C-w／空格 s", say!("hint.sidebar.back-to-text")),
                    ("q", say!("hint.close")),
                ]);
        }
        // A reference standing half-typed, with the panel already open under
        // it: Tab is the key, and it is a key that means nothing anywhere else
        // in Insert, so nobody would try it unasked (#418).
        if self.reference_open() {
            return Hint::Keys(say!("hint.reference"), vec![
                ("Tab", say!("hint.reference.pick")),
            ]);
        }
        // Standing on a footnote reference, the key that shows the note is
        // worth saying: it is the one place `gd` has an answer that the reader
        // could not guess from the page.
        if self.mode == Mode::Normal && self.note_tag_at_cursor().is_some() {
            return Hint::Keys(say!("hint.footnote"), vec![
                ("gd", say!("hint.footnote.show-or-write")),
            ]);
        }
        match self.mode {
            Mode::Ruby if self.ruby_target.is_some() => {
                Hint::Keys(say!("hint.reading"), vec![("Enter", say!("hint.keep-it")), ("Esc", say!("hint.cancel"))])
            }
            // The one key worth saying inside a cell — without it a person
            // types a value, presses Esc, walks right and types the next.
            Mode::Insert if self.insert_bounds().is_some() => Hint::Keys(say!("hint.table.in-a-cell"), vec![("Tab", say!("hint.table.next-cell")), ("S-Tab", say!("hint.table.previous-cell"))]),
            Mode::Normal if self.table_here() => {
                // Three keys, and every one of them is a key the reader could
                // not have guessed: `t` opens the rest of the table's keys,
                // `T` is the only way to change grain, and `Tab` is the only
                // motion a table has that the text does not. `hjkl`, `c d`,
                // `y Y`, `p` were here too and are gone — they are the keys
                // this reader already has in the text, and `t` lists the
                // ones that are not.
                //
                // ⚠️ **The grain is `T`'s own label, and nowhere else** (#496).
                // The title said 「表格 · 字」 and the status line said 「· 字」
                // again a row above, for a fact `T 按字移動` was already
                // standing there stating — 「似乎不需要吧。因为挺明显的」. So the
                // title is just 表格 in both, and `T` says what pressing it
                // *does*: the grain you are not in.
                let grain = self.table.as_ref().map(|v| v.grain).unwrap_or(Grain::Cell);
                Hint::Keys(say!("label.table"), vec![
                    ("t", say!("hint.table.menu")),
                    ("T", match grain {
                        Grain::Cell => say!("hint.table.by-character-instead"),
                        Grain::Char => say!("hint.table.by-cell-instead"),
                    }),
                    ("Tab", say!("hint.table.next-cell")),
                ])
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
            // 撤銷已經斷了，剩下那個 `u` 吞不吞都行——没什麽要提示的。
            Pending::UndoBreak => return None,
            Pending::None => {
                // A count on its own is a sequence too — `3` is waiting for the
                // motion it multiplies.
                return self
                    .operator_count
                    .map(|n| Hint::Says(say!("hint.count-pending", n)));
            }
            // **A vim operator is waiting** (#429): one line rather than a
            // panel, because what it is waiting for is the whole vocabulary of
            // motions and the reader knows them — what they need to be told is
            // that the editor is still holding the `d`.
            Pending::VimOperator { op, first } => {
                return Some(Hint::Says(match first {
                    None => say!("hint.vim-operator", op),
                    Some(f) => say!("hint.vim-operator-more", op, f),
                }));
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
            Pending::Goto => (
                say!("hint.goto.title"),
                Self::said(match self.layout() == crate::zong::Layout::Vertical {
                    true => Self::GOTO_KEYS_VERTICAL.iter().copied(),
                    false => Self::GOTO_KEYS.iter().copied(),
                }),
            ),
            Pending::Find(_) => (say!("hint.find"), vec![("", say!("hint.type-a-character"))]),
            Pending::Replace => (say!("hint.overwrite"), vec![("", say!("hint.type-a-character-to-overwrite"))]),
            Pending::Case => (say!("hint.case.title"), Self::said(Self::CASE_KEYS.iter().copied())),
            Pending::Confirm => {
                (say!("hint.confirm.title"), Self::said(Self::CONFIRM_KEYS.iter().copied()))
            }
            Pending::ReplaceAll => (
                self.status.clone(),
                Self::said(Self::REPLACE_ALL_KEYS.iter().copied()),
            ),
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
    /// The same answer the command row has always had; it is a panel now because a
    /// row holds four of these and `空格` has fourteen.
    pub fn pending_menu(&self) -> Option<(String, Vec<(&'static str, String)>)> {
        match self.pending_keys()? {
            Hint::Keys(title, keys) => Some((title, keys)),
            _ => None,
        }
    }
}
