//! **`:hard-wrap`: re-filling lines to a width without changing what they
//! mean** (2026-10-30).
//!
//! Two fillers already live in [`crate::reflow`], and both are copies: vim's
//! `gq` and helix's `:reflow`, kept faithful to their originals. Measured on
//! whole files (nvim 0.12, `gqG`), vim's folds `fn f() {}` into the doc
//! comment above it, a Python docstring onto the statement after it, a Typst
//! heading into its paragraph; helix's knows nothing of structure at all. They
//! work because the writer selects exactly the lines that are prose. This one
//! does that selecting itself and **never touches a line outside a fill
//! unit**.
//!
//! What a fill unit is depends on the syntax:
//!
//! - **Text**: a paragraph — blank lines or a line opening with U+3000 (the
//!   two-square indent a Chinese paragraph starts with) begin one.
//! - **Markdown**: a paragraph, a quote's body, a list item's body, a
//!   footnote's body. Fences and what is in them, indented code, tables,
//!   headings, a setext underline and the line above it, rules, HTML blocks,
//!   front matter, reference definitions, `:::` fences and `$$` maths are
//!   left as they are. A line ending in two spaces or a `\` is a hard break:
//!   it ends a unit and keeps its ending byte for byte.
//! - **Typst**: a paragraph or an item's body; `=` headings, `#` code lines,
//!   raw blocks, display maths and block comments are left alone; `\` at the
//!   end of a line is a hard break.
//! - **Code**: only comments. A run of lines with the same indent and the same
//!   marker is one unit, the marker written again on every line; `///`,
//!   `//!` and `//` are three kinds, never merged. A block comment's rail
//!   (` * `) is a leader; its opening and closing lines are left alone. Every
//!   code line, including one with a comment after the code, is left alone.
//!   A Python or Mojo docstring — `"""` right after a `def`, a `class` or at
//!   the top of the file — is documentation, and its body is filled; every
//!   other string is never touched (decided 2026-10-30).
//!
//! A comment's or a quote's body is read as Markdown in turn, so a fenced
//! example inside a `///` is skipped and a list inside a `>` hangs.
//!
//! Width is counted in display cells. Chinese breaks between any two
//! characters, with kinsoku: no line starts with 。，、」） and none ends with
//! 「（; a word or a number is never split; the width is strict, so what does
//! not fit moves down. Joining two lines puts nothing between two wide
//! characters and a space wherever a letter is on either side — the rule `J`
//! follows (#326). A continuation line never begins with a token that would
//! open a block (`-`, `1.`, `#`, `>`, `|`; `=` in Typst). Width `0` joins
//! each unit to one line.

use crate::syntax::Syntax;
use yumete_cjk::{char_width, str_width};

/// What stands before a unit's words on its first line and on the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Leader {
    first: String,
    rest: String,
}

impl Leader {
    fn plain(indent: &str) -> Leader {
        Leader { first: indent.to_string(), rest: indent.to_string() }
    }

    /// A marker written again on every line.
    fn again(lead: &str) -> Leader {
        Leader { first: lead.to_string(), rest: lead.to_string() }
    }

    /// A marker on the first line; the rest hang under its text.
    fn once(lead: &str) -> Leader {
        let indent: String = lead.chars().take_while(|c| matches!(c, ' ' | '\t')).collect();
        let hang = format!("{indent}{}", " ".repeat(str_width(lead) - str_width(&indent)));
        Leader { first: lead.to_string(), rest: hang }
    }
}

/// One line, classified.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    /// Written back as it is.
    Keep(String),
    /// Blank: ends a unit, kept.
    Blank,
    /// Part of a unit: its leader, its words, and the hard break it ends in.
    Fill { leader: Leader, text: String, hard: Option<String>, joins: bool },
}

/// The lines filled to `width` under the rules of `syntax`; `0` is one line
/// per unit.
pub fn hard_wrap(lines: &[String], syntax: Syntax, width: usize) -> Vec<String> {
    let classified = classify(lines, syntax);
    let mut out = Vec::new();
    let mut unit: Vec<(Leader, String, Option<String>)> = Vec::new();
    let flush = |unit: &mut Vec<(Leader, String, Option<String>)>, out: &mut Vec<String>| {
        if unit.is_empty() {
            return;
        }
        let leader = unit[0].0.clone();
        let parts: Vec<(String, Option<String>)> = unit.drain(..).map(|(_, t, h)| (t, h)).collect();
        out.extend(fill(&leader, &parts, width, syntax));
    };
    for line in classified {
        match line {
            Line::Keep(text) => {
                flush(&mut unit, &mut out);
                out.push(text);
            }
            Line::Blank => {
                flush(&mut unit, &mut out);
                out.push(String::new());
            }
            Line::Fill { leader, text, hard, joins } => {
                let goes_on = joins && unit.last().is_some_and(|(l, _, _)| l.rest == leader.rest || l.rest == leader.first);
                if !goes_on {
                    flush(&mut unit, &mut out);
                }
                unit.push((leader, text, hard));
            }
        }
    }
    flush(&mut unit, &mut out);
    out
}

/// The lines of the unit `line` is in — what `:hard-wrap` takes with no
/// selection. `None` on a line nothing fills: blank, a heading, code.
pub fn unit_around(lines: &[String], syntax: Syntax, line: usize) -> Option<std::ops::Range<usize>> {
    let classified = classify(lines, syntax);
    let is_fill = |i: usize| matches!(classified.get(i), Some(Line::Fill { .. }));
    if !is_fill(line) {
        return None;
    }
    let continues = |i: usize| match (&classified[i - 1], &classified[i]) {
        (Line::Fill { leader: a, .. }, Line::Fill { leader: b, joins, .. }) => {
            *joins && (a.rest == b.rest || a.rest == b.first)
        }
        _ => false,
    };
    let mut from = line;
    while from > 0 && continues(from) {
        from -= 1;
    }
    let mut to = line + 1;
    while to < classified.len() && continues(to) {
        to += 1;
    }
    Some(from..to)
}

// ---- Classifying ----------------------------------------------------------

fn classify(lines: &[String], syntax: Syntax) -> Vec<Line> {
    match syntax {
        Syntax::Markdown => classify_markdown(lines),
        Syntax::Typst => classify_typst(lines),
        Syntax::Code(language) => classify_code(lines, language),
        Syntax::Text | Syntax::Diff => classify_text(lines),
    }
}

/// Paragraphs, as prose has them.
fn classify_text(lines: &[String]) -> Vec<Line> {
    lines
        .iter()
        .map(|line| match line.trim().is_empty() {
            true => Line::Blank,
            false => {
                let (indent, body) = split_indent(line);
                let (text, hard) = hard_break(body);
                Line::Fill {
                    leader: Leader { first: indent.to_string(), rest: indent_without_paragraph_mark(indent) },
                    text,
                    hard,
                    joins: !indent.contains('\u{3000}'),
                }
            }
        })
        .collect()
}

/// A U+3000 indent marks a paragraph; the lines under it carry none.
fn indent_without_paragraph_mark(indent: &str) -> String {
    match indent.contains('\u{3000}') {
        true => String::new(),
        false => indent.to_string(),
    }
}

fn split_indent(line: &str) -> (&str, &str) {
    let body = line.trim_start_matches([' ', '\t', '\u{3000}']);
    (&line[..line.len() - body.len()], body)
}

/// A Markdown hard break at the end of a line: two or more spaces, or a
/// backslash. The text without it, and the break itself to put back.
fn hard_break(body: &str) -> (String, Option<String>) {
    let trimmed = body.trim_end_matches([' ', '\t']);
    let trailing = &body[trimmed.len()..];
    if trailing.chars().filter(|&c| c == ' ').count() >= 2 {
        return (trimmed.to_string(), Some(trailing.to_string()));
    }
    if let Some(before) = trimmed.strip_suffix('\\') {
        if !before.ends_with('\\') {
            let kept = before.trim_end();
            return (kept.to_string(), Some(trimmed[kept.len()..].to_string()));
        }
    }
    (trimmed.to_string(), None)
}

/// A comment's or a quote's body, read as Markdown and put back behind its
/// leader: a fence inside a `///` is skipped, a list inside a `>` hangs.
fn nested(bodies: &[String], lead: &str) -> Vec<Line> {
    classify_markdown(bodies)
        .into_iter()
        .map(|line| match line {
            Line::Keep(text) if text.is_empty() => Line::Keep(lead.trim_end().to_string()),
            Line::Keep(text) => Line::Keep(format!("{lead}{text}")),
            Line::Blank => Line::Keep(lead.trim_end().to_string()),
            Line::Fill { leader, text, hard, joins } => Line::Fill {
                leader: Leader { first: format!("{lead}{}", leader.first), rest: format!("{lead}{}", leader.rest) },
                text,
                hard,
                joins,
            },
        })
        .collect()
}

fn classify_markdown(lines: &[String]) -> Vec<Line> {
    use crate::markdown::{Block, BlockScanner};
    let mut scanner = BlockScanner::new();
    let blocks: Vec<Block> = lines
        .iter()
        .map(|line| {
            let prefix: String = line.chars().take(crate::markdown::PREFIX).collect();
            scanner.feed(&prefix, line.chars().count())
        })
        .collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut html = false;
    let mut maths = false;
    let mut i = 0;
    let mut return_to: Vec<usize> = Vec::new();
    while i < lines.len() {
        if let Some(j) = return_to.pop() {
            i = j;
            if i >= lines.len() {
                break;
            }
        }
        let line = &lines[i];
        i += 1;
        let i = i - 1;
        let trimmed = line.trim();
        // Blocks the scanner does not name: HTML, display maths, a setext
        // underline with the heading above it, a reference definition.
        if html {
            out.push(Line::Keep(line.clone()));
            if trimmed.is_empty() {
                html = false;
            }
            continue;
        }
        if maths {
            out.push(Line::Keep(line.clone()));
            if trimmed.starts_with("$$") {
                maths = false;
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("$$") {
            maths = !rest.contains("$$");
            out.push(Line::Keep(line.clone()));
            continue;
        }
        if opens_html(trimmed) {
            html = true;
            out.push(Line::Keep(line.clone()));
            continue;
        }
        if is_setext_underline(line) && i > 0 && matches!(blocks[i - 1], Block::Prose) && !lines[i - 1].trim().is_empty() {
            if let Some(last) = out.last_mut() {
                *last = Line::Keep(lines[i - 1].clone());
            }
            out.push(Line::Keep(line.clone()));
            continue;
        }
        if is_reference_definition(trimmed) {
            out.push(Line::Keep(line.clone()));
            continue;
        }
        if trimmed.is_empty() {
            out.push(Line::Blank);
            continue;
        }
        match blocks[i] {
            Block::Prose => {
                let (indent, body) = split_indent(line);
                // Four spaces in is indented code — unless it hangs under a
                // list item or footnote, which the unit above says.
                let hangs = matches!(out.last(), Some(Line::Fill { leader, .. }) if str_width(indent) >= str_width(&leader.rest) && !leader.rest.is_empty());
                if indent.replace('\t', "    ").len() >= 4 && !hangs {
                    out.push(Line::Keep(line.clone()));
                    continue;
                }
                let (text, hard) = hard_break(body);
                let leader = match hangs {
                    true => match out.last() {
                        Some(Line::Fill { leader, .. }) => Leader::again(&leader.rest),
                        _ => Leader::plain(indent),
                    },
                    false => Leader { first: indent.to_string(), rest: indent_without_paragraph_mark(indent) },
                };
                out.push(Line::Fill { leader, text, hard, joins: !indent.contains('\u{3000}') });
            }
            Block::Quote { .. } => {
                // The run of lines under the same `>` marks, read as Markdown
                // of their own and put back behind the marks.
                let marks_of = |l: &str| -> (String, String) {
                    let (indent, body) = split_indent(l);
                    let text = body.trim_start_matches(['>', ' ']);
                    let marks = body[..body.len() - text.len()].trim_end();
                    (format!("{indent}{marks}"), text.to_string())
                };
                let (marks, _) = marks_of(line);
                let mut bodies = Vec::new();
                let mut j = i;
                while j < lines.len() && matches!(blocks[j], Block::Quote { .. }) && marks_of(&lines[j]).0 == marks {
                    bodies.push(marks_of(&lines[j]).1);
                    j += 1;
                }
                out.extend(nested(&bodies, &format!("{marks} ")));
                return_to.push(j);
            }
            Block::Item { .. } => match item_marker(line) {
                Some((lead, text)) => {
                    let (text, hard) = hard_break(text);
                    out.push(Line::Fill { leader: Leader::once(&lead), text, hard, joins: false });
                }
                None => out.push(Line::Keep(line.clone())),
            },
            Block::FootnoteDef => {
                let (indent, body) = split_indent(line);
                match body.find("]:") {
                    Some(close) if body.starts_with("[^") => {
                        let lead = format!("{indent}{} ", &body[..close + 2]);
                        let text = body[close + 2..].trim_start();
                        let (text, hard) = hard_break(text);
                        out.push(Line::Fill { leader: Leader { first: lead, rest: format!("{indent}    ") }, text, hard, joins: false });
                    }
                    _ => out.push(Line::Keep(line.clone())),
                }
            }
            Block::Heading(_)
            | Block::Rule
            | Block::Code { .. }
            | Block::FrontMatter
            | Block::Container(_)
            | Block::Table { .. }
            | Block::Comment { .. }
            | Block::Conflict(_) => out.push(Line::Keep(line.clone())),
        }
    }
    out
}

/// `- `, `* `, `+ `, `1. `, `1) `, with a task box: the marker with its
/// indent, and the text after it.
fn item_marker(line: &str) -> Option<(String, &str)> {
    let (indent, body) = split_indent(line);
    let ordered = body.find(['.', ')']).filter(|&n| n > 0 && n <= 9 && body[..n].chars().all(|c| c.is_ascii_digit()));
    let marker_len = match body.chars().next() {
        Some('-' | '*' | '+') => 1,
        _ => ordered.map(|n| n + 1)?,
    };
    let after = &body[marker_len..];
    if !after.starts_with([' ', '\t']) {
        return None;
    }
    let mut text = after.trim_start_matches([' ', '\t']);
    let mut marker = &body[..body.len() - text.len()];
    // `- [ ] ` and `- [x] `: the box is part of the marker.
    if let Some(rest) = text.strip_prefix("[ ] ").or_else(|| text.strip_prefix("[x] ")).or_else(|| text.strip_prefix("[X] ")) {
        marker = &body[..body.len() - rest.len()];
        text = rest;
    }
    Some((format!("{indent}{marker}"), text))
}

fn opens_html(trimmed: &str) -> bool {
    let mut chars = trimmed.chars();
    chars.next() == Some('<')
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!')
        && !trimmed.starts_with("<!--")
}

fn is_setext_underline(line: &str) -> bool {
    let t = line.trim();
    let indent = line.len() - line.trim_start().len();
    indent <= 3 && !t.is_empty() && (t.chars().all(|c| c == '=') || t.chars().all(|c| c == '-'))
}

fn is_reference_definition(trimmed: &str) -> bool {
    trimmed.starts_with('[') && !trimmed.starts_with("[^") && trimmed.contains("]:") && {
        let close = trimmed.find("]:").unwrap_or(0);
        !trimmed[1..close].contains(']')
    }
}

fn classify_typst(lines: &[String]) -> Vec<Line> {
    use crate::markdown::typst::BlockScanner;
    use crate::markdown::Block;
    let mut scanner = BlockScanner::new();
    let mut out = Vec::with_capacity(lines.len());
    let mut comment = false;
    let mut maths = false;
    for line in lines {
        let prefix: String = line.chars().take(crate::markdown::PREFIX).collect();
        let block = scanner.feed(&prefix, line.chars().count());
        let trimmed = line.trim();
        if comment {
            out.push(Line::Keep(line.clone()));
            if trimmed.contains("*/") {
                comment = false;
            }
            continue;
        }
        if maths {
            out.push(Line::Keep(line.clone()));
            if trimmed.ends_with('$') {
                maths = false;
            }
            continue;
        }
        if trimmed.starts_with("/*") {
            comment = !trimmed.contains("*/");
            out.push(Line::Keep(line.clone()));
            continue;
        }
        if trimmed.starts_with("$ ") || trimmed == "$" {
            maths = !(trimmed.len() > 1 && trimmed.ends_with('$'));
            out.push(Line::Keep(line.clone()));
            continue;
        }
        if trimmed.is_empty() {
            out.push(Line::Blank);
            continue;
        }
        let (indent, body) = split_indent(line);
        if let Some(text) = body.strip_prefix("//") {
            let text = text.trim_start();
            out.push(Line::Fill { leader: Leader::again(&format!("{indent}// ")), text: text.to_string(), hard: None, joins: true });
            continue;
        }
        match block {
            Block::Prose => {
                let (text, hard) = typst_break(body);
                out.push(Line::Fill { leader: Leader::plain(indent), text, hard, joins: true });
            }
            Block::Item { .. } => {
                let marker_len = if body.starts_with("/ ") { body.find(':').map(|n| n + 1).unwrap_or(1) } else { 1 };
                let text = body[marker_len..].trim_start();
                let lead = format!("{indent}{}", &body[..body.len() - text.len()]);
                let (text, hard) = typst_break(text);
                out.push(Line::Fill { leader: Leader::once(&lead), text, hard, joins: false });
            }
            _ => out.push(Line::Keep(line.clone())),
        }
    }
    out
}

/// Typst's hard break: a `\` at the end of the line.
fn typst_break(body: &str) -> (String, Option<String>) {
    let trimmed = body.trim_end();
    match trimmed.strip_suffix('\\') {
        Some(before) if !before.ends_with('\\') => {
            let kept = before.trim_end();
            (kept.to_string(), Some(trimmed[kept.len()..].to_string()))
        }
        _ => (trimmed.to_string(), None),
    }
}

fn classify_code(lines: &[String], language: crate::code::Language) -> Vec<Line> {
    use crate::code::Language;
    let marks = crate::comment::marks(Syntax::Code(language));
    let docstrings = matches!(language, Language::Python | Language::Mojo);
    let mut out = Vec::with_capacity(lines.len());
    let mut in_block: Option<(&str, String, Vec<String>)> = None;
    let mut in_docstring: Option<(String, String)> = None;
    let mut last_code: Option<String> = None;
    let mut seen_code = false;
    // A run of line comments with one leader, read as Markdown together.
    let mut run: Option<(String, Vec<String>)> = None;
    let flush_run = |run: &mut Option<(String, Vec<String>)>, out: &mut Vec<Line>| {
        if let Some((lead, bodies)) = run.take() {
            out.extend(nested(&bodies, &lead));
        }
    };
    for line in lines {
        let (indent, body) = split_indent(line);
        let trimmed = body.trim_end();
        if let Some((quote, base)) = in_docstring.clone() {
            if trimmed.contains(quote.as_str()) {
                in_docstring = None;
                out.push(Line::Keep(line.clone()));
                continue;
            }
            if trimmed.is_empty() {
                out.push(Line::Blank);
                continue;
            }
            // Deeper than the docstring's own indent: a unit of its own, as
            // Sphinx reads an indented block; a field (`:param x:`, `Args:`)
            // starts a unit too.
            let field = trimmed.starts_with(':') || trimmed.starts_with('@') || trimmed.ends_with(':');
            let (text, hard) = hard_break(body);
            out.push(Line::Fill { leader: Leader::plain(indent), text, hard, joins: !field && str_width(indent) == str_width(&base) });
            continue;
        }
        if let Some((close, lead, bodies)) = in_block.as_mut() {
            if trimmed.contains(*close) {
                let (lead, bodies) = (lead.clone(), std::mem::take(bodies));
                in_block = None;
                out.extend(nested(&bodies, &lead));
                out.push(Line::Keep(line.clone()));
                continue;
            }
            // The rail (` * `) comes off; the first body line says whether
            // there is one.
            let text = match trimmed.strip_prefix('*') {
                Some(rest) if !rest.starts_with('/') => rest.trim_start(),
                _ => trimmed,
            };
            if bodies.is_empty() && lead.is_empty() {
                *lead = match trimmed.starts_with('*') {
                    true => format!("{indent}* "),
                    false => indent.to_string(),
                };
            }
            bodies.push(text.to_string());
            continue;
        }
        if trimmed.is_empty() {
            flush_run(&mut run, &mut out);
            out.push(Line::Blank);
            continue;
        }
        // A block comment opening on a line of its own.
        if let Some((open, close)) = marks.block {
            if trimmed.starts_with(open) && !trimmed.starts_with("/*/") {
                let rest = &trimmed[open.len()..];
                flush_run(&mut run, &mut out);
                if rest.contains(close) {
                    out.push(Line::Keep(line.clone()));
                } else {
                    // The opening line stays; the body is read when the
                    // closing line is reached.
                    in_block = Some((close, String::new(), Vec::new()));
                    out.push(Line::Keep(line.clone()));
                }
                continue;
            }
        }
        // A line comment: the marker, doc variants first so they stay apart.
        if let Some(mark) = marks.line {
            let variants: Vec<String> = match mark {
                "//" => vec!["//!".into(), "///".into(), "//".into()],
                "#" => vec!["#'".into(), "#!".into(), "#".into()],
                m => vec![m.to_string()],
            };
            if let Some(variant) = variants.iter().find(|v| trimmed.starts_with(v.as_str())) {
                let after = &trimmed[variant.len()..];
                if variant == "#!" || !(after.is_empty() || after.starts_with([' ', '\t'])) {
                    out.push(Line::Keep(line.clone()));
                    continue;
                }
                let text = after.trim_start();
                let lead = format!("{indent}{variant} ");
                match run.as_mut() {
                    Some((l, bodies)) if *l == lead => bodies.push(text.to_string()),
                    _ => {
                        flush_run(&mut run, &mut out);
                        run = Some((lead, vec![text.to_string()]));
                    }
                }
                continue;
            }
        }
        flush_run(&mut run, &mut out);
        // A docstring: `"""` or `'''` opening right after a `def`/`class`
        // line, or as the first statement of the file.
        if docstrings {
            flush_run(&mut run, &mut out);
            let quote = ["\"\"\"", "'''"].into_iter().find(|q| trimmed.starts_with(q));
            if let Some(quote) = quote {
                let after_def = last_code.as_deref().is_some_and(|l| {
                    let l = l.trim_start();
                    l.ends_with(':') && (l.starts_with("def ") || l.starts_with("class ") || l.starts_with("async def ") || l.starts_with("fn ") || l.starts_with("struct ") || l.starts_with("trait "))
                });
                let rest = &trimmed[quote.len()..];
                if (after_def || !seen_code) && !rest.contains(quote) {
                    in_docstring = Some((quote.to_string(), indent.to_string()));
                    match rest.trim().is_empty() {
                        true => out.push(Line::Keep(line.clone())),
                        false => {
                            let (text, hard) = hard_break(rest.trim_start());
                            out.push(Line::Fill { leader: Leader { first: format!("{indent}{quote}"), rest: indent.to_string() }, text, hard, joins: true });
                        }
                    }
                    continue;
                }
            }
        }
        seen_code = true;
        last_code = Some(line.clone());
        out.push(Line::Keep(line.clone()));
    }
    flush_run(&mut run, &mut out);
    if let Some((_, lead, bodies)) = in_block {
        out.extend(nested(&bodies, &lead));
    }
    out
}

// ---- Filling ---------------------------------------------------------------

/// A piece that is never broken: a word, a wide character with the
/// punctuation that clings to it, a code span, a link.
#[derive(Debug, Clone)]
struct Atom {
    text: String,
    /// Whether a space stood before it in the source (on the same line).
    spaced: bool,
    /// Whether it is the first atom of a source line.
    line_start: bool,
}

/// One unit filled: `parts` are its lines' texts with the hard break each
/// ended in. Greedy, in cells.
fn fill(leader: &Leader, parts: &[(String, Option<String>)], width: usize, syntax: Syntax) -> Vec<String> {
    let width = match width {
        0 => usize::MAX,
        w => w,
    };
    let mut out: Vec<String> = Vec::new();
    let mut line = leader.first.clone();
    let mut has_word = false;
    let mut last_char: Option<char> = None;
    let mut first_line = true;
    let flush_line = |line: &mut String, out: &mut Vec<String>, leader: &Leader| {
        out.push(line.trim_end().to_string());
        *line = leader.rest.clone();
    };
    // Segments between hard breaks fill separately; the break is put back
    // on the segment's last line.
    let mut segment: Vec<Atom> = Vec::new();
    let mut segments: Vec<(Vec<Atom>, Option<String>)> = Vec::new();
    for (text, hard) in parts {
        let mut atoms = atoms_of(text);
        if let Some(first) = atoms.first_mut() {
            first.line_start = true;
        }
        segment.extend(atoms);
        if let Some(hard) = hard {
            segments.push((std::mem::take(&mut segment), Some(hard.clone())));
        }
    }
    if !segment.is_empty() || segments.is_empty() {
        segments.push((segment, None));
    }
    for (atoms, hard) in segments {
        let mut pending: Option<Atom> = None;
        let mut i = 0;
        while i < atoms.len() || pending.is_some() {
            let atom = match pending.take() {
                Some(a) => a,
                None => {
                    i += 1;
                    atoms[i - 1].clone()
                }
            };
            let first_char = atom.text.chars().next();
            let gap = match has_word {
                false => "",
                // Across a former line end: nothing between two wide
                // characters, a space otherwise. Within a line: as written.
                true if atom.line_start => match wide(last_char) && wide(first_char) {
                    true => "",
                    false => " ",
                },
                true => match atom.spaced {
                    true => " ",
                    false => "",
                },
            };
            let fits = !has_word || str_width(&line) + str_width(gap) + str_width(&atom.text) <= width;
            if fits {
                line.push_str(gap);
                line.push_str(&atom.text);
                has_word = true;
                last_char = atom.text.chars().last();
                continue;
            }
            // It goes to the next line — unless it would open a block there,
            // in which case the atom before it comes down with it.
            if opens_a_block(&atom.text, syntax) && line.trim_end().len() > leader_len(&line, leader, first_line) {
                let moved = take_last_atom(&mut line, leader, first_line);
                flush_line(&mut line, &mut out, leader);
                first_line = false;
                line.push_str(&moved.text);
                has_word = true;
                last_char = moved.text.chars().last();
                pending = Some(Atom { spaced: moved.spaced, line_start: false, ..atom });
                continue;
            }
            flush_line(&mut line, &mut out, leader);
            first_line = false;
            line.push_str(&atom.text);
            has_word = true;
            last_char = atom.text.chars().last();
        }
        if let Some(hard) = hard {
            // The break is kept byte for byte — trailing spaces included.
            out.push(format!("{}{hard}", line.trim_end()));
            line = leader.rest.clone();
            first_line = false;
            has_word = false;
            last_char = None;
        }
    }
    if has_word || out.is_empty() {
        out.push(line.trim_end().to_string());
    }
    out
}

fn wide(c: Option<char>) -> bool {
    c.is_some_and(|c| char_width(c) == 2)
}

fn leader_len(line: &str, leader: &Leader, first: bool) -> usize {
    let lead = match first {
        true => &leader.first,
        false => &leader.rest,
    };
    lead.len().min(line.len())
}

/// Pull the last atom off a line that is being broken before a block
/// opener. The line keeps its leader; the atom's spacing is kept.
fn take_last_atom(line: &mut String, leader: &Leader, first: bool) -> Atom {
    let lead = leader_len(line, leader, first);
    let body = line[lead..].to_string();
    let trimmed = body.trim_end();
    let cut = trimmed.rfind(' ').map(|n| n + 1).unwrap_or_else(|| {
        // No space: the last wide cluster.
        let mut chars: Vec<(usize, char)> = trimmed.char_indices().collect();
        let mut end = chars.len();
        while end > 0 && closing(chars[end - 1].1) {
            end -= 1;
        }
        let at = end.saturating_sub(1);
        chars.truncate(at + 1);
        chars.get(at).map_or(0, |&(b, _)| b)
    });
    let spaced = cut > 0 && trimmed[..cut].ends_with(' ');
    let atom = trimmed[cut..].to_string();
    line.truncate(lead + cut);
    Atom { text: atom, spaced, line_start: false }
}

/// A token a continuation line may not begin with, since there it would
/// open a block.
fn opens_a_block(text: &str, syntax: Syntax) -> bool {
    let bare = text.trim_end_matches([' ', '\t']);
    let ordered = bare.strip_suffix('.').or_else(|| bare.strip_suffix(')')).is_some_and(|n| !n.is_empty() && n.len() <= 9 && n.chars().all(|c| c.is_ascii_digit()));
    match syntax {
        Syntax::Typst => matches!(bare, "-" | "+" | "/") || bare.starts_with('=') && bare.chars().all(|c| c == '=') || bare.starts_with('#') && bare.chars().nth(1).is_some_and(|c| c.is_alphabetic()),
        Syntax::Code(_) => false,
        _ => matches!(bare, "-" | "*" | "+" | ">" | "|" | "```" | "~~~") || bare.chars().all(|c| c == '#') && !bare.is_empty() || ordered,
    }
}

/// Opening punctuation that clings to what follows.
fn opening(c: char) -> bool {
    matches!(c, '「' | '『' | '（' | '【' | '《' | '〈' | '“' | '‘' | '〔' | '［' | '｛')
}

/// Closing punctuation that clings to what precedes.
fn closing(c: char) -> bool {
    matches!(
        c,
        '。' | '，' | '、' | '；' | '：' | '？' | '！' | '」' | '』' | '）' | '】' | '》' | '〉' | '”' | '’' | '…' | '—' | '〕' | '］' | '｝' | '・' | '～' | 'ー'
    )
}

/// The atoms of one source line.
fn atoms_of(text: &str) -> Vec<Atom> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Atom> = Vec::new();
    let mut i = 0;
    let mut spaced = false;
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' || c == '\t' {
            spaced = true;
            i += 1;
            continue;
        }
        let end = match protected_end(&chars, i) {
            Some(end) => end,
            None if wide(Some(c)) || opening(c) => {
                // A wide character with any opening marks before it and the
                // closing marks after it — one cluster, so no line starts
                // with 。 or ends with 「.
                let mut end = i + 1;
                while end < chars.len() && opening(chars[end - 1]) && chars[end] != ' ' {
                    end += 1;
                }
                while end < chars.len() && closing(chars[end]) {
                    end += 1;
                }
                end
            }
            None => {
                // A word: up to a space or a wide character; a closing mark
                // right after it stays with it.
                let mut end = i + 1;
                while end < chars.len() && !matches!(chars[end], ' ' | '\t') && !wide(Some(chars[end])) && !opening(chars[end]) {
                    end += 1;
                }
                while end < chars.len() && closing(chars[end]) {
                    end += 1;
                }
                end
            }
        };
        out.push(Atom { text: chars[i..end].iter().collect(), spaced, line_start: false });
        spaced = false;
        i = end;
    }
    out
}

/// The end of a span that is never broken inside, starting at `at`:
/// `` `code` ``, `[[link]]`, `[text](url)`, `$maths$`.
fn protected_end(chars: &[char], at: usize) -> Option<usize> {
    let find = |from: usize, pat: &[char]| -> Option<usize> {
        (from..=chars.len().checked_sub(pat.len())?).find(|&i| chars[i..i + pat.len()] == *pat)
    };
    match chars[at] {
        '`' => {
            let run = chars[at..].iter().take_while(|&&c| c == '`').count();
            let fence: Vec<char> = vec!['`'; run];
            let close = find(at + run, &fence)?;
            let mut end = close + run;
            while end < chars.len() && closing(chars[end]) {
                end += 1;
            }
            Some(end)
        }
        '[' if chars.get(at + 1) == Some(&'[') => {
            let close = find(at + 2, &[']', ']'])?;
            let mut end = close + 2;
            while end < chars.len() && (closing(chars[end]) || matches!(chars[end], '.' | ',' | ';' | ':' | '!' | '?')) {
                end += 1;
            }
            Some(end)
        }
        '[' => {
            let close = find(at + 1, &[']'])?;
            if chars.get(close + 1) != Some(&'(') {
                return None;
            }
            let shut = find(close + 2, &[')'])?;
            let mut end = shut + 1;
            while end < chars.len() && (closing(chars[end]) || matches!(chars[end], '.' | ',' | ';' | ':' | '!' | '?')) {
                end += 1;
            }
            Some(end)
        }
        '$' if chars.get(at + 1).is_some_and(|c| !c.is_whitespace() && *c != '$') => {
            let close = find(at + 1, &['$'])?;
            Some(close + 1)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::Language;

    fn wrap(text: &str, syntax: Syntax, width: usize) -> String {
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        hard_wrap(&lines, syntax, width).join("\n")
    }

    #[test]
    fn prose_fills_and_a_paragraph_mark_starts_a_paragraph() {
        assert_eq!(
            wrap("one two three four five six\nseven eight\n\nnext", Syntax::Text, 12),
            "one two\nthree four\nfive six\nseven eight\n\nnext"
        );
        assert_eq!(wrap("\u{3000}甲乙\n\u{3000}丙丁", Syntax::Text, 20), "\u{3000}甲乙\n\u{3000}丙丁");
        assert_eq!(wrap("one two\nthree", Syntax::Text, 0), "one two three");
    }

    #[test]
    fn chinese_breaks_between_characters_with_kinsoku_and_joins_without_a_space() {
        let text = "這是一句很長的中文句子，寫到第二十個字就要折行了。";
        assert_eq!(
            wrap(text, Syntax::Text, 20),
            "這是一句很長的中文句\n子，寫到第二十個字就\n要折行了。",
            "，clings to 子 and comes down with it; every line is within 20 cells"
        );
        assert_eq!(wrap("甲乙\n丙丁", Syntax::Text, 12), "甲乙丙丁");
        assert_eq!(wrap("甲乙\nab 丙", Syntax::Text, 12), "甲乙 ab 丙");
        assert_eq!(wrap("他說「你好」。", Syntax::Text, 8), "他說「你\n好」。", "「 clings to 你, 。 to 」");
        assert_eq!(wrap("他說「好」。", Syntax::Text, 4), "他說\n「好」。", "「 never ends a line");
        assert_eq!(wrap("中文English混排 word", Syntax::Text, 10), "中文\nEnglish混\n排 word", "a word is never split");
    }

    #[test]
    fn markdown_keeps_its_structure() {
        let md = "# Title\n\njumps  \nover the lazy dog and keeps going on\n\n- item with words to wrap\n  more\n\n```\ncode [[x]] is long enough to wrap\n```\n\n| a | b |\n| - | - |\n\n> quoted text that is long enough to wrap\n\nSetext\n======\n\n[ref]: https://example.com/a/very/long/url\n\nfoot[^1]\n\n[^1]: a footnote that is long enough to wrap around\n";
        let out = wrap(md, Syntax::Markdown, 20);
        assert_eq!(
            out,
            "# Title\n\njumps  \nover the lazy dog\nand keeps going on\n\n- item with words to\n  wrap more\n\n```\ncode [[x]] is long enough to wrap\n```\n\n| a | b |\n| - | - |\n\n> quoted text that\n> is long enough to\n> wrap\n\nSetext\n======\n\n[ref]: https://example.com/a/very/long/url\n\nfoot[^1]\n\n[^1]: a footnote\n    that is long\n    enough to wrap\n    around"
        );
    }

    #[test]
    fn a_backslash_break_and_a_block_opener_are_respected() {
        assert_eq!(wrap("foo bar\\\nbaz qux quux", Syntax::Markdown, 12), "foo bar\\\nbaz qux quux");
        // `-` would open a list at the start of a line: `b` comes down with it.
        assert_eq!(wrap("aaaa bbb - ccc", Syntax::Markdown, 9), "aaaa\nbbb - ccc");
        assert_eq!(wrap("aaa bbb - ccc", Syntax::Markdown, 8), "aaa\nbbb -\nccc", "a `-` may end a line");
        // Protected spans stay whole.
        assert_eq!(wrap("see `a b c` and [[雨 夜]] and [t x](u)", Syntax::Markdown, 10), "see\n`a b c`\nand\n[[雨 夜]]\nand\n[t x](u)");
    }

    #[test]
    fn width_zero_joins_each_unit_but_not_across_a_hard_break() {
        assert_eq!(
            wrap("- item with words to wrap\n  more\n\njumps  \nover\nthe", Syntax::Markdown, 0),
            "- item with words to wrap more\n\njumps  \nover the"
        );
    }

    #[test]
    fn typst_keeps_headings_code_and_breaks() {
        let typ = "= Heading long enough\nA paragraph of Typst text that is long enough to wrap \\\nafter a forced break and more words follow here\n- an item long enough to wrap around the width here\n#set page(width: 10cm)\n// a comment that is long enough to wrap\n";
        assert_eq!(
            wrap(typ, Syntax::Typst, 30),
            "= Heading long enough\nA paragraph of Typst text that\nis long enough to wrap \\\nafter a forced break and more\nwords follow here\n- an item long enough to wrap\n  around the width here\n#set page(width: 10cm)\n// a comment that is long\n// enough to wrap"
        );
    }

    #[test]
    fn code_fills_only_its_comments() {
        let rs = "/// A doc comment that goes on long enough to wrap twice over\n/// second line of it\n///\n/// ```\n/// let x = 1; // example code that is long enough to wrap\n/// ```\nfn f() {}\n// trailing comment after code that is quite long indeed\nlet x = 1; // a trailing comment that is long enough to wrap\n//! inner doc that is long enough to wrap around\n";
        assert_eq!(
            wrap(rs, Syntax::Code(Language::Rust), 30),
            "/// A doc comment that goes on\n/// long enough to wrap twice\n/// over second line of it\n///\n/// ```\n/// let x = 1; // example code that is long enough to wrap\n/// ```\nfn f() {}\n// trailing comment after code\n// that is quite long indeed\nlet x = 1; // a trailing comment that is long enough to wrap\n//! inner doc that is long\n//! enough to wrap around"
        );
        let c = "/*\n * a block comment that is long enough to wrap around\n * and a second line\n */\nint x; /* trailing block comment long enough */\n";
        assert_eq!(
            wrap(c, Syntax::Code(Language::C), 30),
            "/*\n * a block comment that is\n * long enough to wrap around\n * and a second line\n */\nint x; /* trailing block comment long enough */"
        );
    }

    #[test]
    fn a_python_docstring_fills_and_other_strings_do_not() {
        let py = "def f(x):\n    \"\"\"A docstring that is long enough to need wrapping at thirty.\n\n    Args:\n        x: the argument, described at some length here\n    \"\"\"\n    s = \"\"\"a string that is long enough to wrap but is data\n    and must stay\"\"\"\n    return s\n# a python comment long enough to wrap around the width\n";
        assert_eq!(
            wrap(py, Syntax::Code(Language::Python), 30),
            "def f(x):\n    \"\"\"A docstring that is\n    long enough to need\n    wrapping at thirty.\n\n    Args:\n        x: the argument,\n        described at some\n        length here\n    \"\"\"\n    s = \"\"\"a string that is long enough to wrap but is data\n    and must stay\"\"\"\n    return s\n# a python comment long enough\n# to wrap around the width"
        );
    }

    #[test]
    fn the_unit_under_a_line_is_found_and_a_heading_has_none() {
        let lines: Vec<String> = "# T\n\na b\nc d\n\n- i\n  j\n```\nx\n```".lines().map(str::to_string).collect();
        assert_eq!(unit_around(&lines, Syntax::Markdown, 3), Some(2..4));
        assert_eq!(unit_around(&lines, Syntax::Markdown, 6), Some(5..7));
        assert_eq!(unit_around(&lines, Syntax::Markdown, 0), None);
        assert_eq!(unit_around(&lines, Syntax::Markdown, 8), None);
    }
}
