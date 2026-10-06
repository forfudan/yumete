//! The panel that explains what the cursor is standing on (#296).
//!
//! A table row read out field by field, or a footnote read where it is
//! referenced — two subjects that share one panel, one key and one width.

use super::*;

impl Editor {
    /// Whether the detail panel is showing.
    pub fn detail_visible(&self) -> bool {
        self.detail().is_some() && self.show_detail.unwrap_or_else(|| self.detail_opens_here())
    }

    /// Whether the panel opens **without being asked**, where the cursor is.
    ///
    /// Warning: **A row on a prose page never opens it** (#495) — `to`, `tb` and
    /// `tf` alike. `tf` used to, on the reasoning that a level which folds
    /// cells away owes the reader a way to read one whole. True in `tt`, where
    /// the grid has the window; wrong on a page of prose, because the panel
    /// takes a fifth of the width and **the paragraphs above and below rewrap
    /// when it appears**: 「markdown 中如果向下移动遇到表格总是会发生 wrap
    /// 跳动」. A panel that costs the whole page a reflow is not worth opening
    /// unasked, and `t i` opens it in one keystroke for the reader who wants
    /// it. A note in prose is not a row and is not affected: there is nothing
    /// on the page that says what a footnote holds.
    fn detail_opens_here(&self) -> bool {
        !(self.detail_shows_a_row()
            && !self.table.as_ref().is_some_and(|view| view.pane))
    }

    /// **`空格 t i`/`空格 t I`（與 `空格 i`/`空格 I`）：把這一行攤開**。
    ///
    /// 小寫浮、大寫一定進邊欄——五種信息同一條規矩（#426）。
    pub(super) fn show_the_record_here(&mut self, afloat: bool) {
        if self.detail().is_none() {
            // Warning: **不是字典那一句**（2026-09-30 報的）。從前這裏借用
            // 「光標下沒有字可查」——那是 `空格 d` 查不到字時說的，跟這一鍵沒
            // 有關係，而讀者只會以為自己按錯了鍵。
            self.status = say!("ui.cursor-is-not-in-a-table");
            if !afloat {
                self.make_room_for_the_info();
            }
            return;
        }
        // **再按一次同一個鍵就收起來**，同別的四種。Warning: 判準是
        // `detail_visible()` 而不是 `info_asked`——記錄在 `t t` 那一檔下是**即
        // 時**的（沒人叫過它），而 `t i` 照樣該收得掉它。
        let here = self.info_in_the_sidebar().is_none();
        if self.detail_visible() && here == afloat {
            self.show_detail = Some(false);
            self.info_asked = None;
            self.status = say!("ui.detail-panel-off");
            return;
        }
        self.show_detail = Some(true);
        self.put_this_info_here(crate::sidebar::Info::Record, afloat);
        self.status = say!("ui.detail-panel-on");
    }

    /// Show or hide the 記錄 panel (`t i`).
    ///
    /// Warning: **同時記一句「這一種是手動叫出來的」**（#426）：五種信息共用一
    /// 格，不記的話這一格下一幀就被即時的那一種頂掉了。
    pub fn toggle_detail(&mut self) {
        let want = !self.detail_visible();
        self.show_detail = Some(want);
        match want {
            // Warning: **走不收起的那一支**（2026-09-30 審出來的）。`ask_for_info`
            // 的判準是「此刻擺的就是它，而且畫在這一鍵要的地方」——`q` 關掉之後
            // `info_asked` 還記着記錄，於是這一下被當成「又按了一次」當場關回
            // 去，要按兩下纔出得來。同 `look_up_here` 那一個。
            true => {
                self.put_this_info_here(crate::sidebar::Info::Record, true);
                self.status = say!("ui.detail-panel-on");
            }
            false => {
                self.info_asked = None;
                self.status = say!("ui.detail-panel-off");
            }
        }
    }

    /// What the detail panel should show, if anything.
    ///
    /// One panel, one question — "what is here?" — asked of whatever the
    /// cursor is in. A table row answers with its fields; other things will
    /// answer with theirs. The editor works out *what* to say; the front end
    /// decides where to put it.
    pub fn detail(&self) -> Option<Detail> {
        // **Whatever the cursor is standing in answers** (#283). It used to
        // be whatever the *file* was: a `Bounds::Md` view sent every question
        // to the note panel, so `t i` inside a Markdown table — a table key,
        // pressed in a table — answered 「這裏沒有註」 and the row panel could
        // only ever be reached by opening a `.csv`.
        //
        // The old reasoning was that a Markdown table is a page of a document
        // and the document's question is the one worth asking. That is right
        // in the *paragraph*, which is why the note panel still answers there
        // — and wrong in the row, the more so since a cell wide enough to be
        // folded away is one this panel is now the way to read whole.
        //
        // Warning: **站在註上比在行裏精確，所以註先。**（2026-09-22 報的：`[^37]` 在
        // 一格表格裏，浮窗什麽都不出，而狀態欄照樣寫着「腳注」。）兩條規矩夾出
        // 一個洞：這裏無條件先給行，而行在散文頁上又**不主動打開**（#495），於是
        // 兩樣都没有。`note_detail` 只在光標**正壓着** `[^n]` 或 `%%註%%` 的時候
        // 纔答得出東西——那是讀者指着的那一個，而「這一格屬於某一行」只是它待的
        // 地方。#283 要的「行裏按 `t i` 給行」照舊：讓開的只有光標壓着註的那幾格。
        self.note_detail().or_else(|| match self.in_a_table_row() {
            true => self.row_detail(),
            false => None,
        })
    }

    /// Whether the panel is about to show a **row** rather than a note.
    ///
    /// The two want different shapes, and the shape used to be picked from
    /// whether the table had taken the window (#283): a row in a Markdown
    /// document got the note's four-line strip along the bottom, so a
    /// five-column row showed two of its fields and a 拆分表 row showed two
    /// of twenty-eight. What decides is what the panel *holds* — a row is a
    /// tall thing wherever it is written.
    pub fn detail_shows_a_row(&self) -> bool {
        // 與 [`Self::detail`] 同一個次序，否則這兩句會各說各話：註贏了卻仍被
        // 當成「這是一行」，於是那一條「散文頁上的行不主動打開」把註也擋住。
        self.note_detail().is_none() && self.in_a_table_row() && self.row_detail().is_some()
    }

    /// Whether the cursor is standing in a row a table panel can read.
    ///
    /// Not the header and not the `|---|` — neither is a row, and both would
    /// otherwise be shown as one with every field empty.
    fn in_a_table_row(&self) -> bool {
        let Some(view) = self.table.as_ref() else {
            return false;
        };
        let line = self.cursor_line();
        match view.bounds {
            Bounds::WholeFile => true,
            _ => match self.prose_region() {
                Some(region) => {
                    region.holds(line) && line != region.first && Some(line) != region.rule
                }
                None => false,
            },
        }
    }

    /// The schema of the table the cursor is **in**, not the file's (#283).
    ///
    /// A `.csv` has one schema and it is the view's. A Markdown document has
    /// as many tables as somebody typed, each with its own header, and the
    /// view carries the *first* one's — that is what made `t y` in the second
    /// table of a file report the first table's column name. Every question
    /// about columns asks this instead, and it is worked out from the header
    /// row that is actually above the cursor.
    pub(super) fn schema_here(&self) -> Option<std::borrow::Cow<'_, crate::table::Schema>> {
        use std::borrow::Cow;
        let view = self.table.as_ref()?;
        match view.bounds {
            // **Only a `|` table has a header to read back.** A guessed block
            // is walked out afresh every time the cursor enters one
            // (`Reach::Cursor`), so the view's numbered schema is already this
            // block's — while reading its first line as a Markdown header
            // split a tab-delimited row on pipes and called the whole thing one
            // column, which is what emptied the panel over a 碼表.
            Bounds::Md => {
                let region = self.prose_region()?;
                let header = self.line_text(region.first)?;
                Some(Cow::Owned(crate::mdtable::schema(&header)))
            }
            _ => Some(Cow::Borrowed(&view.schema)),
        }
    }

    /// The note the cursor is standing on — Feature #119.
    ///
    /// The panel that answers "what is this?" already exists for a table row;
    /// a footnote reference is the same question about a different thing. In
    /// 所見即所得 a `[^3]` is one small mark and the note itself is a hundred
    /// lines away, so reading it means losing your place — which for a
    /// footnote, whose whole purpose is to be read *beside* the sentence, is
    /// the wrong way round.
    ///
    /// A comment is the other case: `%%…%%` is dimmed but still on the page,
    /// and what the panel adds is room to read a long one without it pushing
    /// the paragraph about.
    fn note_detail(&self) -> Option<Detail> {
        if self.current_buffer().syntax() != crate::syntax::Syntax::Markdown {
            return None;
        }
        let rope = self.current_buffer().rope();
        let line = self.cursor_line();
        let within = self.caret() - rope.line_to_char(line);
        // Which block the line is in decides whether its `[^1]` is a footnote
        // at all — inside a fence it is four characters of code.
        let block = self.block_of(line);
        // The construct under the cursor, not the run: standing on the `%%` of
        // a comment is standing on the comment, and a reader who has just
        // moved onto its opening mark expects the panel then, not one step
        // later.
        let runs = self.markup_line_in(line, block);
        // Warning: **最裏面那一條，不是第一條**（2026-09-28）。`spans` 交出來的是嵌套的：
        // `# 見[^1]` 交 `HeadingMark`/`Heading 1..6`/`Footnote 2..6`，而 `Heading`
        // 排在前面。取第一條拿到的是標題那個構造，再去它裏面找腳註當然找不到——
        // **標題裏的腳註從此沒有詳情面板**。不變式保證「起點最靠後的那一條」就是最內層。
        let construct = runs
            .iter()
            .filter(|s| within >= s.start && within < s.end)
            .max_by_key(|s| s.start)?
            .construct;
        let span = runs.iter().find(|s| {
            s.construct == construct
                && matches!(
                    s.kind,
                    crate::markdown::Kind::Footnote | crate::markdown::Kind::Comment
                )
        })?;
        let text: String = rope
            .line(line)
            .chars()
            .skip(span.start)
            .take(span.end - span.start)
            .collect();
        match span.kind {
            crate::markdown::Kind::Comment => Some(Detail {
                title: say!("detail.comment"),
                here: String::new(),
                rows: vec![(String::new(), Some(text.trim_matches('%').trim().to_string()))],
                links: Vec::new(),
            }),
            _ => {
                let tag = text.trim_end_matches(':');
                let (at, body) = self.footnote_body(tag)?;
                Some(Detail {
                    // A definition names itself; standing on one, the panel is
                    // showing you where it is *used* is not yet a thing it can
                    // do, so it simply reads the note back.
                    title: tag.to_string(),
                    here: String::new(),
                    rows: vec![(String::new(), Some(body))],
                    links: vec![('↩', Some(at))],
                })
            }
        }
    }

    /// Follow a footnote to where it is written, or come back from it.
    ///
    /// One key, both directions: from a reference it goes to the note, and
    /// from the note it goes back to the sentence you left. A note read at the
    /// foot of a hundred-page file is no use if finding your place again is a
    /// search. A link, a `[[章節]]` and a `#錨點` are followed the same way —
    /// they are definitions too, and the ones a manuscript has most of.
    ///
    /// Warning: **Standing on none of them it does nothing** (#454). It used to fall
    /// through to 「這個詞還在哪裏」, so `gd` in the middle of a paragraph
    /// scattered hits across the book and moved the caret. That question is
    /// still one key away and always was: `g/`.
    fn follow_note(&mut self) {
        // A reference with no note is the ordinary way a note gets written:
        // you type `[^1]` in the sentence and then need somewhere to put it.
        if let Some(tag) = self.note_tag_at_cursor() {
            if self.footnote_body(&tag).is_none() {
                self.write_note(&tag);
                return;
            }
        }
        let Some(detail) = self.note_detail() else {
            // **A link is a definition too**, and the one a manuscript has most
            // of: `[手冊](docs/manual.md)`, `[[靈明]]`, `[雪](#雪)`. `gx` has
            // always followed them; `gd` asks the same question in the same
            // words, so it follows them as well.
            if self.link_under_cursor().is_some() {
                // Warning: **`gD` on a link is `gd`** — say so rather than pretend.
                // Everywhere else the capital means 「shown over there, and you
                // do not move」, and `follow_link` has three destinations (a
                // web page, another chapter, a heading in this file) of which
                // only the last could be previewed at all. Doing it for one of
                // three would be a rule nobody could hold; `Space w` puts the
                // other pane up and `C-o` comes back.
                return self.follow_link();
            }
            // **A wiki name is a definition too** (#287): `gd` opens the file
            // its entry is written in, on the heading.
            if self.follow_wiki() {
                return;
            }
            // **代碼檔上，這個問題歸語言服務器**（#53 ②，2026-09-21）。前面那
            // 三種——腳注、鏈接、百科名——是稿子裏的「定義」，而一份 `.rs` 裏一
            // 個都不會有；`gd` 問的還是同一句話（「這個東西寫在哪」），只是這一
            // 次答得出來的是別人。
            if self.ask_where_this_is_written() {
                return;
            }
            // Warning: **And on ordinary writing it does nothing** (#454). It used to
            // fall through here too — a whole-document search for whatever the
            // cursor happened to be on — so `gd` in the middle of a paragraph
            // scattered hits across the book and moved the caret.
            // 「我希望它对于普通文本不适用（按下去没有效果）」. That search is
            // still one key away and always was: `g/`.
            self.status = say!("note.nothing-to-go-to");
            return;
        };
        let Some(&(_, Some(at))) = detail.links.first() else {
            self.status = say!("note.points-nowhere");
            return;
        };
        if at == self.cursor_line() {
            self.status = say!("note.already-on-this-line");
            return;
        }
        let preview = self.definition_preview;
        self.land_on_row(at, preview);
    }

    /// `gd`: **what is this?** — the note this reference points at.
    ///
    /// The other half of the pair `Enter` is one half of. Shown in the other
    /// work area like everything else, and on a footnote reference that has no
    /// note yet it **writes the note** and shows that: following a link to a
    /// page that does not exist is how one gets written, which is what every
    /// wiki-shaped editor does and what a writer typing `[^1]` means.
    ///
    /// Warning: **`g` is the document's group and `t` is the table's** (2026-09-12).
    /// This key used to mean something else inside a grid — 「which row has
    /// this in the key column」 — and a `[^1]` written into a table row then
    /// searched the grid instead of going to its note, which is the one thing
    /// `gd` is named for. A key whose meaning turns over depending on what the
    /// cursor happens to be standing in cannot be relied on; the table's own
    /// questions are asked with `t/`, `t?` and `:table-jump`.
    ///
    /// Warning: **And that rule is the whole rule** (#454). A 拆分 cell holding 目
    /// still *names* another row, and this key still refuses it: 「g 不管表格，
    /// 表格的我们以后再说」. `t?` is the one that asks it.
    pub(super) fn show_definition(&mut self, preview: bool) {
        self.definition_preview = preview;
        self.follow_note();
    }

    /// **`*`/`A-*`：記下要找什麼，不動光標**（2026-10-06，helix 的
    /// `search_selection_detect_word_boundaries`/`search_selection`）。
    ///
    /// `bounded` 為真時兩端各補一個 `\b`——**補不補是逐端看的**，同 helix：
    /// 選區的開頭落在一個詞的開頭上才補前面那個，末尾落在詞尾上才補後面那個。
    /// 站在 `one` 的 `o` 上按 `*` 得到的是 `\bo`，不是 `\bo\b`。
    ///
    /// Warning: **說一句「幾處」而不是 helix 那句「寄存器設成了…」。** 一個不動光標的
    /// 鍵最像按壞了，而「共 N 處」既說明它收下了，也說明接着按 `n` 會走到哪裏；
    /// 寄存器那句話對這個倉的讀者什麼都不是。
    pub(super) fn remember_the_search(&mut self, bounded: bool) {
        let rope = self.current_buffer().rope();
        let (from, to) = self.selection();
        let to = to.min(rope.len_chars());
        if to <= from {
            self.status = say!("find.nothing-here-to-look-for");
            return;
        }
        let word = |at: usize| {
            rope.get_char(at).is_some_and(|c| c.is_alphanumeric() || c == '_')
        };
        let head = bounded && word(from) && (from == 0 || !word(from - 1));
        let tail = bounded && to > 0 && word(to - 1) && !word(to);
        let text: String = rope.slice(from..to).chars().collect();
        let pattern = format!(
            "{}{}{}",
            match head {
                true => "\\b",
                false => "",
            },
            regex::escape(&text),
            match tail {
                true => "\\b",
                false => "",
            }
        );
        let found = self.every_match(&pattern).len();
        self.last_search = pattern;
        // 這一趟不是走到某一處，所以上一張命中名單作廢——`n` 從現在的地方數起。
        self.hits = None;
        self.status = say!("search.hits", found);
    }

    /// `g/` and `g?` (and `*`): **who else says this?**
    ///
    /// One key, one meaning, in a table and out of it: 「在另一個工作區給我看
    /// 這個詞還出現在哪裏」. The selection is the question when there is one —
    /// so a phrase is asked about by selecting it — and the word under the
    /// cursor when there is not, which is what `w` would have taken.
    /// `back` ＝ 往回找（vim 的 `#`）：取光標**之前**最後一處，繞回去就是最末一處。
    pub(super) fn search_the_page(&mut self, back: bool) {
        let rope = self.current_buffer().rope();
        let (from, to) = self.selection();
        // Warning: **一個字寬的「選區」是光標，不是選中**（2026-10-06）。這個編輯器
        // 裏每一次移動都留下選區，而光標蓋着自己那一格——所以 `to > from` 恆真，
        // 下面那條「取光標下那個詞」的路**一次都沒走到過**。搜索面板那一扇早就
        // 按這條規矩辦（`open_search` 的 `to > from + 1`），這裏跟上。
        let needle = match to > from + 1 {
            true => rope.slice(from..to.min(rope.len_chars())).to_string(),
            false => {
                // Warning: **問 `line_words`，不要問 `segment_line`**（2026-10-06 查出來
                // 的）。`segment_line` 是**畫分詞底線**用的，它有意把拉丁詞整段濾
                // 掉（`words.rs`：「標點 runs and Latin words are never words to
                // paint」）——於是這裏在英文上永遠落到下面那條退路，取的是**一個
                // 字母**。實測 `one two three alpha` 站在 `o` 上按 `*`，搜的是 `o`、
                // 報「第 2 處，共 5 處」；`l*` 搜 `n`。中文從來沒事，所以躲了很久。
                //
                // `line_words` 正是 `w`/`b` 問的那一支（`word_object_span` 也問
                // 它），所以這一行的註釋「which is what `w` would have taken」
                // 現在是真的。
                let line = rope.char_to_line(self.sel.head());
                let words = crate::motion::line_words(rope, line, self.word_grain(), self.segmenter.as_ref());
                let here = self.sel.head();
                match words.iter().find(|&&(a, b)| (a..b).contains(&here)) {
                    Some(&(a, b)) => rope.slice(a..b.min(rope.len_chars())).to_string(),
                    None => rope.get_char(here).map(|c| c.to_string()).unwrap_or_default(),
                }
            }
        };
        let needle = needle.trim().to_string();
        if needle.is_empty() {
            self.status = say!("find.nothing-here-to-look-for");
            return;
        }
        let spans = self.every_match(&regex::escape(&needle));
        if spans.len() <= 1 {
            self.status = say!("find.only-here", needle);
            self.hits = None;
            return;
        }
        self.last_search = regex::escape(&needle);
        // The first one *after* where you are standing: the useful answer to
        // 「還在哪裏」 is the next place, not the first page of the book.
        let here = self.sel.head();
        // Warning: **往回要從這一處的開頭數起，不是從光標數起**（2026-10-06，和
        // `repeat_search` 裏那一條同一個病）。落地之後光標停在匹配的**最後一個
        // 字**上，開頭在 `anchor`——拿光標去問，當前這一處的開頭就在光標之前，於是
        // `#` 把自己又找了一遍，一動不動。匹配只有一個字的時候它是好的，所以這個
        // 洞要等到 `g/` 真的取整個詞之後才露出來。
        let from_here = self.sel.anchor().min(here);
        let at = match back {
            // 往回：光標**之前**最後一處；前面沒有就繞到最末一處。
            true => spans
                .iter()
                .rposition(|&(from, _)| from < from_here)
                .unwrap_or(spans.len() - 1),
            false => spans.iter().position(|&(from, _)| from > here).unwrap_or(0),
        };
        self.remember_hits(spans, at);
        self.show_table_hit();
    }

    /// Every match of `pattern` in the buffer, as character ranges.
    fn every_match(&self, pattern: &str) -> Vec<(usize, usize)> {
        let Ok(re) = self.compile(pattern) else {
            return Vec::new();
        };
        let rope = self.current_buffer().rope();
        let mut hits = Vec::new();
        let mut at = 0usize;
        for line in 0..rope.len_lines() {
            let text = rope.line(line).to_string();
            let start = at;
            at += rope.line(line).len_chars();
            for m in re.find_iter(&text) {
                let before = text[..m.start()].chars().count();
                let length = text[m.start()..m.end()].chars().count();
                hits.push((start + before, start + before + length));
            }
        }
        hits
    }

    /// The footnote reference the cursor is standing in, if it is in one.
    pub(super) fn note_tag_at_cursor(&self) -> Option<String> {
        if self.current_buffer().syntax() != crate::syntax::Syntax::Markdown {
            return None;
        }
        let rope = self.current_buffer().rope();
        let line = self.cursor_line();
        let within = self.caret() - rope.line_to_char(line);
        let block = self.block_of(line);
        let runs = self.markup_line_in(line, block);
        let span = runs.iter().find(|s| {
            s.kind == crate::markdown::Kind::Footnote && within >= s.start && within < s.end
        })?;
        let text: String = rope
            .line(line)
            .chars()
            .skip(span.start)
            .take(span.end - span.start)
            .collect();
        let tag = text.trim_end_matches(':').to_string();
        // The *definition* is not a reference: standing on `[^1]:` there is
        // nothing to go to — you are already there.
        match text.ends_with(':') {
            true => None,
            false => Some(tag),
        }
    }

    /// Write the note for `tag` at the foot of the file, and show it.
    fn write_note(&mut self, tag: &str) {
        let rope = self.current_buffer().rope();
        let end = rope.len_chars();
        let text = rope.to_string();
        // One blank line between the manuscript and its notes, and none added
        // when the file already ends with one.
        let lead = match text.ends_with("\n\n") {
            true => String::new(),
            false => match text.ends_with('\n') {
                true => "\n".to_string(),
                false => "\n\n".to_string(),
            },
        };
        let note = format!("{lead}{tag}: ");
        // The same gate every other writer passes: a note appended to a grid
        // gives it two one-column rows. `:markdown-footnote` reaches this from
        // a key now, so「表格裏不寫註」 has to be said here rather than assumed.
        if let Some(why) = self.replacement_reshapes_the_grid((end, end), &note) {
            self.status = why;
            return;
        }
        if self.table_here() {
            self.status = say!("note.not-in-a-grid");
            return;
        }
        self.snapshot();
        self.write_the_note(end, &note, tag);
    }

    /// The note itself, with the undo point already taken.
    ///
    /// **One edit, one `u`.** `:markdown-footnote` writes the tag *and* the
    /// note, and two snapshots left a `[^1]` pointing at nothing after a single
    /// undo — so the caller takes the one snapshot that covers both.
    pub(super) fn write_the_note(&mut self, end: usize, note: &str, tag: &str) {
        if self.refuse_readonly() {
            return;
        }
        let at = end;
        let done = self.current_buffer_mut().insert(at, note);
        if !self.applied(done) {
            return;
        }
        // The other area is opened **at the end of the stub**, not at the head
        // of its line: 空格 w lands where the note is going to be typed, which
        // is the only place anybody is going next.
        let caret = at + note.chars().count();
        let line = self.current_buffer().rope().char_to_line(caret);
        match self.definition_preview {
            true => {
                let caption = say!(
                    "show.row-in-file",
                    self.current_buffer().display_name(),
                    line + 1
                );
                self.show_in_split(caret, None, caption);
                self.status = say!("note.written-other-pane", tag);
            }
            // `gd` goes, and a stub is written to be typed into, so it lands
            // at the end of it with Insert one keystroke away.
            false => {
                self.remember_jump();
                self.set_cursor(caret);
                self.status = say!("note.written-same-pane", tag);
            }
        }
    }

    /// Where a footnote is defined and what it says.
    pub(super) fn footnote_body(&self, tag: &str) -> Option<(usize, String)> {
        let rope = self.current_buffer().rope();
        let opener = format!("{tag}:");
        for line in 0..rope.len_lines() {
            let text = rope.line(line).to_string();
            let trimmed = text.trim_start();
            if let Some(rest) = trimmed.strip_prefix(&opener) {
                return Some((line, rest.trim().to_string()));
            }
        }
        None
    }

    /// What a table row is, field by field.
    fn row_detail(&self) -> Option<Detail> {
        let view = self.table.as_ref()?;
        let schema = self.schema_here()?;
        let (line, cell) = self.cell_position()?;
        // The header names the columns; it is not a row and has no fields.
        // In prose the header is wherever the table starts, and the `|---|`
        // under it is not a row either — [`Self::in_a_table_row`] knows both,
        // and it is the same question.
        if view.bounds == Bounds::WholeFile {
            if schema.header && line == 0 {
                return None;
            }
        } else if !self.in_a_table_row() {
            return None;
        }
        // Nor is the empty line a file ending in a newline leaves behind — the
        // same thing `row_is_ragged` already knows not to complain about.
        let rope = self.current_buffer().rope();
        if line + 1 == rope.len_lines() && rope.line(line).len_chars() == 0 {
            return None;
        }
        let text = self.current_buffer().rope().line(line).to_string();
        // **The view splits the row, not the delimiter** (#283). A `|` row
        // begins and ends with the separator, so splitting it on the character
        // gives an empty cell at each end — and the panel then answered every
        // question one column to the left, while `cell_position`, which asks
        // the view, was pointing one column to the right.
        let spans = self.row_cells(line);
        let value = |name: &str| -> String {
            schema
                .index_of(name)
                .and_then(|i| spans.get(i))
                .map(|&s| crate::table::cell_text(&text, s))
                .unwrap_or_default()
        };
        // Warning: **照這張表數，不照這個檔數**（2026-09-23 審出來的）。一個 `.csv`
        // 攤成整扇窗的時候，行號欄寫 1、狀態欄寫「行 1」，而這裏從前寫 2——
        // 同一行三個數字兩種口徑。`table_row_base` 就是那兩處用的那一個。
        // Warning: **一律寫行號，不寫主鍵的值**（2026-09-30 定，原話「我建议
        // 都用行号数字，不要用主键的值（主键的值可能很长）」）。從前有主鍵就拿
        // 主鍵的值當標題，而一格主鍵裏可以是一整句話，浮窗的標題欄放不下。
        let title = format!("{}", line.saturating_sub(self.table_row_base()) + 1);
        // The field the cursor is in is shown even when it is empty: that it
        // *is* empty is the answer to "what is in this cell".
        // **Numbered the same way the rows are**, or the panel can never find
        // the field the cursor is in: it compared 「unicode」 against 「 9
        // unicode」, never matched, and so never scrolled to it and never lit
        // it — both of the things it promises.
        let here_name = schema
            .columns
            .get(cell)
            .map(|c| format!("{:>2} {}", cell + 1, c.heading()))
            .unwrap_or_default();
        let mut rows: Vec<(String, Option<String>)> = schema
            .columns
            .iter()
            .enumerate()
            .filter(|(i, column)| !column.hidden || spans.get(*i).is_some())
            .map(|(i, column)| {
                // `None` when the row has no such field — a short row, which
                // the grid already marks as ragged. An empty field is
                // `Some("")`, and they are different answers to 「這一格有什麼」.
                let text = spans.get(i).map(|&s| crate::table::cell_text(&text, s));
                // **Numbered**, because the keys count columns: `t3/` looks in
                // the third, `t20,20g` goes to a cell by number, and the panel
                // is where a reader finds out which number a field is without
                // counting along the header.
                (format!("{:>2} {}", i + 1, column.heading()), text)
            })
            // **Every column, empty ones included** — an empty field *is* a
            // finding in a 拆分表, and a panel that leaves it out is a panel
            // that cannot answer 「這一格是不是空的」. They were hidden because
            // twenty-three blanks pushed the 部件 list off the bottom; the
            // panel scrolls to the field the cursor is in, so there is
            // somewhere for them to go — and `t20,20g` reaches any of them by
            // number, which is what the numbers are for.
            .collect();
        // Worked out, not stored — and marked as such, so nobody goes looking
        // for a column that is not in the file.
        for detail in &schema.details {
            let from = value(detail.compute.column());
            rows.push((
                format!("{}*", detail.name),
                Some(detail.compute.apply(&from, &schema.ranges)),
            ));
        }
        Some(Detail {
            title,
            here: here_name,
            rows,
            links: self.cell_links(),
        })
    }

    /// The rows this cell's contents name, when its column is a foreign key.
    ///
    /// A 拆分 cell is a *sequence* of components, each of which is a character
    /// with a row of its own — so one cell points at several rows, and which
    /// one is a question only a person can answer.
    fn cell_links(&self) -> Vec<(char, Option<usize>)> {
        let Some(view) = &self.table else {
            return Vec::new();
        };
        let Some(link) = &view.schema.link else {
            return Vec::new();
        };
        let Some((line, cell)) = self.cell_position() else {
            return Vec::new();
        };
        let Some(column) = view.schema.columns.get(cell) else {
            return Vec::new();
        };
        if !link.from.contains(&column.name) {
            return Vec::new();
        }
        let text = self.cell_text(line, cell);
        let mut seen: Vec<char> = Vec::new();
        for c in text.chars() {
            // ⿰⿱⿲… are not components, they are the *grammar* saying how the
            // components are arranged, and no table has a row for one. They
            // appear 9,046 times in the ids_y column alone, and every one used
            // to get the red 「—」 that means "no row for this" — so the
            // panel's one validation signal was false on nearly every
            // structured row, which is the same as not having one.
            if is_ids_operator(c) {
                continue;
            }
            if !seen.contains(&c) {
                seen.push(c);
            }
        }
        seen.into_iter().map(|c| (c, self.row_named(c))).collect()
    }
}
