//! **Reading YFM** (yumete flavored Markdown) into blocks and inline runs.
//!
//! Two passes, the same split as the editor's own Markdown:
//!
//! - **Blocks** are read line by line here, each knowing the source lines it
//!   came from (`lines`, first and last, counted from 0). This reader began as
//!   the one on the set-aside `md-preview` branch; what it reads beyond
//!   CommonMark and GFM is yumete's own: `:::` containers (VitePress's), GitHub
//!   alerts `> [!NOTE]`, `$$` display formulas, front matter, `<!-- -->` and
//!   `%%` notes left out, and a 宇浩 document's HTML (`<style>` and
//!   `<script>` left out, a `<div>` line looked through, a Vue component shown
//!   as a placeholder).
//! - **Inline** markup is read by [`yumete_core::markdown::spans`], the one
//!   reader the editor's colouring and `:export` use, so the preview and the
//!   page never disagree about what is bold. On top of its spans this file adds
//!   HTML ruby, `<kbd>`/`<sup>`/`<mark>`…, `<img>` and bare URLs.
//!
//! **A line break inside a paragraph** follows `:export`: between two Latin
//! lines it is a space (a hard-wrapped sentence going on), anywhere else it
//! stays a break, because in Chinese a line break is usually meant.
//!
//! **Adding an extension** is a variant in [`BlockKind`] read in
//! [`Reader::blocks`] (or one in [`InlineKind`] read in [`Reader::inlines`]),
//! and its picture in `layout.rs`.

use yumete_core::markdown::{self, Kind as SpanKind};
use yumete_core::mdtable;

/// A line of the file: its number (from 0) and its text.
type Line = (usize, String);

/// **One block**, with the source lines it covers, first and last.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Block {
    pub lines: (usize, usize),
    pub kind: BlockKind,
}

/// What a block is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Heading { level: u8, text: Vec<Inline> },
    Paragraph(Vec<Inline>),
    /// A fenced (or indented) code block: its info string and its lines.
    Code { info: String, lines: Vec<String> },
    /// A display formula, `$$ … $$`, in TeX.
    Maths(String),
    Rule,
    Quote(Vec<Block>),
    /// A `:::` container (`fenced`) or a GitHub alert: which, the word that
    /// named it, its own title if it gives one, and what is in it.
    Callout { kind: Callout, name: String, title: Option<Vec<Inline>>, fenced: bool, body: Vec<Block> },
    List { ordered: bool, start: u64, loose: bool, items: Vec<Item> },
    /// A pipe table: each column's alignment, the heading row's cells, and
    /// each body row's line and cells.
    Table { align: Vec<Align>, head: Vec<Vec<Inline>>, rows: Vec<(usize, Vec<Vec<Inline>>)> },
    /// The `---` metadata a file may open with: its `key: value` lines.
    FrontMatter(Vec<String>),
    /// HTML this preview does not draw (a Vue component): its source, shown
    /// as a placeholder.
    Html(String),
    /// The footnotes, in the order they were first cited: set at the end.
    Footnotes(Vec<Note>),
}

/// One list item: its task box, if it is a task, and its blocks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Item {
    pub task: Option<bool>,
    pub lines: (usize, usize),
    pub body: Vec<Block>,
}

/// A footnote: its number, its name, its lines and its blocks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Note {
    pub number: usize,
    pub name: String,
    pub lines: (usize, usize),
    pub body: Vec<Block>,
}

/// A table column's alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Align {
    /// No colon: the heading row centred, the body to the left.
    #[default]
    Plain,
    Left,
    Center,
    Right,
}

/// Which colour a callout is: GitHub's five, plus `details`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Callout {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
    Details,
}

impl Callout {
    /// The word after `:::` or `[!`: VitePress's set and GitHub's names.
    /// `caution` is the red one, as on GitHub (and `danger` in VitePress).
    pub fn parse(word: &str) -> Option<Callout> {
        match word.trim().to_ascii_lowercase().as_str() {
            "note" | "info" => Some(Callout::Note),
            "tip" => Some(Callout::Tip),
            "important" => Some(Callout::Important),
            "warning" => Some(Callout::Warning),
            "caution" | "danger" | "error" => Some(Callout::Caution),
            "details" => Some(Callout::Details),
            _ => None,
        }
    }
}

/// **One run of a paragraph**, with the source line it stands on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Inline {
    pub line: usize,
    pub kind: InlineKind,
}

/// What a run is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InlineKind {
    Text { text: String, marks: Marks },
    /// `$…$`, or `$$…$$` in the run of a paragraph.
    Formula { tex: String, display: bool },
    /// `![alt](src)` or `<img src alt>`: `src` as written.
    Image { src: String, alt: String },
    /// A break the writer asked for.
    Break,
    /// `<ruby>base<rt>reading</rt></ruby>`.
    Ruby { base: String, reading: String, marks: Marks },
    /// `[^name]`, as the number it was given.
    NoteRef(usize),
}

/// The inline markup over a run of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Marks {
    pub strong: bool,
    pub emph: bool,
    pub code: bool,
    pub strike: bool,
    pub highlight: bool,
    pub link: bool,
    pub wiki: bool,
    pub kbd: bool,
    pub sup: bool,
    pub sub: bool,
    pub underline: bool,
    /// A `<span lang="…">` over it: 0 for the document's own, else 1 more
    /// than [`crate::fonts::Han`]'s index.
    pub lang: u8,
}

/// **Read `text`** into blocks. The footnotes, if any, come last, as one
/// [`BlockKind::Footnotes`].
pub fn parse(text: &str) -> Vec<Block> {
    parse_with(text, &mut Cache::default())
}

/// **What a parse keeps for the next one**: the runs of every paragraph read,
/// by its text, so an edit reads again only the paragraph it changed (the
/// inline reading is most of a parse). A paragraph that cites a footnote is
/// not kept: its numbers depend on what comes before it.
#[derive(Default)]
pub struct Cache {
    runs: std::collections::HashMap<u64, Vec<Inline>>,
}

/// [`parse`], keeping what can be kept in `cache` for the next time.
pub fn parse_with(text: &str, cache: &mut Cache) -> Vec<Block> {
    let lines = prepare(text);
    let mut reader = Reader { old: std::mem::take(&mut cache.runs), refs: link_definitions(&lines), ..Reader::default() };
    let mut blocks = Vec::new();
    // Front matter: `---` on the first line, `key: value` under it, to a `---`.
    let mut from = 0;
    let all: Vec<&str> = text.lines().collect();
    if all.first().is_some_and(|l| l.trim_end() == "---") && all.get(1).is_some_and(|l| metadata(l.trim())) {
        if let Some(close) = (1..all.len()).find(|&k| all[k].trim_end() == "---") {
            let kept = all[1..close].iter().map(|l| l.to_string()).collect();
            blocks.push(Block { lines: (0, close), kind: BlockKind::FrontMatter(kept) });
            from = lines.partition_point(|l| l.0 <= close);
        }
    }
    blocks.extend(reader.blocks(&lines[from..]));
    // A note never cited still gets a number, after the cited ones.
    for note in &reader.notes {
        if !reader.cited.contains(&note.name) {
            reader.cited.push(note.name.clone());
        }
    }
    let mut notes: Vec<Note> = std::mem::take(&mut reader.notes)
        .into_iter()
        .map(|mut n| {
            n.number = reader.cited.iter().position(|c| *c == n.name).map_or(0, |k| k + 1);
            n
        })
        .collect();
    notes.sort_by_key(|n| n.number);
    notes.dedup_by(|a, b| a.name == b.name);
    if !notes.is_empty() {
        let first = notes.iter().map(|n| n.lines.0).min().unwrap_or(0);
        let last = notes.iter().map(|n| n.lines.1).max().unwrap_or(first);
        blocks.push(Block { lines: (first, last), kind: BlockKind::Footnotes(notes) });
    }
    cache.runs = reader.new;
    blocks
}

/// **The lines to read**, numbered, with the lines inside a `<!--` or `%%`
/// note left out, as `:export` leaves them. A note that never closes runs to
/// the end of its own line only.
fn prepare(text: &str) -> Vec<Line> {
    let all: Vec<&str> = text.lines().collect();
    let mut out = Vec::with_capacity(all.len());
    let mut i = 0;
    let mut fence: Option<char> = None;
    while i < all.len() {
        let line = all[i].trim_end_matches('\r');
        let body = line.trim_start();
        let opens = if body.starts_with("```") {
            Some('`')
        } else if body.starts_with("~~~") {
            Some('~')
        } else {
            None
        };
        match (fence, opens) {
            (None, Some(c)) => fence = Some(c),
            (Some(f), Some(c)) if f == c => fence = None,
            _ => {}
        }
        let at = i;
        i += 1;
        if fence.is_some() || opens.is_some() {
            out.push((at, line.to_string()));
            continue;
        }
        let may_open = line.contains("<!--") || line.contains("%%");
        let open = may_open.then(|| markdown::comment_left_open(line)).flatten().and_then(|closer| {
            let close = (i..all.len()).find_map(|k| markdown::comment_closes(all[k], closer).map(|c| (k, c)))?;
            Some((closer, close))
        });
        match open {
            Some((closer, (k, c))) => {
                let opener = if closer == "%%" { "%%" } else { "<!--" };
                let before = line.rfind(opener).map_or(line, |b| &line[..b]);
                if !before.trim().is_empty() {
                    out.push((at, before.to_string()));
                }
                let rest: String = all[k].chars().skip(c + closer.chars().count()).collect();
                if !rest.trim().is_empty() {
                    out.push((k, rest));
                }
                i = k + 1;
            }
            None => out.push((at, line.to_string())),
        }
    }
    out
}

/// Whether a line looks like `key: value`.
fn metadata(text: &str) -> bool {
    text.split_once(':')
        .is_some_and(|(key, _)| !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'))
}

/// How far a line is indented, in bytes of spaces and tabs.
fn lead(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// `---`, `***` or `___`, spaced or not: three or more of one, alone.
fn is_rule(body: &str) -> bool {
    let mut marks = body.chars().filter(|c| !c.is_whitespace());
    let Some(first) = marks.next().filter(|c| matches!(c, '-' | '*' | '_')) else { return false };
    let mut n = 1;
    for c in marks {
        if c != first {
            return false;
        }
        n += 1;
    }
    n >= 3
}

/// `[^name]: text`: the name and the start of the text.
fn note_opening(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("[^")?;
    let close = rest.find("]:")?;
    let name = &rest[..close];
    (!name.is_empty() && !name.contains(char::is_whitespace)).then(|| (name, rest[close + 2..].trim()))
}

/// `[name]: target`: a link reference definition, which is not shown.
fn is_link_definition(body: &str) -> bool {
    let Some(rest) = body.strip_prefix('[') else { return false };
    if rest.starts_with('^') {
        return false;
    }
    rest.find("]:").is_some_and(|close| close > 0 && !rest[close + 2..].trim().is_empty())
}

/// A link label as CommonMark matches it: case and runs of spaces folded.
fn fold_label(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// **The link reference definitions** `[label]: address "title"` of a file.
fn link_definitions(lines: &[Line]) -> std::collections::BTreeMap<String, String> {
    let mut refs = std::collections::BTreeMap::new();
    for (_, line) in lines {
        let body = line.trim_start();
        if lead(line) >= 4 || !is_link_definition(body) {
            continue;
        }
        let close = body.find("]:").unwrap_or(0);
        let label = fold_label(&body[1..close]);
        let rest = body[close + 2..].trim();
        let address = match rest.strip_prefix('<') {
            Some(r) => r.split('>').next().unwrap_or_default(),
            None => rest.split_whitespace().next().unwrap_or_default(),
        };
        refs.entry(label).or_insert_with(|| address.to_string());
    }
    refs
}

/// **Reference links written out as inline ones**: `[text][label]`,
/// `[label][]` and `[label]`, with an image's `!` before any of them, become
/// `[text](address)` when `label` is defined, so the one inline reader reads
/// them. Code spans, wiki links and footnotes are left alone.
fn resolve_references(line: &str, refs: &std::collections::BTreeMap<String, String>) -> String {
    if refs.is_empty() || !line.contains('[') {
        return line.to_string();
    }
    let chars: Vec<char> = line.chars().collect();
    let close_of = |from: usize| -> Option<usize> {
        let mut depth = 0;
        for (k, &c) in chars.iter().enumerate().skip(from) {
            match c {
                '[' => depth += 1,
                ']' if depth == 0 => return Some(k),
                ']' => depth -= 1,
                '`' => return None,
                _ => {}
            }
        }
        None
    };
    let address = |label: &str| -> Option<String> {
        let a = refs.get(&fold_label(label))?;
        Some(match a.contains([' ', '(', ')']) {
            true => format!("<{a}>"),
            false => a.clone(),
        })
    };
    let mut out = String::with_capacity(line.len() + 16);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // A code span is copied as it stands.
        if c == '`' {
            let ticks = chars[i..].iter().take_while(|&&t| t == '`').count();
            let shut = (i + ticks..chars.len()).find(|&k| chars[k..].iter().take_while(|&&t| t == '`').count() == ticks && (k == 0 || chars[k - 1] != '`'));
            let end = shut.map_or(i + ticks, |k| k + ticks);
            out.extend(&chars[i..end]);
            i = end;
            continue;
        }
        let escaped = i > 0 && chars[i - 1] == '\\';
        if c != '[' || escaped || chars.get(i + 1).is_some_and(|&n| n == '[' || n == '^') {
            // `[[wiki]]` is skipped whole, so its inner `[` is not read.
            if c == '[' && chars.get(i + 1) == Some(&'[') {
                if let Some(end) = (i + 2..chars.len().saturating_sub(1)).find(|&k| chars[k] == ']' && chars[k + 1] == ']') {
                    out.extend(&chars[i..end + 2]);
                    i = end + 2;
                    continue;
                }
            }
            out.push(c);
            i += 1;
            continue;
        }
        let Some(close) = close_of(i + 1) else {
            out.push(c);
            i += 1;
            continue;
        };
        let text: String = chars[i + 1..close].iter().collect();
        match chars.get(close + 1) {
            Some('(') | Some(':') => {
                out.extend(&chars[i..=close]);
                i = close + 1;
            }
            Some('[') => match close_of(close + 2) {
                Some(end) => {
                    let label: String = chars[close + 2..end].iter().collect();
                    let label = if label.trim().is_empty() { text.clone() } else { label };
                    match address(&label) {
                        Some(a) => out.push_str(&format!("[{text}]({a})")),
                        None => out.extend(&chars[i..=end]),
                    }
                    i = end + 1;
                }
                None => {
                    out.extend(&chars[i..=close]);
                    i = close + 1;
                }
            },
            _ => {
                match address(&text) {
                    Some(a) => out.push_str(&format!("[{text}]({a})")),
                    None => out.extend(&chars[i..=close]),
                }
                i = close + 1;
            }
        }
    }
    out
}

/// Where a list item's marker is and what it says.
#[derive(Clone, Copy)]
struct Marker {
    indent: usize,
    ordered: bool,
    start: u64,
    /// Where the item's text begins, in bytes: a line indented this far
    /// belongs to the item.
    width: usize,
    /// The same, for the first line's text.
    text: usize,
}

/// The list item `line` opens, if it opens one.
fn list_item(line: &str) -> Option<Marker> {
    let indent = lead(line);
    let body = &line[indent..];
    if is_rule(body.trim_end()) {
        return None;
    }
    let digits = body.find(|c: char| !c.is_ascii_digit()).unwrap_or(body.len());
    let (ordered, start, marker) = match body.chars().next()? {
        '-' | '*' | '+' => (false, 1, 1),
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
    Some(Marker { indent, ordered, start, width, text: indent + marker + gap })
}

/// The fence a line opens, if it opens one: its character and how many.
fn fence_of(body: &str) -> Option<(char, usize)> {
    let c = body.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let n = body.chars().take_while(|&d| d == c).count();
    (n >= 3).then_some((c, n))
}

/// Whether a line goes on the paragraph above it (lazy continuation).
fn plain_line(line: &str) -> bool {
    let body = line.trim_start();
    !(body.starts_with(['#', '>', '|']) || body.starts_with("```") || body.starts_with("~~~") || body.starts_with(":::"))
        && list_item(line).is_none()
        && !is_rule(body)
}

/// The HTML element a line opens with, lower-cased, and whether it closes.
fn html_open(body: &str) -> Option<(String, bool)> {
    let rest = body.strip_prefix('<')?;
    let closing = rest.starts_with('/');
    let rest = rest.trim_start_matches('/');
    let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let after = rest[name.len()..].chars().next();
    if !after.is_none_or(|c| c.is_whitespace() || c == '>' || c == '/') {
        return None;
    }
    Some((name, closing))
}

/// A line that is one element and nothing else: `<div class="x">`, `</div>`.
fn wrapper_line(body: &str) -> bool {
    const LOOKED_THROUGH: &[&str] = &["div", "center", "section", "article", "span", "p", "figure", "template"];
    let Some((name, _)) = html_open(body) else { return false };
    LOOKED_THROUGH.contains(&name.to_ascii_lowercase().as_str()) && body.trim_end().ends_with('>') && body.matches('<').count() == 1
}

/// Reads blocks, numbering footnotes as they are cited.
#[derive(Default)]
struct Reader {
    cited: Vec<String>,
    notes: Vec<Note>,
    /// The file's link reference definitions: label (folded) to address.
    refs: std::collections::BTreeMap<String, String>,
    /// The runs kept from the last parse, and those kept for the next.
    old: std::collections::HashMap<u64, Vec<Inline>>,
    new: std::collections::HashMap<u64, Vec<Inline>>,
}

impl Reader {
    /// The number of footnote `name`, given at its first citation.
    fn cite(&mut self, name: &str) -> usize {
        match self.cited.iter().position(|c| c == name) {
            Some(k) => k + 1,
            None => {
                self.cited.push(name.to_string());
                self.cited.len()
            }
        }
    }

    /// A paragraph of `lines`: a lone display formula is a block of its own.
    fn paragraph(&mut self, lines: &[Line], out: &mut Vec<Block>) {
        if lines.is_empty() {
            return;
        }
        let span = (lines[0].0, lines[lines.len() - 1].0);
        let runs = self.inlines(lines);
        // A paragraph of nothing but notes to self is no paragraph.
        let visible = runs.iter().any(|r| match &r.kind {
            InlineKind::Text { text, .. } => !text.trim().is_empty(),
            InlineKind::Break => false,
            _ => true,
        });
        if !visible {
            return;
        }
        let kind = match runs.as_slice() {
            [Inline { kind: InlineKind::Formula { tex, display: true }, .. }] => BlockKind::Maths(tex.clone()),
            _ => BlockKind::Paragraph(runs),
        };
        out.push(Block { lines: span, kind });
    }

    /// **Read `lines` into blocks**; footnotes are taken out of the flow.
    fn blocks(&mut self, lines: &[Line]) -> Vec<Block> {
        let mut out = Vec::new();
        let mut para: Vec<Line> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let no = lines[i].0;
            let raw = &lines[i].1;
            let line = raw.trim_end();
            let body = line.trim_start();
            let indent = lead(line);
            if body.is_empty() {
                let taken = std::mem::take(&mut para);
                self.paragraph(&taken, &mut out);
                i += 1;
                continue;
            }
            // An indented code block: four spaces, not going on a paragraph.
            if indent >= 4 && para.is_empty() {
                let mut held: Vec<Line> = Vec::new();
                while i < lines.len() && (lead(&lines[i].1) >= 4 || lines[i].1.trim().is_empty()) {
                    let text = &lines[i].1;
                    held.push((lines[i].0, text.get(4.min(lead(text))..).unwrap_or_default().to_string()));
                    i += 1;
                }
                while held.last().is_some_and(|l| l.1.trim().is_empty()) {
                    held.pop();
                }
                let span = (held[0].0, held[held.len() - 1].0);
                out.push(Block {
                    lines: span,
                    kind: BlockKind::Code { info: String::new(), lines: held.into_iter().map(|l| l.1).collect() },
                });
                continue;
            }
            // A fence swallows everything up to the fence that closes it.
            if let Some((c, n)) = fence_of(body).filter(|_| indent < 4) {
                let taken = std::mem::take(&mut para);
                self.paragraph(&taken, &mut out);
                let info = body.trim_start_matches(c).trim().to_string();
                let mut held = Vec::new();
                let mut last = no;
                i += 1;
                while i < lines.len() {
                    let (k, text) = &lines[i];
                    last = *k;
                    i += 1;
                    if fence_of(text.trim()).is_some_and(|(d, m)| d == c && m >= n) && text.trim().trim_start_matches(c).is_empty() {
                        break;
                    }
                    let cut = lead(text).min(indent);
                    held.push(text[cut..].to_string());
                }
                out.push(Block { lines: (no, last), kind: BlockKind::Code { info, lines: held } });
                continue;
            }
            // A `:::` container, to the `:::` that closes it.
            if let Some(rest) = body.strip_prefix(":::").filter(|_| indent < 4) {
                let mut words = rest.split_whitespace();
                if let Some(name) = words.next() {
                    let taken = std::mem::take(&mut para);
                    self.paragraph(&taken, &mut out);
                    let title: Vec<&str> = words.collect();
                    let kind = Callout::parse(name).unwrap_or(Callout::Note);
                    let title = (!title.is_empty()).then(|| title.join(" "));
                    let mut depth = 1;
                    let mut inner = Vec::new();
                    let mut last = no;
                    i += 1;
                    while i < lines.len() {
                        let (k, text) = &lines[i];
                        last = *k;
                        i += 1;
                        if let Some(r) = text.trim().strip_prefix(":::") {
                            match r.split_whitespace().next() {
                                Some(_) => depth += 1,
                                None => depth -= 1,
                            }
                            if depth == 0 {
                                break;
                            }
                        }
                        inner.push((*k, text.clone()));
                    }
                    let title = title.map(|t| self.inlines(&[(no, t)]));
                    let body = self.blocks(&inner);
                    out.push(Block {
                        lines: (no, last),
                        kind: BlockKind::Callout { kind, name: name.to_string(), title, fenced: true, body },
                    });
                    continue;
                }
            }
            // Display maths on lines of its own: `$$` … `$$`, closed before a
            // blank line, or the whole formula on one line.
            if body.starts_with("$$") && indent < 4 {
                let after = &body[2..];
                if let Some(close) = after.find("$$").filter(|&c| after[c + 2..].trim().is_empty()) {
                    if !after[..close].trim().is_empty() {
                        let taken = std::mem::take(&mut para);
                        self.paragraph(&taken, &mut out);
                        out.push(Block { lines: (no, no), kind: BlockKind::Maths(after[..close].trim().to_string()) });
                        i += 1;
                        continue;
                    }
                }
                if !after.contains("$$") {
                    let end = (i + 1..lines.len())
                        .take_while(|&k| !lines[k].1.trim().is_empty())
                        .find(|&k| lines[k].1.contains("$$"));
                    if let Some(end) = end {
                        let taken = std::mem::take(&mut para);
                        self.paragraph(&taken, &mut out);
                        let held: Vec<String> = lines[i..=end].iter().map(|(_, t)| t.trim().to_string()).collect();
                        let tex = markdown::maths_block_source(&held, false).trim().to_string();
                        out.push(Block { lines: (no, lines[end].0), kind: BlockKind::Maths(tex) });
                        i = end + 1;
                        continue;
                    }
                }
            }
            // A footnote's text, out of the flow, with the lines that go on.
            if indent == 0 {
                if let Some((name, first)) = note_opening(body) {
                    let taken = std::mem::take(&mut para);
                    self.paragraph(&taken, &mut out);
                    let mut text = vec![(no, first.to_string())];
                    let mut last = no;
                    i += 1;
                    while i < lines.len() {
                        let (k, next) = &lines[i];
                        let blank = next.trim().is_empty();
                        let indented = next.starts_with([' ', '\t']) && !blank;
                        let goes_on = blank
                            && lines.get(i + 1).is_some_and(|(_, after)| after.starts_with([' ', '\t']) && !after.trim().is_empty());
                        let lazy = !blank && !text.last().is_some_and(|l| l.1.is_empty()) && plain_line(next) && note_opening(next).is_none();
                        if !(indented || goes_on || lazy) {
                            break;
                        }
                        text.push((*k, next.trim().to_string()));
                        last = *k;
                        i += 1;
                    }
                    let body = self.blocks(&text);
                    self.notes.push(Note { number: 0, name: name.to_string(), lines: (no, last), body });
                    continue;
                }
                if is_link_definition(body) {
                    let taken = std::mem::take(&mut para);
                    self.paragraph(&taken, &mut out);
                    i += 1;
                    continue;
                }
            }
            if let Some(level) = markdown::heading_marks(body, '#').filter(|_| indent < 4) {
                let title = body[level..].trim();
                let title = match title.trim_end_matches('#') {
                    t if t.ends_with(' ') || t.is_empty() => t.trim_end(),
                    _ => title,
                };
                let taken = std::mem::take(&mut para);
                self.paragraph(&taken, &mut out);
                let text = self.inlines(&[(no, title.to_string())]);
                out.push(Block { lines: (no, no), kind: BlockKind::Heading { level: level.min(6) as u8, text } });
                i += 1;
                continue;
            }
            // A setext heading: the paragraph above, underlined.
            if !para.is_empty() && indent < 4 {
                let level = match body {
                    b if b.chars().all(|c| c == '=') => Some(1),
                    b if b.len() >= 2 && b.chars().all(|c| c == '-') => Some(2),
                    _ => None,
                };
                if let Some(level) = level {
                    let taken = std::mem::take(&mut para);
                    let text = self.inlines(&taken);
                    out.push(Block { lines: (taken[0].0, no), kind: BlockKind::Heading { level, text } });
                    i += 1;
                    continue;
                }
            }
            if indent < 4 && is_rule(body) {
                let taken = std::mem::take(&mut para);
                self.paragraph(&taken, &mut out);
                out.push(Block { lines: (no, no), kind: BlockKind::Rule });
                i += 1;
                continue;
            }
            // HTML: `<style>` and `<script>` are not shown, a lone `<div>` line
            // is looked through, a component is a placeholder.
            if indent < 4 && para.is_empty() {
                if let Some((name, closing)) = html_open(body) {
                    let lower = name.to_ascii_lowercase();
                    if !closing && (lower == "style" || lower == "script") {
                        let close = format!("</{lower}");
                        while i < lines.len() && !lines[i].1.to_ascii_lowercase().contains(&close) {
                            i += 1;
                        }
                        i += 1;
                        continue;
                    }
                    if wrapper_line(body) {
                        i += 1;
                        continue;
                    }
                    let component = name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) || name.contains('-');
                    if component && !closing {
                        let mut text = body.to_string();
                        let mut last = no;
                        while !text.trim_end().ends_with('>') && i + 1 < lines.len() && !lines[i + 1].1.trim().is_empty() {
                            i += 1;
                            last = lines[i].0;
                            text.push(' ');
                            text.push_str(lines[i].1.trim());
                        }
                        i += 1;
                        out.push(Block { lines: (no, last), kind: BlockKind::Html(text) });
                        continue;
                    }
                    if component && closing {
                        i += 1;
                        continue;
                    }
                }
            }
            if indent < 4 && body.starts_with('>') {
                let taken = std::mem::take(&mut para);
                self.paragraph(&taken, &mut out);
                let mut inner: Vec<Line> = Vec::new();
                while i < lines.len() {
                    let text = lines[i].1.trim_start();
                    if let Some(rest) = text.strip_prefix('>') {
                        inner.push((lines[i].0, rest.strip_prefix(' ').unwrap_or(rest).to_string()));
                    } else if !text.is_empty() && inner.last().is_some_and(|l| !l.1.trim().is_empty()) && plain_line(text) {
                        // A lazy line goes on the quoted paragraph.
                        inner.push((lines[i].0, text.to_string()));
                    } else {
                        break;
                    }
                    i += 1;
                }
                let span = (no, inner.last().map_or(no, |l| l.0));
                match inner.first().and_then(|first| alert(&first.1)) {
                    Some((kind, name, title)) => {
                        let title = title.map(|t| self.inlines(&[(no, t)]));
                        let body = self.blocks(&inner[1..]);
                        out.push(Block { lines: span, kind: BlockKind::Callout { kind, name, title, fenced: false, body } });
                    }
                    None => {
                        let body = self.blocks(&inner);
                        out.push(Block { lines: span, kind: BlockKind::Quote(body) });
                    }
                }
                continue;
            }
            let rowish = |l: &str| mdtable::is_row(l) || mdtable::has_bar(l);
            let align = match para.is_empty() && line.contains('|') && rowish(line) {
                true => lines.get(i + 1).and_then(|l| mdtable::rule_of(&l.1)),
                false => None,
            };
            if let Some(align) = align {
                let head_line = no;
                let head: Vec<Vec<Inline>> = mdtable::split(line).into_iter().map(|c| self.inlines(&[(no, c)])).collect();
                let align: Vec<Align> = align
                    .into_iter()
                    .map(|a| match a {
                        mdtable::Align::Plain => Align::Plain,
                        mdtable::Align::Left => Align::Left,
                        mdtable::Align::Center => Align::Center,
                        mdtable::Align::Right => Align::Right,
                    })
                    .collect();
                let mut last = lines[i + 1].0;
                i += 2;
                let mut rows = Vec::new();
                while i < lines.len() && !lines[i].1.trim().is_empty() && rowish(&lines[i].1) {
                    let (k, text) = &lines[i];
                    let cells = mdtable::split(text).into_iter().map(|c| self.inlines(&[(*k, c)])).collect();
                    rows.push((*k, cells));
                    last = *k;
                    i += 1;
                }
                out.push(Block { lines: (head_line, last), kind: BlockKind::Table { align, head, rows } });
                continue;
            }
            if let Some(first) = list_item(line).filter(|m| m.indent < 4) {
                // An ordered list may interrupt a paragraph only from 1.
                if para.is_empty() || !first.ordered || first.start == 1 {
                    let taken = std::mem::take(&mut para);
                    self.paragraph(&taken, &mut out);
                    out.push(self.list(lines, &mut i, first));
                    continue;
                }
            }
            para.push((no, raw.trim_end_matches(['\r', '\n']).to_string()));
            i += 1;
        }
        let taken = std::mem::take(&mut para);
        self.paragraph(&taken, &mut out);
        out
    }

    /// A list, from the item at `*i`; `*i` ends past it.
    fn list(&mut self, lines: &[Line], i: &mut usize, first: Marker) -> Block {
        let mut items = Vec::new();
        let mut item = first;
        let mut loose = false;
        loop {
            let opener = lines[*i].0;
            let mut held = vec![(opener, lines[*i].1.get(item.text..).unwrap_or_default().trim_end().to_string())];
            *i += 1;
            while *i < lines.len() {
                let next = &lines[*i].1;
                let blank = next.trim().is_empty();
                if !blank && lead(next) >= item.width {
                    held.push((lines[*i].0, next[item.width.min(next.len())..].to_string()));
                } else if blank {
                    let goes_on = lines.get(*i + 1).is_some_and(|(_, after)| !after.trim().is_empty() && lead(after) >= item.width);
                    if !goes_on {
                        break;
                    }
                    loose = true;
                    held.push((lines[*i].0, String::new()));
                } else if list_item(next).is_none() && !held.last().is_some_and(|l| l.1.trim().is_empty()) && plain_line(next) {
                    held.push((lines[*i].0, next.trim().to_string()));
                } else {
                    break;
                }
                *i += 1;
            }
            // A task's box opens its first line.
            let task = held.first().and_then(|(_, first)| {
                let mark = first.strip_prefix('[')?.chars().next()?;
                let rest = first.get(3..)?;
                (first.get(2..3) == Some("]") && (rest.is_empty() || rest.starts_with(' ')) && matches!(mark, ' ' | 'x' | 'X'))
                    .then_some(mark != ' ')
            });
            if task.is_some() {
                held[0].1 = held[0].1.get(3..).unwrap_or_default().trim_start().to_string();
            }
            let last = held.iter().rev().find(|l| !l.1.trim().is_empty()).map_or(opener, |l| l.0);
            let body = self.blocks(&held);
            // A loose item's blank line between two blocks makes the list loose.
            items.push(Item { task, lines: (opener, last), body });
            let mut at = *i;
            while at < lines.len() && lines[at].1.trim().is_empty() {
                at += 1;
            }
            match lines.get(at).and_then(|l| list_item(&l.1)) {
                Some(next) if next.indent == item.indent && next.ordered == item.ordered && at <= *i + 1 => {
                    loose |= at > *i;
                    *i = at;
                    item = next;
                }
                _ => break,
            }
        }
        let span = (items[0].lines.0, items.last().map_or(items[0].lines.1, |it| it.lines.1));
        Block { lines: span, kind: BlockKind::List { ordered: first.ordered, start: first.start, loose, items } }
    }

    /// **The runs of a paragraph's lines.** The lines are read as one text,
    /// so emphasis may run from one line to the next.
    fn inlines(&mut self, lines: &[Line]) -> Vec<Inline> {
        use std::hash::{Hash, Hasher};
        let Some(base) = lines.first().map(|l| l.0) else { return Vec::new() };
        if lines.iter().any(|l| l.1.contains("[^")) {
            return self.read_inlines(lines);
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (no, text) in lines {
            (no - base, text).hash(&mut h);
        }
        // A reference link reads differently once its definition changes.
        if lines.iter().any(|l| l.1.contains('[')) {
            self.refs.hash(&mut h);
        }
        let key = h.finish();
        let mut runs = match self.old.remove(&key).or_else(|| self.new.get(&key).cloned()) {
            Some(kept) => kept,
            None => {
                let mut read = self.read_inlines(lines);
                for r in &mut read {
                    r.line -= base;
                }
                read
            }
        };
        self.new.insert(key, runs.clone());
        for r in &mut runs {
            r.line += base;
        }
        runs
    }

    /// The runs of `lines`, read.
    fn read_inlines(&mut self, lines: &[Line]) -> Vec<Inline> {
        // The text, a `\n` between two lines, and where each line starts.
        let mut text = String::new();
        let mut starts: Vec<(usize, usize, bool)> = Vec::new(); // char start, line, hard break after
        let mut count = 0;
        for (k, (no, raw)) in lines.iter().enumerate() {
            let hard = raw.ends_with("  ") || raw.trim_end().ends_with('\\');
            let mut body = resolve_references(raw.trim(), &self.refs);
            if body.ends_with('\\') && k + 1 < lines.len() {
                body.pop();
                body.truncate(body.trim_end().len());
            }
            if k > 0 {
                text.push('\n');
                count += 1;
            }
            starts.push((count, *no, hard));
            count += body.chars().count();
            text.push_str(&body);
        }
        let chars: Vec<char> = text.chars().collect();
        let spans = markdown::spans(&text);
        let groups = yumete_core::ruby::groups(&chars, yumete_core::ruby::Dialects::only(yumete_core::ruby::Dialect::Html));
        let line_at = |i: usize| -> usize {
            let k = starts.partition_point(|s| s.0 <= i).saturating_sub(1);
            starts.get(k).map_or(0, |s| s.1)
        };
        // What covers each character, worked out once: the marks over it,
        // the innermost construct that hides it, whether it is code.
        let mut covered = vec![false; chars.len() + 1];
        let mut span_marks = vec![Marks::default(); chars.len() + 1];
        let mut hides: Vec<Option<usize>> = vec![None; chars.len() + 1];
        for (k, s) in spans.iter().enumerate() {
            let hidden = matches!(s.kind, SpanKind::Marker | SpanKind::HeadingMark | SpanKind::Comment | SpanKind::Html | SpanKind::Math | SpanKind::Footnote);
            for i in s.start..s.end.min(chars.len()) {
                covered[i] = true;
                add_mark(&mut span_marks[i], s.kind);
                if hidden {
                    hides[i] = Some(k);
                }
            }
        }
        let urls = bare_urls(&chars, &covered);
        let marks_at = |i: usize, html: Marks, url: bool| -> Marks {
            let mut m = span_marks[i];
            for (on, html_on) in [
                (&mut m.strong, html.strong),
                (&mut m.emph, html.emph),
                (&mut m.code, html.code),
                (&mut m.strike, html.strike),
                (&mut m.highlight, html.highlight),
                (&mut m.kbd, html.kbd),
                (&mut m.sup, html.sup),
                (&mut m.sub, html.sub),
                (&mut m.underline, html.underline),
            ] {
                *on |= html_on;
            }
            m.lang = html.lang;
            m.link |= url;
            m
        };
        let mut out: Vec<Inline> = Vec::new();
        let mut html = Marks::default();
        let mut spans_open: Vec<u8> = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let line = line_at(i);
            if let Some(group) = groups.iter().find(|g| g.start == i) {
                let base = group.dialect.unescape(&group.base_text(&chars).iter().collect::<String>());
                let reading = group.dialect.unescape(&group.reading_text(&chars).iter().collect::<String>());
                let marks = marks_at(i, html, false);
                out.push(Inline { line, kind: InlineKind::Ruby { base, reading, marks } });
                i = group.end.max(i + 1);
                continue;
            }
            let owner = |construct: usize, kind: SpanKind| spans.iter().find(|s| s.construct == construct && s.kind == kind);
            let last_marker = |construct: usize| spans.iter().rfind(|s| s.construct == construct && s.kind == SpanKind::Marker);
            match hides[i].map(|k| &spans[k]) {
                None => {
                    let c = chars[i];
                    if c == '\n' {
                        let in_code = span_marks[i].code;
                        let k = starts.partition_point(|s| s.0 <= i).saturating_sub(1);
                        let hard = starts.get(k).is_some_and(|s| s.2);
                        let latin = |c: Option<&char>| c.is_some_and(|&c| yumete_cjk::char_width(c) == 1);
                        if in_code || (!hard && latin(chars.get(i.wrapping_sub(1))) && latin(chars.get(i + 1))) {
                            push_text(&mut out, line, " ", marks_at(i, html, urls[i]));
                        } else {
                            out.push(Inline { line, kind: InlineKind::Break });
                        }
                    } else {
                        let mut buf = [0u8; 4];
                        push_text(&mut out, line, c.encode_utf8(&mut buf), marks_at(i, html, urls[i]));
                    }
                    i += 1;
                }
                Some(span) => match span.kind {
                    // `![alt](src)`: a picture.
                    SpanKind::Marker if span.start == i && chars.get(i) == Some(&'!') && chars.get(i + 1) == Some(&'[') => {
                        let close = last_marker(span.construct).filter(|c| c.start > span.start);
                        match close {
                            Some(c) => {
                                let alt: String = chars[span.end..c.start].iter().collect();
                                let src = image_source(&chars[c.start..c.end].iter().collect::<String>());
                                out.push(Inline { line, kind: InlineKind::Image { src, alt: alt.trim().to_string() } });
                                i = c.end;
                            }
                            None => i += 1,
                        }
                    }
                    // `$…$`: the formula.
                    SpanKind::Marker | SpanKind::Math if chars.get(i) == Some(&'$') && span.start == i && owner(span.construct, SpanKind::Math).is_some() => {
                        let body = owner(span.construct, SpanKind::Math).copied();
                        let close = last_marker(span.construct).filter(|c| c.start > i);
                        match (body, close) {
                            (Some(body), Some(close)) => {
                                let tex: String = chars[body.start..body.end].iter().collect();
                                let display = span.end - span.start == 2;
                                out.push(Inline { line, kind: InlineKind::Formula { tex: tex.trim().to_string(), display } });
                                i = close.end;
                            }
                            _ => i += 1,
                        }
                    }
                    SpanKind::Footnote => {
                        let name: String = chars[span.start..span.end].iter().collect();
                        let name = name.trim_start_matches("[^").trim_end_matches(':').trim_end_matches(']').to_string();
                        let n = self.cite(&name);
                        out.push(Inline { line, kind: InlineKind::NoteRef(n) });
                        i = span.end;
                    }
                    SpanKind::Html => {
                        let tag: String = chars[span.start..span.end].iter().collect();
                        html_tag(&tag, &mut html, &mut spans_open, line, &mut out);
                        i = span.end;
                    }
                    _ => i += 1,
                },
            }
        }
        // Trailing spaces and breaks say nothing.
        while matches!(out.last(), Some(Inline { kind: InlineKind::Break, .. })) {
            out.pop();
        }
        out
    }
}

/// Text appended to the last run when its marks are the same.
fn push_text(out: &mut Vec<Inline>, line: usize, s: &str, marks: Marks) {
    if let Some(Inline { line: was, kind: InlineKind::Text { text, marks: m } }) = out.last_mut() {
        if *m == marks && *was == line {
            text.push_str(s);
            return;
        }
    }
    out.push(Inline { line, kind: InlineKind::Text { text: s.to_string(), marks } });
}

/// The mark a span of `kind` puts on what it covers.
fn add_mark(m: &mut Marks, kind: SpanKind) {
    match kind {
        SpanKind::Strong => m.strong = true,
        SpanKind::Emphasis => m.emph = true,
        SpanKind::Code => m.code = true,
        SpanKind::Strike => m.strike = true,
        SpanKind::Highlight => m.highlight = true,
        SpanKind::Link => m.link = true,
        SpanKind::WikiLink => m.wiki = true,
        _ => {}
    }
}

/// An HTML tag in the run of the text: `<br>` breaks, `<img>` is a picture,
/// the phrase tags mark, anything else comes off.
fn html_tag(tag: &str, html: &mut Marks, spans_open: &mut Vec<u8>, line: usize, out: &mut Vec<Inline>) {
    let lower = tag.to_ascii_lowercase();
    let closing = lower.starts_with("</");
    let name: String = lower.trim_start_matches("</").trim_start_matches('<').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    let on = !closing;
    match name.as_str() {
        "br" => out.push(Inline { line, kind: InlineKind::Break }),
        "img" if on => {
            let src = attribute(tag, "src").unwrap_or_default();
            let alt = attribute(tag, "alt").unwrap_or_default();
            out.push(Inline { line, kind: InlineKind::Image { src, alt } });
        }
        "kbd" => html.kbd = on,
        "sup" => html.sup = on,
        "sub" => html.sub = on,
        "mark" => html.highlight = on,
        "b" | "strong" => html.strong = on,
        "i" | "em" | "cite" | "dfn" | "var" => html.emph = on,
        "u" | "ins" => html.underline = on,
        "s" | "del" => html.strike = on,
        "code" | "samp" => html.code = on,
        // `<span lang="ja">`: the run is set in that writing system's face.
        "span" if on => {
            spans_open.push(html.lang);
            if let Some(han) = attribute(tag, "lang").and_then(|l| crate::fonts::Han::from_tag(&l)) {
                html.lang = han.index() as u8 + 1;
            }
        }
        "span" => html.lang = spans_open.pop().unwrap_or(0),
        _ => {}
    }
}

/// The value of attribute `name` in a tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name) {
        let at = from + at;
        let before = lower[..at].chars().last();
        let rest = tag[at + name.len()..].trim_start();
        if before.is_some_and(char::is_whitespace) {
            if let Some(value) = rest.strip_prefix('=') {
                let value = value.trim_start();
                return Some(match value.chars().next() {
                    Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or_default().to_string(),
                    _ => value.split(|c: char| c.is_whitespace() || c == '>' || c == '/').next().unwrap_or_default().to_string(),
                });
            }
        }
        from = at + name.len();
    }
    None
}

/// Where an image's file is, from its closing `](src "title")`.
fn image_source(closing: &str) -> String {
    let inside = closing.strip_prefix("](").unwrap_or(closing);
    let inside = inside.strip_suffix(')').unwrap_or(inside).trim();
    match inside.strip_prefix('<') {
        Some(rest) => rest.split('>').next().unwrap_or_default().to_string(),
        None => inside.split_whitespace().next().unwrap_or_default().to_string(),
    }
}

/// **Bare URLs** (GFM's extended autolinks): `https://…`, `http://…` and
/// `www.…` outside any markup, to the first space or 漢字, without the
/// punctuation that ends a sentence.
fn bare_urls(chars: &[char], covered: &[bool]) -> Vec<bool> {
    let mut out = vec![false; chars.len() + 1];
    let covered = |i: usize| covered[i];
    let starts_with = |i: usize, s: &str| s.chars().enumerate().all(|(k, c)| chars.get(i + k) == Some(&c));
    let mut i = 0;
    while i < chars.len() {
        let word_start = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
        if word_start && !covered(i) && (starts_with(i, "https://") || starts_with(i, "http://") || starts_with(i, "www.")) {
            let mut end = i;
            while end < chars.len() && !chars[end].is_whitespace() && yumete_cjk::char_width(chars[end]) == 1 && chars[end] != '<' && !covered(end) {
                end += 1;
            }
            while end > i && matches!(chars[end - 1], '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '"' | ')') {
                if chars[end - 1] == ')' && chars[i..end].iter().filter(|&&c| c == '(').count() >= chars[i..end].iter().filter(|&&c| c == ')').count() {
                    break;
                }
                end -= 1;
            }
            for flag in &mut out[i..end] {
                *flag = true;
            }
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    out
}

/// The `[!NOTE]` an alert opens with: its kind, its word, and the title after
/// it.
fn alert(first: &str) -> Option<(Callout, String, Option<String>)> {
    let rest = first.trim().strip_prefix("[!")?;
    let close = rest.find(']')?;
    let word = &rest[..close];
    let kind = Callout::parse(word)?;
    let title = rest[close + 1..].trim();
    Some((kind, word.to_string(), (!title.is_empty()).then(|| title.to_string())))
}

/// **The pictures `block` shows**, as written.
pub fn images(block: &Block) -> Vec<&str> {
    fn runs<'a>(rs: &'a [Inline], out: &mut Vec<&'a str>) {
        for r in rs {
            if let InlineKind::Image { src, .. } = &r.kind {
                out.push(src);
            }
        }
    }
    fn walk<'a>(b: &'a Block, out: &mut Vec<&'a str>) {
        match &b.kind {
            BlockKind::Heading { text, .. } | BlockKind::Paragraph(text) => runs(text, out),
            BlockKind::Quote(body) => body.iter().for_each(|c| walk(c, out)),
            BlockKind::Callout { title, body, .. } => {
                if let Some(t) = title {
                    runs(t, out);
                }
                body.iter().for_each(|c| walk(c, out));
            }
            BlockKind::List { items, .. } => items.iter().flat_map(|i| &i.body).for_each(|c| walk(c, out)),
            BlockKind::Table { head, rows, .. } => {
                head.iter().for_each(|c| runs(c, out));
                rows.iter().flat_map(|r| &r.1).for_each(|c| runs(c, out));
            }
            BlockKind::Footnotes(notes) => notes.iter().flat_map(|n| &n.body).for_each(|c| walk(c, out)),
            BlockKind::Code { .. } | BlockKind::Maths(_) | BlockKind::Rule | BlockKind::FrontMatter(_) | BlockKind::Html(_) => {}
        }
    }
    let mut out = Vec::new();
    walk(block, &mut out);
    out
}

/// **`block` with its lines counted from `base`**: what the layout cache keys
/// on, so a block that only moved down the file is not laid out again.
pub fn rebased(block: &Block, base: usize) -> Block {
    let mut b = block.clone();
    shift_block(&mut b, base);
    b
}

fn shift_block(b: &mut Block, base: usize) {
    b.lines = (b.lines.0.saturating_sub(base), b.lines.1.saturating_sub(base));
    match &mut b.kind {
        BlockKind::Heading { text, .. } | BlockKind::Paragraph(text) => shift_inlines(text, base),
        BlockKind::Quote(body) => body.iter_mut().for_each(|c| shift_block(c, base)),
        BlockKind::Callout { title, body, .. } => {
            if let Some(t) = title {
                shift_inlines(t, base);
            }
            body.iter_mut().for_each(|c| shift_block(c, base));
        }
        BlockKind::List { items, .. } => {
            for it in items {
                it.lines = (it.lines.0.saturating_sub(base), it.lines.1.saturating_sub(base));
                it.body.iter_mut().for_each(|c| shift_block(c, base));
            }
        }
        BlockKind::Table { head, rows, .. } => {
            head.iter_mut().for_each(|c| shift_inlines(c, base));
            for (line, cells) in rows {
                *line = line.saturating_sub(base);
                cells.iter_mut().for_each(|c| shift_inlines(c, base));
            }
        }
        BlockKind::Footnotes(notes) => {
            for n in notes {
                n.lines = (n.lines.0.saturating_sub(base), n.lines.1.saturating_sub(base));
                n.body.iter_mut().for_each(|c| shift_block(c, base));
            }
        }
        BlockKind::Code { .. } | BlockKind::Maths(_) | BlockKind::Rule | BlockKind::FrontMatter(_) | BlockKind::Html(_) => {}
    }
}

fn shift_inlines(runs: &mut [Inline], base: usize) {
    for r in runs {
        r.line = r.line.saturating_sub(base);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<BlockKind> {
        parse(text).into_iter().map(|b| b.kind).collect()
    }

    fn plain(runs: &[Inline]) -> String {
        runs.iter()
            .map(|r| match &r.kind {
                InlineKind::Text { text, .. } => text.clone(),
                InlineKind::Break => "\n".to_string(),
                InlineKind::Formula { tex, .. } => format!("${tex}$"),
                InlineKind::Image { alt, .. } => format!("[img {alt}]"),
                InlineKind::Ruby { base, reading, .. } => format!("{base}({reading})"),
                InlineKind::NoteRef(n) => format!("[{n}]"),
            })
            .collect()
    }

    fn para(text: &str) -> Vec<Inline> {
        match parse(text).remove(0).kind {
            BlockKind::Paragraph(runs) => runs,
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    fn marks_of(runs: &[Inline], word: &str) -> Marks {
        runs.iter()
            .find_map(|r| match &r.kind {
                InlineKind::Text { text, marks } if text.contains(word) => Some(*marks),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{word:?} not in {runs:?}"))
    }

    #[test]
    fn headings_paragraphs_and_their_lines() {
        let blocks = parse("# 第一章\n\n那年冬天，\n雪下得很大。\n\nSetext\n======\n");
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].lines, (0, 0));
        assert!(matches!(&blocks[0].kind, BlockKind::Heading { level: 1, text } if plain(text) == "第一章"));
        assert_eq!(blocks[1].lines, (2, 3));
        // A Chinese line break stays a break, as `:export` keeps it.
        assert!(matches!(&blocks[1].kind, BlockKind::Paragraph(r) if plain(r) == "那年冬天，\n雪下得很大。"));
        assert_eq!(blocks[2].lines, (5, 6));
        assert!(matches!(&blocks[2].kind, BlockKind::Heading { level: 2.., .. } | BlockKind::Heading { level: 1, .. }));
    }

    #[test]
    fn latin_lines_join_with_a_space() {
        let runs = para("one line\ngoes on");
        assert_eq!(plain(&runs), "one line goes on");
        let runs = para("hard  \nbreak");
        assert_eq!(plain(&runs), "hard\nbreak");
        // Each run knows its line.
        assert_eq!(runs.last().unwrap().line, 1);
    }

    #[test]
    fn inline_marks() {
        let runs = para("**粗** *斜* `碼` ~~刪~~ ==亮== [連結](x) [[維基|頁]] <kbd>K</kbd> H<sub>2</sub>O");
        assert!(marks_of(&runs, "粗").strong);
        assert!(marks_of(&runs, "斜").emph);
        assert!(marks_of(&runs, "碼").code);
        assert!(marks_of(&runs, "刪").strike);
        assert!(marks_of(&runs, "亮").highlight);
        assert!(marks_of(&runs, "連結").link);
        assert!(marks_of(&runs, "頁").wiki);
        assert!(marks_of(&runs, "K").kbd);
        assert!(marks_of(&runs, "2").sub);
        assert_eq!(plain(&runs), "粗 斜 碼 刪 亮 連結 頁 K H2O");
    }

    #[test]
    fn emphasis_runs_across_a_line_break() {
        let runs = para("**one\ntwo**");
        assert!(marks_of(&runs, "one").strong && marks_of(&runs, "two").strong);
        assert_eq!(plain(&runs), "one two");
    }

    #[test]
    fn bare_urls_are_links() {
        let runs = para("見 https://example.org/a_(b)。後");
        assert!(marks_of(&runs, "https://example.org/a_(b)").link);
        assert!(!marks_of(&runs, "後").link);
    }

    #[test]
    fn formulas_images_ruby_and_notes() {
        let blocks = parse("面積 $\\pi r^2$ 與 ![圖](a.png) 和 <ruby>韋<rt>wéi</rt></ruby>[^n]\n\n[^n]: 注。\n");
        let BlockKind::Paragraph(runs) = &blocks[0].kind else { panic!() };
        assert_eq!(plain(runs), "面積 $\\pi r^2$ 與 [img 圖] 和 韋(wéi)[1]");
        assert!(runs.iter().any(|r| matches!(&r.kind, InlineKind::Image { src, .. } if src == "a.png")));
        let BlockKind::Footnotes(notes) = &blocks[1].kind else { panic!("{:?}", blocks[1]) };
        assert_eq!(notes.len(), 1);
        assert_eq!((notes[0].number, notes[0].lines), (1, (2, 2)));
    }

    #[test]
    fn footnotes_are_numbered_as_cited() {
        let blocks = parse("甲[^b]乙[^a]\n\n[^a]: A\n[^b]: B\n[^c]: never cited\n");
        let BlockKind::Footnotes(notes) = &blocks.last().unwrap().kind else { panic!() };
        let order: Vec<(&str, usize)> = notes.iter().map(|n| (n.name.as_str(), n.number)).collect();
        assert_eq!(order, [("b", 1), ("a", 2), ("c", 3)]);
    }

    #[test]
    fn display_maths_blocks() {
        let blocks = parse("$$\nx^2\n$$\n\n$$y$$\n\n文 $$z$$ 字\n");
        assert_eq!(blocks[0], Block { lines: (0, 2), kind: BlockKind::Maths("x^2".into()) });
        assert_eq!(blocks[1], Block { lines: (4, 4), kind: BlockKind::Maths("y".into()) });
        assert!(matches!(&blocks[2].kind, BlockKind::Paragraph(_)));
    }

    #[test]
    fn lists_nest_count_and_tick() {
        let blocks = parse("1. one\n2. two\n   - [x] done\n   - [ ] open\n3. three\n");
        assert_eq!(blocks.len(), 1);
        let BlockKind::List { ordered: true, start: 1, loose: false, items } = &blocks[0].kind else { panic!("{:?}", blocks[0]) };
        assert_eq!(items.len(), 3);
        assert_eq!(items[1].lines, (1, 3));
        let BlockKind::List { items: inner, .. } = &items[1].body[1].kind else { panic!("{:?}", items[1].body) };
        assert_eq!(inner[0].task, Some(true));
        assert_eq!(inner[1].task, Some(false));
        assert_eq!(inner[1].lines, (3, 3));
    }

    #[test]
    fn loose_lists() {
        let BlockKind::List { loose, .. } = &kinds("- a\n\n- b\n")[0] else { panic!() };
        assert!(*loose);
    }

    #[test]
    fn quotes_containers_and_alerts() {
        let blocks = parse("> 引\n> 文\n\n> [!WARNING] 小心\n> 內容\n\n::: tip 提示\n裏面\n\n- 列表\n  :::\n\n後面\n");
        assert!(matches!(&blocks[0].kind, BlockKind::Quote(b) if b.len() == 1));
        assert_eq!(blocks[0].lines, (0, 1));
        let BlockKind::Callout { kind: Callout::Warning, fenced: false, title: Some(t), body, .. } = &blocks[1].kind else { panic!("{:?}", blocks[1]) };
        assert_eq!(plain(t), "小心");
        assert_eq!(body[0].lines, (4, 4));
        // The closing `:::` may be indented (a 宇浩 document does it).
        let BlockKind::Callout { kind: Callout::Tip, fenced: true, body, .. } = &blocks[2].kind else { panic!("{:?}", blocks[2]) };
        assert_eq!(blocks[2].lines, (6, 10));
        assert_eq!(body.len(), 2);
        assert!(matches!(&blocks[3].kind, BlockKind::Paragraph(r) if plain(r) == "後面"));
    }

    #[test]
    fn tables() {
        let blocks = parse("| 城市 | 人口 |\n| :-- | --: |\n| 杭州 | 1252 |\n| 苏州 | 1291 |\n\nafter\n");
        let BlockKind::Table { align, head, rows } = &blocks[0].kind else { panic!() };
        assert_eq!(align, &[Align::Left, Align::Right]);
        assert_eq!(plain(&head[0]), "城市");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].0, 3);
        assert_eq!(plain(&rows[1].1[1]), "1291");
        assert_eq!(blocks[0].lines, (0, 3));
        // `<br>` in a cell breaks it.
        let blocks = parse("| a<br>b |\n| --- |\n| c |\n");
        let BlockKind::Table { head, .. } = &blocks[0].kind else { panic!() };
        assert_eq!(plain(&head[0]), "a\nb");
    }

    #[test]
    fn code_blocks() {
        let blocks = parse("```rust\nfn main() {}\n```\n\n    indented\n    code\n\ntext\n");
        assert_eq!(blocks[0], Block { lines: (0, 2), kind: BlockKind::Code { info: "rust".into(), lines: vec!["fn main() {}".into()] } });
        assert_eq!(blocks[1].lines, (4, 5));
        assert!(matches!(&blocks[1].kind, BlockKind::Code { lines, .. } if lines == &["indented", "code"]));
    }

    #[test]
    fn notes_metadata_and_html() {
        let text = "---\ntitle: 書\n---\n\n<style>\n.a {}\n</style>\n\n<!-- 註\n多行 -->\n可見 %%不見%% 字\n\n<div class=\"x\">\n\n裏\n\n</div>\n\n<CharPlot char='认' />\n\n[ref]: https://x\n";
        let blocks = parse(text);
        assert!(matches!(&blocks[0].kind, BlockKind::FrontMatter(l) if l == &["title: 書"]));
        assert!(matches!(&blocks[1].kind, BlockKind::Paragraph(r) if plain(r) == "可見  字"), "{:?}", blocks[1]);
        assert_eq!(blocks[1].lines, (10, 10));
        assert!(matches!(&blocks[2].kind, BlockKind::Paragraph(r) if plain(r) == "裏"));
        assert!(matches!(&blocks[3].kind, BlockKind::Html(h) if h.starts_with("<CharPlot")));
        assert_eq!(blocks.len(), 4);
    }

    /// **What a parse keeps changes nothing it reads**: a second parse, one
    /// after an edit, one after lines were put in above, all read as a parse
    /// from nothing does.
    #[test]
    fn the_kept_runs_read_as_new_ones() {
        let text = "# 題\n\n甲 **乙**\n丙\n\n- 一 `碼`\n- 二[^n]\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n[^n]: 注\n";
        let mut cache = Cache::default();
        assert_eq!(parse_with(text, &mut cache), parse(text));
        assert_eq!(parse_with(text, &mut cache), parse(text));
        let edited = text.replace("丙", "丙丁");
        assert_eq!(parse_with(&edited, &mut cache), parse(&edited));
        let moved = format!("\n\n{edited}");
        assert_eq!(parse_with(&moved, &mut cache), parse(&moved));
    }

    #[test]
    fn reference_links_are_links() {
        let text = "見 [文檔][docs]、[Docs][] 與 [docs]，圖 ![圖][pic]，`[docs]` 不是，[未定義][nope] 也不是。\n\n[docs]: https://example.org/docs\n[pic]: <a b.png> \"題\"\n";
        let runs = para(text);
        assert!(marks_of(&runs, "文檔").link);
        assert!(marks_of(&runs, "Docs").link);
        assert!(runs.iter().any(|r| matches!(&r.kind, InlineKind::Image { src, .. } if src == "a b.png")));
        assert!(marks_of(&runs, "[docs]").code, "inside code it stays as written");
        assert!(!marks_of(&runs, "未定義").link);
        assert!(plain(&runs).contains("[未定義][nope]"));
        // The shortcut form, and a definition that is not shown.
        let blocks = parse("[docs]\n\n[docs]: x\n");
        assert_eq!(blocks.len(), 1);
        let BlockKind::Paragraph(runs) = &blocks[0].kind else { panic!() };
        assert!(marks_of(runs, "docs").link);
    }

    #[test]
    fn rules_and_rebase() {
        let blocks = parse("a\n\n---\n\n- x\n  y\n");
        assert_eq!(blocks[1], Block { lines: (2, 2), kind: BlockKind::Rule });
        let moved = rebased(&blocks[2], 4);
        assert_eq!(moved.lines, (0, 1));
        let again = rebased(&parse("\n\n\n\n\n\n- x\n  y\n")[0], 6);
        assert_eq!(moved, again, "a block that only moved is the same block");
    }
}
