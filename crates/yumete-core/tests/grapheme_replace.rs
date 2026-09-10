//! `r` writes one character per **glyph**, not per code point (#324).
use yumete_core::{editor::Editor, Key};

fn after(text: &str, keys: &str) -> String {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    for c in text.chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    for c in keys.chars() {
        ed.on_key(Key::Char(c));
    }
    ed.current_buffer().rope().to_string()
}

#[test]
fn one_glyph_becomes_one_character() {
    // A decomposed か is two code points and one glyph. `h` and `l` walk by
    // glyph, so a writer who選中 one and pressed `r` selected one thing.
    assert_eq!(after("か字", "rZ"), "Z字");
    // …and a combining acute, and a half-width dakuten.
    assert_eq!(after("é字", "rZ"), "Z字");
    assert_eq!(after("ｶﾞ字", "rZ"), "Z字");
}

#[test]
fn a_line_ending_is_still_not_written_over() {
    // `x` selects the line including its newline; `x r Z` must not run the
    // line into the next one.
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    for c in "ab\ncd".chars() {
        ed.on_key(if c == '\n' { Key::Enter } else { Key::Char(c) });
    }
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('x'));
    ed.on_key(Key::Char('r'));
    ed.on_key(Key::Char('Z'));
    assert_eq!(ed.current_buffer().rope().to_string(), "ZZ\ncd");
}
