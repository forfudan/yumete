//! **The candidate list keeps its columns with ambiguous characters drawn
//! wide.**
//!
//! A CJK terminal draws `…` and `○` two cells wide; ratatui lays them out in
//! one. The panel is measured by the editor's width and every such glyph is
//! given its second cell (`yumete_cjk::needs_a_second_cell`), so the glyph
//! paints over a blank of its own and not over the candidate beside it.
//!
//! The ring around the box is East Asian Ambiguous too: `╭` `─` `│` are two
//! cells in the same terminal, so each stroke is given its second cell the
//! same way and the box stays a rectangle (`chrome::draw_in_editor_cells`).
//!
//! Its own binary because `set_ambiguous_wide` is process-wide.

use yumete_config::{Config, PanelDisplay};
use yumete_core::editor::Editor;
use yumete_core::input::Key;
use yumete_core::zong::Layout;
use yumete_ime::{ImeSession, Scheme};

/// Every `…` and `○` in `text` is followed by the blank it paints over. (The
/// ring's box-drawing strokes are the frame's, not the candidates'.)
fn each_wide_glyph_has_its_blank(text: &str) -> Result<(), String> {
    for row in text.lines() {
        let chars: Vec<char> = row.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if matches!(c, '…' | '○') && chars.get(i + 1) != Some(&' ') {
                return Err(format!("{c:?} has no second cell in {row:?}"));
            }
        }
    }
    Ok(())
}

/// A row of the frame as `(cell, character)`: where ratatui put each glyph.
fn cells(row: &str) -> Vec<(usize, char)> {
    let mut at = 0;
    row.chars()
        .map(|c| {
            let here = at;
            at += yumete_cjk::drawn_width(&c.to_string()).max(1);
            (here, c)
        })
        .collect()
}

fn is_stroke(c: char) -> bool {
    ('\u{2500}'..='\u{257f}').contains(&c)
}

/// **The ring is a rectangle as a CJK terminal draws it**: each glyph paints
/// the cells this editor counts for it (`char_width`), so no glyph may paint
/// over another, and every row of the ring starts and ends on the same cells.
fn the_ring_is_a_rectangle(text: &str) -> Result<(), String> {
    let rows: Vec<&str> = text.lines().collect();
    let top = rows.iter().position(|r| r.contains(['╭', '┌'])).ok_or("no ring")?;
    let bottom = rows.iter().position(|r| r.contains(['╰', '└'])).ok_or("no bottom")?;
    let mut edges = None;
    for row in &rows[top..=bottom] {
        let cells = cells(row);
        for (i, &(at, c)) in cells.iter().enumerate() {
            let ends = at + yumete_cjk::char_width(c);
            if let Some(&(next, d)) = cells[i + 1..].iter().find(|&&(n, d)| n < ends && d != ' ') {
                if c != ' ' {
                    return Err(format!("{c:?} at {at} paints over {d:?} at {next} in {row:?}"));
                }
            }
        }
        let strokes: Vec<(usize, char)> = cells.into_iter().filter(|&(_, c)| is_stroke(c)).collect();
        let (&(from, _), &(last, c)) = strokes.first().zip(strokes.last()).ok_or(format!("no wall in {row:?}"))?;
        let here = (from, last + yumete_cjk::char_width(c));
        match edges {
            None => edges = Some(here),
            Some(first) if first != here => return Err(format!("{row:?} spans {here:?}, the top {first:?}")),
            _ => {}
        }
    }
    Ok(())
}

#[test]
fn ambiguous_candidates_keep_the_panel_s_columns() {
    yumete_cjk::set_ambiguous_wide(true);
    assert!(yumete_cjk::needs_a_second_cell('…') && yumete_cjk::needs_a_second_cell('○'));
    let shapes = [
        (PanelDisplay::Box, Layout::Horizontal),
        (PanelDisplay::Bottom, Layout::Horizontal),
        (PanelDisplay::Inline, Layout::Horizontal),
        (PanelDisplay::Box, Layout::Vertical),
    ];
    for (display, layout) in shapes {
        let mut editor = Editor::new();
        editor.set_layout(layout);
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "a 字 …… ○○\nab ○乙\n");
        ime.set_panel_display(display);
        ime.input('a');
        let text = yumete_tui::frame_to_text(&mut editor, &Config::default(), &ime, 40, 14, None);
        let shape = format!("{display:?} {layout:?}");
        each_wide_glyph_has_its_blank(&text).unwrap_or_else(|e| panic!("{shape}: {e}\n{text}"));
        if display == PanelDisplay::Box {
            the_ring_is_a_rectangle(&text).unwrap_or_else(|e| panic!("{shape}: {e}\n{text}"));
            for rounded in [false, true] {
                let mut config = Config::default();
                config.panel.rounded = rounded;
                let text = yumete_tui::frame_to_text(&mut editor, &config, &ime, 41, 14, None);
                the_ring_is_a_rectangle(&text).unwrap_or_else(|e| panic!("{shape} rounded {rounded}: {e}\n{text}"));
            }
        }
        if layout == Layout::Horizontal {
            // Measured narrow, a row would be cut short of its last glyph.
            for whole in ["… …", "○ ○"] {
                assert!(text.contains(whole), "{shape}: {whole:?} is not whole\n{text}");
            }
        }
        if display == PanelDisplay::Box && layout == Layout::Horizontal {
            // The keys still owed close the widest row: a panel measured
            // narrow would cut them off.
            assert!(text.contains("○ 乙 b"), "the keys owed after an ambiguous candidate\n{text}");
        }
    }
}
