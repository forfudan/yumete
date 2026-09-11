//! A separator is a separator (#378).
//!
//! The author, 2026-09-11：「markdown 中的表格使用 | 分隔，tsv 用 tab，csv 用
//! 逗号。他们本质上都是分隔符。所以 tb / tf 模式下他们显示效果应该是一样的。」
//!
//! So these tests are written the same way for all of them: one table, said in
//! four punctuations, and the assertion is that the page cannot tell which one
//! it was given.

use yumete_core::{editor::Editor, Key};

fn typed(text: &str) -> Editor {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    for c in text.chars() {
        ed.on_key(if c == '\n' { Key::Enter } else { Key::Char(c) });
    }
    ed.on_key(Key::Esc);
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed
}

fn press(ed: &mut Editor, keys: &str) {
    for c in keys.chars() {
        ed.on_key(Key::Char(c));
    }
}

/// The screen column each row's **last wall** stands in.
///
/// Punctuation-independent on purpose: a `|` row's last wall is the pipe that
/// closes it and a CSV row's is the comma before its final cell, and「the
/// table lines up」is the same sentence about both — the walls of a column
/// stand over one another. Measured the way the page measures: the text by
/// grapheme, plus whatever is drawn before each character.
fn last_wall_column(ed: &Editor, rows: usize) -> Vec<usize> {
    (0..rows)
        .map(|line| {
            let rope = ed.current_buffer().rope();
            let text: String = rope.line(line).chars().collect();
            let drawn = ed.drawn_on_line(line);
            let walls = ed.wall_columns(line);
            let Some(&wall) = walls.last() else {
                panic!("row {line} has a wall: {walls:?}");
            };
            let mut at = 0usize;
            let mut column = 0usize;
            for g in yumete_cjk::graphemes(text.trim_end_matches(['\n', '\r'])) {
                for (_, run) in drawn.iter().filter(|&&(a, _)| a == at) {
                    column += yumete_cjk::str_width(run);
                }
                if at == wall {
                    return column;
                }
                column += yumete_cjk::grapheme_width(g);
                at += g.chars().count();
            }
            column
        })
        .collect()
}

/// The same table in four punctuations lines its second column up in the same
/// place, whichever one it was written in.
#[test]
fn four_punctuations_one_table() {
    // A space is not sniffed as a separator and must not be — prose would
    // become a table. `Wall::Between(' ')` costs nothing and is ready for the
    // day a view says so; see `a_space_is_a_wall_like_any_other`.
    let said = [
        ("csv", "ch,錐\nlongcode,蜘\nbk,裘\n"),
        ("tsv", "ch\t錐\nlongcode\t蜘\nbk\t裘\n"),
        ("scsv", "ch;錐\nlongcode;蜘\nbk;裘\n"),
    ];
    for (name, text) in said {
        let mut ed = typed(text);
        press(&mut ed, "tf");
        let at = last_wall_column(&ed, 3);
        assert_eq!(at[0], at[1], "{name}: {at:?}");
        assert_eq!(at[1], at[2], "{name}: {at:?}");
    }
}

/// 基本 squares a delimited table up exactly as it squares a `|` one up —
/// which is what 「tb / tf 模式下他们显示效果应该是一样的」 asks for, and what
/// `to` deliberately does not do.
#[test]
fn the_levels_mean_the_same_thing_whatever_the_punctuation() {
    for text in ["ch,錐\nlongcode,蜘\nbk,裘\n", "|ch|錐|\n|longcode|蜘|\n|bk|裘|\n"] {
        let mut ed = typed(text);

        press(&mut ed, "to");
        assert!(
            (0..3).all(|l| ed.drawn_on_line(l).is_empty()),
            "源碼 draws the file as it is written: {:?}",
            (0..3).map(|l| ed.drawn_on_line(l)).collect::<Vec<_>>()
        );

        for level in ["tb", "tf"] {
            press(&mut ed, level);
            let at = last_wall_column(&ed, 3);
            assert_eq!(at[0], at[1], "{level} squares it up: {at:?}");
            assert_eq!(at[1], at[2], "{level} squares it up: {at:?}");
        }
    }
}

/// **A comma is not cushioned.** One space off each wall is a Markdown
/// convention (`|a|b|` is the same table as `| a | b |`), not a fact about
/// separators — a CSV drawn as `a , b` is a CSV nobody writes.
#[test]
fn only_the_pipe_is_written_with_a_space_off_it() {
    let mut ed = typed("ab,cd\nef,gh\n");
    press(&mut ed, "tf");
    assert!(
        ed.drawn_on_line(0).is_empty(),
        "every cell is already the column's width: {:?}",
        ed.drawn_on_line(0)
    );

    let mut ed = typed("|ab|cd|\n|ef|gh|\n");
    press(&mut ed, "tf");
    assert!(
        !ed.drawn_on_line(0).is_empty(),
        "and a pipe table still gets its spaces"
    );
}

/// A column is measured over **the rows on the page** — because a delimited
/// file's table *is* the file: the 碼表 this editor exists for is 124,083 rows,
/// and measuring them all took 160 ms on every keystroke.
///
/// The window was one page either side of the row being asked about until the
/// author asked why, 2026-09-11：「可以都只量屏幕上的吗？」It can, and it
/// should: the wider window let a cell a page below the screen widen every
/// column in view with nothing visible to say why.
#[test]
fn a_table_is_measured_over_what_is_on_the_page() {
    use yumete_core::mdtable::measured_window;
    // A table shorter than the page is measured whole.
    assert_eq!(measured_window(10, 40, 20, 0, 60), (10, 40));
    // A file-sized one is measured over the page and no more.
    assert_eq!(measured_window(0, 124_082, 60_000, 59_990, 40), (59_990, 60_030));
    // …and never past the table it is measuring.
    assert_eq!(measured_window(0, 124_082, 10, 0, 40), (0, 40));
    assert_eq!(measured_window(0, 100, 90, 80, 40), (80, 100));
    // A row asked about from off the page is taken in rather than refused, so
    // a `:shot` or a caret readout after a jump still gets an answer.
    let (first, last) = measured_window(0, 124_082, 5, 60_000, 40);
    assert!(first <= 5 && 5 <= last, "{first}..={last}");
    // The row asked about is always in the window, which is what makes the
    // answer an answer at all.
    for (at, top) in [(0, 0), (1, 0), (500, 480), (124_082, 124_050)] {
        let (first, last) = measured_window(0, 124_082, at, top, 40);
        assert!(first <= at && at <= last, "{at} from {top}: {first}..={last}");
    }
}

/// **`hjkl` are letters at every level**, and `T` is what hands them to the
/// cells (#356).
///
/// Written down as a test because the doc comment that said otherwise stood
/// for months after #356 made it false, and on 2026-09-11 it was read as law:
/// an argument against recognising a delimited file on open was built on「自动
/// 进 tb 会悄悄改掉 hjkl 的含义」, which the author answered with 「tb tf to
/// 模式都是按字走的不是按格走的」. A comment cannot fail; this can.
#[test]
fn the_levels_leave_hjkl_alone() {
    for level in ["to", "tb", "tf", "tt"] {
        let mut ed = typed("ch\t錐\nbkd\t蜘\n");
        press(&mut ed, level);
        let mut walk = vec![ed.cursor()];
        for _ in 0..3 {
            ed.on_key(Key::Char('l'));
            walk.push(ed.cursor());
        }
        assert_eq!(walk, vec![0, 1, 2, 3], "{level} walks characters");
    }
}

/// A step the reader cannot see is not a step (#379).
///
/// Under `t f` the file's own padding comes off the page, so `l` through a
/// cell's trailing spaces moved the caret and moved nothing on the screen —
/// the author, 2026-09-11：「如果我一直按 l，光标是定住不动的，然后突然跳到右
/// 边一格……既然没有显示，就应该允许用户直接跳过去。」
#[test]
fn the_caret_steps_over_what_a_table_keeps_off_the_page() {
    let long = "甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉戌亥天地玄黃宇宙洪荒日月盈昃";
    let text = format!(
        "| 名 | 傳 |\n| --- | --- |\n| 洛陽 | {long} |\n| 甲 | 乙                                    |\n"
    );
    let mut ed = typed(&text);
    ed.execute(":syntax markdown").unwrap();
    ed.execute("4").unwrap();
    press(&mut ed, "tf");
    let hidden = ed.cell_hidden_on_line(3);
    assert!(!hidden.is_empty(), "the file's padding is off the page: {hidden:?}");
    let (from, upto) = hidden[0];

    // Walk to the last character before the hidden run…
    ed.execute("4").unwrap();
    let start = ed.cursor();
    for _ in 0..from - 1 {
        ed.on_key(Key::Char('l'));
    }
    assert_eq!(ed.cursor() - start, from - 1, "at the last visible character");
    let before = ed.cursor_visual_column();

    // …and one more press clears the whole run rather than walking it blind.
    ed.on_key(Key::Char('l'));
    assert_eq!(ed.cursor() - start, upto, "one press, the whole run: {hidden:?}");
    assert!(
        ed.cursor_visual_column() > before,
        "and the page moved with it: {before} -> {}",
        ed.cursor_visual_column()
    );

    // Back the same way.
    ed.on_key(Key::Char('h'));
    assert_eq!(ed.cursor() - start, from - 1, "and back over it in one");
}

/// Markup is the other half of what is hidden and it is **not** skipped: it
/// comes back the moment the caret is in it, which is what 所見即所得 means.
#[test]
fn the_caret_still_walks_into_markup_that_opens_for_it() {
    let mut ed = typed("前文 **重點** 後文\n");
    ed.execute(":syntax markdown").unwrap();
    ed.execute(":render full").unwrap();
    ed.execute("1").unwrap();
    let start = ed.cursor();
    for _ in 0..4 {
        ed.on_key(Key::Char('l'));
    }
    assert_eq!(ed.cursor() - start, 4, "one press, one character, into the `**`");
}
