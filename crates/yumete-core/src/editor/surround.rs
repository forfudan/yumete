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
//! All of these are one table, [`KEYS`], which the menus after the keys
//! are drawn from too.
//!
//! Where the marks go follows what was named ([`Around`]): around the
//! text for `ys` over characters and Visual mode's `S`; on lines of their
//! own for `ys` over lines, indenting what they wrap for `yS`, `ySS` and
//! `V`'s `S`; around each line's piece for a block's `S`.
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

/// **How the pair goes around what was named**: vim-surround's `s:wrap`
/// reads the register's type and its `special` flag, and so does this.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum Around {
    /// Hugging the text: `ys` over characters, `vS`, and helix's `ms`.
    #[default]
    Text,
    /// Whole lines, the marks on lines of their own: `ys` over lines
    /// (`ysj`, `ysip`); `indent` for `yS` over lines, `ySS` and `VS`,
    /// which indent what they wrap by one level.
    Lines { indent: bool },
    /// `yS` over characters (`ySiw`): the opener where the text began, the
    /// text indented on a line of its own, the closer on the next.
    OwnLines,
    /// `S` on a block: each line's piece, wrapped where it lies; the
    /// block's top-left corner, where the cursor ends, and its size for `.`.
    Block { pieces: Vec<(usize, usize)>, corner: usize, size: block::Size },
}

/// What a key after `ys`, `S`, or `cs`'s old pair puts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Put {
    Pair(&'static str, &'static str),
    Ask(NameAsked),
}

/// What a key after `ds`, or `cs` before its new pair, takes off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Off {
    /// The pair of this closer; `trim` takes the blanks inside too.
    Pair { closer: char, trim: bool },
    Tag,
    Call,
    /// The nearest straight quotes of any kind.
    Quote,
}

/// **vim-surround's keys**, both ways: what each puts on and what it takes
/// off (`s:wrap` and `s:dosurround`). The code reads it and so do the menus
/// after `ys`, `ds` and `cs`, where keys that mean the same share a row.
const KEYS: &[(char, Option<Put>, Option<Off>)] = &[
    ('(', Some(Put::Pair("( ", " )")), Some(Off::Pair { closer: ')', trim: true })),
    (')', Some(Put::Pair("(", ")")), Some(Off::Pair { closer: ')', trim: false })),
    ('b', Some(Put::Pair("(", ")")), Some(Off::Pair { closer: ')', trim: false })),
    ('{', Some(Put::Pair("{ ", " }")), Some(Off::Pair { closer: '}', trim: true })),
    ('}', Some(Put::Pair("{", "}")), Some(Off::Pair { closer: '}', trim: false })),
    ('B', Some(Put::Pair("{", "}")), Some(Off::Pair { closer: '}', trim: false })),
    ('[', Some(Put::Pair("[ ", " ]")), Some(Off::Pair { closer: ']', trim: true })),
    (']', Some(Put::Pair("[", "]")), Some(Off::Pair { closer: ']', trim: false })),
    ('r', Some(Put::Pair("[", "]")), Some(Off::Pair { closer: ']', trim: false })),
    ('<', Some(Put::Ask(NameAsked::Tag)), Some(Off::Pair { closer: '>', trim: true })),
    ('>', Some(Put::Pair("<", ">")), Some(Off::Pair { closer: '>', trim: false })),
    ('a', Some(Put::Pair("<", ">")), Some(Off::Pair { closer: '>', trim: false })),
    ('t', Some(Put::Ask(NameAsked::Tag)), Some(Off::Tag)),
    ('f', Some(Put::Ask(NameAsked::Function { spaced: false })), Some(Off::Call)),
    ('F', Some(Put::Ask(NameAsked::Function { spaced: true })), None),
    ('s', Some(Put::Pair(" ", "")), None),
    ('q', Some(Put::Pair("\"", "\"")), Some(Off::Quote)),
    ('"', Some(Put::Pair("\"", "\"")), Some(Off::Pair { closer: '"', trim: false })),
    ('\'', Some(Put::Pair("'", "'")), Some(Off::Pair { closer: '\'', trim: false })),
    ('`', Some(Put::Pair("`", "`")), Some(Off::Pair { closer: '`', trim: false })),
];

fn put_of(c: char) -> Option<Put> {
    KEYS.iter().find(|&&(key, ..)| key == c).and_then(|&(_, put, _)| put)
}

fn off_of(c: char) -> Option<Off> {
    KEYS.iter().find(|&&(key, ..)| key == c).and_then(|&(_, _, off)| off)
}

/// What a character puts around the text.
pub(super) enum Marks {
    Put(String, String),
    Ask(NameAsked),
    /// A letter vim-surround has no meaning for.
    Nothing,
}

/// vim-surround's replacement for `c` (`s:wrap`): a key of [`KEYS`], or
/// any other character but a letter on both sides.
pub(super) fn vim_surround_marks(c: char) -> Marks {
    match put_of(c) {
        Some(Put::Pair(open, close)) => Marks::Put(open.to_string(), close.to_string()),
        Some(Put::Ask(name)) => Marks::Ask(name),
        None if c.is_ascii_alphabetic() => Marks::Nothing,
        None => {
            let (open, close) = pair_of(c).unwrap_or((c, c));
            Marks::Put(open.to_string(), close.to_string())
        }
    }
}

type Entry = (char, Option<Put>, Option<Off>);

/// Rows of a menu drawn from [`KEYS`]: the keys whose `side` is the same
/// share a row, in the table's order.
fn rows_of<T: PartialEq + Copy>(side: impl Fn(&Entry) -> Option<T>, describe: impl Fn(T) -> String) -> KeyRows {
    let mut grouped: Vec<(String, T)> = Vec::new();
    for entry in KEYS {
        let Some(what) = side(entry) else { continue };
        match grouped.iter_mut().find(|(_, seen)| *seen == what) {
            Some((keys, _)) => {
                keys.push(' ');
                keys.push(entry.0);
            }
            None => grouped.push((entry.0.to_string(), what)),
        }
    }
    grouped.into_iter().map(|(keys, what)| (keys.into(), describe(what))).collect()
}

/// **The menu after `ys`, `S`, or `cs`'s old pair** under vim-surround.
pub(super) fn put_rows() -> KeyRows {
    let mut rows = rows_of(
        |&(_, put, _)| put,
        |put| match put {
            Put::Pair(" ", "") => say!("hint.surround.space-before"),
            Put::Pair(open, close) => format!("{open}…{close}"),
            Put::Ask(NameAsked::Tag) => say!("hint.surround.tag"),
            Put::Ask(NameAsked::Function { spaced: false }) => say!("hint.surround.function", "f(…)"),
            Put::Ask(NameAsked::Function { spaced: true }) => say!("hint.surround.function", "f( … )"),
        },
    );
    rows.push((" ".into(), say!("hint.surround.any-put")));
    rows
}

/// **The menu after `ds` and `cs`** under vim-surround, then the editor's
/// own `m` and `k`.
pub(super) fn off_rows() -> KeyRows {
    let mut rows = rows_of(
        |&(_, _, off)| off,
        |off| match off {
            Off::Pair { closer, trim } => {
                let (open, close) = pair_of(closer).unwrap_or((closer, closer));
                match trim {
                    true => say!("hint.surround.and-blanks", format!("{open}…{close}")),
                    false => format!("{open}…{close}"),
                }
            }
            Off::Tag => say!("hint.surround.tag-pair"),
            Off::Call => say!("hint.surround.call"),
            Off::Quote => say!("hint.surround.any-quote"),
        },
    );
    rows.push(("m".into(), say!("hint.obj.nearest-pair")));
    rows.push(("k".into(), say!("hint.obj.markup")));
    rows.push((" ".into(), say!("hint.pair.any")));
    rows
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

    /// `ys` or `S` then `c` under vim-surround's words.
    pub(super) fn vim_surround_add(&mut self, c: char) {
        match vim_surround_marks(c) {
            Marks::Put(open, close) => self.wrap_as_named(&open, &close, &c.to_string()),
            Marks::Ask(name) => self.ask_surround_name(SurroundAsk { name, change: None }),
            Marks::Nothing => self.leave_visual_after_surround(),
        }
    }

    /// **The wait for a pair to put on** (`ms`, and under the vim keymap
    /// Visual mode's `S`): what is selected is what gets wrapped, as
    /// vim-surround's `vS` takes Visual mode's kind — `V` the lines,
    /// `C-v` each line's piece of the block.
    pub(super) fn begin_surround(&mut self) {
        self.surround_around = match self.extend && self.vim_lines && self.keymap_rules().vim_surround {
            true => Around::Lines { indent: true },
            false => Around::Text,
        };
        self.pending = Pending::Surround;
    }

    /// **Put `open` and `close` around what was named, as it was named**
    /// ([`Around`]). `typed` is what followed `S` on a block, for `.`.
    fn wrap_as_named(&mut self, open: &str, close: &str, typed: &str) {
        match self.surround_around.clone() {
            Around::Text => self.surround_with(open, close),
            Around::Lines { indent } => self.surround_lines(open, close, indent),
            Around::OwnLines => self.surround_on_own_lines(open, close),
            Around::Block { pieces, corner, size } => self.surround_block(&pieces, corner, size, open, close, typed),
        }
        self.leave_visual_after_surround();
    }

    /// vim-surround's put ends Visual mode.
    fn leave_visual_after_surround(&mut self) {
        if self.keymap_rules().vim_surround && self.extend {
            self.extend = false;
            self.vim_lines = false;
            let at = self.sel.head();
            self.sel.set_anchor(at);
        }
    }

    /// **`ysj`, `ySS`, `VS`: the lines between the marks**
    /// (`s:wrap` for a linewise register). The opener and the closer go on
    /// lines of their own at the first line's indent, the blanks inside
    /// them dropped; `indent` (the `special` of `yS` and `VS`) puts every
    /// line wrapped one level deeper, blank ones too, as `s:fixindent`
    /// does. The cursor goes to the opener's line, at its start.
    fn surround_lines(&mut self, open: &str, close: &str, indent: bool) {
        let (start, end) = self.selection();
        let rope = self.current_buffer().rope();
        let first = rope.char_to_line(start.min(rope.len_chars()));
        let last = rope.char_to_line(end.saturating_sub(1).max(start).min(rope.len_chars()));
        let from = rope.line_to_char(first);
        let to = rope.line_to_char((last + 1).min(rope.len_lines()));
        let text = rope.slice(from..to).to_string();
        let ending = self.current_buffer().ending();
        let body = text.strip_suffix('\n').map(|t| t.strip_suffix('\r').unwrap_or(t));
        let broke = body.is_some();
        let body = body.unwrap_or(&text);
        let initspaces: String = body.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let unit = match indent {
            true => self.indent_unit(&initspaces),
            false => String::new(),
        };
        let (open, close) = (open.trim_end_matches(' '), close.trim_start_matches(' '));
        let mut out = format!("{initspaces}{open}{ending}");
        for line in body.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            out.push_str(&format!("{unit}{line}{ending}"));
        }
        out.push_str(&format!("{initspaces}{close}"));
        if broke {
            out.push_str(ending);
        }
        self.snapshot();
        let done = self.current_buffer_mut().replace(from..to, &out);
        if !self.applied(done) {
            return;
        }
        self.sel.set_anchor(from);
        self.sel.set_head(from);
        self.clamp_cursor();
    }

    /// **`ySiw`: the text on a line of its own** (`s:wrap` for a
    /// characterwise register with `special`): the opener stays where the
    /// text began, the text goes on the next line one level deeper than
    /// its line's indent, and the closer on the line after at that indent.
    /// The cursor goes to the opener.
    fn surround_on_own_lines(&mut self, open: &str, close: &str) {
        let (start, end) = self.selection();
        let end = end.max(start);
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(start.min(rope.len_chars()));
        let initspaces: String = rope.line(line).chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let kept = rope.slice(start..end).to_string();
        let ending = self.current_buffer().ending();
        let unit = self.indent_unit(&initspaces);
        let (open, close) = (open.trim_end_matches(' '), close.trim_start_matches(' '));
        let mut out = format!("{open}{ending}");
        for one in format!("{initspaces}{kept}").split('\n') {
            let one = one.strip_suffix('\r').unwrap_or(one);
            out.push_str(&format!("{unit}{one}{ending}"));
        }
        out.push_str(&format!("{initspaces}{close}"));
        self.snapshot();
        let done = self.current_buffer_mut().replace(start..end, &out);
        if !self.applied(done) {
            return;
        }
        self.sel.set_anchor(start);
        self.sel.set_head(start);
        self.clamp_cursor();
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
            None => {
                // What `.` presses again after `S` on a block: the key that
                // asked, the answer, and the Enter a `>` stands in for.
                let key = match ask.name {
                    NameAsked::Tag => 't',
                    NameAsked::Function { spaced: false } => 'f',
                    NameAsked::Function { spaced: true } => 'F',
                };
                let enter = match ask.name == NameAsked::Tag && text.ends_with('>') {
                    true => "",
                    false => "\n",
                };
                let typed = format!("{key}{text}{enter}");
                self.edit_each(|e| e.wrap_as_named(&open, &close, &typed))
            }
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
        let one_each_side = |(open, close): (usize, usize)| (open, open + 1, close, close + 1);
        let (out, mut inn, mut inn_end, out_end) = match (which, off_of(which)) {
            ('k', _) => return self.markup_around_the_cursor(),
            (_, Some(Off::Tag)) => tag_around(rope, here, count)?,
            (_, Some(Off::Call)) => function_call_around(rope, here, count)?,
            (_, Some(Off::Quote)) => ['"', '\'', '`']
                .into_iter()
                .filter_map(|q| self.pair_to_change(q, count))
                .max_by_key(|&(open, _)| open)
                .map(one_each_side)?,
            (_, Some(Off::Pair { closer, .. })) => self.pair_to_change(closer, count).map(one_each_side)?,
            (c, None) => self.pair_to_change(c, count).map(one_each_side)?,
        };
        if matches!(off_of(which), Some(Off::Pair { trim: true, .. })) {
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
