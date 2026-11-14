//! **vim-surround's words for the surround keys** (`Rules::vim_surround`).
//!
//! The keys are the editor's own (`ys`/`ds`/`cs` set the same waits as
//! `ms`/`md`/`mr`); what changes under the vim keymap is what the character
//! after them names, as vim-surround's `surround.txt` lists it:
//!
//! * `(` `[` `{` put a space inside, `)` `]` `}` do not; `b` `B` `r` `a`
//!   are `)` `}` `]` `>`. Taking a pair off by its opener takes the spaces
//!   inside with it.
//! * `t` and `<` ask for a tag, `f` and `F` for a function's name, on the
//!   value line — the answer may be typed with the input method off, and
//!   `.` plays the keys typed there back, so it never asks twice.
//! * `s` puts a space before, and any other letter puts nothing.
//!
//! `q` (any straight quote, `"` when putting one on) and `dsf` are
//! mini.surround's: yumete reads a mini.surround config onto these keys,
//! and vim-surround has nothing under either letter.

use super::*;

/// What `t` or `f` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NameAsked {
    /// `t` and `<`: a tag, maybe with attributes.
    Tag,
    /// `f`, and `F` with a space inside the brackets.
    Function { spaced: bool },
}

/// A name being asked for, and whether it is to change a pair (`cs`: which
/// pair, and the count) or to wrap the selection (`ys`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SurroundAsk {
    name: NameAsked,
    change: Option<(char, usize)>,
}

/// What a character puts around the text.
pub(super) enum Marks {
    Put(String, String),
    Ask(NameAsked),
    /// A letter vim-surround has no meaning for.
    Nothing,
}

/// vim-surround's replacement for `c` (`s:wrap`).
pub(super) fn vim_surround_marks(c: char) -> Marks {
    let put = |open: &str, close: &str| Marks::Put(open.to_string(), close.to_string());
    match c {
        'b' | ')' => put("(", ")"),
        '(' => put("( ", " )"),
        'B' | '}' => put("{", "}"),
        '{' => put("{ ", " }"),
        'r' | ']' => put("[", "]"),
        '[' => put("[ ", " ]"),
        'a' | '>' => put("<", ">"),
        't' | '<' => Marks::Ask(NameAsked::Tag),
        'f' => Marks::Ask(NameAsked::Function { spaced: false }),
        'F' => Marks::Ask(NameAsked::Function { spaced: true }),
        's' => put(" ", ""),
        'q' => put("\"", "\""),
        c if c.is_ascii_alphabetic() => Marks::Nothing,
        c => {
            let (open, close) = pair_of(c).unwrap_or((c, c));
            Marks::Put(open.to_string(), close.to_string())
        }
    }
}

/// How far either side of the caret a tag pair is looked for.
const TAG_REACH: usize = 200_000;

/// A tag that opens or closes: where it starts, one past its `>`, its name.
struct TagMark {
    start: usize,
    end: usize,
    name: String,
    closing: bool,
}

/// The tags in `chars`; a self-closing one (`<br/>`) pairs with nothing and
/// is left out.
fn tag_marks(chars: &[char]) -> Vec<TagMark> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '<' {
            i += 1;
            continue;
        }
        let closing = chars.get(i + 1) == Some(&'/');
        let name_at = i + 1 + usize::from(closing);
        if !chars.get(name_at).is_some_and(|c| c.is_alphabetic()) {
            i += 1;
            continue;
        }
        let mut j = name_at;
        while chars.get(j).is_some_and(|&c| !c.is_whitespace() && !matches!(c, '>' | '/' | '<')) {
            j += 1;
        }
        let mut k = j;
        while k < chars.len() && !matches!(chars[k], '>' | '<') {
            k += 1;
        }
        if chars.get(k) != Some(&'>') {
            i += 1;
            continue;
        }
        if chars[k - 1] != '/' {
            out.push(TagMark { start: i, end: k + 1, name: chars[name_at..j].iter().collect(), closing });
        }
        i = k + 1;
    }
    out
}

/// **The tag pair around `here`**, the `count`th out (vim's `it`/`at`): the
/// caret on either tag is inside its pair. `(outer start, inner start,
/// inner end, outer end)`, half-open ends.
fn tag_around(rope: &Rope, here: usize, count: usize) -> Option<(usize, usize, usize, usize)> {
    let from = here.saturating_sub(TAG_REACH);
    let to = (here + TAG_REACH).min(rope.len_chars());
    let chars: Vec<char> = rope.slice(from..to).chars().collect();
    let here = here - from;
    let marks = tag_marks(&chars);
    let mut open: Vec<usize> = Vec::new();
    let mut pairs = Vec::new();
    for (at, mark) in marks.iter().enumerate() {
        if !mark.closing {
            open.push(at);
            continue;
        }
        // A closer pairs with the last opener of its name; openers left
        // unclosed inside it pair with nothing.
        if let Some(depth) = open.iter().rposition(|&o| marks[o].name == mark.name) {
            let o = &marks[open[depth]];
            if o.start <= here && here < mark.end {
                pairs.push((o.start, o.end, mark.start, mark.end));
            }
            open.truncate(depth);
        }
    }
    pairs.sort_by_key(|&(start, ..)| std::cmp::Reverse(start));
    let (a, b, c, d) = *pairs.get(count.max(1) - 1)?;
    Some((a + from, b + from, c + from, d + from))
}

/// A character of a function's name, as mini.surround reads one.
fn names_a_function(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.')
}

/// **The function call around `here`**, the `count`th out: a name, and the
/// brackets right after it. The caret on the name is in its call.
fn function_call_around(rope: &Rope, here: usize, count: usize) -> Option<(usize, usize, usize, usize)> {
    let len = rope.len_chars();
    let name_before = |open: usize| {
        let mut start = open;
        while start > 0 && names_a_function(rope.char(start - 1)) {
            start -= 1;
        }
        (start < open).then_some(start)
    };
    let mut calls: Vec<(usize, usize, usize)> = Vec::new();
    if here < len && names_a_function(rope.char(here)) {
        let mut open = here;
        while open < len && names_a_function(rope.char(open)) {
            open += 1;
        }
        if open < len && rope.char(open) == '(' {
            if let (Some(close), Some(start)) = (find_forward(rope, open, '(', ')'), name_before(open)) {
                calls.push((start, open, close));
            }
        }
    }
    let mut pair = surrounding(rope, here, '(', ')');
    while let Some((open, close)) = pair {
        if let Some(start) = name_before(open) {
            if calls.last().map(|&(_, o, _)| o) != Some(open) {
                calls.push((start, open, close));
            }
        }
        pair = next_pair_out(rope, open, &[('(', ')')]);
    }
    let (start, open, close) = *calls.get(count.max(1) - 1)?;
    Some((start, open + 1, close, close + 1))
}

/// What a tag the reader is changing carries after its name, for the new
/// one to keep (vim-surround: 「If replacing a tag, its attributes are
/// kept」): ` class="x"` of `<p class="x">`.
fn attributes_of(opener: &str) -> String {
    let Some(rest) = opener.strip_prefix('<') else { return String::new() };
    let after_name = rest.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(rest.len());
    let rest = &rest[after_name..];
    rest[..rest.find('>').unwrap_or(rest.len())].to_string()
}

impl Editor {
    /// Does `c`, after `ys` or `cs`, ask for a name? Then it is asked once,
    /// whatever the number of cursors, and the answer goes to each.
    pub(super) fn surround_asks_a_name(&self, c: char) -> bool {
        self.keymap_rules().vim_surround && matches!(vim_surround_marks(c), Marks::Ask(_))
    }

    /// `ys` then `c` under vim-surround's words.
    pub(super) fn vim_surround_add(&mut self, c: char) {
        match vim_surround_marks(c) {
            Marks::Put(open, close) => self.surround_with(&open, &close),
            Marks::Ask(name) => self.ask_surround_name(SurroundAsk { name, change: None }),
            Marks::Nothing => {}
        }
    }

    /// `cs{from}{to}` under vim-surround's words.
    pub(super) fn vim_surround_change(&mut self, from: char, to: char, count: usize) {
        match vim_surround_marks(to) {
            Marks::Put(open, close) => self.change_surround(from, count, &open, &close),
            Marks::Ask(name) => self.ask_surround_name(SurroundAsk { name, change: Some((from, count)) }),
            Marks::Nothing => {}
        }
    }

    fn ask_surround_name(&mut self, ask: SurroundAsk) {
        if self.refuse_readonly() {
            return;
        }
        let label = match ask.name {
            NameAsked::Tag => say!("edit.surround-ask-tag"),
            NameAsked::Function { .. } => say!("edit.surround-ask-function"),
        };
        self.surround_asking = Some(ask);
        self.open_value_line(&label, "");
    }

    /// Whether the value line is asking for a tag, where `>` ends the answer
    /// (vim-surround maps it to `><CR>` while it asks).
    pub(super) fn asking_for_a_tag(&self) -> bool {
        self.surround_asking.is_some_and(|ask| ask.name == NameAsked::Tag)
    }

    /// The value line closed. `Some(text)` is the answer, `None` Esc.
    /// Whether the line is the surround's to answer.
    pub(super) fn answer_surround_name(&mut self, answer: Option<&str>) -> bool {
        let Some(ask) = self.surround_asking.take() else { return false };
        let Some(text) = answer.filter(|t| !t.is_empty()) else { return true };
        let (open, close) = match ask.name {
            NameAsked::Tag => {
                // A `>` at the end drops the old tag's attributes.
                let keep = !text.ends_with('>');
                let tag = text.trim_end_matches('>');
                if tag.is_empty() {
                    return true;
                }
                let attributes = match (ask.change, keep) {
                    (Some((from, count)), true) => self
                        .pair_to_replace(from, count)
                        .map(|(out, inn, ..)| attributes_of(&self.current_buffer().rope().slice(out..inn).to_string()))
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                match tag.strip_suffix('/') {
                    Some(tag) => (format!("<{tag}{attributes} />"), String::new()),
                    None => {
                        let name = tag.split_whitespace().next().unwrap_or(tag);
                        (format!("<{tag}{attributes}>"), format!("</{name}>"))
                    }
                }
            }
            NameAsked::Function { spaced } => {
                let name = text.strip_suffix('(').unwrap_or(text);
                match spaced {
                    true => (format!("{name}( "), " )".to_string()),
                    false => (format!("{name}("), ")".to_string()),
                }
            }
        };
        match ask.change {
            None => self.edit_each(|e| e.surround_with(&open, &close)),
            Some((from, count)) => self.edit_each(|e| e.change_surround(from, count, &open, &close)),
        }
        true
    }

    /// **Take off the pair `from` names and put `open` `close` in its
    /// place**; the selection keeps its place in the text.
    fn change_surround(&mut self, from: char, count: usize, open: &str, close: &str) {
        if self.refuse_readonly() {
            return;
        }
        let Some((out, inn, inn_end, out_end)) = self.pair_to_replace(from, count) else {
            self.status = say!("edit.no-pair-to-delete");
            return;
        };
        self.snapshot();
        let done = {
            let buffer = self.current_buffer_mut();
            // The closer first, so replacing it cannot shift the opener.
            buffer.replace(inn_end..out_end, close).and_then(|()| buffer.replace(out..inn, open))
        };
        if !self.applied(done) {
            return;
        }
        let changes = [
            (inn_end, out_end - inn_end, close.chars().count()),
            (out, inn - out, open.chars().count()),
        ];
        let (anchor, head) = (self.sel.anchor(), self.sel.head());
        self.sel.set_anchor(multi::through(&changes, anchor));
        self.sel.set_head(multi::through(&changes, head));
        self.clamp_cursor();
    }

    /// **The pair `ds`/`cs` take off, in vim-surround's words**:
    /// `(outer start, inner start, inner end, outer end)`. An opener
    /// (`(` `[` `{` `<`) takes the blanks inside too (`s:dosurround` trims
    /// the kept text for `[\[({<]`).
    pub(super) fn vim_pair_to_take_off(&self, which: char, count: usize) -> Option<(usize, usize, usize, usize)> {
        let rope = self.current_buffer().rope();
        let here = self.sel.head();
        let (out, mut inn, mut inn_end, out_end) = match which {
            'k' => return self.markup_around_the_cursor(),
            't' => tag_around(rope, here, count)?,
            'f' => function_call_around(rope, here, count)?,
            'q' => ['"', '\'', '`']
                .into_iter()
                .filter_map(|q| self.pair_to_change(q, count))
                .max_by_key(|&(open, _)| open)
                .map(|(open, close)| (open, open + 1, close, close + 1))?,
            c => {
                let c = match c {
                    'b' => ')',
                    'B' => '}',
                    'r' => ']',
                    'a' => '>',
                    c => c,
                };
                self.pair_to_change(c, count).map(|(open, close)| (open, open + 1, close, close + 1))?
            }
        };
        if matches!(which, '(' | '[' | '{' | '<') {
            let blank = |c: char| c == ' ' || c == '\t';
            while inn < inn_end && blank(rope.char(inn)) {
                inn += 1;
            }
            while inn_end > inn && blank(rope.char(inn_end - 1)) {
                inn_end -= 1;
            }
        }
        Some((out, inn, inn_end, out_end))
    }

    /// The pair `mr`/`cs` change: one character each side under helix's
    /// words, vim-surround's pair under the vim keymap.
    pub(super) fn pair_to_replace(&self, from: char, count: usize) -> Option<(usize, usize, usize, usize)> {
        match self.keymap_rules().vim_surround {
            true => self.vim_pair_to_take_off(from, count),
            false => self.pair_to_change(from, count).map(|(open, close)| (open, open + 1, close, close + 1)),
        }
    }
}
