//! Continuing a paragraph's wrap across an edit gives the same answer as
//! making it again (#366).
//!
//! The optimisation is allowed to give up; it is not allowed to be wrong. So
//! every test here is the same shape: edit a paragraph somewhere, and compare
//! the rows the continued wrap produced against the rows a from-scratch wrap
//! of the same text produces.

use yumete_core::{wrap, Rope};

/// The rows a paragraph really has, with nothing remembered.
fn truth(text: &str, width: usize) -> Vec<(usize, usize)> {
    wrap::line_rows(text, width)
}

/// The rows the editor gives after an edit, continued where it can be.
fn after_an_edit(
    id: u64,
    text: &str,
    width: usize,
    at: usize,
    insert: &str,
) -> Vec<(usize, usize)> {
    let mut rope = Rope::from_str(text);
    // The answer before the edit, which is what there is to continue from.
    let before = wrap::Measure::plain(width).with_version(id, 1);
    let _ = wrap::rows_of_line_for_test(&rope, 0, before);
    rope.insert(at, insert);
    let after = wrap::Measure::plain(width)
        .with_version(id, 2)
        .with_edit(Some((at, insert.chars().count() as isize)));
    wrap::rows_of_line_for_test(&rope, 0, after)
}

/// **A fresh buffer for every case.** What is remembered is keyed by the
/// buffer and the revision, so two cases that call themselves buffer 1 at
/// revisions 1 and 2 hand each other their row lists — which is a test lying,
/// and it lied convincingly enough to look like a bug in the code.
fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[test]
fn the_continued_wrap_says_what_a_fresh_one_says() {
    let width = 20;
    for body in [
        // 中文: breaks fall wherever they like, so the tail's boundaries stay
        // on their grid and its content slides under them.
        "甲乙丙丁戊己庚辛".repeat(60),
        // Prose: breaks fall on spaces, so an inserted character pushes the
        // whole tail along — this is where the old wrap comes back.
        "the quick brown fox jumps over the lazy dog ".repeat(20),
        // Mixed, and with punctuation 禁則 has opinions about.
        "他說「好」，然後 walked away 了。".repeat(30),
    ] {
        for at in [0usize, 1, 17, 40, 41, 199, body.chars().count() / 2] {
            let at = at.min(body.chars().count());
            for insert in ["乙", "x", "  ", "、"] {
                let mut want = body.clone();
                let byte = want
                    .char_indices()
                    .nth(at)
                    .map(|(b, _)| b)
                    .unwrap_or(want.len());
                want.insert_str(byte, insert);
                assert_eq!(
                    after_an_edit(next_id(), &body, width, at, insert),
                    truth(&want, width),
                    "inserting {insert:?} at {at} of {:?}…",
                    body.chars().take(12).collect::<String>()
                );
            }
        }
    }
}

/// A removal is an edit too, and its delta is the other way.
#[test]
fn a_removal_is_continued_the_same_way() {
    let width = 16;
    let body = "the quick brown fox jumps over the lazy dog ".repeat(20);
    for at in [3usize, 30, 100] {
        let mut rope = Rope::from_str(&body);
        let id = next_id();
        let before = wrap::Measure::plain(width).with_version(id, 1);
        let _ = wrap::rows_of_line_for_test(&rope, 0, before);
        rope.remove(at..at + 4);
        let after = wrap::Measure::plain(width)
            .with_version(id, 2)
            .with_edit(Some((at, -4)));
        let got = wrap::rows_of_line_for_test(&rope, 0, after);
        let mut want_text = body.clone();
        let from = want_text.char_indices().nth(at).unwrap().0;
        let upto = want_text.char_indices().nth(at + 4).unwrap().0;
        want_text.replace_range(from..upto, "");
        assert_eq!(got, truth(&want_text, width), "removing four at {at}");
    }
}
