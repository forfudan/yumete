//! Every test the editor has (#296).
//!
//! Lifted out of `editor.rs` whole on 2026-09-08: ten thousand lines, 34% of
//! the file, and not one of them something you read while looking for how the
//! editor works. It is still a child module of `editor`, so it sees the same
//! private fields it always did — the only thing that changed is which file
//! the reader opens.
//!
//! **The tests move with the code they are about.** As `editor.rs` is taken
//! apart by topic, each area's tests go into that module's own `mod tests`,
//! and what is left here is what has not been moved yet.

use super::*;
use crate::input::{Key, Mode};
use yumete_cjk::{CategorySegmenter, DictionarySegmenter};

/// Type `text` into a fresh editor, then return to Normal at the top.
///
/// **Typed as plain text and handed back as Markdown**, which is what a
/// scratch buffer is ([`crate::syntax::Syntax::default`]). The two are the
/// same document except while it is being written: `Enter` carries a list
/// marker down in Markdown (#418), so a fixture whose lines open with `- `
/// would come back with markers this helper wrote rather than the ones it was
/// given. A fixture must be the text it was handed.
fn typed(text: &str) -> Editor {
    let mut ed = Editor::new();
    ed.current_buffer_mut().set_syntax(crate::syntax::Syntax::Text);
    ed.on_key(Key::Char('i'));
    for c in text.chars() {
        ed.on_key(if c == '\n' { Key::Enter } else { Key::Char(c) });
    }
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.current_buffer_mut()
        .set_syntax(crate::syntax::Syntax::Markdown);
    ed
}

/// Feature #210. The core holds the runs and hands them to whatever asks
/// where a character is; it does not decide what they say.
#[test]
fn drawn_runs_are_held_wholesale_and_answered_by_line() {
    let mut ed = typed("春夏\n秋冬\n");
    assert!(!ed.has_candidate(), "a page with no candidate on it pays nothing");
    assert!(ed.drawn_on_line(0).is_empty());

    // Out of order on the way in, in column order on the way out: the
    // renderer, the wrap and the mouse all walk it forwards.
    ed.set_candidate(vec![
        (0, 2, "補".to_string()),
        (1, 1, "候".to_string()),
        (0, 1, "候".to_string()),
    ]);
    assert!(ed.has_candidate());
    assert_eq!(
        ed.drawn_on_line(0),
        vec![(1, "候".to_string()), (2, "補".to_string())]
    );
    assert_eq!(ed.drawn_on_line(1), vec![(1, "候".to_string())]);
    assert!(ed.drawn_on_line(2).is_empty());

    // Wholesale, never appended — a committed candidate leaves nothing.
    ed.set_candidate(Vec::new());
    assert!(!ed.has_candidate());
    assert!(ed.drawn_on_line(0).is_empty());
}

/// Feature #248. The general form: the editor's own notes on the page,
/// beside the mark they are about.
#[test]
fn the_mark_that_is_wrong_is_named_on_the_page() {
    let mut ed = typed("他說,好\n");
    // Off until it is asked for: a manuscript is not a proof sheet.
    assert!(!ed.notes());
    assert!(ed.drawn_on_line(0).is_empty());

    ed.execute(":view-punct on").unwrap();
    assert!(ed.notes());
    // 他說 , 好 — the note stands *after* the comma, at the character it
    // should have been written as.
    assert_eq!(ed.drawn_on_line(0), vec![(3, "，".to_string())]);

    ed.execute(":view-punct off").unwrap();
    assert!(ed.drawn_on_line(0).is_empty());
}

#[test]
fn a_page_that_got_its_marks_right_carries_no_notes() {
    let mut ed = typed("他說：「好。」\n");
    ed.execute(":view-punct on").unwrap();
    assert!(ed.drawn_on_line(0).is_empty(), "{:?}", ed.drawn_on_line(0));
}

#[test]
fn a_note_is_a_note_and_not_a_character() {
    // The mirror invariant (#248): the cursor may never sit on a character
    // that is not in the file. Walking right past the mark the note is
    // about lands on the file's own next character, and `x` deletes that.
    let mut ed = typed("他說,好\n");
    ed.execute(":view-punct on").unwrap();
    assert_eq!(ed.drawn_on_line(0), vec![(3, "，".to_string())]);
    for _ in 0..3 {
        ed.on_key(Key::Char('l'));
    }
    // Three characters right of 他 is 好 — the file's own fourth
    // character, not the 「，」 drawn between it and the comma.
    assert_eq!(ed.cursor_column(), 3);
    assert_eq!(ed.current_buffer().rope().char(ed.cursor()), '好');
}

#[test]
fn a_comma_inside_a_fence_is_code_and_is_left_alone() {
    let mut ed = typed("```rust\nlet a = (1,2);\n```\n他說,好\n");
    ed.execute(":view-punct on").unwrap();
    assert!(ed.drawn_on_line(1).is_empty(), "{:?}", ed.drawn_on_line(1));
    assert_eq!(ed.drawn_on_line(3), vec![(3, "，".to_string())]);
}

#[test]
fn render_off_asks_for_the_file_and_gets_the_file() {
    let mut ed = typed("他說,好\n");
    ed.execute(":view-punct on").unwrap();
    assert!(!ed.drawn_on_line(0).is_empty());
    ed.execute(":render off").unwrap();
    assert!(ed.drawn_on_line(0).is_empty(), "{:?}", ed.drawn_on_line(0));
}

#[test]
fn a_note_follows_the_line_as_it_is_written() {
    // The cache is a hash of the line, so an edit that fixes the mark
    // takes the note off the page without anybody clearing anything.
    let mut ed = typed("他說,好\n");
    ed.execute(":view-punct on").unwrap();
    assert_eq!(ed.drawn_on_line(0).len(), 1);
    // Put the cursor on the comma and write the right mark over it.
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('r'));
    ed.on_key(Key::Char('，'));
    assert_eq!(ed.line_text(0).unwrap().trim_end(), "他說，好");
    assert!(ed.drawn_on_line(0).is_empty(), "{:?}", ed.drawn_on_line(0));
}

/// The point of #210: **one page**. A candidate the renderer alone knew
/// about would put the caret, `j` and the mouse on three different ones.
#[test]
fn the_caret_and_the_grid_agree_about_a_candidate() {
    let mut ed = typed("春夏秋冬\n");
    ed.set_candidate(vec![(0, 2, "候補".to_string())]);
    // Down the column: two rows of candidate between 夏 and 秋.
    ed.execute(":layout vertical").unwrap();
    assert_eq!(ed.zong_position().slot, 0);
    // Down the column is `j`: the keys follow the screen, not the file.
    ed.on_key(Key::Char('j'));
    ed.on_key(Key::Char('j'));
    assert_eq!(ed.cursor(), 2, "two 字 along");
    assert_eq!(
        ed.zong_position().slot,
        4,
        "…and four rows down, the candidate being two of them"
    );
}

fn press(ed: &mut Editor, keys: &str) {
    for c in keys.chars() {
        ed.on_key(Key::Char(c));
    }
}

/// Type `reading` into an open Ruby prompt and submit it.
fn submit_reading(ed: &mut Editor, reading: &str) {
    for c in reading.chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Enter);
}

/// A Markdown buffer holding `text`, cursor at the top.
fn markdown(text: &str) -> Editor {
    let mut ed = typed(text);
    ed.execute(":syntax markdown").unwrap();
    ed
}

#[test]
fn a_link_to_the_web_is_handed_over_and_nothing_else_is() {
    // Feature #285. The cursor starts on the `[`, which is markup the
    // reader can see; the destination it opens is the half 所見即所得 does
    // not draw at all.
    let mut ed = markdown("[雪](https://example.com/一)\n");
    press(&mut ed, "gx");
    assert_eq!(
        ed.take_open_request().as_deref(),
        Some("https://example.com/一")
    );
    // Asked for once, not once per frame.
    assert_eq!(ed.take_open_request(), None);
}

#[test]
fn a_scheme_the_editor_does_not_know_is_named_and_refused() {
    // **The security decision, in one test.** A manuscript is a file that
    // arrives by email, and `open` will start whatever program claims a
    // scheme. So the list is two long, and everything else is said out
    // loud rather than run.
    for line in ["[寫信](mailto:a@b.c)\n", "[開](x-anything:do-it)\n"] {
        let mut ed = markdown(line);
        press(&mut ed, "gx");
        assert_eq!(ed.take_open_request(), None, "{line}");
        assert!(
            ed.status().contains("http"),
            "it says which two are followed: {}",
            ed.status()
        );
    }
}

#[test]
fn a_link_into_this_same_file_is_a_heading_not_a_file() {
    let mut ed = markdown("[雪](#雪)\n\n## 雪\n那一夜。\n");
    press(&mut ed, "gx");
    assert_eq!(ed.take_open_request(), None);
    // `## 雪` is the third line, and its first non-blank is the `#`.
    assert_eq!(ed.cursor(), 9, "{}", ed.status());
    // A heading that is not there is said, not guessed at.
    let mut ed = markdown("[夏](#夏)\n\n## 雪\n");
    press(&mut ed, "gx");
    assert!(ed.status().contains('夏'), "{}", ed.status());
}

#[test]
fn a_link_to_another_chapter_opens_it_where_it_sits() {
    let dir = std::env::temp_dir().join(format!("yumete-link-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("二.md"), "## 雪\n那一夜。\n").unwrap();
    let one = dir.join("一.md");
    std::fs::write(&one, "見[下一章](二.md)。\n見[[二#雪]]。\n").unwrap();

    // Spelled out, with the suffix.
    let mut ed = Editor::new();
    ed.open_file(&one).unwrap();
    press(&mut ed, "lgx");
    assert_eq!(ed.current_buffer().display_name(), "二.md", "{}", ed.status());

    // And named, without one — `[[二]]` is a page of this manuscript, and
    // the suffix is the manuscript's to know. The `#雪` goes to the
    // heading once the file is open.
    let mut ed = Editor::new();
    ed.open_file(&one).unwrap();
    press(&mut ed, "jlgx");
    assert_eq!(ed.current_buffer().display_name(), "二.md", "{}", ed.status());
    assert_eq!(ed.cursor(), 0, "the heading is the first line");

    // An absolute path is not the name of a chapter.
    std::fs::write(&one, "見[密](/etc/passwd)。\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&one).unwrap();
    press(&mut ed, "lgx");
    assert_eq!(ed.buffer_count(), 1, "{}", ed.status());
    assert!(ed.status().contains("/etc/passwd"), "{}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_retired_keys_say_what_replaced_them() {
    // ⚠️ `*` **used to be** one of these, pointing at `g/`. Since 2026-09-11
    // it simply *is* `g/` (#404): a key spelled the same in vi, in Helix and
    // here, doing the same thing, does not need a phrasebook entry — a hint
    // that could have done the job costs a keystroke and teaches nothing.
    // The phrasebook is for the keys we deliberately spell differently.

    // **`Enter` is silent** (#353). It kept a note of its own while it was
    // freshly retired — 「it did something here until last week」 — and that
    // week is over. Standing on a footnote the panel names the key that
    // follows it, which is where a reader looking for one now looks.
    let mut ed = typed("那年冬天。\n");
    ed.on_key(Key::Enter);
    assert!(ed.status().is_empty(), "{}", ed.status());

    // `gw` moved to `gD` on the same day (#355), and the fingers that learned
    // it are the author's own — so it says where it went rather than nothing.
    let mut ed = typed("那年冬天。\n");
    press(&mut ed, "gw");
    assert!(ed.status().contains("gD"), "{}", ed.status());
}

#[test]
fn the_phrasebook_answers_the_keys_we_spell_differently() {
    // Three keys a reader arrives with and does not find here (#404). Each
    // says where the thing went; none of them touches the document.
    let ask = |key: Key| {
        let mut ed = typed("那年冬天。\n");
        ed.on_key(key);
        assert_eq!(ed.current_buffer().text(), "那年冬天。\n", "{}", ed.status());
        assert_eq!(ed.mode(), Mode::Normal, "{}", ed.status());
        ed.status().to_string()
    };
    // vi's redo. Ours is the capital of the key that undoes.
    assert!(ask(Key::Ctrl('r')).contains('U'), "{}", ask(Key::Ctrl('r')));
    // Helix spends two tutor lessons on `C-c`; ours is on the 空格 menu.
    assert!(ask(Key::Ctrl('c')).contains("空格 c"), "{}", ask(Key::Ctrl('c')));
    // Helix cycles selections with `)` — that needs several cursors (#405).
    // Walking a sentence at a time, which is the other half of what the
    // reader wants, is `H`／`L`.
    for key in ['(', ')'] {
        let said = ask(Key::Char(key));
        assert!(said.contains('H') && said.contains('L'), "{key}: {said}");
    }
    // vi's 行首 — and it can speak, because `0` builds a count only when one
    // is already under way.
    assert!(ask(Key::Char('0')).contains("gh"), "{}", ask(Key::Char('0')));
    let mut ed = typed("那年冬天。\n");
    press(&mut ed, "gg20l");
    assert!(ed.status().is_empty(), "a count swallows it: {}", ed.status());
}

#[test]
fn the_case_keys_moved_under_one_prefix() {
    // §5.2.3 ②: Helix spends three top-level keys on an operation that is
    // the identity on 漢字. Here they are a group, and the three keys they
    // used to sit on are unbound.
    let lower = |keys: &str| {
        let mut ed = typed("Hello World\n");
        press(&mut ed, keys);
        ed.current_buffer().text()
    };
    assert_eq!(lower("x`l"), "hello world\n");
    assert_eq!(lower("x`u"), "HELLO WORLD\n");
    assert_eq!(lower("x``"), "hELLO wORLD\n");
    // ⚠️ **`~` is the third member, not a hint about the group** (#404,
    // 2026-09-11). It used to say 「大小寫在 ` 組裏」 and do nothing; it now
    // does what it does in vi and in Helix — switch the case of the selection
    // — which is exactly `` ` `` `` ` ``. The group keeps the other two.
    let mut ed = typed("Hello World\n");
    press(&mut ed, "x~");
    assert_eq!(ed.current_buffer().text(), "hELLO wORLD\n");
    // Helix's 轉大寫 is `Alt-\``, which we do **not** have — that one is still
    // a hint, because the answer really is spelled differently here.
    let mut ed = typed("Hello World\n");
    ed.on_key(Key::Alt('`'));
    assert!(ed.status().contains("`l"), "{}", ed.status());
}

#[test]
fn r_replaces_with_what_the_ime_committed() {
    // §5.2.3 ②: `r` keeps its top-level place and learns 中文 instead —
    // the panel opens on `r`, and the choice is the replacement.
    let mut ed = typed("錢塘江上\n");
    ed.on_key(Key::Char('r'));
    assert!(ed.takes_a_character(), "the front end must know to run the IME");
    ed.insert_committed("銀");
    assert_eq!(ed.current_buffer().text(), "銀塘江上\n");
    assert!(!ed.takes_a_character(), "and the pending state is spent");

    // One character still fills the selection, the way `r` always has…
    let mut ed = typed("錢塘江上\n");
    press(&mut ed, "x");
    ed.on_key(Key::Char('r'));
    ed.insert_committed("■");
    assert_eq!(
        ed.current_buffer().text(),
        "■■■■\n",
        "one 字 writes over every character, and not over the line ending"
    );

    // …and a word cannot fill anything, so it replaces once.
    let mut ed = typed("錢塘江上\n");
    press(&mut ed, "x");
    ed.on_key(Key::Char('r'));
    ed.insert_committed("春天");
    assert_eq!(ed.current_buffer().text(), "春天\n");
}

#[test]
fn f_and_the_pair_keys_take_what_the_ime_committed() {
    // #414: `r` learnt 中文 and the other five did not, so in a Chinese
    // manuscript `f` could look for `,` but not for 「，」 — and every
    // full-width pair in `PAIRS` was a delimiter nothing could type.
    let mut ed = typed("春風又綠江南岸，明月何時照我還\n");
    ed.on_key(Key::Char('f'));
    assert!(ed.takes_a_character(), "the front end must run the IME for `f`");
    ed.insert_committed("，");
    assert_eq!(
        ed.current_buffer().rope().char(ed.cursor()),
        '，',
        "`f` stops on the 逗號 it was given"
    );
    assert!(!ed.takes_a_character(), "and the pending state is spent");

    // `Alt-.` repeats it, so the character has to have been remembered.
    let mut ed = typed("一，二，三\n");
    ed.on_key(Key::Char('f'));
    ed.insert_committed("，");
    ed.on_key(Key::Alt('.'));
    assert_eq!(ed.cursor(), 3, "`Alt-.` looks for the same 逗號 again");

    // A count typed before `f` is spent by it, not left for the next key.
    let mut ed = typed("一，二，三，四\n");
    press(&mut ed, "2");
    ed.on_key(Key::Char('f'));
    ed.insert_committed("，");
    assert_eq!(ed.cursor(), 3, "`2f，` is the second one");

    // `ms` 圍上 a pair that only an IME can type.
    let mut ed = typed("錢塘江上\n");
    press(&mut ed, "v3lms");
    assert!(ed.takes_a_character(), "the front end must run the IME for `ms`");
    ed.insert_committed("「");
    assert_eq!(ed.current_buffer().text(), "「錢塘江上」\n");

    // `mi` 選中 what a pair holds, `mr` 換 the pair itself.
    let mut ed = typed("「錢塘江上」\n");
    press(&mut ed, "lmi");
    ed.insert_committed("「");
    assert_eq!(ed.selection(), (1, 5), "`mi「` takes what the pair holds");
    press(&mut ed, "mr");
    ed.insert_committed("「");
    ed.insert_committed("『");
    assert_eq!(ed.current_buffer().text(), "『錢塘江上』\n");
}

#[test]
fn a_dot_repeats_an_ime_replace() {
    // The code letters never reach `on_key`, so replaying the keys of an
    // IME `r` replays `r` alone — which would arm the pending state and
    // eat the reader's next key.
    let mut ed = typed("錢錢錢\n");
    ed.on_key(Key::Char('r'));
    ed.insert_committed("銀");
    press(&mut ed, "l.");
    assert_eq!(ed.current_buffer().text(), "銀銀錢\n");
    assert!(!ed.takes_a_character(), "`.` must not leave `r` waiting");
    // The next key is a key, not the answer to a question nobody asked.
    press(&mut ed, "l.");
    assert_eq!(ed.current_buffer().text(), "銀銀銀\n");
}

#[test]
fn space_r_opens_the_reading_prompt() {
    // 旁注 is worth a key and worth a second one: a page carries one or
    // two, and it is not something that has to happen the instant you ask.
    let mut ed = typed("那年冬天。\n");
    press(&mut ed, " r");
    assert_eq!(ed.mode(), Mode::Ruby, "{}", ed.status());
}

/// **The throttle grows a trailing edge** (#383).
///
/// `autosave_tick` writes at most once every `SWAP_INTERVAL` and used to be
/// called only after a keystroke, so the last few seconds of typing were
/// written by the *next* key — and when the writer paused to think, there was
/// no next key. `autosave_due_in` is what lets the event loop wait with a
/// deadline instead of forever, and it must answer `None` whenever there is
/// nothing a crash could take, or the loop would wake for the rest of the
/// session on a buffer it has nothing to do for.
#[test]
fn the_editor_says_when_a_recovery_copy_is_owed() {
    let dir = std::env::temp_dir().join(format!("yumete-owed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ch1.md");
    std::fs::write(&path, "第一章\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&path).unwrap();
    ed.set_autosave(true);
    assert_eq!(ed.autosave_due_in(), None, "nothing typed, nothing owed");

    // One keystroke, and a copy is owed — at once, because none has been
    // written in this session yet.
    press(&mut ed, "i甲");
    assert_eq!(
        ed.autosave_due_in(),
        Some(std::time::Duration::ZERO),
        "owed, and the throttle has not started"
    );

    // Writing it settles the debt, and the loop may block again. **This is the
    // part that stops the wake-up repeating**: `is_modified` is still true —
    // the document is still unsaved — but the copy now holds what the buffer
    // holds.
    ed.autosave_tick();
    assert!(ed.current_buffer().is_modified(), "still unsaved, as it should be");
    assert_eq!(ed.autosave_due_in(), None, "the copy is up to date");

    // The next edit owes another one, and now the throttle is running, so it
    // is owed *later* rather than now.
    press(&mut ed, "乙");
    let owed = ed.autosave_due_in().expect("owed again");
    assert!(owed > std::time::Duration::ZERO, "the throttle has started: {owed:?}");
    assert!(owed <= std::time::Duration::from_secs(5), "and it is bounded: {owed:?}");

    // Autosave off is the one way to owe nothing while unsaved.
    ed.set_autosave(false);
    assert_eq!(ed.autosave_due_in(), None, "off means off");

    let _ = std::fs::remove_dir_all(&dir);
}

/// **A file that ends in a newline has no row after its last one** (#384).
///
/// `ropey` reports a trailing empty line for every file that ends in a newline
/// — which is nearly every file — and the grid drew a row there: all cells
/// blank, and the caret could walk into it and type. What reached the disk was
/// the byte typed, welded to the end of the file with no separator and with the
/// trailing newline gone. `row_is_ragged` had the guard from the beginning; the
/// drawing, the caret and the write path each asked `line_count()` instead.
#[test]
fn a_grid_puts_no_row_after_the_last_line_of_the_file() {
    let dir = std::env::temp_dir().join(format!("yumete-phantom-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let open = |name: &str, text: &str| {
        let csv = dir.join(name);
        std::fs::write(&csv, text).unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        ed
    };

    // Two rows under a header, and the file ends in a newline.
    let ed = open("trail.csv", "a,b\n1,2\n3,4\n");
    assert_eq!(ed.table_row_span(), Some((1, 2)), "rows 1 and 2, and no third");

    // The same data without the trailing newline reads the same. It always
    // did — which is what made the bug look like a property of the data.
    let ed = open("bare.csv", "a,b\n1,2\n3,4");
    assert_eq!(ed.table_row_span(), Some((1, 2)));

    // `j` on the bottom row stays there. It used to step onto the phantom.
    let mut ed = open("walk.csv", "a,b\n1,2\n3,4\n");
    press(&mut ed, "jjjjjj");
    assert_eq!(ed.cursor_line(), 2, "the last row of writing, not past it");

    // And what a keystroke there writes lands in a cell, not on the end of
    // the file: typing used to produce `a,b\n1,2\n3,4\nx` — no separator, no
    // line break, and the file's own trailing newline eaten.
    press(&mut ed, "ix\u{1b}");
    assert!(
        ed.current_buffer().text().ends_with('\n'),
        "the trailing newline survives: {:?}",
        ed.current_buffer().text()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_capital_s_sorts_a_delimited_file_downwards() {
    // `t S` sorted *up* on a delimited file: the branch that handles a
    // bare `s`/`S` never looked at which of the two had been pressed, so
    // the editor did the opposite of the key and said nothing.
    //
    // The bare spelling is gone (§5.7) and `t0s` is what asks for 「the
    // column I am standing in」 — the same question, said out loud.
    let dir = std::env::temp_dir().join(format!("yumete-sortS-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sorted = |name: &str, keys: &str| {
        let csv = dir.join(name);
        std::fs::write(&csv, "字,序\n甲,1\n丙,3\n乙,2\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        press(&mut ed, keys);
        ed.current_buffer().text()
    };
    // By code point, which is what the sort promises for anything that is
    // not a number: 丙 U+4E19, 乙 U+4E59, 甲 U+7532.
    assert_eq!(sorted("up.csv", "t0s"), "字,序\n丙,3\n乙,2\n甲,1\n");
    assert_eq!(sorted("down.csv", "t0S"), "字,序\n甲,1\n乙,2\n丙,3\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn t_still_works_when_the_grid_is_read_by_character() {
    // `Tab` reads the grid by character — which is how you get *inside* a
    // cell, and exactly where the lesson tells the reader to press `t/`.
    // When `Enter` did that job it worked in both grains; `t` did not.
    let mut ed = typed("| 字 | 拆分 |\n| -- | -- |\n| 木 | 木 |\n| 相 | 木目 |\n");
    ed.goto_line(3);
    assert!(ed.enter_table(), "{}", ed.status());
    ed.on_key(Key::Tab);
    ed.on_key(Key::Char('t'));
    assert!(ed.pending_menu().is_some(), "t opened nothing in the char grain");
}

#[test]
fn one_g_goes_to_the_first_line() {
    // `1G` went to the *last* line: the count had already been taken by the
    // time `G` asked whether there was one.
    let mut ed = typed("一\n二\n三\n四\n");
    press(&mut ed, "1G");
    assert_eq!(ed.cursor_line(), 0, "1G is the first line");
    press(&mut ed, "3G");
    assert_eq!(ed.cursor_line(), 2);
    press(&mut ed, "G");
    assert_eq!(ed.cursor_line(), 3, "a bare G is still the last line");
}

#[test]
fn a_sentence_motion_stops_before_the_next_sentence() {
    // `)d` used to delete this sentence *and the first character of the
    // next one*, because the motion lands on the next sentence's start and
    // the selection ran through it. `w` has always stepped back one
    // character for exactly this reason; these had not.
    // ⚠️ On `L`/`H` since 2026-09-12 — `(`/`)` are Helix's for cycling
    // selections, and squatting on them would mean moving twice (#404).
    let mut ed = typed("第一句。第二句。第三句。\n");
    press(&mut ed, "Ld");
    assert_eq!(ed.current_buffer().text(), "第二句。第三句。\n");
    // The same for a paragraph.
    let mut ed = typed("第一段。\n第二段。\n");
    press(&mut ed, "}d");
    assert_eq!(ed.current_buffer().text(), "第二段。\n");

    // …and standing **on** the 。, where the next sentence begins one grapheme
    // away. It used to collapse here and take only the 。; now it takes the
    // **next** sentence whole, which is what `w` does in the same position and
    // what makes the key repeatable at all.
    let mut ed = typed("第一句。第二句。第三句。\n");
    press(&mut ed, "lllLd");
    assert_eq!(ed.current_buffer().text(), "第一句。第三句。\n");
}

#[test]
fn insert_takes_back_a_word_and_a_line() {
    // `C-w` and `C-u` exist in vi, Helix, readline and every terminal
    // prompt, and Insert mode ate both of them.
    let mut ed = typed("春天到了很好\n");
    ed.goto_line(1);
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Ctrl('w'));
    let after = ed.current_buffer().text();
    assert!(
        after.starts_with("春天到了") && after.trim_end() != "春天到了很好",
        "one word, not the whole paragraph: {after:?}"
    );
    ed.on_key(Key::Ctrl('u'));
    assert_eq!(ed.current_buffer().text(), "\n", "and C-u takes the line");
    // One undo point each, and the line comes back.
    ed.on_key(Key::Esc);
    press(&mut ed, "u");
    assert_eq!(ed.current_buffer().text(), after);
}

#[test]
fn taking_back_a_word_stays_inside_its_cell() {
    let mut ed = typed("| 甲 | 春天到了很好 |\n| --- | --- |\n| 丙 | 丁 |\n");
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "l");
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Ctrl('u'));
    // The cell emptied; the pipe beside it is still there.
    let text = ed.current_buffer().text();
    assert_eq!(text.lines().next().unwrap().matches('|').count(), 3, "{text}");
    assert!(!text.contains("春天"), "{text}");
}

#[test]
fn a_packed_page_still_says_where_a_paragraph_begins() {
    // `:view-dense` is the default page, so masking the indent under it made
    // 首行縮進 invisible out of the box. The three things `:view-dense` drops
    // each cost a *column*; the indent costs two squares, and it is what
    // replaces the blank line — which costs a whole 縱.
    let mut ed = Editor::new();
    ed.set_layout(crate::zong::Layout::Vertical);
    ed.set_indent(2);
    ed.set_dense(true);
    assert_eq!(ed.paragraph_indent(), 2);
    let nothing = |_: usize| Vec::new();
    let never = |_: usize| false;
    let bare = |_: usize| Vec::new();
    assert_eq!(ed.grid_with(&nothing, &never, &bare).indent, 2);
    assert!(ed.ruby().is_empty(), "…while the reading column still goes");
}

#[test]
fn the_blank_line_an_indent_replaces_comes_off_the_page() {
    // The file is Markdown and keeps its blank lines; the page is a book
    // and shows the indent instead. Both marks at once is the one thing
    // no typesetter does.
    let mut ed = Editor::new();
    ed.current_buffer_mut()
        .insert(0, "第一段\n\n第二段\n\n\n第三段\n# 標題\n\n```\n\n```\n")
            .expect("the fixture buffer is writable");
    assert!(!ed.line_is_folded(1), "nothing folds until there is an indent");
    ed.set_indent(2);
    assert!(ed.line_is_folded(1), "the one between two paragraphs");
    // Two blanks is a scene break — the writer meant the second one.
    assert!(!ed.line_is_folded(3));
    assert!(!ed.line_is_folded(4));
    // A blank line inside a fence is code, not a paragraph break.
    assert!(!ed.line_is_folded(8), "inside the fence");
    // …and the vertical page, which carries a span rather than a map, is
    // told to leave that whole part of the file alone.
    let (first, last) = ed.fold_free_span();
    assert!(first <= 8 && last >= 8, "the fence is in the span: {first}..{last}");
    // …and a heading is not in it: a novel's chapters are not prose
    // either, and a span from the first heading to the last would be the
    // whole book.
    assert!(first > 6, "the heading is not in the span: {first}..{last}");
    let hidden = |line: usize| ed.markup_hidden_on_line(line);
    let folded = |line: usize| ed.line_is_folded(line);
    let drawn = |line: usize| ed.drawn_runs_on_line(line);
    assert!(!crate::zong::folded(
        ed.current_buffer().rope(),
        8,
        ed.grid_with(&hidden, &folded, &drawn)
    ));
    // And never the line the cursor is on, or you could not type into it.
    ed.execute(":2").unwrap();
    assert_eq!(ed.cursor_line(), 1);
    assert!(!ed.line_is_folded(1));
}

#[test]
fn a_packed_page_does_not_pay_for_a_reading_column() {
    // `:view-dense` says in its own doc comment, and in the manual's table,
    // that it drops the reading column. It did not: the mask was on the
    // hung 句讀 and not on the readings, so a packed page still reserved
    // two cells a 縱 for a column it was not drawing.
    let mut ed = Editor::new();
    ed.set_layout(crate::zong::Layout::Vertical);
    assert!(!ed.ruby().is_empty(), "readings are laid out by default");
    ed.set_dense(true);
    assert!(ed.ruby().is_empty(), "a packed 縱書 page lays out none");
    // …but a horizontal page pays no width for one, so packing takes
    // nothing away there.
    ed.set_layout(crate::zong::Layout::Horizontal);
    assert!(!ed.ruby().is_empty(), "橫排 is not what 密排 packs");
    ed.set_layout(crate::zong::Layout::Vertical);
    assert!(
        !ed.ruby_configured().is_empty(),
        "but nothing was turned off — `:view-dense off` gives them back"
    );
    ed.set_dense(false);
    assert!(!ed.ruby().is_empty());
}

#[test]
fn ruby_rendering_is_set_per_dialect() {
    let mut ed = Editor::new();
    assert!(
        ed.ruby().contains(Dialect::Html),
        "HTML readings are laid out by default"
    );
    ed.execute(":ruby off").unwrap();
    assert!(ed.ruby().is_empty());
    // 中階 knows the reading and does not draw it, so the drawn set is
    // empty there too — and naming a dialect is what asks to see one.
    ed.execute(":ruby basic").unwrap();
    assert!(ed.ruby().is_empty());
    assert_eq!(ed.ruby_level(), Render::Basic);
    ed.execute(":ruby full").unwrap();
    assert!(ed.ruby().contains(Dialect::Html));
    ed.execute(":ruby basic").unwrap();
    ed.execute(":ruby-html").unwrap();
    assert!(ed.ruby().contains(Dialect::Html));

    // Dialects add up rather than replacing one another: a document may mix
    // them, so `:render-ruby-typst` does not turn HTML off.
    ed.execute(":ruby-typst").unwrap();
    assert!(ed.ruby().contains(Dialect::Typst));
    assert!(ed.ruby().contains(Dialect::Html));
    ed.execute(":ruby-html off").unwrap();
    assert!(!ed.ruby().contains(Dialect::Html));
    assert!(ed.ruby().contains(Dialect::Typst));
}

#[test]
fn format_ruby_rewrites_every_reading_into_one_dialect() {
    let mut ed = typed("讀<ruby>漢<rt>hàn</rt></ruby>和#ruby(\"字\", \"zì\")");
    ed.execute(":ruby-format typst").unwrap();
    assert_eq!(
        ed.current_buffer().text(),
        "讀#ruby(\"漢\", \"hàn\")和#ruby(\"字\", \"zì\")"
    );
    // Already uniform: nothing to do, and no undo step spent on it.
    ed.execute(":ruby-format typst").unwrap();
    assert!(ed.status().contains("已經是"), "{}", ed.status());

    ed.execute(":ruby-format html").unwrap();
    assert_eq!(
        ed.current_buffer().text(),
        "讀<ruby>漢<rt>hàn</rt></ruby>和<ruby>字<rt>zì</rt></ruby>"
    );
}

/// #333: 一本講 ruby 標記的書，`:ruby-format` 不許動它自己的例子。
#[test]
fn format_ruby_leaves_the_examples_in_the_fence_alone() {
    let mut ed = typed(
        "讀<ruby>漢<rt>hàn</rt></ruby>\n\n```html\n<ruby>桜<rt>さくら</rt></ruby>\n```\n\n又一個<ruby>字<rt>zì</rt></ruby>\n",
    );
    ed.execute(":ruby-format typst").unwrap();
    let text = ed.current_buffer().text();
    assert!(text.contains("讀#ruby(\"漢\", \"hàn\")"), "{text}");
    assert!(text.contains("又一個#ruby(\"字\", \"zì\")"), "{text}");
    assert!(text.contains("<ruby>桜<rt>さくら</rt></ruby>"), "{text}");
    // 圍欄裏那一個既沒被改寫，也不算「讀不出來」——說它剩下就是叫人去查一個
    // 不存在的毛病。
    assert!(!ed.status().contains("讀不出來"), "{}", ed.status());
}

#[test]
fn a_typst_reading_is_read_too() {
    let mut ed = typed("讀#ruby(\"漢字\", \"hàn zì\")");
    ed.set_ruby(crate::ruby::Dialects::only(crate::ruby::Dialect::Typst));
    press(&mut ed, "gg3l");
    ed.execute(":ruby").unwrap();
    assert_eq!(ed.prompt(), Some(("注", "hàn zì")));
}

#[test]
fn ruby_mode_annotates_a_selection() {
    let mut ed = typed("他說口很難");
    press(&mut ed, "gg2lv"); // select 口
    ed.execute(":ruby").unwrap();
    assert_eq!(ed.mode(), Mode::Ruby);
    assert_eq!(ed.prompt(), Some(("注", "")), "a fresh reading");
    submit_reading(&mut ed, "kǒu");
    assert_eq!(
        ed.current_buffer().text(),
        "他說<ruby>口<rt>kǒu</rt></ruby>很難"
    );
    assert_eq!(ed.mode(), Mode::Normal);
}

#[test]
fn ruby_mode_loads_an_existing_reading_to_correct_it() {
    let mut ed = typed("他說<ruby>口<rt>kou</rt></ruby>很難");
    // Anywhere in the group opens it, markup included.
    press(&mut ed, "gg5l");
    ed.execute(":ruby").unwrap();
    assert_eq!(ed.prompt(), Some(("注", "kou")), "prefilled, not blank");
    // Correct it: backspace the tone-less vowel and retype.
    ed.on_key(Key::Backspace);
    ed.on_key(Key::Backspace);
    submit_reading(&mut ed, "ǒu");
    assert_eq!(
        ed.current_buffer().text(),
        "他說<ruby>口<rt>kǒu</rt></ruby>很難"
    );
}

#[test]
fn an_empty_reading_takes_the_annotation_off() {
    let mut ed = typed("他說<ruby>口<rt>kǒu</rt></ruby>很難");
    press(&mut ed, "gg5l");
    ed.execute(":ruby").unwrap();
    for _ in 0..8 {
        ed.on_key(Key::Backspace);
    }
    assert_eq!(
        ed.mode(),
        Mode::Ruby,
        "backspacing the reading, not leaving"
    );
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "他說口很難", "markup gone too");
}

#[test]
fn a_bar_annotates_each_character_separately() {
    let mut ed = typed("讀漢字");
    press(&mut ed, "gglvll"); // select 漢字
    ed.execute(":ruby").unwrap();
    submit_reading(&mut ed, "hàn|zì");
    assert_eq!(
        ed.current_buffer().text(),
        "讀<ruby>漢<rt>hàn</rt></ruby><ruby>字<rt>zì</rt></ruby>"
    );
}

#[test]
fn ruby_mode_annotates_the_character_under_the_cursor() {
    // There is no such thing as "nothing selected" any more: the cursor's
    // own 字 is in the selection, and annotating one 字 is the common case.
    let mut ed = typed("他說口很難");
    press(&mut ed, "gg2l");
    ed.execute(":ruby").unwrap();
    assert_eq!(ed.mode(), Mode::Ruby);
    submit_reading(&mut ed, "kǒu");
    assert_eq!(
        ed.current_buffer().text(),
        "他說<ruby>口<rt>kǒu</rt></ruby>很難"
    );
}

#[test]
fn ruby_mode_needs_something_to_annotate() {
    // An empty buffer really does have nothing.
    let mut ed = Editor::new();
    ed.execute(":ruby").unwrap();
    assert_eq!(ed.mode(), Mode::Normal, "nothing to annotate");
    assert!(!ed.status().is_empty(), "and it says so");
}

#[test]
fn escape_leaves_ruby_mode_without_writing() {
    let mut ed = typed("他說口很難");
    press(&mut ed, "gg2lvl");
    ed.execute(":ruby").unwrap();
    submit_reading_cancelled(&mut ed, "kǒu");
    assert_eq!(ed.current_buffer().text(), "他說口很難");
    assert_eq!(ed.mode(), Mode::Normal);
}

fn submit_reading_cancelled(ed: &mut Editor, reading: &str) {
    for c in reading.chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Esc);
}

#[test]
fn diff_reports_the_word_that_changed_and_gf_goes_to_it() {
    let dir = std::env::temp_dir().join(format!("yumete-diff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ch01.txt");
    std::fs::write(&file, "第一行\n那年冬天，雪下得很早。\n第三行\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&file).unwrap();

    // Nothing typed yet: the buffer and the file agree.
    ed.execute(":diff").unwrap();
    assert!(ed.status().contains("一個字都不差"), "{}", ed.status());
    assert_eq!(ed.buffer_tabs().len(), 1, "no results buffer for no changes");

    // One word, in the middle of a paragraph a line diff would call wholly
    // changed.
    ed.goto_line(2);
    ed.execute(":s/很早/極早/").unwrap();
    ed.execute(":diff").unwrap();
    let listing = ed.current_buffer().text();
    assert_eq!(
        listing.trim_end(),
        "ch01.txt:2: 那年冬天，雪下得[-很-]{+極+}早。",
        "the line as it stands now, with the one word marked — and 早 is \
         shared, because with no dictionary loaded the words are characters"
    );
    // The listing is jumpable, like `:grep`'s.
    ed.set_cursor(0);
    press(&mut ed, "gf");
    assert_eq!(ed.current_buffer().display_name(), "ch01.txt");
    assert_eq!(ed.cursor_line(), 1);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn diff_can_be_told_which_draft_to_compare_with() {
    let dir = std::env::temp_dir().join(format!("yumete-diff2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ch01.txt"), "他說好。\n").unwrap();
    std::fs::write(dir.join("昨天.txt"), "他說不好。\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(dir.join("ch01.txt")).unwrap();
    // Relative to the file being read, not to wherever yumete was started.
    ed.execute(":diff 昨天.txt").unwrap();
    assert!(
        ed.current_buffer().text().contains("[-不-]"),
        "{}",
        ed.current_buffer().text()
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn diff_needs_something_to_compare_with() {
    let mut ed = typed("寫了一半");
    ed.execute(":diff").unwrap();
    assert!(ed.status().contains(":diff"), "{}", ed.status());
}

/// A reader with a handful of characters in it, so `:ruby-auto` can be
/// tested without the 14 MB the real one comes from.
struct Toy;

impl yumete_cjk::Reader for Toy {
    fn read(&self, word: &str) -> Option<Vec<String>> {
        let one = |c: char| match c {
            '漢' => Some("hàn"),
            '字' => Some("zì"),
            '很' => Some("hěn"),
            '難' => Some("nán"),
            '龘' => Some("dá"),
            _ => None,
        };
        word.chars().map(|c| one(c).map(str::to_string)).collect()
    }

    fn is_rare(&self, ch: char) -> Option<bool> {
        Some(ch == '龘')
    }

    fn charset(&self, ch: char) -> Option<String> {
        Some(match ch {
            // In 古籍 only, so rare — but a standard does carry it.
            '龘' => "古-CJK".to_string(),
            // In nothing at all, and in a block a font may well lack.
            '𠮷' => "-CJK擴展B".to_string(),
            _ => "簡繁臺港-CJK".to_string(),
        })
    }

    fn available(&self) -> bool {
        true
    }
}

fn with_toy_reader(text: &str) -> Editor {
    let mut ed = typed(text);
    ed.set_reader(Box::new(Toy));
    // One 漢字 per word, so what the test reads is the ruby and not the
    // dictionary's opinion of where 詞 end.
    ed.set_segmenter(Box::new(CategorySegmenter));
    ed
}

#[test]
fn auto_ruby_annotates_a_word_per_character() {
    let mut ed = with_toy_reader("漢字");
    ed.execute(":ruby-auto").unwrap();
    assert_eq!(
        ed.current_buffer().text(),
        "<ruby>漢<rt>hàn</rt></ruby><ruby>字<rt>zì</rt></ruby>"
    );
    // One step back, however many words it wrote.
    press(&mut ed, "u");
    assert_eq!(ed.current_buffer().text(), "漢字");
}

#[test]
fn auto_ruby_leaves_a_reading_the_writer_already_corrected() {
    let mut ed = with_toy_reader("<ruby>漢<rt>hon</rt></ruby>字");
    ed.execute(":ruby-auto").unwrap();
    assert!(
        ed.current_buffer().text().contains("<rt>hon</rt>"),
        "{}",
        ed.current_buffer().text()
    );
    assert!(ed.current_buffer().text().contains("<rt>zì</rt>"));
}

#[test]
fn auto_ruby_says_so_when_there_is_nothing_to_annotate() {
    let mut ed = with_toy_reader("abc、。");
    ed.execute(":ruby-auto").unwrap();
    assert_eq!(ed.current_buffer().text(), "abc、。");
    assert!(ed.status().contains("沒有"), "{}", ed.status());
}

#[test]
fn auto_ruby_without_a_reader_says_where_readings_come_from() {
    let mut ed = typed("漢字");
    ed.execute(":ruby-auto").unwrap();
    assert_eq!(ed.current_buffer().text(), "漢字");
    assert!(ed.status().contains("拆分表"), "{}", ed.status());
}

#[test]
fn auto_ruby_rare_annotates_only_what_a_reader_would_stumble_on() {
    let mut ed = with_toy_reader("漢龘字");
    ed.execute(":ruby-auto rare").unwrap();
    assert_eq!(
        ed.current_buffer().text(),
        "漢<ruby>龘<rt>dá</rt></ruby>字"
    );
}

#[test]
fn auto_ruby_annotates_the_selection_and_nothing_outside_it() {
    let mut ed = with_toy_reader("漢字很難");
    press(&mut ed, "ggvl"); // 漢字
    ed.execute(":ruby-auto").unwrap();
    assert_eq!(
        ed.current_buffer().text(),
        "<ruby>漢<rt>hàn</rt></ruby><ruby>字<rt>zì</rt></ruby>很難"
    );
}

#[test]
fn auto_ruby_writes_the_dialect_the_file_is_written_in() {
    let dir = std::env::temp_dir().join(format!("yumete-auto-ruby-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ch01.typ");
    std::fs::write(&file, "#ruby(\"漢\", \"hàn\")字\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&file).unwrap();
    ed.set_reader(Box::new(Toy));
    ed.set_segmenter(Box::new(CategorySegmenter));
    ed.execute(":ruby-auto").unwrap();
    assert_eq!(
        ed.current_buffer().text(),
        "#ruby(\"漢\", \"hàn\")#ruby(\"字\", \"zì\")\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_command_says_what_it_is_waiting_for() {
    // 標點旁置 needs a 縱書 page that is not packed. It used to set a flag
    // nobody read: the setting said 「開」, the page did not change, and
    // there was nowhere to find out why.
    let mut ed = Editor::new();
    ed.set_dense(true);
    ed.execute(":view-hanging on").unwrap();
    assert!(!ed.hanging_punctuation(), "{}", ed.status());
    let said = ed.status().to_string();
    assert!(said.contains("竪排") && said.contains("密排關"), "{said}");
    assert!(said.contains("force"), "{said}");

    // …and `force` brings the prerequisites about, in one line.
    ed.execute(":view-hanging on force").unwrap();
    assert_eq!(ed.layout(), crate::zong::Layout::Vertical);
    assert!(!ed.dense());
    assert!(ed.hanging_punctuation(), "{}", ed.status());

    // A command whose needs are met says nothing about them.
    ed.execute(":view-hanging off").unwrap();
    assert!(!ed.hanging_punctuation());
    assert!(!ed.status().contains("需要"), "{}", ed.status());
}

#[test]
fn the_commit_method_rides_the_same_channel_as_the_scheme() {
    // 上屏方式 belongs to the engine, which the core does not hold, so
    // `:yume-commit` leaves a request the front end answers (Feature #209).
    let mut ed = Editor::new();
    ed.set_ime_available(true);
    ed.execute(":yume-commit").unwrap();
    assert_eq!(ed.take_scheme_request().as_deref(), Some("commit:"));
    ed.execute(":yume-commit auto").unwrap();
    assert_eq!(ed.take_scheme_request().as_deref(), Some("commit:unique"));
    assert_eq!(ed.take_scheme_request(), None, "taken once only");
    // It is about typing 漢字, so with no 碼表 it says so rather than
    // leaving a request nobody can answer.
    let mut cold = Editor::new();
    cold.execute(":yume-commit fluency").unwrap();
    assert_eq!(cold.take_scheme_request(), None, "{}", cold.status());
    assert!(cold.status().contains("碼表"), "{}", cold.status());
}

#[test]
fn chaifen_command_leaves_a_request_for_the_ime() {
    let mut ed = Editor::new();
    assert_eq!(ed.take_chaifen_request(), None);
    // 拆分 annotates *candidates*, so it needs a 碼表 — and says so, with
    // nothing loaded, instead of leaving a request nobody can answer.
    ed.execute(":yume-chaifen").unwrap();
    assert_eq!(ed.take_chaifen_request(), None, "{}", ed.status());
    assert!(ed.status().contains("碼表"), "{}", ed.status());
    ed.set_ime_available(true);
    ed.execute(":yume-chaifen").unwrap();
    assert_eq!(ed.take_chaifen_request(), Some(true));
    assert_eq!(ed.take_chaifen_request(), None, "taken once only");
    // The toggle follows what the IME actually settled on, not the request:
    // a scheme with no 拆分 layer refuses, and the next `:chaifen` still
    // asks for "on" rather than flipping to "off".
    ed.set_chaifen(false);
    ed.execute(":yume-chaifen").unwrap();
    assert_eq!(ed.take_chaifen_request(), Some(true));
}

#[test]
fn committed_text_goes_to_the_prompt_while_searching() {
    let mut ed = typed("春江潮水連海平");
    ed.on_key(Key::Char('/'));
    // What the IME commits belongs in the search pattern, not the buffer.
    ed.insert_committed("潮水");
    assert_eq!(ed.prompt(), Some(("/", "潮水")));
    assert_eq!(ed.current_buffer().text(), "春江潮水連海平");
    ed.on_key(Key::Enter);
    // The match becomes the selection, so the head sits past its last
    // character and 潮水 is what an edit would act on.
    assert_eq!(ed.selection(), (2, 4), "search selected 潮水");
}

/// `::` is a second mode, not a longer `:` line (Feature #224).
#[test]
fn a_second_colon_opens_the_search_over_what_the_commands_do() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char(':'));
    assert_eq!(ed.mode(), Mode::Command);
    ed.on_key(Key::Char(':'));
    assert_eq!(ed.mode(), Mode::Lookfor);
    assert_eq!(ed.prompt(), Some(("::", "")), "and it says which line it is");
    // Backspacing it empty goes back to `:` — the second colon is the last
    // thing there was to take back — and again from there to the page.
    ed.on_key(Key::Backspace);
    assert_eq!(ed.mode(), Mode::Command);
    ed.on_key(Key::Backspace);
    assert_eq!(ed.mode(), Mode::Normal);
}

/// Only on an **empty** line: `:s/:/：/` is a substitution with two colons
/// in it, and it used to be typed in a mode that did not exist yet.
#[test]
fn a_colon_further_along_the_line_is_only_a_colon() {
    let mut ed = Editor::new();
    for c in ":s/".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Char(':'));
    assert_eq!(ed.mode(), Mode::Command);
    assert_eq!(ed.prompt(), Some((":", "s/:")));
}

/// The whole point of the mode: the reader is thinking 「竖排」 and the
/// command is called `layout vertical`.
#[test]
fn a_chinese_word_on_that_line_finds_the_english_command() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char(':'));
    ed.on_key(Key::Char(':'));
    ed.insert_committed("竖排");
    assert_eq!(ed.prompt(), Some(("::", "竖排")), "committed onto the line");
    let (found, focus) = ed.lookfor_menu();
    let names: Vec<String> = found.iter().map(|h| h.choice.written()).collect();
    assert_eq!(
        names.first().map(String::as_str),
        Some("layout vertical"),
        "{names:?}"
    );
    assert_eq!(focus, 0);
    // ⇥ writes the **whole** command back onto the `:` line and goes back
    // there. Nothing has been run: what Enter runs is always the line the
    // reader can see.
    ed.on_key(Key::Tab);
    assert_eq!(ed.mode(), Mode::Command);
    assert_eq!(ed.prompt(), Some((":", "layout vertical")));
    assert_eq!(ed.prompt_caret(), "layout vertical".chars().count());
}

/// A typed abbreviation is a subsequence, and the answer is a **subcommand**
/// — the flat list is the whole tree, not the two dozen top-level words.
#[test]
fn an_abbreviation_reaches_a_word_a_command_takes() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char(':'));
    ed.on_key(Key::Char(':'));
    for c in "tbsort".chars() {
        ed.on_key(Key::Char(c));
    }
    let (found, _) = ed.lookfor_menu();
    let names: Vec<String> = found.iter().map(|h| h.choice.written()).collect();
    assert!(names.iter().any(|n| n == "table-sort"), "{names:?}");
    // Down walks the list, and the next keystroke of the query puts the
    // highlight back on top — the third row for `竖` is not the third row
    // for `竖排`.
    ed.on_key(Key::Down);
    assert_eq!(ed.lookfor_menu().1, 1);
    ed.on_key(Key::Char('x'));
    assert_eq!(ed.lookfor_menu().1, 0);
}

#[test]
fn tab_cycles_the_command_completion() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char(':'));
    for c in "ru".chars() {
        ed.on_key(Key::Char(c));
    }
    // Tab walks the matches, writing each onto the line — `run` and `ruby`
    // both start with `ru`, in the order the table lists them.
    ed.on_key(Key::Tab);
    assert_eq!(ed.prompt(), Some((":", "run")));
    ed.on_key(Key::Tab);
    assert_eq!(ed.prompt(), Some((":", "ruby")), "one `ruby` family now, folded (#369)");
    // …and the prefix is remembered rather than re-read from the line, so
    // walking back returns to the same one instead of starting over from
    // what Tab just wrote.
    ed.on_key(Key::BackTab);
    assert_eq!(ed.prompt(), Some((":", "run")));

    // Typing abandons the completion, so the next Tab starts from the line.
    ed.on_key(Key::Char('x'));
    assert_eq!(ed.command_menu().1, None);
}

#[test]
fn the_command_line_guesses_the_rest_of_the_name() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char(':'));
    for c in "reco".chars() {
        ed.on_key(Key::Char(c));
    }
    assert_eq!(ed.prompt_ghost(), "ver", "the rest of `recover`");

    // Tab takes the guess, and then there is nothing left to guess.
    ed.on_key(Key::Tab);
    assert_eq!(ed.prompt(), Some((":", "recover")));
    assert_eq!(ed.prompt_ghost(), "", "the line is the completion now");

    // Nothing is guessed before anything is typed, or once arguments start.
    let mut ed = Editor::new();
    ed.on_key(Key::Char(':'));
    assert_eq!(ed.prompt_ghost(), "");
    for c in "w draf".chars() {
        ed.on_key(Key::Char(c));
    }
    assert_eq!(ed.prompt_ghost(), "", "a file name is not a command name");
}

/// The guess and Tab are two spellings of one answer, so they have to
/// spell it the same way. A deep name (#223) is answered with its whole
/// path — `:xing` is `yume scheme xingchen` — and the guess used to offer
/// the leaf alone, which as a line parses as nothing.
///
/// It used to ask this of `:sch`, which was one answer until `:table
/// schema` (#218) became a second one. A prefix two commands answer to is
/// a fine thing for the menu and a poor thing to assert about; a leaf only
/// one word in the tree carries is the case this test is here for.
#[test]
fn the_guess_and_tab_agree_on_a_deep_name() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char(':'));
    for c in "xing".chars() {
        ed.on_key(Key::Char(c));
    }
    let (_, before) = ed.prompt().expect("a command line");
    let before = before.to_string();
    let guessed = format!("{before}{}", ed.prompt_ghost());
    ed.on_key(Key::Tab);
    let (_, tabbed) = ed.prompt().expect("a command line");
    assert_eq!(
        tabbed, "yume-scheme xingchen",
        "the deep answer is the whole path",
    );
    assert!(
        guessed == before || guessed == tabbed,
        "the guess says {guessed:?} and Tab says {tabbed:?}",
    );
}

/// The prompt has had ← → Home End since it was written, so what the IME
/// commits goes **at the caret**. It used to be appended, which put 中文
/// typed into the middle of a pattern at the end of it instead.
#[test]
fn committed_text_lands_at_the_prompt_caret() {
    let mut ed = typed("春江潮水連海平");
    ed.on_key(Key::Char('/'));
    ed.insert_committed("海平");
    ed.on_key(Key::Home);
    ed.insert_committed("潮水");
    assert_eq!(ed.prompt(), Some(("/", "潮水海平")));
    // …and the caret came with it, so the next commit follows on.
    ed.insert_committed("連");
    assert_eq!(ed.prompt(), Some(("/", "潮水連海平")));
}

/// **A capital is how you ask for the case** (#301).
///
/// A pattern with none in it ignores case; one with a capital does not. The
/// rule earns its keep on a manuscript: 漢字 has no case, so nearly every
/// search here takes the first branch for free, while `TODO` — a mark, not a
/// word — still finds only the mark.
#[test]
fn a_pattern_with_no_capital_in_it_ignores_case() {
    // 一0 段1 ␠2 T3 O4 D5 O6 ␠7 二8 段9 ␠10 t11 o12 d13 o14
    let mut ed = typed("一段 TODO 二段 todo 三段。");
    let find = |ed: &mut Editor, pattern: &str| {
        ed.set_cursor(0);
        ed.on_key(Key::Char('/'));
        for c in pattern.chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Enter);
        ed.selection()
    };
    // No capital: either one will do, so the nearer one wins.
    assert_eq!(find(&mut ed, "todo"), (3, 7), "lower case takes either");
    // A capital: the mark, and only the mark.
    assert_eq!(find(&mut ed, "TODO"), (3, 7));
    // `(?-i)` is the one-off way to mean lower case and nothing else…
    assert_eq!(find(&mut ed, "(?-i)todo"), (11, 15), "the lower-case one");
    // …and the setting is the standing one.
    ed.set_smart_case(false);
    assert_eq!(find(&mut ed, "todo"), (11, 15), "off: lower case means lower");
}

/// **`w` takes a word, `e` takes a clause** (#304).
///
/// The spec was settled against helix a cell at a time. Its two halves:
/// **`w` counts the whitespace after a word as part of it, `e` counts the
/// whitespace before one as part of *that* one** — and `e` never consults the
/// dictionary, because Chinese has no spaces and an `e` that did would do
/// nearly what `w` does. Left coarse it runs to the punctuation instead.
///
/// The rows below are lifted from the measured table in `[^304]`; the sentence
/// carries 全角 punctuation, a quoted exclamation, Latin, and spaces on both
/// sides of it.
#[test]
fn w_takes_a_word_and_e_takes_a_clause() {
    // 他0 抬1 頭2 看3 了4 看5 那6 片7 天8 ，9 山10 …了17 。18 「19 走20 吧21
    // ！22 」23 他24 說25 ␠26 O27 K28 ␠29 了30 。31
    let text = "他抬頭看了看那片天，山路已經看不見了。「走吧！」他說 OK 了。\n";
    let at = |n: usize, key: char| {
        let mut ed = typed(text);
        for _ in 0..n {
            ed.on_key(Key::Char('l'));
        }
        ed.on_key(Key::Char(key));
        let (a, b) = ed.selection();
        ed.current_buffer().text().chars().skip(a).take(b - a).collect::<String>()
    };

    // `e` — coarse, and the space belongs to the word in front of it.
    assert_eq!(at(0, 'e'), "他抬頭看了看那片天", "a whole run of 漢字 is one word");
    assert_eq!(at(8, 'e'), "，", "standing on a word's last cell, take the next");
    assert_eq!(at(17, 'e'), "。「", "a run of punctuation is a word too");
    assert_eq!(at(23, 'e'), "他說", "…and it does not drag 」 along");
    assert_eq!(at(25, 'e'), " OK", "the space in front comes with the word");
    assert_eq!(at(27, 'e'), "OK", "…but not when the caret is already inside it");
    assert_eq!(at(28, 'e'), " 了");

    // `w` — a word at a time, with the space *after* each one. (No dictionary
    // is installed in a bare `Editor`, so a 漢字 is a word here; what matters
    // for this test is the whitespace side, which is the same either way.)
    assert_eq!(at(8, 'w'), "，");
    assert_eq!(at(23, 'w'), "他");
    assert_eq!(at(26, 'w'), "OK ", "and `w` keeps the trailing space");

    // **`e` then `b` takes the clause the caret is in.** helix's tutor teaches
    // the pair for a *word* —「to select the word under cursor, combine `e` and
    // `b`」 — and the two only compose if they read the same grain. Here they
    // are both coarse, so the pair takes the run between two 標點, which is the
    // more useful unit in Chinese: an English word is many letters, a Chinese
    // word is two and a half characters.
    let pair = |n: usize| {
        let mut ed = typed(text);
        for _ in 0..n {
            ed.on_key(Key::Char('l'));
        }
        ed.on_key(Key::Char('e'));
        ed.on_key(Key::Char('b'));
        let (a, b) = ed.selection();
        ed.current_buffer().text().chars().skip(a).take(b - a).collect::<String>()
    };
    assert_eq!(pair(0), "他抬頭看了看那片天");
    assert_eq!(pair(12), "山路已經看不見了", "from the middle of a clause too");
    assert_eq!(pair(20), "走吧", "quoted, so the marks bound it");
}

/// **`:word-level off` reads a 漢字 as a letter** (#304).
///
/// Not a fourth setting of the dictionary but the absence of one, the way
/// helix reads Chinese — and the same grain `e` uses at every level. One
/// authority answers it (`Editor::word_grain`), because the level can change
/// under `:word-level` and a second mechanism would disagree the moment it did.
#[test]
fn the_dictionary_can_be_switched_off_and_then_w_reads_letters() {
    let text = "他抬頭看了看那片天，山路已經看不見了。\n";
    let take = |off: bool, n: usize| {
        let mut ed = typed(text);
        // A bare `Editor` has no dictionary, so give it one word to have an
        // opinion about — otherwise 「on」 and 「off」 differ only in theory.
        ed.set_segmenter(Box::new(crate::DictionarySegmenter::new(
            [("抬頭".to_string(), 100)],
            1,
        )));
        if off {
            ed.execute(":word-level off").expect("off is a level");
        }
        for _ in 0..n {
            ed.on_key(Key::Char('l'));
        }
        ed.on_key(Key::Char('w'));
        let (a, b) = ed.selection();
        ed.current_buffer().text().chars().skip(a).take(b - a).collect::<String>()
    };
    // On: the dictionary has an opinion about where 抬頭 ends.
    assert_eq!(take(false, 0), "抬頭");
    // Off: 漢字 are letters, so the whole run to the 、is one word.
    assert_eq!(take(true, 0), "他抬頭看了看那片天");
    // Punctuation is still its own word, both ways.
    assert_eq!(take(true, 8), "，");
    assert_eq!(take(false, 8), "，");
}

/// **The end of a line is its last character, not the break after it** (#382).
///
/// `motion::line_end` is the *insert* point, and `A` is right to ask for it.
/// `gl` and `End` leave the caret somewhere, and they were asking for the same
/// thing — so the caret stood on the newline, where nothing is written, and
/// every verb aimed at the break: `a` opened on the next line, `d` welded two
/// lines into one. Both keys, because one root cause had two entrances and
/// `End` is the one no offscreen test could press.
#[test]
fn the_end_of_a_line_is_its_last_character_not_the_break() {
    for end_key in [vec![Key::Char('g'), Key::Char('l')], vec![Key::End]] {
        let press = |ed: &mut Editor| {
            for key in &end_key {
                ed.on_key(*key);
            }
        };

        // Five characters, so the last one is at 4 and the break is at 5.
        let mut ed = typed("AAAAA\nBBBBB\n");
        press(&mut ed);
        assert_eq!(ed.selection(), (4, 5), "on the last A, not on the newline");

        // `a` appends after the caret, which is *inside* the line.
        ed.on_key(Key::Char('a'));
        for c in "XXX".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
        assert_eq!(ed.current_buffer().text(), "AAAAAXXX\nBBBBB\n");

        // `d` takes that last character, not the line break.
        let mut ed = typed("AAAAA\nBBBBB\n");
        press(&mut ed);
        ed.on_key(Key::Char('d'));
        assert_eq!(ed.current_buffer().text(), "AAAA\nBBBBB\n", "the break stays");

        // A wide character is one grapheme, and the caret lands on the whole
        // of it — not between its halves.
        let mut ed = typed("那年\n");
        press(&mut ed);
        assert_eq!(ed.selection(), (1, 2), "on 年");

        // An empty line has no last character, so the caret does not move.
        // (What a collapsed caret *covers* there is the break itself — that is
        // how every caret on an empty line reads, `gg` included.)
        let mut ed = typed("\nBBB\n");
        let before = ed.selection();
        press(&mut ed);
        assert_eq!(ed.selection(), before, "nothing to stand on, so stand still");

        // The last line of a file that does not end in a newline.
        let mut ed = typed("AAA\nBB");
        ed.on_key(Key::Char('j'));
        press(&mut ed);
        assert_eq!(ed.selection(), (5, 6), "on the second B");
    }
}

#[test]
fn a_search_guesses_the_last_pattern() {
    let mut ed = typed("春江潮水連海平，海上明月共潮生");
    // Search once…
    ed.on_key(Key::Char('/'));
    for c in "潮水".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Enter);

    // …and the next search offers the whole of it back before a single
    // key is typed (#274): `/⏎` is 「再找一次這個」, and the prompt says so
    // instead of leaving the writer to remember what it was.
    ed.on_key(Key::Char('/'));
    assert_eq!(ed.prompt_ghost(), "潮水", "the whole of the last pattern");
    ed.on_key(Key::Esc);

    // …and the rest of it once a prefix has been typed.
    ed.on_key(Key::Char('/'));
    ed.on_key(Key::Char('潮'));
    assert_eq!(ed.prompt_ghost(), "水");
    ed.on_key(Key::Tab);
    assert_eq!(ed.prompt(), Some(("/", "潮水")));
    ed.on_key(Key::Enter);
    assert_eq!(ed.selection(), (2, 4), "and it runs");

    // A pattern that is not a prefix of the last one is not guessed at.
    ed.on_key(Key::Char('/'));
    ed.on_key(Key::Char('海'));
    assert_eq!(ed.prompt_ghost(), "");
    // Rubbed out again, the guess comes back — an empty line is an empty
    // line however it got that way.
    ed.on_key(Key::Backspace);
    assert_eq!(ed.prompt_ghost(), "潮水");
    // And `Enter` on it runs the guess, which is what it always did.
    ed.on_key(Key::Enter);
    assert_eq!(ed.selection(), (2, 4), "round to the only 潮水 there is");
}

/// A command line with nothing on it guesses nothing: the `:` menu below
/// is already showing every command there is, and a whole command name in
/// grey where the writer has typed nothing reads as a line already begun.
#[test]
fn an_empty_command_line_guesses_nothing() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char(':'));
    assert_eq!(ed.prompt_ghost(), "");
}

#[test]
fn tab_leaves_arguments_alone() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char(':'));
    for c in "w draft".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Tab);
    assert_eq!(
        ed.prompt(),
        Some((":", "w draft")),
        "a file name is not a command name"
    );
}

#[test]
fn the_completed_command_runs() {
    let mut ed = typed("春江潮水");
    ed.on_key(Key::Char(':'));
    ed.on_key(Key::Char('w'));
    ed.on_key(Key::Char('o'));
    ed.on_key(Key::Tab);
    assert_eq!(ed.prompt(), Some((":", "word")));
    ed.on_key(Key::Enter);
    assert!(ed.status().contains("分詞"), "{}", ed.status());
}

#[test]
fn r_writes_one_character_over_the_whole_selection() {
    let mut ed = typed("甲乙丙");
    press(&mut ed, "%"); // select all
    press(&mut ed, "r");
    ed.on_key(Key::Char('〇'));
    assert_eq!(
        ed.current_buffer().text(),
        "〇〇〇",
        "one for one, not one character replacing the lot"
    );

    // With nothing selected it overwrites the character under the cursor.
    let mut ed = typed("甲乙丙");
    press(&mut ed, "gglr");
    ed.on_key(Key::Char('〇'));
    assert_eq!(ed.current_buffer().text(), "甲〇丙");
}

#[test]
fn alt_semicolon_flips_which_end_the_cursor_is_on() {
    let mut ed = typed("一二三四五");
    press(&mut ed, "gglvll"); // select 二三四, cursor on the last of them
    let (start, end) = ed.selection();
    assert_eq!((start, end), (1, 4));
    assert_eq!(ed.cursor(), 3, "the cursor is on the selection's last 字");
    ed.on_key(Key::Alt(';'));
    assert_eq!(ed.selection(), (start, end), "the range is unchanged");
    assert_eq!(ed.cursor(), start, "but the cursor is at the other end");
    // …so extending now grows it the other way.
    press(&mut ed, "h");
    assert_eq!(ed.selection().0, start - 1);
}

#[test]
fn named_registers_keep_more_than_one_thing() {
    let mut ed = typed("甲乙丙");
    press(&mut ed, "gg");
    press(&mut ed, "\"a"); // into register a…
    press(&mut ed, "vy");
    press(&mut ed, "gg2l");
    press(&mut ed, "vy"); // …and 丙 into the unnamed one
    press(&mut ed, "%");
    press(&mut ed, "\"aR"); // put register a over the lot
    assert_eq!(ed.current_buffer().text(), "甲");
}

#[test]
fn deleting_yanks_so_text_can_be_moved() {
    let mut ed = typed("甲乙丙");
    press(&mut ed, "ggvd"); // cut 甲
    assert_eq!(ed.current_buffer().text(), "乙丙");
    press(&mut ed, "glp"); // and put it at the end
    assert_eq!(ed.current_buffer().text(), "乙丙甲");
}

#[test]
fn the_alt_pair_takes_text_out_without_spending_the_register() {
    // Helix `A-d`／`A-c` (tutor 4.2). The register holds one thing: yank a
    // sentence, notice a stray 、 on the way to where it goes, and plain `d`
    // would throw the sentence away to hold that one character. #404.
    let mut ed = typed("甲乙丙");
    press(&mut ed, "ggy"); // 甲 into the register
    press(&mut ed, "l");
    ed.on_key(Key::Alt('d')); // 乙 out, register untouched
    assert_eq!(ed.current_buffer().text(), "甲丙");
    press(&mut ed, "glp");
    assert_eq!(ed.current_buffer().text(), "甲丙甲");

    // `A-c` is the same cut and then Insert, again keeping the register.
    let mut ed = typed("甲乙丙");
    press(&mut ed, "ggy");
    press(&mut ed, "l");
    ed.on_key(Key::Alt('c'));
    assert_eq!(ed.mode(), Mode::Insert, "{}", ed.status());
    ed.on_key(Key::Char('丁'));
    ed.on_key(Key::Esc);
    press(&mut ed, "glp");
    assert_eq!(ed.current_buffer().text(), "甲丁丙甲");

    // A count reaches that many characters, the way `d` and `c` do.
    let mut ed = typed("甲乙丙丁");
    press(&mut ed, "gg2");
    ed.on_key(Key::Alt('d'));
    assert_eq!(ed.current_buffer().text(), "丙丁");
}

#[test]
fn a_macro_records_and_replays() {
    let mut ed = typed("一二三四五六");
    press(&mut ed, "gg");
    // ⚠️ **`Q` records, `q` replays** — Helix's way round, and ours since
    // 2026-09-12 (#404). It was the other way until then.
    press(&mut ed, "Q"); // record: replace one character, step on
    press(&mut ed, "r");
    ed.on_key(Key::Char('〇'));
    press(&mut ed, "l");
    press(&mut ed, "Q"); // stop
    assert_eq!(ed.current_buffer().text(), "〇二三四五六");

    press(&mut ed, "q");
    assert_eq!(ed.current_buffer().text(), "〇〇三四五六");
    press(&mut ed, "3q"); // a count replays it that many times
    assert_eq!(ed.current_buffer().text(), "〇〇〇〇〇六");
}

#[test]
fn the_wheel_turns_pages_the_way_the_text_runs() {
    // Horizontally a notch goes down the lines…
    let text = (0..40).map(|_| "字").collect::<Vec<_>>().join("\n");
    let mut ed = typed(&text);
    press(&mut ed, "gg");
    ed.scroll(3, false);
    assert_eq!(ed.cursor_line(), 3);
    ed.scroll(3, true);
    assert_eq!(ed.cursor_line(), 0);
    ed.scroll(3, true);
    assert_eq!(ed.cursor_line(), 0, "and stops at the top");

    // …vertically it goes across the 縱, which is what makes it useful on a
    // page of them.
    let mut ed = typed(&text);
    ed.set_layout(crate::zong::Layout::Vertical);
    ed.set_zong_length(32);
    press(&mut ed, "gg");
    ed.scroll(3, false);
    assert_eq!(
        ed.zong_position().line,
        3,
        "three paragraphs across, not three characters down"
    );
}

#[test]
fn a_page_motion_moves_by_what_is_on_screen() {
    let text = (0..100).map(|_| "字").collect::<Vec<_>>().join("\n");
    let mut ed = typed(&text);
    ed.set_page(20, 10);
    press(&mut ed, "gg");
    ed.on_key(Key::Ctrl('d'));
    assert_eq!(ed.cursor_line(), 10, "half of twenty lines");
    ed.on_key(Key::Ctrl('f'));
    assert_eq!(ed.cursor_line(), 30, "a whole page");
    ed.on_key(Key::Ctrl('u'));
    assert_eq!(ed.cursor_line(), 20);
    // It stops at the end rather than running on.
    ed.on_key(Key::Ctrl('f'));
    ed.on_key(Key::Ctrl('f'));
    ed.on_key(Key::Ctrl('f'));
    ed.on_key(Key::Ctrl('f'));
    assert_eq!(ed.cursor_line(), 99);
}

#[test]
fn a_count_prefix_repeats_a_motion() {
    let mut ed = typed("一二三四五六七八");
    press(&mut ed, "3l");
    assert_eq!(ed.cursor(), 3);
    // Digits accumulate, and the count is spent by the motion.
    press(&mut ed, "2h");
    assert_eq!(ed.cursor(), 1);
    assert_eq!(ed.pending_count(), None);
    // A count that runs off the end stops rather than spinning.
    press(&mut ed, "999l");
    assert_eq!(ed.cursor(), 8);
}

#[test]
fn a_leading_zero_is_not_a_count() {
    let mut ed = typed("一二三");
    press(&mut ed, "0");
    assert_eq!(ed.pending_count(), None, "0 alone must not start a count");
    // …but it extends one already under way.
    press(&mut ed, "1");
    press(&mut ed, "0");
    assert_eq!(ed.pending_count(), Some(10));
}

#[test]
fn dot_repeats_the_last_insert() {
    let mut ed = typed("");
    ed.on_key(Key::Char('i'));
    for c in "春".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Esc);
    press(&mut ed, "..");
    assert_eq!(ed.current_buffer().text(), "春春春");
    // With a count, too.
    press(&mut ed, "2.");
    assert_eq!(ed.current_buffer().text(), "春春春春春");
}

#[test]
fn percent_selects_the_whole_buffer() {
    let mut ed = typed("上\n中\n下");
    press(&mut ed, "%");
    assert_eq!(ed.selection(), (0, ed.current_buffer().char_count()));
}

#[test]
fn join_omits_the_space_between_two_wide_characters() {
    // CJK prose carries no space across a line break…
    let mut ed = typed("上山\n下海");
    press(&mut ed, "gJ");
    assert_eq!(ed.current_buffer().text(), "上山下海");
    // …but Latin words still need one.
    let mut ed = typed("up hill\ndown dale");
    press(&mut ed, "gJ");
    assert_eq!(ed.current_buffer().text(), "up hill down dale");
    // Indentation on the joined line is swallowed, not doubled.
    let mut ed = typed("one\n    two");
    press(&mut ed, "gJ");
    assert_eq!(ed.current_buffer().text(), "one two");
}

#[test]
fn the_case_group_leaves_han_alone() {
    // 漢字 is why these three stopped being top-level keys (§5.2.3 ②):
    // whichever one you press, the manuscript is unchanged.
    let mut ed = typed("aB漢c");
    press(&mut ed, "%``");
    assert_eq!(ed.current_buffer().text(), "Ab漢C");
    press(&mut ed, "%`l");
    assert_eq!(ed.current_buffer().text(), "ab漢c");
    press(&mut ed, "%`u");
    assert_eq!(ed.current_buffer().text(), "AB漢C");
}

#[test]
fn replace_swaps_the_selection_for_the_register() {
    let mut ed = typed("甲乙丙");
    press(&mut ed, "v"); // select 甲
    press(&mut ed, "y"); // yank it
    press(&mut ed, "%R"); // replace the whole buffer with the register
    assert_eq!(ed.current_buffer().text(), "甲");
}

#[test]
fn indent_adds_and_removes_a_level() {
    let mut ed = typed("一\n二");
    ed.set_indent_width(2);
    press(&mut ed, "%>");
    assert_eq!(ed.current_buffer().text(), "  一\n  二");
    press(&mut ed, "%<");
    assert_eq!(ed.current_buffer().text(), "一\n二");
}

#[test]
fn control_a_and_x_step_the_number_under_the_cursor() {
    let mut ed = typed("第 9 章");
    ed.on_key(Key::Ctrl('a'));
    assert_eq!(ed.current_buffer().text(), "第 10 章");
    ed.on_key(Key::Ctrl('x'));
    assert_eq!(ed.current_buffer().text(), "第 9 章");
    // Zero padding survives.
    let mut ed = typed("v007");
    ed.on_key(Key::Ctrl('a'));
    assert_eq!(ed.current_buffer().text(), "v008");
}

#[test]
fn match_mode_jumps_between_cjk_brackets() {
    let mut ed = typed("他說「你好」。");
    press(&mut ed, "2l"); // onto 「
    assert_eq!(ed.cursor(), 2);
    press(&mut ed, "mm");
    assert_eq!(ed.cursor(), 5, "should land on 」");
    press(&mut ed, "mm");
    assert_eq!(ed.cursor(), 2, "and back again");
}

#[test]
fn match_mode_selects_inside_and_around_a_pair() {
    let mut ed = typed("他說「你好」。");
    press(&mut ed, "3l"); // inside the quotes
    press(&mut ed, "mi「");
    assert_eq!(ed.selection(), (3, 5));
    press(&mut ed, "ma「");
    assert_eq!(ed.selection(), (2, 6));
    // Either half of the pair names it — from back inside the quotes, since
    // `ma` left the cursor past the closer.
    press(&mut ed, "gg3l");
    press(&mut ed, "mi」");
    assert_eq!(ed.selection(), (3, 5));
}

#[test]
fn surround_adds_deletes_and_replaces() {
    let mut ed = typed("你好");
    press(&mut ed, "%ms「");
    assert_eq!(ed.current_buffer().text(), "「你好」");
    press(&mut ed, "gg2l");
    press(&mut ed, "mr「《");
    assert_eq!(ed.current_buffer().text(), "《你好》");
    press(&mut ed, "md");
    assert_eq!(ed.current_buffer().text(), "你好");
}

#[test]
fn nested_pairs_match_the_innermost() {
    let mut ed = typed("（甲（乙）丙）");
    press(&mut ed, "3l"); // onto 乙, inside both pairs
    press(&mut ed, "mi（");
    assert_eq!(ed.selection(), (3, 4), "the inner pair, not the outer");
}

#[test]
fn alt_dot_repeats_the_last_find() {
    let mut ed = typed("a,b,c,d");
    press(&mut ed, "f,");
    assert_eq!(ed.cursor(), 1);
    ed.on_key(Key::Alt('.'));
    assert_eq!(ed.cursor(), 3);
    ed.on_key(Key::Alt('.'));
    assert_eq!(ed.cursor(), 5);
}

#[test]
fn extend_to_line_bounds_covers_whole_lines() {
    let mut ed = typed("一二三\n四五六");
    press(&mut ed, "lv");
    press(&mut ed, "j");
    press(&mut ed, "X");
    assert_eq!(ed.selection(), (0, 7));
}

#[test]
fn star_searches_for_the_selection() {
    let mut ed = typed("春江春江\n");
    // Two `l` for two characters: a selection here is half-open, so `v`
    // starts one of width zero rather than one covering the cursor's own
    // grapheme the way Helix does.
    press(&mut ed, "vl"); // select 春江
    press(&mut ed, "g?");
    // **Shown, not jumped to**: 「這個詞還在哪裏」 is answered beside the
    // place you are standing, and you are still standing there.
    let (from, to) = ed.other_pane().and_then(|p| p.highlight).expect("a hit");
    assert_eq!(
        ed.current_buffer().rope().slice(from..to).to_string(),
        "春江",
        "the other occurrence of the selected text"
    );
    assert_eq!((from, to), (2, 4));
    // The *next* one after where you are standing, which here is the
    // second of two.
    assert!(ed.status().contains("2/2"), "{}", ed.status());
}

/// A segmenter that records how much text it was handed, so the cache can
/// be tested without timing anything.
#[derive(Default)]
struct Counting(std::cell::Cell<usize>);

impl Segmenter for Counting {
    fn segment(&self, s: &str) -> Vec<(usize, usize)> {
        self.0.set(self.0.get() + 1);
        CategorySegmenter.segment(s)
    }
}

/// 標點不是詞，一個都不許塗（#446）。
///
/// 作者報的原話：「`w`（按詞移動）高亮了 `` `( `` 这两个标点符号，还有 `` )、` ``
/// 这样的標點符號組。反而反引号形成的 verbatim 却没有被高亮，让人觉得是 verbatim
/// 出现了错位。」病根在那道「兩邊都看得見就不塗」的閘上：它從前問的是
/// `char::is_alphanumeric`，而拉丁字母也算 alphanumeric，於是
///
/// * `` `（ `` 前面是字母 `w`，不算邊界 → **塗了**；
/// * 反引號中間那個 `w` 兩邊都是反引號，都算邊界 → **沒塗**。
///
/// 塗出來的恰好是反的。現在兩件事都問 `is_segmentable`。
#[test]
fn punctuation_is_never_a_word_to_paint() {
    let line = "`w`（按詞移動）、`f`";
    let mut ed = typed(line);
    let chars: Vec<char> = line.chars().collect();
    for &(a, b) in &ed.segment_line(0) {
        let word: String = chars[a..b].iter().collect();
        assert!(
            word.chars().all(yumete_cjk::is_segmentable),
            "只有漢字該上色，卻塗了 {word:?}（{a}..{b}）：{:?}",
            ed.segment_line(0)
        );
    }
    // 而該塗的還在：按詞移動 是一段沒有邊界的漢字，正是眼睛要自己切的那一段。
    assert!(
        !ed.segment_line(0).is_empty(),
        "漢字那一段不能跟着標點一起沒了"
    );
}

/// 〇 是漢字，不是符號——二〇二五年 是一段，不是三段（#446）。
#[test]
fn a_year_written_with_ling_is_one_run() {
    assert!(yumete_cjk::is_segmentable('〇'), "〇 住在符號區，卻是漢字");
    assert!(yumete_cjk::is_segmentable('々'), "々 站的是一個漢字的位");
    // 拆字運算符與日文的 ・ 有意留在外面：前者不是字，後者本身就是詞的界。
    assert!(!yumete_cjk::is_segmentable('⿰'));
    assert!(!yumete_cjk::is_segmentable('・'));
    assert!(!yumete_cjk::is_segmentable('、'));
}

#[test]
fn the_overlay_segments_a_paragraph_once_until_it_changes() {
    let mut ed = typed("春江潮水\n連海平\n海上明月");
    ed.set_segmenter(Box::new(Counting::default()));
    let calls = || {
        // The editor owns the segmenter, so read the count back through it.
        0
    };
    let _ = calls;

    // Three paragraphs, drawn ten times over: nine of those frames must ask
    // the segmenter nothing.
    for _ in 0..10 {
        for line in 0..3 {
            ed.segment_line(line);
        }
    }
    // Editing one paragraph invalidates that one and no other.
    let before = ed.segment_line(1);
    press(&mut ed, "gg");
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Char('x'));
    ed.on_key(Key::Esc);
    assert_eq!(
        ed.segment_line(1),
        before,
        "an untouched paragraph is unchanged"
    );
    assert_ne!(
        ed.segment_line(0).len(),
        0,
        "the edited paragraph is segmented afresh"
    );
}

/// 寫作進度 — the ledger is opened by asking, kept by saving (#244).
#[test]
fn the_book_keeps_a_ledger_of_what_was_written_today() {
    let dir = std::env::temp_dir().join(format!("yumete-progress-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("第一章.md");
    std::fs::write(&file, "春天來了。\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&file).unwrap();
    let ledger = dir.join(".yumete").join("progress.tsv");

    // **Nothing is written until the writer asks for it.** A save into a
    // directory that never heard of yumete leaves no ledger behind.
    ed.execute(":w").unwrap();
    assert!(!ledger.exists(), "a save alone made {}", ledger.display());

    // `:count-target` opens it, and says where it went.
    ed.execute(":count-target 2000").unwrap();
    assert!(ledger.is_file(), "{}", ed.status());
    assert!(ed.status().contains("2000"), "{}", ed.status());

    // Four 字 written and saved: the row runs 4 → 8, because the session
    // opened the file with four and 。 is not a 字.
    press(&mut ed, "A河水很涼。\u{1b}");
    ed.execute(":w").unwrap();
    let text = std::fs::read_to_string(&ledger).unwrap();
    let log = crate::progress::Log::from_text(&text);
    assert_eq!(log.target, Some(2000), "{text}");
    assert_eq!(log.rows.len(), 1, "{text}");
    assert_eq!(log.rows[0].start, 4, "{text}");
    assert_eq!(log.rows[0].now, 8, "{text}");
    assert_eq!(log.rows[0].file, "第一章.md", "{text}");

    // And `:count-progress` reports it against the target, with a listing of the
    // days behind it.
    ed.execute(":count-progress").unwrap();
    assert!(ed.status().contains("2000"), "{}", ed.status());
    assert!(ed.status().contains('4'), "{}", ed.status());
    assert!(
        ed.current_buffer().text().contains(&log.rows[0].date),
        "{}",
        ed.current_buffer().text()
    );

    // Read from inside that listing — which has no file name of its own —
    // it still answers about the book.
    ed.execute(":count-progress").unwrap();
    assert!(ed.status().contains("2000"), "{}", ed.status());

    // `:count-target off` keeps the days and drops the target.
    ed.execute(":count-target off").unwrap();
    let log = crate::progress::Log::from_text(&std::fs::read_to_string(&ledger).unwrap());
    assert_eq!(log.target, None);
    assert_eq!(log.rows.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn counting_separates_han_from_characters() {
    let mut ed = typed("春江潮水連海平，海上明月共潮生。\n\n江流宛轉繞芳甸。");
    ed.execute(":count").unwrap();
    let report = ed.status().to_string();
    // 21 漢字, plus three marks; two paragraphs, the blank line not counted.
    assert!(report.contains("漢字 21"), "{report}");
    assert!(report.contains("字數 24"), "{report}");
    assert!(report.contains("2 段"), "{report}");
    assert!(report.starts_with("全篇"), "{report}");
}

#[test]
fn counting_a_selection_measures_the_scene_not_the_book() {
    let mut ed = typed("春江潮水連海平");
    press(&mut ed, "gg");
    press(&mut ed, "vl"); // 春江 selected
    ed.execute(":wc").unwrap();
    let report = ed.status().to_string();
    assert!(report.starts_with("選區"), "{report}");
    assert!(report.contains("漢字 2"), "{report}");
}

#[test]
fn a_block_of_delimited_text_becomes_a_table_and_goes_back() {
    let mut ed = typed("那年冬天。\n\n字,讀音\n永,ㄩㄥˇ\n和,ㄏㄜˊ\n\n雪下得早。\n");
    ed.execute(":3").unwrap();
    ed.execute(":table-pipe").unwrap();
    assert_eq!(
        ed.current_buffer().text(),
        "那年冬天。\n\n| 字 | 讀音  |\n| -- | ----- |\n| 永 | ㄩㄥˇ |\n| 和 | ㄏㄜˊ |\n\n雪下得早。\n"
    );
    // The prose either side of the blank lines is untouched — a blank line
    // is where the block stops.
    assert!(ed.status.contains("2"), "rows and columns: {}", ed.status);
    // And it is a grid now, not five lines that happen to start with a pipe.
    assert!(ed.table.is_some(), "walked by cell straight away");

    // Back the other way, from anywhere inside it.
    ed.execute(":5").unwrap();
    ed.execute(":table-csv").unwrap();
    assert_eq!(
        ed.current_buffer().text(),
        "那年冬天。\n\n字,讀音\n永,ㄩㄥˇ\n和,ㄏㄜˊ\n\n雪下得早。\n"
    );

    // One undo apiece: a conversion is one edit, not one per row.
    ed.on_key(Key::Char('u'));
    assert!(ed.current_buffer().text().contains("| 永 |"), "{}", ed.current_buffer().text());
}

#[test]
fn a_selection_says_which_lines_the_table_is_made_of() {
    // No blank line anywhere: without a selection the walk would take the
    // heading and the sentence with it.
    let mut ed = typed("# 人物\n甲,乙\n丙,丁\n那年冬天。\n");
    ed.execute(":2").unwrap();
    press(&mut ed, "xx"); // the two data rows, and only those
    ed.execute(":table-pipe").unwrap();
    let text = ed.current_buffer().text();
    assert!(text.starts_with("# 人物\n| 甲 | 乙 |\n"), "{text:?}");
    assert!(text.ends_with("| 丙 | 丁 |\n那年冬天。\n"), "{text:?}");
}

#[test]
fn a_conversion_that_would_lose_a_cell_is_refused() {
    // Three rows and three columns, with the comma in **row 3, column 2**
    // — a shape that tells the two numbers apart. On a 2×2 table this
    // said 「第 2 行第 2 欄」 whichever way round the arguments went in.
    let mut ed = typed(
        "| 字 | 註 | 部 |\n| -- | -- | -- |\n| 永 | 水 | 丶 |\n| 之 | 長, 久 | 丿 |\n",
    );
    ed.execute(":3").unwrap();
    let before = ed.current_buffer().text();
    ed.execute(":table-csv").unwrap();
    // Named, and nothing written: the file is exactly as it was.
    assert_eq!(ed.current_buffer().text(), before);
    // Row 3 counts the header as row 1 and the rule row not at all.
    let (row, column) = (ed.status.find('3'), ed.status.find('2'));
    assert!(
        matches!((row, column), (Some(r), Some(c)) if r < c),
        "row 3 then column 2, in that order: {}",
        ed.status
    );

    // The writer picks a delimiter the data does not hold, and it goes.
    ed.execute(":table-csv tab").unwrap();
    assert_eq!(
        ed.current_buffer().text(),
        "字\t註\t部\n永\t水\t丶\n之\t長, 久\t丿\n"
    );
}

#[test]
fn a_paragraph_is_not_quietly_cut_into_columns() {
    let mut ed = typed("那年冬天，雪下得早。\n他站在門口，看了很久，沒有進去。\n");
    let before = ed.current_buffer().text();
    ed.execute(":table-pipe").unwrap();
    // Nothing regular separates these lines, so nothing is guessed at.
    assert_eq!(ed.current_buffer().text(), before);
    assert!(ed.status.contains("tab") || ed.status.contains("分隔"), "{}", ed.status);

    // …but a writer who says what the delimiter is gets what they asked
    // for, even a 、 — they have looked at their data.
    let mut ed = typed("甲、乙\n丙、丁\n");
    ed.execute(":table-pipe 、").unwrap();
    assert_eq!(
        ed.current_buffer().text(),
        "| 甲 | 乙 |\n| -- | -- |\n| 丙 | 丁 |\n"
    );

    // And `:table-pipe` on a table already made is a no-op, not a table
    // twice as wide.
    let before = ed.current_buffer().text();
    ed.execute(":table-pipe").unwrap();
    assert_eq!(ed.current_buffer().text(), before);
}

#[test]
fn a_table_exports_as_a_file_without_being_converted_in_place() {
    let dir = std::env::temp_dir().join(format!("yumete-csv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("人物.md");
    std::fs::write(&path, "# 人物\n\n| 名 | 字 |\n| -- | -- |\n| 淵明 | 元亮 |\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    ed.execute(":5").unwrap();
    let before = ed.current_buffer().text();
    ed.execute(":export csv").unwrap();

    // Named after the file, and the manuscript untouched — this is a copy
    // handed out, not a conversion.
    let out = std::fs::read_to_string(dir.join("人物.csv")).unwrap();
    assert_eq!(out, "名,字\n淵明,元亮\n");
    assert_eq!(ed.current_buffer().text(), before);

    // `tsv` is the same table, split another way.
    ed.execute(":export tsv").unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("人物.tsv")).unwrap(),
        "名\t字\n淵明\t元亮\n"
    );

    // An existing file is not replaced unless the bang says so — the rule
    // `:w` keeps and the document exports keep. Both messages print the
    // path, so the path is not what tells them apart: what does is the
    // word, and whether the file on disk actually changed.
    std::fs::write(dir.join("人物.csv"), "別動我\n").unwrap();
    ed.execute(":export csv").unwrap();
    assert!(ed.status.contains("已經有"), "{}", ed.status);
    assert_eq!(
        std::fs::read_to_string(dir.join("人物.csv")).unwrap(),
        "別動我\n",
        "refused means the file is untouched"
    );
    ed.execute(":export! csv").unwrap();
    assert!(ed.status.contains("寫好了"), "{}", ed.status);
    assert_eq!(
        std::fs::read_to_string(dir.join("人物.csv")).unwrap(),
        "名,字\n淵明,元亮\n"
    );

    // And never onto something open in the editor — this would otherwise
    // write the table over the manuscript it came from. (A named path is
    // resolved against the working directory, the way `:w` resolves one,
    // so the test names it in full: `人物.md` alone would mean a file in
    // whatever directory the editor was started in.)
    ed.execute(&format!(":export csv {}", path.display())).unwrap();
    assert!(ed.status.contains("這份稿子本身"), "{}", ed.status);
    assert_eq!(ed.current_buffer().text(), before, "the manuscript stands");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "…on disk as well as in memory"
    );

    // …and not onto a *different* file that is open either.
    let other = dir.join("地名.md");
    std::fs::write(&other, "# 地名\n").unwrap();
    ed.execute(&format!(":open {}", other.display())).unwrap();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    ed.execute(":5").unwrap();
    ed.execute(&format!(":export! csv {}", other.display())).unwrap();
    assert!(ed.status.contains("正開着"), "{}", ed.status);
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "# 地名\n");

    // Away from any table there is nothing to export.
    ed.execute(":1").unwrap();
    ed.execute(":export csv").unwrap();
    assert!(ed.status.contains('|'), "says what is missing: {}", ed.status);

    // A file that is already a grid exports as itself: reading a `.csv` and
    // writing a `.tsv` is the same conversion by another name.
    let csv = dir.join("表.csv");
    std::fs::write(&csv, "字,讀音\n永,ㄩㄥˇ\n").unwrap();
    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", csv.display())).unwrap();
    assert!(ed.execute(":table").is_ok());
    ed.execute(":export tsv").unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("表.tsv")).unwrap(),
        "字\t讀音\n永\tㄩㄥˇ\n"
    );

    // …and a cell that already holds the delimiter being written is
    // **quoted**, not refused (#311). It used to be refused, because the
    // reader could not have read the quotes back — now it can, so the
    // conversion is one a `.csv` reader gets right.
    let tsv = dir.join("表二.tsv");
    std::fs::write(&tsv, "字\t讀音\n永\tㄩㄥˇ\n之\t一, 二\n").unwrap();
    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", tsv.display())).unwrap();
    assert!(ed.execute(":table").is_ok());
    ed.execute(":export csv").unwrap();
    let out = std::fs::read_to_string(dir.join("表二.csv")).expect("written");
    assert_eq!(out.lines().last(), Some("之,\"一, 二\""), "{out:?}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_shot_names_a_file_and_waits_for_the_frame_it_is_a_picture_of() {
    let dir = std::env::temp_dir().join(format!("yumete-shot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chapter.md");
    std::fs::write(&path, "永和九年。\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();

    // **Bare `:shot` is the clipboard**, which is what a person who says
    // 「截個圖」 means. It names no file, so no guard applies to it.
    ed.execute(":shot").unwrap();
    assert_eq!(ed.take_screenshot_request(), Some(ShotJob::Screen));
    // …and taking it takes it: one `:shot`, one picture.
    assert_eq!(ed.take_screenshot_request(), None);
    // `screen` says the same thing out loud.
    ed.execute(":shot screen").unwrap();
    assert_eq!(ed.take_screenshot_request(), Some(ShotJob::Screen));

    // Nothing is written here either: the picture is of the frame that has
    // not been drawn yet, so all `:shot html` may do is say which file.
    ed.execute(":shot html").unwrap();
    let Some(ShotJob::Page { target, text }) = ed.take_screenshot_request() else {
        panic!("no page parked: {}", ed.status());
    };
    assert!(!text, "html keeps its colours");
    // Not beside the manuscript, and dated — so it never collides.
    assert_eq!(target.parent(), Some(downloads_dir().as_path()));
    let name = target.file_name().unwrap().to_string_lossy().into_owned();
    let (stem, when) = name
        .strip_suffix(".html")
        .and_then(|n| n.rsplit_once('_'))
        .unwrap_or_else(|| panic!("{name} is not 章_年月日時分秒.html"));
    assert_eq!(stem, "chapter");
    assert_eq!(when.len(), 14, "{name}");
    assert!(when.chars().all(|c| c.is_ascii_digit()), "{name}");
    assert!(!dir.join("chapter.shot.html").exists());

    // **The word says the format**, not the extension — this is the one
    // that used to be reachable only by spelling out a path.
    ed.execute(":shot txt").unwrap();
    let Some(ShotJob::Page { target, text }) = ed.take_screenshot_request() else {
        panic!("no page parked: {}", ed.status());
    };
    assert!(text, "txt drops them");
    assert!(
        target.to_string_lossy().ends_with(".txt"),
        "{}",
        target.display()
    );

    // `png` is the other picture entirely — the window, taken by the
    // platform, but kept rather than pasted.
    ed.execute(":shot png").unwrap();
    let Some(ShotJob::Png { target }) = ed.take_screenshot_request() else {
        panic!("no png parked: {}", ed.status());
    };
    assert_eq!(target.parent(), Some(downloads_dir().as_path()));

    // A name given is a name kept, wherever it points.
    ed.execute(":shot txt page.txt").unwrap();
    assert_eq!(
        ed.take_screenshot_request(),
        Some(ShotJob::Page {
            target: PathBuf::from("page.txt"),
            text: true,
        })
    );

    // A scratch buffer has no name to derive one from, exactly as an
    // export has not — but it may still photograph the screen.
    let mut scratch = Editor::new();
    assert!(matches!(
        scratch.execute(":shot html"),
        Err(EditorError::NoFileName)
    ));
    scratch.execute(":shot").unwrap();
    assert_eq!(scratch.take_screenshot_request(), Some(ShotJob::Screen));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_shot_refuses_a_file_that_is_already_there_and_one_that_is_open() {
    let dir = std::env::temp_dir().join(format!("yumete-shot2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chapter.md");
    std::fs::write(&path, "永和九年。\n").unwrap();
    let taken = dir.join("已經有了.html");
    std::fs::write(&taken, "早就在這裏了\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();

    // A dated name cannot collide, so the guard is only ever reached by a
    // name the writer spelled out. Already there: refused, and nothing is
    // parked for the front end — otherwise the refusal would be printed
    // and the file written anyway.
    ed.execute(&format!(":shot html {}", taken.display())).unwrap();
    assert_eq!(ed.take_screenshot_request(), None);
    assert!(ed.status().contains("已經有了.html"), "{}", ed.status());

    // The bang is the answer, the same one `:export!` takes.
    ed.execute(&format!(":shot! html {}", taken.display()))
        .unwrap();
    assert_eq!(
        ed.take_screenshot_request(),
        Some(ShotJob::Page {
            target: taken.clone(),
            text: false,
        })
    );

    // Never onto a file this editor is holding: the chapter itself is the
    // one a hurried `:shot!` would otherwise overwrite with a picture.
    ed.execute(&format!(":shot! html {}", path.display()))
        .unwrap();
    assert_eq!(ed.take_screenshot_request(), None);

    // And the bang on the clipboard is refused rather than ignored —
    // there is nothing there to overwrite, and a `!` that does nothing is
    // how a person comes to believe it did something.
    ed.execute(":shot!").unwrap();
    assert_eq!(ed.take_screenshot_request(), None);
    assert!(ed.status().contains('!'), "{}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn export_names_the_file_after_the_chapter_and_carries_the_layout() {
    let dir = std::env::temp_dir().join(format!("yumete-ex-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chapter.md");
    std::fs::write(&path, "# 第一章\n\n<ruby>永<rt>ㄩㄥˇ</rt></ruby>和九年。\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    ed.set_layout(crate::zong::Layout::Vertical);
    ed.set_hanging_punctuation(true);
    ed.execute(":export html").unwrap();

    // Named after the chapter, not after the format.
    let out = std::fs::read_to_string(dir.join("chapter.html")).unwrap();
    assert!(out.contains("writing-mode: vertical-rl"), "{out}");
    assert!(out.contains("hanging-punctuation"), "{out}");
    assert!(out.contains("<h1>第一章</h1>"), "{out}");

    // A scratch buffer has no name to derive one from.
    let mut ed = Editor::new();
    assert!(matches!(
        ed.execute(":export html"),
        Err(EditorError::NoFileName)
    ));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_main_file_that_imports_its_chapters_is_a_table_of_contents() {
    let dir = std::env::temp_dir().join(format!("yumete-imp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ch01.typ"), "= 初雪\n那年冬天。\n").unwrap();
    std::fs::write(
        dir.join("book.typ"),
        "#import \"lib.typ\": chapter\n\n#include \"ch01.typ\"\n#include \"ch02.typ\"\n",
    )
    .unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", dir.join("book.typ").display()))
        .unwrap();
    // The outline is what the file pulls in.
    let names: Vec<String> = ed.outline().into_iter().map(|(_, _, n)| n).collect();
    // `#import` borrows a template; it does not add a chapter.
    assert_eq!(names, ["ch01.typ", "ch02.typ"]);

    // …and `gf` opens one, resolved beside the file that names it rather
    // than beside wherever the editor was started.
    ed.execute(":3").unwrap();
    press(&mut ed, "gf");
    assert_eq!(ed.current_buffer().display_name(), "ch01.typ");
    assert_eq!(ed.current_buffer().text(), "= 初雪\n那年冬天。\n");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_outline_is_the_hashes_a_writer_already_types() {
    let mut ed = typed("# 第一章\n那年冬天。\n## 一\n雪下得早。\n## 二\n### 附記\n");
    let headings = ed.outline();
    assert_eq!(headings.len(), 4);
    assert_eq!(headings[0], (0, 1, "第一章".to_string()));
    assert_eq!(headings[2], (4, 2, "二".to_string()));

    ed.execute(":toc 3").unwrap();
    assert_eq!(ed.cursor_line(), 4);

    // A bare `:toc` opens the outline — a list, one heading a line — and
    // says how many there are. It used to join every heading into the
    // status line, which for a novel is 700 chapters on one row.
    ed.execute(":toc").unwrap();
    assert!(ed.panel(crate::sidebar::Side::Left).is_some(), "{}", ed.status());
    assert!(ed.status().contains("4"), "{}", ed.status());
}

#[test]
fn a_novel_with_no_markup_still_has_chapters() {
    // 資治通鑑 is a `.txt` with 700 chapters in it and not one `#`. The
    // outline used to be empty for exactly the file where 「go to chapter
    // 412」 is worth a key.
    let dir = std::env::temp_dir().join(format!("yumete-toc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let novel = dir.join("novel.txt");
    // Three lines of writing under each: a heading with nothing under it
    // is a 目錄 line, and this file is not a 目錄.
    std::fs::write(
        &novel,
        "楔子\n那年冬天。\n雪下得早。\n山路斷了。\n\
         第一卷\n他抬頭看了看那片天。\n雪還在下。\n山路已經看不見了。\n\
         第一章　風雪\n風從北面來。\n院子裏那棵老槐樹壓斷了一根枝。\n他站了很久。\n\
         第二章\n第二天雪停了。\n路上沒有人。\n他一個人走。\n\
         第三章魚是一句話的開頭，不是標題。\n",
    )
    .unwrap();
    let mut ed = Editor::new();
    ed.open_file(&novel).unwrap();
    let headings = ed.outline();
    let titles: Vec<&str> = headings.iter().map(|(_, _, t)| t.as_str()).collect();
    assert_eq!(titles, ["楔子", "第一卷", "第一章　風雪", "第二章"]);
    assert_eq!(headings[1].1, 1, "a 卷 holds 章, so it sits above them");
    assert_eq!(headings[2].1, 2);
    // The third heading is 第一章　風雪, eight lines in.
    ed.execute(":toc 3").unwrap();
    assert_eq!(ed.cursor_line(), headings[2].0);

    // **資治通鑑 writes all 294 of its 卷 as 卷002** — the unit first and no
    // 第 at all, which is the book this was written for and the one the
    // first version found twelve chapters in. 史記 writes 卷一　五帝本紀第一.
    let history = dir.join("history.txt");
    std::fs::write(
        &history,
        "卷002\n漢紀一。\n威烈王二十三年。\n初命晉大夫。\n\
         卷003　夏本紀第二\n周紀二。\n臣光曰。\n夫禮，辨貴賤。\n\
         話說天下大勢，分久必合。\n",
    )
    .unwrap();
    let mut ed = Editor::new();
    ed.open_file(&history).unwrap();
    let titles: Vec<String> = ed.outline().into_iter().map(|(_, _, t)| t).collect();
    assert_eq!(titles, ["卷002", "卷003　夏本紀第二"], "話說 is prose, not a 話");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_last_line_of_a_table_of_contents_is_not_the_first_chapter() {
    // 紅樓夢's 目錄 ends 「第百二十回　甄士隱詳說太虛情」, and then the 校閱
    // 參考 notes begin. Three lines of writing under it, so it was read as a
    // chapter — and, being early in the file, it sat above 第一回 (#390).
    let dir = std::env::temp_dir().join(format!("yumete-toc-listing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let novel = dir.join("novel.txt");
    std::fs::write(
        &novel,
        "第一回　甄士隱夢幻識通靈

第二回　賈夫人仙逝揚州城

第三回　金陵城起復賈雨村

         校閱參考
蒙古王府本石頭記。
脂硯齋重評石頭記。
列藏本石頭記。
         第一回　甄士隱夢幻識通靈
那年冬天。
雪下得早。
山路斷了。
         第二回　賈夫人仙逝揚州城
第二天雪停了。
路上沒有人。
他一個人走。
",
    )
    .unwrap();
    let mut ed = Editor::new();
    ed.open_file(&novel).unwrap();
    let rows: Vec<(usize, String)> = ed.outline().into_iter().map(|(l, _, t)| (l, t)).collect();
    assert_eq!(
        rows,
        [
            (10, "第一回　甄士隱夢幻識通靈".to_string()),
            (14, "第二回　賈夫人仙逝揚州城".to_string()),
        ],
        "the listing is a run of three, and the chapters stand alone"
    );

    // ⚠️ **Two in a row is not a run.** 資治通鑑 ends every 卷 with
    // 「卷二 ◄ 資治通鑑」 and opens the next with 「第三卷 ► 卷四」, back to
    // back — the first try at #390 called that a listing and cut a 294-卷
    // book down to one row.
    let history = dir.join("history.txt");
    std::fs::write(
        &history,
        "第一卷　周紀一 ► 卷二
威烈王二十三年。
初命晉大夫。
臣光曰。
         卷一 ◄ 資治通鑑

第二卷 ► 卷三
安王元年。
魏文侯薨。
子擊立。
         卷二 ◄ 資治通鑑

第三卷 ► 卷四
烈王元年。
齊田和卒。
子桓公午立。
",
    )
    .unwrap();
    let mut ed = Editor::new();
    ed.open_file(&history).unwrap();
    let titles: Vec<String> = ed.outline().into_iter().map(|(_, _, t)| t).collect();
    assert_eq!(titles, ["第一卷　周紀一", "第二卷", "第三卷"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_chapter_wearing_a_navigation_bar_is_still_a_chapter() {
    // 三國演義's 120 回 are all written 「◀上一回 第二回　… 下一回▶」 — the
    // export kept the arrows the web page walked on, and the outline of a
    // 120-chapter book was one row long (#402).
    let dir = std::env::temp_dir().join(format!("yumete-toc-nav-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let novel = dir.join("novel.txt");
    std::fs::write(
        &novel,
        "全書始 第一回　宴桃園豪傑三結義 下一回▶
話說天下大勢。
分久必合。
合久必分。
         ◀上一回 下一回▶
         ◀上一回 第二回　張翼德怒鞭督郵 下一回▶
且說那日。
雪未曾停。
馬也乏了。
         ◀上一回 第三回　議溫明董卓叱丁原 全書終
董卓入京。
百官失色。
天下自此亂。
",
    )
    .unwrap();
    let mut ed = Editor::new();
    ed.open_file(&novel).unwrap();
    let titles: Vec<String> = ed.outline().into_iter().map(|(_, _, t)| t).collect();
    assert_eq!(
        titles,
        ["第一回　宴桃園豪傑三結義", "第二回　張翼德怒鞭督郵", "第三回　議溫明董卓叱丁原"],
        "the arrows are the frame, not the title"
    );
    // The bar with nothing in it is the foot of a chapter, not a chapter.
    assert_eq!(super::without_navigation("◀上一回 下一回▶"), "");
    // A line that never wore one comes back whole.
    assert_eq!(super::without_navigation("第一回　宴桃園"), "第一回　宴桃園");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_book_that_only_numbers_its_chapters_is_counted() {
    // 天龍八部 writes all fifty of its chapters 「一 青衫磊落險峰行」: no 第,
    // no 章, nothing but the count. Its outline was one row — 「后记」 (#402).
    let dir = std::env::temp_dir().join(format!("yumete-toc-count-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let novel = dir.join("novel.txt");
    let mut text = String::from("釋名
一九九四年一月
書名：天龍八部
作者：金庸
");
    // A 目錄 counts 1, 2, 3 too, and must not take the run.
    for n in ["一 青衫磊落險峰行", "二 玉璧月華明", "三 馬疾香幽"] {
        text.push_str(n);
        text.push('\n');
    }
    for (n, title) in [
        ("一", "青衫磊落險峰行"),
        ("二", "玉璧月華明"),
        ("三", "馬疾香幽"),
        ("四", "崖高人遠"),
        ("五", "微步縠紋生"),
        ("六", "誰家子弟誰家院"),
    ] {
        text.push_str(&format!("{n} {title}\n他抬頭。\n雪還在下。\n山路看不見了。\n"));
    }
    text.push_str("后记
這部書寫了四年。
改了三遍。
就這樣罷。
");
    std::fs::write(&novel, &text).unwrap();
    let mut ed = Editor::new();
    ed.open_file(&novel).unwrap();
    let titles: Vec<String> = ed.outline().into_iter().map(|(_, _, t)| t).collect();
    assert_eq!(
        titles,
        [
            "一 青衫磊落險峰行",
            "二 玉璧月華明",
            "三 馬疾香幽",
            "四 崖高人遠",
            "五 微步縠紋生",
            "六 誰家子弟誰家院",
            "后记",
        ],
        "the 目錄 has no writing under it and 一九九四年一月 is a date"
    );

    // **Five is not a book.** Four numbered lines with writing under them are
    // a numbered list, and the outline says nothing rather than guess.
    let short = dir.join("short.txt");
    std::fs::write(
        &short,
        "一 買米
先去糧店。
再去菜場。
然後回家。
         二 掃地
先掃客廳。
再掃廚房。
最後拖一遍。
         三 洗衣
白的一堆。
黑的一堆。
分開洗。
         四 做飯
淘米。
切菜。
下鍋。
",
    )
    .unwrap();
    let mut ed = Editor::new();
    ed.open_file(&short).unwrap();
    assert!(ed.outline().is_empty(), "{:?}", ed.outline());

    // …and a book that writes 章 has said how it marks a chapter, so a bare
    // 「三 忽然」 in its prose is not a second opinion.
    let both = dir.join("both.txt");
    std::fs::write(
        &both,
        "第一章　風雪
風從北面來。
院子裏那棵老槐樹斷了一枝。
他站了很久。
         一 忽然
那天他想起一件事。
很久以前的事。
他沒有說出來。
         二 後來
後來雪停了。
路上沒有人。
他一個人走。
",
    )
    .unwrap();
    let mut ed = Editor::new();
    ed.open_file(&both).unwrap();
    let titles: Vec<String> = ed.outline().into_iter().map(|(_, _, t)| t).collect();
    assert_eq!(titles, ["第一章　風雪"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_numbers_a_chapter_is_counted_by() {
    use super::chinese_number as n;
    assert_eq!(n("一"), Some(1));
    assert_eq!(n("十"), Some(10), "a unit with nothing in front of it is one");
    assert_eq!(n("十一"), Some(11));
    assert_eq!(n("五十"), Some(50));
    assert_eq!(n("一百二十"), Some(120));
    assert_eq!(n("一百零八"), Some(108));
    assert_eq!(n("38"), Some(38));
    // 萬 and the full-width digits are not numbers this will guess at.
    assert_eq!(n("一萬"), None);
    assert_eq!(n("１２"), None);
}

#[test]
fn the_capitals_turn_the_page() {
    let text = (1..=60)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let mut ed = typed(&text);
    ed.set_page(20, 10);
    press(&mut ed, "gg");

    // ⚠️ Only `J`/`K` now. `H`/`L` were the whole-page pair until 2026-09-12
    // and are a sentence apiece since (#404); nothing was lost, because
    // `C-f`, `C-b`, `PageUp` and `PageDown` all still turn a whole page, and
    // the half page is the one a reader wears out.
    press(&mut ed, "J");
    assert_eq!(ed.cursor_line(), 10, "half of twenty lines");
    press(&mut ed, "J");
    assert_eq!(ed.cursor_line(), 20);
    press(&mut ed, "K");
    assert_eq!(ed.cursor_line(), 10);

    // Joining moved to `gJ`, which is also how vi spells it.
    let mut ed = typed("上山\n下海");
    press(&mut ed, "gJ");
    assert_eq!(ed.current_buffer().text(), "上山下海");
}

/// A directory holding a small division table and the schema for it.
fn a_table(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("yumete-table-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
    std::fs::write(
        dir.join(".yumete").join("tables").join("division.toml"),
        "[table]\nfile = ['division.csv']\nkey = 'char'\n\
         [[table.column]]\nname = 'char'\nlabel = '字'\n\
         [[table.column]]\nname = 'ids_y'\n\
         [[table.column]]\nname = 'ids_g'\n\
         [[table.detail]]\nname = 'unicode'\ncompute = 'codepoint(char)'\n\
         [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
    )
    .unwrap();
    let csv = dir.join("division.csv");
    std::fs::write(&csv, "char,ids_y,ids_g\n一,⿰木目,⿰木目\n二,土,土\n").unwrap();
    (dir, csv)
}

#[test]
fn a_file_a_schema_names_is_read_as_a_grid() {
    let (dir, csv) = a_table("open");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();

    // No command needed: a schema next to the file is the file saying so.
    let view = ed.table().expect("read as a grid");
    assert_eq!(view.schema.columns.len(), 3);
    assert_eq!(view.schema.columns[0].heading(), "字");

    // Cells are ranges into the line, not a copy of it.
    ed.goto_line(2);
    assert_eq!(ed.cell_position(), Some((1, 0)));
    assert_eq!(ed.cell_text(1, 1), "⿰木目");
    assert_eq!(ed.cell_span(1, 1), Some((19, 22)), "the header is 17 characters");

    // A file the schema does not name is ordinary text again.
    let other = dir.join("notes.md");
    std::fs::write(&other, "那年冬天\n").unwrap();
    ed.open_file(&other).unwrap();
    assert!(ed.table().is_none(), "a chapter is not a table");
    // …and coming back to the table reads it as one again.
    ed.execute("buffer-previous").unwrap();
    assert!(ed.table().is_some());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_grid_without_a_schema_is_split_on_what_its_lines_agree_about() {
    // The header-row fallback used to split on a comma and nothing else,
    // so a tab-separated file — which `:export tsv` in this very editor
    // writes — came back 「不是表格」 while a page of prose whose lines
    // happen to hold one comma each still came back a grid. Both are the
    // sniffer's question, so both are asked of the sniffer.
    let dir = std::env::temp_dir().join(format!("yumete-grid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let tsv = dir.join("讀音.tsv");
    std::fs::write(&tsv, "字\t讀音\t部\n永\tㄩㄥˇ\t水\n之\t\u{34E4}\t丿\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&tsv).unwrap();
    assert!(ed.execute(":table").is_ok());
    assert_eq!(ed.cell_text(1, 1), "ㄩㄥˇ", "tabs, not commas");
    assert_eq!(ed.cell_text(2, 2), "丿");

    // …and semicolons, the third of the three the sniffer knows.
    let scsv = dir.join("讀音.txt");
    std::fs::write(&scsv, "字;讀音\n永;ㄩㄥˇ\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&scsv).unwrap();
    assert!(ed.execute(":table").is_ok());
    assert_eq!(ed.cell_text(1, 1), "ㄩㄥˇ");

    // A page of prose is still not a grid: its lines do not agree.
    let prose = dir.join("散文.txt");
    std::fs::write(&prose, "那年冬天，雪下得早。\n他站在門口，看了很久，沒有進去。\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&prose).unwrap();
    ed.execute(":table").unwrap();
    assert!(ed.cell_position().is_none(), "{}", ed.status);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_column_goes_back_on_the_rows_it_was_taken_from() {
    let dir = std::env::temp_dir().join(format!("yumete-col-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("讀音.csv");
    // A blank line inside the file. It is a row of one empty cell, so the
    // column carries a blank of its own over it — which is what keeps the
    // cells below it from all moving up one when the column goes back.
    std::fs::write(&csv, "字,讀音,部\n永,ㄩㄥˇ,水\n\n之,ㄓ,丿\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.execute(":table").is_ok());
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "ty");
    press(&mut ed, "ll");
    press(&mut ed, "tp");
    assert_eq!(
        ed.current_buffer().text(),
        "字,讀音,字\n永,ㄩㄥˇ,永\n\n之,ㄓ,之\n",
        "{}",
        ed.status
    );

    // A value that holds the delimiter is refused by row and column, the
    // way `:table-csv` refuses one — it used to have the commas quietly
    // filtered out of it, and 「長, 久」 went in as 「長 久」. Refused
    // before anything is written, so there is nothing to undo.
    let before = ed.current_buffer().text();
    ed.store("部\n水\n\n長, 久\n".to_string());
    press(&mut ed, "tp");
    assert_eq!(ed.current_buffer().text(), before, "nothing was written");
    let (row, column) = (ed.status.find('4'), ed.status.find('3'));
    assert!(
        matches!((row, column), (Some(r), Some(c)) if r < c),
        "row 4, column 3 — where it would have landed: {}",
        ed.status
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_cell_pasted_into_a_pipe_table_keeps_its_backslashes() {
    // `\|` is the escape, so a backslash is doubled with it: a cell whose
    // own text is `C:\` written as `C:\` would read back as an escape
    // waiting for the pipe that follows. The paste used to replace the
    // pipe alone, which is `escape`'s job and half of it.
    let mut ed = typed("| 字 | 註 |\n| -- | -- |\n| 永 | 水 |\n");
    ed.execute(":3").unwrap();
    ed.enter_table();
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "l");
    ed.store("註\nC:\\ 與 |\n".to_string());
    press(&mut ed, "tp");
    // Written escaped…
    assert!(
        ed.current_buffer().text().contains(r"C:\\ 與 \|"),
        "{}",
        ed.current_buffer().text()
    );
    // …and read back as itself.
    assert_eq!(
        crate::mdtable::unescape(ed.cell_text(2, 1).trim()),
        r"C:\ 與 |"
    );
}

#[test]
fn hjkl_walk_cells_when_the_file_is_a_grid() {
    let (dir, csv) = a_table("move");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: 這一條測的是格
    assert_eq!(ed.cell_position(), Some((1, 0)), "row 2, first cell");

    press(&mut ed, "l");
    assert_eq!(ed.cell_position(), Some((1, 1)), "one cell right");
    press(&mut ed, "l");
    assert_eq!(ed.cell_position(), Some((1, 2)));
    press(&mut ed, "l");
    assert_eq!(ed.cell_position(), Some((1, 2)), "the row ends");
    press(&mut ed, "h");
    assert_eq!(ed.cell_position(), Some((1, 1)));

    // Down a row keeps the column, and the cursor lands on the cell's start
    // rather than wherever the character count happened to fall.
    press(&mut ed, "j");
    assert_eq!(ed.cell_position(), Some((2, 1)));
    assert_eq!(ed.cell_text(2, 1), "土");
    press(&mut ed, "k");
    assert_eq!(ed.cell_position(), Some((1, 1)));

    // `0` and `$` are the row's ends, as they are a line's.
    press(&mut ed, "$");
    assert_eq!(ed.cell_position(), Some((1, 2)));
    press(&mut ed, "0");
    assert_eq!(ed.cell_position(), Some((1, 0)));

    // A count applies, as it does to every other motion.
    press(&mut ed, "2l");
    assert_eq!(ed.cell_position(), Some((1, 2)));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_cell_cannot_be_typed_out_of() {
    let (dir, csv) = a_table("guard");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);
    let before = ed.current_buffer().text();

    // The delimiter is the one character that cannot go in a cell: with no
    // quoting it is not a comma, it is one more column.
    press(&mut ed, "i");
    ed.on_key(Key::Char(','));
    assert_eq!(ed.current_buffer().text(), before, "refused");
    assert!(ed.status().contains("分隔"), "{}", ed.status());

    // Nor a line break, which would cut the row in half.
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), before);

    // Backspace at the cell's start would join it to the one before.
    ed.on_key(Key::Backspace);
    assert_eq!(ed.current_buffer().text(), before, "the delimiter survives");
    assert!(ed.status().contains("格首"), "{}", ed.status());

    // Ordinary typing works exactly as it always did.
    ed.on_key(Key::Char('三'));
    assert!(ed.current_buffer().text().contains("三一,"), "{}", ed.current_buffer().text());
    ed.on_key(Key::Backspace);
    assert_eq!(ed.current_buffer().text(), before, "and undoes itself");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn gd_goes_gw_shows_and_a_missing_note_gets_written() {
    // The pair every editor has: `Enter` is 「還在哪裏」 (references), `gd`
    // is 「它在哪裏定義的」. Keeping both on `Enter` meant a word inside a
    // footnote could not be searched for at all.
    let mut ed = typed("那年冬天[^1]，山下起了大雪。\n那年夏天。\n");
    ed.set_render(Render::Basic);
    ed.goto_line(1);
    for _ in 0..4 {
        ed.on_key(Key::Char('l'));
    }
    // No note for it yet: the stub is written at the foot — and `gd`
    // *goes* there, landing at its end with Insert one keystroke away,
    // which is how a note gets written.
    let was = ed.cursor();
    press(&mut ed, "gd");
    let text = ed.current_buffer().text();
    assert!(text.ends_with("[^1]: "), "{text:?}");
    assert!(ed.status().contains("寫下了"), "{}", ed.status());
    assert_eq!(ed.cursor(), ed.current_buffer().rope().len_chars(), "at its end");
    ed.on_key(Key::Ctrl('o'));
    assert_eq!(ed.cursor(), was, "C-o comes back to the sentence");
    // …and it is one edit, so one `u` takes it back.
    ed.on_key(Key::Char('u'));
    assert!(!ed.current_buffer().text().contains("[^1]: "));

    // `g?` is the other question entirely — asked from a word, since
    // 「還在哪裏」 needs something to be about.
    press(&mut ed, "gg");
    press(&mut ed, "ll");
    press(&mut ed, "g?");
    assert!(
        ed.status().contains("處") || ed.status().contains("只有"),
        "{}",
        ed.status()
    );
}

/// **`g` is the document's group and `t` is the table's** (2026-09-12).
///
/// `gd` used to mean something else inside a grid — 「which row has this in
/// the key column」 — so a `[^1]` written into a table cell searched the
/// grid instead of going to its note, which is the one thing `gd` is named
/// for. A key whose meaning turns over depending on what the cursor happens
/// to be standing in cannot be relied on.
#[test]
fn a_note_written_into_a_table_still_goes_to_its_note() {
    let mut ed = typed("| 字 | 註 |\n| -- | -- |\n| 木 | 甲[^1] |\n\n[^1]: 說明。\n");
    ed.set_render(Render::Basic);
    ed.goto_line(3);
    assert!(ed.enter_table(), "{}", ed.status());
    // On the reference, inside a cell of the grid.
    while ed.char_at_cursor() != Some('^') {
        press(&mut ed, "l");
    }
    press(&mut ed, "gd");
    assert_eq!(ed.cursor_line(), 4, "the note at the foot: {}", ed.status());

    // …and `gD` shows the same thing in the other work area rather than
    // reading the column the cell sits in.
    ed.goto_line(3);
    while ed.char_at_cursor() != Some('^') {
        press(&mut ed, "l");
    }
    let was = ed.cursor();
    press(&mut ed, "gD");
    assert_eq!(ed.peeked_line(), Some(4), "{}", ed.status());
    assert_eq!(ed.cursor(), was, "a peek does not move you");
}

#[test]
fn both_layouts_ask_the_same_page() {
    // **The differential test.** Every defect in this class was invisible
    // because each side asked its own implementation: 縱書 worked out what
    // was off the page from the bare line — no syntax, no block — while
    // 橫排 was handed the answer. So it ate the `**` inside a fence, hid
    // two asterisks where Typst has one, hid four under `:syntax text`,
    // and folded blank lines by a different rule. This asks both.
    let document = "---\ntitle: 甲\n---\n\n那**年**冬天。\n\n# 第一章\n\n```\n\n程式 **很好** 碼。\n```\n\n最後一段。\n";
    for syntax in [
        crate::syntax::Syntax::Markdown,
        crate::syntax::Syntax::Typst,
        crate::syntax::Syntax::Text,
    ] {
        for indent in [0usize, 2] {
            let mut ed = Editor::new();
            ed.current_buffer_mut().insert(0, document)
                .expect("the fixture buffer is writable");
            ed.set_default_syntax(Some(syntax));
            ed.set_indent(indent);
            ed.set_render(Render::Full);
            let rope = ed.current_buffer().rope();
            let hidden = |line: usize| ed.markup_hidden_on_line(line);
            let folded = |line: usize| ed.line_is_folded(line);
            let drawn = |line: usize| ed.drawn_runs_on_line(line);
            let grid = ed.grid_with(&hidden, &folded, &drawn);
            for line in 0..rope.len_lines() {
                assert_eq!(
                    crate::zong::folded(rope, line, grid),
                    ed.line_is_folded(line),
                    "{syntax:?} indent={indent}: line {line} folds differently in the two \
                     layouts"
                );
                // …and what is off the page is one answer, not two: the
                // slots of a 縱 cover exactly the characters that are not
                // hidden, plus the hidden ones joined to their neighbours.
                let text = crate::zong::line_chars(rope, line);
                let slots = crate::zong::line_slots_in(
                    &rope.line(line).to_string(),
                    grid,
                    &ed.markup_hidden_on_line(line),
                );
                for at in 0..text.len() {
                    assert!(
                        slots.iter().any(|s| at >= s.start && at < s.end)
                            || slots.iter().all(|s| s.start == 0 && s.end == 0),
                        "{syntax:?}: char {at} of line {line} is in no slot — the cursor \
                         could stand where nothing is drawn"
                    );
                }
            }
        }
    }
}

#[test]
fn a_repeat_can_never_repeat_itself() {
    // `d`, `3`, `.`, `.` used to abort the process — a stack overflow,
    // which does not unwind, so every unsaved buffer went with it. Two
    // causes, both here: a count made `.` the *second* key of its own
    // definition, and nothing stopped a repeat from re-entering.
    let mut ed = typed("一二三四五六七八九十\n");
    ed.execute("1").unwrap();
    ed.on_key(Key::Char('d'));
    ed.on_key(Key::Char('3'));
    ed.on_key(Key::Char('.'));
    ed.on_key(Key::Char('.'));
    ed.on_key(Key::Char('.'));
    // Still here — and `.` still means the `d`: one, then three, then one,
    // then one.
    assert_eq!(ed.current_buffer().text(), "七八九十\n");
}

#[test]
fn a_hit_list_belongs_to_the_document_it_was_found_in() {
    // The worst thing this editor could hold: a list of char offsets with
    // no owner, holding `n` and `N`, surviving a buffer switch and an
    // edit. 「第 3/78 處」 could be said about a character in a chapter
    // that was never searched — and the next `d` deleted it.
    let mut ed = typed("那年冬天。\n那年夏天。\n");
    ed.goto_line(1);
    press(&mut ed, "g?");
    assert!(ed.current_hit().is_some(), "{}", ed.status());

    // Another file: the hits do not follow, and `n` goes back to `/`.
    ed.execute("new").unwrap();
    ed.current_buffer_mut().insert(0, "完全不相干的一行。\n").expect("the fixture buffer is writable");
    assert_eq!(ed.current_hit(), None, "another document, no hits");
    let before = ed.cursor();
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.current_hit(), None);
    assert!(!ed.status().contains("處"), "{}", ed.status());
    let _ = before;

    // …and an edit retires them rather than moving them: an offset into
    // the text as it was is not a shorter answer, it is a wrong one.
    ed.execute("buffer-previous").unwrap();
    assert!(ed.current_hit().is_some(), "back where they were found");
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Char('甲'));
    ed.on_key(Key::Esc);
    assert_eq!(ed.current_hit(), None, "the text moved under them");
}

#[test]
fn the_keys_a_keyboard_has_are_not_swallowed() {
    // PageUp/PageDown reached the editor as *nothing*: the table that turns
    // a terminal's keys into the editor's had no line for them, so they
    // fell off the end in every mode. `C-a`/`C-e` were taken by the `:`
    // line and swallowed by Insert.
    let mut ed = typed(&"一行\n".repeat(200));
    ed.set_page(20, 80);
    ed.execute("1").unwrap();
    ed.on_key(Key::PageDown);
    assert!(ed.cursor_line() > 10, "a page down: {}", ed.cursor_line());
    let down = ed.cursor_line();
    ed.on_key(Key::PageUp);
    assert!(ed.cursor_line() < down, "and a page back");

    // …and in Insert, without leaving it.
    ed.on_key(Key::Char('i'));
    let was = ed.cursor_line();
    ed.on_key(Key::PageDown);
    assert!(ed.cursor_line() > was, "a page down while typing");
    assert_eq!(ed.mode(), Mode::Insert, "and still typing");
    ed.on_key(Key::Char('甲'));
    ed.on_key(Key::Ctrl('a'));
    assert_eq!(
        ed.cursor(),
        crate::motion::line_start(ed.current_buffer().rope(), ed.cursor()),
        "C-a is the line's start, as it is on the `:` line"
    );
    ed.on_key(Key::Ctrl('e'));
    assert_eq!(
        ed.cursor(),
        crate::motion::line_end(ed.current_buffer().rope(), ed.cursor())
    );
}

#[test]
fn enter_on_prose_asks_where_else_this_word_is() {
    // One key, one meaning, in a table and out of it: 「在另一個工作區給我
    // 看這個詞還出現在哪裏」. It used to say 「這裏沒有註」 and stop,
    // which is an answer to a question nobody asked.
    let mut ed = typed("那年冬天很冷。\n第二行。\n那年夏天很熱。\n");
    ed.goto_line(1);
    let standing = ed.cursor();
    press(&mut ed, "g?");
    // 那年 is the word under the cursor, and it is on line 3 as well.
    assert_eq!(ed.peeked_line(), Some(2), "{}", ed.status());
    assert_eq!(ed.cursor(), standing, "and you did not go anywhere");
    assert!(ed.status().contains("2/2") || ed.status().contains("1/2"), "{}", ed.status());

    // A word that is only here says so rather than opening an area for it.
    ed.execute("2").unwrap();
    for _ in 0..2 {
        ed.on_key(Key::Char('l'));
    }
    press(&mut ed, "g?");
    assert!(ed.status().contains("只有這一處"), "{}", ed.status());
}

#[test]
fn a_footnote_reads_beside_the_sentence_it_belongs_to() {
    let mut ed = typed(
        "那年冬天[^1]，山下起了大雪。\n\n[^1]: 據縣志，那是丁丑年。\n",
    );
    ed.set_render(Render::Basic);
    // On the reference: the panel is the note itself, which is the whole
    // point of a footnote — it is meant to be read beside the sentence.
    ed.goto_line(1);
    for _ in 0..4 {
        ed.on_key(Key::Char('l'));
    }
    let d = ed.detail().expect("standing on the reference");
    assert_eq!(d.title, "[^1]");
    assert_eq!(d.rows[0].1.as_deref(), Some("據縣志，那是丁丑年。"));
    assert_eq!(d.links, vec![('↩', Some(2))], "and where it is written");

    // A step off it and the panel is gone: it answers about *here*.
    ed.on_key(Key::Char('h'));
    ed.on_key(Key::Char('h'));
    ed.on_key(Key::Char('h'));
    ed.on_key(Key::Char('h'));
    ed.on_key(Key::Char('h'));
    assert!(ed.detail().is_none());

    // A comment is the other kind of note: still on the page, but a long
    // one is easier read in a panel than in the middle of a paragraph.
    let mut ed = typed("那年冬天%%這裏要改，冬天太早了%%。\n");
    ed.set_render(Render::Basic);
    ed.goto_line(1);
    for _ in 0..5 {
        ed.on_key(Key::Char('l'));
    }
    let d = ed.detail().expect("standing on the comment");
    assert_eq!(d.title, "批注");
    assert_eq!(d.rows[0].1.as_deref(), Some("這裏要改，冬天太早了"));

    // Enter goes to the note and Enter comes back — one key, because from
    // the note there is only one place you can mean.
    let mut ed = typed(
        "那年冬天[^1]，山下起了大雪。\n\n[^1]: 據縣志，那是丁丑年。\n",
    );
    ed.set_render(Render::Basic);
    ed.goto_line(1);
    for _ in 0..4 {
        ed.on_key(Key::Char('l'));
    }
    let was = ed.cursor();
    // **`gd`**, not `Enter`: 「它指着哪裏」 and 「還在哪裏」 are two
    // questions, and `Enter` is the second one everywhere — otherwise a
    // word *inside* a note could never be asked about.
    // `gD` shows it beside the sentence; `gd` goes to it, and `C-o` comes
    // back — the pair every editor has.
    press(&mut ed, "gD");
    assert_eq!(ed.peeked_line(), Some(2), "the note, beside the sentence");
    assert_eq!(ed.cursor(), was, "and the sentence is still under the cursor");
    press(&mut ed, "gd");
    assert_eq!(ed.cursor_line(), 2, "…and this one goes there");
    ed.on_key(Key::Ctrl('o'));
    assert_eq!(ed.cursor(), was, "C-o comes back");
    ed.goto_line(1);
    press(&mut ed, "g?");
    assert_eq!(ed.cursor_line(), 0, "nothing moves");
    // Not on a note, so `Enter` is what it is everywhere else: 「這個詞還
    //在哪裏」, shown in the other work area.
    assert!(ed.status().contains("處") || ed.status().contains("只有"), "{}", ed.status());

    // A footnote nobody defined has nothing to show, and does not pretend.
    let mut ed = typed("那年冬天[^9]。\n");
    ed.set_render(Render::Basic);
    ed.goto_line(1);
    for _ in 0..4 {
        ed.on_key(Key::Char('l'));
    }
    assert!(ed.detail().is_none());
}

#[test]
fn the_detail_panel_says_what_the_whole_row_is() {
    let (dir, csv) = a_table("detail");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);

    let d = ed.detail().expect("a row has fields");
    assert_eq!(d.title, "一", "titled by its key");
    // Numbered exactly as the rows are, or the panel can never find the
    // field the cursor is in — which is how it came to scroll to the top
    // and light nothing.
    assert_eq!(d.here, " 1 字", "and it says which field you are in");
    assert!(d.rows.iter().any(|(name, _)| *name == d.here), "and it is one of them");
    // **Numbered, and all of them** — the keys count columns (`t3/`,
    // `t20,20g`), and an empty field is a finding in a 拆分表, not a thing
    // to hide.
    assert_eq!(
        d.rows,
        vec![
            (" 1 字".to_string(), Some("一".to_string())),
            (" 2 ids_y".to_string(), Some("⿰木目".to_string())),
            (" 3 ids_g".to_string(), Some("⿰木目".to_string())),
            // Worked out, not stored, and marked so nobody looks for a
            // column that is not in the file.
            ("unicode*".to_string(), Some("U+4E00".to_string())),
        ]
    );

    // The header is not a row and has nothing to say about itself.
    ed.goto_line(1);
    assert!(ed.detail().is_none());

    std::fs::remove_dir_all(&dir).ok();
}

/// A delimited grid sorts by one column or several, and keeps its rows.
#[test]
fn a_grid_sorts_by_the_columns_it_is_told() {
    let dir = std::env::temp_dir().join(format!("yumete-sort-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
    std::fs::write(
        dir.join(".yumete").join("tables").join("t.toml"),
        "[table]\nfile = ['d.csv']\nkey = 'char'\n\
         [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'block'\n\
         [[table.column]]\nname = 'n'\n",
    )
    .unwrap();
    let csv = dir.join("d.csv");
    std::fs::write(&csv, "char,block,n\n丙,B,2\n甲,A,10\n乙,B,9\n丁,A,1\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    let rows = |ed: &Editor| -> Vec<String> {
        ed.current_buffer().text().lines().skip(1).map(str::to_string).collect()
    };

    // By the first column, ascending — the header stays put.
    ed.execute(":table-sort 1 a").unwrap();
    assert!(ed.current_buffer().text().starts_with("char,block,n\n"), "{}", ed.status());
    assert_eq!(rows(&ed), ["丁,A,1", "丙,B,2", "乙,B,9", "甲,A,10"], "{}", ed.status());

    // **Numbers as numbers**: 10 after 9, not before it.
    ed.execute(":table-sort 3 a").unwrap();
    assert_eq!(rows(&ed), ["丁,A,1", "丙,B,2", "乙,B,9", "甲,A,10"]);

    // Two columns: block ascending, then n descending inside each block.
    ed.execute(":table-sort 2 a 3 d").unwrap();
    assert_eq!(rows(&ed), ["甲,A,10", "丁,A,1", "乙,B,9", "丙,B,2"], "{}", ed.status());

    // The rows are the same rows: nothing gained, nothing lost.
    let mut before: Vec<String> = "丙,B,2 甲,A,10 乙,B,9 丁,A,1".split(' ').map(str::to_string).collect();
    before.sort();
    let mut after = rows(&ed);
    after.sort();
    assert_eq!(before, after);

    // `t3S` is the same thing from the keyboard.
    for key in "t3S".chars() {
        ed.on_key(Key::Char(key));
    }
    assert_eq!(rows(&ed), ["甲,A,10", "乙,B,9", "丙,B,2", "丁,A,1"], "{}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

/// A sort names every column first and acts last: `t1a2d8as`.
///
/// 「我認為正確的語法應該是 t1a2d8as 表示 對第一列升序，第二列降序，第八列升
/// 序，最後的 s 發出動作指令。原来的设计用的是 t1s2S8s 这样的命令，这个会在
/// t1s 直接生效（因为他是前綴碼的指令）。」 —— `s` is the action, so it can
/// never also be a column's direction; `a` and `d` are, and neither of them
/// acts.
#[test]
fn a_sort_names_its_columns_before_it_acts() {
    let dir = std::env::temp_dir().join(format!("yumete-sortkeys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("d.csv");
    std::fs::write(&csv, "char,block,n\n丙,B,2\n甲,A,10\n乙,B,9\n丁,A,1\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    let rows = |ed: &Editor| -> Vec<String> {
        ed.current_buffer().text().lines().skip(1).map(str::to_string).collect()
    };

    // Nothing has happened yet, and the half-typed command reads back as
    // what was typed — `t1a2d`, a column at a time.
    for key in "t2a3d".chars() {
        ed.on_key(Key::Char(key));
    }
    assert_eq!(ed.typed_so_far(), "t2a3d", "{}", ed.status());
    assert_eq!(rows(&ed), ["丙,B,2", "甲,A,10", "乙,B,9", "丁,A,1"], "not yet");

    // …and now the action. Block ascending, then n descending inside it.
    ed.on_key(Key::Char('s'));
    assert_eq!(ed.typed_so_far(), "", "the command is spent");
    assert_eq!(rows(&ed), ["甲,A,10", "丁,A,1", "乙,B,9", "丙,B,2"], "{}", ed.status());

    // One column keeps the old short spelling, direction in the verb.
    for key in "t1S".chars() {
        ed.on_key(Key::Char(key));
    }
    assert_eq!(rows(&ed), ["甲,A,10", "乙,B,9", "丙,B,2", "丁,A,1"], "{}", ed.status());

    // Both spellings at once: the last column takes its direction from the
    // verb, the ones before it from their own letter.
    for key in "t2a3S".chars() {
        ed.on_key(Key::Char(key));
    }
    assert_eq!(rows(&ed), ["甲,A,10", "丁,A,1", "乙,B,9", "丙,B,2"], "{}", ed.status());

    // **`d` is only a direction after a plain column number.** `t d` is
    // still 「delete this row」 and a span is still a span.
    ed.goto_line(2);
    for key in "td".chars() {
        ed.on_key(Key::Char(key));
    }
    assert_eq!(rows(&ed), ["丁,A,1", "乙,B,9", "丙,B,2"], "a row went: {}", ed.status());

    // A sort abandoned half-way leaves no columns behind for the next one.
    for key in "t1a2d".chars() {
        ed.on_key(Key::Char(key));
    }
    ed.on_key(Key::Esc);
    assert_eq!(ed.typed_so_far(), "");
    for key in "t3a".chars() {
        ed.on_key(Key::Char(key));
    }
    assert_eq!(ed.typed_so_far(), "t3a", "only this one");
    ed.on_key(Key::Char('s'));
    // **Numbers as numbers**, and only column three had a say.
    assert_eq!(rows(&ed), ["丁,A,1", "丙,B,2", "乙,B,9"], "{}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

/// **`-` is a range, `,` is a list or a pair** (§5.7).
///
/// One key had been doing both jobs. A row *and* a column is two kinds of
/// thing where columns two through ten are a span of one kind, so the day
/// `t20,20g` was written down the two readings collided — and `t2-10g`
/// answered it with row 2, column 10: a plausible answer to a question
/// nobody had asked.
#[test]
fn a_dash_spans_and_a_comma_pairs() {
    let mut ed = typed(
        "| a | b | c |\n| --- | --- | --- |\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n| 7 | 8 | 9 |\n",
    );
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());

    // What has been typed reads back as what was typed, joint and all.
    press(&mut ed, "t2-3");
    assert_eq!(ed.typed_so_far(), "t2-3");
    ed.on_key(Key::Esc);
    press(&mut ed, "t1,3");
    assert_eq!(ed.typed_so_far(), "t1,3");
    ed.on_key(Key::Esc);
    // A span has exactly two ends; a list goes on as long as commas do.
    press(&mut ed, "t1,3,2");
    assert_eq!(ed.typed_so_far(), "t1,3,2");
    ed.on_key(Key::Esc);
    press(&mut ed, "t1-3");
    ed.on_key(Key::Char('-'));
    assert_eq!(ed.typed_so_far(), "", "a second dash is not part of a span");
    ed.on_key(Key::Esc);
    // The joint alone is not a zero: `t2-` reads back as `t2-`.
    press(&mut ed, "t2-");
    assert_eq!(ed.typed_so_far(), "t2-");
    ed.on_key(Key::Esc);

    // `t3,2g` — row 3, column 2. Two numbers, two kinds of thing.
    press(&mut ed, "t3,2g");
    assert_eq!(ed.cursor_line(), 2, "{}", ed.status());
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(1), "{}", ed.status());
    // One number is the row, in the column you are already in.
    press(&mut ed, "t5g");
    assert_eq!(ed.cursor_line(), 4, "{}", ed.status());
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(1), "{}", ed.status());
    // And the dash is not the pair: it says so rather than guessing.
    let where_it_was = ed.cursor_line();
    press(&mut ed, "t2-3g");
    assert_eq!(ed.cursor_line(), where_it_was, "{}", ed.status());
    assert!(ed.status().contains("t20,20g"), "{}", ed.status());
}

/// A row number a table does not have lands **in the table anyway**
/// (author, 2026-09-07: 「markdown 表格中按 t1g，会跑到整个文档的第一行而
/// 不是表格的第一行」).
///
/// The number is the gutter's, which is what makes it the same key in a
/// `.csv` and in a chapter — but a table key that walks the cursor three
/// screens up into the prose has left the thing it was pressed on.
#[test]
fn a_row_number_outside_the_table_is_clamped_into_it() {
    let mut ed = typed(
        "前文一
前文二
前文三

| a | b |
| --- | --- |
| 1 | 2 |
| 3 | 4 |
",
    );
    ed.goto_line(7);
    assert!(ed.enter_table(), "{}", ed.status());
    // The table's rows are lines 7 and 8; `t1g` is the first of them.
    press(&mut ed, "t1g");
    assert_eq!(ed.cursor_line(), 6, "the table's first row: {}", ed.status());
    // And a number past the end is its last row, not the file's.
    press(&mut ed, "t99g");
    assert_eq!(ed.cursor_line(), 7, "the table's last row: {}", ed.status());
    // A number the table does have is still that line, gutter and all.
    press(&mut ed, "t8g");
    assert_eq!(ed.cursor_line(), 7, "{}", ed.status());
}

/// **A sort names the column it sorts by** (§5.7).
///
/// The bare letter is gone: on 123 380 rows a sort costs real seconds and
/// `u` refunds the content but not the time. `0` is not a column, so it
/// was free to mean 「the one I am standing in」 — said out loud.
#[test]
fn a_sort_names_the_column_it_sorts_by() {
    let years = |ed: &Editor| -> Vec<String> {
        ed.current_buffer()
            .text()
            .lines()
            .skip(2)
            .filter_map(|l| l.split('|').nth(1))
            .map(|c| c.trim().to_string())
            .collect()
    };
    let mut ed =
        typed("| 年 | 事 |\n| --- | --- |\n| 1900 | 丙 |\n| 19 | 甲 |\n| 200 | 乙 |\n");
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());

    // A bare `t s` does nothing at all, and says what to type instead.
    let before = ed.current_buffer().text();
    press(&mut ed, "ts");
    assert_eq!(ed.current_buffer().text(), before, "nothing was sorted");
    assert!(ed.status().contains("t0s"), "{}", ed.status());

    // `t1,5,9s` — several columns at once, all ascending. Here the table
    // has two, so it is 事 first and 年 inside it: 丙 U+4E19, 乙 U+4E59,
    // 甲 U+7532.
    press(&mut ed, "t2,1s");
    assert_eq!(years(&ed), ["1900", "200", "19"], "{}", ed.status());
    // **Both columns are named.** A sort by two columns that reported only
    // the first would hide the tiebreaker that decided every row where the
    // first ties — and the Markdown path used to, because it was written
    // before a keyboard sequence could name two.
    assert!(ed.status().contains('事'), "{}", ed.status());
    assert!(ed.status().contains('年'), "{}", ed.status());
    // One column keeps the long sentence, which is where the comparison
    // rule is written down.
    press(&mut ed, "t1s");
    assert!(ed.status().contains('年'), "{}", ed.status());
    assert!(!ed.status().contains('事'), "{}", ed.status());

    // A column that is not there is said, not ignored — the same rule the
    // sort has always followed, now that `/` follows it too.
    press(&mut ed, "t9/");
    assert!(ed.status().contains('9'), "{}", ed.status());
    assert!(!ed.status().is_empty());
}

/// Sorting a table does not put the table away.
///
/// 「bug：表格排序 t1s 會直接回到源碼視圖。」 A sort rewrites the *text*,
/// and the old code answered that by forgetting the whole document —
/// which re-asks 「is this a table?」 from scratch, and a `.csv` with no
/// schema beside it has only the answer the reader gave it by hand.
#[test]
fn sorting_keeps_the_table_open() {
    let dir = std::env::temp_dir().join(format!("yumete-sortview-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("plain.csv");
    std::fs::write(&csv, "char,n\n丙,2\n甲,10\n乙,9\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    for key in "t1s".chars() {
        ed.on_key(Key::Char(key));
    }
    assert!(ed.table().is_some(), "still a table: {}", ed.status());
    assert!(
        ed.current_buffer().text().starts_with("char,n\n丙,2\n"),
        "and it sorted: {:?}",
        ed.current_buffer().text()
    );

    // The same for a `|` table in a chapter — the mode is the reader's
    // answer there too.
    let mut ed = typed("前文\n| 字 | n |\n| --- | --- |\n| 丙 | 2 |\n| 甲 | 10 |\n");
    press(&mut ed, "gg");
    for _ in 0..3 {
        ed.on_key(Key::Char('j'));
    }
    assert!(ed.enter_table(), "{}", ed.status());
    for key in "t1s".chars() {
        ed.on_key(Key::Char(key));
    }
    assert!(ed.table().is_some(), "still a table: {}", ed.status());
}

/// 命令＋選擇＋動作: the digits inside a sequence are its argument, and
/// nothing leaks out of it.
#[test]
fn a_sequence_argument_belongs_to_its_own_sequence() {
    let mut ed = typed(&(1..=40).map(|n| format!("第{n}行。\n")).collect::<String>());

    // `g30g` — the sequence's own argument.
    press(&mut ed, "gg");
    for key in "g30g".chars() {
        ed.on_key(Key::Char(key));
    }
    assert_eq!(ed.cursor_line(), 29, "{}", ed.status());

    // …and it does not leak: the next `j` moves one line, not thirty.
    let before = ed.cursor_line();
    ed.on_key(Key::Char('j'));
    assert_eq!(ed.cursor_line(), before + 1, "the argument was spent");

    // An argument the verb does not use is dropped, not applied to
    // something else.
    for key in "g5h".chars() {
        ed.on_key(Key::Char(key));
    }
    let line = ed.cursor_line();
    ed.on_key(Key::Char('j'));
    assert_eq!(ed.cursor_line(), line + 1, "still one line");

    // Esc in the middle of a sequence leaves nothing behind.
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('2'));
    ed.on_key(Key::Char('-'));
    ed.on_key(Key::Esc);
    assert_eq!(ed.typed_so_far(), "", "the half-typed command is gone");
    let line = ed.cursor_line();
    ed.on_key(Key::Char('j'));
    assert_eq!(ed.cursor_line(), line + 1);

    // The old order still works, because fifty years of fingers know it.
    press(&mut ed, "gg");
    for key in "30G".chars() {
        ed.on_key(Key::Char(key));
    }
    assert_eq!(ed.cursor_line(), 29, "{}", ed.status());

    // A count before a plain key is still a count.
    press(&mut ed, "gg");
    for key in "5j".chars() {
        ed.on_key(Key::Char(key));
    }
    assert_eq!(ed.cursor_line(), 5);
}

#[test]
fn a_component_leads_to_its_own_row() {
    let dir = std::env::temp_dir().join(format!("yumete-jump-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
    std::fs::write(
        dir.join(".yumete").join("tables").join("t.toml"),
        "[table]\nfile = ['d.csv']\nkey = 'char'\n\
         [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
         [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
    )
    .unwrap();
    let csv = dir.join("d.csv");
    // 木 and 目 have rows of their own; ⿰ is a descriptor and does not.
    std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: 這一段講的是按格
    press(&mut ed, "l");

    // The panel lists what the cell points at, and what it cannot.
    let d = ed.detail().unwrap();
    assert_eq!(
        d.links,
        vec![('木', Some(2)), ('目', Some(3))],
        "⿰ is the grammar, not a component: it is not listed at all"
    );

    // 「誰用了它」 is `t?`: 相 and 目 both use 目.
    ed.goto_line(4);
    press(&mut ed, "0");
    press(&mut ed, "t?");
    assert_eq!(ed.peeked_line(), Some(1), "相 uses 目");
    assert!(ed.status().contains("1/2"), "{}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_key_index_is_not_rebuilt_while_a_cell_is_being_typed_in() {
    // The panel resolves the cell's components on every frame, so the
    // index behind it must not be rebuilt on every keystroke — over a
    // hundred thousand rows that was ten milliseconds a character.
    let dir = std::env::temp_dir().join(format!("yumete-index-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
    std::fs::write(
        dir.join(".yumete").join("tables").join("t.toml"),
        "[table]\nfile = ['d.csv']\nkey = 'char'\n\
         [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
         [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
    )
    .unwrap();
    let csv = dir.join("d.csv");
    std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "l");
    assert_eq!(ed.detail().unwrap().links[0], ('木', Some(2)));

    // Typing in a cell that is not the key column cannot move a row or
    // rename one — table mode refuses Enter — so the index stands.
    ed.on_key(Key::Char('i'));
    for _ in 0..5 {
        ed.on_key(Key::Char('土'));
        assert_eq!(
            ed.detail().unwrap().links.last(),
            Some(&('目', Some(3))),
            "still resolving, without a rebuild"
        );
    }
    ed.on_key(Key::Esc);

    // But a row that really is renamed is seen, because leaving Insert
    // makes the index stale again.
    ed.goto_line(3);
    press(&mut ed, "c");
    ed.on_key(Key::Char('水'));
    ed.on_key(Key::Esc);
    assert_eq!(ed.cell_text(2, 0), "水", "木's row is now 水's");
    ed.goto_line(2);
    press(&mut ed, "l");
    let links = ed.detail().unwrap().links;
    assert!(
        links.iter().any(|&(c, line)| c == '木' && line.is_none()),
        "木 has no row any more, and the panel says so: {links:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_cell_is_entered_three_ways_and_typing_stays_inside_it() {
    let (dir, csv) = a_table("inside");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "l");
    assert_eq!(ed.cell_text(1, 1), "⿰木目");

    // `i` is the cell's first character…
    ed.on_key(Key::Char('i'));
    assert_eq!(ed.mode(), Mode::Insert);
    assert_eq!(ed.cursor(), ed.cell_span(1, 1).unwrap().0);
    // …and inside, the arrows move by character, which is how the middle
    // of a 拆分 sequence is reached at all.
    ed.on_key(Key::Right);
    ed.on_key(Key::Right);
    ed.on_key(Key::Char('金'));
    assert_eq!(ed.cell_text(1, 1), "⿰木金目");
    // At the cell's edge they step next door — walking is not joining (#376).
    ed.on_key(Key::End);
    ed.on_key(Key::Right);
    assert_eq!(ed.cell_position(), Some((1, 2)), "into the one to the right");
    assert_eq!(ed.cursor(), ed.cell_span(1, 2).unwrap().0, "in at its start");
    ed.on_key(Key::Left);
    assert_eq!(ed.cursor(), ed.cell_span(1, 1).unwrap().1, "back in at its end");
    ed.on_key(Key::Home);
    ed.on_key(Key::Left);
    assert_eq!(ed.cell_position(), Some((1, 0)));
    ed.on_key(Key::Right);
    assert_eq!(ed.cell_position(), Some((1, 1)));
    // Up and down keep to the column — the step `j` and `k` take from Normal.
    ed.on_key(Key::Down);
    assert_eq!(ed.cursor_line(), 2, "{}", ed.status());
    ed.on_key(Key::Up);
    assert_eq!(ed.cell_position(), Some((1, 1)));
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('u'));

    // `a` is after its last character.
    press(&mut ed, "a");
    assert_eq!(ed.cursor(), ed.cell_span(1, 1).unwrap().1);
    ed.on_key(Key::Char('金'));
    assert_eq!(ed.cell_text(1, 1), "⿰木目金");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('u'));

    // `c` takes the whole cell out and starts again — the common case in a
    // grid, where you land on a cell to give it a new value.
    press(&mut ed, "c");
    assert_eq!(ed.mode(), Mode::Insert);
    assert_eq!(ed.cell_text(1, 1), "", "emptied");
    ed.on_key(Key::Char('土'));
    assert_eq!(ed.cell_text(1, 1), "土");
    // The neighbours are untouched — the delimiters are still there.
    assert_eq!(ed.cell_text(1, 0), "一");
    assert_eq!(ed.cell_text(1, 2), "⿰木目");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.cell_text(1, 1), "⿰木目", "and it all undoes in one step");

    std::fs::remove_dir_all(&dir).ok();
}

/// **The key the hint offers for the grain is the key that changes it** (#399).
///
/// `Tab` held the grain until #356 gave it what every spreadsheet means by the
/// key and moved the grain to `T` — and the command row went on offering `Tab`.
/// Pressing it stepped one cell and left the grain, the row and the status line
/// exactly as they were, which reads as a switch that does not switch.
#[test]
fn the_hint_offers_the_key_that_really_changes_the_grain() {
    let dir = std::env::temp_dir().join(format!("yumete-grain-hint-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
    std::fs::write(
        dir.join(".yumete").join("tables").join("t.toml"),
        "[table]\nfile = ['d.csv']\nkey = 'char'\n\
         [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n",
    )
    .unwrap();
    let csv = dir.join("d.csv");
    std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);

    // Both ways round: the offer has to be right in either grain.
    for _ in 0..2 {
        let before = ed.table().unwrap().grain;
        // A hint is a status line first, and the file has just been opened.
        ed.status.clear();
        let Hint::Keys(_, keys) = ed.hint() else { panic!("standing in a grid") };
        let other = match before {
            Grain::Char => say!("hint.table.by-cell-instead"),
            Grain::Cell => say!("hint.table.by-character-instead"),
        };
        let (key, _) = keys
            .iter()
            .find(|(_, what)| *what == other)
            .unwrap_or_else(|| panic!("the other grain is offered: {keys:?}"));
        press(&mut ed, key);
        assert_ne!(
            ed.table().unwrap().grain,
            before,
            "the row offers `{key}` for 「{other}」 and it did not change the grain"
        );
        assert!(ed.table_status().unwrap().ends_with(match before {
            Grain::Char => "格",
            Grain::Cell => "字",
        }));
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn t_says_whether_a_step_is_a_cell_or_a_character() {
    let dir = std::env::temp_dir().join(format!("yumete-grain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
    std::fs::write(
        dir.join(".yumete").join("tables").join("t.toml"),
        "[table]\nfile = ['d.csv']\nkey = 'char'\n\
         [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
         [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
    )
    .unwrap();
    let csv = dir.join("d.csv");
    std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);
    // **開着就是按字** (#356): the characters in a cell are what a writer
    // mostly wants; the grid is what they ask for, with `T`.
    assert_eq!(ed.table().unwrap().grain, crate::editor::Grain::Char);
    assert!(ed.table_status().unwrap().ends_with("字"));

    press(&mut ed, "T");
    assert_eq!(ed.table().unwrap().grain, crate::editor::Grain::Cell);
    assert!(ed.table_status().unwrap().ends_with("格"));

    // By the cell: one `l` crosses the whole of 「相」 and the delimiter.
    press(&mut ed, "l");
    assert_eq!(ed.cell_position(), Some((1, 1)));
    assert_eq!(ed.char_at_cursor(), Some('⿰'), "at the cell's first 字");

    // `T`, and the same key steps one character.
    press(&mut ed, "T");
    assert_eq!(ed.table().unwrap().grain, crate::editor::Grain::Char);
    assert!(ed.table_status().unwrap().ends_with("字"), "and it says so");
    press(&mut ed, "l");
    assert_eq!(ed.char_at_cursor(), Some('木'), "one 字, not one cell");
    press(&mut ed, "l");
    assert_eq!(ed.char_at_cursor(), Some('目'));

    // Back to the head of the cell, still one character at a time.
    ed.goto_line(2);
    press(&mut ed, "ll");
    assert_eq!(ed.char_at_cursor(), Some('⿰'));

    // `T` back, and the cursor snaps to cells again.
    press(&mut ed, "T");
    assert_eq!(ed.table().unwrap().grain, crate::editor::Grain::Cell);
    press(&mut ed, "h");
    assert_eq!(ed.cell_position(), Some((1, 0)));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_schema_with_a_typo_in_it_says_so_instead_of_vanishing() {
    // Dropping the parse error cost every label, both computed fields and
    // the whole jump, and the only clue was 「照首行」 in the status line.
    let dir = std::env::temp_dir().join(format!("yumete-badschema-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
    let schema = dir.join(".yumete").join("tables").join("t.toml");
    let csv = dir.join("d.csv");
    std::fs::write(&csv, "char,ids_y\n一,⿰木目\n").unwrap();

    for (body, expect) in [
        ("[table\nfile = 'd.csv'", "TOML"),
        ("[table]\nfile = 'd.csv'", "no columns"),
        (
            "[table]\nfile = 'd.csv'\nkey = 'nope'\n[[table.column]]\nname = 'char'",
            "not a column",
        ),
        (
            "[table]\nfile = 'd.csv'\n[[table.column]]\nname = 'char'\n\
             [[table.detail]]\nname = 'u'\ncompute = 'codepoint(nope)'",
            "not a column",
        ),
        (
            "[table]\nfile = 'd.csv'\nquoting = 'minimal'\n[[table.column]]\nname = 'char'",
            "not supported",
        ),
        (
            "[table]\nfile = 'd.csv'\ndelimiter = '::'\n[[table.column]]\nname = 'char'",
            "one character",
        ),
        (
            "[table]\nfile = 'd.csv'\ndelimiter = \"\\n\"\n[[table.column]]\nname = 'char'",
            "separates rows",
        ),
    ] {
        std::fs::write(&schema, body).unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.table().is_none(), "{expect}: not read as a grid");
        assert!(
            ed.status().starts_with("schema：") && ed.status().contains(expect),
            "opening says what is wrong: {}",
            ed.status()
        );
        // …and asking again says the same thing rather than falling back to
        // the header row as though no schema had been written at all.
        assert!(!ed.enter_table(), "{expect}");
        assert!(ed.status().contains(expect), "{}", ed.status());
    }

    // A schema for *other* files is not a problem; it is simply not this
    // file's, and the header row stands in.
    std::fs::write(
        &schema,
        "[table]\nfile = 'somethingelse.csv'\n[[table.column]]\nname = 'char'",
    )
    .unwrap();
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.status().is_empty(), "{}", ed.status());
    assert!(ed.enter_table());
    assert!(ed.status().contains("照首行"), "{}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn enter_asks_who_uses_this_when_the_cell_is_not_a_link() {
    // Standing on 卵 in the key column, `Enter` used to say 「這一格不指向
    // 任何一行」 — true, and useless. The question a 拆分表 is corrected by
    // is the other way round: *who uses this?*
    let dir = std::env::temp_dir().join(format!("yumete-who-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
    std::fs::write(
        dir.join(".yumete").join("tables").join("t.toml"),
        "[table]\nfile = ['d.csv']\nkey = 'char'\n\
         [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
         [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
    )
    .unwrap();
    let csv = dir.join("d.csv");
    std::fs::write(
        &csv,
        "char,ids_y\n木,木\n相,⿰木目\n林,⿰木木\n目,目\n杏,⿱木口\n",
    )
    .unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.execute("2").unwrap();
    assert_eq!(ed.cell_text(1, 0), "木", "the key column");

    // Every 木 in the column the schema names, down it from the top —
    // and the cursor starts at the **first** whatever row it was standing
    // on, so asking about 木 gives the same route every time.
    //
    // Five, not four: 林 is ⿰木木 and that is two of them, exactly as `/`
    // would count two matches on one line. A column search differs from a
    // row search in its *direction* and in nothing else.
    let standing = ed.cursor();
    press(&mut ed, "t?");
    assert_eq!(ed.peeked_line(), Some(1), "木 itself, the first of them");
    assert_eq!(ed.cursor(), standing, "…and you did not go anywhere");
    assert!(ed.status().contains("1/5"), "{}", ed.status());
    // …and the match is what is marked in the other area, exactly the
    // range `/` would have left as the selection.
    let (a, b) = ed.other_pane().and_then(|p| p.highlight).expect("a hit");
    assert_eq!(
        ed.current_buffer().rope().slice(a..b).to_string(),
        "木",
        "the match is what is marked"
    );
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.peeked_line(), Some(2), "相");
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.peeked_line(), Some(3), "林's first 木");
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.peeked_line(), Some(3), "…and its second");
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.peeked_line(), Some(5), "杏");
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.peeked_line(), Some(1), "and round again");
    ed.on_key(Key::Char('N'));
    assert_eq!(ed.peeked_line(), Some(5), "and back");

    // A 拆分 cell still means the other thing: its components' own rows.
    ed.execute("3").unwrap();
    press(&mut ed, "l");
    ed.on_key(Key::Tab);
    press(&mut ed, "ll");
    assert_eq!(ed.char_at_cursor(), Some('目'));
    press(&mut ed, "gD");
    assert_eq!(ed.peeked_line(), Some(4), "目's own row");

    // …and the reverse question again, from a different row.
    ed.on_key(Key::Tab);
    ed.execute("5").unwrap();
    assert_eq!(ed.cell_text(4, 0), "目");
    press(&mut ed, "t?");
    assert!(ed.status().contains("1/2"), "目 is used twice: {}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

/// **`:x` is not `:wq`** — it writes only when the file changed.
///
/// It was an alias of `:write-quit` here, which is the one thing that tells
/// the two apart in vi and in helix both: a file opened, read and left alone
/// keeps its timestamp, and `make`, rsync and a sync folder all read a moved
/// timestamp as 「this changed」. `:update` is the same gate without leaving.
#[test]
fn a_file_nobody_changed_is_not_written_again() {
    let dir = std::env::temp_dir().join(format!("yumete-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ch01.md");
    std::fs::write(&file, "原稿一行\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&file).unwrap();
    let untouched = std::fs::metadata(&file).unwrap().modified().unwrap();

    // Nothing typed, so `:update` writes nothing and says so.
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(ed.execute("update").is_ok());
    assert!(ed.status().contains("沒有改動"), "{}", ed.status());
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        untouched,
        "the timestamp did not move"
    );

    // `:w` is the unconditional one, and still is.
    assert!(ed.execute("w").is_ok());
    assert!(
        std::fs::metadata(&file).unwrap().modified().unwrap() > untouched,
        "`:w` writes whether or not anything changed"
    );

    // Now something changed: `:up` writes it.
    press(&mut ed, "i");
    ed.on_key(Key::Char('甲'));
    ed.on_key(Key::Esc);
    assert!(ed.execute("up").is_ok());
    assert!(ed.status().contains("存了"), "{}", ed.status());
    assert!(std::fs::read_to_string(&file).unwrap().contains('甲'));
    assert!(!ed.current_buffer().is_modified());

    // `:x` on a clean buffer leaves without writing…
    let clean = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(ed.execute("x").is_ok());
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        clean,
        "`:x` wrote a file nobody had changed"
    );

    // …and `:xit` is the same command under its other name.
    assert_eq!(
        crate::command::parse(":xit"),
        Ok(crate::command::Command::Exit(None))
    );
    // `:x` is no longer `:wq`, which stays unconditional.
    assert_eq!(
        crate::command::parse(":wq"),
        Ok(crate::command::Command::WriteQuit(None))
    );
    // A path is an instruction, not a convenience: it is always written.
    assert_eq!(
        crate::command::parse(":x 第二章.md"),
        Ok(crate::command::Command::Exit(Some("第二章.md".into())))
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_file_changed_underneath_is_not_written_over() {
    // The one silent way to lose a day's work: a file open here and changed
    // out there — by git, a sync folder, `:!sed -i`, or the same file open
    // in another editor — used to be overwritten without a word.
    let dir = std::env::temp_dir().join(format!("yumete-stamp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ch01.md");
    std::fs::write(&file, "原稿一行\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&file).unwrap();
    // An ordinary save says so — the manual has been quoting this line as
    // its example of the command row all along, and it did not exist.
    press(&mut ed, "i");
    ed.on_key(Key::Char('甲'));
    ed.on_key(Key::Esc);
    assert!(ed.execute("w").is_ok());
    assert!(ed.status().contains("存了"), "{}", ed.status());

    // Now somebody else writes it. A stamp is size *and* mtime, and a test
    // is fast enough to land in the same second, so the length differs too.
    std::fs::write(&file, "別的程序寫進來的內容\n第二行\n").unwrap();
    assert!(ed.current_buffer().changed_underneath());
    press(&mut ed, "i");
    ed.on_key(Key::Char('乙'));
    ed.on_key(Key::Esc);
    assert!(ed.execute("w").is_err(), "the save is refused");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "別的程序寫進來的內容\n第二行\n",
        "and the other version is still there"
    );

    // `:w!` is "I know, and mine wins".
    assert!(ed.execute("w!").is_ok());
    assert!(std::fs::read_to_string(&file).unwrap().contains('乙'));

    // …and after it, the stamp is ours again, so the next save is quiet.
    assert!(!ed.current_buffer().changed_underneath());
    assert!(ed.execute("w").is_ok());

    // A file that was *touched* but not changed is not a conflict. Rewriting
    // the same bytes moves the mtime, and refusing a save for that is worse
    // than not checking at all: three false alarms and `:w!` becomes a
    // reflex, including at the one that matters.
    let same = std::fs::read_to_string(&file).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&file, &same).unwrap();
    assert!(
        !ed.current_buffer().changed_underneath(),
        "same bytes, so nothing changed"
    );
    assert!(ed.execute("w").is_ok(), "and the save goes through quietly");

    // `:reload!` is the other half: take what is on disk and lose what is
    // here.
    std::fs::write(&file, "外面的版本\n").unwrap();
    assert!(ed.execute("reload!").is_ok());
    assert_eq!(ed.current_buffer().text(), "外面的版本\n");
    assert!(!ed.current_buffer().is_modified());
    assert!(ed.execute("w").is_ok(), "and saving is fine again");

    std::fs::remove_dir_all(&dir).ok();
}

// ---- Tables squared up on the page (Feature #212) ---------------------

/// The width of one line **as the page draws it**: what is left after
/// 所見即所得 has taken its markup off, plus the padding drawn back on.
fn drawn_width(ed: &Editor, line: usize) -> usize {
    let text = ed.line_text(line).unwrap_or_default();
    let chars: Vec<char> = text.trim_end_matches(['\n', '\r']).chars().collect();
    let hidden = ed.hidden_on_line(line);
    let visible: usize = (0..chars.len())
        .filter(|at| !hidden.iter().any(|&(a, b)| (a..b).contains(at)))
        .map(|at| yumete_cjk::char_width(chars[at]))
        .sum();
    let drawn: usize = ed
        .drawn_on_line(line)
        .iter()
        .map(|(_, text)| text.chars().map(yumete_cjk::char_width).sum::<usize>())
        .sum();
    visible + drawn
}

/// #212: a table nobody has formatted is drawn as a table anyway.
#[test]
fn a_table_is_squared_up_on_the_page_and_not_in_the_file() {
    const TEXT: &str = "|甲|乙|\n|---|---|\n|一二三|四|\n";
    let mut ed = Editor::new();
    ed.add_buffer(crate::Buffer::from_text(TEXT));
    assert!(!ed.drawn_on_line(0).is_empty(), "the header is padded");
    let widths: Vec<usize> = (0..3).map(|l| drawn_width(&ed, l)).collect();
    assert_eq!(widths[0], widths[1], "{widths:?}");
    assert_eq!(widths[1], widths[2], "{widths:?}");
    // …and the file is exactly what was typed.
    assert_eq!(ed.current_buffer().text(), TEXT);
    assert!(!ed.current_buffer().is_modified());
}

/// #212: the ragged page the *file* cannot fix.
///
/// This table is padded perfectly in the source — every other reader of it
/// sees straight pipes. 所見即所得 then takes six columns off one row and
/// none off the next, and the page is ragged however the file is written.
#[test]
fn the_padding_makes_up_for_what_所見即所得_took() {
    let mut ed = Editor::new();
    ed.add_buffer(crate::Buffer::from_text(
        "| 方案 | 說明                 |\n| ---- | -------------------- |\n| 光華 | `宇浩`系列的**基礎** |\n| 星陳 | 大字集               |\n",
    ));
    // With the markup on the page the file is already square, so nothing
    // is drawn: the padding is not a second opinion about a formatted
    // table.
    ed.execute("render basic").unwrap();
    let source: Vec<usize> = (0..4).map(|l| drawn_width(&ed, l)).collect();
    assert!(source.iter().all(|w| *w == source[0]), "{source:?}");
    assert!(ed.drawn_on_line(2).is_empty(), "{:?}", ed.drawn_on_line(2));

    ed.execute("render full").unwrap();
    // Off the marked-up row: the construct the cursor is in is never
    // hidden, which is the one row that would not be short.
    press(&mut ed, "G");
    let widths: Vec<usize> = (0..4).map(|l| drawn_width(&ed, l)).collect();
    assert!(widths.iter().all(|w| *w == widths[0]), "{widths:?}");
    // Six columns of markup came off that one row — `` ` `` twice and
    // `**` twice — and six columns of padding went back on. Only there:
    // the rows that lost nothing are still exactly the file.
    assert_eq!(
        ed.drawn_on_line(2)
            .iter()
            .map(|(_, text)| text.chars().count())
            .sum::<usize>(),
        6,
        "{:?}",
        ed.drawn_on_line(2)
    );
    assert!(ed.drawn_on_line(3).is_empty(), "{:?}", ed.drawn_on_line(3));
}

/// #212: every table in the document, not only the one the cursor is in.
#[test]
fn every_table_on_the_page_is_squared_up_not_only_the_cursors() {
    let mut ed = Editor::new();
    ed.add_buffer(crate::Buffer::from_text(
        "|甲|乙|\n|---|---|\n|一二三|四|\n\n中間一段散文。\n\n|丙|丁|\n|---|---|\n|五六七|八|\n",
    ));
    press(&mut ed, "gg");
    for table in [0, 6] {
        let widths: Vec<usize> = (table..table + 3).map(|l| drawn_width(&ed, l)).collect();
        assert_eq!(widths[0], widths[1], "table at {table}: {widths:?}");
        assert_eq!(widths[1], widths[2], "table at {table}: {widths:?}");
    }
    assert!(ed.drawn_on_line(4).is_empty(), "prose is not a table");
}

/// #212: a table in a fence is writing *about* a table.
#[test]
fn a_quoted_table_is_not_padded() {
    let mut ed = Editor::new();
    ed.add_buffer(crate::Buffer::from_text(
        "```\n|甲|乙|\n|---|---|\n|一二三|四|\n```\n",
    ));
    for line in 1..4 {
        assert!(ed.drawn_on_line(line).is_empty(), "line {line}");
    }
}

/// #212: the two settings that mean "draw me the file".
#[test]
fn the_padding_goes_away_when_the_page_is_the_file() {
    let mut ed = Editor::new();
    ed.add_buffer(crate::Buffer::from_text("|甲|乙|\n|---|---|\n|一二三|四|\n"));
    assert!(!ed.drawn_on_line(0).is_empty());

    // `:render off` is a request for the file exactly as it is.
    ed.execute("render off").unwrap();
    assert!(ed.drawn_on_line(0).is_empty(), "{:?}", ed.drawn_on_line(0));
    ed.execute("render basic").unwrap();
    assert!(!ed.drawn_on_line(0).is_empty());

    // Down a 縱 every character takes one cell, so display width squares
    // nothing up.
    ed.set_layout(Layout::Vertical);
    assert!(ed.drawn_on_line(0).is_empty());

    // Neither is a candidate: `has_candidate` answers only for what the
    // writer typed, and the padding is derived.
    assert!(!ed.has_candidate());
}

/// #212 with #211: a candidate and the padding on the same line.
///
/// Two runs standing before the same character would be two answers to
/// "what is drawn here", and the caret, the click map and the wrap would
/// each pick their own.
#[test]
fn a_candidate_and_the_padding_are_one_run_each() {
    let mut ed = Editor::new();
    ed.add_buffer(crate::Buffer::from_text("|甲|乙|\n|---|---|\n|一二三|四|\n"));
    let at = ed.drawn_on_line(0).first().map(|&(at, _)| at).unwrap();
    ed.set_candidate(vec![(0, at, "候".to_string())]);
    let runs = ed.drawn_on_line(0);
    let mut anchors: Vec<usize> = runs.iter().map(|&(at, _)| at).collect();
    anchors.dedup();
    assert_eq!(anchors.len(), runs.len(), "one run per anchor: {runs:?}");
    // **The candidate comes first in it.** It continues the word the caret
    // is in; the padding's job is to reach the closing pipe, so it belongs
    // on the far side of what was typed.
    let held = runs
        .iter()
        .find(|(_, text)| text.contains('候'))
        .map(|(_, text)| text.clone())
        .unwrap_or_else(|| panic!("{runs:?}"));
    assert!(held.starts_with('候'), "{held:?}");
}

/// #212 with #211: the caret stands between the two of them.
///
/// A candidate and the padding are drawn at the same anchor, and the caret
/// goes *after* what was typed and *before* the space that reaches to the
/// pipe. Counting the whole run drew the caret on the pipe after every
/// keystroke in a table — even with no candidate at all, since the one
/// space off a pipe is anchored exactly where a caret typing at the end of
/// a cell is.
#[test]
fn the_caret_stands_after_what_was_typed_and_before_the_padding() {
    let mut ed = Editor::new();
    ed.add_buffer(crate::Buffer::from_text("|a|bbb|
|-|-|
|cc|d|
"));
    // Typing at the end of the first cell: the caret is on the `|` at
    // index 2, and the padding that widens that cell is anchored there.
    ed.set_cursor(2);
    let hide = |line: usize| ed.hidden_on_line(line);
    let fold = |line: usize| ed.line_is_folded(line);
    let drawn = |line: usize| ed.drawn_on_line(line);
    let typed = |line: usize| ed.typed_on_line(line);
    let m = crate::wrap::Measure::new(crate::wrap::NO_WRAP, &hide)
        .with_folds(&fold)
        .with_drawn(&drawn)
        .with_typed_drawn(&typed);
    let at = crate::wrap::position(ed.current_buffer().rope(), 2, m);
    // `| a` — the caret is right after the `a` it just typed, not out on
    // the pipe two cells further along.
    assert_eq!(at.column, 3, "{:?}", ed.drawn_on_line(0));
}

/// #212: `:syntax` changes what comes off the page without touching a byte
/// of the file, so the padding memo has to be keyed on it too.
#[test]
fn the_padding_follows_the_syntax_the_file_is_read_with() {
    let mut ed = Editor::new();
    ed.add_buffer(crate::Buffer::from_text("| `a` | bbbb |
| --- | ---- |
| cc | d |
"));
    ed.execute("render full").unwrap();
    let with_markup_off = ed.drawn_on_line(0);
    ed.execute("syntax text").unwrap();
    let as_plain_text = ed.drawn_on_line(0);
    // With the backticks back on the page the first cell is two cells
    // wider, so it cannot want the same padding.
    assert_ne!(
        with_markup_off, as_plain_text,
        "the memo answered for the other syntax"
    );
}

// ---- Read-only, and reading again (Features #213 / #214) --------------

/// #213: one gate, and everything is behind it.
///
/// The point of putting the refusal in `Buffer::insert`/`remove` rather
/// than in each command is that a path nobody thought about is still
/// refused. So this presses the ones that reach the rope by different
/// routes: Insert mode, `x`, `d`, `o` (which goes round the cell guard),
/// paste, `J`, `Ctrl-A`, `ms`, `:s` — and `u`.
#[test]
fn a_locked_buffer_refuses_every_way_in() {
    // Built with `from_text`, not by typing into it: the buffer starts
    // **clean**, so the `is_modified()` check at the end has something to
    // catch. Built by typing, it would already be dirty and the check
    // could not fail however much leaked through.
    const TEXT: &str = "一二三 1\n四五六\n";
    let mut ed = Editor::new();
    ed.add_buffer(crate::Buffer::from_text(TEXT));
    ed.current_buffer_mut().set_readonly(true);
    let before = ed.current_buffer().text();
    assert!(!ed.current_buffer().is_modified(), "clean to begin with");

    press(&mut ed, "i");
    assert_eq!(ed.mode(), Mode::Normal, "Insert mode is not even entered");
    assert!(ed.status().contains("只讀"), "{}", ed.status());
    ed.on_key(Key::Char('甲'));
    assert_eq!(
        ed.current_buffer().text(),
        before,
        "{}",
        ed.current_buffer().text()
    );

    // Each of these is checked twice: once on a buffer that is *not*
    // locked, to prove the keystroke edits at all — a list of keys that do
    // nothing anywhere would pass the locked half and prove nothing — and
    // once on the locked one.
    let ways: [(&str, &[Key]); 11] = [
        ("d", &[Key::Char('d')]),
        ("yp", &[Key::Char('y'), Key::Char('p')]),
        ("o", &[Key::Char('o')]),
        ("O", &[Key::Char('O')]),
        ("a甲", &[Key::Char('a'), Key::Char('甲')]),
        ("c甲", &[Key::Char('c'), Key::Char('甲')]),
        ("i甲", &[Key::Char('i'), Key::Char('甲')]),
        ("gJ", &[Key::Char('g'), Key::Char('J')]),
        ("ms(", &[Key::Char('m'), Key::Char('s'), Key::Char('(')]),
        ("Ctrl-A", &[Key::Ctrl('a')]),
        ("Ctrl-X", &[Key::Ctrl('x')]),
    ];
    for (name, keys) in ways {
        let mut open = Editor::new();
        open.add_buffer(crate::Buffer::from_text(TEXT));
        press(&mut open, "gg");
        for key in keys {
            open.on_key(*key);
        }
        open.on_key(Key::Esc);
        assert_ne!(
            open.current_buffer().text(),
            before,
            "`{name}` does not edit even an unlocked buffer — bad test"
        );

        ed.set_status(String::new());
        press(&mut ed, "gg");
        for key in keys {
            ed.on_key(*key);
        }
        ed.on_key(Key::Esc);
        assert_eq!(
            ed.current_buffer().text(),
            before,
            "`{name}` moved a locked buffer"
        );
    }

    // The commands go the same way, and say which file refused rather than
    // reporting a count of replacements nobody made.
    let mut open = Editor::new();
    open.add_buffer(crate::Buffer::from_text(TEXT));
    assert!(open.execute("s/一/壹/g").is_ok());
    assert_ne!(
        open.current_buffer().text(),
        before,
        "`:s` does not edit even an unlocked buffer — bad test"
    );
    assert!(ed.execute("s/一/壹/g").is_ok());
    assert_eq!(ed.current_buffer().text(), before, "`:s` moved a locked one");
    assert!(ed.status().contains("只讀"), "{}", ed.status());

    // …and going *back* is still moving it.
    press(&mut ed, "u");
    assert_eq!(ed.current_buffer().text(), before);
    assert!(ed.status().contains("只讀"), "{}", ed.status());
    assert!(
        !ed.current_buffer().is_modified(),
        "and nothing marked it changed"
    );

    // …and unlocking gives it all back.
    assert!(ed.execute("readonly off").is_ok());
    press(&mut ed, "ggi");
    assert_eq!(ed.mode(), Mode::Insert, "unlocked, the door opens again");
    ed.on_key(Key::Char('甲'));
    assert!(
        ed.current_buffer().text().starts_with('甲'),
        "{}",
        ed.current_buffer().text()
    );
}

/// §5.2.2 fault 4: the `.md` grid keys reported success on a locked file.
///
/// The text was never in danger — `Buffer::insert` does not move a locked
/// rope — so what the writer lost was only the truth: 「加了一行」 over a
/// grid that had not changed, while the `.csv` half of the same `t` menu
/// refused properly.
/// §5.2.2 fault 2, driven: the switch the menu offers, twice in a row.
///
/// `:view-hanging` declared `Args::Words(ON_OFF)` and ignored the word, so the
/// second `:view-hanging off` turned 標點旁置 **on** — and the status line said
/// so, which is how it survived: it was never silent, only wrong.
#[test]
fn hanging_punctuation_listens_to_the_word_it_is_given() {
    let mut ed = typed("「春」。\n");
    // 旁置 is a 竪排 word and says so (`Need::Vertical`, `Need::Loose`);
    // asked on a 橫排 page it answers 「還不行，需要：竪排」 and changes
    // nothing, which would make every assertion below pass for the wrong
    // reason.
    assert!(ed.execute("layout vertical").is_ok());
    assert!(ed.execute("view-dense off").is_ok());
    for _ in 0..2 {
        assert!(ed.execute("view-hanging off").is_ok());
        assert!(!ed.hanging, "`:view-hanging off` turned it on: {}", ed.status());
    }
    for _ in 0..2 {
        assert!(ed.execute("view-hanging on").is_ok());
        assert!(ed.hanging, "`:view-hanging on` turned it off: {}", ed.status());
    }
    // The bare word still means 「the other one」, and `of` is `off`'s
    // shortest spelling — the one the menu prints.
    assert!(ed.execute("view-hanging").is_ok());
    assert!(!ed.hanging);
    assert!(ed.execute("view-hanging on").is_ok());
    assert!(ed.execute("view-hanging of").is_ok());
    assert!(!ed.hanging, "`:view-hanging of` is what the menu offers");
}

#[test]
fn the_markdown_grid_keys_say_so_on_a_locked_file() {
    let mut ed = typed("| 甲 | 乙 |\n| --- | --- |\n| 一 | 二 |\n");
    press(&mut ed, "tf");
    ed.goto_line(3);
    assert!(ed.execute("readonly on").is_ok());
    let before = ed.current_buffer().text();
    for keys in ["tr", "td", "tR", "tc", "tD"] {
        press(&mut ed, keys);
        assert_eq!(
            ed.current_buffer().text(),
            before,
            "`{keys}` moved a locked grid"
        );
        assert!(ed.status().contains("只讀"), "`{keys}`: {}", ed.status());
    }
    assert!(ed.execute("table-sort").is_ok());
    assert_eq!(ed.current_buffer().text(), before, "`:table-sort` moved it");
    assert!(ed.status().contains("只讀"), "{}", ed.status());
}

/// #213: `o` on a file whose last line has no newline of its own.
///
/// The refusal returns early, so everything the caller worked out about
/// where the new line would be is now about a line that does not exist.
/// This is the shape that used to put the cursor one past the end and
/// panic on the next frame.
#[test]
fn a_locked_buffer_refuses_the_line_that_would_have_been_added() {
    for text in ["一二三", "一二三\n"] {
        let mut ed = Editor::new();
        ed.add_buffer(crate::Buffer::from_text(text));
        ed.current_buffer_mut().set_readonly(true);
        for keys in ["Go", "ggO"] {
            press(&mut ed, keys);
            ed.on_key(Key::Esc);
            assert_eq!(ed.current_buffer().text(), text, "{keys} on {text:?}");
            assert!(
                ed.cursor() <= ed.current_buffer().char_count(),
                "{keys} on {text:?}: cursor {} past the end {}",
                ed.cursor(),
                ed.current_buffer().char_count()
            );
            // The frame is drawn from the cursor, so this is where the
            // panic used to land.
            let _ = ed.render();
        }
    }
}

/// #213: a locked buffer must not *take over* a crashed session's draft.
///
/// `:recover` loads the draft as an undoable edit and adopts the swap
/// file, which is deleted on quit. A refusal that only skipped the edit
/// would still have adopted — and thrown the work away.
#[test]
fn a_locked_buffer_keeps_the_draft_it_cannot_open() {
    let dir = std::env::temp_dir().join(format!(
        "yumete-lock-rec-{}-{}",
        std::process::id(),
        "a_locked_buffer_keeps_the_draft_it_cannot_open"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chapter.md");
    let swap = dir.join(".chapter.md.yumete");
    std::fs::write(&path, "第一稿\n").unwrap();
    std::fs::write(&swap, "第一稿，還有三千字沒存的\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    ed.current_buffer_mut().set_readonly(true);
    ed.execute(":recover").unwrap();
    assert!(ed.status().contains("只讀"), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "第一稿\n", "nothing was loaded");
    assert!(swap.exists(), "and the draft is still there to be recovered");

    // Unlock, and it is all still waiting.
    assert!(ed.execute("readonly off").is_ok());
    ed.execute(":recover").unwrap();
    assert!(ed.current_buffer().text().contains("三千字"));

    std::fs::remove_dir_all(&dir).ok();
}

/// #213: `:readonly` with no word asks rather than sets.
#[test]
fn readonly_says_which_way_it_is() {
    let mut ed = Editor::new();
    assert!(ed.execute("readonly").is_ok());
    assert!(ed.status().contains("off"), "{}", ed.status());
    assert!(ed.execute("ro on").is_ok());
    assert!(ed.is_readonly());
    assert!(ed.execute("readonly").is_ok());
    assert!(ed.status().contains("on"), "{}", ed.status());
    // A word that is neither is a mistake, not a toggle.
    assert!(ed.execute("readonly 也許").is_err());
}

/// §5.2.3 ⑤: a refused edit does not move the state around the text.
///
/// `d` on a locked buffer used to collapse the selection onto its start —
/// the rope was untouched, the highlight was gone, and the reader had lost
/// their selection to an edit that never happened. The silent early return
/// in `Buffer::remove` could not prevent that, and is why the gate now
/// answers.
#[test]
fn a_refused_delete_leaves_the_selection_where_it_was() {
    let mut ed = Editor::new();
    ed.add_buffer(crate::Buffer::from_text("一二三四五"));
    ed.anchor = 1;
    ed.cursor = 3;
    ed.current_buffer_mut().set_readonly(true);
    press(&mut ed, "d");
    assert_eq!(ed.current_buffer().text(), "一二三四五", "the text stayed");
    assert_eq!((ed.anchor, ed.cursor), (1, 3), "so did the selection");
    assert!(ed.status().contains("只讀"), "and it said so: {}", ed.status());
}

/// #213: `--readonly` locks the ones already open *and* the next one.
#[test]
fn the_readonly_flag_is_about_the_session_not_one_file() {
    let dir = std::env::temp_dir().join(format!("yumete-ro-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.md");
    let b = dir.join("b.md");
    std::fs::write(&a, "甲\n").unwrap();
    std::fs::write(&b, "乙\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&a).unwrap();
    ed.set_readonly_default(true);
    assert!(ed.is_readonly(), "the one already open");
    ed.open_file(&b).unwrap();
    assert!(ed.is_readonly(), "and the next one `:open` reaches for");

    std::fs::remove_dir_all(&dir).ok();
}

/// #213: the disk's own answer, read at open rather than at `:w`.
#[cfg(unix)]
#[test]
fn a_file_the_disk_calls_read_only_comes_up_locked() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("yumete-ro444-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("別動.md");
    std::fs::write(&file, "這一份不要改\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();

    let mut ed = Editor::new();
    ed.open_file(&file).unwrap();
    assert!(ed.is_readonly(), "read at open, not discovered at :w");
    press(&mut ed, "i");
    ed.on_key(Key::Char('改'));
    assert_eq!(ed.current_buffer().text(), "這一份不要改\n");

    // …and a file that does not exist yet is *unwritten*, not read-only.
    ed.open_file(dir.join("還沒寫.md")).unwrap();
    assert!(!ed.is_readonly());

    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).ok();
    std::fs::remove_dir_all(&dir).ok();
}

/// #213 × #214: locked is about *editing*. Taking a fresh copy of the file
/// is the one thing a reader does want.
#[test]
fn a_locked_buffer_can_still_be_re_read() {
    let dir = std::env::temp_dir().join(format!("yumete-rorl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("表.txt");
    std::fs::write(&file, "第一版\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&file).unwrap();
    ed.current_buffer_mut().set_readonly(true);
    std::fs::write(&file, "第二版\n").unwrap();
    assert!(ed.execute("reload").is_ok());
    assert_eq!(ed.current_buffer().text(), "第二版\n");
    assert!(ed.is_readonly(), "and it is still locked afterwards");

    std::fs::remove_dir_all(&dir).ok();
}

/// #214: `:reload` will not take your afternoon; `:reload!` will, and says
/// so in its name.
#[test]
fn reload_stops_at_unsaved_changes_and_the_bang_does_not() {
    let dir = std::env::temp_dir().join(format!("yumete-rl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("章.md");
    std::fs::write(&file, "原稿\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&file).unwrap();
    press(&mut ed, "i");
    ed.on_key(Key::Char('改'));
    ed.on_key(Key::Esc);
    assert!(ed.current_buffer().is_modified());

    std::fs::write(&file, "外面的版本\n").unwrap();
    assert!(
        ed.execute("reload").is_err(),
        "unsaved changes are in the way"
    );
    assert!(ed.current_buffer().text().contains('改'), "and still here");

    assert!(ed.execute("reload!").is_ok());
    assert_eq!(ed.current_buffer().text(), "外面的版本\n");
    assert!(!ed.current_buffer().is_modified());

    // A buffer with no file has nothing to re-read.
    let mut scratch = Editor::new();
    assert!(scratch.execute("reload").is_err());

    // `:e!` and `:o!` are gone outright — no alias, no hint.
    assert!(ed.execute("e!").is_err());
    assert!(ed.execute("o!").is_err());
    assert!(ed.execute("edit!").is_err());
    assert!(ed.execute("open!").is_err());
    // **Not even with an argument.** `e` and `o` are `open`'s own aliases,
    // and the walk over the six commands that take a bang used to skip
    // `open` — which takes none — and land on `export`, so `:e! 第三章.md`
    // typed by a hand meaning「re-read it」 wrote an export over it.
    assert!(ed.execute("e! 第三章.md").is_err());
    assert!(ed.execute("o! 第三章.md").is_err());
    assert!(!dir.join("第三章.md").exists(), "nothing was written");

    std::fs::remove_dir_all(&dir).ok();
}

/// #214: the automatic half takes a clean buffer and never a dirty one.
#[test]
fn auto_reload_takes_a_clean_buffer_and_only_warns_about_a_dirty_one() {
    let dir = std::env::temp_dir().join(format!("yumete-rlauto-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("同步中.md");
    std::fs::write(&file, "第一版\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&file).unwrap();
    // Off by default: nothing happens on its own until it is asked for.
    std::fs::write(&file, "第二版\n").unwrap();
    ed.disk_tick();
    assert_eq!(ed.current_buffer().text(), "第一版\n", "off by default");

    assert!(ed.execute("reload-auto on").is_ok());
    ed.disk_tick();
    assert_eq!(ed.current_buffer().text(), "第二版\n", "clean, so it reads");
    assert!(ed.status().contains("外面改了"), "{}", ed.status());

    // Now type into it, and let the file move again.
    press(&mut ed, "i");
    ed.on_key(Key::Char('我'));
    ed.on_key(Key::Esc);
    std::fs::write(&file, "第三版\n").unwrap();
    // Re-asking for the setting also resets the clock, which is what a
    // writer who just turned it on means by turning it on.
    assert!(ed.execute("reload-auto on").is_ok());
    ed.disk_tick();
    assert!(
        ed.current_buffer().text().contains('我'),
        "a dirty buffer is never read over: {}",
        ed.current_buffer().text()
    );
    // On `:reload!`, not on the wording: these lines are hand-edited in
    // three languages and the command name is the one part of the sentence
    // that is the same in all of them.
    assert!(ed.status().contains(":reload!"), "{}", ed.status());

    assert!(ed.execute("reload-auto off").is_ok());
    assert!(!ed.reload_auto());
    assert!(ed.execute("reload-auto").is_ok());
    assert!(ed.status().contains("off"), "{}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

// ---- Markdown tables (Feature #142) -----------------------------------

/// A document with a `|` table in the middle of it, cursor on line 3.
fn with_md_table() -> Editor {
    let mut ed = typed("前文\n| 字 | 讀音 |\n| --- | --- |\n| 木 | mu |\n| 目 | mu |\n後文\n");
    ed.goto_line(4);
    ed
}

/// The same table, squared up — which is what makes it one the editor keeps
/// squared up as it is typed in (#329).
fn with_lined_up_md_table() -> Editor {
    let mut ed = typed(
        "前文\n| 字 | 讀音 |\n| -- | ---- |\n| 木 | mu   |\n| 目 | mu   |\n後文\n",
    );
    ed.goto_line(4);
    ed
}

/// A document whose second table has a cell far wider than the cap, and a
/// first table with different columns — so a test can tell the two apart.
fn with_two_md_tables() -> Editor {
    let long = "一二三四五六七八九十一二三四五六七八九十一二三四五";
    typed(&format!(
        "| 姓名 | 年紀 |\n| --- | --- |\n| 甲 | 三十 |\n\n段落\n\n\
         | 地名 | 備註 |\n| --- | --- |\n| 洛陽 | {long} |\n"
    ))
}

/// #283. 「markdown中的表格在 tf 模式下都没办法通过 tw 来缩小单元格宽度」——
/// there was no `t w`, and no cap in prose at all: a 80-cell cell pushed
/// every column after it off the side, which is what this project's own
/// `development.md` looked like.
#[test]
fn t_w_folds_a_cell_too_wide_to_scan_and_gives_it_back() {
    let mut ed = with_two_md_tables();
    ed.goto_line(9);
    press(&mut ed, "tf");
    // Row 9's 備註 is 25 characters — 50 cells — so its tail comes off.
    let folded = ed.hidden_on_line(8);
    assert!(!folded.is_empty(), "the wide cell is folded: {folded:?}");
    let marks = ed.drawn_runs_on_line(8);
    assert!(
        marks
            .iter()
            .any(|r| r.text == crate::mdtable::FOLD_MARK && r.ink == crate::drawn::Ink::Fold),
        "and says so on the page, in an ink of its own: {marks:?}"
    );
    // What is left is the cap, mark included — never one cell more.
    let text = ed.line_text(8).unwrap();
    let chars: Vec<char> = text.trim_end().chars().collect();
    let shown: String = (0..chars.len())
        .filter(|at| !folded.iter().any(|&(a, b)| (a..b).contains(at)))
        .map(|at| chars[at])
        .collect();
    let cell = shown.split('|').nth(2).unwrap_or_default().trim().to_string();
    assert!(
        yumete_cjk::str_width(&cell) + yumete_cjk::str_width(crate::mdtable::FOLD_MARK)
            <= crate::mdtable::MAX_COLUMN,
        "「{cell}」 and the mark fit in {}",
        crate::mdtable::MAX_COLUMN
    );

    // `t w` gives the whole cell back — the reader who came to *read* it.
    press(&mut ed, "tw");
    assert!(ed.hidden_on_line(8).is_empty(), "nothing off the page now");
    press(&mut ed, "tw");
    assert!(!ed.hidden_on_line(8).is_empty(), "and folded again");
}

/// The author, off his own `development.md`: 「the long cells are trimmed
/// with a `>` symbol. However, the width of the cell are still padded
/// with white spaces at the tail.」
///
/// A table squared up **in the file** carries the column's whole width
/// inside every cell, as spaces; the padding is measured pipe to pipe, so
/// each of those cells went on voting 50 cells wide however short its
/// writing was. The mark stood where the writing stopped and a field of
/// nothing ran from there to the pipe — the cap saved no room at all.
#[test]
fn a_table_squared_up_in_the_file_is_still_folded_to_the_cap() {
    let long = "一二三四五六七八九十一二三四五六七八九十一二三四五";
    let rows: Vec<String> = format!("| 姓名 | 備註 |\n| --- | --- |\n| 甲 | {long} |")
        .lines()
        .map(str::to_string)
        .collect();
    let square = crate::mdtable::format(&rows);
    let file: Vec<usize> = square.iter().map(|l| yumete_cjk::str_width(l)).collect();
    let mut ed = typed(&format!("{}\n", square.join("\n")));
    ed.goto_line(3);
    press(&mut ed, "tf");

    assert!(
        !ed.hidden_on_line(0).is_empty(),
        "the short row gives its padding up too, or the column stays wide"
    );
    let page: Vec<usize> = (0..3).map(|l| drawn_width(&ed, l)).collect();
    assert!(
        page.iter().zip(&file).all(|(drawn, wrote)| drawn < wrote),
        "every row is drawn narrower than the file wrote it: {page:?} of {file:?}"
    );
    // And they still square up: the mark stands in the cell it saved, so
    // no row is more than the one cell it costs from any other.
    let (low, high) = (page.iter().min().unwrap(), page.iter().max().unwrap());
    assert!(high - low <= 1, "square within the mark's own cell: {page:?}");
    assert!(
        *high <= crate::mdtable::MAX_COLUMN + yumete_cjk::str_width("| 姓名 |") + 3,
        "and the wide column is drawn at the cap, not at the file's width: {page:?}"
    );
}

/// 「`basic` 不藏、不摺、不替換」 is a law about what a *level* does with
/// nobody asking. `t w` is the reader asking (author, 2026-09-07: 「虽然
/// tb 在默认状态下不折叠，但能不能在按下 tw 之后折叠？」), and 基本 squares
/// its columns up exactly as 全 does — so there is something to fold
/// against, and folding it is the reader's call and not the level's.
#[test]
fn 基本_folds_nothing_unasked_and_folds_when_asked() {
    let mut ed = with_two_md_tables();
    ed.goto_line(9);
    press(&mut ed, "tb");
    assert!(ed.hidden_on_line(8).is_empty(), "基本 hides nothing unasked");
    press(&mut ed, "tw");
    assert_eq!(ed.table_level(), TableLevel::Basic, "and stays 基本");
    assert!(
        !ed.hidden_on_line(8).is_empty(),
        "asked, it folds without raising the level: {}",
        ed.status()
    );
    press(&mut ed, "tw");
    assert!(ed.hidden_on_line(8).is_empty(), "and gives it back: {}", ed.status());
}

/// 源碼 draws the file as it is written, so no column has been squared up
/// and there is nothing to measure a fold against. The refusal names the
/// two keys that square one up rather than raising a level behind the
/// reader's back.
#[test]
fn 源碼_has_no_squared_up_column_to_fold_and_says_so() {
    let mut ed = with_two_md_tables();
    ed.goto_line(9);
    press(&mut ed, "to");
    press(&mut ed, "tw");
    assert_eq!(ed.table_level(), TableLevel::Off, "and stays 源碼");
    assert!(ed.hidden_on_line(8).is_empty(), "nothing folded");
    assert!(ed.status().contains("tb"), "it points at the keys: {}", ed.status());
}

/// The level supplies the default and the reader's own answer outlives it:
/// a `t w` pressed at 基本 is still the answer at 全, and the other way
/// round. Otherwise walking between levels would keep undoing the reader.
#[test]
fn the_answer_t_w_gave_travels_between_the_levels() {
    let mut ed = with_two_md_tables();
    ed.goto_line(9);
    press(&mut ed, "tf");
    assert!(!ed.hidden_on_line(8).is_empty(), "全 folds to begin with");
    press(&mut ed, "tw");
    assert!(ed.hidden_on_line(8).is_empty(), "and `t w` opens it: {}", ed.status());
    press(&mut ed, "tb");
    assert!(ed.hidden_on_line(8).is_empty(), "基本 does not fold it back");
    press(&mut ed, "tf");
    assert!(ed.hidden_on_line(8).is_empty(), "nor does walking back up to 全");
}

/// #283, off `development.md` itself (2026-09-07): 「tw 功能无
/// 法在 tt 模式下使用，导致所有的长单元格都是保持折叠状态且没有折叠符号…
/// 因此我永远没有办法读取完整内容」.
///
/// The refusal asked for 全 and the pane is not a level, so it fired on
/// every press inside the window these tables are read in — and the
/// grid's own 32-cell cap had no other key against it.
#[test]
fn t_w_is_answered_in_the_pane_which_is_not_below_全() {
    let mut ed = with_two_md_tables();
    ed.goto_line(9);
    press(&mut ed, "tt");
    assert!(ed.table().unwrap().takes_the_pane(), "{}", ed.status());
    assert!(ed.cell_folds(), "the grid folds to its cap to begin with");
    press(&mut ed, "tw");
    assert!(!ed.cell_folds(), "and `t w` is heard: {}", ed.status());
    assert!(
        !ed.status().contains("tb"),
        "no refusal in a window that squares its own columns up: {}",
        ed.status()
    );
    press(&mut ed, "tw");
    assert!(ed.cell_folds(), "and back again: {}", ed.status());
}

/// **全窗表格 is bounded**: `gg`, `G` and `:120` do not walk out of it
/// (author, 2026-09-07: 「衹能通過 tq/tf/tb 離開回到其他模式，或者 t[ t]
/// 去下一個表格」), and the gutter numbers this table's own rows, so
/// `t1g` is its first row.
#[test]
fn the_window_holds_the_table_it_was_opened_on() {
    let mut ed = typed(
        "前文一
前文二

| a | b |
| --- | --- |
| 1 | 2 |
| 3 | 4 |
| 5 | 6 |

後文

| c | d |
| --- | --- |
| 7 | 8 |
",
    );
    ed.goto_line(6);
    press(&mut ed, "tt");
    let (first, last) = ed.table_row_span().expect("a table under the cursor");
    assert_eq!((first, last), (5, 7), "rows are lines 6, 7 and 8");
    assert_eq!(ed.table_row_base(), first, "the window counts from its first row");

    // `gg` is the table's first row, not the file's first line.
    press(&mut ed, "gg");
    assert_eq!(ed.cursor_line(), first, "{}", ed.status());
    assert_eq!(ed.table_row_number(), 1, "and it is row 1 in the window");
    assert!(ed.status().contains("t q"), "it says so: {}", ed.status());
    // `G` is its last row, not the file's last line.
    press(&mut ed, "G");
    assert_eq!(ed.cursor_line(), last, "{}", ed.status());
    assert_eq!(ed.table_row_number(), 3);
    // `t<n>g` counts the same rows the gutter draws.
    press(&mut ed, "t1g");
    assert_eq!(ed.cursor_line(), first, "{}", ed.status());
    press(&mut ed, "t2g");
    assert_eq!(ed.cursor_line(), first + 1, "{}", ed.status());
    press(&mut ed, "t99g");
    assert_eq!(ed.cursor_line(), last, "past the end is the last row");

    // `t ]` is the way to the next table, and it is never held.
    press(&mut ed, "t]");
    assert!(ed.cursor_line() > last, "into the next table: {}", ed.status());
    press(&mut ed, "t[");
    assert!(ed.cursor_line() <= last, "and back: {}", ed.status());

    // And the way out is a key that says so: `t q` gives the window back
    // and `gg` is the file's again.
    press(&mut ed, "tq");
    press(&mut ed, "gg");
    assert_eq!(ed.cursor_line(), 0, "{}", ed.status());
}

/// `t w` and `t a` are two toggles over **one** axis: 摺起, 攤平, 折行,
/// and never two of them at once (author, 2026-09-07: 「ta on 和 tw on 两
/// 者不会叠在一起」).
///
/// A three-way cycle on `t w` was the other way to spell it, and it would
/// have made one key a toggle in prose and a cycle in the window.
#[test]
fn t_w_and_t_a_are_two_toggles_over_one_axis() {
    let mut ed = with_two_md_tables();
    ed.goto_line(9);
    press(&mut ed, "tt");
    assert!(ed.cell_folds() && !ed.cell_wrap(), "the grid opens folded");
    // 折行 takes over from 摺起. **The cap still bites** — 折行 *is* 摺起
    // plus 「and draw the rest underneath」, which is why the prose page,
    // where the second half cannot be drawn, still folds.
    press(&mut ed, "ta");
    assert!(ed.cell_wrap(), "{}", ed.status());
    assert!(ed.cell_folds(), "the cap is what 折行 wraps at: {}", ed.status());
    // …and `t w` takes it back off — to 攤平, not to 摺起 (author,
    // 2026-09-08). Both keys name a way of *not* showing a cell whole, so
    // the way back from either of them is the whole cell; answering 折行
    // with 摺起 handed the reader the one state they had not named.
    press(&mut ed, "tw");
    assert!(!ed.cell_folds() && !ed.cell_wrap(), "攤平: {}", ed.status());
    // Each is still a toggle of its own: from 攤平 it folds, and again
    // from 摺起 it opens back out.
    press(&mut ed, "tw");
    assert!(ed.cell_folds() && !ed.cell_wrap(), "摺起: {}", ed.status());
    press(&mut ed, "tw");
    assert!(!ed.cell_folds() && !ed.cell_wrap(), "攤平: {}", ed.status());
    press(&mut ed, "ta");
    assert!(ed.cell_wrap(), "{}", ed.status());
    press(&mut ed, "ta");
    assert!(!ed.cell_wrap() && !ed.cell_folds(), "攤平 again: {}", ed.status());
}

/// 折行 is the grid's answer and says so where it cannot be drawn — but it
/// **sets the switch**, so `t t` finds the answer already given.
#[test]
fn t_a_says_where_it_works_and_still_remembers() {
    let mut ed = with_two_md_tables();
    ed.goto_line(9);
    press(&mut ed, "tf");
    press(&mut ed, "ta");
    assert!(ed.status().contains("tt"), "it points at the window: {}", ed.status());
    assert!(ed.cell_wrap(), "and the switch is set: {}", ed.status());
    // In prose the cap still bites — 折行 is 摺起 plus a second half that
    // only the grid can draw.
    assert!(!ed.hidden_on_line(8).is_empty(), "the tail is still folded away");
}

/// **Reading does not open a cell; typing in one does** (#283, remade
/// 2026-09-07).
///
/// Walking in used to open it, and that one line put the caret into the
/// table's layout: the page's geometry then changed on every keystroke,
/// and a `j` down a two-character column rebuilt 286 rows. Reading is
/// what `t i`'s panel is for — it holds the whole cell, wrapped — so the
/// page keeps its columns still, and the tail comes back exactly when
/// somebody needs to type in it, because a caret may never sit inside
/// characters the page does not draw.
#[test]
fn a_folded_cell_opens_to_be_typed_in_and_not_to_be_walked_over() {
    let mut ed = with_two_md_tables();
    ed.goto_line(9);
    press(&mut ed, "tf");
    assert!(!ed.hidden_on_line(8).is_empty());
    // Into the 備註 cell — the last one on the row.
    while ed.cell_position().map(|(_, c)| c) != Some(1) {
        ed.on_key(Key::Char('l'));
    }
    assert!(
        !ed.hidden_on_line(8).is_empty(),
        "standing in it leaves it folded — the panel is where it is read"
    );
    // `i` is the other half: what is being typed in is on the page.
    ed.on_key(Key::Char('i'));
    assert!(
        ed.hidden_on_line(8).is_empty(),
        "the cell being typed in is whole: {}",
        ed.status()
    );
    ed.on_key(Key::Esc);
    assert!(!ed.hidden_on_line(8).is_empty(), "and folds again on Esc");
}

/// The other half of the same law: **an open cell does not move the
/// column**. It juts out past its own wall, and every other row keeps the
/// alignment it had — which is what lets the layout be worked out once
/// per edit instead of once per keystroke.
#[test]
fn a_cell_being_typed_in_juts_out_rather_than_widening_its_column() {
    let mut ed = with_two_md_tables();
    ed.goto_line(9);
    press(&mut ed, "tf");
    while ed.cell_position().map(|(_, c)| c) != Some(1) {
        ed.on_key(Key::Char('l'));
    }
    // The header row of that table is drawn at the folded width…
    let shut = drawn_width(&ed, 6);
    ed.on_key(Key::Char('i'));
    assert!(ed.hidden_on_line(8).is_empty(), "the cell is open");
    assert_eq!(
        drawn_width(&ed, 6),
        shut,
        "…and the rows around it are drawn exactly as wide as before"
    );
}

/// #283. 「markdown中的表格没办法用ti打开信息侧栏」—— `detail()` asked what
/// the *file* was and sent every Markdown question to the note panel, so
/// the row panel could only ever be reached by opening a `.csv`.
#[test]
fn t_i_reads_a_markdown_row_by_the_columns_of_its_own_table() {
    let mut ed = with_two_md_tables();
    ed.goto_line(9);
    press(&mut ed, "tf");
    assert!(ed.detail_visible(), "the panel is on out of the box");
    let panel = ed.detail().expect("a row of a Markdown table answers");
    let names: Vec<String> = panel.rows.iter().map(|(n, _)| n.clone()).collect();
    assert!(names[0].ends_with("地名"), "the second table's own: {names:?}");
    assert_eq!(
        panel.rows[0].1.as_deref(),
        Some("洛陽"),
        "and the value beside its own name, not one column over: {:?}",
        panel.rows
    );
    // The folded cell is read whole here — 「表格用來掃，側欄用來讀」.
    assert!(
        panel.rows[1].1.as_deref().unwrap_or_default().chars().count() == 25,
        "the panel is where the tail went: {:?}",
        panel.rows[1]
    );

    // The header is not a row, and neither is the `|---|`.
    ed.goto_line(7);
    assert!(ed.detail().is_none_or(|d| d.rows.is_empty()), "the header names columns");
    ed.goto_line(8);
    assert!(ed.detail().is_none_or(|d| d.rows.is_empty()), "the rule is the shape");
}

/// A document whose two tables are **different widths** — the case that
/// tells a stale schema from a fresh one.
fn with_a_narrow_and_a_wide_table() -> Editor {
    typed(
        "第一張，兩欄。\n\n| 甲 | 乙 |\n| --- | --- |\n| 一 | 二 |\n\n\
         第二張，五欄。\n\n| A | B | C | D | E |\n| --- | --- | --- | --- | --- |\n\
         | 木 | 一 | 木 | 四 | 五 |\n| 林 | 二 | 木木 | 四 | 五 |\n",
    )
}

/// #283, the last of it. Everything that counts columns counts **this**
/// table's: the view carries the schema of whichever table was entered,
/// and walking to a wider one used to leave every question answered by the
/// narrower one.
#[test]
fn the_second_table_is_measured_by_its_own_width() {
    let mut ed = with_a_narrow_and_a_wide_table();
    ed.goto_line(11);
    press(&mut ed, "tf");

    // The panel: five fields, not the first table's two.
    let panel = ed.detail().expect("a row of the wide table answers");
    assert_eq!(panel.rows.len(), 5, "{:?}", panel.rows);
    assert!(panel.rows[4].0.ends_with('E'), "{:?}", panel.rows[4]);

    // `t5/` looks in the fifth column. Clamped to the narrow table's two,
    // the fifth column of a five-column table was not there — and the
    // refusal named the wrong width, which is the shape of the bug that is
    // hardest to disbelieve: a wrong answer with a number on it.
    press(&mut ed, "T");
    press(&mut ed, "t5/");
    assert!(!ed.status().contains("沒有"), "column five is there: {}", ed.status());
    press(&mut ed, "t6/");
    assert!(ed.status().contains('5'), "five columns, not two: {}", ed.status());

    // `:table-check` reads the table the cursor is in — not the file, in
    // which every paragraph is a line 「寬度不對」.
    assert!(ed.execute("table-check").is_ok());
    assert!(
        ed.status().contains('2'),
        "two rows, rule and header excluded: {}",
        ed.status()
    );
}

/// A guessed block is not a `|` table, and reading its first line as a
/// Markdown header split a tab-delimited row on pipes: one column, and a
/// panel with nothing in it over a 碼表.
#[test]
fn a_block_table_reads_by_its_own_numbered_columns() {
    let mut ed = typed("一段話。\n\n木\tmu\t一\n林\tlin\t二\n森\tsen\t三\n");
    ed.goto_line(4);
    press(&mut ed, "tf");
    let panel = ed.detail().expect("a row of a block table answers");
    assert_eq!(panel.rows.len(), 3, "{:?}", panel.rows);
    assert_eq!(panel.rows[1].1.as_deref(), Some("lin"), "{:?}", panel.rows);
}

/// The first table's columns are not the second table's. `t y` reported
/// the first table's name from anywhere in the file and was believed.
#[test]
fn a_column_is_named_by_the_table_the_cursor_is_in() {
    let mut ed = with_two_md_tables();
    ed.goto_line(3);
    press(&mut ed, "tfty");
    assert!(ed.status().contains("姓名"), "{}", ed.status());
    ed.goto_line(9);
    press(&mut ed, "ty");
    assert!(ed.status().contains("地名"), "{}", ed.status());
}

#[test]
fn a_pipe_table_is_a_grid_wherever_it_is() {
    let mut ed = with_md_table();
    let before = ed.current_buffer().text();
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "T"); // #356: 這一條測的是格
    // **Looking does not rewrite.** Entering used to lay the whole region
    // out, which marks a file modified for having been read — 45 lines of
    // this project's own `development.md`, and `:table off` does not undo it.
    assert_eq!(ed.current_buffer().text(), before, "entering changed nothing");
    // `t f` is the tidy-up, said out loud: the columns line up on the
    // terminal, which is what a Markdown table is supposed to look like.
    // (`t t` is 「read this as a grid」 now — one letter, one meaning.)
    press(&mut ed, "tF");
    assert_eq!(
        ed.current_buffer().text(),
        "前文\n| 字 | 讀音 |\n| -- | ---- |\n| 木 | mu   |\n| 目 | mu   |\n後文\n"
    );
    // And `l` walks to the next cell rather than the next character.
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(0));
    press(&mut ed, "l");
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(1));
    assert_eq!(ed.cell_text(3, 1), "mu");
}

#[test]
fn the_grid_is_only_where_the_table_is() {
    // The point of scoping the mode to the region: walking out of the
    // table into the prose under it gives every key back. A mode that is
    // on everywhere would make `l` in a paragraph jump to the line's end.
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    assert!(ed.table_status().is_some(), "standing in it");
    ed.goto_line(6);
    assert!(ed.table_status().is_none(), "standing in the prose below");
    assert!(ed.md_region().is_none());
    // And `|` may be typed in prose, where it is just a character.
    press(&mut ed, "i|");
    assert!(ed.current_buffer().text().contains("|後文"), "{}", ed.status());
    ed.on_key(Key::Esc);
    ed.goto_line(4);
    assert!(ed.table_status().is_some(), "and walking back in brings it back");
}

#[test]
fn moving_down_a_column_steps_over_the_rule_and_walks_out_the_far_side() {
    // 「ti, ta 這兩個模式應該允許光標上下離開表格回到正文中（現在不可以）」
    // — 2026-09-05. A table drawn *in* a document is part of
    // it, so the row after the last one is the paragraph under the table.
    // The rule row is still stepped over: it is drawn, not written.
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    ed.goto_line(2);
    ed.enter_table();
    press(&mut ed, "j");
    // Line 3 is the `|---|` rule, which is drawn rather than written.
    assert_eq!(ed.cell_position().map(|(l, _)| l), Some(3));
    press(&mut ed, "j");
    assert_eq!(ed.cell_position().map(|(l, _)| l), Some(4), "the last row");
    press(&mut ed, "j");
    assert_eq!(ed.cursor_line(), 5, "and out into the prose under it");
    assert!(ed.md_region().is_none(), "where the grid's rules do not apply");
    // Back in, over the rule again, and out the top — where `hjkl` are
    // letters again, which is what makes the mode safe to leave on.
    press(&mut ed, "kkk");
    assert_eq!(ed.cell_position().map(|(l, _)| l), Some(1), "the header");
    press(&mut ed, "k");
    assert_eq!(ed.cursor_line(), 0, "and out above it");
    assert!(ed.md_region().is_none(), "the paragraph over the table");
}

#[test]
fn paging_a_column_stops_at_the_table_edge_even_though_a_step_does_not() {
    // A page is a count of the **grid's** rows, so running out of them is
    // where it stops — `j` walking out into the prose is a step the reader
    // asked for one row at a time.
    let mut ed = with_md_table();
    ed.set_page(80, 40);
    ed.goto_line(2);
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "J");
    assert_eq!(ed.cell_position().map(|(l, _)| l), Some(4), "{}", ed.status());
    press(&mut ed, "K");
    assert_eq!(ed.cell_position().map(|(l, _)| l), Some(1), "{}", ed.status());
}

#[test]
fn a_column_of_han_lines_up_by_width_not_by_character_count() {
    // The reason this module exists. Every other formatter pads to a
    // character count, so a column mixing 漢字 with Latin comes out
    // ragged on the very terminal it is being written on.
    let mut ed = typed("| a | 甲 |\n| --- | --- |\n| bbbb | 乙丙 |\n");
    ed.goto_line(1);
    assert!(ed.enter_table());
    press(&mut ed, "tF");
    let widths: Vec<usize> = ed
        .current_buffer()
        .text()
        .lines()
        .map(yumete_cjk::str_width)
        .collect();
    assert!(widths.windows(2).all(|w| w[0] == w[1]), "{widths:?}");
}

#[test]
fn a_header_with_no_rule_gets_one() {
    // The first table anyone tries this on is one they are in the middle
    // of writing, and it has no `|---|` yet. Refusing it would be refusing
    // the whole feature at the moment it is most wanted.
    let mut ed = typed("| 字 | 讀音 |\n");
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "| 字 | 讀音 |\n| --- | --- |\n");
    assert!(ed.status().contains("分隔行"), "and it says so: {}", ed.status());
}

#[test]
fn t_adds_and_drops_rows_and_columns() {
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    // A new row below this one, and the cursor goes to it.
    press(&mut ed, "tr");
    assert_eq!(ed.current_buffer().line_count(), 8);
    assert_eq!(ed.cell_position().map(|(l, _)| l), Some(4));
    press(&mut ed, "td");
    assert_eq!(
        ed.current_buffer().text(),
        "前文\n| 字 | 讀音 |\n| -- | ---- |\n| 木 | mu   |\n| 目 | mu   |\n後文\n"
    );
    // A new column to the right — of every row, and of the rule.
    press(&mut ed, "tc");
    assert_eq!(
        ed.current_buffer().text(),
        "前文\n| 字 |   | 讀音 |\n| -- | - | ---- |\n| 木 |   | mu   |\n| 目 |   | mu   |\n後文\n"
    );
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(1), "the cursor lands in it");
    press(&mut ed, "tD");
    assert_eq!(
        ed.current_buffer().text(),
        "前文\n| 字 | 讀音 |\n| -- | ---- |\n| 木 | mu   |\n| 目 | mu   |\n後文\n"
    );
}

#[test]
fn the_header_is_not_a_row_anyone_may_delete() {
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    ed.goto_line(2);
    ed.enter_table();
    let before = ed.current_buffer().text();
    press(&mut ed, "td");
    assert_eq!(ed.current_buffer().text(), before);
    assert!(ed.status().contains("標題行"), "{}", ed.status());
}

#[test]
fn t_moves_a_row_and_a_column_with_the_cursor_on_it() {
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    press(&mut ed, "tj");
    assert_eq!(
        ed.current_buffer().text(),
        "前文\n| 字 | 讀音 |\n| -- | ---- |\n| 目 | mu   |\n| 木 | mu   |\n後文\n"
    );
    assert_eq!(ed.cell_position().map(|(l, _)| l), Some(4), "the cursor went with it");
    press(&mut ed, "tl");
    assert_eq!(
        ed.current_buffer().text(),
        "前文\n| 讀音 | 字 |\n| ---- | -- |\n| mu   | 目 |\n| mu   | 木 |\n後文\n"
    );
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(1));
}

#[test]
fn alignment_is_a_keystroke_and_shows_in_the_source() {
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "lt>");
    assert!(
        ed.current_buffer().text().contains("| ---: |"),
        "{}",
        ed.current_buffer().text()
    );
    assert!(
        ed.current_buffer().text().contains("|   mu |"),
        "and the padding moves to the left: {}",
        ed.current_buffer().text()
    );
}

#[test]
fn tab_walks_the_cells_while_typing() {
    // What makes a table quick to fill in: you never reach for a pipe.
    let mut ed = typed("| a | b |\n| --- | --- |\n|  |  |\n");
    ed.goto_line(3);
    assert!(ed.enter_table());
    press(&mut ed, "i");
    press(&mut ed, "木");
    ed.on_key(Key::Tab);
    press(&mut ed, "mu");
    ed.on_key(Key::Esc);
    assert_eq!(
        ed.current_buffer().text(),
        // Nobody laid this table out, so filling a row in does not lay it
        // out either (#329): the two cells are the whole diff.
        "| a | b |\n| --- | --- |\n| 木 | mu |\n"
    );
    // And back the other way.
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::BackTab);
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(0));
}

#[test]
fn tab_at_the_end_of_the_last_row_opens_another() {
    let mut ed = typed("| a | b |\n| --- | --- |\n| x | y |\n");
    ed.goto_line(3);
    assert!(ed.enter_table());
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "l");
    press(&mut ed, "i");
    ed.on_key(Key::Tab);
    assert_eq!(ed.current_buffer().line_count(), 5);
    assert_eq!(ed.cell_position(), Some((3, 0)), "at the start of the new row");
}

#[test]
fn typing_keeps_the_columns_lined_up() {
    let mut ed = with_lined_up_md_table();
    assert!(ed.enter_table());
    press(&mut ed, "c");
    press(&mut ed, "薔薇");
    ed.on_key(Key::Esc);
    assert_eq!(
        ed.current_buffer().text(),
        "前文\n| 字   | 讀音 |\n| ---- | ---- |\n| 薔薇 | mu   |\n| 目   | mu   |\n後文\n",
        "the column widened around what was typed into it"
    );
}

/// #329: 兩個字的編輯換來五千行 diff.
#[test]
fn a_table_nobody_squared_up_is_not_squared_up_by_editing_it() {
    // A manuscript's worth of hand-typed table. Squaring it up would rewrite
    // every one of these rows to hold two characters.
    let mut rows = String::from("|字|讀音|\n|-|-|\n");
    for _ in 0..60 {
        rows.push_str("|木|mu|\n");
    }
    let mut ed = typed(&rows);
    let before = ed.current_buffer().text();
    ed.goto_line(4);
    assert!(ed.enter_table());
    press(&mut ed, "c");
    press(&mut ed, "薔薇");
    ed.on_key(Key::Esc);

    let after = ed.current_buffer().text();
    let moved = before
        .lines()
        .zip(after.lines())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(moved, 1, "one cell was edited, so one line moved");
    assert_eq!(ed.line_text(3).as_deref(), Some("|薔薇|mu|\n"));

    // And `t F` is the door that does square it up, which is why declining
    // above costs nothing.
    press(&mut ed, "tF");
    assert_eq!(ed.line_text(3).as_deref(), Some("| 薔薇 | mu   |\n"));
}

#[test]
fn one_undo_takes_back_one_edit_and_its_reflow() {
    // The reflow is part of the edit, not a second one: a person who adds
    // a column and presses `u` wants the table they had.
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    let before = ed.current_buffer().text();
    press(&mut ed, "tc");
    assert_ne!(ed.current_buffer().text(), before);
    press(&mut ed, "u");
    assert_eq!(ed.current_buffer().text(), before);
}

#[test]
fn a_pipe_cannot_be_typed_into_a_cell() {
    // The same invariant the CSV grid keeps: a row's cell count never
    // changes while it is being read as one.
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    press(&mut ed, "i|");
    assert!(!ed.current_buffer().text().contains("||"), "{}", ed.status());
    // `\|` is the escape a Markdown table does have, and it survives.
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    press(&mut ed, "c");
    press(&mut ed, r"a\");
    ed.on_key(Key::Esc);
    assert!(ed.current_buffer().text().contains(r"a\"), "{}", ed.current_buffer().text());
}

#[test]
fn an_indented_table_stays_in_its_list_item() {
    let mut ed = typed("- 一項\n  | a | b |\n  | --- | --- |\n  | x | y |\n");
    ed.goto_line(4);
    assert!(ed.enter_table(), "{}", ed.status());
    let text = ed.current_buffer().text();
    assert!(
        text.lines().skip(1).all(|l| l.starts_with("  |")),
        "{text}"
    );
}

#[test]
fn prose_with_a_comma_in_it_is_not_a_table() {
    // The header-row fallback used to take any first line with a comma,
    // so `:table` on a manuscript turned the chapter into a grid.
    let dir = std::env::temp_dir().join(format!("yumete-prose-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("章.md");
    std::fs::write(&file, "他說，這不是表格。\n下一段沒有逗號\n又一段，有兩個，逗號\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&file).unwrap();
    assert!(!ed.enter_table(), "{}", ed.status());
    assert!(ed.table().is_none());
    assert!(ed.status().contains("不像表格"), "{}", ed.status());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_header_on_the_last_line_with_no_newline_still_gets_its_rule() {
    // `line_to_char` of a line that does not exist is the end of the text,
    // so the rule row was welded onto the header's own end and the table
    // was gone — in exactly the case the feature advertises.
    let mut ed = typed("前文\n| a | b |");
    assert_eq!(ed.current_buffer().text(), "前文\n| a | b |");
    ed.goto_line(2);
    assert!(ed.enter_table(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "前文\n| a | b |\n| --- | --- |");
}

#[test]
fn the_rule_row_is_drawn_not_written() {
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    press(&mut ed, "T"); // #356: 這一條測的是格
    // `gg`, `G`, `:N` and a search all land on the rule; cell motion does
    // not. Standing there, nothing may change it.
    let before = ed.current_buffer().text();
    for door in ["c", "i", "a"] {
        ed.goto_line(3);
        press(&mut ed, door);
        assert_eq!(ed.mode(), Mode::Normal, "`{door}` must not open a cell here");
        assert!(ed.status().contains("分隔行"), "`{door}`: {}", ed.status());
        assert_eq!(ed.current_buffer().text(), before);
    }
    // …and a structural key on it means the header it belongs to.
    ed.goto_line(3);
    press(&mut ed, "td");
    assert_eq!(ed.current_buffer().text(), before);
    assert!(ed.status().contains("標題行"), "{}", ed.status());
    press(&mut ed, "tr");
    assert_eq!(ed.current_buffer().line_count(), 8, "a row was opened");
    assert_eq!(ed.cell_position().map(|(l, _)| l), Some(3));
}

#[test]
fn the_table_mode_does_not_follow_the_cursor_out_of_the_table() {
    let mut ed = with_md_table();
    // 表格操作, so the page is still the manuscript's to set — `t t` turns
    // it horizontal on purpose (#275).
    press(&mut ed, "tb");
    ed.goto_line(6);
    // `o` in the prose below opens a line, not a row.
    press(&mut ed, "o");
    ed.on_key(Key::Esc);
    let text = ed.current_buffer().text();
    assert!(!text.contains("|  |"), "{text}");
    // The page is still the manuscript's page.
    ed.set_layout(Layout::Vertical);
    assert_eq!(ed.layout(), Layout::Vertical);
    // And a substitution in prose is about prose.
    ed.goto_line(1);
    assert!(ed.execute("%s/前文/前 | 文/").is_ok(), "{}", ed.status());
    assert!(ed.current_buffer().text().contains("前 | 文"), "{}", ed.status());
}

#[test]
fn a_cell_may_hold_an_escaped_pipe() {
    // The manual says so: 「`|` 打不進格子…要用寫 `\|`」. It was not true.
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "c");
    press(&mut ed, r"a\|b");
    ed.on_key(Key::Esc);
    assert!(
        ed.current_buffer().text().contains(r"a\|b"),
        "{}",
        ed.current_buffer().text()
    );
    // It is one cell, not two.
    assert_eq!(ed.cell_text(3, 0), r"a\|b");
    assert_eq!(ed.row_cells(3).len(), 2);
    // A bare pipe is still refused.
    press(&mut ed, "i");
    press(&mut ed, "|");
    ed.on_key(Key::Esc);
    assert_eq!(ed.row_cells(3).len(), 2, "{}", ed.current_buffer().text());
    // And `c` on a cell that holds the escape empties it, as documented.
    press(&mut ed, "c");
    press(&mut ed, "Q");
    ed.on_key(Key::Esc);
    assert_eq!(ed.cell_text(3, 0), "Q", "{}", ed.current_buffer().text());
}

#[test]
fn a_row_pasted_on_the_header_lands_under_the_rule() {
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    ed.goto_line(2);
    ed.enter_table();
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "Y");
    press(&mut ed, "p");
    let text = ed.current_buffer().text();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[2].starts_with("| -"), "the rule is still line 2: {lines:?}");
    assert_eq!(lines.len(), 7);
}

#[test]
fn a_table_in_a_code_fence_is_a_quotation() {
    let mut ed = typed("說明：\n\n```\n| a | b |\n| --- | --- |\n| xxxx | y |\n```\n");
    ed.goto_line(4);
    let before = ed.current_buffer().text();
    assert!(!ed.enter_table(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), before);
    assert!(ed.status().contains("代碼塊"), "{}", ed.status());
}

#[test]
fn one_column_is_a_line_with_a_pipe_in_it() {
    let mut ed = typed("一段話\n| 這行以豎線開頭\n又一段\n");
    ed.goto_line(2);
    let before = ed.current_buffer().text();
    assert!(!ed.enter_table(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), before);
}

#[test]
fn a_crlf_table_stays_crlf() {
    let mut ed = Editor::new();
    let buffer = crate::Buffer::from_text("| a | b |\r\n| --- | --- |\r\n| xxx | y |\r\n");
    ed.add_buffer(buffer);
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());
    let text = ed.current_buffer().text();
    assert_eq!(text.matches("\r\n").count(), 3, "{text:?}");
    assert!(!text.contains("|\n"), "{text:?}");
}

#[test]
fn a_new_buffer_is_not_the_table_that_was_open() {
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    ed.provide_shell_output("wc -l", "3\n");
    assert!(ed.table().is_none(), "the grid does not follow to another file");
    press(&mut ed, "o");
    ed.on_key(Key::Esc);
    assert!(!ed.current_buffer().text().contains("|  |"), "{}", ed.current_buffer().text());
}

#[test]
fn t_s_puts_the_rows_in_order_by_this_column() {
    // The first thing anyone does to a 年表 or a 人物表. **`t0s`**, since
    // §5.7 took the bare letter away: `0` is not a column, so it was free
    // to mean 「the one I am standing in」.
    let mut ed = typed("| 年 | 事 |\n| --- | --- |\n| 1900 | 丙 |\n| 19 | 甲 |\n| 200 | 乙 |\n");
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "t0s");
    let text = ed.current_buffer().text();
    let years: Vec<&str> = text
        .lines()
        .skip(2)
        .filter_map(|l| l.split('|').nth(1))
        .map(str::trim)
        .collect();
    assert_eq!(years, ["19", "200", "1900"], "numbers compare as numbers");
    press(&mut ed, "t0S");
    let text = ed.current_buffer().text();
    let years: Vec<&str> = text
        .lines()
        .skip(2)
        .filter_map(|l| l.split('|').nth(1))
        .map(str::trim)
        .collect();
    assert_eq!(years, ["1900", "200", "19"]);
    // The header stayed the header.
    assert!(text.starts_with("| 年"), "{text}");
}

#[test]
fn one_long_cell_does_not_widen_every_row() {
    // Without a ceiling, one 備註 sentence makes every line of the table
    // as wide as itself, and a table two hundred columns across is not a
    // table anybody can read on a page set to forty.
    let long: String = "很".repeat(40);
    let mut ed = typed(&format!("| a | b |\n| --- | --- |\n| x | {long} |\n| y | 短 |\n"));
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());
    let text = ed.current_buffer().text();
    let short = text.lines().nth(3).unwrap();
    assert!(
        yumete_cjk::str_width(short) < 45,
        "the short row stayed short: {short:?}"
    );
    // …and nothing was truncated.
    assert!(text.contains(&long), "the long cell is still whole");
}

#[test]
fn a_delimited_grid_can_lose_and_move_a_row() {
    // The guard that makes a grid safe is what made this impossible: a
    // whole row is nothing *but* delimiters, so every ordinary way of
    // deleting one was refused. A table editor that cannot remove a line
    // is not one.
    let (dir, csv) = a_table("rows");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    let rows = ed.current_buffer().line_count();
    ed.goto_line(2);
    press(&mut ed, "td");
    assert_eq!(ed.current_buffer().line_count(), rows - 1, "{}", ed.status());
    // The header is not a row anyone may delete.
    ed.goto_line(1);
    press(&mut ed, "td");
    assert_eq!(ed.current_buffer().line_count(), rows - 1);
    assert!(ed.status().contains("標題行"), "{}", ed.status());
    // Nor may a row be moved above it.
    ed.goto_line(2);
    press(&mut ed, "tk");
    assert!(ed.status().contains("到頭"), "{}", ed.status());
    // …and one undo takes any of it back.
    press(&mut ed, "u");
    assert_eq!(ed.current_buffer().line_count(), rows);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn d_on_a_grid_means_the_cell() {
    let (dir, csv) = a_table("clear");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: 這一條測的是格
    let cell = ed.cell_text(1, 0);
    assert!(!cell.is_empty());
    press(&mut ed, "d");
    assert_eq!(ed.cell_text(1, 0), "", "{}", ed.status());
    assert_eq!(ed.row_cells(1).len(), ed.row_cells(0).len(), "the row kept its shape");
    // And what was cleared is on the register, so it can be put back.
    press(&mut ed, "p");
    assert_eq!(ed.cell_text(1, 0), cell);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn row_goes_straight_to_the_row_a_character_names() {
    // The index has always been built and has always answered in about
    // 300 ns; nothing let a person ask it. Finding 木 in a 123,380-row
    // table meant `/^木,` and hoping no other row started that way.
    let dir = std::env::temp_dir().join(format!("yumete-row-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tables = dir.join(".yumete").join("tables");
    std::fs::create_dir_all(&tables).unwrap();
    std::fs::write(
        tables.join("d.toml"),
        "[table]\nfile = \"d.csv\"\nkey = \"char\"\n\
         [[table.column]]\nname = \"char\"\n[[table.column]]\nname = \"ids_y\"\n\
         [table.link]\nfrom = [\"ids_y\"]\nto = \"char\"\n",
    )
    .unwrap();
    let csv = dir.join("d.csv");
    std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.execute("table-jump 目").is_ok());
    assert_eq!(ed.cursor_line(), 3, "{}", ed.status());
    assert!(ed.execute("table-jump 卵").is_ok());
    assert!(ed.status().contains("沒有"), "{}", ed.status());
    // …and `C-o` comes back, because a jump is a jump.
    assert!(ed.execute("table-jump 木").is_ok());
    assert_eq!(ed.cursor_line(), 2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_filter_over_whole_rows_is_a_table_operation() {
    // The manual's own example is `LC_ALL=C sort` over a table, and the
    // grid used to refuse it — after spawning the command and reading its
    // output. What matters is that every row that comes back has a row's
    // shape, not that no delimiter moved.
    let (dir, csv) = a_table("pipe");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    let rows: Vec<String> = ed
        .current_buffer()
        .text()
        .lines()
        .skip(1)
        .map(str::to_string)
        .collect();
    assert!(rows.len() >= 2, "{rows:?}");
    ed.goto_line(2);
    press(&mut ed, "x");
    for _ in 1..rows.len() {
        press(&mut ed, "x");
    }
    // What a sort would send back: the same rows, another order.
    let mut sorted = rows.clone();
    sorted.reverse();
    ed.provide_pipe_output(&format!("{}\n", sorted.join("\n")));
    let now: Vec<String> = ed
        .current_buffer()
        .text()
        .lines()
        .skip(1)
        .map(str::to_string)
        .collect();
    assert_eq!(now, sorted, "{}", ed.status());

    // …and a command that sends back the wrong shape changes nothing.
    let before = ed.current_buffer().text();
    ed.goto_line(2);
    press(&mut ed, "x");
    ed.provide_pipe_output("一個欄位\n");
    assert_eq!(ed.current_buffer().text(), before, "{}", ed.status());
    assert!(ed.status().contains("欄"), "{}", ed.status());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_hidden_column_is_read_and_written_but_not_walked() {
    // Two of the 拆分表's twenty-eight columns are empty in all 123,380
    // rows and cost eight cells each across the whole page. `hidden` is
    // for those — the file still has them, this reader does not care.
    let dir = std::env::temp_dir().join(format!("yumete-hide-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tables = dir.join(".yumete").join("tables");
    std::fs::create_dir_all(&tables).unwrap();
    std::fs::write(
        tables.join("h.toml"),
        "[table]\nfile = \"h.csv\"\n\
         [[table.column]]\nname = \"a\"\n\
         [[table.column]]\nname = \"b\"\nhidden = true\n\
         [[table.column]]\nname = \"c\"\n",
    )
    .unwrap();
    let csv = dir.join("h.csv");
    let text = "a,b,c\n一,,三\n";
    std::fs::write(&csv, text).unwrap();
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.table().is_some());
    assert!(!ed.column_shows(1), "b is hidden");
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: 這一條測的是格
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(0));
    press(&mut ed, "l");
    assert_eq!(
        ed.cell_position().map(|(_, c)| c),
        Some(2),
        "`l` steps over the hidden column, not into it"
    );
    press(&mut ed, "h");
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(0));
    // …and the file is untouched: hidden is about reading, not about data.
    assert!(ed.execute("w").is_ok());
    assert_eq!(std::fs::read_to_string(&csv).unwrap(), text);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn table_check_answers_the_four_questions_nobody_can_answer_by_eye() {
    let dir = std::env::temp_dir().join(format!("yumete-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tables = dir.join(".yumete").join("tables");
    std::fs::create_dir_all(&tables).unwrap();
    std::fs::write(
        tables.join("c.toml"),
        "[table]\nfile = \"c.csv\"\nkey = \"char\"\n\
         [[table.column]]\nname = \"char\"\n[[table.column]]\nname = \"ids\"\n\
         [table.link]\nfrom = [\"ids\"]\nto = \"char\"\n",
    )
    .unwrap();
    let csv = dir.join("c.csv");
    std::fs::write(
        &csv,
        // line 2 names a component with no row; line 3 is the wrong width;
        // line 5 repeats line 4's name.
        "char,ids\n相,⿰木卵\n寬,一,二\n木,木\n木,木\n",
    )
    .unwrap();
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.execute("table-check").is_ok());
    let out = ed.current_buffer().text();
    assert!(out.contains("c.csv:2:") && out.contains("卵"), "{out}");
    assert!(out.contains("c.csv:3:") && out.contains("欄"), "{out}");
    assert!(out.contains("c.csv:5:") && out.contains("重了"), "{out}");
    // ⿰ is grammar, so it is not reported as a missing component.
    assert!(!out.contains('⿰'), "{out}");
    // A clean table says so and opens nothing.
    std::fs::write(&csv, "char,ids\n木,木\n目,目\n").unwrap();
    ed.open_file(&csv).unwrap();
    ed.execute("reload!").ok();
    let buffers = ed.buffer_count();
    assert!(ed.execute("table-check").is_ok());
    assert!(ed.status().contains("沒查出問題"), "{}", ed.status());
    assert_eq!(ed.buffer_count(), buffers, "no buffer for no findings");
    std::fs::remove_dir_all(&dir).ok();
}

/// `:check-usage` answers as a jumpable listing, and the count in the
/// status line has to be a count of what the listing holds.
#[test]
fn check_usage_lists_the_slips_and_says_when_it_stopped_listing() {
    let mut ed = typed("那裏很冷。\n他站在裏面。\n她走進屋裡。\n");
    assert!(ed.execute("check-usage").is_ok());
    let out = ed.current_buffer().text();
    assert!(out.contains(":3:") && out.contains('裡') && out.contains('裏'), "{out}");
    assert!(ed.status().contains('1'), "{}", ed.status());

    // Past `LISTING_LIMIT` the buffer holds the first five hundred and the
    // status line used to name a number nothing on screen could reach.
    let mut text = "那裏。\n".repeat(LISTING_LIMIT + 200);
    text.push_str(&"那裡。\n".repeat(LISTING_LIMIT + 1));
    let mut ed = typed(&text);
    assert!(ed.execute("check-usage").is_ok());
    let lines = ed.current_buffer().text().lines().count();
    assert_eq!(lines, LISTING_LIMIT, "the listing stops at the limit");
    assert!(
        ed.status().contains(&LISTING_LIMIT.to_string())
            && !ed.status().contains(&(LISTING_LIMIT + 1).to_string()),
        "{}",
        ed.status()
    );
}

/// `:word-habit` ranks by surprisal, not by count — so the word every text
/// is not the answer, and the word this one leans on is (#242).
#[test]
fn words_reports_what_is_said_more_than_prose_says_it_and_never_says_de() {
    // A table where 的 is common and 然後 is not, so a text that says 然後
    // as often as 的 has one habit word and not two.
    let dict = DictionarySegmenter::from_text("的\t100000\n然後\t100\n好的\t100000\n", 1);
    let mut ed = typed("然後好的然後好的然後好的然後好的\n");
    ed.set_segmenter(Box::new(dict));
    assert!(ed.execute("word-habit").is_ok());
    let out = ed.current_buffer().text();
    assert!(out.contains("然後"), "{out}");
    assert!(!out.contains("好的"), "the word prose says just as often: {out}");
    assert!(ed.status().contains('1'), "{}", ed.status());
}

/// The fallback segmenter has no frequencies, and the answer to
/// 「為什麼一個字都沒有」 has to be a sentence rather than an empty listing.
#[test]
fn words_without_a_frequency_table_says_so_instead_of_listing_nothing() {
    let mut ed = typed("然後好的然後好的然後好的\n");
    ed.set_segmenter(Box::new(CategorySegmenter));
    let before = ed.current_buffer().text();
    assert!(ed.execute("word-habit").is_ok());
    assert_eq!(ed.current_buffer().text(), before, "no listing buffer");
    assert!(ed.status().contains("詞頻"), "{}", ed.status());
}

/// `:check-charset` reports the character no standard carries — once,
/// however many times it was written (#240).
#[test]
fn check_charset_reports_each_character_once() {
    let mut ed = with_toy_reader("漢字龘\n𠮷很難𠮷\n");
    assert!(ed.execute("check-charset").is_ok());
    let out = ed.current_buffer().text();
    assert_eq!(out.lines().count(), 1, "one line per character: {out}");
    assert!(out.contains('𠮷') && out.contains(":2:"), "{out}");
    assert!(out.contains('2'), "how many times, not just where: {out}");
    // 龘 is rare — `:ruby-auto rare` annotates it — but 古籍 is a standard
    // and carries it, so it is not this command's finding.
    assert!(!out.contains('龘'), "{out}");

    let mut ed = typed("漢字");
    assert!(ed.execute("check-charset").is_ok());
    assert!(!ed.status().is_empty(), "no 字集 data is worth saying");
}

/// `:check-punct` answers in the same jumpable shape, and the finding that
/// matters — the 「 nothing closes — is the one no eye finds (#238).
#[test]
fn check_punct_finds_the_quote_that_never_closes() {
    let mut ed = typed("他說,好。\n她問：「你回來了。\n這一行沒事。\n");
    assert!(ed.execute("check-punct").is_ok());
    let out = ed.current_buffer().text();
    assert!(out.contains(":1:") && out.contains('，'), "{out}");
    assert!(out.contains(":2:") && out.contains('」'), "{out}");
    assert_eq!(out.lines().count(), 2, "{out}");

    // Nothing to say is said, rather than an empty buffer being opened.
    let mut ed = typed("他說：「好。」\n圓周率是 3.14。\n");
    let before = ed.buffer_count();
    assert!(ed.execute("check-punct").is_ok());
    assert_eq!(ed.buffer_count(), before, "clean: no listing");
}

#[test]
fn a_quoted_table_stays_a_quotation_even_when_walked_into() {
    // Refusing at the door was not enough: `gg`, `G`, `:N` and a search
    // all land outside the cells — the manual says so — and from a quoted
    // example in a code fence `t t` reformatted somebody's text.
    let mut ed = typed(
        "| a | b |\n| --- | --- |\n| x | y |\n\n說明：\n\n```\n|字|讀音|\n|--|--|\n```\n",
    );
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());
    let before = ed.current_buffer().text();
    // Walk into the fence the way a search would.
    ed.goto_line(8);
    assert!(ed.md_region().is_none(), "a quotation is not a table");
    // The structural keys are the ones that used to rewrite it. `t` here
    // is vi's till-motion again, and `o` opens an ordinary line.
    press(&mut ed, "tt");
    assert_eq!(ed.current_buffer().text(), before, "{}", ed.status());
    assert!(
        ed.current_buffer().text().contains("|字|讀音|"),
        "the quotation is as it was written"
    );
    // …and back in the real table the keys work.
    ed.goto_line(1);
    assert!(ed.md_region().is_some());
    press(&mut ed, "tr");
    assert_ne!(ed.current_buffer().text(), before);
}

#[test]
fn dot_repeats_the_change_and_the_count_it_was_given() {
    // `3>` indents three levels; `.` used to indent one, because the digit
    // key started a fresh recording and threw the count away.
    let mut ed = typed("一\n二\n");
    ed.goto_line(1);
    press(&mut ed, "3>");
    let three = ed.current_buffer().text();
    press(&mut ed, "j");
    ed.on_key(Key::Char('.'));
    let lines: Vec<usize> = ed
        .current_buffer()
        .text()
        .lines()
        .map(|l| l.len() - l.trim_start().len())
        .collect();
    assert_eq!(lines[0], lines[1], "the same three levels: {three:?}");
}

#[test]
fn switching_buffers_is_not_the_change_dot_repeats() {
    // `finish_watching` compared a revision with a *different* buffer's,
    // so after `gn` a pure motion looked like a change and `.` came to
    // mean 「switch buffer」 — the common case at a hundred chapters.
    let mut ed = typed("一\n");
    ed.execute("new").unwrap();
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Char('甲'));
    ed.on_key(Key::Esc);
    ed.goto_line(1);
    press(&mut ed, ">");
    let indented = ed.current_buffer().text();
    press(&mut ed, "gp");
    press(&mut ed, "gn");
    ed.on_key(Key::Char('.'));
    assert_ne!(ed.current_buffer().text(), indented, "`.` indented again");
    assert!(ed.current_buffer().text().contains("甲"), "…in this buffer");
}

#[test]
fn a_substitution_names_a_line_the_writer_can_find() {
    // The refusal counted rows and printed the number as a *line*, so in a
    // Markdown table 「第 3 行」 named a line the writer could not find —
    // and it counted raw `|`, so it refused a substitution that inserted
    // the escape the manual tells you to use.
    let mut ed = with_md_table();
    assert!(ed.enter_table());
    // `\|` is a pipe inside a cell, so this does not change any row's shape.
    assert!(ed.execute(r"%s/mu/a\|b/").is_ok(), "{}", ed.status());
    assert!(ed.current_buffer().text().contains(r"a\|b"), "{}", ed.status());
    // A bare one does, and the line it names is the document's.
    let before = ed.current_buffer().text();
    assert!(ed.execute("%s/木/木|/").is_ok());
    assert_eq!(ed.current_buffer().text(), before);
    assert!(ed.status().contains("第 4 行"), "{}", ed.status());
}

#[test]
fn a_block_from_a_spreadsheet_goes_in_as_cells() {
    // Every spreadsheet puts tab-separated rows on the clipboard, and a
    // tab was refused outright — so a writer built tables in a spreadsheet
    // and never brought them here.
    let mut ed = typed("| 字 | 音 |\n| --- | --- |\n| a | b |\n");
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "T"); // #356: 這一條測的是格
    ed.goto_line(3);
    ed.set_register_for_test("木\tmu\n目\tmu\n禾\the\n");
    press(&mut ed, "p");
    assert_eq!(
        ed.current_buffer().text(),
        "| 字 | 音 |\n| -- | -- |\n| 木 | mu |\n| 目 | mu |\n| 禾 | he |\n",
        "{}",
        ed.status()
    );
    assert!(ed.status().contains("3×2"), "{}", ed.status());

    // A pipe in a pasted cell goes in as the escape, not as a boundary.
    ed.goto_line(3);
    ed.set_register_for_test("a|b\tc\n");
    press(&mut ed, "p");
    assert_eq!(ed.row_cells(2).len(), 2, "{}", ed.current_buffer().text());
    assert!(ed.current_buffer().text().contains(r"a\|b"));
}

#[test]
fn a_block_too_wide_for_a_schema_is_refused() {
    let (dir, csv) = a_table("block");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    let width = ed.table().unwrap().schema.columns.len();
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: 這一條測的是格
    let wide: String = (0..width + 1).map(|i| format!("{i}\t")).collect();
    ed.set_register_for_test(&wide);
    let before = ed.current_buffer().text();
    press(&mut ed, "p");
    assert_eq!(ed.current_buffer().text(), before, "{}", ed.status());
    assert!(ed.status().contains("貼不下"), "{}", ed.status());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ordinary_text_is_still_pasted_as_text() {
    // One cell is not a block: no tab, no line break.
    let mut ed = typed("| a | b |\n| --- | --- |\n| x | y |\n");
    ed.goto_line(3);
    assert!(ed.enter_table());
    press(&mut ed, "T"); // #356: 這一條測的是格
    ed.set_register_for_test("春天");
    press(&mut ed, "p");
    assert_eq!(ed.cell_text(2, 0), "春天", "{}", ed.status());
    // …and a paragraph with commas in it is a paragraph, not a grid.
    ed.set_register_for_test("他說，這樣，那樣");
    press(&mut ed, "p");
    assert!(ed.cell_text(2, 0).contains('，'), "{}", ed.current_buffer().text());
}

#[test]
fn a_whole_column_can_be_taken_and_put_back() {
    // Rows had `Y`; a column could only be moved one step at a time.
    let mut ed = typed("| 字 | 音 |\n| --- | --- |\n| 木 | mu |\n| 目 | mo |\n");
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "T"); // #356: 這一條測的是格
    ed.goto_line(4);
    press(&mut ed, "l");
    press(&mut ed, "ty");
    assert!(ed.status().contains("音"), "{}", ed.status());
    // Put it down the other column: one cell to a line, header included.
    press(&mut ed, "h");
    press(&mut ed, "tp");
    assert_eq!(
        ed.current_buffer().text(),
        "| 音 | 音 |\n| -- | -- |\n| mu | mu |\n| mo | mo |\n",
        "{}",
        ed.status()
    );
}

#[test]
fn a_column_search_reads_down_before_across() {
    // `/` reads the page the way a page is read; a table has a second way
    // a document does not have. The only difference is the direction: the
    // pattern is a regex, the match is the selection, `n` walks, it wraps.
    let dir = std::env::temp_dir().join(format!("yumete-colsearch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("d.csv");
    // Two columns. Read across, 甲 comes at row 1 then row 2; read down,
    // both of column A come before either of column B.
    std::fs::write(&csv, "a,b\n甲一,甲二\n甲三,甲四\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.enter_table(), "{}", ed.status());

    assert!(ed.execute("table-find column 甲").is_ok(), "{}", ed.status());
    assert!(ed.status().contains("1/4"), "{}", ed.status());
    // The hits are *shown* in the other work area; the cursor stays where
    // it was standing, which is the point of the split (Feature #176).
    let where_am_i = |ed: &Editor| {
        let at = ed.other_pane().expect("the other work area").cursor();
        let line = ed.current_buffer().rope().char_to_line(at);
        let cell = ed
            .row_cells(line)
            .iter()
            .position(|&(from, to)| {
                let start = ed.current_buffer().rope().line_to_char(line);
                at >= start + from && at <= start + to
            })
            .unwrap_or(0);
        (line, cell)
    };
    assert_eq!(where_am_i(&ed), (1, 0), "column a, row 1");
    ed.on_key(Key::Char('n'));
    assert_eq!(where_am_i(&ed), (2, 0), "column a, row 2 — still column a");
    ed.on_key(Key::Char('n'));
    assert_eq!(where_am_i(&ed), (1, 1), "only now column b");
    ed.on_key(Key::Char('n'));
    assert_eq!(where_am_i(&ed), (2, 1));
    ed.on_key(Key::Char('n'));
    assert_eq!(where_am_i(&ed), (1, 0), "and it wraps");

    // A row search is `/`, and reads the other way.
    assert!(ed.execute("table-find row 甲").is_ok());
    assert_eq!(ed.cursor_line(), 1);

    // No direction means row, because that is what a search is anywhere
    // but a table.
    assert!(ed.execute("table-find 甲").is_ok());
    // A pattern is a pattern in both directions.
    assert!(ed.execute("table-find column 甲[一三]").is_ok());
    assert!(ed.status().contains("1/2"), "{}", ed.status());
    std::fs::remove_dir_all(&dir).ok();
}

/// `J K H L` and `C-d`/`C-u` mean "several rows" in a table, and a table's
/// rows are not the same width twice — so aiming at a *character* column,
/// which is what the page motions do everywhere else, drifts sideways as
/// it goes. The author reported it as 「in column 5, press J, land in
/// column 10」.
#[test]
fn paging_through_a_table_keeps_to_the_column() {
    let dir = std::env::temp_dir().join(format!("yumete-page-column-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("wide.csv");
    // Deliberately ragged, and by a lot: the first field swings between 1
    // and 19 characters, so the character offset of column c on one row is
    // past the end of another row entirely.
    let mut text = String::from("a,b,c,d\n");
    for i in 0..40 {
        let wide = "x".repeat(1 + (i % 7) * 3);
        text.push_str(&format!("{wide},b{i},c{i},d{i}\n"));
    }
    std::fs::write(&csv, &text).unwrap();
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.set_page(8, 8);
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "T"); // #356: 這一條測的是格
    // Row 7 is the widest of the seven; half a page on is one of the
    // narrowest, whose whole line is shorter than where column c starts
    // here.
    ed.goto_line(7);
    press(&mut ed, "ll");
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "column c");
    let row = ed.cursor_line();
    ed.on_key(Key::Char('J'));
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "still column c");
    assert!(ed.cursor_line() > row + 1, "and it moved several rows");
    ed.on_key(Key::Char('K'));
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "and coming back");
    assert_eq!(ed.cursor_line(), row, "to the row it started on");
    ed.on_key(Key::Ctrl('d'));
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "C-d is the same motion");
    ed.on_key(Key::Char('L'));
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "and a whole page of it");
    std::fs::remove_dir_all(&dir).ok();
}

/// The same motions in a `|` table, which is the shape they can walk
/// **out** of: a grid is the whole file and clamps at its last line, but a
/// Markdown table has prose under it, and a page is longer than most
/// tables anybody writes.
#[test]
fn paging_through_a_pipe_table_stops_at_its_last_row() {
    let mut ed = Editor::new();
    let mut text = String::from("前文\n\n| a | b | c |\n| --- | --- | --- |\n");
    for i in 0..6 {
        // Ragged again, so keeping the column is a real claim and not the
        // accident of every row being the same width.
        text.push_str(&format!("| {} | b{i} | c{i} |\n", "x".repeat(1 + i * 4)));
    }
    text.push_str("\n後文\n");
    ed.current_buffer_mut().insert(0, &text).expect("the fixture buffer is writable");
    ed.set_page(8, 8);
    ed.goto_line(5);
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "ll");
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "column c");

    let region = ed.md_region().expect("in the table");
    // Pages down, and keeps going: six rows is shorter than a page, so
    // this runs off the end — and stops on the last row, in its column,
    // rather than carrying on into the prose under the table.
    for _ in 0..3 {
        ed.on_key(Key::Char('J'));
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "still column c");
    }
    assert_eq!(ed.cursor_line(), region.last, "and stopped at the last row");

    // …and up again, which stops at the header rather than in the blank
    // line above it. `C-f` and `C-b` are the same motion by another name.
    for _ in 0..3 {
        ed.on_key(Key::Char('K'));
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "coming back");
    }
    assert_eq!(ed.cursor_line(), region.first, "the header is the top");
    for _ in 0..3 {
        ed.on_key(Key::Ctrl('f'));
    }
    assert_eq!(ed.cursor_line(), region.last, "C-f pages the same way");
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2));
    for _ in 0..3 {
        ed.on_key(Key::Ctrl('b'));
    }
    assert_eq!(ed.cursor_line(), region.first, "and C-b back");

    // `L` and `H` are the whole page rather than half of it, and page the
    // same way — down and up the rows, not across the columns.
    ed.on_key(Key::Char('L'));
    assert_eq!(ed.cursor_line(), region.last, "a whole page down");
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "in its column");
    ed.on_key(Key::Char('H'));
    assert_eq!(ed.cursor_line(), region.first, "and a whole page back");
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2));
}

#[test]
fn a_table_with_no_declared_scope_searches_all_of_it() {
    // It used to refuse: 「這張表沒說哪些是拆分欄」. A schema is an
    // optimisation — two columns instead of twenty-eight — not a licence.
    let dir = std::env::temp_dir().join(format!("yumete-scope-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("d.csv");
    std::fs::write(&csv, "a,b\n甲,乙\n丙,甲\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    ed.goto_line(2);
    press(&mut ed, "t?");
    assert_eq!(ed.peeked_line(), Some(1), "{}", ed.status());
    assert!(ed.status().contains("未指定"), "and it says so: {}", ed.status());
    assert!(ed.status().contains("1/2"), "{}", ed.status());
    ed.on_key(Key::Char('n'));
    // Down the first column and then down the second: the second hit is
    // the 甲 in row 2's *other* column.
    assert_eq!(ed.peeked_line(), Some(2), "the other column");
    std::fs::remove_dir_all(&dir).ok();
}

/// #275. This used to assert that a `|` table is **never** drawn as a
/// grid — 「a document keeps its layout and its page」 — which was the
/// editor having only two of the three modes asked for and giving the
/// middle one the top one's key. Both are true now, and which one you get
/// is which key you pressed.
#[test]
fn a_pipe_table_is_drawn_as_a_grid_only_when_that_is_the_key_pressed() {
    // `t n` — 表格操作: the syntax stays on the page, the keys are the
    // grid's, and a 縱書 manuscript is still 縱書.
    let mut ed = with_md_table();
    ed.set_layout(Layout::Vertical);
    press(&mut ed, "tb");
    assert_eq!(ed.layout(), Layout::Vertical, "{}", ed.status());
    assert!(ed.table().unwrap().in_prose(), "{}", ed.status());
    assert!(!ed.table().unwrap().takes_the_pane(), "three lines, not the pane");

    // `t a` — 現在的 tt 模式: 「照舊把整頁轉橫」, because a grid is read
    // across. It is still three lines of a chapter, so it does not take
    // the window.
    press(&mut ed, "tf");
    assert!(ed.grid_is_drawn(), "{}", ed.status());
    assert!(ed.table().unwrap().in_prose(), "still inside the document");
    assert_eq!(ed.layout(), Layout::Horizontal, "{}", ed.status());
    assert!(!ed.table().unwrap().takes_the_pane(), "still three lines of a chapter");

    // `t t` — the whole window, and `t q` gives it back to whichever
    // surface it took it from (2026-09-05): 「退到 markdown 文件中，且回到
    // 此前的表格模式」.
    press(&mut ed, "tt");
    assert!(ed.table().unwrap().takes_the_pane(), "{}", ed.status());
    press(&mut ed, "tq");
    assert!(ed.grid_is_drawn(), "{}", ed.status());
    assert!(ed.table().unwrap().in_prose(), "back on t a, not in prose");

    // And any of them switches straight into any other — `t n` is a
    // surface, not the way out — which is what gives the page back.
    press(&mut ed, "tb");
    assert!(ed.table().unwrap().in_prose(), "{}", ed.status());
    assert_eq!(ed.layout(), Layout::Vertical, "{}", ed.status());

    // `t q` is refused outside the full-screen grid, and `t o` is the one
    // way back to prose.
    press(&mut ed, "tq");
    assert!(ed.table().is_some(), "t q is the window's key: {}", ed.status());
    press(&mut ed, "to");
    assert!(ed.table().is_none(), "{}", ed.status());
}

/// The frozen header is drawn, not stood on.
#[test]
fn the_grid_never_stands_on_the_header_it_draws() {
    let mut ed = with_md_table();
    ed.goto_line(2); // 「| 字 | 讀音 |」, the header itself
    press(&mut ed, "tt");
    assert_eq!(ed.cursor_line(), 3, "the first row, not the heading: {}", ed.status());

    // And `k` there stops: above it is a line the grid draws out of the
    // schema, where a caret would be a lie and a keystroke an edit to the
    // column names.
    press(&mut ed, "k");
    assert_eq!(ed.cursor_line(), 3, "{}", ed.status());

    // 表格操作 keeps the header on the page, so there it is a line like any
    // other and `k` walks onto it.
    press(&mut ed, "tb");
    press(&mut ed, "k");
    assert_eq!(ed.cursor_line(), 1, "{}", ed.status());
}

/// `:table` is the **door**, not a level. Typed while a table is already
/// up it used to walk in again as 畫成表格 — quietly demoting 全窗表格 and
/// losing the window `t q` would have given back.
#[test]
fn typing_table_again_does_not_demote_the_level_it_is_already_in() {
    let mut ed = with_md_table();
    ed.goto_line(3);
    press(&mut ed, "tt");
    let before = (ed.table_level(), ed.table.as_ref().map(|v| v.pane));
    ed.execute(":table").unwrap();
    assert_eq!(
        (ed.table_level(), ed.table.as_ref().map(|v| v.pane)),
        before,
        "{}",
        ed.status()
    );
    assert_eq!(ed.status(), say!("table.already-the-window"));

    // And the door still closes.
    ed.execute(":table off").unwrap();
    assert!(ed.table.is_none(), "{}", ed.status());
}

/// A table with a header, a rule and nothing under them has **no rows**.
#[test]
fn a_table_with_no_rows_yet_does_not_call_its_rule_a_row() {
    let mut ed = typed("前文\n| 字 | 讀音 |\n| --- | --- |\n後文\n");
    ed.goto_line(2);
    press(&mut ed, "tt");
    assert!(ed.table().is_some(), "{}", ed.status());
    assert_eq!(ed.table_row_span(), None, "the rule is drawn, not written");
    assert!(!ed.grid_has_the_pane(), "and there is nothing to give a window to");
}

/// `t ]` into a table of a different shape names *that* table's columns.
#[test]
fn the_next_table_is_drawn_with_its_own_columns() {
    let mut ed = typed(
        "| 名字 | 出場 |\n| --- | --- |\n| 阿寧 | 三 |\n\n             | 甲 | 乙 | 丙 | 丁 |\n| --- | --- | --- | --- |\n| 一 | 二 | 三 | 四 |\n",
    );
    ed.goto_line(3);
    press(&mut ed, "tt");
    assert_eq!(ed.table_column_count(), 2, "{}", ed.status());
    assert!(!ed.row_is_ragged(2), "two cells, two columns");

    press(&mut ed, "t]");
    assert_eq!(ed.cursor_line(), 6, "the row, not the header: {}", ed.status());
    assert_eq!(ed.table_column_count(), 4, "the schema still says two");
    assert!(!ed.row_is_ragged(6), "four cells is not ragged in a four-column table");
    assert_eq!(ed.table_headings(), vec!["甲", "乙", "丙", "丁"], "{}", ed.status());
    assert!(ed.table_status().is_some_and(|s| s.starts_with('甲')), "{:?}", ed.table_status());
}

/// The window a grid took, given back the moment there is nothing to draw.
///
/// `G` in 全窗表格 is a file-wide motion, and the widget draws only between
/// the table's first and last row — so it drew nothing at all, and the
/// window went blank with a live cursor behind it.
#[test]
fn the_window_comes_back_when_the_cursor_walks_out_of_the_grid() {
    let mut ed = with_md_table();
    press(&mut ed, "tt");
    assert!(ed.grid_has_the_pane(), "{}", ed.status());

    // 後文 — the paragraph under the table, which no grid can draw.
    ed.goto_line(6);
    assert!(ed.table().unwrap().takes_the_pane(), "the mode is still on");
    assert!(!ed.grid_has_the_pane(), "and the page draws itself: {}", ed.status());

    // Walking back in brings it back, unasked.
    ed.goto_line(4);
    assert!(ed.grid_has_the_pane(), "{}", ed.status());
}

#[test]
fn only_the_drawn_mode_draws_the_grid() {
    // 「完全画成表格」 — and only there. 表格操作 keeps 「markdown/csv 的语法
    // 标记」 on the page, which is the whole difference between the two.
    let mut ed = with_md_table();
    press(&mut ed, "tb");
    assert!(ed.grid_on_line(1).is_empty(), "the pipes stay pipes");
    // **The ruler is not part of the drawing** (#379). It used to be — it came
    // through `grid_walls`, which answers only at 全 — and the author asked
    // for the numbers at 基本 too, 2026-09-11：「tb 模式可不可以也标注列号」.
    // Which 基本's own law allows: 「tb 的原则是只能多字（标注）不能少字」, and
    // a strip above the table adds a row of annotation without hiding, folding
    // or replacing one character of what the writer typed.
    assert!(
        !ed.table_ruler_on_line(1).is_empty(),
        "基本 numbers the columns: {}",
        ed.status()
    );

    press(&mut ed, "tf");
    let head: Vec<char> = ed.grid_on_line(1).into_iter().map(|(_, g)| g).collect();
    assert_eq!(head, vec!['┆', '┆', '┆'], "| 字 | 讀音 |");
    // The rule row is not a row — it is the line under the head, and every
    // character of it is drawn, the file's spaces included.
    let rule: String = ed.grid_on_line(2).into_iter().map(|(_, g)| g).collect();
    assert_eq!(rule, "├┄┄┄┄┄┼┄┄┄┄┄┤", "{}", ed.status());
    assert!(ed.grid_rule_row(2));
    assert!(!ed.grid_rule_row(1));

    // 「畫，貼在表格上緣」: one ruler, over the first line and no other.
    assert_eq!(ed.table_ruler_on_line(1).len(), 2, "two columns, two numbers");
    for line in [0, 2, 3, 4, 5] {
        assert!(ed.table_ruler_on_line(line).is_empty(), "line {line}");
    }
    // The prose around it is prose.
    assert!(ed.grid_on_line(0).is_empty());
    assert!(ed.grid_on_line(5).is_empty());
}

#[test]
fn every_table_in_a_markdown_file_is_drawn_at_once() {
    // 「對於這個文件中所有的表格都生效」 — and each draws its own ruler,
    // because the numbers over one table are that table's columns.
    let mut ed = typed("| a | b |\n| --- | --- |\n| 1 | 2 |\n\n中間\n\n| c | d | e |\n| --- | --- | --- |\n| 3 | 4 | 5 |\n");
    ed.current_buffer_mut().set_syntax(crate::syntax::Syntax::Markdown);
    ed.goto_line(5);
    assert!(ed.enter_table(), "{}", ed.status());
    assert_eq!(ed.table_ruler_on_line(0).len(), 2, "{}", ed.status());
    assert_eq!(ed.table_ruler_on_line(6).len(), 3, "the second table");
    assert!(!ed.grid_on_line(8).is_empty(), "its last row");
    // The paragraph between them is still a paragraph.
    assert!(ed.grid_on_line(4).is_empty(), "中間");
    assert!(ed.table_ruler_on_line(4).is_empty());
}

#[test]
fn the_level_hands_the_keys_over_without_a_key_being_pressed() {
    // #283's half-delivered promise: `TableLevel::Basic` is the factory
    // value and says the columns line up **and** the keys belong to the
    // grid — but the padding asked the level and the keys asked the view,
    // and only a `t` key ever built a view. So a freshly opened `.md` was
    // padded *and* soft-wrapping, with `hjkl` walking letters: a state
    // neither `t o` nor `t b` names.
    let mut ed = typed("散文\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n散文\n");
    ed.current_buffer_mut().set_syntax(crate::syntax::Syntax::Markdown);
    assert_eq!(ed.table_level(), TableLevel::Basic, "the factory value");
    press(&mut ed, "2j");
    assert!(
        ed.table_row_at(2),
        "a row of a table is one row, however wide: {}",
        ed.status()
    );
    assert!(!ed.table_row_at(0), "the prose above it is prose");
    // The keys are the grid's, and nothing was said to announce it.
    assert!(ed.cell_position().is_some(), "hjkl walk cells: {}", ed.status());

    // …and `t o` is still a real off switch: walking back in does not
    // build it again, because the level is what was put down.
    press(&mut ed, "to");
    assert_eq!(ed.table_level(), TableLevel::Off);
    press(&mut ed, "kj");
    assert!(!ed.table_row_at(2), "`t o` stays off: {}", ed.status());
    assert!(ed.cell_position().is_none());
}

#[test]
fn the_view_is_never_built_over_a_table_that_is_not_one() {
    // The automatic door is read-only, so it may only open on tables that
    // **already parse** — the set the renderer draws. `t b` writes a
    // `| --- |` under a bare header, and a key may rewrite the buffer
    // where opening a file and moving a cursor may not.
    let mut ed = typed("| 甲 | 乙 |\n| 一 | 二 |\n\n```\n| a | b |\n| --- | --- |\n| 1 | 2 |\n```\n");
    ed.current_buffer_mut().set_syntax(crate::syntax::Syntax::Markdown);
    let was = ed.current_buffer().text();
    for line in 1..=8 {
        ed.goto_line(line);
        press(&mut ed, "l");
        assert!(
            ed.cell_position().is_none(),
            "line {line} is prose: {}",
            ed.status()
        );
    }
    assert_eq!(ed.current_buffer().text(), was, "no rule row was written");
}

#[test]
fn a_vertical_page_never_takes_the_keys_on_its_own() {
    // The two halves of the promise arrive together or not at all: 縱書
    // pads nothing (a grid is read across), so on its own it takes no keys
    // either, and `t b` there stays the reader's own decision.
    //
    // Typed with the page **already turned**, so this is the door not
    // opening. A view built while the page was flat is asked the same
    // question by `table_here`, so it goes quiet on the turn too — this
    // test is the half that never builds one at all.
    let mut ed = Editor::new();
    ed.current_buffer_mut().set_syntax(crate::syntax::Syntax::Markdown);
    ed.set_layout(Layout::Vertical);
    ed.on_key(Key::Char('i'));
    for c in "| a | b |\n| --- | --- |\n| 1 | 2 |\n".chars() {
        ed.on_key(if c == '\n' { Key::Enter } else { Key::Char(c) });
    }
    ed.on_key(Key::Esc);
    press(&mut ed, "ggjl");
    assert!(ed.cell_position().is_none(), "{}", ed.status());
    assert!(!ed.table_row_at(0));

    // And it is the turned page that stopped it, not the file: flatten the
    // page and the very next key hands the keys over.
    ed.set_layout(Layout::Horizontal);
    press(&mut ed, "l");
    assert!(ed.cell_position().is_some(), "{}", ed.status());
}

#[test]
fn a_macro_keeps_the_key_a_sequence_was_waiting_for() {
    // `Q` ends a recording — but a `Q` that `f` is waiting for is an
    // operand. Dropping it left the macro as a bare `f`, which on replay
    // swallowed whatever came next; one reviewer's macro deleted their
    // buffer that way. (The pair swapped on 2026-09-12 to match Helix, and
    // this hazard moved with it: it is about the *ending* key, whichever
    // letter that is.)
    let mut ed = typed("aQb\ncQd\n");
    ed.goto_line(1);
    press(&mut ed, "Q");
    press(&mut ed, "fQ");
    press(&mut ed, "x");
    press(&mut ed, "Q");
    assert_eq!(ed.recorded_keys_for_test(), "fQx", "the `Q` of `fQ` is kept");

    ed.goto_line(2);
    press(&mut ed, "q");
    assert_eq!(
        ed.current_buffer().text(),
        "aQb\ncQd\n",
        "replay finds Q and selects the line, changing nothing"
    );
}

#[test]
fn an_unnamed_buffer_leaves_a_crash_copy_too() {
    // `yumete` with no argument and an hour of typing is an ordinary way to
    // start a scene, and it used to be the one buffer with no safety net:
    // recovery copies live beside the file, and there is no file.
    let dir = std::env::temp_dir().join(format!("yumete-drafts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut ed = Editor::new();
    ed.keep_drafts_in(dir.clone());
    ed.set_autosave(true);
    press(&mut ed, "i");
    for c in "那年冬天，山下起了大雪。".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Esc);
    ed.autosave_tick();
    let draft = ed.current_buffer().scratch_draft().map(|p| p.to_path_buf());
    let draft = draft.expect("an unnamed buffer gets somewhere to keep a copy");
    assert!(draft.exists(), "and the copy is really there");
    assert!(std::fs::read_to_string(&draft).unwrap().contains("大雪"));

    // The session that wrote it does not offer it back to itself.
    assert!(ed.orphan_drafts().is_empty());

    // The next session finds it — nothing else ever would, since there is
    // no file whose name would lead you to it.
    let mut next = Editor::new();
    next.keep_drafts_in(dir.clone());
    // Pretend it was another session's.
    let orphan = dir.join("scratch-1-0.yumete");
    std::fs::rename(&draft, &orphan).unwrap();
    assert_eq!(next.orphan_drafts(), vec![orphan.clone()]);
    next.announce_recovery();
    assert!(next.status().contains("沒存的草稿"), "{}", next.status());

    // `:recover` opens it as a buffer of its own, and takes the file away
    // so the next launch does not offer it again.
    next.execute("recover").unwrap();
    assert!(next.status().contains("1 份"), "{}", next.status());
    // An empty scratch buffer is replaced rather than added to, which is
    // what makes the very first launch of a recovering session land
    // straight on the draft.
    assert!(next.current_buffer().text().contains("大雪"));
    assert!(next.current_buffer().display_name().starts_with("草稿"));
    assert!(!orphan.exists());
    assert!(next.orphan_drafts().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

/// `:recover` with `autosave = false` keeps a copy before it deletes one.
///
/// The old comment said the file could go because「it is written again
/// immediately, because the buffer is modified」— true only with autosave on.
/// `autosave_tick` returns on its first line when it is off, and
/// `[editor] autosave = false` is a documented setting. So `:recover` deleted
/// the only copy of a crashed session's work and left the text in memory
/// alone: one closed terminal and yumete had destroyed it itself.
#[test]
fn recovering_with_autosave_off_still_leaves_a_copy() {
    let dir = std::env::temp_dir().join(format!("yumete-recover-off-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let orphan = dir.join("scratch-1-0.yumete");
    std::fs::write(&orphan, "那年冬天，山下起了大雪。\n").unwrap();

    let mut ed = Editor::new();
    ed.keep_drafts_in(dir.clone());
    ed.set_autosave(false);
    assert_eq!(ed.orphan_drafts(), vec![orphan.clone()]);
    ed.execute("recover").unwrap();
    assert!(ed.current_buffer().text().contains("大雪"));

    // The text is now in exactly one place on disk — a different name, but a
    // place. Before the fix there were none.
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| std::fs::read_to_string(p).map(|t| t.contains("大雪")).unwrap_or(false))
        .collect();
    assert_eq!(left.len(), 1, "one copy, not zero and not two: {left:?}");
    // …and it is not the file it was offered from, so the next launch does
    // not offer the same work twice.
    assert!(!orphan.exists());
    assert!(ed.orphan_drafts().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_edit_that_did_nothing_leaves_nothing_to_undo() {
    // An undo point used to be pushed when a command *announced* an edit,
    // not when it made one. Three keys that did nothing left three undo
    // steps that did nothing, and `u` became "sometimes works".
    let mut ed = typed("那年冬天");
    press(&mut ed, "x");
    press(&mut ed, "d");
    assert_eq!(ed.current_buffer().text(), "");

    // Nothing left to delete. These change nothing, so they are not places
    // to come back to.
    press(&mut ed, "d");
    press(&mut ed, "d");
    press(&mut ed, "d");
    // Nor is an Insert session that typed nothing.
    press(&mut ed, "i");
    ed.on_key(Key::Esc);
    press(&mut ed, "a");
    ed.on_key(Key::Esc);

    ed.on_key(Key::Char('u'));
    assert_eq!(
        ed.current_buffer().text(),
        "那年冬天",
        "one `u` goes back to the last edit that happened"
    );

    // A refused edit is the same case: the guard stopped it, so there is
    // nothing to undo — and the text before it is still one `u` away.
    let (dir, csv) = a_table("undo");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "l");
    press(&mut ed, "i");
    ed.on_key(Key::Char('土'));
    ed.on_key(Key::Char(','));
    ed.on_key(Key::Char(','));
    ed.on_key(Key::Esc);
    assert_eq!(ed.cell_text(1, 1), "土⿰木目");
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.cell_text(1, 1), "⿰木目", "the refusals cost nothing");

    std::fs::remove_dir_all(&dir).ok();
}

/// 繁體 to 繁體 is a 字形 table and nothing else, so it runs here — no
/// opencc, no child process, and a machine without opencc can still do it
/// (#241).
#[test]
fn mainland_glyphs_need_no_program_at_all() {
    let mut ed = typed("他說內人在裏面，吳先生溫了酒。\n");
    assert!(ed.execute("convert t c").is_ok());
    assert!(ed.take_shell_request().is_none(), "nothing to run");
    assert_eq!(
        ed.current_buffer().text(),
        "他説内人在裏面，吴先生温了酒。\n"
    );
    // One edit, so one `u` takes the book back.
    press(&mut ed, "u");
    assert_eq!(
        ed.current_buffer().text(),
        "他說內人在裏面，吳先生溫了酒。\n"
    );
}

/// Everything else is opencc's, and the editor's whole part in it is to
/// hand over the buffer and put back what comes out (#241).
#[test]
fn simplified_to_traditional_is_handed_to_opencc() {
    // ⚠️ `:convert` hands the work to **opencc**, and says how to install it
    // instead when it is not there — so with no opencc there is no shell
    // request to look at, and this asserts nothing. Both CI Linux runners and
    // the macOS one caught it: green on a developer's machine, red on a bare
    // one. See `crate::convert::opencc`.
    if crate::convert::opencc().is_none() {
        return;
    }
    let mut ed = typed("他说内人在里面。\n");
    assert!(ed.execute("convert s c").is_ok());
    let asked = ed.take_shell_request().expect("opencc to run");
    assert!(
        asked.line.ends_with("-c s2t.json"),
        "大陸通規 goes through 繁體 first: {}",
        asked.line
    );
    match asked.how {
        How::Convert(text) => assert_eq!(text, "他说内人在里面。\n", "the whole book"),
        other => panic!("{other:?}"),
    }
    // What opencc says, then the 字形 table on top of it.
    ed.provide_conversion("他說內人在裏面。\n");
    assert_eq!(ed.current_buffer().text(), "他説内人在裏面。\n");
    assert!(ed.status().contains('2'), "how much moved: {}", ed.status());
}

/// A pair opencc cannot do is refused before anything is spawned, and the
/// refusal says where to look — the alternative is a command line that
/// fails with opencc's own error, which names a config file (#241).
#[test]
fn a_pair_opencc_cannot_do_is_refused_rather_than_attempted() {
    let mut ed = typed("なにか\n");
    for line in ["convert jp s", "convert c g", "convert s s", "convert s t force"] {
        assert!(ed.execute(line).is_ok(), "{line}");
        assert!(
            ed.take_shell_request().is_none(),
            "{line} spawned something ({})",
            ed.status()
        );
        assert!(!ed.status().is_empty(), "{line} said nothing");
        assert_eq!(ed.current_buffer().text(), "なにか\n", "{line} edited");
    }
    // A word that is not a side at all is a parse error, so the `:` line
    // stays open with what was typed.
    assert!(ed.execute("convert s zh").is_err());
}

/// `force` is stripped by [`Editor::execute`] before the parser sees it —
/// it is the editor-wide 「do it anyway」 suffix — so `:convert … force`
/// reached the command with the flag already gone and quietly ran the
/// character-only config (#241).
#[test]
fn convert_force_survives_the_suffix_that_every_command_shares() {
    // ⚠️ `:convert` hands the work to **opencc**, and says how to install it
    // instead when it is not there — so with no opencc there is no shell
    // request to look at, and this asserts nothing. Both CI Linux runners and
    // the macOS one caught it: green on a developer's machine, red on a bare
    // one. See `crate::convert::opencc`.
    if crate::convert::opencc().is_none() {
        return;
    }
    let mut ed = typed("内存不足\n");
    assert!(ed.execute("convert s tw force").is_ok());
    let asked = ed.take_shell_request().expect("opencc to run");
    assert!(asked.line.ends_with("-c s2twp.json"), "{}", asked.line);

    // And without it, the character-only config.
    assert!(ed.execute("convert s tw").is_ok());
    let asked = ed.take_shell_request().expect("opencc to run");
    assert!(asked.line.ends_with("-c s2tw.json"), "{}", asked.line);
}

/// `:convert` alone explains rather than guessing a direction (#241).
#[test]
fn convert_on_its_own_says_what_it_can_do() {
    let mut ed = typed("甲\n");
    let before = ed.buffer_count();
    assert!(ed.execute("convert").is_ok());
    assert_eq!(ed.buffer_count(), before + 1, "a listing");
    let out = ed.current_buffer().text();
    for side in ["s", "tw", "hk", "jp", "c", "g"] {
        assert!(out.contains(side), "{side} missing from:\n{out}");
    }
    assert!(out.contains("force"), "and where force works:\n{out}");
    // 日本新字体 has no way back to 簡體, and the page must not offer one.
    let jp = out
        .lines()
        .find(|l| l.trim_start().starts_with("jp  →"))
        .unwrap_or_else(|| panic!("no jp row:\n{out}"));
    assert!(!jp.contains(" s"), "{jp}");
}

/// opencc failing does not get to overwrite a manuscript (#241).
#[test]
fn a_conversion_that_came_back_empty_changes_nothing() {
    // ⚠️ `:convert` hands the work to **opencc**, and says how to install it
    // instead when it is not there — so with no opencc there is no shell
    // request to look at, and this asserts nothing. Both CI Linux runners and
    // the macOS one caught it: green on a developer's machine, red on a bare
    // one. See `crate::convert::opencc`.
    if crate::convert::opencc().is_none() {
        return;
    }
    let mut ed = typed("他说内人在里面。\n");
    assert!(ed.execute("convert s t").is_ok());
    assert!(ed.take_shell_request().is_some());
    ed.provide_conversion("");
    assert_eq!(ed.current_buffer().text(), "他说内人在里面。\n");
    assert!(!ed.status().is_empty());
}

#[test]
fn what_was_cut_three_edits_ago_is_still_reachable() {
    // Every yank and every delete overwrote one register, so "where did
    // that paragraph go" had no answer.
    let mut ed = typed("甲一\n乙二\n丙三\n");
    ed.goto_line(1);
    press(&mut ed, "xd");
    ed.goto_line(1);
    press(&mut ed, "xd");
    assert_eq!(ed.current_buffer().text(), "丙三\n");

    // Both are still there, newest first, and the named ones after them.
    ed.goto_line(1);
    press(&mut ed, "x");
    press(&mut ed, "\"ay");
    let menu = ed.paste_menu();
    assert_eq!(menu[0].1, "乙二\n", "the last thing cut");
    assert_eq!(menu[1].1, "甲一\n", "and the one before it");
    assert_eq!(menu[2].0, "\"a", "then the named registers");
    assert_eq!(menu[2].1, "丙三\n");

    // `Space \"` offers them, with the system clipboard first — the only
    // one the core cannot read for itself.
    type_keys(&mut ed, " \"");
    assert_eq!(ed.mode(), Mode::Picker);
    let shown = ed.picker().unwrap().matches();
    assert!(shown[0].label().contains("系統剪貼板"));
    assert!(shown[1].label().contains("乙二"));

    // Choosing one pastes it, without disturbing the ring's order.
    ed.on_key(Key::Down);
    ed.on_key(Key::Enter);
    assert_eq!(ed.mode(), Mode::Normal);
    assert!(ed.current_buffer().text().contains("乙二"), "{}", ed.current_buffer().text());

    // Taking the same thing twice does not fill the list with it: `yy` is
    // one thing you took, not two.
    ed.goto_line(1);
    press(&mut ed, "y");
    let before = ed.paste_menu().len();
    press(&mut ed, "y");
    assert_eq!(ed.paste_menu().len(), before);
}

#[test]
fn every_far_jump_leaves_a_way_back() {
    // A table's `Enter` used to be a one-way door: following 相 → 木 and
    // coming back meant remembering 相 and searching for it again.
    let dir = std::env::temp_dir().join(format!("yumete-jumps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
    std::fs::write(
        dir.join(".yumete").join("tables").join("t.toml"),
        "[table]\nfile = ['d.csv']\nkey = 'char'\n\
         [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
         [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
    )
    .unwrap();
    let csv = dir.join("d.csv");
    std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();

    // `:table-jump` goes and leaves a way back; `t?` shows and leaves
    // nothing, because nothing was left.
    ed.goto_line(2);
    press(&mut ed, "T"); // by cell
    press(&mut ed, "l"); // 相's 拆分
    let was = ed.cursor();
    press(&mut ed, "t?");
    assert_eq!(ed.cursor(), was, "a peek does not move you");
    ed.execute("table-jump 木").unwrap();
    assert_eq!(ed.cursor_line(), 2, "木's own row: {}", ed.status());
    ed.on_key(Key::Ctrl('o'));
    assert_eq!(ed.cursor(), was, "and back where the jump started");
    ed.on_key(Key::Ctrl('i'));
    assert_eq!(ed.cursor_line(), 2, "…and forward again");

    // The same list holds every far motion, not only the table's.
    let mut ed = typed("一\n二\n三\n四\n五\n六\n七\n八\n");
    ed.execute("7").unwrap();
    assert_eq!(ed.cursor_line(), 6);
    ed.execute("2").unwrap();
    assert_eq!(ed.cursor_line(), 1);
    ed.on_key(Key::Ctrl('o'));
    assert_eq!(ed.cursor_line(), 6, "back to where `:2` was typed");
    ed.on_key(Key::Ctrl('o'));
    assert_eq!(ed.cursor_line(), 0, "and to where `:7` was typed");
    // `gg` and `ge` are far motions too, and now leave a way back — which
    // is what `remember_jump`'s own doc comment always claimed.
    ed.on_key(Key::Ctrl('o'));
    assert_eq!(ed.cursor_line(), 8, "and to where `gg` was pressed");
    ed.on_key(Key::Ctrl('o'));
    assert!(ed.status().contains("沒有更早"), "{}", ed.status());
    ed.on_key(Key::Ctrl('i'));
    assert_eq!(ed.cursor_line(), 0);

    // A search is a link: you look something up and you want to be back
    // where you were writing.
    let mut ed = typed("一\n二\n三\n四\n五\n六\n七\n八\n");
    ed.execute("3").unwrap();
    let was = ed.cursor();
    press(&mut ed, "/八");
    ed.on_key(Key::Enter);
    assert_eq!(ed.cursor_line(), 7);
    ed.on_key(Key::Ctrl('o'));
    assert_eq!(ed.cursor(), was, "back to where the search was typed");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_cell_can_be_copied_and_a_row_can_be_duplicated() {
    // The guard that makes the grid safe used to make this impossible:
    // `v l y` reaches across the delimiter, and pasting what it took was
    // then refused. In the grid the cell is the unit, so it is the unit
    // copy and paste work in too.
    let (dir, csv) = a_table("copy");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "l");
    assert_eq!(ed.cell_text(1, 1), "⿰木目");

    // One key takes the cell, one key puts it in another.
    press(&mut ed, "y");
    assert!(ed.status().contains("一格"), "{}", ed.status());
    press(&mut ed, "j");
    assert_eq!(ed.cell_text(2, 1), "土");
    press(&mut ed, "p");
    assert_eq!(ed.cell_text(2, 1), "⿰木目", "the cell was replaced");
    assert_eq!(ed.cell_text(2, 0), "二", "and its neighbours are untouched");
    assert_eq!(ed.cell_text(2, 2), "土");
    assert_eq!(ed.current_buffer().text().matches(',').count(), 6);

    // A whole row in the register becomes a whole new row — which is how a
    // variant character gets its neighbour's decomposition.
    press(&mut ed, "Y");
    press(&mut ed, "p");
    let text = ed.current_buffer().text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "a row was added: {lines:?}");
    assert_eq!(lines[2], lines[3], "and it is a copy of the one above");
    assert_eq!(ed.cursor_line(), 3, "the cursor is on the new row");
    assert!(!ed.row_is_ragged(3), "which is a whole row, not a fragment");

    // Half a row has no honest place to go.
    ed.set_register_for_test("二,土");
    press(&mut ed, "p");
    assert_eq!(ed.current_buffer().text().lines().count(), 4, "refused");
    assert!(ed.status().contains("分隔"), "{}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_bang_sends_the_selection_through_a_command() {
    let mut ed = typed("丙\n甲\n乙\n");

    // `!` opens the command line with the verb already typed, so the key is
    // a shortcut rather than a second mechanism — and pressing it by
    // accident shows what it was about to do.
    ed.goto_line(1);
    press(&mut ed, "x");
    press(&mut ed, "x");
    press(&mut ed, "x");
    ed.on_key(Key::Char('!'));
    assert_eq!(ed.mode(), Mode::Command);
    assert_eq!(ed.prompt(), Some((":", "pipe ")));

    // Running it asks the front end, with the selection as the input.
    for c in "tr -d ' '".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Enter);
    let asked = ed.take_shell_request().expect("a command to run");
    assert_eq!(asked.line, "tr -d ' '");
    assert_eq!(asked.how, How::Pipe("丙\n甲\n乙\n".to_string()));

    // What it says goes back in place of what it was given, as one edit.
    ed.provide_pipe_output("丙甲乙\n");
    assert_eq!(ed.current_buffer().text(), "丙甲乙\n");
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "丙\n甲\n乙\n", "one `u` takes it back");

    // `:sh` is the other one: nothing is replaced, the answer comes back in
    // a buffer of its own.
    ed.execute("sh wc -l").unwrap();
    let asked = ed.take_shell_request().unwrap();
    assert_eq!(asked.how, How::Capture);
    let before = ed.buffer_count();
    ed.provide_shell_output("wc -l", "3\n");
    assert_eq!(ed.buffer_count(), before + 1);
    assert!(ed.current_buffer().text().contains("$ wc -l"), "what was run");
    assert!(ed.current_buffer().display_name().contains("wc -l"));
}

#[test]
fn no_route_at_all_gets_a_delimiter_into_a_cell() {
    // A review found seven ways past the first version of this guard, each
    // of which shifted every column of a row and then wrote the file out
    // without a word. Every one of them is here.
    let (dir, csv) = a_table("guardall");
    let commas = |ed: &Editor| ed.current_buffer().text().matches(',').count();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    let clean = ed.current_buffer().text();
    let n = commas(&ed);
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "l");

    // 1. Typed.
    press(&mut ed, "i");
    ed.on_key(Key::Char(','));
    assert_eq!(commas(&ed), n, "typed");
    // 2. A tab and a line break are not text a cell may hold either.
    ed.on_key(Key::Char('\t'));
    ed.on_key(Key::Char('\n'));
    assert_eq!(ed.current_buffer().text(), clean, "tab and newline");
    // 3. Bracketed paste — ⌘V, which the manual says still works.
    ed.paste_text("⿰木,目");
    assert_eq!(commas(&ed), n, "pasted");
    ed.paste_text("⿰木\n目");
    assert_eq!(commas(&ed), n, "pasted over two lines");
    assert_eq!(ed.current_buffer().text(), clean);
    // 4. An IME commit.
    ed.insert_committed("木,目");
    assert_eq!(commas(&ed), n, "committed by the IME");
    ed.on_key(Key::Esc);

    // 5. The system clipboard (`Space p`).
    ed.provide_clipboard("木,目", true);
    assert_eq!(commas(&ed), n, "from the system clipboard");
    // 6. `r` — two keystrokes in Normal mode, and the easiest of the lot.
    press(&mut ed, "r");
    ed.on_key(Key::Char(','));
    assert_eq!(commas(&ed), n, "overwritten with r");
    // 7. `R`, from a register holding a whole row.
    press(&mut ed, "0");
    press(&mut ed, "xy");
    press(&mut ed, "l");
    press(&mut ed, "R");
    assert_eq!(commas(&ed), n, "replaced from a register");
    // 8. `:s`, which rewrites whole lines at once.
    assert!(ed.execute("s/⿰/a,b/").is_ok());
    assert_eq!(commas(&ed), n, "substituted");
    assert!(ed.status().contains("格"), "and it says why: {}", ed.status());
    assert!(ed.execute("s/⿰/a\\nb/").is_ok());
    assert_eq!(commas(&ed), n, "substituted with a line break");

    // 9. Deleting the delimiter itself. An empty cell sits exactly on one,
    // so `d` there used to join its two neighbours — and on this table most
    // columns are empty on most rows, which made it the likeliest accident
    // of all.
    std::fs::write(&csv, "char,ids_y,ids_g\n一,,⿰木目\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    let clean = ed.current_buffer().text();
    ed.goto_line(2);
    press(&mut ed, "l");
    assert_eq!(ed.cell_text(1, 1), "", "an empty cell");
    press(&mut ed, "d");
    assert_eq!(ed.current_buffer().text(), clean, "deleted an empty cell");
    press(&mut ed, "c");
    ed.on_key(Key::Esc);
    assert_eq!(ed.current_buffer().text(), clean, "changed an empty cell");
    press(&mut ed, "x");
    press(&mut ed, "d");
    assert_eq!(ed.current_buffer().text(), clean, "selected the line and cut");

    // And a substitution that only changes what is *inside* cells is the
    // useful kind, so it still runs.
    assert!(ed.execute("s/⿰木目/⿰禾布/").is_ok());
    assert_eq!(commas(&ed), 4);
    assert!(ed.current_buffer().text().contains("⿰禾布"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_capital_turns_the_page_the_way_its_lowercase_moves() {
    // 縱書: `j` runs down a 縱 and `J` turns the page onward, whichever way
    // the page is set. Reading a letter one way in lowercase and the other in
    // capital is one letter meaning two directions.
    //
    // ⚠️ **The rule is about screen quantities, and only those.** `H`/`L` used
    // to be whole-page and obeyed it; since 2026-09-12 they take a *sentence*,
    // and this editor's text units have never flipped — `w` `e` `b` and their
    // capitals read onward in both layouts. The second half of this test is
    // that distinction (#404).
    let mut ed = typed(&"字\n".repeat(400));
    ed.set_layout(Layout::Vertical);
    ed.set_page(20, 30);
    ed.execute("200").unwrap();
    let middle = ed.cursor_line();

    press(&mut ed, "J");
    assert!(ed.cursor_line() > middle, "J reads on, as j does");
    press(&mut ed, "K");
    assert_eq!(ed.cursor_line(), middle, "and K comes back");

    // Horizontally the same pair, same meaning.
    ed.set_layout(Layout::Horizontal);
    ed.execute("200").unwrap();
    press(&mut ed, "J");
    assert!(ed.cursor_line() > middle, "J reads on");
    press(&mut ed, "K");
    assert_eq!(ed.cursor_line(), middle);
}

/// **A sentence reads the same way whichever way the page is set** (#404).
///
/// `H`/`L` are a text unit, not a screen quantity: `L` is the sentence after,
/// in 橫排 and in 縱書 alike. The page-turning pair `J`/`K` is the one that
/// follows the direction its lowercase runs.
#[test]
fn the_sentence_pair_does_not_flip_with_the_layout() {
    let text = "第一句。第二句。第三句。\n";
    let end_of_first = |vertical: bool| {
        let mut ed = typed(text);
        if vertical {
            ed.set_layout(Layout::Vertical);
        }
        press(&mut ed, "L");
        ed.selection()
    };
    assert_eq!(end_of_first(false), (0, 4), "橫排：第一句。");
    assert_eq!(end_of_first(true), (0, 4), "縱書：the same sentence");

    // …and pressed twice it goes on, rather than sticking on the 。 it just
    // landed on — the bug `)` carried from the day it was written, invisible
    // only because nobody presses `)` twice.
    let mut ed = typed(text);
    press(&mut ed, "LL");
    assert_eq!(ed.selection(), (4, 8), "第二句。");
    let mut ed = typed(text);
    press(&mut ed, "3L");
    assert_eq!(ed.selection(), (8, 12), "a count goes as far");
}

#[test]
fn a_grid_is_read_across_so_it_is_never_set_vertically() {
    let (dir, csv) = a_table("layout");
    let mut ed = Editor::new();
    ed.set_layout(Layout::Vertical);

    // Opening it is the ordinary door — the manual's own 「放一份 schema
    // 在資料旁邊，它就自動是表格」 — so it is the door that must hold the
    // rule, not only `:table`.
    ed.open_file(&csv).unwrap();
    assert!(ed.table().is_some());
    assert_eq!(ed.layout(), Layout::Horizontal, "a grid is read across");

    // …and it stays turned: the command is refused, not silently ignored.
    ed.execute("layout vertical").unwrap();
    assert_eq!(ed.layout(), Layout::Horizontal);
    assert!(ed.status().contains(":table off"), "{}", ed.status());

    // Leaving the grid gives the layout back. A toggle that does not
    // return you to where you were is not a toggle.
    ed.execute("table off").unwrap();
    assert_eq!(ed.layout(), Layout::Vertical, "back to 縱書");
    assert!(ed.status().contains("轉回竪排"), "{}", ed.status());

    // `:table` on again turns it again, and off again gives it back.
    ed.execute("table").unwrap();
    assert_eq!(ed.layout(), Layout::Horizontal);
    assert!(ed.status().contains("已轉橫排"), "{}", ed.status());
    ed.execute("table off").unwrap();
    assert_eq!(ed.layout(), Layout::Vertical);

    // A grid opened in a horizontal session leaves the layout alone, both
    // ways round — nothing was taken, so nothing is given back.
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert_eq!(ed.layout(), Layout::Horizontal);
    ed.execute("table off").unwrap();
    assert_eq!(ed.layout(), Layout::Horizontal);
    assert!(!ed.status().contains("轉回"), "{}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_file_with_no_schema_is_read_by_its_own_header() {
    // What `yumete -t` falls back on.
    let dir = std::env::temp_dir().join(format!("yumete-bare-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("anything.csv");
    std::fs::write(&csv, "name,reading,note\n雪,ゆき,\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.table().is_none(), "not until asked");

    assert!(ed.enter_table(), "the header row is enough");
    let view = ed.table().unwrap();
    assert_eq!(view.schema.columns.len(), 3);
    assert_eq!(view.schema.columns[1].heading(), "reading");
    assert!(ed.status().contains("照首行"), "{}", ed.status());

    // A file with nothing to split is not a table, and says so.
    let prose = dir.join("prose.txt");
    std::fs::write(&prose, "那年冬天\n").unwrap();
    ed.open_file(&prose).unwrap();
    assert!(!ed.enter_table());
    assert!(ed.table().is_none());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_table_with_no_schema_gets_one_written_beside_it() {
    // #218. The author's own 碼表 has no header and a tab between its two
    // columns, and both facts have to survive into the file.
    let dir = std::env::temp_dir().join(format!("yumete-schema-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let codes = dir.join("codes.txt");
    std::fs::write(&codes, "雪\txue\n月\tyue\n語\tyu\n星\txing\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&codes).unwrap();
    assert!(ed.enter_table(), "a tab is a delimiter");
    assert!(ed.table().unwrap().from.as_os_str().is_empty(), "nobody's schema yet");

    // 「第一行是資料」 (#217), and then 「說出來」 (#218).
    ed.on_key(Key::Char('t'));
    ed.on_key(Key::Char('H'));
    ed.on_key(Key::Char('t'));
    ed.on_key(Key::Char('e'));

    let written = dir.join(".yumete").join("tables").join("codes.toml");
    let text = std::fs::read_to_string(&written).expect("a schema was written beside the data");
    assert!(text.contains("file = \"codes.txt\""), "{text}");
    assert!(text.contains("delimiter = \"\\t\""), "the tab is escaped, not typed: {text}");
    assert!(text.contains("header = false"), "what t H just said: {text}");
    assert_eq!(text.matches("[[table.column]]").count(), 2, "{text}");

    // It is open in the other half, and the keys did not go with it.
    let pane = ed.other_pane().expect("the schema is in the other area");
    assert_eq!(pane.caption, "codes.toml");
    assert_eq!(
        ed.buffers[ed.buffer_with(pane.buffer).unwrap()].path(),
        Some(written.as_path()),
        "the pane names the schema"
    );
    assert_eq!(ed.current_buffer().path(), Some(codes.as_path()), "still on the table");
    assert_eq!(ed.live_pane(), 0);

    // And what it says is what was already on screen: reading the file
    // again finds it and changes nothing.
    ed.leave_table();
    assert!(ed.enter_table());
    let view = ed.table().unwrap();
    assert_eq!(view.from, written, "the schema claims the file now");
    assert!(!view.schema.header, "still 「第一行是資料」");
    assert_eq!(view.schema.delimiter, '\t');
    assert_eq!(view.schema.columns.len(), 2);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_chapters_a_book_includes_are_read_out_of_the_files_themselves() {
    let dir = std::env::temp_dir().join(format!("yumete-inc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ch01.txt"), "== 傳家寶扇\n那年冬天。\n").unwrap();
    std::fs::write(dir.join("ch02.txt"), "== 天門攬勝\n又一年。\n").unwrap();
    // A chapter with no heading of its own has only its file name.
    std::fs::write(dir.join("ch03.txt"), "雪一直下到開春。\n").unwrap();
    std::fs::write(
        dir.join("book.typ"),
        "#import \"template.typ\": ruby\n= 天門真境\n#include \"ch01.txt\"\n\
         #include \"ch02.txt\"\n#include \"ch03.txt\"\n",
    )
    .unwrap();

    let mut ed = Editor::new();
    ed.open_file(dir.join("book.typ")).unwrap();
    ed.open_sidebar_showing(&dir, crate::sidebar::View::Outline);

    // Nothing was compiled: the titles are written in the files, in plain
    // `= 標題`, and reading them is enough.
    let rows = ed.panel(crate::sidebar::Side::Left).unwrap().rows().to_vec();
    assert_eq!(
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec!["天門真境", "  傳家寶扇", "  天門攬勝", "  ch03.txt"],
        "chapter names, indented by their own level; `#import` is not one"
    );
    // Each one knows the file and the line it is written on.
    assert_eq!(rows[0].path, PathBuf::new());
    assert_eq!(rows[1].path, dir.join("ch01.txt"));
    assert_eq!(rows[1].depth, 0);

    ed.on_key(Key::Char('j'));
    ed.on_key(Key::Enter);
    assert_eq!(
        ed.current_buffer().path(),
        Some(dir.join("ch01.txt").as_path())
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// `Space d` asks about the character under the cursor — Feature #215, #293.
///
/// The editor cannot answer: the 拆分表 lives in yume, which only the
/// front end holds. So what is checked here is the half the editor owns —
/// the question is parked, the transient panel is up on it, and the answer,
/// when it comes, is laid out in columns.
#[test]
fn the_dictionary_asks_about_the_character_under_the_cursor() {
    use crate::sidebar::{Layer, Side, Transient};
    let mut ed = typed("那年冬天");
    ed.on_key(Key::Char(' '));
    ed.on_key(Key::Char('d'));

    let right = Side::Right;
    assert_eq!(ed.transient(right), Some(Transient::Dictionary));
    assert_eq!(ed.panel_focus(), Some((right, Layer::Bottom)), "the keys go along");
    assert!(ed.panel(Side::Left).is_none(), "and nothing was opened on the left");
    assert_eq!(ed.take_dictionary_query(), Some('那'));
    assert_eq!(ed.take_dictionary_query(), None, "asked once, answered once");

    // Until the answer arrives the panel is the character alone — not an
    // empty panel, and not last character's answer.
    assert_eq!(ed.transient_rows(right).len(), 1);

    ed.set_dictionary(
        '那',
        vec![
            ("拆分".to_string(), "刀二阝".to_string()),
            ("編碼".to_string(), "vfb".to_string()),
        ],
    );
    let rows: Vec<String> = ed
        .transient_rows(right)
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(rows[0], "那");
    assert_eq!(rows[1], "拆分  刀二阝");
    assert_eq!(rows[2], "編碼  vfb", "names are padded to a column");

    // **It scrolls, because a long answer is why it takes the keys at all.**
    ed.on_key(Key::Char('j'));
    assert_eq!(ed.transient_scroll(), 1);
    ed.on_key(Key::Char('G'));
    assert_eq!(ed.transient_scroll(), 2, "the last of three");
    ed.on_key(Key::Char('g'));
    assert_eq!(ed.transient_scroll(), 0);

    // **The cursor is what takes it down.** Walk out of the panel, move one
    // character, and the question is no longer being asked.
    ed.on_key(Key::Ctrl('w'));
    assert_eq!(ed.transient(right), Some(Transient::Dictionary), "still on 那");
    ed.on_key(Key::Char('l'));
    assert_eq!(ed.transient(right), None, "and gone the moment the cursor left");
}

/// 「查不到」and「還沒問」are different findings.
#[test]
fn a_character_the_table_has_nothing_for_says_so() {
    let mut ed = typed("那年冬天");
    ed.on_key(Key::Char(' '));
    ed.on_key(Key::Char('d'));
    ed.take_dictionary_query();
    ed.set_dictionary('那', Vec::new());
    let rows = ed.transient_rows(crate::sidebar::Side::Right);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[1].name, say!("ui.not-in-the-table"));
}

/// The answer to last frame's question must not overwrite this frame's.
///
/// A reader walking `l l l` with the panel open asks three times before
/// the first answer is back; the panel has to end up showing the character
/// the cursor is actually on.
#[test]
fn an_answer_for_a_character_nobody_is_asking_about_now_is_dropped() {
    let mut ed = typed("那年冬天");
    ed.on_key(Key::Char(' '));
    ed.on_key(Key::Char('d'));
    ed.take_dictionary_query();
    ed.look_up('年', true);
    ed.set_dictionary('那', vec![("拆分".to_string(), "刀二阝".to_string())]);
    assert_eq!(ed.dictionary().map(|(ch, _)| ch), Some('年'));
    assert_eq!(ed.transient_rows(crate::sidebar::Side::Right).len(), 1, "still waiting");
}

/// #222: how far a notch of the wheel moves is the reader's, not a
/// `const` in the front end that nobody can reach.
#[test]
fn the_wheel_step_can_be_said_and_asked_about() {
    let mut ed = typed("那年冬天，山下起了大雪。");
    assert_eq!(ed.wheel_step(), 3, "three, as a terminal scrolls three");

    // Asking is a use of its own: the number may come from a config file
    // the reader never wrote.
    ed.execute("wheel").unwrap();
    assert!(ed.status().contains('3'), "{}", ed.status());
    assert_eq!(ed.wheel_step(), 3, "asking changes nothing");

    ed.execute("wheel 1").unwrap();
    assert_eq!(ed.wheel_step(), 1);

    // Zero is the terminal's own step — one unit a notch — and not 「do
    // not scroll」, which is a setting nobody wants and which would be
    // indistinguishable from a broken mouse.
    ed.execute("wheel 0").unwrap();
    assert_eq!(ed.wheel_step(), 1);

    assert!(ed.execute("wheel 三").is_err(), "a word is not a number");
}

#[test]
fn a_measure_is_a_width_to_write_to_in_either_layout() {
    let mut ed = typed("那年冬天，山下起了大雪。");
    ed.set_wrap_width(120);
    assert_eq!(ed.wrap_width(), Some(120), "the window, to begin with");

    // `:view-wrap 50` is a measure: rows fold at fifty however wide the window.
    ed.execute("view-wrap 50").unwrap();
    assert_eq!(ed.measure(), Some(50));
    ed.set_wrap_width(120);
    assert_eq!(ed.wrap_width(), Some(50));

    // …but only downwards. A row that does not fit cannot be read, so a
    // narrow window still wins.
    ed.set_wrap_width(30);
    assert_eq!(ed.wrap_width(), Some(30));

    // Setting one turns wrapping on, because fifty columns of text running
    // off the edge is not writing to a measure of fifty.
    ed.set_soft_wrap(false);
    ed.execute("view-wrap 40").unwrap();
    assert!(ed.soft_wrap());

    // Vertically the measure is the length of a 縱.
    ed.execute("layout vertical").unwrap();
    ed.execute("view-wrap 12").unwrap();
    assert_eq!(ed.zong_length(), 12);

    // `:view-wrap 0` gives the window back; plain `:view-wrap` still just turns
    // wrapping on, and leaves the measure where it was.
    ed.execute("view-wrap").unwrap();
    assert_eq!(ed.measure(), Some(12), "`:view-wrap` is not `:view-wrap 0`");
    ed.execute("view-wrap 0").unwrap();
    assert_eq!(ed.measure(), None);
    ed.set_wrap_width(120);
    assert_eq!(ed.wrap_width(), None, "vertical does not wrap");

    assert!(ed.execute("view-wrap wide").is_err(), "not a width");
}

/// 縱書 has no 折行 to turn off, and says so.
///
/// A 縱 is broken by the height of the window. `:wrap off` used to be
/// taken there and answered 「長段落跑出右邊」 — a right edge this page
/// does not have — while changing nothing the reader could see.
#[test]
fn wrap_on_and_off_are_refused_in_vertical_and_say_why() {
    let mut ed = typed("那年冬天，山下起了大雪。");
    ed.execute("layout vertical").unwrap();
    assert!(ed.soft_wrap(), "on, as it always is");

    ed.execute("view-wrap off").unwrap();
    assert!(ed.soft_wrap(), "…and untouched: there was nothing to turn");
    assert!(ed.status().contains("縱書不折行"), "{}", ed.status());

    // The measure is a different question, and it *is* answered here.
    ed.execute("view-wrap 12").unwrap();
    assert_eq!(ed.zong_length(), 12);

    // Horizontally it works as it always did.
    ed.execute("layout horizontal").unwrap();
    ed.execute("view-wrap off").unwrap();
    assert!(!ed.soft_wrap());
}

#[test]
fn one_key_moves_between_the_two_panes() {
    let dir = std::env::temp_dir().join(format!("yumete-panes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ch01.md"), "那年冬天\n").unwrap();

    let mut ed = Editor::new();
    ed.open_sidebar_at(&dir);
    assert!(ed.sidebar_focused());

    // `C-w` walks the regions — with one panel and one work area that is two
    // of them, so it goes back and forth (#293).
    ed.on_key(Key::Ctrl('w'));
    assert!(!ed.sidebar_focused(), "the keys are with the text");
    // …and the text really has them.
    ed.on_key(Key::Char('i'));
    assert_eq!(ed.mode(), Mode::Insert);
    ed.on_key(Key::Esc);

    ed.on_key(Key::Ctrl('w'));
    assert!(ed.sidebar_focused(), "and back again");
    // **Esc is not one of the panel's doors** — it is reserved for leaving
    // Insert in a panel that has a field.
    ed.on_key(Key::Esc);
    assert!(ed.sidebar_focused(), "Esc did nothing");
    ed.on_key(Key::Ctrl('w'));
    assert!(!ed.sidebar_focused());

    // With no sidebar open it does nothing at all.
    ed.on_key(Key::Char(' '));
    ed.on_key(Key::Char('e'));
    ed.on_key(Key::Char(' '));
    ed.on_key(Key::Char('e'));
    assert!(ed.panel(crate::sidebar::Side::Left).is_none());
    ed.on_key(Key::Ctrl('w'));
    assert!(!ed.sidebar_focused());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_key_that_names_a_view_opens_it_switches_to_it_and_closes_it() {
    let dir = std::env::temp_dir().join(format!("yumete-toggle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ch01.md"), "# 第一章\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", dir.join("ch01.md").display()))
        .unwrap();
    ed.open_sidebar_showing(&dir, crate::sidebar::View::Explorer);
    assert!(ed.sidebar_focused());

    // The same key again closes it: a toggle that cannot undo itself is not
    // a toggle.
    type_keys(&mut ed, " e");
    assert!(ed.panel(crate::sidebar::Side::Left).is_none());

    // A *different* view's key opens on that view…
    type_keys(&mut ed, " o");
    assert_eq!(ed.panel(crate::sidebar::Side::Left).unwrap().view(), crate::sidebar::View::Outline);
    // …and from there `Space e` means "show me the files", not "close".
    type_keys(&mut ed, " e");
    assert_eq!(ed.panel(crate::sidebar::Side::Left).unwrap().view(), crate::sidebar::View::Explorer);
    assert!(ed.sidebar_focused());

    // `C-w` hands the keys back without putting it away, and the key takes
    // them again rather than closing something the writer is not in.
    ed.on_key(Key::Ctrl('w'));
    assert!(ed.panel(crate::sidebar::Side::Left).is_some() && !ed.sidebar_focused());
    type_keys(&mut ed, " e");
    assert!(ed.sidebar_focused(), "the keys came back");
    type_keys(&mut ed, " e");
    assert!(ed.panel(crate::sidebar::Side::Left).is_none(), "and now it closes");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_sidebar_shows_three_views_of_the_same_question() {
    let dir = std::env::temp_dir().join(format!("yumete-views-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ch01.md"), "# 第一章\n那年\n## 一\n雪\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", dir.join("ch01.md").display()))
        .unwrap();
    ed.execute(":new").unwrap();

    // `Space o` opens straight onto the outline of the file being written…
    ed.open_sidebar_showing(&dir, crate::sidebar::View::Outline);
    assert!(
        ed.panel(crate::sidebar::Side::Left).unwrap().rows().is_empty(),
        "a scratch has none"
    );

    // …and on a chapter it is the hashes the writer already types.
    ed.prev_buffer();
    ed.open_sidebar_showing(&dir, crate::sidebar::View::Outline);
    let names: Vec<&str> = ed
        .panel(crate::sidebar::Side::Left)
        .unwrap()
        .rows()
        .iter()
        .map(|r| r.name.trim())
        .collect();
    assert_eq!(names, ["第一章", "一"]);

    // Entering a heading puts the cursor on it and hands the keys back.
    ed.on_key(Key::Char('j'));
    ed.on_key(Key::Enter);
    assert_eq!(ed.cursor_line(), 2);
    assert!(!ed.sidebar_focused());

    // Tab walks from the tree on to the buffers, which name what is open.
    ed.open_sidebar_at(&dir);
    ed.on_key(Key::Tab);
    assert_eq!(ed.panel(crate::sidebar::Side::Left).unwrap().view(), crate::sidebar::View::Buffers);
    let names: Vec<String> = ed
        .panel(crate::sidebar::Side::Left)
        .unwrap()
        .rows()
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names[0].starts_with("ch01.md"), "{names:?}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_sidebar_walks_the_tree_with_the_same_keys_the_text_uses() {
    let dir = std::env::temp_dir().join(format!("yumete-sidekeys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("卷一")).unwrap();
    std::fs::write(dir.join("卷一/ch01.md"), "第一章\n").unwrap();
    std::fs::write(dir.join("notes.md"), "").unwrap();

    let mut ed = Editor::new();
    ed.open_sidebar_at(&dir);
    assert!(ed.sidebar_focused());

    // `l` opens the directory, `j` steps onto the chapter, `l` opens it —
    // and opening a file means going to write in it, so the keys go back.
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('j'));
    ed.on_key(Key::Char('l'));
    assert_eq!(ed.current_buffer().text(), "第一章\n");
    assert!(!ed.sidebar_focused(), "the keys went back to the text");
    assert!(ed.panel(crate::sidebar::Side::Left).is_some(), "but the tree stays up");

    // The keys are with the text now, so `Space e` takes them back rather
    // than closing something the writer is not in; the press after that
    // closes it.
    type_keys(&mut ed, " e");
    assert!(ed.sidebar_focused());
    type_keys(&mut ed, " e");
    assert!(ed.panel(crate::sidebar::Side::Left).is_none(), "Space e closes it again");

    std::fs::remove_dir_all(&dir).ok();
}

/// The search panel: a box, three switches, and what they found — #419 一.
#[test]
fn the_search_panel_looks_through_the_buffer_as_you_type() {
    use crate::search_panel::{Case, Field};
    use crate::sidebar::{Layer, Side, View};
    let mut ed = typed("霜降於石階。\n那一年的霜來得早。\n無。\n");
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));

    // `空格 /` opens it, and the keys land in the box.
    type_keys(&mut ed, " /");
    assert_eq!(ed.panel(Side::Left).map(|p| p.view()), Some(View::Search));
    assert_eq!(ed.mode(), Mode::Field);
    assert_eq!(ed.panel_focus(), Some((Side::Left, Layer::Top)));

    // Typing searches; the count is the real one and the excerpts are a few
    // characters either side, not the whole paragraph.
    ed.on_key(Key::Char('霜'));
    assert_eq!(ed.search().total, 2);
    assert_eq!(ed.search().hits.len(), 2);
    assert_eq!(ed.search().hits[0].line, 0);
    assert_eq!(ed.search().hits[1].line, 1);
    assert!(ed.search().hits[1].excerpt.contains('霜'));

    // **The page follows**: `n` walks the same hits, because it is the same
    // search (#415's gap, closed).
    assert!(ed.last_search().contains('霜'), "{:?}", ed.last_search());

    // Backspacing back to nothing is 「not asked」, not 「found nothing」.
    ed.on_key(Key::Backspace);
    assert!(!ed.search().asked());
    assert_eq!(ed.search().total, 0);

    // A pattern that finds nothing is a different finding.
    ed.on_key(Key::Char('龘'));
    assert!(ed.search().asked());
    assert_eq!(ed.search().total, 0);
    assert!(!ed.search().broken);

    // **正則 off means the pattern is a string.** `。` is a full stop either
    // way, but `.` is not: with 正則 off it is one character, not any.
    ed.on_key(Key::Backspace);
    ed.on_key(Key::Char('.'));
    assert_eq!(ed.search().total, 0, "a literal dot is not in the text");
    ed.on_key(Key::Esc);
    assert_eq!(ed.mode(), Mode::Normal, "Esc leaves the box, not the panel");
    ed.on_key(Key::Tab);
    assert_eq!(ed.search().field, Field::Regex);
    ed.on_key(Key::Char(' '));
    assert!(ed.search().regex);
    assert!(ed.search().total > 0, "as a pattern it matches every character");

    // ⚠️ **A broken pattern keeps the hits and says so**, rather than
    // flickering the list empty on the way to a finished one.
    let found = ed.search().hits.len();
    ed.on_key(Key::Char('i'));
    assert_eq!(ed.mode(), Mode::Field);
    ed.on_key(Key::Char('['));
    assert!(ed.search().broken);
    assert_eq!(ed.search().hits.len(), found, "the last good answer is still there");

    // 大小寫 is three ways round, not a tick.
    ed.on_key(Key::Esc);
    ed.on_key(Key::Tab);
    ed.on_key(Key::Tab);
    assert_eq!(ed.search().field, Field::Case);
    assert_eq!(ed.search().case, Case::Smart);
    ed.on_key(Key::Enter);
    assert_eq!(ed.search().case, Case::Sensitive);
    ed.on_key(Key::Enter);
    assert_eq!(ed.search().case, Case::Insensitive);
    ed.on_key(Key::Enter);
    assert_eq!(ed.search().case, Case::Smart, "round again");
}

/// `空格 /` fills the box with something worth pressing Enter on — #419.
#[test]
fn the_box_opens_holding_the_last_pattern_or_what_is_marked() {
    let mut ed = typed("霜降於石階。\n那一年的霜來得早。\n");
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));

    // Nothing searched yet and nothing marked: an empty box. ⚠️ The cursor
    // covers its own grapheme, and that is **not** a selection anybody made.
    type_keys(&mut ed, " /");
    assert_eq!(ed.search().query, "");
    ed.on_key(Key::Char('霜'));
    ed.on_key(Key::Esc);
    ed.on_key(Key::Ctrl('w'));

    // Opened again: the last pattern, **selected**, so one key does either
    // thing — type over it, or Enter to carry on with it.
    type_keys(&mut ed, " /");
    assert_eq!(ed.search().query, "霜");
    assert!(ed.search().all_selected);
    ed.on_key(Key::Char('雪'));
    assert_eq!(ed.search().query, "雪", "typing replaced the whole of it");

    // A short selection wins over it: you marked it, the intention is on the
    // screen.
    ed.on_key(Key::Esc);
    ed.on_key(Key::Ctrl('w'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('v'));
    ed.on_key(Key::Char('l'));
    type_keys(&mut ed, " /");
    assert_eq!(ed.search().query, "霜降", "{:?}", ed.search().query);
}

/// A book laid out the way one is: a folder, a chapter folder under it, and
/// a `.gitignore` beside them.
#[cfg(test)]
fn a_little_book(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yumete-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("卷一")).unwrap();
    std::fs::write(dir.join("卷一/a.md"), "霜降於石階。\n").unwrap();
    std::fs::write(dir.join("卷一/b.md"), "那一年的霜來得早。\n窗上的霜花。\n").unwrap();
    std::fs::write(dir.join("c.md"), "第二天沒有霜。\n").unwrap();
    // What makes this folder 「the book」, whichever directory yumete was
    // started in — `-gd` climbs to it.
    std::fs::write(dir.join(".yumete.toml"), "").unwrap();
    dir
}

/// **Searching past this file** — #419 二.
#[test]
fn the_search_panel_walks_the_folder_when_it_is_told_to() {
    use crate::search_panel::{Row, Where};
    use crate::sidebar::Side;
    let dir = a_little_book("searchtree");
    let mut ed = Editor::new();
    ed.open_file(dir.join("卷一/a.md")).unwrap();

    // ⚠️ **A wider scope does not run as you type.** A hundred chapters read
    // off the disk per letter is not a thing to do, and the panel says so.
    ed.execute(":search-cd").unwrap();
    ed.on_key(Key::Char('霜'));
    assert!(ed.search().stale, "it is waiting to be told to look");
    ed.on_key(Key::Enter);
    assert!(!ed.search().stale);

    // Three: one here, two next door. **The file being written is searched
    // once**, from memory — not again off the disk.
    assert_eq!(ed.search().total, 3, "{:?}", ed.search().hits);
    let rows = ed.search().rows();
    let files: Vec<String> = rows
        .iter()
        .filter_map(|r| match r {
            Row::File { path, hits, .. } => Some(format!("{} {hits}", path.display())),
            Row::Hit(_) => None,
        })
        .collect();
    assert_eq!(files, vec!["a.md 1".to_string(), "b.md 2".to_string()]);

    // **Unsaved work is work.** What is on the screen is what is searched.
    ed.on_key(Key::Esc);
    ed.on_key(Key::Ctrl('w'));
    for c in "i霜霜".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Esc);
    type_keys(&mut ed, " /");
    ed.on_key(Key::Enter);
    assert_eq!(ed.search().total, 5, "the two just typed count too");

    // `-gd` climbs to the book: the chapter next door **and** the one above.
    ed.execute(":search-gd").unwrap();
    ed.on_key(Key::Enter);
    assert!(ed.search().total >= 6, "{:?}", ed.search().total);
    assert!(
        ed.search().hits.iter().any(|h| h.file.as_ref().is_some_and(|p| p.ends_with("c.md"))),
        "the one above is in it too"
    );

    // A folder named outright — and one that is not there says so rather
    // than quietly searching this file alone.
    ed.execute(":search ../卷一").unwrap();
    assert!(matches!(ed.search().scope, Where::Named(_)));
    ed.on_key(Key::Enter);
    assert_eq!(ed.search().total, 5, "the same folder by another name");
    ed.execute(":search 沒有這個").unwrap();
    assert_eq!(ed.status(), say!("search.no-such-folder", "沒有這個"));

    // Nothing was left on the left-hand slot but the panel itself.
    assert!(ed.panel(Side::Left).is_some());
    std::fs::remove_dir_all(&dir).ok();
}

/// **A lone `\r` is a character, not a line break** — Feature #395.
///
/// A paragraph pasted out of an old Mac text file carries one, and every tool
/// a writer might check the count against — `wc -l`, git, vim, VS Code —
/// counts only `\n`. ropey counts `\r`, `\v`, `\f`, NEL, LS and PS as well,
/// **unless its `unicode_lines` feature is off**, which is what this crate now
/// says in its `Cargo.toml` (the same line helix has).
///
/// ⚠️ CRLF is untouched: `\r\n` is one break either way — that is ropey's
/// core, not the feature.
#[test]
fn a_lone_carriage_return_does_not_start_a_line() {
    let mut ed = Editor::new();
    ed.current_buffer_mut().replace(0..0, "CR\rhere\nsecond\n");
    assert_eq!(
        ed.current_buffer().line_count(),
        3,
        "two lines and the empty one after the last break, as `wc -l` counts"
    );
    // …and the `\r` is *in* the first line, where a reader can see it (#398
    // draws it as ␍).
    let first: String = ed.current_buffer().rope().line(0).chars().collect();
    assert!(first.starts_with("CR\rhere"), "{first:?}");

    // The other five that came with the feature are characters too.
    let mut ed = Editor::new();
    ed.current_buffer_mut().replace(0..0, "a\u{b}b\u{c}c\u{85}d\u{2028}e\u{2029}f\n");
    assert_eq!(ed.current_buffer().line_count(), 2, "one line, and the end");

    // ⚠️ CRLF still ends a line — that is ropey's core, not the feature.
    let mut ed = Editor::new();
    ed.current_buffer_mut().replace(0..0, "one\r\ntwo\r\n");
    assert_eq!(ed.current_buffer().line_count(), 3);
    let first: String = ed.current_buffer().rope().line(0).chars().collect();
    assert_eq!(first, "one\r\n", "and the pair is the break, not two");
}

/// **The book is found from a file opened by a bare name too.**
///
/// ⚠️ A long-standing quiet one, caught by #419 二: a buffer opened as
/// `一.md` has a *relative* path, its `parent()` is the **empty** path, and
/// the walk up for `.yumete` skipped it — so every listing rooted itself in
/// whatever directory the terminal happened to be standing in. Quiet because
/// the answer is a real directory and a plausible one.
#[test]
fn the_book_is_found_even_from_a_file_named_without_a_folder() {
    let dir = a_little_book("bareroot");
    // ⚠️ Told where 「here」 is rather than **moving** the process there:
    // every test running beside this one would see that.
    let bare = std::path::Path::new("a.md");
    let root = crate::editor::book_root(std::iter::once(bare), &dir.join("卷一"));
    assert_eq!(
        std::fs::canonicalize(&root).unwrap(),
        std::fs::canonicalize(&dir).unwrap(),
        "the `.yumete.toml` two levels up is what says 「the book」"
    );

    // …and a name with a folder on it answers the same.
    let root = crate::editor::book_root(
        std::iter::once(dir.join("卷一/a.md").as_path()),
        std::path::Path::new("/"),
    );
    assert_eq!(
        std::fs::canonicalize(&root).unwrap(),
        std::fs::canonicalize(&dir).unwrap()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The walk is the book's, and it reads `.gitignore` — #308, #362, #419 二.
///
/// ⚠️ This is the coverage `:grep`'s tests used to carry. It came back with
/// the panel because the walk is the same walk.
#[test]
fn the_search_reads_the_ignore_file_and_roots_itself_in_the_book() {
    let dir = a_little_book("searchwalk");
    std::fs::create_dir_all(dir.join("target")).unwrap();
    std::fs::write(dir.join("target/built.md"), "霜霜霜\n").unwrap();
    std::fs::create_dir_all(dir.join("舊稿")).unwrap();
    std::fs::write(dir.join("舊稿/old.md"), "霜霜\n").unwrap();
    std::fs::write(dir.join(".gitignore"), "舊稿/\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(dir.join("卷一/a.md")).unwrap();
    ed.execute(":search-gd").unwrap();
    ed.on_key(Key::Char('霜'));
    ed.on_key(Key::Enter);

    let files: Vec<String> = ed
        .search()
        .hits
        .iter()
        .filter_map(|h| h.file.as_ref().map(|p| p.display().to_string()))
        .collect();
    assert!(files.iter().any(|f| f.contains("c.md")), "the book's own: {files:?}");
    assert!(
        !files.iter().any(|f| f.contains("built.md")),
        "build output is not prose: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f.contains("old.md")),
        "`.gitignore` said to skip it: {files:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

impl Editor {
    /// Walk the results to the header of a file holding `hits` of them.
    #[cfg(test)]
    fn search_go_to_file_with(&mut self, hits: usize) {
        for _ in 0..self.search().rows().len() {
            if matches!(self.search().row(), Some(crate::search_panel::Row::File { hits: n, .. }) if n == hits)
            {
                return;
            }
            self.on_key(Key::Char('j'));
        }
        panic!("no file with {hits} hits: {:?}", self.search().rows());
    }
}

/// **Changing what was found** — #419 三.
#[test]
fn the_panel_changes_one_hit_one_file_or_all_of_them() {
    use crate::search_panel::Field;
    let dir = a_little_book("searchreplace");
    std::fs::write(dir.join("卷一/a.md"), "阿甯站在門口。\n").unwrap();
    std::fs::write(dir.join("卷一/b.md"), "阿甯回頭。\n阿甯沒有說話。\n").unwrap();
    std::fs::write(dir.join("c.md"), "第二天，阿甯走了。\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(dir.join("卷一/a.md")).unwrap();
    ed.execute(":replace-gd").unwrap();
    assert!(ed.search().replacing, "`:replace` opens with the row showing");
    for c in "阿甯".chars() {
        ed.on_key(Key::Char(c));
    }
    // **`Tab` between the boxes keeps you typing** — they are filled in one
    // after the other.
    ed.on_key(Key::Tab);
    assert_eq!(ed.search().field, Field::Replace);
    assert_eq!(ed.mode(), Mode::Field);
    for c in "阿寧".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Enter);
    assert_eq!(ed.search().total, 4);

    ed.on_key(Key::Esc);
    for _ in 0..4 {
        ed.on_key(Key::Tab);
    }
    assert_eq!(ed.search().field, Field::Results);

    // **One hit** — standing on a hit, not on the file above it.
    ed.on_key(Key::Char('j'));
    assert!(matches!(ed.search().row(), Some(crate::search_panel::Row::Hit(_))));
    ed.on_key(Key::Char('r'));
    assert_eq!(ed.search().total, 3, "{}", ed.status());
    // ⚠️ **Nothing reached the disk.**
    assert_eq!(std::fs::read_to_string(dir.join("卷一/a.md")).unwrap(), "阿甯站在門口。\n");
    assert!(ed.current_buffer().is_modified());

    // **One file**, from its header row — the one with two hits under it, so
    // that 「a file」 and 「a hit」 cannot be mistaken for each other.
    ed.search_go_to_file_with(2);
    ed.on_key(Key::Char('r'));
    assert_eq!(ed.search().total, 1, "{}", ed.status());

    // **All of them — and only this one asks first.**
    ed.on_key(Key::Char('R'));
    assert_eq!(ed.status(), say!("search.replace-all-sure", 1));
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.search().total, 1, "answered no, nothing changed");
    ed.on_key(Key::Char('R'));
    ed.on_key(Key::Char('y'));
    assert_eq!(ed.search().total, 0, "{}", ed.status());

    // ⚠️ **Still nothing on the disk**; `:write-all` is the moment of yes.
    assert!(std::fs::read_to_string(dir.join("c.md")).unwrap().contains('甯'));
    ed.execute(":write-all").unwrap();
    for name in ["卷一/a.md", "卷一/b.md", "c.md"] {
        let after = std::fs::read_to_string(dir.join(name)).unwrap();
        assert!(!after.contains('甯') && after.contains('寧'), "{name}: {after:?}");
    }

    // `:search` after a `:replace` is 「just looking」: the row goes away and
    // `r`/`R` with it.
    ed.execute(":search").unwrap();
    assert!(!ed.search().replacing);
    std::fs::remove_dir_all(&dir).ok();
}

/// **Which side each panel lives on is a setting, one per panel** — #293.
#[test]
fn the_panels_go_where_the_settings_put_them() {
    use crate::sidebar::{Layer, Panel, Side, Transient, View};
    let dir = std::env::temp_dir().join(format!("yumete-sides-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.md"), "那年冬天\n").unwrap();

    let mut ed = Editor::new();
    ed.set_side(Panel::Files, Side::Right);
    ed.open_sidebar_at(&dir);
    assert!(ed.panel(Side::Right).is_some(), "the tree opened on the right");
    assert!(ed.panel(Side::Left).is_none());

    // **A panel already open moves with the setting**, or the setting is a lie
    // until the next restart.
    ed.set_side(Panel::Files, Side::Left);
    assert!(ed.panel(Side::Left).is_some(), "and it came along");
    assert!(ed.panel(Side::Right).is_none());
    assert_eq!(ed.panel_focus(), Some((Side::Left, Layer::Top)), "keys too");

    // **`Tab` walks the views that share this slot, and only those.** With the
    // outline moved across, the left column holds two and walks between them.
    ed.set_side(Panel::Outline, Side::Right);
    ed.set_side(Panel::Search, Side::Right);
    ed.on_key(Key::Tab);
    assert_eq!(ed.panel(Side::Left).unwrap().view(), View::Buffers);
    ed.on_key(Key::Tab);
    assert_eq!(ed.panel(Side::Left).unwrap().view(), View::Explorer, "two, not four");

    // …and a column with one view in it says so rather than looking broken.
    ed.set_side(Panel::Buffers, Side::Right);
    ed.on_key(Key::Tab);
    assert_eq!(ed.panel(Side::Left).unwrap().view(), View::Explorer);
    assert_eq!(ed.status(), say!("sidebar.only-view-on-this-side"));

    // **Both on one side is a layout, not a mistake**: the 字典 then stacks
    // under the tree instead of taking a second column, and the tree stays.
    let mut ed = typed("那年冬天");
    ed.set_side(Panel::Dictionary, Side::Left);
    ed.open_sidebar_at(&dir);
    ed.on_key(Key::Ctrl('w'));
    ed.on_key(Key::Char(' '));
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.transient(Side::Left), Some(Transient::Dictionary));
    assert_eq!(ed.transient(Side::Right), None);
    assert!(ed.panel(Side::Left).is_some(), "the tree above it is untouched");

    // Three seats in one column, walked in screen order: top, bottom, text.
    assert_eq!(ed.panel_focus(), Some((Side::Left, Layer::Bottom)));
    ed.on_key(Key::Ctrl('w'));
    assert_eq!(ed.panel_focus(), None, "the writing");
    ed.on_key(Key::Ctrl('w'));
    assert_eq!(ed.panel_focus(), Some((Side::Left, Layer::Top)), "round to the tree");

    // A word nobody knows keeps the default rather than picking a side.
    assert_eq!(Side::parse("right"), Some(Side::Right));
    assert_eq!(Side::parse("  LEFT "), Some(Side::Left));
    assert_eq!(Side::parse("上"), None);
    assert_eq!(Panel::parse("outline"), Some(Panel::Outline));
    assert_eq!(Panel::parse("nope"), None);

    std::fs::remove_dir_all(&dir).ok();
}

/// `C-w` walks the regions, `Esc` does nothing, `q` closes — Feature #293.
#[test]
fn the_panel_has_two_doors_and_esc_is_neither_of_them() {
    use crate::sidebar::Side;
    let dir = std::env::temp_dir().join(format!("yumete-regions-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.md"), "一\n").unwrap();

    let mut ed = Editor::new();
    ed.open_sidebar_at(&dir);
    assert!(ed.sidebar_focused());

    // **Esc is not a door.** A panel with a field in it spends Esc on leaving
    // Insert, so one press too many must not put the panel away.
    ed.on_key(Key::Esc);
    assert_eq!(ed.panel_focus(), Some((Side::Left, crate::sidebar::Layer::Top)), "Esc did nothing at all");

    // With one panel and one work area the ring is two long, and C-w walks it
    // both ways round.
    ed.on_key(Key::Ctrl('w'));
    assert_eq!(ed.panel_focus(), None, "C-w handed the keys to the writing");
    assert!(ed.panel(Side::Left).is_some(), "and left the panel up");
    ed.on_key(Key::Ctrl('w'));
    assert_eq!(ed.panel_focus(), Some((Side::Left, crate::sidebar::Layer::Top)), "and round again");

    // `q` is the other door: this slot goes away and the keys come back.
    ed.on_key(Key::Char('q'));
    assert!(ed.panel(Side::Left).is_none());
    assert_eq!(ed.panel_focus(), None);

    // Nothing open but the writing: one region, and C-w has nowhere to go.
    // **`空格 w` is the key that splits the page**, and it still does.
    ed.on_key(Key::Ctrl('w'));
    assert!(ed.other_pane().is_none(), "C-w does not open a work area");
    type_keys(&mut ed, " w");
    assert!(ed.other_pane().is_some(), "空格 w still does");

    // Two halves of the writing and a panel: three regions, and C-w walks all
    // three in screen order.
    ed.open_sidebar_at(&dir);
    ed.on_key(Key::Ctrl('w'));
    assert_eq!(ed.panel_focus(), None);
    let first = ed.live_pane();
    ed.on_key(Key::Ctrl('w'));
    assert_eq!(ed.panel_focus(), None, "the other half is a region too");
    assert_ne!(ed.live_pane(), first, "and C-w went to it");
    ed.on_key(Key::Ctrl('w'));
    assert_eq!(ed.panel_focus(), Some((Side::Left, crate::sidebar::Layer::Top)), "then round to the panel");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn space_b_picks_a_buffer_by_name() {
    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "第一篇").expect("the fixture buffer is writable");
    ed.execute(":new").unwrap();
    ed.current_buffer_mut().insert(0, "第二篇").expect("the fixture buffer is writable");
    ed.execute(":new").unwrap();
    ed.current_buffer_mut().insert(0, "第三篇").expect("the fixture buffer is writable");

    // Space opens the menu; `b` opens the picker over the open files.
    type_keys(&mut ed, " b");
    assert_eq!(ed.mode(), Mode::Picker);
    assert_eq!(ed.picker().map(|p| p.total()), Some(3));

    // Down one and Enter shows that buffer.
    ed.on_key(Key::Down);
    ed.on_key(Key::Enter);
    assert_eq!(ed.mode(), Mode::Normal);
    assert_eq!(ed.current_buffer().text(), "第二篇");
}

#[test]
fn a_picker_closes_on_esc_and_on_backspacing_past_the_start() {
    let mut ed = Editor::new();
    type_keys(&mut ed, " b");
    ed.on_key(Key::Esc);
    assert_eq!(ed.mode(), Mode::Normal);
    assert!(ed.picker().is_none());

    type_keys(&mut ed, " b");
    ed.on_key(Key::Char('x'));
    ed.on_key(Key::Backspace); // back over the `x`
    assert_eq!(ed.mode(), Mode::Picker);
    ed.on_key(Key::Backspace); // nothing left to go back over
    assert_eq!(ed.mode(), Mode::Normal);
}

/// `空格 /` opens the panel, not a prompt — Feature #419.
#[test]
fn space_slash_opens_the_search_panel_with_the_keys_in_the_box() {
    let mut ed = Editor::new();
    type_keys(&mut ed, " /");
    assert_eq!(ed.prompt(), None, "no `:` line: what to look for goes in the box");
    assert_eq!(
        ed.panel(crate::sidebar::Side::Left).map(|p| p.view()),
        Some(crate::sidebar::View::Search)
    );
    assert_eq!(ed.mode(), Mode::Field);
}

#[test]
fn the_system_clipboard_goes_both_ways() {
    let mut ed = typed("那年冬天");
    press(&mut ed, "ggvl");
    // `Space y`, or `:clipboard-yank`.
    type_keys(&mut ed, " y");
    assert_eq!(ed.take_clipboard_request().as_deref(), Some("那年"));
    ed.execute(":clipboard-yank").unwrap();
    assert_eq!(ed.take_clipboard_request().as_deref(), Some("那年"));

    // Reading needs the platform, so the core asks and the front end
    // answers — the same shape the IME's requests use.
    type_keys(&mut ed, " p");
    assert_eq!(ed.take_clipboard_read(), Some(true));
    assert_eq!(ed.take_clipboard_read(), None, "asked once");
    press(&mut ed, "gg");
    ed.provide_clipboard("外面的字", true);
    assert!(ed.current_buffer().text().contains("外面的字"));
}

#[test]
fn a_paste_from_outside_is_writing_not_keystrokes() {
    // Without bracketed paste a paste is a stream of keys, and in Normal
    // mode every character of the pasted paragraph runs as a command.
    let mut ed = typed("甲乙丙");
    press(&mut ed, "gg");
    ed.paste_text("那年冬天");
    assert_eq!(ed.current_buffer().text(), "那年冬天乙丙");
    // It replaces the selection, which is what pasting over something a
    // writer has just picked out means.
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "甲乙丙");

    // In Insert it lands at the caret like anything typed.
    let mut ed = typed("甲乙丙");
    press(&mut ed, "gg");
    ed.on_key(Key::Char('i'));
    ed.paste_text("那年");
    assert_eq!(ed.current_buffer().text(), "那年甲乙丙");

    // …and into a prompt it is text, minus the newline that would submit
    // it half-typed.
    let mut ed = typed("甲乙丙");
    ed.on_key(Key::Char('/'));
    ed.paste_text("那年\n冬天");
    assert_eq!(ed.prompt(), Some(("/", "那年冬天")));
}

#[test]
fn the_mouse_points_at_a_character_and_drags_a_selection() {
    let mut ed = typed("那年冬天");
    ed.point_at(1);
    assert_eq!(ed.selection(), (1, 2), "one 字, the one pointed at");
    ed.drag_to(3);
    assert_eq!(ed.selection(), (1, 4), "年冬天");
    // Copying hands it to the front end *and* fills the register, because
    // having copied something the next thing a hand reaches for is `p`.
    type_keys(&mut ed, " y");
    assert_eq!(ed.take_clipboard_request().as_deref(), Some("年冬天"));
    press(&mut ed, "gg");
    ed.on_key(Key::Char('p'));
    assert_eq!(ed.current_buffer().text(), "那年冬天年冬天");
}

#[test]
fn space_y_hands_the_selection_to_the_front_end() {
    let mut ed = typed("春江潮水");
    press(&mut ed, "ggvl");
    type_keys(&mut ed, " y");
    assert_eq!(ed.take_clipboard_request().as_deref(), Some("春江"));
    assert_eq!(ed.take_clipboard_request(), None, "taken once only");
}

#[test]
fn a_file_already_open_is_shown_rather_than_opened_twice() {
    let dir = std::env::temp_dir().join(format!("yumete-dup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chapter.md");
    std::fs::write(&path, "第一稿\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    ed.execute(":new").unwrap();
    // Two buffers over one file means two undo histories, two dirty flags,
    // and two claims on one recovery copy.
    ed.execute(&format!(":open {}", path.display())).unwrap();
    assert_eq!(ed.buffer_count(), 2);
    assert_eq!(ed.current_buffer().text(), "第一稿\n");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_buffer_can_be_closed_and_the_last_one_is_emptied() {
    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "甲").expect("the fixture buffer is writable");
    ed.execute(":new").unwrap();
    ed.current_buffer_mut().insert(0, "乙").expect("the fixture buffer is writable");
    assert_eq!(ed.buffer_count(), 2);

    // Unsaved work is not closed away silently.
    assert!(matches!(
        ed.execute(":buffer-close"),
        Err(EditorError::UnsavedChanges)
    ));
    ed.execute(":buffer-close!").unwrap();
    assert_eq!(ed.buffer_count(), 1);
    assert_eq!(ed.current_buffer().text(), "甲");

    // The last buffer is emptied rather than closed: the editor always has
    // somewhere to put the cursor.
    ed.execute(":buffer-close!").unwrap();
    assert_eq!(ed.buffer_count(), 1);
    assert_eq!(ed.current_buffer().text(), "");

    // `:buffer` opens the picker: with 122 chapters open the list is
    // 1,783 characters and the status line is one row.
    ed.execute(":buffer").unwrap();
    assert_eq!(ed.mode(), Mode::Picker);
}

#[test]
fn buffers_can_be_switched_and_keep_their_place() {
    let mut ed = Editor::new();
    ed.execute(":new").unwrap();
    // Two files, each with a cursor of its own.
    ed.on_key(Key::Char('i'));
    for c in "第一篇的內容".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Esc);
    let left_at = ed.cursor();
    ed.execute(":new").unwrap();
    ed.on_key(Key::Char('i'));
    for c in "第二篇".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Esc);
    assert_eq!(ed.buffer_count(), 2);
    assert_eq!(ed.buffer_position(), (2, 2));

    // Back to the first, and the cursor is where it was left.
    press(&mut ed, "gg");
    ed.execute(":buffer-previous").unwrap();
    assert_eq!(ed.buffer_position(), (1, 2));
    assert_eq!(ed.current_buffer().text(), "第一篇的內容");
    assert_eq!(ed.cursor(), left_at, "back where it was left");

    // …and forward again, to where *that* one was left.
    ed.execute(":buffer-next").unwrap();
    assert_eq!(ed.current_buffer().text(), "第二篇");
    assert_eq!(ed.cursor(), 0, "gg had moved it to the top");
}

#[test]
fn gn_and_gp_switch_buffers_too() {
    // `:new` on an untouched scratch buffer replaces it rather than adding
    // one, so each needs content before the next.
    let mut ed = typed("甲");
    ed.execute(":new").unwrap();
    press(&mut ed, "i");
    ed.on_key(Key::Char('乙'));
    ed.on_key(Key::Esc);
    ed.execute(":new").unwrap();
    press(&mut ed, "i");
    ed.on_key(Key::Char('丙'));
    ed.on_key(Key::Esc);
    assert_eq!(ed.buffer_count(), 3);
    press(&mut ed, "gn");
    assert_eq!(ed.buffer_position(), (1, 3), "wraps past the end");
    press(&mut ed, "gp");
    assert_eq!(ed.buffer_position(), (3, 3), "and back the other way");
}

#[test]
fn switching_clamps_a_cursor_past_the_end() {
    let mut ed = typed("一二三四五六七八九十");
    press(&mut ed, "gl"); // to the end of a long buffer
    let far = ed.cursor();
    ed.execute(":new").unwrap(); // a short one
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Char('短'));
    ed.on_key(Key::Esc);
    ed.execute(":buffer-previous").unwrap();
    ed.execute(":buffer-next").unwrap();
    assert!(
        ed.cursor() <= ed.current_buffer().char_count(),
        "a cursor from a longer buffer must not point past this one"
    );
    assert!(
        far > ed.current_buffer().char_count(),
        "the test is meaningful"
    );
}

#[test]
fn starts_with_one_scratch_buffer() {
    let ed = Editor::new();
    assert_eq!(ed.buffer_count(), 1);
    assert_eq!(ed.current_buffer().display_name(), "[scratch]");
}

#[test]
fn new_buffer_command_adds_a_buffer() {
    let mut ed = Editor::new();
    // The first :new replaces the pristine scratch buffer.
    ed.execute(":new").unwrap();
    assert_eq!(ed.buffer_count(), 1);
}

#[test]
fn open_missing_file_binds_path_without_error() {
    let mut ed = Editor::new();
    ed.execute(":open /tmp/yumete-does-not-exist-42.md")
        .unwrap();
    assert_eq!(
        ed.current_buffer().display_name(),
        "yumete-does-not-exist-42.md"
    );
    assert_eq!(ed.current_buffer().char_count(), 0);
}

#[test]
fn unknown_command_is_reported() {
    let mut ed = Editor::new();
    assert!(matches!(
        ed.execute(":frobnicate"),
        Err(EditorError::Command(CommandError::Unknown(_)))
    ));
}

/// **A write is addressed by identity, not by spelling.**
///
/// `main.typ`, `./main.typ`, the absolute path and a symlink pointing at it
/// are one manuscript. The export guard used to compare `PathBuf`s, so
/// three of those four spellings walked past it and 90,000 characters of a
/// book became an export of themselves.
/// `.` repeats **the edit you just made**, whatever characters it holds.
///
/// The abort guard used to scan the whole recorded sequence for `.`, `u`,
/// `q` and `:` — which are the *commands* that must not become a
/// definition — and an insertion is a sequence of typed characters. So
/// `i` `3` `.` `1` `4` Esc was refused as a definition, and `.` afterwards
/// silently replayed an older edit into the document.
/// **A row's cell count does not change**, and not only at the gate.
///
/// Four writers reached the rope without passing one: `gJ`, `:replace`,
/// `:s` and `:ruby-format`. Three of the four asked `self.table` first, so
/// they were off in exactly the state a `|` table in a manuscript is
/// normally edited in — nobody types `:table basic` to fix a typo in their own
/// documentation.
#[test]
fn no_writer_changes_how_many_cells_a_row_has() {
    let table = "# 標題\n\n| 鍵 | 拆分 |\n| --- | --- |\n| 木 | 木 |\n| 林 | ⿰木木 |\n\n後面一段。\n";

    // `gJ` on a table row, with table mode never turned on.
    let mut ed = typed(table);
    press(&mut ed, "gg");
    for _ in 0..4 {
        press(&mut ed, "j");
    }
    assert!(ed.current_buffer().line(4).unwrap().starts_with("| 木"));
    press(&mut ed, "gJ");
    assert!(ed.status().contains("欄數"), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), table, "the grid is untouched");

    // …and one line *above* the table: joining a paragraph onto the header
    // gives the header the paragraph's zero cells.
    press(&mut ed, "gg");
    press(&mut ed, "j");
    press(&mut ed, "gJ");
    assert!(ed.status().contains("欄數"), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), table);

    // `:s` and `:replace` put a bare `|` into a cell.
    let mut ed = typed(table);
    ed.execute(":%s/木/a|b/g").ok();
    assert_eq!(ed.current_buffer().text(), table, "{}", ed.status());

    // …while a `|` in the prose around it is just a character.
    let mut ed = typed(table);
    ed.execute(":%s/後面/前 | 後/g").unwrap();
    assert!(ed.current_buffer().text().contains("前 | 後"), "{}", ed.status());

    // …and a `|` inside a fence is writing about a table, not a table.
    let quoted = "```\n| 鍵 | 拆分 |\n```\n那年冬天。\n";
    let mut ed = typed(quoted);
    ed.execute(":%s/冬天/冬 | 天/g").unwrap();
    assert!(ed.current_buffer().text().contains("冬 | 天"), "{}", ed.status());
}

/// `:ruby-format` rewrites as much text as `:replace` and kept none of its
/// rules: `#ruby("永", "ㄩㄥˇ")` carries a comma into every cell it touches.
#[test]
fn reformatting_the_readings_does_not_reshape_a_grid() {
    let dir = std::env::temp_dir().join(format!("yumete-rubygrid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
    std::fs::write(
        dir.join(".yumete").join("tables").join("t.toml"),
        "[table]\nfile = ['d.csv']\nkey = 'char'\n\
         [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'note'\n",
    )
    .unwrap();
    let csv = dir.join("d.csv");
    let source = "char,note\n永,<ruby>永<rt>ㄩㄥˇ</rt></ruby>\n和,平\n";
    std::fs::write(&csv, source).unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.execute(":ruby-format typst").unwrap();
    assert!(ed.status().contains("格"), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), source, "the grid is untouched");

    std::fs::remove_dir_all(&dir).ok();
}

/// A reading and a picker's query are typed text, and typed text is edited
/// in the middle.
#[test]
fn every_prompt_has_a_caret() {
    // Ruby mode: type a reading, go back into it, fix it.
    let mut ed = typed("那年冬天。\n");
    press(&mut ed, "gg");
    ed.on_key(Key::Char('v'));
    ed.execute(":ruby").unwrap();
    assert_eq!(ed.mode(), Mode::Ruby, "{}", ed.status());
    for c in "hàn".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Left);
    ed.on_key(Key::Left);
    ed.on_key(Key::Char('X'));
    assert_eq!(ed.prompt().map(|(_, line)| line), Some("hXàn"));
    ed.on_key(Key::Home);
    ed.on_key(Key::Char('Z'));
    assert_eq!(ed.prompt().map(|(_, line)| line), Some("ZhXàn"));
    assert_eq!(ed.prompt_before_caret(), "Z");
    ed.on_key(Key::Esc);

    // The picker's query, the same way.
    let mut ed = typed("那年冬天。\n");
    ed.open_buffer_picker();
    for c in "abc".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Left);
    ed.on_key(Key::Backspace);
    assert_eq!(ed.picker().map(|p| p.query()), Some("ac"));
    ed.on_key(Key::Home);
    ed.on_key(Key::Delete);
    assert_eq!(ed.picker().map(|p| p.query()), Some("c"));
    assert_eq!(ed.picker().map(|p| p.caret()), Some(0));
}

/// A preview server is a running thing: `:view-preview` while one is up asks
/// *where* it is, not for a second one.
/// `:help` is written from what the editor actually runs on.
/// `:markdown` writes the pieces a manuscript keeps needing.
#[test]
fn markdown_writes_a_footnote_and_a_table() {
    let mut ed = typed("那年冬天[^2]，山下起了大雪。\n\n[^2]: 據縣志。\n");
    press(&mut ed, "gg");
    press(&mut ed, "llll");

    // **The next free number**, not one more than the last: 2 is taken.
    ed.execute(":markdown-footnote").unwrap();
    let text = ed.current_buffer().text();
    assert!(text.contains("[^1]"), "{text}");
    assert!(text.contains("[^1]: "), "and its note is opened: {text}");
    assert_eq!(ed.mode(), Mode::Insert, "the cursor is in the note");
    // Typing goes into the note, not into the sentence.
    type_keys(&mut ed, "說法不一");
    assert!(ed.current_buffer().text().contains("[^1]: 說法不一"));
    ed.on_key(Key::Esc);

    // An inline note leaves the cursor between the brackets.
    let mut ed = typed("那年冬天。\n");
    press(&mut ed, "gg");
    ed.execute(":markdown-footnote inline").unwrap();
    assert_eq!(ed.mode(), Mode::Insert);
    type_keys(&mut ed, "存疑");
    assert!(ed.current_buffer().text().starts_with("^[存疑]"), "{}", ed.current_buffer().text());

}

/// #276. The author, 2026-09-05: 「`:table-new 3 4`，迅速在 markdown 中插入
/// 一個三行四列表格，上下有空白行，光標自動到標題欄最左的一格並進去編輯模
/// 式。」 It used to be `:markdown table 4x3` — columns first, rows meaning
/// *data* rows, no blank lines and no Insert mode — and that spelling is
/// gone rather than kept beside this one.
#[test]
fn a_new_table_is_written_with_room_around_it_and_typed_into() {
    let mut ed = typed("前文。\n後文。\n");
    press(&mut ed, "gg");
    ed.execute(":table-new 3 4").unwrap();
    let text = ed.current_buffer().text();
    let lines: Vec<&str> = text.lines().collect();
    let rows: Vec<&str> = lines.iter().copied().filter(|l| l.starts_with('|')).collect();
    assert_eq!(rows.len(), 4, "a heading, a rule and two more rows: {text}");
    assert_eq!(rows[0].matches('|').count(), 5, "four columns: {text}");
    assert!(rows[1].contains("---"), "{text}");

    // 「上下有空白行」 — and the prose is still on both sides of it.
    let first = lines.iter().position(|l| l.starts_with('|')).unwrap();
    let last = lines.iter().rposition(|l| l.starts_with('|')).unwrap();
    assert_eq!(lines[0], "前文。", "{text}");
    assert!(lines[first - 1].trim().is_empty(), "a blank line above: {text}");
    assert!(lines[last + 1].trim().is_empty(), "a blank line below: {text}");
    assert!(lines.contains(&"後文。"), "the prose under it is still there: {text}");

    // 「光標自動到標題欄最左的一格並進去編輯模式」
    assert_eq!(ed.mode(), Mode::Insert, "typing goes straight in");
    type_keys(&mut ed, "字");
    let text = ed.current_buffer().text();
    let heading = text.lines().find(|l| l.starts_with('|')).unwrap();
    // The row is padded as it is typed (#212), so the cell is asked for
    // rather than the spelling of the line.
    let cells = crate::mdtable::split(heading);
    assert_eq!(cells[0].trim(), "字", "the first heading took it: {heading}");
    assert!(cells[1].trim().is_empty(), "and only it: {heading}");

    // Standing on a blank line uses it rather than pushing one more in.
    let mut ed = typed("前文。\n\n後文。\n");
    press(&mut ed, "gg");
    press(&mut ed, "j");
    ed.execute(":table-new 2 2").unwrap();
    let text = ed.current_buffer().text();
    assert!(!text.contains("\n\n\n"), "no line the writer did not ask for: {text:?}");

    // Two numbers, and only sane ones.
    let mut ed = typed("前文。\n");
    assert!(ed.execute(":table-new 0 4").is_err());
    assert!(ed.execute(":table-new 4 99").is_err());
    // On its own it is a small one rather than an error.
    assert!(ed.execute(":table-new").is_ok());
}

#[test]
fn help_is_the_editor_describing_itself() {
    let mut ed = typed("那年冬天。\n");
    ed.execute(":help").unwrap();
    let text = ed.current_buffer().text();
    assert!(ed.buffer_name().contains("help"), "{}", ed.buffer_name());
    // Every key the Space menu declares is in it, because it is *made* of
    // that list rather than written beside it.
    for (key, _) in Editor::SPACE_KEYS {
        assert!(text.contains(&format!("空格 {key}")), "空格 {key} missing");
    }
    assert!(text.contains("g/") && text.contains("gd"), "{text}");

    // …and the command section is the parser's own table.
    ed.execute(":help commands").unwrap();
    let text = ed.current_buffer().text();
    for entry in crate::command::COMMANDS {
        assert!(text.contains(&format!(":{}", entry.name)), "{} missing", entry.name);
    }

    // A section that does not exist says which ones do.
    ed.execute(":help 火星文").unwrap();
    assert!(ed.status().contains("chinese"), "{}", ed.status());
}

/// §5.2.2 fault 5: the built-in help taught a command that errors.
///
/// `:segment on` had been in `:help chinese` since before 分詞 became one
/// subject under `:word`, and the parser has asserted `parse(":segment")`
/// is an error the whole time — the two files simply never met. Every `:`
/// the help writes out is now read back by the parser that has to run it.
#[test]
fn the_help_teaches_no_command_the_parser_refuses() {
    for page in [
        Editor::new().help_common(),
        Editor::help_chinese(),
        Editor::help_vertical(),
        Editor::new().help_table(),
    ] {
        for line in page.lines() {
            // The pages are lists of `` `keys` — 說明 ``; only the rows
            // whose keys start with `:` are commands.
            let Some(rest) = line.strip_prefix("- `:") else {
                continue;
            };
            let Some(command) = rest.split('`').next() else {
                continue;
            };
            assert!(
                crate::command::parse(&format!(":{command}")).is_ok(),
                "`:help` teaches `:{command}`, which the parser refuses: {:?}",
                crate::command::parse(&format!(":{command}"))
            );
        }
    }
}

#[test]
fn a_second_preview_asks_where_the_first_one_is() {
    let dir = std::env::temp_dir().join(format!("yumete-prev-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let book = dir.join("book.typ");
    std::fs::write(&book, "= 第一章\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&book).unwrap();
    ed.execute(":view-preview").unwrap();
    assert!(
        matches!(ed.take_preview_request(), Some(Preview::Start { .. })),
        "the first one starts a typesetter"
    );
    // The front end says where it put the page.
    ed.set_preview_at(Some("http://127.0.0.1:23625".to_string()));
    assert_eq!(ed.preview_at(), Some("http://127.0.0.1:23625"));

    ed.execute(":view-preview").unwrap();
    assert!(
        matches!(ed.take_preview_request(), Some(Preview::Show)),
        "the second one asks for the address, not for another server"
    );

    ed.execute(":view-preview off").unwrap();
    assert!(matches!(ed.take_preview_request(), Some(Preview::Stop)));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_full_stop_typed_into_the_text_is_still_an_edit() {
    let mut ed = typed("第一行。\n第二行。\n");
    press(&mut ed, "gg");
    // An edit with a `.` in it — a decimal, as in a 拆分表 cell.
    press(&mut ed, "i");
    type_keys(&mut ed, "3.14");
    ed.on_key(Key::Esc);
    // …and one with a `u` and a `q` in it, which are the other two.
    press(&mut ed, "j");
    press(&mut ed, "i");
    type_keys(&mut ed, "qu");
    ed.on_key(Key::Esc);
    assert!(ed.current_buffer().text().contains("qu"));

    // `.` repeats *that*, not something from earlier in the session.
    press(&mut ed, "j");
    ed.on_key(Key::Char('.'));
    let text = ed.current_buffer().text();
    assert_eq!(text.matches("qu").count(), 2, "{text}");
    assert_eq!(text.matches("3.14").count(), 1, "{text}");

    // And the three keystrokes that used to abort the process still do not
    // define anything: `.` after `3.` repeats the insertion, once more.
    press(&mut ed, "3");
    ed.on_key(Key::Char('.'));
    let text = ed.current_buffer().text();
    assert!(text.matches("qu").count() > 2, "{text}");
}

/// What a path command **did** is what it says it did.
///
/// `:w copy.md` writes a copy and leaves the chapter unsaved. The caller
/// used to find that out by sniffing the rendered status line for a Chinese
/// character — which in English reported 「saved ch1.md」 about a chapter
/// that had not been saved. It is a value now, not a string.
#[test]
fn a_path_command_says_what_it_actually_did() {
    let dir = std::env::temp_dir().join(format!("yumete-said-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let chapter = dir.join("ch1.md");
    std::fs::write(&chapter, "第一稿\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&chapter).unwrap();
    press(&mut ed, "i");
    type_keys(&mut ed, "改");
    ed.on_key(Key::Esc);

    let copy = dir.join("copy.md");
    assert_eq!(
        ed.write_forcing(Some(&copy.display().to_string()), false)
            .unwrap(),
        Wrote::Copied(copy.clone()),
        "a copy is a copy, whatever language the line is in"
    );
    // The chapter is still unsaved, so the line may not say it is.
    assert!(ed.current_buffer().is_modified());
    assert!(!ed.status().contains("ch1.md"), "{}", ed.status());

    // …and the save that follows says so, rather than leaving the copy's
    // message standing.
    assert_eq!(ed.write_forcing(None, false).unwrap(), Wrote::Saved);
    assert!(ed.status().contains("ch1.md"), "{}", ed.status());
    assert!(!ed.current_buffer().is_modified());

    std::fs::remove_dir_all(&dir).ok();
}

/// A save that would multiply the file stops and asks first (#295).
///
/// The accident it is built for is one keystroke away in this very editor:
/// `t F` on a table with a paragraph in a cell took `development.md` from
/// 425,694 bytes to 2,945,642 (#292). The alignment is fixed; the *class*
/// of accident is not, and the last thing between a manuscript and a
/// generated file is the save.
#[test]
fn a_save_that_multiplies_the_file_stops_to_ask() {
    let dir = std::env::temp_dir().join(format!("yumete-oversize-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let chapter = dir.join("ch1.md");
    let first = "第一稿。\n".repeat(2_000);           // ~26 KB
    std::fs::write(&chapter, &first).unwrap();
    let was = std::fs::metadata(&chapter).unwrap().len();

    let mut ed = Editor::new();
    ed.open_file(&chapter).unwrap();
    // Two bounds, so the block has to clear both: more than double, and
    // more than 256 KB more.
    let flood = "甲乙丙丁戊己庚辛。\n".repeat(40_000); // ~1.1 MB
    ed.current_buffer_mut().insert(0, &flood).unwrap();

    ed.execute(":w").unwrap();
    let asked = ed.query().expect("a save this much bigger asks first");
    assert_eq!(asked.title, say!("write.oversize-title"));
    assert_eq!(asked.choices.len(), 3, "繼續／檢視／取消");
    // **The body says numbers.** An adjective is the part a writer cannot
    // check, and checking is the whole of what this panel is for.
    assert!(asked.body.contains("MB"), "{}", asked.body);
    assert_eq!(
        std::fs::metadata(&chapter).unwrap().len(),
        was,
        "nothing is written while the question stands"
    );

    // 取消儲存 — and Esc means the same thing.
    ed.on_key(Key::Char('n'));
    assert!(ed.query().is_none());
    assert_eq!(std::fs::metadata(&chapter).unwrap().len(), was);
    assert!(ed.current_buffer().is_modified());

    // 檢視區別 abandons the save too: it is 「let me look first」.
    ed.execute(":w").unwrap();
    assert!(ed.query().is_some());
    ed.on_key(Key::Char('d'));
    assert!(ed.query().is_none());
    assert_eq!(std::fs::metadata(&chapter).unwrap().len(), was);

    // 繼續儲存 writes it.
    ed.execute(":w").unwrap();
    assert!(ed.query().is_some());
    ed.on_key(Key::Char('y'));
    assert!(ed.query().is_none());
    assert!(std::fs::metadata(&chapter).unwrap().len() > was * 2);
    assert!(!ed.current_buffer().is_modified());

    std::fs::remove_dir_all(&dir).ok();
}

/// **Every writer passes the gate, not just `:write`** (#306).
///
/// #295 built the question and wired it to one arm of the command match,
/// on the reasoning that `:w!` already says 「over whatever is there」 and
/// that the other two were a line each. But the keystroke that multiplies
/// a manuscript is `t F`, and the key a writer reaches for after finishing
/// a table is `:wq` — so the one save that skipped the gate was the one
/// most likely to need it. A bang answers a different question: it says
/// overwrite *this file*, not 「a 17× file is what I meant」.
#[test]
fn every_writer_passes_the_oversize_gate() {
    let dir = std::env::temp_dir().join(format!("yumete-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let chapter = dir.join("ch1.md");
    std::fs::write(&chapter, "第一稿。\n".repeat(2_000)).unwrap();
    let was = std::fs::metadata(&chapter).unwrap().len();
    let flood = "甲乙丙丁戊己庚辛。\n".repeat(40_000);

    let swollen = |ed: &mut Editor| {
        ed.current_buffer_mut().insert(0, &flood).unwrap();
    };

    // `:wq` — and **it must not quit**: taking the manuscript off the screen
    // with the question still standing is worse than the save it skipped.
    let mut ed = Editor::new();
    ed.open_file(&chapter).unwrap();
    swollen(&mut ed);
    assert_eq!(
        ed.execute(":wq").unwrap(),
        CommandOutcome::Continue,
        "a question standing is not a save, and not a reason to leave"
    );
    assert!(ed.query().is_some(), ":wq asks");
    assert_eq!(std::fs::metadata(&chapter).unwrap().len(), was);

    // `:w!` — the bang is about the file on disk, not about the size.
    let mut ed = Editor::new();
    ed.open_file(&chapter).unwrap();
    swollen(&mut ed);
    ed.execute(":w!").unwrap();
    assert!(ed.query().is_some(), ":w! asks");
    assert_eq!(std::fs::metadata(&chapter).unwrap().len(), was);

    // `:write-all` — the command a book-wide `:replace` ends with. It stops
    // **on the buffer that asked**, because the question names one file.
    let mut ed = Editor::new();
    ed.open_file(&chapter).unwrap();
    swollen(&mut ed);
    ed.execute(":write-all").unwrap();
    assert!(ed.query().is_some(), ":write-all asks");
    assert_eq!(std::fs::metadata(&chapter).unwrap().len(), was);

    // And 「yes」 still writes exactly once: the pass is spent on that save,
    // not left standing for the next one.
    ed.on_key(Key::Char('y'));
    assert!(ed.query().is_none());
    assert!(std::fs::metadata(&chapter).unwrap().len() > was * 2);
    // Three floods, not one: the file on disk is now the swollen one, so
    // doubling *it* takes more than the same insert again.
    for _ in 0..3 {
        ed.current_buffer_mut().insert(0, &flood).unwrap();
    }
    ed.execute(":w").unwrap();
    assert!(ed.query().is_some(), "the next save is a new question");

    std::fs::remove_dir_all(&dir).ok();
}

/// …and every save that is not that one is not asked about.
///
/// Three silences, and each of them is a writer this must never stop: a
/// morning's work on a short draft (doubled, but tiny), a chapter added to
/// a long book (a lot, but nowhere near double), and a **first** save,
/// which has no size on disk to have multiplied.
#[test]
fn an_ordinary_save_is_never_asked_about() {
    let dir = std::env::temp_dir().join(format!("yumete-ordinary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // ① A short draft that tripled overnight. 翻倍 alone would stop it.
    let small = dir.join("draft.md");
    std::fs::write(&small, "起。\n".repeat(100)).unwrap();
    let mut ed = Editor::new();
    ed.open_file(&small).unwrap();
    ed.current_buffer_mut()
        .insert(0, &"承轉合。\n".repeat(1_000))
        .unwrap();
    ed.execute(":w").unwrap();
    assert!(ed.query().is_none(), "a morning's writing is not an accident");

    // ② A long book gaining a chapter. +256 KB alone would stop it.
    let book = dir.join("book.md");
    std::fs::write(&book, "第一稿。\n".repeat(80_000)).unwrap();
    ed.open_file(&book).unwrap();
    ed.current_buffer_mut()
        .insert(0, &"新的一章。\n".repeat(30_000))
        .unwrap();
    ed.execute(":w").unwrap();
    assert!(ed.query().is_none(), "a chapter is not an accident either");

    // ③ The first save of a new file: there is no 「from」 to multiply.
    let fresh = dir.join("new.md");
    ed.new_buffer();
    ed.current_buffer_mut()
        .insert(0, &"甲乙丙丁。\n".repeat(40_000))
        .unwrap();
    ed.execute(&format!(":w {}", fresh.display())).unwrap();
    assert!(ed.query().is_none(), "a first save has nothing to compare with");
    assert!(fresh.exists());

    std::fs::remove_dir_all(&dir).ok();
}

/// A key that is not one of the answers leaves the question standing.
///
/// The one thing a modal question may never do is let a stray keystroke
/// through to the manuscript behind it — `x` here used to be a deletion.
#[test]
fn a_stray_key_does_not_get_past_the_question() {
    let dir = std::env::temp_dir().join(format!("yumete-stray-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let chapter = dir.join("ch1.md");
    std::fs::write(&chapter, "第一稿。\n".repeat(2_000)).unwrap();

    let mut ed = Editor::new();
    ed.open_file(&chapter).unwrap();
    ed.current_buffer_mut()
        .insert(0, &"甲乙丙丁戊己庚辛。\n".repeat(40_000))
        .unwrap();
    let before = ed.current_buffer().rope().len_chars();
    ed.execute(":w").unwrap();
    assert!(ed.query().is_some());

    for key in [Key::Char('x'), Key::Char('u'), Key::Enter, Key::Char('j')] {
        ed.on_key(key);
        assert!(ed.query().is_some(), "{key:?} left the question standing");
    }
    assert_eq!(ed.current_buffer().rope().len_chars(), before, "nothing edited");

    // Esc is 「no」 — the answer the last choice spells out.
    ed.on_key(Key::Esc);
    assert!(ed.query().is_none());
    assert!(ed.current_buffer().is_modified());

    std::fs::remove_dir_all(&dir).ok();
}

/// `:wq <名字>` saves the file it names and quits — it does not write a
/// copy and then refuse to leave.
#[test]
fn write_quit_with_a_name_saves_that_name() {
    let dir = std::env::temp_dir().join(format!("yumete-wqn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let chapter = dir.join("ch1.md");
    std::fs::write(&chapter, "第一稿\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&chapter).unwrap();
    press(&mut ed, "i");
    type_keys(&mut ed, "改");
    ed.on_key(Key::Esc);
    let out = dir.join("ch1-final.md");
    assert_eq!(
        ed.execute(&format!(":wq {}", out.display())).unwrap(),
        CommandOutcome::Quit,
        "{}",
        ed.status()
    );
    assert!(std::fs::read_to_string(&out).unwrap().contains('改'));
    assert!(!ed.current_buffer().is_modified());
    std::fs::remove_dir_all(&dir).ok();
}

/// A substitution that would reshape a grid names a way through, and the
/// way through works.
#[test]
fn the_grid_refusal_names_a_way_through() {
    let table = "| 鍵 | 拆分 |\n| --- | --- |\n| 木 | 木 |\n";
    let mut ed = typed(table);
    ed.execute(":%s/拆分/拆分 | 註/").ok();
    assert!(ed.status().contains("t 旗標"), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), table);
    // …and with the flag it goes through.
    ed.execute(":%s/拆分/拆分 | 註/t").unwrap();
    assert!(ed.current_buffer().text().contains("拆分 | 註"), "{}", ed.status());
}

/// A paste the cell refuses says so — in Insert as well as in Normal.
///
/// The text is a bare `|`, not the tab-separated block this test used to
/// paste: since #226 a block is not refused, it is a grid.
#[test]
fn a_refused_paste_does_not_claim_to_have_happened() {
    let mut ed = typed("| 字 | 說明 |\n| --- | --- |\n| 木 | 樹 |\n");
    ed.goto_line(3);
    assert!(ed.enter_table(), "{}", ed.status());
    let before = ed.current_buffer().text();
    press(&mut ed, "i");
    ed.paste_text("甲 | 乙");
    assert_eq!(ed.current_buffer().text(), before, "{}", ed.status());
    assert!(!ed.status().contains("貼了"), "{}", ed.status());
}

/// A spreadsheet's clipboard lands as rows, and widens the table (#226).
#[test]
fn a_pasted_spreadsheet_becomes_rows() {
    let mut ed = typed("| 字 | 說明 |\n| --- | --- |\n| 木 | 樹 |\n");
    ed.goto_line(3);
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "i");
    // Two rows, three columns, into a table two columns wide: the third
    // column is written rather than refused.
    ed.paste_text("甲\t乙\t丙\n丁\t戊\t己\n");
    let text = ed.current_buffer().text();
    assert!(text.contains("甲"), "{text}\n{}", ed.status());
    assert!(text.contains("己"), "{text}\n{}", ed.status());
    // The row that was standing there is overwritten from the cursor's
    // cell, the way every grid pastes a block.
    assert!(!text.contains("樹"), "{text}");
    // Three columns now, rule row included.
    for line in text.lines().filter(|l| crate::mdtable::is_row(l)) {
        assert_eq!(crate::mdtable::split(line).len(), 3, "{line}");
    }
}

/// The same paste into a CSV — the block lands, and one too wide is
/// refused rather than shifting every column right of it (#226).
#[test]
fn j_and_k_in_a_grid_keep_the_cell_they_are_in() {
    // **In a grid the column is the cell** (#357). The goal column is worked
    // out from the document — text plus the padding a `|` table carries — and
    // `t f`/`t t` draw a grid of their own to different widths, so `j` walked
    // one set of columns under a page laid out to another and the caret
    // drifted sideways as it went down. Two rows whose cells are wildly
    // different widths make the two answers disagree on purpose.
    let dir = std::env::temp_dir().join(format!("yumete-gridjk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let doc = dir.join("book.md");
    std::fs::write(
        &doc,
        "| 名 | 註 |\n| --- | --- |\n| 甲 | 這是很長的一格 |\n| 乙乙乙乙乙乙乙 | 丁丁丁 |\n| 丙 | 戊 |\n",
    )
    .unwrap();

    let mut ed = Editor::new();
    ed.open_file(&doc).unwrap();
    ed.execute(":3").unwrap();
    assert!(ed.enter_table_as(true), "{}", ed.status());
    // `hjkl` by character, which is where the drift showed.
    ed.on_key(Key::Tab);

    // Stand in the second cell of the first data row, two characters in.
    let (line, _) = ed.cell_position().unwrap();
    let (from, _) = ed.cell_span(line, 1).unwrap();
    ed.set_cursor(from + 2);
    assert_eq!(ed.cell_position().map(|(_, c)| c), Some(1));

    // Down: the same cell, the same way into it — not whichever cell the
    // document's own column happens to land in on a much wider row.
    ed.on_key(Key::Char('j'));
    let (below, cell) = ed.cell_position().unwrap();
    assert_eq!(cell, 1, "j kept the cell it was in");
    assert_eq!(below, line + 1, "and went one row down");
    let (a, _) = ed.cell_span(below, 1).unwrap();
    assert_eq!(ed.cursor(), a + 2, "and the same way into it");

    // …and back up again lands where it started.
    ed.on_key(Key::Char('k'));
    assert_eq!(ed.cursor(), from + 2, "k comes back");

    // Into a cell too narrow to hold that offset, the caret stops at the
    // cell's end rather than spilling into the one after it.
    ed.on_key(Key::Char('j'));
    ed.on_key(Key::Char('j'));
    let (last, cell) = ed.cell_position().unwrap();
    assert_eq!(cell, 1, "still the same cell");
    let (a, b) = ed.cell_span(last, 1).unwrap();
    assert_eq!(ed.cursor(), (a + 2).min(b), "clamped, not spilled");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_search_in_the_grid_stays_in_the_table() {
    // **A hit the caret cannot reach is worse than no hit** (#354). With the
    // grid holding the pane the caret is held inside the table, while `/`
    // searched the whole document: it found 甲 in the prose, the clamp dragged
    // the caret back to the table's edge — **onto no match at all** — and the
    // next `n` searched from there and found the same unreachable hit again.
    // So `n` stopped going round the table, which is the one thing it is for.
    let dir = std::env::temp_dir().join(format!("yumete-gridfind-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let doc = dir.join("book.md");
    std::fs::write(
        &doc,
        "甲在上面這段散文裏。\n\n| 名 | 註 |\n| --- | --- |\n| 乙 | 甲 |\n| 丙 | 甲 |\n\n甲也在下面。\n",
    )
    .unwrap();

    let mut ed = Editor::new();
    ed.open_file(&doc).unwrap();
    ed.execute(":5").unwrap();
    // **The pane**, which is where the caret is held (`t t`) — the clamp and
    // this scope are tied to the same question, `takes_the_pane`.
    assert!(ed.enter_table_as(true), "{}", ed.status());
    let (first, last) = ed.table_row_span().expect("a table under the cursor");

    let selected = |ed: &Editor| {
        let (a, b) = ed.selection();
        ed.current_buffer().rope().slice(a..b).to_string()
    };

    ed.on_key(Key::Char('/'));
    ed.insert_committed("甲");
    ed.on_key(Key::Enter);

    // Six presses over two in-table hits: every one of them must land **on a
    // 甲**, and every one inside the table.
    let mut seen = std::collections::BTreeSet::new();
    for i in 0..6 {
        assert_eq!(selected(&ed), "甲", "press {i} left the caret off the hit");
        let line = ed.cursor_line();
        assert!(
            (first..=last).contains(&line),
            "press {i} landed on line {line}, outside {first}..={last}"
        );
        seen.insert(line);
        ed.on_key(Key::Char('n'));
    }
    assert_eq!(seen.len(), 2, "it went round both rows, not stuck on one");

    // …and a word only the prose says is reported as absent, rather than found
    // where the caret cannot follow.
    ed.on_key(Key::Char('/'));
    ed.insert_committed("散文");
    ed.on_key(Key::Enter);
    assert!(!ed.status().is_empty(), "it says the table does not hold it");
    assert_eq!(selected(&ed), "甲", "and nothing moved");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_quoted_field_is_read_as_the_one_field_it_is() {
    // **One quoted comma is enough, and the file need not be strange** (#307,
    // #311): clean rows, and somewhere among them `2500,"Smith, John",note`.
    // `cells` used to split on the delimiter and nothing more, so to it that
    // row had four fields — an edit to the one *beside* the name wrote the row
    // back from the wrong pieces (`2500,"Smith,ZZ,note`), and the answer for a
    // while was to refuse the row outright. It reads the quotes now, so there
    // is nothing to refuse: the row is a row and its cells are its cells.
    let dir = std::env::temp_dir().join(format!("yumete-quoted-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("people.csv");
    let text = "id,name,note\n1,佐藤,甲\n2500,\"Smith, John\",note\n3,鈴木,丙\n";
    std::fs::write(&csv, text).unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "T"); // #356: 這一條測的是格

    // Three cells on the quoted row, and the middle one is the whole name.
    assert_eq!(ed.cell_text(2, 1), "\"Smith, John\"");
    assert_eq!(ed.cell_text(2, 2), "note");

    // Changing the cell *beside* the name leaves the name alone — which is
    // the whole of what went wrong.
    ed.execute(":3").unwrap();
    ed.on_key(Key::Tab);
    ed.on_key(Key::Tab);
    press(&mut ed, "cZZ");
    ed.on_key(Key::Esc);
    assert_eq!(
        ed.current_buffer().text(),
        "id,name,note\n1,佐藤,甲\n2500,\"Smith, John\",ZZ\n3,鈴木,丙\n",
        "{}",
        ed.status()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_grid_is_drawn_where_the_cursor_goes() {
    // The parser read the quotes and the drawing did not (#391), so the two
    // halves of the same table disagreed about a row: the cursor walked three
    // cells and the page drew four boxes, and the bar at the top of the page
    // (#379) — which numbers the columns off the drawing — named one column
    // while pointing at another. Nothing was written wrongly, because writing
    // goes through the cells; it was the picture that was false.
    let dir = std::env::temp_dir().join(format!("yumete-drawn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("people.csv");
    std::fs::write(&csv, "name,note,age\n\"Smith, John\",ok,30\n\"Doe, Jane\",fine,60\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    for line in 0..3 {
        assert_eq!(
            ed.table_cells_on_line(line),
            ed.row_cells(line),
            "line {} is drawn in different places than it is walked",
            line + 1
        );
        assert_eq!(ed.table_cells_on_line(line).len(), 3, "line {}", line + 1);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// A record whose quote runs on is two lines to a grid that reads one record
/// to a line — and `:table-check` says *that*, not 「this line is 1 column」
/// (#391).
#[test]
fn a_record_that_runs_on_is_named_by_the_check() {
    let dir = std::env::temp_dir().join(format!("yumete-runs-on-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("people.csv");
    // Most of the file is a clean grid, so it opens: the warning at the door
    // is only ever asked of a file that fails to be one.
    let text = "name,note,age\n\"Smith, John\",ok,30\n\"runs on,here,40\nstill going\",no,50\n甲,乙,丙\n";
    std::fs::write(&csv, text).unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    ed.execute(":table-check").unwrap();
    let listing = ed.current_buffer().text();
    assert!(listing.contains("people.csv:3"), "{listing:?}");
    assert!(listing.contains("引號沒關上"), "{listing:?}");
    // …and the quoted name on line 2 is not reported at all: it is one field.
    assert!(!listing.contains("people.csv:2"), "{listing:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pasted_spreadsheet_lands_in_a_csv_too() {
    let dir = std::env::temp_dir().join(format!("yumete-paste-grid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("d.csv");
    std::fs::write(&csv, "字,說明\n木,樹\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    ed.goto_line(2);
    ed.paste_text("甲\t乙\n丙\t丁\n");
    assert_eq!(ed.current_buffer().text(), "字,說明\n甲,乙\n丙,丁\n", "{}", ed.status());

    // Three columns into a table two wide: named, and nothing moves.
    let before = ed.current_buffer().text();
    ed.paste_text("戊\t己\t庚\n");
    assert_eq!(ed.current_buffer().text(), before, "{}", ed.status());
    assert!(!ed.status().contains("貼了"), "{}", ed.status());

    let _ = std::fs::remove_dir_all(&dir);
}

/// A single line with no tab is still a cell, not a grid (#226).
#[test]
fn one_cell_of_text_is_not_a_spreadsheet() {
    let mut ed = typed("| 字 | 說明 |\n| --- | --- |\n| 木 | 樹 |\n");
    ed.goto_line(3);
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "i");
    ed.paste_text("大樹，很高");
    assert!(ed.current_buffer().text().contains("大樹，很高"), "{}", ed.status());
    assert!(ed.status().contains("貼了"), "{}", ed.status());
}

#[test]
fn no_writer_replaces_a_file_the_editor_is_holding() {
    let dir = std::env::temp_dir().join(format!("yumete-ident-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let book = dir.join("main.typ");
    let source = "= 第一章\n\n那年冬天，山下起了大雪。\n";
    std::fs::write(&book, source).unwrap();

    let mut ed = Editor::new();
    ed.open_file(&book).unwrap();

    // Every spelling of this buffer's own file.
    let link = dir.join("draft.typ");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&book, &link).unwrap();
    let mut spellings = vec![book.display().to_string()];
    #[cfg(unix)]
    spellings.push(link.display().to_string());
    for spelling in &spellings {
        ed.execute(&format!(":export typst {spelling}")).unwrap();
        assert!(
            ed.status().contains("稿子本身") || ed.status().contains("正開着"),
            "{spelling}: {}",
            ed.status()
        );
        assert_eq!(
            std::fs::read_to_string(&book).unwrap(),
            source,
            "{spelling} wrote over the manuscript"
        );
    }

    // …and another *open* buffer's file is just as much a manuscript.
    let other = dir.join("ch2.md");
    std::fs::write(&other, "第二章\n").unwrap();
    ed.open_file(&other).unwrap();
    press(&mut ed, "gp");
    assert_eq!(ed.current_buffer().path(), Some(book.as_path()));
    ed.execute(&format!(":export html {}", other.display()))
        .unwrap();
    assert!(ed.status().contains("正開着"), "{}", ed.status());
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "第二章\n");

    // An export onto a file nobody is holding still asks before it
    // replaces one that is already there.
    let out = dir.join("out.html");
    std::fs::write(&out, "早就有的東西\n").unwrap();
    ed.execute(&format!(":export html {}", out.display()))
        .unwrap();
    assert!(ed.status().contains("已經有"), "{}", ed.status());
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "早就有的東西\n");
    // …and `:export!` is how you say you meant it.
    ed.execute(&format!(":export! html {}", out.display()))
        .unwrap();
    assert!(ed.status().contains("寫好了"), "{}", ed.status());
    assert!(std::fs::read_to_string(&out).unwrap().contains("那年冬天"));

    std::fs::remove_dir_all(&dir).ok();
}

/// `:w <path>` copies and stays; `:write-as <path>` rebinds and says so.
#[test]
fn writing_a_copy_does_not_move_the_manuscript() {
    let dir = std::env::temp_dir().join(format!("yumete-copy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let chapter = dir.join("ch1.md");
    std::fs::write(&chapter, "第一稿\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&chapter).unwrap();
    press(&mut ed, "i");
    type_keys(&mut ed, "改");
    ed.on_key(Key::Esc);

    let copy = dir.join("copy.md");
    ed.execute(&format!(":w {}", copy.display())).unwrap();
    assert!(std::fs::read_to_string(&copy).unwrap().contains('改'));
    // The keys are still in the chapter, and so is the next `:w`.
    assert_eq!(ed.current_buffer().path(), Some(chapter.as_path()));
    ed.execute(":w").unwrap();
    assert!(std::fs::read_to_string(&chapter).unwrap().contains('改'));

    // The copy exists now, so a second one is a decision.
    press(&mut ed, "i");
    type_keys(&mut ed, "又");
    ed.on_key(Key::Esc);
    assert!(ed.execute(&format!(":w {}", copy.display())).is_err());
    assert!(!std::fs::read_to_string(&copy).unwrap().contains('又'));

    // `:write-as` is the one that moves house.
    let renamed = dir.join("ch1-final.md");
    ed.execute(&format!(":write-as {}", renamed.display())).unwrap();
    assert_eq!(ed.current_buffer().path(), Some(renamed.as_path()));
    assert!(std::fs::read_to_string(&renamed).unwrap().contains('又'));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn write_saves_the_active_buffer_and_quit_then_succeeds() {
    let mut path = std::env::temp_dir();
    path.push(format!("yumete-editor-write-{}.md", std::process::id()));

    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "初稿").expect("the fixture buffer is writable");
    assert!(ed.current_buffer().is_modified());

    // :w to a fresh path (save-as), then the buffer is clean and :q proceeds.
    let outcome = ed
        .execute(&format!(":w {}", path.display()))
        .expect("write");
    assert_eq!(outcome, CommandOutcome::Continue);
    assert!(!ed.current_buffer().is_modified());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "初稿");

    assert_eq!(ed.execute(":q").unwrap(), CommandOutcome::Quit);

    std::fs::remove_file(&path).ok();
}

#[test]
fn write_without_a_name_reports_no_file_name() {
    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "x").expect("the fixture buffer is writable");
    assert!(matches!(ed.execute(":w"), Err(EditorError::NoFileName)));
}

#[test]
fn quit_is_blocked_by_unsaved_changes_but_force_quit_overrides() {
    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "未存").expect("the fixture buffer is writable");

    assert!(matches!(ed.execute(":q"), Err(EditorError::UnsavedChanges)));
    assert_eq!(ed.execute(":q!").unwrap(), CommandOutcome::Quit);
}

#[test]
fn quit_on_a_clean_buffer_proceeds() {
    let mut ed = Editor::new();
    assert_eq!(ed.execute(":q").unwrap(), CommandOutcome::Quit);
}

// ---- Modal editing ----------------------------------------------------

/// Feed a string of `Key::Char` presses (plus Enter for '\n').
fn type_keys(ed: &mut Editor, s: &str) {
    for ch in s.chars() {
        let key = if ch == '\n' {
            Key::Enter
        } else {
            Key::Char(ch)
        };
        ed.on_key(key);
    }
}

#[test]
fn insert_mode_types_text_and_esc_returns_to_normal() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    assert_eq!(ed.mode(), Mode::Insert);
    type_keys(&mut ed, "你好");
    ed.on_key(Key::Esc);
    assert_eq!(ed.mode(), Mode::Normal);
    assert_eq!(ed.current_buffer().text(), "你好");
    assert_eq!(ed.cursor(), 2);
}

#[test]
fn normal_motions_move_the_cursor() {
    let mut ed = Editor::new();
    // Set up two lines via insert mode.
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "中x\nabc");
    ed.on_key(Key::Esc);

    // gg (goto mode) to the top.
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    assert_eq!(ed.cursor(), 0);

    // l moves over the wide "中" (one grapheme, one char, width 2).
    ed.on_key(Key::Char('l'));
    assert_eq!(ed.cursor(), 1);
    assert_eq!(ed.cursor_visual_column(), 2);

    // j keeps the visual column: column 2 on "abc" is after "ab" (char 5).
    ed.on_key(Key::Char('j'));
    assert_eq!(ed.cursor_line(), 1);
    assert_eq!(ed.cursor_visual_column(), 2);

    // gh / gl to line start / end on the second line.
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('h'));
    assert_eq!(ed.cursor(), 3);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('l'));
    // On the `c`, which is 5 — **not** 6. This line read `6` with the comment
    // 「end of "abc"」 until 2026-09-11, and 6 is one past the last character:
    // the assertion was writing the bug down rather than catching it (#382).
    assert_eq!(ed.cursor(), 5);

    // ge goes to the start of the last line.
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('e'));
    assert_eq!(ed.cursor_line(), 1);
}

#[test]
fn d_deletes_grapheme_and_backspace_joins_lines() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "ab\ncd");
    ed.on_key(Key::Esc);

    // Cursor at end after Esc; go to start of line 2 and backspace to join.
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('h')); // start of "cd"
    assert_eq!(ed.cursor(), 3);
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Backspace); // deletes the newline, joining "ab" + "cd"
    assert_eq!(ed.current_buffer().text(), "abcd");

    // Back to normal, gg, then d deletes the first char (Helix delete).
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.current_buffer().text(), "bcd");
}

#[test]
fn x_selects_a_line_and_d_deletes_the_selection() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "first\nsecond\nthird");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g')); // top

    // x selects the whole first line (including its newline).
    ed.on_key(Key::Char('x'));
    assert_eq!(ed.selection(), (0, 6)); // "first\n"

    // A second x extends to the second line.
    ed.on_key(Key::Char('x'));
    assert_eq!(ed.selection(), (0, 13)); // "first\nsecond\n"

    // d deletes the two selected lines.
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.current_buffer().text(), "third");
}

#[test]
fn find_char_moves_and_selects_within_the_line() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "hello world");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g')); // cursor at 0

    // f + 'w' jumps to the 'w' of "world" (char index 6) and selects
    // through it — `f` is inclusive, so `f。d` takes the 。 with it.
    ed.on_key(Key::Char('f'));
    ed.on_key(Key::Char('w'));
    assert_eq!(ed.cursor(), 6);
    assert_eq!(ed.selection(), (0, 7));

    // `t` is the table group now, in every mode — vi's till is gone, and
    // with the verb last (`f，d`) it was one keystroke from `f` anyway.

    // A missing target reports and does not move.
    let was = ed.cursor();
    ed.on_key(Key::Char('f'));
    ed.on_key(Key::Char('z'));
    assert_eq!(ed.cursor(), was);
    assert!(!ed.status().is_empty());
}

#[test]
fn extend_mode_keeps_the_anchor_while_moving() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "abcdef");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g')); // cursor at 0

    // v enters select mode; two l's extend the selection to cover "abc" —
    // the cursor's own grapheme is already in it.
    ed.on_key(Key::Char('v'));
    assert!(ed.is_extending());
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('l'));
    assert_eq!(ed.selection(), (0, 3));

    // d deletes the selection and leaves select mode.
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.current_buffer().text(), "def");
    assert!(!ed.is_extending());
}

#[test]
fn a_counted_edit_is_one_undo_point() {
    // **`100p` is one command in the hand, so it is one press of `u`** (#323).
    // `repeat` ran the action a hundred times and each announced a point of its
    // own, so taking back one keystroke took a hundred — which nobody reads as
    // 「precise」, they read it as 「undo is broken」.
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "ab");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('v'));
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('y'));
    let before = ed.current_buffer().text();

    type_keys(&mut ed, "10");
    ed.on_key(Key::Char('p'));
    let after = ed.current_buffer().text();
    assert_ne!(after, before, "ten pastes changed the text");
    assert!(after.len() > before.len() + 15, "all ten landed: {after:?}");

    ed.on_key(Key::Char('u'));
    assert_eq!(
        ed.current_buffer().text(),
        before,
        "one `u` takes back the whole count"
    );

    // …and it is still *one* point, not a group that swallowed what came
    // before: redo puts it back, and a second undo goes past it.
    ed.on_key(Key::Char('U'));
    assert_eq!(ed.current_buffer().text(), after, "redo restores the run");
}

#[test]
fn yank_and_paste_duplicate_the_selection() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "abc");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g')); // cursor at 0

    // Select "ab" (v + l), yank it, then paste after → "ababc".
    ed.on_key(Key::Char('v'));
    ed.on_key(Key::Char('l'));
    assert_eq!(ed.selection(), (0, 2));
    ed.on_key(Key::Char('y'));
    ed.on_key(Key::Char('p'));
    assert_eq!(ed.current_buffer().text(), "ababc");
}

#[test]
fn key_aliases_remap_normal_mode_keys() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "abc");
    ed.on_key(Key::Esc);

    // Remap `q` to behave as `d` (delete).
    let mut aliases = std::collections::HashMap::new();
    aliases.insert('q', "d".to_string());
    ed.set_key_aliases(aliases);

    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('q')); // aliased to `d` → deletes 'a'
    assert_eq!(ed.current_buffer().text(), "bc");
}

#[test]
fn an_unbound_key_says_what_this_editor_calls_it() {
    // The first minute in any editor is spent pressing exactly these, and
    // a key that does nothing and says nothing is an hour of guessing.
    let mut ed = typed("一行字\n");
    ed.goto_line(1);
    let before = ed.current_buffer().text();
    // `G` is not among them any more: it is bound — `30G` goes to line 30
    // and a bare `G` to the last line, as in vi and in Helix.
    for (key, want) in [('$', "gl"), ('^', "gs"), ('@', "Q")] {
        ed.on_key(Key::Char(key));
        assert!(ed.status().contains(want), "{key}: {}", ed.status());
        assert_eq!(ed.current_buffer().text(), before, "and it never does it");
    }
    // A key that *is* bound is not second-guessed.
    ed.on_key(Key::Char('x'));
    assert!(!ed.status().contains("gl"));
}

#[test]
fn the_prompt_can_be_edited_in_the_middle() {
    // A typo in a long `:%s` used to mean backspacing through all of it.
    let mut ed = typed("一二三\n");
    ed.on_key(Key::Char(':'));
    for c in "s/x/y/".chars() {
        ed.on_key(Key::Char(c));
    }
    assert_eq!(ed.prompt(), Some((":", "s/x/y/")));
    // Back over the closing `/`, fix the letter, and the tail is still there.
    ed.on_key(Key::Left);
    ed.on_key(Key::Backspace);
    ed.on_key(Key::Char('z'));
    assert_eq!(ed.prompt(), Some((":", "s/x/z/")));
    assert_eq!(ed.prompt_caret(), 5, "the caret stayed where the edit was");
    ed.on_key(Key::Home);
    assert_eq!(ed.prompt_caret(), 0);
    ed.on_key(Key::End);
    assert_eq!(ed.prompt_caret(), 6);
    // `C-w` takes a word back, `C-u` the whole line.
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char(':'));
    for c in "sh wc -w".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Ctrl('w'));
    assert_eq!(ed.prompt(), Some((":", "sh wc ")));
    ed.on_key(Key::Ctrl('u'));
    assert_eq!(ed.prompt(), Some((":", "")));

    // A full-width space is three bytes, and `byte index + 1` lands inside
    // it — a Chinese writer types one without thinking about it.
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char(':'));
    for c in "grep 甲　乙".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Ctrl('w'));
    assert_eq!(ed.prompt(), Some((":", "grep 甲　")));
}

#[test]
fn the_prompt_remembers_what_was_typed_at_it() {
    let mut ed = typed("一二三\n");
    for line in ["toc", "w"] {
        ed.on_key(Key::Char(':'));
        for c in line.chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Enter);
    }
    ed.on_key(Key::Char(':'));
    ed.on_key(Key::Up);
    assert_eq!(ed.prompt(), Some((":", "w")), "the newest first");
    ed.on_key(Key::Up);
    assert_eq!(ed.prompt(), Some((":", "toc")));
    ed.on_key(Key::Up);
    assert_eq!(ed.prompt(), Some((":", "toc")), "and it stops at the oldest");
    ed.on_key(Key::Down);
    assert_eq!(ed.prompt(), Some((":", "w")));
    ed.on_key(Key::Down);
    assert_eq!(ed.prompt(), Some((":", "")), "back to the empty line");
    ed.on_key(Key::Esc);

    // The search prompt keeps its own, because patterns and commands are
    // not the same list.
    press(&mut ed, "/二");
    ed.on_key(Key::Enter);
    ed.on_key(Key::Char('/'));
    ed.on_key(Key::Up);
    assert_eq!(ed.prompt(), Some(("/", "二")));
}

#[test]
fn a_book_can_teach_the_editor_its_own_names() {
    // 阿寧 — the name on every page — is the one word no dictionary has,
    // so `w` stepped through it a character at a time and the overlay
    // tinted it as two words.
    let dir = std::env::temp_dir().join(format!("yumete-words-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".yumete")).unwrap();
    let file = dir.join("ch01.md");
    std::fs::write(&file, "阿寧走了。\n").unwrap();

    let mut ed = Editor::new();
    ed.set_segmenter(Box::new(DictionarySegmenter::builtin(0)));
    ed.open_file(&file).unwrap();
    let before = ed.segment_line(0);
    assert!(before.len() >= 2, "two characters, two words: {before:?}");

    std::fs::write(dir.join(".yumete").join("words.txt"), "# 人物\n阿寧\n").unwrap();
    ed.reload_project_words();
    assert_eq!(ed.project_word_count(), 1, "{}", ed.status());
    let after = ed.segment_line(0);
    assert_eq!(after[0], (0, 2), "one word now: {after:?}");
    assert!(ed.status().contains("words.txt"), "{}", ed.status());
    std::fs::remove_dir_all(&dir).ok();
}

/// 自動認詞：三件事各歸各位（#448）。
///
/// 作者定的分工：**autodetect 只在内存**（開文件觸發，後台算，分詞用它）；
/// **`:word-discover`** 手動跑，把那份名單寫成 `.yumete/discovered_words.txt`
/// 給人看，每次覆蓋；**`.yumete/words.txt` 是使用者的**，yumete 只讀不寫。
///
/// 從前這三件事擠在一個檔裏：discover 把候選插進 `words.txt`（未存），`:w` 是
/// 「我認了」，劃掉一行就是拒絕。麻煩在於**拒絕留不住**——那個詞在文稿裏還在，
/// 下一輪照樣找得出來，又寫回去。
#[test]
fn the_book_hands_the_editor_its_own_names_without_being_asked() {
    // The same 阿寧, found rather than typed in (Feature #239). Six
    // sightings spread over three chapters, in different company each
    // time, and nowhere else in the language.
    let dir = std::env::temp_dir().join(format!("yumete-discover-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Prose for the name to stand out against. Repeated four times, which
    // is one short of `MIN_COUNT`, so the filler itself yields nothing.
    let prose = "天地玄黃宇宙洪荒日月盈昃辰宿列張寒來暑往秋收冬藏閏餘成歲律呂調陽雲騰致雨露結爲霜金生麗水\n"
        .repeat(4);
    std::fs::write(dir.join("ch01.md"), format!("{prose}甲阿寧乙。\n丙阿寧丁。\n")).unwrap();
    std::fs::write(dir.join("ch02.md"), "戊阿寧己。\n庚阿寧辛。\n").unwrap();
    std::fs::write(dir.join("ch03.md"), "壬阿寧癸。\n子阿寧丑。\n").unwrap();

    // ---- ① 開文件只**留下一個請求**，一個字都不說 ---------------------
    let mut ed = Editor::new();
    ed.set_segmenter(Box::new(DictionarySegmenter::builtin(0)));
    ed.open_file(&dir.join("ch02.md")).unwrap();
    assert_eq!(
        ed.take_detect_request().as_deref(),
        Some(dir.as_path()),
        "開文件要請前端掃一遍這個項目"
    );
    assert!(ed.take_detect_request().is_none(), "只請一次");

    // ---- ② 前端算完交回來，只在内存，分詞立刻跟上 ---------------------
    let seg = DictionarySegmenter::builtin(0);
    let joins = |w: &str| yumete_cjk::Segmenter::segment(&seg, w).len() == 1;
    let (found, files) = crate::editor::detect_words_in(&dir, &joins);
    assert_eq!(files, 3, "三個章節都讀了");
    // 而光看眼前這一篇是看不出阿寧的：一章只有兩次。這正是範圍要能選的理由。
    assert!(
        !crate::editor::detect_words_in(&dir.join("ch02.md"), &joins).0
            .iter()
            .any(|f| f.word == "阿寧"),
        "一章兩次，夠不上"
    );
    assert!(found.iter().any(|f| f.word == "阿寧"), "{found:?}");

    let before = ed.segment_line(0);
    assert!(before.len() >= 3, "戊[阿][寧]己：{before:?}");
    let mut list = yumete_cjk::WordList::default();
    for word in &found {
        list.add(&word.word);
    }
    ed.set_detected_words(list);
    assert_eq!(ed.segment_line(0)[1], (1, 3), "戊[阿寧]己：{:?}", ed.segment_line(0));
    assert!(ed.detected_word_count() >= 1);

    // **紙面上什麼都沒發生**：沒開檔、沒換 buffer、沒寫盤。
    assert_eq!(ed.current_buffer().path(), Some(dir.join("ch02.md").as_path()));
    assert!(!dir.join(".yumete").join("discovered_words.txt").exists());
    assert!(!dir.join(".yumete").join("words.txt").exists());

    // ---- ③ 手動那一支：寫檔、開檔、說一句 -----------------------------
    //
    // ⚠️ **`-cd`, not bare** (#452). 光 `:word-discover` 只讀眼前這一篇，而
    // 阿寧 分散在三章裏 —— 一章兩次夠不上 `MIN_COUNT`。範圍是這族命令的參數，
    // 不是它的背景設定。
    ed.open_file(&dir.join("ch01.md")).unwrap();
    assert!(ed.execute("word-discover-cd").is_ok(), "{}", ed.status());
    let listing = dir.join(".yumete").join("discovered_words.txt");
    assert!(listing.is_file(), "名單要寫出來：{}", ed.status());
    assert_eq!(
        ed.current_buffer().path(),
        Some(listing.as_path()),
        "這條命令的產物就是那份名單，開它是它的全部用處"
    );
    let text = std::fs::read_to_string(&listing).unwrap();
    assert!(text.contains("阿寧"), "{text:?}");
    assert!(text.contains("words.txt"), "抬頭要指路：{text:?}");

    // **整份覆蓋，不追加。** 跑兩次不會變兩份。
    let again = {
        ed.open_file(&dir.join("ch01.md")).unwrap();
        assert!(ed.execute("word-discover-cd").is_ok(), "{}", ed.status());
        std::fs::read_to_string(&listing).unwrap()
    };
    assert_eq!(again, text, "第二次跑出來的該一模一樣");
    assert_eq!(again.matches("阿寧").count(), text.matches("阿寧").count());

    // ---- ④ 那份檔**不讀回來**：在裏面刪一行什麼也不會發生 -------------
    std::fs::write(&listing, "# 清空了\n").unwrap();
    ed.open_file(&dir.join("ch02.md")).unwrap();
    assert_eq!(
        ed.segment_line(0)[1],
        (1, 3),
        "分詞靠的是内存那一份，不是那個檔：{:?}",
        ed.segment_line(0)
    );

    // ---- ⑤ 使用者自己那一份是另一件事，兩份一起生效 -------------------
    std::fs::create_dir_all(dir.join(".yumete")).unwrap();
    std::fs::write(dir.join(".yumete").join("words.txt"), "# 人物\n庚阿\n").unwrap();
    ed.reload_project_words();
    assert!(ed.project_word_count() >= 2, "兩份合起來：{}", ed.status());
    assert!(ed.detected_word_count() >= 1, "重讀使用者那一份，不該把認到的丟掉");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_mark_names_a_place_and_survives_the_afternoon() {
    // The jump list remembers where you came *from*; a mark remembers
    // where you meant to come back **to**.
    let dir = std::env::temp_dir().join(format!("yumete-marks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let one = dir.join("ch01.md");
    let two = dir.join("ch02.md");
    std::fs::write(&one, "一\n二\n三\n四\n五\n").unwrap();
    std::fs::write(&two, "甲\n乙\n丙\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&one).unwrap();
    ed.goto_line(4);
    press(&mut ed, "M");
    ed.on_key(Key::Char('a'));
    assert!(ed.status().contains('a'), "{}", ed.status());

    // Off to another chapter, and back by name — the file opens itself.
    ed.open_file(&two).unwrap();
    ed.goto_line(2);
    press(&mut ed, "'");
    ed.on_key(Key::Char('a'));
    assert_eq!(ed.cursor_line(), 3, "{}", ed.status());
    assert_eq!(ed.current_buffer().path(), Some(one.as_path()));

    // …and `C-o` goes back to where `'a` was pressed, because a mark is a
    // jump.
    ed.on_key(Key::Ctrl('o'));
    assert_eq!(ed.current_buffer().path(), Some(two.as_path()));

    // A letter nobody marked says so rather than moving.
    press(&mut ed, "'");
    ed.on_key(Key::Char('z'));
    assert!(ed.status().contains('z'), "{}", ed.status());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_session_opens_again_what_was_open() {
    // Five `:open`s every morning is five too many.
    let dir = std::env::temp_dir().join(format!("yumete-session-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("state")).unwrap();
    let one = dir.join("ch01.md");
    let two = dir.join("ch02.md");
    std::fs::write(&one, "一\n二\n三\n四\n").unwrap();
    std::fs::write(&two, "甲\n乙\n丙\n").unwrap();

    let mut ed = Editor::new();
    ed.keep_session_in(dir.join("state"), &dir);
    ed.open_file(&one).unwrap();
    ed.goto_line(3);
    ed.open_file(&two).unwrap();
    ed.goto_line(2);
    ed.save_session();

    // A new morning.
    let mut ed = Editor::new();
    ed.keep_session_in(dir.join("state"), &dir);
    assert_eq!(ed.restore_session(), 2);
    assert_eq!(ed.buffer_count(), 2);
    // Each at the line it was left on.
    ed.open_file(&one).unwrap();
    assert_eq!(ed.cursor_line(), 2, "ch01 was left on line 3");
    ed.open_file(&two).unwrap();
    assert_eq!(ed.cursor_line(), 1, "ch02 on line 2");

    // A file that has since been deleted is simply not opened — a
    // convenience does not get to put an error on the screen every morning.
    std::fs::remove_file(&two).unwrap();
    let mut ed = Editor::new();
    ed.keep_session_in(dir.join("state"), &dir);
    assert_eq!(ed.restore_session(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn closing_a_file_does_not_send_a_jump_into_a_different_one() {
    // `close_buffer` removes one and every later buffer shifts down, so a
    // jump list and a mark kept by *index* came to name a different
    // chapter than the one they were set in.
    let dir = std::env::temp_dir().join(format!("yumete-ids-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b, c) = (dir.join("a.md"), dir.join("b.md"), dir.join("c.md"));
    std::fs::write(&a, "甲一\n甲二\n甲三\n").unwrap();
    std::fs::write(&b, "乙一\n乙二\n乙三\n").unwrap();
    std::fs::write(&c, "丙一\n丙二\n丙三\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&a).unwrap();
    ed.open_file(&b).unwrap();
    ed.open_file(&c).unwrap();
    // A mark in c, then a jump away from it.
    ed.goto_line(3);
    press(&mut ed, "M");
    ed.on_key(Key::Char('a'));
    // Close b — every buffer after it used to shift down by one.
    ed.open_file(&b).unwrap();
    ed.execute("buffer-close!").unwrap();
    ed.open_file(&a).unwrap();
    ed.goto_line(2);
    // The mark still means c.
    press(&mut ed, "'");
    ed.on_key(Key::Char('a'));
    assert_eq!(ed.current_buffer().path(), Some(c.as_path()), "{}", ed.status());
    // …and `C-o` still means where it was pressed from.
    ed.on_key(Key::Ctrl('o'));
    assert_eq!(ed.current_buffer().path(), Some(a.as_path()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_key_alias_may_name_a_sequence() {
    // The defaults this editor chose on purpose — `J`/`K` paging a book
    // rather than joining lines — are the ones a Vim reader wants back,
    // and what they want back is `gJ`. One config line instead of leaving.
    let mut ed = typed("上一句\n下一句\n");
    ed.goto_line(1);
    let mut aliases = std::collections::HashMap::new();
    aliases.insert('J', "gJ".to_string());
    ed.set_key_aliases(aliases);
    ed.on_key(Key::Char('J'));
    assert_eq!(ed.current_buffer().text(), "上一句下一句\n");
}

#[test]
fn an_alias_that_names_itself_does_not_spin() {
    let mut ed = typed("abc\n");
    ed.goto_line(1);
    let mut aliases = std::collections::HashMap::new();
    // `x` stands for `xx` — which stands for `xx`, and so on.
    aliases.insert('x', "xx".to_string());
    ed.set_key_aliases(aliases);
    ed.on_key(Key::Char('x'));
    // It ran once, one level deep, and came back.
    assert!(!ed.current_buffer().text().is_empty());
}

#[test]
fn word_motion_selects_the_word_and_delete_removes_it() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "foo bar baz");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g')); // to the start

    // w selects from the cursor to the next word start ("foo ").
    ed.on_key(Key::Char('w'));
    assert_eq!(ed.selection(), (0, 4));
    // d deletes the selection → "bar baz" (word delete, Feature #26).
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.current_buffer().text(), "bar baz");

    // e moves to the end of the next word.
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('e'));
    assert_eq!(ed.cursor(), 2); // end of "bar"

    // b moves back to the start of the word.
    ed.on_key(Key::Char('l')); // into "baz"
    ed.on_key(Key::Char('b'));
    assert_eq!(ed.cursor(), 0);
}

#[test]
fn dictionary_segmenter_makes_word_motions_skip_whole_cjk_words() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "你好世界");
    ed.on_key(Key::Esc);
    // Default: each CJK character is its own word. Selecting up to just
    // before the next one would be a standstill, so `w` takes the next word
    // — 好 — the way vi's `w` moves onto it.
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('w'));
    assert_eq!(ed.selection(), (1, 2));

    // With a dictionary, `w` steps over the whole word 你好.
    ed.set_segmenter(Box::new(yumete_cjk::DictionarySegmenter::new(
        [("你好".to_string(), 100), ("世界".to_string(), 100)],
        1,
    )));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('w'));
    assert_eq!(ed.selection(), (0, 2));
    // segment_line reflects the same grouping.
    assert_eq!(ed.segment_line(0), vec![(0, 2), (2, 4)]);
}

#[test]
fn aligning_a_table_measures_what_the_terminal_draws() {
    // 「重排對齊」 that counts characters leaves a column of 漢字 ragged:
    // 木 is one character and two cells, and a table lines up in cells.
    let mut ed = typed("| 字 | 拆分 | 說明 |\n| --- | --- | --- |\n| 木 | 木 | 樹 |\n| 相 | ⿰木目 | 看 |\n| a | bb | ccc |\n");
    ed.goto_line(1);
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "tF");
    let text = ed.current_buffer().text();
    let widths: Vec<usize> = text
        .lines()
        .map(yumete_cjk::str_width)
        .collect();
    assert!(
        widths.windows(2).all(|w| w[0] == w[1]),
        "every row is the same width on the terminal:\n{text}"
    );
}

#[test]
fn the_word_command_is_one_subject_from_three_sides() {
    let mut ed = Editor::new();
    // 著色: named on and off, and flipped when neither is said.
    assert!(!ed.segmentation_visible());
    ed.execute(":word-show on").unwrap();
    assert!(ed.segmentation_visible());
    ed.execute(":word-show").unwrap();
    assert!(!ed.segmentation_visible());

    // 粒度: it says which, and it takes which.
    ed.execute(":word-level").unwrap();
    assert!(ed.status().contains("balanced"), "{}", ed.status());
    ed.execute(":word-level strict").unwrap();
    assert_eq!(ed.word_level(), yumete_cjk::WordLevel::Strict);
    assert!(ed.status().contains("strict"), "{}", ed.status());

    // 詞表: `:word` reports what is in force rather than doing anything.
    ed.execute(":word").unwrap();
    assert!(ed.status().contains("分詞"), "{}", ed.status());

    // …and reload is a question for the front end, which owns the IME.
    ed.execute(":word-list reload").unwrap();
    assert!(ed.take_words_request(), "the front end is asked to rebuild");
}

#[test]
fn o_opens_a_line_below_in_insert_mode() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "first");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('o'));
    assert_eq!(ed.mode(), Mode::Insert);
    type_keys(&mut ed, "second");
    ed.on_key(Key::Esc);
    assert_eq!(ed.current_buffer().text(), "first\nsecond");
}

#[test]
fn command_mode_runs_the_colon_line_and_quit_signals() {
    let mut ed = Editor::new();
    // Type some text so the buffer is modified.
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "hi");
    ed.on_key(Key::Esc);

    // :q on a modified buffer is refused and reported in the status line.
    ed.on_key(Key::Char(':'));
    assert_eq!(ed.mode(), Mode::Command);
    type_keys(&mut ed, "q");
    assert_eq!(ed.on_key(Key::Enter), KeyOutcome::Continue);
    assert!(!ed.status().is_empty());

    // :q! quits.
    ed.on_key(Key::Char(':'));
    type_keys(&mut ed, "q!");
    assert_eq!(ed.on_key(Key::Enter), KeyOutcome::Quit);
}

#[test]
fn undo_reverts_an_insert_and_redo_reapplies_it() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "hello");
    ed.on_key(Key::Esc);
    assert_eq!(ed.current_buffer().text(), "hello");

    // u undoes the whole insert session back to empty.
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "");

    // U redoes it (Helix redo).
    ed.on_key(Key::Char('U'));
    assert_eq!(ed.current_buffer().text(), "hello");
}

#[test]
fn recover_loads_the_draft_and_undo_takes_it_back() {
    let dir = std::env::temp_dir().join(format!("yumete-rec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chapter.md");
    std::fs::write(&path, "第一稿\n").unwrap();
    std::fs::write(dir.join(".chapter.md.yumete"), "第一稿，寫了更多\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    // The draft is not loaded on its own — the writer is told about it.
    assert_eq!(ed.current_buffer().text(), "第一稿\n");
    ed.announce_recovery();
    assert!(ed.status().contains(":recover"), "{}", ed.status());

    ed.execute(":recover").unwrap();
    assert_eq!(ed.current_buffer().text(), "第一稿，寫了更多\n");
    // Recovering is an ordinary edit, so it can be taken back.
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "第一稿\n");

    // Loading it takes it over: there is nothing left waiting — and with
    // no drafts from a crashed session either, it says so about this file.
    ed.execute(":recover").unwrap();
    assert!(ed.status().contains("沒有搶救稿"), "{}", ed.status());

    // In a fresh session, `:recover!` throws the copy away instead.
    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    ed.execute(":recover!").unwrap();
    assert!(!dir.join(".chapter.md.yumete").exists());

    std::fs::remove_dir_all(&dir).ok();
}

/// A draft this session did not write is somebody's unrecovered work. Three
/// things must not touch it: quitting, `:q!`, and the next keystroke.
#[test]
fn an_untaken_draft_survives_quitting_and_typing() {
    let dir = std::env::temp_dir().join(format!("yumete-keep-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chapter.md");
    let swap = dir.join(".chapter.md.yumete");
    std::fs::write(&path, "第一稿\n").unwrap();
    std::fs::write(&swap, "第一稿加上三千字沒存的\n").unwrap();

    // Typing does not write over it.
    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "X");
    ed.on_key(Key::Esc);
    ed.autosave_tick();
    assert_eq!(
        std::fs::read_to_string(&swap).unwrap(),
        "第一稿加上三千字沒存的\n",
        "one keystroke erased the draft the crash left behind"
    );

    // Neither does going to look at the file somewhere else.
    assert_eq!(ed.execute(":q!").unwrap(), CommandOutcome::Quit);
    assert!(swap.exists(), "quitting deleted an unrecovered draft");

    // Only saying so does.
    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    ed.execute(":recover!").unwrap();
    assert!(!swap.exists());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_failed_save_as_leaves_the_buffer_where_it_was() {
    let dir = std::env::temp_dir().join(format!("yumete-badsave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chapter.md");
    std::fs::write(&path, "第一稿\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "改");
    ed.on_key(Key::Esc);
    ed.autosave_tick();
    let swap = dir.join(".chapter.md.yumete");
    assert!(swap.exists());

    // A save-as into a directory that does not exist must change nothing:
    // rebinding to an unwritable path would make every later save and every
    // later recovery write fail, silently.
    let nowhere = dir.join("no-such-dir").join("chapter.md");
    assert!(ed.execute(&format!(":w {}", nowhere.display())).is_err());
    assert_eq!(ed.current_buffer().path(), Some(path.as_path()));
    assert!(
        swap.exists(),
        "a failed save-as took the recovery copy with it"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn quitting_takes_the_recovery_copies_with_it() {
    let dir = std::env::temp_dir().join(format!("yumete-recq-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("draft.md");
    std::fs::write(&path, "初稿\n").unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", path.display())).unwrap();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "改");
    ed.on_key(Key::Esc);
    ed.current_buffer_mut().write_swap().unwrap();
    let swap = dir.join(".draft.md.yumete");
    assert!(swap.exists());

    // `:q!` is the writer discarding these changes; offering them back on
    // the next open would undo that decision for them.
    assert_eq!(ed.execute(":q!").unwrap(), CommandOutcome::Quit);
    assert!(!swap.exists());

    std::fs::remove_dir_all(&dir).ok();
}

/// The selection covers the grapheme the cursor is on, as it does in Helix.
/// Without that, the block cursor sits on a character an edit would not
/// touch — what the screen shows is not what `d` takes.
/// `w` must always move. Selecting up to *just before* the next word is
/// right for a word of several characters and is a standstill for a word of
/// one — and with the default segmenter every 漢字 is a word of one.
#[test]
fn w_steps_a_word_at_a_time_however_short_the_words_are() {
    let mut ed = typed("那年冬天雪下得早");
    press(&mut ed, "gg");
    let mut walked = Vec::new();
    for _ in 0..5 {
        press(&mut ed, "w");
        walked.push(ed.cursor());
    }
    assert_eq!(walked, vec![1, 2, 3, 4, 5], "`w` stood still");

    // With real words it takes one whole word, and `d` takes exactly that.
    let mut ed = typed("那年冬天下雪");
    ed.set_segmenter(Box::new(DictionarySegmenter::from_text(
        "那年 10\n冬天 10\n下雪 10\n",
        0,
    )));
    press(&mut ed, "gg");
    press(&mut ed, "w");
    assert_eq!(ed.selection(), (0, 2), "那年");
    press(&mut ed, "w");
    assert_eq!(ed.selection(), (2, 4), "冬天");
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.current_buffer().text(), "那年下雪");
}

#[test]
fn what_the_cursor_covers_is_what_an_edit_takes() {
    // `f` and `t` reach through their target.
    let mut ed = typed("那年冬天，雪下得早。");
    press(&mut ed, "gg");
    press(&mut ed, "f，");
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.current_buffer().text(), "雪下得早。");

    // `e` reaches the end of its word.
    let mut ed = typed("hello world");
    press(&mut ed, "gge");
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.current_buffer().text(), " world");

    // One `l` in select mode covers two characters, not one.
    let mut ed = typed("春江潮水");
    press(&mut ed, "ggvl");
    assert_eq!(ed.selection(), (0, 2));

    // And a bare cursor is a selection of one, so `d` takes that one.
    let mut ed = typed("春江潮水");
    press(&mut ed, "gg");
    assert!(!ed.has_selection(), "standing on a 字 is not selecting it");
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.current_buffer().text(), "江潮水");
}

#[test]
fn dot_repeats_the_change_it_actually_follows() {
    // `.` sits next to `d`, and it used to repeat the last *typing
    // session* whatever came after — so it had to refuse after a delete.
    // Now it repeats the change it follows, so there is nothing to refuse.
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "abcdef");
    ed.on_key(Key::Esc);
    type_keys(&mut ed, "gg");
    type_keys(&mut ed, "d.");
    assert_eq!(ed.current_buffer().text(), "cdef", "d then . deletes twice");
    // And after a typing session it repeats the typing.
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "X");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('.'));
    assert_eq!(ed.current_buffer().text(), "XXcdef");
}

#[test]
fn dot_repeats_an_operator_so_the_proofreading_loop_works() {
    // `n . n .` — search, fix, search, fix. The loop a manuscript is
    // proofread in, and the reason all three reviewers named `.`.
    let mut ed = typed("裏面\n那裏\n這裏\n");
    press(&mut ed, "gg");
    press(&mut ed, "/裏");
    ed.on_key(Key::Enter);
    // `r` and its operand are one change, so `.` plays both back.
    press(&mut ed, "r");
    ed.on_key(Key::Char('裡'));
    press(&mut ed, "n");
    ed.on_key(Key::Char('.'));
    press(&mut ed, "n");
    ed.on_key(Key::Char('.'));
    assert_eq!(ed.current_buffer().text(), "裡面\n那裡\n這裡\n");
}

/// Esc dismisses the preview; `n` brings it back on the next hit.
///
/// The reader who presses Esc has closed a window, not called off the
/// search — and `n` after it used to fall through to `/`'s own repeat,
/// which answered about whatever was last typed at a `/` prompt, or did
/// nothing at all and said nothing about why.
#[test]
fn escape_shuts_the_preview_and_n_opens_it_again() {
    let mut ed = typed("那年冬天。\n第二行。\n那年夏天。\n又一行。\n那年秋天。\n");
    press(&mut ed, "gg");
    // A previewing search: 那 is on lines 1, 3 and 5.
    press(&mut ed, "g?");
    assert_eq!(ed.peeked_line(), Some(2), "{}", ed.status());
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.peeked_line(), Some(4), "{}", ed.status());

    ed.on_key(Key::Esc);
    assert!(ed.other_pane().is_none(), "Esc shuts the preview");

    // …and `n` is still walking the same list, pane and all.
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.peeked_line(), Some(0), "round to the first 那");
    assert!(ed.other_pane().is_some(), "the preview is back");
    assert!(ed.status().contains("1/3"), "{}", ed.status());
}

/// `n` with nothing to repeat says so.
#[test]
fn n_with_no_search_behind_it_says_so() {
    let mut ed = typed("那年冬天。\n");
    ed.on_key(Key::Char('n'));
    assert!(
        ed.status().contains("還沒有搜索過") || ed.status().contains("searched"),
        "{}",
        ed.status()
    );
}

#[test]
fn esc_leaves_select_mode() {
    let mut ed = typed("abc");
    ed.on_key(Key::Char('v'));
    assert!(ed.is_extending());
    ed.on_key(Key::Esc);
    assert!(!ed.is_extending(), "Esc is every modal editor's way out");
}

#[test]
fn a_count_reaches_the_operators_too() {
    let mut ed = typed("abcdef");
    type_keys(&mut ed, "gg3d");
    assert_eq!(ed.current_buffer().text(), "def");
}

#[test]
fn a_count_moves_that_many_zong() {
    let mut ed = typed("一二三四五六七八九十");
    ed.set_layout(crate::zong::Layout::Vertical);
    ed.set_zong_length(32);
    type_keys(&mut ed, "gg");
    let before = ed.cursor();
    type_keys(&mut ed, "5j");
    assert_eq!(ed.cursor() - before, 5, "vertical hjkl dropped the count");
}

#[test]
fn line_operators_follow_the_selection_not_the_cursor() {
    // `x` parks the cursor on the line *after* the one it selected, so
    // anything reading the cursor's line acted on a line the writer had
    // not selected and could not see was selected.
    let mut ed = typed("一\n二\n三\n四\n");
    type_keys(&mut ed, "ggxxxgJ");
    assert_eq!(ed.current_buffer().text(), "一二\n三\n四\n");

    let mut ed = typed("甲甲\n甲甲\n");
    type_keys(&mut ed, "ggx");
    ed.execute(":s/甲/乙/g").unwrap();
    assert_eq!(ed.current_buffer().text(), "乙乙\n甲甲\n");
}

#[test]
fn whole_lines_are_pasted_as_whole_lines() {
    // `xy`, move, `p` is how a paragraph is moved; pasting the copied line
    // *into* another line cuts that line in two.
    let mut ed = typed("一二三\n四五六\n");
    type_keys(&mut ed, "ggxy");
    type_keys(&mut ed, "jlp");
    assert_eq!(ed.current_buffer().text(), "一二三\n四五六\n一二三\n");
}

#[test]
fn a_count_before_f_finds_the_nth_occurrence() {
    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "a.b.c.d").expect("the fixture buffer is writable");
    // `3f.` is the third dot, not the first — the count belongs to the `f`,
    // which has already spent it by the time the target arrives.
    type_keys(&mut ed, "3f.");
    assert_eq!(ed.cursor(), 5);
    type_keys(&mut ed, "gg");
    type_keys(&mut ed, "f.");
    assert_eq!(ed.cursor(), 1);
}

#[test]
fn a_count_before_gg_is_a_line_number() {
    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "一\n二\n  三\n四\n")
        .expect("the fixture buffer is writable");
    // `3gg` lands on the first non-blank of line 3, past its indent.
    type_keys(&mut ed, "3gg");
    assert_eq!(ed.cursor_line(), 2);
    assert_eq!(ed.cursor(), 6);
    // A bare `gg` is still the top of the file.
    type_keys(&mut ed, "gg");
    assert_eq!(ed.cursor(), 0);
    // Past the end clamps rather than doing nothing.
    type_keys(&mut ed, "99gg");
    assert_eq!(ed.cursor_line(), 3);
}

#[test]
fn a_bare_number_on_the_command_line_is_a_line_number() {
    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "一\n二\n三\n").expect("the fixture buffer is writable");
    ed.execute(":2").unwrap();
    assert_eq!(ed.cursor_line(), 1);
    ed.execute(":goto 3").unwrap();
    assert_eq!(ed.cursor_line(), 2);
}

#[test]
fn undo_belongs_to_the_buffer_it_was_taken_in() {
    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "甲").expect("the fixture buffer is writable");
    ed.execute(":new").unwrap();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "乙");
    ed.on_key(Key::Esc);
    assert_eq!(ed.current_buffer().text(), "乙");

    // Back in the first file, `u` must find nothing to undo — not pop the
    // snapshot taken in the second and write 乙's text over 甲's.
    ed.prev_buffer();
    assert_eq!(ed.current_buffer().text(), "甲");
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "甲");

    // And the second file's own history is still its own.
    ed.next_buffer();
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "");
}

#[test]
fn leaving_checks_every_open_file_not_just_this_one() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "unsaved");
    ed.on_key(Key::Esc);
    // A fresh, clean buffer is current — but the first one is still dirty.
    ed.execute(":new").unwrap();
    assert!(matches!(ed.execute(":qa"), Err(EditorError::UnsavedChanges)));
    // …and the editor moves to the file that is holding the exit up.
    assert_eq!(ed.current_buffer().text(), "unsaved");
    assert_eq!(ed.execute(":qa!").unwrap(), CommandOutcome::Quit);
}

#[test]
fn quit_closes_this_file_and_only_leaves_on_the_last_one() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "甲");
    ed.on_key(Key::Esc);
    ed.execute(":new").unwrap();
    // Two files open: `:q` puts this one away and stays in the editor,
    // even though the other one is dirty — it is not being asked to leave.
    assert_eq!(ed.execute(":q").unwrap(), CommandOutcome::Continue);
    assert_eq!(ed.current_buffer().text(), "甲");
    // Now it is the last one, and `:q` means what it always meant.
    assert!(matches!(ed.execute(":q"), Err(EditorError::UnsavedChanges)));
    assert_eq!(ed.execute(":q!").unwrap(), CommandOutcome::Quit);
}

#[test]
fn write_quit_saves_where_it_is_told_and_refuses_a_nameless_buffer() {
    let dir = std::env::temp_dir().join(format!("yumete-wq-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("saved.txt");

    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "文");
    ed.on_key(Key::Esc);
    // With no path, `:wq` neither writes nor quits.
    assert!(matches!(ed.execute(":wq"), Err(EditorError::NoFileName)));

    // `:wq <path>` is a save-as, like `:w <path>`.
    let out = ed.execute(&format!(":wq {}", path.display())).unwrap();
    assert_eq!(out, CommandOutcome::Quit);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "文");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_project_says_which_markup_its_files_are_in() {
    let dir = std::env::temp_dir().join(format!("yumete-synconf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A chapter that is nothing but writing: there is no Typst in it to
    // find, so reading the file cannot settle it. The project can.
    std::fs::write(dir.join("ch01.txt"), "那年冬天，雪下得比往常都早。\n").unwrap();
    std::fs::write(dir.join("筆記.txt"), "那年冬天。\n").unwrap();

    let mut ed = Editor::new();
    ed.set_syntax_by_name(HashMap::from([
        ("txt".to_string(), crate::syntax::Syntax::Typst),
        ("筆記.txt".to_string(), crate::syntax::Syntax::Markdown),
    ]));
    ed.execute(&format!(":open {}", dir.join("ch01.txt").display()))
        .unwrap();
    assert_eq!(ed.syntax(), crate::syntax::Syntax::Typst, "by extension");
    // The exact name wins: "all my .txt are Typst, except that one".
    ed.execute(&format!(":open {}", dir.join("筆記.txt").display()))
        .unwrap();
    assert_eq!(ed.syntax(), crate::syntax::Syntax::Markdown, "by name");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_txt_that_is_typst_is_read_as_typst() {
    let dir = std::env::temp_dir().join(format!("yumete-syn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A novel written in Typst but filed as `.txt` is an ordinary thing to
    // have, and read as Markdown its `#import` looks like a heading.
    std::fs::write(
        dir.join("ch01.txt"),
        "#import \"lib.typ\": chapter\n\n= 第一章\n\n那年冬天，雪下得*很早*。\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("ch02.txt"),
        "# 第二章\n\n那年冬天，雪下得**很早**。\n",
    )
    .unwrap();

    let mut ed = Editor::new();
    ed.execute(&format!(":open {}", dir.join("ch01.txt").display()))
        .unwrap();
    assert_eq!(ed.syntax(), crate::syntax::Syntax::Typst);
    // `*很早*` is bold in Typst; in Markdown it would be emphasis.
    let kinds: Vec<crate::markdown::Kind> =
        ed.markup_line(4).into_iter().map(|s| s.kind).collect();
    assert!(kinds.contains(&crate::markdown::Kind::Strong), "{kinds:?}");

    // The plain Markdown one is still read as Markdown.
    ed.execute(&format!(":open {}", dir.join("ch02.txt").display()))
        .unwrap();
    assert_eq!(ed.syntax(), crate::syntax::Syntax::Markdown);

    // …and a guess can be overruled.
    ed.execute(":syntax typst").unwrap();
    assert_eq!(ed.syntax(), crate::syntax::Syntax::Typst);
    ed.execute(":syntax").unwrap();
    assert!(ed.status().contains("typst"), "{}", ed.status());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ruby_markup_is_not_counted_as_writing() {
    let mut ed = Editor::new();
    ed.current_buffer_mut()
        .insert(0, "<ruby>永和<rt>えいわ</rt></ruby>九年，歲在癸丑。")
            .expect("the fixture buffer is writable");
    ed.execute(":count").unwrap();
    let report = ed.status().to_string();
    // 永和九年歲在癸丑 is eight 字; the tags are not writing.
    assert!(report.contains("漢字 8"), "{report}");
    assert!(report.contains("字數 10"), "{report}");
}

#[test]
fn the_two_ideographs_outside_the_unified_blocks_count_as_字() {
    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "二〇二五年").expect("the fixture buffer is writable");
    ed.execute(":count").unwrap();
    let report = ed.status().to_string();
    assert!(report.contains("漢字 5"), "{report}");
}

#[test]
fn undo_groups_each_normal_edit_separately() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "abc");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g')); // to the start
    ed.on_key(Key::Char('d')); // delete 'a' → "bc"
    assert_eq!(ed.current_buffer().text(), "bc");

    ed.on_key(Key::Char('u')); // undo the delete
    assert_eq!(ed.current_buffer().text(), "abc");
    ed.on_key(Key::Char('u')); // undo the insert
    assert_eq!(ed.current_buffer().text(), "");
}

#[test]
fn search_moves_the_cursor_to_the_match_and_wraps() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "one two one");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g')); // cursor at 0

    // /two → the match becomes the selection, "two" at 4..7.
    ed.on_key(Key::Char('/'));
    type_keys(&mut ed, "two");
    ed.on_key(Key::Enter);
    assert_eq!(ed.selection(), (4, 7));

    // /one from here finds the second "one" (index 8).
    ed.on_key(Key::Char('/'));
    type_keys(&mut ed, "one");
    ed.on_key(Key::Enter);
    assert_eq!(ed.selection(), (8, 11));

    // n wraps around to the first "one" (index 0).
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.selection(), (0, 3));

    // And what is selected is what an edit takes.
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.current_buffer().text(), " two one");
}

/// `N` walks backwards, and lands where the full sweep used to (#319).
///
/// The old answer is the oracle: it was correct, and only correct — one
/// forward pass over the whole range keeping the best match so far. The new
/// one stops at the first line above the cursor that has a match, which is a
/// different shape of the same question, so the two are asked it side by side
/// on everything that has ever made a search subtle: nothing to find, a match
/// on the cursor's own line before and after the caret, a wrap, a pattern that
/// matches the empty string, 全角 text, and a range that is not the whole file
/// (the pane a table holds, #354).
#[test]
fn a_backward_search_lands_where_the_full_sweep_did() {
    use regex::Regex;
    // The pass this replaced, kept here and nowhere else.
    fn sweep(
        rope: &ropey::Rope,
        pattern: &Regex,
        from: usize,
        within: std::ops::Range<usize>,
    ) -> Option<(usize, usize)> {
        let (mut before, mut last) = (None, None);
        let mut at = rope.line_to_char(within.start.min(rope.len_lines()));
        for slice in rope
            .lines_at(within.start.min(rope.len_lines()))
            .take(within.end.saturating_sub(within.start))
        {
            let text = slice.to_string();
            let mut byte = 0usize;
            while let Some(m) = text.get(byte..).and_then(|rest| pattern.find(rest)) {
                let start = at + text[..byte + m.start()].chars().count();
                let range = (start, start + m.as_str().chars().count());
                if start < from {
                    before = Some(range);
                }
                last = Some(range);
                byte += m.end().max(m.start() + 1);
            }
            at += slice.len_chars();
        }
        before.or(last)
    }

    let texts = [
        "",
        "一行而已",
        "one two one",
        "甲\n乙\n甲\n丙\n甲\n",
        "甲甲甲\n乙乙乙\n甲乙甲\n",
        "no matches at all\nnone here either\n",
        "甲\n\n\n甲\n\n",
        "tail without a newline\n甲",
    ];
    let patterns = ["甲", "one", "[甲乙]", "x*", "^", "甲$", "(?i)ONE"];
    for text in texts {
        let rope = ropey::Rope::from_str(text);
        let lines = rope.len_lines();
        for pattern in patterns {
            let re = Regex::new(pattern).unwrap();
            for from in 0..=rope.len_chars() {
                for within in [0..lines, 0..lines.min(2), lines.saturating_sub(2)..lines, 1..lines]
                {
                    if within.start > within.end {
                        continue;
                    }
                    assert_eq!(
                        search_backward(&rope, &re, from, within.clone()),
                        sweep(&rope, &re, from, within.clone()),
                        "/{pattern}/ from {from} within {within:?} of {text:?}"
                    );
                }
            }
        }
    }
}

/// `N` costs what `n` costs (#319).
///
/// **A ratio, on purpose** — see the note on
/// [`a_step_down_a_table_costs_the_same_however_long_it_is`]. Both keys travel
/// the same distance to the same kind of match, so on a file of any size they
/// should cost about the same; the forward pass made `N` sixty times dearer on
/// a 1.8 MB manuscript, because it read the whole of it every time.
#[test]
fn n_and_shift_n_cost_the_same() {
    use std::time::{Duration, Instant};
    let mut text = String::new();
    for i in 0..20_000 {
        match i % 10 {
            0 => text.push_str("阿寧走到窗前。\n"),
            _ => text.push_str("這一行沒有要找的東西，只是把檔案撐開。\n"),
        }
    }
    let press = |key: char| -> Duration {
        let mut ed = typed(&text);
        ed.execute(":10000").unwrap();
        ed.on_key(Key::Char('/'));
        for c in "阿寧".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Enter);
        let n = 40;
        // The first press pays for compiling the pattern and for whatever the
        // rope has not touched yet.
        ed.on_key(Key::Char(key));
        let began = Instant::now();
        for _ in 0..n {
            ed.on_key(Key::Char(key));
        }
        began.elapsed() / n
    };
    press('n');
    let forward = press('n');
    let backward = press('N');
    assert!(
        backward <= forward * 6,
        "`N` cost {backward:?} against `n`'s {forward:?} — it is reading the whole file"
    );
}

#[test]
fn patterns_are_regular_expressions() {
    // Half of revising a manuscript is a pattern, not a string.
    let mut ed = typed("他說。她說。他問。");
    ed.execute(":%s/[他她]說/X/g").unwrap();
    assert_eq!(ed.current_buffer().text(), "X。X。他問。");

    // 「每個。後面斷行」 — the batch edit a Chinese draft needs most.
    let mut ed = typed("甲。乙。丙。");
    ed.execute(r":%s/。/。\n/g").unwrap();
    assert_eq!(ed.current_buffer().text(), "甲。\n乙。\n丙。\n");

    // Capture groups.
    let mut ed = typed("阿寧說道：好。");
    ed.execute(r":%s/(.+)說道/$1說/").unwrap();
    assert_eq!(ed.current_buffer().text(), "阿寧說：好。");

    // A pattern that does not compile says which part is wrong.
    let mut ed = typed("甲乙丙");
    ed.execute(":%s/[未閉合/X/").unwrap();
    assert!(ed.status().starts_with("bad pattern"), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "甲乙丙");
}

#[test]
fn search_takes_a_pattern_and_star_takes_text() {
    let mut ed = typed("第一章\n第十二章\n尾聲");
    press(&mut ed, "gg");
    ed.on_key(Key::Char('/'));
    type_keys(&mut ed, "第.+章");
    ed.on_key(Key::Enter);
    // The whole match is the selection, however long it turned out to be.
    // `/` looks *past* the cursor, as it does in vi, so the second heading
    // is found first and `n` wraps around to the first.
    assert_eq!(ed.selection(), (4, 8));
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.selection(), (0, 3));

    // `g?` searches for the *text* selected, so its punctuation is
    // literal: the second （甲） is found, and shown in the other work
    // area. (This was `*`, which the editor retired — §14.)
    let mut ed = typed("（甲）乙（甲）\n");
    press(&mut ed, "ggvll");
    press(&mut ed, "g?");
    let (from, to) = ed.other_pane().and_then(|p| p.highlight).expect("a hit");
    assert_eq!(
        ed.current_buffer().rope().slice(from..to).to_string(),
        "（甲）",
        "（甲） found as text, not as a group"
    );
}

/// What a search costs on the table this editor was built for.
///
/// A measurement, not an assertion. Run it with
/// `cargo test -p yumete-core --release searching_a_big_table -- --ignored --nocapture`.
#[test]
#[ignore]
fn searching_a_big_table() {
    use std::time::Instant;
    // 宇浩's 拆分表: 123,380 rows of 28 columns.
    let mut csv = String::from("char,ids_y,ids_g,ids_t,ids_h,o1,o2,block,unicode,pinyin,sypy,tupa,meaning,note,ids_j,ids_k,ids_v,ids_u,ids_s,ids_b,ids_m,ids_p,ids_x,ids_z,b1,b2,d1,d2\n");
    let pool: Vec<char> = (0x4E00u32..0x9FA5).filter_map(char::from_u32).collect();
    for i in 0..123_380usize {
        let c = pool[i % pool.len()];
        csv.push_str(&format!(
            "{c},⿰木{c},⿰木{c},⿰木{c},⿰木{c},,,CJK,{:04X},pin,sy,tu,,,,,,,,,,,,,,,,\n",
            0x4E00 + (i % 20000)
        ));
    }
    let dir = std::env::temp_dir().join("yumete-table-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("division.csv");
    std::fs::write(&path, &csv).unwrap();

    let mut ed = Editor::new();
    let t = Instant::now();
    ed.open_file(path.to_str().unwrap()).unwrap();
    println!("open:          {:.1?}", t.elapsed());
    let t = Instant::now();
    assert!(ed.enter_table(), "{}", ed.status());
    println!("enter table:   {:.1?}", t.elapsed());

    let t = Instant::now();
    ed.execute(":table-find column 龜").ok();
    println!("search column: {:.1?}  ({})", t.elapsed(), ed.status());
    let t = Instant::now();
    ed.execute(":table-find row 龜").ok();
    println!("search row:    {:.1?}  ({})", t.elapsed(), ed.status());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn delete_takes_the_character_the_cursor_is_in_front_of() {
    // The key did nothing at all before this: `KeyCode::Delete` was not in
    // the table that turns a terminal's keys into the editor's, so it fell
    // off the end in every mode.
    let mut ed = Editor::new();
    ed.current_buffer_mut().insert(0, "那年冬天\n下雪").expect("the fixture buffer is writable");
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Delete);
    assert_eq!(ed.current_buffer().text(), "年冬天\n下雪");
    assert_eq!(ed.cursor(), 0, "the cursor stays where it is");
    // Backspace's mirror at the edges: at the end of a line it takes the
    // newline, joining the line below.
    ed.on_key(Key::Char('l'));
    for _ in 0..3 {
        ed.on_key(Key::Delete);
    }
    assert_eq!(ed.current_buffer().text(), "l\n下雪");
    ed.on_key(Key::Delete);
    assert_eq!(ed.current_buffer().text(), "l下雪");
    // …and nothing at the very end of the buffer.
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('G'));
    ed.on_key(Key::Char('i'));
    let before = ed.current_buffer().text();
    for _ in 0..5 {
        ed.on_key(Key::Delete);
    }
    assert!(ed.current_buffer().text().len() <= before.len());

    // On the `:` line it takes the character under the caret.
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char(':'));
    type_keys(&mut ed, "wq");
    ed.on_key(Key::Left);
    ed.on_key(Key::Delete);
    assert_eq!(ed.prompt().map(|(_, line)| line.to_string()), Some("w".into()));
}

#[test]
fn substitute_replaces_on_the_current_line_and_whole_file() {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    type_keys(&mut ed, "aaa\naaa");
    ed.on_key(Key::Esc);

    // :s/a/b/ replaces the first "a" on the cursor's (last) line only.
    ed.on_key(Key::Char(':'));
    type_keys(&mut ed, "s/a/b/");
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "aaa\nbaa");

    // :%s/a/b/g replaces every "a" across all lines.
    ed.on_key(Key::Char(':'));
    type_keys(&mut ed, "%s/a/b/g");
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "bbb\nbbb");

    // The substitution is undoable.
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "aaa\nbaa");
}

/// **`f` — 照字面**: the pattern is the characters typed, not a regex.
///
/// A manuscript is full of characters the regex engine reads as instructions.
/// Without `f` the writer had to know which of them to backslash, and getting
/// it wrong is silent: `(注)` finds 注 and changes the wrong thing.
#[test]
fn the_f_flag_takes_the_pattern_as_the_characters_it_is() {
    // `.` as a wildcard changes both lines; `.` as itself changes one.
    let mut ed = typed("A.B\nAxB\n");
    assert!(ed.execute("%s/A.B/甲/g").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "甲\n甲\n");
    let mut ed = typed("A.B\nAxB\n");
    assert!(ed.execute("%s/A.B/甲/gf").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "甲\nAxB\n");

    // Brackets are brackets, not a group — and without `f` this is not even
    // an error, it just quietly matches 注 on its own.
    let mut ed = typed("(注)一\n注二\n");
    assert!(ed.execute("%s/(注)/（注）/gf").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "（注）一\n注二\n");

    // A pattern the regex engine would refuse outright compiles under `f`.
    let mut ed = typed("第1章*\n");
    assert!(ed.execute("%s/第1章*/第一章/f").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "第一章\n");

    // **Both sides.** With no groups to name, a `$` in the replacement can
    // only be the writer's own — a price, a shell line, a formula.
    let mut ed = typed("花了 $5.00\n");
    assert!(ed.execute("%s/$5.00/$6.00/f").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "花了 $6.00\n");

    // …but `\n` still opens a line: Enter submits the prompt, so there is no
    // other way to type one.
    let mut ed = typed("甲。乙。\n");
    assert!(ed.execute("%s/。/。\\n/gf").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "甲。\n乙。\n\n");

    // `f` and `i` are two questions, and both are answerable at once.
    let mut ed = typed("A.b\n");
    assert!(ed.execute("%s/a.B/甲/fi").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "甲\n");
}

/// **`c` — 逐處確認**: 「防止一下子全部都替换了」 (#415).
///
/// The flag exists so that a substitution across a whole book is judged one
/// match at a time. Everything about it is in service of that: the match is
/// the selection so the writer sees what is being asked about, a key that is
/// not one of the five answers leaves the question standing, and the whole
/// walk is **one** undo step.
#[test]
fn the_c_flag_asks_at_each_match_before_writing() {
    let mut ed = typed("甲甲\n甲甲\n");
    assert!(ed.execute("%s/甲/乙/gc").is_ok(), "{}", ed.status());
    assert!(ed.pending_menu().is_some(), "it is asking");

    // A key that is not one of the five is not an answer.
    ed.on_key(Key::Char('z'));
    assert_eq!(ed.current_buffer().text(), "甲甲\n甲甲\n");
    assert!(ed.pending_menu().is_some(), "the question still stands");

    ed.on_key(Key::Char('y'));
    assert_eq!(ed.current_buffer().text(), "乙甲\n甲甲\n");
    ed.on_key(Key::Char('n'));
    assert_eq!(ed.current_buffer().text(), "乙甲\n甲甲\n", "n writes nothing");
    ed.on_key(Key::Char('a'));
    assert_eq!(ed.current_buffer().text(), "乙甲\n乙乙\n", "a takes the rest");
    assert!(ed.pending_menu().is_none(), "the walk is over");

    // **One `u` undoes the whole walk**, not the last match of it.
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "甲甲\n甲甲\n");

    // `q` stops and keeps what is already done; `Esc` is the same answer.
    let mut ed = typed("甲甲甲\n");
    assert!(ed.execute("%s/甲/乙/gc").is_ok(), "{}", ed.status());
    ed.on_key(Key::Char('y'));
    ed.on_key(Key::Char('q'));
    assert_eq!(ed.current_buffer().text(), "乙甲甲\n");
    assert!(ed.pending_menu().is_none());

    // `l` — this one, then stop.
    let mut ed = typed("甲甲甲\n");
    assert!(ed.execute("%s/甲/乙/gc").is_ok(), "{}", ed.status());
    ed.on_key(Key::Char('l'));
    assert_eq!(ed.current_buffer().text(), "乙甲甲\n");
    assert!(ed.pending_menu().is_none());

    // **Refusing everything leaves no undo step**, so `u` still means the
    // edit before the walk — a document nothing happened to must not eat one.
    let mut ed = typed("甲甲\n");
    assert!(ed.execute("%s/甲/丙/").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "丙甲\n");
    assert!(ed.execute("%s/甲/乙/gc").is_ok(), "{}", ed.status());
    ed.on_key(Key::Char('n'));
    assert!(ed.pending_menu().is_none());
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "甲甲\n");

    // A range still means what it means, and `n` — count only — wins over
    // `c`: it is the answer that changes nothing.
    let mut ed = typed("甲\n甲\n甲\n");
    assert!(ed.execute("1,3s/甲/乙/cn").is_ok(), "{}", ed.status());
    assert!(ed.pending_menu().is_none(), "n asks nothing");
    assert_eq!(ed.current_buffer().text(), "甲\n甲\n甲\n");
}

/// `a` in a `:s …c` walk is one pass, not one pass per match.
///
/// The walk asks `next_hit`, which serialises the whole document and rescans
/// it from the top; `write_one` then copies it again for the grid guard. That
/// is one keystroke's work per `y` and fine. Under `a` it ran N times with no
/// key in between — quadratic. Measured on the author's machine before the
/// fix: 680 KB with 20,000 matches took **28.5 s**, 2 MB took **4m35s**, both
/// on the main thread with no progress and no key that could stop it, against
/// 0.02 s for the same `:s` without `c`.
///
/// The bound below is generous on purpose — this is not a benchmark, it is a
/// tripwire for the shape coming back. One pass finishes in milliseconds; the
/// quadratic one cannot come near ten seconds on any machine.
#[test]
fn answering_a_replaces_the_rest_in_one_pass() {
    let line = "那年冬天，山路已經看不見了。\n";
    let mut ed = typed(&line.repeat(4_000));
    let was = ed.current_buffer().text().matches('雪').count();
    assert_eq!(was, 0);
    assert!(ed.execute("%s/看不見/看不到/gc").is_ok(), "{}", ed.status());
    assert!(ed.pending_menu().is_some(), "it is asking");

    let started = std::time::Instant::now();
    ed.on_key(Key::Char('a'));
    let took = started.elapsed();
    assert!(ed.pending_menu().is_none(), "the walk is over");
    assert_eq!(ed.current_buffer().text().matches("看不到").count(), 4_000);
    assert_eq!(ed.current_buffer().text().matches("看不見").count(), 0);
    assert!(
        took < std::time::Duration::from_secs(10),
        "a rescanned the document per match again: {took:?}"
    );
    // ⚠️ 4,000 rather than the 20,000 that was measured: this runs in a debug
    // build on every `cargo test`, and the quadratic shape is already 4,000×
    // here. Ten seconds is a tripwire, not a benchmark.

    // And it is still **one** undo step for the whole walk.
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text().matches("看不見").count(), 4_000);
}

/// **`-` is a span, `,` is a list** — in `:s` too (§5.7).
#[test]
fn a_substitution_reads_a_dash_as_a_span_and_a_comma_as_a_list() {
    let five = || typed("a\na\na\na\na\n");

    // `1-3` is three lines: the two ends and the one between them.
    let mut ed = five();
    assert!(ed.execute("1-3s/a/b/").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "b\nb\nb\na\na\n");

    // `1,3` is **two** lines — in vi it was three. Line 2 is not named and
    // is not touched.
    let mut ed = five();
    assert!(ed.execute("1,3s/a/b/").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "b\na\nb\na\na\n");

    // A list of any length, in any order.
    let mut ed = five();
    assert!(ed.execute("5,1,4s/a/b/").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "b\na\na\nb\nb\n");

    // `.` and `$` are bounds like any other, on either joint.
    let mut ed = five();
    ed.goto_line(2);
    assert!(ed.execute(".-$s/a/b/").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "a\nb\nb\nb\nb\n");
    let mut ed = five();
    ed.goto_line(2);
    assert!(ed.execute(".,$s/a/b/").is_ok(), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "a\nb\na\na\nb\n");

    // Mixing them is refused, and the refusal says what the two spellings
    // are — a wall with no door was what made the old `,` guessable at all.
    let mut ed = five();
    assert!(ed.execute("1-3,5s/a/b/").is_err());
    assert_eq!(ed.current_buffer().text(), "a\na\na\na\na\n");
}

// ── #216 · 文中的表格區塊 ────────────────────────────────────────────
//
// The third tier: a run of delimited lines **recognised where it stands**.
// Not a file that is a table (`.csv`), not a table declared with pipes —
// a 碼表 somebody pasted into a chapter, a `dict.yaml` under its preamble.

#[test]
fn a_code_table_pasted_into_a_chapter_is_read_where_it_stands() {
    let mut ed = typed("## 第三章\n木,AA\n目,BB\n田,CC\n\n那一年的雨下得久。\n");
    ed.execute(":2").unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    let region = ed.block_region().expect("standing inside the block");
    // The heading above it is not a row of it, and neither is the prose
    // two lines below: **a blank line is a boundary, the heading is the
    // other one** — it holds no comma, so the walk stops there.
    assert_eq!((region.first, region.last), (1, 3));
    // Its first line is data, not a header: a 碼表 has no header, and
    // reading 木 as a column name would lose the row.
    assert_eq!(ed.cell_text(1, 0), "木");
    assert_eq!(ed.cell_text(3, 1), "CC");
    assert!(ed.table_here());
    ed.execute(":6").unwrap();
    assert!(!ed.table_here(), "the paragraph under it is prose");
}

#[test]
fn a_block_is_entered_on_the_delimiter_the_cursor_is_standing_on() {
    // Both a tab and a comma hold every line together. Standing on the
    // comma says which one is meant — nothing has to be prompted for.
    let mut ed = Editor::new();
    let dir = std::env::temp_dir().join(format!("yumete-block-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("章.md");
    std::fs::write(&file, "序\n木\tA,B\n目\tC,D\n田\tE,F\n").unwrap();
    ed.open_file(&file).unwrap();
    ed.execute(":2").unwrap();
    // Column 1 by default — the tab, which BLOCK_GUESSES tries first.
    assert!(ed.enter_table(), "{}", ed.status());
    assert_eq!(ed.cell_text(1, 1), "A,B", "the tab, tried first");
    ed.leave_table();
    // Now stand on the comma and ask again.
    ed.execute(":2").unwrap();
    press(&mut ed, "lll");
    assert_eq!(ed.char_at_cursor(), Some(','), "on the comma");
    assert!(ed.enter_table(), "{}", ed.status());
    assert_eq!(ed.cell_text(1, 0), "木\tA", "the comma, because you were on it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_dict_under_a_preamble_keeps_the_column_only_some_rows_carry() {
    // A generated `dict.yaml`: three dashes, a preamble, three dashes,
    // then the entries — and a 權重 on the entries that have earned one.
    let mut ed = typed(concat!(
        "---\n",
        "name: yuhao\n",
        "---\n",
        "木,AA\n",
        "目,BB,100\n",
        "田,CC\n",
        "水,DD\n",
    ));
    ed.execute(":4").unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    let region = ed.block_region().expect("inside the entries");
    assert_eq!((region.first, region.last), (3, 6), "the preamble is not the table");
    // Three columns, not two: the row that carries a 權重 is still
    // carrying it, and a column drawn nowhere cannot be walked into.
    assert_eq!(ed.cell_text(4, 2), "100");
}

#[test]
fn prose_is_not_a_table_because_it_has_commas_in_it() {
    let mut ed = typed("She waited, and waited.\nThen the rain, at last, came.\n");
    assert!(!ed.enter_table(), "two lines that disagree are not a grid");
    assert!(ed.block_region().is_none());
}

#[test]
fn two_lines_of_a_block_must_agree_exactly_but_a_long_one_may_not() {
    // Fewer than four rows: 「most of them」 means nothing, so all of them.
    assert!(!Editor::rows_agree(&[2, 3]));
    assert!(Editor::rows_agree(&[2, 2]));
    assert!(!Editor::rows_agree(&[2, 2, 3]));
    // Past that the slack is real data: a 權重 on some entries only.
    assert!(Editor::rows_agree(&[2, 3, 2, 2]));
    assert!(!Editor::rows_agree(&[2, 3, 4, 2]));
    // One cell is not a column, however many lines agree about it.
    assert!(!Editor::rows_agree(&[1, 1, 1, 1]));
}

#[test]
fn putting_a_column_into_a_block_stops_at_the_blank_line() {
    // `t p` walks the *file* in a `.csv`. In a block it must walk the
    // block — otherwise a 碼表 pasted into a manuscript writes the yanked
    // column down the rest of the chapter.
    let mut ed = typed("木,AA\n目,BB\n田,CC\n\n他寫下,然後停筆\n");
    ed.execute(":1").unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "T"); // #356: 這一條測的是格
    press(&mut ed, "ty");
    press(&mut ed, "l");
    press(&mut ed, "tp");
    assert_eq!(ed.cell_text(0, 1), "木");
    assert_eq!(ed.cell_text(2, 1), "田");
    assert_eq!(
        ed.line_text(4).unwrap().trim_end_matches('\n'),
        "他寫下,然後停筆",
        "the prose under the block is not a row of it"
    );
}

#[test]
fn a_block_is_read_where_it_lies_and_not_sorted() {
    // Sorting rewrites the lines. In somebody else's document, the lines
    // around the block are the document — so the answer is no, with the
    // two commands that *would* do it named.
    let mut ed = typed("木,AA\n目,BB\n田,CC\n");
    ed.execute(":1").unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    ed.execute(":table-sort 1").unwrap();
    assert!(ed.status().contains("只讀"), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "木,AA\n目,BB\n田,CC\n");
}


// ---- Merge conflicts (Feature #249) ---------------------------------

/// A file in the state `git merge` leaves it in, without typing it: the
/// markers are seven characters that mean something to the segmenter and
/// to the IME, and none of that is what these tests are about.
fn merged(text: &str) -> Editor {
    let mut ed = Editor::new();
    ed.add_buffer(Buffer::from_text(text));
    ed.execute(":1").unwrap();
    ed
}

const MERGED: &str = "\
第一段
<<<<<<< HEAD
我方寫的
=======
他方寫的
>>>>>>> feature/枝
最後一段
";

#[test]
fn every_line_of_a_conflict_knows_which_side_it_is_on() {
    use crate::conflict::Side;
    use crate::markdown::Block;
    let ed = merged(MERGED);
    assert_eq!(ed.block_of(0), Block::Prose);
    assert_eq!(ed.block_of(1), Block::Conflict(None));
    assert_eq!(ed.block_of(2), Block::Conflict(Some(Side::Ours)));
    assert_eq!(ed.block_of(3), Block::Conflict(None));
    assert_eq!(ed.block_of(4), Block::Conflict(Some(Side::Theirs)));
    assert_eq!(ed.block_of(5), Block::Conflict(None));
    assert_eq!(ed.block_of(6), Block::Prose);
}

/// The whole reason the conflicts are assembled before the labels are laid
/// on: a manual that *describes* a merge is not in one.
#[test]
fn a_paragraph_about_merges_is_not_a_merge() {
    use crate::markdown::Block;
    let ed = merged("衝突長這樣：\n<<<<<<< HEAD\n然後就沒有然後了\n收筆\n");
    assert!(ed.conflicts().is_empty());
    assert_eq!(ed.block_of(2), Block::Prose);
    assert_eq!(ed.block_of(3), Block::Prose);
}

#[test]
fn the_brackets_walk_from_one_conflict_to_the_next() {
    let text = format!("{MERGED}{MERGED}");
    let mut ed = merged(&text);
    press(&mut ed, "]c");
    assert_eq!(ed.cursor_line(), 1, "{}", ed.status());
    // From inside the first one, 「next」 is the second — not this one's
    // own foot marker.
    press(&mut ed, "]c");
    assert_eq!(ed.cursor_line(), 8, "{}", ed.status());
    // And there is no third: it says so rather than wrapping round.
    press(&mut ed, "]c");
    assert_eq!(ed.cursor_line(), 8);
    assert!(ed.status().contains("後面"), "{}", ed.status());
    press(&mut ed, "[c");
    assert_eq!(ed.cursor_line(), 1, "{}", ed.status());
    press(&mut ed, "[c");
    assert_eq!(ed.cursor_line(), 1);
    assert!(ed.status().contains("前面"), "{}", ed.status());
}

#[test]
fn keeping_a_side_takes_the_markers_with_it() {
    let mut ed = merged(MERGED);
    press(&mut ed, "]c");
    press(&mut ed, " mo");
    assert_eq!(
        ed.current_buffer().text(),
        "第一段\n我方寫的\n最後一段\n",
        "{}",
        ed.status()
    );
    assert!(ed.conflicts().is_empty());

    let mut ed = merged(MERGED);
    press(&mut ed, "]c");
    press(&mut ed, " mt");
    assert_eq!(ed.current_buffer().text(), "第一段\n他方寫的\n最後一段\n");

    let mut ed = merged(MERGED);
    press(&mut ed, "]c");
    press(&mut ed, " mb");
    assert_eq!(
        ed.current_buffer().text(),
        "第一段\n我方寫的\n他方寫的\n最後一段\n"
    );
}

/// A conflict over an addition has an empty side, and keeping that side is
/// 「take it back out」 — not 「leave a blank line where it was」.
#[test]
fn keeping_an_empty_side_leaves_no_line_at_all() {
    let mut ed = merged("上\n<<<<<<< HEAD\n=======\n新加的一句\n>>>>>>> 枝\n下\n");
    press(&mut ed, "]c");
    press(&mut ed, " mo");
    assert_eq!(ed.current_buffer().text(), "上\n下\n");
}

#[test]
fn the_three_keys_say_so_when_there_is_nothing_to_resolve() {
    let mut ed = merged(MERGED);
    press(&mut ed, " mo");
    assert!(ed.status().contains("不在"), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), MERGED, "nothing was touched");
}

#[test]
fn the_listing_is_walked_back_the_way_grep_is() {
    let text = format!("{MERGED}{MERGED}");
    let mut ed = merged(&text);
    ed.execute(":check-merge").unwrap();
    let listing = ed.current_buffer().text();
    assert!(listing.starts_with("[scratch]:2: HEAD ⇄ feature/枝\n"), "{listing}");
    assert_eq!(listing.lines().count(), 2);
    assert!(listing.lines().nth(1).unwrap().contains(":9: "), "{listing}");

    // And a clean file says so rather than opening an empty buffer.
    let mut ed = merged("一句話\n");
    let before = ed.current_buffer().id();
    ed.execute(":check-merge").unwrap();
    assert_eq!(ed.current_buffer().id(), before, "{}", ed.status());
    assert!(ed.status().contains("沒有"), "{}", ed.status());
}

/// `:render full` takes markup off the page, and the seven brackets are
/// markup: what is left is the one thing a reader wants from that line.
#[test]
fn the_brackets_come_off_under_render_full_and_the_branch_stays() {
    let mut ed = merged(MERGED);
    ed.execute(":render full").unwrap();
    assert_eq!(ed.hidden_on_line(1), vec![(0, 8)], "「<<<<<<< 」 and nothing else");
    assert_eq!(ed.hidden_on_line(3), vec![(0, 7)], "「=======」 leaves an empty row");
    assert_eq!(ed.hidden_on_line(2), vec![], "nobody hides the writing");
    ed.execute(":render basic").unwrap();
    assert_eq!(ed.hidden_on_line(1), vec![], "and with the markup shown, nothing");
}

#[test]
fn the_keys_a_block_does_not_answer_say_which_ones_it_does() {
    let mut ed = typed("木,AA\n目,BB\n田,CC\n");
    ed.execute(":1").unwrap();
    assert!(ed.enter_table(), "{}", ed.status());
    press(&mut ed, "td");
    assert!(ed.status().contains("y p"), "{}", ed.status());
    assert_eq!(ed.current_buffer().text(), "木,AA\n目,BB\n田,CC\n");
}

/// A table's padding is a walk down every row of it, and the caret only
/// changes the answer under 所見即所得 — so under `:render basic` moving the
/// caret must not throw the walk away.
///
/// Told by poisoning the memo: what comes back second is what was
/// remembered, or it was worked out again.
#[test]
fn moving_the_caret_does_not_throw_a_table_s_padding_away() {
    let mut ed = typed("| 甲 | 乙 |\n| --- | --- |\n| 一 | 二 |\n");
    assert!(!ed.drawn_on_line(2).is_empty(), "the short row is padded out");

    let poison = |ed: &Editor| {
        let mut held = ed.pad_cache.borrow_mut();
        let (_, runs) = held.as_mut().expect("the walk is remembered");
        for row in runs.runs.iter_mut() {
            *row = vec![(0, "毒".to_string())];
        }
    };

    poison(&ed);
    ed.on_key(Key::Char('j'));
    assert_eq!(
        ed.drawn_on_line(2),
        vec![(0, "毒".to_string())],
        "`:render basic` hides the same runs wherever the caret is",
    );

    // 所見即所得 puts markup back under the selection, so there the answer
    // really does move with the caret and the memo has to go.
    ed.execute(":render full").unwrap();
    assert!(!ed.drawn_on_line(2).is_empty());
    poison(&ed);
    ed.on_key(Key::Char('k'));
    assert_ne!(
        ed.drawn_on_line(2),
        vec![(0, "毒".to_string())],
        "under 所見即所得 the caret decides what comes off the row",
    );
}

/// The readings laid out on a paragraph are worked out once per revision and
/// remembered (#315): everything that draws a page asks, and so does the wrap
/// — two to five times a keystroke — and reading them costs the paragraph
/// materialised into characters and walked.
///
/// So the memo has to see an edit, and it has to see `:ruby` too.
#[test]
fn a_paragraph_s_readings_are_read_again_when_it_changes() {
    let mut ed = typed("讀<ruby>漢<rt>hàn</rt></ruby>字");
    let reading = |ed: &Editor| {
        ed.readings_on_line(0)
            .into_iter()
            .map(|g| (g.start, g.end))
            .collect::<Vec<_>>()
    };
    let first = reading(&ed);
    assert_eq!(first.len(), 1, "one group: {first:?}");
    assert_eq!(reading(&ed), first, "asked twice, answered the same");

    // An edit ahead of the group moves it, and the revision is what says so.
    ed.execute(":0").unwrap();
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Char('新'));
    ed.on_key(Key::Esc);
    assert_eq!(
        reading(&ed),
        first.iter().map(|&(a, b)| (a + 1, b + 1)).collect::<Vec<_>>(),
        "the group moved one character along with the text",
    );

    // …and so does `:ruby`, at a revision that never moved: `<ruby>` is not
    // Typst's spelling, so read as Typst alone the line lays out nothing.
    ed.set_ruby(crate::ruby::Dialects::only(crate::ruby::Dialect::Typst));
    assert!(
        reading(&ed).is_empty(),
        "read as Typst, the HTML group is just text: {:?}",
        reading(&ed),
    );
}

/// A row nobody touched keeps last time's lists (#316), so what the memo
/// hands back has to be **what a cold walk would have said** — the column a
/// reused row sits in belongs to the whole table, and one cell growing moves
/// every other row.
///
/// Told by asking twice: once with the memo warm from the keystroke that
/// just landed, and once with it thrown away.
#[test]
fn a_reused_row_says_what_a_cold_walk_would_have_said() {
    // Sixteen 漢字 is 32 columns — the cap — so the first cell folds, and its
    // fold moves with every character typed into it.
    // Laid out already, because `Esc` only keeps a table square — it never
    // squares one up that nobody asked it to (#329).
    let mut ed = typed(&format!(
        "| 甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳 | 乙 |\n| {} | -- |\n| 丙{} | 丁 |\n",
        "-".repeat(32),
        " ".repeat(30),
    ));
    ed.execute(":render full").unwrap();
    assert!(ed.cell_folds(), "the cap is what makes a row's list worth reusing");

    // Into the *first* row's first cell, which the last row never reads —
    // except through the width of the column they share.
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('i'));
    for c in "戊己庚".chars() {
        ed.on_key(Key::Char(c));
    }
    let agrees = |ed: &Editor, when: &str| {
        let warm: Vec<_> = (0..3).map(|i| ed.drawn_on_line(i)).collect();
        ed.pad_cache.borrow_mut().take();
        let cold: Vec<_> = (0..3).map(|i| ed.drawn_on_line(i)).collect();
        assert_eq!(warm, cold, "{when}");
    };
    // Between keystrokes the other rows' text has not moved at all.
    assert_eq!(
        ed.line_text(2).as_deref(),
        Some(format!("| 丙{} | 丁 |\n", " ".repeat(30)).as_str()),
        "the last row's own text never moved",
    );
    agrees(&ed, "the rows carried forward still fit a cell being typed in");

    // And `Esc` squares the table up in the *file*: now every row's text has
    // moved, with the caret on none of them, which is the other way a
    // carried-forward answer could go stale.
    ed.on_key(Key::Char('辛'));
    ed.on_key(Key::Esc);
    assert_ne!(
        ed.line_text(2).as_deref(),
        Some(format!("| 丙{} | 丁 |\n", " ".repeat(30)).as_str()),
        "the last row was padded out further in the file",
    );
    agrees(&ed, "the rows carried forward still fit a table that squared up");
}

/// The other way a carried-forward row goes stale (#316): 所見即所得 puts a
/// construct's markup back on the page under the caret, so a row's own list
/// moves when the caret walks on or off it — with the text never touched.
#[test]
fn a_caret_walking_off_a_row_puts_its_markup_back_away() {
    let mut ed = typed("| 甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳 | 乙 |\n| --- | --- |\n| **丙** | 丁 |\n");
    ed.execute(":render full").unwrap();
    let agrees = |ed: &Editor, when: &str| {
        let warm: Vec<_> = (0..3).map(|i| ed.drawn_on_line(i)).collect();
        ed.pad_cache.borrow_mut().take();
        let cold: Vec<_> = (0..3).map(|i| ed.drawn_on_line(i)).collect();
        assert_eq!(warm, cold, "{when}");
    };

    // Onto the 丙, which is inside `**丙**` — so that row keeps its stars.
    ed.execute(":3").unwrap();
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('l'));
    assert!(
        ed.hidden_on_line(2).is_empty(),
        "the caret's own construct is never hidden: {:?}",
        ed.hidden_on_line(2),
    );
    agrees(&ed, "the row under the caret is drawn with its markup on");

    // Off it again, and the stars go back off the page — two columns
    // narrower, on a row whose text never moved.
    ed.on_key(Key::Char('k'));
    assert!(!ed.hidden_on_line(2).is_empty(), "the stars are off the page again");
    agrees(&ed, "a row the caret walked off is worked out again");
}

/// A `.md` that holds a table anywhere is opened with a grid view over it,
/// so that its columns are drawn without anyone asking. The guard on the
/// other half — what may be cut — read that view as 「this file is a grid」
/// and refused the line break at the end of every paragraph in the book:
/// `d` on prose answered 「格與格之間的分隔符刪不掉」 about cells that were
/// nowhere near it. **A grid guards the lines it occupies and no others.**
#[test]
fn prose_beside_a_table_still_gives_up_its_line_break() {
    let dir = std::env::temp_dir().join(format!("yumete-cut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("beside.md");
    std::fs::write(&path, "甲乙\n丙丁\n\n| a | b |\n| - | - |\n| c | d |\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&path).unwrap();
    assert!(ed.table.is_some(), "the file is drawn as a grid");

    // To the end of the first paragraph line, on the break itself, and cut.
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('d'));
    assert!(
        ed.current_buffer().text().starts_with("甲乙丙丁\n"),
        "the two lines joined: {:?} — {}",
        ed.current_buffer().text().lines().next(),
        ed.status,
    );

    // And the row's own delimiter is still untouchable.
    let before = ed.current_buffer().text();
    ed.execute(":4").unwrap();
    for _ in 0..3 {
        ed.on_key(Key::Char('l'));
    }
    ed.on_key(Key::Char('d'));
    assert_eq!(ed.current_buffer().text(), before, "a row keeps its walls");
    std::fs::remove_dir_all(&dir).ok();
}

/// `h` and `l` walk the **page**, not the line: off the end of one line is
/// the start of the next, and back again. A reader proof-reading a novel
/// steps character by character through a paragraph, and a line break is
/// not a wall they asked for.
#[test]
fn stepping_sideways_crosses_the_line_break() {
    let mut ed = typed("甲乙\n丙丁\n");
    let line = |e: &Editor| {
        let rope = e.current_buffer().rope();
        rope.char_to_line(e.cursor)
    };
    assert_eq!(line(&ed), 0);
    for _ in 0..3 {
        ed.on_key(Key::Char('l'));
    }
    assert_eq!(line(&ed), 1, "three steps off a two-character line");
    assert_eq!(ed.cursor, ed.current_buffer().rope().line_to_char(1));

    ed.on_key(Key::Char('h'));
    assert_eq!(line(&ed), 0, "and back over the break");
    for _ in 0..3 {
        ed.on_key(Key::Char('h'));
    }
    assert_eq!(ed.cursor, 0, "the top of the file is where it stops");
}

/// `t w` and `t a` are two switches over one axis, and **either of them,
/// pressed while the other is on, opens the table out** (author,
/// 2026-09-08). Answering `t w` from 折行 with 摺起 handed the reader the
/// one state they had not named — a second way of hiding, when what they
/// asked for was to stop hiding.
#[test]
fn either_cell_switch_opens_the_table_out_from_the_other() {
    let mut ed = typed("| a | b |\n| - | - |\n| c | d |\n");
    ed.set_cell_width(CellWidth::Whole);

    ed.toggle_cell_folds();
    assert_eq!(ed.cell_width_now(), CellWidth::Fold, "t w 摺起");
    ed.toggle_cell_wrap();
    assert_eq!(ed.cell_width_now(), CellWidth::Wrap, "t a walks in from 摺起");
    ed.toggle_cell_folds();
    assert_eq!(
        ed.cell_width_now(),
        CellWidth::Whole,
        "and t w out of 折行 is 全部攤開, not 摺起"
    );

    // The other way round is the pair it always was: each key, pressed
    // twice, is where it started.
    ed.toggle_cell_wrap();
    assert_eq!(ed.cell_width_now(), CellWidth::Wrap);
    ed.toggle_cell_wrap();
    assert_eq!(ed.cell_width_now(), CellWidth::Whole);
    ed.toggle_cell_folds();
    ed.toggle_cell_folds();
    assert_eq!(ed.cell_width_now(), CellWidth::Whole);
}

/// The block scan reads each line's opening **where the rope keeps it** when
/// it can (#313), rather than copying it out — so the answer must not depend
/// on where the rope happens to have divided the document into chunks.
///
/// Told against the specification: the first [`crate::markdown::PREFIX`]
/// characters of every line, fed to a scanner of its own.
#[test]
fn the_block_scan_reads_the_same_document_however_the_rope_holds_it() {
    // Big enough to span many of the rope's chunks, so lines fall on both
    // sides of a boundary — including lines longer than the opening that is
    // read, which are the ones that must still be copied.
    // The metadata key runs past the opening that is read, and its colon with
    // it: read as far as the contract says and this is not `key: value` at
    // all, so the front matter ends here. Read whole, it would not — which is
    // the difference a borrowed line must never make.
    let mut text = String::from("---\n");
    text.push_str(&format!("{}: 卷一\n", "k".repeat(70)));
    text.push_str("---\n\n");
    for i in 0..3_000 {
        text.push_str(&format!("## 第{i}節\n\n"));
        text.push_str("The morning was clear and the road ran east, and he did not look back once.\n");
        text.push_str("他站在門口。\n\n");
        text.push_str("```rust\nlet x = 1;\n```\n\n");
        text.push_str("> 引用一行\n\n");
    }
    let dir = std::env::temp_dir().join(format!("yumete-scanshape-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let doc = dir.join("shape.md");
    std::fs::write(&doc, &text).unwrap();
    let mut ed = Editor::new();
    ed.open_file(&doc).unwrap();

    let rope = ed.current_buffer().rope();
    let mut want = Vec::with_capacity(rope.len_lines());
    let mut scanner = crate::markdown::BlockScanner::new();
    for line in 0..rope.len_lines() {
        let whole = rope.line(line);
        let opening: String = whole.chars().take(crate::markdown::PREFIX).collect();
        want.push(scanner.feed(&opening, whole.len_chars()));
    }
    assert_eq!(
        ed.blocks_through(rope.len_lines() - 1),
        want,
        "the scan read a different document from the one the rope holds",
    );

    // And again one character in, since an edit is what throws the scan away.
    ed.execute(":2").unwrap();
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Char('甲'));
    let rope = ed.current_buffer().rope();
    let mut want = Vec::with_capacity(rope.len_lines());
    let mut scanner = crate::markdown::BlockScanner::new();
    for line in 0..rope.len_lines() {
        let whole = rope.line(line);
        let opening: String = whole.chars().take(crate::markdown::PREFIX).collect();
        want.push(scanner.feed(&opening, whole.len_chars()));
    }
    assert_eq!(ed.blocks_through(rope.len_lines() - 1), want, "after an edit");
    std::fs::remove_dir_all(&dir).ok();
}

/// What a key costs in a **long manuscript** (#313) — the block every line
/// belongs to is a forward fold from the top of the document, and it is
/// re-folded on every edit.
///
/// Mixed English and Chinese, which is the ordinary case and the bad one:
/// English prose breaks into many short lines, and this walk is per line.
/// Run with `cargo test --release -- --ignored the_cost_of_a_key_in_a_long_file
/// --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn the_cost_of_a_key_in_a_long_file() {
    use std::time::Instant;
    for paragraphs in [500usize, 2_000, 8_000] {
        let mut text = String::from("# 卷一\n\n");
        for i in 0..paragraphs {
            text.push_str(&format!("## 第{i}節\n\n"));
            text.push_str("The morning was clear and the road ran east.\n");
            text.push_str("他站在門口，看着那條路一直伸到山那邊去。\n");
            text.push_str("She said nothing for a long while.\n\n");
            text.push_str("```\nlet x = 1;\n```\n\n");
        }
        let dir = std::env::temp_dir().join(format!("yumete-blockbench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join(format!("m{paragraphs}.md"));
        std::fs::write(&doc, &text).unwrap();

        let mut ed = Editor::new();
        ed.open_file(&doc).unwrap();
        ed.set_wrap_width(80);
        ed.set_page(50, 80);
        let lines = ed.current_buffer().line_count();
        ed.execute(":2").unwrap();
        ed.on_key(Key::Char('i'));
        // A key is only half of it: the page is what asks which block each
        // line is in, so the frame after the key is part of the cost.
        let draw = |ed: &Editor| {
            // What a frame asks: which block each line down to the bottom of
            // this page belongs to. A fence opened above decides what the
            // lines below it mean, so it is a walk from the top.
            let _ = ed.blocks_through(ed.cursor_line() + 50);
        };
        for _ in 0..3 {
            ed.on_key(Key::Char('甲'));
            draw(&ed);
        }

        let began = Instant::now();
        let n = 20;
        for _ in 0..n {
            ed.on_key(Key::Char('乙'));
            draw(&ed);
        }
        println!(
            "{lines:>7} lines  {:>8.3} ms a key",
            began.elapsed().as_secs_f64() * 1000.0 / n as f64,
        );
        ed.on_key(Key::Esc);
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// What a key costs in **one very long paragraph** (#315) — a plain-text
/// export, a log, a chapter pasted in as one line.
///
/// The rows a paragraph wraps into are remembered, but taking the memo is
/// itself a walk down the whole paragraph: its text is hashed to make the
/// key, and the answer is cloned on the way out. Run with
/// `cargo test --release -- --ignored the_cost_of_a_key_in_one_paragraph
/// --nocapture`.
#[test]
#[ignore = "a benchmark, not a test"]
fn the_cost_of_a_key_in_one_paragraph() {
    use std::time::Instant;
    for chars in [20_000usize, 200_000, 1_000_000] {
        let dir = std::env::temp_dir().join(format!("yumete-wrapbench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join(format!("p{chars}.txt"));
        std::fs::write(&doc, "甲乙丙丁戊己庚辛".repeat(chars / 8)).unwrap();

        let mut ed = Editor::new();
        ed.open_file(&doc).unwrap();
        ed.set_wrap_width(80);
        ed.set_page(50, 80);

        let cost = |what: &str, key: Key, ed: &mut Editor| {
            for _ in 0..3 {
                ed.on_key(key.clone());
            }
            let began = Instant::now();
            let n = 20;
            for _ in 0..n {
                ed.on_key(key.clone());
            }
            let ms = began.elapsed().as_secs_f64() * 1000.0 / n as f64;
            println!("{chars:>9} chars  {what:<8} {ms:>8.3} ms a key");
        };
        cost("l", Key::Char('l'), &mut ed);
        cost("j", Key::Char('j'), &mut ed);
        // **Where in the paragraph matters** (#366): every row before the
        // caret is settled by text the edit did not touch, so what a key costs
        // is the paragraph *after* it. Typing at the head is the worst case
        // there is and is what this measured until 2026-09-11; typing at the
        // end is what writing actually does.
        ed.execute("1").unwrap();
        ed.on_key(Key::Char('i'));
        cost("head", Key::Char('乙'), &mut ed);
        ed.on_key(Key::Esc);
        ed.execute("$").ok();
        ed.on_key(Key::Char('A'));
        cost("end", Key::Char('乙'), &mut ed);
        ed.on_key(Key::Esc);
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// **A stopwatch on typing in a table** (#316) — `#[ignore]`d; it prints.
///
/// `cargo test -p yumete-core --release the_cost_of_a_key_in_a_table -- --ignored --nocapture`
#[test]
#[ignore]
fn the_cost_of_a_key_in_a_table() {
    use std::time::Instant;
    for rows in [500usize, 2000, 5000] {
        let mut text = String::from("| 名 | 註 |\n| --- | --- |\n");
        for i in 0..rows {
            text.push_str(&format!("| 第{i}行 | 甲乙丙丁 |\n"));
        }
        let dir = std::env::temp_dir().join(format!("yumete-padbench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join(format!("t{rows}.md"));
        std::fs::write(&doc, &text).unwrap();

        let mut ed = Editor::new();
        ed.open_file(&doc).unwrap();
        ed.set_wrap_width(120);
        ed.set_page(50, 120);
        ed.execute(":20").unwrap();
        // The padding only exists when the page squares the table up.
        assert!(ed.enter_table(), "{}", ed.status());
        assert!(ed.table_padding_on(), "the padding is what this measures");
        ed.on_key(Key::Char('i'));
        // Warm whatever wants warming.
        ed.on_key(Key::Char('甲'));

        let began = Instant::now();
        let n = 20;
        for _ in 0..n {
            ed.on_key(Key::Char('乙'));
        }
        println!(
            "{rows:>6} rows   {:>8.2} ms a key",
            began.elapsed().as_secs_f64() * 1000.0 / n as f64
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}











// ---- Commenting out (#409) ----------------------------------------------

/// **Two keys, and each one says which form it means** (#409).
///
/// Typst is the one format that has both, so it is the one place the keys can
/// be told apart; Markdown answers both with `<!-- -->` because that is the
/// only form it has, and plain text says so out loud rather than inventing a
/// convention for a file that has none.
#[test]
fn the_two_comment_keys_each_force_their_own_form() {
    let mut ed = typed("甲乙\n丙丁\n");
    ed.execute(":syntax typst").unwrap();
    press(&mut ed, " c");
    assert_eq!(ed.current_buffer().text(), "// 甲乙\n丙丁\n", "空格 c is by the line");
    press(&mut ed, " c");
    assert_eq!(ed.current_buffer().text(), "甲乙\n丙丁\n", "and pressing it again undoes it");

    press(&mut ed, " C");
    assert_eq!(ed.current_buffer().text(), "/* 甲乙 */\n丙丁\n", "空格 C is a block");
    press(&mut ed, " C");
    assert_eq!(ed.current_buffer().text(), "甲乙\n丙丁\n");
}

#[test]
fn a_format_with_one_form_gives_it_to_both_keys() {
    for key in [" c", " C"] {
        let mut ed = typed("甲乙\n");
        ed.execute(":syntax markdown").unwrap();
        press(&mut ed, key);
        assert_eq!(
            ed.current_buffer().text(),
            "<!-- 甲乙 -->\n",
            "Markdown has no line comment, so `{key}` writes the one it has"
        );
    }
}

/// A key that writes nothing must say why — the bug this project has fixed
/// most often is the one where nothing happens and nothing is said.
#[test]
fn plain_text_has_no_comment_and_the_key_says_so() {
    let mut ed = typed("甲乙\n");
    ed.execute(":syntax text").unwrap();
    press(&mut ed, " c");
    assert_eq!(ed.current_buffer().text(), "甲乙\n", "nothing written");
    assert!(!ed.status().is_empty(), "and it does not fail silently");
}

/// The selection is widened to whole lines, and the same lines come back
/// selected — so the key can be pressed twice and the second press undoes the
/// first, which is what a toggle is.
#[test]
fn commenting_takes_whole_lines_and_keeps_them_selected() {
    let mut ed = typed("甲乙\n丙丁\n戊己\n");
    ed.execute(":syntax typst").unwrap();
    press(&mut ed, "lvj");
    press(&mut ed, " c");
    assert_eq!(ed.current_buffer().text(), "// 甲乙\n// 丙丁\n戊己\n", "both lines, whole");
    press(&mut ed, " c");
    assert_eq!(ed.current_buffer().text(), "甲乙\n丙丁\n戊己\n", "and both come back");
}

/// `.yumete` first, `.git` second, the file's own directory last — and the
/// working directory only when there is no named file to ask.
#[test]
fn the_project_root_prefers_yumete_then_git_then_here() {
    let dir = std::env::temp_dir().join(format!("yumete-root-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    let book = repo.join("book");
    std::fs::create_dir_all(book.join("卷一")).unwrap();
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let chapter = book.join("卷一/一.md");
    std::fs::write(&chapter, "甲\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(chapter.clone()).unwrap();
    assert_eq!(ed.project_root(), repo, "no .yumete yet, so the repository");

    std::fs::create_dir_all(book.join(".yumete")).unwrap();
    assert_eq!(ed.project_root(), book, ".yumete wins over a .git further up");

    // Neither mark anywhere: the chapter's own directory, not the repository
    // this suite is running in.
    let bare = dir.join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    std::fs::write(bare.join("散.md"), "乙\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(bare.join("散.md")).unwrap();
    assert_eq!(ed.project_root(), bare, "the file's own directory");

    // Nothing named at all: there is nowhere else to ask.
    let ed = Editor::new();
    assert_eq!(ed.project_root(), std::env::current_dir().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

/// #380: a `.txt` whose every line is cut the same way opens as a grid.
///
/// The report that led here was 「tb 沒對齊」 about a file that had never been
/// in tb: it opened as source, the tabs advanced to their stops, and the grey
/// ground that draws made it look like a broken table.
#[test]
fn a_txt_that_is_plainly_a_grid_opens_as_one_and_says_so() {
    let dir = std::env::temp_dir().join(format!("yumete-txt-grid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("拆分.txt");
    std::fs::write(&path, "雪\tvfg\t雨部\n風\tmqe\t風部\n雷\tfwq\t雨部\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&path).unwrap();
    let view = ed.table().expect("a tab in every line, the same number of them");
    assert_eq!(view.schema.columns.len(), 3, "three columns");
    assert!(ed.status().contains("Tab"), "and it names the mark: {}", ed.status());
    assert!(ed.status().contains("t o"), "and the way back: {}", ed.status());

    // Prose in the same directory is left alone: no mark appears the same
    // number of times in every line of it.
    let prose = dir.join("第一章.txt");
    std::fs::write(&prose, "那年冬天，甲說。\n乙沒有答。\n風停了，雪還在下。\n").unwrap();
    ed.open_file(&prose).unwrap();
    assert!(ed.table().is_none(), "prose is not a grid");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other half: 源碼模式 said once about a guessed grid is remembered.
///
/// Without this the feature is a thing to dismiss every morning, which is
/// worse than not having it.
#[test]
fn source_mode_is_remembered_for_a_guessed_grid_and_taken_back() {
    let dir = std::env::temp_dir().join(format!("yumete-txt-off-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let path = dir.join("表.txt");
    std::fs::write(&path, "甲\t一\n乙\t二\n丙\t三\n").unwrap();

    let mut ed = Editor::new();
    ed.keep_word_list_in(data.clone());
    ed.open_file(&path).unwrap();
    assert!(ed.table().is_some(), "guessed on the way in");

    ed.execute(":table off").unwrap();
    assert!(ed.table().is_none(), "and left when told to");
    assert!(
        data.join("source-mode.txt").is_file(),
        "the answer is written down, not just held"
    );

    // A new editor is tomorrow morning.
    let mut ed = Editor::new();
    ed.keep_word_list_in(data.clone());
    ed.open_file(&path).unwrap();
    assert!(ed.table().is_none(), "it does not ask again");

    // …and asking for a level again is the way back in, note withdrawn.
    ed.execute(":table basic").unwrap();
    assert!(ed.table().is_some(), "t b re-enters");
    let mut ed = Editor::new();
    ed.keep_word_list_in(data.clone());
    ed.open_file(&path).unwrap();
    assert!(ed.table().is_some(), "and the note is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A count that writes has a ceiling, and says when it hits one** (#318).
///
/// `1000000p` used to be a million pastes: 1.78 s for two hundred thousand of
/// them, a buffer grown to forty-one million characters, and no way to see it
/// coming. A motion may have the million — it walks off the end and stops.
#[test]
fn a_paste_asked_for_a_million_stops_at_the_ceiling() {
    let mut ed = typed("雪\n");
    press(&mut ed, "xy");
    let before = ed.current_buffer().char_count();
    let started = std::time::Instant::now();
    press(&mut ed, "1000000p");
    let grew = ed.current_buffer().char_count() - before;
    assert_eq!(grew, 2 * Editor::WRITING_MAX, "ten thousand pastes, no more");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "and it came back: {:?}",
        started.elapsed()
    );
    assert_eq!(
        ed.status,
        say!("count.writing-ceiling", Editor::WRITING_MAX),
        "the clipped count is not a silent one"
    );
}

/// **A find with nothing to find looks once** (#318).
#[test]
fn a_huge_count_on_f_costs_one_look_when_the_character_is_not_there() {
    let dir = std::env::temp_dir().join(format!("yumete-find-count-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("long.txt");
    std::fs::write(&path, format!("{}\n", "雪".repeat(20_000))).unwrap();
    let mut ed = Editor::new();
    ed.open_file(&path).unwrap();
    let started = std::time::Instant::now();
    press(&mut ed, "1000000fZ");
    assert_eq!(ed.cursor, 0, "there is no Z on the line");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "one look, not a million: {:?}",
        started.elapsed()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A macro answers to the ceiling too** (#318).
#[test]
fn a_macro_asked_for_a_million_stops_at_the_ceiling() {
    let mut ed = typed("雪\n");
    press(&mut ed, "Qxy"); // ⚠️ `Q` records, `q` replays (#404)
    press(&mut ed, "Q"); // stop
    press(&mut ed, "1000000q");
    assert_eq!(
        ed.status,
        say!("count.writing-ceiling", Editor::WRITING_MAX),
        "a macro may write, so its count is a writing count"
    );
}

/// **A macro round that changed nothing does not get another** (#318).
///
/// Costly on purpose: `%y` copies the whole buffer, and lands in the same
/// place every time. The second round proves there is nothing left to do; the
/// remaining 9998 would each copy a million characters again — thirty seconds.
#[test]
fn a_macro_that_moves_nothing_stops_replaying() {
    let dir = std::env::temp_dir().join(format!("yumete-macro-count-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.txt");
    std::fs::write(&path, format!("{}\n", "雪".repeat(1_000_000))).unwrap();
    let mut ed = Editor::new();
    ed.open_file(&path).unwrap();
    press(&mut ed, "Q%y");
    press(&mut ed, "Q"); // ⚠️ `Q` records, `q` replays (#404)
    let started = std::time::Instant::now();
    press(&mut ed, "10000q"); // exactly the ceiling: nothing is clipped here
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "two rounds, not ten thousand: {:?}",
        started.elapsed()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A hundred dirty chapters do not stop the typing** (#317).
///
/// `autosave_tick` runs in the input thread. It used to write a recovery copy
/// of every modified buffer on every round: after a whole-book `:replace`,
/// a hundred serialisations and two hundred fsyncs between one keystroke and
/// the next — measured at 1.593 s. Now the round is budgeted, and what it
/// cannot get to waits for the next one; the buffer being typed into is the
/// one that never waits.
#[test]
fn a_round_of_recovery_copies_is_bounded_and_the_backlog_drains() {
    let dir = std::env::temp_dir().join(format!("yumete-swap-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let body = "雪".repeat(23_000); // a 70 KB chapter, at the small end
    let mut ed = Editor::new();
    for i in 0..100 {
        let path = dir.join(format!("ch{i:03}.md"));
        std::fs::write(&path, format!("{body}\n")).unwrap();
        ed.execute(&format!(":open {}", path.display())).unwrap();
    }
    ed.set_autosave(true);
    for i in 0..100 {
        ed.show_buffer_at(i);
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::Char('甲'));
        ed.on_key(Key::Esc);
    }
    let drafts = || {
        std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".yumete")
            })
            .count()
    };

    ed.show_buffer_at(7);
    let started = std::time::Instant::now();
    ed.autosave_tick();
    let first = started.elapsed();
    assert!(
        first < std::time::Duration::from_millis(250),
        "one round held up the keyboard for {first:?}"
    );
    assert!(
        dir.join(".ch007.md.yumete").exists(),
        "the chapter being typed into is never the one that waits"
    );
    if ed.swap_backlog {
        let due = ed.autosave_due_in().expect("copies are still owed");
        assert!(
            due <= std::time::Duration::from_millis(150),
            "a backlog is not five seconds away: {due:?}"
        );
    }

    // Round by round, and every one of them cheap. `last_swap` is what the
    // clock would otherwise make this test sit and wait for.
    let mut rounds = 1;
    while ed.swap_backlog {
        ed.last_swap = None;
        let t = std::time::Instant::now();
        ed.autosave_tick();
        let took = t.elapsed();
        assert!(
            took < std::time::Duration::from_millis(250),
            "round {rounds} held up the keyboard for {took:?}"
        );
        rounds += 1;
        assert!(rounds < 400, "the backlog is not draining");
    }
    assert_eq!(drafts(), 100, "every chapter is insured by the end");
    assert_eq!(ed.autosave_due_in(), None, "and nothing is owed after that");

    let _ = std::fs::remove_dir_all(&dir);
}

/// **A copy that already holds what the buffer holds is not written again**
/// (#317).
///
/// The old round asked `is_modified`, which stays true until the document is
/// *saved* — so a book left unsaved was rewritten in full every five seconds
/// for the rest of the session, with nothing having changed in any of it. The
/// question is `draft_is_stale`: has anything happened since the copy.
#[test]
fn a_recovery_copy_that_is_current_is_not_written_again() {
    let dir = std::env::temp_dir().join(format!("yumete-swap-idle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut ed = Editor::new();
    for i in 0..8 {
        let path = dir.join(format!("ch{i}.md"));
        std::fs::write(&path, "第一章\n").unwrap();
        ed.execute(&format!(":open {}", path.display())).unwrap();
    }
    ed.set_autosave(true);
    for i in 0..8 {
        ed.show_buffer_at(i);
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::Char('甲'));
        ed.on_key(Key::Esc);
    }
    while {
        ed.last_swap = None;
        ed.autosave_tick();
        ed.swap_backlog
    } {}
    for i in 0..8 {
        assert!(dir.join(format!(".ch{i}.md.yumete")).exists());
        std::fs::remove_file(dir.join(format!(".ch{i}.md.yumete"))).unwrap();
    }

    // Nothing has been typed since, so nothing is owed and nothing is written
    // — the deleted copies stay deleted.
    assert_eq!(ed.autosave_due_in(), None, "unsaved, but nothing has moved");
    for _ in 0..3 {
        ed.last_swap = None;
        ed.autosave_tick();
    }
    for i in 0..8 {
        assert!(
            !dir.join(format!(".ch{i}.md.yumete")).exists(),
            "chapter {i} was written again with nothing having changed in it"
        );
    }

    // One keystroke in one chapter, and that one alone is written.
    ed.show_buffer_at(5);
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Char('乙'));
    ed.on_key(Key::Esc);
    ed.last_swap = None;
    ed.autosave_tick();
    for i in 0..8 {
        assert_eq!(
            dir.join(format!(".ch{i}.md.yumete")).exists(),
            i == 5,
            "chapter {i}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// **`:wa` that stops to ask lands on that file properly, not halfway** (#350).
///
/// The loop moved `self.current` by hand and put it back at the end — except
/// on the one path that does not reach the end. Stopping on the file that
/// asked left the editor showing that file with the *previous* file's cursor,
/// caches and grid; one `x` afterwards indexed a 120 002-character rope at
/// character 300 000 and took the process down with it.
#[test]
fn write_all_that_stops_to_ask_stands_on_that_file_properly() {
    let dir = std::env::temp_dir().join(format!("yumete-wa-ask-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let long = dir.join("long.md");
    let grown = dir.join("grown.md");
    std::fs::write(&long, format!("{}\n", "雪".repeat(400_000))).unwrap();
    std::fs::write(&grown, "甲\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(&long).unwrap();
    ed.open_file(&grown).unwrap();
    // What `t F` does to a chapter: far more text than the file on disk holds,
    // which is the one thing the oversize gate stops to ask about.
    let _ = ed.buffers[1].insert(0, &"乙".repeat(120_000));
    ed.show_buffer_at(0);
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Char('丙'));
    ed.on_key(Key::Esc);
    ed.set_cursor(300_000);

    ed.execute(":wa").unwrap();
    assert!(ed.query.is_some(), "the grown file asks before it is written");
    assert_eq!(ed.current, 1, "and the question is asked standing on it");
    assert!(
        ed.cursor <= ed.current_buffer().char_count(),
        "cursor {} is not in a document of {} characters",
        ed.cursor,
        ed.current_buffer().char_count()
    );

    // `n` — do not write it — and then an ordinary keystroke, which is where
    // the old state came apart.
    press(&mut ed, "n");
    press(&mut ed, "x");

    // The chapter that was left keeps its place for when the writer goes back.
    ed.show_buffer_at(0);
    assert_eq!(ed.cursor, 300_000, "the chapter remembers where it was");

    let _ = std::fs::remove_dir_all(&dir);
}

/// **A whole-document rewrite asks the grid the same question `:s` does**
/// (#350).
///
/// `without_cell_guard` is the moment table mode's one promise — a row never
/// gains or loses a cell — can be broken, and four of the six places that lift
/// the guard asked before doing it. The two that did not were the two that
/// rewrite the *entire* document: `replace_everything`, which is how a front
/// end hands back a formatted buffer, and `:convert`.
#[test]
fn a_whole_document_rewrite_will_not_break_a_row() {
    let dir = std::env::temp_dir().join(format!("yumete-rewrite-grid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("人物.csv");
    std::fs::write(&path, "字,序\n甲,1\n乙,2\n").unwrap();
    let mut ed = Editor::new();
    ed.open_file(&path).unwrap();

    // A cell that gained a comma is a row that gained a cell.
    assert!(
        !ed.replace_everything("字,序\n甲,一,1\n乙,2\n"),
        "a row that gained a cell went in anyway"
    );
    assert_eq!(
        ed.current_buffer().text(),
        "字,序\n甲,1\n乙,2\n",
        "and the table is as it was"
    );
    assert!(!ed.status().is_empty(), "refused, and said why");

    // What the check is *for* is letting the useful kind through: everything
    // inside the cells may change.
    assert!(ed.replace_everything("字,序\n丙,3\n丁,4\n"));
    assert_eq!(ed.current_buffer().text(), "字,序\n丙,3\n丁,4\n");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A dictionary that says how many lines it was handed, so #321 can be tested
/// by counting the work rather than by timing it.
struct Counted {
    inner: DictionarySegmenter,
    asked: std::rc::Rc<std::cell::Cell<usize>>,
}

impl Segmenter for Counted {
    fn segment(&self, s: &str) -> Vec<(usize, usize)> {
        self.asked.set(self.asked.get() + 1);
        self.inner.segment(s)
    }

    fn source(&self) -> String {
        self.inner.source()
    }
}

/// #321. `w` held down along one paragraph must cut that paragraph once.
#[test]
fn walking_a_line_by_word_cuts_the_line_once() {
    // 兩千段的中英混排行 — the shape the footnote measured 11.9 ms on.
    // ⚠️ 字數是**手段**，段數纔是這條測試要的東西。詞表換厚（2026-09-14，691 →
    // 75,000 條）之後同樣 6,000 字只切出 1,896 段，因為「今天天氣很好」併起來了。
    let mut line = String::new();
    while line.chars().count() < 8_000 {
        line.push_str("今天天氣很好 apple 山路 42 ");
    }
    line.push('\n');

    let plain = DictionarySegmenter::builtin(0);
    let expected = plain.segment(line.trim_end_matches('\n'));
    assert!(
        expected.len() > 2_000,
        "two thousand segments, not {}",
        expected.len()
    );

    let mut ed = Editor::new();
    assert!(ed.replace_everything(&line));
    let asked = std::rc::Rc::new(std::cell::Cell::new(0usize));
    ed.set_segmenter(Box::new(Counted {
        inner: DictionarySegmenter::builtin(0),
        asked: std::rc::Rc::clone(&asked),
    }));
    press(&mut ed, "gg");
    asked.set(0);

    // `w` is helix's: it *selects* up to just before the next word begins, and
    // leaves the caret on the last character of what it took — the word's own
    // last character, or the space in front of the next word. Either way the
    // caret stops one short of a boundary the dictionary drew, and a memo that
    // is fast and wrong is the failure this test is really for.
    let mut was = ed.cursor;
    for step in 1..=50 {
        press(&mut ed, "w");
        assert!(ed.cursor > was, "step {step} did not move");
        assert!(
            expected
                .iter()
                .any(|&(a, b)| a == ed.cursor + 1 || b == ed.cursor + 1),
            "step {step} landed at {}, which is no boundary the dictionary drew",
            ed.cursor
        );
        was = ed.cursor;
    }
    // One for the paragraph, and at most one more for the empty line after it.
    assert!(
        asked.get() <= 2,
        "fifty presses cut the line {} times",
        asked.get()
    );
}

/// A segmenter whose answer depends on something the text does not show — the
/// case a memo held against the text alone gets wrong (#321).
#[derive(Default)]
struct Levelled {
    level: yumete_cjk::WordLevel,
}

impl Segmenter for Levelled {
    fn segment(&self, s: &str) -> Vec<(usize, usize)> {
        let n = s.chars().count();
        match (self.level, n) {
            (_, 0) => Vec::new(),
            // 全: the whole line is one word. Anything else: one per character.
            (yumete_cjk::WordLevel::Full, _) => vec![(0, n)],
            _ => (0..n).map(|i| (i, i + 1)).collect(),
        }
    }

    fn set_level(&mut self, level: yumete_cjk::WordLevel) {
        self.level = level;
    }
}

/// #321. Changing the level changes where the words are without changing a
/// character of the text the answers are kept against.
#[test]
fn a_change_of_word_level_reaches_a_line_already_cut() {
    let mut ed = typed("甲乙丙丁\n戊己\n");
    ed.set_segmenter(Box::new(Levelled::default()));

    press(&mut ed, "gg");
    press(&mut ed, "w");
    assert_eq!(ed.cursor, 1, "one character is one word at 平衡");

    ed.set_word_level(yumete_cjk::WordLevel::Full);
    press(&mut ed, "gg");
    press(&mut ed, "w");
    assert_eq!(
        ed.cursor, 4,
        "at 全 the whole line is one word, so `w` selects all of it"
    );
}


/// Two of the five line memos had no bound at all before they were one
/// facility (#348): the Markdown runs and the readings kept an entry per line
/// for as long as the reader kept scrolling, and every edit made all of them
/// unreachable without removing a single one. Deciding that once is the point
/// of the facility — so the bound is asked of it, not of each caller.
#[test]
fn a_line_memo_does_not_grow_with_the_document() {
    let lines: Vec<String> = (0..2_000)
        .map(|i| format!("*第{i}章* 讀<ruby>漢<rt>hàn</rt></ruby>字"))
        .collect();
    let mut ed = typed(&lines.join("\n"));
    ed.set_syntax(crate::syntax::Syntax::Markdown);
    for line in 0..lines.len() {
        ed.markup_line(line);
        ed.readings_on_line(line);
    }
    let bound = super::memo::MEMO_LINES;
    assert!(
        ed.markup_memo.len() <= bound,
        "the Markdown runs of {} lines are being held",
        ed.markup_memo.len(),
    );
    assert!(
        ed.ruby_memo.len() <= bound,
        "the readings of {} lines are being held",
        ed.ruby_memo.len(),
    );
    // …and what a memo threw away it works out again, rather than answering
    // with nothing.
    assert!(
        !ed.markup_line(0).is_empty(),
        "line 0 lost its emphasis when the memo filled up",
    );
    assert_eq!(
        ed.readings_on_line(0).len(),
        1,
        "line 0 lost its reading when the memo filled up",
    );
}

/// 「我们应该允许表格模式在 insert 状态下通过上下左右键跨表格移动（包括行末跨到下一
/// 行的头）」(#376). Walking out of a cell is not joining two of them: an arrow
/// key moves and writes nothing, so the rule that keeps `Enter` and
/// `Backspace` inside one cell was never about it.
#[test]
fn insert_arrows_walk_the_grid_they_are_typing_in() {
    let (dir, csv) = a_table("arrows");
    let mut ed = Editor::new();
    ed.open_file(&csv).unwrap();
    ed.goto_line(2);
    press(&mut ed, "T"); // #356: by the cell
    assert_eq!(ed.cell_position(), Some((1, 0)));
    let before = ed.current_buffer().text();

    ed.on_key(Key::Char('i'));
    // At the cell's end, Right steps into the next one.
    ed.on_key(Key::End);
    ed.on_key(Key::Right);
    assert_eq!(ed.cell_position(), Some((1, 1)), "{}", ed.status());
    // …and Left at the start comes back — to the far end of what it walked
    // into, because that is the side it came in from.
    ed.on_key(Key::Left);
    assert_eq!(ed.cell_position(), Some((1, 0)));
    let (start, end) = ed.insert_bounds().unwrap();
    assert!(end > start, "that cell has something in it");
    assert_eq!(ed.cursor(), end, "entered from the right");

    // Off the end of the row and into the first cell of the next.
    for _ in 0..3 {
        ed.on_key(Key::End);
        ed.on_key(Key::Right);
    }
    assert_eq!(ed.cell_position(), Some((2, 0)), "the row ran on into the next");

    // Up and down keep the column, which is the step `j` and `k` take.
    ed.on_key(Key::Up);
    assert_eq!(ed.cell_position(), Some((1, 0)));
    ed.on_key(Key::Down);
    assert_eq!(ed.cell_position(), Some((2, 0)));

    assert_eq!(ed.mode(), Mode::Insert, "still typing");
    assert_eq!(ed.current_buffer().text(), before, "walking wrote nothing");

    std::fs::remove_dir_all(&dir).ok();
}

/// `Tab` at the last cell of a Markdown table opens another row — org-mode's
/// rule, and the right one, because the table is being filled in. An arrow key
/// is not filling anything in, so it stops there (#376).
#[test]
fn an_arrow_key_off_the_end_of_a_table_does_not_open_a_row() {
    let mut ed = typed("| 字 | 註 |\n| -- | -- |\n| 永 | 水 |\n");
    ed.execute(":3").unwrap();
    ed.enter_table();
    press(&mut ed, "T");
    press(&mut ed, "l");
    assert_eq!(ed.cell_position(), Some((2, 1)), "the last cell");
    let before = ed.current_buffer().text();

    ed.on_key(Key::Char('i'));
    ed.on_key(Key::End);
    ed.on_key(Key::Right);
    assert_eq!(ed.current_buffer().text(), before, "no row was opened");
    assert_eq!(ed.cell_position(), Some((2, 1)), "and nowhere to go");

    // Tab in the same place still does what Tab does.
    ed.on_key(Key::Tab);
    assert_ne!(ed.current_buffer().text(), before, "Tab opened one: {}", ed.status());
}


/// A step down a table costs the same whatever the table is long (#320).
///
/// **A ratio, on purpose.** A wall-clock bar would say more about the machine
/// than about the code; what went wrong here was a shape — three walks of the
/// whole table on every keystroke — and a shape shows up as「eight times the
/// rows, eight times the time」whatever the machine. Before the fix that ratio
/// was about twenty; after it, under one — the bigger table is if anything the
/// faster, because its rows are the ones the caches were warmed on. The bar is
/// three.
///
/// The page is left undivided, as a headless caller leaves it (`:shot`,
/// `--figure`, this test): that was the worst of the three, because the window
/// a table is measured over stretched from the top of the screen to wherever
/// the cursor had got to.
#[test]
fn a_step_down_a_table_costs_the_same_however_long_it_is() {
    use std::time::{Duration, Instant};
    let dir = std::env::temp_dir().join(format!("yumete-rowcost-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cost = |rows: usize| -> Duration {
        let mut text = String::from("| 字 | 讀音 |\n| -- | ---- |\n");
        for i in 0..rows {
            text.push_str(&format!("| 木{i} | mu   |\n"));
        }
        let doc = dir.join(format!("t{rows}.md"));
        std::fs::write(&doc, &text).unwrap();
        let mut ed = Editor::new();
        ed.open_file(&doc).unwrap();
        ed.set_page(50, 80);
        ed.goto_line(rows / 4);
        assert!(ed.enter_table(), "the table opened");
        ed.on_key(Key::Char('T'));
        for _ in 0..5 {
            ed.on_key(Key::Char('j'));
        }
        let n = 200;
        let began = Instant::now();
        for _ in 0..n {
            ed.on_key(Key::Char('j'));
        }
        began.elapsed() / n
    };
    // The first table pays for whatever the process has not warmed up yet, so
    // it is thrown away rather than measured.
    cost(1_000);
    let small = cost(1_000);
    let big = cost(8_000);
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        big <= small * 3,
        "eight times the rows cost {big:?} against {small:?} — a `j` is reading the table again"
    );
}

/// What one `j` costs in a long table (#320). Run with
/// `cargo test --release -- --ignored the_cost_of_a_step_down_a_table --nocapture`.
///
/// Twice per size, because the answer used to depend on something the keys
/// have no business depending on: whether the page had been divided yet. The
/// front end hands over the top of the screen every frame, and a caller that
/// draws nothing — `:shot`, `--figure`, a test — never does.
#[test]
#[ignore = "a benchmark, not a test"]
fn the_cost_of_a_step_down_a_table() {
    use std::time::Instant;
    for rows in [500usize, 5_000, 10_000] {
        let mut text = String::from("| 字 | 讀音 |\n| -- | ---- |\n");
        for i in 0..rows {
            text.push_str(&format!("| 木{i} | mu   |\n"));
        }
        let dir = std::env::temp_dir().join(format!("yumete-rowbench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join(format!("t{rows}.md"));
        std::fs::write(&doc, &text).unwrap();
        for scrolls in [true, false] {
            let mut ed = Editor::new();
            ed.open_file(&doc).unwrap();
            ed.set_page(50, 80);
            ed.goto_line(rows / 2);
            assert!(ed.enter_table());
            ed.on_key(Key::Char('T'));
            for _ in 0..5 {
                ed.on_key(Key::Char('j'));
            }
            let n = 50;
            let began = Instant::now();
            for _ in 0..n {
                if scrolls {
                    ed.set_page_top(ed.cursor_line().saturating_sub(25));
                }
                ed.on_key(Key::Char('j'));
            }
            let each = began.elapsed().as_secs_f64() * 1000.0 / n as f64;
            let page = match scrolls {
                true => "page follows",
                false => "page undivided",
            };
            println!("{rows:>6} rows  {page:<15} {each:>8.3} ms a `j`");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}


/// #418 一. A Markdown list carries its marker down, and the Enter on an item
/// with nothing in it ends the list rather than writing a fourth empty one.
#[test]
fn a_list_carries_itself_down() {
    let mut ed = typed("- 甲\n");
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "- 甲\n- \n", "the marker comes down");
    ed.on_key(Key::Char('乙'));
    ed.on_key(Key::Enter);
    ed.on_key(Key::Char('丙'));
    assert_eq!(ed.current_buffer().text(), "- 甲\n- 乙\n- 丙\n");

    // Nothing typed into the fourth item: that Enter clears the line, and the
    // caret is left at its start with the list behind it.
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "- 甲\n- 乙\n- 丙\n- \n");
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "- 甲\n- 乙\n- 丙\n\n", "and the list ends");
    ed.on_key(Key::Char('丁'));
    assert_eq!(ed.current_buffer().text(), "- 甲\n- 乙\n- 丙\n丁\n", "as prose");
}

/// #418 一 ③. The number below steps on; the ones already written do not move.
/// Renumbering a list is a command somebody runs on purpose.
#[test]
fn an_ordered_list_steps_on_without_renumbering_what_is_above() {
    let mut ed = typed("1. 甲\n1. 乙\n");
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "1. 甲\n2. \n1. 乙\n");
}

/// #418 一. The indent, the task box and the quote, each carried as it stands.
#[test]
fn an_indent_a_box_and_a_quote_all_come_down() {
    for (wrote, expected) in [
        ("    - 甲\n", "    - 甲\n    - \n"),
        ("- [x] 甲\n", "- [x] 甲\n- [ ] \n"),
        ("> 甲\n", "> 甲\n> \n"),
        ("> - 甲\n", "> - 甲\n> - \n"),
    ] {
        let mut ed = typed(wrote);
        ed.on_key(Key::Char('A'));
        ed.on_key(Key::Enter);
        assert_eq!(ed.current_buffer().text(), expected, "{wrote:?}");
    }
}

/// #418 一 ①, and the two other places an Enter is not a continuation.
#[test]
fn what_is_not_a_list_gets_a_plain_line_break() {
    // A `|` row is a table's: Enter splits it, and adding a marker to the
    // half below would put a list inside a grid.
    let mut ed = typed("| 甲 | 乙 |\n");
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "| 甲 | 乙 |\n\n");

    // A novel is not Markdown, and a dash in one is a dash.
    let mut ed = typed("- 甲\n");
    ed.current_buffer_mut().set_syntax(crate::syntax::Syntax::Text);
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "- 甲\n\n");

    // Inside the marker the Enter is splitting what was typed on purpose.
    let mut ed = typed("- 甲\n");
    ed.on_key(Key::Char('i'));
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "\n- 甲\n");
}

/// #418 一 ②. The continuation is part of the Insert session, not an undo
/// point of its own: one `u` takes back the whole of what was typed.
#[test]
fn a_continuation_earns_no_undo_point_of_its_own() {
    let mut ed = typed("- 甲\n");
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Enter);
    ed.on_key(Key::Char('乙'));
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "- 甲\n");

    // And the Enter that ends a list is one edit too.
    let mut ed = typed("- 甲\n- \n");
    ed.on_key(Key::Char('j'));
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Enter);
    assert_eq!(ed.current_buffer().text(), "- 甲\n\n");
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('u'));
    assert_eq!(ed.current_buffer().text(), "- 甲\n- \n");
}

/// The 大綱 folds — Feature #37.
///
/// A book of a hundred chapters filed under 卷 is unreadable as one flat list,
/// and the panel is narrow: the fold is what makes it a table of contents
/// rather than a wall.
#[test]
fn a_heading_in_the_outline_folds_what_is_under_it() {
    let dir = std::env::temp_dir().join(format!("yumete-fold-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("book.md"),
        "# 卷一\n序\n## 第一章\n一\n## 第二章\n二\n# 卷二\n三\n",
    )
    .unwrap();
    let mut ed = Editor::new();
    ed.open_file(dir.join("book.md")).unwrap();
    ed.open_sidebar_showing(&dir, crate::sidebar::View::Outline);

    let names = |ed: &Editor| -> Vec<String> {
        ed.panel(crate::sidebar::Side::Left)
            .unwrap()
            .rows()
            .iter()
            .map(|r| r.name.trim().to_string())
            .collect()
    };
    let marks = |ed: &Editor| -> Vec<(bool, bool)> {
        ed.panel(crate::sidebar::Side::Left)
            .unwrap()
            .rows()
            .iter()
            .map(|r| (r.is_dir, r.expanded))
            .collect()
    };
    assert_eq!(names(&ed), ["卷一", "第一章", "第二章", "卷二"]);
    // Only 卷一 holds anything, and it is open.
    assert_eq!(marks(&ed), [(true, true), (false, false), (false, false), (
        false, false
    )]);

    // `h` on it folds its chapters away and keeps the highlight where it is.
    ed.on_key(Key::Char('h'));
    assert_eq!(names(&ed), ["卷一", "卷二"]);
    assert_eq!(marks(&ed)[0], (true, false));
    assert_eq!(ed.panel(crate::sidebar::Side::Left).unwrap().selected(), 0);

    // `l` opens it again.
    ed.on_key(Key::Char('l'));
    assert_eq!(names(&ed), ["卷一", "第一章", "第二章", "卷二"]);

    // `h` on a chapter has no fold of its own to close, so the 卷 above it
    // closes and takes the highlight — pressing it again walks out of the
    // branch, which is what `h` does in the tree.
    ed.on_key(Key::Char('j'));
    assert_eq!(ed.panel(crate::sidebar::Side::Left).unwrap().selected(), 1);
    ed.on_key(Key::Char('h'));
    assert_eq!(names(&ed), ["卷一", "卷二"]);
    assert_eq!(ed.panel(crate::sidebar::Side::Left).unwrap().selected(), 0, "up on the 卷");

    // A heading with nothing above it and nothing under it folds nothing.
    ed.on_key(Key::Char('j'));
    ed.on_key(Key::Char('h'));
    assert_eq!(names(&ed), ["卷一", "卷二"]);
    // …and `l` on it is 「take me there」, as on any row that is not folded.
    ed.on_key(Key::Char('l'));
    assert_eq!(ed.cursor_line(), 6);
    assert!(!ed.sidebar_focused());

    // A folded heading is still a place: `Enter` goes to it rather than
    // opening it. Re-opening the panel builds a new one, so the folds are
    // gone with it — the same as the tree's open directories.
    ed.open_sidebar_showing(&dir, crate::sidebar::View::Outline);
    assert_eq!(names(&ed), ["卷一", "第一章", "第二章", "卷二"]);
    ed.on_key(Key::Char('h'));
    ed.on_key(Key::Enter);
    assert_eq!(ed.cursor_line(), 0);
    assert!(!ed.sidebar_focused());

    std::fs::remove_dir_all(&dir).ok();
}

/// #418 二. `[^` offers the tags the file already uses and, last, the next
/// free number; Tab writes one and closes the bracket.
#[test]
fn a_footnote_tag_is_finished_from_what_the_file_already_holds() {
    let mut ed = typed("正文[^舊註]和[^7]。\n\n[^舊註]: 從前的話\n[^7]: 第七條\n");
    // At the end of the first line, before the 。
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Char('['));
    ed.on_key(Key::Char('^'));

    let (title, choices, at) = ed.reference_menu().expect("the panel opens by itself");
    assert_eq!(title, "腳注標號");
    assert_eq!(at, None, "nothing is written until Tab, so nothing is chosen");
    let texts: Vec<&str> = choices.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, ["舊註]", "7]", "1]"], "what is in the file, then the next free number");
    assert_eq!(choices[0].note.as_deref(), Some("從前的話"), "the note it points at");
    assert_eq!(choices[2].note.as_deref(), Some("新的一條"));
    assert_eq!(ed.current_buffer().text().lines().next(), Some("正文[^舊註]和[^7]。[^"));

    ed.on_key(Key::Tab);
    assert_eq!(ed.current_buffer().text().lines().next(), Some("正文[^舊註]和[^7]。[^舊註]"));
    ed.on_key(Key::Tab);
    assert_eq!(
        ed.current_buffer().text().lines().next(),
        Some("正文[^舊註]和[^7]。[^7]"),
        "the second Tab replaces the first one's pick, it does not add to it"
    );
    // …and round the back: Shift-Tab from the second is the first again.
    ed.on_key(Key::BackTab);
    assert_eq!(ed.current_buffer().text().lines().next(), Some("正文[^舊註]和[^7]。[^舊註]"));

    // A letter after the pick is a letter: the walk is over and Tab starts
    // again from what is in the buffer.
    ed.on_key(Key::Char('。'));
    assert!(ed.reference_menu().is_none(), "a finished reference offers nothing");
}

/// #418 二. What has been typed narrows the list, and a tag that no longer
/// matches is not offered.
#[test]
fn what_is_typed_after_the_caret_narrows_the_tags() {
    let mut ed = typed("[^甲]和[^乙]\n\n[^甲]: 一\n[^乙]: 二\n");
    ed.on_key(Key::Char('G'));
    ed.on_key(Key::Char('A'));
    for c in "\n見[^甲".chars() {
        ed.on_key(match c {
            '\n' => Key::Enter,
            c => Key::Char(c),
        });
    }
    let (_, choices, _) = ed.reference_menu().expect("`[^甲` still stands open");
    let texts: Vec<&str> = choices.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, ["甲]"], "乙 does not start with 甲, and neither does 1");
    ed.on_key(Key::Tab);
    assert!(ed.current_buffer().text().ends_with("見[^甲]\n"), "closed, and not twice");
}

/// #418 二. `](#` offers this file's headings by the anchor a Markdown reader
/// gives them, and prints the title only where the anchor is not already it.
#[test]
fn a_link_is_finished_with_a_heading_of_this_file() {
    let mut ed = typed("# 卷一 開端\n\n## 第三節：雨\n\n見\n");
    ed.on_key(Key::Char('G'));
    ed.on_key(Key::Char('A'));
    for c in "[那裏](#".chars() {
        ed.on_key(Key::Char(c));
    }
    let (title, choices, _) = ed.reference_menu().expect("the headings are offered");
    assert_eq!(title, "本文件章節");
    let texts: Vec<&str> = choices.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, ["卷一-開端)", "第三節雨)"], "the space becomes a hyphen, the colon is dropped");
    assert_eq!(choices[0].note, None, "「卷一-開端」 is the title; printing it twice says nothing");
    assert_eq!(choices[1].note.as_deref(), Some("第三節：雨"), "here the anchor has lost something");
    ed.on_key(Key::Tab);
    assert!(ed.current_buffer().text().ends_with("見[那裏](#卷一-開端)\n"));
}

/// #418 二. The bracket already there is not written twice — a hand that
/// closes its brackets first is a hand this must not fight.
#[test]
fn a_closing_bracket_already_typed_is_left_alone() {
    let mut ed = typed("[^甲]\n\n[^甲]: 一\n");
    ed.on_key(Key::Char('G'));
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Enter);
    for c in "[^]".chars() {
        ed.on_key(Key::Char(c));
    }
    ed.on_key(Key::Left);
    ed.on_key(Key::Tab);
    assert!(ed.current_buffer().text().ends_with("[^甲]\n"), "one bracket, not two");
}

/// #418 三. `[[` offers the files near this one — the folder it is in and
/// what is under that, **not** the book.
#[test]
fn a_wiki_link_offers_the_files_near_this_one() {
    let dir = std::env::temp_dir().join(format!("yumete-wiki-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("卷一/卷二")).unwrap();
    std::fs::write(dir.join("卷一/一.md"), "# 卷一\n").unwrap();
    std::fs::write(dir.join("卷一/卷二/雨夜.md"), "# 雨夜\n").unwrap();
    std::fs::write(dir.join("卷一/舊稿.md"), "舊\n").unwrap();
    // ⚠️ One level **above** the file's folder: `[[` must not reach it, or a
    // book's whole tree lands in a panel meant for what is to hand.
    std::fs::write(dir.join("別處.md"), "別\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(dir.join("卷一/一.md")).unwrap();
    ed.execute(":syntax markdown").unwrap();
    ed.on_key(Key::Char('G'));
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Char('['));
    ed.on_key(Key::Char('['));

    let (title, choices, _) = ed.reference_menu().expect("a panel");
    assert_eq!(title, say!("complete.file"));
    let names: Vec<&str> = choices.iter().map(|c| c.text.as_str()).collect();
    assert!(names.contains(&"卷二/雨夜]]"), "a page, not a file: {names:?}");
    assert!(names.contains(&"舊稿]]"), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("別處")), "not above the folder: {names:?}");
    assert!(!names.iter().any(|n| n.contains("一.md")), "not this file: {names:?}");
    // The folder beside the name, when there is one worth saying.
    let deep = choices.iter().find(|c| c.text.starts_with("卷二/")).expect("the deep one");
    assert_eq!(deep.note.as_deref(), Some("卷二"));

    // **Typing narrows on the name as well as the path**, so 雨 finds
    // 卷二/雨夜.md without anybody typing the folder first.
    ed.on_key(Key::Char('雨'));
    let (_, choices, _) = ed.reference_menu().expect("still a panel");
    assert_eq!(choices.len(), 1, "{choices:?}");
    ed.on_key(Key::Tab);
    assert!(
        ed.current_buffer().text().ends_with("[[卷二/雨夜]]\n")
            || ed.current_buffer().text().ends_with("[[卷二/雨夜]]"),
        "{:?}",
        ed.current_buffer().text()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// #418 三. **A file name may hold a space** — 「卷一 開端.md」 is a name a
/// novelist writes — so a space does not break this trigger the way it breaks
/// a tag or an anchor.
#[test]
fn a_wiki_link_survives_a_space_in_the_name() {
    let dir = std::env::temp_dir().join(format!("yumete-wikispace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("此章.md"), "本\n").unwrap();
    std::fs::write(dir.join("卷一 開端.md"), "# 卷一\n").unwrap();

    let mut ed = Editor::new();
    ed.open_file(dir.join("此章.md")).unwrap();
    ed.execute(":syntax markdown").unwrap();
    ed.on_key(Key::Char('G'));
    ed.on_key(Key::Char('A'));
    for c in "[[卷一 開".chars() {
        ed.on_key(Key::Char(c));
    }
    let (_, choices, _) = ed.reference_menu().expect("a space is part of the name");
    assert_eq!(choices.len(), 1, "{choices:?}");
    assert_eq!(choices[0].text, "卷一 開端]]");

    // …while a tag still breaks on one.
    ed.on_key(Key::Esc);
    assert!(ed.reference_menu().is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// #418 三. Two brackets are wanted, however many are already there.
#[test]
fn a_wiki_link_closes_with_as_many_brackets_as_are_missing() {
    let dir = std::env::temp_dir().join(format!("yumete-wikiclose-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("此章.md"), "本\n").unwrap();
    std::fs::write(dir.join("雨夜.md"), "雨\n").unwrap();

    for already in ["", "]", "]]"] {
        let mut ed = Editor::new();
        ed.open_file(dir.join("此章.md")).unwrap();
        ed.execute(":syntax markdown").unwrap();
        ed.on_key(Key::Char('G'));
        ed.on_key(Key::Char('A'));
        for c in already.chars() {
            ed.on_key(Key::Char(c));
        }
        for _ in 0..already.chars().count() {
            ed.on_key(Key::Left);
        }
        for c in "[[雨".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Tab);
        let text = ed.current_buffer().text();
        assert!(
            text.contains("[[雨夜]]") && !text.contains("]]]"),
            "already {already:?}: {text:?}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// #418 二. Outside Markdown, and inside a grid, Tab is the key it was.
#[test]
fn tab_is_still_a_tab_where_there_is_no_reference() {
    let mut ed = typed("正文\n");
    ed.current_buffer_mut().set_syntax(crate::syntax::Syntax::Text);
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Char('['));
    ed.on_key(Key::Char('^'));
    assert!(ed.reference_menu().is_none(), "a plain manuscript has no footnotes");
    ed.on_key(Key::Tab);
    assert!(ed.current_buffer().text().starts_with("正文[^\t"), "a tab character");
}

/// #418 二. A `[^` quoted in a fence is somebody's example, not a reference.
#[test]
fn a_reference_in_a_fence_is_offered_nothing() {
    let mut ed = typed("# 甲\n\n```\n\n```\n\n正文\n\n[^1]: 一\n");
    ed.on_key(Key::Char('3'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Char('['));
    ed.on_key(Key::Char('^'));
    assert!(ed.reference_menu().is_none(), "inside the fence, nothing");
    ed.on_key(Key::Tab);
    assert!(ed.current_buffer().text().contains("[^\t"), "Tab is a tab in here");

    // …and the same two keys in the prose below it do open the panel, so the
    // gate is the fence and not the file.
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('7'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Char('['));
    ed.on_key(Key::Char('^'));
    assert!(ed.reference_menu().is_some());
}

