//! Colouring Markdown without hiding it — Feature #96.
//!
//! A renderer's job is to turn `**字**` into a bold 字 and throw the asterisks
//! away. This does the opposite half: the asterisks **stay on the page**,
//! because the file is the manuscript and a writer needs to see what is in it —
//! but the 字 between them is set bold, so a page of prose shows its own shape.
//!
//! Deliberately not a CommonMark parser, and deliberately not `pulldown-cmark`:
//!
//! - **It is per line.** Everything here is redrawn on every keystroke, and a
//!   parse of the whole document per frame is what this editor has already had
//!   to take out of word motion, the segmentation overlay and search. Emphasis
//!   in prose does not straddle a paragraph, so a paragraph is the right unit —
//!   and one paragraph's answer can be cached against its own text.
//! - **CommonMark's emphasis rules are wrong for 漢字.** They are built on the
//!   left/right-flanking of *space-delimited* words; between two 漢字 they
//!   misfire, and it is a long-standing complaint against every implementation.
//!   Here a delimiter is a delimiter.
//! - **It is flat.** No emphasis inside emphasis. On a page of prose the extra
//!   fidelity buys nothing and costs predictability.

/// What a span of a line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `**bold**` — the text between the markers.
    Strong,
    /// `*italic*` or `_italic_`.
    Emphasis,
    /// `` `code` ``.
    Code,
    /// `~~struck~~`.
    Strike,
    /// The text of a heading line, after its hashes.
    Heading,
    /// The visible text of a `[link](target)`.
    Link,
    /// `==marked==` — set on a ground, the way a highlighter pen leaves it.
    Highlight,
    /// A footnote's number, `[^1]`, and the `[^1]:` that opens its text.
    Footnote,
    /// The page named by a `[[wiki]]` reference.
    WikiLink,
    /// `%%a note to myself%%` or `<!-- one -->` — in the manuscript, not in the
    /// book. Set well back, and dropped by `:export`.
    Comment,
    /// Typst code: `#import`, `#let`, a call. Scaffolding, set back — and never
    /// hidden, because unlike `**` it is not *decorating* writing, it is the
    /// instructions that produce it, and a writer needs to see them.
    Code2,
    /// The markup itself — the asterisks, the brackets and the target. Shown
    /// and set back in source mode; hidden in 所見即所得.
    Marker,
    /// `[-什麼走了-]` in a `:diff` listing — the old draft's words.
    ///
    /// Not Markdown; the listing has [`crate::syntax::Syntax::Diff`] and this
    /// is what that syntax produces. It lives in this enum because everything
    /// downstream of a span — hiding the markers, painting the run, measuring
    /// the width — is written once against `Kind` and would have to be written
    /// again for a second one.
    Gone,
    /// `{+什麼來了+}` in a `:diff` listing — the new draft's words.
    Added,
    /// A heading's hashes.
    ///
    /// A [`Marker`](Kind::Marker) that is never hidden: a terminal cannot make
    /// a heading bigger, so the hashes are the only thing that says whether
    /// this is a chapter or a scene, and a writer needs to know which.
    HeadingMark,
    /// A run of code inside a fence, as its own grammar reads it (#420).
    ///
    /// Not markup and never hidden: it is what the file says, in colour.
    Token(crate::code::Token),
}

/// What kind of block a line belongs to.
///
/// Blocks are the part of Markdown that is *not* line-local: a fence opened
/// three paragraphs ago decides whether this line is code, and `:::` runs until
/// it is closed. So they are not parsed per line but scanned in order, by
/// [`BlockScanner`], which is cheap enough to run from the top of the document
/// to the bottom of the page on every frame — it looks at the first few
/// characters of each line and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Block {
    /// Ordinary writing.
    #[default]
    Prose,
    /// `# 第一章`, at that depth.
    Heading(usize),
    /// `> 引文`, and which `:::` container it is inside, if any.
    Quote { inside: Option<Callout> },
    /// A `-`, `*` or `1.` item, and whether it is a task and done.
    Item { task: Option<bool> },
    /// `---` or `***` on its own.
    Rule,
    /// Inside a ``` fence, or the fence line itself — and which `:::` container
    /// it is inside, if any.
    ///
    /// ⚠️ **A block inside a callout is still that block** (#468). A fence in a
    /// `::: details` used to come out as `Container`, which was harmless while
    /// every block wore the same grey band and wrong the moment they stopped:
    /// a fence draws no ground of its own now, so it punched a page-coloured
    /// hole straight through the callout. A quote lost its 綠 the same way.
    Code { inside: Option<Callout> },
    /// The `---`-delimited metadata a file may open with.
    FrontMatter,
    /// Inside a `::: tip` container, or its fence.
    Container(Callout),
    /// A `|`-delimited table row: **which row of its table** — `0` is the
    /// header, `1` the `---` rule under it, and the body counts on from `2` —
    /// and which `:::` container it is inside, if any.
    ///
    /// Counted here rather than by whoever draws, because the scanner is the
    /// one walking the document in order: a renderer that counts backwards
    /// from the line it is drawing does it again for every row on the screen,
    /// and gets it wrong for a table whose head has scrolled off the top.
    Table {
        nth: usize,
        inside: Option<Callout>,
    },
    /// `[^1]: the note itself`.
    FootnoteDef,
    /// A line **after** the one a `<!--` or `%%` opened on, up to the line that
    /// closes it (#288) — and where on that line it closes: `close` is the char
    /// index of the `-->` or `%%`, or `None` when the whole line is the note.
    ///
    /// Not read by [`BlockScanner`], which looks at a line's opening only and
    /// could not see a `<!--` halfway along a paragraph; the editor finds the
    /// comments in the same walk and lays them over its answer, as it does a
    /// merge conflict. The opening line itself stays whatever it was: its own
    /// runs already carry the comment to the end of the line.
    Comment { close: Option<usize> },
    /// A line of a merge conflict (#249) — which side of it, or [`None`] for
    /// one of the four marker lines.
    ///
    /// Not read by [`BlockScanner`], which walks forward and could not know
    /// whether a `<<<<<<<` ever closes; the document's conflicts are assembled
    /// from the markers that walk collects and laid over its answer. Everything
    /// downstream still asks one question of one line, which is the point.
    Conflict(Option<crate::conflict::Side>),
}

/// Which kind of aside a `:::` container is — **GitHub's five** (#483).
///
/// GitHub's alerts and VitePress's containers are two spellings of one set, and
/// a 宇浩 document uses both:
///
/// | 這裏 | GitHub | VitePress |
/// | --- | --- | --- |
/// | [`Callout::Note`] | `[!NOTE]` | `info`, `details` |
/// | [`Callout::Tip`] | `[!TIP]` | `tip` |
/// | [`Callout::Important`] | `[!IMPORTANT]` | — |
/// | [`Callout::Warning`] | `[!WARNING]` | `warning` |
/// | [`Callout::Caution`] | `[!CAUTION]` | `danger` |
///
/// GitHub's own words for the order they run in: 「information users should
/// take into account」, 「optional information to help a user be more
/// successful」, 「crucial information necessary for users to succeed」,
/// 「critical content demanding immediate attention due to potential risks」,
/// 「negative potential consequences of an action」.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Callout {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl Callout {
    /// Parse the word after `:::`.
    ///
    /// **VitePress's own set**, which is what a 宇浩 document is written
    /// against: `info` `tip` `warning` `danger` `details`, plus the names its
    /// GitHub-flavoured alerts use — `note`, `important`, `caution` (#482).
    ///
    /// ⚠️ **`caution` is the red one, not a warning.** GitHub gives
    /// `[!CAUTION]` the red it gives nothing else, and VitePress's `danger` is
    /// the same block; reading `caution` as a warning made the severest of the
    /// five the second-severest here.
    fn parse(word: &str) -> Option<Callout> {
        match word.trim().to_ascii_lowercase().as_str() {
            "note" | "info" | "details" => Some(Callout::Note),
            "tip" => Some(Callout::Tip),
            "important" => Some(Callout::Important),
            "warning" => Some(Callout::Warning),
            "caution" | "danger" | "error" => Some(Callout::Caution),
            _ => None,
        }
    }
}

impl Block {
    /// Whether the line's *characters* are markup at all.
    ///
    /// Inside a fence or a page's metadata they are not: what is written there
    /// is written verbatim, and colouring `**` in a code block — or worse,
    /// taking it off the page — changes what the reader believes the file says.
    pub fn is_literal(self) -> bool {
        // A conflict **marker** is literal; the two sides are ordinary
        // writing. `=======` is otherwise read as a `==highlight==` that opens
        // and never closes, which is how the divider between two halves of a
        // merge came to be drawn as somebody's yellow pen (#249).
        matches!(
            self,
            Block::Code { .. } | Block::FrontMatter | Block::Conflict(None)
        )
    }

    /// Which side of a merge conflict this line is on, if it is in one.
    pub fn conflict_side(self) -> Option<crate::conflict::Side> {
        match self {
            Block::Conflict(side) => side,
            _ => None,
        }
    }
}

/// Walks a document in order, saying which block each line belongs to.
///
/// Fed from the top: a fence, a container or a front-matter block opened
/// earlier changes what a later line means, and there is no way to know that
/// from the line itself.
#[derive(Debug, Default)]
pub struct BlockScanner {
    line: usize,
    /// Which fence opened the code block, so `~~~` does not close a ``` one.
    fence: Option<char>,
    in_front: bool,
    /// Whether the opening `---` is still only a guess at metadata.
    front_unproven: bool,

    /// The containers open, innermost last — `:::` nests, and an inner one must
    /// not close the outer.
    containers: Vec<Callout>,
    /// How many `|` rows have run without a break — what [`Block::Table`]
    /// carries, so the rows can be banded alternately.
    table_row: usize,
}

impl BlockScanner {
    /// A scanner at the top of a document.
    pub fn new() -> BlockScanner {
        BlockScanner::default()
    }

    /// Take the next line and say what it is.
    ///
    /// `prefix` is the line's opening — the first [`PREFIX`] characters is
    /// enough for every decision here — and `len` is how long the whole line
    /// is, which only the rule needs. Taking a prefix rather than the line is
    /// what keeps this from copying every long paragraph of a novel on every
    /// frame.
    pub fn feed(&mut self, prefix: &str, len: usize) -> Block {
        let at = self.line;
        self.line += 1;
        // ⚠️ **The table counter is reset by default, and only the table arm
        // puts it back** (#467). It used to be cleared on two of the eleven
        // paths that end a table — so a second table separated from the first
        // by a heading, a `:::`, a fence, a quote or a list item went on
        // counting, and its header was drawn as a body row with the banding
        // inverted. Clearing here and restoring there cannot be forgotten by
        // whoever adds the twelfth path.
        let table_row = std::mem::take(&mut self.table_row);
        let text = prefix.trim_end_matches(['\n', '\r']);
        let trimmed = text.trim_start();
        let whole = len <= PREFIX;

        // Front matter only counts at the very top, which is what keeps a `---`
        // between two paragraphs a rule rather than the start of metadata.
        // Front matter is metadata, and metadata is `key: value`. The line
        // after the opening `---` is what settles it: a manuscript that opens
        // with a scene break would otherwise have its whole first paragraph
        // swallowed and set back as furniture. (Line 0 keeps the label either
        // way — a rule is drawn the same as metadata, so nothing is lost.)
        if self.in_front {
            let unproven = std::mem::take(&mut self.front_unproven);
            if whole && trimmed == "---" {
                self.in_front = false;
                return Block::FrontMatter;
            }
            if unproven && !trimmed.is_empty() && !is_metadata(trimmed) {
                self.in_front = false;
                // …and fall through: this line is writing like any other.
            } else {
                return Block::FrontMatter;
            }
        }
        // Front matter is metadata, and metadata has `key: value` in it. A
        // manuscript that opens with a `---` scene break would otherwise have
        // its whole first paragraph swallowed and set back as furniture.
        if at == 0 && whole && trimmed == "---" {
            self.in_front = true;
            self.front_unproven = true;
            return Block::FrontMatter;
        }

        // A code fence swallows everything, markup included — and only the
        // fence that opened it can close it, so ``` inside a ~~~ block is code
        // like everything else in there.
        let fence = trimmed
            .starts_with("```")
            .then_some('`')
            .or_else(|| trimmed.starts_with("~~~").then_some('~'));
        let inside = self.containers.last().copied();
        match (self.fence, fence) {
            (None, Some(opened)) => {
                self.fence = Some(opened);
                return Block::Code { inside };
            }
            (Some(open), Some(closing)) if open == closing => {
                self.fence = None;
                return Block::Code { inside };
            }
            (Some(_), _) => return Block::Code { inside },
            (None, None) => {}
        }

        // `:::` with a name opens a container; `:::` alone closes the
        // innermost one. They nest, so an inner `::: tip` must not end the
        // `::: warning` it sits in.
        if let Some(rest) = trimmed.strip_prefix(":::") {
            let named = rest.split_whitespace().next().unwrap_or("");
            if named.is_empty() {
                let kind = self.containers.pop();
                return Block::Container(kind.unwrap_or(Callout::Note));
            }
            let kind = Callout::parse(named).unwrap_or(Callout::Note);
            self.containers.push(kind);
            return Block::Container(kind);
        }
        // **A table inside a `:::` is still a table** (#463). Everything else
        // in a container is the container's — a heading in one is not a
        // heading — but a table is a *shape*, and it has a ground of its own
        // that says which row is which. Losing it left a 表格 in a `::: details`
        // with the callout's flat wash and no banding at all.
        //
        // Before the container arm and not after it, because that arm swallows
        // every line: which callout it is in rides along, so whoever draws can
        // lay the one over the other.
        if trimmed.starts_with('|') && trimmed.len() > 1 {
            self.table_row = table_row + 1;
            return Block::Table {
                nth: table_row,
                inside,
            };
        }
        // A quotation inside a callout is a quotation — same reason as the
        // fence above and the table before it.
        if trimmed.starts_with('>') {
            return Block::Quote { inside };
        }
        if let Some(kind) = inside {
            return Block::Container(kind);
        }

        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
        if hashes > 0 && hashes <= 6 && matches!(trimmed.chars().nth(hashes), Some(' ') | None) {
            return Block::Heading(hashes);
        }
        if trimmed.starts_with('>') {
            return Block::Quote { inside: None };
        }
        if whole && is_rule(trimmed) {
            return Block::Rule;
        }
        if trimmed.starts_with("[^") && trimmed.contains("]:") {
            return Block::FootnoteDef;
        }
        if let Some(rest) = item_body(trimmed) {
            let task = rest
                .strip_prefix('[')
                .and_then(|r| r.get(..1).zip(r.get(1..2)))
                .and_then(|(mark, close)| (close == "]").then_some(mark != " "));
            return Block::Item { task };
        }
        Block::Prose
    }
}

/// How much of a line the block scan looks at.
///
/// Every decision here is about a line's opening. Reading the whole line meant
/// copying every paragraph of a novel on every frame, which on a 600 KB
/// manuscript cost nine milliseconds a keystroke.
pub const PREFIX: usize = 64;

/// Whether a line looks like `key: value`, which is what metadata is made of.
fn is_metadata(text: &str) -> bool {
    match text.split_once(':') {
        Some((key, _)) => {
            !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        }
        None => false,
    }
}

/// Whether the line is `---`, `***` or `___` on its own — a scene break.
///
/// One pass that stops at the first character that settles it, rather than two
/// passes over the line: a paragraph opening with `- ` used to be counted twice
/// from end to end before being ruled out.
fn is_rule(text: &str) -> bool {
    let mut chars = text.chars().filter(|c| !c.is_whitespace());
    let Some(first) = chars.next() else {
        return false;
    };
    if !matches!(first, '-' | '*' | '_') {
        return false;
    }
    let mut seen = 1;
    for c in chars {
        if c != first {
            return false;
        }
        seen += 1;
    }
    seen >= 3
}

/// The text after a list marker, if the line opens a list item.
fn item_body(text: &str) -> Option<&str> {
    if let Some(rest) = text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")) {
        return Some(rest);
    }
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && text[digits..].starts_with(". ") {
        return Some(&text[digits + 2..]);
    }
    None
}

/// What a list line opens with, so Enter can carry it down (#418).
///
/// `Editor::continue_the_list` is the only caller; it lives here because what
/// counts as a marker is a fact about Markdown, not about the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    /// How many characters the indent and the marker take together — where the
    /// item's own writing starts. An Enter that falls *inside* the marker is
    /// splitting it, not continuing it, and is left alone.
    pub width: usize,
    /// What the line below opens with: the same indent, the same marker, and
    /// an ordered list's number stepped on by one.
    pub next: String,
    /// Nothing but the marker on the line — the Enter that ends the list.
    pub empty: bool,
}

/// The marker `line` opens a list item with, if it opens one.
///
/// Five shapes, which are the five a manuscript has: `- `, `* `, `+ `, `1. `
/// (or `1) `), `> `, and `- [ ] `; a quote carries whatever list is inside it
/// down as well. A task's box comes down **unticked** — the next thing to do
/// is not already done.
///
/// **The space is required**, and that is the whole guard against guessing:
/// `-` alone is a dash whose second character has not been typed yet, and an
/// editor that turns the next Enter into a list is the kind people switch off.
/// It costs nothing, because every marker this hands back ends in one.
pub fn opening(line: &str) -> Option<Opening> {
    let line = line.trim_end_matches(['\n', '\r']);
    // `- - -` is a scene break drawn with dashes, not an item whose writing is
    // a dash. `is_rule` settles it, as it does for the block scan above.
    if is_rule(line) {
        return None;
    }
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[indent..];
    // A `|` row is a table's, and an Enter in one is splitting a row — the
    // question [`crate::mdtable::is_row`] answers, asked before the markers so
    // that a row whose first cell opens with a dash is never read as an item.
    if rest.starts_with('|') {
        return None;
    }
    let (marker, body) = match rest.starts_with('>') {
        // A quote nests, so the whole run of `>` is its marker, written back
        // with one space after it however it was spaced.
        true => {
            let run = rest.find(|c| c != '>' && c != ' ').unwrap_or(rest.len());
            let (mark, body) = rest.split_at(run);
            let quote = format!("{} ", mark.trim_end());
            match marker(body) {
                Some((inner, body)) => (format!("{quote}{inner}"), body),
                None => (quote, body),
            }
        }
        false => marker(rest)?,
    };
    Some(Opening {
        width: line.chars().count() - body.chars().count(),
        next: format!("{}{marker}", &line[..indent]),
        empty: body.trim().is_empty(),
    })
}

/// A bullet or a number and what it leaves: the marker the next line opens
/// with, and the item's own writing.
fn marker(rest: &str) -> Option<(String, &str)> {
    if let Some((bullet, after)) = bullet(rest) {
        return Some(match task_box(after) {
            Some(rest) => (format!("{bullet} [ ] "), rest),
            None => (format!("{bullet} "), after),
        });
    }
    let (number, punctuation, after) = numbered(rest)?;
    Some((format!("{number}{punctuation} "), after))
}

/// The bullet character and the rest of the line after the space it ends with.
fn bullet(rest: &str) -> Option<(char, &str)> {
    ["- ", "* ", "+ "].iter().find_map(|mark| {
        rest.strip_prefix(mark)
            .map(|after| (mark.as_bytes()[0] as char, after))
    })
}

/// The `[ ] ` of a task item, and what it leaves. `[x] ` is a task too — the
/// next one down opens unticked either way.
fn task_box(after: &str) -> Option<&str> {
    let inside = after.strip_prefix('[')?;
    let mark = inside.chars().next()?;
    if !matches!(mark, ' ' | 'x' | 'X') || inside.chars().nth(1) != Some(']') {
        return None;
    }
    inside[2..].strip_prefix(' ')
}

/// An ordered item's number, the `.` or `)` after it, and the rest of the line.
///
/// The number handed back is **the next one**. What is written above it is not
/// touched: renumbering a list is a command somebody runs on purpose, not
/// something Enter does to the paragraph behind the cursor.
fn numbered(rest: &str) -> Option<(u64, char, &str)> {
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    // Nine digits is a list nobody is writing, and it is what keeps the `+ 1`
    // inside a `u64` with nothing to say about overflow.
    if digits == 0 || digits > 9 {
        return None;
    }
    let punctuation = rest[digits..].chars().next()?;
    if !matches!(punctuation, '.' | ')') {
        return None;
    }
    let after = rest[digits + 1..].strip_prefix(' ')?;
    Some((rest[..digits].parse::<u64>().ok()? + 1, punctuation, after))
}

/// The markup to take off the page in 所見即所得 mode, as char ranges.
///
/// Everything a construct is made of *except* the writing inside it — but never
/// the construct the cursor is in, which is shown whole so the writer can edit
/// it. That one rule is what keeps the rest coherent: **the cursor is never
/// inside text that is not on the screen**, so `d`, `x` and every motion act on
/// exactly what can be seen.
///
/// A heading's hashes are never hidden. A terminal cannot make a heading
/// bigger, so they are the only thing that says whether this is a chapter or a
/// scene, and that is the writer's own structure.
pub fn hidden(spans: &[Span], selected: Option<(usize, usize)>) -> Vec<(usize, usize)> {
    // Every construct the selection touches is shown whole — not only the one
    // the cursor is in. The anchor is a place in the text too, and a highlight
    // that covered fewer characters than `d` would take is the screen lying
    // about what an edit does.
    let open: Vec<usize> = match selected {
        Some((from, to)) => {
            // A selection covers `from..to`; a bare cursor covers the one
            // grapheme it stands on. Its far edge is *exclusive* — a selection
            // ending where a construct begins has not reached it — while its
            // near edge is inclusive, so standing just past a construct keeps
            // it open and the line does not flicker as the cursor leaves.
            let reach = to.max(from + 1);
            spans
                .iter()
                .filter(|s| reach > s.start && from <= s.end)
                .map(|s| s.construct)
                .collect()
        }
        None => Vec::new(),
    };
    spans
        .iter()
        .filter(|s| s.kind == Kind::Marker && !open.contains(&s.construct))
        .map(|s| (s.start, s.end))
        .collect()
}

/// A run of one line, in char indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    /// Which construct this run belongs to — `**`, its text and the closing
    /// `**` share one. It is what lets the markup of the construct the cursor
    /// is in be shown while every other construct's stays hidden.
    pub construct: usize,
}

/// The marked-up runs of `line`, in order and non-overlapping.
///
/// Anything not covered is ordinary prose and gets no span.
pub fn spans(line: &str) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();

    // A heading is the whole line, so it is decided before anything else and
    // the rest of the line is still scanned for emphasis inside the title.
    let mut from = 0;
    let hashes = chars.iter().take_while(|&&c| c == '#').count();
    if hashes > 0 && hashes <= 6 && matches!(chars.get(hashes), Some(' ') | None) {
        out.push(Span {
            start: 0,
            end: hashes,
            kind: Kind::HeadingMark,
            construct: 0,
        });
        from = hashes;
        // The title itself, under whatever emphasis it also carries.
        if from < chars.len() {
            out.push(Span {
                start: from,
                end: chars.len(),
                kind: Kind::Heading,
                construct: 0,
            });
        }
    }
    let mut construct = 1usize;

    let mut at = from;
    while at < chars.len() {
        // A comment is the writer talking to themselves — everything in it is
        // theirs, markup included — so it is taken before anything else.
        if let Some((open, close, end)) = comment(&chars, at) {
            mark(&mut out, at, at + open, Kind::Marker, construct);
            mark(&mut out, at + open, end - close, Kind::Comment, construct);
            mark(&mut out, end - close, end, Kind::Marker, construct);
            at = end;
            construct += 1;
            continue;
        }
        // Code next: inside a code span nothing else is markup.
        if chars[at] == '`' {
            if let Some(close) = find(&chars, at + 1, |c| c == '`') {
                mark(&mut out, at, at + 1, Kind::Marker, construct);
                mark(&mut out, at + 1, close, Kind::Code, construct);
                mark(&mut out, close, close + 1, Kind::Marker, construct);
                at = close + 1;
                construct += 1;
                continue;
            }
        }
        if let Some(len) = fence(&chars, at) {
            let kind = match (chars[at], len) {
                ('*', 2) | ('_', 2) => Kind::Strong,
                ('~', 2) => Kind::Strike,
                ('=', 2) => Kind::Highlight,
                _ => Kind::Emphasis,
            };
            if let Some(close) = closing(&chars, at + len, chars[at], len) {
                mark(&mut out, at, at + len, Kind::Marker, construct);
                mark(&mut out, at + len, close, kind, construct);
                mark(&mut out, close, close + len, Kind::Marker, construct);
                at = close + len;
                construct += 1;
                continue;
            }
        }
        // `[[第三章]]`, `[[第三章|那一夜]]`, `[[第三章#雪]]` — a reference to
        // somewhere else in the same manuscript.
        if chars[at] == '[' && chars.get(at + 1) == Some(&'[') {
            if let Some(close) = run(&chars, at + 2, "]]") {
                // What is shown is the alias if there is one, else the target.
                let body = at + 2..close;
                let shown = chars[body.clone()]
                    .iter()
                    .position(|&c| c == '|')
                    .map(|i| at + 2 + i + 1..close)
                    .unwrap_or(body);
                mark(&mut out, at, shown.start, Kind::Marker, construct);
                mark(&mut out, shown.start, shown.end, Kind::WikiLink, construct);
                mark(&mut out, shown.end, close + 2, Kind::Marker, construct);
                at = close + 2;
                construct += 1;
                continue;
            }
        }
        // `[^1]`, and the `[^1]:` that opens the note itself.
        if chars[at] == '[' && chars.get(at + 1) == Some(&'^') {
            if let Some(close) = find(&chars, at + 2, |c| c == ']') {
                let end = close + 1 + usize::from(chars.get(close + 1) == Some(&':'));
                mark(&mut out, at, end, Kind::Footnote, construct);
                at = end;
                construct += 1;
                continue;
            }
        }
        // `[text](target)` — the text is what the reader reads.
        if chars[at] == '[' {
            if let Some(close) = find(&chars, at + 1, |c| c == ']') {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = find(&chars, close + 2, |c| c == ')') {
                        mark(&mut out, at, at + 1, Kind::Marker, construct);
                        mark(&mut out, at + 1, close, Kind::Link, construct);
                        mark(&mut out, close, end + 1, Kind::Marker, construct);
                        at = end + 1;
                        construct += 1;
                        continue;
                    }
                }
            }
        }
        at += 1;
    }
    out
}

/// Where a link points, and how it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// What was written as the destination — a URL, a path, or a page name.
    /// Empty when the link points inside this same file (`[雪](#雪)`).
    pub target: String,
    /// The `#雪` half, without the `#`.
    pub anchor: Option<String>,
    /// `[[第三章]]`, which names a page in this manuscript rather than a place
    /// on the machine, and so is looked for by name and not by path.
    pub wiki: bool,
}

/// The link the character at `at` stands in, destination and all.
///
/// [`spans`] says where a link's *text* is, because that is what has to be
/// drawn. This says where the link *goes* — the half 所見即所得 hides — and it
/// answers anywhere inside the whole construct, brackets and hidden target
/// included: a reader who cannot see the target cannot be asked to stand on it.
pub fn link_at(line: &str, at: usize) -> Option<Link> {
    let all = spans(line);
    // Which construct the cursor is in, and only if that construct is a link:
    // the emphasis two words earlier shares the line, not the destination.
    // Backwards, because a heading's span covers its whole line and the link
    // inside it is marked afterwards — later spans win, as they do when the
    // line is drawn.
    let here = all.iter().rev().find(|s| (s.start..s.end).contains(&at))?;
    let construct = all
        .iter()
        .any(|s| s.construct == here.construct && matches!(s.kind, Kind::Link | Kind::WikiLink))
        .then_some(here.construct)?;
    let mine = || all.iter().filter(|s| s.construct == construct);
    let start = mine().map(|s| s.start).min()?;
    let end = mine().map(|s| s.end).max()?;
    let chars: Vec<char> = line.chars().collect();
    let whole: String = chars.get(start..end)?.iter().collect();

    let (written, wiki) = match whole.strip_prefix("[[").and_then(|b| b.strip_suffix("]]")) {
        // `[[第三章|那一夜]]` — the alias is what the reader reads, so the
        // destination is the half before the bar.
        Some(body) => (body.split('|').next().unwrap_or(body).to_string(), true),
        None => {
            let shut = whole.rfind("](")?;
            let inside = whole.get(shut + 2..whole.len().checked_sub(1)?)?.trim();
            // `[甲](url "說明")` — the title is for a reader, not for whatever
            // opens the link. `<…>` is how a destination with a space in it is
            // written, and unwrapping it is the only way that one works.
            let bare = match inside.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
                Some(angled) => angled.to_string(),
                None => inside.split_whitespace().next().unwrap_or("").to_string(),
            };
            (bare, false)
        }
    };
    // A `#` inside a URL opens its fragment, which is the same thing one line
    // down and not this editor's business — but splitting it off costs nothing
    // and an anchor nobody uses is dropped, not obeyed.
    let (target, anchor) = match written.split_once('#') {
        Some((before, after)) => (before.to_string(), Some(after.to_string())),
        None => (written, None),
    };
    Some(Link {
        target,
        anchor: anchor.filter(|a| !a.is_empty()),
        wiki,
    })
}

/// How many delimiter characters start at `at`, or `None` when none do.
///
/// An underscore only opens where a word does not: `snake_case` is a name, not
/// an emphasis, and in a manuscript full of file names that matters more than
/// being able to italicise with `_`.
fn fence(chars: &[char], at: usize) -> Option<usize> {
    let c = chars[at];
    if !matches!(c, '*' | '_' | '~' | '=') {
        return None;
    }
    let len = chars[at..].iter().take_while(|&&x| x == c).count().min(2);
    // `~` and `=` only ever come in pairs: a lone one is a dash or an equals.
    if matches!(c, '~' | '=') && len < 2 {
        return None;
    }
    if c == '_' {
        let before = at.checked_sub(1).and_then(|i| chars.get(i));
        if before.is_some_and(|c| c.is_alphanumeric()) {
            return None;
        }
    }
    // A delimiter with nothing after it opens nothing.
    (at + len < chars.len()).then_some(len)
}

/// Where the run of `len` `delimiter`s closing the one opened at `from` begins.
///
/// The content may not be empty and may not start with a space: `** ` in the
/// middle of a sentence is two asterisks, not the start of a bold run.
fn closing(chars: &[char], from: usize, delimiter: char, len: usize) -> Option<usize> {
    if chars.get(from) == Some(&' ') {
        return None;
    }
    let mut at = from;
    while at + len <= chars.len() {
        if chars[at..at + len].iter().all(|&c| c == delimiter)
            && chars.get(at.wrapping_sub(1)) != Some(&' ')
            && at > from
        {
            return Some(at);
        }
        at += 1;
    }
    None
}

/// A comment opening at `at`: the lengths of its two markers and where it ends.
///
/// `%%…%%` is Obsidian's, `<!-- … -->` is HTML's, and a manuscript uses both
/// for the same thing — a note that is not part of the book. An unclosed one
/// runs to the end of the line, because a half-typed note is still a note.
fn comment(chars: &[char], at: usize) -> Option<(usize, usize, usize)> {
    for (open, close) in [("%%", "%%"), ("<!--", "-->")] {
        let open: Vec<char> = open.chars().collect();
        if chars[at..].starts_with(&open) {
            let end = run(chars, at + open.len(), close)
                .map(|i| i + close.chars().count())
                .unwrap_or(chars.len());
            let closed = end < chars.len() || run(chars, at + open.len(), close).is_some();
            return Some((
                open.len(),
                if closed { close.chars().count() } else { 0 },
                end,
            ));
        }
    }
    None
}

/// The closer of a `<!--` or `%%` that `line` opens and does not close, if it
/// does (#288).
///
/// Read the way [`spans`] reads a line — a comment is taken before a code span
/// at the same place, and a code span hides a `<!--` inside it — so a line this
/// says is left open is a line whose runs carry a comment to its end.
pub fn comment_left_open(line: &str) -> Option<&'static str> {
    let chars: Vec<char> = line.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        if let Some((open, close, end)) = comment(&chars, at) {
            if close == 0 {
                return Some(if chars[at] == '%' { "%%" } else { "-->" });
            }
            at = end.max(at + open);
            continue;
        }
        if chars[at] == '`' {
            if let Some(close) = find(&chars, at + 1, |c| c == '`') {
                at = close + 1;
                continue;
            }
        }
        at += 1;
    }
    None
}

/// Where on `line` a comment left open above closes: the char index of
/// `closer`, if the line holds one (#288).
pub fn comment_closes(line: &str, closer: &str) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    run(&chars, 0, closer)
}

/// `text` with every `<!-- -->` and `%% %%` taken out — across lines too — for a
/// file handed to somebody else (#288). A line that was nothing but a note goes
/// with it, so a note between two paragraphs does not become a third, empty
/// one. Fences keep everything: a `<!--` in a code block is code.
pub fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut open: Option<&'static str> = None;
    let mut fence: Option<char> = None;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let ending = &line[body.len()..];
        let trimmed = body.trim_start();
        if open.is_none() {
            let mark = ['`', '~']
                .into_iter()
                .find(|&c| trimmed.starts_with(&c.to_string().repeat(3)));
            match (fence, mark) {
                (None, Some(m)) => fence = Some(m),
                (Some(f), Some(m)) if f == m => fence = None,
                _ => {}
            }
            if fence.is_some() || mark.is_some() {
                out.push_str(line);
                continue;
            }
        }
        let chars: Vec<char> = body.chars().collect();
        let mut kept = String::new();
        let mut at = 0;
        if let Some(closer) = open {
            match run(&chars, 0, closer) {
                Some(i) => {
                    at = i + closer.chars().count();
                    open = None;
                }
                None => at = chars.len(),
            }
        }
        while at < chars.len() {
            if let Some((_, close, end)) = comment(&chars, at) {
                if close == 0 {
                    open = Some(if chars[at] == '%' { "%%" } else { "-->" });
                }
                at = end;
                continue;
            }
            if chars[at] == '`' {
                if let Some(close) = find(&chars, at + 1, |c| c == '`') {
                    kept.extend(&chars[at..=close]);
                    at = close + 1;
                    continue;
                }
            }
            kept.push(chars[at]);
            at += 1;
        }
        if kept.trim().is_empty() && !body.trim().is_empty() {
            continue;
        }
        out.push_str(&kept);
        out.push_str(ending);
    }
    out
}

/// Where `text` next occurs at or after `from`.
fn run(chars: &[char], from: usize, text: &str) -> Option<usize> {
    let want: Vec<char> = text.chars().collect();
    (from..chars.len().saturating_sub(want.len() - 1)).find(|&i| chars[i..].starts_with(&want))
}

/// The first index at or after `from` whose character satisfies `f`.
fn find(chars: &[char], from: usize, f: impl Fn(char) -> bool) -> Option<usize> {
    (from..chars.len()).find(|&i| f(chars[i]))
}

/// Push a span, dropping empty ones.
fn mark(out: &mut Vec<Span>, start: usize, end: usize, kind: Kind, construct: usize) {
    if end > start {
        out.push(Span {
            start,
            end,
            kind,
            construct,
        });
    }
}

/// Every footnote tag `text` already uses, references and notes alike, in the
/// order they first appear and each one once (#418).
///
/// A tag is not always a number: this manual's own notes are `[^418]` but a
/// manuscript's are as often `[^舊註]`, and completion has to offer what is in
/// the file rather than what the numbering command would have written.
pub fn footnote_tags(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 2 < chars.len() {
        if chars[i] == '[' && chars[i + 1] == '^' {
            let mut j = i + 2;
            while j < chars.len() && !matches!(chars[j], ']' | '[' | '^') && !chars[j].is_whitespace() {
                j += 1;
            }
            if j > i + 2 && chars.get(j) == Some(&']') {
                let tag: String = chars[i + 2..j].iter().collect();
                if !out.contains(&tag) {
                    out.push(tag);
                }
                i = j;
            }
        }
        i += 1;
    }
    out
}

/// The anchor a Markdown reader gives a heading — what `](#…)` has to name.
///
/// The rule every renderer of consequence follows: lower case, punctuation
/// dropped, runs of space turned into one hyphen. 漢字 are kept as they are,
/// which is why this cannot simply be an ASCII slug: 「第十七章　雨夜」 is
/// `第十七章雨夜` and a writer typing that out by hand gets it wrong once in
/// three.
pub fn anchor(title: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in title.trim().chars() {
        if c.is_whitespace() {
            gap = !out.is_empty();
            continue;
        }
        if !(c.is_alphanumeric() || c == '-' || c == '_') {
            continue;
        }
        if gap {
            out.push('-');
            gap = false;
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// Every footnote number already used in `text`, references and notes alike.
///
/// So that a new one can be *the next free number* rather than one more than
/// the last, which after a deletion points at somebody else's note.
pub fn footnote_numbers(text: &str) -> Vec<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < chars.len() {
        if chars[i] == '[' && chars[i + 1] == '^' {
            let mut j = i + 2;
            let mut n = 0usize;
            let mut digits = 0;
            while j < chars.len() && chars[j].is_ascii_digit() {
                n = n.saturating_mul(10).saturating_add(chars[j] as usize - '0' as usize);
                j += 1;
                digits += 1;
            }
            if digits > 0 && chars.get(j) == Some(&']') {
                out.push(n);
                i = j;
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every path that ends a table resets its row counter (#467).
    ///
    /// The counter used to be cleared on two of the eleven returns that end a
    /// table, so a second table separated from the first by anything other
    /// than a blank line went on counting: its header came out as a body row
    /// and the banding was inverted for the rest of the table. No file in the
    /// repository triggered it, which is exactly why it wanted a test.
    #[test]
    fn a_second_table_starts_counting_from_its_own_header() {
        let nths = |between: &str| -> Vec<usize> {
            let mut scan = BlockScanner::new();
            let mut out = Vec::new();
            let doc = format!("| 甲 | 乙 |\n| --- | --- |\n{between}\n| 丙 | 丁 |\n| 戊 | 己 |");
            for line in doc.lines() {
                if let Block::Table { nth, .. } = scan.feed(line, line.len()) {
                    out.push(nth);
                }
            }
            out
        };
        // ⚠️ A lone ``` **opens** a fence and swallows what follows, so the
        // fence case is a fence: opened and closed.
        for between in ["", "## 標題", ":::", "```\n```", "> 注", "- 注", "---", "[^1]: 註"] {
            assert_eq!(
                nths(between),
                vec![0, 1, 0, 1],
                "a {between:?} between two tables must end the first"
            );
        }
        // …and a `|` row that is *not* a break does not restart it.
        let mut scan = BlockScanner::new();
        let doc = "| 甲 |\n| --- |\n| 丙 |\n| 丁 |";
        let got: Vec<usize> = doc
            .lines()
            .filter_map(|l| match scan.feed(l, l.len()) {
                Block::Table { nth, .. } => Some(nth),
                _ => None,
            })
            .collect();
        assert_eq!(got, vec![0, 1, 2, 3]);
    }

    /// A quote, a fence and a table inside a `:::` are still themselves (#468).
    ///
    /// Everything inside a container used to come back as `Container`, which
    /// was harmless while every block wore the same grey band and wrong the
    /// moment they stopped: a fence draws no ground of its own now, so it
    /// punched a page-coloured hole through the callout, and a quote lost its
    /// colour entirely.
    #[test]
    fn a_block_inside_a_callout_is_still_that_block() {
        let mut scan = BlockScanner::new();
        let doc = "::: note\n> 引\n```\nx\n```\n| 甲 |\n| --- |\n散文\n:::";
        let got: Vec<Block> = doc.lines().map(|l| scan.feed(l, l.len())).collect();
        assert!(matches!(got[1], Block::Quote { inside: Some(Callout::Note) }), "{:?}", got[1]);
        assert!(matches!(got[2], Block::Code { inside: Some(Callout::Note) }), "{:?}", got[2]);
        assert!(matches!(got[4], Block::Code { inside: Some(Callout::Note) }), "{:?}", got[4]);
        assert!(
            matches!(got[5], Block::Table { nth: 0, inside: Some(Callout::Note) }),
            "{:?}",
            got[5]
        );
        // The prose between them is the callout's, and so is the closing line.
        assert!(matches!(got[7], Block::Container(Callout::Note)));
        assert!(matches!(got[8], Block::Container(Callout::Note)));
        // Outside one, the same three carry no callout.
        let mut scan = BlockScanner::new();
        for line in ["> 引", "```", "```", "| 甲 |"] {
            match scan.feed(line, line.len()) {
                Block::Quote { inside } | Block::Code { inside } | Block::Table { inside, .. } => {
                    assert_eq!(inside, None, "{line:?}")
                }
                other => panic!("{line:?} came back as {other:?}"),
            }
        }
    }

    /// The kind covering each character, for reading a line's shape at a glance.
    /// Later spans win, which is how a heading's title also shows its emphasis.
    fn shape(line: &str) -> String {
        let n = line.chars().count();
        let mut out = vec![' '; n];
        for span in spans(line) {
            let mark = match span.kind {
                Kind::Strong => 'B',
                Kind::Emphasis => 'I',
                Kind::Code => 'C',
                Kind::Strike => 'S',
                Kind::Heading => 'H',
                Kind::Link => 'L',
                Kind::Highlight => 'M',
                Kind::Footnote => 'F',
                Kind::WikiLink => 'W',
                Kind::Comment => '%',
                Kind::Code2 => '#',
                Kind::Marker | Kind::HeadingMark => '.',
                // Neither `spans` makes these — they are `diff::spans`'.
                Kind::Gone => '-',
                Kind::Added => '+',
                // Only a fence's grammar makes these (`code::highlight`).
                Kind::Token(_) => '~',
            };
            for slot in out.iter_mut().take(span.end.min(n)).skip(span.start) {
                *slot = mark;
            }
        }
        out.into_iter().collect()
    }

    #[test]
    fn the_markers_stay_and_the_text_between_them_is_set() {
        assert_eq!(shape("那**年**天"), " ..B.. ");
        assert_eq!(shape("那*年*天"), " .I. ");
        assert_eq!(shape("那`碼`天"), " .C. ");
        assert_eq!(shape("那~~年~~天"), " ..S.. ");
    }

    #[test]
    fn a_heading_is_the_line_and_its_hashes_are_markup() {
        assert_eq!(shape("## 第一章"), "..HHHH");
        // And emphasis inside the title still shows.
        assert_eq!(shape("# **甲**"), ".H..B..");
        // Six is the deepest; seven hashes is just a line of hashes.
        assert_eq!(shape("####### 甲"), " ".repeat(9));
        // A run of hashes with no title is an empty heading — markup, and
        // nothing else. It contributes nothing to the outline either.
        assert_eq!(shape("###"), "...");
    }

    #[test]
    fn emphasis_between_two_漢字_works_here_even_though_commonmark_argues() {
        // The whole reason this is not `pulldown-cmark`.
        assert_eq!(shape("中**文**中"), " ..B.. ");
        assert_eq!(shape("「**甲**」"), " ..B.. ");
    }

    #[test]
    fn an_underscore_inside_a_word_is_part_of_the_word() {
        assert_eq!(shape("snake_case_name"), " ".repeat(15));
        // At a word boundary it still emphasises.
        assert_eq!(shape("那 _年_ 天"), "  .I.  ");
    }

    #[test]
    fn an_unclosed_or_empty_delimiter_is_just_a_character() {
        assert_eq!(shape("那**年"), " ".repeat(4));
        assert_eq!(shape("那 ** 年"), " ".repeat(6));
        assert_eq!(shape("2 * 3 * 4"), " ".repeat(9));
    }

    #[test]
    fn code_masks_the_markup_inside_it() {
        assert_eq!(shape("`**not bold**`"), ".CCCCCCCCCCCC.");
    }

    #[test]
    fn a_link_shows_its_text_and_sets_its_target_back() {
        assert_eq!(shape("見[附錄](a.md)"), " .LL.......");
    }

    #[test]
    fn a_link_says_where_it_goes_from_anywhere_inside_it() {
        let line = "見[附錄](a.md)";
        // The text, the brackets, and the target 所見即所得 hides — all of it
        // is the link, because standing on the half you cannot see is not
        // something a reader can be asked to do.
        for at in 1..line.chars().count() {
            let link = link_at(line, at).unwrap_or_else(|| panic!("nothing at {at}"));
            assert_eq!(link.target, "a.md");
            assert!(!link.wiki);
        }
        // And the 見 before it is prose.
        assert_eq!(link_at(line, 0), None);
    }

    #[test]
    fn what_a_link_names_is_read_off_the_way_it_was_written() {
        let target = |line: &str| link_at(line, 3).map(|l| (l.target, l.anchor, l.wiki));
        // A bar means an alias, and the destination is the other half.
        assert_eq!(
            target("見[[第三章|那一夜]]"),
            Some(("第三章".into(), None, true))
        );
        // A place *within* a page, and a place within this one.
        assert_eq!(
            target("見[[第三章#雪]]"),
            Some(("第三章".into(), Some("雪".into()), true))
        );
        assert_eq!(target("見[雪](#雪)"), Some((String::new(), Some("雪".into()), false)));
        // A title is written for a reader, not for whatever opens the link;
        // and angle brackets are how a destination with a space in it is
        // written, which is the only way that one works at all.
        assert_eq!(
            target("見[附錄](a.md \"說明\")"),
            Some(("a.md".into(), None, false))
        );
        assert_eq!(
            target("見[附錄](<第 三 章.md>)"),
            Some(("第 三 章.md".into(), None, false))
        );
        // A footnote is not a link, and neither is the emphasis beside one.
        assert_eq!(target("見[^1]。"), None);
        assert_eq!(target("那**年**天"), None);
    }

    #[test]
    fn a_link_inside_a_heading_is_still_a_link() {
        // The heading's span covers the whole line, so whichever span is
        // consulted last has to be the one that decides.
        assert_eq!(
            link_at("# 見[附錄](a.md)", 5).map(|l| l.target),
            Some("a.md".into())
        );
    }

    #[test]
    fn the_extended_syntax_a_manuscript_actually_uses() {
        // A highlighter pen.
        assert_eq!(shape("那==年==天"), " ..M.. ");
        // A footnote's number, and the line that answers it.
        assert_eq!(shape("見[^1]。"), " FFFF ");
        assert_eq!(shape("[^1]: 出自《詩》"), "FFFFF      ");
        // A reference to somewhere else in the same manuscript, and the alias
        // that says what to call it here.
        assert_eq!(shape("見[[第三章]]"), " ..WWW..");
        assert_eq!(shape("見[[第三章|那一夜]]"), " ......WWW..");
        // A note to oneself: not part of the book, and not part of the markup
        // inside it either.
        assert_eq!(shape("寫到這裏 %%**這句再想想**%%"), "     ..%%%%%%%%%..");
        assert_eq!(shape("甲<!-- 待查 -->乙"), " ....%%%%... ");
    }

    #[test]
    fn an_unclosed_comment_still_reads_as_one() {
        // A half-typed note is still a note; treating it as prose would set the
        // rest of the line back to ordinary weight mid-word.
        assert_eq!(shape("寫到這裏 %%再想想"), "     ..%%%");
    }

    /// The blocks of a whole document, as one letter each.
    fn walk(text: &str) -> String {
        let mut scanner = BlockScanner::new();
        text.lines()
            .map(|line| match scanner.feed(line, line.chars().count()) {
                Block::Prose => '.',
                Block::Heading(n) => char::from_digit(n as u32, 10).unwrap_or('#'),
                Block::Quote { .. } => '>',
                Block::Item { task: None } => '-',
                Block::Item { task: Some(false) } => 'o',
                Block::Item { task: Some(true) } => 'x',
                Block::Rule => '_',
                Block::Code { .. } => '`',
                Block::FrontMatter => 'y',
                Block::Container(_) => ':',
                Block::Table { .. } => '|',
                Block::FootnoteDef => 'F',
                Block::Comment { .. } => '%',
                // The scanner never says this — a conflict is laid over its
                // answer by the editor, which is the only thing that can know
                // whether a `<<<<<<<` ever closes.
                Block::Conflict(_) => '!',
            })
            .collect()
    }

    #[test]
    fn blocks_are_read_in_order_because_they_are_not_line_local() {
        // A fence three paragraphs up decides what this line is.
        assert_eq!(walk("那年\n```\n**not bold**\n```\n冬天"), ".```.");
        // `:::` runs until it is closed.
        assert_eq!(walk("::: warning 小心\n這一段\n:::\n之後"), ":::.");
        // Front matter only at the very top; a `---` between paragraphs is a
        // scene break, not the start of metadata.
        assert_eq!(walk("---\ntitle: 甲\n---\n那年\n---\n冬天"), "yyy._.");
        // …and a manuscript that *opens* with a scene break keeps its first
        // paragraph, rather than having it swallowed as metadata.
        assert_eq!(walk("---\n那年冬天\n---\n又一年"), "y._.");
    }

    #[test]
    fn containers_nest_and_only_the_fence_that_opened_a_block_closes_it() {
        // An inner `::: tip` must not end the `::: warning` it sits in, or the
        // text inside both loses its ground and the text after both gains one.
        assert_eq!(
            walk("::: warning 小心\n甲\n::: tip\n乙\n:::\n丙\n:::\n丁"),
            ":::::::."
        );
        // `~~~` does not close a ``` block: everything in it is code.
        assert_eq!(walk("```\n甲\n~~~\n乙\n```\n丙"), "`````.");
    }

    #[test]
    fn the_blocks_prose_is_made_of() {
        assert_eq!(walk("# 第一章\n### 三"), "13");
        assert_eq!(walk("> 昨夜星辰\n> 昨夜風"), ">>");
        assert_eq!(walk("- 阿寧\n1. 第一場\n- [ ] 待寫\n- [x] 寫完"), "--ox");
        assert_eq!(walk("| 甲 | 乙 |\n| -- | -- |"), "||");
        assert_eq!(walk("[^1]: 出自《詩》"), "F");
        // A rule is a scene break; three of anything on its own line.
        assert_eq!(walk("***\n___\n- - -"), "___");
    }

    /// What a line looks like with its markup taken off, the cursor at `at`.
    fn rendered(line: &str, at: Option<usize>) -> String {
        let hide = hidden(&spans(line), at.map(|i| (i, i)));
        line.chars()
            .enumerate()
            .filter(|(i, _)| !hide.iter().any(|&(a, b)| *i >= a && *i < b))
            .map(|(_, c)| c)
            .collect()
    }

    #[test]
    fn the_markup_comes_off_except_where_the_cursor_is() {
        let line = "那**年**冬**天**";
        // Nowhere near it: all of it comes off.
        assert_eq!(rendered(line, None), "那年冬天");
        // In the first construct: that one is whole, the other still off.
        assert_eq!(rendered(line, Some(4)), "那**年**冬天");
        // Approaching its markup from either side opens it too — which is what
        // keeps the cursor from ever being inside text that is not on screen.
        assert_eq!(rendered(line, Some(1)), "那**年**冬天");
        assert_eq!(rendered(line, Some(6)), "那**年**冬天");
        // And the second construct opens on its own.
        assert_eq!(rendered(line, Some(9)), "那年冬**天**");
    }

    #[test]
    fn everything_a_selection_touches_is_shown_whole() {
        let line = "那**年**冬**天**";
        let hide = |from, to| {
            let h = hidden(&spans(line), Some((from, to)));
            line.chars()
                .enumerate()
                .filter(|(i, _)| !h.iter().any(|&(a, b)| *i >= a && *i < b))
                .map(|(_, c)| c)
                .collect::<String>()
        };
        // A selection running across both constructs shows both — a highlight
        // that covered fewer characters than `d` takes would be the screen
        // lying about what an edit does.
        assert_eq!(hide(3, 10), "那**年**冬**天**");
        // One that reaches only the first shows only the first.
        assert_eq!(hide(0, 4), "那**年**冬天");
        // And one that stops exactly where a construct begins has not reached
        // it: `to` is the far edge, and it is exclusive.
        assert_eq!(hide(0, 1), "那年冬天");
    }

    #[test]
    fn a_headings_hashes_stay_because_nothing_else_says_the_level() {
        // A terminal cannot make a heading bigger; the hashes are the level.
        assert_eq!(rendered("## 第一章", None), "## 第一章");
        assert_eq!(rendered("### **甲**", None), "### 甲");
    }

    #[test]
    fn every_kind_of_markup_comes_off() {
        assert_eq!(rendered("那`碼`天", None), "那碼天");
        assert_eq!(rendered("那==年==天", None), "那年天");
        assert_eq!(rendered("見[附錄](a.md)", None), "見附錄");
        assert_eq!(rendered("見[[第三章|那一夜]]", None), "見那一夜");
        assert_eq!(rendered("寫到這裏 %%再想想%%", None), "寫到這裏 再想想");
        // The note's own text stays: a note you cannot see is a note you will
        // not act on. Only its fence comes off.
        assert_eq!(rendered("甲<!-- 待查 -->乙", None), "甲 待查 乙");
        // A footnote's number is what the reader reads, so it stays whole.
        assert_eq!(rendered("見[^1]。", None), "見[^1]。");
    }

    #[test]
    fn ordinary_prose_carries_no_spans() {
        assert!(spans("那年冬天，雪下得早。").is_empty());
        assert!(spans("").is_empty());
    }
}

/// Typst's inline markup, in the shape the rest of this module works in.
///
/// A separate scan rather than a flag on the Markdown one, because the two
/// languages disagree about the same characters: `*粗*` is bold in Typst and
/// nothing in Markdown, `#` opens code in Typst and a heading in Markdown, and
/// `_斜_` is emphasis in Typst wherever it appears. Sharing one scanner would
/// mean a flag at every branch, which is two parsers wearing one coat.
pub mod typst {
    use super::{Block, Kind, Span};

    /// The marked-up runs of one line of Typst.
    pub fn spans(line: &str) -> Vec<Span> {
        let chars: Vec<char> = line.chars().collect();
        let mut out = Vec::new();
        let mut construct = 1usize;
        let mut at = 0usize;

        // A heading is `=` through `======`, and the rest of the line is it.
        let equals = chars.iter().take_while(|&&c| c == '=').count();
        if equals > 0 && equals <= 6 && matches!(chars.get(equals), Some(' ') | None) {
            push(&mut out, 0, equals, Kind::HeadingMark, 0);
            push(&mut out, equals, chars.len(), Kind::Heading, 0);
            at = equals;
        }

        while at < chars.len() {
            // `// to the end of the line` is a note to oneself.
            if chars[at] == '/' && chars.get(at + 1) == Some(&'/') {
                push(&mut out, at, chars.len(), Kind::Comment, construct);
                break;
            }
            // `#import`, `#let`, `#show`, `#set` and every call: the
            // instructions, not the writing. Shown, always — a writer needs to
            // see what produces the page.
            if chars[at] == '#' && chars.get(at + 1).is_some_and(|c| c.is_alphabetic()) {
                let end = code_end(&chars, at + 1);
                push(&mut out, at, end, Kind::Code2, construct);
                at = end;
                construct += 1;
                continue;
            }
            // `$maths$`.
            if chars[at] == '$' {
                if let Some(close) = (at + 1..chars.len()).find(|&i| chars[i] == '$') {
                    push(&mut out, at, at + 1, Kind::Marker, construct);
                    push(&mut out, at + 1, close, Kind::Code, construct);
                    push(&mut out, close, close + 1, Kind::Marker, construct);
                    at = close + 1;
                    construct += 1;
                    continue;
                }
            }
            // `*粗*` and `_斜_` — one delimiter, not two.
            if matches!(chars[at], '*' | '_') {
                let kind = if chars[at] == '*' {
                    Kind::Strong
                } else {
                    Kind::Emphasis
                };
                if let Some(close) = closing(&chars, at + 1, chars[at]) {
                    push(&mut out, at, at + 1, Kind::Marker, construct);
                    push(&mut out, at + 1, close, kind, construct);
                    push(&mut out, close, close + 1, Kind::Marker, construct);
                    at = close + 1;
                    construct += 1;
                    continue;
                }
            }
            // `` `code` ``.
            if chars[at] == '`' {
                if let Some(close) = (at + 1..chars.len()).find(|&i| chars[i] == '`') {
                    push(&mut out, at, at + 1, Kind::Marker, construct);
                    push(&mut out, at + 1, close, Kind::Code, construct);
                    push(&mut out, close, close + 1, Kind::Marker, construct);
                    at = close + 1;
                    construct += 1;
                    continue;
                }
            }
            at += 1;
        }
        out
    }

    /// Which block a line of Typst belongs to.
    ///
    /// Far less state than Markdown's: Typst has no `:::` and its raw blocks
    /// use the same ``` fence, so this is a small scanner of its own.
    #[derive(Debug, Default)]
    pub struct BlockScanner {
        in_raw: bool,
        /// How deep inside a `{`, `(` or `[` the scan is.
        ///
        /// Typst code runs across lines — `#show heading: it => block({` opens
        /// a body that closes several lines later — and every one of those
        /// lines is code, not writing. Counting brackets is not parsing Typst,
        /// but it is enough to tell a template apart from a manuscript.
        depth: i32,
    }

    impl BlockScanner {
        pub fn new() -> BlockScanner {
            BlockScanner::default()
        }

        pub fn feed(&mut self, prefix: &str, len: usize) -> Block {
            let line = prefix.trim_end_matches(['\n', '\r']);
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") {
                self.in_raw = !self.in_raw;
                self.depth = 0;
                return Block::Code { inside: None };
            }
            if self.in_raw {
                return Block::Code { inside: None };
            }
            // Inside a code body that opened on an earlier line.
            let was_open = self.depth > 0;
            // **A blank line closes it.** The bracket count is a guess, and a
            // guess that has gone wrong must not run to the end of the file —
            // one line used to be able to tint an entire manuscript. The cost
            // is a `#show` body with a blank line in it, which loses its shade
            // from there on: a shade, against a whole novel.
            if trimmed.is_empty() {
                self.depth = 0;
                return Block::Prose;
            }
            // **Only a line seen whole may be counted.** The caller hands over
            // the first [`PREFIX`] characters, not the line — so
            // `#set par(first-line-indent: (amount: 2em, all: true), …)` had
            // its closing `)` cut off, left `depth` at 1, and made every line
            // after it code, blank ones included (2026-09-08). Markdown's
            // scanner asks the same question one field along; this one took
            // the length and threw it away. A body whose opening line is
            // longer than that simply never opens: one block loses its shade,
            // where miscounting lost the rest of the manuscript.
            let whole = len <= super::PREFIX;
            if whole && (was_open || trimmed.starts_with('#')) {
                for c in line.chars() {
                    match c {
                        '{' | '(' | '[' => self.depth += 1,
                        '}' | ')' | ']' => self.depth = (self.depth - 1).max(0),
                        _ => {}
                    }
                }
            }
            if was_open {
                return Block::Code { inside: None };
            }
            let equals = trimmed.chars().take_while(|&c| c == '=').count();
            if equals > 0 && equals <= 6 && matches!(trimmed.chars().nth(equals), Some(' ') | None)
            {
                return Block::Heading(equals);
            }
            if trimmed.starts_with("- ") || trimmed.starts_with("+ ") {
                return Block::Item { task: None };
            }
            Block::Prose
        }
    }

    /// Where a `#…` run of code ends.
    ///
    /// Typst code runs to the end of its expression, which needs a parser. This
    /// takes the identifier and whatever brackets follow it, balanced — enough
    /// to set `#chapter[初雪]` and `#import "lib.typ": chapter` apart from the
    /// prose around them, which is all a page of writing asks of it.
    fn code_end(chars: &[char], from: usize) -> usize {
        let mut at = from;
        while at < chars.len()
            && (chars[at].is_alphanumeric() || chars[at] == '.' || chars[at] == '_')
        {
            at += 1;
        }
        // A statement — `#import "lib.typ": chapter`, `#let x = 1` — runs to
        // the end of its line. Only a *call* stops at its brackets.
        let word: String = chars[from..at].iter().collect();
        if matches!(word.as_str(), "import" | "include" | "let" | "set" | "show") {
            return chars.len();
        }
        // A trailing bracket group, and the string or arguments in it.
        while let Some(&open) = chars.get(at) {
            let close = match open {
                '(' => ')',
                '[' => ']',
                '{' => '}',
                // `#import "x": a, b` — the rest of the line belongs to it.
                ':' | '"' => return chars.len(),
                _ => break,
            };
            let mut depth = 0usize;
            let mut scan = at;
            while scan < chars.len() {
                if chars[scan] == open {
                    depth += 1;
                } else if chars[scan] == close {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                scan += 1;
            }
            at = (scan + 1).min(chars.len());
        }
        at.max(from)
    }

    /// Where the run closing the delimiter opened at `from` is.
    fn closing(chars: &[char], from: usize, delimiter: char) -> Option<usize> {
        if chars.get(from).is_none_or(|&c| c == ' ') {
            return None;
        }
        (from + 1..chars.len()).find(|&i| chars[i] == delimiter && chars.get(i - 1) != Some(&' '))
    }

    fn push(out: &mut Vec<Span>, start: usize, end: usize, kind: Kind, construct: usize) {
        if end > start {
            out.push(Span {
                start,
                end,
                kind,
                construct,
            });
        }
    }
}

#[cfg(test)]
mod typst_tests {
    use super::typst::*;
    use super::{hidden, Block, Kind};

    fn shape(line: &str) -> String {
        let n = line.chars().count();
        let mut out = vec![' '; n];
        for span in spans(line) {
            let mark = match span.kind {
                Kind::Strong => 'B',
                Kind::Emphasis => 'I',
                Kind::Code => 'C',
                Kind::Code2 => '#',
                Kind::Heading => 'H',
                Kind::Comment => '%',
                Kind::Marker | Kind::HeadingMark => '.',
                // Neither `spans` makes these — they are `diff::spans`'.
                Kind::Gone => '-',
                Kind::Added => '+',
                _ => '?',
            };
            for slot in out.iter_mut().take(span.end.min(n)).skip(span.start) {
                *slot = mark;
            }
        }
        out.into_iter().collect()
    }

    #[test]
    fn typst_spells_its_emphasis_with_one_delimiter() {
        // The whole reason this is a scanner of its own: `**` is Markdown's
        // bold and Typst's is `*`.
        assert_eq!(shape("那*年*天"), " .B. ");
        assert_eq!(shape("那_年_天"), " .I. ");
        assert_eq!(shape("那`碼`天"), " .C. ");
    }

    #[test]
    fn a_typst_heading_is_an_equals_sign() {
        assert_eq!(shape("== 第一章"), "..HHHH");
        assert_eq!(
            shape("# 第一章"),
            "     ",
            "that is Markdown's, not Typst's"
        );
    }

    #[test]
    fn code_is_shown_because_it_is_what_produces_the_page() {
        // Not decoration around writing — the instructions that make it. A
        // writer has to see them, so they are never hidden, only set back.
        assert_eq!(shape("#chapter[初雪]"), "############");
        assert_eq!(shape("那年#emph[冬天]。"), "  ######### ");
        // An import takes the rest of its line.
        assert_eq!(
            shape("#import \"lib.typ\": chapter"),
            "##########################"
        );
        // …and a comment takes the rest of its line too.
        assert_eq!(shape("那年 // 待查"), "   %%%%%");
    }

    #[test]
    fn typst_code_never_comes_off_the_page() {
        // `**` is decoration and can go; `#chapter[…]` is the instruction that
        // makes the chapter, and a page that hid it would be lying.
        let line = "#chapter[初雪]那年*冬*天";
        let hide = hidden(&spans(line), None);
        let shown: String = line
            .chars()
            .enumerate()
            .filter(|(i, _)| !hide.iter().any(|&(a, b)| *i >= a && *i < b))
            .map(|(_, c)| c)
            .collect();
        assert_eq!(shown, "#chapter[初雪]那年冬天");
    }

    #[test]
    fn the_blocks_of_a_typst_manuscript() {
        let mut scanner = BlockScanner::new();
        let walk: String = "= 第一章\n那年\n```\n#let x = 1\n```\n- 阿寧"
            .lines()
            .map(|l| match scanner.feed(l, l.chars().count()) {
                Block::Heading(n) => char::from_digit(n as u32, 10).unwrap_or('#'),
                Block::Code { .. } => '`',
                Block::Item { .. } => '-',
                _ => '.',
            })
            .collect();
        assert_eq!(walk, "1.```-");
    }

    /// **One `#set` used to tint everything under it** (#303).
    ///
    /// The scanner is fed a line's first `PREFIX` characters, not the line.
    /// A `#set par(…)` long enough to have its closing `)` cut off left the
    /// bracket count at 1, and every line after it — prose, blank lines, the
    /// whole rest of the manuscript — came back `Code`.
    #[test]
    fn a_line_too_long_to_be_seen_whole_may_not_open_a_code_block() {
        let long = "#set par(first-line-indent: (amount: 2em, all: true), leading: 1em, justify: true)";
        assert!(long.chars().count() > crate::markdown::PREFIX, "the case is the truncation");
        let text = format!("{long}\n#set text(size: 10pt)\n= 第一章\n那年冬天雪下得早。\n");

        let mut scanner = BlockScanner::new();
        let walk: String = text
            .lines()
            .map(|line| {
                let len = line.chars().count();
                let prefix: String = line.chars().take(crate::markdown::PREFIX).collect();
                match scanner.feed(&prefix, len) {
                    Block::Heading(n) => char::from_digit(n as u32, 10).unwrap_or('#'),
                    Block::Code { .. } => '`',
                    _ => '.',
                }
            })
            .collect();
        // The heading is still a heading and the prose is still prose.
        assert_eq!(walk, "..1.", "one long line tinted the rest of the file");
    }

    /// A body that really does run over several lines still shades them, and
    /// a blank line is the fuse that stops a wrong guess from running away.
    #[test]
    fn a_multi_line_body_is_code_until_the_brackets_close_or_a_blank_line() {
        let mut scanner = BlockScanner::new();
        let walk: String = "#show heading: it => block({\n  it\n})\n那年\n#let f(x) = {\n\n那年"
            .lines()
            .map(|line| {
                let len = line.chars().count();
                let prefix: String = line.chars().take(crate::markdown::PREFIX).collect();
                match scanner.feed(&prefix, len) {
                    Block::Code { .. } => '`',
                    Block::Prose => '.',
                    _ => '?',
                }
            })
            .collect();
        assert_eq!(walk, ".``....", "{walk}");
    }
}

/// #418 二. What the reference completion reads out of a file.
#[cfg(test)]
mod reference_tests {
    use super::*;

    #[test]
    fn a_tag_is_whatever_stands_between_the_caret_and_the_bracket() {
        let text = "甲[^1]乙[^舊註]丙[^1]\n\n[^1]: 一\n[^舊註]: 二\n";
        assert_eq!(footnote_tags(text), ["1", "舊註"], "in file order, and each one once");
        // Not a tag: a space in it, nothing in it, or no bracket to close it.
        assert!(footnote_tags("[^ 甲] [^] [^乙").is_empty());
    }

    #[test]
    fn an_anchor_is_the_heading_lower_cased_with_its_spaces_hyphenated() {
        assert_eq!(anchor("卷一 開端"), "卷一-開端");
        assert_eq!(anchor("The Long Way"), "the-long-way");
        assert_eq!(anchor("第三節：雨"), "第三節雨", "punctuation is dropped, not hyphenated");
        assert_eq!(anchor("  兩   個  "), "兩-個", "a run of space is one hyphen");
        assert_eq!(anchor("《》"), "", "a heading with nothing to name it by");
    }
}

#[cfg(test)]
mod list_tests {
    use super::*;

    /// The five shapes, and what each one hands the line below (#418).
    #[test]
    fn a_marker_says_what_the_next_line_opens_with() {
        let next = |line: &str| opening(line).map(|o| (o.next, o.width, o.empty));
        assert_eq!(next("- 甲"), Some(("- ".into(), 2, false)));
        assert_eq!(next("* 甲"), Some(("* ".into(), 2, false)));
        assert_eq!(next("+ 甲"), Some(("+ ".into(), 2, false)));
        // The number steps on; the punctuation is whichever was written.
        assert_eq!(next("1. 甲"), Some(("2. ".into(), 3, false)));
        assert_eq!(next("9) 甲"), Some(("10) ".into(), 3, false)));
        // A box comes down unticked whether or not this one is ticked.
        assert_eq!(next("- [ ] 甲"), Some(("- [ ] ".into(), 6, false)));
        assert_eq!(next("- [x] 甲"), Some(("- [ ] ".into(), 6, false)));
        // The indent is copied as it stands, and a quote carries its list.
        assert_eq!(next("    - 甲"), Some(("    - ".into(), 6, false)));
        assert_eq!(next("> 甲"), Some(("> ".into(), 2, false)));
        assert_eq!(next("> > 甲"), Some(("> > ".into(), 4, false)));
        assert_eq!(next("> - 甲"), Some(("> - ".into(), 4, false)));
        // Nothing typed into the item: the Enter that ends the list.
        assert_eq!(next("- "), Some(("- ".into(), 2, true)));
        assert_eq!(next("  1. "), Some(("  2. ".into(), 5, true)));
        assert_eq!(next(">"), Some(("> ".into(), 1, true)));
        // The newline the rope hands over with the line changes nothing.
        assert_eq!(next("- 甲\r\n"), Some(("- ".into(), 2, false)));
    }

    /// What is *not* a list, which is the half that keeps a novel a novel.
    #[test]
    fn a_dash_is_not_always_a_list() {
        for line in [
            "-甲",         // no space: a dash whose word has begun
            "-",           // no space, and nothing after it
            "---",         // a scene break
            "- - -",       // the same break, drawn spaced
            "| 甲 | 乙 |", // a table row
            "1.甲",        // no space again
            "1234567890. 甲", // ten digits is not a list
            "甲乙丙",
            "",
        ] {
            assert_eq!(opening(line), None, "{line:?}");
        }
    }
}
