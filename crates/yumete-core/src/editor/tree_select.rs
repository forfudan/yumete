//! **helix's syntax-tree keys** (2026-11-02): `A-o` `A-i` grow and shrink
//! the selection by the tree, `A-n` `A-p` step to the next and previous
//! node, `A-a` selects every sibling, `A-I` every child, and `A-e` `A-b` go
//! to the end and the start of the node around the cursor. Each follows
//! helix's `object.rs` and `movement.rs`, on the tree of a code file or of
//! the fenced block the caret stands in.

use super::fences::TreeHere;
use super::*;
use crate::selection::Range;
use tree_sitter::TreeCursor;

/// What a key does to one selection's node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TreeStep {
    Expand,
    Shrink,
    NextSibling,
    PrevSibling,
    AllSiblings,
    AllChildren,
}

/// helix's `reset_to_byte_range`: the deepest node holding the span.
fn cursor_at(tree: &tree_sitter::Tree, start: usize, end: usize) -> TreeCursor<'_> {
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if start < node.start_byte() || end > node.end_byte() {
            cursor.goto_parent();
            break;
        }
        if cursor.goto_first_child_for_byte(start).is_none() {
            break;
        }
    }
    cursor
}

/// The named children of the cursor's node, as byte spans.
fn named_children(cursor: &mut TreeCursor) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if cursor.goto_first_child() {
        loop {
            let node = cursor.node();
            if node.is_named() {
                out.push((node.start_byte(), node.end_byte()));
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
    out
}

/// helix's `find_parent_start`: the first named ancestor that starts
/// before this node does.
fn parent_start(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let start = node.start_byte();
    let mut node = node;
    while node.start_byte() >= start || !node.is_named() {
        node = node.parent()?;
    }
    Some(node)
}

impl Editor {
    /// One of the selection keys: each selection's node, by `step`. Prose
    /// has no tree and walks its own ladder (`prose_tree.rs`).
    pub(super) fn tree_select(&mut self, step: TreeStep) {
        let here = self.tree_here();
        if here.is_none() && !self.prose_here() {
            self.status = say!("code.no-tree-here");
            return;
        }
        // `A-i` first takes back what `A-o` grew, as helix's does, while the
        // selections still hold the ones they grew from.
        if step == TreeStep::Shrink {
            if let Some(before) = self.grown_from.pop() {
                let now: Vec<(usize, usize)> = self.sel.iter().map(|one| self.drawn(*one)).collect();
                let inside = before.iter().all(|one| {
                    let (from, to) = self.drawn(*one);
                    now.iter().any(|&(a, b)| a <= from && to <= b)
                });
                if inside {
                    let primary = self.sel.primary_index().min(before.len().saturating_sub(1));
                    self.sel.rebuild(before, primary);
                    return;
                }
                self.grown_from.clear();
            }
        }
        let answers = match &here {
            Some(here) => self.tree_spans(here, step),
            None => self.prose_select(step),
        };
        let was: Vec<Range> = self.sel.iter().copied().collect();
        let primary = self.sel.primary_index();
        let mut ranges: Vec<Range> = Vec::new();
        let mut new_primary = 0;
        for (i, spans) in answers {
            let one = was[i];
            if i == primary {
                new_primary = ranges.len();
            }
            if spans.is_empty() {
                ranges.push(one);
                continue;
            }
            let backward = one.head < one.anchor;
            for (from, to) in spans {
                let mut range = self.from_drawn(from, to.max(from + 1));
                if backward {
                    std::mem::swap(&mut range.anchor, &mut range.head);
                }
                ranges.push(range);
            }
        }
        if ranges == was {
            return;
        }
        if step == TreeStep::Expand {
            self.grown_from.push(was);
        }
        self.sel.rebuild(ranges, new_primary);
    }

    /// Each selection's answer on a syntax tree, in the buffer's characters.
    fn tree_spans(&self, here: &TreeHere, step: TreeStep) -> Vec<(usize, Vec<(usize, usize)>)> {
        let mut out = Vec::new();
        for (i, one) in self.sel.iter().enumerate() {
            let (from, to) = self.drawn(*one);
            let (start, end) = (here.byte_at(from), here.byte_at(to));
            let mut cursor = cursor_at(&here.tree, start, end);
            let spans: Vec<(usize, usize)> = match step {
                TreeStep::Expand => {
                    while (cursor.node().start_byte(), cursor.node().end_byte()) == (start, end) {
                        if !cursor.goto_parent() {
                            break;
                        }
                    }
                    vec![(cursor.node().start_byte(), cursor.node().end_byte())]
                }
                TreeStep::Shrink => {
                    cursor.goto_first_child();
                    vec![(cursor.node().start_byte(), cursor.node().end_byte())]
                }
                TreeStep::NextSibling | TreeStep::PrevSibling => {
                    let forward = step == TreeStep::NextSibling;
                    loop {
                        let moved = match forward {
                            true => cursor.goto_next_sibling(),
                            false => cursor.goto_previous_sibling(),
                        };
                        if moved || !cursor.goto_parent() {
                            break;
                        }
                    }
                    vec![(cursor.node().start_byte(), cursor.node().end_byte())]
                }
                TreeStep::AllSiblings => {
                    let mut found = false;
                    while cursor.goto_parent() {
                        if cursor.node().child_count() > 1 {
                            found = true;
                            break;
                        }
                    }
                    match found {
                        true => named_children(&mut cursor),
                        false => Vec::new(),
                    }
                }
                TreeStep::AllChildren => named_children(&mut cursor),
            };
            let spans = spans.into_iter().map(|(a, b)| (here.char_at(a), here.char_at(b))).collect();
            out.push((i, spans));
        }
        out
    }

    /// **`A-e` / `A-b`** (helix's `move_parent_node_end` / `_start`): to
    /// the end of the node around the cursor, or its start, then its
    /// parent's; while extending, the anchor stays.
    pub(super) fn tree_node_bound(&mut self, forward: bool) {
        let Some(here) = self.tree_here() else {
            if !self.prose_here() {
                self.status = say!("code.no-tree-here");
                return;
            }
            let extend = self.extend;
            let primary = self.sel.primary_index();
            let ranges: Vec<Range> = self
                .sel
                .iter()
                .map(|one| match self.prose_bound(one, forward) {
                    Some(head) => {
                        let head = head.min(self.current_buffer().rope().len_chars().saturating_sub(1));
                        match extend {
                            true => Range { head, ..*one },
                            false => Range { anchor: head, head, ..*one },
                        }
                    }
                    None => *one,
                })
                .collect();
            self.sel.rebuild(ranges, primary);
            return;
        };
        let rope = self.current_buffer().rope().clone();
        let extend = self.extend;
        let primary = self.sel.primary_index();
        let ranges: Vec<Range> = self
            .sel
            .iter()
            .map(|one| {
                let (from, to) = self.drawn(*one);
                let (start, end) = (here.byte_at(from), here.byte_at(to));
                let Some(mut node) = here.tree.root_node().named_descendant_for_byte_range(start, end) else {
                    return *one;
                };
                let head = match forward {
                    true => here.char_at(node.end_byte()),
                    false => {
                        let at = here.char_at(node.start_byte());
                        match at == one.head {
                            true => {
                                node = parent_start(node).unwrap_or(node);
                                here.char_at(node.start_byte())
                            }
                            false => at,
                        }
                    }
                };
                let head = head.min(rope.len_chars().saturating_sub(1));
                match extend {
                    true => Range { head, ..*one },
                    false => Range { anchor: head, head, ..*one },
                }
            })
            .collect();
        self.sel.rebuild(ranges, primary);
    }
}
