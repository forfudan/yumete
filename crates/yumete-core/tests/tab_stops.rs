//! A TAB advances to the next stop, and is drawn rather than measured (#374).
//!
//! **The tab keeps its own cell; the drawing makes up the rest.** A tab is one
//! cell to every width this editor asks for, so what is drawn beside it is
//! `room - 1` — the whole advance, less the cell the character already has.
//! That is what lets a table *replace* a separator with a one-cell glyph and
//! still line its columns up.

use yumete_core::{drawn::Ink, editor::Editor, Key};

fn typed(text: &str) -> Editor {
    let mut ed = Editor::new();
    ed.on_key(Key::Char('i'));
    for c in text.chars() {
        ed.on_key(if c == '\t' { Key::Char('\t') } else { Key::Char(c) });
    }
    ed.on_key(Key::Esc);
    ed
}

#[test]
fn a_tab_advances_to_the_next_stop() {
    // A 碼表 is the case: `ch` is two cells and `bkd` is three, and what makes
    // the file readable is that both put their character in the same column.
    let ed = typed("ch\t錐");
    let runs = ed.drawn_runs_on_line(0);
    let tab: Vec<&yumete_core::drawn::Run> =
        runs.iter().filter(|r| r.ink == Ink::Tab).collect();
    assert_eq!(tab.len(), 1, "{runs:?}");
    assert_eq!(tab[0].column, 2, "it stands where the tab is");
    assert_eq!(tab[0].text, "     ", "five drawn, and the tab's own cell is the sixth");

    let ed = typed("bkd\t蜘");
    let runs = ed.drawn_runs_on_line(0);
    let tab: Vec<&yumete_core::drawn::Run> =
        runs.iter().filter(|r| r.ink == Ink::Tab).collect();
    assert_eq!(tab[0].text, "    ", "one fewer drawn, and the same column reached");
}

#[test]
fn a_tab_at_a_stop_advances_a_whole_one() {
    // Never nothing: a tab always moves.
    let ed = typed("fvtf\t裘");
    let runs = ed.drawn_runs_on_line(0);
    let tab: Vec<&yumete_core::drawn::Run> =
        runs.iter().filter(|r| r.ink == Ink::Tab).collect();
    assert_eq!(tab[0].text.chars().count(), 3, "three drawn plus its own: on to eight");
}

#[test]
fn a_wide_character_is_counted_in_cells() {
    // 漢字 are two cells each, so a tab after two of them is already at four.
    let ed = typed("漢字\t甲");
    let runs = ed.drawn_runs_on_line(0);
    let tab: Vec<&yumete_core::drawn::Run> =
        runs.iter().filter(|r| r.ink == Ink::Tab).collect();
    assert_eq!(tab[0].text.chars().count(), 3, "four cells used, four to go, three of them drawn");
}

#[test]
fn a_line_without_a_tab_draws_nothing_for_one() {
    let ed = typed("ch錐");
    assert!(ed.drawn_runs_on_line(0).iter().all(|r| r.ink != Ink::Tab));
}

#[test]
fn the_caret_stands_where_the_tab_put_it() {
    // The invariant #212 exists for: the caret may never sit in a column the
    // page does not have. A tab drawn as nothing broke it — the readout said
    // one column and the screen had another.
    let mut ed = typed("ch\t錐");
    // `ch` is two cells, the tab carries to eight, 錐 is two more.
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('l'));
    ed.on_key(Key::Char('l'));
    // Now on the tab itself.
    assert_eq!(ed.cursor_visual_column(), 2, "the tab begins at two");
    ed.on_key(Key::Char('l'));
    assert_eq!(ed.cursor_visual_column(), 8, "and 錐 begins at the stop");
}

/// The author, 2026-09-11: 「tf状态下，tab 分隔符会被表格虚线接管，不需要绘
/// 制…tb状态下，tab分隔符会被表格的对齐代替，这时候他也是个普通的符号，显示
/// 1格宽都行（和markdown 中的pipe一样）」.
///
/// A separator is not indentation. Advanced to its stop it would push every
/// column right of it out of line with the rows above — and in 全 the grid
/// draws a `┆` *over* that one character, which a tab three cells wide has
/// nowhere to put.
#[test]
fn a_tab_a_table_has_taken_over_is_a_separator_and_not_a_stop() {
    let mut ed = typed("ch\t錐\nbkd\t蜘\n");
    let tabs = |ed: &Editor| -> usize {
        ed.drawn_runs_on_line(0).iter().filter(|r| r.ink == Ink::Tab).count()
    };
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));

    // 源碼: no table has taken anything over, so a tab is a tab.
    for c in "to".chars() {
        ed.on_key(Key::Char(c));
    }
    assert_eq!(tabs(&ed), 1, "源碼 draws the file as it is written: {}", ed.status());

    // 基本 and 全 both read it as a table, and in both the separator is the
    // table's punctuation — one cell, like a pipe.
    for level in ["tb", "tf"] {
        for c in level.chars() {
            ed.on_key(Key::Char(c));
        }
        assert_eq!(tabs(&ed), 0, "{level} takes the separator over: {}", ed.status());
    }

    // And 全 draws its rule on that very character, which is only possible
    // because the character still has a cell of its own.
    let walls: Vec<usize> = ed.grid_on_line(0).into_iter().map(|(at, _)| at).collect();
    assert_eq!(walls, vec![2], "the rule stands on the tab: {}", ed.status());
}

/// The same rule with the same code path, in the punctuation everyone else
/// writes tables in — the separator is a value the view carries, never a
/// branch per file type.
#[test]
fn a_pipe_and_a_comma_are_walls_by_the_very_same_door() {
    let mut ed = typed("a,bb,c\nlonger,b,cc\n");
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('g'));
    for c in "tf".chars() {
        ed.on_key(Key::Char(c));
    }
    let walls: Vec<usize> = ed.grid_on_line(0).into_iter().map(|(at, _)| at).collect();
    assert_eq!(walls, vec![1, 4], "both commas: {}", ed.status());
}
