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
    /// The markup itself — the asterisks, the brackets and the target. Shown
    /// and set back in source mode; hidden in 所見即所得.
    Marker,
    /// A heading's hashes.
    ///
    /// A [`Marker`](Kind::Marker) that is never hidden: a terminal cannot make
    /// a heading bigger, so the hashes are the only thing that says whether
    /// this is a chapter or a scene, and a writer needs to know which.
    HeadingMark,
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
    /// `> 引文`.
    Quote,
    /// A `-`, `*` or `1.` item, and whether it is a task and done.
    Item { task: Option<bool> },
    /// `---` or `***` on its own.
    Rule,
    /// Inside a ``` fence, or the fence line itself.
    Code,
    /// The `---`-delimited metadata a file may open with.
    FrontMatter,
    /// Inside a `::: tip` container, or its fence.
    Container(Callout),
    /// A `|`-delimited table row.
    Table,
    /// `[^1]: the note itself`.
    FootnoteDef,
}

/// Which kind of aside a `:::` container is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Callout {
    Note,
    Tip,
    Warning,
    Danger,
}

impl Callout {
    /// Parse the word after `:::`.
    fn parse(word: &str) -> Option<Callout> {
        match word.trim().to_ascii_lowercase().as_str() {
            "note" | "info" | "details" => Some(Callout::Note),
            "tip" => Some(Callout::Tip),
            "warning" | "caution" => Some(Callout::Warning),
            "danger" | "error" => Some(Callout::Danger),
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
        matches!(self, Block::Code | Block::FrontMatter)
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
    in_code: bool,
    /// Which fence opened the code block, so `~~~` does not close a ``` one.
    fence: Option<char>,
    in_front: bool,
    /// Whether the opening `---` is still only a guess at metadata.
    front_unproven: bool,

    /// The containers open, innermost last — `:::` nests, and an inner one must
    /// not close the outer.
    containers: Vec<Callout>,
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
        match (self.fence, fence) {
            (None, Some(opened)) => {
                self.fence = Some(opened);
                return Block::Code;
            }
            (Some(open), Some(closing)) if open == closing => {
                self.fence = None;
                return Block::Code;
            }
            (Some(_), _) => return Block::Code,
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
        if let Some(&kind) = self.containers.last() {
            return Block::Container(kind);
        }

        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
        if hashes > 0 && hashes <= 6 && matches!(trimmed.chars().nth(hashes), Some(' ') | None) {
            return Block::Heading(hashes);
        }
        if trimmed.starts_with('>') {
            return Block::Quote;
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
        if trimmed.starts_with('|') && trimmed.len() > 1 {
            return Block::Table;
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

#[cfg(test)]
mod tests {
    use super::*;

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
                Kind::Marker | Kind::HeadingMark => '.',
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
                Block::Quote => '>',
                Block::Item { task: None } => '-',
                Block::Item { task: Some(false) } => 'o',
                Block::Item { task: Some(true) } => 'x',
                Block::Rule => '_',
                Block::Code => '`',
                Block::FrontMatter => 'y',
                Block::Container(_) => ':',
                Block::Table => '|',
                Block::FootnoteDef => 'F',
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
