//! `:toc` against a real 700-chapter book, when one is on this machine.
//!
//! Skipped automatically when the corpus is not there, the way
//! `yumete-ime/tests/real_data.rs` skips without the installed tables. The
//! chapter rule is guesswork about how books are actually written, and the only
//! honest test of a guess is the books themselves.

use std::path::PathBuf;

use yumete_core::Editor;

/// One of the novels, when it is on this machine.
fn book(name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let path = home
        .join("Library/CloudStorage/Dropbox/Programs/YuhaoInputMethod/assets")
        .join("articles")
        .join(format!("{name}.txt"));
    path.is_file().then_some(path)
}

/// Its outline, or nothing when the book is not here.
fn outline_of(name: &str) -> Option<Vec<(usize, usize, String)>> {
    let path = book(name)?;
    let mut editor = Editor::new();
    editor.open_file(&path).expect("open the book");
    Some(editor.outline())
}

#[test]
fn a_seven_hundred_chapter_book_has_seven_hundred_chapters() {
    let Some(outline) = outline_of("資治通鑑") else {
        eprintln!("skipping: the corpus is not on this machine");
        return;
    };
    // 294 卷, and the outline is that many and not much more: the 目錄 at the
    // top of the file names every one of them and is *not* in it.
    assert!(
        (250..350).contains(&outline.len()),
        "{} headings in a book of 294 卷",
        outline.len()
    );
    // The listing lines — 卷002, 卷003 one after another with nothing under
    // them — are gone, and the ones that are left have writing under them.
    assert!(
        outline.iter().filter(|(line, _, _)| *line < 300).count() <= 1,
        "the 目錄 at the top is still in the outline: {:?}",
        outline.iter().take(4).collect::<Vec<_>>()
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

/// 三國演義, from the same wikisource export, keeps its headings inside the
/// navigation bar: 「◀上一回 第二回　張翼德怒鞭督郵 下一回▶」. Before #402 the
/// outline of a 120-chapter novel was **one row**, and that row was 「序」.
///
/// This copy is the book twice over — the 繁體 text and then the 简体 text —
/// so 240 chapters is the file telling the truth, not a double count.
#[test]
fn a_book_that_kept_its_navigation_bar_has_its_chapters() {
    let Some(outline) = outline_of("三國演義") else {
        eprintln!("skipping: the corpus is not on this machine");
        return;
    };
    assert!(
        (230..260).contains(&outline.len()),
        "{} headings in a file holding 120 回 twice",
        outline.len()
    );
    assert!(
        outline.iter().any(|(_, _, t)| t.starts_with("第一回")),
        "the first chapter is not in {:?}",
        outline.iter().take(4).collect::<Vec<_>>()
    );
    // The arrows are the frame the title was printed in, and they come off.
    assert!(
        outline.iter().all(|(_, _, t)| !t.contains('▶') && !t.contains('◀')),
        "an arrow is still in a title"
    );
}

/// 天龍八部 numbers its fifty chapters and writes no 章 at all: 「一 青衫磊落
/// 險峰行」. Before #402 its outline was one row, and that row was 「后记」.
#[test]
fn a_book_that_only_counts_has_its_chapters() {
    let Some(outline) = outline_of("天龙八部") else {
        eprintln!("skipping: the corpus is not on this machine");
        return;
    };
    // Fifty chapters and the 后记 after them.
    assert_eq!(outline.len(), 51, "{:?}", outline.first());
    assert_eq!(outline[0].2, "一 青衫磊落险峰行");
    assert_eq!(outline[49].2.split(' ').next(), Some("五十"));
    assert_eq!(outline[50].2, "后记");
}

/// 笑傲江湖 is the book that keeps this honest: 39 rows for 40 chapters is
/// **right**. This copy has no 第十二章 and no 第十三章, and writes 第十四章
/// twice running — so the outline is 38 chapters and the 後記 the file opens
/// with. A fix that turns this into 40 has invented two chapters.
#[test]
fn a_defective_copy_is_reported_as_it_is() {
    let Some(outline) = outline_of("笑傲江湖") else {
        eprintln!("skipping: the corpus is not on this machine");
        return;
    };
    assert_eq!(outline.len(), 39, "{:?}", outline.last());
    assert_eq!(outline[0].2, "後記", "it is written at the top of the file");
    assert_eq!(
        outline.iter().filter(|(_, _, t)| t.starts_with("第十四章")).count(),
        1,
        "the same chapter twice running is one chapter"
    );
}

/// 紅樓夢 marks its chapters plainly — 「第一回　甄士隱夢幻識通靈」 — and is
/// here so that a change made for the other books has to leave it alone.
#[test]
fn the_book_that_was_already_right_stays_right() {
    let Some(outline) = outline_of("紅樓夢") else {
        eprintln!("skipping: the corpus is not on this machine");
        return;
    };
    // 120 回 and one line of the 目錄 that gets in (#390).
    assert_eq!(outline.len(), 121, "{:?}", outline.first());
}
