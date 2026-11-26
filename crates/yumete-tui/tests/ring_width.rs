//! **With `ambiguous_width = "wide"` every ring on the screen is a
//! rectangle.**
//!
//! The box-drawing block is East Asian Ambiguous: a CJK terminal draws `╭`
//! `─` `│` two cells wide, so each stroke of a ring is given the second cell
//! the editor counts for it (`chrome::draw`), and whatever a panel writes
//! inside stays inside the walls.
//!
//! Its own binary because `set_ambiguous_wide` is process-wide.

mod common;

use yumete_ime::{ImeSession, Scheme};

/// The words between a row's two walls, spaces folded.
fn inside(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|r| {
            let from = r.find('│')? + '│'.len_utf8();
            let to = r.rfind('│')?;
            (from <= to).then(|| r[from..to].split_whitespace().collect::<Vec<_>>().join(" "))
        })
        .filter(|r| !r.is_empty())
        .collect()
}

/// One test, so nothing else in the binary sees the width switched back and
/// forth: each panel is drawn first as it is by default, to say what has to
/// be inside its walls, and then with ambiguous characters wide.
#[test]
fn every_panel_s_ring_is_a_rectangle_with_ambiguous_wide() {
    let ime = ImeSession::empty(Scheme::LINGMING);
    let scenes = common::scenes();
    assert!(scenes.iter().any(|(name, ..)| *name == "query"), "the question opens");
    for (name, mut editor, config) in scenes {
        yumete_cjk::set_ambiguous_wide(false);
        let narrow = yumete_tui::frame_to_text(&mut editor, &config, &ime, 100, 30, None);
        yumete_cjk::set_ambiguous_wide(true);
        assert_eq!(yumete_cjk::char_width('│'), 2);
        // **What a panel writes stays inside its walls**: the rows of the
        // question and of the character float keep their last characters.
        if matches!(name, "query" | "character float") {
            let wide = yumete_tui::frame_to_text(&mut editor, &config, &ime, 100, 30, None);
            let (want, got) = (inside(&narrow), inside(&wide));
            assert!(!want.is_empty(), "{name}: nothing inside\n{narrow}");
            for row in &want {
                assert!(got.contains(row), "{name}: {row:?} is not whole inside the walls\n{wide}");
            }
        }
        // Even and odd widths: an odd one is where a rule stops short of its
        // corner.
        for (w, h) in [(80u16, 24u16), (81, 24), (41, 14), (100, 30)] {
            let text = yumete_tui::frame_to_text(&mut editor, &config, &ime, w, h, None);
            match common::rings_are_rectangles(&text) {
                Ok(0) => panic!("{name} {w}x{h}: no ring on the page\n{text}"),
                Ok(_) => {}
                Err(e) => panic!("{name} {w}x{h}: {e}\n{text}"),
            }
        }
    }
}
