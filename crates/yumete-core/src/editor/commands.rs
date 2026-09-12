//! `execute` — every command, and where it goes (#296).
//!
//! One 800-line `match` on `Command`, plus the two things that hang off
//! saving: the 安全核驗 question (#295) and the write that answers it.

use super::*;

impl Editor {
    /// Create a new, empty scratch buffer and make it active.
    pub fn new_buffer(&mut self) {
        self.add_buffer(Buffer::scratch());
    }

    /// Run a `:` command line.
    ///
    /// Returns [`CommandOutcome::Quit`] when a `:q` / `:q!` should end the
    /// session, and [`CommandOutcome::Continue`] otherwise.
    pub fn execute(&mut self, line: &str) -> Result<CommandOutcome, EditorError> {
        // **The line belongs to this command.** A command that fails says so
        // through its error, and the status is where the *last* command's
        // answer was: a failed `:export` used to leave 「存了 ch1.md」 standing,
        // which reads as an export that worked.
        self.status.clear();
        // What this command needs before it can mean anything (Feature #170).
        // A setting whose prerequisite is missing used to be *set* and then
        // read by nobody: `:view-hanging on` on a horizontal page turned a flag on,
        // changed nothing, and said 「標點旁置：開」, which is three kinds of
        // wrong at once.
        let (line, force) = match line.trim_end().strip_suffix(" force") {
            Some(rest) => (rest.trim_end(), true),
            None => (line, false),
        };
        let unmet: Vec<command::Need> = command::needs_of(line)
            .iter()
            .copied()
            .filter(|need| !self.meets(*need))
            .collect();
        if !unmet.is_empty() {
            if !force {
                let what: Vec<String> = unmet.iter().map(|n| n.says()).collect();
                self.status = say!(
                    "cmd.needs-these-first",
                    what.join(&say!("label.comma"))
                );
                return Ok(CommandOutcome::Continue);
            }
            // Satisfied until they stay satisfied: turning the page 縱書 can
            // make a second prerequisite start mattering, and a `force` that
            // half-worked would be worse than one that did not.
            for _ in 0..3 {
                let left: Vec<command::Need> = command::needs_of(line)
                    .iter()
                    .copied()
                    .filter(|need| !self.meets(*need))
                    .collect();
                if left.is_empty() {
                    break;
                }
                for need in left {
                    self.satisfy(need);
                }
            }
        }
        let mut asked = command::parse(line)?;
        // `force` is stripped above, where it means 「do it anyway」 for a
        // command whose prerequisites are missing. `:convert … force` spells
        // the same word for the same kind of reason — do the *bigger* edit,
        // the one that changes words and not just characters — and the strip
        // above ate it before [`command::parse`] ever saw it. Handing it back
        // is the whole fix: every other reader of that word (the `:` menu,
        // [`command::parse`] called directly) sees it in place.
        if force {
            if let Command::Convert(command::ConvertAsk::Run { force, .. }) = &mut asked {
                *force = true;
            }
        }
        match asked {
            Command::Open(path) => {
                self.open_file(path).map_err(EditorError::Io)?;
                // A file opened mid-session can carry a draft just as one named
                // on the command line can.
                self.announce_recovery();
                Ok(CommandOutcome::Continue)
            }
            Command::NewBuffer => {
                self.new_buffer();
                Ok(CommandOutcome::Continue)
            }
            Command::Write(path) => {
                self.write_current(path.as_deref())?;
                Ok(CommandOutcome::Continue)
            }
            Command::SaveAs { path, force } => {
                let target = PathBuf::from(&path);
                // The same one rule: a file another buffer is holding may only
                // be written by that buffer.
                if let Some(which) = self.buffer_holding(&target) {
                    if which != self.current {
                        let name = self.buffers[which].display_name();
                        self.status = say!("buffer.already-open-elsewhere", name);
                        return Ok(CommandOutcome::Continue);
                    }
                }
                self.current_buffer_mut()
                    .save_as(target, force)
                    .map_err(EditorError::Io)?;
                self.status = say!("buffer.saved", self.current_buffer().display_name());
                Ok(CommandOutcome::Continue)
            }
            Command::WriteForce(path) => {
                self.write_forcing(path.as_deref(), true)?;
                Ok(CommandOutcome::Continue)
            }
            Command::Reload { force } => {
                self.reload(force)?;
                Ok(CommandOutcome::Continue)
            }
            Command::ReloadAuto(on) => {
                match on {
                    Some(on) => {
                        self.reload_auto = on;
                        // Asked now, not at the next keystroke: turning it on
                        // is usually a writer who already suspects the file
                        // moved.
                        self.last_disk_check = None;
                        self.reload_warned = false;
                        let word = if on { "on" } else { "off" };
                        self.status = say!("autoreload.set", word);
                    }
                    None => {
                        self.status = say!(
                            "autoreload.set",
                            if self.reload_auto { "on" } else { "off" }
                        )
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            Command::SetReadonly(on) => {
                match on {
                    Some(on) => {
                        self.current_buffer_mut().set_readonly(on);
                        // **`off` lifts `-R` too.** The flag locks every file
                        // the session opens, and a writer who has just said
                        // 「no, I do mean to edit this」 should not find the
                        // next `:open` locked again with no way to say it.
                        if !on {
                            self.readonly_default = false;
                        }
                        let word = if on { "on" } else { "off" };
                        // Locking a buffer that has unsaved changes takes `u`
                        // away with everything else, so say so once rather
                        // than let it be discovered at the worst moment.
                        self.status = match on && self.current_buffer().is_modified() {
                            true => say!("readonly.on-with-unsaved"),
                            false => say!("readonly.set", word),
                        };
                    }
                    None => {
                        self.status = say!(
                            "readonly.set",
                            if self.current_buffer().is_readonly() { "on" } else { "off" }
                        )
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            Command::Quit { force } => self.quit(force),
            Command::QuitAll { force } => self.quit_all(force),
            Command::Substitute {
                pattern,
                replacement,
                global,
                ignore_case,
                count_only,
                reshape,
                rows,
            } => {
                self.substitute(Substitution {
                    pattern: &pattern,
                    replacement: &replacement,
                    global,
                    ignore_case,
                    count_only,
                    reshape,
                    rows,
                });
                Ok(CommandOutcome::Continue)
            }
            Command::Undo => {
                self.undo();
                Ok(CommandOutcome::Continue)
            }
            Command::Redo => {
                self.redo();
                Ok(CommandOutcome::Continue)
            }
            Command::SetLayout(direction) => {
                // A grid runs across and down; a 縱書 page runs down and to the
                // left. There is no honest way to draw one as the other, so the
                // command is refused rather than quietly doing something else —
                // and it says which key gets you out.
                let wants_vertical = match direction {
                    Some(l) => l == Layout::Vertical,
                    None => self.layout == Layout::Horizontal,
                };
                if self.grid_is_drawn() && wants_vertical {
                    self.status = say!("table.vertical-not-allowed");
                    return Ok(CommandOutcome::Continue);
                }
                let layout = match direction {
                    Some(l) => {
                        self.set_layout(l);
                        l
                    }
                    None => self.toggle_layout(),
                };
                // Not `layout.label()`: that is the config's spelling, in
                // English, and it was being read out inside a Chinese
                // sentence to a reader who had just switched to 竪排.
                self.status = match layout {
                    Layout::Vertical => say!("layout.vertical"),
                    Layout::Horizontal => say!("layout.horizontal"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::Ruby => {
                self.enter_ruby_mode();
                Ok(CommandOutcome::Continue)
            }
            Command::RenderRuby { dialect, on } => {
                self.render_ruby(dialect, on);
                let listed_names: Vec<String> =
                    self.ruby.iter().map(|d| d.name().to_string()).collect();
                self.status = if listed_names.is_empty() {
                    say!("ruby.layout-off")
                } else {
                    say!("ruby.layout-on", listed(&listed_names))
                };
                Ok(CommandOutcome::Continue)
            }
            Command::AutoRuby { rare } => {
                self.auto_ruby(rare);
                Ok(CommandOutcome::Continue)
            }
            Command::FormatRuby(dialect) => {
                self.format_ruby(dialect);
                Ok(CommandOutcome::Continue)
            }
            Command::WriteQuit(path) => {
                // **`:wq <名字>` means 「save it as this, I am done」.** Left as
                // `:w <path>` it wrote a *copy* and then refused to quit,
                // because the chapter itself was still unsaved — and a writer
                // with vi's muscle memory reads that refusal and reaches for
                // `:q!`. So it rebinds, exactly as `:write-as` does, and the file
                // that is saved is the one the name says.
                match path.as_deref() {
                    Some(path) => {
                        let target = PathBuf::from(path);
                        if let Some(which) = self.buffer_holding(&target) {
                            if which != self.current {
                                let name = self.buffers[which].display_name();
                                self.status =
                                    say!("buffer.already-open-elsewhere", name);
                                return Ok(CommandOutcome::Continue);
                            }
                        }
                        // `save_as` does not go through the funnel, so the gate
                        // is asked for here too — `:wq 第二章.md` over an
                        // existing chapter multiplies it exactly as `:wq` does.
                        if !self.oversize_answered {
                            if let Some(ask) = self.oversize_query(Some(path)) {
                                self.query = Some(ask);
                                return Ok(CommandOutcome::Continue);
                            }
                        }
                        self.current_buffer_mut()
                            .save_as(target, false)
                            .map_err(EditorError::Io)?;
                        self.status = say!("buffer.saved", self.current_buffer().display_name());
                    }
                    None => {
                        // A question standing is not a save, and `:wq` on a
                        // save that did not happen would take the manuscript
                        // off the screen with the answer still unanswered.
                        if matches!(self.write_current(None)?, Wrote::Asked) {
                            return Ok(CommandOutcome::Continue);
                        }
                    }
                }
                // Saving *this* buffer is not saving the session: another open
                // file may still be dirty, and `:wq` reads as "everything is
                // safe now", so it is held to the same check `:q` is.
                self.quit(false)
            }
            Command::ReplaceFound(text, reshape) => {
                self.replace_found(&text, reshape);
                Ok(CommandOutcome::Continue)
            }
            Command::WriteAll => self.write_all(),
            Command::Search { pattern, by } => {
                match by {
                    crate::command::Axis::Row => {
                        self.last_search = pattern;
                        // A row search takes `n` back from a column search's
                        // answers, the way `/` does.
                        self.hits = None;
                        let forward = self.search_forward;
                        self.repeat_search(forward);
                    }
                    // A column search is a table's; a document has no columns
                    // to run down.
                    crate::command::Axis::Column if self.table_here() => {
                        // Shown in the other work area, which is what a column
                        // search is for: 卵's own row and a row that uses 卵
                        // are two places, and the question is about both.
                        self.definition_preview = true;
                        self.search_columns(&pattern)
                    }
                    crate::command::Axis::Column => {
                        self.status = say!("table.not-in-a-table")
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            Command::Word(what) => self.word_command(what),
            Command::SetBands(n) => {
                self.set_bands(n);
                Ok(CommandOutcome::Continue)
            }
            Command::SetNumberFill(on) => {
                self.number_fill = on.unwrap_or(!self.number_fill);
                self.status = match self.number_fill {
                    true => say!("layout.line-numbers-on-a-band"),
                    false => say!("layout.line-numbers-on-the-page"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetIndentHint(hint) => {
                self.indent_hint = hint;
                self.status = say!("layout.indent-hint", hint.name());
                Ok(CommandOutcome::Continue)
            }
            Command::SetIndent(n) => {
                self.set_indent(n);
                Ok(CommandOutcome::Continue)
            }
            Command::CheckTable => {
                self.check_table();
                Ok(CommandOutcome::Continue)
            }
            Command::CheckPunct => {
                self.check_punct();
                Ok(CommandOutcome::Continue)
            }
            Command::CheckCharset => {
                self.check_charset();
                Ok(CommandOutcome::Continue)
            }
            Command::CheckUsage => {
                self.check_usage();
                Ok(CommandOutcome::Continue)
            }
            Command::Progress => {
                self.progress_report();
                Ok(CommandOutcome::Continue)
            }
            Command::Target(target) => {
                self.set_target(target);
                Ok(CommandOutcome::Continue)
            }
            Command::GotoRow(key) => {
                self.goto_row(&key);
                Ok(CommandOutcome::Continue)
            }
            Command::Recover { discard } => self.recover(discard),
            Command::GotoLine(n) => {
                self.goto_line(n);
                Ok(CommandOutcome::Continue)
            }
            Command::Count => {
                self.status = self.count_report();
                Ok(CommandOutcome::Continue)
            }
            Command::NextBuffer => {
                self.next_buffer();
                Ok(CommandOutcome::Continue)
            }
            Command::PreviousBuffer => {
                self.prev_buffer();
                Ok(CommandOutcome::Continue)
            }
            Command::CloseBuffer { force } => self.close_buffer(force),
            Command::Export {
                format,
                path,
                force,
            } => self.export(&format, path.as_deref(), force),
            Command::Grep(pattern) => {
                let root = self.project_root();
                self.grep(&pattern, &root)
            }
            Command::Conflicts => {
                self.list_conflicts();
                Ok(CommandOutcome::Continue)
            }
            Command::Diff(against) => {
                self.diff_against(against.as_deref());
                Ok(CommandOutcome::Continue)
            }
            Command::Outline(nth) => {
                let headings = self.outline();
                if headings.is_empty() {
                    self.status = say!("goto.no-headings");
                    return Ok(CommandOutcome::Continue);
                }
                match nth {
                    // `:toc <n>` goes to the nth heading…
                    Some(n) => match headings.get(n.saturating_sub(1)) {
                        Some(&(line, _, _)) => self.goto_line(line + 1),
                        None => self.status = say!("goto.only-n-headings", headings.len()),
                    },
                    // …and a bare `:toc` opens the outline, which is a *list*
                    // — one heading a line, scrollable, with the keys. It used
                    // to join all of them into the status line with three
                    // spaces between, which for a novel is 700 chapters on one
                    // row, of which the reader can see four.
                    None => {
                        self.show_sidebar(crate::sidebar::View::Outline);
                        self.status = say!("toc.headings-found", headings.len());
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            Command::Tutor => {
                self.open_tutor();
                Ok(CommandOutcome::Continue)
            }
            Command::Help(topic) => {
                self.open_help(topic.as_deref());
                Ok(CommandOutcome::Continue)
            }
            Command::ListBuffers => {
                // The picker, not the status line: with 122 chapters open the
                // list is 1,783 characters and the status line is one row. A
                // list you cannot read is not a list — and the picker is the
                // same list, searchable, which is what you wanted anyway.
                self.open_buffer_picker();
                Ok(CommandOutcome::Continue)
            }
            Command::SetHanging(want) => {
                let on = want.unwrap_or(!self.hanging);
                self.set_hanging_punctuation(on);
                self.status = if on {
                    say!("layout.hung-punctuation-on")
                } else {
                    say!("layout.hung-punctuation-off")
                };
                Ok(CommandOutcome::Continue)
            }
            Command::Clipboard { yank } => {
                if yank {
                    self.copy_to_clipboard();
                } else {
                    self.clipboard_paste(true);
                }
                Ok(CommandOutcome::Continue)
            }
            Command::SetSyntax(name) => {
                match name {
                    Some(name) => match crate::syntax::Syntax::parse(&name) {
                        Some(syntax) => {
                            self.set_syntax(syntax);
                            self.status = say!("render.markup-is", syntax.name());
                        }
                        None => {
                            self.status = say!("render.no-such-markup", name)
                        }
                    },
                    None => {
                        self.status = say!("render.markup-is", self.syntax().name());
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            // Both are questions only the front end can answer — it is the one
            // holding the IME — so they go out as requests, like `:yume-scheme`.
            Command::YumeStatus => {
                self.scheme_request = Some(String::from("?"));
                Ok(CommandOutcome::Continue)
            }
            Command::BuiltinScheme => {
                self.scheme_request = Some(String::from("!"));
                Ok(CommandOutcome::Continue)
            }
            // 上屏方式 is the engine's, so it rides the same channel: a word
            // rather than a sigil, because there is no scheme called `commit:`
            // and a request one can read is worth the four characters.
            Command::YumeCommit(mode) => {
                self.scheme_request = Some(format!("commit:{}", mode.unwrap_or_default()));
                Ok(CommandOutcome::Continue)
            }
            // 候選面板 rides it too, for the same reason: the front end is the
            // one that draws a panel, so it is the one that can be asked not
            // to (Feature #211).
            Command::YumePanel(mode) => {
                self.scheme_request = Some(format!("panel:{}", mode.unwrap_or_default()));
                Ok(CommandOutcome::Continue)
            }
            // Taken by the front end **after the next frame**: the command
            // line is still open on this one, and a picture of the thing you
            // are debugging with the debugger's own prompt across it is not a
            // picture of the thing.
            Command::Screenshot { shot, force } => self.take_a_picture(shot, force),
            Command::InstalledScheme => {
                self.scheme_request = Some(String::from("~"));
                Ok(CommandOutcome::Continue)
            }
            Command::YumeLanguage(want) => {
                // `lang:` for the same reason `commit:` and `panel:` have a
                // prefix: the front end holds the session, and this is one
                // more question about it — asked by name now that there are
                // three answers and not two (#290).
                self.scheme_request = Some(format!(
                    "lang:{}",
                    match want {
                        command::Engagement::Chinese => "chinese",
                        command::Engagement::Ascii => "abc",
                        command::Engagement::Off => "off",
                    }
                ));
                Ok(CommandOutcome::Continue)
            }
            Command::UserTable(path) => {
                // The `=` marks it as a path rather than a scheme tag: the
                // front end holds the IME, and this is the third thing to ask
                // it about the same session.
                self.scheme_request = Some(format!("={path}"));
                Ok(CommandOutcome::Continue)
            }
            Command::Shell { line, interactive } => {
                self.shell_request = Some(Shell {
                    line,
                    how: if interactive { How::Terminal } else { How::Capture },
                });
                Ok(CommandOutcome::Continue)
            }
            Command::Pipe(line) => {
                let (start, end) = self.selection();
                let text = self.current_buffer().rope().slice(start..end).to_string();
                self.shell_request = Some(Shell {
                    line,
                    how: How::Pipe(text),
                });
                Ok(CommandOutcome::Continue)
            }
            Command::Convert(ask) => {
                self.convert(ask);
                Ok(CommandOutcome::Continue)
            }
            Command::SetPreview(on) => {
                // Already running: the question 「where is it?」 is the one a
                // writer actually asks, and the address was said once and lost.
                if on && self.preview_at.is_some() {
                    self.preview_request = Some(Preview::Show);
                    return Ok(CommandOutcome::Continue);
                }
                self.preview_request = Some(if on {
                    match self.current_buffer().path() {
                        Some(path) => Preview::Start {
                            path: path.to_path_buf(),
                            syntax: self.current_buffer().syntax(),
                        },
                        None => {
                            self.status = say!("preview.save-first");
                            return Ok(CommandOutcome::Continue);
                        }
                    }
                } else {
                    Preview::Stop
                });
                Ok(CommandOutcome::Continue)
            }
            Command::SetRender(how) => {
                self.set_render(how);
                self.status = match how {
                    Render::Off => say!("render.source"),
                    Render::Basic => say!("render.full"),
                    Render::Full => say!("render.wysiwyg"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::ReportRender => {
                let word = |level: TableLevel| match level {
                    TableLevel::Off => say!("level.off"),
                    TableLevel::Basic => say!("level.basic"),
                    TableLevel::Full => say!("level.full"),
                };
                // **The three it writes**, not four. `:indent` reports itself
                // — it is not `:render`'s to set, so saying its level here
                // would read as a claim that it is. All three have three
                // levels since 2026-09-06: ruby used to have two states and an
                // N-to-1 mapping onto the three names, and the state it was
                // missing is the one the law asks for — know a reading without
                // drawing it.
                let level = |how: Render| match how {
                    Render::Off => TableLevel::Off,
                    Render::Basic => TableLevel::Basic,
                    Render::Full => TableLevel::Full,
                };
                self.status = say!(
                    "render.is",
                    word(level(self.render)),
                    word(self.table_level),
                    word(level(self.ruby_level()))
                );
                Ok(CommandOutcome::Continue)
            }
            // `:view-hud` sets and reports with the same three sentences: what the
            // level *is* and what it was just changed to are the same fact,
            // and two wordings of it would be two things to keep true.
            Command::SetHud(how) => {
                self.hud = how;
                self.status = hud_says(how);
                Ok(CommandOutcome::Continue)
            }
            Command::ReportHud => {
                self.status = hud_says(self.hud);
                Ok(CommandOutcome::Continue)
            }
            // A measure is only a measure if the rows honour it, so setting
            // one turns wrapping on: `:view-wrap 50` says "write to fifty", and
            // fifty columns of text running off the edge is not that.
            Command::SetDense(on) => {
                self.set_dense(on);
                Ok(CommandOutcome::Continue)
            }
            Command::SetSentences(on) => {
                self.set_sentences(on);
                Ok(CommandOutcome::Continue)
            }
            // The palette lives in the front end — the core does not know a
            // colour exists — so the request is left here and answered there,
            // the same way `:yume-scheme` reaches the input method.
            Command::Theme { name, mood } => {
                self.theme_request = Some((name, mood));
                Ok(CommandOutcome::Continue)
            }
            Command::TableToPipe(delimiter) => {
                self.table_to_pipe(delimiter);
                Ok(CommandOutcome::Continue)
            }
            Command::TableToDelimited(delimiter) => {
                self.table_to_delimited(delimiter);
                Ok(CommandOutcome::Continue)
            }
            Command::SortTable(keys) => {
                self.sort_table(&keys);
                Ok(CommandOutcome::Continue)
            }
            Command::ShowDetail(want) => {
                let want = want.unwrap_or(!self.detail_visible());
                self.show_detail = Some(want);
                self.status = match want {
                    true => say!("ui.detail-panel-on"),
                    false => say!("ui.detail-panel-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetDetailWidth(n) => {
                self.detail_width = Some(n.clamp(12, 80));
                self.show_detail = Some(true);
                self.status = say!("ui.detail-panel-width", self.detail_width.unwrap_or(n));
                Ok(CommandOutcome::Continue)
            }
            Command::Language(verb) => {
                let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
                    self.status = say!("language.save-first");
                    return Ok(CommandOutcome::Continue);
                };
                self.language_run = Some(LanguageRun {
                    verb,
                    language: self.current_buffer().syntax().name().to_string(),
                    path,
                });
                Ok(CommandOutcome::Continue)
            }
            Command::Markdown(bit) => {
                self.write_markdown(bit);
                Ok(CommandOutcome::Continue)
            }
            Command::SetTypewriter(want) => {
                self.typewriter = want.unwrap_or(!self.typewriter);
                self.status = match self.typewriter {
                    true => say!("layout.typewriter-on"),
                    false => say!("layout.typewriter-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetFocus(want) => {
                self.focus = want.unwrap_or(!self.focus);
                self.status = match self.focus {
                    true => say!("layout.focus-on"),
                    false => say!("layout.focus-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetMeter(want) => {
                self.meter = want.unwrap_or(!self.meter);
                self.status = match (self.meter, self.reader.available()) {
                    // 平仄 come out of the 拆分表, and an editor without one
                    // would turn the mode on and draw an empty margin. Say
                    // which of the two it is, rather than letting the writer
                    // conclude their poem has no tones in it.
                    (true, false) => say!("layout.meter-no-readings"),
                    (true, true) => say!("layout.meter-on"),
                    (false, _) => say!("layout.meter-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetNote(want) => {
                self.notes = want.unwrap_or(!self.notes);
                self.status = match (self.notes, self.markup_visible()) {
                    // A note is drawn on the page, and `:render off` is the one
                    // setting that says 「draw nothing the file does not
                    // contain」. Turning notes on under it would leave the
                    // writer waiting for a mark that is never coming.
                    (true, false) => say!("layout.note-no-render"),
                    (true, true) => say!("layout.note-on"),
                    (false, _) => say!("layout.note-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetTableNumbers(on) => {
                self.table_numbers = on;
                self.status = match on {
                    true => say!("table.column-numbers-on"),
                    false => say!("table.column-numbers-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetTableHeader(want) => {
                self.set_table_header(want);
                Ok(CommandOutcome::Continue)
            }
            Command::OpenTableSchema => {
                self.open_schema();
                Ok(CommandOutcome::Continue)
            }
            Command::NewTable { rows, columns } => {
                self.new_table(rows, columns);
                Ok(CommandOutcome::Continue)
            }
            Command::SetTableRules(rules) => {
                if let Some(rules) = rules {
                    self.table_rules = rules;
                }
                self.status = say!("table.column-rules", self.table_rules.name());
                Ok(CommandOutcome::Continue)
            }
            Command::EnterTable => {
                // **`:table` is the door, not a surface.** Typed while the
                // grid had the window it went in again as 畫成表格 — a silent
                // demotion that also threw away `t q`'s way back.
                match self.table.as_ref().map(|v| v.pane) {
                    None => {
                        self.enter_table();
                    }
                    Some(true) => self.status = say!("table.already-the-window"),
                    Some(false) => match self.table_level {
                        TableLevel::Full => self.status = say!("table.already-drawn"),
                        _ => self.status = say!("table.already-operated"),
                    },
                }
                Ok(CommandOutcome::Continue)
            }
            Command::SetTableLevel(level) => {
                self.set_table_level(level);
                Ok(CommandOutcome::Continue)
            }
            Command::ReportIndent => {
                self.status = self.indent_report();
                Ok(CommandOutcome::Continue)
            }
            Command::SetIndentLevel(how) => {
                self.set_indent_level(how);
                self.status = self.indent_report();
                Ok(CommandOutcome::Continue)
            }
            Command::SetRubyLevel(how) => {
                self.set_ruby_level(how);
                self.status = match self.ruby_level() {
                    Render::Off => say!("ruby.level-off"),
                    Render::Basic => say!("ruby.level-basic"),
                    Render::Full => say!("ruby.level-full"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetWheelStep(step) => {
                match step {
                    // Asking is a use of its own: the number is in a config
                    // file the reader may never have written.
                    None => self.status = say!("wheel.is", self.wheel_step),
                    Some(step) => {
                        self.set_wheel_step(step);
                        self.status = say!("wheel.set", self.wheel_step);
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            Command::SetMeasure(measure) => {
                if measure.is_some() {
                    self.set_soft_wrap(true);
                }
                self.set_measure(measure);
                self.refresh_goal_column();
                Ok(CommandOutcome::Continue)
            }
            Command::SetSoftWrap(on) => {
                // **縱書 has nothing to turn off.** A 縱 is broken by the
                // height of the window, and that is not the writer's to set —
                // `:view-wrap 40` does set the 縱 length, in either layout, but
                // 開／關 does not reach it. Taking it anyway and answering
                // 「長段落跑出右邊」 named a right edge this page does not
                // have, and left the reader looking for a change that had not
                // been made.
                if self.layout == Layout::Vertical {
                    self.status = say!("wrap.vertical-has-no-wrap");
                    return Ok(CommandOutcome::Continue);
                }
                self.set_soft_wrap(on);
                self.refresh_goal_column();
                self.status = if on {
                    say!("wrap.on")
                } else {
                    say!("wrap.off")
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetScheme(tag) => {
                self.scheme_request = Some(tag);
                Ok(CommandOutcome::Continue)
            }
            Command::SetChaifen(want) => {
                self.chaifen = want.unwrap_or(!self.chaifen);
                self.chaifen_request = Some(self.chaifen);
                Ok(CommandOutcome::Continue)
            }

        }
    }

    /// The question the editor has stopped to ask, if it has (#295).
    ///
    /// The front end draws it and stops drawing everything that answers keys.
    pub fn query(&self) -> Option<&Query> {
        self.query.as_ref()
    }

    /// Answer the open question. Every key comes here while one stands.
    ///
    /// A key that is not one of the choices **leaves the question standing**
    /// rather than falling through to the manuscript: the one thing a modal
    /// question may never do is let a stray keystroke edit the file behind it.
    /// `Esc` is 「no」 — the same answer the last choice spells out, because a
    /// reader who wants out of a dialog reaches for `Esc` before reading it.
    pub(super) fn answer_query(&mut self, key: Key) {
        let Some(asked) = self.query.take() else { return };
        let answer = match key {
            Key::Esc => 'n',
            Key::Char(c) => c.to_ascii_lowercase(),
            _ => {
                self.query = Some(asked);
                return;
            }
        };
        if !asked.choices.iter().any(|a| a.key == answer) {
            self.query = Some(asked);
            return;
        }
        match asked.what {
            Asking::OversizeWrite { path } => match answer {
                // Yes: the same save, with the gate already answered.
                'y' => {
                    // One write, with the gate already answered. Scoped to this
                    // call: leaving it set would wave through the *next* save
                    // too, and the next save is a different question.
                    self.oversize_answered = true;
                    let _ = self.write_forcing(path.as_deref(), false);
                    self.oversize_answered = false;
                }
                // 檢視區別 **abandons the save**. Nothing is written, and the
                // buffer is left exactly as it was — which is the point: the
                // reader is going to look at what changed and decide again.
                'd' => self.diff_against(None),
                _ => self.status = say!("write.oversize-stopped"),
            },
        }
    }

    /// Whether this `:write` would make the file on disk very much bigger, and
    /// the two sizes if it would (Feature #295).
    ///
    /// **Never on a first save.** A file that is not there yet has no size to
    /// have multiplied, and a writer saving a new chapter is the one person
    /// this must not stop. The buffer is measured in bytes off the rope rather
    /// than by rendering it: the question is about an order of magnitude, and
    /// a line-ending pass would cost a copy of the manuscript to sharpen a
    /// number that is about to be rounded to 「MB」 anyway.
    fn oversize_write(&self, path: Option<&str>) -> Option<(u64, u64)> {
        let target = match path {
            Some(p) => PathBuf::from(p),
            None => self.current_buffer().path()?.to_path_buf(),
        };
        let was = std::fs::metadata(&target).ok()?.len();
        let now = self.current_buffer().rope().len_bytes() as u64;
        let doubled = was > 0 && now / 2 >= was;
        let jumped = now >= was.saturating_add(OVERSIZE_JUMP);
        (doubled && jumped).then_some((was, now))
    }

    /// The question `:write` asks when [`Self::oversize_write`] says to.
    fn oversize_query(&self, path: Option<&str>) -> Option<Query> {
        let (was, now) = self.oversize_write(path)?;
        let times = now as f64 / was as f64;
        Some(Query {
            title: say!("write.oversize-title"),
            body: say!(
                "write.oversize-what",
                self.current_buffer().display_name(),
                human_size(was),
                human_size(now),
                format!("{times:.1}")
            ),
            choices: vec![
                Answer { key: 'y', label: say!("write.oversize-go") },
                Answer { key: 'd', label: say!("write.oversize-look") },
                Answer { key: 'n', label: say!("write.oversize-no") },
            ],
            what: Asking::OversizeWrite { path: path.map(str::to_string) },
        })
    }

    /// Save the active buffer, optionally to a new `path` (save-as).
    pub(super) fn write_current(&mut self, path: Option<&str>) -> Result<Wrote, EditorError> {
        self.write_forcing(path, false)
    }

    /// The same, and `force` writes over a file that changed on disk (`:w!`).
    ///
    /// Returns **what it did**, because the three are different things and the
    /// caller has to be able to tell them apart. It used to return `()` and the
    /// caller sniffed the rendered status line for a Chinese character to find
    /// out — which in English said 「saved ch1.md」 about a chapter that had not
    /// been saved, and in Chinese left a stale 「抄了一份」 standing over a save
    /// that had happened.
    pub(super) fn write_forcing(&mut self, path: Option<&str>, force: bool) -> Result<Wrote, EditorError> {
        // **The gate belongs on the path every write takes**, not on one arm of
        // the command match (#306). It was built on `:write` alone and left
        // there deliberately — `:w!` already says 「over whatever is there」 and
        // the other two were 「one line each」 — but the line that multiplies a
        // manuscript is `t F`, and the key a writer reaches for after it is
        // `:wq`. A bang answers a different question anyway: it says overwrite
        // *this file*, not 「a 17× file is what I meant」.
        if !self.oversize_answered {
            if let Some(ask) = self.oversize_query(path) {
                self.query = Some(ask);
                return Ok(Wrote::Asked);
            }
        }
        let saved: Result<Wrote, EditorError> = match path {
            // `:w path` writes a **copy** and stays here; `:w! path` writes it
            // over whatever is already there. Rebinding this buffer to another
            // name is `:write-as`, which says so — `:w chapter-copy.md` used to
            // rebind silently, and every save after it went to the copy while
            // the chapter itself stayed at the version before.
            Some(p) => {
                let target = PathBuf::from(p);
                match self.buffer_holding(&target) {
                    // Its own file, spelled another way: an ordinary save.
                    Some(which) if which == self.current => {
                        return self.write_forcing(None, force)
                    }
                    // Somebody else's file, and that somebody is holding it in
                    // memory: a copy written here is text they will overwrite
                    // from a buffer that still believes it is clean.
                    Some(which) => {
                        let name = self.buffers[which].display_name();
                        self.status = say!("buffer.already-open-elsewhere", name);
                        return Err(EditorError::Io(std::io::Error::other(
                            self.status.clone(),
                        )));
                    }
                    // A buffer with no name of its own takes this one, as vi
                    // does: there is no manuscript here for the copy to be a
                    // copy *of*, and a scratch buffer that stayed nameless
                    // after `:w 第一章.md` would ask again at the next save.
                    None if self.current_buffer().path().is_none() => self
                        .current_buffer_mut()
                        .save_as(target, force)
                        .map(|()| Wrote::Saved)
                        .map_err(EditorError::Io),
                    None => self
                        .current_buffer()
                        .write_copy(&target, force)
                        .map(|()| Wrote::Copied(target))
                        .map_err(EditorError::Io),
                }
            }
            None => {
                if self.current_buffer().path().is_none() {
                    return Err(EditorError::NoFileName);
                }
                self.current_buffer_mut()
                    .save_forcing(force)
                    .map(|()| Wrote::Saved)
                    .map_err(EditorError::Io)
            }
        };
        // A save that said nothing was a save you could not tell from a save
        // that did not happen — and the manual has been quoting this line as
        // its example of the hint row all along.
        // 寫作進度 is kept from the save, not from the keystroke: what a day
        // holds is what the writer committed to disk that day (Feature #244).
        let word_list = matches!(saved, Ok(Wrote::Saved)) && self.note_word_list_saved();
        if matches!(saved, Ok(Wrote::Saved)) {
            self.note_progress();
        }
        match &saved {
            // A saved word list says so itself, and says how many words are in
            // force now — 「存了 words.txt」 alone would leave the reader
            // wondering whether the weeding took effect.
            Ok(Wrote::Saved) if word_list => {}
            Ok(Wrote::Saved) => {
                self.status = say!("buffer.saved", self.current_buffer().display_name())
            }
            Ok(Wrote::Copied(to)) => self.status = say!("buffer.copied-to", to.display()),
            // Unreachable — the gate returns before anything is written — and
            // spelled out rather than lumped in with `Err` so that adding a
            // second question later cannot make this quietly claim a save.
            Ok(Wrote::Asked) => {}
            Err(_) => {}
        }
        saved
    }
}
