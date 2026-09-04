//! `:toc` against a real 700-chapter book, when one is on this machine.
//!
//! Skipped automatically when the corpus is not there, the way
//! `yumete-ime/tests/real_data.rs` skips without the installed tables. The
//! chapter rule is guesswork about how books are actually written, and the only
//! honest test of a guess is the books themselves.

use std::path::PathBuf;

use yumete_core::Editor;

/// 資治通鑑: 294 卷, not one `#`, and every one of them written `卷002`.
fn book() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let path = home
        .join("Library/CloudStorage/Dropbox/Programs/YuhaoInputMethod/assets")
        .join("articles/資治通鑑.txt");
    path.is_file().then_some(path)
}

#[test]
fn a_seven_hundred_chapter_book_has_seven_hundred_chapters() {
    let Some(path) = book() else {
        eprintln!("skipping: the corpus is not on this machine");
        return;
    };
    let mut editor = Editor::new();
    editor.open_file(&path).expect("open 資治通鑑");
    let outline = editor.outline();
    assert!(
        outline.len() > 250,
        "only {} headings in a book of 294 卷",
        outline.len()
    );
    // The unit comes first and there is no 第 — the spelling that made the
    // first version of this find twelve chapters in the whole book.
    assert!(
        outline.iter().any(|(_, _, title)| title.starts_with("卷")),
        "no 卷 among {:?}",
        outline.iter().take(5).collect::<Vec<_>>()
    );
    // …and prose is not a heading: 資治通鑑 is nothing but 「臣光曰」 paragraphs.
    assert!(
        outline.iter().all(|(_, _, title)| title.chars().count() <= 40),
        "something long got in"
    );
}
