//! **The Markdown preview, drawn** (2026-11-08): a Markdown file rendered
//! beside the text as one picture the window's size, following the cursor,
//! scrolled by its own keys, laid out again once the typing settles, and
//! asked for nothing when nothing changed.
//!
//! A binary of its own: whether pictures can be shown is process-wide
//! (`graphics::set_support`).

use std::time::{Duration, Instant};

use yumete_config::Config;
use yumete_core::buffer::PreviewKind;
use yumete_core::editor::Editor;
use yumete_core::input::Key;
use yumete_ime::{ImeSession, Scheme};
use yumete_tui::graphics::{self, Support, PLACEHOLDER};
use yumete_tui::md_window::{self, MdPreviews, Top};

/// One test at a time: they set the process-wide picture support.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn frame(editor: &mut Editor, config: &Config) -> String {
    let ime = ImeSession::empty(Scheme::LINGMING);
    yumete_tui::frame_to_text(editor, config, &ime, 100, 30, None)
}

/// Turns of the loop, a frame each, until `done` or 30 seconds.
fn until(editor: &mut Editor, config: &Config, md: &mut MdPreviews, mut done: impl FnMut(&MdPreviews) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        frame(editor, config);
        md.follow(editor, config);
        if done(md) {
            return;
        }
        assert!(Instant::now() < deadline, "never came");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The top of the picture preview `id` shows now.
fn top_of(id: u64) -> Option<f32> {
    md_window::shot_of(id).map(|s| s.top)
}

fn a_long_note(dir: &std::path::Path) -> std::path::PathBuf {
    let mut text = String::from("# 筆記 Notes\n\n");
    for k in 0..80 {
        text.push_str(&format!("第 {k} 段：那年冬天，雪下得很大。The quick brown fox jumps over the lazy dog.\n\n"));
    }
    let main = dir.join("notes.md");
    std::fs::write(&main, text).unwrap();
    main
}

#[test]
fn a_markdown_preview_is_drawn_and_follows_the_cursor() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("yumete-md-window-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = a_long_note(&dir);
    let mut editor = Editor::new();
    editor.open_file(&main).unwrap();
    let config = Config::default();
    let mut md = MdPreviews::default();
    let text = editor.live_window();
    let page = editor.open_page_window_of(main.clone(), PreviewKind::Markdown).expect("room");
    let preview = editor.preview_in(page).unwrap();

    // **No pictures here**: said in words; `:view-preview` would go to the
    // browser for the reason `why_not` gives.
    graphics::set_support(Support::None);
    let plain = frame(&mut editor, &config);
    assert!(!plain.contains(PLACEHOLDER));
    assert!(plain.contains(&yumete_core::say!("preview.no-pictures")[..6]), "{plain}");
    assert_eq!(md_window::why_not(), Some(yumete_core::say!("preview.no-pictures")));

    // A terminal that can: the first picture is the window's size exactly.
    graphics::set_support(Support::Kitty);
    assert_eq!(md_window::why_not(), None);
    let _ = graphics::take_pending();
    until(&mut editor, &config, &mut md, |_| md_window::shot_of(preview).is_some());
    let (columns, rows) = md_window::cells_of(preview).unwrap();
    let shot = md_window::shot_of(preview).unwrap();
    assert_eq!((shot.columns, shot.rows), (columns, rows));
    let cell = graphics::cell_pixels();
    let decoded = png_size(&shot.png);
    assert_eq!(decoded, (columns as u32 * cell.0 as u32, rows as u32 * cell.1 as u32), "made at the window's pixels");
    assert_eq!(shot.top, 0.0, "the cursor is on the first line");
    let shown = frame(&mut editor, &config);
    let drawn: Vec<&str> = shown.lines().filter(|r| r.contains(PLACEHOLDER)).collect();
    assert!(drawn.len() as u16 >= rows.saturating_sub(1), "the picture fills the window: {shown}");
    let column = drawn[0].find(PLACEHOLDER).map(|at| yumete_cjk::str_width(&drawn[0][..at])).unwrap();
    assert!(column >= 45, "beside the text: column {column}");
    assert!(String::from_utf8(graphics::take_pending()).unwrap().contains("a=T"), "uploaded");

    // **Nothing changed, nothing asked** — the feedback loop the Typst page
    // once had: frames and turns go by and the worker is sent nothing new,
    // and the same picture is not uploaded again.
    let sent = md.jobs_sent(preview);
    for _ in 0..5 {
        frame(&mut editor, &config);
        md.follow(&mut editor, &config);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(md.jobs_sent(preview), sent, "asked again with nothing changed");
    assert!(graphics::take_pending().is_empty(), "an unchanged picture is not sent twice");

    // **The cursor followed**: line 100 a third of the way down.
    editor.execute(":100").unwrap();
    until(&mut editor, &config, &mut md, |_| top_of(preview).is_some_and(|t| t > 0.0));
    let far = top_of(preview).unwrap();
    editor.execute(":60").unwrap();
    until(&mut editor, &config, &mut md, |_| top_of(preview).is_some_and(|t| t < far));
    let nearer = top_of(preview).unwrap();
    assert!(nearer > 0.0 && nearer < far, "{nearer} between 0 and {far}");
    assert_eq!(md.sent(preview).map(|w| w.top), Some(Top::Line(59)));

    // **Typing waits for a pause**: the edit is not laid out at once, but
    // after the settle, and the loop is told when to wake.
    let before = md.jobs_sent(preview);
    editor.on_key(Key::Char('A'));
    for c in "改".chars() {
        editor.on_key(Key::Char(c));
    }
    editor.on_key(Key::Esc);
    frame(&mut editor, &config);
    md.follow(&mut editor, &config);
    assert_eq!(md.jobs_sent(preview), before, "not at once");
    assert!(md.due_in().is_some_and(|d| d <= md_window::SETTLE));
    std::thread::sleep(md_window::SETTLE + Duration::from_millis(20));
    md.follow(&mut editor, &config);
    assert_eq!(md.jobs_sent(preview), before + 1, "after the pause");

    // **Its keys scroll it**, a row a cell tall; `J` does nothing; `G` the
    // end and `gg` the top. The cursor in the text stays where it was.
    editor.on_key(Key::Ctrl('w'));
    editor.on_key(Key::Char('w'));
    assert!(editor.page_window_has_the_keys());
    until(&mut editor, &config, &mut md, |m| m.sent(preview).is_some_and(|w| matches!(w.top, Top::Line(_)))
        && md_window::shot_of(preview).is_some());
    let at = top_of(preview).unwrap();
    editor.on_key(Key::Char('j'));
    md.follow(&mut editor, &config);
    assert_eq!(md.sent(preview).map(|w| w.top), Some(Top::At((at + cell.1 as f32) as i64)));
    let after_j = md.sent(preview).map(|w| w.top);
    editor.on_key(Key::Char('J'));
    md.follow(&mut editor, &config);
    assert_eq!(md.sent(preview).map(|w| w.top), after_j, "`J` has no page to turn to");
    editor.on_key(Key::Char('g'));
    editor.on_key(Key::Char('g'));
    md.follow(&mut editor, &config);
    assert_eq!(md.sent(preview).map(|w| w.top), Some(Top::At(0)));
    editor.on_key(Key::Char('G'));
    md.follow(&mut editor, &config);
    let Some(Top::At(bottom)) = md.sent(preview).map(|w| w.top) else { panic!() };
    assert!(bottom > far as i64, "the end: {bottom}");
    editor.on_key(Key::Ctrl('w'));
    editor.on_key(Key::Char('w'));
    assert_eq!(editor.live_window(), text);

    // **A window made narrower**: the next picture is its new size.
    let (old_columns, _) = md_window::cells_of(preview).unwrap();
    let ime = ImeSession::empty(Scheme::LINGMING);
    let _ = yumete_tui::frame_to_text(&mut editor, &config, &ime, 80, 30, None);
    let (new_columns, _) = md_window::cells_of(preview).unwrap();
    assert!(new_columns < old_columns);
    let deadline = Instant::now() + Duration::from_secs(30);
    while md_window::shot_of(preview).is_none_or(|s| s.columns != new_columns) {
        let _ = yumete_tui::frame_to_text(&mut editor, &config, &ime, 80, 30, None);
        md.follow(&mut editor, &config);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }

    // **Closed**: its pictures freed.
    editor.execute(":view-preview off").unwrap();
    let _ = graphics::take_pending();
    md.follow(&mut editor, &config);
    assert!(md_window::shot_of(preview).is_none());
    assert!(String::from_utf8(graphics::take_pending()).unwrap().contains("a=d,d=I"), "freed");
    md_window::clear();
    graphics::drop_pages();
    std::fs::remove_dir_all(&dir).ok();
}

/// **A Typst preview and a Markdown one**: the Markdown previews draw their
/// own and leave the Typst one to tinymist.
#[test]
fn the_markdown_previews_leave_a_typst_one_alone() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("yumete-md-window-two-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = a_long_note(&dir);
    let typ = dir.join("main.typ");
    std::fs::write(&typ, "= 一\n").unwrap();
    let mut editor = Editor::new();
    editor.open_file(&typ).unwrap();
    editor.open_page_window(typ.clone()).unwrap();
    editor.open_file(&main).unwrap();
    editor.open_page_window(main.clone()).unwrap();
    let typst = editor.preview_of(&typ).unwrap();
    let markdown = editor.preview_of(&main).unwrap();
    let config = Config::default();
    let mut md = MdPreviews::default();
    graphics::set_support(Support::Kitty);
    until(&mut editor, &config, &mut md, |_| md_window::shot_of(markdown).is_some());
    assert_eq!(md.jobs_sent(typst), 0);
    assert!(md_window::shot_of(typst).is_none());
    md_window::clear();
    graphics::drop_pages();
    std::fs::remove_dir_all(&dir).ok();
}

fn png_size(png: &[u8]) -> (u32, u32) {
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    (w, h)
}
