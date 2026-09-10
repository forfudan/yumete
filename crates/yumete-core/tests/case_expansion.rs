//! A case change may be more than one character (#325).
//!
//! `ﬁ` upper-cases to `FI` and `ß` to `SS`. Taking only the first of them —
//! which a `char -> char` signature forces — deleted the rest in silence, so
//! `ﬁ` came back `F` and `ß` came back `S`. Ligatures pasted out of a PDF and
//! German `ß` are ordinary in an English manuscript.

use yumete_core::{editor::Editor, Key};

fn cased(text: &str, keys: &str) -> String {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    for c in text.chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Esc);
    // The whole line, then the operator.
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('x'));
    for c in keys.chars() {
        ed.on_key(Key::Char(c));
    }
    ed.current_buffer().rope().to_string()
}

#[test]
fn upper_case_keeps_every_character_it_makes() {
    assert_eq!(cased("aβ漢ＡＢＣﬁß x", "`u"), "AΒ漢ＡＢＣFISS X");
}

#[test]
fn lower_case_keeps_every_character_it_makes() {
    // `İ` — a dotted capital I — lower-cases to `i` plus a combining dot.
    assert_eq!(cased("İ", "`l"), "i\u{307}");
}

#[test]
fn switching_keeps_them_too() {
    assert_eq!(cased("ß", "``"), "SS");
}
