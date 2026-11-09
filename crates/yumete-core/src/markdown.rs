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
//! - **CommonMark 的 flanking 規則這裏不做。** Warning: **2026-09-28 更正**：從前這一條寫的是
//!   「CommonMark 的強調規則對漢字是錯的，兩個漢字之間它會失手」，**那句話說過頭了**。
//!   實測 pulldown-cmark 與 comrak 的預設配置，`中**文**中` 兩家都出粗體——漢字在
//!   flanking 眼裏是 `Lo`，和拉丁字母同一類。真正失手的是**強調內側貼着全角標點**：
//!   `他說**「好」**。` 兩家都不認（兩家各有一個預設關着的 `cjk_friendly_emphasis`
//!   擴展在修這個，同一份上游規範的移植）。
//!
//!   不做 flanking 的理由因此換一條：**這裏是編輯器不是 CommonMark 實現**，屏幕上的着
//!   色要和寫的人按下去的鍵對得上，而 flanking 的判準要看左右兩個字符的類別——同一串
//!   星號在句中和句末着色不同，讀者看不出規律。一個分隔符就是一個分隔符。
//! - **套得起來，但只到三層**（2026-09-28）。一個構造配對成功之後對它的文字再掃一遍，
//!   `**粗的`碼`**` 因此認得出來。不做通用的 delimiter stack。

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
    /// **`$a + b$`** — Typst 的數學（2026-09-28）。
    ///
    /// Warning: **從前它借 `Kind::Code`**，於是 `:export` 把公式導成 `` `a + b` ``——一條反引
    /// 號在 Typst 裏是**原樣文本**，公式就此不是公式了。借用在屏幕上看不出來（兩者
    /// 都該畫成一個要逐字讀的字面），是導出的時候壞的。
    Math,
    /// **`@定理一`** — Typst 的交叉引用（2026-09-28）。整串連 `@` 一起。
    Ref,
    /// **`<定理一>`** — Typst 的標籤，被 [`Kind::Ref`] 指的那一頭。整串連尖括號一起。
    Label,
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
    /// An inline HTML tag, `<kbd>` or `</kbd>`, the whole of it. Set back as
    /// a marker is, but never hidden: `<br>` says something `**` does not.
    /// The text between two tags is prose.
    Html,
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
    /// **The whole of an old row in a `:diff` listing** (2026-10-16).
    ///
    /// A changed line is drawn as two rows, the draft before and the draft
    /// after, and the row itself carries a faint ground so the pair reads as a
    /// pair before a word of it is read. The words that actually changed sit
    /// on [`Gone`](Kind::Gone) / [`Added`](Kind::Added), which is the louder
    /// step of the same colour — 「mark the words changed in brighter colors」.
    GoneRow,
    /// The whole of a new row — see [`GoneRow`](Kind::GoneRow).
    AddedRow,
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
    /// Warning: **A block inside a callout is still that block** (#468). A fence in a
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
    /// **A line of display maths that runs over several lines** (2026-11-06):
    /// Markdown's `$$` … `$$`, Typst's `$` … `$`. `first` is the line the
    /// opening delimiter is on, `last` the line the closing one is on; the
    /// lines between are the formula through and through.
    ///
    /// Not read by either scanner, which look at a line's opening only and
    /// could not know whether a `$$` ever closes; [`maths_blocks`] finds them
    /// after the scan, the way the merge conflicts are found, and only a block
    /// that closes before a blank line takes the label. Until then only the
    /// one-line forms were coloured, and the formula a writer sets on lines of
    /// its own was drawn as prose — its `\\` and `&` and all.
    Math { first: bool, last: bool },
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
    /// Warning: **`caution` is the red one, not a warning.** GitHub gives
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
    /// Where a footnote's text stands: what decides whether the next line
    /// is still the note. See [`Note`].
    note: Note,
}

/// **A footnote runs on past its first line** (2026-10-25).
///
/// `[^1]: text` is followed by the lines that continue it, the way GFM and
/// pandoc read one: an indented line is the note's, after a blank line too
/// (a note of two paragraphs), and a plain line straight after the note's text
/// is its lazy continuation. Only the `[^1]:` line used to be set back as
/// furniture, so a note hard-wrapped into six lines had its first line grey and
/// the other five drawn as writing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Note {
    #[default]
    None,
    /// The line above was the note's text.
    Open,
    /// Blank lines since the note's text: only an indented line goes on.
    Gap,
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
        // Warning: **The table counter is reset by default, and only the table arm
        // puts it back** (#467). It used to be cleared on two of the eleven
        // paths that end a table — so a second table separated from the first
        // by a heading, a `:::`, a fence, a quote or a list item went on
        // counting, and its header was drawn as a body row with the banding
        // inverted. Clearing here and restoring there cannot be forgotten by
        // whoever adds the twelfth path.
        let table_row = std::mem::take(&mut self.table_row);
        // Same shape: every path ends the note unless it says otherwise.
        let note = std::mem::take(&mut self.note);
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
        // An indented line goes on with an open note whatever it starts with —
        // `    - one` in a note is the note's list, not a new one.
        if note != Note::None && !trimmed.is_empty() && text.starts_with([' ', '\t']) {
            self.note = Note::Open;
            return Block::FootnoteDef;
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
            self.note = Note::Open;
            return Block::FootnoteDef;
        }
        if let Some(rest) = item_body(trimmed) {
            let task = rest
                .strip_prefix('[')
                .and_then(|r| r.get(..1).zip(r.get(1..2)))
                .and_then(|(mark, close)| (close == "]").then_some(mark != " "));
            return Block::Item { task };
        }
        match (note, trimmed.is_empty()) {
            (Note::None, _) => {}
            (_, true) => self.note = Note::Gap,
            (Note::Open, false) => {
                self.note = Note::Open;
                return Block::FootnoteDef;
            }
            (Note::Gap, false) => {}
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

/// **這一行是不是標題，是幾級。**
///
/// `mark` 是 `#`（Markdown）或者 `=`（Typst）。規矩一條：一到六個記號，後面跟着**空白**
/// 或者行尾。
///
/// Warning: **後面那個空白是 CommonMark §4.2 明寫的**，不是我們加嚴的：`#128` 是一段話，
/// 不是標題。Markdown 1.0 原版寬鬆，而那條規矩存在的理由正是 `#128`、`#!/bin/sh`、
/// `#include` 這一族。Typst 的 `=` 同樣要求後跟空白。
///
/// Warning: **全角空格也算空白**——寫中文的人最常見的縮進寫法是 `= 　第一章`。
///
/// Warning: **標題本身空不空由呼叫方決定。** 大綱要求非空（光一個 `##` 是一條線，不是標題），
/// 而着色那一支不要求。
///
/// Warning: **這一支存在的理由是樹裏本來有三份不一樣的判準**（2026-09-28 查出來的）：着色那一
/// 份要求空白、大綱那一份要求空白但不封頂六級、`typst_headings` 兩樣都不要求。同一行在
/// 正文裏不畫成標題、卻出現在大綱上。
pub fn heading_marks(line: &str, mark: char) -> Option<usize> {
    let level = line.chars().take_while(|&c| c == mark).count();
    if level == 0 || level > 6 {
        return None;
    }
    match line.chars().nth(level) {
        None => Some(level),
        Some(c) if c.is_whitespace() => Some(level),
        Some(_) => None,
    }
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
    /// **套了幾層**（2026-09-28）：頂層是 0，`**粗的 `碼`**` 裏那個 `` ` `` 是 1。
    ///
    /// 算得出來（包住它的有幾條），存下來是因為 `:export` 那一趟棧式遍歷有了它是十行、
    /// 没有就得每一步做區間比較。
    pub depth: u8,
}

/// **這一行有哪些標記**，按起點排好，**外層排在內層前面**。
///
/// 没被蓋到的就是普通正文，不交 span。
///
/// # Warning: 兩條不變式，下游全靠它們
///
/// 一、**任意兩條要麼不相交，要麼一條完全包含另一條**。不會出現半重疊。
///
/// 二、**按 `start` 排好；包住別人的那一條排在前面。** 於是
/// 「蓋住某一格的最內層那一條」＝ 覆蓋它的裏面 `start` 最大的那一條
/// （`max_by_key(start)`，`mi k` 就是這麼取的）。
///
/// Warning: **「不重疊」那句話 2026-09-28 之前就已經不成立了**，標題那一支先壓一條
/// `Kind::Heading` 蓋住整行、行內的構造再壓上去。當時没寫下約定，於是七個消費方各猜各
/// 的，其中三個猜錯：`panel.rs` 那個順序寫字的循環會切出反向區間**當場崩**，`export.rs`
/// 與 `detail.rs` 的 `.find()` 取到的是外層那一條（標題裏的腳註因此没有詳情面板）。
///
/// # 嵌套
///
/// 一個構造配對成功之後，對它的**文字**那一段再掃一遍（[`DEPTH`] 層封頂）。所以
/// ``**粗的`碼`**`` 裏的行內代碼認得出來，`[**粗**的](x)` 也是。
///
/// Warning: **行內代碼裏面不掃**：那是 CommonMark 的規矩，也是常識——`` `a*b*c` `` 裏的星號
/// 是代碼的一部分。批注 `%%…%%` 同理，裏面的東西整個是寫的人自己的。
pub fn spans(line: &str) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();

    // A heading is the whole line, so it is decided before anything else and
    // the rest of the line is still scanned for emphasis inside the title.
    let mut from = 0;
    let mut depth = 0u8;
    // 判準在 [`heading_marks`]，三處共用一支。
    if let Some(hashes) = heading_marks(line, '#') {
        out.push(Span {
            start: 0,
            end: hashes,
            kind: Kind::HeadingMark,
            construct: 0,
            depth: 0,
        });
        from = hashes;
        // The title itself, under whatever emphasis it also carries.
        if from < chars.len() {
            out.push(Span {
                start: from,
                end: chars.len(),
                kind: Kind::Heading,
                construct: 0,
                depth: 0,
            });
            // 標題底下的東西都在它裏面，所以從第 1 層數起。
            depth = 1;
        }
    }
    let mut construct = 1usize;
    scan(&chars, from, chars.len(), depth, &mut construct, &mut out);
    out
}

/// **套到第幾層為止。**
///
/// Warning: 不做通用的 delimiter stack（CommonMark §6.2 那一套兩趟加棧）。真會寫出來的形狀
/// ——``**粗的`碼`**``、`[**粗**的](x)`、`==**重點**==`——兩層就夠，而三層是留給
/// `**《書名》的`碼`**` 這種。再深的層數在一篇文章裏見不到，而每深一層就是整段重掃一遍。
pub const DEPTH: u8 = 3;

/// [`spans`] 的一趟：掃 `from..to`，交出來的 span 都在這個範圍裏。
///
/// Warning: **每一處算出來的 `end` 都要對着 `to` 驗一次。** 底下那些 `code_span`/`closing`
/// /`find` 找的是**整行**，遞歸進來的時候它們會越過 `to` 去找閉合符——``**a`b**c`` 裏
/// 從 `a` 那一段往後找反引號，會找到 `to` 外面去。
fn scan(
    chars: &[char],
    from: usize,
    to: usize,
    depth: u8,
    construct: &mut usize,
    out: &mut Vec<Span>,
) {
    let mut at = from;
    while at < to {
        // A comment is the writer talking to themselves — everything in it is
        // theirs, markup included — so it is taken before anything else.
        // **`\*` 不是斜體的開頭**（2026-09-28）。反斜杠自己算標記，所見即所得把它藏
        // 起來，剩下那個字符原樣是正文。
        //
        // Warning: **可轉義的只有 ASCII 標點**，CommonMark §2.4 的規矩，兩家實現也一樣：
        // `\甲` 裏那個反斜杠是一個反斜杠，不是轉義。要是連漢字都能轉義，稿子裏每一個
        // 反斜杠都會憑空消失。
        if chars[at] == '\\'
            && chars.get(at + 1).is_some_and(|&c| escapable(c)) {
                mark(out, at, at + 1, Kind::Marker, *construct, depth);
                *construct += 1;
                at += 2;
                continue;
            }
        if let Some((open, close, end)) = comment(chars, at).filter(|&(_, _, e)| e <= to) {
            // Warning: **批注裏面不掃**：`%%…%%` 整個是寫的人對自己說的話，標記也是他的。
            mark(out, at, at + open, Kind::Marker, *construct, depth);
            mark(out, at + open, end - close, Kind::Comment, *construct, depth);
            mark(out, end - close, end, Kind::Marker, *construct, depth);
            at = end;
            *construct += 1;
            continue;
        }
        // Code next: inside a code span nothing else is markup.
        if chars[at] == '`' {
            if let Some((open, close, end)) = code_span(chars, at).filter(|&(_, _, e)| e <= to) {
                // Warning: **行內代碼裏面不掃**，CommonMark 的規矩：`` `a*b*c` `` 裏的星號
                // 是代碼的一部分。
                mark(out, at, at + open, Kind::Marker, *construct, depth);
                mark(out, at + open, close, Kind::Code, *construct, depth);
                mark(out, close, end, Kind::Marker, *construct, depth);
                at = end;
                *construct += 1;
                continue;
            }
        }
        // `$a + b$` and `$$x^2$$`, as Pandoc reads them: nothing inside is
        // markup, and `$3 and $5` is two prices. This and the two below stop
        // at a backtick: a code span after them is still a code span.
        if chars[at] == '$' {
            match math(chars, at, to) {
                Some((open, close)) => {
                    mark(out, at, at + open, Kind::Marker, *construct, depth);
                    mark(out, at + open, close, Kind::Math, *construct, depth);
                    mark(out, close, close + open, Kind::Marker, *construct, depth);
                    at = close + open;
                    *construct += 1;
                }
                // An unclosed `$$` is not a `$` and then another.
                None => at += 1 + usize::from(chars.get(at + 1) == Some(&'$')),
            }
            continue;
        }
        // `<https://…>` is a link with the address for its text; `<kbd>` and
        // `</kbd>` are HTML, and what stands between them is prose.
        if chars[at] == '<' {
            if let Some(end) = autolink(chars, at, to) {
                mark(out, at, at + 1, Kind::Marker, *construct, depth);
                mark(out, at + 1, end - 1, Kind::Link, *construct, depth);
                mark(out, end - 1, end, Kind::Marker, *construct, depth);
                at = end;
                *construct += 1;
                continue;
            }
            if let Some(end) = html_tag(chars, at, to) {
                mark(out, at, end, Kind::Html, *construct, depth);
                at = end;
                *construct += 1;
                continue;
            }
        }
        if let Some((len, run)) = fence(chars, at, to) {
            let kind = match (chars[at], len) {
                ('*', 2) | ('_', 2) => Kind::Strong,
                ('~', 2) => Kind::Strike,
                ('=', 2) => Kind::Highlight,
                _ => Kind::Emphasis,
            };
            if let Some(close) = closing(chars, at + len, to, chars[at], len, run) {
                mark(out, at, at + len, Kind::Marker, *construct, depth);
                mark(out, at + len, close, kind, *construct, depth);
                let mine = *construct;
                *construct += 1;
                // **裏面再掃一遍**（2026-09-28）：``**粗的`碼`**``。前序——外層那一條
                // 已經發出去了，閉標記留到遞歸之後發，於是整串仍然按起點排好。
                if depth + 1 < DEPTH {
                    scan(chars, at + len, close, depth + 1, construct, out);
                }
                mark(out, close, close + len, Kind::Marker, mine, depth);
                at = close + len;
                continue;
            }
        }
        // `[[第三章]]`, `[[第三章|那一夜]]`, `[[第三章#雪]]` — a reference to
        // somewhere else in the same manuscript.
        if chars[at] == '[' && chars.get(at + 1) == Some(&'[') {
            if let Some(close) = run(chars, at + 2, "]]").filter(|&c| c + 2 <= to) {
                // What is shown is the alias if there is one, else the target.
                let body = at + 2..close;
                let shown = chars[body.clone()]
                    .iter()
                    .position(|&c| c == '|')
                    .map(|i| at + 2 + i + 1..close)
                    .unwrap_or(body);
                // Warning: **雙鏈裏面不掃**：那是一個頁名（或者頁名加一個別名），不是正文。
                mark(out, at, shown.start, Kind::Marker, *construct, depth);
                mark(out, shown.start, shown.end, Kind::WikiLink, *construct, depth);
                mark(out, shown.end, close + 2, Kind::Marker, *construct, depth);
                at = close + 2;
                *construct += 1;
                continue;
            }
        }
        // `[^1]`, and the `[^1]:` that opens the note itself.
        if chars[at] == '[' && chars.get(at + 1) == Some(&'^') {
            if let Some(close) = find(chars, at + 2, |c| c == ']') {
                let end = close + 1 + usize::from(chars.get(close + 1) == Some(&':'));
                if end <= to {
                    mark(out, at, end, Kind::Footnote, *construct, depth);
                    at = end;
                    *construct += 1;
                    continue;
                }
            }
        }
        // `[text](target)` — the text is what the reader reads.
        //
        // Warning: **`![圖](a.png)` 的那個驚嘆號也在構造裏**（2026-09-28）。從前它不在，於是
        // 所見即所得把 `[` `](a.png)` 藏起來之後，行首孤零零留着一個 `!`。
        let bang = chars[at] == '!' && chars.get(at + 1) == Some(&'[');
        if chars[at] == '[' || bang {
            let open = at + usize::from(bang);
            // Warning: **兩頭都數括號**（2026-09-28）。從前是「往後找第一個 `]`/`)`」：
            // `[甲[乙]丙](x)` 在第一個 `]` 上就斷了，整條鏈接不認；
            // `[連結](…/中文_(消歧義))` 在裏面那個 `)` 上就收了口，末尾那個括號掉在正
            // 文上。中文維基的地址裏帶括號是常事。
            if let Some(close) = balanced(chars, open, ']', to) {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = balanced(chars, close + 1, ')', to) {
                        mark(out, at, open + 1, Kind::Marker, *construct, depth);
                        mark(out, open + 1, close, Kind::Link, *construct, depth);
                        let mine = *construct;
                        *construct += 1;
                        // 看得見的那幾個字也可以帶標記：`[**粗**的](x)`。
                        if depth + 1 < DEPTH {
                            scan(chars, open + 1, close, depth + 1, construct, out);
                        }
                        mark(out, close, end + 1, Kind::Marker, mine, depth);
                        at = end + 1;
                        continue;
                    }
                }
            }
        }
        at += 1;
    }
}

/// Inline maths opening at `at`: how many `$` open and close it (1 or 2),
/// and where the closing ones start.
///
/// Pandoc's `tex_math_dollars`: an opening `$` has no space after it, the
/// closing one no space before it and no digit after it, and the first `$`
/// that comes is the one that has to close. `$$…$$` takes anything.
fn math(chars: &[char], at: usize, to: usize) -> Option<(usize, usize)> {
    let stop = |i: usize| chars[i] == '`' || (chars[i] == '$' && !escaped(chars, i));
    if chars.get(at + 1) == Some(&'$') {
        let close = (at + 2..to.saturating_sub(1))
            .find(|&i| chars[i] == '`' || stop(i) && chars[i + 1] == '$')?;
        return (close > at + 2 && chars[close] == '$').then_some((2, close));
    }
    if chars.get(at + 1).is_none_or(|c| c.is_whitespace()) {
        return None;
    }
    let close = (at + 1..to).find(|&i| stop(i)).filter(|&i| chars[i] == '$')?;
    let shuts = !chars[close - 1].is_whitespace()
        && !chars.get(close + 1).is_some_and(|c| c.is_ascii_digit());
    shuts.then_some((1, close))
}

/// An autolink opening at `at`, CommonMark's two kinds: `<scheme:…>` and
/// `<name@host>`. Where it ends, past its `>`.
fn autolink(chars: &[char], at: usize, to: usize) -> Option<usize> {
    let close = (at + 1..to).find(|&i| matches!(chars[i], '>' | '<' | '`') || chars[i].is_whitespace())?;
    if chars[close] != '>' {
        return None;
    }
    let body = &chars[at + 1..close];
    let scheme = body.iter().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')).count();
    let uri = (2..=32).contains(&scheme)
        && body[0].is_ascii_alphabetic()
        && body.get(scheme) == Some(&':');
    let email = || {
        let Some(sign) = body.iter().position(|&c| c == '@') else {
            return false;
        };
        let (local, host) = (&body[..sign], &body[sign + 1..]);
        !local.is_empty()
            && local.iter().all(|&c| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c))
            && host.split(|&c| c == '.').all(|label| {
                !label.is_empty()
                    && label.iter().all(|&c| c.is_ascii_alphanumeric() || c == '-')
                    && label[0] != '-'
                    && label[label.len() - 1] != '-'
            })
    };
    (uri || email()).then_some(close + 1)
}

/// The elements a tag in prose may name. CommonMark takes any name, and
/// `Vec<String>` or `a<b and c>d` then lost their words on export.
const HTML_ELEMENTS: &[&str] = &[
    "a", "abbr", "audio", "b", "bdi", "bdo", "blockquote", "br", "center", "cite", "code", "dd", "del", "details",
    "dfn", "div", "dl", "dt", "em", "figcaption", "figure", "font", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "i",
    "iframe", "img", "ins", "kbd", "li", "mark", "ol", "p", "picture", "pre", "q", "rp", "rt", "ruby", "s", "samp",
    "small", "source", "span", "strong", "sub", "summary", "sup", "table", "tbody", "td", "tfoot", "th", "thead",
    "tr", "u", "ul", "var", "video", "wbr",
];

/// The attributes a tag may carry without a value.
const HTML_FLAGS: &[&str] = &["autoplay", "checked", "controls", "disabled", "hidden", "loop", "muted", "open"];

/// An HTML tag opening at `at` — `<kbd>`, `<br/>`, `<a href="x">`, `</kbd>` —
/// on one line: a name from [`HTML_ELEMENTS`], and attributes written
/// `name="…"`, `name='…'`, `name=value` or one of [`HTML_FLAGS`]. Where it
/// ends, past its `>`.
fn html_tag(chars: &[char], at: usize, to: usize) -> Option<usize> {
    let get = |i: usize| (i < to).then(|| chars[i]);
    let closing = get(at + 1) == Some('/');
    let mut i = at + 1 + usize::from(closing);
    if !get(i)?.is_ascii_alphabetic() {
        return None;
    }
    let name_at = i;
    while get(i).is_some_and(|c| c.is_ascii_alphanumeric() || c == '-') {
        i += 1;
    }
    let name = chars[name_at..i].iter().collect::<String>().to_ascii_lowercase();
    if !HTML_ELEMENTS.contains(&name.as_str()) {
        return None;
    }
    let space = |i: &mut usize| {
        let from = *i;
        while get(*i).is_some_and(char::is_whitespace) {
            *i += 1;
        }
        *i > from
    };
    if !closing {
        // Attributes, each after a space: a name, and maybe `=` and a value.
        loop {
            let mut j = i;
            if !space(&mut j) || !get(j).is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '_' | ':')) {
                break;
            }
            let attribute_at = j;
            while get(j).is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-')) {
                j += 1;
            }
            let attribute = chars[attribute_at..j].iter().collect::<String>().to_ascii_lowercase();
            let mut k = j;
            space(&mut k);
            if get(k) != Some('=') && !HTML_FLAGS.contains(&attribute.as_str()) {
                return None;
            }
            if get(k) == Some('=') {
                k += 1;
                space(&mut k);
                match get(k)? {
                    quote @ ('"' | '\'') => {
                        k = (k + 1..to).find(|&n| chars[n] == quote || chars[n] == '`')? + 1;
                        if chars[k - 1] != quote {
                            return None;
                        }
                    }
                    _ => {
                        let from = k;
                        while get(k).is_some_and(|c| !c.is_whitespace() && !"\"'=<>`".contains(c)) {
                            k += 1;
                        }
                        if k == from {
                            return None;
                        }
                    }
                }
                j = k;
            }
            i = j;
        }
    }
    space(&mut i);
    if !closing && get(i) == Some('/') {
        i += 1;
    }
    (get(i)? == '>').then_some(i + 1)
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
    // **蓋住光標的那些構造裏，最裏面那個「是鏈接」的。**
    //
    // Warning: **不能只看最裏面那一條**（2026-09-28）：`[**粗**的](x)` 裏光標站在「粗」上，
    // 最裏面的是那個 `Strong`，它自己不是鏈接——只看它的話，文字是粗體的鏈接就跟不
    // 了了。往外走一層就到了。
    //
    // Warning: 也不能只看最外面那一條：標題那一條蓋住整行，而標題本身不是鏈接。
    let construct = all
        .iter()
        .filter(|s| (s.start..s.end).contains(&at))
        .filter(|s| {
            all.iter()
                .any(|t| t.construct == s.construct && matches!(t.kind, Kind::Link | Kind::WikiLink))
        })
        .max_by_key(|s| s.start)?
        .construct;
    let mine = || all.iter().filter(|s| s.construct == construct);
    let start = mine().map(|s| s.start).min()?;
    let end = mine().map(|s| s.end).max()?;
    let chars: Vec<char> = line.chars().collect();
    let whole: String = chars.get(start..end)?.iter().collect();

    let double = whole.strip_prefix("[[").and_then(|b| b.strip_suffix("]]"));
    let angled = whole.strip_prefix('<').and_then(|b| b.strip_suffix('>'));
    let (written, wiki) = match (double, angled) {
        // `[[第三章|那一夜]]` — the alias is what the reader reads, so the
        // destination is the half before the bar.
        (Some(body), _) => (body.split('|').next().unwrap_or(body).to_string(), true),
        // `<https://…>` names itself; `<name@host>` is an address to write to.
        (None, Some(body)) => match body.contains(':') {
            true => (body.to_string(), false),
            false => (format!("mailto:{body}"), false),
        },
        (None, None) => {
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
    // A path written as a URL (`my%20file.md`) names the file with the space
    // (2026-10-29, review); an address keeps its escapes for the browser.
    let target = match wiki || target.contains(':') {
        true => target,
        false => percent_decoded(&target),
    };
    Some(Link {
        target,
        anchor: anchor.filter(|a| !a.is_empty()),
        wiki,
    })
}

/// `%20` and the other `%XX` escapes read back as the characters they stand
/// for; the text as it was when they do not spell UTF-8.
pub fn percent_decoded(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes
            .get(at + 1..at + 3)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[at], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

/// How many delimiter characters start at `at`, or `None` when none do.
///
/// An underscore only opens where a word does not: `snake_case` is a name, not
/// an emphasis, and in a manuscript full of file names that matters more than
/// being able to italicise with `_`.
fn fence(chars: &[char], at: usize, to: usize) -> Option<(usize, usize)> {
    let c = chars[at];
    if !matches!(c, '*' | '_' | '~' | '=') {
        return None;
    }
    // Warning: **整行只有這一個字符，那是一條線，不是強調**（2026-10-02 一輪掃查
    // 報來的）。十四個 `=` 自己一行，從前被當成反覆的 `==高亮==` 一對一對吃掉：
    // 畫出來只剩六個，而且光標走過去的時候那一行在 6/10/14 格之間變來變去。
    // setext 標題的下劃線（`====`）和散文裏的 `*****` 分隔線都中。
    //
    // CommonMark 也是這麼分的：`***` 自己一行是 thematic break，`===` 自己一行是
    // setext 的下劃線——兩個都不是強調。只收「整行」這一種，別的照舊。
    if chars.iter().all(|&x| x == c || x.is_whitespace()) {
        return None;
    }
    let run = chars[at..to].iter().take_while(|&&x| x == c).count();
    // `~` and `=` only ever come in pairs: a lone one is a dash or an equals,
    // and three of them are a code fence or a setext rule, not a longer mark.
    if matches!(c, '~' | '=') && run != 2 {
        return None;
    }
    // Warning: **一串三顆星是一層套一層**（2026-10-03 修）：CommonMark 把 `***x***` 讀成
    // 外面一顆（斜）包着裏面兩顆（粗）。從前這裏寫的是 `.min(2)`，於是外面那一層一上來
    // 就拿走兩顆，首尾各剩一顆掉在正文上——`***both***` 畫出來是 `*both`。
    // 開標記從一串的**前面**取，閉標記從它那一串的**後面**取，剩下的留給外面那一層。
    let len = if run.is_multiple_of(2) { 2 } else { 1 };
    if c == '_' {
        // Warning: **漢字不算「詞內」**（2026-09-28 修）。這一條本來是保 `snake_case` 的：
        // 拉丁詞中間的下劃線不是強調。可是 `is_alphanumeric()` 對漢字也回真，於是
        // `這_是_重點` 一條 span 都不交——這是這一族裏唯一一個我們自己造出來的中文
        // 問題。Typst 官方那條 `in_word` 也是明確把漢字、假名、諺文排除在外的。
        let before = at.checked_sub(1).and_then(|i| chars.get(i));
        if before.is_some_and(|&c| c.is_alphanumeric() && !yumete_cjk::is_han(c)) {
            return None;
        }
    }
    // A delimiter with nothing after it opens nothing.
    (at + len < to).then_some((len, run))
}

/// Where the marker closing the run opened at `from` begins, searching `from..to`.
///
/// `len` is how many characters the opening marker took and `run` how long the
/// whole opening run was. The content may not be empty and may not start with a
/// space: `** ` in the middle of a sentence is two asterisks, not the start of
/// a bold run.
///
/// # Warning: 閉合的那一串要**整串一樣長**，不許在一串中間收口
///
/// 2026-10-03 修。從前這裏是逐格比對「接下來 `len` 個都是分隔符」，於是
/// `*em **strong** em*` 裏那一顆星的強調走到 `**` 的**第二顆**就收了口，畫出來是
/// `em *`——`**` 被拆掉一顆。
///
/// 只擋「串中間」不夠：那樣一顆星的強調會改收在後面那個 `**` 上（`*em **strong*`），
/// 只是換一種錯法。真正分得開兩層的是**串長**：一顆星配一顆星那一串，兩顆配兩顆。
/// 而 `***x***` 兩串都是三顆，外層取頭一顆、閉合取末一顆（`end - len`），裏面那兩顆
/// 留給遞歸那一趟。
fn closing(
    chars: &[char],
    from: usize,
    to: usize,
    delimiter: char,
    len: usize,
    run: usize,
) -> Option<usize> {
    if chars.get(from) == Some(&' ') {
        return None;
    }
    // 掃從**整串開標記之後**起，不是從正文起：`***x***` 的外層只拿走頭一顆，剩下
    // 兩顆還在原地，從正文起掃第一個看見的就是它們自己。
    let mut at = from + run - len;
    // **還開着幾層。** 一串分隔符既能開也能閉的時候（`**粗***斜*` 中間那三顆），
    // 要分得清它是在收我這一層還是在開新的一層——這就是那個計數。
    let mut depth = 0usize;
    while at < to {
        // Warning: **轉義掉的那一個不算閉合**（2026-09-28）：`*斜\*體*` 的斜體到最後那個
        // 星號纔收口，不是到中間那個。
        if chars[at] != delimiter || escaped(chars, at) {
            at += 1;
            continue;
        }
        let end = at + (at..to).take_while(|&i| chars[i] == delimiter).count();
        // CommonMark 的 flanking：左邊不是空白就收得了口，右邊不是空白就開得了頭。
        let shuts = at > 0 && chars.get(at - 1) != Some(&' ');
        let opens = end < to && chars.get(end) != Some(&' ');
        // **CommonMark 的「三的規矩」**（§6.2 那一條）。一串既能開又能閉的時候
        // （`**甲*乙*丙**` 中間那一顆），兩串長度之和是 3 的倍數就不許配對——除非
        // 兩串自己都是 3 的倍數。没有這一條，那一顆星會**收掉**外面的粗體，而它
        // 要做的是開裏面的斜體。
        let both = shuts && opens;
        let allowed = !both
            || !(run + (end - at)).is_multiple_of(3)
            || (run.is_multiple_of(3) && (end - at).is_multiple_of(3));
        if shuts && depth == 0 && allowed {
            // **一樣長的那一對，開標記取串頭、閉標記取串尾**（`***x***`：外面一顆
            // 斜，裏面兩顆粗，中間那幾個字留給遞歸）。不一樣長的時候閉標記取**串
            // 頭**，剩下的留給右邊那一條構造——`**粗***斜*` 中間那三顆，兩顆收粗
            // 體、一顆開斜體。
            return Some(match end - at == run {
                true => end - len,
                false => at,
            });
        }
        // **收得了口就先收**：裏面還開着一層的時候，這一串是來收它的，不是來
        // 再開一層的（`**甲*乙*丙**` 第二顆星）。没有開着的層纔算它開了新的一層。
        if shuts && depth > 0 {
            depth -= 1;
        } else if opens {
            depth += 1;
        }
        at = end;
    }
    None
}

/// **CommonMark 認得的可轉義字符**：ASCII 標點，僅此而已（§2.4）。
///
/// Warning: 不收漢字也不收全角標點。收了的話稿子裏每一個反斜杠後面那個字都會被當成轉義，
/// 而中文稿子裏的反斜杠多半就是一個反斜杠。
fn escapable(c: char) -> bool {
    c.is_ascii_punctuation()
}

/// `chars[at]` 是不是被前面那個反斜杠轉義掉了。
///
/// Warning: **反斜杠自己也能被轉義**（`\\*` 是一個反斜杠加一個真的星號），所以要往回數有幾
/// 個連着的反斜杠：奇數個纔是轉義。
fn escaped(chars: &[char], at: usize) -> bool {
    if !escapable(chars[at]) {
        return false;
    }
    let mut back = 0usize;
    let mut i = at;
    while i > 0 && chars[i - 1] == '\\' {
        back += 1;
        i -= 1;
    }
    back % 2 == 1
}

/// A comment opening at `at`: the lengths of its two markers and where it ends.
///
/// `%%…%%` is Obsidian's, `<!-- … -->` is HTML's, and a manuscript uses both
/// for the same thing — a note that is not part of the book. An unclosed one
/// runs to the end of the line, because a half-typed note is still a note.
fn comment(chars: &[char], at: usize) -> Option<(usize, usize, usize)> {
    // Warning: **asked at every character of every line drawn** (2026-11-08):
    // the scan in [`spans`] tries a comment first, everywhere. It built two
    // `Vec<char>` each time — the openers and, through [`run`], the closer —
    // and that was most of what reading a line cost. Nothing here allocates.
    for (open, close) in [("%%", "%%"), ("<!--", "-->")] {
        if starts_at(chars, at, open) {
            // Both markers are ASCII: a byte is a char.
            let found = run(chars, at + open.len(), close);
            let end = found.map_or(chars.len(), |i| i + close.len());
            return Some((open.len(), if found.is_some() { close.len() } else { 0 }, end));
        }
    }
    None
}

/// Whether `text` stands in `chars` at `at`.
fn starts_at(chars: &[char], at: usize, text: &str) -> bool {
    (at..).zip(text.chars()).all(|(k, c)| chars.get(k) == Some(&c))
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
            if let Some((_, _, end)) = code_span(&chars, at) {
                at = end;
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
    // Warning: **A note that is never closed may not eat the rest of the book**
    // (2026-09-19). A comment carries across lines by design, so a `<!--` with
    // no `-->` swallowed every line after it — and silently: `:export html` on
    // a chapter with one unfinished note handed the publisher the first
    // paragraph and nothing else, while the page on screen still showed the
    // whole thing. [`comment`]'s own rule is that an unclosed one runs to the
    // end of **its line**, so that is what this falls back to: remember where
    // the next line begins, and if the file ends with the note still open,
    // wind back to there and copy the rest through untouched.
    let mut resume: Option<(usize, usize)> = None;
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let here = offset;
        offset += line.len();
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
                if let Some((_, _, end)) = code_span(&chars, at) {
                    kept.extend(&chars[at..end]);
                    at = end;
                    continue;
                }
            }
            kept.push(chars[at]);
            at += 1;
        }
        let dropped = kept.trim().is_empty() && !body.trim().is_empty();
        if !dropped {
            out.push_str(&kept);
            out.push_str(ending);
        }
        // The line the note opened on is the last one that is honestly gone;
        // everything after it is what gets wound back.
        if open.is_some() && resume.is_none() {
            resume = Some((out.len(), here + line.len()));
        }
        if open.is_none() {
            resume = None;
        }
    }
    if let (Some(_), Some((len, from))) = (open, resume) {
        out.truncate(len);
        out.push_str(&text[from.min(text.len())..]);
    }
    out
}

/// Where `text` next occurs at or after `from`.
fn run(chars: &[char], from: usize, text: &str) -> Option<usize> {
    // No `Vec` of `text`'s chars: this is asked once a character of a line.
    let n = text.chars().count();
    (from..chars.len().saturating_sub(n.saturating_sub(1))).find(|&i| starts_at(chars, i, text))
}

/// **An inline code span, counted in backticks** (2026-09-19).
///
/// CommonMark's rule: a run of *n* backticks opens a code span, and only a run
/// of **exactly n** closes it. Everything here used to take 「the next backtick」
/// instead, which is right for `` `code` `` and wrong the moment two of them
/// stand together: ```` ``%%`` ```` was read as an *empty* code span made of the
/// first two backticks, and the scan came back out standing on the `%%` — which
/// then opened a comment and greyed the rest of the file. 2026-09-19:
/// 「markdown 中的百分號會把後面的所有文字變成註釋」. The same shape did it in
/// this project's own manual (`docs/manual_tc.md:394`, a line of quoted markup),
/// and the exporter — which strips comments — **dropped those paragraphs from
/// the exported file**.
///
/// Returns `(how many backticks opened it, where the closing run starts, where
/// it ends)`.
fn code_span(chars: &[char], at: usize) -> Option<(usize, usize, usize)> {
    let open = chars[at..].iter().take_while(|&&c| c == '`').count();
    if open == 0 {
        return None;
    }
    let mut i = at + open;
    while i < chars.len() {
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let run = chars[i..].iter().take_while(|&&c| c == '`').count();
        if run == open {
            return Some((open, i, i + run));
        }
        i += run;
    }
    None
}

/// The first index at or after `from` whose character satisfies `f`.
fn find(chars: &[char], from: usize, f: impl Fn(char) -> bool) -> Option<usize> {
    (from..chars.len()).find(|&i| f(chars[i]))
}

/// **和 `chars[from]` 配對的那一個 `shut` 在哪**，中間的同類括號要數進去。
///
/// `from` 指着開括號本身。回的是閉括號自己的下標，`to` 之前找不到就回 `None`。
///
/// Warning: **為什麼不是「往後找第一個」**（2026-09-28）：`[連結](…/中文_(消歧義))` 的地址裏
/// 有一對括號，找第一個 `)` 會在 `消歧義` 後面收口，末尾那個括號掉在正文上；
/// `[甲[乙]丙](x)` 同理，在第一個 `]` 上就斷了，整條鏈接不認。中文維基的地址裏帶括號
/// 是常事。
fn balanced(chars: &[char], from: usize, shut: char, to: usize) -> Option<usize> {
    let open = chars[from];
    let mut deep = 0usize;
    for at in from..to.min(chars.len()) {
        if escaped(chars, at) {
            continue;
        }
        if chars[at] == open {
            deep += 1;
        } else if chars[at] == shut {
            deep -= 1;
            if deep == 0 {
                return Some(at);
            }
        }
    }
    None
}

/// **The formula the cursor is in** (2026-11-06), as the float under the
/// cursor draws it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Maths {
    /// The formula, without its delimiters.
    pub source: String,
    /// Whether it is LaTeX (Markdown's maths) rather than Typst's own.
    pub latex: bool,
    /// Whether it is set on its own (`$$…$$`, Typst's `$ … $`) rather than
    /// in the run of the text.
    pub display: bool,
}

/// The construct a display-maths block's runs carry on every line
/// (2026-11-06), clear of the ones the line's own writing has — so the formula
/// can be told from a `*` beside it on the line that opens it.
pub const MATHS_BLOCK_CONSTRUCT: usize = usize::MAX / 4;

/// How many lines a display-maths block may run before it is taken for a
/// stray delimiter instead (2026-11-06). A blank line already ends one; this
/// bounds a paragraph that never has one.
const MATHS_BLOCK_LINES: usize = 400;

/// **Find the display maths that runs over several lines** (2026-11-06) and
/// label its lines [`Block::Math`].
///
/// Markdown's opens on a line that begins with `$$` and does not close it,
/// and closes at the next `$$`; Typst's opens at a `$` its line leaves
/// unpaired and closes at the next `$`. Either way **a blank line before the
/// close means it was never maths** — Pandoc ends a paragraph there, and a
/// stray dollar must not tint the rest of the chapter. Nothing is labelled
/// until the close is found, which a forward scan could not promise.
///
/// `has_dollar` is asked first, so a line without a `$` is never copied;
/// `text` hands over a line without its line ending.
pub fn maths_blocks(
    blocks: &mut [Block],
    typst: bool,
    has_dollar: impl Fn(usize) -> bool,
    text: impl Fn(usize) -> String,
) {
    let opens = |i: usize| has_dollar(i) && maths_opener(&text(i).chars().collect::<Vec<_>>(), typst).is_some();
    let blank = |j: usize| text(j).trim().is_empty();
    let closes = |j: usize| maths_closer(&text(j).chars().collect::<Vec<_>>(), 0, typst).is_some();
    maths_blocks_by(blocks, typst, opens, blank, closes);
}

/// A line's part in display maths, worked out once ([`maths_facts`]): it
/// holds a `$`.
pub const MATHS_DOLLAR: u8 = 1;
/// It may open a display-maths block ([`maths_opener`]).
pub const MATHS_OPENS: u8 = 2;
/// It may close one ([`maths_closer`]).
pub const MATHS_CLOSES: u8 = 4;
/// It is blank, which ends one unclosed.
pub const MATHS_BLANK: u8 = 8;

/// **What [`maths_blocks`] asks of a line, all at once**, for a scan that
/// keeps the answers line by line and asks again only for lines that
/// changed (2026-11-09). `line` is without its line ending.
pub fn maths_facts(line: &str, typst: bool) -> u8 {
    let mut facts = 0;
    if line.trim().is_empty() {
        facts |= MATHS_BLANK;
    }
    if line.contains('$') {
        facts |= MATHS_DOLLAR;
        let chars: Vec<char> = line.chars().collect();
        if maths_opener(&chars, typst).is_some() {
            facts |= MATHS_OPENS;
        }
        if maths_closer(&chars, 0, typst).is_some() {
            facts |= MATHS_CLOSES;
        }
    }
    facts
}

/// [`maths_blocks`], asking each line what [`maths_facts`] says of it.
pub fn maths_blocks_by(
    blocks: &mut [Block],
    typst: bool,
    opens: impl Fn(usize) -> bool,
    blank: impl Fn(usize) -> bool,
    closes: impl Fn(usize) -> bool,
) {
    let opens_here = |block: Block| match typst {
        true => matches!(block, Block::Prose | Block::Item { .. }),
        false => matches!(block, Block::Prose | Block::Item { .. } | Block::FootnoteDef),
    };
    // What may stand inside one: anything a formula's own lines can look like
    // (`- b`, `| x |`, `= y`), but not the structure a fence or a merge is.
    let walks = |block: Block| {
        !matches!(
            block,
            Block::Code { .. }
                | Block::FrontMatter
                | Block::Container(_)
                | Block::Conflict(_)
                | Block::Comment { .. }
        )
    };
    let mut i = 0;
    while i < blocks.len() {
        if !opens_here(blocks[i]) || !opens(i) {
            i += 1;
            continue;
        }
        let close = (i + 1..blocks.len().min(i + 1 + MATHS_BLOCK_LINES))
            .take_while(|&j| walks(blocks[j]))
            .take_while(|&j| !blank(j))
            .find(|&j| closes(j));
        let Some(last) = close else {
            i += 1;
            continue;
        };
        for (j, block) in blocks.iter_mut().enumerate().take(last + 1).skip(i) {
            *block = Block::Math { first: j == i, last: j == last };
        }
        i = last + 1;
    }
}

/// Where the opening delimiter of a display-maths block stands on its first
/// line, as a char range, or `None` when the line opens none.
fn maths_opener(chars: &[char], typst: bool) -> Option<std::ops::Range<usize>> {
    if typst {
        // The `$` the line leaves unpaired: one no span of the line covers,
        // so not in a comment, a raw span, a call or a closed `$…$`.
        let line: String = chars.iter().collect();
        let spans = typst::spans(&line);
        let at = (0..chars.len()).rev().find(|&i| {
            chars[i] == '$' && !escaped(chars, i) && !spans.iter().any(|s| s.start <= i && i < s.end)
        })?;
        return Some(at..at + 1);
    }
    let start = chars.iter().take_while(|c| c.is_whitespace()).count();
    let opens = chars.get(start) == Some(&'$') && chars.get(start + 1) == Some(&'$');
    (opens && maths_closer(chars, start + 2, false).is_none()).then_some(start..start + 2)
}

/// Where the closing delimiter of a display-maths block stands on a line,
/// looking from `from`: the first `$$` (Markdown) or `$` (Typst) no
/// backslash escapes.
fn maths_closer(chars: &[char], from: usize, typst: bool) -> Option<std::ops::Range<usize>> {
    let n = if typst { 1 } else { 2 };
    (from..chars.len())
        .find(|&i| {
            chars[i] == '$' && !escaped(chars, i) && (typst || chars.get(i + 1) == Some(&'$'))
        })
        .map(|i| i..i + n)
}

/// One line of a display-maths block, cut where its delimiters stand:
/// `(open, body, close)` as char ranges.
fn maths_parts(
    chars: &[char],
    first: bool,
    last: bool,
    typst: bool,
) -> (Option<std::ops::Range<usize>>, std::ops::Range<usize>, Option<std::ops::Range<usize>>) {
    let open = first.then(|| maths_opener(chars, typst)).flatten();
    let from = open.as_ref().map_or(0, |o| o.end);
    let close = last.then(|| maths_closer(chars, from, typst)).flatten();
    let to = close.as_ref().map_or(chars.len(), |c| c.start);
    (open, from..to.max(from), close)
}

/// **The runs of one line of a display-maths block** (2026-11-06): the
/// delimiters as markup, the formula as [`Kind::Math`], and whatever stands
/// before the opening delimiter or after the closing one read as the line's
/// own writing.
pub fn maths_block_spans(line: &str, first: bool, last: bool, typst: bool) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let (open, body, close) = maths_parts(&chars, first, last, typst);
    let read = |text: &str| match typst {
        true => typst::spans(text),
        false => spans(text),
    };
    let mine = MATHS_BLOCK_CONSTRUCT;
    let mut out = Vec::new();
    if let Some(open) = &open {
        let before: String = chars[..open.start].iter().collect();
        out.extend(read(&before));
        mark(&mut out, open.start, open.end, Kind::Marker, mine, 0);
    }
    mark(&mut out, body.start, body.end, Kind::Math, mine, 0);
    if let Some(close) = &close {
        mark(&mut out, close.start, close.end, Kind::Marker, mine, 0);
        let after: String = chars[close.end..].iter().collect();
        out.extend(read(&after).into_iter().map(|s| Span {
            start: s.start + close.end,
            end: s.end + close.end,
            construct: s.construct + mine + 1,
            ..s
        }));
    }
    out
}

/// **The formula a display-maths block holds** (2026-11-06): its lines from
/// the one that opens it to the one that closes it, less the delimiters.
pub fn maths_block_source(lines: &[String], typst: bool) -> String {
    let n = lines.len();
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let chars: Vec<char> = line.chars().collect();
            let (_, body, _) = maths_parts(&chars, i == 0, i + 1 == n, typst);
            chars[body].iter().collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// **The one-line formula a char column of `line` stands in** (2026-11-06),
/// delimiters included: `$a$`, `$$a$$`, Typst's `$a$` and `$ a $`. `runs`
/// are the line's runs as the page has them, so the answer is the formula
/// the page draws.
pub fn maths_at(line: &str, runs: &[Span], column: usize, typst: bool) -> Option<Maths> {
    // The run the column is in — the formula or one of its two delimiters,
    // which share its construct.
    let construct = runs
        .iter()
        .filter(|s| s.start <= column && column < s.end)
        .filter(|s| matches!(s.kind, Kind::Math | Kind::Marker))
        .max_by_key(|s| s.start)?
        .construct;
    let body = runs.iter().find(|s| s.kind == Kind::Math && s.construct == construct)?;
    let chars: Vec<char> = line.chars().collect();
    let source: String = chars[body.start..body.end].iter().collect();
    let display = match typst {
        // Typst sets it on its own when there is space inside both ends.
        true => source.starts_with(char::is_whitespace) && source.ends_with(char::is_whitespace),
        false => body.start >= 2 && chars[body.start - 2] == '$' && chars[body.start - 1] == '$',
    };
    Some(Maths { source: source.trim().to_string(), latex: !typst, display })
}

/// Push a span, dropping empty ones.
fn mark(
    out: &mut Vec<Span>,
    start: usize,
    end: usize,
    kind: Kind,
    construct: usize,
    depth: u8,
) {
    if end > start {
        out.push(Span {
            start,
            end,
            kind,
            construct,
            depth,
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
        scan(&chars, true, 0)
    }

    /// How deep `*…*` and `_…_` are read inside each other, as Markdown's
    /// [`super::DEPTH`].
    const DEPTH: u8 = 3;

    /// One pass over `chars`: a whole line, or the text inside a construct.
    /// Only a whole line can be a heading.
    fn scan(chars: &[char], line: bool, depth: u8) -> Vec<Span> {
        let mut out = Vec::new();
        let mut construct = 1usize;
        let mut at = 0usize;

        // A heading is `=` through `======`, and the rest of the line is it.
        // 判準在 `super::heading_marks`，三處共用一支。
        let heading = match line {
            true => super::heading_marks(&chars.iter().collect::<String>(), '='),
            false => None,
        };
        if let Some(equals) = heading {
            push(&mut out, 0, equals, Kind::HeadingMark, 0);
            push(&mut out, equals, chars.len(), Kind::Heading, 0);
            at = equals;
        }

        while at < chars.len() {
            // **`\*` 不是強調的開頭**（2026-09-28，同 Markdown 那一邊）。這個倉自己的
            // `export.rs` 正在生成 `\*`，導出的檔用自己的編輯器打開會亂。
            if chars[at] == '\\' && chars.get(at + 1).is_some_and(|&c| super::escapable(c)) {
                push(&mut out, at, at + 1, Kind::Marker, construct);
                construct += 1;
                at += 2;
                continue;
            }
            // **A bare `https://…` is a link**, as Typst reads it in writing; its
            // `//` drew the rest of the line as a comment (review, 2026-11-05).
            // It runs to a space, less the stop or comma after it.
            if (at == 0 || !chars[at - 1].is_alphanumeric())
                && ["https://", "http://"].iter().any(|p| chars[at..].starts_with(&p.chars().collect::<Vec<_>>()))
            {
                let mut end = (at..chars.len()).find(|&i| chars[i].is_whitespace()).unwrap_or(chars.len());
                while end > at && matches!(chars[end - 1], '.' | ',' | ';' | ':' | '!' | '?' | ')' | '。' | '，') {
                    end -= 1;
                }
                push(&mut out, at, end, Kind::Link, construct);
                construct += 1;
                at = end;
                continue;
            }
            // `// to the end of the line` is a note to oneself.
            if chars[at] == '/' && chars.get(at + 1) == Some(&'/') {
                push(&mut out, at, chars.len(), Kind::Comment, construct);
                break;
            }
            // `/* a note */`, to its `*/` or the end of the line: one left
            // open carries on below ([`comment_left_open`]). Its `*` used to
            // open a strong run.
            if chars[at] == '/' && chars.get(at + 1) == Some(&'*') {
                let end = block_comment_end(chars, at + 2).unwrap_or(chars.len());
                push(&mut out, at, end, Kind::Comment, construct);
                construct += 1;
                at = end;
                continue;
            }
            // `#import`, `#let`, `#show`, `#set` and every call: the
            // instructions, not the writing. Shown, always — a writer needs to
            // see what produces the page. `#(1 + 2)`, `#{…}` and `#[…]` are code
            // too, the last with writing inside (review, 2026-11-06).
            if chars[at] == '#'
                && chars.get(at + 1).is_some_and(|&c| c.is_alphabetic() || matches!(c, '(' | '{' | '['))
            {
                let (end, bodies) = code_parts(chars, at + 1);
                let mine = construct;
                construct += 1;
                // **方括號裏裝的是正文，不是代碼**（2026-09-28）。`#chapter[初雪]`、
                // `#quote[…]`、`#figure(caption: [說明])` 是中文 Typst 稿裏最常見的三
                // 種寫法，從前整塊畫成代碼色，等於把寫稿人寫的字藏起來。
                //
                // Warning: **這是官方那套模式切換裏唯一值得學的一層**：Typst 的 `[]` 從 code
                // 模式切回 markup 模式。再往裏（`#if x { [文字] }` 這種）只出現在模板檔
                // 裏，不出現在正文裏，而模板整塊灰掉反而好讀。
                //
                // Warning: **代碼色只蓋代碼那兩截**（`#chapter[` 和 `]`），中間**不蓋**。
                // 蓋了的話裏面的字還是灰的——正文没有自己的 span，它就是「没被蓋到的
                // 那些格子」，所以要真的讓開，不能靠往上再壓一層。
                let mut cut = at;
                for (a, b) in bodies {
                    push(&mut out, cut, a, Kind::Code2, mine);
                    graft(&mut out, scan(&chars[a..b], true, depth + 1), a, &mut construct);
                    cut = b;
                }
                push(&mut out, cut, end, Kind::Code2, mine);
                at = end;
                continue;
            }
            // `$maths$`.
            if chars[at] == '$' {
                if let Some(close) = (at + 1..chars.len()).find(|&i| chars[i] == '$') {
                    push(&mut out, at, at + 1, Kind::Marker, construct);
                    push(&mut out, at + 1, close, Kind::Math, construct);
                    push(&mut out, close, close + 1, Kind::Marker, construct);
                    at = close + 1;
                    construct += 1;
                    continue;
                }
            }
            // **`@定理一`** — 交叉引用（2026-09-28）。Warning: 一個孤零零的 `@` 不是引用。
            if chars[at] == '@' {
                let mut end = (at + 1..chars.len())
                    .take_while(|&i| ref_name(chars[i]))
                    .last()
                    .map(|i| i + 1);
                // A `.` or `:` at the end ends the sentence, not the name, as
                // typst reads it: `See @intro.` refers to `intro` (review,
                // 2026-11-06).
                while let Some(e) = end.filter(|&e| e > at + 1 && matches!(chars[e - 1], '.' | ':')) {
                    end = Some(e - 1);
                }
                let end = end.filter(|&e| e > at + 1);
                if let Some(end) = end {
                    push(&mut out, at, end, Kind::Ref, construct);
                    at = end;
                    construct += 1;
                    continue;
                }
            }
            // **`<定理一>`** — 標籤，被 `@` 指的那一頭。
            if chars[at] == '<' {
                let close = (at + 1..chars.len())
                    .find(|&i| chars[i] == '>')
                    .filter(|&c| c > at + 1 && (at + 1..c).all(|i| ref_name(chars[i])));
                if let Some(close) = close {
                    push(&mut out, at, close + 1, Kind::Label, construct);
                    at = close + 1;
                    construct += 1;
                    continue;
                }
            }
            // `*粗*` and `_斜_` — one delimiter, not two.
            if matches!(chars[at], '*' | '_') && !in_word(chars, at) {
                let kind = if chars[at] == '*' {
                    Kind::Strong
                } else {
                    Kind::Emphasis
                };
                if let Some(close) = closing(chars, at + 1, chars[at]) {
                    let mine = construct;
                    construct += 1;
                    push(&mut out, at, at + 1, Kind::Marker, mine);
                    push(&mut out, at + 1, close, kind, mine);
                    // `*粗的 _斜的_*`: the text inside is read again, so both
                    // styles land on 斜的. Before the closing marker, to keep
                    // the runs in order.
                    if depth + 1 < DEPTH {
                        graft(&mut out, scan(&chars[at + 1..close], false, depth + 1), at + 1, &mut construct);
                    }
                    push(&mut out, close, close + 1, Kind::Marker, mine);
                    at = close + 1;
                    continue;
                }
            }
            // `` `code` ``. Counted in backticks — see `super::code_span`;
            // without that ``` ``//`` ``` opened a Typst comment.
            if chars[at] == '`' {
                if let Some((open, close, end)) = super::code_span(chars, at) {
                    push(&mut out, at, at + open, Kind::Marker, construct);
                    push(&mut out, at + open, close, Kind::Code, construct);
                    push(&mut out, close, end, Kind::Marker, construct);
                    at = end;
                    construct += 1;
                    continue;
                }
            }
            at += 1;
        }
        out
    }

    /// `inner`, read from `offset` of the run it came out of, laid into `out`
    /// with constructs of its own.
    fn graft(out: &mut Vec<Span>, inner: Vec<Span>, offset: usize, construct: &mut usize) {
        let top = inner.iter().map(|s| s.construct).max().unwrap_or(0);
        for mut one in inner {
            one.start += offset;
            one.end += offset;
            one.construct += *construct;
            one.depth += 1;
            out.push(one);
        }
        *construct += top + 1;
    }

    /// Where the `/*` whose body starts at `from` ends, past its `*/`. Typst's
    /// comments nest, so `/* a /* b */ c */` is one; counted on this line only.
    fn block_comment_end(chars: &[char], from: usize) -> Option<usize> {
        comment_walk(chars, from, 1).ok()
    }

    /// Walk a comment `deep` levels deep from `from`: where it ends, past its
    /// last `*/`, or how deep it still is at the end of the line. The pairs
    /// are read as Typst's lexer reads them, so the `*` of a `*/` is not the
    /// start of a `*/`'s neighbour `/*`.
    fn comment_walk(chars: &[char], from: usize, mut deep: usize) -> Result<usize, usize> {
        let mut at = from;
        while at + 1 < chars.len() {
            match (chars[at], chars[at + 1]) {
                ('/', '*') => deep += 1,
                ('*', '/') => deep -= 1,
                _ => {
                    at += 1;
                    continue;
                }
            }
            at += 2;
            if deep == 0 {
                return Ok(at);
            }
        }
        Err(deep)
    }

    /// Whether `line` opens a `/*` it does not close, read as [`spans`] reads
    /// it: a `/*` in a raw span or a call's string is not one.
    pub fn comment_left_open(line: &str) -> bool {
        comment_depth_left(line) > 0
    }

    /// **How many `/*` deep `line` leaves a comment it opens** — 0 when it
    /// closes what it opens. Typst's comments nest (review, 2026-11-09):
    /// `/* 刪去的一章` then `/* 舊註 */` on the next line is still one level
    /// deep, and the chapter below it is still the comment's.
    pub fn comment_depth_left(line: &str) -> usize {
        let chars: Vec<char> = line.chars().collect();
        spans(line)
            .iter()
            .filter(|s| s.kind == Kind::Comment && chars[s.start..].starts_with(&['/', '*']))
            .find_map(|s| comment_walk(&chars, s.start + 2, 1).err())
            .unwrap_or(0)
    }

    /// **Where on `line` a comment left open `deep` levels deep above
    /// closes**: the char index of the `*/` that closes it, or how deep it
    /// still is when the line ends.
    pub fn comment_closes(line: &str, deep: usize) -> Result<usize, usize> {
        let chars: Vec<char> = line.chars().collect();
        comment_walk(&chars, 0, deep).map(|end| end - 2)
    }

    /// What a line of Typst is inside when it begins: one entry per bracket
    /// still open above it, innermost last, and the `#` that opened them.
    ///
    /// **Typst's modes, switched where Typst switches them** (2026-11-08). In
    /// a `{` or a `(` every line is code until it closes, blank lines and all;
    /// a `[` inside code is writing again; a `#` inside writing is code again.
    /// The scanner used to count brackets in any order, strings and comments
    /// included, and gave up at the first blank line — so the body of
    /// `#let style(body) = { … }` came out as one flat purple down to its
    /// first blank line and as plain writing after it, and only the
    /// `[#box(…)]` inside it, which the line scan sees for itself, was drawn
    /// as code.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Frame {
        /// `{`: a code block.
        Brace,
        /// `(`: arguments, a group, an array or a dictionary — code.
        Paren,
        /// `[` opened in code: content, which is writing.
        Bracket,
        /// A `[` written in writing, which Typst keeps in step with the `]`s
        /// so that only an unmatched one closes the content around it.
        Text,
        /// A `/*` that code leaves open. One left open in writing is carried
        /// by the editor's note scan instead ([`comment_left_open`]), which
        /// is never shown the lines of a code body.
        Note,
        /// `#let`, `#set`, `#show`, `#import`, `#include` or `#context` in
        /// writing: code to the end of its line, or to a `;`.
        Statement,
        /// `#name` in writing: code for as long as a bracket or a `.field`
        /// follows it.
        Call,
        /// `#if`, `#for` or `#while` in writing: code up to its block, and on
        /// past an `else`; `true` once a block has opened.
        Head(bool),
    }

    /// Whether the scan is in writing when `frames` are open.
    fn writing(frames: &[Frame]) -> bool {
        matches!(frames.last(), None | Some(Frame::Bracket | Frame::Text))
    }

    /// What one character of a line is, as [`walk`] reads it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Piece {
        Code,
        Comment,
        Writing,
    }

    /// **What runs on from one line of Typst to the next** besides brackets
    /// and notes (review, 2026-11-09).
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    struct Open {
        /// The backticks that opened a raw text still open, 0 outside one.
        ///
        /// **Typst's raw rules, not a fence's**: a raw text opens at any
        /// backtick in writing or code, a raw block on a line of its own
        /// being only the usual case, and closes at the first run of as many
        /// backticks wherever it stands ([`raw_at`]). This used to be a
        /// switch thrown by a line that began with ```, so a ``` at the end
        /// of `let s = "字";```` closed a block for Typst and not for the
        /// scan, and everything after was read the wrong way up.
        raw: usize,
        /// An equation is open: a `$` in writing that its line does not
        /// close. Typst's maths runs to the next `$`, over blank lines too,
        /// and a backtick in it is a symbol, not raw text. The page draws
        /// only what [`super::maths_blocks`] finds as display maths; this
        /// is for where a piece may begin ([`BlockScanner::in_math`]).
        math: bool,
    }

    /// Where an equation whose maths starts at `from` closes on its line:
    /// past its `$`. An escaped `$`, one in a string and one in a comment
    /// are the maths' own; a `//` or a `/*` left open runs past the line.
    fn math_end(chars: &[char], from: usize) -> Option<usize> {
        let n = chars.len();
        let mut at = from;
        while at < n {
            match chars[at] {
                '\\' => at += 2,
                '"' => {
                    at += 1;
                    while at < n && chars[at] != '"' {
                        at += if chars[at] == '\\' { 2 } else { 1 };
                    }
                    at += 1;
                }
                '/' if chars.get(at + 1) == Some(&'/') => return None,
                '/' if chars.get(at + 1) == Some(&'*') => at = block_comment_end(chars, at + 2)?,
                '$' => return Some(at + 1),
                _ => at += 1,
            }
        }
        None
    }

    /// **A raw text that starts at the backtick at `at`**, read as Typst's
    /// lexer reads it (`raw` in `lexer.rs`; review, 2026-11-09): the run of
    /// backticks that opens it, two being an empty raw on their own, ends
    /// at the first run of as many backticks — anywhere, mid-line too, a
    /// longer run closing with its first ones and leaving the rest to open
    /// another. A language tag after three or more is inside it and changes
    /// nothing about where it ends. Where it ends on this line, and 0; or
    /// the end of the line and the backticks still to be met on another.
    fn raw_at(chars: &[char], at: usize) -> (usize, usize) {
        let ticks = chars[at..].iter().take_while(|&&c| c == '`').count();
        if ticks == 2 {
            return (at + 2, 0);
        }
        match raw_end(chars, at + ticks, ticks) {
            Some(end) => (end, 0),
            None => (chars.len(), ticks),
        }
    }

    /// Where a raw text opened by `ticks` backticks ends on a line read from
    /// `from`: past the first run of `ticks` backticks in a row.
    fn raw_end(chars: &[char], from: usize, ticks: usize) -> Option<usize> {
        let mut found = 0;
        for (at, &c) in chars.iter().enumerate().skip(from) {
            found = if c == '`' { found + 1 } else { 0 };
            if found == ticks {
                return Some(at + 1);
            }
        }
        None
    }

    /// Read one line from inside `frames`, and leave them as the line leaves
    /// them: what each character is.
    ///
    /// Writing is only walked through here — its runs are [`scan`]'s — but
    /// the things in it that hide a `#` or a `]` are stepped over whole, the
    /// way `scan` reads them: an escape, a link, a comment, a raw span, maths.
    /// Code skips its strings and comments, so the `(` in `"(甲)"` or in
    /// `// (說明` opens nothing.
    ///
    /// A raw text is read by Typst's rules ([`raw_at`]); one the line leaves
    /// open sets `raw` to the backticks that opened it, and is the rest of
    /// the line.
    fn walk(chars: &[char], frames: &mut Vec<Frame>, open: &mut Open) -> Vec<Piece> {
        let n = chars.len();
        let mut out = vec![Piece::Writing; n];
        let word_end = |from: usize| {
            (from..n).find(|&i| !(chars[i].is_alphanumeric() || chars[i] == '_')).unwrap_or(n)
        };
        let mut at = 0usize;
        while at < n {
            let c = chars[at];
            let next = chars.get(at + 1).copied();
            let top = frames.last().copied();
            let mut piece = Piece::Code;
            let mut end = at + 1;
            match top {
                // Inside a `/*`: to its `*/`. Typst's comments nest.
                Some(Frame::Note) => {
                    piece = Piece::Comment;
                    if c == '*' && next == Some('/') {
                        frames.pop();
                        end = at + 2;
                    } else if c == '/' && next == Some('*') {
                        frames.push(Frame::Note);
                        end = at + 2;
                    }
                }
                // After `#name`, `#name(…)` or `#name[…]`: another bracket or
                // a `.field` goes on with it, and anything else ends it.
                Some(Frame::Call) => match c {
                    '(' => frames.push(Frame::Paren),
                    '[' => frames.push(Frame::Bracket),
                    '{' => frames.push(Frame::Brace),
                    '.' if next.is_some_and(|d| d.is_alphabetic() || d == '_') => end = word_end(at + 1),
                    _ => {
                        frames.pop();
                        continue;
                    }
                },
                // After `#if x [甲]`: an `else` goes on with it.
                Some(Frame::Head(true)) => {
                    let word = (at..n).find(|&i| chars[i] != ' ').unwrap_or(n);
                    let word_ends = word_end(word);
                    if chars[word..word_ends].iter().copied().eq("else".chars()) {
                        if let Some(head) = frames.last_mut() {
                            *head = Frame::Head(false);
                        }
                        end = word_ends;
                    } else {
                        frames.pop();
                        continue;
                    }
                }
                None | Some(Frame::Bracket | Frame::Text) => {
                    piece = Piece::Writing;
                    if c == '\\' && next.is_some_and(super::escapable) {
                        end = at + 2;
                    } else if (at == 0 || !chars[at - 1].is_alphanumeric())
                        && ["https://", "http://"].iter().any(|p| chars[at..].starts_with(&p.chars().collect::<Vec<_>>()))
                    {
                        end = (at..n).find(|&i| chars[i].is_whitespace()).unwrap_or(n);
                    } else if c == '/' && next == Some('/') {
                        end = n;
                    } else if c == '/' && next == Some('*') {
                        end = block_comment_end(chars, at + 2).unwrap_or(n);
                    } else if c == '`' {
                        (end, open.raw) = raw_at(chars, at);
                    } else if c == '$' {
                        (end, open.math) = match math_end(chars, at + 1) {
                            Some(close) => (close, false),
                            None => (n, true),
                        };
                    } else if c == '#' && next.is_some_and(|d| d.is_alphabetic() || matches!(d, '(' | '{' | '[')) {
                        piece = Piece::Code;
                        end = word_end(at + 1);
                        let word: String = chars[at + 1..end].iter().collect();
                        frames.push(match word.as_str() {
                            "let" | "set" | "show" | "import" | "include" | "context" => Frame::Statement,
                            "if" | "for" | "while" => Frame::Head(false),
                            _ => Frame::Call,
                        });
                    } else if c == '[' && top.is_some() {
                        frames.push(Frame::Text);
                    } else if c == ']' && top == Some(Frame::Text) {
                        frames.pop();
                    } else if c == ']' && top == Some(Frame::Bracket) {
                        piece = Piece::Code;
                        frames.pop();
                    }
                }
                // Code: a `{` or `(` body, a statement, or the head of an `if`.
                Some(_) => {
                    if c == '"' {
                        end = at + 1;
                        while end < n && chars[end] != '"' {
                            end += if chars[end] == '\\' { 2 } else { 1 };
                        }
                        end = (end + 1).min(n);
                    } else if c == '/' && next == Some('/') {
                        piece = Piece::Comment;
                        end = n;
                    } else if c == '/' && next == Some('*') {
                        piece = Piece::Comment;
                        frames.push(Frame::Note);
                        end = at + 2;
                    } else if c == '`' {
                        (end, open.raw) = raw_at(chars, at);
                    } else if c == '$' {
                        // An equation in code, `let x = $a ` b$`: Typst's
                        // lexer reads maths to its `$` here as in writing, so
                        // a backtick in it is a symbol and its brackets open
                        // and close nothing of the code around it.
                        (end, open.math) = match math_end(chars, at + 1) {
                            Some(close) => (close, false),
                            None => (n, true),
                        };
                    } else if matches!(c, '{' | '[' | '(') {
                        // An `if`'s head ends at its block.
                        if c != '(' && top == Some(Frame::Head(false)) {
                            if let Some(head) = frames.last_mut() {
                                *head = Frame::Head(true);
                            }
                        }
                        frames.push(match c {
                            '{' => Frame::Brace,
                            '[' => Frame::Bracket,
                            _ => Frame::Paren,
                        });
                    } else if let Some(opener) = match c {
                        '}' => Some(Frame::Brace),
                        ')' => Some(Frame::Paren),
                        ']' => Some(Frame::Bracket),
                        _ => None,
                    } {
                        close(frames, opener);
                    } else if c == ';' && top == Some(Frame::Statement) {
                        frames.pop();
                    }
                }
            }
            let end = end.clamp(at + 1, n);
            for slot in &mut out[at..end] {
                *slot = piece;
            }
            at = end;
        }
        // A `#` expression in writing ends with its line; a bracket does not,
        // nor does an equation it opened (`#let x = $a` on one line, `b$` on
        // the next): the expression goes on past the `$` that closes it.
        while !open.math && matches!(frames.last(), Some(Frame::Statement | Frame::Call | Frame::Head(_))) {
            frames.pop();
        }
        out
    }

    /// Close the innermost `opener`, and whatever code was left open inside
    /// it. A closer with writing between it and its opener is a stray, and
    /// closes nothing.
    fn close(frames: &mut Vec<Frame>, opener: Frame) {
        for i in (0..frames.len()).rev() {
            if frames[i] == opener {
                frames.truncate(i);
                return;
            }
            if matches!(frames[i], Frame::Bracket | Frame::Text) {
                return;
            }
        }
    }

    /// The marked-up runs of a line of Typst that begins inside `frames` —
    /// what [`BlockScanner::frames`] said just before it was fed the line.
    ///
    /// Code is [`Kind::Code2`], whose tokens the screen colours; a comment in
    /// it is a comment; writing is read by [`scan`], as any other. On a line
    /// that begins in code the writing also takes a plain token, so the `　`
    /// in `[　]` is drawn in the page's ink rather than the code block's.
    pub fn spans_from(line: &str, frames: &[Frame]) -> Vec<Span> {
        if frames.is_empty() {
            return spans(line);
        }
        let chars: Vec<char> = line.chars().collect();
        let pieces = walk(&chars, &mut frames.to_vec(), &mut Open::default());
        let code = !writing(frames);
        let mut out = Vec::new();
        let mut construct = 1usize;
        let mut at = 0usize;
        while at < chars.len() {
            let piece = pieces[at];
            let end = (at..chars.len()).find(|&i| pieces[i] != piece).unwrap_or(chars.len());
            match piece {
                Piece::Code => push(&mut out, at, end, Kind::Code2, construct),
                Piece::Comment => push(&mut out, at, end, Kind::Comment, construct),
                Piece::Writing => {
                    if code {
                        push(&mut out, at, end, Kind::Token(crate::code::Token::Plain), construct);
                    }
                    construct += 1;
                    graft(&mut out, scan(&chars[at..end], at == 0, 0), at, &mut construct);
                }
            }
            construct += 1;
            at = end;
        }
        out
    }

    /// Which block a line of Typst belongs to.
    ///
    /// Far less state than Markdown's: Typst has no `:::` and its raw blocks
    /// use the same ``` fence, so this is a small scanner of its own.
    #[derive(Debug, Default, Clone, PartialEq, Eq)]
    pub struct BlockScanner {
        /// A raw text or an equation the lines above left open ([`Open`]).
        open: Open,
        /// What the scan is inside, innermost last ([`Frame`]).
        ///
        /// Typst code runs across lines — `#show heading: it => block({` opens
        /// a body that closes several lines later — and every one of those
        /// lines is code, not writing. The lines inside a `[` are writing.
        frames: Vec<Frame>,
        /// A blank line has gone by inside a code body, and the next line
        /// decides whether the body goes on.
        blank: bool,
    }

    impl BlockScanner {
        pub fn new() -> BlockScanner {
            BlockScanner::default()
        }

        /// What the next line begins inside: hand it to [`spans_from`].
        pub fn frames(&self) -> &[Frame] {
            &self.frames
        }

        /// Whether the next line begins inside a raw text.
        pub fn in_raw(&self) -> bool {
            self.open.raw > 0
        }

        /// Whether the next line begins inside an equation.
        pub fn in_math(&self) -> bool {
            self.open.math
        }

        /// What follows a raw text that closed mid-line: in code, walked
        /// as code is; in writing, for its raw texts alone.
        fn walk_rest(&mut self, chars: &[char]) {
            match self.frames.is_empty() {
                true => walk(chars, &mut Vec::new(), &mut self.open),
                false => walk(chars, &mut self.frames, &mut self.open),
            };
        }

        /// [`Self::feed_line`], with nothing but the line's opening at hand:
        /// a line longer than that is not walked.
        pub fn feed(&mut self, prefix: &str, len: usize) -> Block {
            self.feed_line(prefix, len, || None)
        }

        /// The block of the next line, from its first [`super::PREFIX`]
        /// characters (`prefix`) and its length; `whole` hands over the whole
        /// line when the opening is not all of it and the line has to be
        /// walked — the first line of a body, or one inside it, or one with
        /// a backtick in it.
        pub fn feed_line(&mut self, prefix: &str, len: usize, whole: impl FnOnce() -> Option<String>) -> Block {
            self.feed_line_ticked(prefix, len, None, whole)
        }

        /// [`Self::feed_line`], told whether the line holds a backtick or a
        /// `$` at all (`ticks`), so that a paragraph without one is never
        /// copied to look for one. `None` looks.
        pub fn feed_line_ticked(
            &mut self,
            prefix: &str,
            len: usize,
            ticks: Option<bool>,
            whole: impl FnOnce() -> Option<String>,
        ) -> Block {
            let line = prefix.trim_end_matches(['\n', '\r']);
            let trimmed = line.trim_start();
            let mut whole = Some(whole);
            let mut text = |line: &str| -> Option<Vec<char>> {
                match len <= super::PREFIX {
                    true => Some(line.chars().collect()),
                    false => whole.take().and_then(|w| w()).map(|l| l.trim_end_matches(['\n', '\r']).chars().collect()),
                }
            };
            // Inside a raw text: to the run of backticks that closes it, and
            // what follows that read as any line's rest.
            if self.open.raw > 0 {
                let Some(chars) = text(line) else { return Block::Code { inside: None } };
                if let Some(end) = raw_end(&chars, 0, self.open.raw) {
                    self.open.raw = 0;
                    self.walk_rest(&chars[end..]);
                }
                return Block::Code { inside: None };
            }
            // Inside an equation: to its `$`. The line is maths, not a
            // heading or an item, whatever it begins with.
            if self.open.math {
                let code = !writing(&self.frames);
                if let Some(chars) = text(line) {
                    if let Some(end) = math_end(&chars, 0) {
                        self.open.math = false;
                        self.walk_rest(&chars[end..]);
                    }
                }
                return match code {
                    true => Block::Code { inside: None },
                    false => Block::Prose,
                };
            }
            let fence = trimmed.starts_with("```");
            if trimmed.is_empty() {
                self.blank = !self.frames.is_empty();
                return Block::Prose;
            }
            // **A blank line, then a line that is not indented, closes a code
            // body** (2026-11-08). The count is a guess, and a guess that has
            // gone wrong must not run to the end of the file — one line used
            // to be able to tint an entire manuscript. It used to be any blank
            // line, which cut every template's `#let style(body) = { … }` in
            // two at the first one; a body's lines are indented, the writing
            // after a stray `(` is not. A full-width `　　` is a paragraph's
            // indent, not code's, so only ASCII counts.
            if std::mem::take(&mut self.blank)
                && matches!(self.frames.last(), Some(Frame::Brace | Frame::Paren))
                && !line.starts_with([' ', '\t'])
                && !trimmed.starts_with(['}', ')', ']'])
            {
                self.frames.clear();
            }
            let code = !writing(&self.frames);
            // Inside a `[` the line is writing, but for the `]` that ends it.
            let closes = !self.frames.is_empty() && trimmed.starts_with(']');
            // **Only a line seen whole may be counted.** The caller hands over
            // the first [`PREFIX`] characters, not the line — so
            // `#set par(first-line-indent: (amount: 2em, all: true), …)` had
            // its closing `)` cut off, left `depth` at 1, and made every line
            // after it code, blank ones included (2026-09-08). A long line is
            // walked only when `whole` can hand it over; otherwise it opens
            // and closes nothing.
            //
            // A line of writing with a backtick in it is walked for its raw
            // texts alone: the brackets it leaves open are not counted, as
            // they never were on a line that does not begin with `#`.
            let ticked = ticks.unwrap_or_else(|| line.contains(['`', '$']) || len > super::PREFIX);
            if !self.frames.is_empty() || trimmed.starts_with('#') {
                if let Some(chars) = text(line) {
                    walk(&chars, &mut self.frames, &mut self.open);
                }
            } else if ticked {
                if let Some(chars) = text(line) {
                    walk(&chars, &mut Vec::new(), &mut self.open);
                }
            }
            if fence || code || closes {
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
    /// [`code_end`]，外加**方括號裏那幾段**的範圍——那幾段是正文，要當標記再掃一遍。
    ///
    /// 只收最外面那一層（`[甲 [乙] 丙]` 收整段，裏面那一對交給遞歸）。
    ///
    /// Warning: **字符串字面量裏的方括號不算**：`#f("]")` 裏那個不是括號。這是報告裏說的
    /// 「真要修就只修這一小步，別往上走」——完整的 Typst 表達式解析器換不來什麼。
    // 走的是 `from..end` 這一段，而 `at` 是要記進 `bodies` 的絕對位置。
    #[allow(clippy::needless_range_loop)]
    fn code_parts(chars: &[char], from: usize) -> (usize, Vec<(usize, usize)>) {
        let end = code_end(chars, from);
        let mut bodies = Vec::new();
        let (mut quoted, mut deep, mut opened) = (false, 0usize, 0usize);
        for at in from..end.min(chars.len()) {
            match chars[at] {
                '"' => quoted = !quoted,
                _ if quoted => {}
                '[' => {
                    if deep == 0 {
                        opened = at;
                    }
                    deep += 1;
                }
                ']' if deep > 0 => {
                    deep -= 1;
                    if deep == 0 && at > opened + 1 {
                        bodies.push((opened + 1, at));
                    }
                }
                _ => {}
            }
        }
        (end, bodies)
    }

    fn code_end(chars: &[char], from: usize) -> usize {
        let mut at = from;
        // A `.` goes on only into a field or method (`#w.len()`); the one
        // in `第 #w. 項` ends the sentence, as Typst reads it (review,
        // 2026-11-06).
        let goes_on = |i: usize| chars.get(i + 1).is_some_and(|&c| c.is_alphanumeric() || c == '_');
        while at < chars.len()
            && (chars[at].is_alphanumeric() || chars[at] == '_' || (chars[at] == '.' && goes_on(at)))
        {
            at += 1;
        }
        // A statement — `#import "lib.typ": chapter`, `#let x = 1` — runs to
        // the end of its line. Only a *call* stops at its brackets.
        let word: String = chars[from..at].iter().collect();
        if matches!(word.as_str(), "import" | "include" | "let" | "set" | "show") {
            return chars.len();
        }
        // `#if x [甲] else [乙]`, `#for x in xs [丙]`, `#while c {…}`: not a
        // call, so the brackets do not follow the name. The head runs to the
        // first block, and an `else` takes another.
        if matches!(word.as_str(), "if" | "for" | "while") {
            return keyword_end(chars, at);
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

    /// Where `#if`, `#for` or `#while` ends, from just after the keyword.
    fn keyword_end(chars: &[char], mut at: usize) -> usize {
        loop {
            let (mut deep, mut quoted) = (0usize, false);
            while at < chars.len() {
                match chars[at] {
                    '"' => quoted = !quoted,
                    _ if quoted => {}
                    '(' => deep += 1,
                    ')' => deep = deep.saturating_sub(1),
                    '[' | '{' if deep == 0 => break,
                    _ => {}
                }
                at += 1;
            }
            // A block left open runs on below; the line is all head.
            let Some(open) = chars.get(at).copied() else {
                return chars.len();
            };
            let shut = if open == '[' { ']' } else { '}' };
            let mut deep = 0usize;
            let close = (at..chars.len()).find(|&i| {
                deep += usize::from(chars[i] == open);
                deep -= usize::from(chars[i] == shut);
                deep == 0
            });
            let Some(close) = close else {
                return chars.len();
            };
            at = close + 1;
            let next = (at..chars.len()).find(|&i| chars[i] != ' ').unwrap_or(chars.len());
            let word: String = chars[next..].iter().take_while(|c| c.is_alphanumeric()).collect();
            if word != "else" {
                return at;
            }
            at = next + 4;
        }
    }

    /// 一個引用名裏許不許出現這個字符。
    ///
    /// Warning: 收漢字：Typst 的標籤名可以是中文，而這個編輯器是給寫中文的人用的。
    fn ref_name(c: char) -> bool {
        c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
    }

    /// Where the run closing the delimiter opened at `from` is.
    ///
    /// **Typst 官方那條「詞內不算分隔符」**（`typst-syntax/src/lexer.rs:628` 的
    /// `in_word`，2026-09-28 照抄）。
    ///
    /// 一個 `*` 或 `_` 前後都貼着**西文**字母或數字的時候，它是那個詞的一部分，不是分隔
    /// 符——`snake_case` 不該畫成斜體，`a*b*c` 不該畫成粗體。
    ///
    /// Warning: **漢字、假名、諺文明確排除在「詞內」之外**，這是官方自己寫的，不是我們加的。
    /// 所以 `這*是*重點` 照樣是強調，而中文這一側一個字符都不用另立規矩。
    fn in_word(chars: &[char], at: usize) -> bool {
        let wordy = |c: Option<&char>| {
            c.is_some_and(|&c| c.is_alphanumeric() && !cjk_script(c))
        };
        wordy(at.checked_sub(1).and_then(|i| chars.get(i))) && wordy(chars.get(at + 1))
    }

    /// 漢字、平假名、片假名、諺文。
    ///
    /// Warning: 自己數碼位，不拉 `unicode-script` 進來：這個倉只需要「是不是這四種」這一個
    /// 問題，而那個 crate 帶着一整張 Script 表。漢字那一半走 `yumete_cjk::is_han`，
    /// 「一個概念一處權威」。
    fn cjk_script(c: char) -> bool {
        yumete_cjk::is_han(c)
            || matches!(c as u32,
                0x3040..=0x30FF      // 平假名、片假名
                | 0x31F0..=0x31FF    // 片假名語音擴展
                | 0xFF66..=0xFF9D    // 半角片假名
                | 0x1100..=0x11FF    // 諺文字母
                | 0x3130..=0x318F    // 諺文兼容字母
                | 0xAC00..=0xD7AF)   // 諺文音節
    }

    fn closing(chars: &[char], from: usize, delimiter: char) -> Option<usize> {
        // Warning: **空白那兩道閘去掉了**（2026-09-28）。從前是「開標記後面不許是空格、閉標記
        // 前面不許是空格」，那是照 Markdown 抄的；Typst 没有這條規矩，`* 文*` 在官方
        // 那裏就是強調。判準換成官方唯一的那一條：[`in_word`]。
        (from..chars.len()).find(|&i| chars[i] == delimiter && !in_word(chars, i))
    }

    fn push(out: &mut Vec<Span>, start: usize, end: usize, kind: Kind, construct: usize) {
        if end > start {
            out.push(Span {
                start,
                end,
                kind,
                construct,
                depth: 0,
            });
        }
    }
}

#[cfg(test)]
mod typst_tests {
    use super::typst::*;
    use super::{hidden, Block, Kind};

    /// **A sentence's full stop is not the reference's** (review,
    /// 2026-11-06): typst reads `See @intro.` as `@intro`.
    #[test]
    fn a_reference_leaves_the_full_stop_to_the_sentence() {
        let refs = |line: &str| -> Vec<(usize, usize)> {
            spans(line).into_iter().filter(|s| s.kind == Kind::Ref).map(|s| (s.start, s.end)).collect()
        };
        assert_eq!(refs("See @intro."), [(4, 10)]);
        assert_eq!(refs("See @fig:a: here"), [(4, 10)]);
        assert_eq!(refs("See @sec.two now"), [(4, 12)], "a dot inside the name stays");
    }

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
                Kind::GoneRow => '<',
                Kind::AddedRow => '>',
                Kind::Math => '$',
                Kind::Ref => '@',
                Kind::Label => '<',
                Kind::Link => 'L',
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

    /// **公式、交叉引用、標籤各有自己的 `Kind`**（2026-09-28）。
    ///
    /// Warning: 公式從前借 `Kind::Code`，`:export` 因此把 `$a + b$` 導成 `` `a + b` ``——
    /// 一條反引號在 Typst 裏是原樣文本，公式就此不是公式了。
    #[test]
    fn maths_and_references_are_not_code() {
        assert_eq!(shape("$a + b$"), ".$$$$$.");
        assert_eq!(shape("見 @定理一 那一節"), "  @@@@    ");
        assert_eq!(shape("定理 <定理一>"), "   <<<<<");
        // 一個孤零零的 `@` 不是引用，帶空格的尖括號也不是標籤。
        assert_eq!(shape("a @ b"), "     ");
        assert_eq!(shape("1 < 2 > 3"), "         ");
    }

    /// **Typst's `$` … `$` over several lines is maths** (2026-11-06): it
    /// opens at the `$` its line leaves unpaired and closes at the next `$`.
    #[test]
    fn typst_maths_over_several_lines_is_maths() {
        use super::{maths_at, maths_block_source, maths_blocks, Maths};
        let doc = ["見 $x$ 與 $", "  a + b", "  = c $ 下文", "後文 $y$"];
        let mut blocks = vec![Block::Prose; doc.len()];
        maths_blocks(&mut blocks, true, |i| doc[i].contains('$'), |i| doc[i].to_string());
        assert_eq!(blocks[0], Block::Math { first: true, last: false });
        assert_eq!(blocks[1], Block::Math { first: false, last: false });
        assert_eq!(blocks[2], Block::Math { first: false, last: true });
        assert_eq!(blocks[3], Block::Prose, "a closed `$y$` opens nothing");
        let shape = |line, first, last| super::tests::block_shape(line, first, last, true);
        // `= c` is the formula going on, not a heading.
        assert_eq!(shape("  = c $ 下文", false, true), "$$$$$$.   ");
        assert_eq!(shape("  a + b", false, false), "$$$$$$$");
        let lines: Vec<String> = doc[..3].iter().map(|s| s.to_string()).collect();
        assert_eq!(maths_block_source(&lines, true), "\n  a + b\n  = c ");
        // A blank line ends the guess.
        let doc = ["$ a", "", "b $"];
        let mut blocks = vec![Block::Prose; doc.len()];
        maths_blocks(&mut blocks, true, |i| doc[i].contains('$'), |i| doc[i].to_string());
        assert!(blocks.iter().all(|b| *b == Block::Prose));
        // One-line forms: space inside both ends sets it on its own.
        assert_eq!(maths_at("$ x $", &spans("$ x $"), 2, true), Some(Maths { source: "x".into(), latex: false, display: true }));
        assert_eq!(maths_at("文 $x$", &spans("文 $x$"), 3, true), Some(Maths { source: "x".into(), latex: false, display: false }));
    }

    /// **Typst 官方那條「詞內不算分隔符」**（`typst-syntax/src/lexer.rs:628` 的
    /// `in_word`，2026-09-28 照抄）。
    #[test]
    fn a_delimiter_inside_a_latin_word_is_not_a_delimiter() {
        // 從前這兩行被畫成斜體和粗體。
        assert_eq!(shape("snake_case_here"), "               ");
        assert_eq!(shape("a*b*c"), "     ");
        // Warning: **中文那一側一個字符不變**：官方明確把漢字、假名、諺文排除在「詞內」外。
        assert_eq!(shape("這*是*重點"), " .B.  ");
        // Typst 没有 Markdown 那條「開標記後面不許是空格」，`* 文*` 就是強調。
        assert_eq!(shape("* 文*"), ".BB.");
    }

    /// **反斜杠轉義，以及全角空格開頭的標題**（2026-09-28）。
    #[test]
    fn a_backslash_escapes_and_a_fullwidth_space_still_opens_a_heading() {
        assert_eq!(shape(r"\*不是強調\*"), ".     . ");
        // Warning: 全角空格是寫中文的人最常見的縮進，從前它讓整行標題不上色。
        assert_eq!(shape("= 　第一章"), ".HHHHH");
        assert_eq!(shape("= 第一章"), ".HHHH");
    }

    #[test]
    fn code_is_shown_because_it_is_what_produces_the_page() {
        // Not decoration around writing — the instructions that make it. A
        // writer has to see them, so they are never hidden, only set back.
        //
        // Warning: **方括號裏的那幾個字不是代碼**（2026-09-28）。`#chapter[初雪]` 裏「初雪」
        // 是寫稿人寫的字，從前整塊畫成代碼色，等於把它藏起來。現在代碼色只蓋
        // `#chapter[` 和 `]` 兩截。這一條原先寫的是 `"############"`。
        assert_eq!(shape("#chapter[初雪]"), "#########  #");
        assert_eq!(shape("那年#emph[冬天]。"), "  ######  # ");
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

    /// Each line of `text`: `` ` `` for a line the scan calls code, `$` for
    /// one that begins inside an equation, `.` for anything else.
    fn raw_and_maths(text: &str) -> String {
        let mut scanner = BlockScanner::new();
        text.lines()
            .map(|line| {
                let maths = scanner.in_math();
                let prefix: String = line.chars().take(crate::markdown::PREFIX).collect();
                match scanner.feed_line(&prefix, line.chars().count(), || Some(line.to_string())) {
                    _ if maths => '$',
                    Block::Code { .. } => '`',
                    _ => '.',
                }
            })
            .collect()
    }

    /// **Raw text by Typst's rules** (review, 2026-11-09): it opens at any
    /// backtick, mid-line too, and closes at the first run of as many
    /// backticks wherever it stands; two are an empty raw; a longer run
    /// closes with its first ones and opens another with the rest. A
    /// backtick escaped, in a comment, in a string or in maths is none.
    #[test]
    fn typst_raw_text_opens_and_closes_where_typst_s_does() {
        // Opened at the end of a line, closed in the middle of one.
        assert_eq!(raw_and_maths("那年 ```rust\nlet s = 1;\nlet t = 2;``` 雪\n後來"), ".``.");
        // One backtick runs over lines, as Typst's does.
        assert_eq!(raw_and_maths("甲 `乙\n丙\n丁` 戊\n己"), ".``.");
        // Two are an empty raw.
        assert_eq!(raw_and_maths("甲 `` 乙\n丙"), "..");
        // Four inside a raw of three: the first three close it, the fourth
        // opens a raw of one, closed on the line after.
        assert_eq!(raw_and_maths("```\nx````y\nz`\n甲"), "```.");
        // A tag only after the opening run; a run of three in the text of a
        // raw of four closes nothing.
        assert_eq!(raw_and_maths("````typ\n```\n````\n甲"), "```.");
        // A line that opens and closes a raw block leaves nothing open.
        assert_eq!(raw_and_maths("```rust fn main() {} ```\n甲"), "`.");
        // Escaped, in a comment, in maths: no raw.
        assert_eq!(raw_and_maths("甲 \\` 乙\n丙"), "..");
        assert_eq!(raw_and_maths("甲 /* ` */ 乙\n丙"), "..");
        assert_eq!(raw_and_maths("甲 $a ` b$ 乙\n丙"), "..");
        // In code: a string's backtick is none; a raw in a code body opens
        // and the body goes on after it.
        assert_eq!(raw_and_maths("#let s = \"`\"\n甲"), "..");
        assert_eq!(raw_and_maths("#let f() = {\n  let r = ```\n  }\n  ```\n  r\n}\n甲"), ".`````.");
    }

    /// **An equation runs to its `$`, over blank lines too**, as Typst's
    /// maths does; no piece may begin inside one.
    #[test]
    fn a_typst_equation_runs_over_lines_to_its_dollar() {
        assert_eq!(raw_and_maths("甲 $x\n\n= y\n$ 乙\n\n丙"), ".$$$..");
        // Its own `$` in a string, escaped or in a comment does not close it.
        assert_eq!(raw_and_maths("$ a \"$\" b\n\\$ /* $ */ c\nd $\n甲"), ".$$.");
        // And a backtick inside it is a symbol.
        assert_eq!(raw_and_maths("$ a\n` b $\n甲"), ".$.");
    }

    /// **An equation written in code is maths too** (review, 2026-11-09),
    /// as Typst's lexer reads `$` in code: a backtick in it is a symbol, not
    /// raw text, and its brackets open and close nothing of the code.
    #[test]
    fn a_typst_equation_in_code_hides_its_backticks_and_brackets() {
        // On one line: no raw is left open.
        assert_eq!(raw_and_maths("#let x = $a ` b$\n甲"), "..");
        // Over lines, from a statement: the line after is maths, the
        // statement goes on past the `$`, and the writing after is writing.
        assert_eq!(raw_and_maths("#let x = $a `\nb$ + 1\n甲"), ".$.");
        // In a code body: the body's lines stay code, and a `(` or a `}` in
        // the maths does not open or close the body.
        assert_eq!(raw_and_maths("#let f() = {\n  let y = $ ( } `\n  b $\n  y\n}\n甲"), ".`$``.");
        let mut scanner = BlockScanner::new();
        for line in ["#let f() = {", "  let y = $ ( } `", "  b $"] {
            scanner.feed_line(line, line.chars().count(), || Some(line.to_string()));
        }
        assert_eq!(scanner.frames(), &[Frame::Statement, Frame::Brace]);
        assert!(!scanner.in_raw() && !scanner.in_math());
    }

    /// Each line of `text` as the editor reads it: its block (`` ` `` code,
    /// `.` anything else), then its runs from the brackets it begins inside —
    /// `#` code, `%` a comment, `w` writing on a code line, `B` strong, and a
    /// space for nothing at all.
    fn walked(text: &str) -> Vec<String> {
        let mut scanner = BlockScanner::new();
        text.lines()
            .map(|line| {
                let frames = scanner.frames().to_vec();
                let len = line.chars().count();
                let prefix: String = line.chars().take(crate::markdown::PREFIX).collect();
                let block = match scanner.feed_line(&prefix, len, || Some(line.to_string())) {
                    Block::Code { .. } => '`',
                    _ => '.',
                };
                let mut out = vec![' '; len];
                for span in spans_from(line, &frames) {
                    let mark = match span.kind {
                        Kind::Code2 => '#',
                        Kind::Comment => '%',
                        Kind::Strong => 'B',
                        Kind::Marker => '.',
                        Kind::Token(_) => 'w',
                        _ => '?',
                    };
                    for slot in out.iter_mut().take(span.end.min(len)).skip(span.start) {
                        *slot = mark;
                    }
                }
                std::iter::once(block).chain(out).collect()
            })
            .collect()
    }

    /// **A template's code block is code from its `{` to its `}`**
    /// (2026-11-08): blank lines, `set` and `show` rules, closures, comments
    /// and all. Its lines used to be one flat colour down to the first blank
    /// line and plain writing after it.
    #[test]
    fn a_code_block_is_code_down_to_its_closing_brace() {
        let text = "\
#import \"@preview/tablex:0.0.8\": tablex, cellx

#let chapter(title, body) = {
  set page(paper: \"a5\", margin: (x: 2cm, y: 10pt))
  // set par(leading: 1em, justify: true)
  set text(font: \"思源宋體 Heavy\", size: 10.5pt) // 正文字體

  show heading.where(level: 1): it => {
    pagebreak(weak: true)
    block(above: 0.5em, [第 #counter(heading).display() 章])
  }
  show \"(註)\": set text(fill: gray)
  if title == none { [無題] } else {
    heading(title)
  }

  body
}
那年冬天，*雪*下得早。";
        let got = walked(text);
        let line = |n: usize| got[n].as_str();
        let width = |n: usize| text.lines().nth(n).map_or(0, |l| l.chars().count());
        assert_eq!(line(0), format!(".{}", "#".repeat(width(0))), "an import is code to its end");
        assert_eq!(line(2), format!(".{}", "#".repeat(width(2))), "the opening line is the line scan's");
        for (n, shape) in got.iter().enumerate().take(18).skip(3) {
            let blank = text.lines().nth(n).is_some_and(|l| l.trim().is_empty());
            assert_eq!(shape.starts_with('`'), !blank, "line {n} is code: {got:#?}");
        }
        assert_eq!(line(3), format!("`{}", "#".repeat(width(3))), "a set rule is code, every character");
        assert_eq!(line(4), format!("`##{}", "%".repeat(width(4) - 2)), "a comment in a code block is a comment");
        assert_eq!(&line(5)[..3], "`##", "{}", line(5));
        assert!(line(5).ends_with("#%%%%%%%"), "the comment after a rule: {}", line(5));
        assert!(!line(5)[..line(5).len() - 7].contains('%'), "the font name is a string: {}", line(5));
        assert_eq!(line(7), format!("`{}", "#".repeat(width(7))), "after a blank line, still code");
        // The writing in `[…]` is writing, and the `#` in it code again.
        let content: String = text.lines().nth(9).unwrap().chars().zip(line(9).chars().skip(1)).filter(|&(_, m)| m == 'w').map(|(c, _)| c).collect();
        assert_eq!(content, "第  章", "{}", line(9));
        // A `(` in a string opens nothing: the lines after it are code still.
        assert!(line(12).starts_with('`'), "{}", line(12));
        let content: String = text.lines().nth(12).unwrap().chars().zip(line(12).chars().skip(1)).filter(|&(_, m)| m == 'w').map(|(c, _)| c).collect();
        assert_eq!(content, "無題");
        assert_eq!(line(17), "`#", "the closing brace");
        assert_eq!(line(18), ".     .B.    ", "and the writing after the block is writing");
    }

    /// An argument list over several lines, with a note after each argument:
    /// the bracket in a comment closes nothing.
    #[test]
    fn an_argument_list_is_code_and_its_comments_are_comments() {
        let got = walked("#let ruby = get-ruby(\n  size: 0.5em, // 註音字號（(大小\n  dy: 0pt,\n)\n那年");
        assert_eq!(got[1], "`###############%%%%%%%%%%%");
        assert_eq!(got[2], "`##########");
        assert_eq!(got[3], "`#");
        assert_eq!(got[4], ".  ", "the list closed at its `)`");
    }

    /// Content opened in code is writing on every line until its `]`, which
    /// is code again.
    #[test]
    fn content_over_several_lines_is_writing() {
        let got = walked("#let note = [\n這是*重點*\n]\n#let n = 1");
        assert_eq!(got[1], ".  .BB.", "writing, read as writing");
        assert_eq!(got[2], "`#", "its `]` is code");
        assert_eq!(got[3], ".##########", "and the brackets are all closed");
    }

    /// `#if` in writing goes on past its block to an `else`, and stops
    /// after it.
    #[test]
    fn an_if_in_writing_runs_through_its_else() {
        let mut scanner = BlockScanner::new();
        scanner.feed("#if x [甲] else [乙] 丙", 18);
        assert!(scanner.frames().is_empty(), "{:?}", scanner.frames());
        let got = walked("#if x {\n  a\n} else {\n  b\n}\n丙");
        assert_eq!(got.iter().map(|l| l.chars().next().unwrap()).collect::<String>(), ".````.");
        assert_eq!(got[5], ". ");
    }

    /// A blank line still ends a body when the line after it is not
    /// indented — a paragraph's full-width indent included — so a stray
    /// bracket cannot tint the rest of a manuscript.
    #[test]
    fn a_stray_bracket_stops_at_a_blank_line_and_writing() {
        let got = walked("#emph(那年\n\n　　冬天雪下得早。\n");
        assert!(got[2].starts_with('.'), "{got:?}");
        let got = walked("#let f(x) = {\n  x\n\n  x\n}\n");
        assert_eq!(got.iter().map(|l| l.chars().next().unwrap()).collect::<String>(), ".`.``");
    }

    /// A line longer than the opening the scanner is handed is walked whole
    /// when the caller can hand it over.
    #[test]
    fn a_long_opening_line_is_walked_whole() {
        let long = "#let style(body) = { // the style every chapter of the book shares";
        assert!(long.chars().count() > crate::markdown::PREFIX, "the case is the truncation");
        let got = walked(&format!("{long}\n  set text(size: 10pt)\n}}\n那年"));
        assert_eq!(got.iter().map(|l| l.chars().next().unwrap()).collect::<String>(), ".``.");
    }

    /// `/* … */` is a comment, as `//` is (2026-11-05): its `*` opened a
    /// strong run, so the note was drawn bold.
    #[test]
    fn a_block_comment_is_a_comment_not_a_strong_run() {
        assert_eq!(shape("那年 /* 註 */ 冬天"), "   %%%%%%%   ");
        assert_eq!(shape("/* 甲 /* 乙 */ 丙 */*粗*"), "%%%%%%%%%%%%%%%%%.B.");
        assert_eq!(shape("那年 /* 沒收"), "   %%%%%");
        assert!(comment_left_open("那年 /* 沒收"));
        assert!(!comment_left_open("那年 /* 收了 */"));
        assert!(!comment_left_open("`/*` 是代碼"));
    }

    /// `#(1 + 2)` and `#{…}` are code, as `#set` is (2026-11-05).
    #[test]
    fn an_expression_after_a_hash_is_code() {
        assert_eq!(shape("值 #(1 + 2) 個"), "  ########  ");
        assert_eq!(shape("#{ x }"), "######");
        // A `#` before a space or a digit is a `#`.
        assert_eq!(shape("第 # 3 #1"), "        ");
    }

    /// `*粗 _斜_ 粗*` sets 斜 both ways, and the reverse (2026-11-05).
    #[test]
    fn emphasis_and_strong_nest_inside_each_other() {
        let kinds_at = |line: &str, at: usize| -> Vec<Kind> {
            spans(line).iter().filter(|s| s.start <= at && at < s.end).map(|s| s.kind).collect()
        };
        assert_eq!(kinds_at("*粗 _斜_ 粗*", 4), [Kind::Strong, Kind::Emphasis]);
        assert_eq!(kinds_at("_斜 *粗* 斜_", 4), [Kind::Emphasis, Kind::Strong]);
        assert_eq!(kinds_at("*粗 _斜_ 粗*", 3), [Kind::Strong, Kind::Marker]);
        // The two invariants: nested or apart, in order of start.
        let all = spans("*甲 _乙 `丙` 乙_ 甲* 和 _丁_");
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(a.start <= b.start, "{all:?}");
                assert!(b.start >= a.end || b.end <= a.end, "{all:?}");
            }
        }
    }

    /// `#[…]` is code around writing, and the `.` after `#w` ends it unless
    /// a field follows (review, 2026-11-06).
    #[test]
    fn content_blocks_and_a_stop_after_a_variable() {
        assert_eq!(shape("#[甲 *乙*] 丙"), "##  .B.#  ");
        assert_eq!(shape("第 #w. 項"), "  ##   ");
        assert_eq!(shape("#w.len() x"), "########  ");
    }

    /// **A bare URL is a link, not a comment from its `//`** (review,
    /// 2026-11-05).
    #[test]
    fn a_bare_url_is_a_link() {
        assert_eq!(shape("see https://e.com/a. Then"), "    LLLLLLLLLLLLLLL      ");
        assert_eq!(shape("x // note"), "  %%%%%%%");
    }

    /// `#if x [甲] else [乙]`: the head is code and the brackets are writing
    /// (2026-11-05); the whole of it was code, or none of the head was.
    #[test]
    fn a_keyword_block_is_code_around_writing() {
        assert_eq!(shape("#if x [甲 *乙*] else [丙]"), "#######  .B.######## #");
        assert_eq!(shape("#for x in (1, 2) [第 #x 項]"), "##################  ##  #");
        assert_eq!(shape("#while i < 3 { i += 1 } 後"), "#######################  ");
        // A block left open runs on below: the line is all head.
        assert_eq!(shape("#if cond ["), "##########");
    }

    /// The lines inside a `[` opened by code are writing; inside `{` or `(`
    /// they are code (2026-11-05).
    #[test]
    fn the_lines_of_a_content_block_are_writing() {
        let mut scanner = BlockScanner::new();
        let walk: String = "#if cond [\n  多行 *正文*\n  - 一項\n]\n#let f() = {\n  [甲]\n}\n那年"
            .lines()
            .map(|line| match scanner.feed(line, line.chars().count()) {
                Block::Code { .. } => '`',
                Block::Item { .. } => '-',
                Block::Prose => '.',
                _ => '?',
            })
            .collect();
        assert_eq!(walk, "..-`.``.", "{walk}");
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

// Warning: **測試模組一律擺在檔尾。** `yumete-core/tests/messages.rs` 那張「每個標籤都
// 有條目」的網把源碼切在**第一個**頂格的 `#[cfg(test)]\nmod ` 處——擺在檔案中間，
// 它後面的生產代碼就整段從網裏消失，於是那裏加一則文案，面板上直接印標籤而測試
// 全綠。這一支從前擺在中間，後面壓着 445 行（2026-09-24 審出來的）。
#[cfg(test)]
mod tests {
    use super::*;

    /// **A comment found without allocating reads as it always did**
    /// (2026-11-08): opened and closed, opened only, closed at once, at the
    /// end of the line, and a closer that is only half there.
    #[test]
    fn comments_are_found_where_they_were() {
        let at = |line: &str, k: usize| comment(&line.chars().collect::<Vec<_>>(), k);
        assert_eq!(at("a %%note%% b", 2), Some((2, 2, 10)));
        assert_eq!(at("a %%note", 2), Some((2, 0, 8)));
        assert_eq!(at("%%%%", 0), Some((2, 2, 4)));
        assert_eq!(at("x <!-- 註 --> y", 2), Some((4, 3, 12)));
        assert_eq!(at("x <!-- 註 --", 2), Some((4, 0, 11)));
        assert_eq!(at("<!-", 0), None);
        assert_eq!(at("%", 0), None);
        assert_eq!(run(&['a', ']', ']'], 0, "]]"), Some(1));
        assert_eq!(run(&['a', ']'], 0, "]]"), None);
        assert_eq!(comment_left_open("正文 %%寫給自己"), Some("%%"));
        assert_eq!(comment_left_open("正文 <!-- 一則 --> 正文"), None);
        assert_eq!(comment_closes("還在寫 --> 後面", "-->"), Some(4));
    }

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
        // Warning: A lone ``` **opens** a fence and swallows what follows, so the
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
                Kind::GoneRow => '<',
                Kind::AddedRow => '>',
                // Only a fence's grammar makes these (`code::highlight`).
                Kind::Token(_) => '~',
                // Typst 那一支的三個（2026-09-28）。
                Kind::Math => '$',
                Kind::Ref => '@',
                Kind::Label => '<',
                Kind::Html => 'T',
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

    /// 2026-09-19：**一個代碼段由幾個反引號開，就要幾個反引號關**。從前這裏
    /// 取的是「下一個反引號」，所以兩個挨在一起的反引號被當成一對空代碼段，
    /// 掃描回到 `%%` 上——於是它開了一條註釋，後面半本書變灰，導出的時候那
    /// 幾段**整個不見**。2026-09-19 報的就是這個，而這個檔自己的手冊
    /// （`docs/manual_tc.md:394`，引用各種標記的那一行）正是這個形狀。
    ///
    /// Warning: 上面那一條只用了單反引號，所以它永遠是綠的。
    #[test]
    fn a_code_span_closes_on_as_many_backticks_as_opened_it() {
        assert_eq!(shape("``%%``"), "..CC..", "兩個開，兩個關");
        assert_eq!(shape("`` `裏面` ``"), "..CCCCCC..", "裏面那一個反引號是內容");
        assert_eq!(shape("``%%`` 後面"), "..CC..   ", "後面是正文，不是註釋");
        // 單個的照舊。
        assert_eq!(shape("`%%`"), ".CC.");
        // 反引號之外的 `%%` 照舊開註釋（那是本來的功能，`.` 是記號、`%` 是註釋）。
        assert_eq!(shape("%%註釋%%"), "..%%..");
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
                // Nor this: display maths is found after the scan.
                Block::Math { .. } => '$',
            })
            .collect()
    }

    /// 2026-10-25: a note hard-wrapped with an indent was grey on its first line
    /// only.
    #[test]
    fn a_footnote_runs_on_over_its_indented_and_lazy_lines() {
        // Indented lines, a second paragraph after a blank, then writing again.
        assert_eq!(walk("[^a]: one\n    two\n    three\n\n    four\n\n那年"), "FFF.F..");
        // A plain line straight after is the note's; after a blank it is not.
        assert_eq!(walk("[^a]: one\ntwo\n\nthree"), "FF..");
        // A heading, a list item or the next note ends it.
        assert_eq!(walk("[^a]: one\n## 二\n[^b]: x\n- y"), "F2F-");
        // An indented list inside the note stays the note's.
        assert_eq!(walk("[^a]: one\n    - y"), "FF");
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

    /// **標記套得起來了**（2026-09-28）。
    ///
    /// 從前 ``**粗的`碼`**`` 交出來的是三段，中間那一段連反引號一起算成粗體的正文。
    #[test]
    fn marks_nest_now() {
        let inner = |line: &str, want: Kind| {
            spans(line)
                .into_iter()
                .filter(|s| s.kind == want)
                .map(|s| line.chars().skip(s.start).take(s.end - s.start).collect::<String>())
                .collect::<Vec<_>>()
        };
        assert_eq!(inner("**粗的`碼`**", Kind::Code), vec!["碼"], "粗體裏的行內代碼");
        assert_eq!(inner("**粗的`碼`**", Kind::Strong), vec!["粗的`碼`"], "外層照舊蓋着");
        assert_eq!(inner("[**粗**的](x)", Kind::Strong), vec!["粗"], "鏈接文字裏的粗體");
        assert_eq!(inner("==**重點**==", Kind::Strong), vec!["重點"], "標記裏的粗體");

        // Warning: **行內代碼與批注裏面不掃。**
        assert!(inner("`a*b*c`", Kind::Emphasis).is_empty(), "代碼裏的星號是代碼");
        assert!(inner("%%批注 **粗**%%", Kind::Strong).is_empty(), "批注整個是寫的人的");

        // Warning: **遞歸不許越過外層的閉合符**：``**a`b**c`` 裏那個反引號後面没有配對的。
        assert!(inner("**a`b**c", Kind::Code).is_empty(), "反引號不許找到外面去");
    }

    /// **一層套一層的強調**（2026-10-03 修）。
    ///
    /// 三條都是 2026-10-02 那一輪掃查報來的：`*em **strong** em*` 畫成 `em *`、
    /// `***both***` 畫成 `*both`、`**粗 *斜* 粗**` 是對的。前兩條的根子在
    /// [`closing`]——它在一串分隔符的中間收口。
    #[test]
    fn emphasis_nests_without_eating_a_delimiter() {
        let inner = |line: &str, want: Kind| {
            spans(line)
                .into_iter()
                .filter(|s| s.kind == want)
                .map(|s| line.chars().skip(s.start).take(s.end - s.start).collect::<String>())
                .collect::<Vec<_>>()
        };

        // 一顆星的強調不許收在 `**` 的第二顆上。
        assert_eq!(inner("*em **strong** em*", Kind::Emphasis), vec!["em **strong** em"]);
        assert_eq!(inner("*em **strong** em*", Kind::Strong), vec!["strong"]);

        // 一串三顆：外面一顆是斜，裏面兩顆是粗（CommonMark 也是這麼讀的）。
        assert_eq!(inner("***both***", Kind::Emphasis), vec!["**both**"]);
        assert_eq!(inner("***both***", Kind::Strong), vec!["both"]);

        // 從前就對的那一條，別改壞。
        assert_eq!(inner("**outer *inner* outer**", Kind::Strong), vec!["outer *inner* outer"]);
        assert_eq!(inner("**outer *inner* outer**", Kind::Emphasis), vec!["inner"]);

        // 一行上兩段，各自配各自的。
        assert_eq!(inner("**甲**和**乙**", Kind::Strong), vec!["甲", "乙"]);
        assert_eq!(inner("*甲*和*乙*", Kind::Emphasis), vec!["甲", "乙"]);

        // **兩段貼在一起，中間那一串是共用的**（2026-10-03 一輪審查報來的，是我
        // 當天自己挖的洞）。`**粗***斜*` 中間三顆星：兩顆收粗體、一顆開斜體。
        // 從前這裏要求「閉合那一串和開頭那一串一樣長」，於是兩邊都配不上，兩個
        // 構造一起落回正文——`**粗***斜*` 畫出來是一條斜體裏裝着三顆裸星。
        assert_eq!(inner("**粗***斜*", Kind::Strong), vec!["粗"]);
        assert_eq!(inner("**粗***斜*", Kind::Emphasis), vec!["斜"]);
        assert_eq!(inner("*a***b**", Kind::Emphasis), vec!["a"]);
        assert_eq!(inner("*a***b**", Kind::Strong), vec!["b"]);
        // 多出來的那一顆是正文，不是標記。
        assert_eq!(inner("**both***", Kind::Strong), vec!["both"]);
    }

    /// **兩條不變式**：按起點排好，任意兩條要麼不交要麼全含。
    ///
    /// 下游全靠它們——`mi k` 用「起點最靠後的那一條」當最內層，詳情面板同理，而兩個
    /// 順序寫字的循環從前假定不重疊，切出反向區間當場崩。
    #[test]
    fn the_two_invariants_hold() {
        let lines = [
            "**粗的`碼`**",
            "# **甲** `碼`",
            "[**粗**的](x)",
            "==**重點**==",
            "普通一行，没有標記。",
            "# 見[^1]",
            "**《書名》的`碼`**",
            "%%批注%%和**粗**",
            "[[雙鏈]]與*斜*",
        ];
        for line in lines {
            let got = spans(line);
            let mut prev = 0usize;
            for one in &got {
                assert!(one.start >= prev, "按起點排好：{line:?} {got:?}");
                prev = one.start;
            }
            for a in &got {
                for b in &got {
                    let apart = a.end <= b.start || b.end <= a.start;
                    let holds = (a.start <= b.start && b.end <= a.end)
                        || (b.start <= a.start && a.end <= b.end);
                    assert!(apart || holds, "要麼不交要麼全含：{line:?} {a:?} {b:?}");
                }
            }
        }
    }


    /// **鏈接那三個洞**（2026-09-28）。
    #[test]
    fn a_link_holds_together_through_brackets_and_a_bang() {
        let marker = |line: &str| {
            spans(line)
                .into_iter()
                .filter(|s| s.kind == Kind::Marker)
                .map(|s| line.chars().skip(s.start).take(s.end - s.start).collect::<String>())
                .collect::<Vec<_>>()
        };
        let text = |line: &str| {
            spans(line)
                .into_iter()
                .find(|s| s.kind == Kind::Link)
                .map(|s| line.chars().skip(s.start).take(s.end - s.start).collect::<String>())
        };

        // Warning: 一、`!` 在構造裏——不然所見即所得會留一個孤零零的驚嘆號。
        assert_eq!(marker("![圖](a.png)")[0], "![", "驚嘆號跟着開括號走");

        // Warning: 二、地址裏的括號要數着走。中文維基的地址常帶括號。
        let wiki = "[連結](https://x/中文_(消歧義))";
        assert_eq!(marker(wiki).last().unwrap(), "](https://x/中文_(消歧義))");

        // Warning: 三、看得見的那幾個字裏也可以有方括號。
        assert_eq!(text("[甲[乙]丙](x)").as_deref(), Some("甲[乙]丙"));

        // 一行兩條鏈接還是兩條，没有被貪心地併成一條。
        let two = spans("看 [甲](b) 和 [乙](c)");
        let links: Vec<usize> = two.iter().filter(|s| s.kind == Kind::Link).map(|s| s.start).collect();
        assert_eq!(links.len(), 2, "兩條各歸各的：{two:?}");

        // 没配對的不算鏈接。
        assert!(spans("[没配對(x)").is_empty());
    }

    /// Warning: **文字是粗體的鏈接照樣跟得了**（2026-09-28）。
    ///
    /// 最裏面那一條是 `Strong`，它自己不是鏈接；要往外走一層纔找得到。
    #[test]
    fn a_link_whose_text_is_bold_is_still_a_link() {
        let line = "[**粗**的](地址)";
        // 光標站在「粗」上（0[ 1* 2* 3粗）。
        let got = link_at(line, 3).expect("跟得到：{line}");
        assert_eq!(got.target, "地址");
    }


    /// **反斜杠轉義**（2026-09-28）。從前没有，於是正文裏寫不出一個字面的星號。
    ///
    /// Warning: 這個倉自己的 `export.rs` 正在生成 `\*`，導出的檔用自己的編輯器打開會亂。
    #[test]
    fn a_backslash_escapes_the_mark_after_it() {
        let kinds = |line: &str| spans(line).into_iter().map(|s| s.kind).collect::<Vec<_>>();

        // 轉義掉的星號不開斜體，只剩兩個反斜杠各自是一個標記。
        assert_eq!(kinds(r"\*不是斜體\*"), vec![Kind::Marker, Kind::Marker]);

        // Warning: **中間那個轉義掉的不算閉合**：斜體一直到最後那個星號。
        let body = spans(r"*斜\*體*")
            .into_iter()
            .find(|s| s.kind == Kind::Emphasis)
            .map(|s| r"*斜\*體*".chars().skip(s.start).take(s.end - s.start).collect::<String>());
        assert_eq!(body.as_deref(), Some(r"斜\*體"));

        // Warning: **只有 ASCII 標點能被轉義。** 漢字前面那個反斜杠就是一個反斜杠。
        assert!(spans(r"\甲不是轉義").is_empty(), "漢字不可轉義");

        // 鏈接裏轉義掉的方括號不收口。
        let text = spans(r"[文字\]還在](x)")
            .into_iter()
            .find(|s| s.kind == Kind::Link)
            .map(|s| r"[文字\]還在](x)".chars().skip(s.start).take(s.end - s.start).collect::<String>());
        assert_eq!(text.as_deref(), Some(r"文字\]還在"));

        // 没被轉義的照舊。
        assert_eq!(kinds("*真斜體*"), vec![Kind::Marker, Kind::Emphasis, Kind::Marker]);
    }


    /// Warning: **`_` 的詞中保護不許把漢字也擋掉**（2026-09-28 修）。
    ///
    /// 那一條是保 `snake_case` 的，而 `is_alphanumeric()` 對漢字回真，於是
    /// `這_是_重點` 一條 span 都不交。這是這一族裏唯一一個我們自己造出來的中文問題。
    #[test]
    fn an_underscore_between_han_still_emphasises() {
        let kinds = |line: &str| spans(line).into_iter().map(|s| s.kind).collect::<Vec<_>>();
        assert_eq!(
            kinds("這_是_重點"),
            vec![Kind::Marker, Kind::Emphasis, Kind::Marker],
            "漢字之間的下劃線照樣是強調"
        );
        // 拉丁詞中間的照舊不是。
        assert!(kinds("snake_case_here").is_empty(), "snake_case 還是要保住");
    }

    /// **一長串 `=` 不是反覆的 `==高亮==`**（2026-10-11 報的）。
    ///
    /// 十四個 `=` 畫出來只剩六個：掃描器把它們兩個兩個配成 `==…==`，每配成一對
    /// 就把那四個字符當標記收走。光標走過去那一行在 6／10／14 格之間跳，因為所
    /// 見即所得只在光標不在那一行的時候把標記藏起來。
    #[test]
    fn a_run_of_equals_is_not_a_row_of_highlights() {
        for line in ["==============", "=====", "===="] {
            let marks: usize = spans(line)
                .into_iter()
                .filter(|s| s.kind == Kind::Marker)
                .map(|s| s.end - s.start)
                .sum();
            assert_eq!(marks, 0, "{line:?} 整條都是正文，一個標記都不該有");
        }
        // 真的高亮照舊。
        let kinds = |line: &str| spans(line).into_iter().map(|s| s.kind).collect::<Vec<_>>();
        assert_eq!(kinds("==重點=="), vec![Kind::Marker, Kind::Highlight, Kind::Marker]);
    }

    /// **`***三個***` 兩層都算數**（2026-10-11 報的）。
    ///
    /// CommonMark 把它讀成 `<em><strong>`。這裏不做 delimiter stack，可至少不該
    /// 吃掉一半——從前畫出來是 `*粗斜**`。
    #[test]
    fn three_stars_are_both_layers() {
        let line = "***又粗又斜***";
        let body: Vec<String> = spans(line)
            .into_iter()
            .filter(|s| s.kind == Kind::Strong || s.kind == Kind::Emphasis)
            .map(|s| line.chars().skip(s.start).take(s.end - s.start).collect())
            .collect();
        assert!(
            body.iter().any(|one| one == "又粗又斜"),
            "裏頭那四個字要整段算進去：{body:?}"
        );
        // 標記一個不剩地收走：三個開、三個閉。
        let marks: usize = spans(line)
            .into_iter()
            .filter(|s| s.kind == Kind::Marker)
            .map(|s| s.end - s.start)
            .sum();
        assert_eq!(marks, 6, "六個星號全是標記");
    }

    /// **嵌套的強調，裏層不許把外層的閉合吃掉**（2026-10-11 報的）。
    ///
    /// `*斜 **粗** 斜*` 從前畫成 `斜 *粗** 斜`——找閉合那一步不跳過比自己長的
    /// 那一串分隔符，於是外層的 `*` 配到了裏層 `**` 的第一個星號上。
    #[test]
    fn the_inner_emphasis_does_not_eat_the_outer_one() {
        let line = "*斜 **粗** 斜*";
        let got: Vec<(Kind, String)> = spans(line)
            .into_iter()
            .map(|s| (s.kind, line.chars().skip(s.start).take(s.end - s.start).collect()))
            .collect();
        let said = |kind: Kind| -> Vec<String> {
            got.iter().filter(|(k, _)| *k == kind).map(|(_, t)| t.clone()).collect()
        };
        assert_eq!(said(Kind::Strong), ["粗"], "裏層：{got:?}");
        assert!(
            said(Kind::Emphasis).iter().any(|one| one.contains("斜 ") && one.ends_with(" 斜")),
            "外層從頭斜到尾：{got:?}"
        );
    }

    /// `$a + b$` and `$$x^2$$` are maths, as Pandoc reads them (2026-11-05).
    #[test]
    fn dollars_around_a_formula_are_maths_and_prices_are_not() {
        assert_eq!(shape("式 $a + b$ 和 $$x^2$$"), "  .$$$$$.   ..$$$..");
        // No space inside the dollars, no digit after the closing one.
        assert_eq!(shape("價 $3 and $5"), "           ");
        assert_eq!(shape("$ a$ 和 $a $"), "           ");
        assert_eq!(shape("$5 到 10$5"), "         ");
        assert_eq!(shape(r"\$a$"), ".   ");
        // Nothing inside is markup, and a code span keeps winning.
        assert_eq!(shape("$a*b*c$"), ".$$$$$.");
        assert_eq!(shape("`$a$` 碼"), ".CCC.  ");
        assert_eq!(shape("$a `b$` c"), "   .CC.  ");
        // Inside emphasis.
        assert_eq!(shape("**$x$**"), "...$...");
    }

    /// The labels [`maths_blocks`] gives a small document, a line each.
    fn maths_labels(doc: &[&str], typst: bool) -> Vec<Block> {
        let mut blocks = vec![Block::Prose; doc.len()];
        maths_blocks(&mut blocks, typst, |i| doc[i].contains('$'), |i| doc[i].to_string());
        blocks
    }

    /// The runs of one line of a maths block, one character a cell.
    pub(super) fn block_shape(line: &str, first: bool, last: bool, typst: bool) -> String {
        let n = line.chars().count();
        let mut out = vec![' '; n];
        for span in maths_block_spans(line, first, last, typst) {
            let c = match span.kind {
                Kind::Math => '$',
                Kind::Marker => '.',
                Kind::Strong => 'B',
                _ => '?',
            };
            for slot in out.iter_mut().take(span.end.min(n)).skip(span.start) {
                *slot = c;
            }
        }
        out.into_iter().collect()
    }

    /// **`$$` on a line of its own opens display maths that runs to the next
    /// `$$`** (2026-11-06). Only the one-line form used to be coloured, so a
    /// formula set on lines of its own was drawn as prose.
    #[test]
    fn display_maths_over_several_lines_is_maths() {
        let doc = ["前文", "$$", r"\begin{aligned}", r"a &= b \\", r"\end{aligned}", "$$", "後文"];
        let blocks = maths_labels(&doc, false);
        assert_eq!(blocks[0], Block::Prose);
        assert_eq!(blocks[1], Block::Math { first: true, last: false });
        assert_eq!(blocks[3], Block::Math { first: false, last: false });
        assert_eq!(blocks[5], Block::Math { first: false, last: true });
        assert_eq!(blocks[6], Block::Prose);
        assert_eq!(block_shape("$$", true, false, false), "..");
        assert_eq!(block_shape(r"a &= b \\", false, false, false), "$$$$$$$$$");
        // Text on the delimiters' lines: the formula up to the `$$`, the
        // writing after it read as writing.
        assert_eq!(block_shape("$$ x +", true, false, false), "..$$$$");
        assert_eq!(block_shape("y $$ **是**", false, true, false), "$$.. ..B..");
        let lines: Vec<String> = doc[1..6].iter().map(|s| s.to_string()).collect();
        assert_eq!(maths_block_source(&lines, false), "\n\\begin{aligned}\na &= b \\\\\n\\end{aligned}\n");
    }

    /// **A blank line before the close means it was never maths** — Pandoc
    /// ends the paragraph there, and a stray `$$` must not tint the chapter.
    #[test]
    fn a_dollar_pair_with_a_blank_line_between_is_not_a_block() {
        assert!(maths_labels(&["$$", "a", "", "b", "$$"], false).iter().all(|b| *b == Block::Prose));
        // A one-line formula is not a block, and neither is a price.
        assert!(maths_labels(&["$$x$$", "a", "$$"], false)[0] == Block::Prose);
        assert!(maths_labels(&["價 $3", "和 $5"], false).iter().all(|b| *b == Block::Prose));
        // A fence between them is a fence.
        let mut blocks = vec![Block::Prose, Block::Code { inside: None }, Block::Prose];
        let doc = ["$$", "```", "$$"];
        maths_blocks(&mut blocks, false, |i| doc[i].contains('$'), |i| doc[i].to_string());
        assert_eq!(blocks[0], Block::Prose);
    }

    /// **The formula a column stands in**, delimiters included (2026-11-06).
    #[test]
    fn the_formula_under_a_column() {
        let line = "式 $a + b$ 和 $$x^2$$";
        let inline = Maths { source: "a + b".into(), latex: true, display: false };
        assert_eq!(maths_at(line, &spans(line), 2, false), Some(inline.clone()), "on the opening `$`");
        assert_eq!(maths_at(line, &spans(line), 4, false), Some(inline.clone()));
        assert_eq!(maths_at(line, &spans(line), 8, false), Some(inline), "on the closing `$`");
        assert_eq!(maths_at(line, &spans(line), 9, false), None, "past it");
        assert_eq!(maths_at(line, &spans(line), 0, false), None);
        assert_eq!(
            maths_at(line, &spans(line), 14, false),
            Some(Maths { source: "x^2".into(), latex: true, display: true })
        );
        assert_eq!(maths_at("價 $3 and $5", &spans("價 $3 and $5"), 3, false), None);
    }

    /// `<https://…>` and `<name@host>` are links (2026-11-05), and say where
    /// they go.
    #[test]
    fn an_autolink_is_a_link() {
        assert_eq!(shape("見 <https://a.b/c> 頁"), "  .LLLLLLLLLLLLL.  ");
        assert_eq!(shape("<mailto:a@b.c>"), ".LLLLLLLLLLLL.");
        assert_eq!(shape("<a@b.cn>"), ".LLLLLL.");
        assert_eq!(link_at("見 <https://a.b/c> 頁", 5).map(|l| l.target), Some("https://a.b/c".into()));
        assert_eq!(link_at("<a@b.cn>", 0).map(|l| l.target), Some("mailto:a@b.cn".into()));
        // A space, or no scheme, and it is not one.
        assert_eq!(shape("<https://a b>"), "             ");
        assert_eq!(shape("1 < 2 > 3"), "         ");
    }

    /// `<kbd>`, `</kbd>` and `<br>` are HTML: set back, never hidden, and
    /// the text between them is prose (2026-11-05).
    #[test]
    fn an_html_tag_is_markup_and_what_it_holds_is_prose() {
        assert_eq!(shape("按 <kbd>Esc</kbd> 鍵"), "  TTTTT   TTTTTT  ");
        assert_eq!(shape("行<br>行<br/>"), " TTTT TTTTT");
        assert_eq!(shape(r#"<a href="x y" id=z>甲</a>"#), "TTTTTTTTTTTTTTTTTTT TTTT");
        assert!(hidden(&spans("按 <kbd>Esc</kbd>"), None).is_empty(), "a tag stays on the page");
        // Not a tag: a space after `<`, an attribute without a space before it.
        assert_eq!(shape("a < b > c"), "         ");
        assert_eq!(shape("<a\"x\">"), "      ");
        assert_eq!(shape("<1>"), "   ");
        // A comment is still a comment, and code still wins.
        assert_eq!(shape("<!-- 註 -->"), "....%%%...");
        assert_eq!(shape("`<br>`"), ".CCCC.");
    }

    /// A tag names an HTML element and carries attributes as HTML writes
    /// them; generics and comparisons in prose are prose.
    #[test]
    fn only_a_real_element_is_a_tag() {
        let prose = |line: &str| shape(line).trim().is_empty();
        let tag = |line: &str| shape(line).chars().all(|c| c == 'T');
        assert!(prose("Vec<String> 和 Option<i32>"));
        assert!(prose("a<b and c>d"));
        assert!(prose("<kbd x>"));
        assert_eq!(shape("<B>粗</B>"), "TTT TTTT");
        assert!(tag("<details open>"));
        assert!(tag("<video src=a.mp4 controls>"));
        assert!(tag("<span lang='en'>"));
    }
}
