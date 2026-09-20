//! `:check` — the four questions a person cannot answer by eye (#296).
//!
//! Moved out of `editor.rs` whole on 2026-09-08. Five commands that all do the
//! same thing in five subjects: walk the whole document, and hand back a
//! **results buffer** in the shape `gf` already reads — the 拆分表
//! (`:table-check`), the 用詞 groups (#233), the crutch words (`:word-habit`,
//! #242), the 標點 nothing closes (#238), and the 字集 a typesetter will not
//! have (#240).
//!
//! A child module of `editor`, so `Editor`'s private fields and the free
//! functions beside it are as reachable here as they were where this code used
//! to stand.

use super::*;

impl Editor {
    /// Look the whole table over and list what is wrong (`:table-check`).
    ///
    /// Four questions a person asks of a 拆分表 and cannot answer by eye at
    /// 123,380 rows: is any row's name used twice, does every component named
    /// have a row, is any row the wrong width, and is any character outside the
    /// declared code space. The answer is a **results buffer** in the shape
    /// `gf` already reads, because that is the shape every answer in this
    /// editor has.
    pub(super) fn check_table(&mut self) {
        let Some(view) = self.table.as_ref() else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        let separator = view.separator;
        // **The table the cursor is in** (#283). A `.csv` is one table and the
        // file's bounds are its bounds; a document is as many tables as
        // somebody typed, and checking the file meant reading every paragraph
        // in it as a row 「寬度不對」. The schema comes from the same place as
        // the bounds do, so the two can never disagree.
        let (first_line, last_line) = match view.bounds {
            Bounds::WholeFile => (0, motion::last_line(self.current_buffer().rope())),
            _ => match self.prose_region() {
                Some(region) => (region.first, region.last),
                None => {
                    self.status = say!("table.not-in-a-table");
                    return;
                }
            },
        };
        let Some(schema) = self.schema_here().map(|s| s.into_owned()) else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        let key_at = schema.key.as_deref().and_then(|k| schema.index_of(k));
        let jump_from: Vec<usize> = schema
            .link
            .as_ref()
            .map(|j| j.from.iter().filter_map(|n| schema.index_of(n)).collect())
            .unwrap_or_default();
        let want = schema.columns.len();
        let rope = self.current_buffer().rope();
        // The rule row is punctuation, not a row — it is the one line in a
        // Markdown table that is *meant* to hold nothing but dashes.
        let rule = self.table.as_ref().and_then(|v| match v.bounds {
            Bounds::WholeFile => None,
            _ => self.prose_region().and_then(|r| r.rule),
        });
        let last = last_line;
        let first = first_line + usize::from(schema.header);
        let mut found: Vec<String> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut keys: Vec<String> = Vec::new();
        for line in first..=last {
            let text = rope.line(line).to_string();
            let spans = match separator {
                Separator::Pipe => crate::mdtable::cells(&text),
                Separator::Delimiter(d) => crate::table::cells(&text, d),
            };
            if text.trim().is_empty() || Some(line) == rule {
                continue;
            }
            let cell = |i: usize| {
                spans
                    .get(i)
                    .map(|&s| crate::table::cell_text(&text, s))
                    .unwrap_or_default()
            };
            // **Why the count is wrong, when that is the reason** (#391). A
            // record whose quote runs on is two lines to this grid, and both
            // of them then have the wrong number of columns — reporting that
            // twice and calling it a column count sends its writer looking at
            // the commas. `field_runs_on` has always known; until now the only
            // place that said so was the door the file did not get through.
            let runs_on = match separator {
                Separator::Pipe => false,
                Separator::Delimiter(d) => crate::table::field_runs_on(&text, d),
            };
            if runs_on {
                found.push(say!("chaifen.field-runs-on", name, line + 1));
            } else if spans.len() != want {
                found.push(say!(
                    "chaifen.wrong-column-count",
                    name,
                    line + 1,
                    spans.len(),
                    want
                ));
            }
            if let Some(at) = key_at {
                let k = cell(at);
                if !k.is_empty() {
                    if let Some(&was) = seen.get(&k) {
                        found.push(say!(
                            "chaifen.duplicate-row-name",
                            name,
                            line + 1,
                            k,
                            was + 1
                        ));
                    } else {
                        seen.insert(k.clone(), line);
                        keys.push(k);
                    }
                }
            }
        }
        // The components second, because answering them needs every key first.
        let known: std::collections::HashSet<char> = keys
            .iter()
            .filter_map(|k| {
                let mut c = k.chars();
                c.next().filter(|_| c.next().is_none())
            })
            .collect();
        for line in first..=last {
            if Some(line) == rule {
                continue;
            }
            let text = rope.line(line).to_string();
            let spans = match separator {
                Separator::Pipe => crate::mdtable::cells(&text),
                Separator::Delimiter(d) => crate::table::cells(&text, d),
            };
            let mut missing: Vec<char> = Vec::new();
            for &i in &jump_from {
                let Some(&span) = spans.get(i) else { continue };
                for c in crate::table::cell_text(&text, span).chars() {
                    if is_ids_operator(c) || known.contains(&c) || missing.contains(&c) {
                        continue;
                    }
                    missing.push(c);
                }
            }
            if !missing.is_empty() {
                let list: String = missing.iter().collect();
                found.push(say!("chaifen.component-not-found", name, line + 1, list));
            }
            if found.len() >= LISTING_LIMIT {
                break;
            }
        }
        if found.is_empty() {
            // The rule row was walked past, so it is not one of the rows the
            // count reports either.
            let rows = last + 1 - first - usize::from(rule.is_some());
            self.status = say!("table.check-clean", name, rows);
            return;
        }
        found.sort_by_key(|l| {
            l.split(':')
                .nth(1)
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0)
        });
        let n = found.len();
        let mut listing = String::new();
        for line in &found {
            listing.push_str(line);
            listing.push('\n');
        }
        let mut buffer = Buffer::from_text(&listing);
        buffer.name_as(&say!("chaifen.lint-results", name));
        self.listing_root = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        self.add_buffer(buffer);
        self.set_cursor(0);
        self.status = say!("table.check-problems", n);
    }

    /// The reader's own 用字 groups, from the config (#233).
    pub fn set_usage_groups(&mut self, groups: Vec<String>) {
        self.usage_groups = groups;
    }

    /// `:check-usage` — where the manuscript wrote the other spelling (#233).
    ///
    /// **Not spelling, consistency.** 裏 four hundred times and 裡 three is not
    /// three mistakes — both are correct 漢字 — it is one manuscript that has
    /// not settled, and nothing tells the writer: Word checks 病句, Grammarly
    /// is English, and every spell-checker there is tokenizes on spaces and
    /// sees a chapter as one word. So the question is asked of the document
    /// rather than of a dictionary: a group is reported only when both
    /// spellings are written here, and the one written more is the one it
    /// meant.
    ///
    /// The answer is a jumpable listing, the shape `:grep` and `:table-check`
    /// already use — three hundred slips are not a status line, and `gf` on a
    /// row is how a reader goes and fixes one.
    pub(super) fn check_usage(&mut self) {
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        let text = self.current_buffer().rope().to_string();
        let slips = crate::usage::check(&text, &self.usage_groups);
        if slips.is_empty() {
            self.status = say!("check.usage-clean", name);
            return;
        }
        let n = slips.len();
        let mut listing = String::new();
        for slip in slips.iter().take(LISTING_LIMIT) {
            listing.push_str(&say!(
                "check.usage-slip",
                name,
                slip.line + 1,
                slip.written,
                slip.instead,
                slip.written_count,
                slip.instead_count
            ));
            listing.push('\n');
        }
        self.show_listing(listing, say!("check.usage-results", name));
        // The listing stops at `LISTING_LIMIT`; the count must say so, or the
        // status line reports 八百處 over a buffer holding five hundred and the
        // reader believes they have seen them all. `:grep` has always said it.
        self.status = match n > LISTING_LIMIT {
            true => say!("check.usage-too-many", LISTING_LIMIT),
            false => say!("check.usage-found", n),
        };
    }

    /// `:check-names` — a 百科 name written one homophone out (2026-09-20).
    ///
    /// **The other half of [`Self::check_usage`].** That one settles 裏 against
    /// 裡, and it works because both spellings are in a list. A proper name is
    /// in nobody's list — 返塵亭 is this book's — so the one place it came out
    /// 返塵停 is invisible to every checker there is. The book's 百科 *does*
    /// know the name, and the reading table knows that 亭 and 停 are one sound,
    /// which is what tells a typo from a different word (see [`crate::names`]).
    pub(super) fn check_names(&mut self) {
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        let names: Vec<String> = self.wiki.by_name.keys().cloned().collect();
        if names.is_empty() {
            self.status = say!("check.names-no-wiki");
            return;
        }
        // ⚠️ **No reading table, no answer.** Without it this would be
        // 「one character different」, which in Chinese is most of the
        // vocabulary; a page of noise is worse than the silence it replaces.
        if !self.reader.available() {
            self.status = say!("check.names-no-readings");
            return;
        }
        // One lookup per character, not per comparison: the same handful of
        // characters is asked about all the way down a chapter.
        let sounds: std::cell::RefCell<std::collections::HashMap<char, Option<String>>> =
            Default::default();
        let alike = |a: char, b: char| {
            let say = |c: char| {
                sounds
                    .borrow_mut()
                    .entry(c)
                    .or_insert_with(|| {
                        self.reader.read(&c.to_string()).and_then(|r| r.first().cloned())
                    })
                    .clone()
            };
            match (say(a), say(b)) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
        };
        let text = self.current_buffer().rope().to_string();
        let slips = crate::names::check(&text, &names, alike);
        if slips.is_empty() {
            self.status = say!("check.names-clean", name);
            return;
        }
        let n = slips.len();
        let mut listing = String::new();
        for slip in slips.iter().take(LISTING_LIMIT) {
            listing.push_str(&say!(
                "check.names-slip",
                name,
                slip.line + 1,
                slip.written,
                slip.name,
                slip.wrong,
                slip.right
            ));
            listing.push('\n');
        }
        self.show_listing(listing, say!("check.names-results", name));
        self.status = match n > LISTING_LIMIT {
            true => say!("check.names-too-many", LISTING_LIMIT),
            false => say!("check.names-found", n),
        };
    }

    /// `:word-habit` — the words this manuscript leans on (Feature #242).
    ///
    /// **Sorting a word count says 的.** Every manuscript in the language gives
    /// that answer, and a writer learns nothing from it. So the words are
    /// ranked by how much *more* often this chapter says them than ordinary
    /// prose does — 然後 forty-seven times where prose would have said it
    /// eleven — which is the writer's own tic and is invisible from inside the
    /// draft. The arithmetic, and the three things it deliberately stays quiet
    /// about, are in [`crate::words`].
    ///
    /// **It needs a 詞頻表, and says so when it has none.** The fallback
    /// segmenter gives every 漢字 a word of its own and knows no rates; asked
    /// this question it would either say nothing or report every proper noun in
    /// the book. The answer to 「為什麼一個字都沒有」 has to be a sentence, not
    /// an empty listing.
    pub(super) fn habit_words(&mut self) {
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        let text = self.current_buffer().rope().to_string();
        let segment = |line: &str| self.segmenter.segment(line);
        let log_prob = |word: &str| self.segmenter.log_prob(word);
        // 的 is in every table there is, so if the commonest word in the
        // language has no rate then no word does.
        if log_prob("的").is_none() {
            self.status = say!("words.no-table", self.words_in_force());
            return;
        }
        let found = crate::words::habits(&text, &segment, &log_prob);
        if found.is_empty() {
            self.status = say!("words.clean", name);
            return;
        }
        let n = found.len();
        let mut listing = String::new();
        for word in found.iter().take(LISTING_LIMIT) {
            listing.push_str(&say!(
                "words.habit",
                name,
                word.line + 1,
                word.word,
                word.count,
                format!("{:.1}", word.ratio)
            ));
            listing.push('\n');
        }
        self.show_listing(listing, say!("words.results", name));
        self.status = match n > LISTING_LIMIT {
            true => say!("words.too-many", LISTING_LIMIT),
            false => say!("words.found", n),
        };
    }

    /// Put a `檔名:行號: …` listing in a buffer of its own and go to it.
    ///
    /// The shape `:grep`, `:table-check` and both `:check` share: `gf` on a row
    /// is how a reader goes and fixes one, and that needs `listing_root` to be the
    /// directory the file it was run on lives in.
    pub(super) fn show_listing(&mut self, listing: String, name: String) {
        self.show_listing_as(listing, name, None)
    }

    /// The same, with a syntax of its own (#499).
    ///
    /// Only `:diff` uses it: its listing carries `[-走了-]{+來了+}`, which is
    /// markup in exactly one place in the program. The other listings are read
    /// as Markdown, which is what they are — a `:check-punct` report quotes the
    /// manuscript's own lines.
    pub(super) fn show_listing_as(
        &mut self,
        listing: String,
        name: String,
        syntax: Option<crate::syntax::Syntax>,
    ) {
        let mut buffer = Buffer::from_text(&listing);
        buffer.name_as(&name);
        if let Some(syntax) = syntax {
            buffer.set_syntax(syntax);
        }
        self.listing_root = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        self.add_buffer(buffer);
        self.set_cursor(0);
    }

    /// `:check-punct` — the marks a manuscript cannot see (Feature #238).
    ///
    /// **The one that matters is the third.** A half-width comma is ugly and a
    /// `...` is wrong, and both are caught by a careful read. A 「 that never
    /// closes is not: it does not look wrong on the line it is on, and from
    /// there to the end of the chapter every quotation mark means the opposite
    /// of what it says. Nobody proofreading a page finds it, because the page
    /// is fine.
    ///
    /// **Only where it is Chinese.** `3.14`, `1,000`, `README.md` and an
    /// English sentence are full of half-width marks and every one of them is
    /// right; what makes a mark wrong is the 漢字 beside it. See
    /// [`crate::punct`] for the rest of that rule, and for why a 「 left open
    /// at the end of a paragraph may be perfectly correct.
    pub(super) fn check_punct(&mut self) {
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        let text = self.current_buffer().rope().to_string();
        let slips = crate::punct::check(&text);
        if slips.is_empty() {
            self.status = say!("check.punct-clean", name);
            return;
        }
        let n = slips.len();
        let mut listing = String::new();
        for slip in slips.iter().take(LISTING_LIMIT) {
            let at = slip.line + 1;
            listing.push_str(&match slip.kind {
                crate::punct::Kind::HalfWidth => {
                    say!("check.punct-half", name, at, slip.written, slip.wanted)
                }
                crate::punct::Kind::Ellipsis => {
                    say!("check.punct-ellipsis", name, at, slip.written)
                }
                crate::punct::Kind::Unclosed => {
                    say!("check.punct-unclosed", name, at, slip.written, slip.wanted)
                }
                crate::punct::Kind::Unopened => {
                    say!("check.punct-unopened", name, at, slip.written, slip.wanted)
                }
            });
            listing.push('\n');
        }
        self.show_listing(listing, say!("check.punct-results", name));
        self.status = match n > LISTING_LIMIT {
            true => say!("check.punct-too-many", LISTING_LIMIT),
            false => say!("check.punct-found", n),
        };
    }

    /// `:check-charset` — the characters that are in no standard (#240).
    ///
    /// **The failure this exists to prevent happens after the manuscript
    /// leaves.** A character outside 通用規範／臺灣／香港／古籍 reads perfectly
    /// on the screen it was typed on, because the editor has a font with it;
    /// the typesetter's does not, and it comes back as a box, or as a
    /// substituted glyph in a face that does not match, three weeks later and
    /// once per printing. Nothing in the writing tools asks the question, and
    /// the data to answer it — the 字集 column of the 拆分表 — has been sitting
    /// in the editor since `:yume-scheme`.
    ///
    /// **One line per character, not per occurrence.** A 名字 with a rare 字 in
    /// it appears four hundred times and is *one* decision: keep it, or change
    /// it everywhere. Four hundred rows would bury the other three characters
    /// that are the actual finding. So the listing gives each character its
    /// first place, its count, and the Unicode block it lives in — the block
    /// being the part that predicts whether a font will have it.
    pub(super) fn check_charset(&mut self) {
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        if !self.reader.available() {
            self.status = say!("check.charset-no-data");
            return;
        }
        let text = self.current_buffer().rope().to_string();
        // First line, count, block — keyed by the character, with `order`
        // keeping the sequence they were first written in so that the listing
        // reads down the page the way every other results buffer does.
        let mut seen: std::collections::HashMap<char, (usize, usize, String)> =
            std::collections::HashMap::new();
        let mut order: Vec<char> = Vec::new();
        for (line, body) in text.lines().enumerate() {
            for ch in body.chars() {
                if !is_han(ch) || EVERY_FONT_HAS.contains(&ch) {
                    continue;
                }
                if let Some(entry) = seen.get_mut(&ch) {
                    entry.1 += 1;
                    continue;
                }
                let Some(field) = self.reader.charset(ch) else {
                    // The reader answers 「I have no 字集 data」 the same way for
                    // every character; one no is the whole answer.
                    self.status = say!("check.charset-no-data");
                    return;
                };
                let (tags, block) = split_charset(&field);
                if !tags.is_empty() {
                    continue;
                }
                let block = block.to_string();
                order.push(ch);
                seen.insert(ch, (line, 1, block));
            }
        }
        if order.is_empty() {
            self.status = say!("check.charset-clean", name);
            return;
        }
        let n = order.len();
        let mut listing = String::new();
        for ch in order.iter().take(LISTING_LIMIT) {
            let (line, count, block) = &seen[ch];
            listing.push_str(&say!(
                "check.charset-outside",
                name,
                line + 1,
                ch,
                block,
                count
            ));
            listing.push('\n');
        }
        self.show_listing(listing, say!("check.charset-results", name));
        self.status = match n > LISTING_LIMIT {
            true => say!("check.charset-too-many", LISTING_LIMIT),
            false => say!("check.charset-found", n),
        };
    }
}
