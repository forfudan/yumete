//! **One completion panel, one set of keys.**
//!
//! Words, paths and a language server's list ([`super::suggest`], `page.rs`)
//! and references (`[[`, `[^`, `](#`, `[@`, Typst `@`, [`super::complete`])
//! are two sources of one thing: a short list under the caret, walked by the
//! hand that is typing. They share the panel and the keys, which are helix's
//! completion menu's (`helix-term/src/ui/menu.rs`, `ui/completion.rs`):
//!
//! | key | does |
//! | --- | --- |
//! | `Tab` `C-n` `Down` | choose the next one and write it in |
//! | `S-Tab` `C-p` `Up` | the one before, written in |
//! | typing on | keeps what is written and goes on |
//! | `Enter` | keeps it and closes the panel, no line break |
//! | `Esc` | keeps it, closes the panel and leaves Insert |
//! | `C-c` | takes it back out, restoring what was typed; Insert stays |
//!
//! and vim's two for its popup menu, under both keymaps and only while the
//! panel is open: `C-y` as `Enter`, `C-e` as `C-c`.

use super::*;

/// One row of the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRow {
    /// What the list shows, in the text's ink.
    pub label: String,
    /// The quiet column on the right: a server item's kind, a page's folder,
    /// a heading's level. `None` for a word, whose kind says nothing.
    pub note: Option<String>,
    /// A folder, drawn in the link's ink as well as with its `/`.
    pub folder: bool,
    /// An item the server marks deprecated, drawn crossed out.
    pub deprecated: bool,
}

/// What the panel draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionPanel {
    pub rows: Vec<CompletionRow>,
    /// The row written into the text, if one is.
    pub chosen: Option<usize>,
    /// How many cells of the caret's line the completion has already taken —
    /// from where it starts to the caret — so the rows can stand under it.
    pub lead: usize,
}

/// A language server's `CompletionItemKind`, named as helix names it.
fn kind_name(kind: i64) -> Option<String> {
    Some(match kind {
        1 => say!("complete.kind.text"),
        2 => say!("complete.kind.method"),
        3 => say!("complete.kind.function"),
        4 => say!("complete.kind.constructor"),
        5 => say!("complete.kind.field"),
        6 => say!("complete.kind.variable"),
        7 => say!("complete.kind.class"),
        8 => say!("complete.kind.interface"),
        9 => say!("complete.kind.module"),
        10 => say!("complete.kind.property"),
        11 => say!("complete.kind.unit"),
        12 => say!("complete.kind.value"),
        13 => say!("complete.kind.enum"),
        14 => say!("complete.kind.keyword"),
        15 => say!("complete.kind.snippet"),
        16 => say!("complete.kind.color"),
        17 => say!("complete.kind.file"),
        18 => say!("complete.kind.reference"),
        19 => say!("complete.kind.folder"),
        20 => say!("complete.kind.enum-member"),
        21 => say!("complete.kind.constant"),
        22 => say!("complete.kind.struct"),
        23 => say!("complete.kind.event"),
        24 => say!("complete.kind.operator"),
        25 => say!("complete.kind.type-parameter"),
        crate::lsp::KIND_LINK => say!("complete.kind.link"),
        _ => return None,
    })
}

/// What a key does while the panel is open.
enum Step {
    Next,
    Back,
    Keep,
    TakeBack,
    Other,
}

impl Editor {
    /// **The panel, if one is open** — words, paths, a server's list or a
    /// reference, whichever stands at the caret.
    pub fn completion_panel(&self) -> Option<CompletionPanel> {
        if self.mode != Mode::Insert {
            return None;
        }
        if let Some((items, chosen)) = self.offers_here() {
            let rows = items
                .iter()
                .map(|offer| CompletionRow {
                    label: offer.label.clone(),
                    note: kind_name(offer.kind),
                    folder: offer.kind == crate::lsp::KIND_FOLDER
                        || (offer.kind == crate::lsp::KIND_LINK && offer.label.ends_with('/')),
                    deprecated: offer.deprecated,
                })
                .collect();
            let start = match self.offering.as_ref().and_then(|o| o.prefill.as_ref()) {
                Some(prefill) => prefill.start,
                None => items.first().map_or(self.sel.head(), |item| self.offer_span(item).0),
            };
            return Some(CompletionPanel { rows, chosen, lead: self.cells_from(start) });
        }
        // One reading of the reference: it is worked out afresh from the text,
        // and the panel and the command row both ask on every frame.
        let (choices, chosen, start) = match &self.reference {
            Some(walk) => (walk.choices.clone(), Some(walk.index.min(walk.choices.len().saturating_sub(1))), walk.at),
            None if self.reference_dismissed() => return None,
            None => {
                let offer = self.reference_offer()?;
                (offer.choices, None, offer.at)
            }
        };
        let rows = choices
            .into_iter()
            .map(|c| CompletionRow { label: c.label, note: c.note, folder: false, deprecated: false })
            .collect();
        Some(CompletionPanel { rows, chosen, lead: self.cells_from(start) })
    }

    /// How wide the caret's line is from `start` to the caret.
    fn cells_from(&self, start: usize) -> usize {
        let rope = self.current_buffer().rope();
        let head = self.sel.head().min(rope.len_chars());
        let line_start = rope.line_to_char(rope.char_to_line(head));
        let start = start.clamp(line_start, head);
        yumete_cjk::str_width(&rope.slice(start..head).to_string())
    }

    /// **A key, while the panel is open**: whether the panel took it. Keys it
    /// does not take go on to Insert mode as they always do — after the
    /// written choice has been kept.
    pub(super) fn completion_key(&mut self, key: Key) -> bool {
        let words = self.offers_here().is_some();
        if !words && !self.reference_open() {
            return false;
        }
        let step = match key {
            Key::Tab | Key::Ctrl('n') | Key::Down => Step::Next,
            Key::BackTab | Key::Ctrl('p') | Key::Up => Step::Back,
            Key::Enter | Key::Ctrl('y') => Step::Keep,
            Key::Ctrl('c') | Key::Ctrl('e') => Step::TakeBack,
            _ => Step::Other,
        };
        match words {
            true => self.word_panel_key(key, step),
            false => self.reference_panel_key(key, step),
        }
    }

    fn word_panel_key(&mut self, key: Key, step: Step) -> bool {
        let chosen = self.offering.as_ref().is_some_and(|o| o.chosen.is_some());
        match step {
            Step::Next => self.choose_offer(true),
            Step::Back => self.choose_offer(false),
            // `Enter` with nothing chosen is the line break it always is, as
            // in helix, and the list goes with the line it was about.
            Step::Keep if !chosen && key == Key::Enter => {
                self.offering = None;
                false
            }
            Step::Keep => {
                self.offering = None;
                true
            }
            Step::TakeBack => {
                self.cancel_offer();
                true
            }
            // **`C-x` asks again, by hand**, over the list typing opened:
            // that list never holds the pinyin offers, and `C-n` inside it
            // only moves.
            Step::Other if key == Key::Ctrl('x') => self.offer_from_the_text(true),
            // Anything else keeps what is written; the key then does what it
            // does, and typing on may open a new list.
            Step::Other => {
                if chosen {
                    self.offering = None;
                }
                false
            }
        }
    }

    fn reference_panel_key(&mut self, key: Key, step: Step) -> bool {
        let chosen = self.reference.is_some();
        match step {
            Step::Next => self.cycle_reference(1),
            Step::Back => self.cycle_reference(-1),
            Step::Keep if !chosen && key == Key::Enter => false,
            Step::Keep => {
                self.dismiss_reference();
                true
            }
            Step::TakeBack => {
                self.cancel_reference();
                true
            }
            Step::Other => {
                self.reference = None;
                false
            }
        }
    }

    /// **`Tab 补全 · 3/12`** on the command row while the panel is open: the
    /// key, and where in the list the choice is.
    pub(super) fn completion_hint(&self) -> Option<Hint> {
        let panel = self.completion_panel()?;
        let said = match panel.chosen {
            Some(at) => say!("hint.complete", at + 1, panel.rows.len()),
            None => say!("hint.complete-count", panel.rows.len()),
        };
        Some(Hint::Keys(String::new(), vec![("Tab".into(), said)]))
    }
}
