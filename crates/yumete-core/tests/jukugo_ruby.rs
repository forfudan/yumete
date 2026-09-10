//! 熟語振假名, and the three HTML spellings that were invisible (#330, #331).
//!
//! `<ruby>漢<rt>かん</rt>字<rt>じ</rt></ruby>` is one element holding two
//! base-and-reading pairs — the standard shape for a compound read a character
//! at a time, which is most of what a Japanese manuscript annotates. Read as
//! one pair by looking for `</rt></ruby>`, the second base and the tags around
//! it were swallowed into the first reading, and `:ruby-format typst` wrote
//! `#ruby("漢", "かん</rt>字<rt>じ")`: 字 stopped being text.

use yumete_core::editor::Editor;
use yumete_core::ruby::{groups, unread, Dialect, Dialects};
use yumete_core::Key;

fn read(text: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let say = |a: usize, b: usize| -> String { chars[a..b.min(chars.len())].iter().collect() };
    groups(&chars, Dialects::only(Dialect::Html))
        .iter()
        .map(|g| (say(g.base.0, g.base.1), say(g.reading.0, g.reading.1)))
        .collect()
}

#[test]
fn one_ruby_may_hold_a_reading_for_each_character() {
    assert_eq!(
        read("<ruby>漢<rt>かん</rt>字<rt>じ</rt></ruby>"),
        [("漢".to_string(), "かん".to_string()), ("字".into(), "じ".into())]
    );
}

#[test]
fn the_three_spellings_that_were_invisible_are_read() {
    // The `<rp>` fallback the W3C recommends: the parentheses are what a
    // reader without ruby support sees, and are neither base nor reading.
    assert_eq!(
        read("<ruby>東京<rp>(</rp><rt>とうきょう</rt><rp>)</rp></ruby>"),
        [("東京".to_string(), "とうきょう".to_string())]
    );
    assert_eq!(
        read("<ruby lang=\"ja\">漢<rt>かん</rt></ruby>"),
        [("漢".to_string(), "かん".to_string())]
    );
    assert_eq!(
        read("<RUBY>漢<RT>かん</RT></RUBY>"),
        [("漢".to_string(), "かん".to_string())]
    );
}

#[test]
fn what_cannot_be_read_yields_nothing_rather_than_a_guess() {
    // A `<ruby>` inside a `<ruby>`, and one that never closes. Guessing at
    // these is exactly how 字 was written into a reading.
    assert_eq!(read("<ruby>外<ruby>內<rt>な</rt></ruby><rt>そ</rt></ruby>").len(), 1);
    assert!(read("<ruby>漢<rt>かん</rt>").is_empty());
    assert_eq!(unread("<ruby>漢<rt>かん</rt>", Dialect::Typst), 1);
    assert_eq!(unread("#ruby(\"漢\", \"かん\")", Dialect::Typst), 0);
}

#[test]
fn the_command_keeps_every_character_it_was_given() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    for c in "讀<ruby>漢<rt>かん</rt>字<rt>じ</rt></ruby>了".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Esc);
    ed.execute(":ruby-format typst").unwrap();
    assert_eq!(
        ed.current_buffer().rope().to_string(),
        "讀#ruby(\"漢\", \"かん\")#ruby(\"字\", \"じ\")了",
        "{}",
        ed.status()
    );
}
