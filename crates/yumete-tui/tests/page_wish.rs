//! **`:yume-menu-size 3` says 3, on the frame where it was typed.**
//!
//! The size the user asks for is clamped to the window by
//! [`yumete_tui::fit_the_page`], which runs before the *next* frame — and the
//! status line is written now. So the command used to report the size it was
//! replacing: 2026-10-19 `:yume-menu-size 3` answered 「候選面板一頁 6 條」,
//! and only a second, bare `:yume-menu-size` said 3.
//!
//! Warning: **its own test binary.** 「每頁幾個」 is a process-global
//! (`PAGE_WISH`, and now the window height beside it), so a test that sets it
//! beside two hundred others is a test that sets it *under* them.

use yumete_config::Config;
use yumete_core::{Editor, Key};
use yumete_ime::{ImeSession, Scheme};

/// A session, an editor in insert mode, and the factory config.
fn bench() -> (ImeSession, Editor, Config) {
    let mut editor = Editor::new();
    editor.on_key(Key::Char('i'));
    let ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八 把 爸 罷\n");
    (ime, editor, Config::default())
}

/// What the command answers is what the next frame draws — the whole claim.
fn asks_for(n: usize, width: u16, height: u16) -> (String, usize, usize) {
    let (mut ime, mut editor, config) = bench();
    yumete_tui::fit_the_page(&mut ime, &mut editor, &config, width, height);
    let said = yumete_tui::settings_request(&mut ime, &format!("menu:{n}"))
        .expect("the session answers 「每頁幾個」 itself");
    let at_once = ime.page_size();
    yumete_tui::fit_the_page(&mut ime, &mut editor, &config, width, height);
    (said, at_once, ime.page_size())
}

#[test]
fn the_size_it_reports_is_the_size_the_next_frame_draws() {
    // Tall enough for anything: the answer is simply the number asked for.
    let (said, at_once, next_frame) = asks_for(3, 80, 24);
    assert_eq!(at_once, 3, "said {said:?}");
    assert_eq!(next_frame, 3, "the frame after changes nothing");
    assert!(said.contains('3'), "the report names it: {said:?}");

    // Short: the window wins, and the report says the number that will be
    // drawn rather than the one that was asked for.
    let (said, at_once, next_frame) = asks_for(9, 80, 6);
    assert!(at_once < 9, "a six-row window cannot hold nine: {said:?}");
    assert_eq!(at_once, next_frame, "said {said:?}");
    assert!(said.contains(&next_frame.to_string()), "{said:?}");
}

/// Asked rather than set, it still reports — and still does not move.
#[test]
fn the_bare_question_moves_nothing() {
    let (mut ime, mut editor, config) = bench();
    yumete_tui::fit_the_page(&mut ime, &mut editor, &config, 80, 24);
    let before = ime.page_size();
    let said = yumete_tui::settings_request(&mut ime, "menu:").expect("an answer");
    assert_eq!(ime.page_size(), before, "{said:?}");
    assert!(said.contains(&before.to_string()), "{said:?}");
}
