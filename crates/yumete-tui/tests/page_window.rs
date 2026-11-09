//! **The page window, drawn**: a page beside the text in
//! placeholder cells, its picture uploaded once, the page number under it;
//! and words instead where no picture can be drawn.
//!
//! A binary of its own: whether pictures can be shown is process-wide
//! (`graphics::set_support`), and a neighbour drawing a frame while this one
//! has it on would find a picture it never asked for.

use yumete_config::Config;
use yumete_core::editor::Editor;
use yumete_core::preview::Jump;
use yumete_ime::{ImeSession, Scheme};
use yumete_tui::graphics::{self, Support, PLACEHOLDER};
use yumete_tui::page_window::{self, Page};

/// A PNG header saying `width` × `height` pixels — all the window reads of it.
fn a_png(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend(width.to_be_bytes());
    png.extend(height.to_be_bytes());
    png.extend([8, 6, 0, 0, 0]);
    png
}

fn frame(editor: &mut Editor, config: &Config) -> String {
    let ime = ImeSession::empty(Scheme::LINGMING);
    yumete_tui::frame_to_text(editor, config, &ime, 80, 24, None)
}

/// The column of the first placeholder on a row, counted in cells.
fn first_placeholder(row: &str) -> Option<usize> {
    let at = row.find(PLACEHOLDER)?;
    Some(yumete_cjk::str_width(&row[..at]))
}

#[test]
fn the_page_is_drawn_beside_the_text_and_uploaded_once() {
    let dir = std::env::temp_dir().join(format!("yumete-page-window-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.typ");
    std::fs::write(&main, "= 第一章\n\n那年冬天。\n").unwrap();
    let mut editor = Editor::new();
    editor.open_file(&main).unwrap();
    let config = Config::default();
    let text = editor.live_window();
    let page = editor.open_page_window(main.clone()).expect("room");
    assert_ne!(page, text);
    let preview = editor.preview_in(page).expect("a preview tab");

    // No picture can be drawn here: the window says so, in words.
    graphics::set_support(Support::None);
    let plain = frame(&mut editor, &config);
    assert!(!plain.contains(PLACEHOLDER), "{plain}");
    assert!(plain.contains(&yumete_core::say!("preview.no-pictures")[..6]), "{plain}");

    // **The tab line and the status line**: the preview's tab
    // by its name; with the keys in it, the status line says that name and
    // nothing of a place, a count or a lock its empty buffer has no use for.
    let label = yumete_core::say!("preview.tab", "main.typ");
    assert!(plain.lines().next().unwrap().contains(&label), "{plain}");
    editor.on_key(yumete_core::input::Key::Ctrl('w'));
    editor.on_key(yumete_core::input::Key::Char('w'));
    let inside = frame(&mut editor, &config);
    let status = inside.lines().rev().nth(1).unwrap();
    assert!(status.trim_end().ends_with(&label), "only the name: {status:?}\n{inside}");
    assert!(!status.contains(yumete_core::say!("ui.readonly-tag").trim()), "{status:?}");
    editor.on_key(yumete_core::input::Key::Ctrl('w'));
    editor.on_key(yumete_core::input::Key::Char('w'));
    assert_eq!(editor.live_window(), text);

    // A terminal that can, and a page made (840 × 1188 px at 144 ppi, page 1
    // of 2), the cursor's text a third of the way down it.
    graphics::set_support(Support::Kitty);
    let _ = graphics::take_pending();
    page_window::preload(
        preview,
        Page::from_png(a_png(840, 1188), 144, 1, Some(2)).unwrap(),
        Some(Jump { page: 1, x: 72.0, y: 200.0 }),
    );
    let shown = frame(&mut editor, &config);
    let rows: Vec<&str> = shown.lines().filter(|r| r.contains(PLACEHOLDER)).collect();
    assert!(rows.len() > 10, "the page fills the window's height: {shown}");
    let column = first_placeholder(rows[0]).unwrap();
    assert!(column >= 40, "in the right half, beside the text: column {column}\n{shown}");
    assert!(shown.contains("1/2"), "the page number under it: {shown}");
    let owed = String::from_utf8(graphics::take_pending()).unwrap();
    assert!(owed.contains("a=T,U=1,f=100"), "uploaded: {owed:?}");
    let id: u32 = owed.split("i=").nth(1).and_then(|s| s.split(',').next()).and_then(|s| s.parse().ok()).unwrap();
    assert!(id & (1 << 23) != 0, "a page's id, not a formula's: {id:#x}");

    // The same page again: nothing sent.
    assert_eq!(frame(&mut editor, &config), shown);
    assert!(graphics::take_pending().is_empty(), "an unchanged page is not uploaded again");

    // The whole page instead: smaller, and the picture placed again.
    editor.execute(":preview-fit page").unwrap();
    let whole = frame(&mut editor, &config);
    let whole_rows = whole.lines().filter(|r| r.contains(PLACEHOLDER)).count();
    assert!(whole_rows <= 22 && whole_rows > 5, "{whole}");
    assert!(!graphics::take_pending().is_empty(), "another size is another upload");

    // Closed: the window goes, and the pictures are freed.
    editor.execute(":preview off").unwrap();
    page_window::clear();
    graphics::drop_pages();
    let owed = String::from_utf8(graphics::take_pending()).unwrap();
    assert!(owed.contains("a=d,d=I"), "{owed:?}");
    assert!(!frame(&mut editor, &config).contains(PLACEHOLDER));

    // **Two previews on the screen at once**: two documents,
    // each page under picture ids of its own; closing one frees its pair
    // and no more.
    editor.execute(":preview-fit width").unwrap();
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("note.typ"), "= 附記\n").unwrap();
    let window = editor.open_page_window(main.clone()).expect("room");
    let one = editor.preview_in(window).unwrap();
    editor.open_file(other.join("note.typ")).unwrap();
    let window = editor.open_page_window(other.join("note.typ")).expect("room");
    let two = editor.preview_in(window).unwrap();
    assert_ne!(one, two);
    assert_eq!(editor.page_windows().len(), 2, "both on the screen");
    let _ = graphics::take_pending();
    page_window::preload(one, Page::from_png(a_png(840, 1188), 144, 1, Some(2)).unwrap(), None);
    page_window::preload(two, Page::from_png(a_png(600, 848), 144, 1, Some(1)).unwrap(), None);
    let both = frame(&mut editor, &config);
    let owed = String::from_utf8(graphics::take_pending()).unwrap();
    let mut ids: Vec<u32> = owed
        .split("a=T,")
        .skip(1)
        .filter_map(|s| s.split("i=").nth(1)?.split(',').next()?.parse().ok())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 2, "two pictures, two ids: {owed:?}\n{both}");
    assert!(ids.iter().all(|id| id & (1 << 23) != 0), "{ids:?}");
    editor.close_preview(one);
    graphics::drop_pages_of(one);
    let owed = String::from_utf8(graphics::take_pending()).unwrap();
    assert_eq!(owed.matches("a=d,d=I").count(), 1, "only the closed one's: {owed:?}");
    assert!(frame(&mut editor, &config).contains(PLACEHOLDER), "the other still drawn");
    page_window::clear();
    graphics::drop_pages();
    std::fs::remove_dir_all(&dir).ok();
}
