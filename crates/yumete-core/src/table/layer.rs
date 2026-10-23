//! **Which layer of a grid the keys are in** — the model settled 2026-10-21.
//!
//! A grid has two units and one keyboard, and for a year `hjkl` tried to owe
//! both: in a table they walked cells, in the same table a key away they
//! walked characters, and which one you got depended on a toggle with no mark
//! on the screen. Every table bug reported so far has been that one question
//! asked twice — 「is this a line or a record, a character or a cell?」 Layers
//! answer it once: at the top there are only cells, inside a cell there are
//! only characters, and a key is pressed to cross between them.
//!
//! **This module is the model and nothing else.** It holds no buffer, touches
//! no editor and draws nothing. Three things live here:
//!
//! 1. the state — [`Layer`], one of two, and where a grid starts;
//! 2. the only places that state moves — [`Layer::step`];
//! 3. what a layer claims of a key before anything happens — [`Layer::claim`].
//!
//! The editor executes what [`Layer::claim`] returns; the terminal draws what
//! [`Layer::caret`] and [`Layer::edge`] say. Both ask; neither decides. That
//! is the whole of the decoupling: a question about the model is answered by
//! reading one short file, and a change to the model is made in it.
//!
//! The third layer is [`Mode::Insert`], which is not a value here on purpose —
//! see [`Layer::step`].

use crate::input::{Key, Mode};

/// Which layer of a grid the keys are in.
///
/// **The view's, not the editor's**: it is a field of the table view, so
/// closing the grid takes the layer with it and no stale value can be left
/// behind for the next table to inherit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layer {
    /// **The top layer: the whole cell is the unit.** No caret is drawn, the
    /// selection is exactly one cell, and `hjkl` walk cells and rows.
    ///
    /// **Where a grid opens** (2026-10-21 定): reading a table is commoner
    /// than editing one, and going down costs one key.
    #[default]
    Whole,
    /// **The middle layer: inside one cell.** An ordinary caret, and the
    /// cell's edges are the walls — `gg` is the cell's first character, `G`
    /// its last.
    Inside,
}

/// Where a key takes the keyboard next, as [`Layer::claim`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// A motion over cells. The editor moves the selection; the layer does
    /// not care how.
    Cell(Step),
    /// **Take a whole thing as the selection.** A key that selects means
    /// something different where the unit is the cell: `x` takes a record
    /// rather than a line of the file, and `%` takes this table rather than
    /// the document it is sitting in (2026-10-21 作者定: 「`x` 选整条记录 is
    /// a must have feature. Also nice if users want to copy the whole
    /// table.」).
    Take(Whole),
    /// Cross into another layer.
    Cross(Layer),
    /// **The table's command group, as though `␣t` had been pressed.**
    ///
    /// `t` at the top layer and nowhere else (2026-10-20 作者定: 「now we have a
    /// TAB mode and we can use the short cut `t` as an alias of `_t`, as
    /// "till" is not useful in the TAB mode … `_t` can be kept」). There is no
    /// character to stop before when the whole cell is the unit, so the key is
    /// free there; inside a cell `t` is vi's till again, and so it is
    /// everywhere outside the grid.
    Menu,
    /// **Not this layer's business.** The ordinary editor takes the key, and
    /// the grid changes nothing about what it means. Paging, search, `:` and
    /// every operator arrive here.
    Fall,
}

/// One step over cells.
///
/// Named by what the reader wanted, not by the key pressed: the three keymap
/// presets spell these differently and the model must not know which preset is
/// loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Left,
    Right,
    /// One row down, in the same column.
    Down,
    /// One row up, in the same column.
    Up,
    /// The first cell of this record.
    RecordStart,
    /// The last cell of this record.
    RecordEnd,
    /// The next cell, carrying on to the next record at the end of this one —
    /// what every spreadsheet means by `Tab`.
    Next,
    /// The previous cell, the way `Next` came.
    Prev,
}

/// What a selection key takes, where the unit is the cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Whole {
    /// **One record**, and one more each time the key is pressed again — the
    /// same thing `x` means in prose, counted in records instead of lines. A
    /// field holding a line break makes one record several lines, and the
    /// reader asked for the record.
    Record,
    /// **This table**, header and all — not the document it sits in.
    Table,
}

/// How the active cell is bracketed — [`Layer`]'s one instruction to the
/// terminal besides [`Layer::caret`].
///
/// The mark sits in the seam the grid already keeps between columns, so it
/// costs the cell no width (#407). **Only the two vertical edges exist**: a
/// row has nothing to spend on a horizontal rule but its own characters.
///
/// **One mark, in all three layers** (2026-10-22 作者定: 「the CEL mode, the
/// highlighted border can use the same style as NOR mode (dashed and
/// golden). It does not need to be normal line. This makes the border style
/// consistent in all three layers.」). Where the grid draws its own rule the
/// seam keeps that rule and merely turns 金; these two glyphs are what stands
/// there when it draws none (`:table-rules off`, `colour`).
///
/// It went half-block → one stroke → the grid's own rule in two days, and
/// each step had the same reason: the mark started out as the only thing
/// saying 「the whole cell is the unit」 and had to be loud enough to say it
/// alone. The 金 ground says it now, so the edge is free to be drawn in the
/// weight the rest of the grid is — and a cell's border changing shape as the
/// caret goes in and out of it was one answer too many to 「does this grid
/// draw rules」.
///
/// The two are mirrored because they are ink pushed to one side of a cell,
/// and the side that faces the text is a different side on the left and on
/// the right.
///
/// The seam **before** the cell: ▕ right one eighth block.
pub const EDGE_BEFORE: &str = "\u{2595}";
/// The seam **after** it: ▏ left one eighth block.
pub const EDGE_AFTER: &str = "\u{258f}";

impl Layer {
    /// **The status line's word.**
    ///
    /// The layer prefixes whatever word the editor had already worked out for
    /// the mode, so `REP` and `SEL` come along for free and this file does not
    /// have to know they exist. The one special case is the top layer at rest,
    /// which says `CEL` alone: there is no second word to add, because the
    /// keys there are the layer's own.
    ///
    /// The dotted spelling is the one this editor already uses for 「where the
    /// keys are」 — `PIC.INS` in a picker, `PAN.NOR` in a side panel.
    ///
    /// Warning: **`CEL.INS` cannot say which layer `Esc` goes back to.** Both
    /// ways into insert read the same, and that is accepted (2026-10-21):
    /// `Esc` means 「back where I was」 whichever it was, and a fourth word
    /// would cost cells the status line does not have (#394).
    pub fn label(self, mode: &str) -> String {
        match (self, mode) {
            (Layer::Whole, "NOR") => "CEL".to_string(),
            _ => format!("CEL.{mode}"),
        }
    }

    /// **Whether the whole cell is the unit at this moment** — the one
    /// question everything the reader sees is an answer to.
    ///
    /// The layer alone cannot say it, because **typing is inside a cell
    /// whichever layer you came from**. `i` at the top layer leaves the field
    /// at [`Layer::Whole`] on purpose — that is the whole of 「`Esc` goes back
    /// to the layer you came from」 ([`Layer::step`]) — so the mark has to ask
    /// the mode too, or `i` from the top layer would leave the page looking
    /// like a cell was selected while characters were going into it
    /// (2026-10-21 作者報的:「Using `i` from CEL -> INS remains the golden
    /// thick borders. So it looks differently from using `i` from NOR -> INS.
    /// This is a bug.」).
    pub fn the_cell_is_the_unit(self, mode: Mode) -> bool {
        self == Layer::Whole && mode == Mode::Normal
    }

    /// Whether a caret is drawn at all.
    ///
    /// `false` in one case only: while the whole cell is the unit, where
    /// there is no character to stand on. The terminal's own cursor is left
    /// unset for that frame, which is how this editor already hides it over a
    /// settings page and in a panel field.
    pub fn caret(self, mode: Mode) -> bool {
        !self.the_cell_is_the_unit(mode)
    }

    /// **Whether the cell is the whole document at this layer.**
    ///
    /// `true` in the middle, where the cell's edges are the walls: `l` stops
    /// at its last character, `gg` is its first, `$` its end, and `d w`
    /// cannot reach the delimiter. That is what 「inside one cell」 means, and
    /// it is the half of the middle layer this model could not state until
    /// the motions were bounded (2026-10-21).
    ///
    /// `false` at the top, where motions are over cells and there is nothing
    /// inside one to be walled.
    ///
    /// Warning: **this is not a list of keys**, and it must not become one. The
    /// three keymap presets spell `l` and `gg` differently; what is walled is
    /// a *motion*, which the editor holds as a value, and which of those are
    /// walled is the editor's to say — see `Editor::walled`.
    pub fn walled(self) -> bool {
        self == Layer::Inside
    }

    /// Whether the selection is a whole cell at this layer.
    ///
    /// The top layer's selection *is* the cell, which is what makes `y`, `d`,
    /// `c` and `R` cell operators without any of them being taught about
    /// tables.
    pub fn selects_the_cell(self) -> bool {
        self == Layer::Whole
    }

    /// **The only place a layer changes** — `None` when this key does not
    /// cross.
    ///
    /// Warning: **insert never moves the layer.** That is the whole mechanism
    /// behind 「`Esc` goes back to the layer you came from」 (2026-10-21 定):
    /// `i` at the top leaves this field at [`Layer::Whole`] and `i` in the
    /// middle leaves it at [`Layer::Inside`], so leaving insert only has to
    /// read it. One field, no second flag, and therefore no way for a flag and
    /// a layer to disagree.
    ///
    /// `Esc` at the top layer answers `None` here, and it must stay that way:
    /// it clears a count and a selection and **does not leave the grid**
    /// (people press it hard when stuck, and losing the whole view is the
    /// worst available answer). Leaving is `␣t q` / `␣t o`.
    pub fn step(self, mode: Mode, key: Key) -> Option<Layer> {
        if mode == Mode::Insert {
            return None;
        }
        match (self, key) {
            (Layer::Whole, Key::Enter) => Some(Layer::Inside),
            (Layer::Inside, Key::Esc) => Some(Layer::Whole),
            _ => None,
        }
    }

    /// **What this layer claims of a key**, before the editor does anything.
    ///
    /// Everything not named here falls through, which is the half that keeps
    /// the grid honest: a table changes what `hjkl` mean and nothing else, so
    /// `/`, `:`, `u`, paging and every operator are the same keys they are in
    /// a novel.
    ///
    /// Warning: **`Tab` is claimed in both layers.** It is the one cell motion a
    /// reader wants while the caret is inside a cell — it is how a table is
    /// filled in — and it already meant that in insert mode before any of this
    /// (`prompt.rs`). A key claimed by both layers with the *same* meaning is
    /// not the ambiguity this model removes; `hjkl` claimed by one layer with
    /// two meanings was.
    pub fn claim(self, mode: Mode, key: Key) -> Claim {
        if mode != Mode::Normal {
            return match key {
                Key::Tab => Claim::Cell(Step::Next),
                Key::BackTab => Claim::Cell(Step::Prev),
                _ => Claim::Fall,
            };
        }
        if let Some(to) = self.step(mode, key) {
            return Claim::Cross(to);
        }
        match key {
            Key::Tab => Claim::Cell(Step::Next),
            Key::BackTab => Claim::Cell(Step::Prev),
            _ if self == Layer::Inside => Claim::Fall,
            Key::Char('h') | Key::Left => Claim::Cell(Step::Left),
            Key::Char('l') | Key::Right => Claim::Cell(Step::Right),
            Key::Char('j') | Key::Down => Claim::Cell(Step::Down),
            Key::Char('k') | Key::Up => Claim::Cell(Step::Up),
            Key::Char('0') | Key::Home => Claim::Cell(Step::RecordStart),
            Key::Char('$') | Key::End => Claim::Cell(Step::RecordEnd),
            Key::Char('x') => Claim::Take(Whole::Record),
            Key::Char('%') => Claim::Take(Whole::Table),
            Key::Char('t') => Claim::Menu,
            _ => Claim::Fall,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key this model is asked about, so the invariants below can be
    /// stated over all of them rather than over the few that came to mind.
    fn every_key() -> Vec<Key> {
        let mut keys = vec![
            Key::Tab,
            Key::BackTab,
            Key::Enter,
            Key::Esc,
            Key::Backspace,
            Key::Delete,
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
        ];
        for c in ('a'..='z').chain('A'..='Z').chain('0'..='9') {
            keys.push(Key::Char(c));
            keys.push(Key::Ctrl(c));
            keys.push(Key::Alt(c));
        }
        for c in "$^|/:%<>,.;'\"-=+*#@!?()[]{}".chars() {
            keys.push(Key::Char(c));
        }
        // Stated over *every* key, so a duplicate would make the counts in
        // these tests say something other than what they read as.
        let mut seen = std::collections::HashSet::new();
        keys.retain(|k| seen.insert(format!("{k:?}")));
        keys
    }

    const BOTH: [Layer; 2] = [Layer::Whole, Layer::Inside];

    /// **Each layer has exactly one unit, and they are not the same one.**
    ///
    /// The two questions the editor asks of a layer before it moves anything:
    /// 「is the whole cell the thing I am acting on?」 and 「is the cell the
    /// whole document?」. One is true at the top, the other in the middle, and
    /// neither is ever true at both — that is the model in one line.
    #[test]
    fn one_layer_takes_the_cell_and_the_other_is_taken_by_it() {
        for layer in BOTH {
            assert_ne!(
                layer.selects_the_cell(),
                layer.walled(),
                "{layer:?} answers both unit questions the same way"
            );
        }
        assert!(Layer::Whole.selects_the_cell());
        assert!(Layer::Inside.walled());
    }

    /// **A grid opens at the top layer** (2026-10-21 定).
    #[test]
    fn a_table_opens_with_the_whole_cell() {
        assert_eq!(Layer::default(), Layer::Whole);
        assert!(Layer::default().selects_the_cell());
        assert!(!Layer::default().walled());
        assert!(!Layer::default().caret(Mode::Normal));
        assert!(Layer::default().the_cell_is_the_unit(Mode::Normal));
    }

    /// **Insert never moves the layer** — the mechanism behind 「`Esc` goes
    /// back to the layer you came from」. Nothing else records that.
    #[test]
    fn insert_never_moves_the_layer() {
        for from in BOTH {
            for key in every_key() {
                assert_eq!(
                    from.step(Mode::Insert, key),
                    None,
                    "{from:?} + {key:?} moved the layer while typing"
                );
            }
        }
    }

    /// **One key down, one key up, and no others.** A third way across would
    /// be a third thing to remember and a third thing to get wrong.
    ///
    /// Counted rather than matched: a test written as the same `match` the
    /// implementation holds is a copy of it, and would agree with any mistake
    /// made in both places at once.
    #[test]
    fn one_key_crosses_each_way() {
        for from in BOTH {
            let crossing: Vec<Key> = every_key()
                .into_iter()
                .filter(|&k| from.step(Mode::Normal, k).is_some())
                .collect();
            assert_eq!(crossing.len(), 1, "{from:?} crosses on {crossing:?}");
        }
        assert_eq!(
            Layer::Whole.step(Mode::Normal, Key::Enter),
            Some(Layer::Inside),
            "Enter goes down"
        );
        assert_eq!(
            Layer::Inside.step(Mode::Normal, Key::Esc),
            Some(Layer::Whole),
            "Esc comes up"
        );
    }

    /// **`Esc` at the top layer does not cross** — and so cannot be the key
    /// that leaves the grid either.
    #[test]
    fn esc_at_the_top_stays_at_the_top() {
        assert_eq!(Layer::Whole.step(Mode::Normal, Key::Esc), None);
        assert_eq!(Layer::Whole.claim(Mode::Normal, Key::Esc), Claim::Fall);
    }

    /// **`Tab` walks cells in both layers, with one meaning.**
    #[test]
    fn tab_walks_cells_from_either_layer() {
        for from in BOTH {
            for mode in [Mode::Normal, Mode::Insert] {
                assert_eq!(from.claim(mode, Key::Tab), Claim::Cell(Step::Next));
                assert_eq!(from.claim(mode, Key::BackTab), Claim::Cell(Step::Prev));
            }
        }
    }

    /// **The two keys that select take a cell-sized thing**, and only at the
    /// top layer: inside a cell `x` is a line of the file and `%` is the
    /// document, which is what they are in prose.
    #[test]
    fn a_selection_key_takes_a_cell_sized_thing() {
        assert_eq!(
            Layer::Whole.claim(Mode::Normal, Key::Char('x')),
            Claim::Take(Whole::Record)
        );
        assert_eq!(
            Layer::Whole.claim(Mode::Normal, Key::Char('%')),
            Claim::Take(Whole::Table)
        );
        for key in [Key::Char('x'), Key::Char('%')] {
            assert_eq!(Layer::Inside.claim(Mode::Normal, key), Claim::Fall, "{key:?}");
        }
    }

    /// **The middle layer claims no letter.** Inside a cell this is an
    /// ordinary file, so `hjkl`, the operators and the selection mean what
    /// they mean everywhere else.
    #[test]
    fn the_middle_layer_claims_nothing_but_tab() {
        for key in every_key() {
            let claim = Layer::Inside.claim(Mode::Normal, key);
            let expected = match key {
                Key::Tab => Claim::Cell(Step::Next),
                Key::BackTab => Claim::Cell(Step::Prev),
                Key::Esc => Claim::Cross(Layer::Whole),
                _ => Claim::Fall,
            };
            assert_eq!(claim, expected, "{key:?} inside a cell");
        }
    }

    /// **The top layer claims the eight motions and nothing else** — every
    /// other key is the editor's, unchanged by the grid.
    #[test]
    fn the_top_layer_claims_the_motions_and_no_more() {
        let cells: Vec<Key> = every_key()
            .into_iter()
            .filter(|&k| matches!(Layer::Whole.claim(Mode::Normal, k), Claim::Cell(_)))
            .collect();
        assert_eq!(
            cells,
            vec![
                Key::Tab,
                Key::BackTab,
                Key::Up,
                Key::Down,
                Key::Left,
                Key::Right,
                Key::Home,
                Key::End,
                Key::Char('h'),
                Key::Char('j'),
                Key::Char('k'),
                Key::Char('l'),
                Key::Char('0'),
                Key::Char('$'),
            ],
            "the top layer's key map changed"
        );
    }

    /// **`t` is the table's group at the top, and nowhere else** (2026-10-20).
    ///
    /// Stated over every key, so a second key that opens the menu would show
    /// up here as well as a `t` that stopped opening it; and over both layers
    /// and every mode, so the alias cannot leak into a cell, where `t` is
    /// vi's till and a reader stops before a character with it.
    #[test]
    fn t_opens_the_table_group_at_the_top_layer_only() {
        let menu: Vec<(Layer, Mode, Key)> = BOTH
            .into_iter()
            .flat_map(|l| [Mode::Normal, Mode::Insert, Mode::Command].map(move |m| (l, m)))
            .flat_map(|(l, m)| every_key().into_iter().map(move |k| (l, m, k)))
            .filter(|&(l, m, k)| matches!(l.claim(m, k), Claim::Menu))
            .collect();
        assert_eq!(
            menu,
            vec![(Layer::Whole, Mode::Normal, Key::Char('t'))],
            "who opens the table group"
        );
    }

    /// **The status word is prefixed once**, and the only bare word is the top
    /// layer at rest.
    #[test]
    fn the_status_word_says_which_layer() {
        assert_eq!(Layer::Whole.label("NOR"), "CEL");
        assert_eq!(Layer::Inside.label("NOR"), "CEL.NOR");
        assert_eq!(Layer::Whole.label("INS"), "CEL.INS");
        assert_eq!(Layer::Inside.label("INS"), "CEL.INS");
        // The words the rest of the editor computes come along unchanged.
        assert_eq!(Layer::Inside.label("REP"), "CEL.REP");
        assert_eq!(Layer::Whole.label("SEL"), "CEL.SEL");
    }

    /// **The word 「NOR」 is pinned**, because the test above matches on it:
    /// if the mode's own label is ever respelt, the top layer would quietly
    /// start reading `CEL.NOR` and the bare `CEL` would never appear again.
    #[test]
    fn the_resting_word_this_file_matches_on_is_the_editors_own() {
        assert_eq!(Mode::Normal.label(), Some("NOR"));
        assert_eq!(Mode::Insert.label(), Some("INS"));
    }

    /// **One moment, one answer, and the two marks never disagree.** Typing
    /// has a caret wherever it happens and no 金 ground wherever it happens;
    /// a page that washed a cell round a caret would be saying two things at
    /// once (2026-10-21 作者報的).
    ///
    /// The second mark used to be the edge, and is not any more
    /// (2026-10-22): the two seams are 金 in either layer, so the ground is
    /// the whole of the difference. A ground is the terminal's to draw, so
    /// that it follows this answer is asserted over there, in
    /// `yumete_tui::tests::the_top_layer_washes_its_cell_and_draws_no_caret`.
    #[test]
    fn the_caret_is_drawn_wherever_the_cell_is_not_the_unit() {
        let every = || {
            BOTH.into_iter()
                .flat_map(|l| [Mode::Normal, Mode::Insert, Mode::Command].map(move |m| (l, m)))
        };
        for (layer, mode) in every() {
            assert_eq!(
                layer.caret(mode),
                !layer.the_cell_is_the_unit(mode),
                "{layer:?} in {mode:?} washes the cell and draws a caret in it at once"
            );
        }
        let whole: Vec<(Layer, Mode)> =
            every().filter(|&(l, m)| l.the_cell_is_the_unit(m)).collect();
        assert_eq!(whole, vec![(Layer::Whole, Mode::Normal)], "when is a cell the unit");
    }

    /// **The two marks hug the cell, one column each.**
    ///
    /// They are mirrored because they are ink pushed to one side of a cell,
    /// and the side that faces the text is a different side on the left and
    /// on the right. One cell each, or the grid's columns would shift.
    #[test]
    fn the_edge_is_one_column_wide_on_either_side() {
        assert_ne!(EDGE_BEFORE, EDGE_AFTER, "each hugs the cell from its own side");
        for glyph in [EDGE_BEFORE, EDGE_AFTER] {
            assert_eq!(yumete_cjk::str_width(glyph), 1);
        }
    }
}
