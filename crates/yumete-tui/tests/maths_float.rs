//! **The formula float** (2026-11-06): drawn only where the terminal can
//! show a picture and the setting is on, and then as placeholder cells.
//!
//! A binary of its own: whether pictures can be shown is process-wide
//! (`graphics::set_support`), and a neighbour drawing a frame while this one
//! has it on would find a picture it never asked for.

use yumete_config::Config;
use yumete_core::editor::Editor;
use yumete_core::input::Key;
use yumete_core::markdown::Maths;
use yumete_core::syntax::Syntax;
use yumete_ime::{ImeSession, Scheme};
use yumete_tui::graphics::{self, Support, PLACEHOLDER};

/// A Markdown page with `$x^2$` on its second line, the cursor in it.
fn on_a_formula() -> Editor {
    let mut editor = Editor::new();
    editor.on_key(Key::Char('i'));
    for c in "前文\n式 $x^2$ 呢\n後文".chars() {
        editor.on_key(if c == '\n' { Key::Enter } else { Key::Char(c) });
    }
    editor.on_key(Key::Esc);
    editor.set_syntax(Syntax::Markdown);
    for c in "ggj0lll".chars() {
        editor.on_key(Key::Char(c));
    }
    assert!(editor.maths_here().is_some(), "the cursor is in the formula");
    editor
}

/// A PNG header saying 40 × 40 pixels — all the float reads of it.
fn a_picture() -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend(40u32.to_be_bytes());
    png.extend(40u32.to_be_bytes());
    png.extend([8, 6, 0, 0, 0]);
    png
}

fn frame(editor: &mut Editor, config: &Config) -> String {
    let ime = ImeSession::empty(Scheme::LINGMING);
    yumete_tui::frame_to_text(editor, config, &ime, 60, 16, None)
}

#[test]
fn the_float_is_drawn_only_where_it_can_be() {
    let mut editor = on_a_formula();
    let mut config = Config::default();

    // A terminal that cannot show pictures — every frame `--shot` and the
    // golden frames draw — has nothing new on it.
    let plain = frame(&mut editor, &config);
    assert!(!plain.contains(PLACEHOLDER), "{plain}");
    assert!(graphics::take_pending().is_empty(), "nothing uploaded");

    // Made already, so no typst is needed: 40 × 40 pixels at the guessed
    // 10 × 20 cell is four cells by two. The ink is read after a frame, which
    // settles the mood the page is drawn in.
    let ink = match yumete_tui::theme::Palette::of(&config).text() {
        ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
        other => panic!("not RGB: {other:?}"),
    };
    let maths = Maths { source: "x^2".into(), latex: true, display: false };
    yumete_tui::maths::preload(
        yumete_tui::maths::Key { maths, ink, cell_height: graphics::cell_pixels().1 },
        a_picture(),
    );

    // One that can, with the setting off: the same page.
    graphics::set_support(Support::Kitty);
    config.editor.maths_preview = false;
    assert_eq!(frame(&mut editor, &config), plain);
    assert!(graphics::take_pending().is_empty(), "nothing uploaded with the setting off");

    // And on: the picture under the cursor, in placeholder cells, and its
    // upload owed to the terminal before them.
    config.editor.maths_preview = true;
    let shown = frame(&mut editor, &config);
    let rows: Vec<&str> = shown.lines().filter(|r| r.contains(PLACEHOLDER)).collect();
    assert_eq!(rows.len(), 2, "two rows of picture:\n{shown}");
    assert_eq!(rows[0].matches(PLACEHOLDER).count(), 4, "four cells across:\n{shown}");
    let caret_row = shown.lines().position(|r| r.contains("式")).expect("the formula's line");
    let first_picture_row = shown.lines().position(|r| r.contains(PLACEHOLDER)).unwrap();
    assert_eq!(first_picture_row, caret_row + 2, "under the cursor, inside the ring:\n{shown}");
    let owed = String::from_utf8(graphics::take_pending()).unwrap();
    assert!(owed.starts_with("\x1b_Ga=T,U=1,f=100,"), "{owed:?}");
    assert!(owed.contains(",c=4,r=2,"), "{owed:?}");

    // Off the formula, no float.
    editor.on_key(Key::Char('j'));
    let off = frame(&mut editor, &config);
    assert!(!off.contains(PLACEHOLDER), "{off}");
    graphics::set_support(Support::None);
}
