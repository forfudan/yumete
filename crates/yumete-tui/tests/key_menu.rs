//! **The `Space` menu on an 80 × 24 page reads whole**.
//!
//! A binary of its own: the interface language is process-wide.

use yumete_config::Config;
use yumete_core::editor::Editor;
use yumete_core::input::Key;
use yumete_core::messages::{self, Language};
use yumete_ime::{ImeSession, Scheme};

/// A page of prose, the caret on its first line, `Space` half pressed.
fn space_menu_on_a_page(language: Language) -> String {
    messages::set_language(language);
    let mut editor = Editor::new();
    editor.on_key(Key::Char('i'));
    for n in 0..40 {
        for c in format!("Line {n} of a page of prose, long enough to be a line.").chars() {
            editor.on_key(Key::Char(c));
        }
        editor.on_key(Key::Enter);
    }
    editor.on_key(Key::Esc);
    for c in "gg ".chars() {
        editor.on_key(Key::Char(c));
    }
    assert!(editor.pending_menu().is_some(), "Space is half pressed");
    let ime = ImeSession::empty(Scheme::LINGMING);
    yumete_tui::frame_to_text(&mut editor, &Config::default(), &ime, 80, 24, None)
}

/// The rows of the menu's box, top border to bottom border.
fn menu_rows(page: &str) -> Vec<String> {
    let rows: Vec<&str> = page.lines().collect();
    let edge = |corner: char| rows.iter().position(|r| r.contains(corner) && r.matches('─').count() > 10);
    let (top, bottom) = match (edge('╭').or_else(|| edge('┌')), edge('╰').or_else(|| edge('└'))) {
        (Some(top), Some(bottom)) => (top, bottom),
        _ => panic!("no menu on the page:\n{page}"),
    };
    rows[top..=bottom].iter().map(|r| r.to_string()).collect()
}

/// Every meaning of the `Space` menu fits, in Chinese and in English: 43 keys at half the page
/// would be five columns of 「文件挑選…」.
#[test]
fn the_space_menu_is_not_cut_at_80_by_24() {
    for language in [Language::Traditional, Language::English] {
        let page = space_menu_on_a_page(language);
        let menu = menu_rows(&page);
        let cut: Vec<&String> = menu.iter().filter(|r| r.contains('…')).collect();
        assert!(cut.is_empty(), "{language:?}: meanings cut:\n{}", menu.join("\n"));
        // The caret's line is the first, and the menu stays clear of it.
        assert!(!page.lines().next().unwrap_or("").contains('│'), "the menu covers the caret:\n{page}");
    }
}
