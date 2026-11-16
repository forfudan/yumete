//! **A display formula drawn in place**: under `:render full`, a display
//! block on lines of its own is its picture in the rows its source takes —
//! only where the terminal can show a picture and the setting is on, and
//! never while the cursor is in it.
//!
//! A binary of its own: whether pictures can be shown is process-wide
//! (`graphics::set_support`), and a neighbour drawing a frame while this one
//! has it on would find a picture it never asked for.

use yumete_config::Config;
use yumete_core::editor::{Editor, Render};
use yumete_core::input::Key;
use yumete_core::markdown::Maths;
use yumete_core::syntax::Syntax;
use yumete_ime::{ImeSession, Scheme};
use yumete_tui::graphics::{self, Support, PLACEHOLDER};

/// A buffer of `syntax` holding `text`, under `:render full`, the cursor
/// at the top.
fn page(text: &str, syntax: Syntax) -> Editor {
    let mut editor = Editor::new();
    editor.on_key(Key::Char('i'));
    for c in text.chars() {
        editor.on_key(if c == '\n' { Key::Enter } else { Key::Char(c) });
    }
    editor.on_key(Key::Esc);
    editor.set_syntax(syntax);
    editor.set_render(Render::Full);
    for c in "gg".chars() {
        editor.on_key(Key::Char(c));
    }
    editor
}

fn frame(editor: &mut Editor, config: &Config) -> String {
    let ime = ImeSession::empty(Scheme::LINGMING);
    yumete_tui::frame_to_text(editor, config, &ime, 60, 16, None)
}

/// The screen rows that hold a placeholder.
fn picture_rows(shown: &str) -> Vec<usize> {
    shown.lines().enumerate().filter(|(_, r)| r.contains(PLACEHOLDER)).map(|(n, _)| n).collect()
}

/// The screen row whose text holds `what`.
fn row_of(shown: &str, what: &str) -> usize {
    shown.lines().position(|r| r.contains(what)).unwrap_or_else(|| panic!("no {what}:\n{shown}"))
}

/// Whether pictures can be shown is one setting for the process, so the
/// tests here take turns.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

const DOC: &str = "前文\n$$\n\\frac{a}{b} + c\n$$\n後文";

#[test]
fn a_display_block_is_its_picture_where_it_can_be() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    graphics::set_support(Support::None);
    let mut editor = page(DOC, Syntax::Markdown);
    let mut config = Config::default();
    // The float under a cursor standing in the block is another test's.
    config.editor.maths_preview = false;

    // A terminal that cannot show pictures — every frame `--shot` and the
    // golden frames draw — shows the source.
    let plain = frame(&mut editor, &config);
    assert!(!plain.contains(PLACEHOLDER), "{plain}");
    assert!(plain.contains("frac"), "{plain}");
    let (before, after) = (row_of(&plain, "前文"), row_of(&plain, "後文"));
    assert_eq!(after, before + 4, "a row a line:\n{plain}");

    // One that can: the block's three rows are the picture, and nothing
    // above or below it moves.
    graphics::set_support(Support::Kitty);
    let _ = graphics::take_pending();
    let shown = frame(&mut editor, &config);
    let rows = picture_rows(&shown);
    assert!(!rows.is_empty(), "{shown}");
    assert!(rows.iter().all(|&r| r > before && r < after), "inside the block's rows {before}..{after}:\n{shown}");
    assert!(!shown.contains("frac"), "the source is under the picture:\n{shown}");
    assert_eq!((row_of(&shown, "前文"), row_of(&shown, "後文")), (before, after), "{shown}");
    let owed = String::from_utf8(graphics::take_pending()).unwrap();
    assert!(owed.starts_with("\x1b_Ga=T,U=1,f=100,"), "{owed:?}");
    // The same page again: nothing to upload.
    assert_eq!(frame(&mut editor, &config), shown);
    assert!(graphics::take_pending().is_empty(), "an unchanged picture is not sent twice");

    // The setting off: the source.
    config.editor.maths_in_place = false;
    assert_eq!(frame(&mut editor, &config), plain);
    config.editor.maths_in_place = true;

    // `:render basic` keeps the markup on the page, and the formula's source with it.
    editor.set_render(Render::Basic);
    assert!(!frame(&mut editor, &config).contains(PLACEHOLDER));
    editor.set_render(Render::Full);

    // `j` into the block: its source, the rows where they were.
    for _ in 0..2 {
        editor.on_key(Key::Char('j'));
    }
    let open = frame(&mut editor, &config);
    assert!(!open.contains(PLACEHOLDER), "{open}");
    assert!(open.contains("frac"), "{open}");
    assert_eq!((row_of(&open, "前文"), row_of(&open, "後文")), (before, after), "{open}");
    // And out of it again.
    for _ in 0..2 {
        editor.on_key(Key::Char('j'));
    }
    assert_eq!(picture_rows(&frame(&mut editor, &config)), rows);

    // A broken formula stays source, never an empty block.
    let mut broken = page("前文\n$$\n\\frac{a}{\n$$\n後文", Syntax::Markdown);
    let shown = frame(&mut broken, &config);
    assert!(!shown.contains(PLACEHOLDER) && shown.contains("frac"), "{shown}");

    graphics::set_support(Support::None);
}

/// **Typst's block**, its picture made off the drawing thread: put in by
/// hand here, as the float's test does, since there may be no typst.
#[test]
fn a_typst_block_is_its_picture_once_made() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    graphics::set_support(Support::Kitty);
    let config = Config::default();
    // A frame with no formula settles the ink, and asks typst for nothing
    // that could land over the picture put in below.
    let _ = frame(&mut Editor::new(), &config);
    let mut editor = page("前\n$\n  a + b\n$\n後", Syntax::Typst);
    let ink = match yumete_tui::theme::Palette::of(&config).text() {
        ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
        other => panic!("not RGB: {other:?}"),
    };
    let maths = Maths { source: "a + b".into(), latex: false, display: true };
    let key = yumete_tui::maths::Key { maths, ink, cell_height: graphics::cell_pixels().1 };
    // A PNG header saying 40 × 40 pixels: four cells by two at 10 × 20.
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend(40u32.to_be_bytes());
    png.extend(40u32.to_be_bytes());
    yumete_tui::maths::preload(key, png);
    let shown = frame(&mut editor, &config);
    let (before, after) = (row_of(&shown, "前"), row_of(&shown, "後"));
    let rows = picture_rows(&shown);
    let (columns, tall) = yumete_tui::maths::cells_for((40, 40), graphics::cell_pixels(), (60, 3));
    assert_eq!(rows.len(), tall as usize, "{shown}");
    assert!(rows.iter().all(|&r| r > before && r < after), "{shown}");
    assert_eq!(shown.lines().nth(rows[0]).unwrap().matches(PLACEHOLDER).count(), columns as usize, "{shown}");
    graphics::set_support(Support::None);
}
