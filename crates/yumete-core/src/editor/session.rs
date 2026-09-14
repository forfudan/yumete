//! Autosave, re-reading, the session, and crash recovery (#43 / #79 / #214).
//!
//! Four subjects that are one subject: what is on disk, and what is in the
//! editor, and how they are kept the same.

use super::*;

impl Editor {
    // ---- Crash recovery (Feature #79) --------------------------------------

    /// Whether a recovery copy is kept beside each document.
    pub fn autosave(&self) -> bool {
        self.autosave
    }

    /// Set whether recovery copies are kept.
    pub fn set_autosave(&mut self, on: bool) {
        self.autosave = on;
    }

    /// How long until a recovery copy is due, if one is (#383).
    ///
    /// `None` means there is nothing a crash could take — autosave off, or
    /// every buffer's copy already holds what the buffer holds — and the event
    /// loop may then wait for a key with no timeout at all. `Some(d)` is how
    /// long the throttle still has to run; `Some(ZERO)` means now.
    ///
    /// **This is the trailing edge the throttle never had.** `autosave_tick`
    /// used to be called only after a keystroke, and it writes at most once
    /// every [`SWAP_INTERVAL`] — so the last few seconds of typing were only
    /// ever written by the *next* key. Stop typing and that key never comes:
    /// the writer pauses to think, the machine dies, and the sentence they
    /// were looking at was never insured. The old note here said 「nothing is
    /// being written while nothing is being typed, so there is nothing to
    /// insure」, which is true of everything except the one window that
    /// matters.
    pub fn autosave_due_in(&self) -> Option<std::time::Duration> {
        if !self.autosave || !self.buffers.iter().any(|b| b.draft_is_stale()) {
            return None;
        }
        let Some(last) = self.last_swap else {
            return Some(std::time::Duration::ZERO);
        };
        Some(self.swap_interval().saturating_sub(last.elapsed()))
    }

    /// How long between rounds: [`SWAP_INTERVAL`], or
    /// [`SWAP_BACKLOG_INTERVAL`] while copies are still owed (#317).
    fn swap_interval(&self) -> std::time::Duration {
        if self.swap_backlog {
            SWAP_BACKLOG_INTERVAL
        } else {
            SWAP_INTERVAL
        }
    }

    /// Write a recovery copy for **every** buffer that has unsaved changes, and
    /// say how many landed.
    ///
    /// The path with no time to ask: a panic. `autosave_tick` is throttled and
    /// asks whether autosave is even on; this asks neither, because the next
    /// thing that happens is the process ending. Failures are counted rather
    /// than reported — there is nowhere left to report them to, and the ones
    /// that did land are what matters.
    pub fn rescue_drafts(&mut self) -> usize {
        self.name_scratch_drafts();
        let mut saved = 0;
        for buffer in &mut self.buffers {
            if buffer.is_modified() && buffer.write_swap().is_ok() {
                saved += 1;
            }
        }
        saved
    }

    /// One round of recovery copies: the buffer being typed into, then as many
    /// of the others as [`SWAP_BUDGET`] buys (#317).
    ///
    /// This runs **in the input thread**, between one keystroke and the next,
    /// so what it costs is what the writer feels. It used to write every
    /// modified buffer every time: a hundred chapters left dirty by a
    /// whole-book `:replace` was a hundred serialisations and two hundred
    /// fsyncs, 1.593 s, and then 1.367 s five seconds later, for ever — the
    /// old loop asked `is_modified`, which stays true after the copy is
    /// written, so the same hundred files were rewritten on every round with
    /// nothing having changed in any of them.
    ///
    /// Two answers, and both are needed. `draft_is_stale` instead of
    /// `is_modified` means a buffer is copied once per edit rather than once
    /// per round, which alone empties the steady state. The budget is for the
    /// round that really does owe a hundred copies: the current buffer is
    /// never deferred — it holds the sentence being typed — and the rest take
    /// turns from [`Self::swap_cursor`], one round to the next, until the
    /// backlog is gone. Rounds come every [`SWAP_BACKLOG_INTERVAL`] while it
    /// lasts, so a hundred chapters are all insured within a few seconds
    /// instead of the eight minutes one-per-`SWAP_INTERVAL` would take.
    pub fn autosave_tick(&mut self) {
        if !self.autosave || self.buffers.is_empty() {
            return;
        }
        self.name_scratch_drafts();
        let now = std::time::Instant::now();
        if let Some(last) = self.last_swap {
            if now.duration_since(last) < self.swap_interval() {
                return;
            }
        }
        self.last_swap = Some(now);
        let mut failed = None;
        if self.buffers[self.current].draft_is_stale() {
            if let Err(err) = self.buffers[self.current].write_swap() {
                failed = Some(format!("{}: {err}", self.buffers[self.current].display_name()));
            }
        }
        let count = self.buffers.len();
        let mut backlog = false;
        let mut wrote_another = false;
        // `from`, not `self.swap_cursor`: the cursor moves as we write, and
        // reading it each time round would make the scan skip.
        let from = self.swap_cursor;
        for step in 0..count {
            let i = (from + step) % count;
            if i == self.current || !self.buffers[i].draft_is_stale() {
                continue;
            }
            // At least one other buffer every round, whatever the clock says:
            // a budget that can refuse them all is a backlog that never
            // drains. The overrun is one buffer's worth, once.
            if wrote_another && now.elapsed() >= SWAP_BUDGET {
                self.swap_cursor = i;
                backlog = true;
                break;
            }
            if let Err(err) = self.buffers[i].write_swap() {
                failed = Some(format!("{}: {err}", self.buffers[i].display_name()));
            }
            wrote_another = true;
            self.swap_cursor = (i + 1) % count;
        }
        self.swap_backlog = backlog;

        // Said once, not on every tick: a directory that cannot be written to
        // will not start being writable, and a status line repeating itself is
        // one the writer stops reading. Silence would be worse — the manual
        // promises a copy is being kept.
        if let Some(what) = failed {
            if !self.swap_warned {
                self.swap_warned = true;
                self.status = say!("recover.no-draft-kept", what);
            }
        } else {
            self.swap_warned = false;
        }
    }

    // ---- Reading the file again (Feature #214) ----------------------------

    /// Re-read the file from disk, throwing away what is in the buffer.
    ///
    /// `force` is the `!`: without it a buffer with unsaved changes is refused,
    /// because re-reading over them is losing them and the writer has to be the
    /// one who says so.
    pub(super) fn reload(&mut self, force: bool) -> Result<(), EditorError> {
        if self.current_buffer().path().is_none() {
            return Err(EditorError::NoFileName);
        }
        if self.current_buffer().is_modified() && !force {
            return Err(EditorError::UnsavedChanges);
        }
        self.reread_now();
        self.status = say!("buffer.re-read", self.current_buffer().display_name());
        Ok(())
    }

    /// Take what is on disk, and put the editor back in step with it.
    ///
    /// Three caches answer questions *about the whole document* and every one
    /// of them is now about a document that is no longer here.
    fn reread_now(&mut self) {
        // A locked buffer is still re-readable: read-only is about **editing**
        // it, and taking a fresh copy of the file is the one thing a reader
        // does want.
        if let Err(err) = self.current_buffer_mut().reread() {
            self.status = say!("reload.failed", err.to_string());
            // **Latched, or it says it every two seconds.** A file that was
            // moved or deleted out from under a `:reload-auto` session
            // answers `changed_underneath` yes for ever, and the failure
            // would stamp over whatever the reader is actually reading.
            self.reload_warned = true;
            return;
        }
        self.clamp_cursor();
        // The one list, not the three caches this used to clear: a re-read is
        // a new document, and `md_cache`, `fold_cache` and `key_index` all
        // described the old one. It also puts the warning latch back.
        self.forget_the_document();
    }

    /// Notice a file that changed underneath, if `:reload-auto on` (Feature
    /// #214).
    ///
    /// Throttled the way [`Editor::autosave_tick`] is, so this is a clock check
    /// on most keys: `changed_underneath` costs a `stat` on the cheap path and
    /// a whole read on the expensive one, and a held-down `j` would pay it per
    /// row.
    pub fn disk_tick(&mut self) {
        if !self.reload_auto {
            return;
        }
        let now = std::time::Instant::now();
        if let Some(last) = self.last_disk_check {
            if now.duration_since(last) < DISK_INTERVAL {
                return;
            }
        }
        self.last_disk_check = Some(now);
        if !self.current_buffer().changed_underneath() {
            // All is well again — so the next divergence is worth saying out
            // loud, even though this one has been said.
            self.reload_warned = false;
            return;
        }
        // Said once already about this file, and nothing has changed since:
        // the writer knows, and the status line is theirs to use.
        if self.reload_warned {
            return;
        }
        // **A dirty buffer is never re-read behind the writer's back.** The
        // whole point of the setting is convenience, and there is no
        // convenience worth an afternoon's typing: this is the one case where
        // it stops and asks.
        if self.current_buffer().is_modified() {
            self.reload_warned = true;
            self.status = say!("autoreload.both-changed");
            return;
        }
        self.reread_now();
        self.status = say!("autoreload.reread", self.current_buffer().display_name());
    }

    /// Say so, on opening a file, when a newer draft is waiting.
    ///
    /// The draft is *not* loaded on its own: silently showing text that is not
    /// what is on disk is how a writer ends up unsure which version they are
    /// reading. `:recover` loads it; `:recover!` throws it away.
    /// Where buffers with no file keep their recovery copies.
    ///
    /// Set by the front end, which is the only part that knows where the data
    /// directory is. Without it a file-less buffer keeps no copy at all, which
    /// is what it did before.
    pub fn keep_drafts_in(&mut self, dir: PathBuf) {
        self.drafts_dir = Some(dir);
    }

    // ---- The session (Feature #43) ----------------------------------------

    /// Where to remember which files are open (`<data>/sessions/<key>.txt`).
    ///
    /// Kept in the data directory rather than in the project, because a
    /// session is a fact about *you* and this afternoon, not about the book —
    /// and because an editor should not leave a file in every directory it is
    /// ever run in.
    pub fn keep_session_in(&mut self, dir: PathBuf, project: &Path) {
        let mut hasher = DefaultHasher::new();
        project.hash(&mut hasher);
        let key = format!("{:016x}", hasher.finish());
        self.session_file = Some(dir.join(format!("{key}.txt")));
    }

    /// Write down which files are open and where the cursor is in each.
    ///
    /// One line per file: `path\tline`. A plain list rather than a format,
    /// because the only thing that reads it is the next hour of this editor,
    /// and a person looking at it should be able to see what it says.
    pub fn save_session(&mut self) {
        let Some(file) = self.session_file.clone() else {
            return;
        };
        let here = self.cursor;
        self.buffers[self.current].save_cursor(here);
        // The file you are in first, then the rest in order — and **capped**.
        // `:replace` opens every file it changes, so a rename across a book
        // leaves 120 buffers open, and a session that remembered all of them
        // would reopen 120 files tomorrow morning.
        let order = std::iter::once(self.current).chain(
            (0..self.buffers.len()).filter(|&i| i != self.current),
        );
        let mut out = String::new();
        for i in order.take(SESSION_FILES) {
            let buffer = &self.buffers[i];
            let Some(path) = buffer.path() else { continue };
            let line = buffer
                .rope()
                .char_to_line(buffer.saved_cursor().min(buffer.rope().len_chars()));
            out.push_str(&format!("{}\t{}\n", path.display(), line + 1));
        }
        if out.is_empty() {
            // Nothing was open, so there is nothing to come back to — and a
            // stale session is worse than none.
            let _ = std::fs::remove_file(&file);
            return;
        }
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&file, out);
    }

    /// Open again what was open last time, each at the line it was left on.
    ///
    /// Only when the editor was started with **no file named**: someone who
    /// said which file they wanted gets that file. Returns how many were
    /// opened, so the front end can say so — restoring five chapters silently
    /// would leave a person wondering what they were looking at.
    pub fn restore_session(&mut self) -> usize {
        let Some(file) = self.session_file.clone() else {
            return 0;
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            return 0;
        };
        let mut opened = 0usize;
        let mut first = None;
        for line in text.lines() {
            let (path, at) = match line.split_once('\t') {
                Some((p, n)) => (PathBuf::from(p), n.parse::<usize>().unwrap_or(1)),
                None => (PathBuf::from(line), 1),
            };
            // A file that has since been moved or deleted is simply not opened:
            // the session is a convenience, and a convenience does not get to
            // put an error on the screen every morning.
            if !path.is_file() || self.open_file(&path).is_err() {
                continue;
            }
            self.move_to_line(at);
            self.buffers[self.current].save_cursor(self.cursor);
            first.get_or_insert(self.current);
            opened += 1;
        }
        if let Some(i) = first {
            self.show_buffer(i);
        }
        self.status = String::new();
        opened
    }

    /// Give every unsaved file-less buffer a name to keep its draft under.
    ///
    /// Named late, and only once there is something to lose: an empty scratch
    /// buffer that is never typed into should leave nothing behind.
    fn name_scratch_drafts(&mut self) {
        let Some(dir) = self.drafts_dir.clone() else {
            return;
        };
        let session = std::process::id();
        for (i, buffer) in self.buffers.iter_mut().enumerate() {
            if buffer.path().is_none() && buffer.is_modified() {
                buffer.keep_drafts_at(dir.join(format!("scratch-{session}-{i}.yumete")));
            }
        }
    }

    /// The drafts left behind by a session that did not end properly.
    ///
    /// Anything in the drafts directory that is not this session's. A draft
    /// belonging to a *live* other session will be listed too — offering it is
    /// harmless, since taking it copies the text into a new buffer and leaves
    /// the file alone.
    pub fn orphan_drafts(&self) -> Vec<PathBuf> {
        let Some(dir) = &self.drafts_dir else {
            return Vec::new();
        };
        let mine = format!("scratch-{}-", std::process::id());
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("scratch-") && !n.starts_with(&mine))
            })
            .collect();
        found.sort();
        found
    }

    /// Open every orphaned draft as a buffer of its own.
    fn take_orphan_drafts(&mut self) -> usize {
        let orphans = self.orphan_drafts();
        let mut taken = 0;
        for path in &orphans {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            if text.trim().is_empty() {
                let _ = std::fs::remove_file(path);
                continue;
            }
            let mut buffer = crate::Buffer::from_text(&text);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            buffer.name_as(&say!("recover.draft-name", name));
            // **It is text that exists nowhere else.** Recovered clean, it had
            // no file, no draft (the line below deletes it), and no dirty flag
            // — so `:q` threw away the crashed session's work without a word,
            // and the autosave never wrote it either.
            buffer.mark_modified();
            self.add_buffer(buffer);
            // The copy is now in a buffer the writer can see and save; leaving
            // the file behind would offer it again on the next launch — and it
            // is written again immediately, because the buffer is modified.
            //
            // ⚠️ **Written again by whom.** That sentence was true only with
            // `autosave` on: `autosave_tick` returns on its first line when it
            // is off, and `[editor] autosave = false` is a documented setting.
            // With it off this deleted the only copy of a crashed session's
            // work and left the text in memory alone — one more crash, or one
            // closed terminal, and yumete had destroyed it itself.
            //
            // So the copy is written *here* before its file goes, and if that
            // write fails the old file stays. A crash between the two costs
            // nothing: the draft is still there and gets offered again.
            // ⚠️ `write_swap` answers `Ok(())` for a buffer with nowhere to
            // put a copy, so「沒有出錯」is not「留下了一份」. The recovered
            // buffer is unnamed, and it is `name_scratch_drafts` that gives an
            // unnamed buffer somewhere to write — so that runs first, and the
            // proof is `recovery_copy()`, not the return value.
            let kept = self.autosave || {
                self.name_scratch_drafts();
                match self.buffers.last_mut() {
                    Some(b) => b.write_swap().is_ok() && b.recovery_copy().is_some(),
                    None => false,
                }
            };
            if kept {
                let _ = std::fs::remove_file(path);
            }
            taken += 1;
        }
        taken
    }

    pub fn announce_recovery(&mut self) {
        // A session that crashed with an unnamed buffer left its work under a
        // name nobody would think to open. Nothing else will ever mention it,
        // so this does.
        let orphans = self.orphan_drafts().len();
        if orphans > 0 {
            self.status = say!("recover.drafts-waiting", orphans);
        }
        let waiting: Vec<String> = self
            .buffers
            .iter()
            .filter(|b| b.recovered_draft().is_some())
            .map(|b| b.display_name())
            .collect();
        if waiting.is_empty() {
            return;
        }
        // The status line is cleared by the next keystroke, so the buffer also
        // wears a `[draft]` tag until the draft is taken or thrown away — the
        // notice has to still be there when the writer looks up.
        self.status = say!("recover.drafts-newer-than-file", listed(&waiting));
    }

    /// Load this buffer's recovery draft, or throw it away (`:recover[!]`).
    pub(super) fn recover(&mut self, discard: bool) -> Result<CommandOutcome, EditorError> {
        let Some(draft) = self.current_buffer().recovered_draft().map(str::to_string) else {
            // No draft for *this file* — but a session that crashed with an
            // unnamed buffer left its work somewhere with no file to open it
            // by, and this is the only command that would ever go looking.
            if discard {
                let orphans = self.orphan_drafts();
                for path in &orphans {
                    let _ = std::fs::remove_file(path);
                }
                self.status = match orphans.len() {
                    0 => say!("recover.no-drafts"),
                    n => say!("recover.drafts-dropped", n),
                };
                return Ok(CommandOutcome::Continue);
            }
            let taken = self.take_orphan_drafts();
            self.status = match taken {
                0 => say!("recover.no-draft-for-this-file"),
                n => say!("recover.drafts-opened", n),
            };
            return Ok(CommandOutcome::Continue);
        };
        if discard {
            self.current_buffer_mut().discard_swap();
            self.status = say!("recover.draft-dropped");
            return Ok(CommandOutcome::Continue);
        }
        // An ordinary, undoable edit: `u` puts the file on disk back, so
        // recovering is a decision the writer can take back.
        //
        // **Which a locked buffer cannot do at all**, and must not pretend to:
        // `adopt_draft` below takes the swap file over, and on quit it is
        // deleted — so a `:recover` that quietly changed nothing would throw
        // away the crashed session's work while saying it had opened it.
        if self.refuse_readonly() {
            return Ok(CommandOutcome::Continue);
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.current_buffer_mut().replace(0..len, &draft);
        if !self.applied(done) {
            return Ok(CommandOutcome::Continue);
        }
        self.current_buffer_mut().adopt_draft();
        self.clamp_cursor();
        self.status = say!("recover.draft-opened");
        Ok(CommandOutcome::Continue)
    }


    // ---- What is on disk and what is here (Feature #211) ------------------

    /// `:diff [檔名]` — what changed, by 詞 (Feature #235).
    ///
    /// **The grain is the point.** Every diff a writer can reach is a line
    /// diff, and a Chinese paragraph is one line: move one 的 in a 五百字 段落
    /// and `git diff` paints the whole paragraph away and back again. The
    /// answer is true and useless, because the one thing being asked — *which
    /// word* — is buried in five hundred characters. So this one runs over the
    /// 詞 the editor's own segmenter finds, the same ones `w` steps over, and
    /// answers with the line as it stands now and `[-走了-]{+來了+}` in it.
    ///
    /// **What it compares against.** With no argument, the file on disk: 「這
    /// 一坐下來我改了什麼」, which is the question with the shortest half-life.
    /// With a path, that file — yesterday's chapter, a copy kept before a
    /// rewrite. Not the recovery copy: [`crate::buffer::Buffer::write_swap`]
    /// overwrites one path with the text as it stands, so it is never a base
    /// to compare against.
    pub(super) fn diff_against(&mut self, other: Option<&str>) {
        let here = self
            .current_buffer()
            .path()
            .map(Path::to_path_buf);
        let path = match other {
            Some(given) => {
                let given = Path::new(given.trim());
                match (given.is_absolute(), here.as_ref().and_then(|p| p.parent())) {
                    (false, Some(dir)) => dir.join(given),
                    _ => given.to_path_buf(),
                }
            }
            None => match here {
                Some(path) => path,
                None => {
                    self.status = say!("diff.no-file");
                    return;
                }
            },
        };
        let old = match std::fs::read_to_string(&path) {
            // Same byte-order mark `Buffer::decode` drops on the way in. Left
            // on, it is a token of its own on line 1 and every file a Windows
            // editor has touched reports a change it does not have.
            Ok(text) => text.strip_prefix('\u{feff}').unwrap_or(&text).to_string(),
            Err(err) => {
                self.status = say!("buffer.cannot-open", path.display(), err.to_string());
                return;
            }
        };
        let new = self.current_buffer().rope().to_string();
        // The editor's segmenter, handed one line at a time — which is how its
        // own cache is keyed, so most of these lines are already answered.
        let segment = |line: &str| self.segmenter.segment(line);
        let (a, b) = (
            crate::diff::tokens(&old, &segment),
            crate::diff::tokens(&new, &segment),
        );
        let Some(ops) = crate::diff::script(&a, &b) else {
            self.status = say!("diff.too-far", path.display());
            return;
        };
        let changes = crate::diff::line_changes(&ops);
        let against = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        if changes.is_empty() {
            self.status = say!("diff.same", against);
            return;
        }
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        let n = changes.len();
        let mut listing = String::new();
        for change in changes.iter().take(LISTING_LIMIT) {
            listing.push_str(&say!("diff.line", name, change.line + 1, change.marked));
            listing.push('\n');
        }
        self.show_listing(listing, say!("diff.results", name, against));
        // The count and the listing have to agree: saying "1,200 lines differ"
        // over a buffer holding 500 of them sends the reader looking for rows
        // that were never written.
        self.status = match n > LISTING_LIMIT {
            true => say!("diff.too-many", LISTING_LIMIT, against),
            false => say!("diff.changed", n, against),
        };
    }
}
