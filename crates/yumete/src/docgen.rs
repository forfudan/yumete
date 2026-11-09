//! **`ye --docgen <dir>`: the site's tables, written from the editor's own**
//! — helix's `cargo xtask docgen`, for the commands, the keys,
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
    // The fence is one backtick longer than the longest run inside, so
    // `` `` `` (two backticks, the case group's key pressed twice) holds.
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    match longest {
        0 => format!("`{text}`"),
        n => {
            let fence = "`".repeat(n + 1);
            format!("{fence} {text} {fence}")
        }
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

use yumete_config::{Flavor, FlavorItem, FlavorLevel, KeyPreset};
use yumete_core::editor::{DocKeyMenu, DocKeymap, Editor};

/// What a key runs, in words: a command's description, a command line, or
/// the keys it plays (`{n}`, where vim's count goes, left out).
fn what_runs(run: &str) -> String {
    run.split(", ")
        .map(|one| match yumete_core::actions::bound(one) {
            Some(yumete_core::actions::Bound::Named(action)) => say(action.help, &[]),
            Some(yumete_core::actions::Bound::Command(line)) => code(&format!(":{line}")),
            Some(yumete_core::actions::Bound::Keys(keys)) => {
                format!("as {}", code(&keys_written(&presses(&keys.replace("{n}", "")))))
            }
            None => one.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Keys that run the same thing, on one row, in the order they came.
fn by_run(keys: &[yumete_core::editor::DocKey]) -> Vec<(Vec<String>, String)> {
    let mut rows: Vec<(Vec<String>, String)> = Vec::new();
    for key in keys {
        let written = code(&cell(&keys_written(&key.keys)));
        match rows.iter_mut().find(|(_, run)| *run == key.run) {
            Some((all, _)) => all.push(written),
            None => rows.push((vec![written], key.run.clone())),
        }
    }
    rows
}

/// A menu's way in, and its rows' keys after it: `gg` and `]d` together,
/// `Space f` and `C-w s` apart.
fn menu_keys(menu: &DocKeyMenu) -> (String, impl Fn(&str) -> String) {
    let way_in = menu.opened_by.first().cloned().unwrap_or_default();
    let prefix = keys_written(&way_in);
    let together = way_in.iter().all(|k| k.chars().count() == 1 && k != " " && key_part(k) == *k);
    let gap = if together { "" } else { " " };
    let written = prefix.clone();
    (prefix, move |row: &str| match row.trim() {
        "" => String::new(),
        row => row
            .split(' ')
            .map(|one| match one {
                // A space key drawn as a space is no key at all.
                "␣" if gap == " " => format!("{written} Space"),
                "⌫" if gap == " " => format!("{written} Backspace"),
                one => format!("{written}{gap}{one}"),
            })
            .collect::<Vec<_>>()
            .join(" / "),
    })
}

/// The flavour with nothing on: the keymap as upstream has it.
fn no_flavor() -> Flavor {
    Flavor { level: FlavorLevel::Off, items: Default::default() }
}

/// What a keymap calls a selection being extended.
fn select_mode(preset: KeyPreset) -> &'static str {
    match preset {
        KeyPreset::Helix => "Select mode",
        KeyPreset::Vim => "Visual mode",
    }
}

/// **One keymap's page**: its keys with the flavour off, so
/// the page is helix's or vim's and nothing of yumete's on top.
fn keymap(preset: KeyPreset) -> String {
    let DocKeymap { normal, select, insert, menus } = Editor::keymap_for_docs(preset, no_flavor());
    let mut out = String::new();
    let keys_table = |out: &mut String, title: &str, keys: &[yumete_core::editor::DocKey]| {
        let _ = writeln!(out, "## {title}\n\n| Key | What it does | Command |\n| --- | --- | --- |");
        for (written, run) in by_run(keys) {
            let _ = writeln!(out, "| {} | {} | {} |", written.join(" "), cell(&what_runs(&run)), code(&cell(&run)));
        }
        out.push('\n');
    };
    keys_table(&mut out, "Normal mode", &normal);
    keys_table(&mut out, select_mode(preset), &select);
    keys_table(&mut out, "Insert mode", &insert);
    for menu in &menus {
        let (_, row_keys) = menu_keys(menu);
        let ways: Vec<String> = menu.opened_by.iter().map(|k| code(&keys_written(k))).collect();
        let _ = writeln!(out, "## {} ({})\n\n| Key | What it does |\n| --- | --- |", say(menu.title, &[]), ways.join(", "));
        for (keys, what) in &menu.rows {
            let keys = match row_keys(keys) {
                k if k.is_empty() => k,
                k => code(&cell(&k)),
            };
            let _ = writeln!(out, "| {keys} | {} |", cell(&say(what, &[])));
        }
        out.push('\n');
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
        Some("pageup") => "PageUp".to_string(),
        Some("pagedown") => "PageDown".to_string(),
        Some("S-tab") => "S-Tab".to_string(),
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
    // A table's grid: what each key means there, by command (`[keys.table]`).
    out.push_str("\n### In a table's grid (`[keys.table]`)\n\n| Key | Command |\n| --- | --- |\n");
    for (key, names) in yumete_core::editor::Editor::table_keys_for_docs() {
        let names: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
        let _ = writeln!(out, "| {} | {} |", code(&cell(&keys_written(&[key]))), names.join(" "));
    }
    // A prompt's, a picker's and a sidebar's (step 9), each under its table.
    for (place, rows) in yumete_core::editor::Editor::place_commands_for_docs() {
        let _ = writeln!(out, "\n### `[keys.{}]`\n\n| Command | What it does | Keys |\n| --- | --- | --- |", place.name());
        for (name, help, keys) in rows {
            let keys: Vec<Vec<String>> = keys.into_iter().map(|k| vec![k]).collect();
            let _ = writeln!(out, "| `{name}` | {} | {} |", cell(&say(help, &[])), column(&keys));
        }
    }
    out
}

/// A key run as `[keys.normal]` spells it (`C-we`), one keypress each in
/// the internal spelling, as [`keys_written`] takes them.
fn presses(spelled: &str) -> Vec<String> {
    let internal = yumete_core::actions::written(spelled);
    let mut out: Vec<String> = Vec::new();
    let mut chars = internal.chars();
    while let Some(c) = chars.next() {
        match c {
            yumete_core::actions::ALT => out.push(chars.next().map(|n| format!("{c}{n}")).unwrap_or_default()),
            c => out.push(c.to_string()),
        }
    }
    out
}

/// One row of a keymap, to be compared with the same keymap under another
/// flavour: where (`Normal mode`, a menu's title), the key as a reader
/// presses it, the key within its menu, and what it does.
struct Listed {
    place: String,
    pressed: String,
    within: String,
    what: String,
}

fn listed(preset: KeyPreset, flavor: Flavor) -> Vec<Listed> {
    let DocKeymap { normal, select, insert, menus } = Editor::keymap_for_docs(preset, flavor);
    let mut out = Vec::new();
    for (place, keys) in [("Normal mode", &normal), (select_mode(preset), &select), ("Insert mode", &insert)] {
        for key in keys {
            let pressed = keys_written(&key.keys);
            out.push(Listed { place: place.to_string(), within: pressed.clone(), pressed, what: what_runs(&key.run) });
        }
    }
    // A menu's rows are compared within it: `vim_space_menu` moves the
    // whole Space menu from `\` to Space, and that is one key, not forty.
    for menu in &menus {
        let (_, row_keys) = menu_keys(menu);
        for (keys, what) in &menu.rows {
            out.push(Listed {
                place: say(menu.title, &[]),
                pressed: row_keys(keys),
                within: keys.clone(),
                what: say(what, &[]),
            });
        }
    }
    out
}

/// **What one item changes under `preset`**: each key that does something
/// else with the item than without it, as (where, key, with, without),
/// from the factory flavour with the item turned on and off.
fn item_changes(item: FlavorItem, preset: KeyPreset) -> Vec<[String; 4]> {
    let with = |on: bool| {
        let mut flavor = Flavor::default();
        flavor.items.insert(item, on);
        listed(preset, flavor)
    };
    let (on, off) = (with(true), with(false));
    let same_key = |a: &Listed, b: &Listed| a.place == b.place && a.within == b.within;
    let mut out: Vec<[String; 4]> = Vec::new();
    // A menu's note (no key of its own) is the menu's, not the item's.
    let (on, off): (Vec<Listed>, Vec<Listed>) = (
        on.into_iter().filter(|row| !row.within.is_empty()).collect(),
        off.into_iter().filter(|row| !row.within.is_empty()).collect(),
    );
    for row in &on {
        let before = off.iter().find(|o| same_key(o, row));
        if before.is_some_and(|o| o.what == row.what) {
            continue;
        }
        let before = before.map(|o| o.what.clone()).unwrap_or_default();
        out.push([row.place.clone(), row.pressed.clone(), row.what.clone(), before]);
    }
    for row in off.iter().filter(|o| !on.iter().any(|r| same_key(r, o))) {
        out.push([row.place.clone(), row.pressed.clone(), String::new(), row.what.clone()]);
    }
    out
}

/// **The flavour's page**: each level's items with their keys
/// under each keymap, then what each item changes, key by key, so
/// `[keys.flavor_items]` can be written from it.
fn flavor() -> String {
    let mut out = String::new();
    let keys_under = |item: FlavorItem, preset: KeyPreset| -> String {
        match item.info().keymaps.contains(&preset) {
            true => item.keys_under(preset).iter().map(|k| code(&cell(&keys_written(&presses(k))))).collect::<Vec<_>>().join(" "),
            false => "—".to_string(),
        }
    };
    for level in [FlavorLevel::Basic, FlavorLevel::Full] {
        let _ = writeln!(out, "## `{}`\n\n| Item | helix | vim | What it does |\n| --- | --- | --- | --- |", level.name());
        for item in FlavorItem::ALL.iter().copied().filter(|i| i.info().level == level) {
            let _ = writeln!(
                out,
                "| [`{id}`](#{id}) | {} | {} | {} |",
                keys_under(item, KeyPreset::Helix),
                keys_under(item, KeyPreset::Vim),
                cell(&say(item.info().label, &[])),
                id = item.info().id,
            );
        }
        out.push('\n');
    }
    out.push_str("## What each item changes\n\n");
    for item in FlavorItem::ALL.iter().copied() {
        let info = item.info();
        let _ = writeln!(out, "### `{}` {{#{}}}\n\n{}\n", info.id, info.id, cell(&say(info.hint, &[])));
        let per: Vec<(KeyPreset, Vec<[String; 4]>)> =
            info.keymaps.iter().map(|preset| (*preset, item_changes(item, *preset))).collect();
        if per.iter().all(|(_, rows)| rows.is_empty()) {
            out.push_str("It changes what its keys do, not which command they run.\n\n");
            continue;
        }
        let table = |out: &mut String, rows: &[[String; 4]]| {
            out.push_str("| Where | Key | With the item | Without it |\n| --- | --- | --- | --- |\n");
            for [place, key, on, off] in rows {
                let key = match key.is_empty() {
                    true => String::new(),
                    false => code(&cell(key)),
                };
                let _ = writeln!(out, "| {} | {key} | {} | {} |", cell(place), cell(on), cell(off));
            }
            out.push('\n');
        };
        match per.as_slice() {
            [(_, one)] => table(&mut out, one),
            [(_, a), (_, b)] if a == b => table(&mut out, a),
            several => {
                for (preset, rows) in several {
                    let _ = writeln!(out, "Under {}:\n", preset.name());
                    match rows.is_empty() {
                        true => out.push_str("It changes what its keys do, not which command they run.\n\n"),
                        false => table(&mut out, rows),
                    }
                }
            }
        }
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
        std::fs::write(at.join("keymap-helix.md"), format!("{note}{}", keymap(KeyPreset::Helix)))?;
        std::fs::write(at.join("keymap-vim.md"), format!("{note}{}", keymap(KeyPreset::Vim)))?;
        std::fs::write(at.join("actions.md"), format!("{note}{}", actions()))?;
        std::fs::write(at.join("settings.md"), format!("{note}{}", settings()))?;
        std::fs::write(at.join("flavor.md"), format!("{note}{}", flavor()))?;
        std::fs::write(at.join("languages.md"), format!("{note}{}", languages()))?;
    }
    Ok(())
}
