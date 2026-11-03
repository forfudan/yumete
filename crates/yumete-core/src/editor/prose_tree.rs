//! **helix's tree keys in prose** (2026-11-02, decided after a writer's and
//! a helix user's review): prose has no syntax tree, so it gets a ladder of
//! its own units instead — clause, sentence, list item, block (a paragraph,
//! a list, a quote: lines between blank lines), section (a heading and what
//! is under it), file.
//!
//! **Up from a bare cursor the first rung is the block**, as one `A-o` on a
//! word is the paragraph in helix's Markdown; from a selection that already
//! is a clause or a sentence (`mi u`, `mi s`) it is the rung above. **Down,
//! a block's children are its sentences** (or its list items), a sentence's
//! its clauses: `x A-I` is every sentence in the paragraph.
//!
//! The writer's two follow-ups (2026-11-03): **a clause takes the mark that
//! ends it** when that is a comma, a 、, a semicolon or a colon, so the
//! clauses of a sentence lie end to end and only its 。 is left over; and
//! **a quotation is a rung of its own**, 「…」 『…』 “…” with their marks,
//! between the clauses in it and the sentence around it (or around the
//! sentences in it). `A-n` `A-p` in select mode stay helix's.

use super::tree_select::TreeStep;
use super::*;
use crate::selection::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Clause,
    Quote,
    Sentence,
    Item,
    Block,
    Section,
    File,
}

/// One unit: what it is, and its characters, half-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Node {
    kind: Kind,
    from: usize,
    to: usize,
}

impl Node {
    fn holds(&self, from: usize, to: usize) -> bool {
        self.from <= from && to <= self.to
    }
}

/// A span a `mi` object gave, half-open.
fn half_open(span: motion::Span) -> Option<(usize, usize)> {
    match span {
        motion::Span::Over { anchor, head } => Some((anchor.min(head), anchor.max(head) + 1)),
        _ => None,
    }
}

/// The marks a clause takes with it: those that join clauses, not those
/// that end a sentence or open and close a quotation.
fn joins_clauses(c: char) -> bool {
    matches!(c, '，' | '、' | '；' | '：' | ',' | ';' | ':' | '﹐' | '﹑' | '﹔' | '﹕')
}

/// The quotation marks, opening and closing.
const QUOTES: [(char, char); 3] = [('「', '」'), ('『', '』'), ('“', '”')];

/// A list item's first line: `- `, `* `, `+ `, `1. ` or `1) `, after its indent.
fn list_marker(line: &str) -> Option<usize> {
    let body = line.trim_start();
    let indent = line.len() - body.len();
    let digits = body.chars().take_while(|c| c.is_ascii_digit()).count();
    let marked = match digits {
        0 => body.starts_with("- ") || body.starts_with("* ") || body.starts_with("+ "),
        n => body[n..].starts_with(". ") || body[n..].starts_with(") "),
    };
    marked.then_some(indent)
}

impl Editor {
    /// Whether the buffer is prose: the ladder's ground.
    pub(super) fn prose_here(&self) -> bool {
        !matches!(self.current_buffer().syntax(), crate::syntax::Syntax::Code(_))
    }

    pub(super) fn lists_here(&self) -> bool {
        matches!(self.current_buffer().syntax(), crate::syntax::Syntax::Markdown | crate::syntax::Syntax::Typst)
    }

    /// The sections, as (heading line, depth, first char, end char).
    pub(super) fn prose_sections(&self) -> Vec<(usize, usize, usize, usize)> {
        let rope = self.current_buffer().rope();
        let heads = self.outline();
        heads
            .iter()
            .enumerate()
            .map(|(i, &(line, depth, _))| {
                let end_line = heads[i + 1..].iter().find(|&&(_, d, _)| d <= depth).map(|&(l, _, _)| l);
                let end = end_line.map_or(rope.len_chars(), |l| rope.line_to_char(l));
                (line, depth, rope.line_to_char(line), end)
            })
            .collect()
    }

    /// The list item around `at` within the block `from..to`, if the block
    /// is a list.
    fn list_item_at(&self, at: usize, from: usize, to: usize) -> Option<Node> {
        if !self.lists_here() {
            return None;
        }
        let rope = self.current_buffer().rope();
        let (first, last) = (rope.char_to_line(from), rope.char_to_line(to.saturating_sub(1).max(from)));
        let here = rope.char_to_line(at);
        let marker = |l: usize| list_marker(&rope.line(l).to_string());
        let start = (first..=here).rev().find(|&l| marker(l).is_some())?;
        let indent = marker(start)?;
        let end = (start + 1..=last).find(|&l| marker(l).is_some_and(|i| i <= indent));
        let to = end.map_or(to, |l| rope.line_to_char(l));
        Some(Node { kind: Kind::Item, from: rope.line_to_char(start), to })
    }

    /// **The ladder at `at`**, smallest first, each rung holding the last.
    fn prose_ladder(&self, at: usize) -> Vec<Node> {
        let rope = self.current_buffer().rope();
        let at = at.min(rope.len_chars().saturating_sub(1));
        let mut found: Vec<Node> = Vec::new();
        let mut push = |node: Node| {
            if node.to > node.from && node.from <= at && at < node.to.max(node.from + 1) {
                found.push(node);
            }
        };
        if let Some((from, to)) = self.ladder_clause(at) {
            push(Node { kind: Kind::Clause, from, to });
        }
        for quote in self.quotes_around(at) {
            push(quote);
        }
        if let Some((from, to)) = half_open(self.sentence_span_at(at, false)) {
            let (from, to) = self.without_stray_quotes(from, to);
            push(Node { kind: Kind::Sentence, from, to });
        }
        let (from, to) = self.paragraph_at(at);
        if let Some(item) = self.list_item_at(at, from, to) {
            push(item);
        }
        push(Node { kind: Kind::Block, from, to });
        let mut sections: Vec<(usize, usize, usize, usize)> =
            self.prose_sections().into_iter().filter(|&(_, _, from, to)| from <= at && at < to).collect();
        sections.sort_by_key(|&(_, depth, ..)| std::cmp::Reverse(depth));
        for (_, _, from, to) in sections {
            push(Node { kind: Kind::Section, from, to });
        }
        push(Node { kind: Kind::File, from: 0, to: rope.len_chars() });
        // Smallest first, each rung holding the last: a quotation may be
        // smaller than its sentence or hold several, and a sentence that
        // runs across a quotation's end holds neither way and is left out.
        found.sort_by_key(|n| (n.to - n.from, n.kind));
        let mut out: Vec<Node> = Vec::new();
        for node in found {
            match out.last() {
                Some(last) if !node.holds(last.from, last.to) => {}
                _ => out.push(node),
            }
        }
        out
    }

    /// **The clause at `at`, with the comma or colon that ends it**
    /// (`mi u`'s span, which leaves the mark out, and the joining marks
    /// after it).
    fn ladder_clause(&self, at: usize) -> Option<(usize, usize)> {
        let (from, mut to) = half_open(self.clause_span_at(at, false))?;
        let rope = self.current_buffer().rope();
        while to < rope.len_chars() && joins_clauses(rope.char(to)) {
            to += 1;
        }
        Some((from, to))
    }

    /// **The quotations around `at`** within its paragraph, innermost
    /// first, each from its opening mark through its closing one.
    fn quotes_around(&self, at: usize) -> Vec<Node> {
        let rope = self.current_buffer().rope();
        let (start, stop) = self.paragraph_at(at);
        let mut out = Vec::new();
        for (open, close) in QUOTES {
            // Opening marks before `at` not yet closed, innermost last.
            let mut opened: Vec<usize> = Vec::new();
            for i in start..at.min(stop) {
                match rope.char(i) {
                    c if c == open => opened.push(i),
                    c if c == close => {
                        opened.pop();
                    }
                    _ => {}
                }
            }
            if rope.char(at.min(rope.len_chars().saturating_sub(1))) == open && at < stop {
                opened.push(at);
            }
            // Close them from `at` on, innermost first.
            let mut depth = 0usize;
            let from_here = match opened.last() == Some(&at) {
                true => at + 1,
                false => at,
            };
            for i in from_here..stop {
                let Some(&first) = opened.last() else { break };
                match rope.char(i) {
                    c if c == open => depth += 1,
                    c if c == close && depth > 0 => depth -= 1,
                    c if c == close => {
                        out.push(Node { kind: Kind::Quote, from: first, to: i + 1 });
                        opened.pop();
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// `from..to` less a quotation mark at either end whose other half is
    /// outside it: the first sentence of 「走吧。天黑了。」 is 走吧。, as
    /// the second is 天黑了。.
    fn without_stray_quotes(&self, mut from: usize, mut to: usize) -> (usize, usize) {
        let rope = self.current_buffer().rope();
        let closed_within = |open: char, close: char, from: usize, to: usize| {
            let mut depth = 0usize;
            for i in from..to {
                match rope.char(i) {
                    c if c == open => depth += 1,
                    c if c == close => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            false
        };
        let opened_within = |open: char, close: char, from: usize, to: usize| {
            let mut depth = 0usize;
            for i in (from..to).rev() {
                match rope.char(i) {
                    c if c == close => depth += 1,
                    c if c == open => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            false
        };
        while from < to {
            match QUOTES.iter().find(|(open, _)| *open == rope.char(from)) {
                Some(&(open, close)) if !closed_within(open, close, from, to) => from += 1,
                _ => break,
            }
        }
        while to > from {
            match QUOTES.iter().find(|(_, close)| *close == rope.char(to - 1)) {
                Some(&(open, close)) if !opened_within(open, close, from, to) => to -= 1,
                _ => break,
            }
        }
        (from, to)
    }

    /// The quotations wholly inside `from..to`, outermost only.
    fn quotes_in(&self, from: usize, to: usize) -> Vec<Node> {
        let rope = self.current_buffer().rope();
        let mut out: Vec<Node> = Vec::new();
        for (open, close) in QUOTES {
            let mut opened: Vec<usize> = Vec::new();
            for i in from..to {
                match rope.char(i) {
                    c if c == open => opened.push(i),
                    c if c == close => {
                        if let Some(first) = opened.pop() {
                            if opened.is_empty() {
                                out.push(Node { kind: Kind::Quote, from: first, to: i + 1 });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // A quotation inside another of a different kind is not outermost.
        let all = out.clone();
        out.retain(|q| !all.iter().any(|o| o != q && o.holds(q.from, q.to)));
        out.sort_by_key(|q| q.from);
        out
    }

    /// The smallest unit holding `from..to`.
    fn prose_node(&self, from: usize, to: usize) -> Option<Node> {
        self.prose_ladder(from).into_iter().find(|n| n.holds(from, to))
    }

    /// The smallest unit holding `node` and larger than it.
    fn prose_parent(&self, node: Node) -> Option<Node> {
        self.prose_ladder(node.from)
            .into_iter()
            .find(|n| n.holds(node.from, node.to) && (n.from, n.to) != (node.from, node.to))
    }

    /// The runs a `mi` object finds from `from` to `to`, one after another.
    fn runs_in(&self, from: usize, to: usize, kind: Kind) -> Vec<Node> {
        let rope = self.current_buffer().rope();
        let mut out = Vec::new();
        let mut at = from;
        while at < to {
            if rope.char(at).is_whitespace() {
                at += 1;
                continue;
            }
            let span = match kind {
                Kind::Clause => self.ladder_clause(at),
                _ => half_open(self.sentence_span_at(at, false)).map(|(a, b)| self.without_stray_quotes(a, b)),
            };
            match span {
                Some((a, b)) if b > at => {
                    out.push(Node { kind, from: a.max(from), to: b.min(to) });
                    at = b;
                }
                _ => at += 1,
            }
        }
        out
    }

    /// `runs` with each quotation wholly inside `from..to` standing for the
    /// runs within it, as one child.
    fn with_quotes(&self, runs: Vec<Node>, from: usize, to: usize) -> Vec<Node> {
        let quotes = self.quotes_in(from, to);
        if quotes.is_empty() {
            return runs;
        }
        let mut out: Vec<Node> = runs.into_iter().filter(|r| !quotes.iter().any(|q| r.from < q.to && q.from < r.to)).collect();
        out.extend(quotes);
        out.sort_by_key(|n| n.from);
        out
    }

    /// **A unit's children**: a file's or a section's blocks and sections
    /// (a heading line is a block of its own), a list's items, a block's or
    /// an item's sentences, a sentence's clauses.
    fn prose_children(&self, node: Node) -> Vec<Node> {
        let rope = self.current_buffer().rope();
        match node.kind {
            Kind::Clause => Vec::new(),
            // A quotation's sentences, or its clauses when it is one.
            Kind::Quote => {
                let (from, to) = (node.from + 1, node.to.saturating_sub(1).max(node.from + 1));
                let sentences = self.with_quotes(self.runs_in(from, to, Kind::Sentence), from, to);
                match sentences.len() > 1 {
                    true => sentences,
                    false => self.with_quotes(self.runs_in(from, to, Kind::Clause), from, to),
                }
            }
            Kind::Sentence => self.with_quotes(self.runs_in(node.from, node.to, Kind::Clause), node.from, node.to),
            Kind::Item => self.runs_in(node.from, node.to, Kind::Sentence),
            Kind::Block => {
                let mut items = Vec::new();
                let mut at = node.from;
                while let Some(item) = self.list_item_at(at, node.from, node.to) {
                    if item.to <= at {
                        break;
                    }
                    items.push(item);
                    at = item.to;
                    if at >= node.to {
                        break;
                    }
                }
                match items.len() > 1 || items.first().is_some_and(|i| (i.from, i.to) == (node.from, node.to)) {
                    true => items,
                    false => self.runs_in(node.from, node.to, Kind::Sentence),
                }
            }
            Kind::Section | Kind::File => {
                let sections = self.prose_sections();
                let mut out = Vec::new();
                let mut line = rope.char_to_line(node.from);
                // A section's own heading is its first child, a line alone.
                if node.kind == Kind::Section {
                    let to = rope.line_to_char((line + 1).min(rope.len_lines()));
                    out.push(Node { kind: Kind::Block, from: node.from, to: to.min(node.to) });
                    line += 1;
                }
                while line < rope.len_lines() && rope.line_to_char(line) < node.to {
                    let at = rope.line_to_char(line);
                    if let Some(&(_, _, from, to)) = sections.iter().find(|s| s.0 == line) {
                        out.push(Node { kind: Kind::Section, from, to: to.min(node.to) });
                        line = rope.char_to_line(to.min(rope.len_chars())).max(line + 1);
                        if to >= rope.len_chars() {
                            break;
                        }
                        continue;
                    }
                    if rope.line(line).to_string().trim().is_empty() {
                        line += 1;
                        continue;
                    }
                    let (from, to) = self.paragraph_at(at);
                    // A paragraph runs into the next heading only when no blank
                    // line comes between: stop it there.
                    let stop = sections.iter().map(|s| s.2).filter(|&s| s > from && s < to).min().unwrap_or(to);
                    out.push(Node { kind: Kind::Block, from, to: stop.min(node.to) });
                    let next = rope.char_to_line(stop.min(rope.len_chars()));
                    line = next.max(line + 1);
                    if stop >= rope.len_chars() {
                        break;
                    }
                }
                out
            }
        }
    }

    /// One tree key on one selection in prose: the new spans, half-open.
    fn prose_step(&self, step: TreeStep, from: usize, to: usize, caret: bool) -> Vec<(usize, usize)> {
        let Some(node) = self.prose_node(from, to) else { return Vec::new() };
        let span = |n: Node| (n.from, n.to);
        match step {
            TreeStep::Expand => self
                .prose_ladder(from)
                .into_iter()
                .filter(|n| !caret || n.kind >= Kind::Item)
                // `mi u` and then `A-o` is the sentence, not the same clause
                // with its comma.
                .filter(|n| {
                    let rope = self.current_buffer().rope();
                    n.kind != Kind::Clause || n.from != from || !(to..n.to).all(|i| joins_clauses(rope.char(i)))
                })
                .find(|n| n.holds(from, to) && (n.from, n.to) != (from, to))
                .map(span)
                .into_iter()
                .collect(),
            TreeStep::Shrink => self.prose_children(node).first().copied().map(span).into_iter().collect(),
            TreeStep::AllChildren => self.prose_children(node).into_iter().map(span).collect(),
            TreeStep::NextSibling | TreeStep::PrevSibling => {
                let mut node = node;
                while let Some(parent) = self.prose_parent(node) {
                    let siblings = self.prose_children(parent);
                    let at = siblings.iter().position(|s| (s.from, s.to) == (node.from, node.to));
                    let next = match (step, at) {
                        (TreeStep::NextSibling, Some(i)) => siblings.get(i + 1),
                        (_, Some(i)) => i.checked_sub(1).and_then(|i| siblings.get(i)),
                        // Not one of its parent's children exactly (a clause of a
                        // list item): the nearest one past it.
                        (TreeStep::NextSibling, None) => siblings.iter().find(|s| s.from >= node.to),
                        (_, None) => siblings.iter().rev().find(|s| s.to <= node.from),
                    };
                    if let Some(next) = next {
                        return vec![span(*next)];
                    }
                    node = parent;
                }
                Vec::new()
            }
            TreeStep::AllSiblings => {
                let mut node = node;
                while let Some(parent) = self.prose_parent(node) {
                    let siblings = self.prose_children(parent);
                    if siblings.len() > 1 {
                        return siblings.into_iter().map(span).collect();
                    }
                    node = parent;
                }
                Vec::new()
            }
        }
    }

    /// The tree keys' answer in prose for every selection, as
    /// `tree_select` takes it from the tree.
    pub(super) fn prose_select(&self, step: TreeStep) -> Vec<(usize, Vec<(usize, usize)>)> {
        self.sel
            .iter()
            .enumerate()
            .map(|(i, one)| {
                let (from, to) = self.drawn(*one);
                (i, self.prose_step(step, from, to, one.is_caret()))
            })
            .collect()
    }

    /// `A-e` / `A-b` in prose: the end of the unit around the selection, or
    /// its start, then its parent's.
    pub(super) fn prose_bound(&self, one: &Range, forward: bool) -> Option<usize> {
        let (from, to) = self.drawn(*one);
        let node = self.prose_node(from, to)?;
        if forward {
            return Some(node.to);
        }
        if node.from != one.head {
            return Some(node.from);
        }
        // Already at its start: the first unit around it that starts earlier.
        let mut up = node;
        while let Some(parent) = self.prose_parent(up) {
            if parent.from < node.from {
                return Some(parent.from);
            }
            up = parent;
        }
        Some(node.from)
    }
}
