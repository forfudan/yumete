//! **The completion panel**: words, paths, a server's list and references
//! (`yumete_core::editor::completion`), and the programs and files a shell
//! line offers.
//!
//! No ring: the panel stands on the status bar's own ground, which is ground
//! enough to tell it from the page, and a ring around a list that changes
//! with every letter is the loudest thing on the screen. The rows are a name
//! in the text's ink and, on the right, a quiet word about it; the gold `›`
//! is the one written into the text. Where the keys are and how far down the
//! list the choice is goes on the command row (`completion_hint`).

use crate::chrome::{Anchor, Spot};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::Frame;
use yumete_config::Config;
use yumete_core::editor::completion::{CompletionPanel, CompletionRow};

/// The cells in front of a name: the `›` and a space.
const MARK: u16 = 2;
/// Between the name and the note on its right.
const GAP: usize = 2;

/// The panel at the caret.
pub(crate) fn draw(
    frame: &mut Frame,
    config: &Config,
    area: Rect,
    bottom: u16,
    caret: Spot,
    vertical: bool,
    panel: &CompletionPanel,
) -> Option<Rect> {
    // **The names stand under what they complete**, as helix's do: the
    // panel steps back by what has been typed (and by the `›` in front).
    // On a vertical page the text runs down and the panel takes a corner, so
    // there is no column to line up with.
    let x = match vertical {
        false => caret.at.0.saturating_sub(panel.lead as u16).saturating_sub(MARK).max(area.x),
        true => caret.at.0,
    };
    let spot = Spot { at: (x, caret.at.1), keep: caret.keep };
    draw_rows(frame, config, area, bottom, spot, vertical, &panel.rows, panel.chosen)
}

/// Draw `rows` with their left edge at `spot.at.0`, under the row `spot.at.1`
/// or above it when there is no room under it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_rows(
    frame: &mut Frame,
    config: &Config,
    area: Rect,
    bottom: u16,
    spot: Spot,
    vertical: bool,
    rows: &[CompletionRow],
    chosen: Option<usize>,
) -> Option<Rect> {
    let ink = crate::theme::Palette::of(config);
    draw_rows_in(frame, config, ink, area, (bottom, spot, vertical), rows, chosen)
}

/// [`draw_rows`] in a palette already chosen.
pub(crate) fn draw_rows_in(
    frame: &mut Frame,
    config: &Config,
    ink: crate::theme::Palette,
    area: Rect,
    (bottom, spot, vertical): (u16, Spot, bool),
    rows: &[CompletionRow],
    chosen: Option<usize>,
) -> Option<Rect> {
    if rows.is_empty() || area.height == 0 {
        return None;
    }
    let all = rows.len();
    let shown = config.editor.completion_page_size.max(1).min(all).min(area.height as usize);
    let focus = chosen.unwrap_or(0).min(all - 1);
    // The chosen row a little above the middle; at the ends the window keeps
    // to them and leaves no blank row.
    let from = match all <= shown {
        true => 0,
        false => focus.saturating_sub(shown / 2).min(all - shown),
    };
    let scrolled = all > shown;
    // Measured on the whole list, not the window, so the panel keeps its
    // width while the window moves down it.
    let wide = |t: &str| yumete_cjk::str_width(t);
    let mut name_w = rows.iter().map(|r| wide(&r.label)).max().unwrap_or(0).max(1);
    let mut note_w = rows.iter().filter_map(|r| r.note.as_deref()).map(wide).max().unwrap_or(0);
    let bar = usize::from(scrolled);
    // `›`, the name, the gap and the note, a cell of air, the scrollbar.
    let frame_w = MARK as usize + 1 + bar;
    let budget = (area.width as usize).saturating_sub(frame_w);
    let gap = if note_w > 0 { GAP } else { 0 };
    if name_w + gap + note_w > budget {
        // The name is what is chosen by; the note gives way first, to a
        // third of what there is.
        note_w = note_w.min(budget / 3);
        name_w = budget.saturating_sub(gap + note_w).max(1);
    }
    let width = (frame_w + name_w + gap + note_w).clamp(8, area.width as usize) as u16;
    let height = shown as u16;
    let anchor = match vertical {
        false => Anchor::UnderCaret { at: spot.at, bottom },
        true => Anchor::Caret { at: spot.at, keep: spot.keep, bottom, vertical },
    };
    let rect = crate::chrome::place(area, (width, height), anchor)?;
    let ground = crate::status_bar_style_in(config, ink).bg.unwrap_or_else(|| ink.sunken());
    let base = Style::default().bg(ground).fg(ink.text());
    let buf = frame.buffer_mut();
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
                cell.set_symbol(" ").set_style(base);
            }
        }
    }
    let right = rect.x + rect.width;
    let note_end = right - 1 - bar as u16;
    // helix's thumb: as tall as the window is of the list, at least a row.
    let thumb = (shown * shown).div_ceil(all).clamp(1, shown);
    let thumb_at = match all - shown {
        0 => 0,
        room => (from * (shown - thumb) + room / 2) / room,
    };
    for (n, row) in rows[from..from + shown].iter().enumerate() {
        let y = rect.y + n as u16;
        if chosen == Some(from + n) {
            crate::put_text(buf, rect.x, y, right, "›", base.fg(ink.gold()));
        }
        let mut name = base;
        if row.folder {
            name = name.fg(ink.azure());
        }
        if row.deprecated {
            name = name.add_modifier(Modifier::CROSSED_OUT);
        }
        let label = crate::panel::clip(&row.label, name_w);
        crate::put_text(buf, rect.x + MARK, y, right, &label, name);
        if let Some(note) = row.note.as_deref().filter(|_| note_w > 0) {
            let note = crate::panel::clip(note, note_w);
            let at = note_end.saturating_sub(wide(&note) as u16);
            crate::put_text(buf, at, y, note_end, &note, base.fg(ink.quiet()));
        }
        if scrolled {
            let (symbol, fg) = match (thumb_at..thumb_at + thumb).contains(&n) {
                true => ("█", ink.quiet()),
                false => ("│", ink.rule()),
            };
            crate::put_text(buf, right - 1, y, right, symbol, base.fg(fg));
        }
    }
    Some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;
    use ratatui::Terminal;

    const THEMES: &[&str] =
        &["ink", "bw", "morandi", "one-dark", "github", "monokai", "dracula", "tokyo-night", "catppuccin", "gruvbox"];

    fn config_for(theme: &str) -> Config {
        Config { theme: yumete_config::ThemeConfig::named(theme).expect(theme), ..Config::default() }
    }

    fn row(label: &str, note: Option<&str>) -> CompletionRow {
        CompletionRow { label: label.into(), note: note.map(str::to_string), folder: false, deprecated: false }
    }

    /// Draw `rows` in a 40×14 page, the caret at `caret`, in `theme`'s mood.
    fn drawn(theme: &str, dark: bool, rows: &[CompletionRow], chosen: Option<usize>, caret: (u16, u16)) -> (Rect, Buffer) {
        let config = config_for(theme);
        let ink = crate::theme::Palette::in_mood(&config, dark);
        let mut terminal = Terminal::new(TestBackend::new(40, 14)).unwrap();
        let mut rect = None;
        // The frame's own cells: a terminal is never sent the second cell of a
        // wide character, so the backend's copy cannot say what it was painted.
        let mut cells = Buffer::empty(Rect::new(0, 0, 40, 14));
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 13);
                rect = draw_rows_in(frame, &config, ink, area, (13, Spot::cell(caret), false), rows, chosen);
                cells = frame.buffer_mut().clone();
            })
            .unwrap();
        (rect.expect("drawn"), cells)
    }

    fn text_of(buf: &Buffer, y: u16, from: u16, to: u16) -> String {
        (from..to).filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string())).collect()
    }

    /// **No ring, the status bar's ground**, in every theme and both moods —
    /// github's white light paper turns its status bar round, and the panel
    /// turns with it, because both read one function.
    #[test]
    fn the_panel_stands_on_the_status_bar_ground_with_no_ring() {
        let rows = [row("雨夜", Some("卷一/")), row("安寧", Some("人物/")), row("地名辭典", Some("root"))];
        for theme in THEMES {
            for dark in [true, false] {
                let config = config_for(theme);
                let ink = crate::theme::Palette::in_mood(&config, dark);
                let (rect, buf) = drawn(theme, dark, &rows, Some(0), (6, 2));
                assert_eq!((rect.y, rect.height), (3, 3), "{theme} {dark}: under the caret, a row per item, no ring");
                let ground = crate::status_bar_style_in(&config, ink).bg;
                assert!(ground.is_some(), "{theme}");
                for y in rect.y..rect.y + rect.height {
                    for x in rect.x..rect.x + rect.width {
                        assert_eq!(buf.cell((x, y)).unwrap().bg, ground.unwrap(), "{theme} {dark}: ({x},{y})");
                    }
                }
                let ring = ['─', '│', '╭', '╮', '╰', '╯', '┌', '┐', '└', '┘'];
                for y in rect.y.saturating_sub(1)..=rect.y + rect.height {
                    let line = text_of(&buf, y, 0, 40);
                    assert!(!line.chars().any(|c| ring.contains(&c)), "{theme} {dark}: no ring: {line:?}");
                }
                // The gold `›` stays readable on the new ground.
                let mark = buf.cell((rect.x, rect.y)).unwrap();
                assert_eq!((mark.symbol(), mark.fg), ("›", ink.gold()), "{theme} {dark}");
                let apart = crate::theme::contrast(ink.gold(), ground.unwrap());
                assert!(apart >= 2.5, "{theme} {dark}: the gold mark on the ground is {apart:.2}:1");
            }
        }
    }

    /// **Name in the text's ink, a folder in the link's, a deprecated item
    /// crossed out, the note quiet and flush right**; the `›` only on the
    /// chosen row, and none before anything is chosen.
    #[test]
    fn each_row_is_inked_by_what_it_is() {
        let mut folder = row("卷一/", Some("文件夹"));
        folder.folder = true;
        let mut old = row("count_old", Some("函数"));
        old.deprecated = true;
        let rows = [row("雨夜.md", Some("文件")), folder, old];
        for dark in [true, false] {
            let config = config_for("ink");
            let ink = crate::theme::Palette::in_mood(&config, dark);
            let (rect, buf) = drawn("ink", dark, &rows, Some(1), (4, 1));
            let cell = |x: u16, y: u16| buf.cell((x, y)).unwrap().clone();
            assert_eq!(cell(rect.x, rect.y).symbol(), " ", "not chosen, no mark");
            assert_eq!(cell(rect.x, rect.y + 1).symbol(), "›");
            assert_eq!(cell(rect.x + 2, rect.y).fg, ink.text(), "a name in the text's ink");
            assert_eq!(cell(rect.x + 2, rect.y + 1).fg, ink.azure(), "a folder in the link's ink");
            assert!(cell(rect.x + 2, rect.y + 2).modifier.contains(Modifier::CROSSED_OUT), "deprecated");
            assert!(!cell(rect.x + 2, rect.y).modifier.contains(Modifier::CROSSED_OUT));
            // The note ends one cell short of the right edge, in the quiet ink.
            let right = rect.x + rect.width;
            assert_eq!(cell(right - 7, rect.y + 1).symbol(), "文", "flush right: {:?}", text_of(&buf, rect.y + 1, rect.x, right));
            assert_eq!(text_of(&buf, rect.y + 1, right - 7, right - 1).replace(' ', ""), "文件夹");
            assert_eq!(cell(right - 3, rect.y + 1).fg, ink.quiet(), "a quiet note");
            assert_eq!(cell(right - 1, rect.y + 1).symbol(), " ", "a cell of air after the note");
        }
        let (rect, buf) = drawn("ink", true, &rows, None, (4, 1));
        assert!(!text_of(&buf, rect.y, rect.x, rect.x + rect.width).contains('›'));
        assert!(!text_of(&buf, rect.y + 1, rect.x, rect.x + rect.width).contains('›'), "nothing chosen, no mark");
    }

    /// **A scrollbar in the rightmost column only when the list is longer
    /// than the panel**: the thumb in the quiet ink, the track fainter.
    #[test]
    fn a_long_list_has_a_scrollbar_and_a_short_one_has_none() {
        let many: Vec<CompletionRow> = (1..=12).map(|i| row(&format!("harbour{i}"), None)).collect();
        for dark in [true, false] {
            let config = config_for("ink");
            let ink = crate::theme::Palette::in_mood(&config, dark);
            let (rect, buf) = drawn("ink", dark, &many, Some(0), (2, 1));
            assert_eq!(rect.height, 6, "the page size");
            let right = rect.x + rect.width - 1;
            let bar: Vec<(String, Color)> = (rect.y..rect.y + rect.height)
                .map(|y| buf.cell((right, y)).map(|c| (c.symbol().to_string(), c.fg)).unwrap())
                .collect();
            let thumb = bar.iter().filter(|(s, _)| s == "█").count();
            assert_eq!(thumb, 3, "six of twelve shown: a thumb half the track: {bar:?}");
            assert_eq!(bar[0], ("█".into(), ink.quiet()), "at the top of the list, the thumb at the top");
            assert_eq!(bar[5], ("│".into(), ink.rule()), "the track, fainter");
            assert_ne!(ink.quiet(), ink.rule());

            let (rect, buf) = drawn("ink", dark, &many, Some(11), (2, 1));
            let right = rect.x + rect.width - 1;
            assert_eq!(buf.cell((right, rect.y + 5)).unwrap().symbol(), "█", "at the end, the thumb at the bottom");
            assert!(text_of(&buf, rect.y + 5, rect.x, rect.x + rect.width).contains("› harbour12"));
        }
        let few: Vec<CompletionRow> = (1..=3).map(|i| row(&format!("harbour{i}"), None)).collect();
        let (rect, buf) = drawn("ink", true, &few, Some(0), (2, 1));
        for y in rect.y..rect.y + rect.height {
            let line = text_of(&buf, y, rect.x, rect.x + rect.width);
            assert!(!line.contains('█') && !line.contains('│'), "no scrollbar: {line:?}");
        }
    }

    /// **Above the caret when there is no room under it**, as word completion
    /// has always done.
    #[test]
    fn the_panel_flips_above_the_caret_near_the_foot() {
        let rows: Vec<CompletionRow> = (1..=4).map(|i| row(&format!("harbour{i}"), None)).collect();
        let (rect, _) = drawn("ink", true, &rows, None, (2, 11));
        assert_eq!(rect.y + rect.height, 11, "its last row just over the caret's");
    }
}
