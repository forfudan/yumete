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

/// **這一行是不是標題，是幾級。**
///
/// `mark` 是 `#`（Markdown）或者 `=`（Typst）。規矩一條：一到六個記號，後面跟着**空白**
/// 或者行尾。
///
/// Warning: **後面那個空白是 CommonMark §4.2 明寫的**，不是我們加嚴的：`#128` 是一段話，
/// 不是標題。Markdown 1.0 原版寬鬆，而那條規矩存在的理由正是 `#128`、`#!/bin/sh`、
/// `#include` 這一族。Typst 的 `=` 同樣要求後跟空白。
///
/// Warning: **全角空格也算空白**——中文作者最常見的縮進寫法是 `= 　第一章`。
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
    /// 沒有就得每一步做區間比較。
    pub depth: u8,
}

/// **這一行有哪些標記**，按起點排好，**外層排在內層前面**。
///
/// 沒被蓋到的就是普通正文，不交 span。
///
/// # Warning: 兩條不變式，下游全靠它們
///
/// 一、**任意兩條要麼不相交，要麼一條完全包含另一條**。不會出現半重疊。
///
/// 二、**按 `start` 排好；包住別人的那一條排在前面。** 於是
/// 「蓋住某一格的最內層那一條」＝ 覆蓋它的裏面 `start` 最大的那一條
/// （`max_by_key(start)`，`mi m` 就是這麼取的）。
///
/// Warning: **「不重疊」那句話 2026-09-28 之前就已經不成立了**，標題那一支先壓一條
/// `Kind::Heading` 蓋住整行、行內的構造再壓上去。當時沒寫下約定，於是七個消費方各猜各
/// 的，其中三個猜錯：`panel.rs` 那個順序寫字的循環會切出反向區間**當場崩**，`export.rs`
/// 與 `detail.rs` 的 `.find()` 取到的是外層那一條（標題裏的腳註因此沒有詳情面板）。
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
/// Warning: **每一處算出來的 `end` 都要對着 `to` 驗一次。** 底下那些 `code_span`／`closing`
/// ／`find` 找的是**整行**，遞歸進來的時候它們會越過 `to` 去找閉合符——``**a`b**c`` 裏
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
        if let Some(len) = fence(chars, at) {
            let kind = match (chars[at], len) {
                ('*', 2) | ('_', 2) => Kind::Strong,
                ('~', 2) => Kind::Strike,
                ('=', 2) => Kind::Highlight,
                _ => Kind::Emphasis,
            };
            if let Some(close) = closing(chars, at + len, chars[at], len)
                .filter(|&close| close + len <= to)
            {
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
            // Warning: **兩頭都數括號**（2026-09-28）。從前是「往後找第一個 `]`／`)`」：
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
        // Warning: **轉義掉的那一個不算閉合**（2026-09-28）：`*斜\*體*` 的斜體到最後那個
        // 星號纔收口，不是到中間那個。
        if chars[at..at + len].iter().all(|&c| c == delimiter)
            && chars.get(at.wrapping_sub(1)) != Some(&' ')
            && !escaped(chars, at)
            && at > from
        {
            return Some(at);
        }
        at += 1;
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
    let want: Vec<char> = text.chars().collect();
    (from..chars.len().saturating_sub(want.len() - 1)).find(|&i| chars[i..].starts_with(&want))
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
/// this project's own manual (`docs/manual.md:394`, a line of quoted markup),
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
        let mut out = Vec::new();
        let mut construct = 1usize;
        let mut at = 0usize;

        // A heading is `=` through `======`, and the rest of the line is it.
        // 判準在 `super::heading_marks`，三處共用一支。
        if let Some(equals) = super::heading_marks(line, '=') {
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
            // `// to the end of the line` is a note to oneself.
            if chars[at] == '/' && chars.get(at + 1) == Some(&'/') {
                push(&mut out, at, chars.len(), Kind::Comment, construct);
                break;
            }
            // `#import`, `#let`, `#show`, `#set` and every call: the
            // instructions, not the writing. Shown, always — a writer needs to
            // see what produces the page.
            if chars[at] == '#' && chars.get(at + 1).is_some_and(|c| c.is_alphabetic()) {
                let (end, bodies) = code_parts(&chars, at + 1);
                let mine = construct;
                construct += 1;
                // **方括號裏裝的是正文，不是代碼**（2026-09-28）。`#chapter[初雪]`、
                // `#quote[…]`、`#figure(caption: [說明])` 是中文 Typst 稿裏最常見的三
                // 種寫法，從前整塊畫成代碼色，等於把作者寫的字藏起來。
                //
                // Warning: **這是官方那套模式切換裏唯一值得學的一層**：Typst 的 `[]` 從 code
                // 模式切回 markup 模式。再往裏（`#if x { [文字] }` 這種）只出現在模板檔
                // 裏，不出現在正文裏，而模板整塊灰掉反而好讀。
                //
                // Warning: **代碼色只蓋代碼那兩截**（`#chapter[` 和 `]`），中間**不蓋**。
                // 蓋了的話裏面的字還是灰的——正文沒有自己的 span，它就是「沒被蓋到的
                // 那些格子」，所以要真的讓開，不能靠往上再壓一層。
                let mut cut = at;
                for (a, b) in bodies {
                    push(&mut out, cut, a, Kind::Code2, mine);
                    let body: String = chars[a..b].iter().collect();
                    let inner = spans(&body);
                    let top = inner.iter().map(|s| s.construct).max().unwrap_or(0);
                    for mut one in inner {
                        one.start += a;
                        one.end += a;
                        one.construct += construct;
                        one.depth = 1;
                        out.push(one);
                    }
                    construct += top + 1;
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
                let end = (at + 1..chars.len())
                    .take_while(|&i| ref_name(chars[i]))
                    .last()
                    .map(|i| i + 1);
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
            if matches!(chars[at], '*' | '_') && !in_word(&chars, at) {
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
            // `` `code` ``. Counted in backticks — see `super::code_span`;
            // without that ``` ``//`` ``` opened a Typst comment.
            if chars[at] == '`' {
                if let Some((open, close, end)) = super::code_span(&chars, at) {
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
        // 前面不許是空格」，那是照 Markdown 抄的；Typst 沒有這條規矩，`* 文*` 在官方
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
                Kind::Math => '$',
                Kind::Ref => '@',
                Kind::Label => '<',
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

    /// **Typst 官方那條「詞內不算分隔符」**（`typst-syntax/src/lexer.rs:628` 的
    /// `in_word`，2026-09-28 照抄）。
    #[test]
    fn a_delimiter_inside_a_latin_word_is_not_a_delimiter() {
        // 從前這兩行被畫成斜體和粗體。
        assert_eq!(shape("snake_case_here"), "               ");
        assert_eq!(shape("a*b*c"), "     ");
        // Warning: **中文那一側一個字符不變**：官方明確把漢字、假名、諺文排除在「詞內」外。
        assert_eq!(shape("這*是*重點"), " .B.  ");
        // Typst 沒有 Markdown 那條「開標記後面不許是空格」，`* 文*` 就是強調。
        assert_eq!(shape("* 文*"), ".BB.");
    }

    /// **反斜杠轉義，以及全角空格開頭的標題**（2026-09-28）。
    #[test]
    fn a_backslash_escapes_and_a_fullwidth_space_still_opens_a_heading() {
        assert_eq!(shape(r"\*不是強調\*"), ".     . ");
        // Warning: 全角空格是中文作者最常見的縮進，從前它讓整行標題不上色。
        assert_eq!(shape("= 　第一章"), ".HHHHH");
        assert_eq!(shape("= 第一章"), ".HHHH");
    }

    #[test]
    fn code_is_shown_because_it_is_what_produces_the_page() {
        // Not decoration around writing — the instructions that make it. A
        // writer has to see them, so they are never hidden, only set back.
        //
        // Warning: **方括號裏的那幾個字不是代碼**（2026-09-28）。`#chapter[初雪]` 裏「初雪」
        // 是作者寫的字，從前整塊畫成代碼色，等於把它藏起來。現在代碼色只蓋
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
                // Only a fence's grammar makes these (`code::highlight`).
                Kind::Token(_) => '~',
                // Typst 那一支的三個（2026-09-28）。
                Kind::Math => '$',
                Kind::Ref => '@',
                Kind::Label => '<',
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
    /// （`docs/manual.md:394`，引用各種標記的那一行）正是這個形狀。
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

        // Warning: **遞歸不許越過外層的閉合符**：``**a`b**c`` 裏那個反引號後面沒有配對的。
        assert!(inner("**a`b**c", Kind::Code).is_empty(), "反引號不許找到外面去");
    }

    /// **兩條不變式**：按起點排好，任意兩條要麼不交要麼全含。
    ///
    /// 下游全靠它們——`mi m` 用「起點最靠後的那一條」當最內層，詳情面板同理，而兩個
    /// 順序寫字的循環從前假定不重疊，切出反向區間當場崩。
    #[test]
    fn the_two_invariants_hold() {
        let lines = [
            "**粗的`碼`**",
            "# **甲** `碼`",
            "[**粗**的](x)",
            "==**重點**==",
            "普通一行，沒有標記。",
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

        // 一行兩條鏈接還是兩條，沒有被貪心地併成一條。
        let two = spans("看 [甲](b) 和 [乙](c)");
        let links: Vec<usize> = two.iter().filter(|s| s.kind == Kind::Link).map(|s| s.start).collect();
        assert_eq!(links.len(), 2, "兩條各歸各的：{two:?}");

        // 沒配對的不算鏈接。
        assert!(spans("[沒配對(x)").is_empty());
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


    /// **反斜杠轉義**（2026-09-28）。從前沒有，於是正文裏寫不出一個字面的星號。
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

        // 沒被轉義的照舊。
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

}
