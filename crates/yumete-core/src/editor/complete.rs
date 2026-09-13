//! What `[^` and `](#` can be finished with (#418 二).
//!
//! Two references a manuscript keeps needing and neither of which the hand can
//! remember: the tag of a footnote written four hundred lines down, and the
//! anchor of a chapter whose title is 「第十七章　雨夜」. Both answers are
//! already in the file; the editor was making the writer scroll for them.
//!
//! **Nothing is written until Tab.** The panel opens by itself, because a
//! completion nobody knows about is not one, but the buffer is not touched
//! until the writer asks — so a reference typed out in full never has an
//! editor's guess in it. That also keeps every other key exactly what it was:
//! Enter is a line break, Esc leaves Insert, and Tab outside a trigger is
//! still a tab character.

use super::*;

/// How many references the panel offers at once.
///
/// A manuscript with two hundred notes in it would otherwise open a panel
/// taller than the page, and nobody reads past the first screen of a list they
/// are narrowing by typing anyway.
const MOST: usize = 24;

/// How far back from the caret a trigger may be.
const LOOK_BACK: usize = 96;

/// Which reference is being finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refers {
    /// `[^` — a footnote tag.
    Note,
    /// `](#` — a heading in this file.
    Anchor,
}

impl Refers {
    /// The panel's ring, the way a which-key panel is named.
    pub fn title(self) -> String {
        match self {
            Refers::Note => say!("complete.footnote"),
            Refers::Anchor => say!("complete.heading"),
        }
    }
}

/// One thing the trigger can be finished with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// What gets written in place of what has been typed, closing bracket and
    /// all.
    pub text: String,
    /// What it is, in the quiet ink beside it: a note's first words, a
    /// heading's title.
    pub note: Option<String>,
}

/// A trigger standing open at the cursor, and what it offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub refers: Refers,
    /// Where the typed part starts — the range a pick replaces is `at..cursor`.
    pub at: usize,
    /// What has been typed since the trigger.
    pub typed: String,
    pub choices: Vec<Candidate>,
}

/// The pick Tab is walking, kept because the typed text is replaced by each
/// candidate in turn and so can no longer say what was being completed.
#[derive(Debug, Clone)]
pub(super) struct Walking {
    pub(super) refers: Refers,
    pub(super) at: usize,
    /// What the writer typed, restored to the recording when a pick replaces
    /// an earlier pick.
    pub(super) typed: String,
    /// What the last Tab wrote, so the next one replaces it and not the
    /// writer's own text.
    pub(super) wrote: Option<String>,
    pub(super) choices: Vec<Candidate>,
    pub(super) index: usize,
}

impl Editor {
    /// The reference standing open at the cursor, if one is.
    ///
    /// Recomputed rather than remembered: it is a scan back over a handful of
    /// characters, and only when those characters spell a trigger does it read
    /// the file. Remembering it would mean invalidating it on every edit,
    /// which is the same cost paid at a worse moment.
    pub fn reference_offer(&self) -> Option<Offer> {
        if self.mode != Mode::Insert {
            return None;
        }
        if self.syntax() != crate::syntax::Syntax::Markdown {
            return None;
        }
        // A grid's cell is one field wide and Tab already means「next cell」in
        // it; a footnote inside a table is not worth taking that key away.
        if self.table_here() {
            return None;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let start = rope.line_to_char(line);
        // **A bounded look back.** Neither a tag nor an anchor is long, and a
        // novel's paragraph is one line of some thousands of characters that
        // this would otherwise copy on every frame while the caret sits after
        // a `[`.
        let from = start.max(self.cursor.saturating_sub(LOOK_BACK));
        let before: Vec<char> = rope.slice(from..self.cursor).chars().collect();
        let (refers, typed) = trigger(&before)?;
        // A `[^` quoted in a fence is four characters of somebody's example.
        //
        // ⚠️ **Asked after the trigger**, for the reason [`Self::continue_the_list`]
        // asks it last: `block_of` walks from the top of the file and its cache
        // is keyed on the revision, so a walk per keystroke is a walk per
        // keystroke. Down here it is paid for only while a reference stands
        // open at the caret. `:render off` reports every line as prose and so
        // has the same blind spot there as it has for lists.
        if self.block_of(line).is_literal() {
            return None;
        }
        let at = self.cursor - typed.chars().count();
        let choices = match refers {
            Refers::Note => self.note_choices(&typed),
            Refers::Anchor => self.anchor_choices(&typed),
        };
        match choices.is_empty() {
            true => None,
            false => Some(Offer { refers, at, typed, choices }),
        }
    }

    /// Every footnote tag this file already uses, and — when nothing narrows
    /// it away — the next free number.
    ///
    /// The next free number is the one candidate that is *not* in the file,
    /// and it is the common case: `[^` is usually the start of a new note.
    /// It comes last all the same, because a tag already written is a tag with
    /// a note under it, and picking the wrong one of those silently points a
    /// sentence at somebody else's footnote.
    fn note_choices(&self, typed: &str) -> Vec<Candidate> {
        let rope = self.current_buffer().rope();
        let mut tags: Vec<String> = Vec::new();
        let mut bodies: std::collections::HashMap<String, String> = Default::default();
        let mut taken: std::collections::BTreeSet<usize> = Default::default();
        // **One walk, line by line.** Reading the file into a `String` and
        // asking [`Editor::footnote_body`] per tag is a whole-file scan for
        // every candidate, and this runs while somebody is typing.
        for line in 0..rope.len_lines() {
            let text = rope.line(line).to_string();
            for tag in crate::markdown::footnote_tags(&text) {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
            taken.extend(crate::markdown::footnote_numbers(&text));
            // `[^甲]: …` at the head of a line is where that note is written.
            if let Some((tag, body)) = text.trim_start().strip_prefix("[^").and_then(|rest| rest.split_once("]:")) {
                bodies.insert(tag.to_string(), body.trim().to_string());
            }
        }
        let close = self.closes_with(']');
        let mut out: Vec<Candidate> = tags
            .iter()
            .filter(|tag| tag.starts_with(typed))
            .take(MOST)
            .map(|tag| Candidate {
                text: format!("{tag}{close}"),
                note: bodies.get(tag).cloned().map(first_words),
            })
            .collect();
        let fresh = (1..).find(|n| !taken.contains(n)).unwrap_or(1).to_string();
        if fresh.starts_with(typed) {
            out.push(Candidate {
                text: format!("{fresh}{close}"),
                note: Some(say!("complete.a-new-one")),
            });
        }
        out
    }

    /// This file's headings, as the anchor a Markdown reader would give them.
    fn anchor_choices(&self, typed: &str) -> Vec<Candidate> {
        let close = self.closes_with(')');
        self.outline()
            .into_iter()
            .map(|(_, _, title)| (crate::markdown::anchor(&title), title))
            .filter(|(slug, _)| !slug.is_empty() && slug.starts_with(typed))
            .take(MOST)
            .map(|(slug, title)| {
                // The title beside the anchor only when it is telling the
                // reader something: for 「卷一 開端」 the anchor *is* the
                // title with the space taken out, and printing both fills
                // half the panel with the same words twice.
                let bare: String = title.chars().filter(|c| !c.is_whitespace()).collect();
                let same = slug.replace('-', "") == bare.to_lowercase();
                Candidate {
                    text: format!("{slug}{close}"),
                    note: (!same).then_some(title),
                }
            })
            .collect()
    }

    /// The bracket a pick has to close, or nothing when it is already there.
    ///
    /// `[^1]` typed backwards — the closing bracket first, then the tag — is
    /// how a hand that knows Markdown types it, and writing a second one turns
    /// the reference into `[^1]]`.
    fn closes_with(&self, bracket: char) -> String {
        let rope = self.current_buffer().rope();
        match rope.chars_at(self.cursor.min(rope.len_chars())).next() {
            Some(c) if c == bracket => String::new(),
            _ => bracket.to_string(),
        }
    }

    /// Whether the panel is on the page — for the one row that only needs to
    /// know that, without paying for the list.
    ///
    /// **A walk in progress counts.** Once Tab has written `[^7]` the caret is
    /// past a closing bracket and no trigger stands open any more, but Tab is
    /// still live and still the key that changes the pick; a hint that
    /// vanished there would be telling the reader the opposite.
    pub fn reference_open(&self) -> bool {
        self.mode == Mode::Insert
            && (self.reference.is_some() || self.reference_offer().is_some())
    }

    /// What the panel draws: the ring's name, the rows, and which one Tab is
    /// standing on.
    ///
    /// `None` for the highlight until Tab has been pressed once — nothing has
    /// been written yet, so nothing is chosen yet.
    pub fn reference_menu(&self) -> Option<(String, Vec<Candidate>, Option<usize>)> {
        if self.mode != Mode::Insert {
            return None;
        }
        match &self.reference {
            Some(walk) => Some((
                walk.refers.title(),
                walk.choices.clone(),
                Some(walk.index.min(walk.choices.len().saturating_sub(1))),
            )),
            None => {
                let offer = self.reference_offer()?;
                Some((offer.refers.title(), offer.choices, None))
            }
        }
    }

    /// Step Tab through the references, writing each one into the buffer.
    ///
    /// Whether it answered the Tab: it did not when there is no trigger at the
    /// cursor, and then Tab is the tab character it has always been.
    pub(super) fn cycle_reference(&mut self, step: isize) -> bool {
        let walk = match self.reference.take() {
            Some(walk) => {
                let n = walk.choices.len() as isize;
                let index = ((walk.index as isize + step).rem_euclid(n)) as usize;
                Walking { index, ..walk }
            }
            None => {
                let Some(offer) = self.reference_offer() else {
                    return false;
                };
                let index = match step >= 0 {
                    true => 0,
                    false => offer.choices.len() - 1,
                };
                Walking {
                    refers: offer.refers,
                    at: offer.at,
                    typed: offer.typed,
                    wrote: None,
                    choices: offer.choices,
                    index,
                }
            }
        };
        let text = walk.choices[walk.index].text.clone();
        // What this pick replaces: the writer's own text the first time, the
        // previous pick every time after.
        let taking = walk.wrote.clone().unwrap_or_else(|| walk.typed.clone());
        let upto = self.cursor;
        let done =
            self.without_cell_guard(|e| e.current_buffer_mut().replace(walk.at..upto, &text));
        if !self.applied(done) {
            return false;
        }
        self.cursor = walk.at + text.chars().count();
        self.anchor = self.cursor;
        self.refresh_goal_column();
        // `.` replays what the writer meant, which is the reference and not
        // the two letters they typed before Tab finished it. **Only when the
        // recording actually ends with what is being taken back** — Insert may
        // have opened on top of an already-typed `[^4`, and then those
        // characters were never recorded and are somebody else's.
        if self.insert_recording.ends_with(&taking) {
            let keep = self.insert_recording.len() - taking.len();
            self.insert_recording.truncate(keep);
        }
        self.insert_recording.push_str(&text);
        self.reference = Some(Walking { wrote: Some(text), ..walk });
        true
    }
}

/// The trigger the characters before the cursor spell, and what has been typed
/// since it.
///
/// Read backwards from the cursor, so the answer is about where the caret is
/// rather than about the line: `[^1]` further left on the same line is a
/// finished reference and says nothing about this one.
fn trigger(before: &[char]) -> Option<(Refers, String)> {
    let note = ends_at(before, &['[', '^']).map(|end| (end, Refers::Note));
    let anchor = ends_at(before, &[']', '(', '#']).map(|end| (end, Refers::Anchor));
    // The later of the two, because `](#` holds no `[^` and neither holds the
    // other: whichever opened last is the one the caret is inside.
    let (end, refers) = [note, anchor].into_iter().flatten().max_by_key(|&(end, _)| end)?;
    let typed: String = before[end..].iter().collect();
    // Neither a tag nor an anchor holds a space or a bracket. One turning up
    // means the reference behind it was finished and this caret is somewhere
    // else on the same line.
    let broken = typed
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '[' | ']' | '(' | ')' | '#' | '^'));
    match broken {
        true => None,
        false => Some((refers, typed)),
    }
}

/// Where the last `pattern` in `text` ends, if it is in there at all.
fn ends_at(text: &[char], pattern: &[char]) -> Option<usize> {
    (pattern.len()..=text.len()).rev().find(|&end| &text[end - pattern.len()..end] == pattern)
}

/// A note's opening words, for the column beside the tag.
fn first_words(body: String) -> String {
    const KEEP: usize = 24;
    let body = body.trim().replace(['\n', '\r', '\t'], " ");
    match body.chars().count() > KEEP {
        true => format!("{}…", body.chars().take(KEEP).collect::<String>()),
        false => body,
    }
}
