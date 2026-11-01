//! **`ye --docgen <dir>`: the site's tables, written from the editor's own**
//! (2026-10-29) — helix's `cargo xtask docgen`, for the commands, the keys,
//! the settings and the languages. Each is written once per language the
//! interface speaks (`en`, `zht`, `zhs`), since every description here is
//! already a message in all three. The site's pages include these files, and
//! a check that regenerates them and finds a difference means the code moved
//! and the docs did not.

use std::fmt::Write as _;
use std::path::Path;
use yumete_core::messages::{say, set_language, Language};

/// A table cell: one line, its pipes escaped.
fn cell(text: &str) -> String {
    text.replace('\n', " ").replace('|', "\\|")
}

/// Inline code that survives a backtick inside it (the case group is `` ` ``).
fn code(text: &str) -> String {
    match text.contains('`') {
        true => format!("`` {text} ``"),
        false => format!("`{text}`"),
    }
}

fn commands() -> String {
    let mut out = String::from("| Command | Arguments | What it does |\n| --- | --- | --- |\n");
    for entry in yumete_core::command::COMMANDS {
        let mut name = format!("`:{}`", entry.name);
        for alias in entry.aliases {
            let _ = write!(name, " `:{alias}`");
        }
        let arguments: Vec<String> = entry.params.iter().map(|p| cell(&p.hint())).collect();
        let _ = writeln!(
            out,
            "| {name} | {} | {} |",
            match arguments.is_empty() {
                true => String::new(),
                false => format!("`{}`", arguments.join(" ")),
            },
            cell(&say(entry.help, &[])),
        );
    }
    out
}

/// The default keymap's keys. The vim keymap is helix's with a translation
/// layer on top, so its differences are prose (the "Coming from vim" page),
/// not a second copy of every table.
fn keys() -> String {
    let mut out = String::new();
    let table = |out: &mut String, title: String, rows: Vec<(String, String)>| {
        let _ = writeln!(out, "### {title}\n\n| Key | What it does |\n| --- | --- |");
        for (keys, what) in rows {
            let keys = match keys.trim() {
                "" => String::new(),
                keys => code(&cell(keys)),
            };
            let _ = writeln!(out, "| {keys} | {} |", cell(&what));
        }
        out.push('\n');
    };
    let said = |rows: &[(&str, &str)]| -> Vec<(String, String)> {
        rows.iter().map(|(k, m)| (k.to_string(), say(m, &[]))).collect()
    };
    table(&mut out, say("help.common.motion-title", &[]), said(yumete_core::editor::COMMON_MOTION));
    table(&mut out, say("help.common.editing-title", &[]), said(yumete_core::editor::COMMON_EDITING));
    for (title, prefix, rows) in yumete_core::editor::Editor::menus_for_docs() {
        let rows = rows
            .into_iter()
            .map(|(k, m)| {
                // `t c b` under `g` is three keys, `gt` `gc` `gb`.
                let k = match k.trim() {
                    "" => String::new(),
                    // `g` then `g` is written `gg`, as helix writes it; `Space`
                    // then `f` keeps its space.
                    k => {
                        let gap = if prefix.chars().count() == 1 { "" } else { " " };
                        k.split(' ')
                            .map(|one| match one {
                                // A space key drawn as a space is no key at all.
                                "␣" if gap == " " => format!("{prefix} Space"),
                                "⌫" if gap == " " => format!("{prefix} Backspace"),
                                one => format!("{prefix}{gap}{one}"),
                            })
                            .collect::<Vec<_>>()
                            .join(" / ")
                    }
                };
                (k, say(m, &[]))
            })
            .collect();
        table(&mut out, format!("{} ({})", say(title, &[]), code(&prefix)), rows);
    }
    out
}

/// One keypress as the docs write it: `Space`, `C-w`, `A-d`, `Tab`, `h`.
fn key_part(part: &str) -> String {
    let spelled = yumete_core::actions::spell(part);
    let spelled = spelled.trim_end();
    if spelled == "空格" {
        return "Space".to_string();
    }
    match spelled.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        Some(name) => {
            let mut chars = name.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
        None => spelled.to_string(),
    }
}

/// A run of keypresses: `gh` and `]d` together, as the keymap writes them;
/// `Space f` and `C-w s` apart.
fn keys_written(keys: &[String]) -> String {
    let mut parts: Vec<String> = keys.iter().map(|k| key_part(k)).collect();
    let plain = |part: &String, key: &String| part == key && key.chars().count() == 1;
    // After a one-letter group key the space bar is `␣` (`]␣`), as the
    // keymap page writes it: `] Space` would read as `]`, then `S`.
    if parts.first().zip(keys.first()).is_some_and(|(part, key)| plain(part, key)) {
        for (part, key) in parts.iter_mut().zip(keys).skip(1) {
            if key == " " {
                *part = "␣".to_string();
            }
        }
    }
    let together = parts.iter().zip(keys).all(|(part, key)| plain(part, key) || (part == "␣" && key == " "));
    match together {
        true => parts.concat(),
        false => parts.join(" "),
    }
}

/// **Every command a key can be bound to**, with every key that reaches it
/// under helix's keymap, under vim's, and in Insert mode (step 7 of the
/// keymap plan): so every key the editor has is on the site.
fn actions() -> String {
    let mut out = String::from("| Command | What it does | helix | vim | Insert |\n| --- | --- | --- | --- | --- |\n");
    let column = |keys: &[Vec<String>]| -> String {
        // `C-j` and Enter are one byte, and so are `C-i` and Tab: listed once.
        let mut written: Vec<String> = Vec::new();
        for one in keys.iter().map(|k| code(&cell(&keys_written(k)))) {
            if !written.contains(&one) {
                written.push(one);
            }
        }
        written.join(" ")
    };
    for (name, help, helix, vim, insert) in yumete_core::editor::Editor::commands_for_docs() {
        let _ = writeln!(
            out,
            "| `{name}` | {} | {} | {} | {} |",
            cell(&say(help, &[])),
            column(&helix),
            column(&vim),
            column(&insert),
        );
    }
    out.push_str("\n### vim keys that play other keys\n\n| Key | Plays |\n| --- | --- |\n");
    for (key, run) in yumete_core::editor::Editor::vim_runs_for_docs() {
        let _ = writeln!(out, "| {} | {} |", code(&cell(&keys_written(&[key]))), code(&cell(&run)));
    }
    out
}

fn settings() -> String {
    use yumete_config::settings_ui::{Kind, GROUPS, SETTINGS};
    let mut out = String::new();
    for group in GROUPS {
        let _ = writeln!(out, "### {}\n\n| Setting | Default | What it does |\n| --- | --- | --- |", say(group.label, &[]));
        for s in SETTINGS.iter().filter(|s| s.group == group.group) {
            let choices = match s.kind {
                Kind::Pick(ways) | Kind::PickOr(ways) => format!(
                    " ({})",
                    ways.iter().map(|c| format!("`{}` {}", c.word, say(c.label, &[]))).collect::<Vec<_>>().join(", ")
                ),
                Kind::Count { low, high, .. } => format!(" ({low}–{high})"),
                _ => String::new(),
            };
            let _ = writeln!(
                out,
                "| `[{}] {}` | `{}` | **{}** — {}{} |",
                s.table,
                s.key,
                cell(s.factory),
                cell(&say(s.label, &[])),
                cell(&say(s.hint, &[])),
                cell(&choices),
            );
        }
        out.push('\n');
    }
    out
}

fn languages() -> String {
    let servers = yumete_config::factory_servers();
    let mut out = String::from("| Language | `[lsp.…]` key | Server at the factory |\n| --- | --- | --- |\n");
    for language in yumete_core::code::Language::ALL {
        let name = language.name();
        let server = servers
            .get(name)
            .map(|list| list.iter().map(|s| format!("`{}`", s.command)).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        let _ = writeln!(out, "| {name} | `{name}` | {server} |");
    }
    out
}

/// Write every table into `dir/<language>/`.
pub fn write(dir: &Path) -> std::io::Result<()> {
    for (code, language) in [("en", Language::English), ("zht", Language::Traditional), ("zhs", Language::Simplified)] {
        set_language(language);
        let at = dir.join(code);
        std::fs::create_dir_all(&at)?;
        let note = "<!-- Written by `ye --docgen`. Do not edit: change the editor and regenerate. -->\n\n";
        std::fs::write(at.join("commands.md"), format!("{note}{}", commands()))?;
        std::fs::write(at.join("keys.md"), format!("{note}{}", keys()))?;
        std::fs::write(at.join("actions.md"), format!("{note}{}", actions()))?;
        std::fs::write(at.join("settings.md"), format!("{note}{}", settings()))?;
        std::fs::write(at.join("languages.md"), format!("{note}{}", languages()))?;
    }
    Ok(())
}
