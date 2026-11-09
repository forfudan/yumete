//! Turning the page on screen into a file somebody else can typeset — Feature
//! #88.
//!
//! yumete already knows things about a manuscript that Markdown does not
//! record: which direction it is set in, that its 句讀 hang in the margin, and
//! which of its characters carry readings. All of that lived only on the
//! screen. Export writes it down in a form a typesetter, a browser, or a
//! publisher accepts.
//!
//! Two targets, for two different reasons:
//!
//! - **HTML** can actually set Chinese vertically — `writing-mode: vertical-rl`
//!   is the one place outside a terminal where 縱書 costs one line of CSS — so
//!   this is the export that reproduces what the writer sees.
//! - **Typst** is where a book gets made. It cannot set text vertically yet, so
//!   that export is horizontal on purpose rather than by omission, and says so
//!   in a comment at the head of the file.
//!
//! Neither is a converter for arbitrary Markdown, but both carry what a
//! manuscript is made of: headings, paragraphs and readings, and since
//! 2026-10-29 the lists, block quotes, footnotes, pipe tables and rules a
//! chapter or a paper has. Before that they came out as their source
//! (`<p>- 第一点<br>`, `<p>[^1]: …</p>`).
//!
//! **Lines in a paragraph.** In Chinese a line break inside a paragraph is
//! usually meant, so it stays one; between two Latin lines it is a hard-wrapped
//! sentence going on, so it becomes a space, as CommonMark reads it. And the
//! page's language comes from the text: an English file is no longer set as a
//! Chinese book (`lang="zh"`, a 2em indent, CJK serif fonts).

use crate::ruby::{self, Dialect, Dialects};
use crate::say;

/// **The Typst compiler** (2026-11-06): the `typst` on `PATH`, which
/// `--export pdf` runs and the formula float under the cursor runs too. One
/// place says which program that is, so the two cannot come to disagree.
pub fn typst_command() -> std::process::Command {
    std::process::Command::new("typst")
}

/// Whether [`typst_command`] answers at all.
pub fn typst_is_installed() -> bool {
    typst_command()
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// What to write the manuscript out as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A standalone HTML page, vertical when the editor is.
    Html,
    /// A Typst source file, horizontal.
    Typst,
    /// **An EPUB 3 book** (2026-11-08): the HTML page's text and stylesheet
    /// in a zip a reader app opens, vertical and turning right to left when
    /// the editor is. See [`crate::epub`].
    Epub,
}

impl Format {
    /// Parse a `:export` argument.
    pub fn parse(name: &str) -> Option<Format> {
        match name.trim().to_ascii_lowercase().as_str() {
            "html" | "htm" => Some(Format::Html),
            "typst" | "typ" => Some(Format::Typst),
            "epub" => Some(Format::Epub),
            _ => None,
        }
    }

    /// The extension a file of this kind is named with.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Html => "html",
            Format::Typst => "typ",
            Format::Epub => "epub",
        }
    }
}

/// What separates two cells in the file `:export <name>` writes, or `None` if
/// `name` is not one of the delimited formats — a **table**, not a page.
///
/// These are deliberately not [`Format`] variants. Everything a `Format` names
/// is a way of setting a manuscript: [`export`] is handed the whole text and
/// answers with the whole text. A CSV is a grid, it comes from one table rather
/// than from a document, and a cell that holds the delimiter has to be refused
/// out loud — three things that shape does not fit. So the editor serves them
/// itself, from the table under the cursor, and this is the list it consults
/// (Feature #227). Asking for the delimiter *is* asking whether the name is
/// one of them, so there is one question here and not two — the editor needs
/// the answer either way, and a second predicate would only give the call site
/// a `None` it had already ruled out to make up a default for.
pub fn delimiter_of(name: &str) -> Option<char> {
    match name.trim().to_ascii_lowercase().as_str() {
        "csv" => Some(','),
        "tsv" => Some('\t'),
        _ => None,
    }
}

/// The paper a printed copy is set on, in whole millimetres.
///
/// A trim and not a name: which names exist is a matter of where the book is
/// printed — 32開 is not A5 and neither is 大32開 — and keeping the table of
/// them out here means the export answers to a measurement rather than to a
/// vocabulary. [`Paper::A5`] is what a Chinese novel is usually printed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paper {
    /// Across the page.
    pub width: u32,
    /// Down the page.
    pub height: u32,
}

impl Paper {
    /// 148 × 210 mm.
    pub const A5: Paper = Paper {
        width: 148,
        height: 210,
    };
}

impl Default for Paper {
    fn default() -> Paper {
        Paper::A5
    }
}

impl Paper {
    /// **The paper a name or a `寬x高` names** — `a5`, `32開`, `130x184` —
    /// or `None`.
    ///
    /// Warning: **the same names as `[export] page`**, which yumete-config
    /// reads with its own `parse_page`: that crate cannot see this one, nor
    /// this one that. `yumete-tui/tests/page_names.rs` holds the two to one
    /// answer.
    pub fn named(value: &str) -> Option<Paper> {
        let name = value.trim().to_ascii_lowercase();
        let (width, height) = match name.as_str() {
            "a4" => (210, 297),
            "a5" => (148, 210),
            "a6" => (105, 148),
            "b5" => (176, 250),
            "b6" => (125, 176),
            "letter" => (216, 279),
            "16k" | "16開" | "16开" => (185, 260),
            "32k" | "32開" | "32开" => (130, 184),
            "d32k" | "大32開" | "大32开" => (140, 203),
            _ => {
                let (w, h) = name.split_once(['x', '*', '×'])?;
                (w.trim().parse().ok()?, h.trim().parse().ok()?)
            }
        };
        (width > 0 && height > 0).then_some(Paper { width, height })
    }
}

/// **How a page is set** (2026-11-08): the `[export]` settings, and the words
/// one `:export` or `ye --export` gives for itself.
///
/// Nothing here comes from the editing window. An export is publication, and
/// how the writer sees the draft — `:vertical`, `:view-hanging`, the 縱
/// length, `:ruby-html`, `tatechuyoko`, `:render` — is theirs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Whether the text runs in 縱 from the right edge.
    pub vertical: bool,
    /// Whether 句讀 hang in the margin (標點旁置).
    pub hanging: bool,
    /// Whether `<ruby>` in the source is read as ruby.
    pub ruby_html: bool,
    /// Whether Typst's `#ruby` in the source is read as ruby.
    pub ruby_typst: bool,
    /// Whether the brace form, `{漢字|かんじ}`, is read as ruby in a Markdown
    /// source.
    pub ruby_markdown: bool,
    /// How half-width letters and digits are set down a vertical page.
    pub latin: Latin,
    /// The paper a printed copy is set on.
    pub paper: Paper,
    /// The fonts the page asks for, a CSS font list as VS Code's Markdown
    /// preview writes it.
    pub fonts: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            vertical: false,
            hanging: false,
            ruby_html: true,
            ruby_typst: true,
            ruby_markdown: true,
            latin: Latin::Turn,
            paper: Paper::A5,
            fonts: DEFAULT_FONTS.to_string(),
        }
    }
}

/// **The words `:export` and `ye --export` take** (2026-11-08), each the
/// setting of the same name for one export. `page=` and `font=` take a value.
pub const WORDS: &[&str] = &[
    "vertical", "horizontal", "hanging", "no-hanging", "ruby-html", "no-ruby-html", "ruby-typst", "no-ruby-typst",
    "ruby-markdown", "no-ruby-markdown", "turn", "jlreq", "page=", "font=",
];

impl Settings {
    /// The ruby syntaxes the source is read in.
    pub fn dialects(&self) -> Dialects {
        let mut set = Dialects::NONE;
        if self.ruby_html {
            set.insert(Dialect::Html);
        }
        if self.ruby_typst {
            set.insert(Dialect::Typst);
        }
        if self.ruby_markdown {
            set.insert(Dialect::Markdown);
        }
        set
    }

    /// **Take one word** — `vertical`, `jlreq`, `page=A5`,
    /// `font='"Songti SC", serif'` — into these settings. `None` when `word`
    /// is none of [`WORDS`], so the caller can take it for a path;
    /// `Some(Err(why))` when it is one of them with a value that is not, or
    /// when it ends in a comma.
    ///
    /// **A comma does not carry a word on** (2026-11-09): `font=Songti, b.html`
    /// once read as the font list `Songti, b.html` and wrote the page to the
    /// last file named. A word runs to the next space; a font list with a
    /// space or a comma in it is quoted. A word left ending in a comma is a
    /// list cut short, said so rather than taken for a font or a path.
    pub fn take(&mut self, word: &str) -> Option<Result<(), String>> {
        if word.ends_with(',') {
            return Some(Err(say!("export.trailing-comma", word)));
        }
        let set = |flag: &mut bool, on: bool| {
            *flag = on;
            Some(Ok(()))
        };
        match word {
            "vertical" => set(&mut self.vertical, true),
            "horizontal" => set(&mut self.vertical, false),
            "hanging" => set(&mut self.hanging, true),
            "no-hanging" => set(&mut self.hanging, false),
            "ruby-html" => set(&mut self.ruby_html, true),
            "no-ruby-html" => set(&mut self.ruby_html, false),
            "ruby-typst" => set(&mut self.ruby_typst, true),
            "no-ruby-typst" => set(&mut self.ruby_typst, false),
            "ruby-markdown" => set(&mut self.ruby_markdown, true),
            "no-ruby-markdown" => set(&mut self.ruby_markdown, false),
            "turn" => {
                self.latin = Latin::Turn;
                Some(Ok(()))
            }
            "jlreq" => {
                self.latin = Latin::Jlreq;
                Some(Ok(()))
            }
            _ => {
                if let Some(name) = word.strip_prefix("page=") {
                    return Some(match Paper::named(name) {
                        Some(paper) => {
                            self.paper = paper;
                            Ok(())
                        }
                        None => Err(say!("export.no-such-page", name)),
                    });
                }
                let list = word.strip_prefix("font=")?;
                // `font='"A B", serif'`: the list inside single quotes, as a
                // shell would want it written. `font="A B, serif"` is a list
                // in double quotes, which CSS would read as one name; `"A B"`
                // alone is one name, quotes and all.
                let wrapped = |q: char| list.len() > 1 && list.starts_with(q) && list.ends_with(q) && !list[1..list.len() - 1].contains(q);
                let list = match wrapped('\'') || (wrapped('"') && list.contains(',')) {
                    true => &list[1..list.len() - 1],
                    false => list,
                };
                Some(match list.trim().is_empty() {
                    true => Err(say!("export.no-fonts")),
                    false => {
                        self.fonts = list.trim().to_string();
                        Ok(())
                    }
                })
            }
        }
    }

    /// **The words of one `:export` line, and the path after them**
    /// (2026-11-08): `vertical jlreq page=A5 out/book.epub`. The words come
    /// in any order and the path last, if there is one.
    ///
    /// A word is split at a space, except inside quotes, so a font list
    /// holds its spaces when it is quoted: `font='"PingFang SC", serif'` is
    /// one word.
    /// What is not a word is a path when it looks like one — a `/`, an
    /// extension, a quote — and otherwise a word misspelt, said so rather
    /// than written to a file of that name: a file really called `vertical`
    /// is `./vertical`.
    pub fn read_line(&mut self, line: &str) -> Result<Option<String>, String> {
        let pieces = split_words(line)?;
        let mut path = None;
        for (n, piece) in pieces.iter().enumerate() {
            if let Some(taken) = self.take(piece) {
                taken?;
                continue;
            }
            if !looks_like_a_path(piece) {
                return Err(say!("export.no-such-word", piece));
            }
            if n + 1 < pieces.len() {
                return Err(say!("export.words-after-path", piece));
            }
            path = Some(unquoted(piece));
        }
        Ok(path)
    }
}

/// `line` cut into words: at a space, except inside `"…"` or `'…'`. A
/// backslash keeps the next character (`a\ b.md`) where the system's paths
/// do not use it themselves.
pub(crate) fn split_words(line: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut chars = line.trim().chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => {
                quote = None;
                word.push(c);
            }
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                word.push(c);
            }
            (None, '\\') if !cfg!(windows) => {
                word.push(c);
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            (None, c) if c.is_whitespace() => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            (None, c) => word.push(c),
        }
    }
    if quote.is_some() {
        return Err(say!("export.open-quote", line.trim()));
    }
    if !word.is_empty() {
        out.push(word);
    }
    Ok(out)
}

/// Whether a piece of an `:export` line that is not a word names a file: it
/// holds a `/` (or `\` on Windows), has an extension, starts at `~` or `.`,
/// or is quoted.
pub fn looks_like_a_path(piece: &str) -> bool {
    let name = piece.rsplit(['/', '\\']).next().unwrap_or(piece);
    piece.contains('/')
        || (cfg!(windows) && piece.contains('\\'))
        || piece.starts_with(['~', '.', '"', '\''])
        || name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && !ext.is_empty())
}

/// A path as it was meant: its quotes taken off, or its backslashes read.
pub(crate) fn unquoted(piece: &str) -> String {
    if let Some(q @ ('"' | '\'')) = piece.chars().next() {
        if piece.len() > 1 && piece.ends_with(q) {
            return piece[1..piece.len() - 1].to_string();
        }
    }
    if cfg!(windows) {
        return piece.to_string();
    }
    let mut out = String::with_capacity(piece.len());
    let mut chars = piece.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next().unwrap_or('\\')),
            c => out.push(c),
        }
    }
    out
}

/// 天頭 — the margin above the 版心, as a fraction of the page height.
const MARGIN_TOP: f64 = 0.12;
/// 地腳 — below it. Narrower than 天頭 on purpose: a 版心 set a little above
/// centre is what a book looks like, and a centred one looks like it slipped.
const MARGIN_BOTTOM: f64 = 0.09;
/// Each of the two side margins, as a fraction of the page width.
const MARGIN_SIDE: f64 = 0.10;
/// **The type size of a printed page** (2026-11-08): 五號, 10.5 point, what
/// a Chinese book is set in. The screen page has no measure at all — its
/// lines fill the window across, its 縱 fill it down — so the paper alone
/// says how long a line is in print.
const PRINT_SIZE_PT: f64 = 10.5;
/// The 行距, as a multiple of the character. One number, used both to set
/// `line-height` and to say how many 行 the paper then holds.
const LINE_HEIGHT: f64 = 1.8;

/// What one export is given: how to set the page, and what it is setting.
#[derive(Debug, Clone)]
pub struct Style {
    /// How the page is set: the `[export]` settings and this export's words.
    pub settings: Settings,
    /// The document's title, for the HTML `<title>`.
    pub title: String,
    /// **源檔是什麼語法**（2026-09-28）。
    ///
    /// Warning: **這一格從前没有，於是導出永遠按 Markdown 解析源檔**——一份 Typst 稿子裏的
    /// `*粗*`、`$a+b$`、`@引用` 一個都認不出來，原樣（並且轉義）寫進導出的檔。
    /// `Format` 說的是**寫成什麼**，這一格說的是**讀的是什麼**，兩件事。
    pub source: crate::syntax::Syntax,
    /// The folder the source file sits in, which a picture's path is read
    /// from: an EPUB carries its pictures inside it. `None` for a buffer
    /// never saved.
    pub base: Option<std::path::PathBuf>,
    /// **What draws a TeX formula** (2026-11-08), or `None` to leave Markdown's
    /// `$…$` as written. The front end hands it over: RaTeX lives there, so
    /// that this crate and its tests do not carry KaTeX's fonts.
    pub painter: Option<Painter>,
    /// **How the Markdown preview draws** (2026-11-08), for
    /// `:export-as-preview`: its fonts and colours, and YFM read as it reads
    /// it — callouts, front matter, code in colour. `None` for every other
    /// export, which is set as `[export]` says.
    pub look: Option<crate::as_preview::Look>,
}

/// A TeX formula drawn for a page: a PNG, and how big to show it, in `em` of
/// the text around it.
#[derive(Debug, Clone)]
pub struct Formula {
    pub png: Vec<u8>,
    /// From the picture's left edge to its right.
    pub width: f64,
    /// From the top of the picture to its foot.
    pub height: f64,
    /// How far the picture reaches below the baseline — left of it, for a
    /// picture turned clockwise.
    pub depth: f64,
}

/// Draws `tex`, on its own line when the first flag says so (`$$…$$`), and
/// turned a quarter turn clockwise when the second does — a formula down a
/// 縱, its picture turned in the bitmap and measured as it is shown — in the
/// ink given (red, green, blue: black for a printed page, the writing's
/// colour for `:export-as-preview`), or says what was wrong with it.
pub type Painter = fn(&str, bool, bool, [u8; 3]) -> Result<Formula, String>;

/// **How half-width letters and digits are set down a vertical page**
/// (2026-11-08), `[export] vertical_latin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Latin {
    /// Every run of letters and digits turned a quarter turn clockwise, the
    /// half-width marks joined to it turned with it (`5%`, `don’t`), and
    /// every inline formula turned. No 縱中橫.
    #[default]
    Turn,
    /// As JLREQ §2.3.2 and clreq set them: a word turned whole; capitals and
    /// a single letter upright, one to a square; one digit upright, two
    /// across in one square (縱中橫), three or more turned; a number with its
    /// unit (`5%`) set one way as a whole. A formula is turned, unless it is
    /// one letter or digit or a numeric fraction, which stands upright.
    Jlreq,
}

/// The fonts a page asks for unless `[export] font_family` says otherwise:
/// VS Code's Markdown preview's, the system's own faces.
pub const DEFAULT_FONTS: &str =
    "-apple-system, BlinkMacSystemFont, \"Segoe WPC\", \"Segoe UI\", system-ui, \"Ubuntu\", \"Droid Sans\", sans-serif";

/// What draws a formula into this page, and how the page sets one.
#[derive(Clone, Copy)]
struct Drawing {
    painter: Painter,
    /// Down the page, and so turning what [`Drawing::turns`] says.
    vertical: bool,
    latin: Latin,
    /// What the formula is drawn in: black, or the preview's writing.
    ink: [u8; 3],
}

impl Drawing {
    fn of(style: &Style) -> Option<Drawing> {
        style.painter.map(|painter| Drawing { painter, vertical: style.settings.vertical, latin: style.settings.latin, ink: ink_of(style) })
    }

    /// **Whether `tex` is turned on its side** (2026-11-08). Across the page
    /// nothing is. Down it a formula on a line of its own always is, as
    /// JLREQ §3.7.4 sets one, and an inline one under [`Latin::Turn`]; under
    /// [`Latin::Jlreq`] one letter or digit and a numeric fraction stand
    /// upright in their square, as the same letter or digit would.
    fn turns(&self, tex: &str, display: bool) -> bool {
        self.vertical && (display || self.latin == Latin::Turn || !stands_upright(tex))
    }
}

/// **`style`, its ink the one `pick` chooses** (2026-11-08): a formula in
/// a quotation is in the quotation's colour, in a note in the notes', as
/// the preview draws them. Only the ink of what is written inside changes;
/// without a look the ink is black throughout.
fn with_ink(style: &Style, pick: fn(&crate::as_preview::Colours) -> crate::as_preview::Rgba) -> std::borrow::Cow<'_, Style> {
    match &style.look {
        None => std::borrow::Cow::Borrowed(style),
        Some(look) => {
            let mut inked = style.clone();
            let ink = pick(&look.colours);
            if let Some(look) = inked.look.as_mut() {
                look.colours.text = ink;
            }
            std::borrow::Cow::Owned(inked)
        }
    }
}

/// **The ink a formula is drawn in**: the preview's writing for
/// `:export-as-preview`, black on a printed page.
fn ink_of(style: &Style) -> [u8; 3] {
    style.look.as_ref().map_or([0, 0, 0], |look| {
        let [r, g, b, _] = look.colours.text;
        [r, g, b]
    })
}

/// `x`, `2`, `\frac12`, `\tfrac{3}{4}`, `½`: a formula one square holds,
/// which under [`Latin::Jlreq`] stands upright down a vertical page.
pub fn stands_upright(tex: &str) -> bool {
    static FRACTION: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let tex = tex.trim();
    let mut chars = tex.chars();
    let one = match (chars.next(), chars.next()) {
        (Some(c), None) => c.is_ascii_alphanumeric() || matches!(c as u32, 0xBC..=0xBE | 0x2150..=0x215E),
        _ => false,
    };
    one || FRACTION
        .get_or_init(|| regex::Regex::new(r"^\\[dt]?frac\s*(\d|\{\s*\d+\s*\})\s*(\d|\{\s*\d+\s*\})$").expect("a pattern"))
        .is_match(tex)
}

/// Write `text` out as `format`. An EPUB is a zip, so what comes back for one
/// is the book's text alone, the XHTML page inside it; [`export_file`] is the
/// whole file.
pub fn export(text: &str, format: Format, style: &Style) -> String {
    match format {
        Format::Html => html(text, style),
        Format::Typst => typst(text, style),
        Format::Epub => crate::epub::chapter(text, style).0,
    }
}

/// The bytes of the file `format` is written as.
pub fn export_file(text: &str, format: Format, style: &Style) -> Vec<u8> {
    match format {
        Format::Epub => crate::epub::book(text, style),
        _ => export(text, format, style).into_bytes(),
    }
}

/// One block of the manuscript.
enum Block {
    Heading(usize, String),
    Paragraph(Vec<String>),
    /// A fenced code block, fence lines and all, exactly as it was written.
    ///
    /// It has to be a block of its own: run through the paragraph path, the
    /// opening ```` ```rust ```` met the *inline* scanner, which read two of
    /// its three backticks as an empty code span and swallowed them — the
    /// third came out as `\``, and what reached the `.typ` file was not a code
    /// block at all (#386).
    Code(Vec<String>),
    /// `---`, `***` or `___` on a line of its own.
    Rule,
    /// The `>` lines, read again as blocks of their own.
    Quote(Vec<Block>),
    /// Items with one kind of marker, each item blocks of its own, and
    /// whether it is a task and done (`- [x]`) or not (`- [ ]`).
    List { ordered: bool, start: usize, items: Vec<(Option<bool>, Vec<Block>)> },
    /// A pipe table: its heading row, how each column aligns, its rows, and
    /// its caption — Pandoc's `Table: …` (or `: …`) paragraph just above or
    /// below it (2026-11-08).
    Table { head: Vec<String>, align: Vec<crate::mdtable::Align>, rows: Vec<Vec<String>>, caption: Option<String> },
    /// **A picture alone in its paragraph** (2026-11-08), `![說明](a.png)`:
    /// a figure, its text the caption, as Pandoc reads it.
    Figure { caption: String, src: String },
    /// **A display formula on lines of its own** (2026-11-08): Markdown's
    /// `$$`, the formula, `$$`, delimiters and all. Read as a paragraph, the
    /// three lines came out joined by spaces and no line could be drawn: each
    /// held a piece of the formula.
    Math(Vec<String>),
    /// **A callout** (2026-11-08), read for a page as the Markdown preview
    /// reads YFM: a `:::` container (`fenced`) or a
    /// GitHub alert, `> [!NOTE]`. Its kind ([`CALLOUTS`]), the word it was
    /// written with, the title written after it, and its blocks.
    Callout { kind: usize, name: String, title: Option<String>, fenced: bool, body: Vec<Block> },
    /// **Front matter** (2026-11-08), the `---` lines heading the file,
    /// read for a page: the preview shows it in a frame.
    FrontMatter(Vec<String>),
}

/// **The kinds of callout**, in the order the preview's palette holds their
/// colours, and the words each is written with (VitePress's and GitHub's).
/// `details` has no colour of its own.
pub(crate) const CALLOUTS: [(&str, &[&str]); 6] = [
    ("note", &["note", "info"]),
    ("tip", &["tip"]),
    ("important", &["important"]),
    ("warning", &["warning"]),
    ("caution", &["caution", "danger", "error"]),
    ("details", &["details"]),
];

/// Which of [`CALLOUTS`] `word` names, in any case.
fn callout_kind(word: &str) -> Option<usize> {
    let word = word.to_ascii_lowercase();
    CALLOUTS.iter().position(|(_, words)| words.contains(&word.as_str()))
}

/// A footnote's name and the lines of its text, `[^1]: …`.
type Note = (String, Vec<String>);

/// The manuscript as blocks, and its footnotes' texts taken out of the flow.
///
/// `yfm` reads YFM as the Markdown preview reads it, for the HTML and EPUB
/// exports: callouts and front matter. Task lists are read everywhere.
fn blocks(text: &str, source: crate::syntax::Syntax, yfm: bool) -> (Vec<Block>, Vec<Note>) {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut notes = Vec::new();
    let markdown = !matches!(source, crate::syntax::Syntax::Typst | crate::syntax::Syntax::Text);
    // Front matter: `---` on the first line, to the `---` or `...` that
    // closes it.
    let mut front = None;
    if yfm && markdown && lines.first().is_some_and(|l| l.trim_end() == "---") {
        if let Some(end) = lines.iter().skip(1).position(|l| matches!(l.trim_end(), "---" | "...")) {
            front = Some(lines[1..=end].to_vec());
            lines.drain(..=end + 1);
        }
    }
    let mut out = parse(&lines, source, &mut notes, yfm);
    if let Some(front) = front {
        out.insert(0, Block::FrontMatter(front));
    }
    (out, notes)
}

/// Split `lines` into blocks.
///
/// A heading is a line of hashes and a title, as it is everywhere else in
/// yumete; a paragraph is a run of lines with no blank line in it. Lists are
/// read in Markdown and in Typst (`+` numbers there); quotes, tables, rules
/// and footnotes in Markdown, which is where they are written that way.
fn parse(lines: &[String], source: crate::syntax::Syntax, notes: &mut Vec<Note>, yfm: bool) -> Vec<Block> {
    use crate::syntax::Syntax;
    // 標題在 Markdown 裏是 `#`，在 Typst 裏是 `=`（2026-09-28）。
    let mark = match source {
        Syntax::Typst => '=',
        _ => '#',
    };
    let markdown = !matches!(source, Syntax::Typst | Syntax::Text);
    let lists = !matches!(source, Syntax::Text);
    let mut out = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let flush = |para: &mut Vec<String>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Paragraph(std::mem::take(para)));
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim_end();
        let body = line.trim_start();
        let indent = line.len() - body.len();
        // Inside a fence nothing is markup — not a blank line, not a `#`, not
        // a backtick. One nobody closed still comes out, since the writer's
        // text is the writer's, half-written or not.
        //
        // **The fence is read as the Markdown preview reads it**
        // (2026-11-09), CommonMark's rule: three or more backticks or tildes
        // open it, and a line of the same character, at least as many and
        // nothing after them, closes it. It used to be three backticks and
        // the next line opening with three: a `~~~` block's `# 注释` came out
        // a heading, and a ```` ```` ```` fence ended at the ```` ``` ```` it
        // was there to show.
        if let Some((mark, run)) = fence_of(body).filter(|_| indent < 4) {
            flush(&mut para, &mut out);
            let mut held = vec![lines[i].clone()];
            i += 1;
            while i < lines.len() {
                held.push(lines[i].clone());
                i += 1;
                if closes_fence(&lines[i - 1], mark, run) {
                    break;
                }
            }
            out.push(Block::Code(held));
            continue;
        }
        if body.is_empty() {
            flush(&mut para, &mut out);
            i += 1;
            continue;
        }
        // **An indented code block** (2026-11-09, reported: `    let x = 1;`
        // came out a paragraph, and a fence indented inside it a code block of
        // its own): four spaces or a tab, not going on a paragraph, as the
        // preview reads it. Its lines, the four columns taken off, go into a
        // fence of backticks longer than any run inside, so a fence written in
        // it is code text, in HTML and in Typst alike.
        if markdown && para.is_empty() && columns_in(line) >= 4 {
            let mut held: Vec<String> = Vec::new();
            while i < lines.len() && (columns_in(&lines[i]) >= 4 || lines[i].trim().is_empty()) {
                held.push(without_columns(&lines[i], 4));
                i += 1;
            }
            while held.last().is_some_and(|l| l.trim().is_empty()) {
                held.pop();
            }
            let longest = held.iter().flat_map(|l| l.split(|c| c != '`')).map(str::len).max().unwrap_or(0);
            let fence = "`".repeat(longest.max(2) + 1);
            let mut block = vec![fence.clone()];
            block.extend(held);
            block.push(fence);
            out.push(Block::Code(block));
            continue;
        }
        if markdown && para.is_empty() {
            if let Some(end) = display_maths_end(lines, i) {
                out.push(Block::Math(lines[i..=end].iter().map(|l| l.trim().to_string()).collect()));
                i = end + 1;
                continue;
            }
        }
        if markdown && indent == 0 {
            if let Some((name, first)) = note_opening(body) {
                flush(&mut para, &mut out);
                let mut text = vec![first.to_string()];
                i += 1;
                while i < lines.len() && lines[i].starts_with([' ', '\t']) && !lines[i].trim().is_empty() {
                    text.push(lines[i].trim().to_string());
                    i += 1;
                }
                notes.push((name.to_string(), text));
                continue;
            }
        }
        let depth = crate::markdown::heading_marks(line, mark).unwrap_or(0);
        if depth > 0 && !line[depth..].trim().is_empty() {
            flush(&mut para, &mut out);
            out.push(Block::Heading(depth.min(6), line[depth..].trim().to_string()));
            i += 1;
            continue;
        }
        // A setext heading: the paragraph above, underlined.
        if markdown && !para.is_empty() && indent < 4 {
            let level = match body {
                b if b.chars().all(|c| c == '=') => Some(1),
                b if b.len() >= 2 && b.chars().all(|c| c == '-') => Some(2),
                _ => None,
            };
            if let Some(level) = level {
                let title = std::mem::take(&mut para).iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
                out.push(Block::Heading(level, title));
                i += 1;
                continue;
            }
        }
        if markdown && indent < 4 && is_rule(body) {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        if markdown && indent < 4 && body.starts_with('>') {
            flush(&mut para, &mut out);
            let mut inner = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let rest = &lines[i].trim_start()[1..];
                inner.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
                i += 1;
            }
            // A GitHub alert: `[!NOTE]` on the quote's first line.
            let alert = inner.first().filter(|_| yfm).and_then(|first| {
                let rest = first.trim().strip_prefix("[!")?;
                let close = rest.find(']')?;
                let kind = callout_kind(&rest[..close])?;
                let title = rest[close + 1..].trim();
                Some((kind, rest[..close].to_string(), (!title.is_empty()).then(|| title.to_string())))
            });
            out.push(match alert {
                Some((kind, name, title)) => {
                    let body = parse(&inner[1..], source, notes, yfm);
                    Block::Callout { kind, name, title, fenced: false, body }
                }
                None => Block::Quote(parse(&inner, source, notes, yfm)),
            });
            continue;
        }
        // A `:::` container, to the `:::` that closes it.
        if yfm && markdown && indent < 4 && body.starts_with(":::") {
            let mut words = body[3..].split_whitespace();
            if let Some(name) = words.next() {
                flush(&mut para, &mut out);
                let title: Vec<&str> = words.collect();
                let title = (!title.is_empty()).then(|| title.join(" "));
                let (mut depth, mut inner) = (1, Vec::new());
                i += 1;
                while i < lines.len() {
                    let text = lines[i].trim();
                    i += 1;
                    if let Some(rest) = text.strip_prefix(":::") {
                        depth = match rest.split_whitespace().next() {
                            Some(_) => depth + 1,
                            None => depth - 1,
                        };
                        if depth == 0 {
                            break;
                        }
                    }
                    inner.push(lines[i - 1].clone());
                }
                let body = parse(&inner, source, notes, yfm);
                let kind = callout_kind(name).unwrap_or(0);
                out.push(Block::Callout { kind, name: name.to_string(), title, fenced: true, body });
                continue;
            }
        }
        let rowish = |l: &str| crate::mdtable::is_row(l) || crate::mdtable::has_bar(l);
        // **A table is as wide as its header**, as GFM has it: a row's cells
        // past the header's are dropped and a short row is filled out, and a
        // header with no cells (a bare `|`) is no table — in Typst it would
        // be `columns: 0`, which typst refuses.
        let table = match markdown && rowish(line) && !crate::mdtable::split(line).is_empty() {
            true => lines.get(i + 1).and_then(|l| crate::mdtable::rule_of(l)),
            false => None,
        };
        if let Some(mut align) = table {
            flush(&mut para, &mut out);
            let head = crate::mdtable::split(line);
            let width = head.len();
            align.resize(width, crate::mdtable::Align::Plain);
            i += 2;
            let mut rows = Vec::new();
            while i < lines.len() && rowish(&lines[i]) {
                let mut row = crate::mdtable::split(&lines[i]);
                row.resize(width, String::new());
                rows.push(row);
                i += 1;
            }
            // The caption: the paragraph just above, if it was one, or the
            // one just below, a blank line between or not.
            let mut caption = match out.last() {
                Some(Block::Paragraph(held)) if caption_of(&held.join(" ")).is_some() => {
                    let Some(Block::Paragraph(held)) = out.pop() else { unreachable!() };
                    caption_of(&held.join(" ")).map(str::to_string)
                }
                _ => None,
            };
            if caption.is_none() {
                let at = i + usize::from(lines.get(i).is_some_and(|l| l.trim().is_empty()));
                if let Some(first) = lines.get(at).and_then(|l| caption_of(l.trim())) {
                    let mut held = vec![first.to_string()];
                    i = at + 1;
                    while i < lines.len() && !lines[i].trim().is_empty() {
                        held.push(lines[i].trim().to_string());
                        i += 1;
                    }
                    caption = Some(held.join(" "));
                }
            }
            out.push(Block::Table { head, align, rows, caption });
            continue;
        }
        if markdown && para.is_empty() && indent < 4 && lines.get(i + 1).is_none_or(|l| l.trim().is_empty()) {
            if let Some((caption, src)) = figure_of(body) {
                out.push(Block::Figure { caption: caption.to_string(), src: src.to_string() });
                i += 1;
                continue;
            }
        }
        let opened = match lists {
            true => list_item(line, source),
            false => None,
        };
        if let Some(first) = opened {
            flush(&mut para, &mut out);
            let mut items = Vec::new();
            let mut item = first;
            loop {
                // The item's own lines: its first, then what is indented past
                // its marker, then — with no blank line between — a line that
                // simply goes on (CommonMark's lazy continuation).
                let mut held = vec![line_after(&lines[i], item.text)];
                i += 1;
                while i < lines.len() {
                    let next = &lines[i];
                    let blank = next.trim().is_empty();
                    let deep = lead(next) >= item.width;
                    if !blank && deep {
                        held.push(next[item.width.min(next.len())..].to_string());
                    } else if blank {
                        // A blank line ends the item unless what follows is
                        // still inside it.
                        let goes_on = lines.get(i + 1).is_some_and(|after| {
                            !after.trim().is_empty() && lead(after) >= item.width
                        });
                        if !goes_on {
                            break;
                        }
                        held.push(String::new());
                    } else if list_item(next, source).is_none()
                        && !held.last().is_some_and(|l| l.trim().is_empty())
                        && !next.trim_start().starts_with(['#', '>', '|'])
                    {
                        held.push(next.trim().to_string());
                    } else {
                        break;
                    }
                    i += 1;
                }
                items.push((item.task, parse(&held, source, notes, yfm)));
                // The next item, when it is a sibling: the same kind of marker
                // at the same indent, with at most a blank line between.
                let mut at = i;
                while at < lines.len() && lines[at].trim().is_empty() {
                    at += 1;
                }
                match lines.get(at).and_then(|l| list_item(l, source)) {
                    Some(next) if next.indent == item.indent && next.ordered == item.ordered && at <= i + 1 => {
                        i = at;
                        item = next;
                    }
                    _ => break,
                }
            }
            out.push(Block::List { ordered: first.ordered, start: first.start, items });
            continue;
        }
        // The line as written, trailing spaces kept: two of them are a hard
        // break, which `html_lines` reads (2026-10-30: trimmed here, every
        // hard break was lost).
        para.push(lines[i].trim_end_matches(['\r', '\n']).to_string());
        i += 1;
    }
    flush(&mut para, &mut out);
    out
}

/// **How far `line` is indented, in columns**: a space one, a tab to the
/// next multiple of four, as CommonMark counts it.
fn columns_in(line: &str) -> usize {
    let mut n = 0;
    for c in line.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 4 - n % 4,
            _ => break,
        }
    }
    n
}

/// `line` with `n` columns of its indent taken off, a tab that spans the
/// cut leaving the spaces past it.
fn without_columns(line: &str, n: usize) -> String {
    let mut at = 0;
    for (b, c) in line.char_indices() {
        if at >= n {
            return line[b..].to_string();
        }
        match c {
            ' ' => at += 1,
            '\t' => {
                let next = at + 4 - at % 4;
                if next > n {
                    return format!("{}{}", " ".repeat(next - n), &line[b + 1..]);
                }
                at = next;
            }
            _ => return line[b..].to_string(),
        }
    }
    String::new()
}

/// **The fence `body` opens**, if it opens one: its character, a backtick
/// or a tilde, and how many of it, three or more — the Markdown preview's
/// rule, and CommonMark's.
pub(crate) fn fence_of(body: &str) -> Option<(char, usize)> {
    let c = body.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let n = body.chars().take_while(|&d| d == c).count();
    (n >= 3).then_some((c, n))
}

/// Whether `line` closes a fence of `run` × `mark`: the same character, at
/// least as many of it, and nothing else on the line.
fn closes_fence(line: &str, mark: char, run: usize) -> bool {
    let body = line.trim();
    fence_of(body).is_some_and(|(c, n)| c == mark && n >= run) && body.trim_start_matches(mark).is_empty()
}

/// **The last line of a display formula opened at `lines[at]`**: a line that
/// opens with `$$` and does not close it, then the line that ends with `$$`,
/// with no blank line between (Pandoc's rule). `None` when `lines[at]` opens
/// none, or nothing closes it.
fn display_maths_end(lines: &[String], at: usize) -> Option<usize> {
    let first = lines[at].trim();
    let rest = first.strip_prefix("$$")?;
    if rest.trim_end().ends_with("$$") {
        return None;
    }
    for (k, line) in lines.iter().enumerate().skip(at + 1) {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        if line.ends_with("$$") {
            return Some(k);
        }
    }
    None
}

/// The text of a table's caption line, Pandoc's `Table: …` or `: …`.
fn caption_of(line: &str) -> Option<&str> {
    let text = line.strip_prefix("Table:").or_else(|| line.strip_prefix(": "))?;
    Some(text.trim()).filter(|t| !t.is_empty())
}

/// A line that is one picture and nothing else, `![caption](src "title")`:
/// its caption and where the picture is.
///
/// **One picture** (2026-11-09): the caption holds no `]`, so
/// `![两张](a.png) 和 ![图](b.png)` is two pictures in a line of text, as
/// the preview reads it, and not one figure captioned `两张](a.png) 和 ![图`.
/// A path in angle brackets may hold spaces, `![图](<my pic.png>)`.
fn figure_of(line: &str) -> Option<(&str, &str)> {
    static FIGURE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let found = FIGURE
        .get_or_init(|| {
            regex::Regex::new(r#"^!\[([^\]]*)\]\(\s*(?:<([^<>\n]*)>|([^\s<>]+))(?:\s+(?:"[^"]*"|'[^']*'|\([^()]*\)))?\s*\)$"#)
                .expect("a pattern")
        })
        .captures(line)?;
    let src = found.get(2).or_else(|| found.get(3))?.as_str();
    (!src.is_empty()).then_some((found.get(1)?.as_str(), src))
}

/// `[^name]: text` — a footnote's name and the start of its text.
fn note_opening(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("[^")?;
    let close = rest.find("]:")?;
    let name = &rest[..close];
    (!name.is_empty() && !name.contains(char::is_whitespace)).then(|| (name, rest[close + 2..].trim()))
}

/// `---`, `***`, `___`, spaced or not: three or more of one of them, alone.
fn is_rule(body: &str) -> bool {
    let marks: Vec<char> = body.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3 && matches!(marks[0], '-' | '*' | '_') && marks.iter().all(|&c| c == marks[0])
}

/// Where a list item's marker is and what it says.
#[derive(Clone, Copy)]
struct Item {
    /// The marker's column.
    indent: usize,
    ordered: bool,
    start: usize,
    /// Where the item's text begins, in bytes: a line indented this far
    /// belongs to the item.
    width: usize,
    /// The same, for the first line's text: past a task's box.
    text: usize,
    /// **A task** (2026-11-08), GFM's `- [ ]` and `- [x]`: done or not.
    task: Option<bool>,
}

/// The list item `line` opens, if it opens one: `-`, `*` or `+` and a space,
/// or a number and `.` or `)`. In Typst `+` numbers the item.
fn list_item(line: &str, source: crate::syntax::Syntax) -> Option<Item> {
    let indent = lead(line);
    let body = &line[indent..];
    if is_rule(body.trim_end()) {
        return None;
    }
    let typst = source == crate::syntax::Syntax::Typst;
    let digits = body.find(|c: char| !c.is_ascii_digit()).unwrap_or(body.len());
    let (ordered, start, marker) = match body.chars().next()? {
        '-' => (false, 1, 1),
        '*' if !typst => (false, 1, 1),
        '+' => (typst, 1, 1),
        c if c.is_ascii_digit() && (1..=9).contains(&digits) && body[digits..].starts_with(['.', ')']) => {
            (true, body[..digits].parse().ok()?, digits + 1)
        }
        _ => return None,
    };
    let after = &body[marker..];
    let gap = after.len() - after.trim_start_matches(' ').len();
    if !(after.is_empty() || gap > 0) {
        return None;
    }
    let width = indent + marker + gap.clamp(1, 4);
    let text = indent + marker + gap;
    // A task's box: `[ ]`, `[x]` or `[X]`, then a space or the line's end.
    let rest = &line[text..];
    let task = match (typst, rest.get(..3)) {
        (false, Some("[ ]")) => Some(false),
        (false, Some("[x]" | "[X]")) => Some(true),
        _ => None,
    }
    .filter(|_| rest[3..].is_empty() || rest[3..].starts_with(' '));
    let text = match task {
        Some(_) => text + 3 + usize::from(rest[3..].starts_with(' ')),
        None => text,
    };
    Some(Item { indent, ordered, start, width, text, task })
}

/// **How far a line is indented, in bytes of spaces and tabs** (2026-10-30).
///
/// Only those two, as CommonMark counts indentation. `trim_start` also takes
/// U+3000, the full-width space a Chinese paragraph opens with, which is three
/// bytes: counted as indent it made `next[width..]` cut into the middle of one,
/// and `:export` panicked on a list followed by an indented paragraph.
fn lead(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// What follows the marker on an item's first line.
fn line_after(line: &str, at: usize) -> String {
    line.get(at..).unwrap_or_default().trim_end().to_string()
}

/// Rewrite one line for `dialect`: its readings, and its markup.
///
/// Two things happen here that used not to. **`**很好**` comes out as bold**,
/// rather than as four asterisks — an export that prints the markup literally
/// is not an export, it is a copy. And **a 批注 does not leave the manuscript**:
/// `%%私話%%` is the writer talking to themselves, the manual says so, and it
/// was going into the file handed to a publisher.
fn line_into(
    line: &str,
    dialects: Dialects,
    dialect: Dialect,
    escape: fn(&str) -> String,
    source: crate::syntax::Syntax,
    painter: Option<Drawing>,
) -> String {
    use crate::syntax::Syntax;
    let chars: Vec<char> = line.chars().collect();
    // The brace form is Markdown's (YFM): in a Typst or a plain file a
    // `{a|b}` is the writer's own braces.
    let mut dialects = dialects;
    if source != Syntax::Markdown {
        dialects.remove(Dialect::Markdown);
    }
    let groups = ruby::groups(&chars, dialects);
    // A formula is drawn only where it is TeX — Markdown's — and only into a
    // page that can show a picture; Typst's `$…$` is Typst's maths.
    let paint = match (dialect, source) {
        (Dialect::Html, Syntax::Typst | Syntax::Text) => None,
        (Dialect::Html, _) => painter,
        (Dialect::Typst | Dialect::Markdown, _) => None,
    };
    // Warning: **讀的是源檔的語法，不是要寫成的格式**（2026-09-28）。從前這裏寫死
    // `markdown::spans`，於是一份 Typst 稿子導出去，`*粗*` 是四個字符、`$a+b$` 被
    // 轉義成 `\$a + b\$`。
    let marks = match source {
        Syntax::Typst => crate::markdown::typst::spans(line),
        Syntax::Text => Vec::new(),
        _ if paint.is_some() => crate::markdown::spans(line),
        _ => without_maths(crate::markdown::spans(line)),
    };
    render(&chars, &groups, &marks, None, 0, chars.len(), dialect, escape, paint)
}

/// One run of `chars`, with whatever is marked **inside** it marked too.
///
/// Warning: **巢狀的標記從前整段吞掉**（2026-10-01 修）。這一趟從前是平的：
/// `marks.iter().find(…)` 取頭一個蓋住 `at` 的 span——而 `spans()` 交出來的是
/// **外層在前**的嵌套表，於是取到的永遠是最外那一層，整段原文連同裏層的分隔符
/// 一起交給 [`marked`]，`at` 再一步跨到那一層的末尾。結果：
///
/// | 寫的 | HTML 從前 | Typst 從前 |
/// | --- | --- | --- |
/// | ``**粗體裏的 `代碼`**`` | `<strong>粗體裏的 \`代碼\`</strong>` | `*粗體裏的 \\\`代碼\\\`*` |
/// | `*斜體裏的 **粗體***` | `<em>斜體裏的 **粗體**</em>` | `_斜體裏的 \\*\\*粗體\\*\\*_` |
///
/// 反引號與星號**印進了書裏**，Typst 那一端還帶着反斜線。`Span::depth` 當初存
/// 下來就是為了這一趟（它的註釋寫着「`:export` 那一趟棧式遍歷有了它是十行」），
/// 可這裏不看 `depth`——Typst 的 `typst::spans` 給誰都蓋 `depth: 0`，看它會把
/// Typst 那一端弄壞。認「不是我自己」就夠了：`marks` 是排過序的嵌套表，所以
/// 第一個落在 `from..to` 裏、又不是 `parent` 的那一條，正是下一層。
#[allow(clippy::too_many_arguments)]
fn render(
    chars: &[char],
    groups: &[ruby::Ruby],
    marks: &[crate::markdown::Span],
    // `marks` 裏「我自己」是第幾條——不排除它，它會把自己再匹配一次，無窮遞歸。
    parent: Option<usize>,
    from: usize,
    to: usize,
    dialect: Dialect,
    escape: fn(&str) -> String,
    // Set only for Markdown going to HTML: what draws its TeX.
    paint: Option<Drawing>,
) -> String {
    use crate::markdown::Kind;
    let mut out = String::new();
    let mut plain = String::new();
    let mut at = from;
    while at < to {
        // A reading group first: it is markup of its own, and its base may hold
        // characters the Markdown scan would read as delimiters.
        if let Some(group) = groups.iter().find(|g| g.start == at && g.end <= to) {
            out.push_str(&escape(&std::mem::take(&mut plain)));
            // Read out of the dialect it was written in first (#332).
            let base = group.base_of(chars);
            let reading = group.reading_of(chars);
            // The text inside the markup is the writer's, so it is escaped —
            // but **by the rule of the place it lands in**. Typst's call takes
            // two string literals, where `escape` (which escapes Typst
            // *markup*) writes `\*` and the compiler refuses it: inside `"…"`
            // that is an unknown escape sequence. `Dialect::write` knows this
            // and does it; here the target is markup only for HTML.
            let (base, reading) = match dialect {
                Dialect::Html => (escape(&base), escape(&reading)),
                Dialect::Typst | Dialect::Markdown => (base, reading),
            };
            let base: Vec<char> = base.chars().collect();
            out.push_str(&ruby::markup(&base, &reading, dialect));
            at = group.end;
            continue;
        }
        let next = marks.iter().enumerate().find(|(i, s)| {
            Some(*i) != parent
                && s.start >= from
                && s.end <= to
                && at >= s.start
                && at < s.end
                // Warning: **和外面那一條一樣大的不算「裏面那一層」**（2026-10-01
                // 審出來的，是這一趟改出來的回歸）。只排除「我自己那一條」不
                // 夠：`markdown::spans` 真的交得出**兩條範圍完全相同**的 span。
                // 腳註那一支（`markdown.rs:821`）把整個 `[^1]` 收成一條，前面
                // 不帶 `Marker`——而別的構造都帶，那一截 `Marker` 正是讓裏層嚴
                // 格小於外層的東西。於是 `**[^1]**` 的 `Strong` 與 `Footnote`
                // 一樣大，兩條互相當對方的「裏層」，**無窮遞歸，棧溢出**。
                // `*[^1]*`、`==[^1]==`、`[**[^1]**](u)` 同。
                && !(parent.is_some() && s.start == from && s.end == to)
        });
        match next {
            Some((i, span)) => {
                out.push_str(&escape(&std::mem::take(&mut plain)));
                let start = at.max(span.start);
                let end = span.end.min(to);
                if let (Kind::Math, Some(drawing)) = (span.kind, paint) {
                    let tex: String = chars[start..end].iter().collect();
                    // `$$` on both sides is a formula set on its own line.
                    let display = start >= 2 && chars[start - 2..start] == ['$', '$'];
                    let turn = drawing.turns(&tex, display);
                    out.push_str(&match (drawing.painter)(&tex, display, turn, drawing.ink) {
                        Ok(drawn) => formula_img(&drawn, &tex, display, turn),
                        // What will not draw is left as it was written, `$`s
                        // and all, which is what a reader of the source sees.
                        Err(_) => {
                            let fence = if display { "$$" } else { "$" };
                            format!("{fence}{}{fence}", escape(&tex))
                        }
                    });
                    at = end.max(at + 1);
                    continue;
                }
                // **A picture with no words** (2026-11-09, reported: `![](a.png)`
                // came out as nothing): there is no link span between its two
                // markers, `![` and `](a.png)`, so the picture is drawn at the
                // second.
                let wordless = span.kind == Kind::Marker
                    && dialect == Dialect::Html
                    && chars[start] == ']'
                    && start >= 2
                    && chars[start - 2..start] == ['!', '[']
                    && marks.iter().any(|m| m.kind == Kind::Marker && m.construct == span.construct && m.end == start);
                if let Some(src) = link_target(chars, start, start).filter(|_| wordless) {
                    out.push_str(&format!("<img class=\"inline\" src=\"{}\" alt=\"\">", url_attribute(&src)));
                }
                // **這三種不往裏走。** 代碼與批注裏寫的就是字面（`spans()` 本來
                // 也不往它們裏面遞），而分隔符自己没有內容。
                let leaf = matches!(span.kind, Kind::Code | Kind::Comment | Kind::Marker | Kind::Math | Kind::Html);
                let body = match leaf {
                    true => {
                        let text: String = chars[start..end].iter().collect();
                        // Warning: Typst 的反引號裏是**原樣**文本，`\` 就是一個反
                        // 斜線，所以 `` `code_here` `` 從前出成 `` `code\_here` ``
                        // （#386）。HTML 的 `<code>` 正相反，`&lt;` 是必須的。
                        match (span.kind, dialect) {
                            // A formula is read by the maths, not as markup:
                            // `$a_b$` with its `_` escaped is a different formula.
                            (Kind::Code | Kind::Math, Dialect::Typst) | (Kind::Html, Dialect::Html) => text,
                            _ => escape(&text),
                        }
                    }
                    false => {
                        render(chars, groups, marks, Some(i), start, end, dialect, escape, paint)
                    }
                };
                // **A link is a link on a web page** (2026-11-08): its words
                // in an `<a href>`, as the Markdown preview shows them. In
                // Typst the target is the manuscript's scaffolding and only
                // the words go over.
                let target = match (span.kind, dialect) {
                    (Kind::Link, Dialect::Html) => link_target(chars, span.start, span.end),
                    _ => None,
                };
                // **A picture in a line is a picture** (2026-11-09), as the
                // preview draws it: `![小图](a.png)` among words was a link
                // to the file, which in an EPUB pointed out of the book.
                let picture = span.start >= 2 && chars[span.start - 2] == '!' && chars[span.start - 1] == '[';
                match target {
                    Some(src) if picture => {
                        let alt: String = chars[span.start..span.end].iter().collect();
                        let alt = escape_html(&alt).replace('"', "&quot;");
                        out.push_str(&format!("<img class=\"inline\" src=\"{}\" alt=\"{alt}\">", url_attribute(&src)));
                    }
                    Some(href) => out.push_str(&format!("<a href=\"{}\">{body}</a>", url_attribute(&href))),
                    None => out.push_str(&marked(span.kind, &body, dialect)),
                }
                at = end.max(at + 1);
            }
            None => {
                plain.push(chars[at]);
                at += 1;
            }
        }
    }
    out.push_str(&escape(&plain));
    out
}

/// **Where the link whose words are `chars[start..end]` goes**: the target
/// of `[words](target "title")`, or the address of `<address>`.
fn link_target(chars: &[char], start: usize, end: usize) -> Option<String> {
    if start > 0 && chars[start - 1] == '<' && chars.get(end) == Some(&'>') {
        return Some(chars[start..end].iter().collect());
    }
    if chars.get(end) != Some(&']') || chars.get(end + 1) != Some(&'(') {
        return None;
    }
    let mut depth = 0usize;
    let mut target = String::new();
    for &c in &chars[end + 2..] {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => break,
            ')' => depth -= 1,
            _ => {}
        }
        target.push(c);
    }
    // **`<…>` first** (2026-11-09): a target in angle brackets may hold
    // spaces, as CommonMark has it, and `[文字](<a b.html>)` was cut at its
    // space into `<a`. Otherwise the target ends at a space, where a title
    // may follow.
    let target = target.trim();
    let target = match target.strip_prefix('<') {
        Some(inside) => inside.split('>').next().unwrap_or_default(),
        None => target.split_once(char::is_whitespace).map_or(target, |(t, _)| t),
    };
    (!target.is_empty()).then(|| target.to_string())
}

/// **A path as a page's `src` or `href` takes it**: escaped, and its spaces
/// written `%20`, as a URL holds them (`<my pic.png>` in the source).
fn url_attribute(path: &str) -> String {
    escape_html(&path.replace(' ', "%20")).replace('"', "&quot;")
}

/// **Markdown's maths is LaTeX**, which Typst cannot read (`\frac` stops
/// the PDF): its `$…$` and `$$…$$` go over as written text, escaped, as
/// before Markdown drew them as maths. A formula and its dollars are one
/// construct.
fn without_maths(mut marks: Vec<crate::markdown::Span>) -> Vec<crate::markdown::Span> {
    use crate::markdown::Kind;
    let maths: Vec<(usize, u8)> = marks.iter().filter(|s| s.kind == Kind::Math).map(|s| (s.construct, s.depth)).collect();
    marks.retain(|s| !maths.contains(&(s.construct, s.depth)));
    marks
}

/// One marked run, written the way `dialect` writes it.
///
/// The markers themselves and the 批注 come out as nothing at all — those are
/// the two runs that are *about* the manuscript rather than part of it.
/// Warning: **`text` 已經排好了**（2026-10-01）：裏層的標記已經包過，該轉義的已
/// 經轉義過。這一支只管外面那一層怎麽寫。從前它自己 `escape`，而那在巢狀之後
/// 是錯的——裏層排出來的 `<strong>` 會被當成寫稿人打的字再轉義一遍。
fn marked(kind: crate::markdown::Kind, text: &str, dialect: Dialect) -> String {
    use crate::markdown::Kind;
    match (kind, dialect) {
        // The delimiters, and the writer's private notes. Not in the book.
        (Kind::Marker | Kind::Comment, _) => String::new(),
        // Inline HTML is HTML; Typst has nothing to make of a tag.
        (Kind::Html, Dialect::Html) => text.to_string(),
        (Kind::Html, Dialect::Typst) => String::new(),
        (Kind::Strong, Dialect::Html) => format!("<strong>{text}</strong>"),
        (Kind::Emphasis, Dialect::Html) => format!("<em>{text}</em>"),
        (Kind::Code, Dialect::Html) => format!("<code>{text}</code>"),
        (Kind::Strike, Dialect::Html) => format!("<s>{text}</s>"),
        (Kind::Highlight, Dialect::Html) => format!("<mark>{text}</mark>"),
        // Warning: **公式原樣留着**（2026-09-28 修）。從前它借 `Kind::Code`，於是導出的
        // Typst 裏是 `` `a + b` ``——一條反引號在 Typst 裏是原樣文本，公式就此不是公
        // 式了。HTML 那一端也留着 `$…$`：瀏覽器自己不認，可是掛了 MathJax 的頁認，
        // 而換成 `<code>` 是連掛了也認不出來。
        (Kind::Math, _) => format!("${text}$"),
        // 交叉引用在 Typst 裏原樣就對；HTML 没有對應的東西，留着字面比丟掉好。
        (Kind::Ref, _) => text.to_string(),
        // 標籤是稿子的腳手架：Typst 要，書不要。
        (Kind::Label, Dialect::Typst) => text.to_string(),
        (Kind::Label, Dialect::Html) => String::new(),
        (Kind::Strong, Dialect::Typst) => format!("*{text}*"),
        (Kind::Emphasis, Dialect::Typst) => format!("_{text}_"),
        (Kind::Code, Dialect::Typst) => format!("`{text}`"),
        (Kind::Strike, Dialect::Typst) => format!("#strike[{text}]"),
        (Kind::Highlight, Dialect::Typst) => format!("#highlight[{text}]"),
        // A footnote's mark, numbered once the whole text is out (see
        // [`number_the_notes`]): the name, whatever escaping it came with.
        (Kind::Footnote, _) => {
            format!("{NOTE_OPEN}{}{NOTE_CLOSE}", note_name(text))
        }
        // A link's target is scaffolding for a manuscript, not part of the
        // book; its visible text is the book. The heading's own hashes are
        // handled a level up, where the heading is.
        _ => text.to_string(),
    }
}

/// **A drawn formula as an `<img>`** (2026-11-08): the PNG inline, so the page
/// stays one file; the TeX as its `alt`, for a screen reader and for whatever
/// copies the picture's text; and its size in `em`, so it grows and shrinks
/// with the text — the print stylesheet changes the type size, and a size in
/// pixels would not follow it. `--depth` puts its baseline on the line's
/// across the page; down a 縱 it is centred instead (see [`html`]).
///
/// **Turned**, the picture came from the painter already turned in the
/// bitmap and measured as it is shown, so it is an ordinary picture with
/// nothing to rotate: of the ways to turn a formula, the one every engine
/// and every reader app shows the same (2026-11-08, Chrome, Firefox, WebKit
/// and Readium's stylesheets tried). Only its height is given, so a long one
/// that a page cannot hold shrinks whole rather than squeezed.
fn formula_img(drawn: &Formula, tex: &str, display: bool, turn: bool) -> String {
    let class = match (display, turn) {
        (false, false) => "math",
        (true, false) => "math display",
        (false, true) => "math turned",
        (true, true) => "math display turned",
    };
    let alt = escape_html(tex).replace('"', "&quot;");
    let png = crate::preview::base64(&drawn.png);
    format!(
        "<img class=\"{class}\" alt=\"{alt}\" style=\"height: {:.3}em; --depth: {:.3}em\" src=\"data:image/png;base64,{png}\">",
        drawn.height, -drawn.depth,
    )
}

/// **What a vertical page does to its text that CSS cannot be asked to**
/// (2026-11-08), done to the HTML `body` once it is written.
///
/// - **Half-width letters and digits** are set as `latin` says
///   ([`Latin`]), each run of them with the half-width marks joined to it
///   taken as one: `<span class="yoko">` turns it a quarter turn clockwise,
///   `<span class="tcy">` sets it across in one square (縱中橫), and
///   `<span class="tate">` stands its letters upright one to a square.
///   Spelling it out rather than leaving it to `text-orientation: mixed`
///   keeps a run whole: left to the font, `’` down a 縱 is a closing quote,
///   and a CJK font's vertical form for it is a corner bracket — KingHwa
///   OldSong drew `don’t` as `don」t` — and `%` may stand while the `5`
///   beside it lies.
/// - **Curly quotes become corner brackets**, `“”` as `『』` and `‘’` as
///   `「」`, when `quotes` — the page is Chinese. That is what the terminal's
///   `:render full` draws, and an export is the page with its markup taken
///   off, which is what `full` means. A font's own vertical forms cannot be
///   relied on for these: Songti and PingFang leave `“` lying flat, KingHwa
///   OldSong swaps the double and single pairs, and a page cannot know which
///   font the reader has. The brackets are real characters every CJK font
///   turns, so a copy is still Chinese. The other marks — `，。：（）——……` —
///   stay as written: CJK fonts turn those themselves, and their presentation
///   forms (U+FE10…) would break copying and searching for no gain. A single
///   quote between two Latin letters is an apostrophe and is left alone.
///
/// Nothing inside a tag is touched, nor what is in `<code>`, `<pre>`, a
/// reading (`<rt>`) or a script. A table's cells are running text: a table
/// down the page is a vertical one (see [`stylesheet`]).
fn vertical_text(body: &str, latin: Latin, quotes: bool) -> String {
    let body = match quotes {
        true => std::borrow::Cow::Owned(quotes_cornered(body)),
        false => std::borrow::Cow::Borrowed(body),
    };
    text_between_tags(&body, |text| turned(text, latin))
}

/// **`body` with its curly quotes as corner brackets** where
/// [`corner_quotes`] says, each block's text read whole (2026-11-09): a pair
/// is told Chinese or not by what it holds, and `“<strong>走</strong>”` holds
/// its words in another run between tags. A paragraph, a heading, a cell or
/// an item is a block; what is not touched in [`text_between_tags`] is not
/// touched here. A quote and its bracket are three bytes each, so each is
/// put in the other's place.
fn quotes_cornered(body: &str) -> String {
    const SKIP: &[&str] = &["code", "pre", "rt", "rp", "script", "style", "svg", "math", "kbd", "samp"];
    const BLOCKS: &[&str] = &[
        "p", "h1", "h2", "h3", "h4", "h5", "h6", "li", "td", "th", "caption", "figcaption", "blockquote", "div", "section", "br", "hr",
        "table", "tr", "ul", "ol", "figure",
    ];
    let mut bytes = body.as_bytes().to_vec();
    // The block's text: where each character is, and the character it is.
    let mut held: Vec<(usize, char)> = Vec::new();
    let flush = |held: &mut Vec<(usize, char)>, bytes: &mut Vec<u8>| {
        let chars: Vec<char> = held.iter().map(|&(_, c)| c).collect();
        for ((at, was), now) in held.iter().zip(corner_quotes(&chars)) {
            if now != *was && now.len_utf8() == was.len_utf8() {
                now.encode_utf8(&mut bytes[*at..*at + now.len_utf8()]);
            }
        }
        held.clear();
    };
    let mut skipping: Vec<String> = Vec::new();
    let mut at = 0;
    while let Some(c) = body[at..].chars().next() {
        let tag = match c {
            '<' => body[at..].find('>').map(|end| &body[at..=at + end]),
            _ => None,
        };
        let Some(tag) = tag else {
            if skipping.is_empty() {
                // An entity is one character, as `turned` reads it.
                let entity = match c {
                    '&' => body[at..].find(';').filter(|&n| (2..=12).contains(&n)),
                    _ => None,
                };
                held.push((at, if entity.is_some() { '&' } else { c }));
                at += entity.map_or(c.len_utf8(), |n| n + 1);
            } else {
                at += c.len_utf8();
            }
            continue;
        };
        let inner = tag[1..tag.len() - 1].trim();
        let name = inner.trim_start_matches('/').chars().take_while(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
        if BLOCKS.contains(&name.as_str()) {
            flush(&mut held, &mut bytes);
        }
        if inner.starts_with('/') {
            if skipping.last() == Some(&name) {
                skipping.pop();
            }
        } else if SKIP.contains(&name.as_str()) && !inner.ends_with('/') {
            skipping.push(name);
        }
        at += tag.len();
    }
    flush(&mut held, &mut bytes);
    String::from_utf8(bytes).unwrap_or_else(|_| body.to_string())
}

/// **`body` with each run of text between its tags done as `f` says**
/// (2026-11-08): what is inside a tag is not touched, nor what is in
/// `<code>`, `<pre>`, a reading (`<rt>`) or a script.
pub(crate) fn text_between_tags(body: &str, f: impl Fn(&str) -> String) -> String {
    const SKIP: &[&str] = &["code", "pre", "rt", "rp", "script", "style", "svg", "math", "kbd", "samp"];
    let mut out = String::with_capacity(body.len() + body.len() / 8);
    let mut skipping: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut rest = body;
    while let Some(c) = rest.chars().next() {
        let tag = match c {
            '<' => rest.find('>').map(|end| &rest[..=end]),
            _ => None,
        };
        let Some(tag) = tag else {
            text.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        };
        let held = std::mem::take(&mut text);
        out.push_str(&match skipping.is_empty() {
            true => f(&held),
            false => held,
        });
        out.push_str(tag);
        let inner = tag[1..tag.len() - 1].trim();
        let name = inner
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        if inner.starts_with('/') {
            if skipping.last() == Some(&name) {
                skipping.pop();
            }
        } else if SKIP.contains(&name.as_str()) && !inner.ends_with('/') {
            skipping.push(name);
        }
        rest = &rest[tag.len()..];
    }
    out.push_str(&match skipping.is_empty() {
        true => f(&text),
        false => text,
    });
    out
}

/// A Latin letter or digit: alphanumeric, and from a script set across —
/// below U+1100, where the CJK blocks begin.
fn latin_letter(c: char) -> bool {
    c.is_alphanumeric() && (c as u32) < 0x1100
}

/// A half-width character a run of Latin may hold: a letter or digit, or a
/// mark from the same blocks (`%`, `.`, `-`, `(`, `°`). Not a space, and not
/// the punctuation of U+2000 on, which CJK text shares — except `’` between
/// two letters, which is an apostrophe ([`turned`]).
fn half_width(c: char) -> bool {
    (c as u32) < 0x1100 && !c.is_whitespace() && !c.is_control()
}

/// **How a run of half-width letters and digits stands down a vertical
/// page** (2026-11-08): what [`latin_runs`] says of each run, the HTML
/// export's `<span>` classes and the Markdown preview's drawing alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Turned a quarter turn clockwise, as a whole (`yoko`).
    Turned,
    /// Across in one square, 縱中橫 (`tcy`).
    Across,
    /// Upright, one character to a square (`tate`).
    Upright,
}

impl Orientation {
    /// The class of the export's `<span>`.
    fn class(self) -> &'static str {
        match self {
            Orientation::Turned => "yoko",
            Orientation::Across => "tcy",
            Orientation::Upright => "tate",
        }
    }
}

/// **How one run of Latin is set under [`Latin::Jlreq`]** (2026-11-08).
///
/// - A single letter, a capital or a small one: upright (2026-11-09: a small
///   one was turned, while `$x$` beside it stood); with marks joined to it
///   (`x.`, `(x)`) the letter alone, as [`latin_runs`] cuts it.
/// - A phrase (it holds a space) or a word with a small letter in it: turned.
/// - Digits alone: one upright, two across in one square, more turned.
/// - Capitals, with digits or not: upright, one to a square (`A`, `NHK`,
///   `MP3`), except two characters with a digit among them (`A4`, `3D`),
///   which go across in one square as two digits do.
/// - A number with a mark joined to it (`5%`, `3.14`): across in one square
///   if it is two characters, turned otherwise — one way as a whole, never
///   the `5` upright and the `%` lying.
/// - Anything else with a mark in it (`U.S.`, `C++`): turned.
fn jlreq_orientation(run: &[char]) -> Orientation {
    use Orientation::*;
    let letters = run.iter().filter(|c| c.is_alphabetic()).count();
    let digits = run.iter().filter(|c| c.is_numeric()).count();
    let marks = run.len() - letters - digits;
    if run.len() == 1 && letters == 1 {
        return Upright;
    }
    if run.contains(&' ') || run.iter().any(|c| c.is_lowercase()) {
        return Turned;
    }
    match (letters, digits, marks) {
        (0, 1, 0) => Upright,
        (0, 2, 0) => Across,
        (0, _, 0) => Turned,
        (_, 0, 0) => Upright,
        (_, _, 0) if run.len() == 2 => Across,
        (_, _, 0) => Upright,
        (0, _, _) if run.len() <= 2 => Across,
        _ => Turned,
    }
}

/// **Curly quotes as corner brackets** (2026-11-08), `“”` as `『』` and `‘’`
/// as `「」`, the way a vertical Chinese page sets them; a single quote
/// between two Latin letters is an apostrophe and is left alone. Called for
/// a Chinese page only ([`corners_quotes`]).
///
/// **Only round Chinese** (2026-11-09): a pair of quotes round Western text
/// keeps its own marks, as clreq has quoted Western text keep them — `他说
/// “go”` keeps `“go”`, and an English sentence keeps every quote it has.
///
/// - A pair is **Western** only when what it holds has a Latin letter in it
///   and no CJK character; it is **Chinese** when it holds a CJK character.
/// - A pair that holds neither — only marks, digits or code (`“……”`, `“——”`,
///   `“1949”`, ``“`走`”``) — **follows the text round it** (2026-11-09,
///   reported: those stayed curly in a Chinese paragraph): the nearest
///   letter outside it on either side, Chinese if either is CJK, Western if
///   what is found is Latin, and Chinese when there is none, the page being
///   Chinese.
/// - A quote with no partner (a quotation running on over paragraphs opens
///   each one) goes by the first letter on its inner side, and by the text
///   round it when there is none.
pub fn corner_quotes(chars: &[char]) -> Vec<char> {
    let latin_at = |k: Option<usize>| k.and_then(|k| chars.get(k)).is_some_and(|&c| latin_letter(c));
    let apostrophe = |k: usize| latin_at(k.checked_sub(1)) && latin_at(Some(k + 1));
    let cjk = |c: char| (c as u32) >= 0x2E80 && !c.is_whitespace() && !matches!(c, '“' | '”' | '‘' | '’');
    let western = |c: char| c.is_alphabetic() && (c as u32) < 0x1100;
    // What a letter says: Chinese, Western, or nothing (a mark, a digit).
    let says = |c: char| match (cjk(c), western(c)) {
        (true, _) => Some(true),
        (false, true) => Some(false),
        _ => None,
    };
    // Each quote's partner, the quotes nested as they are written.
    let mut partner: Vec<Option<usize>> = vec![None; chars.len()];
    let mut open: Vec<usize> = Vec::new();
    for k in 0..chars.len() {
        let closing = match chars[k] {
            '“' | '‘' if !apostrophe(k) => {
                open.push(k);
                continue;
            }
            '”' if !apostrophe(k) => '“',
            '’' if !apostrophe(k) => '‘',
            _ => continue,
        };
        if let Some(at) = open.iter().rposition(|&o| chars[o] == closing) {
            let o = open[at];
            open.truncate(at);
            partner[o] = Some(k);
            partner[k] = Some(o);
        }
    }
    // The text round `from..=to`: the nearest letter before and after it.
    let round = |from: usize, to: usize| {
        let before = chars[..from].iter().rev().find_map(|&c| says(c));
        let after = chars[to + 1..].iter().find_map(|&c| says(c));
        matches!((before, after), (Some(true), _) | (_, Some(true)) | (None, None))
    };
    let chinese = |k: usize| match partner[k] {
        Some(other) => {
            let (a, b) = (k.min(other), k.max(other));
            let inside = &chars[a + 1..b];
            match (inside.iter().any(|&c| cjk(c)), inside.iter().any(|&c| western(c))) {
                (true, _) => true,
                (false, true) => false,
                (false, false) => round(a, b),
            }
        }
        None => {
            let inner = match chars[k] {
                '“' | '‘' => chars[k + 1..].iter().find_map(|&c| says(c)),
                _ => chars[..k].iter().rev().find_map(|&c| says(c)),
            };
            inner.unwrap_or_else(|| round(k, k))
        }
    };
    (0..chars.len())
        .map(|k| match chars[k] {
            '“' if chinese(k) => '『',
            '”' if chinese(k) => '』',
            '‘' if !apostrophe(k) && chinese(k) => '「',
            '’' if !apostrophe(k) && chinese(k) => '」',
            c => c,
        })
        .collect()
}

/// **Whether a page's curly quotes become corner brackets** (2026-11-09):
/// on a page in Chinese, as [`document_language`] decides it. The one rule
/// for the vertical export, the EPUB and the vertical preview alike — the
/// preview asked its writing system, which a Chinese note with a page of
/// English code in it answered differently.
pub fn corners_quotes(lang: &str) -> bool {
    lang == "zh"
}

/// **The language a page of `text` is set in** (2026-11-09): `zh`, `ja` or
/// `en`, from its prose, comments left out — what the HTML and EPUB exports
/// write as `lang`, and what the vertical preview asks too.
pub fn document_language(text: &str) -> &'static str {
    language_of(&crate::markdown::strip_comments(text))
}

/// **A half-width mark that stands upright down a vertical page**
/// (2026-11-09): those UAX #50 gives orientation U, `§ © ® ± ¼ ½ ¾ × ÷`
/// and two tone marks, which `text-orientation: mixed` leaves standing. On
/// its own, out of a run of Latin, such a mark stands in the export, and the
/// preview stands it too; any other half-width mark on its own is turned.
pub fn upright_mark(c: char) -> bool {
    matches!(c as u32, 0xA7 | 0xA9 | 0xAE | 0xB1 | 0xBC..=0xBE | 0xD7 | 0xF7 | 0x2EA..=0x2EB)
}

/// **The runs of Latin in `chars` and how each stands** (2026-11-08): runs
/// of half-width characters with a letter or digit in them, the half-width
/// marks joined to them (`5%`, `don’t`, `’` taken only between two letters),
/// and words with one space between them joined into one phrase. Under
/// [`Latin::Turn`] every run is turned; under [`Latin::Jlreq`] as
/// [`jlreq_orientation`] says.
pub fn latin_runs(chars: &[char], latin: Latin) -> Vec<(std::ops::Range<usize>, Orientation)> {
    let latin_at = |k: Option<usize>| k.and_then(|k| chars.get(k)).is_some_and(|&c| latin_letter(c));
    let apostrophe = |k: usize| latin_at(k.checked_sub(1)) && latin_at(Some(k + 1));
    let joins = |k: usize| half_width(chars[k]) || (chars[k] == '’' && apostrophe(k));
    // The words: runs of half-width characters with a letter or digit in
    // them. Then the phrases: words with one space between.
    let mut words: Vec<std::ops::Range<usize>> = Vec::new();
    let mut k = 0;
    while k < chars.len() {
        if !joins(k) {
            k += 1;
            continue;
        }
        let from = k;
        while k < chars.len() && joins(k) {
            k += 1;
        }
        if chars[from..k].iter().any(|&c| latin_letter(c)) {
            words.push(from..k);
        }
    }
    //
    // **A single letter, its marks let go** (2026-11-09, reported: `x.`,
    // `(x)` and `x,` were turned while `x` stood): under `jlreq` a word with
    // one letter and no digit stands upright as the letter alone does, the
    // letter in its square and the marks beside it as marks on their own
    // are set; and two such words with a space between (`x, y`) are two
    // letters, not a phrase.
    let single = |r: &std::ops::Range<usize>| {
        let word = &chars[r.clone()];
        word.iter().filter(|c| c.is_alphabetic()).count() == 1 && !word.iter().any(|c| c.is_numeric())
    };
    let mut runs: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
    for word in words {
        match runs.last_mut() {
            Some((last, one))
                if last.end + 1 == word.start
                    && chars[last.end] == ' '
                    && !(latin == Latin::Jlreq && *one && single(last) && single(&word)) =>
            {
                last.end = word.end;
                *one = false;
            }
            _ => runs.push((word, true)),
        }
    }
    runs.into_iter()
        .map(|(run, one)| match latin {
            Latin::Turn => (run, Orientation::Turned),
            // The letter with the marks that combine with it (`e` and U+0301
            // written apart) is one character as a reader sees it: cut by its
            // grapheme cluster, not by `char` (review, 2026-11-09).
            Latin::Jlreq if one && single(&run) => {
                let at = run.start + chars[run.clone()].iter().position(|c| c.is_alphabetic()).unwrap_or(0);
                let rest: String = chars[at..run.end].iter().collect();
                let cluster = yumete_cjk::grapheme::graphemes(&rest).next().map_or(1, |g| g.chars().count()).max(1);
                (at..at + cluster, Orientation::Upright)
            }
            Latin::Jlreq => {
                let how = jlreq_orientation(&chars[run.clone()]);
                (run, how)
            }
        })
        .collect()
}

/// One run of text between tags, done as [`vertical_text`] says. An entity
/// (`&amp;`) is one character, the one it stands for: `R&amp;D` is one run.
/// Its quotes are already what they will be ([`quotes_cornered`]).
fn turned(text: &str, latin: Latin) -> String {
    // What each piece is written as, and the character it stands for.
    let mut written: Vec<&str> = Vec::new();
    let mut typed: Vec<char> = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let entity = match c {
            '&' => rest.find(';').filter(|&n| (2..=12).contains(&n)).map(|n| &rest[..=n]),
            _ => None,
        };
        let piece = entity.unwrap_or(&rest[..c.len_utf8()]);
        typed.push(match piece {
            "&amp;" => '&',
            "&lt;" => '<',
            "&gt;" => '>',
            "&quot;" => '"',
            "&nbsp;" => '\u{a0}',
            _ if entity.is_some() => '&',
            _ => c,
        });
        written.push(piece);
        rest = &rest[piece.len()..];
    }
    let chars = typed;
    let piece = |k: usize| written[k].to_string();
    let runs = latin_runs(&chars, latin);
    let mut out = String::with_capacity(text.len() + runs.len() * 28);
    let mut at = 0;
    for (run, how) in runs {
        (at..run.start).for_each(|k| out.push_str(&piece(k)));
        out.push_str(&format!("<span class=\"{}\">", how.class()));
        run.clone().for_each(|k| out.push_str(&piece(k)));
        out.push_str("</span>");
        at = run.end;
    }
    (at..chars.len()).for_each(|k| out.push_str(&piece(k)));
    out
}

/// `&`, `<` and `>` as a browser needs them.
pub(crate) fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Typst's own escapes: the characters that would otherwise be markup.
fn escape_typst(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '#' | '$' | '\\' | '*' | '_' | '`' | '<' | '>' | '@' | '[' | ']'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// **The font list a page asks for** (2026-11-08): `[export] font_family`,
/// whatever the language, with nothing in it that could end the rule or the
/// `<style>` it is written in. No third-party font is named unless the
/// writer names it: the default is the system's own faces, as VS Code's
/// Markdown preview asks for them.
fn font_list(style: &Style) -> String {
    let fonts: String = style.settings.fonts.chars().filter(|c| !matches!(c, ';' | '{' | '}' | '<' | '>')).collect();
    match fonts.trim() {
        "" => DEFAULT_FONTS.to_string(),
        fonts => fonts.to_string(),
    }
}

/// A standalone HTML page — the export that reproduces what is on screen.
fn html(text: &str, style: &Style) -> String {
    let lang = document_language(text);
    let text = crate::markdown::strip_comments(text);
    let mut out = String::new();
    out.push_str(&format!("<!doctype html>\n<html lang=\"{lang}\">\n<meta charset=\"utf-8\">\n"));
    out.push_str(&format!("<title>{}</title>\n", escape_html(&style.title)));
    out.push_str("<style>\n");
    out.push_str(&stylesheet(style, lang, false));
    out.push_str(&print_rules(style));
    out.push_str("</style>\n");
    out.push_str(&html_body(&text, style, lang));
    out
}

/// The page's stylesheet, but for what only a printer sees.
///
/// A `book` is an EPUB's: a reader app sets the page, so there is no measure
/// to pin and no margin to leave, the writing mode goes on `html` (where
/// Apple Books reads it, under its `-epub-` name as well), and a chapter
/// opens a page there as it does on paper.
pub(crate) fn stylesheet(style: &Style, lang: &str, book: bool) -> String {
    let mut out = String::new();
    let cjk = lang != "en";
    let fonts = font_list(style);
    if book {
        out.push_str(&format!("body {{ font-family: {fonts}; line-height: {LINE_HEIGHT}; }}\n"));
        if style.settings.vertical {
            out.push_str(
                "html { -epub-writing-mode: vertical-rl; -webkit-writing-mode: vertical-rl; writing-mode: vertical-rl; }\n\
                 body { -epub-text-orientation: mixed; text-orientation: mixed; line-break: strict; }\n",
            );
        }
        out.push_str("h1, h2 { break-before: page; }\nbody > :first-child { break-before: auto; }\n");
    } else {
        out.push_str(&format!(
            "body {{ font-family: {fonts}; line-height: {LINE_HEIGHT}; margin: 4rem; }}\n"
        ));
    }
    if style.settings.vertical && !book {
        // The one place outside a terminal where 縱書 costs a line of CSS.
        //
        // **No measure** (2026-11-08): a 縱 is as long as the window is tall,
        // as a line across is as long as the window is wide. A page is
        // publication, and the 縱 length the editing window wraps at is how
        // the writer sees the draft; the paper sets the printed one.
        //
        // Warning: **the writing mode goes on `html`** (2026-11-08). It was on
        // `body` alone, and the 縱 past the body's left edge overflowed where
        // a horizontal page cannot scroll to — a window narrower than the
        // text cut off its opening. On the root, the page opens at its right
        // edge and scrolls left.
        out.push_str("html { writing-mode: vertical-rl; }\n");
        out.push_str(
            // `mixed`, not `upright`: upright sets every Latin letter on its
            // own row, so a pinyin reading comes out one letter at a time and a
            // year comes out as four. `mixed` is what 縱書 means — 漢字 upright,
            // Latin turned — and it is the property's own default. How a run
            // of Latin is set is `vertical_text`'s; `line-break: strict` is
            // 禁則, as the terminal page keeps it.
            "body { writing-mode: vertical-rl; text-orientation: mixed; line-break: strict; }\n",
        );
    }
    if style.settings.hanging {
        // What `:view-hanging` means, said in the way a browser understands it.
        // `allow-end`, not `allow-end last`: `last` narrows it to the final
        // line of the block, which is not what 標點旁置 means — a stop hangs
        // wherever it lands at the end of a 縱.
        out.push_str("body { hanging-punctuation: allow-end; }\n");
    }
    // A Chinese paragraph opens with two 字 of indent and no gap; an English
    // one, on a web page, with a gap and no indent.
    match cjk {
        true => out.push_str("p { text-indent: 2em; margin: 0; }\n"),
        false => out.push_str("p { margin: 0 0 1em; }\n"),
    }
    out.push_str("rt { font-size: 0.5em; }\n");
    // Callouts, front matter, code in colour and task boxes, in GitHub's
    // light colours: the structure the preview's page has, in print.
    out.push_str(&crate::as_preview::structure_rules(&crate::as_preview::Colours::github_light()));
    out.push_str(
        "blockquote { margin: 1em 2em; }\n\
         table { border-collapse: collapse; margin: 1em 0; }\n\
         th, td { border: 1px solid #999; padding: 0.2em 0.6em; }\n\
         caption { padding: 0.3em; }\n\
         figure { margin: 1em 0; }\n\
         figure img { display: block; max-inline-size: 100%; }\n\
         img.inline { max-inline-size: 100%; }\n\
         figcaption { font-size: 0.9em; }\n\
         .footnotes { font-size: 0.9em; }\n\
         img.math { vertical-align: var(--depth); }\n\
         img.math.display { display: block; margin-block: 0.5em; margin-inline: auto; }\n\
         p.math { text-indent: 0; }\n",
    );
    if style.settings.vertical {
        // How a run of Latin is set ([`vertical_text`]): turned, across in
        // one square, or upright one to a square. `sideways` rather than
        // `mixed` for the turned ones, so a mark in the run turns with it.
        //
        // **A table is a vertical one** (2026-11-08), as a 年表 in 史記 is:
        // nothing overrides its writing mode, so each row of the source is a
        // column read from the right, the head row on the right, and its
        // caption runs down beside it. No nested writing mode, which DPFJ
        // advises against and Apple Books does not promise to show. A short
        // cell keeps its line (`.short`): Chrome sizes a vertical table's
        // cells as though anything could break, and wrapped `甲城` into two
        // lines with room to spare; a long cell may wrap, so the table is
        // never longer than the 縱 — a long word or an address in it too,
        // broken anywhere (2026-11-09, reported: a URL ran past the foot of
        // the page, as one word it could not break). Code is turned whole
        // ([`vertical_code_css`]).
        //
        // **A formula on a line of its own** is turned, as JLREQ §3.7.4 sets
        // one, and stands two squares in from the head of its line rather
        // than centred. A figure's picture stays upright; its caption runs
        // down beside it.
        out.push_str(LATIN_RULES);
        out.push_str(
            "table { margin: 0 1em; }\n\
             th, td { overflow-wrap: anywhere; }\n\
             .short { white-space: nowrap; }\n\
             th img.math, td img.math { max-inline-size: 100%; }\n\
             img.math { vertical-align: middle; }\n\
             img.math.display { margin-block: 0.5em; margin-inline: 2em 0; max-inline-size: calc(100% - 2em); }\n\
             figure { margin: 0 1em; }\n\
             sup.fn a { text-decoration: none; }\n",
        );
        out.push_str(&vertical_code_css());
    }
    out
}

/// **How a run of Latin stands down a vertical page** ([`vertical_text`]):
/// turned, across in one square, or upright one to a square. `sideways`
/// rather than `mixed` for the turned ones, so a mark in the run turns with
/// it.
pub(crate) const LATIN_RULES: &str = ".yoko { -webkit-text-orientation: sideways; text-orientation: sideways; }\n\
     .tate { -webkit-text-orientation: upright; text-orientation: upright; }\n\
     .tcy { -epub-text-combine: horizontal; -webkit-text-combine: horizontal; text-combine-upright: all; }\n";

/// The page's text: the blocks, the footnotes after them, and on a vertical
/// Chinese or Japanese page what [`vertical_text`] does to both. An English
/// page set down the page keeps its words whole and turned: 縱中橫 is for the
/// odd number in a CJK line, and every short word of an English one would be
/// squeezed into a square.
pub(crate) fn html_body(text: &str, style: &Style, lang: &str) -> String {
    let text = legal_text(text);
    let (blocks, notes) = blocks(&text, style.source, true);
    let mut body = String::new();
    html_blocks(&blocks, style, &mut body);
    // A note cited again is a second anchor: `fnref-1`, then `fnref-1-2`,
    // as an `id` is one place on the page, and epubcheck refuses a book
    // with two of them.
    let (mut body, order) = number_the_notes(&body, &notes, escape_html, |n, k, _| {
        let id = match k {
            1 => format!("fnref-{n}"),
            k => format!("fnref-{n}-{k}"),
        };
        format!("<sup class=\"fn\"><a href=\"#fn-{n}\" id=\"{id}\">{n}</a></sup>")
    });
    if !order.is_empty() {
        body.push_str("<section class=\"footnotes\">\n<hr>\n<ol>\n");
        for (n, lines) in order.iter().enumerate() {
            let n = n + 1;
            // A formula in a note is in the notes' ink, as the preview sets it.
            let inked = with_ink(style, |c| c.muted);
            body.push_str(&format!("<li id=\"fn-{n}\">{}</li>\n", html_lines(lines, &inked)));
        }
        body.push_str("</ol>\n</section>\n");
    }
    let body = heading_ids(&body);
    match style.settings.vertical && lang != "en" {
        true => vertical_text(&body, style.settings.latin, corners_quotes(lang)),
        false => body,
    }
}

/// **Each heading given an `id` from its words** (review, 2026-11-09:
/// `[第二章](#第二章)` led nowhere, and epubcheck refused the book, RSC-012),
/// as GitHub gives one, so a link written for GitHub or VS Code finds it:
/// the words in small letters, a space as `-`, letters (a Chinese character
/// is one), digits, `-` and `_` kept and every other mark left out; a second
/// heading of the same words takes `-1`, a third `-2`. A footnote's number
/// in a heading is not its words. The HTML page and the EPUB alike, and the
/// EPUB's table of contents points at these.
fn heading_ids(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 64);
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut next: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // The footnotes' own anchors are taken already.
    // A second citation's anchor is `fnref-1-2`.
    let reserved = |id: &str| {
        let number = |n: &str| n.parse::<usize>().is_ok();
        id.strip_prefix("fn-").is_some_and(number)
            || id.strip_prefix("fnref-").is_some_and(|n| match n.split_once('-') {
                Some((n, k)) => number(n) && number(k),
                None => number(n),
            })
    };
    for line in body.split_inclusive('\n') {
        let depth = line.strip_prefix("<h").and_then(|rest| rest.chars().next()).filter(|d| ('1'..='6').contains(d));
        let Some(depth) = depth.filter(|_| line.as_bytes().get(3) == Some(&b'>')) else {
            out.push_str(line);
            continue;
        };
        let close = format!("</h{depth}>");
        let inner = line[4..].split(close.as_str()).next().unwrap_or_default();
        let base = match slug(&plain_words(&without_note_marks(inner))) {
            slug if slug.is_empty() => "heading".to_string(),
            slug => slug,
        };
        // The search for a free suffix starts where the last heading of
        // these words left it, so a book of many headings of the same words
        // is numbered in one pass, not in a pass per heading.
        let mut n = next.get(&base).copied().unwrap_or(0);
        let mut id = match n {
            0 => base.clone(),
            n => format!("{base}-{n}"),
        };
        while taken.contains(&id) || reserved(&id) {
            n += 1;
            id = format!("{base}-{n}");
        }
        next.insert(base, n + 1);
        out.push_str(&format!("<h{depth} id=\"{id}\">"));
        out.push_str(&line[4..]);
        taken.insert(id);
    }
    out
}

/// A heading's markup without the footnote marks in it.
fn without_note_marks(markup: &str) -> String {
    const OPEN: &str = "<sup class=\"fn\">";
    let mut out = String::with_capacity(markup.len());
    let mut rest = markup;
    while let Some(at) = rest.find(OPEN) {
        out.push_str(&rest[..at]);
        rest = match rest[at..].find("</sup>") {
            Some(end) => &rest[at + end + "</sup>".len()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// **GitHub's anchor for a heading's words** (github-slugger): in small
/// letters, each space a `-`, and only letters, digits, the marks that go
/// with a letter, `-` and `_` kept.
pub(crate) fn slug(words: &str) -> String {
    let combining = |c: char| matches!(c as u32, 0x300..=0x36F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F);
    words
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() || combining(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// The words of a run of markup: its tags dropped, and in a `<ruby>` the
/// reading too, so a heading reads as its text in a table of contents or
/// an anchor. Entities are read back into characters.
pub(crate) fn plain_words(markup: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut reading = false;
    let mut tag = String::new();
    for c in markup.chars() {
        match (in_tag, c) {
            (false, '<') => {
                in_tag = true;
                tag.clear();
            }
            (true, '>') => {
                in_tag = false;
                let name = tag.trim_start_matches('/');
                if name.starts_with("rt") || name.starts_with("rp") {
                    reading = !tag.starts_with('/');
                }
            }
            (true, c) => tag.push(c),
            (false, c) if !reading => out.push(c),
            _ => {}
        }
    }
    // Back to characters, so an `&amp;` is not escaped twice.
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#160;", "\u{a0}").replace("&nbsp;", "\u{a0}").replace("&amp;", "&")
}

/// How long a table cell is, in squares: a half-width character is half one.
fn cell_length(text: &str) -> f64 {
    text.chars().map(|c| if half_width(c) || c == ' ' { 0.5 } else { 1.0 }).sum()
}

/// **Whether a cell of a vertical table keeps its line** (2026-11-08): up to
/// six squares, or up to half the table's longest cell, and never past
/// twelve. Chrome sizes a vertical table's cells as though anything could
/// break, so a cell left to wrap is cut short with room to spare — `甲城` in
/// two lines; a long cell must still wrap, or the table runs past the foot
/// of the page. The page has no length of its own to measure a cell against
/// (it is as tall as the window), so the cells are measured against each
/// other.
fn short_cell(length: f64, longest: f64) -> bool {
    length <= (longest / 2.0).clamp(6.0, 12.0)
}

/// **The hook in a wrapped code line's hang** (2026-11-08), ↳ as it is
/// across: down from the line before, then on along this one to an
/// arrowhead. Two strokes of three points each, in the hang's width and the
/// line's height from their top left. The preview draws the same.
pub const CODE_HOOK: [(f32, f32); 6] = [(0.25, 0.2), (0.25, 0.52), (0.72, 0.52), (0.6, 0.42), (0.72, 0.52), (0.6, 0.62)];

/// How far a wrapped code line goes on from the line's start, in cells: a
/// whole number, so its characters keep their columns.
pub const CODE_HANG: u8 = 2;

/// A code line's height, in ems of the code.
const CODE_LINE: f64 = 1.45;

/// **Code on a vertical page** (2026-11-08): turned whole, a block and a
/// span alike — `sideways`, not `mixed`, so 漢字 in it turn with the line:
/// code is verbatim, and its columns line up only if nothing in it stands
/// up. Each line of a block runs down the page, the next on its left.
///
/// A line longer than the 縱 wraps at its foot wherever that falls
/// (`line-break: anywhere`), as a terminal wraps, and goes on [`CODE_HANG`]
/// cells in, with [`CODE_HOOK`] in the hang. The page has no length of its
/// own — a 縱 is as long as the window is tall — so the wrap and the hook are
/// CSS alone: each source line is a block whose first line is pulled back
/// over its hang, and the hook is a mask repeated a line apart down the
/// rest of it. The physical sides are a vertical page's: the line's start is
/// its top, the next line is on its left.
pub(crate) fn vertical_code_css() -> String {
    // The hook's tile, turned as the line is: a line's pitch wide, the hang
    // tall, in hundredths of an em of a face whose cell is 0.6 em.
    let (wide, tall) = (CODE_LINE * 100.0, f64::from(CODE_HANG) * 60.0);
    let at = |(a, b): (f32, f32)| format!("{:.0} {:.0}", (1.0 - f64::from(b)) * wide, f64::from(a) * tall);
    let h = CODE_HOOK.map(at);
    let svg = format!(
        "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {wide:.0} {tall:.0}' preserveAspectRatio='none'%3E\
         %3Cpath d='M{}L{}L{}M{}L{}L{}' fill='none' stroke='%23000' stroke-width='1.2' stroke-linecap='round' stroke-linejoin='round' vector-effect='non-scaling-stroke'/%3E%3C/svg%3E\")",
        h[0], h[1], h[2], h[3], h[4], h[5]
    );
    let hang = CODE_HANG;
    let line = CODE_LINE;
    format!(
        "pre, code {{ -webkit-text-orientation: sideways; text-orientation: sideways; }}\n\
         pre {{ margin: 0 1em; line-height: {line}; white-space: pre-wrap; line-break: anywhere; word-break: break-all; overflow-wrap: anywhere; }}\n\
         pre .l {{ display: inline-block; vertical-align: top; box-sizing: border-box; inline-size: 100%; position: relative; padding-top: {hang}ch; text-indent: -{hang}ch; }}\n\
         pre .l::after {{ content: \"\"; position: absolute; top: 0; right: {line}em; left: 0; height: {hang}ch; background: currentColor; opacity: 0.45; \
         -webkit-mask-image: {svg}; mask-image: {svg}; -webkit-mask-size: {line}em {hang}ch; mask-size: {line}em {hang}ch; \
         -webkit-mask-repeat: repeat-x; mask-repeat: repeat-x; -webkit-mask-position: right top; mask-position: right top; }}\n"
    )
}

/// **A fenced code block as HTML**, its fence lines left out. On a vertical
/// page each line is a block of its own, so that a line wrapped at the foot
/// of the 縱 hangs and carries its hook ([`vertical_code_css`]); its newline
/// stays in it, so the text copied is the text written.
///
/// `painted` is its lines in colour ([`painted_code`]), when its language is
/// known.
fn code_block_html(lines: &[String], vertical: bool, painted: Option<Vec<String>>) -> String {
    let escaped = || code_inside(lines).iter().map(|l| escape_html(l)).collect::<Vec<_>>();
    let body = painted.unwrap_or_else(escaped).into_iter();
    match vertical {
        false => format!("<pre><code>{}</code></pre>\n", body.collect::<Vec<_>>().join("\n")),
        true => format!("<pre><code>{}</code></pre>\n", body.map(|l| format!("<span class=\"l\">{l}\n</span>")).collect::<String>()),
    }
}

/// **A code block's lines in colour** (2026-11-08), as the Markdown
/// preview draws them, when its fence names a language yumete colours:
/// each line escaped, its tokens in `<span>`s of their class
/// ([`crate::as_preview::painted_code`]).
fn painted_code(lines: &[String]) -> Option<Vec<String>> {
    let info = lines.first()?.trim_start().trim_start_matches(['`', '~']).trim();
    let language = crate::code::Language::from_info(info.split_whitespace().next()?)?;
    Some(crate::as_preview::painted_code(language, code_inside(lines)))
}

/// **A fenced block's code**: its lines but the fence that opens it and the
/// one that closes it, if one does — a block left open has no closing
/// fence, and its last line is code.
fn code_inside(lines: &[String]) -> &[String] {
    let Some((first, rest)) = lines.split_first() else { return lines };
    let closed = fence_of(first.trim_start()).is_some_and(|(c, n)| rest.last().is_some_and(|last| closes_fence(last, c, n)));
    &rest[..rest.len() - usize::from(closed)]
}

/// The blocks as HTML.
fn html_blocks(blocks: &[Block], style: &Style, out: &mut String) {
    let line = |text: &str| line_into(text, style.settings.dialects(), Dialect::Html, escape_html, style.source, Drawing::of(style));
    for block in blocks {
        match block {
            Block::Heading(depth, title) => {
                out.push_str(&format!("<h{depth}>{}</h{depth}>\n", line(title)));
            }
            // The fence lines are markup, the rest is verbatim: `<pre>` keeps
            // its own whitespace, and the only thing owed to it is HTML's own
            // escaping — no Markdown is read inside a code block (#386).
            // Code in its colours, as the preview draws it.
            Block::Code(lines) => out.push_str(&code_block_html(lines, style.settings.vertical, painted_code(lines))),
            Block::Paragraph(lines) => {
                out.push_str(&format!("<p>{}</p>\n", html_lines(lines, style)));
            }
            Block::Math(lines) => {
                let tex = lines.join("\n");
                let tex = tex.trim().trim_start_matches("$$").trim_end_matches("$$").trim();
                let tex_source = !matches!(style.source, crate::syntax::Syntax::Typst | crate::syntax::Syntax::Text);
                match style.painter.filter(|_| tex_source).map(|paint| paint(tex, true, style.settings.vertical, ink_of(style))) {
                    Some(Ok(drawn)) => {
                        out.push_str(&format!("<p class=\"math\">{}</p>\n", formula_img(&drawn, tex, true, style.settings.vertical)));
                    }
                    _ => out.push_str(&format!("<p>{}</p>\n", html_lines(lines, style))),
                }
            }
            Block::Rule => out.push_str("<hr>\n"),
            Block::Quote(inner) => {
                out.push_str("<blockquote>\n");
                // A formula in a quotation is in the quotation's ink.
                html_blocks(inner, &with_ink(style, |c| c.quote), out);
                out.push_str("</blockquote>\n");
            }
            Block::List { ordered, start, items } => {
                let open = match (ordered, start) {
                    (true, 1) => "<ol>".to_string(),
                    (true, n) => format!("<ol start=\"{n}\">"),
                    (false, _) => "<ul>".to_string(),
                };
                out.push_str(&open);
                out.push('\n');
                for (task, item) in items {
                    // A task's box, before its words: `- [ ]` and `- [x]`.
                    let (open, tick) = match task {
                        Some(done) => {
                            let checked = if *done { " checked=\"checked\"" } else { "" };
                            ("<li class=\"task\">", format!("<input type=\"checkbox\" disabled=\"disabled\"{checked}>"))
                        }
                        None => ("<li>", String::new()),
                    };
                    // A one-paragraph item is set tight, as a list is read.
                    match item.as_slice() {
                        [Block::Paragraph(lines)] => {
                            out.push_str(&format!("{open}{tick}{}</li>\n", html_lines(lines, style)));
                        }
                        _ => {
                            let mut held = String::new();
                            html_blocks(item, style, &mut held);
                            let held = match held.strip_prefix("<p>") {
                                Some(rest) => format!("<p>{tick}{rest}"),
                                None => format!("{tick}{held}"),
                            };
                            out.push_str(&format!("{open}\n{held}</li>\n"));
                        }
                    }
                }
                out.push_str(if *ordered { "</ol>\n" } else { "</ul>\n" });
            }
            Block::Table { head, align, rows, caption } => {
                // Down the page, a short cell keeps its line (see
                // `stylesheet` and `short_cell`).
                let longest = std::iter::once(head).chain(rows).flatten().map(|c| cell_length(c)).fold(0.0, f64::max);
                let cell = |tag: &str, n: usize, text: &str| {
                    let side = match align.get(n) {
                        Some(crate::mdtable::Align::Center) => " style=\"text-align: center\"",
                        Some(crate::mdtable::Align::Right) => " style=\"text-align: right\"",
                        _ => "",
                    };
                    let short = match style.settings.vertical && short_cell(cell_length(text), longest) {
                        true => " class=\"short\"",
                        false => "",
                    };
                    format!("<{tag}{short}{side}>{}</{tag}>", line(text))
                };
                out.push_str("<table>\n");
                if let Some(caption) = caption {
                    out.push_str(&format!("<caption>{}</caption>\n", line(caption)));
                }
                out.push_str("<thead>\n<tr>");
                for (n, text) in head.iter().enumerate() {
                    out.push_str(&cell("th", n, text));
                }
                out.push_str("</tr>\n</thead>\n<tbody>\n");
                for row in rows {
                    out.push_str("<tr>");
                    for (n, text) in row.iter().enumerate() {
                        out.push_str(&cell("td", n, text));
                    }
                    out.push_str("</tr>\n");
                }
                out.push_str("</tbody>\n</table>\n");
            }
            // The path as written: the page is written beside its source, so
            // it finds the picture where the source does. An EPUB takes the
            // picture in (`epub::pictures_in`).
            Block::Figure { caption, src } => {
                let alt = escape_html(caption).replace('"', "&quot;");
                let src = url_attribute(src);
                out.push_str(&format!("<figure>\n<img class=\"figure\" src=\"{src}\" alt=\"{alt}\">\n"));
                if !caption.trim().is_empty() {
                    out.push_str(&format!("<figcaption>{}</figcaption>\n", line(caption)));
                }
                out.push_str("</figure>\n");
            }
            Block::Callout { kind, name, title, fenced, body } => {
                let class = CALLOUTS[*kind].0;
                let fenced_class = if *fenced { " fenced" } else { "" };
                out.push_str(&format!("<div class=\"callout{fenced_class} {class}\">\n"));
                let title = match title {
                    Some(title) => line(title),
                    None => escape_html(&crate::as_preview::callout_title(name, *kind, *fenced)),
                };
                let open = if CALLOUTS[*kind].0 == "details" { "▾ " } else { "" };
                out.push_str(&format!("<p class=\"callout-title\">{open}{title}</p>\n"));
                html_blocks(body, style, out);
                out.push_str("</div>\n");
            }
            Block::FrontMatter(lines) => {
                let lines: Vec<String> = lines.iter().map(|l| escape_html(l)).collect();
                out.push_str(&format!("<pre class=\"front-matter\">{}</pre>\n", lines.join("\n")));
            }
        }
    }
}

/// A paragraph's lines as one run of HTML: a line break stays one between
/// lines of Chinese, where it is meant, and is a space between two Latin
/// lines, where it is a sentence going on.
fn html_lines(lines: &[String], style: &Style) -> String {
    let mut out = String::new();
    for (k, raw) in lines.iter().enumerate() {
        if k > 0 {
            let before = lines[k - 1].trim_end().chars().last();
            let after = raw.trim_start().chars().next();
            let latin = |c: Option<char>| c.is_some_and(|c| yumete_cjk::char_width(c) == 1);
            let hard = lines[k - 1].ends_with("  ");
            out.push_str(match latin(before) && latin(after) && !hard {
                true => " ",
                false => "<br>\n",
            });
        }
        out.push_str(&line_into(raw.trim(), style.settings.dialects(), Dialect::Html, escape_html, style.source, Drawing::of(style)));
    }
    out
}

/// The marks a footnote reference is written as until the whole text is out
/// and the notes can be numbered in the order they are met.
///
/// **Noncharacters, not private use** (2026-10-30): U+E000 and U+E001 were
/// here, and a 字根 font puts real glyphs in the private-use area, so a
/// writer's U+E000 vanished from the export. U+FDD0 and U+FDD1 are reserved by
/// Unicode for exactly this, a program's own internal marks.
const NOTE_OPEN: char = '\u{FDD0}';
const NOTE_CLOSE: char = '\u{FDD1}';

/// A footnote mark's name as the body carries it: the brackets, the caret
/// and every backslash gone.
fn note_name(text: &str) -> String {
    text.chars().filter(|c| !matches!(c, '\\' | '[' | ']' | '^' | ':')).collect()
}

/// Every footnote mark in `body` replaced by `mark(n, k, note)`, numbered in
/// the order the marks are met (`k` is which citation of note `n` this is,
/// from 1), and the notes in that order. A mark with no note
/// stays as it was written; a note with no mark is left out.
///
/// **The mark's name is escaped and the note's is not**: the body has been
/// through the format's `escape`, so `[^a&b]` reaches here as `a&amp;b` in
/// HTML while its note is `a&b`. Each note's name is put through the same
/// escape before it is compared, or the two would never meet.
fn number_the_notes(
    body: &str,
    notes: &[Note],
    escape: fn(&str) -> String,
    mark: impl Fn(usize, usize, &[String]) -> String,
) -> (String, Vec<Vec<String>>) {
    let mut out = String::with_capacity(body.len());
    let mut order: Vec<(String, Vec<String>)> = Vec::new();
    let mut cited: Vec<usize> = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find(NOTE_OPEN) {
        out.push_str(&rest[..at]);
        let after = &rest[at + NOTE_OPEN.len_utf8()..];
        let Some(close) = after.find(NOTE_CLOSE) else {
            rest = after;
            continue;
        };
        let name = &after[..close];
        rest = &after[close + NOTE_CLOSE.len_utf8()..];
        let n = match order.iter().position(|(seen, _)| seen == name) {
            Some(k) => k + 1,
            None => match notes.iter().find(|(had, _)| had == name || note_name(&escape(had)) == name) {
                Some((_, text)) => {
                    order.push((name.to_string(), text.clone()));
                    order.len()
                }
                None => {
                    out.push_str(&format!("[^{name}]"));
                    continue;
                }
            },
        };
        cited.resize(order.len(), 0);
        cited[n - 1] += 1;
        out.push_str(&mark(n, cited[n - 1], &order[n - 1].1));
    }
    out.push_str(rest);
    (out, order.into_iter().map(|(_, text)| text).collect())
}

/// The page's language, from what it is written in: kana makes it Japanese,
/// more 漢字 than Latin words Chinese, and otherwise English.
///
/// **Only the prose is counted** (2026-11-09): code, in a block or a span,
/// and addresses — a link's target, a bare URL — are left out. Counted, they
/// made a Chinese note with a page of code in it an English one.
pub(crate) fn language_of(text: &str) -> &'static str {
    let prose = prose_of(text);
    let (mut han, mut kana, mut words, mut in_word) = (0usize, 0usize, 0usize, false);
    for c in prose.chars() {
        if yumete_cjk::is_han(c) {
            han += 1;
            in_word = false;
        } else if matches!(c as u32, 0x3041..=0x30FF) {
            kana += 1;
            in_word = false;
        } else if c.is_alphabetic() {
            words += usize::from(!in_word);
            in_word = true;
        } else {
            in_word = false;
        }
    }
    match (kana, han, words) {
        (k, h, _) if k > 0 && k * 4 >= h => "ja",
        (_, h, w) if h >= w => "zh",
        _ => "en",
    }
}

/// **`text` without its code and its addresses**, for [`language_of`]: the
/// lines inside a fence, each code span, each link's target and each bare
/// URL taken out.
fn prose_of(text: &str) -> String {
    static ADDRESS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\]\([^)]*\)|<[A-Za-z][A-Za-z0-9+.-]*:[^>\s]*>|(?:https?://|www\.)\S+").expect("a pattern")
    });
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        match fence {
            Some((c, n)) => {
                if closes_fence(line, c, n) {
                    fence = None;
                }
                continue;
            }
            None => {
                if let Some(open) = fence_of(line.trim_start()) {
                    fence = Some(open);
                    continue;
                }
            }
        }
        // A code span: a run of backticks to the next run of as many.
        let mut kept = String::with_capacity(line.len());
        let mut rest = line;
        while let Some(at) = rest.find('`') {
            kept.push_str(&rest[..at]);
            let run = rest[at..].len() - rest[at..].trim_start_matches('`').len();
            let after = &rest[at + run..];
            let mut close = None;
            let mut k = 0;
            while let Some(p) = after[k..].find('`') {
                let n = after[k + p..].len() - after[k + p..].trim_start_matches('`').len();
                if n == run {
                    close = Some(k + p);
                    break;
                }
                k += p + n;
            }
            rest = match close {
                Some(k) => &after[k + run..],
                None => after,
            };
        }
        kept.push_str(rest);
        out.push_str(&ADDRESS.replace_all(&kept, " "));
        out.push('\n');
    }
    out
}

/// **`text` with nothing in it an XML page may not hold** (2026-11-09): the
/// control characters other than a tab and the line ends — a form feed or a
/// vertical tab as a space, the rest gone — and U+FFFE and U+FFFF. An EPUB's
/// page is XML, and one form feed made it a page a strict reader refuses.
pub(crate) fn legal_text(text: &str) -> std::borrow::Cow<'_, str> {
    let illegal = |c: char| (c < ' ' && !matches!(c, '\t' | '\n' | '\r')) || matches!(c, '\u{FFFE}' | '\u{FFFF}');
    if !text.chars().any(illegal) {
        return std::borrow::Cow::Borrowed(text);
    }
    let out = text
        .chars()
        .filter_map(|c| match c {
            '\u{0B}' | '\u{0C}' => Some(' '),
            c if illegal(c) => None,
            c => Some(c),
        })
        .collect();
    std::borrow::Cow::Owned(out)
}

/// The half of the stylesheet that only a printer sees.
///
/// A browser is the only vertical typesetter most writers have, and `@page` is
/// what makes it one. Without this the print is the screen page cut wherever
/// the paper happens to end: one endless 縱, sliced.
///
/// **The type is 五號, 10.5 point** (2026-11-08), and the paper says how
/// many characters a line or a 縱 then holds. It used to be worked out the
/// other way, from the 縱 length the editing window wraps at; a page is
/// publication, and nothing of the editing window's reaches it.
pub(crate) fn print_rules(style: &Style) -> String {
    let paper = style.settings.paper;
    let across = f64::from(paper.width) * (1.0 - 2.0 * MARGIN_SIDE);
    let down = f64::from(paper.height) * (1.0 - MARGIN_TOP - MARGIN_BOTTOM);
    let (along, athwart) = match style.settings.vertical {
        true => (down, across),
        false => (across, down),
    };
    let size = PRINT_SIZE_PT * 25.4 / 72.0;
    let measure = (along / size).floor().max(1.0);
    let rows = (athwart / (size * LINE_HEIGHT)).floor().max(1.0);
    let side = f64::from(paper.width) * MARGIN_SIDE;

    let mut out = String::new();
    out.push_str(&format!(
        "\n/* Printed: 版心 {measure:.0} 字 × {rows:.0} 行. No 頁碼 and no 書眉 —\n   \
         they would be written in the @page margin boxes, which no browser\n   \
         implements; the print dialog's own header and footer are where a\n   \
         page number comes from. */\n"
    ));
    out.push_str(&format!(
        "@page {{ size: {w}mm {h}mm; margin: {top:.1}mm {side:.1}mm {bottom:.1}mm {side:.1}mm; }}\n",
        w = paper.width,
        h = paper.height,
        top = f64::from(paper.height) * MARGIN_TOP,
        bottom = f64::from(paper.height) * MARGIN_BOTTOM,
    ));
    out.push_str(&format!(
        "@media print {{\n  \
         html, body {{ margin: 0; }}\n  \
         body {{ font-size: {PRINT_SIZE_PT}pt; }}\n  \
         h1, h2 {{ break-before: page; }}\n  \
         body > :first-child {{ break-before: auto; }}\n  \
         h1, h2, h3, h4, h5, h6 {{ break-after: avoid; }}\n  \
         p {{ orphans: 2; widows: 2; }}\n  \
         table, pre, figure, img.math {{ break-inside: avoid; }}\n\
         }}\n"
    ));
    out
}

/// A Typst source file. Horizontal: Typst cannot set CJK vertically yet, and a
/// file that claimed otherwise would simply come out wrong.
fn typst(text: &str, style: &Style) -> String {
    let mut out = String::new();
    out.push_str("// Written out by yumete.\n");
    if style.settings.vertical {
        out.push_str(
            "// Set horizontally: Typst has no vertical writing mode yet, so a
// 縱書 draft is exported as ordinary lines rather than as something
// that would only look like 縱書.\n",
        );
    }
    let text = crate::markdown::strip_comments(text);
    let lang = language_of(&text);
    match lang {
        "en" => {
            out.push_str("#set text(lang: \"en\")\n");
            out.push_str("#set par(justify: true)\n");
        }
        lang => {
            out.push_str(&format!("#set text(font: \"Source Han Serif\", lang: \"{lang}\")\n"));
            out.push_str("#set par(first-line-indent: 2em, justify: true)\n");
        }
    }
    out.push_str(
        "\n// yumete writes readings as `#ruby(base, reading)`; define it to
// taste, or replace it with a package.\n\
         #let ruby(base, reading) = box(baseline: 0pt)[\n  \
         #place(top + center, dy: -0.9em, text(size: 0.5em, reading))\n  #base\n]\n\n",
    );

    let (blocks, notes) = blocks(&text, style.source, false);
    let mut body = String::new();
    typst_blocks(&blocks, style, &mut body);
    // Typst sets a footnote where it is called, so the note's text goes in
    // at the mark.
    let (body, _) = number_the_notes(&body, &notes, escape_typst, |_, _, text| {
        let lines: Vec<String> = text
            .iter()
            .map(|l| line_into(l, style.settings.dialects(), Dialect::Typst, escape_typst, style.source, None))
            .collect();
        format!("#footnote[{}]", lines.join(" "))
    });
    out.push_str(&body);
    out
}

/// The blocks as Typst markup.
fn typst_blocks(blocks: &[Block], style: &Style, out: &mut String) {
    let line = |text: &str| line_into(text, style.settings.dialects(), Dialect::Typst, escape_typst, style.source, None);
    for block in blocks {
        match block {
            Block::Heading(depth, title) => {
                out.push_str(&format!("{} {}\n\n", "=".repeat(*depth), line(title)));
            }
            // **Straight through.** Typst spells a raw block with the same
            // three backticks Markdown does, so the writer's fence is already
            // Typst — reading anything inside it is the whole of #386.
            //
            // Typst has backticks only: a `~~~` fence, or a short one round
            // a longer run of backticks, goes over as backticks enough to
            // hold what is inside.
            Block::Code(lines) => {
                let opening = lines[0].trim_start();
                let (mark, run) = fence_of(opening).unwrap_or(('`', 3));
                let inner = code_inside(lines);
                let longest = inner.iter().flat_map(|l| l.split(|c| c != '`')).map(str::len).max().unwrap_or(0);
                // **A block left open is closed** (review, 2026-11-09): Markdown
                // ends it with the document or the quotation it stands in, and
                // Typst's raw text would run on over the rest — the `]` of a
                // `#quote` too — so its fence is written.
                let closed = inner.len() + 2 == lines.len();
                if mark == '`' && run > longest && closed {
                    for text in lines {
                        out.push_str(text);
                        out.push('\n');
                    }
                } else {
                    let fence = "`".repeat(longest.max(2) + 1);
                    let info = opening.trim_start_matches(mark).trim();
                    out.push_str(&format!("{fence}{info}\n"));
                    for text in inner {
                        out.push_str(text);
                        out.push('\n');
                    }
                    out.push_str(&fence);
                    out.push('\n');
                }
                out.push('\n');
            }
            // A display formula is TeX, which Typst cannot read: its lines go
            // over as text, as they did when they were read as a paragraph.
            Block::Paragraph(lines) | Block::Math(lines) => {
                for text in lines {
                    out.push_str(&line(text.trim()));
                    out.push('\n');
                }
                out.push('\n');
            }
            Block::Rule => out.push_str("#line(length: 100%)\n\n"),
            Block::Quote(inner) => {
                let mut held = String::new();
                typst_blocks(inner, style, &mut held);
                out.push_str(&format!("#quote(block: true)[\n{}\n]\n\n", held.trim_end()));
            }
            // Typst's own list markup: `-` an item, `3.` a numbered one, and
            // what belongs to an item indented under it.
            Block::List { ordered, start, items } => {
                for (n, (task, item)) in items.iter().enumerate() {
                    let mut held = String::new();
                    typst_blocks(item, style, &mut held);
                    // A task's box goes with its marker.
                    let tick = match task {
                        Some(true) => "☑ ",
                        Some(false) => "☐ ",
                        None => "",
                    };
                    let marker = match ordered {
                        true => format!("{}. {tick}", start + n),
                        false => format!("- {tick}"),
                    };
                    // An empty item still takes its marker, or the numbers
                    // after it shift (2026-10-30: `1.` alone vanished).
                    if held.trim().is_empty() {
                        out.push_str(marker.trim_end());
                        out.push('\n');
                        continue;
                    }
                    for (k, text) in held.trim_end().lines().enumerate() {
                        match (k, text.is_empty()) {
                            (0, _) => out.push_str(&format!("{marker}{text}")),
                            (_, true) => {}
                            _ => out.push_str(&format!("  {text}")),
                        }
                        out.push('\n');
                    }
                }
                out.push('\n');
            }
            // A picture is the caption's words, as it was before figures were
            // read: the PDF is compiled where a missing or remote picture
            // would stop it.
            Block::Figure { caption, .. } => {
                out.push_str(&line(caption));
                out.push_str("\n\n");
            }
            // Read only for `:export-as-preview`, which writes no Typst.
            Block::Callout { body, .. } => typst_blocks(body, style, out),
            Block::FrontMatter(_) => {}
            Block::Table { head, align, rows, caption, .. } => {
                let side = |a: &crate::mdtable::Align| match a {
                    crate::mdtable::Align::Center => "center",
                    crate::mdtable::Align::Right => "right",
                    _ => "left",
                };
                let cells = |row: &[String]| -> String {
                    row.iter().map(|c| format!("[{}]", line(c))).collect::<Vec<_>>().join(", ")
                };
                let (open, close) = match caption {
                    Some(caption) => ("#figure(table(".to_string(), format!("), caption: [{}])\n\n", line(caption))),
                    None => ("#table(".to_string(), ")\n\n".to_string()),
                };
                out.push_str(&format!(
                    "{open}\n  columns: {},\n  align: ({},),\n  table.header({}),\n",
                    head.len(),
                    align.iter().map(side).collect::<Vec<_>>().join(", "),
                    cells(head),
                ));
                for row in rows {
                    out.push_str(&format!("  {},\n", cells(row)));
                }
                out.push_str(&close);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> Style {
        Style {
            settings: Settings { vertical: true, hanging: true, ruby_html: true, ruby_typst: false, ..Settings::default() },
            title: "第一章".to_string(),
            source: crate::syntax::Syntax::Markdown,
            base: None,
            painter: None,
            look: None,
        }
    }

    /// **`Format` 說寫成什麼，`Style::source` 說讀的是什麼**（2026-09-28）。
    ///
    /// Warning: 從前只有前一半，於是導出**永遠按 Markdown 解析源檔**：一份 Typst 稿子裏的
    /// `*粗*` 是四個字符、`$a+b$` 被當成兩個字面美元號轉義掉。
    #[test]
    fn export_reads_the_source_syntax_not_the_output_format() {
        // Markdown 源：一個價錢裏的 `$` 就是一個 `$`，寫進 Typst 要轉義（Typst 那一端
        // 它開公式）。A Markdown formula is LaTeX, which Typst cannot read, so
        // it goes over as written, escaped.
        let mut md = style();
        md.settings.vertical = false;
        let out = export("$3 and $5\n", Format::Typst, &md);
        assert!(out.contains(r"\$3 and \$5"), "Markdown 源裏它是字面：{out}");
        let out = export("$a_b$\n", Format::Typst, &md);
        assert!(out.contains(r"\$a\_b\$"), "LaTeX, escaped: {out}");

        // Typst 源：那是一條公式，原樣過去。
        let mut ts = style();
        ts.settings.vertical = false;
        ts.source = crate::syntax::Syntax::Typst;
        let out = export("$a + b$\n", Format::Typst, &ts);
        assert!(out.contains("$a + b$"), "公式原樣：{out}");

        // Warning: Typst 源裏 `*粗*` 是強調，導成 HTML 要出 `<strong>`。
        let out = export("*粗*\n", Format::Html, &ts);
        assert!(out.contains("<strong>粗</strong>"), "{out}");

        // Warning: 標題在 Typst 裏是 `=`，在 Markdown 裏不是。
        let out = export("= 第一章\n", Format::Html, &ts);
        assert!(out.contains("<h1"), "= 是標題：{out}");
        let out = export("= 第一章\n", Format::Html, &md);
        assert!(!out.contains("<h1"), "Markdown 源裏它是一行字：{out}");
    }

    /// **Print is 五號 on the paper asked for** (2026-11-08): 10.5 point,
    /// whatever the editing window wraps at, and the paper says how many
    /// characters a 縱 or a line then holds.
    #[test]
    fn print_is_ten_and_a_half_point_on_the_paper() {
        // A5 is 210 mm down, of which the 版心 is 79% — 165.9 mm — or 44
        // characters of 3.70 mm; across, 118.4 mm at 1.8 line heights is 17 行.
        let out = export("句", Format::Html, &style());
        assert!(out.contains("@page { size: 148mm 210mm;"), "{out}");
        assert!(out.contains("body { font-size: 10.5pt; }"), "{out}");
        assert!(out.contains("版心 44 字 × 17 行"), "{out}");
        let mut a4 = style();
        a4.settings.paper = Paper { width: 210, height: 297 };
        let out = export("句", Format::Html, &a4);
        assert!(out.contains("@page { size: 210mm 297mm;") && out.contains("版心 63 字 × 25 行"), "{out}");
        // Across the page it is the width that holds the line.
        let mut flat = style();
        flat.settings.vertical = false;
        let out = export("句", Format::Html, &flat);
        assert!(out.contains("body { font-size: 10.5pt; }") && out.contains("版心 31 字 × 24 行"), "{out}");
    }

    #[test]
    fn a_chapter_starts_on_a_new_page_and_the_first_one_does_not() {
        let out = export("# 第一章\n\n句\n", Format::Html, &style());
        assert!(out.contains("h1, h2 { break-before: page; }"), "{out}");
        assert!(
            out.contains("body > :first-child { break-before: auto; }"),
            "{out}"
        );
        // A heading stranded at the foot of a page is the one thing every
        // typesetter fixes by hand.
        assert!(
            out.contains("h1, h2, h3, h4, h5, h6 { break-after: avoid; }"),
            "{out}"
        );
    }

    /// **The screen page has no measure** (2026-11-08): a line fills the
    /// window across, a 縱 fills it down, and the vertical page's writing
    /// mode is the root's, so it opens at its right edge.
    #[test]
    fn the_screen_page_has_no_measure() {
        let out = export("句", Format::Html, &style());
        assert!(out.contains("html { writing-mode: vertical-rl; }"), "{out}");
        assert!(out.contains("body { writing-mode: vertical-rl; text-orientation: mixed; line-break: strict; }"), "{out}");
        let mut flat = style();
        flat.settings.vertical = false;
        for out in [out, export("句", Format::Html, &flat)] {
            assert!(!out.contains("max-width") && !out.contains("block-size: 32em") && !out.contains("32em"), "{out}");
            assert!(!out.contains("body { max-inline-size"), "{out}");
        }
    }

    #[test]
    fn what_is_on_the_screen_does_not_change_with_the_paper() {
        // The print rules are an addition. A writer who never prints should
        // not be able to tell that any of this is in the file.
        let a5 = export("# 第一章\n\n句\n", Format::Html, &style());
        let mut big = style();
        big.settings.paper = Paper {
            width: 210,
            height: 297,
        };
        let a4 = export("# 第一章\n\n句\n", Format::Html, &big);
        let cut = |s: &str| s.split("\n/* Printed:").next().unwrap().to_string();
        assert_eq!(cut(&a5), cut(&a4));
    }

    #[test]
    fn markup_is_rendered_and_a_note_to_yourself_is_not_exported() {
        // An export that prints the markup literally is not an export, it is a
        // copy — and 批注 is the writer talking to themselves, which the manual
        // says is not part of the book. It was going into the file handed to a
        // publisher.
        let style = Style {
            settings: Settings { vertical: false, hanging: false, ruby_html: false, ruby_typst: false, ..Settings::default() },
            title: "t".to_string(),
            source: crate::syntax::Syntax::Markdown,
            base: None,
            painter: None,
            look: None,
        };
        let out = export("他**很好**，%%這裏要改%%不過還行。\n", Format::Html, &style);
        assert!(out.contains("<strong>很好</strong>"), "{out}");
        assert!(!out.contains("**"), "the asterisks are gone: {out}");
        assert!(!out.contains("這裏要改"), "the 批注 is not in the book: {out}");
        assert!(!out.contains("%%"), "{out}");
        assert!(out.contains("不過還行"), "and the writing is: {out}");

        // Typst writes the same thing its own way.
        let out = export("他**很好**，%%私話%%好。\n", Format::Typst, &style);
        assert!(out.contains("*很好*"), "{out}");
        assert!(!out.contains("私話"), "{out}");
    }

    /// **一層套一層的標記，一層一層排出來**（2026-10-01）。
    ///
    /// Warning: 從前那一趟是平的：取頭一個蓋住光標的 span，而 `spans()` 交出來的
    /// 是**外層在前**的嵌套表，於是永遠取到最外那一層，整段原文連同裏層的分隔
    /// 符一起包進去——`**粗`碼`**` 出成 `<strong>粗`碼`</strong>`，反引號印進了
    /// 書裏；Typst 那一端還被 `escape_typst` 加上反斜線，成了 `` \` ``。
    #[test]
    fn markup_inside_markup_is_exported_all_the_way_down() {
        let style = |source| Style {
            settings: Settings { vertical: false, hanging: false, ruby_html: false, ruby_typst: false, ..Settings::default() },
            title: "t".to_string(),
            source,
            base: None,
            painter: None,
            look: None,
        };
        let md = style(crate::syntax::Syntax::Markdown);
        // 只看正文那一段——整份輸出裏還有 CSS，裏頭本來就有星號。
        let body = |text: &str, format| -> String {
            let out = export(text, format, &md);
            let wanted = |l: &&str| match format {
                Format::Html => l.starts_with("<p>"),
                _ => l.contains('甲') || l.contains('說'),
            };
            out.lines().find(wanted).unwrap_or_else(|| panic!("{out}")).to_string()
        };

        // 粗體裏的代碼。
        let one = body("他**說`let x`罷**了。\n", Format::Html);
        assert_eq!(one, "<p>他<strong>說<code>let x</code>罷</strong>了。</p>");
        let one = body("他**說`let x`罷**了。\n", Format::Typst);
        assert_eq!(one, "他*說`let x`罷*了。", "Typst 那一端不許帶反斜線");

        // 粗體裏的斜體。Warning: 反過來寫（`*甲**乙**丙*`）在**解析那一層**就不是
        // 嵌套——`markdown::spans` 把它讀成三段並排的 `Emphasis`。那是解析器的
        // 事，不是這一趟的事。
        assert_eq!(body("**甲*乙*丙**\n", Format::Html), "<p><strong>甲<em>乙</em>丙</strong></p>");

        // 高亮裏的粗體。
        assert_eq!(body("==甲**乙**丙==\n", Format::Html), "<p><mark>甲<strong>乙</strong>丙</mark></p>");

        // 代碼裏寫着星號，那是代碼，不是標記。
        assert_eq!(body("甲`a**b`\n", Format::Html), "<p>甲<code>a**b</code></p>");

        // 稿子裏自己打的尖括號照舊轉義——遞歸之後也只轉一遍。
        // (`<b>` would be an HTML tag, which goes through as HTML.)
        assert_eq!(body("甲**a<1>c**\n", Format::Html), "<p>甲<strong>a&lt;1&gt;c</strong></p>");

        // Warning: **一條腳註正好填滿一層標記，兩條 span 一樣大**（2026-10-01 審出
        // 來的回歸）。腳註那一支不帶前導 `Marker`，所以 `Strong` 與 `Footnote`
        // 的範圍完全相同——只認「不是我自己那一條」的話，兩條互相當對方的裏
        // 層，**棧溢出**。這幾個輸入從前（平的那一趟）都是好的。
        for one in ["甲**[^1]**", "甲*[^1]*", "甲==[^1]==", "甲~~[^1]~~"] {
            let text = format!("{one}\n");
            let out = export(&text, Format::Html, &md);
            assert!(out.contains("甲"), "{out}");
            let out = export(&text, Format::Typst, &md);
            assert!(out.contains("甲"), "{out}");
        }
    }

    #[test]
    fn a_vertical_page_sets_latin_the_way_縱書_does() {
        let style = Style {
            settings: Settings { vertical: true, hanging: true, ruby_html: false, ruby_typst: false, ..Settings::default() },
            title: "t".to_string(),
            source: crate::syntax::Syntax::Markdown,
            base: None,
            painter: None,
            look: None,
        };
        let out = export("一二三\n", Format::Html, &style);
        // `upright` sets every Latin letter on its own row — a pinyin reading
        // one letter at a time, a year as four.
        assert!(out.contains("body { writing-mode: vertical-rl; text-orientation: mixed;"), "{out}");
        assert!(!out.contains("body { writing-mode: vertical-rl; text-orientation: upright"), "{out}");
        // `last` narrows the hang to the block's final line, which is not what
        // 標點旁置 means.
        assert!(out.contains("hanging-punctuation: allow-end;"), "{out}");
    }

    #[test]
    fn html_carries_the_direction_the_page_was_set_in() {
        let out = export(
            "# 第一章\n\n<ruby>永<rt>ㄩㄥˇ</rt></ruby>和九年。\n",
            Format::Html,
            &style(),
        );
        assert!(out.contains("writing-mode: vertical-rl"), "{out}");
        assert!(out.contains("hanging-punctuation"), "{out}");
        assert!(out.contains("<h1 id=\"第一章\">第一章</h1>"), "{out}");
        assert!(
            out.contains("<ruby>永<rt>ㄩㄥˇ</rt></ruby>和九年。"),
            "{out}"
        );
    }

    /// **A code block comes out as a code block** (#386).
    ///
    /// `blocks` knew only headings and paragraphs, so the opening ```` ```rust ````
    /// went down the paragraph path and met the *inline* scanner, which read
    /// two of its three backticks as an empty code span and ate them; the third
    /// was escaped. What reached the `.typ` file was `\`rust` and a body — not
    /// a code block, and not what was written either.
    #[test]
    fn a_fenced_block_survives_the_export_fence_and_all() {
        let text = "# 標題\n\n```rust\nfn main() { let x_y = 1; }\n```\n\n收尾。\n";

        // Typst spells a raw block with the same three backticks, so the
        // writer's fence is already Typst: it goes straight through.
        let out = export(text, Format::Typst, &style());
        assert!(out.contains("```rust\n"), "the fence, opened: {out}");
        assert!(
            out.contains("fn main() { let x_y = 1; }"),
            "and the code untouched — no `\\_` in it: {out}"
        );
        assert!(!out.contains("\\`"), "no escaped backtick anywhere: {out}");
        assert!(!out.contains("x\\_y"), "nothing inside it is markup: {out}");

        // HTML wants its own wrapper, and its own escaping inside it; the
        // code is in its colours (2026-11-08), its words as written.
        let mut across = style();
        across.settings.vertical = false;
        let out = export(text, Format::Html, &across);
        assert!(out.contains("<pre><code>"), "{out}");
        assert!(out.contains("<span class=\"t-keyword\">fn</span>") && out.contains("x_y"), "{out}");
    }

    /// **Task lists** (2026-11-08), GFM's `- [ ]` and `- [x]`: a box before
    /// the item's words, ticked or not, in HTML and in Typst; `[ ]` that is
    /// not followed by a space or the line's end is the item's words.
    #[test]
    fn a_task_list_has_its_boxes() {
        let text = "- [ ] 買菜\n- [x] 洗衣\n- [ ]\n- [x]真的字\n";
        let mut across = style();
        across.settings.vertical = false;
        let out = export(text, Format::Html, &across);
        assert!(out.contains("<li class=\"task\"><input type=\"checkbox\" disabled=\"disabled\">買菜</li>"), "{out}");
        assert!(out.contains("<li class=\"task\"><input type=\"checkbox\" disabled=\"disabled\" checked=\"checked\">洗衣</li>"), "{out}");
        assert!(out.contains("<li>[x]真的字</li>"), "{out}");
        let out = export(text, Format::Typst, &across);
        assert!(out.contains("- ☐ 買菜\n- ☑ 洗衣\n"), "{out}");
    }

    /// **Code down the page is turned whole** (2026-11-08), a block and a
    /// span alike, 漢字 in it too, under `turn` and `jlreq`: it is verbatim,
    /// never upright, never 縱中橫. A block's lines are boxes of their own,
    /// each keeping its newline, so a line too long for the 縱 wraps, hangs
    /// and carries the hook, and what is copied is what was written.
    #[test]
    fn code_down_the_page_is_turned_whole() {
        let text = "行内 `取值(x, 12)` 收尾。\n\n```\nfn 主() {}\n\n```\n";
        for s in [style(), jlreq()] {
            let out = export(text, Format::Html, &s);
            assert!(out.contains("pre, code { -webkit-text-orientation: sideways; text-orientation: sideways; }"), "{out}");
            assert!(out.contains("<code>取值(x, 12)</code>"), "nothing in a span is wrapped: {out}");
            assert!(out.contains("<pre><code><span class=\"l\">fn 主() {}\n</span><span class=\"l\">\n</span></code></pre>"), "{out}");
            assert!(out.contains("white-space: pre-wrap; line-break: anywhere;"), "{out}");
            assert!(out.contains("pre .l::after") && out.contains("mask-image: url(\"data:image/svg+xml,"), "{out}");
            assert!(!out.contains("horizontal-tb"), "{out}");
        }
        // Across the page a block is as it was.
        let mut flat = style();
        flat.settings.vertical = false;
        let out = export(text, Format::Html, &flat);
        assert!(out.contains("<pre><code>fn 主() {}\n</code></pre>"), "{out}");
        assert!(!out.contains("class=\"l\"") && !out.contains("sideways"), "{out}");
    }

    /// **Typst's backticks hold raw text, so nothing inside them is escaped.**
    ///
    /// The escaping used to happen before `marked` was called, so every run got
    /// the same treatment and `` `code_here` `` came out `` `code\_here` `` —
    /// a backslash that Typst prints. HTML is the opposite: `&lt;` is required
    /// inside `<code>`, so this is a per-dialect answer (#386).
    #[test]
    fn an_inline_code_span_is_raw_in_typst_and_escaped_in_html() {
        let out = export("行內 `code_here` 收尾。\n", Format::Typst, &style());
        assert!(out.contains("`code_here`"), "{out}");
        assert!(!out.contains("code\\_here"), "{out}");

        let out = export("行內 `a<b>c` 收尾。\n", Format::Html, &style());
        assert!(out.contains("<code>a&lt;b&gt;c</code>"), "{out}");
    }

    #[test]
    fn typst_says_why_it_is_horizontal_rather_than_pretending() {
        let out = export("永和九年。\n", Format::Typst, &style());
        assert!(out.contains("no vertical writing mode"), "{out}");
        assert!(!out.contains("vertical-rl"));
    }

    #[test]
    fn a_reading_is_rewritten_into_the_target_dialect() {
        let mut s = style();
        s.settings.vertical = false;
        let out = export(
            "<ruby>永和<rt>えいわ</rt></ruby>九年。\n",
            Format::Typst,
            &s,
        );
        assert!(out.contains(r#"#ruby("永和", "えいわ")"#), "{out}");
    }

    /// #332: the base and the reading land inside `#ruby("…", "…")`, which
    /// takes **string literals** — where Typst's markup escaping is not just
    /// unnecessary but an error.
    #[test]
    fn a_reading_exported_to_typst_is_escaped_as_a_string_not_as_markup() {
        let mut s = style();
        s.settings.vertical = false;
        // A star, a hash and an underscore are ordinary characters in a
        // string. Escaped as markup they were `\*`, which the compiler
        // refuses as an unknown escape sequence.
        let out = export(
            "<ruby>a*b#c_d<rt>え_い</rt></ruby>九年。\n",
            Format::Typst,
            &s,
        );
        assert!(out.contains(r#"#ruby("a*b#c_d", "え_い")"#), "{out}");
        assert!(!out.contains(r"\*"), "{out}");

        // What a string *does* need escaping for is the quote and the
        // backslash, and that is written by the dialect itself.
        let out = export(
            "<ruby>桜<rt>say \"hi\"</rt></ruby>\n",
            Format::Typst,
            &s,
        );
        assert!(out.contains(r#"#ruby("桜", "say \"hi\"")"#), "{out}");

        // Coming the other way, the escaping comes back off: a Typst source
        // exported as HTML shows the quote, not the backslash.
        let mut s = style();
        s.settings.vertical = false;
        (s.settings.ruby_html, s.settings.ruby_typst) = (false, true);
        let out = export("#ruby(\"桜\", \"say \\\"hi\\\"\")\n", Format::Html, &s);
        assert!(out.contains("<rt>say \"hi\"</rt>"), "{out}");
    }

    #[test]
    fn the_writers_own_angle_brackets_survive() {
        let mut s = style();
        (s.settings.ruby_html, s.settings.ruby_typst) = (false, false);
        let out = export("他說「a < b」。\n", Format::Html, &s);
        assert!(out.contains(" &lt; "), "{out}");
    }

    #[test]
    fn blank_lines_separate_paragraphs_and_line_breaks_are_kept() {
        let mut s = style();
        (s.settings.ruby_html, s.settings.ruby_typst) = (false, false);
        let out = export("一\n二\n\n三\n", Format::Html, &s);
        assert!(out.contains("<p>一<br>\n二</p>"), "{out}");
        assert!(out.contains("<p>三</p>"), "{out}");
    }

    /// **Lists, quotes, footnotes, tables and rules come out as what they
    /// are** (2026-10-29); before, each came out as its source.
    #[test]
    fn markdown_structure_reaches_both_formats() {
        let mut flat = style();
        flat.settings.vertical = false;
        let text = "# A Title\n\nWrapped at\nthe source.[^a]\n\n- one\n- two\n  1. inner\n\n> quoted\n> on\n\n---\n\n| N | C |\n| :- | -: |\n| x | 3 |\n\n[^a]: The note.\n";
        let html = export(text, Format::Html, &flat);
        assert!(html.contains("<html lang=\"en\">"), "{html}");
        assert!(html.contains("<p>Wrapped at the source.<sup class=\"fn\"><a href=\"#fn-1\" id=\"fnref-1\">1</a></sup></p>"), "{html}");
        assert!(html.contains("<ul>\n<li>one</li>\n<li>\n<p>two</p>\n<ol>\n<li>inner</li>\n</ol>\n</li>\n</ul>"), "{html}");
        assert!(html.contains("<blockquote>\n<p>quoted on</p>\n</blockquote>"), "{html}");
        assert!(html.contains("<hr>\n<table>"), "{html}");
        assert!(html.contains("<td style=\"text-align: right\">3</td>"), "{html}");
        assert!(html.contains("<li id=\"fn-1\">The note.</li>"), "{html}");
        assert!(!html.contains("[^a]"), "{html}");
        assert!(html.contains("p { margin: 0 0 1em; }") && !html.contains("text-indent: 2em"), "{html}");

        let typ = export(text, Format::Typst, &flat);
        assert!(typ.contains("#set text(lang: \"en\")"), "{typ}");
        assert!(typ.contains("the source.#footnote[The note.]"), "{typ}");
        assert!(typ.contains("- one\n- two\n"), "{typ}");
        assert!(typ.contains("  1. inner"), "{typ}");
        assert!(typ.contains("#quote(block: true)[\nquoted\non\n]"), "{typ}");
        assert!(typ.contains("#line(length: 100%)"), "{typ}");
        assert!(typ.contains("align: (left, right,),\n  table.header([N], [C]),\n  [x], [3],"), "{typ}");
    }

    /// **Footnotes whose name is markup, and a note cited twice**: `[^a&b]`
    /// finds its note although the body has escaped its name, and each
    /// citation of one note has an anchor of its own.
    #[test]
    fn a_note_named_with_markup_is_found_and_each_citation_has_its_own_anchor() {
        let mut flat = style();
        flat.settings.vertical = false;
        let text = "甲[^a&b]乙[^<x>]丙[^a&b]。\n\n[^a&b]: 註一。\n[^<x>]: 註二。\n";
        let html = export(text, Format::Html, &flat);
        assert!(html.contains("<li id=\"fn-1\">註一。</li>\n<li id=\"fn-2\">註二。</li>"), "{html}");
        assert!(html.contains("甲<sup class=\"fn\"><a href=\"#fn-1\" id=\"fnref-1\">1</a></sup>"), "{html}");
        assert!(html.contains("乙<sup class=\"fn\"><a href=\"#fn-2\" id=\"fnref-2\">2</a></sup>"), "{html}");
        assert!(html.contains("丙<sup class=\"fn\"><a href=\"#fn-1\" id=\"fnref-1-2\">1</a></sup>"), "{html}");
        assert!(!html.contains("[^"), "{html}");
        let typ = export(text, Format::Typst, &flat);
        assert!(typ.contains("甲#footnote[註一。]乙#footnote[註二。]丙#footnote[註一。]"), "{typ}");
        // A heading of the words a second citation's anchor has does not take it.
        let html = export("甲[^n]乙[^n]。\n\n# fnref-1-2\n\n[^n]: 註。\n", Format::Html, &flat);
        assert!(html.contains("<h1 id=\"fnref-1-2-1\">"), "{html}");
    }

    /// **A table is as wide as its header**, as GFM has it: a row's extra
    /// cells are dropped, a short row is filled out, and a bare `|` over a
    /// rule is no table (Typst refuses `columns: 0`).
    #[test]
    fn a_table_is_as_wide_as_its_header() {
        let mut flat = style();
        flat.settings.vertical = false;
        let text = "| 名 | 數 |\n| - | - |\n| 甲 | 1 | 多 | 餘 |\n| 乙 |\n";
        let html = export(text, Format::Html, &flat);
        assert!(html.contains("<tr><td>甲</td><td>1</td></tr>"), "{html}");
        assert!(html.contains("<tr><td>乙</td><td></td></tr>"), "{html}");
        assert!(!html.contains("多") && !html.contains("餘"), "{html}");
        let typ = export(text, Format::Typst, &flat);
        assert!(typ.contains("columns: 2,") && typ.contains("  [甲], [1],\n  [乙], [],\n"), "{typ}");
        let typ = export("|\n| - |\n| 甲 |\n", Format::Typst, &flat);
        assert!(!typ.contains("columns: 0"), "{typ}");
    }

    /// **Headings of the same words are numbered in one pass**: twenty
    /// thousand of them take a moment, not a pass each.
    #[test]
    fn many_headings_of_the_same_words_are_numbered_at_once() {
        let body = "<h2>同</h2>\n".repeat(20_000);
        let started = std::time::Instant::now();
        let out = heading_ids(&body);
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
        assert!(out.starts_with("<h2 id=\"同\">同</h2>\n<h2 id=\"同-1\">"), "{}", &out[..80]);
        assert!(out.ends_with("<h2 id=\"同-19999\">同</h2>\n"));
        // The same answers as GitHub's where a heading's words are a suffix.
        let out = heading_ids("<h1>a</h1>\n<h1>a-1</h1>\n<h1>a</h1>\n<h1>a</h1>\n");
        assert_eq!(out, "<h1 id=\"a\">a</h1>\n<h1 id=\"a-1\">a-1</h1>\n<h1 id=\"a-2\">a</h1>\n<h1 id=\"a-3\">a</h1>\n");
    }

    /// **A line break stays one in Chinese and is a space between Latin
    /// lines**, and the page's language is the text's.
    #[test]
    fn line_breaks_and_the_language_follow_the_text() {
        let mut flat = style();
        flat.settings.vertical = false;
        let html = export("一\n二\n", Format::Html, &flat);
        assert!(html.contains("<p>一<br>\n二</p>") && html.contains("lang=\"zh\""), "{html}");
        assert!(html.contains("text-indent: 2em"), "{html}");
        let html = export("one\ntwo\n", Format::Html, &flat);
        assert!(html.contains("<p>one two</p>") && html.contains("lang=\"en\""), "{html}");
        let html = export("一个 word\nand 二\n", Format::Html, &flat);
        assert!(html.contains("<p>一个 word and 二</p>"), "Latin meets Latin at the break: {html}");
        let html = export("かな漢字\n", Format::Html, &flat);
        assert!(html.contains("lang=\"ja\""), "{html}");
    }

    /// A mark with no note stays as it was written; a note nobody marks is
    /// left out; one note marked twice keeps its one number.
    #[test]
    fn footnotes_are_numbered_by_where_they_are_met() {
        let mut flat = style();
        flat.settings.vertical = false;
        let html = export("a[^x] b[^y] c[^x]\n\n[^y]: Why.\n[^x]: Ex.\n[^z]: Zed.\n", Format::Html, &flat);
        assert!(html.contains("a<sup class=\"fn\"><a href=\"#fn-1\""), "{html}");
        assert!(html.contains("b<sup class=\"fn\"><a href=\"#fn-2\""), "{html}");
        assert!(html.contains("c<sup class=\"fn\"><a href=\"#fn-1\""), "{html}");
        assert!(html.contains("<li id=\"fn-1\">Ex.</li>\n<li id=\"fn-2\">Why.</li>"), "{html}");
        assert!(!html.contains("Zed"), "{html}");
        let html = export("a[^none]\n", Format::Html, &flat);
        assert!(html.contains("a[^none]"), "{html}");
    }

    /// **Found in review, 2026-10-30**: a U+3000-indented line after a list
    /// item panicked; a hard break was lost; a writer's U+E000 vanished; an
    /// empty first item lost its number in Typst.
    #[test]
    fn what_the_night_review_found_in_export() {
        let mut flat = style();
        flat.settings.vertical = false;
        let html = export("- 第一點\n\u{3000}\u{3000}續寫的一行\n", Format::Html, &flat);
        assert!(html.contains("第一點"), "{html}");
        let html = export("Roses are red,  \nviolets are blue.\n", Format::Html, &flat);
        assert!(html.contains("red,<br>"), "{html}");
        let html = export("私用\u{E000}字\n", Format::Html, &flat);
        assert!(html.contains("私用\u{E000}字"), "{html}");
        let typst = export("1.\n2. 二\n", Format::Typst, &flat);
        assert!(typst.contains("1.\n2. 二"), "{typst}");
    }

    /// Inline HTML is HTML (2026-11-05): through as it is to an HTML page,
    /// and gone from a Typst one, which has nothing to make of a tag.
    #[test]
    fn inline_html_goes_through_to_html_and_not_to_typst() {
        let mut md = style();
        md.settings.vertical = false;
        let out = export("按 <kbd>Esc</kbd> 退出\n", Format::Html, &md);
        assert!(out.contains("按 <kbd>Esc</kbd> 退出"), "{out}");
        let out = export("按 <kbd>Esc</kbd> 退出\n", Format::Typst, &md);
        assert!(out.contains("按 Esc 退出"), "{out}");
    }

    /// Markdown maths is LaTeX: Typst gets it as escaped text, so the PDF
    /// builds, and HTML as written, `$$` and all.
    #[test]
    fn markdown_maths_goes_over_as_written() {
        let mut md = style();
        md.settings.vertical = false;
        let out = export("Area $\\frac{a}{b}$ and $$E=mc^2$$\n", Format::Typst, &md);
        assert!(out.contains(r"Area \$\\frac{a}{b}\$ and \$\$E=mc^2\$\$"), "{out}");
        let out = export("Area $\\frac{a}{b}$ and $$E=mc^2$$\n", Format::Html, &md);
        assert!(out.contains(r"Area $\frac{a}{b}$ and $$E=mc^2$$"), "{out}");
        let out = export("**$x_1$** 粗\n", Format::Typst, &md);
        assert!(out.contains(r"*\$x\_1\$* 粗"), "{out}");
    }

    /// Only a real tag is HTML: `Vec<String>` and `a<b and c>d` are prose,
    /// on either page.
    #[test]
    fn angle_brackets_that_are_not_a_tag_stay_prose() {
        let mut md = style();
        md.settings.vertical = false;
        let out = export("Vec<String> and a<b and c>d\n", Format::Typst, &md);
        assert!(out.contains(r"Vec\<String\> and a\<b and c\>d"), "{out}");
        let out = export("Vec<String> and a<b and c>d\n", Format::Html, &md);
        assert!(out.contains("Vec&lt;String&gt; and a&lt;b and c&gt;d"), "{out}");
    }

    /// A formula that always draws: three bytes, a size anyone can check.
    fn drawn(_: &str, _: bool, _: bool, _: [u8; 3]) -> Result<Formula, String> {
        Ok(Formula { png: vec![1, 2, 3], width: 3.0, height: 1.5, depth: 0.25 })
    }

    fn refused(_: &str, _: bool, _: bool, _: [u8; 3]) -> Result<Formula, String> {
        Err("no".to_string())
    }

    /// The same, saying in its size whether it was asked to turn: a turned
    /// picture is as tall as the upright one is wide.
    fn drawn_turning(_: &str, _: bool, turned: bool, _: [u8; 3]) -> Result<Formula, String> {
        let (width, height) = if turned { (1.5, 3.0) } else { (3.0, 1.5) };
        Ok(Formula { png: vec![1, 2, 3], width, height, depth: 0.25 })
    }

    fn jlreq() -> Style {
        let mut s = style();
        s.settings.latin = Latin::Jlreq;
        s
    }

    /// **`turn`, the default** (2026-11-08): every run of half-width letters
    /// and digits turned, the marks joined to it turned with it, words with
    /// one space between turned as one; nothing set across.
    #[test]
    fn turn_turns_every_run_of_latin() {
        let out = export("1997年3月12日，A股跌了5%。他说 don’t worry，书店叫 Old Books。\n", Format::Html, &style());
        assert!(
            out.contains("<span class=\"yoko\">1997</span>年<span class=\"yoko\">3</span>月<span class=\"yoko\">12</span>日，<span class=\"yoko\">A</span>股跌了<span class=\"yoko\">5%</span>。"),
            "{out}"
        );
        assert!(out.contains("他说 <span class=\"yoko\">don’t worry</span>，"), "{out}");
        assert!(out.contains("书店叫 <span class=\"yoko\">Old Books</span>。"), "{out}");
        assert!(!out.contains("class=\"tcy\"") && !out.contains("class=\"tate\""), "{out}");
        assert!(out.contains(".yoko { -webkit-text-orientation: sideways; text-orientation: sideways; }"), "{out}");
        // A mark on its own is left to the font.
        let out = export("甲·乙 / 丙\n", Format::Html, &style());
        assert!(out.contains("甲·乙 / 丙"), "{out}");
        // Across the page nothing is wrapped.
        let mut flat = style();
        flat.settings.vertical = false;
        let out = export("1997年12月\n", Format::Html, &flat);
        assert!(!out.contains("class=\"yoko\"") && !out.contains("text-orientation: sideways"), "{out}");
    }

    /// **`jlreq`** (2026-11-08): JLREQ §2.3.2 and clreq. A word turned whole,
    /// capitals and single letters upright, one digit upright, two across,
    /// more turned, and a number with its unit one way as a whole.
    #[test]
    fn jlreq_sets_latin_as_the_requirements_do() {
        let set = |text: &str| vertical_text(text, Latin::Jlreq, true);
        assert_eq!(set("3月12日"), "<span class=\"tate\">3</span>月<span class=\"tcy\">12</span>日");
        assert_eq!(set("1997年"), "<span class=\"yoko\">1997</span>年");
        assert_eq!(set("A股跌了5%"), "<span class=\"tate\">A</span>股跌了<span class=\"tcy\">5%</span>");
        assert_eq!(set("涨了50%"), "涨了<span class=\"yoko\">50%</span>");
        assert_eq!(set("叫 NHK 的"), "叫 <span class=\"tate\">NHK</span> 的");
        assert_eq!(set("A4纸与MP3"), "<span class=\"tcy\">A4</span>纸与<span class=\"tate\">MP3</span>");
        assert_eq!(set("用 Python 写"), "用 <span class=\"yoko\">Python</span> 写");
        assert_eq!(set("说 don’t worry 了"), "说 <span class=\"yoko\">don’t worry</span> 了");
        assert_eq!(set("即 U.S. 的"), "即 <span class=\"yoko\">U.S.</span> 的");
        assert_eq!(set("和 Win10"), "和 <span class=\"yoko\">Win10</span>");
    }

    /// Only the text is touched: not a tag, not an entity's letters, and not
    /// code or a reading. An entity is the one character it stands for.
    #[test]
    fn what_is_not_running_text_is_left_alone() {
        let text = "那年冬天镇上下了一场很长的雨。甲 `x12` 乙 <ruby>漢<rt>han4</rt></ruby> R&D <span title=\"12\">丙</span>\n";
        let out = export(text, Format::Html, &style());
        assert!(out.contains("<code>x12</code>"), "{out}");
        assert!(out.contains("<rt>han4</rt>"), "{out}");
        assert!(out.contains("<span class=\"yoko\">R&amp;D</span>"), "{out}");
        assert!(out.contains("<span title=\"12\">丙</span>"), "{out}");
    }

    /// **Curly quotes become corner brackets down a Chinese 縱** (2026-11-08):
    /// what `:render full` draws. An apostrophe stays one, and a page across
    /// or in English keeps the writer's quotes.
    #[test]
    fn curly_quotes_turn_into_corner_brackets_down_the_page() {
        let out = export("他说：“走吧，‘快’。”\n", Format::Html, &style());
        assert!(out.contains("他说：『走吧，「快」。』"), "{out}");
        let out = export("他说 don’t 了。\n", Format::Html, &style());
        assert!(out.contains("<span class=\"yoko\">don’t</span>") && !out.contains("」"), "{out}");
        let mut flat = style();
        flat.settings.vertical = false;
        let out = export("他说：“走吧。”\n", Format::Html, &flat);
        assert!(out.contains("“走吧。”"), "{out}");
        let out = export("He said “go” and ‘stay’, the end of it.\n", Format::Html, &style());
        // Nor are its words wrapped.
        assert!(out.contains("He said “go” and ‘stay’"), "{out}");
    }

    /// **A table down the page is a vertical one** (2026-11-08): no writing
    /// mode of its own, its cells set by the same rules as the text, a short
    /// cell kept on one line, its caption with it; a printer does not cut it
    /// in two.
    #[test]
    fn a_table_down_the_page_is_a_vertical_table() {
        let text = "| 城 | 年 | 备注 |\n| --- | --- | --- |\n| 甲 | 1997 | 这一格写得很长很长，比它分到的那一段长 |\n\nTable: 表一　雨量\n";
        let out = export(text, Format::Html, &style());
        assert!(!out.contains("horizontal-tb; }\ntable") && !out.contains("table, pre { writing-mode"), "{out}");
        assert!(out.contains("<table>\n<caption>表一　雨量</caption>\n<thead>"), "{out}");
        assert!(out.contains("<td class=\"short\"><span class=\"yoko\">1997</span></td>"), "{out}");
        assert!(out.contains("<td>这一格写得很长很长"), "{out}");
        assert!(out.contains(".short { white-space: nowrap; }"), "{out}");
        assert!(out.contains("table, pre, figure, img.math { break-inside: avoid; }"), "{out}");
        assert!(!out.contains("<p>Table:"), "{out}");
        // Above the table is the same, and Typst writes it as a figure.
        let above = "Table: 表一　雨量\n\n| 城 |\n| --- |\n| 甲 |\n";
        assert!(export(above, Format::Html, &style()).contains("<caption>表一　雨量</caption>"));
        let typ = export(above, Format::Typst, &style());
        assert!(typ.contains("#figure(table(\n") && typ.contains("), caption: [表一　雨量])"), "{typ}");
        let mut flat = style();
        flat.settings.vertical = false;
        let out = export(text, Format::Html, &flat);
        assert!(!out.contains("class=\"short\""), "{out}");
    }

    /// **A picture alone in its paragraph is a figure** (2026-11-08): the
    /// picture where the source finds it, its text the caption.
    #[test]
    fn a_picture_alone_is_a_figure() {
        let out = export("看图。\n\n![一棵树，1997年](pics/tree.png)\n\n完。\n", Format::Html, &style());
        assert!(
            out.contains("<figure>\n<img class=\"figure\" src=\"pics/tree.png\" alt=\"一棵树，1997年\">\n<figcaption>一棵树，<span class=\"yoko\">1997</span>年</figcaption>\n</figure>"),
            "{out}"
        );
        assert!(out.contains("figure img { display: block; max-inline-size: 100%; }"), "{out}");
        let typ = export("![一棵树](tree.png)\n", Format::Typst, &style());
        assert!(typ.contains("一棵树") && !typ.contains("tree.png"), "{typ}");
    }

    /// **The fonts are the writer's** (2026-11-08): `[export] font_family`
    /// for every language, VS Code's default list when it is not set, and
    /// nothing in it can end the rule.
    #[test]
    fn the_fonts_are_the_ones_the_setting_names() {
        for text in ["中文。\n", "かなの文。\n", "Plain English words only here.\n"] {
            let out = export(text, Format::Html, &style());
            assert!(out.contains(&format!("body {{ font-family: {DEFAULT_FONTS}; ")), "{out}");
        }
        let mut s = style();
        s.settings.fonts = "\"LXGW WenKai\", serif; } body { color: red".to_string();
        let out = export("中文。\n", Format::Html, &s);
        assert!(out.contains("font-family: \"LXGW WenKai\", serif  body  color: red;"), "{out}");
        assert!(!out.contains("Source Han") && !out.contains("Georgia"), "{out}");
    }

    /// **A TeX formula is drawn** when the front end hands over a painter
    /// (2026-11-08): a PNG inline, the TeX as its `alt`, and its size in `em`.
    #[test]
    fn a_formula_is_drawn_into_the_page() {
        let mut s = style();
        s.settings.vertical = false;
        s.painter = Some(drawn);
        let out = export("从 $x^2 + y^2 = 1$ 起。\n", Format::Html, &s);
        assert!(
            out.contains("从 <img class=\"math\" alt=\"x^2 + y^2 = 1\" style=\"height: 1.500em; --depth: -0.250em\" src=\"data:image/png;base64,AQID\"> 起。"),
            "{out}"
        );
        let out = export("写下：\n\n$$\n\\frac{a}{b}\n$$\n\n完。\n", Format::Html, &s);
        assert!(out.contains("<p class=\"math\"><img class=\"math display\" alt=\"\\frac{a}{b}\""), "{out}");
        let out = export("写下 $$E=mc^2$$ 完。\n", Format::Html, &s);
        assert!(out.contains("<img class=\"math display\" alt=\"E=mc^2\""), "{out}");
    }

    /// **Down the page a formula is turned** (2026-11-08), in the bitmap by
    /// the painter: every one under `turn`; under `jlreq` all but one letter
    /// or digit and a numeric fraction; one on a line of its own always, set
    /// two squares in from the head of its line.
    #[test]
    fn a_formula_down_the_page_is_turned() {
        let mut s = style();
        s.painter = Some(drawn_turning);
        let out = export("从 $x$ 到 $\\frac12$ 再到 $x^2+1$。\n", Format::Html, &s);
        assert_eq!(out.matches("<img class=\"math turned\"").count(), 3, "{out}");
        assert!(out.contains("style=\"height: 3.000em;"), "{out}");
        // Not a run of Latin inside the picture's attributes either.
        assert!(!out.contains("alt=\"<span") && !out.contains("x^<span"), "{out}");
        let out = export("写下：\n\n$$\nE=mc^2\n$$\n", Format::Html, &s);
        assert!(out.contains("<p class=\"math\"><img class=\"math display turned\""), "{out}");
        assert!(out.contains("img.math.display { margin-block: 0.5em; margin-inline: 2em 0;"), "{out}");
        let mut j = jlreq();
        j.painter = Some(drawn_turning);
        let out = export("从 $x$ 到 $\\frac12$、$\\tfrac{3}{4}$、$½$ 再到 $x^2+1$ 和 $\\frac{a}{2}$。\n", Format::Html, &j);
        assert_eq!(out.matches("<img class=\"math\" ").count(), 4, "{out}");
        assert_eq!(out.matches("<img class=\"math turned\"").count(), 2, "{out}");
        let out = export("写下：\n\n$$\nx\n$$\n", Format::Html, &j);
        assert!(out.contains("<img class=\"math display turned\""), "{out}");
    }

    /// **The brace form goes out as ruby** (2026-11-08): `<ruby>` in HTML, a
    /// reading a character where the bars give one, `#ruby` in Typst; left
    /// as written with `no-ruby-markdown`, and never read in a Typst source.
    #[test]
    fn the_brace_form_goes_out_as_ruby() {
        let mut s = style();
        s.settings.vertical = false;
        let out = export("山顶的{气象站|qìxiàngzhàn}，{雨量|yǔ|liàng}。\n", Format::Html, &s);
        assert!(out.contains("山顶的<ruby>气象站<rt>qìxiàngzhàn</rt></ruby>，<ruby>雨<rt>yǔ</rt></ruby><ruby>量<rt>liàng</rt></ruby>。"), "{out}");
        let typ = export("{雨量|yǔ|liàng}\n", Format::Typst, &s);
        assert!(typ.contains("#ruby(\"雨\", \"yǔ\")#ruby(\"量\", \"liàng\")"), "{typ}");
        s.settings.ruby_markdown = false;
        assert!(export("{雨|yǔ}\n", Format::Html, &s).contains("{雨|yǔ}"));
        s.settings.ruby_markdown = true;
        s.source = crate::syntax::Syntax::Typst;
        assert!(!export("{雨|yǔ}\n", Format::Html, &s).contains("<ruby>"));
        // In a table cell the reading's bar is the cell's.
        s.source = crate::syntax::Syntax::Markdown;
        let out = export("| 月份 | 雨量 |\n| --- | --- |\n| 七月 | {雨量|yǔ|liàng}多 |\n", Format::Html, &s);
        assert!(out.contains("<td><ruby>雨<rt>yǔ</rt></ruby><ruby>量<rt>liàng</rt></ruby>多</td>"), "{out}");
        let mut words = Settings::default();
        assert_eq!(words.read_line("no-ruby-markdown").unwrap(), None);
        assert!(!words.ruby_markdown && !words.dialects().contains(Dialect::Markdown));
    }

    /// **The words of an `:export` line** (2026-11-08): any order, the path
    /// last, a font list held whole by its quotes and commas, and a word
    /// misspelt said rather than taken for a file.
    #[test]
    fn an_export_line_reads_its_words_and_its_path() {
        let mut s = Settings::default();
        let path = s.read_line("vertical jlreq page=32開 hanging no-ruby-typst out/book.epub").unwrap();
        assert_eq!(path.as_deref(), Some("out/book.epub"));
        assert!(s.vertical && s.hanging && s.ruby_html && !s.ruby_typst && s.latin == Latin::Jlreq);
        assert_eq!(s.paper, Paper { width: 130, height: 184 });
        let mut s = Settings::default();
        assert_eq!(s.read_line(r#"font='"PingFang SC", "Noto Sans CJK SC", sans-serif' turn"#).unwrap(), None);
        assert_eq!(s.fonts, r#""PingFang SC", "Noto Sans CJK SC", sans-serif"#);
        assert_eq!(s.read_line(r#"font="Songti SC, serif""#).unwrap(), None);
        assert_eq!(s.fonts, "Songti SC, serif", "a list in double quotes is a list");
        assert_eq!(s.read_line("font='Songti SC, serif' turn").unwrap(), None);
        assert_eq!(s.fonts, "Songti SC, serif", "as the messages show it");
        assert_eq!(s.read_line(r#"font="Songti SC""#).unwrap(), None);
        assert_eq!(s.fonts, r#""Songti SC""#, "one name keeps its quotes");
        assert_eq!(s.read_line("font=Songti,serif").unwrap(), None);
        assert_eq!(s.fonts, "Songti,serif", "an unquoted list runs to the space");
        assert_eq!(s.read_line(r#"font='"Songti SC", serif' "my book.html""#).unwrap().as_deref(), Some("my book.html"));
        assert_eq!(s.fonts, r#""Songti SC", serif"#);
        let mut s = Settings::default();
        assert_eq!(s.read_line("").unwrap(), None);
        assert_eq!(s.read_line("chapter.html vertical").unwrap_err(), say!("export.words-after-path", "chapter.html"));
        assert_eq!(s.read_line("verticle").unwrap_err(), say!("export.no-such-word", "verticle"));
        assert_eq!(s.read_line("./vertical").unwrap().as_deref(), Some("./vertical"));
        assert_eq!(s.read_line("page=quarto").unwrap_err(), say!("export.no-such-page", "quarto"));
        assert!(s.read_line("font=").is_err() && s.read_line(r#"font="Songti"#).is_err());
        assert_eq!(s, Settings::default(), "nothing is taken from a line that is refused");
        // A comma does not carry a word on past a space (2026-11-09): the
        // file named after it was once read into the font list.
        assert_eq!(s.read_line("font=Songti, b.html").unwrap_err(), say!("export.trailing-comma", "font=Songti,"));
        assert_eq!(s.read_line(r#"font="PingFang SC", serif"#).unwrap_err(), say!("export.trailing-comma", r#"font="PingFang SC","#));
        assert_eq!(s.read_line("vertical, c.html").unwrap_err(), say!("export.trailing-comma", "vertical,"));
        assert_eq!(s.read_line("c.html,").unwrap_err(), say!("export.trailing-comma", "c.html,"));
        assert_eq!(s.read_line("'odd,'").unwrap().as_deref(), Some("odd,"), "a quoted path may end in a comma");
        // The menu's short spellings are written out by the parser, and a
        // quoted path is left alone.
        let parsed = crate::command::parse(r#":export html v jl "my v.html""#).unwrap();
        assert!(matches!(parsed, crate::command::Command::Export { ref words, .. } if words == r#"vertical jlreq "my v.html""#), "{parsed:?}");
    }

    /// What does not draw stays as written, and nothing is drawn where the
    /// formula is not TeX, or the page cannot show a picture.
    #[test]
    fn a_formula_that_cannot_be_drawn_stays_as_written() {
        let mut s = style();
        s.settings.vertical = false;
        s.painter = Some(refused);
        let out = export("Area $\\frac{a}{b}$ and $$E=mc^2$$\n", Format::Html, &s);
        assert!(out.contains(r"Area $\frac{a}{b}$ and $$E=mc^2$$"), "{out}");
        let out = export("写下：\n\n$$\nx < y\n$$\n", Format::Html, &s);
        assert!(out.contains("<p>$$ x &lt; y $$</p>"), "{out}");
        s.painter = Some(drawn);
        s.source = crate::syntax::Syntax::Typst;
        let out = export("面積 $a + b$。\n", Format::Html, &s);
        assert!(!out.contains("<img"), "{out}");
        s.source = crate::syntax::Syntax::Markdown;
        let out = export("写下：\n\n$$\nx_1\n$$\n", Format::Typst, &s);
        assert!(out.contains("\\$\\$\nx\\_1\n\\$\\$"), "{out}");
    }

    /// **A fence is read as the preview reads it** (2026-11-09): tildes as
    /// well as backticks, and closed only by a line of the same character, at
    /// least as long.
    #[test]
    fn a_fence_of_tildes_or_of_four_is_read_as_the_preview_reads_it() {
        let mut s = style();
        s.settings.vertical = false;
        let out = export("前文。\n\n~~~python\n# 注释 *不是标题*\nx = 1\n~~~\n\n后文。\n", Format::Html, &s);
        assert!(!out.contains("<h1") && out.contains("# 注释 *不是标题*") && !out.contains("<em>"), "{out}");
        assert!(out.contains("<p>后文。</p>"), "{out}");
        let out = export("````\n```\ninner\n```\n````\n\n后文。\n", Format::Html, &s);
        assert!(out.contains("<pre><code>```\ninner\n```</code></pre>"), "{out}");
        assert!(out.contains("<p>后文。</p>") && !out.contains("````"), "{out}");
        // A shorter fence does not close a longer one, nor `~~~` a backtick one.
        let out = export("````\n```\n~~~~\n````\n", Format::Html, &s);
        assert!(out.contains("<pre><code>```\n~~~~</code></pre>"), "{out}");
        // Left open, the last line is still code.
        let out = export("```\nlast line\n", Format::Html, &s);
        assert!(out.contains("<pre><code>last line</code></pre>"), "{out}");
        // Typst has backticks only: a `~~~` block goes over in them, enough
        // of them to hold what is inside.
        let out = export("~~~rust\nlet a = 1;\n~~~\n", Format::Typst, &s);
        assert!(out.contains("```rust\nlet a = 1;\n```\n"), "{out}");
        let out = export("````md\n```\ninner\n```\n````\n", Format::Typst, &s);
        assert!(out.contains("````md\n```\ninner\n```\n````\n"), "{out}");
        let out = export("~~~\n```\n~~~\n", Format::Typst, &s);
        assert!(out.contains("````\n```\n````\n"), "{out}");
    }

    /// **A picture in a line is a picture** (2026-11-09), as the preview
    /// draws it, and two on one line are two such pictures, not a figure; a
    /// target in angle brackets may hold a space.
    #[test]
    fn a_picture_in_a_line_is_a_picture_and_two_are_not_a_figure() {
        let mut s = style();
        s.settings.vertical = false;
        let out = export("文中有图 ![小图](a.png) 在此。\n", Format::Html, &s);
        assert!(out.contains("文中有图 <img class=\"inline\" src=\"a.png\" alt=\"小图\"> 在此。"), "{out}");
        assert!(!out.contains("<a href=\"a.png\""), "{out}");
        assert!(out.contains("img.inline { max-inline-size: 100%; }"), "{out}");
        let out = export("![两张](a.png) 和 ![图](b.png)\n", Format::Html, &s);
        assert!(!out.contains("<figure>"), "{out}");
        assert!(out.contains("<img class=\"inline\" src=\"a.png\" alt=\"两张\"> 和 <img class=\"inline\" src=\"b.png\" alt=\"图\">"), "{out}");
        // Angle brackets: the space is the target's, written as a URL writes it.
        let out = export("[文字](<a b.html>) 与 [普通](x.html \"标题\")\n", Format::Html, &s);
        assert!(out.contains("<a href=\"a%20b.html\">文字</a> 与 <a href=\"x.html\">普通</a>"), "{out}");
        let out = export("![尖括号](<my pic.png>)\n", Format::Html, &s);
        assert!(out.contains("<img class=\"figure\" src=\"my%20pic.png\" alt=\"尖括号\">"), "{out}");
        let out = export("看 ![x](<my pic.png> 'title') 了\n", Format::Html, &s);
        assert!(out.contains("<img class=\"inline\" src=\"my%20pic.png\" alt=\"x\">"), "{out}");
        assert_eq!(figure_of("![图](<my pic.png> \"t\")"), Some(("图", "my pic.png")));
        assert_eq!(figure_of("![图](a(1).png)"), Some(("图", "a(1).png")));
        assert_eq!(figure_of("![两张](a.png) 和 ![图](b.png)"), None);
        // In Typst a picture is its words, as before.
        let out = export("文中有图 ![小图](a.png) 在此。\n", Format::Typst, &s);
        assert!(out.contains("文中有图 小图 在此。"), "{out}");
    }

    /// **The page's language is its prose's** (2026-11-09): code and
    /// addresses are not counted, so a Chinese note full of code is Chinese.
    #[test]
    fn the_language_is_read_from_the_prose_alone() {
        let code = "这是一段说明。\n\n```rust\nfn main() { let value = compute_the_answer(); println!(\"{value}\"); }\n```\n\n用 `let mut counter = make_a_counter()` 计数，见 https://example.com/some/long/path/with/words 与 [文档](https://example.com/docs/and/more/words)。\n";
        assert_eq!(language_of(code), "zh");
        assert_eq!(language_of("~~~\nmany english words in here and here\n~~~\n短文。\n"), "zh");
        assert_eq!(language_of("An English essay about things, set down the page.\n"), "en");
        assert_eq!(language_of("ひらがなとカタカナ。漢字。\n"), "ja");
    }

    /// **Nothing XML refuses reaches a page** (2026-11-09): a control
    /// character other than a tab or a line end goes, a form feed as a space.
    #[test]
    fn control_characters_do_not_reach_the_page() {
        let mut s = style();
        s.settings.vertical = false;
        let out = export("甲\u{0C}乙\u{01}丙\u{FFFF}丁\t戊\n", Format::Html, &s);
        assert!(out.contains("<p>甲 乙丙丁\t戊</p>"), "{out}");
        assert_eq!(legal_text("tab\tand\nline"), "tab\tand\nline");
        assert!(matches!(legal_text("plain"), std::borrow::Cow::Borrowed(_)));
    }

    /// **Under jlreq a single letter stands upright** (2026-11-09), a small
    /// one as a capital does and as `$x$` does beside it; a word of small
    /// letters is still turned.
    #[test]
    fn under_jlreq_a_single_letter_stands_upright() {
        let set = |text: &str| vertical_text(text, Latin::Jlreq, true);
        assert_eq!(set("变量 x 和 A"), "变量 <span class=\"tate\">x</span> 和 <span class=\"tate\">A</span>");
        assert_eq!(set("用 xy 写"), "用 <span class=\"yoko\">xy</span> 写");
        assert_eq!(jlreq_orientation(&['x']), Orientation::Upright);
        assert!(stands_upright("x"), "and the formula $x$ stands as the letter does");
    }

    /// **Corner brackets only round Chinese** (2026-11-09): quotes round
    /// Western words keep their marks, as clreq has them, on a Chinese page
    /// too; a pair holding Chinese turns, its words in other tags or not.
    #[test]
    fn quotes_round_western_words_stay_curly() {
        let out = export("他说“go”，又说“走”，还有‘stay’与‘留’。\n", Format::Html, &style());
        assert!(out.contains("他说“<span class=\"yoko\">go</span>”，又说『走』，还有‘<span class=\"yoko\">stay</span>’与「留」。"), "{out}");
        let out = export("他说：“**走吧**，‘快’。”\n", Format::Html, &style());
        assert!(out.contains("他说：『<strong>走吧</strong>，「快」。』"), "{out}");
        // A quotation running on over paragraphs opens each, and closes once.
        let out = export("“第一段。\n\n“第二段。”\n", Format::Html, &style());
        assert!(out.contains("<p>『第一段。</p>") && out.contains("<p>『第二段。』</p>"), "{out}");
        assert_eq!(corner_quotes(&"“go” “走” it’s".chars().collect::<Vec<_>>()).iter().collect::<String>(), "“go” 『走』 it’s");
    }

    /// **A pair of quotes holding no letter follows the text round it**
    /// (2026-11-09, reported: `“……”`, `“——”` and `“1949”` stayed curly in a
    /// Chinese paragraph): Chinese round it, corner brackets; Western round
    /// it, curly; a pair holding a Latin letter and no CJK stays curly.
    #[test]
    fn quotes_holding_no_letter_follow_the_text_round_them() {
        let q = |t: &str| corner_quotes(&t.chars().collect::<Vec<_>>()).iter().collect::<String>();
        assert_eq!(q("“……”与“——”与“1949”与“？”。"), "『……』与『——』与『1949』与『？』。");
        assert_eq!(q("他喊“OK”就走了，那是‘1949’年。"), "他喊“OK”就走了，那是「1949」年。");
        assert_eq!(q("He wrote “1949” and ‘—’ there."), "He wrote “1949” and ‘—’ there.");
        assert_eq!(q("“……”"), "『……』", "nothing round it: the page is Chinese");
        assert_eq!(q("“……他说"), "『……他说", "a quote with no partner looks past the marks");
        // Code is not text a quote is told by: it follows the text round it.
        let out = vertical_text("<p>代码“<code>走</code>”与“<code>go</code>”。</p>", Latin::Jlreq, true);
        assert!(out.contains("代码『<code>走</code>』与『<code>go</code>』。"), "{out}");
    }

    /// **Under jlreq a single letter with marks joined to it stands as the
    /// letter alone does** (2026-11-09, reported: `x.`, `(x)` and `x,` were
    /// turned), its marks beside it; `x, y` is two letters, not a phrase.
    /// Under `turn` nothing changes.
    #[test]
    fn under_jlreq_a_letter_with_its_marks_stands_upright() {
        let set = |text: &str| vertical_text(text, Latin::Jlreq, true);
        let up = |c: &str| format!("<span class=\"tate\">{c}</span>");
        assert_eq!(set("变量 x, y 和 z。"), format!("变量 {}, {} 和 {}。", up("x"), up("y"), up("z")));
        assert_eq!(set("又 (x) 与 x. 和 A,"), format!("又 ({}) 与 {}. 和 {},", up("x"), up("x"), up("A")));
        assert_eq!(set("和 x’s 与 f(x, y)"), "和 <span class=\"yoko\">x’s</span> 与 <span class=\"yoko\">f(x, y)</span>");
        assert_eq!(vertical_text("变量 x, y", Latin::Turn, true), "变量 <span class=\"yoko\">x, y</span>");
    }

    /// **A letter written with a combining mark stands whole** (review,
    /// 2026-11-09): `e` and U+0301 are one character to a reader, so under
    /// jlreq the upright square holds both.
    #[test]
    fn under_jlreq_a_letter_with_a_combining_mark_stands_whole() {
        let set = |text: &str| vertical_text(text, Latin::Jlreq, true);
        assert_eq!(set("又 e\u{301} 写"), "又 <span class=\"tate\">e\u{301}</span> 写");
        assert_eq!(set("又 (e\u{301}). 写"), "又 (<span class=\"tate\">e\u{301}</span>). 写");
        assert_eq!(set("又 é 写"), "又 <span class=\"tate\">é</span> 写");
    }

    /// **Headings take GitHub's anchors** (review, 2026-11-09), on the HTML
    /// page as in the book: the words in small letters, a space as `-`,
    /// marks left out, Chinese kept, the same words again numbered.
    #[test]
    fn headings_take_github_s_anchors() {
        let mut s = style();
        s.settings.vertical = false;
        let out = export("# 第一章：雪\n\n## Hello, World!\n\n# 第一章：雪\n\n## “引号” 与 **粗**[^1]\n\n[^1]: 注。\n", Format::Html, &s);
        assert!(out.contains("<h1 id=\"第一章雪\">"), "{out}");
        assert!(out.contains("<h2 id=\"hello-world\">"), "{out}");
        assert!(out.contains("<h1 id=\"第一章雪-1\">"), "{out}");
        assert!(out.contains("<h2 id=\"引号-与-粗\">"), "{out}");
        assert_eq!(slug("  A_b-c  d "), "a_b-c--d");
        assert_eq!(slug("！？"), "");
        let out = export("# ！？\n", Format::Html, &s);
        assert!(out.contains("<h1 id=\"heading\">"), "{out}");
    }

    /// **A code block left open is closed in Typst** (review, 2026-11-09):
    /// Markdown ends it with the document or the quotation, and Typst's raw
    /// text would run on, over a `#quote`'s `]` too.
    #[test]
    fn a_code_block_left_open_is_closed_in_typst() {
        let s = style();
        let out = export("甲。\n\n```\n未闭\n", Format::Typst, &s);
        assert!(out.trim_end().ends_with("```\n未闭\n```"), "{out}");
        let out = export("> 引文\n> ```\n> 代码 ]\n\n乙。\n", Format::Typst, &s);
        assert!(out.contains("```\n代码 ]\n```\n]"), "{out}");
        let out = export("```rust\nlet x = 1;\n```\n", Format::Typst, &s);
        assert!(out.contains("```rust\nlet x = 1;\n```\n") && !out.contains("```\n```"), "{out}");
    }

    /// **A formula in a vertical table's cell is held to the cell** (review,
    /// 2026-11-09), so a turned one does not run past its band.
    #[test]
    fn a_formula_in_a_vertical_cell_is_held_to_the_cell() {
        let css = stylesheet(&style(), "zh", false);
        assert!(css.contains("th img.math, td img.math { max-inline-size: 100%; }"), "{css}");
    }

    /// **A picture with no words is drawn** (2026-11-09, reported:
    /// `![](a.png)` came out as nothing), with an empty `alt`.
    #[test]
    fn a_picture_with_no_words_is_drawn() {
        let mut s = style();
        s.settings.vertical = false;
        let out = export("前文 ![](a.png) 后文，又 ![有字](b.png) 与 ![](<my pic.png>) 了。\n", Format::Html, &s);
        assert!(out.contains("前文 <img class=\"inline\" src=\"a.png\" alt=\"\"> 后文"), "{out}");
        assert!(out.contains("<img class=\"inline\" src=\"b.png\" alt=\"有字\">"), "{out}");
        assert!(out.contains("<img class=\"inline\" src=\"my%20pic.png\" alt=\"\">"), "{out}");
        assert!(!out.contains("`![](a.png)`"), "{out}");
        let out = export("代码 `![](a.png)` 里的不算。\n", Format::Html, &s);
        assert!(!out.contains("<img"), "{out}");
    }

    /// **An indented code block is code** (2026-11-09, reported: it came out
    /// a paragraph, its `#` line beside the words), four spaces or a tab, as
    /// the preview reads it; a fence indented inside it is code text, in
    /// HTML and in Typst.
    #[test]
    fn an_indented_code_block_is_code() {
        let mut s = style();
        s.settings.vertical = false;
        let text = "甲段。\n\n    let x = 1;\n    # 不是标题\n\n乙段。\n\n    ```\n    # 缩进围栏\n    ```\n\n丙段。\n\n\tlet y = 2;\n";
        let out = export(text, Format::Html, &s);
        assert!(out.contains("<pre><code>let x = 1;\n# 不是标题</code></pre>"), "{out}");
        assert!(out.contains("<pre><code>```\n# 缩进围栏\n```</code></pre>"), "{out}");
        assert!(out.contains("<pre><code>let y = 2;</code></pre>"), "a tab: {out}");
        assert!(!out.contains("<h1") && out.contains("<p>丙段。</p>"), "{out}");
        let typ = export(text, Format::Typst, &s);
        assert!(typ.contains("````\n```\n# 缩进围栏\n```\n````"), "{typ}");
        assert!(!typ.contains("= 不是标题"), "{typ}");
        // Going on a paragraph, four spaces are the paragraph's.
        let out = export("一行\n    接着\n", Format::Html, &s);
        assert!(!out.contains("<pre>"), "{out}");
    }

    /// **A long word in a vertical table's cell breaks anywhere** (2026-11-09,
    /// reported: a URL ran past the foot of the page).
    #[test]
    fn a_vertical_tables_cells_break_a_long_word() {
        assert!(stylesheet(&style(), "zh", false).contains("th, td { overflow-wrap: anywhere; }"));
        let mut flat = style();
        flat.settings.vertical = false;
        assert!(!stylesheet(&flat, "zh", false).contains("overflow-wrap"));
    }

    /// **One rule for whether quotes turn** (2026-11-09): the page's language
    /// as the export decides it, which the vertical preview asks too; an
    /// English page quoting Chinese keeps its quotes.
    #[test]
    fn quotes_turn_on_a_chinese_page_only() {
        assert!(corners_quotes(document_language("这是一段中文，引用了“走吧”。\n")));
        assert!(!corners_quotes(document_language("This essay is mostly written in English words, and it quotes “走吧” once.\n")));
        assert!(!corners_quotes(document_language("ひらがなとカタカナ。“漢字”。\n")));
        let out = export("This essay is mostly written in English words, and it quotes “走吧” once.\n", Format::Html, &style());
        assert!(out.contains("“走吧”") && !out.contains('『'), "{out}");
    }
}
