//! The command row's standing content (#122, #302).
//!
//! What the keys mean from where the cursor is standing, one line of it.

use super::*;

/// **走到下一區那個鍵的名字**：`\u{2423}w`。
///
/// `\u{2423}`（OPEN BOX）就是空格鍵，一根橫綫兩端向上。2026-09-29 定，原話：
/// 「不用显示 C-w……_ 其实是空格符号，就是短横+两端两个向上的竖线的符号。这样的
/// 话节约空间而且用户也能理解。」
///
/// Warning: **`C-w` 不寫了。** 同一件事兩種按法，而鍵位那一行是硬砍的——兩種都寫
/// 要十一格，寫一種只要三格，省下的八格後面還排着別的鍵。`C-w` 照樣管用。
///
/// Warning: **這一支從前分語言**（英文界面寫 `C-w / Space w`）。`\u{2423}` 在哪
/// 種語言裏都是那個記號，所以不必分了。
fn back_to_text_key() -> &'static str {
    "\u{2423}w"
}

impl Editor {
    /// **The motions a vim operator may end with, as one line** (2026-10-06).
    ///
    /// Warning: **It used to be a hand-written list inside the message**, and it had
    /// drifted: it said `w e b W E B $ 0 ^ G gg j k { } f i a` while the
    /// editor also took `h l ␣ F t T ge gE H L ; ,` and, since the surround
    /// round, `s`. A list of keys is data; writing it out a second time in
    /// prose is how the two come apart. The wording around it did not change.
    fn vim_motion_list() -> String {
        // The table's own order, which groups them the way a reader learnt
        // them (words, then the line, then down the page, then find).
        let mut out: Vec<&str> = yumete_cjk::keymap::VIM_MOTIONS.iter().map(|(k, _)| *k).collect();
        // Warning: **The table is not the whole truth** (2026-10-06): what
        // `vim_operator_key` actually asks is [`crate::vim::step_for`], and
        // that match arm takes more than `VIM_MOTIONS` lists. `h`/`l`/`␣` are
        // one grapheme; `ge`/`gE` are vim's 「back to the end of the previous
        // word」, which the table deliberately leaves out because helix's `ge`
        // means something else (that reason stopped applying when `vim.rs`
        // took them, 2026-10-02); `;`/`,` are 「that find again」 and live in
        // `vim_operator_key` because the editor remembers which find it was;
        // `s` is vim-surround's.
        out.extend(["h", "l", "␣", "ge", "gE", ";", ",", "s"]);
        out.join(" ")
    }

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
        // **信息那一格：讀，不是走進去**（#293、#426）。這一行只說它真有的那
        // 幾個鍵，而標題寫的是**此刻擺着哪一種**——五種共用一格，只寫「信息」
        // 的話讀者不知道眼前這一則是字典還是診斷。
        if let Some(side) = self.panel_focus() {
            // Warning: **問的是「這一格是信息嗎」，不是「它此刻有東西嗎」**
            // （2026-09-30 看圖看出來的）。空着的時候問後者會落到下面那一行通用
            // 的邊欄提示上——而那一行寫着 `Tab 換視圖`、不寫 `PgUp PgDn`，於是
            // 一個空面板教的是一套它沒有的鍵。標題同理：空着寫「信息」。
            if self.panel(side).map(|p| p.view()) == Some(crate::sidebar::View::Info) {
                let what = match self.info_in_this_sidebar(side) {
                    Some(one) => crate::messages::say(one.tag(), &[]),
                    None => say!("label.panel.info"),
                };
                // Warning: **一段文章是「滾」，一張單子是「上下」**（2026-09-22
                // 定的分別，2026-09-30 差點丟掉）。百科軟折行，`j` 走的是一屏
                // 行；字典、記錄、診斷是一條一條，`j` 走的是一條。同一個鍵兩
                // 件事，這一行得說對是哪一件。
                let walk = match self.info_in_this_sidebar(side) {
                    Some(crate::sidebar::Info::Wiki) => say!("hint.sidebar.scroll"),
                    _ => say!("hint.sidebar.move"),
                };
                return Hint::Keys(what, vec![
                        // Warning: **`J K g G` 也寫上**（2026-09-30 審出來的）。
                        // 它們在這一格上真的管用，而通用那一行一格就寫得下
                        // `j k J K g G`——只寫 `j k` 是漏報。
                        ("j k J K g G".into(), walk),
                        // Warning: **`w` 也要寫上**（2026-09-29 報的：「不仅没有提示
                        // 而且 w 无效」）。兩件事一起壞的：這一行沒說它，而它
                        // 本來也真的不管用（見 `on_info_key`）。
                        ("w".into(), say!("hint.sidebar.width")),
                        ("q".into(), say!("hint.close")),
                        (back_to_text_key().into(), say!("hint.sidebar.back-to-text")),
                    ]);
            }
        }
        // **In the box, before anything about the panel around it** (#419):
        // the keys are in a field, and the one that is not obvious is what
        // `Enter` does with them.
        //
        // Warning: **`Enter` 兩種範圍下說同一句** (2026-09-25 定)。從前本文件那一種是
        // 「下一處」、鍵留在框裏，跨檔那一種是「開找」、鍵落到結果上——一個鍵兩個
        // 意思，這一行只好分開說。現在兩種都是「去看結果」。
        //
        // Warning: **This row used to say 「Tab 下一格」 in both states, and `Tab`
        // does not do that in either** (2026-09-24 審出來的). In the box it
        // falls through to nothing; in the panel it walks the slot's views.
        // The next cell is `↓`.
        // **挑選器的鍵寫在這一行，不寫在面板裏**（2026-10-01 定，原話：「快捷
        // 键文案是不是可以收到命令行中？」）。搜索面板一直是這樣，而挑選器從前把
        // 鍵寫在自己的腳注上、命令行那一行空着——騰出來的那一行歸列表。
        //
        // 用的詞和搜索面板那一行是同一批（`hint.search.box-*`）：兩扇的框 2026-10-01
        // 起是同一套鍵，說法也該是同一套。
        if let Some(picker) = self.picker() {
            let title = picker.title.clone();
            if picker.typing() {
                return Hint::Keys(title, vec![
                    ("Enter".into(), say!("hint.picker.open")),
                    ("↑ ↓".into(), say!("hint.sidebar.move")),
                    ("Esc".into(), say!("hint.picker.back-to-the-list")),
                ]);
            }
            return Hint::Keys(title, vec![
                ("j k".into(), say!("hint.sidebar.move")),
                ("Enter".into(), say!("hint.picker.open")),
                ("q".into(), say!("hint.close")),
                ("d D".into(), say!("hint.search.box-delete")),
                ("c C".into(), say!("hint.search.box-change")),
                ("a A".into(), say!("hint.search.box-append")),
                ("i I".into(), say!("hint.search.box-insert")),
            ]);
        }
        if self.mode == Mode::Field {
            // **換字那兩個鍵在框裏按不了，所以框裏那一行要指路**（2026-09-25
            // 報的：「替换模式下如何替换？快捷鍵是什麼？如何全部替换？」）。
            // 從前 `r R` 只在鍵已經回到面板之後纔出現在提示行上，而人還在框裏
            // 打「換成什麼」的時候，正是他要問這句話的時候。
            return Hint::Keys(say!("label.panel.search"), vec![
                    ("Enter".into(), say!("hint.search.go-look")),
                    ("↑ ↓".into(), say!("hint.search.next-cell")),
                    ("Esc".into(), say!("hint.search.out-of-the-box")),
                ]);
        }
        // Warning: **這一行不再拿來預覽了**（2026-09-27 定，原話：「命令行现在不显示
        // 预览了，所以可以空出来显示按键提示」）。從前站在一處命中上，這一行寫的
        // 是那一處前後的句子——而預覽現在在正文裏：`jk` 走一步，正文就跳過去、選
        // 區蓋上去（`Editor::show_hit`）。同一句話說兩遍，佔掉的正是讀者最需要看
        // 見鍵位的那一刻。
        // The search panel is a form, not a list: none of the tree's keys
        // mean anything in it (#419).
        if let Some(side) = self.panel_focus() {
            if self.panel(side).map(|p| p.view()) == Some(crate::sidebar::View::Search) {
                // Warning: **這一行是硬砍的，所以次序就是重要性**（2026-09-27 三個試
                // 用的人各自撞上）。從前排頭的是 `/` 和 `d c a`，而砍在尾巴上的
                // 是 `u 撤回`——一個是最不常用的，一個是按錯之後唯一的退路。英文
                // 界面下整行要 170 欄纔寫得完，所以尾巴一定會被砍掉。
                let mut keys: KeyRows = Vec::new();
                // **改稿子的那兩個排最前，而且只在用得上的時候纔畫**
                // （2026-09-29 定，原話：「光标不在结果上，不应该显示『r ....』
                // 的提示。因此如果用户按了 r，也不需要任何提示」，`R` 同）。
                //
                // Warning: 從前它們跟着「勾了替換」一起出現，於是站在搜索框上也寫着
                // `r 換這一處`——按下去只換來一句解釋為什麽沒反應。鍵不在那裏
                // 的時候就別說它在。
                //
                // Warning: **`u 撤銷` 一個字都不寫**（同日定：「理論上用戶是知道 u 是
                // 撤銷的……這個功能應該是常駐功能」）。這一行是硬砍的，常駐的鍵
                // 佔着一格就是把別的擠出畫面。
                if self.search().replacing && self.search().field == crate::search_panel::Field::Results {
                    // Warning: **`r` 說的是站着的這一行**：檔名那一行上它換整個檔，
                    // 命中那一行上它換那一處。讀者站在哪一行，編輯器自己看得見。
                    match self.search().row() {
                        Some(crate::search_panel::Row::File { .. }) => {
                            keys.push(("r".into(), say!("hint.search.replace-file")));
                        }
                        Some(crate::search_panel::Row::Hit(_)) => {
                            keys.push(("r".into(), say!("hint.search.replace-here")));
                        }
                        None => {}
                    }
                    if !self.search().hits.is_empty() {
                        keys.push(("R".into(), say!("hint.search.replace-all")));
                    }
                }
                // The five switches are pressed by number and walked past
                // (2026-09-24) — the row that walks is 範圍／找什麼／結果.
                //
                // Warning: **第七行只有走磁碟的範圍下纔畫**（2026-10-02 查出來的）。這
                // 一行從前無條件寫着 `1–7`，而本文件和緩衝區那兩檔只有六行——按
                // 下去的 `7` 什麼都不做也什麼都不說，而屏幕上寫着它管用。一個寫在
                // 屏幕上、按下去沒反應的鍵，讀者只會以為自己記錯了（§5.12.39
                // 那一族）。
                let numbers = match self.search().on_disk() {
                    true => "1–7",
                    false => "1–6",
                };
                keys.push((numbers.into(), say!("hint.search.switches")));
                keys.push(("Enter".into(), say!("hint.search.use-it")));
                keys.push(("q".into(), say!("hint.close")));
                keys.push((back_to_text_key().into(), say!("hint.sidebar.back-to-text")));
                keys.push(("/".into(), say!("hint.search.new-word")));
                // **站在一個框上纔說得着編輯鍵**（2026-09-25）。站在結果上它們一個
                // 都不管用，而這一行擠不下說了也用不上的東西。
                //
                // 四對，大小寫並排（2026-09-29 定的寫法：「dD 删除 cC 修改
                // aA 追加 iI 插入」）。大寫那一半管到末尾／管到框首框尾，小寫
                // 管光標底下那一個字。
                if self.search().takes_text() {
                    keys.push(("dD".into(), say!("hint.search.box-delete")));
                    keys.push(("cC".into(), say!("hint.search.box-change")));
                    keys.push(("aA".into(), say!("hint.search.box-append")));
                    keys.push(("iI".into(), say!("hint.search.box-insert")));
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
                // 信息那一格在上面自己答完了，走不到這裏。
                Some(crate::sidebar::View::Info) => say!("hint.sidebar.move"),
                _ => say!("hint.sidebar.move"),
            };
            // 六個鍵一格：`j k` 一行、`J K` 半頁、`g G` 兩頭。從前只寫 `j k`，
            // 另外四個管用卻沒人知道（2026-09-23 審出來的）。
            return Hint::Keys(say!("hint.sidebar"), vec![
                    ("j k J K g G".into(), walking),
                    ("Tab".into(), say!("hint.sidebar.other-view")),
                    ("w".into(), say!("hint.sidebar.width")),
                    (back_to_text_key().into(), say!("hint.sidebar.back-to-text")),
                    ("q".into(), say!("hint.close")),
                ]);
        }
        // A reference standing half-typed, with the panel already open under
        // it: Tab is the key, and it is a key that means nothing anywhere else
        // in Insert, so nobody would try it unasked (#418).
        if self.reference_open() {
            return Hint::Keys(say!("hint.reference"), vec![
                ("Tab".into(), say!("hint.reference.pick")),
            ]);
        }
        // Standing on a footnote reference, the key that shows the note is
        // worth saying: it is the one place `gd` has an answer that the reader
        // could not guess from the page.
        if self.mode == Mode::Normal && self.note_tag_at_cursor().is_some() {
            return Hint::Keys(say!("hint.footnote"), vec![
                ("gd".into(), say!("hint.footnote.show-or-write")),
            ]);
        }
        match self.mode {
            Mode::Ruby if self.ruby_target.is_some() => {
                Hint::Keys(say!("hint.reading"), vec![("Enter".into(), say!("hint.keep-it")), ("Esc".into(), say!("hint.cancel"))])
            }
            // The one key worth saying inside a cell — without it a person
            // types a value, presses Esc, walks right and types the next.
            Mode::Insert if self.insert_bounds().is_some() => Hint::Keys(say!("hint.table.in-a-cell"), vec![("Tab".into(), say!("hint.table.next-cell")), ("S-Tab".into(), say!("hint.table.previous-cell"))]),
            Mode::Normal if self.table_here() => {
                // Three keys, and every one of them is a key the reader could
                // not have guessed: `t` opens the rest of the table's keys,
                // `Tab` is the only motion a table has that the text does not,
                // and `空格 I` puts the row in the sidebar. `hjkl`, `c d`,
                // `y Y`, `p` were here too and are gone — they are the keys
                // this reader already has in the text, and `t` lists the
                // ones that are not.
                //
                // Warning: **按格移動不在這一行上了**（2026-09-30 定，原話「因为
                // T 按格移动被折叠到 _t 中了，所以这个提示也就不需要了」）。它
                // 從前寫在這裏是因為 `T` 是頂層的鍵；`空格 t T` 收了它之後，這
                // 一行再寫一遍就是把三格裏最貴的一格花在單子上已經有的東西上。
                Hint::Keys(say!("label.table"), vec![
                    // 2026-09-21 表格組搬到了 `空格 t`；這一行 2026-09-23 纔跟上。
                    ("\u{2423}t".into(), say!("hint.table.menu")),
                    ("Tab".into(), say!("hint.table.next-cell")),
                    // **這一行的記錄去邊欄**（2026-09-30 提）。浮窗那一個是
                    // 小寫的 `空格 i`，與別的四種信息同一條規矩——大寫進邊欄。
                    ("\u{2423}I".into(), say!("hint.table.into-the-sidebar")),
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
        // **浮着一則信息的時候，這一行說怎麽把它送進邊欄**（2026-09-30 定）。
        //
        // 原話：「`␣K 進邊欄` 似乎反而不是很重要的信息，可以在命令行是空的时候
        // 放到命令行中。」——浮窗的底邊那一頭改寫「怎麽翻頁」，那件事更常用；
        // 「送進邊欄」一天用不了一次，擺在一行本來就空着的地方正好。
        if self.mode == Mode::Normal && self.info_afloat().is_some() {
            return Hint::Says(say!("info.into-the-sidebar", self.the_key_into_the_info_panel()));
        }
        if self.mode != Mode::Normal || !self.search_panel_is_open() {
            return Hint::Quiet;
        }
        let title = match self.search().replacing {
            true => say!("label.panel.replace"),
            false => say!("label.panel.search"),
        };
        Hint::Keys(title, vec![
            ("n N".into(), say!("hint.search.walk-the-hits")),
            (back_to_text_key().into(), say!("hint.search.back-to-the-list")),
        ])
    }

    /// The keys that would finish the sequence already begun.
    ///
    /// This is the row's most valuable use: a reader who has pressed `m` and
    /// does not remember what follows it currently has nowhere to look but the
    /// manual, and the editor is sitting there knowing the answer.
    fn pending_keys(&self) -> Option<Hint> {
        // **`go`／`gu` 還在等字母，命令行說一句**（2026-10-04 定）。
        //
        // Warning: **按下 `o`／`u` 的那一刻 `pending` 就回到了 `None`**，於是 HUD
        // 空了、`g` 那扇菜單也收了——而這兩個鍵恰恰是**還要再打幾個字母才算完**
        // 的。屏幕上一個字都不說話，人以為自己回到了 Normal，照 Normal 的習慣按
        // 鍵。原話：「我以为我现在在 normal 模式，但其实 yumete 是在等我打拼
        // 音。這其實有些危險的。」`gw` 沒有這一族：它一按下去標籤就滿屏幕都是。
        //
        // Warning: **說的是一句話，不是再畫一遍 `g` 那扇菜單。** 做過那一版，當場
        // 被否：菜單上那些鍵這時候一個都按不了，而且 `gu` 的候選框一開，兩扇浮窗
        // 疊在同一屏上。這裏走 `Says`，和 vim 的 `d` 等動作同一條路。
        //
        // 標籤亮起來就閉嘴：那時屏幕上全是標籤，自己會說話。
        if let Some(seeking) = self.seeking.as_ref().filter(|s| s.labels.is_empty()) {
            // 打進去的那幾個也跟在後面（2026-10-04 定：「請輸入拼音：don 這個
            // 可以有」）。HUD 旁邊那塊牌子寫的是整串 `gudon`，這裏寫的是查詢本身。
            let so_far = match seeking.typed.is_empty() {
                true => String::new(),
                false => format!("：{}", seeking.typed),
            };
            return Some(Hint::Says(match seeking.reading {
                true => say!("seek.ask-reading", so_far),
                false => say!("seek.ask-letters", so_far),
            }));
        }
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
                    None => say!("hint.vim-operator", op, Self::vim_motion_list()),
                    Some(f) => say!("hint.vim-operator-more", op, f),
                }));
            }
            Pending::Space => (
                say!("hint.space.title"),
                Self::SPACE_KEYS
                    .iter()
                    // Warning: These keys are `char`, and a `char` is not a
                    // `&'static str`. This used to `Box::leak` one string per
                    // key to make it into one; the key column is a `Cow` now,
                    // so the owned string simply goes in.
                    .map(|(key, what)| {
                        (key.to_string().into(), crate::messages::say(what, &[]))
                    })
                    .collect(),
            ),
            Pending::Goto => (
                say!("hint.goto.title"),
                Self::said(
                    match self.layout() == crate::zong::Layout::Vertical {
                        true => Self::GOTO_KEYS_VERTICAL.iter().copied(),
                        false => Self::GOTO_KEYS.iter().copied(),
                    }
                    .chain(
                        match self.key_preset == yumete_cjk::KeyPreset::Vim {
                            true => Self::GOTO_KEYS_VIM,
                            false => &[],
                        }
                        .iter()
                        .copied(),
                    ),
                ),
            ),
            Pending::Find(_) => (say!("hint.find"), vec![("".into(), say!("hint.type-a-character"))]),
            Pending::Replace => (say!("hint.overwrite"), vec![("".into(), say!("hint.type-a-character-to-overwrite"))]),
            Pending::Case => (say!("hint.case.title"), Self::said(Self::CASE_KEYS.iter().copied())),
            Pending::TableConvert => (
                say!("hint.table.convert"),
                Self::said(Self::TABLE_CONVERT_KEYS.iter().copied()),
            ),
            Pending::Confirm => {
                (say!("hint.confirm.title"), Self::said(Self::CONFIRM_KEYS.iter().copied()))
            }
            Pending::ReplaceAll => (
                self.status.clone(),
                Self::said(Self::REPLACE_ALL_KEYS.iter().copied()),
            ),
            // **每一格裝着什麼，就畫什麼**（2026-09-29 定：「Helix這個好」）。
            //
            // 從前這張表寫的是說明——「a–z 哪一個」——而那句話回答不了任何人的問
            // 題：a 是哪一個？哪一個什麼？helix 的答案是不解釋，直接把格子裏存着的
            // 那段字印在右邊（`helix-view/src/info.rs:60`，寬 30），於是「a 是一個
            // 存東西的格子」不必說，看一眼就知道了。**空的格子不列**，所以一個都
            // 沒存過的時候這張表上只有 `#` 一行。
            //
            // Warning: `#` 不是格子，是問題的答案（見 `edits::recall`），所以它沒有內容
            // 可印，只能寫一句說明，也只有它一行是說明。它排在最後。
            Pending::Register => {
                let mut named: Vec<(&char, &String)> = self.registers.iter().collect();
                named.sort();
                let mut rows: KeyRows = named
                    .into_iter()
                    .map(|(name, text)| (name.to_string().into(), Self::register_preview(text)))
                    .collect();
                rows.push(("#".into(), say!("hint.register.hash")));
                (say!("hint.register.title"), rows)
            }
            Pending::Match => (say!("hint.match.title"), Self::said(Self::MATCH_KEYS.iter().copied())),
            // `z` 那一層只有三個鍵，一張三行的小表比一句話好認。
            Pending::Aim => (
                say!("hint.aim.title"),
                Self::said(
                    [
                        ("z", "hint.aim.middle"),
                        ("t", "hint.aim.top"),
                        ("b", "hint.aim.bottom"),
                    ],
                ),
            ),
            Pending::MatchPair { .. } => (say!("hint.bracket"), vec![("".into(), say!("hint.type-a-bracket-or-quote"))]),
            Pending::Surround => (say!("hint.match.surround"), vec![("".into(), say!("hint.type-a-bracket"))]),
            Pending::SurroundOff => (say!("hint.match.take-off"), vec![("".into(), say!("hint.type-the-one-to-take-off"))]),
            Pending::SurroundFrom => (say!("hint.change"), vec![("".into(), say!("hint.type-the-one-being-replaced"))]),
            Pending::SurroundTo(_) => (say!("hint.change-to"), vec![("".into(), say!("hint.type-the-one-to-change-to"))]),
            Pending::Hop { forward } => (
                match forward {
                    true => say!("hint.hop.next"),
                    false => say!("hint.hop.previous"),
                },
                Self::said(Self::HOP_KEYS.iter().copied().chain(
                    match forward {
                        true => Self::HOP_KEYS_FORWARD,
                        false => Self::HOP_KEYS_BACK,
                    }
                    .iter()
                    .copied(),
                )),
            ),
            // **區域那一組**（2026-09-30，照 helix 的 `C-w`）。
            Pending::Region => (say!("hint.region.title"), Self::said(Self::REGION_KEYS.iter().copied())),
            Pending::Conflict => (
                say!("hint.conflict.title"),
                Self::said(Self::CONFLICT_KEYS.iter().copied()),
            ),
            Pending::Mark => (say!("hint.mark.set-here"), vec![("a–z".into(), say!("hint.mark.name-it"))]),
            // **哪幾個字母記過位置，分別記在哪**（2026-09-29，同 `"` 那一張）。
            //
            // 一個標記存的是位置而不是文字，所以右邊印的是「檔名 第幾行」。這一層
            // 與 `"` 共用 a–z 這一套名字卻是兩本帳：`"a` 裝的是一段話，`' a` 記的
            // 是一個地方。
            //
            // Warning: **一個都沒記過的時候也要畫出一行來。** 空的 `Body::Keys` 畫不出
            // 框（`panel.rs` 直接回 `None`），而按了 `'` 屏幕上什麽都不出，讀起來
            // 就是「這個鍵壞了」。
            Pending::Recall => {
                let mut named: Vec<(&char, &Spot)> = self.marks.iter().collect();
                named.sort_by_key(|(name, _)| **name);
                let rows: KeyRows = match named.is_empty() {
                    true => vec![("".into(), say!("hint.mark.none-yet"))],
                    false => named
                        .into_iter()
                        .map(|(name, spot)| (name.to_string().into(), self.mark_place(spot)))
                        .collect(),
                };
                (say!("hint.mark.go-back"), rows)
            }
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
    /// **Where one mark points**, for the panel that lists them.
    ///
    /// A mark in a file knows its path and line without opening anything; one
    /// in a scratch buffer has to ask the buffer, and the buffer may be gone
    /// (`go_to_mark` says so too, in its own words).
    fn mark_place(&self, spot: &Spot) -> String {
        match spot {
            Spot::InFile(path, line) => {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                say!("hint.mark.at", name, line + 1)
            }
            Spot::InBuffer(id, pos) => match self.buffer_with(*id) {
                Some(i) => {
                    let buffer = &self.buffers[i];
                    let rope = buffer.rope();
                    let line = rope.char_to_line((*pos).min(rope.len_chars()));
                    say!("hint.mark.at", buffer.display_name(), line + 1)
                }
                None => say!("hint.mark.gone"),
            },
        }
    }

    /// **One line of what a register holds**, for the panel that lists them.
    ///
    /// A yank is whole paragraphs as often as it is a word, and the panel is a
    /// thing you glance at beside your writing, so what goes in the right
    /// column is a *sample*: every run of whitespace becomes one space, and
    /// what is left is cut at twelve 漢字 with a `…` to say there is more.
    ///
    /// Warning: Cut by **grapheme**, not by `char`: `Warning: ` is two `char` and one
    /// two-cell glyph, and a cut between them leaves a stray VS16 in the box.
    fn register_preview(text: &str) -> String {
        const ROOM: usize = 24;
        let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if yumete_cjk::str_width(&one_line) <= ROOM {
            return one_line;
        }
        let mut out = String::new();
        let mut wide = 0usize;
        for g in yumete_cjk::graphemes(&one_line) {
            let w = yumete_cjk::grapheme_width(g);
            // The `…` is in the budget from the first cell, because the whole
            // point of it is that it must fit.
            if wide + w > ROOM - 1 {
                break;
            }
            out.push_str(g);
            wide += w;
        }
        out.push('…');
        out
    }

    pub fn pending_menu(&self) -> Option<(String, KeyRows)> {
        match self.pending_keys()? {
            Hint::Keys(title, keys) => Some((title, keys)),
            _ => None,
        }
    }
}
