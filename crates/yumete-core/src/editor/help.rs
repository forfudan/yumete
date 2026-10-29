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
        let topic = topic.unwrap_or("").trim();
        let (topic, requested_language) = match topic.rsplit_once('@') {
            Some((topic, lang)) => (topic, Some(lang)),
            None => (topic, None),
        };
        let language = match requested_language {
            Some(lang) => match crate::messages::Language::parse(lang) {
                Some(lang) => lang,
                None => {
                    self.status = say!("help.no-such-section", topic);
                    return;
                }
            },
            None => crate::messages::language(),
        };
        let lang = match language {
            crate::messages::Language::English => "en",
            crate::messages::Language::Simplified => "zh-Hans",
            crate::messages::Language::Traditional => "zh-Hant",
        };
        // Explicit language requests use the book so they never depend on the
        // process-wide locale used by the live overview generators.
        let topic = if requested_language.is_some() {
            match topic {
                "" => "manual",
                "chinese" => "philosophy",
                "vertical" => "vertical-layout",
                "table" => "tables",
                "commands" => "command-reference",
                other => other,
            }
        } else {
            topic
        };
        // Page links and exact contextual identifiers share the same resolver.
        if topic.contains('/') && topic.ends_with(".md") {
            self.open_help_page(&format!("{lang}/{topic}"), 0, None);
            return;
        }
        if !topic.is_empty()
            && ![
                "chinese", "vertical", "table", "commands", "中文", "竪排", "表格", "命令",
            ]
            .contains(&topic)
        {
            match crate::help::lookup(topic) {
                Ok(at) => {
                    let relative = at.page.strip_prefix("en/").unwrap_or(at.page);
                    self.open_help_page(&format!("{lang}/{relative}"), at.line, None);
                    return;
                }
                Err(candidates) if topic.starts_with('\'') && !candidates.is_empty() => {
                    let text = candidates
                        .iter()
                        .map(|name| format!("- `:help {name}`\n"))
                        .collect::<String>();
                    self.open_help_buffer("options", &text);
                    return;
                }
                _ => {}
            }
            let name = topic.strip_prefix(':').unwrap_or(topic);
            if let Some(entry) = crate::command::COMMANDS
                .iter()
                .find(|e| e.name == name || e.aliases.contains(&name))
            {
                let text = format!(
                    "# :{}\n\n`:{}` {}\n\n{}\n\n[Manual contents](help:{lang}/SUMMARY.md)\n",
                    entry.name,
                    entry.name,
                    entry
                        .params
                        .iter()
                        .map(crate::command::Param::hint)
                        .collect::<Vec<_>>()
                        .join(" "),
                    crate::messages::say(entry.help, &[])
                );
                self.open_help_buffer(entry.name, &text);
                return;
            }
        }
        let (title, text) = match topic {
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
                self.status = say!("help.no-such-section", other);
                return;
            }
        };
        let text = format!("{text}\n[Manual contents](help:{lang}/SUMMARY.md) · [Help lookup](help:{lang}/help.md)\n");
        self.open_help_buffer(&title, &text);
    }

    fn open_help_buffer(&mut self, title: &str, text: &str) {
        self.remember_jump();
        let mut buffer = crate::Buffer::from_text(text);
        buffer.name_as(&format!("[help · {title}]"));
        buffer.set_readonly(true);
        self.add_buffer(buffer);
        self.status = say!("help.title", title);
    }

    pub(super) fn open_help_page(&mut self, requested: &str, line: usize, anchor: Option<&str>) {
        let Some((lang, relative)) = requested.split_once('/') else {
            return;
        };
        let Some((actual, body, fallback)) = crate::help::page(relative, lang) else {
            self.status = say!("help.no-such-section", relative);
            return;
        };
        let mut target_line = anchor
            .and_then(|a| crate::help::anchor_line(body, a))
            .unwrap_or(line);
        if !fallback && lang != "en" && line != 0 && anchor.is_none() {
            if let Some((_, english, _)) = crate::help::page(relative, "en") {
                let original = english.lines().nth(line).unwrap_or("");
                // Identifiers in tags, table cells and assignments survive translation.
                let previous = english
                    .lines()
                    .take(line)
                    .filter(|s| !s.trim().is_empty())
                    .last()
                    .unwrap_or("");
                if previous.starts_with("<!-- help:") {
                    target_line = body
                        .lines()
                        .position(|s| s.trim() == previous.trim())
                        .and_then(|at| {
                            body.lines()
                                .enumerate()
                                .skip(at + 1)
                                .find(|(_, s)| {
                                    !s.trim().is_empty() && !s.trim().starts_with("<!--")
                                })
                                .map(|(at, _)| at)
                        })
                        .unwrap_or(0);
                } else {
                    let identifier = if original.starts_with('|') {
                        original.split('|').nth(1).unwrap_or("").trim()
                    } else if let Some((left, _)) = original.split_once('=') {
                        left.trim()
                    } else {
                        original.trim()
                    };
                    target_line = body
                        .lines()
                        .position(|s| s.contains(identifier))
                        .unwrap_or(0);
                }
            }
        }
        let fallback_notice = say!("help.fallback");
        let notice = crate::messages::table()
            .get("help.fallback")
            .map(|entry| match lang {
                "zh-Hans" => entry.zhs,
                "zh-Hant" => entry.zht,
                _ => entry.en,
            })
            .unwrap_or(&fallback_notice);
        let text = if fallback {
            format!("> {notice}\n\n{body}")
        } else {
            body.to_string()
        };
        target_line += if fallback { 2 } else { 0 };
        let name = format!("[help · {requested}]");
        self.remember_jump();
        if let Some(index) = self.buffers.iter().position(|b| b.display_name() == name) {
            self.show_buffer(index);
        } else {
            let mut buffer = crate::Buffer::from_text(&text);
            buffer.name_as(&name);
            buffer.set_readonly(true);
            self.add_buffer(buffer);
        }
        let rope = self.current_buffer().rope();
        let at = rope.line_to_char(target_line.min(rope.len_lines().saturating_sub(1)));
        self.set_cursor(at);
        self.status = if fallback {
            notice.to_string()
        } else {
            say!("help.title", actual)
        };
    }

    /// Handle relative links in embedded Markdown before filesystem navigation.
    pub(super) fn follow_help_link(&mut self, target: &str, anchor: Option<&str>) -> bool {
        if let Some(requested) = target.strip_prefix("help:") {
            self.open_help_page(requested, 0, anchor);
            return true;
        }
        let name = self.current_buffer().display_name();
        let Some(from) = name
            .strip_prefix("[help · ")
            .and_then(|s| s.strip_suffix(']'))
        else {
            return false;
        };
        if !from.ends_with(".md") {
            return false;
        }
        let requested = if target.is_empty() {
            Some(from.to_string())
        } else {
            crate::help::link(from, target)
        };
        if let Some(requested) = requested {
            self.open_help_page(&requested, 0, anchor);
            return true;
        }
        false
    }

    /// The keys a writer uses in the first hour, drawn from what is bound.
    pub(super) fn help_common(&self) -> String {
        let mut out = String::new();
        out.push_str(
            "# yumete

",
        );
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
            // 是上下一句，整頁是 `C-f`/`C-b`）、`gw`（没綁，編輯器自己會說「是
            // gD 了」）、`) (`（没綁，它自己會說「一句一句走是 H/L」）、`}} {{`
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
            out.push_str(&format!(
                "- `{keys}` — {what}
"
            ));
        }
        out.push_str(&format!("\n## {}\n\n", say!("help.common.editing-title")));
        for (keys, what) in [
            ("i a", say!("help.common.insert-before-after")),
            ("o O", say!("help.common.open-line")),
            ("d c", say!("help.common.delete-change")),
            ("x", say!("help.common.select-line")),
            ("v ;", say!("help.common.extend-collapse")),
            ("L H", say!("help.common.sentence-motions")),
            ("} {", say!("help.common.paragraph-motions")),
            ("y p", say!("help.common.yank-put")),
            ("u U", say!("help.common.undo-redo")),
            (".", say!("help.common.repeat-edit")),
            // Warning: **`Q` 先，`q` 後**（2026-10-02 更正）。這張表是**按位置**讀的
            // （`u U` 撤銷/重做、`> <` 縮進/退縮進），而那一句是「錄一段按鍵/
            // 放一遍」——寫成 `q Q` 就是教人用 `q` 開錄。真的綁法是 `Q` 錄、`q`
            // 放（`keys.rs`），而那兩個鍵的註釋自己寫着「**調換了的一對是最糟的
            // 那一種分歧**：手不讀字，就按下去，而這兩個按錯了不是没反應——它開
            // 始錄，蓋掉你本來要放的那一段」（#404）。
            ("Q q", say!("help.common.macros")),
            ("> <", say!("help.common.indent")),
        ] {
            out.push_str(&format!(
                "- `{keys}` — {what}
"
            ));
        }
        out.push_str(&format!("\n## {}\n\n", say!("help.common.space-title")));
        for (key, what) in Self::SPACE_KEYS {
            let key = Self::key_name(*key);
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
            (
                ":yume-chaifen on",
                say!("help.chinese.chaifen-under-candidates"),
            ),
            ("w b e", say!("help.chinese.word-boundaries")),
            (":word-mark tint", say!("help.chinese.word-tint")),
            (
                ":word-list-reload",
                say!("help.chinese.reload-project-words"),
            ),
            (":word-habit", say!("help.chinese.habit-words")),
            (":ruby", say!("help.chinese.annotate-reading")),
            (
                ":ruby-format html",
                say!("help.chinese.unify-reading-spelling"),
            ),
            (":render full", say!("render.wysiwyg")),
            (":indent 2", say!("help.chinese.first-line-indent")),
            (":view-hanging on", say!("help.chinese.hung-punctuation")),
            (":count", say!("help.chinese.count")),
        ] {
            out.push_str(&format!(
                "- `{keys}` — {what}
"
            ));
        }
        out
    }

    pub(super) fn help_vertical() -> String {
        let mut out = format!("# {}\n\n", say!("help.vertical.title"));
        for (keys, what) in [
            (":vertical", say!("help.vertical.turn-vertical")),
            ("h l", say!("help.vertical.previous-next-column")),
            ("j k", say!("help.vertical.down-up-column")),
            (":view-wrap 24", say!("help.vertical.column-length")),
            (":view-pages-vertical 2", say!("help.vertical.pages")),
            (":view-hanging on", say!("help.vertical.hung-punctuation")),
            (":view-margin always", say!("help.vertical.margin")),
            (":view-sentence", say!("help.vertical.sentence")),
            (":indent 2", say!("help.vertical.first-line-indent")),
            (":render full", say!("help.vertical.wysiwyg")),
        ] {
            out.push_str(&format!(
                "- `{keys}` — {what}
"
            ));
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
            // 搬進了空格選單（`t`/`T` 還給 vi 的 till），而這一頁没跟着搬——
            // 上面寫的 `t r`、`t s` 按下去一個都不管用，`t` 當場成了 till。
            ("␣t/ ␣t?", say!("help.table.who-uses-this")),
            ("␣to ␣tb ␣tf ␣tt", say!("help.table.four-surfaces")),
            ("␣ti ␣tw", say!("help.table.detail-and-folds")),
            ("␣tr ␣td", say!("help.table.add-or-drop-row")),
            ("␣ts ␣tS", say!("help.table.sort-by-column")),
            ("␣ty ␣tp", say!("help.table.yank-or-put-column")),
            (":table-rules off", say!("help.table.no-column-rules")),
        ] {
            out.push_str(&format!(
                "- `{keys}` — {what}
"
            ));
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contextual_help_lands_on_reference_and_keeps_history() {
        let mut ed = Editor::new();
        let _ = ed.current_buffer_mut().insert(0, "我的稿子");
        let manuscript = ed.current_buffer().id();
        ed.execute(":h :w@en").unwrap();
        assert!(ed.current_buffer().is_readonly());
        assert!(ed
            .current_buffer()
            .rope()
            .line(ed.cursor_line())
            .to_string()
            .contains("`:write`"));
        ed.walk_jumps(true);
        assert_eq!(ed.current_buffer().id(), manuscript);
        ed.execute(":help 'scrolloff'@en").unwrap();
        assert!(ed
            .current_buffer()
            .rope()
            .line(ed.cursor_line())
            .to_string()
            .starts_with("scrolloff ="));
        ed.execute(":help w@en").unwrap();
        assert!(ed
            .current_buffer()
            .rope()
            .line(ed.cursor_line())
            .to_string()
            .contains("`w`"));
        ed.execute(":help i_<Esc>@en").unwrap();
        assert!(ed
            .current_buffer()
            .rope()
            .line(ed.cursor_line())
            .to_string()
            .contains("`Esc` leaves"));
        let buffer = ed.current_buffer().id();
        ed.execute(":help no-such-topic").unwrap();
        assert_eq!(ed.current_buffer().id(), buffer);
    }
    #[test]
    fn embedded_links_and_language_suffixes() {
        let mut ed = Editor::new();
        ed.execute(":help help@zh-Hant").unwrap();
        assert!(ed.current_buffer().text().contains("查閲幫助"));
        ed.open_help_page("zh-Hant/SUMMARY.md", 0, None);
        assert!(ed.follow_help_link("manual/06-moving.md", None));
        assert!(ed.buffer_name().contains("zh-Hant/manual/06-moving.md"));
        assert!(
            ed.current_buffer().text().contains("本頁尚未翻譯")
                || ed.current_buffer().text().contains("英文")
                || ed.current_buffer().text().contains("not translated")
        );
        ed.walk_jumps(true);
        assert!(ed.buffer_name().contains("SUMMARY.md"));
        ed.execute(":help <Space> f@en").unwrap();
        assert!(ed
            .current_buffer()
            .rope()
            .line(ed.cursor_line())
            .to_string()
            .contains("`空格 f`"));
        ed.open_help_page("en/help.md", 0, None);
        let text = ed.current_buffer().text();
        let at = text[..text.find("[Manual contents]").unwrap()]
            .chars()
            .count();
        ed.set_cursor(at);
        ed.follow_link_here();
        assert!(ed.buffer_name().contains("en/SUMMARY.md"));
        ed.walk_jumps(true);
        assert!(ed.buffer_name().contains("en/help.md"));
    }
    #[test]
    fn help_completion_replaces_the_topic_only() {
        let (at, choices) = crate::command::complete_at("help :wr");
        assert_eq!(at, 5);
        assert!(choices.iter().any(|c| c.written() == ":write"));
        let (_, choices) = crate::command::complete_at("help 'scroll");
        assert!(choices.iter().any(|c| c.written() == "'scrolloff'"));
    }
}
