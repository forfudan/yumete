//! **`ye --md-render`**: a Markdown file drawn by
//! `yumete-mdview` into PNG pictures, one a screen, without the editor — to
//! look at the renderer and to time it. A developer's door, kept out of
//! `--help`.
//!
//! ```text
//! ye --md-render <file.md> [--width 1000] [--scale 2] [--height 1400]
//!                [--theme light|dark] [--palette ink|github|<theme>]
//!                [--han sc|tc|ja] [--no-squeeze] [--screens 3 | --all] [--at-line N]
//!                [--vertical [--latin turn|jlreq] [--hanging]]
//!                [--out DIR] [--bench] [--export-html PAGE.html]
//! ```
//!
//! `--vertical` sets the page as the preview of a vertical text window does
//! (`:preview-vertical`): columns from the right, a screen the window's
//! width further left each time; `--latin` and `--hanging` are `[export]
//! vertical_latin` and `[export] hanging`.
//!
//! `--palette` names an editor theme (`ink`, 墨香, the default, or any name
//! `[theme] name` takes) whose colours the page is drawn in, in the mood
//! `--theme` says; `github` is GitHub's own Markdown colours.
//!
//! `--export-html` also writes the page `:export-as-preview` makes of the
//! file in the same colours, direction and Latin setting, to lay beside the
//! pictures.
//!
//! `--width` and `--height` are the picture's, in device pixels; `--scale`
//! is device pixels to a CSS pixel. `--at-line` starts the first screen at
//! the height that line is shown at, as the preview following the cursor
//! would. `--bench` also times a full layout, a screen, and an edit.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use yumete_mdview::{Options, Preview, Theme};
use yumete_tui::tex::Ratex as Tex;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

pub fn run(args: &[String]) -> ExitCode {
    let mut file: Option<PathBuf> = None;
    let (mut width, mut scale, mut height) = (1000u32, 2.0f32, None::<u32>);
    let (mut dark, mut screens, mut all, mut bench) = (false, 3usize, false, false);
    let mut at_line: Option<usize> = None;
    let mut palette = String::from("ink");
    let mut han: Option<yumete_mdview::fonts::Han> = None;
    let mut squeeze = true;
    let mut vertical = false;
    let mut latin = yumete_core::export::Latin::Turn;
    let mut hanging = false;
    let mut out = PathBuf::from(".");
    let mut page: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| -> Option<String> {
            let v = it.next().cloned();
            if v.is_none() {
                eprintln!("yumete: --md-render: {name} wants a value");
            }
            v
        };
        match arg.as_str() {
            "--width" => width = value("--width").and_then(|v| v.parse().ok()).unwrap_or(width),
            "--scale" => scale = value("--scale").and_then(|v| v.parse().ok()).unwrap_or(scale),
            "--height" => height = value("--height").and_then(|v| v.parse().ok()),
            "--screens" => screens = value("--screens").and_then(|v| v.parse().ok()).unwrap_or(screens),
            "--at-line" => at_line = value("--at-line").and_then(|v| v.parse().ok()),
            "--out" => out = value("--out").map(PathBuf::from).unwrap_or(out),
            "--export-html" => page = value("--export-html").map(PathBuf::from),
            "--theme" => dark = value("--theme").is_some_and(|v| v == "dark"),
            "--palette" => palette = value("--palette").unwrap_or(palette),
            "--han" => {
                han = value("--han").and_then(|v| match v.as_str() {
                    "sc" => Some(yumete_mdview::fonts::Han::Simplified),
                    "tc" => Some(yumete_mdview::fonts::Han::Traditional),
                    "ja" => Some(yumete_mdview::fonts::Han::Japanese),
                    _ => None,
                })
            }
            "--all" => all = true,
            "--vertical" => vertical = true,
            "--hanging" => hanging = true,
            "--latin" => {
                latin = match value("--latin").as_deref() {
                    Some("jlreq") => yumete_core::export::Latin::Jlreq,
                    _ => yumete_core::export::Latin::Turn,
                }
            }
            "--no-squeeze" => squeeze = false,
            "--bench" => bench = true,
            other if !other.starts_with("--") && file.is_none() => file = Some(PathBuf::from(other)),
            other => {
                eprintln!("yumete: --md-render: unknown {other:?}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(file) = file else {
        eprintln!("usage: ye --md-render <file.md> [--width N] [--scale S] [--height N] [--theme light|dark] [--palette ink|github|<theme>] [--han sc|tc|ja] [--screens N | --all] [--at-line N] [--vertical [--latin turn|jlreq] [--hanging]] [--out DIR] [--bench] [--export-html PAGE.html]");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("yumete: --md-render: {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("yumete: --md-render: {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    let height = height.unwrap_or(width * 7 / 5);
    let theme = match (palette.as_str(), dark) {
        ("github", true) => Theme::dark(),
        ("github", false) => Theme::light(),
        (name, dark) => match yumete_config::ThemeConfig::named(name) {
            Some(theme) => yumete_tui::preview_theme::theme(yumete_tui::theme::Palette::of_theme(&theme, dark)),
            None => {
                eprintln!("yumete: --md-render: no theme {name:?}");
                return ExitCode::from(2);
            }
        },
    };
    let base_dir = std::fs::canonicalize(&file).ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    if let Some(page) = &page {
        let settings = yumete_core::export::Settings { vertical, hanging: vertical && hanging, latin, ..Default::default() };
        let look = yumete_mdview::look::look(&theme, &settings.fonts, &text);
        let style = yumete_core::export::Style {
            settings,
            title: file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            source: yumete_core::syntax::Syntax::Markdown,
            base: Some(base_dir.clone()),
            painter: Some(yumete_tui::tex::for_export),
            look: Some(look),
        };
        if let Err(e) = std::fs::write(page, yumete_core::as_preview::html(&text, &style)) {
            eprintln!("yumete: --md-render: {}: {e}", page.display());
            return ExitCode::FAILURE;
        }
        println!("{}", page.display());
    }
    let options = Options { width, scale, theme, base_dir };

    let t = Instant::now();
    let mut preview = Preview::new(options.clone(), Box::new(Tex));
    preview.set_han(han);
    if !squeeze {
        preview.set_squeeze(false);
    }
    let shape = yumete_mdview::Vertical { latin, hanging };
    if vertical {
        preview.set_height(height);
        preview.set_vertical(Some(shape));
    }
    let along = if vertical { width } else { height };
    let t_new = ms(t);
    let t = Instant::now();
    let stats = preview.set_text(&text);
    let t_cold = ms(t);
    let (_, total) = preview.size();
    println!(
        "{} ({:?}): {} lines, {} blocks, page {}x{:.0} px; parse {:.1} ms, cold layout {:.1} ms (font context {:.1} ms)",
        file.display(),
        preview.han(),
        text.lines().count(),
        stats.blocks,
        width,
        total,
        stats.parse_ms,
        t_cold,
        t_new
    );

    let start = at_line.map_or(0.0, |l| preview.y_for_line(l));
    let mut y = start;
    let mut n = 0;
    while y < total && (all || n < screens) {
        let t = Instant::now();
        let picture = preview.render(y, height);
        let t_paint = ms(t);
        let path = out.join(format!("page-{n:03}.png"));
        if let Err(e) = std::fs::write(&path, picture.png()) {
            eprintln!("yumete: --md-render: {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("{} (from y {:.0}, line {}): {:.1} ms", path.display(), y, preview.line_for_y(y), t_paint);
        y += along as f32;
        n += 1;
    }

    if bench {
        // A full layout with the fonts warm but nothing kept.
        let mut fresh = Preview::new(options.clone(), Box::new(Tex));
        if vertical {
            fresh.set_height(height);
            fresh.set_vertical(Some(shape));
        }
        let _ = fresh.set_text("warm the fonts: 漢字 English `code` **bold**");
        let t = Instant::now();
        let s = fresh.set_text(&text);
        println!("full layout, fonts warm, nothing kept: {:.1} ms (parse {:.1} ms)", ms(t), s.parse_ms);
        // Screens: at the top, in the middle, at the end.
        for (name, at) in [("top", 0.0), ("middle", total / 2.0), ("end", (total - along as f32).max(0.0))] {
            let mut best = f64::MAX;
            for _ in 0..5 {
                let t = Instant::now();
                let _ = preview.render(at, height);
                best = best.min(ms(t));
            }
            println!("one screen {width}x{height} at the {name}: {best:.1} ms");
        }
        let t = Instant::now();
        let picture = preview.render(0.0, height);
        let _ = picture.png();
        println!("PNG encoding of a screen: {:.1} ms", ms(t) );
        // An edit: a word typed into a paragraph in the middle.
        let lines: Vec<&str> = text.lines().collect();
        let mid = (lines.len() / 2..lines.len()).find(|&k| lines[k].chars().count() > 20 && !lines[k].starts_with(['#', '|', '-', '>', '`', '$', ' '])).unwrap_or(0);
        let mut edited: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        edited[mid].insert_str(0, "改过的段落 edited ");
        let edited = edited.join("\n");
        let t = Instant::now();
        let s = preview.set_text(&edited);
        let t_layout = ms(t);
        let at = preview.y_for_line(mid);
        let t2 = Instant::now();
        let picture = preview.render((at - 100.0).max(0.0), height);
        let t_paint = ms(t2);
        println!(
            "edit at line {mid}: parse {:.1} ms + relayout {:.1} ms ({} kept, {} laid) + screen {:.1} ms = {:.1} ms",
            s.parse_ms,
            s.layout_ms,
            s.reused,
            s.laid,
            t_paint,
            t_layout + t_paint
        );
        let path = out.join("edited.png");
        let _ = std::fs::write(&path, picture.png());
        let t = Instant::now();
        let mut lookups = 0;
        for line in (0..lines.len()).step_by(7) {
            lookups += preview.y_for_line(line) as usize % 2;
        }
        println!("y_for_line for every 7th line: {:.2} ms ({lookups})", ms(t));
    }
    ExitCode::SUCCESS
}
