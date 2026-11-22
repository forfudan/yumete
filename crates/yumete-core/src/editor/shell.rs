//! The shell, the pipe, and writing markdown by name (#296).
//!
//! Three things the editor cannot do by itself: run a command, hand a
//! selection to one and take the answer back, and — the odd one out that has
//! always sat here — insert a heading, a rule or a note by name.

use super::*;

impl Editor {
    // ---- The command row (Feature #122, #302) ------------------------------

    /// A command line the front end should run.
    pub fn take_shell_request(&mut self) -> Option<Shell> {
        self.shell_request.take()
    }

    /// `C-z` asked the front end to stop the editor until `fg`, once.
    pub fn take_suspend_request(&mut self) -> bool {
        std::mem::take(&mut self.suspend_request)
    }

    /// Put what a command said in place of the text it was given.
    ///
    /// One edit, so one `u` takes it back — which matters more here than
    /// anywhere else, because the text that went in is gone and only the
    /// command knows how to make it again.
    /// **整份進去，整份回來**——`:format` 的 `kind = "filter"` 走這一支。
    ///
    /// Warning: **不許借 [`Self::provide_pipe_output`]**（會毀稿）。那一支換的是
    /// **選區**，而格式化餵進去的是整個緩衝：Normal 模式下選區是光標底下那一個字，
    /// 於是整份格式化好的稿子會被貼在那一個字的位置上，原文原封不動留在後面——
    /// `aaa\nbbb\nccc\n` 過一遍 `tr a-z A-Z` 出來的是
    /// `AAA\nBBB\nCCCaa\nbbb\nccc\n`，整份重了一遍，還少一個字。
    ///
    /// 一個撤回點，所以 `u` 收得回；可這是**看得見纔知道**的那一種壞，而格式化
    /// 完多半没人逐行看。
    pub fn provide_formatted_text(&mut self, output: &str) {
        if self.refuse_readonly() {
            return;
        }
        if output == self.current_buffer().text() {
            return;
        }
        // 同 `:s` 那一道：改完的行數對不上格子就不寫，而是說一句。
        if let Some(why) = self.substitution_breaks_the_grid(output) {
            self.status = why;
            return;
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().rewrite(0..len, output));
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
        self.forget_the_text();
    }

    /// **只有兩個把選區餵進去**（逐條照 helix 的 `shell_impl`）：
    /// `|` 與 `A-|` 餵，`!` 與 `A-!` 不餵——`!date` 是 vi 的 `:r !date`，「跑一條
    /// 命令，把答案放這裏」，光標在哪一段跟它没關係。
    pub(super) fn ask_the_shell(&mut self, line: String, put: crate::editor::Put) {
        use crate::editor::Put;
        let (start, end) = self.selection();
        let fed = match put {
            Put::Replace | Put::Nowhere => self.current_buffer().rope().slice(start..end).to_string(),
            Put::Before | Put::After | Put::Value | Put::Equation | Put::Lines(_) => String::new(),
        };
        let buffer = self.current_buffer();
        self.pipe_at = Some(crate::editor::PipeAt { buffer: buffer.id(), revision: buffer.revision(), start, end });
        self.shell_request = Some(crate::editor::Shell {
            line,
            how: crate::editor::How::Pipe(fed, put),
        });
    }

    /// **helix's `$`**: each selection, in the order they stand, goes to the
    /// command on its own; [`Self::provide_kept`] takes the answers.
    pub(super) fn ask_the_shell_to_keep(&mut self, line: String) {
        let rope = self.current_buffer().rope();
        let fed = self.sel.iter().map(|one| {
            let (start, end) = self.drawn(*one);
            rope.slice(start..end).to_string()
        });
        let fed = fed.collect();
        self.shell_request = Some(crate::editor::Shell { line, how: crate::editor::How::Keep(fed) });
    }

    /// Which selections the command exited 0 on, one answer each in the
    /// order they were sent. None kept leaves them all, and says so (helix's
    /// 「No selections remaining」).
    pub fn provide_kept(&mut self, kept: &[bool]) {
        let ranges: Vec<crate::selection::Range> = self.sel.iter().copied().collect();
        if ranges.len() != kept.len() {
            return;
        }
        let primary = self.sel.primary_index();
        let keep: Vec<(usize, crate::selection::Range)> =
            ranges.into_iter().enumerate().filter(|(i, _)| kept[*i]).collect();
        if keep.is_empty() {
            self.status = say!("shell.none-kept");
            return;
        }
        let at = keep.iter().position(|(i, _)| *i >= primary).unwrap_or(keep.len() - 1);
        self.sel.rebuild(keep.into_iter().map(|(_, one)| one).collect(), at);
    }

    pub fn provide_pipe_output(&mut self, output: &str, put: crate::editor::Put) {
        use crate::editor::Put;
        // The span the command was asked about, not the selection when the
        // answer comes: vim
        // has left Visual mode by the time the answer comes. Text that
        // changed in between gets nothing, since the span no longer names
        // what the command was given.
        let (buffer, revision) = (self.current_buffer().id(), self.current_buffer().revision());
        let (start, end) = match self.pipe_at.take() {
            Some(at) if at.buffer == buffer && at.revision == revision => (at.start, at.end),
            Some(_) => {
                self.status = say!("shell.text-changed-meanwhile");
                return;
            }
            None => self.selection(),
        };
        if let Put::Lines(below) = put {
            return self.read_output_in(output, below);
        }
        // **插在前面/後面的，把原文接回去**。整段仍舊走「換掉這一
        // 段」那條路，所以換行規矩、表格那道閘、撤銷點全是現成的——`!` 和 `|`
        // 於是不會慢慢長出兩套邊界情形。
        let keep = self.current_buffer().rope().slice(start..end).to_string();
        let joined;
        let output = match put {
            Put::Replace | Put::Nowhere | Put::Lines(_) => output,
            Put::Before => {
                joined = format!("{output}{keep}");
                joined.as_str()
            }
            Put::After => {
                joined = format!("{keep}{output}");
                joined.as_str()
            }
            Put::Value | Put::Equation => {
                let value = output.trim_end_matches(['\n', '\r']);
                let body = keep.trim_end_matches(['\n', '\r']);
                let between = if put == Put::Equation { " = " } else { "" };
                joined = format!("{body}{between}{value}{}", &keep[body.len()..]);
                joined.as_str()
            }
        };
        // A filter ends its output with a newline whether or not what it was
        // given had one. Keeping it where the selection did not have one pushes
        // the rest of the paragraph down a line every time; dropping it where
        // the selection *did* have one runs two lines together. So it follows
        // what was there.
        let had = self
            .current_buffer()
            .rope()
            .slice(start..end)
            .to_string()
            .ends_with('\n');
        let text = match had {
            true => output,
            false => output.strip_suffix('\n').unwrap_or(output),
        };
        // A filter over whole rows is the *advertised* use — the manual's own
        // example is `LC_ALL=C sort` over a table — so the grid does not refuse
        // it after spawning the command and reading its output. What matters
        // is not that no delimiter moved, it is that **every row that comes
        // back has a row's shape**; `sort -u` dropping a duplicate row is a
        // table operation, not damage.
        let rows = self.pipe_covers_whole_rows(start, end);
        if rows {
            if let Some(why) = self.rows_break_the_grid(text) {
                self.status = why;
                return;
            }
        }
        self.snapshot();
        let done = if rows {
            self.without_cell_guard(|e| e.overwrite(start, end, text))
        } else {
            self.overwrite(start, end, text)
        };
        if !done {
            return;
        }
        self.sel.set_anchor(start);
        self.sel.set_head((start + text.chars().count()).saturating_sub(1).max(start));
        self.clamp_cursor();
        // What happened to the text: a `|` replaced the span, the other
        // three kept it and put the answer beside it.
        let written = text.chars().count();
        self.status = match put {
            Put::Replace | Put::Nowhere => say!("edit.pipe-replaced-characters", written),
            Put::Before | Put::After | Put::Value | Put::Equation | Put::Lines(_) => {
                say!("edit.pipe-inserted-characters", written.saturating_sub(keep.chars().count()))
            }
        };
    }

    /// **`:read !cmd`'s answer**, as lines below line `below`: nothing for
    /// no output (vim reads no empty line in), and the cursor on the last
    /// line read, at its first non-blank (vim's `do_filter`).
    fn read_output_in(&mut self, output: &str, below: usize) {
        if output.is_empty() {
            return;
        }
        let ending = self.current_buffer().ending();
        let text = output.replace("\r\n", "\n");
        let text = match ending {
            "\n" => text,
            _ => text.replace('\n', ending),
        };
        self.read_in(&text, below, true);
    }

    /// Whether a range covers whole rows of the grid — line start to line end.
    fn pipe_covers_whole_rows(&self, start: usize, end: usize) -> bool {
        if !self.table_here() {
            return false;
        }
        let rope = self.current_buffer().rope();
        let end = end.min(rope.len_chars());
        start == motion::line_start(rope, start)
            && (end == rope.len_chars()
                || end == motion::line_end(rope, end.saturating_sub(1))
                || rope.char(end.saturating_sub(1)) == '\n')
    }

    /// Whether any line of `text` would not be a row of this grid.
    fn rows_break_the_grid(&self, text: &str) -> Option<String> {
        let view = self.table()?;
        // **The table the cursor is in, not the one it was entered in** (#283):
        // a document holds as many tables as somebody typed, and filtering the
        // five-column one through `sort` while the view still carried the
        // two-column one's schema refused every row it was handed.
        let want = self.table_column_count_at(self.cursor_line());
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let got = view.cells(line).len();
            if got != want {
                return Some(say!("table.filter-changed-shape", i + 1, got, want));
            }
        }
        None
    }

    /// Put what a command said into a buffer of its own.
    ///
    /// A buffer rather than a message: the answer to `wc -w` is a number and
    /// would fit anywhere, but the answer to `git log` is two hundred lines,
    /// and a writer wants to search it, yank from it and keep it while they go
    /// on writing. It is the same place `:grep` puts its answers.
    pub fn provide_shell_output(&mut self, line: &str, output: &str) {
        let text = if output.trim().is_empty() {
            format!("$ {line}\n{}\n", say!("shell.no-output"))
        } else {
            format!("$ {line}\n{output}")
        };
        let mut buffer = crate::Buffer::from_text(&text);
        buffer.name_as(&format!("!{line}"));
        self.add_buffer(buffer);
        self.status = say!("shell.done", line);
    }

    /// **`:markdown …`** — write a piece of Markdown at the cursor.
    ///
    /// The things a manuscript keeps needing and nobody wants to type: a
    /// footnote with the next free number *and* its note at the foot, an inline
    /// note, a table of a given size. The cursor is left where the typing goes,
    /// in Insert, because that is the next thing that happens every time.
    pub(super) fn write_markdown(&mut self, bit: crate::command::MarkdownBit) {
        use crate::command::MarkdownBit;
        match bit {
            MarkdownBit::Footnote => {
                // A footnote is Markdown. In a Typst book `[^1]` is four
                // characters of nothing, and in a plain manuscript it is four
                // characters the reader did not ask for.
                if self.current_buffer().syntax() != crate::syntax::Syntax::Markdown {
                    self.status = say!("note.not-markdown");
                    return;
                }
                // **The next free number**, from the file itself: a footnote
                // whose number is already taken is a footnote pointing at
                // somebody else's note.
                let text = self.current_buffer().text();
                let taken: Vec<usize> = crate::markdown::footnote_numbers(&text);
                let n = (1..).find(|n| !taken.contains(n)).unwrap_or(1);
                let tag = format!("[^{n}]");
                self.snapshot();
                let at = self.sel.head();
                if !self.edit_insert(at, &tag) {
                    return;
                }
                self.set_cursor(at + tag.chars().count());
                // …and the note it points at, written and stood in. `gd` does
                // exactly this when it cannot find a note; this is the same
                // path, asked for rather than stumbled into.
                self.definition_preview = false;
                let rope = self.current_buffer().rope();
                let end = rope.len_chars();
                let text = self.current_buffer().text();
                let lead = match text.ends_with("\n\n") {
                    true => String::new(),
                    false => match text.ends_with('\n') {
                        true => "\n".to_string(),
                        false => "\n\n".to_string(),
                    },
                };
                let note = format!("{lead}{tag}: ");
                self.write_the_note(end, &note, &tag);
                self.mode = Mode::Insert;
            }
            MarkdownBit::InlineNote => {
                let at = self.sel.head();
                self.snapshot();
                if !self.edit_insert(at, "^[]") {
                    return;
                }
                // Between the brackets, where the note goes.
                self.set_cursor(at + 2);
                self.mode = Mode::Insert;
                self.status = say!("md.inline-note-inserted");
            }
        }
    }
}

/// **decimo's options among the words written after `:decimo`**, and the
/// expression the rest of them make: `sqrt(e) -P 50` is `sqrt(e)` and
/// `-P 50`. Only decimo's own option names are options, so `-pi` is an
/// expression; an option that takes a value takes the next word.
pub(crate) fn decimo_words(args: &str) -> (String, String) {
    const VALUED: &[&str] = &["-P", "--precision", "-R", "--rounding-mode", "--delimiter"];
    const BARE: &[&str] = &["-S", "--scientific", "-E", "--engineering", "--pad"];
    let (mut expression, mut options): (Vec<&str>, Vec<String>) = (Vec::new(), Vec::new());
    let mut words = args.split_whitespace();
    while let Some(word) = words.next() {
        let name = word.split('=').next().unwrap_or(word);
        let glued = word.len() > 2 && VALUED.iter().any(|v| v.len() == 2 && word.starts_with(v));
        if VALUED.contains(&word) {
            options.push(quoted(word));
            if let Some(value) = words.next() {
                options.push(quoted(value));
            }
        } else if BARE.contains(&word) || glued || (word.contains('=') && VALUED.contains(&name)) {
            options.push(quoted(word));
        } else {
            expression.push(word);
        }
    }
    (expression.join(" "), options.join(" "))
}

/// `word` as one word to the shell commands run through: a POSIX shell's
/// single quotes, a quote in it closed, escaped and opened again; on
/// Windows `cmd.exe`'s double quotes, which a path cannot hold.
pub(crate) fn quoted(word: &str) -> String {
    match cfg!(windows) {
        true => format!("\"{}\"", word.replace('"', "\\\"")),
        false => format!("'{}'", word.replace('\'', "'\\''")),
    }
}

/// **A new folder of this run's own** in the temp folder: made, never found,
/// so it cannot be one somebody else put there, and only this user may
/// write in it.
pub(crate) fn own_temp_dir(what: &str) -> std::io::Result<std::path::PathBuf> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("yumete-{what}-{}-{stamp}", std::process::id()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&dir)?;
    Ok(dir)
}

/// Whether a program called `name` is on `PATH`.
pub(crate) fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|dir| {
        dir.join(name).is_file() || (cfg!(windows) && dir.join(format!("{name}.exe")).is_file())
    })
}

impl Editor {
    /// **Give a program the terminal** in the current file's directory (the
    /// working directory for a buffer with no file), and open the files it
    /// writes to `chooser` when it ends.
    pub(super) fn run_a_program(&mut self, line: String, chooser: Option<std::path::PathBuf>) {
        let dir = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent())
            .filter(|d| d.is_dir())
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| self.working_dir());
        self.shell_request = Some(crate::editor::Shell {
            line,
            how: crate::editor::How::Program { dir, chooser },
        });
    }

    /// **The files a program chose**, one path a line in `chosen` (yazi's
    /// `--chooser-file`), opened in turn, the last one shown; what cannot be
    /// opened is said.
    ///
    /// **A picture is not opened but referred to**: it goes into the current
    /// buffer at the cursor, in the markup that buffer is written in (see
    /// [`picture_reference`]), one a line and one undo step for all of them,
    /// before the other files open. A buffer that cannot take text — locked,
    /// or a preview — gets nothing and nothing opens, so a choice is never
    /// half carried out.
    pub fn open_chosen(&mut self, chosen: &str) {
        let lines = chosen.lines().map(str::trim).filter(|l| !l.is_empty());
        let (pictures, files): (Vec<&str>, Vec<&str>) = lines.partition(|l| is_picture(Path::new(l)));
        if !pictures.is_empty() {
            if self.current_buffer().is_preview() {
                self.status = say!("shell.chosen-pictures-in-preview");
                return;
            }
            if self.current_buffer().is_readonly() {
                self.status = say!("shell.chosen-pictures-readonly");
                return;
            }
            if !self.put_picture_references(&pictures) {
                return;
            }
        }
        let mut opened = 0usize;
        for line in files {
            match self.open_file(line) {
                Ok(()) => opened += 1,
                Err(err) => {
                    self.status = say!("buffer.cannot-open", line, err);
                    return;
                }
            }
        }
        if !pictures.is_empty() {
            self.status = match opened {
                0 => say!("shell.chosen-pictures-put", pictures.len()),
                _ => say!("shell.chosen-pictures-put-files-opened", pictures.len(), opened),
            };
        } else if opened > 1 {
            self.status = say!("shell.opened-chosen", opened.to_string());
        }
    }

    /// One reference per picture at the cursor, as one edit: the caret ends
    /// on the last character put in, as after `P`, or past it in Insert mode.
    /// `false` when the buffer refused, the status line saying why.
    fn put_picture_references(&mut self, pictures: &[&str]) -> bool {
        let base = match self.current_buffer().path().and_then(Path::parent) {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => self.working_dir(),
        };
        let syntax = self.current_buffer().syntax();
        let ending = self.current_buffer().ending();
        let text = pictures
            .iter()
            .map(|p| picture_reference(syntax, &relative_path(Path::new(p), &base)))
            .collect::<Vec<_>>()
            .join(ending);
        let at = self.cursor();
        self.snapshot();
        if !self.overwrite(at, at, &text) {
            return false;
        }
        let end = at + text.chars().count();
        let to = match self.mode == Mode::Insert {
            true => end,
            false => end.saturating_sub(1).max(at),
        };
        self.sel.set_anchor(to);
        self.sel.set_head(to);
        self.clamp_cursor();
        self.refresh_goal_column();
        true
    }

    /// **Look at the disk on the next turn** rather than at the next tick: a
    /// program that had the terminal (lazygit) may have rewritten files.
    pub fn check_the_disk_now(&mut self) {
        self.last_disk_check = None;
        self.reload_warned = false;
    }
}

/// Whether a chosen file is a picture, by its extension in any case.
fn is_picture(path: &Path) -> bool {
    const PICTURES: [&str; 10] = ["png", "jpg", "jpeg", "gif", "webp", "svg", "avif", "bmp", "tiff", "tif"];
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PICTURES.iter().any(|p| p.eq_ignore_ascii_case(e)))
}

/// **`path` as seen from `base`**, with `/` between the parts on every
/// system, so a manuscript moved with its pictures still finds them. The
/// disk's own spelling of both is compared, so `/var` and `/private/var`
/// meet; a path on another drive has no relative form and stays whole.
fn relative_path(path: &Path, base: &Path) -> String {
    let spelled = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (path, base) = (spelled(path), spelled(base));
    let parts = |p: &Path| -> Vec<String> {
        p.components()
            .filter_map(|c| match c {
                std::path::Component::Normal(w) => Some(w.to_string_lossy().into_owned()),
                std::path::Component::ParentDir => Some("..".to_string()),
                std::path::Component::Prefix(w) => {
                    let w = w.as_os_str().to_string_lossy();
                    Some(w.strip_prefix(r"\\?\").unwrap_or(&w).to_string())
                }
                std::path::Component::RootDir | std::path::Component::CurDir => None,
            })
            .collect()
    };
    let (to, from) = (parts(&path), parts(&base));
    let same_root = path.has_root() == base.has_root();
    let shared = match same_root {
        true => to.iter().zip(&from).take_while(|(a, b)| a == b).count(),
        false => 0,
    };
    // Different drives, or nothing in common below a drive letter: no
    // relative spelling reaches it.
    let drive = |p: &[String]| p.first().filter(|w| w.ends_with(':')).cloned();
    if !same_root || (drive(&to).is_some() && shared == 0) {
        let whole = to.join("/");
        return match path.has_root() && drive(&to).is_none() {
            true => format!("/{whole}"),
            false => whole,
        };
    }
    let mut out: Vec<String> = std::iter::repeat_n("..".to_string(), from.len() - shared).collect();
    out.extend(to[shared..].iter().cloned());
    out.join("/")
}

/// **The reference to a picture** at `path`, in the markup of `syntax`:
/// Markdown's `![](path)`, in the angle-bracket form CommonMark asks for when
/// the path holds a space or a character the bare form would read as markup;
/// Typst's `#image("path")`; elsewhere the path alone, which is how plain
/// text or code names a file.
pub(super) fn picture_reference(syntax: crate::syntax::Syntax, path: &str) -> String {
    use crate::syntax::Syntax;
    match syntax {
        Syntax::Markdown => {
            let marked = |c: char| c.is_whitespace() || c.is_control() || "()<>\\".contains(c);
            if !path.chars().any(marked) {
                return format!("![]({path})");
            }
            let mut inside = String::new();
            for c in path.chars() {
                if "<>\\".contains(c) {
                    inside.push('\\');
                }
                inside.push(c);
            }
            format!("![](<{inside}>)")
        }
        Syntax::Typst => format!("#image(\"{}\")", path.replace('\\', "\\\\").replace('"', "\\\"")),
        _ => path.to_string(),
    }
}
