//! What the frame tests share: the panels a test can open, and the check
//! that a ring is a rectangle as a CJK terminal draws it.

#![allow(dead_code)]

use yumete_config::Config;
use yumete_core::editor::Editor;
use yumete_core::input::Key;

/// A page of prose with the caret on its first line, in Normal mode.
pub fn page() -> Editor {
    let mut editor = Editor::new();
    editor.on_key(Key::Char('i'));
    for n in 0..30 {
        for c in format!("第{n}行，風從北邊來，雨落在南邊的屋頂上。").chars() {
            editor.on_key(Key::Char(c));
        }
        editor.on_key(Key::Enter);
    }
    editor.on_key(Key::Esc);
    keys(&mut editor, "gg");
    editor
}

pub fn keys(editor: &mut Editor, keys: &str) {
    for c in keys.chars() {
        editor.on_key(Key::Char(c));
    }
}

/// **Every panel a test can open**, by name, each on its own page.
pub fn scenes() -> Vec<(&'static str, Editor, Config)> {
    let mut out = Vec::new();
    let config = Config::default();

    let mut e = page();
    keys(&mut e, " ");
    out.push(("space menu", e, config.clone()));

    let mut e = page();
    keys(&mut e, "g");
    out.push(("g menu", e, config.clone()));

    let mut e = page();
    keys(&mut e, ":");
    out.push(("command menu", e, config.clone()));

    let mut e = page();
    keys(&mut e, ":w");
    out.push(("command menu, typed", e, config.clone()));

    let mut e = page();
    keys(&mut e, "/");
    out.push(("search hint", e, config.clone()));

    let mut e = page();
    keys(&mut e, " n");
    out.push(("character float", e, config.clone()));

    let mut e = page();
    keys(&mut e, " N");
    out.push(("character sidebar", e, config.clone()));

    let mut e = page();
    keys(&mut e, " /");
    out.push(("search sidebar", e, config.clone()));

    let mut e = page();
    keys(&mut e, " /");
    for c in "北邊".chars() {
        e.on_key(Key::Char(c));
    }
    e.on_key(Key::Esc);
    out.push(("search sidebar, left", e, config.clone()));

    let mut e = page();
    keys(&mut e, " b");
    out.push(("buffer picker", e, config.clone()));

    let mut e = page();
    e.execute(":view-hud full").unwrap();
    keys(&mut e, "3");
    out.push(("hud panel", e, config.clone()));

    let mut square = Config::default();
    square.panel.rounded = false;
    let mut e = page();
    keys(&mut e, "g");
    out.push(("g menu, square", e, square.clone()));
    let mut e = page();
    keys(&mut e, ":");
    out.push(("command menu, square", e, square));

    if let Some(e) = asking() {
        out.push(("query", e, config.clone()));
    }
    out
}

/// A save refused for a character its encoding cannot hold: the editor stops
/// and asks, in the middle of the page.
fn asking() -> Option<Editor> {
    let dir = std::env::temp_dir().join(format!("yumete-ring-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("稿.md");
    std::fs::write(&path, "第一稿。\n").ok()?;
    let mut editor = Editor::new();
    editor.execute(&format!(":open {}", path.display())).ok()?;
    editor.execute(":encoding big5").ok()?;
    editor.paste_text("😀\n");
    let _ = editor.execute(":w");
    editor.query().is_some().then_some(editor)
}

/// A row as `(cell, character)`: where ratatui put each glyph.
pub fn cells(row: &str) -> Vec<(usize, char)> {
    let mut at = 0;
    row.chars()
        .map(|c| {
            let here = at;
            at += yumete_cjk::drawn_width(&c.to_string()).max(1);
            (here, c)
        })
        .collect()
}

pub fn is_stroke(c: char) -> bool {
    ('\u{2500}'..='\u{257f}').contains(&c)
}

/// **Every ring on the frame is a rectangle as a CJK terminal draws it**:
/// each glyph paints the cells this editor counts for it (`char_width`), so
/// within a ring's span no glyph may paint over another, every row between
/// its top and bottom has a stroke on the ring's left and right edges, and
/// the bottom corners stand under the top ones. Gives how many rings it
/// found.
pub fn rings_are_rectangles(text: &str) -> Result<usize, String> {
    let lines: Vec<&str> = text.lines().collect();
    let rows: Vec<Vec<(usize, char)>> = lines.iter().map(|l| cells(l)).collect();
    let at = |y: usize, x: usize| rows.get(y).and_then(|r| r.iter().find(|&&(n, _)| n == x)).map(|&(_, c)| c);
    let mut found = 0;
    for (top, row) in rows.iter().enumerate() {
        for &(left, c) in row {
            if !matches!(c, '╭' | '┌' | '╔') {
                continue;
            }
            let Some(&(right, _)) = row.iter().find(|&&(n, d)| n > left && matches!(d, '╮' | '┐' | '╗')) else {
                return Err(format!("the ring at ({left}, {top}) has no top-right corner in {:?}", lines[top]));
            };
            let Some(bottom) = (top + 1..rows.len()).find(|&y| matches!(at(y, left), Some('╰' | '└' | '╚'))) else {
                return Err(format!("the ring at ({left}, {top}) has no bottom"));
            };
            let end = right + yumete_cjk::char_width(at(top, right).unwrap_or(' '));
            for y in top..=bottom {
                for (edge, x) in [("left", left), ("right", right)] {
                    match at(y, x) {
                        // A sidebar's inner wall carries its grip.
                        Some(c) if is_stroke(c) || c == '⣿' => {}
                        other => {
                            return Err(format!(
                                "row {y} of the ring at ({left}, {top}) has {other:?} for its {edge} wall in {:?}",
                                lines[y]
                            ))
                        }
                    }
                }
                // A glyph just left of the ring may reach into it too.
                let inside: Vec<(usize, char)> = rows[y].iter().copied().filter(|&(n, _)| n + 2 > left && n < end).collect();
                for (i, &(n, c)) in inside.iter().enumerate() {
                    if c == ' ' {
                        continue;
                    }
                    let reaches = n + yumete_cjk::char_width(c);
                    if let Some(&(m, d)) = inside[i + 1..].iter().find(|&&(m, d)| m < reaches && d != ' ') {
                        return Err(format!("{c:?} at {n} paints over {d:?} at {m} in {:?}", lines[y]));
                    }
                }
            }
            // The top and bottom rules reach their right corners: something
            // other than ground covers the cell before each.
            for y in [top, bottom] {
                let reaches = rows[y].iter().any(|&(n, c)| c != ' ' && n < right && n + yumete_cjk::char_width(c) >= right);
                if !reaches {
                    return Err(format!("row {y} of the ring at ({left}, {top}) stops short of its corner in {:?}", lines[y]));
                }
            }
            if !matches!(at(bottom, right), Some('╯' | '┘' | '╝')) {
                return Err(format!(
                    "the bottom-right corner of the ring at ({left}, {top}) is not under its top one in {:?}",
                    lines[bottom]
                ));
            }
            found += 1;
        }
    }
    Ok(found)
}
