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
        out.push_str(&format!("## {}\n\n", say!("help.common.motion-title")));
        for (keys, what) in [
            ("h j k l", say!("help.common.hjkl")),
            ("w b e", say!("help.common.word-motions")),
            ("3w 5j", say!("help.common.count-first")),
            ("g30g / 30G", say!("help.common.go-to-line")),
            ("gg ge", say!("help.common.start-end-of-file")),
            ("gh gl gs", say!("help.common.line-start-end")),
            ("J K", say!("help.common.half-page")),
            ("gd gw", say!("help.common.follow-what-it-points-at")),
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
            ("x", say!("help.common.select-line")),
            ("v ;", say!("help.common.extend-collapse")),
            (") (", say!("help.common.sentence-motions")),
            ("}} {{", say!("help.common.paragraph-motions")),
            ("y p", say!("help.common.yank-put")),
            ("u U", say!("help.common.undo-redo")),
            (".", say!("help.common.repeat-edit")),
            ("q Q", say!("help.common.macros")),
            ("> <", say!("help.common.indent")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out.push_str(&format!("\n## {}\n\n", say!("help.common.space-title")));
        for (key, what) in Self::SPACE_KEYS {
            out.push_str(&format!(
                "- `空格 {key}` — {}
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
            (":yume scheme", say!("help.chinese.switch-scheme")),
            (":yume chaifen on", say!("help.chinese.chaifen-under-candidates")),
            ("w b e", say!("help.chinese.word-boundaries")),
            (":word show on", say!("help.chinese.word-tint")),
            (":word list reload", say!("help.chinese.reload-project-words")),
            (":word habit", say!("help.chinese.habit-words")),
            (":ruby", say!("help.chinese.annotate-reading")),
            (":ruby format html", say!("help.chinese.unify-reading-spelling")),
            (":render full", say!("render.wysiwyg")),
            (":indent 2", say!("help.chinese.first-line-indent")),
            (":view hanging on", say!("help.chinese.hung-punctuation")),
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
            (":view wrap 24", say!("help.vertical.column-length")),
            (":view bands 2", say!("help.vertical.bands")),
            (":view hanging on", say!("help.vertical.hung-punctuation")),
            (":view dense off", say!("help.vertical.loose")),
            (":view sentence", say!("help.vertical.sentence")),
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
            ("gd gw", say!("help.table.which-row-this-names")),
            ("3gd g2-5d", say!("help.table.search-in-columns")),
            ("t/ t?", say!("help.table.who-uses-this")),
            ("t o t b t f t t", say!("help.table.four-surfaces")),
            ("t i t w", say!("help.table.detail-and-folds")),
            ("t r t d", say!("help.table.add-or-drop-row")),
            ("t s t S", say!("help.table.sort-by-column")),
            ("t y t p", say!("help.table.yank-or-put-column")),
            (":table rules off", say!("help.table.no-column-rules")),
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
                entry.args.hint(),
                crate::messages::say(entry.help, &[])
            ));
        }
        out
    }
}
