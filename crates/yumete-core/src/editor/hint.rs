//! The command row's standing content (#122, #302).
//!
//! What the keys mean from where the cursor is standing, one line of it.

use super::*;

/// **`C-w`／`空格 w` 這個鍵的名字，按界面語言寫。**
///
/// 2026-09-27 報的：英文界面上印着 `C-w／空格 w next region`——一個漢字和一個
/// 全角斜槓。鍵位那一欄的型別是 `&'static str`（`hjkl` 在哪種語言裏都是
/// `hjkl`），所以這裏逐語言各寫一個字面量，不去泄漏一份新的字串：這一行每一幀
/// 都算一次。
fn back_to_text_key() -> &'static str {
    match crate::messages::language() {
        crate::messages::Language::English => "C-w / Space w",
        _ => "C-w／空格 w",
    }
}

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
                        (back_to_text_key(), say!("hint.sidebar.back-to-text")),
                    ]);
            }
        }
        // **In the box, before anything about the panel around it** (#419):
        // the keys are in a field, and the one that is not obvious is what
        // `Enter` does with them.
        //
        // ⚠️ **`Enter` 兩種範圍下說同一句** (2026-09-25 定)。從前本文件那一種是
        // 「下一處」、鍵留在框裏，跨檔那一種是「開找」、鍵落到結果上——一個鍵兩個
        // 意思，這一行只好分開說。現在兩種都是「去看結果」。
        //
        // ⚠️ **This row used to say 「Tab 下一格」 in both states, and `Tab`
        // does not do that in either** (2026-09-24 審出來的). In the box it
        // falls through to nothing; in the panel it walks the slot's views.
        // The next cell is `↓`.
        if self.mode == Mode::Field {
            // **換字那兩個鍵在框裏按不了，所以框裏那一行要指路**（2026-09-25
            // 報的：「替换模式下如何替换？快捷鍵是什麼？如何全部替换？」）。
            // 從前 `r R` 只在鍵已經回到面板之後纔出現在提示行上，而人還在框裏
            // 打「換成什麼」的時候，正是他要問這句話的時候。
            let out = match self.search().replacing {
                true => say!("hint.search.out-then-replace"),
                false => say!("hint.search.out-of-the-box"),
            };
            return Hint::Keys(say!("label.panel.search"), vec![
                    ("Enter", say!("hint.search.go-look")),
                    ("↑ ↓", say!("hint.search.next-cell")),
                    ("Esc", out),
                ]);
        }
        // ⚠️ **這一行不再拿來預覽了**（2026-09-27 定，原話：「命令行现在不显示
        // 预览了，所以可以空出来显示按键提示」）。從前站在一處命中上，這一行寫的
        // 是那一處前後的句子——而預覽現在在正文裏：`jk` 走一步，正文就跳過去、選
        // 區蓋上去（`Editor::show_hit`）。同一句話說兩遍，佔掉的正是讀者最需要看
        // 見鍵位的那一刻。
        // The search panel is a form, not a list: none of the tree's keys
        // mean anything in it (#419).
        if let Some(side) = self.panel_focus() {
            if self.transient(side).is_none()
                && self.panel(side).map(|p| p.view()) == Some(crate::sidebar::View::Search)
            {
                // ⚠️ **這一行是硬砍的，所以次序就是重要性**（2026-09-27 三個試
                // 用的人各自撞上）。從前排頭的是 `/` 和 `d c a`，而砍在尾巴上的
                // 是 `u 撤回`——一個是最不常用的，一個是按錯之後唯一的退路。英文
                // 界面下整行要 170 欄纔寫得完，所以尾巴一定會被砍掉。
                let mut keys: Vec<(&'static str, String)> = Vec::new();
                // **會改稿子的那三個排最前。** 它們只在替換那一檔活着，而它們是
                // 這扇面板裏唯一沒有別處可學的鍵。
                if self.search().replacing {
                    // ⚠️ **`r` 說的是站着的這一行**：檔名那一行上它換整個檔，
                    // 命中那一行上它換那一處。一句話寫兩種意思的時候（「換這處
                    // （站在檔名上就是整個檔）」），這一行就長到把後面的 `q` 擠
                    // 出畫面——而讀者站在哪一行，編輯器自己看得見。
                    let here = match self.search().row() {
                        Some(crate::search_panel::Row::File { .. }) => {
                            say!("hint.search.replace-file")
                        }
                        _ => say!("hint.search.replace-here"),
                    };
                    keys.push(("r", here));
                    keys.push(("R", say!("hint.search.replace-all")));
                    keys.push(("u", say!("hint.search.undo")));
                }
                // The five switches are pressed by number and walked past
                // (2026-09-24) — the row that walks is 範圍／找什麼／結果.
                keys.push(("1–7", say!("hint.search.switches")));
                keys.push(("Enter", say!("hint.search.use-it")));
                keys.push(("q", say!("hint.close")));
                keys.push((back_to_text_key(), say!("hint.sidebar.back-to-text")));
                keys.push(("/", say!("hint.search.new-word")));
                // **站在一個框上纔說得着編輯鍵**（2026-09-25）。站在結果上它們一個
                // 都不管用，而這一行擠不下說了也用不上的東西。
                if self.search().takes_text() {
                    keys.push(("d c a", say!("hint.search.edit-in-place")));
                }
                let title = match self.search().replacing {
                    true => say!("label.panel.replace"),
                    false => say!("label.panel.search"),
                };
                return Hint::Keys(title, keys);
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
            // 六個鍵一格：`j k` 一行、`J K` 半頁、`g G` 兩頭。從前只寫 `j k`，
            // 另外四個管用卻沒人知道（2026-09-23 審出來的）。
            return Hint::Keys(say!("hint.sidebar"), vec![
                    ("j k J K g G", walking),
                    ("Tab", say!("hint.sidebar.other-view")),
                    ("w", say!("hint.sidebar.width")),
                    (back_to_text_key(), say!("hint.sidebar.back-to-text")),
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
                    // 2026-09-21 表格組搬到了 `空格 t`；這一行 2026-09-23 纔跟上。
                    ("空格 t", say!("hint.table.menu")),
                    ("T", match grain {
                        Grain::Cell => say!("hint.table.by-character-instead"),
                        Grain::Char => say!("hint.table.by-cell-instead"),
                    }),
                    ("Tab", say!("hint.table.next-cell")),
                ])
            }
            _ => self.the_way_back_to_the_list(),
        }
    }

    /// **The panel is still open and the keys are in the text** — say how to
    /// get back (2026-09-27).
    ///
    /// `Enter` on a result hands the keys to the page and leaves the list
    /// standing there, which is the point: you read the sentence around the
    /// hit in its own context. But the row below went **blank** at that
    /// moment, so a list of eleven hits was on screen with nothing saying how
    /// to walk them or how to get back into it — and the manual's own rule is
    /// that the half holding the keys owes the reader the way out.
    ///
    /// Last of all the branches, so it only ever fills a row that would
    /// otherwise be empty: a pending key, a footnote, a table cell all have
    /// more to say from where the cursor is actually standing.
    fn the_way_back_to_the_list(&self) -> Hint {
        if self.mode != Mode::Normal || !self.search_panel_is_open() {
            return Hint::Quiet;
        }
        let title = match self.search().replacing {
            true => say!("label.panel.replace"),
            false => say!("label.panel.search"),
        };
        Hint::Keys(title, vec![
            ("n N", say!("hint.search.walk-the-hits")),
            (back_to_text_key(), say!("hint.search.back-to-the-list")),
        ])
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
            // ⚠️ **`#` 也要寫上**（2026-09-28）。加了那個寄存器卻沒改這一行，於是屏幕
            // 明明白白告訴讀者「只收 a–z」，而 `"#p` 其實是通的——按下去沒信心是對的。
            Pending::Register => (
                say!("hint.register.title"),
                vec![
                    ("a–z", say!("hint.register.which-one")),
                    ("#", say!("hint.register.hash")),
                ],
            ),
            Pending::Match => (say!("hint.match.title"), Self::said(Self::MATCH_KEYS.iter().copied())),
            // `z` 那一層只有三個鍵，一張三行的小表比一句話好認。
            Pending::Aim => (
                say!("hint.aim.title"),
                Self::said(
                    [
                        ("z", "hint.aim.middle"),
                        ("t", "hint.aim.top"),
                        ("b", "hint.aim.bottom"),
                    ]
                    .into_iter(),
                ),
            ),
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
