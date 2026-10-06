//! `:help` and `:tutor` (#296).
//!
//! The lesson is copied into a **real file** of the reader's own, so `:w`
//! works and every destructive key in it is safe.

use super::*;

impl Editor {
    /// **`:tutor`** — the lesson, copied into a file of the reader's own.
    ///
    /// A **real file**, not a scratch buffer: `:w` works, `u` is part of lesson
    /// one, and every destructive key in it is safe because it is a copy. A
    /// second `:tutor` numbers a fresh one rather than overwriting the first,
    /// which may have a week's notes in it by then.
    pub(super) fn open_tutor(&mut self) {
        let Some(dir) = self.drafts_dir.as_ref().and_then(|d| d.parent()) else {
            // No data directory — the front end never said where it is. The
            // lesson still opens; it just has nowhere to live.
            let mut buffer = crate::Buffer::from_text(crate::tutor::LESSON);
            buffer.name_as("[tutor]");
            self.add_buffer(buffer);
            self.status = say!("tutor.open-unsaved");
            return;
        };
        let dir = dir.to_path_buf();
        let _ = std::fs::create_dir_all(&dir);
        // The first name that is free: a lesson from last week is somebody's
        // notes now.
        let path = (0..100)
            .map(|n| dir.join(crate::tutor::file_name(n)))
            .find(|p| !p.exists());
        let Some(path) = path else {
            self.status = say!("tutor.no-name-left");
            return;
        };
        if let Err(err) = crate::buffer::write_file_atomically(&path, crate::tutor::LESSON) {
            self.status = say!("tutor.cannot-write", err);
            return;
        }
        match self.open_file(&path) {
            Ok(()) => self.status = say!("tutor.this-copy-is-yours"),
            Err(err) => self.status = say!("tutor.cannot-open", err),
        }
    }

    /// **`:help`** — the keys and the commands, in a buffer you can read with
    /// the editor itself.
    ///
    /// Written from the same declarations the editor runs on — `COMMANDS`,
    /// `SPACE_KEYS`, the which-key lists — so it cannot drift from what the
    /// keys actually do. A manual can be out of date; this cannot be, because
    /// there is nothing here to keep up to date.
    ///
    /// It opens as an ordinary buffer, so `/`, `n`, `空格 f` and every motion
    /// work in it: the way to learn an editor is to use it on something, and
    /// this is something.
    pub(super) fn open_help(&mut self, topic: Option<&str>) {
        let (title, text) = match topic.map(str::trim).unwrap_or_default() {
            "" => (say!("help.common.title"), self.help_common()),
            t if "chinese".starts_with(t) || "中文".starts_with(t) => {
                (say!("help.chinese.title"), Self::help_chinese())
            }
            t if "vertical".starts_with(t) || "竪排".starts_with(t) => {
                (say!("help.vertical.title"), Self::help_vertical())
            }
            t if "table".starts_with(t) || "表格".starts_with(t) => {
                (say!("label.table"), self.help_table())
            }
            t if "commands".starts_with(t) || "命令".starts_with(t) => {
                (say!("help.commands.title"), Self::help_commands())
            }
            other => {
                self.status = say!(
                    "help.no-such-section",
                    other
                );
                return;
            }
        };
        let mut buffer = crate::Buffer::from_text(&text);
        buffer.name_as(&format!("[help · {title}]"));
        self.add_buffer(buffer);
        self.status = say!("help.title", title);
    }

    /// The keys a writer uses in the first hour, drawn from what is bound.
    pub(super) fn help_common(&self) -> String {
        let mut out = String::new();
        out.push_str("# yumete

");
        out.push_str(&format!("{}\n\n", say!("help.self-reported")));
        // **窗口要多大**（2026-10-03 定：「就说推荐窗口大小 80*24，这个先放在
        // help」）。擺不下側面板的時候 `空格 /` 會當場退回行內搜索並說一句，而那一句
        // 說的是此刻；這一行說的是往後。
        out.push_str(&format!("{}\n\n", say!("help.common.window-size")));
        out.push_str(&format!("## {}\n\n", say!("help.common.motion-title")));
        for (keys, what) in [
            ("h j k l", say!("help.common.hjkl")),
            ("w b e", say!("help.common.word-motions")),
            ("3w 5j", say!("help.common.count-first")),
            ("g30g / 30G", say!("help.common.go-to-line")),
            // Warning: **這一頁開頭寫着「每一節都是這個編輯器自己報上來的」，而下面
            // 這幾行是手抄的。** 2026-09-24 審出來四條錯的：`L H` 說成整頁（其實
            // 是上下一句，整頁是 `C-f`／`C-b`）、`gw`（沒綁，編輯器自己會說「是
            // gD 了」）、`) (`（沒綁，它自己會說「一句一句走是 H／L」）、`}} {{`
            // （是 `}` `{`——大括號當成 `format!` 的轉義寫了兩遍，而這裏不是格式
            // 串，於是屏幕上真就印出兩個）。
            //
            // Warning: **`documented_keys` 那張網看不見這裏**：它只讀兩份手冊
            // 與教程，不讀這一頁。手抄的鍵表就是這麽爛掉的。
            ("gg ge", say!("help.common.start-end-of-file")),
            ("gh gl gs", say!("help.common.line-start-end")),
            ("C-d C-u", say!("help.common.half-page")),
            ("gd gD", say!("help.common.follow-what-it-points-at")),
            ("g/ g?", say!("help.common.word-elsewhere")),
            ("/ n N", say!("help.common.search")),
            ("C-o C-i", say!("help.common.jump-list")),
            ("M a  'a", say!("help.common.marks")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out.push_str(&format!("\n## {}\n\n", say!("help.common.editing-title")));
        for (keys, what) in [
            ("i a", say!("help.common.insert-before-after")),
            ("o O", say!("help.common.open-line")),
            ("d c", say!("help.common.delete-change")),
            ("D C", say!("help.common.cut-change")),
            ("x", say!("help.common.select-line")),
            ("v ;", say!("help.common.extend-collapse")),
            ("L H", say!("help.common.sentence-motions")),
            ("} {", say!("help.common.paragraph-motions")),
            ("y p", say!("help.common.yank-put")),
            ("u U", say!("help.common.undo-redo")),
            (".", say!("help.common.repeat-edit")),
            // Warning: **`Q` 先，`q` 後**（2026-10-02 更正）。這張表是**按位置**讀的
            // （`u U` 撤銷／重做、`> <` 縮進／退縮進），而那一句是「錄一段按鍵／
            // 放一遍」——寫成 `q Q` 就是教人用 `q` 開錄。真的綁法是 `Q` 錄、`q`
            // 放（`keys.rs`），而那兩個鍵的註釋自己寫着「**調換了的一對是最糟的
            // 那一種分歧**：手不讀字，就按下去，而這兩個按錯了不是沒反應——它開
            // 始錄，蓋掉你本來要放的那一段」（#404）。
            ("Q q", say!("help.common.macros")),
            ("> <", say!("help.common.indent")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out.push_str(&format!("\n## {}\n\n", say!("help.common.space-title")));
        for (key, what) in Self::SPACE_KEYS {
            out.push_str(&format!(
                "- `\u{2423}{key}` — {}
",
                crate::messages::say(what, &[])
            ));
        }
        out.push_str(&format!("\n## {}\n\n", say!("help.common.other-sections")));
        for (topic, what) in [
            ("chinese", say!("help.chinese.section-summary")),
            ("vertical", say!("help.vertical.title")),
            ("table", say!("help.table.section-summary")),
            ("commands", say!("help.common.every-command")),
        ] {
            out.push_str(&format!("- `:help {topic}` — {what}\n"));
        }
        out
    }

    pub(super) fn help_chinese() -> String {
        let mut out = format!("# {}\n\n", say!("help.chinese.title"));
        for (keys, what) in [
            (":yume on", say!("help.chinese.toggle-ime")),
            (":yume-scheme", say!("help.chinese.switch-scheme")),
            (":yume-chaifen on", say!("help.chinese.chaifen-under-candidates")),
            ("w b e", say!("help.chinese.word-boundaries")),
            (":word-show on", say!("help.chinese.word-tint")),
            (":word-list-reload", say!("help.chinese.reload-project-words")),
            (":word-habit", say!("help.chinese.habit-words")),
            (":ruby", say!("help.chinese.annotate-reading")),
            (":ruby-format html", say!("help.chinese.unify-reading-spelling")),
            (":render full", say!("render.wysiwyg")),
            (":indent 2", say!("help.chinese.first-line-indent")),
            (":view-hanging on", say!("help.chinese.hung-punctuation")),
            (":count", say!("help.chinese.count")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out
    }

    pub(super) fn help_vertical() -> String {
        let mut out = format!("# {}\n\n", say!("help.vertical.title"));
        for (keys, what) in [
            (":layout vertical", say!("help.vertical.turn-vertical")),
            ("h l", say!("help.vertical.previous-next-column")),
            ("j k", say!("help.vertical.down-up-column")),
            (":view-wrap 24", say!("help.vertical.column-length")),
            (":view-bands 2", say!("help.vertical.bands")),
            (":view-hanging on", say!("help.vertical.hung-punctuation")),
            (":view-margin always", say!("help.vertical.margin")),
            (":view-sentence", say!("help.vertical.sentence")),
            (":indent 2", say!("help.vertical.first-line-indent")),
            (":render full", say!("help.vertical.wysiwyg")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out
    }

    pub(super) fn help_table(&self) -> String {
        let mut out = format!("# {}\n\n", say!("label.table"));
        for (keys, what) in [
            (":table", say!("help.table.enter-table")),
            ("h j k l", say!("help.table.step-by-cell")),
            ("Tab S-Tab", say!("help.table.next-previous-cell")),
            ("i a c d", say!("help.table.type-in-cell")),
            // Warning: **這一組要寫 `␣t…`**（2026-10-02 更正）。表格那一組 2026-09-21
            // 搬進了空格選單（`t`／`T` 還給 vi 的 till），而這一頁沒跟着搬——
            // 上面寫的 `t r`、`t s` 按下去一個都不管用，`t` 當場成了 till。
            ("␣t/ ␣t?", say!("help.table.who-uses-this")),
            ("␣to ␣tb ␣tf ␣tt", say!("help.table.four-surfaces")),
            ("␣ti ␣tw", say!("help.table.detail-and-folds")),
            ("␣tr ␣td", say!("help.table.add-or-drop-row")),
            ("␣ts ␣tS", say!("help.table.sort-by-column")),
            ("␣ty ␣tp", say!("help.table.yank-or-put-column")),
            (":table-rules off", say!("help.table.no-column-rules")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out
    }

    /// Every `:` command, from the table the parser itself reads.
    fn help_commands() -> String {
        let mut out = format!("# {}\n\n", say!("help.commands.title"));
        for entry in crate::command::COMMANDS {
            let aliases = match entry.aliases.is_empty() {
                true => String::new(),
                false => format!("（{}）", entry.aliases.join(" ")),
            };
            out.push_str(&format!(
                "- `:{}`{} {} — {}
",
                entry.name,
                aliases,
                entry
                    .params
                    .iter()
                    .map(crate::command::Param::hint)
                    .collect::<Vec<_>>()
                    .join(" "),
                crate::messages::say(entry.help, &[])
            ));
        }
        out
    }
}
