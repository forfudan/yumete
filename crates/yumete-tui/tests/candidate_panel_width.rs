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
//! same way and the box stays a rectangle (`chrome::draw`).
//!
//! Its own binary because `set_ambiguous_wide` is process-wide.

mod common;

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
            common::rings_are_rectangles(&text).unwrap_or_else(|e| panic!("{shape}: {e}\n{text}"));
            for rounded in [false, true] {
                let mut config = Config::default();
                config.panel.rounded = rounded;
                let text = yumete_tui::frame_to_text(&mut editor, &config, &ime, 41, 14, None);
                common::rings_are_rectangles(&text).unwrap_or_else(|e| panic!("{shape} rounded {rounded}: {e}\n{text}"));
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
