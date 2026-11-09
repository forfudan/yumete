//! **`yumete --import helix` and `yumete --import lazyvim`** (2026-11-06):
//! another editor's configuration, written once into yumete's `config.toml`.
//!
//! Decided that day, after the survey in `local/survey-0.20/import.md`:
//!
//! - **helix runs nothing.** Its `config.toml` is read as TOML. LazyVim is
//!   *recognised* (`lazyvim.json`, `lazy-lock.json`, `lua/config/lazy.lua`),
//!   and what is written is the config on the docs site's LazyVim page.
//! - **vim, Neovim and LazyVim run the reader's own editor, headless**
//!   (2026-11-08, `--import vim` and `nvim` new, `lazyvim` reading the
//!   reader's own keys and options on top of the page's config): only the
//!   editor can say what a config sets. It runs on a copy of the config, in
//!   a home of its own, and never writes to the reader's files ([`dump`]);
//!   what it says is translated in [`vim`], only where that is certain.
//! - **Once, on request.** yumete never reads another editor's files on its
//!   own (see `write_the_first_config` in `main.rs`): this is a command the
//!   reader types, and its result is an ordinary config file.
//! - **Never over a config that is there.** With no `config.toml` it writes
//!   one; with one, it writes `config.imported.toml` beside it and says so.
//! - **What did not come over is listed** at the end of the file, one line an
//!   item with its reason, so the file is the whole report.
//!
//! The output is checked by the config's own reader in the tests: an import
//! that the editor then complains about at start-up is a broken import.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod dump;
mod vim;

// **The LazyVim config, one copy** (2026-11-06), kept in `yumete-config`.
// The docs site's `from/lazyvim.md` shows the same TOML, and a test below
// holds the page to it, so the page and the importer cannot drift apart.
use yumete_config::LAZYVIM;

/// Where the docs page for each import lives, for the file's header.
const LAZYVIM_PAGE: &str = "https://yumete.shurufa.app/from/lazyvim";

/// An environment to read variables from: the process's own in `run`, a
/// table in the tests (which must never point at the real `~/.config`).
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// A directory a variable names, when it names one.
fn dir_of(env: Env, name: &str) -> Option<PathBuf> {
    env(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// **helix's `config.toml`**: `$XDG_CONFIG_HOME/helix`, then `%APPDATA%\helix`
/// on Windows, then `~/.config/helix` — the same order yumete finds its own
/// (`yumete_config::config_dir`).
pub fn helix_config_path(env: Env) -> Option<PathBuf> {
    if let Some(dir) = dir_of(env, "XDG_CONFIG_HOME") {
        return Some(dir.join("helix").join("config.toml"));
    }
    if cfg!(windows) {
        if let Some(dir) = dir_of(env, "APPDATA") {
            return Some(dir.join("helix").join("config.toml"));
        }
    }
    let home = dir_of(env, "HOME").or_else(|| dir_of(env, "USERPROFILE"))?;
    Some(home.join(".config").join("helix").join("config.toml"))
}

/// **Neovim's config directory**, as Neovim finds it: `$NVIM_APPNAME` (or
/// `nvim`) under `$XDG_CONFIG_HOME`, `%LOCALAPPDATA%` on Windows, or
/// `~/.config`.
///
/// `None` too for a `$NVIM_APPNAME` Neovim refuses ([`nvim_app`]).
pub fn nvim_dir(env: Env) -> Option<PathBuf> {
    let app = nvim_app(env).ok()?;
    if let Some(dir) = dir_of(env, "XDG_CONFIG_HOME") {
        return Some(dir.join(app));
    }
    if cfg!(windows) {
        if let Some(dir) = dir_of(env, "LOCALAPPDATA") {
            return Some(dir.join(app));
        }
    }
    let home = dir_of(env, "HOME").or_else(|| dir_of(env, "USERPROFILE"))?;
    Some(home.join(".config").join(app))
}

/// **The file that says LazyVim is in `nvim`**, or `None`. Read, never run:
/// `lazyvim.json` is LazyVim's own; `lazy-lock.json` lists `LazyVim` once
/// lazy.nvim has installed it; `lua/config/lazy.lua` is the starter's, and
/// names `LazyVim/LazyVim` before anything is installed.
pub fn lazyvim_sign(nvim: &Path) -> Option<PathBuf> {
    let json = nvim.join("lazyvim.json");
    if json.is_file() {
        return Some(json);
    }
    [nvim.join("lazy-lock.json"), nvim.join("lua").join("config").join("lazy.lua")]
        .into_iter()
        .find(|file| std::fs::read_to_string(file).is_ok_and(|text| text.contains("LazyVim")))
}

/// The whole of the `config.toml` the first start writes for a vim reader.
pub const FIRST_CONFIG: &str = "[keys]\npreset = \"vim\"\n";

/// **Where the import goes**: `config.toml` when there is none, else beside
/// it, `config.imported.toml` (then `-2`, `-3`…, so an earlier import the
/// reader may have edited is not written over either). The flag says which.
///
/// The one-line file the first start wrote counts as none (review,
/// 2026-11-06): that start tells a LazyVim reader to run `--import
/// lazyvim`, and the import should then become their config. A name taken by
/// anything, a link to nowhere included, counts as taken.
pub fn target(dir: &Path) -> (PathBuf, bool) {
    let taken = |path: &Path| path.symlink_metadata().is_ok();
    let config = dir.join("config.toml");
    let only_the_first = || std::fs::read_to_string(&config).is_ok_and(|text| text == FIRST_CONFIG);
    if !taken(&config) || only_the_first() {
        return (config, false);
    }
    let first = dir.join("config.imported.toml");
    if !taken(&first) {
        return (first, true);
    }
    let mut n = 2;
    loop {
        let next = dir.join(format!("config.imported-{n}.toml"));
        if !taken(&next) {
            return (next, true);
        }
        n += 1;
    }
}

/// Write the import: a new file is only ever created, never written through
/// one that appeared since, and the first start's one-liner is replaced.
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    if std::fs::read_to_string(path).is_ok_and(|was| was == FIRST_CONFIG) {
        return std::fs::write(path, text);
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(path)?.write_all(text.as_bytes())
}

/// What an import wrote, and how much of it.
#[derive(Debug)]
pub struct Imported {
    /// The whole file.
    pub text: String,
    /// Lines carried over: settings and bindings.
    pub carried: usize,
    /// Items listed under `# Not carried over:`.
    pub skipped: usize,
}

/// The header's first lines, shared by the imports. `word` is what was
/// typed after `--import`; `from` is the file read (helix), the file that
/// showed LazyVim is there, or the config the editor was run with.
fn header(what: &str, word: &str, from: &Path, when: &str, beside: bool) -> String {
    let label = match what {
        "LazyVim" => "LazyVim found in",
        _ => "Source",
    };
    let mut out = format!(
        "# Imported from {what} by `yumete --import {word}` on {when}.\n# {label}: {}\n",
        one_line(&from.display().to_string())
    );
    if beside {
        out.push_str(
            "# config.toml was already there, so this was written beside it.\n\
             # Copy what you want from here into config.toml.\n",
        );
    }
    out
}

/// The list at the end: one comment line an item.
fn not_carried(skipped: &[String]) -> String {
    if skipped.is_empty() {
        return "\n# Not carried over: nothing.\n".to_string();
    }
    let mut out = String::from("\n# Not carried over:\n");
    for item in skipped {
        out.push_str("#   ");
        out.push_str(&one_line(item));
        out.push('\n');
    }
    out
}

/// A comment cannot hold a line break; a path or a value might. Nor any
/// other control character (review, 2026-11-09): a map's description with
/// an escape in it went into a comment, and the editor refused the whole
/// file it was in.
fn one_line(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

// ---- helix ---------------------------------------------------------------

/// helix's command aliases (helix 079a789e8, `typed.rs`), to its own names.
/// Read before yumete's names are looked at, because four of them are
/// yumete's for something else: helix's `:r` is `:read` and yumete's
/// `:replace`, `:u` is `:update` and `:undo`, `:sp` is `:hsplit` and
/// `:search-project`, `:lang` is `:set-language` and `:language`.
const HELIX_ALIASES: &[(&str, &str)] = &[
    ("x", "exit"),
    ("xit", "exit"),
    ("q", "quit"),
    ("o", "open"),
    ("edit", "open"),
    ("e", "open"),
    ("bc", "buffer-close"),
    ("bclose", "buffer-close"),
    ("bco", "buffer-close-others"),
    ("bcloseother", "buffer-close-others"),
    ("bca", "buffer-close-all"),
    ("bcloseall", "buffer-close-all"),
    ("bn", "buffer-next"),
    ("bnext", "buffer-next"),
    ("bp", "buffer-previous"),
    ("bprev", "buffer-previous"),
    ("w", "write"),
    ("wbc", "write-buffer-close"),
    ("n", "new"),
    ("fmt", "format"),
    ("ear", "earlier"),
    ("lat", "later"),
    ("wq", "write-quit"),
    ("wa", "write-all"),
    ("wqa", "write-quit-all"),
    ("xa", "write-quit-all"),
    ("qa", "quit-all"),
    ("cq", "cquit"),
    ("cd", "change-current-directory"),
    ("pushd", "push-directory"),
    ("popd", "pop-directory"),
    ("pwd", "show-directory"),
    ("char", "character-info"),
    ("rl", "reload"),
    ("rla", "reload-all"),
    ("u", "update"),
    ("dbg", "debug-start"),
    ("dbg-tcp", "debug-remote"),
    ("vs", "vsplit"),
    ("vnew", "vsplit-new"),
    ("hs", "hsplit"),
    ("sp", "hsplit"),
    ("hnew", "hsplit-new"),
    ("g", "goto"),
    ("lang", "set-language"),
    ("set", "set-option"),
    ("toggle", "toggle-option"),
    ("get", "get-option"),
    ("ts-subtree", "tree-sitter-subtree"),
    ("|", "pipe"),
    ("sh", "run-shell-command"),
    ("!", "run-shell-command"),
    ("diffget", "reset-diff-change"),
    ("diffg", "reset-diff-change"),
    ("mv", "move"),
    ("r", "read"),
];

/// helix's typable commands that yumete has under another name.
const HELIX_COMMAND_RENAMES: &[(&str, &str)] = &[
    ("change-current-directory", "cd"),
    ("show-directory", "pwd"),
    ("set-language", "syntax"),
    ("insert-output", "pipe-before"),
    ("append-output", "pipe-after"),
    ("run-shell-command", "sh"),
    ("vsplit-new", "vnew"),
    ("hsplit-new", "hnew"),
    ("config-reload", "reload-config"),
];

/// helix's static commands that yumete has under another name.
const HELIX_ACTION_RENAMES: &[(&str, &str)] = &[
    ("half_page_down", "page_cursor_half_down"),
    ("half_page_up", "page_cursor_half_up"),
    ("symbol_picker", "lsp_or_syntax_symbol_picker"),
    ("syntax_symbol_picker", "lsp_or_syntax_symbol_picker"),
    ("workspace_symbol_picker", "lsp_or_syntax_workspace_symbol_picker"),
    ("syntax_workspace_symbol_picker", "lsp_or_syntax_workspace_symbol_picker"),
];

/// **A helix theme's yumete family**, and the mood when the family has two.
/// `None` for a theme yumete has nothing like (and for `gruvbox_material`,
/// which is not gruvbox's palette).
fn helix_theme(name: &str) -> Option<(&'static str, Option<&'static str>)> {
    let n = name.trim().to_ascii_lowercase();
    let family = if n.starts_with("gruvbox_material") || n.starts_with("gruvbox-material") {
        return None;
    } else if n == "onedark" || n == "onedarker" {
        "one-dark"
    } else if n == "github" || n.starts_with("github_") {
        "github"
    } else if n.starts_with("monokai") {
        "monokai"
    } else if n.starts_with("dracula") {
        "dracula"
    } else if n.starts_with("tokyonight") {
        "tokyo-night"
    } else if n.starts_with("catppuccin") {
        "catppuccin"
    } else if n.starts_with("gruvbox") {
        "gruvbox"
    } else {
        return None;
    };
    let light = n.contains("light") || n.ends_with("_day") || n.ends_with("latte");
    let two_moods = matches!(family, "github" | "tokyo-night" | "catppuccin" | "gruvbox");
    match (two_moods, light) {
        (true, true) => Some((family, Some("light"))),
        (true, false) => Some((family, Some("dark"))),
        // A light version of a theme yumete has only dark.
        (false, true) => None,
        (false, false) => Some((family, None)),
    }
}

/// The theme names yumete has, for the reason a theme was not carried over.
const YUMETE_THEMES: &str = "ink, one-dark, github, monokai, dracula, tokyo-night, catppuccin, gruvbox, bw, morandi";

/// yumete's command for a word, by its name or one of its aliases: the
/// name, which is what the import writes.
fn yumete_command(word: &str) -> Option<&'static str> {
    yumete_core::command::COMMANDS
        .iter()
        .find(|e| e.name == word || e.aliases.contains(&word))
        .map(|e| e.name)
}

/// **A helix `:` line as yumete's**, or why not.
fn typable(line: &str) -> Result<String, String> {
    let line = line.trim_start_matches(':').trim();
    let (word, rest) = match line.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (line, ""),
    };
    let (base, bang) = match word.strip_suffix('!') {
        Some(base) if !base.is_empty() => (base, "!"),
        _ => (word, ""),
    };
    let helix = HELIX_ALIASES.iter().find(|(a, _)| *a == base).map_or(base, |&(_, name)| name);
    let (target, rest) = match helix {
        "set-option" | "toggle-option" => return option_command(helix, rest),
        // `:theme catppuccin_mocha` names a helix theme.
        "theme" if !rest.is_empty() => match helix_theme(rest) {
            Some((family, _)) => ("theme", family.to_string()),
            None => return Err(format!("yumete has no theme like {rest}")),
        },
        _ => (
            HELIX_COMMAND_RENAMES.iter().find(|(h, _)| *h == helix).map_or(helix, |&(_, y)| y),
            rest.to_string(),
        ),
    };
    let Some(name) = yumete_command(target) else {
        return Err(format!("yumete has no :{base}"));
    };
    let out = match rest.is_empty() {
        true => format!(":{name}{bang}"),
        false => format!(":{name}{bang} {rest}"),
    };
    match yumete_core::command::parse(&out) {
        Ok(_) => Ok(out),
        Err(_) => Err(format!("yumete's :{name} does not take {line:?}")),
    }
}

/// `:set-option` and `:toggle-option` for the options yumete has a command
/// for. yumete's `:set` is the settings page, so these never pass through.
fn option_command(helix: &str, rest: &str) -> Result<String, String> {
    let mut words = rest.split_whitespace();
    let (option, value) = (words.next().unwrap_or(""), words.next());
    // `soft-wrap.enable` is matched in halves: the messages test reads a
    // dotted literal after a `", "` as a message tag.
    let line = match (helix == "set-option", option.split_once('.'), value) {
        (true, None, Some(how @ ("relative" | "absolute"))) if option == "line-number" => {
            format!(":line-numbers {how}")
        }
        // `:view-wrap` takes `on｜off` and flips when bare (2026-11-07), so
        // both of helix's verbs have a yumete spelling.
        (true, Some(("soft-wrap", "enable")), Some("true")) => ":view-wrap on".to_string(),
        (true, Some(("soft-wrap", "enable")), Some("false")) => ":view-wrap off".to_string(),
        (false, Some(("soft-wrap", "enable")), None) => ":view-wrap".to_string(),
        _ => return Err(format!("yumete has no command for :{helix} {rest}")),
    };
    match yumete_core::command::parse(&line) {
        Ok(_) => Ok(line),
        Err(_) => Err(format!("yumete has no command for :{helix} {rest}")),
    }
}

/// **One key of a helix macro** (`<C-w>`, `<ret>`) in yumete's spelling.
fn macro_key(name: &str) -> Result<String, String> {
    let cannot = || Err(format!("its <{name}> is a key yumete cannot press from a binding"));
    let mut mods = "";
    let mut base = name;
    while base.len() > 2 && ["C-", "A-", "S-"].iter().any(|m| base.starts_with(m)) {
        if !mods.is_empty() {
            return cannot();
        }
        mods = &base[..2];
        base = &base[2..];
    }
    let lower = base.to_ascii_lowercase();
    let named = match lower.as_str() {
        "lt" => return if mods.is_empty() { Ok("<".to_string()) } else { cannot() },
        "gt" => return if mods.is_empty() { Ok(">".to_string()) } else { cannot() },
        "minus" => return if mods.is_empty() { Ok("-".to_string()) } else { cannot() },
        "ret" | "enter" => "enter",
        "del" | "delete" => "delete",
        "space" | "tab" | "backspace" | "up" | "down" | "left" | "right" | "home" | "end" | "pageup" | "pagedown" => {
            lower.as_str()
        }
        _ => {
            let mut chars = base.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else { return cannot() };
            return match mods {
                "" => Ok(c.to_string()),
                "C-" if c.is_ascii_alphabetic() => Ok(format!("C-{c}")),
                "A-" => Ok(format!("A-{c}")),
                "S-" if c.is_alphabetic() => Ok(c.to_uppercase().collect()),
                _ => cannot(),
            };
        }
    };
    match mods {
        "" => Ok(format!("<{named}>")),
        "S-" if named == "tab" => Ok("<S-tab>".to_string()),
        _ => cannot(),
    }
}

/// **A helix `@` macro as yumete's `@` run**, or why not. helix writes keys
/// with names in angle brackets; yumete writes a chord bare (`C-w`) and
/// only the named keys in brackets.
fn helix_macro(keys: &str) -> Result<String, String> {
    let mut out = String::from("@");
    let mut rest = keys;
    while let Some(c) = rest.chars().next() {
        if c == '<' {
            if let Some(end) = rest.find('>') {
                out.push_str(&macro_key(&rest[1..end])?);
                rest = &rest[end + 1..];
                continue;
            }
        }
        // helix reads `A-x` written out as three keys; yumete as one chord.
        if (c == 'A' || c == 'C') && rest[1..].starts_with('-') && rest.len() > 2 {
            return Err(format!("its {c}- would be read as a chord by yumete"));
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    Ok(out)
}

/// **One helix command as yumete's**, or why not.
fn command(rhs: &str, macros: &mut bool) -> Result<String, String> {
    if rhs.starts_with(':') {
        return typable(rhs);
    }
    if let Some(keys) = rhs.strip_prefix('@') {
        let run = helix_macro(keys)?;
        *macros = true;
        return Ok(run);
    }
    let name = HELIX_ACTION_RENAMES.iter().find(|(h, _)| *h == rhs).map_or(rhs, |&(_, y)| y);
    if yumete_core::actions::action(name).is_some() {
        return Ok(name.to_string());
    }
    Err(if rhs.starts_with("dap_") {
        "yumete has no debugger".to_string()
    } else if rhs.contains("primary_clipboard") {
        "yumete has no primary-selection clipboard".to_string()
    } else if rhs.contains("sub_word") {
        "yumete has no sub-word motions".to_string()
    } else {
        format!("yumete has no {rhs}")
    })
}

/// A right-hand side, rendered as TOML, or why not.
fn binding(value: &toml::Value, macros: &mut bool) -> Result<String, String> {
    match value {
        toml::Value::String(one) => command(one, macros).map(|c| quote(&c)),
        toml::Value::Array(many) if !many.is_empty() => {
            let mut out = Vec::new();
            for one in many {
                let Some(one) = one.as_str() else {
                    return Err("a list of something other than commands".to_string());
                };
                out.push(quote(&command(one, macros)?));
            }
            Ok(format!("[{}]", out.join(", ")))
        }
        _ => Err("not a command".to_string()),
    }
}

/// Whether yumete can bind a helix key, and why not.
fn bindable(key: &str) -> Result<(), String> {
    let cannot = || Err(format!("yumete cannot bind {key}"));
    if ["Cmd-", "Meta-", "Win-", "Super-"].iter().any(|m| key.contains(m)) {
        return cannot();
    }
    match yumete_config::helix_key(key) {
        // `S-a` would be read as three keys; helix writes Shift and a
        // letter as the capital, and yumete's Shift is on Tab alone.
        Ok(written) if written.contains("S-") && written != "<S-tab>" => cannot(),
        Ok(_) => Ok(()),
        Err(()) => cannot(),
    }
}

/// What a helix import gathers before it is written out.
#[derive(Default)]
struct Gathered {
    editor: Vec<String>,
    theme: Vec<String>,
    keys: String,
    /// `[[lsp.<language>]]` tables from `languages.toml`, ready to write.
    lsp: String,
    skipped: Vec<String>,
    carried: usize,
    macros: bool,
}

/// **One key table, and the groups under it**, as TOML sections. A table
/// left with no keys is not written: an empty group is a mistake the editor
/// reports at every start-up.
fn key_table(path: &[&str], table: &toml::Table, got: &mut Gathered) {
    let shown = format!("[{}]", path.join("."));
    let mut lines = String::new();
    let mut groups = Vec::new();
    for (key, value) in table {
        if let Err(why) = bindable(key) {
            got.skipped.push(format!("{shown} {key}: {why}"));
            continue;
        }
        match value {
            toml::Value::Table(inner) => groups.push((key.as_str(), inner)),
            _ => match binding(value, &mut got.macros) {
                Ok(rhs) => {
                    lines.push_str(&format!("{} = {rhs}\n", toml_key(key)));
                    got.carried += 1;
                }
                Err(why) => got.skipped.push(format!("{shown} {key} = {}: {why}", inline(value))),
            },
        }
    }
    if !lines.is_empty() {
        let header: Vec<String> = path.iter().map(|p| toml_key(p)).collect();
        got.keys.push_str(&format!("\n[{}]\n{lines}", header.join(".")));
    }
    for (key, inner) in groups {
        let mut deeper = path.to_vec();
        deeper.push(key);
        key_table(&deeper, inner, got);
    }
}

/// A whole number in `low..=high`, or why not.
fn number_in(value: &toml::Value, low: i64, high: i64) -> Result<i64, String> {
    match value.as_integer() {
        Some(n) if (low..=high).contains(&n) => Ok(n),
        Some(n) => Err(format!("{n} is outside yumete's {low}–{high}")),
        None => Err("not a whole number".to_string()),
    }
}

/// One `[editor]` line carried over.
fn set(got: &mut Gathered, key: &str, value: String) {
    got.editor.push(format!("{key} = {value}"));
    got.carried += 1;
}

/// **helix's `[editor]`, for the settings yumete has.**
fn editor_table(table: &toml::Table, got: &mut Gathered) {
    let mut line_numbers: Option<String> = None;
    let mut gutters_hide_numbers = false;
    for (key, value) in table {
        let skip = |got: &mut Gathered, why: &str| got.skipped.push(format!("[editor] {key}: {why}"));
        match key.as_str() {
            "scrolloff" => match number_in(value, 0, 20) {
                Ok(n) => set(got, "scrolloff", n.to_string()),
                Err(why) => skip(got, &why),
            },
            "scroll-lines" => match number_in(value, 1, 10) {
                Ok(n) => set(got, "wheel_step", n.to_string()),
                Err(why) => skip(got, &why),
            },
            "line-number" => match value.as_str() {
                Some(how @ ("absolute" | "relative")) => line_numbers = Some(how.to_string()),
                _ => skip(got, "not absolute or relative"),
            },
            "gutters" => {
                let layout = match value {
                    toml::Value::Array(layout) => Some(layout),
                    toml::Value::Table(t) => t.get("layout").and_then(toml::Value::as_array),
                    _ => None,
                };
                match layout {
                    Some(layout) => {
                        let has = |name: &str| layout.iter().any(|v| v.as_str() == Some(name));
                        gutters_hide_numbers = !has("line-numbers");
                        if !has("diff") {
                            set(got, "diff_gutter", "false".to_string());
                        }
                    }
                    None => skip(got, "no layout yumete can read"),
                }
            }
            "rulers" => {
                let first = value.as_array().and_then(|r| r.first());
                match first.map(|v| number_in(v, 1, 400)) {
                    Some(Ok(n)) => {
                        set(got, "ruler", n.to_string());
                        if value.as_array().is_some_and(|r| r.len() > 1) {
                            got.skipped.push(format!(
                                "[editor] rulers after the first: yumete draws one ruler, at {n}"
                            ));
                        }
                    }
                    Some(Err(why)) => skip(got, &why),
                    None => skip(got, "no ruler in the list"),
                }
            }
            "bufferline" => match value.as_str() {
                Some("always") => set(got, "tabs", quote("always")),
                Some("never") => set(got, "tabs", quote("never")),
                Some("multiple") => set(got, "tabs", quote("auto")),
                _ => skip(got, "not always, never or multiple"),
            },
            "text-width" => match number_in(value, 1, 400) {
                Ok(n) => set(got, "hard_wrap", n.to_string()),
                Err(why) => skip(got, &why),
            },
            "insert-final-newline" => match value.as_bool() {
                Some(on) => set(got, "insert_final_newline", on.to_string()),
                None => skip(got, "not true or false"),
            },
            "end-of-line-diagnostics" => match value.as_str() {
                Some(level @ ("disable" | "hint" | "info" | "warning" | "error")) => {
                    set(got, "end_of_line_diagnostics", quote(level))
                }
                _ => skip(got, "not disable, hint, info, warning or error"),
            },
            "soft-wrap" => sub_table(got, key, value, &mut |got, sub, v| match (sub, v.as_bool()) {
                ("enable", Some(on)) => {
                    set(got, "soft_wrap", on.to_string());
                    true
                }
                _ => false,
            }),
            "indent-guides" => sub_table(got, key, value, &mut |got, sub, v| match sub {
                "render" => match v.as_bool() {
                    Some(on) => {
                        set(got, "indent_guides", on.to_string());
                        true
                    }
                    None => false,
                },
                "character" => match v.as_str() {
                    Some(c) => {
                        set(got, "indent_guides_character", quote(c));
                        true
                    }
                    None => false,
                },
                "skip-levels" => match v.as_integer() {
                    Some(n) if (0..=8).contains(&n) => {
                        set(got, "indent_guides_skip_levels", n.to_string());
                        true
                    }
                    _ => false,
                },
                _ => false,
            }),
            "search" => sub_table(got, key, value, &mut |got, sub, v| match (sub, v.as_bool()) {
                ("smart-case", Some(on)) => {
                    set(got, "smart_case", on.to_string());
                    true
                }
                _ => false,
            }),
            "auto-pairs" => match value {
                toml::Value::Boolean(false) => set(got, "auto_pairs", quote("off")),
                toml::Value::Boolean(true) => set(got, "auto_pairs", quote("basic")),
                _ => skip(got, "yumete's pairs are fixed: auto_pairs is off, basic or full"),
            },
            "word-completion" => {
                let Some(t) = value.as_table() else {
                    skip(got, "not a table");
                    continue;
                };
                let off = t.get("enable").and_then(toml::Value::as_bool) == Some(false);
                if off {
                    set(got, "word_completion", "0".to_string());
                }
                for (sub, v) in t {
                    match sub.as_str() {
                        "enable" => {}
                        "trigger-length" if !off => match number_in(v, 1, 20) {
                            Ok(n) => set(got, "word_completion", n.to_string()),
                            Err(why) => got.skipped.push(format!("[editor.word-completion] {sub}: {why}")),
                        },
                        "trigger-length" => {}
                        _ => got.skipped.push(format!("[editor.word-completion] {sub}: no counterpart in yumete")),
                    }
                }
            }
            "auto-save" => skip(
                got,
                "helix saves the file; yumete's autosave, on already, keeps an unsaved copy beside it instead",
            ),
            _ if value.is_table() => got.skipped.push(format!("[editor.{key}]: no counterpart in yumete")),
            _ => skip(got, "no counterpart in yumete"),
        }
    }
    match (gutters_hide_numbers, line_numbers) {
        (true, _) => set(got, "line_numbers", quote("none")),
        (false, Some(how)) => set(got, "line_numbers", quote(&how)),
        (false, None) => {}
    }
}

/// A helix sub-table of `[editor]` (`[editor.soft-wrap]`): `take` carries
/// what it can and answers whether it did; the rest is listed.
fn sub_table(
    got: &mut Gathered,
    key: &str,
    value: &toml::Value,
    take: &mut dyn FnMut(&mut Gathered, &str, &toml::Value) -> bool,
) {
    let Some(t) = value.as_table() else {
        got.skipped.push(format!("[editor] {key}: not a table"));
        return;
    };
    for (sub, v) in t {
        if !take(got, sub, v) {
            got.skipped.push(format!("[editor.{key}] {sub}: no counterpart in yumete"));
        }
    }
}

/// **helix's `theme`**: one name, or `{ light, dark }`.
fn theme(value: &toml::Value, got: &mut Gathered) {
    let unlike = |name: &str| format!("theme = {}: yumete has no theme like it; it has {YUMETE_THEMES}", quote(name));
    match value {
        toml::Value::String(name) => match helix_theme(name) {
            Some((family, mood)) => {
                got.theme.push(format!("name = {}", quote(family)));
                if let Some(mood) = mood {
                    got.theme.push(format!("mode = {}", quote(mood)));
                }
                got.carried += 1;
            }
            None => got.skipped.push(unlike(name)),
        },
        toml::Value::Table(t) => {
            let light = t.get("light").and_then(toml::Value::as_str);
            let dark = t.get("dark").and_then(toml::Value::as_str);
            match (light.and_then(helix_theme), dark.and_then(helix_theme)) {
                (Some((a, _)), Some((b, _))) if a == b => {
                    got.theme.push(format!("name = {}", quote(a)));
                    got.theme.push(format!("mode = {}", quote("auto")));
                    got.carried += 1;
                }
                _ => got.skipped.push(format!(
                    "theme = {}: yumete follows the terminal's light or dark within one theme, and these are two",
                    inline(value)
                )),
            }
        }
        _ => got.skipped.push(format!("theme = {}: not a theme name", inline(value))),
    }
}

/// **helix's `config.toml` as yumete's**, or the reason it cannot be read.
///
/// `languages` is the `languages.toml` beside it, when there is one: only
/// its servers' `config` is carried ([`helix_languages`]).
pub fn helix(source: &str, from: &Path, when: &str, beside: bool, languages: Option<&str>) -> Result<Imported, String> {
    let table: toml::Table = toml::from_str(source).map_err(|e| e.message().to_string())?;
    let mut got = Gathered::default();
    for (key, value) in &table {
        match (key.as_str(), value) {
            ("keys", toml::Value::Table(modes)) => {
                for (mode, keys) in modes {
                    match (mode.as_str(), keys) {
                        ("normal" | "select" | "insert", toml::Value::Table(keys)) => {
                            key_table(&["keys", mode], keys, &mut got)
                        }
                        _ => got.skipped.push(format!("[keys.{mode}]: yumete has no such mode")),
                    }
                }
            }
            ("editor", toml::Value::Table(editor)) => editor_table(editor, &mut got),
            ("theme", value) => theme(value, &mut got),
            _ => got.skipped.push(format!("{key}: no counterpart in yumete")),
        }
    }
    if let Some(languages) = languages {
        helix_languages(languages, &mut got);
    }
    let mut text = header("helix", "helix", from, when, beside);
    if got.macros {
        text.push_str(
            "# A run written with @ presses yumete's keys: helix's, with the\n\
             # Yumete flavour on top (see Coming from helix on the docs site).\n",
        );
    }
    text.push_str("\n[keys]\npreset = \"helix\"\n");
    if !got.editor.is_empty() {
        text.push_str("\n[editor]\n");
        for line in &got.editor {
            text.push_str(line);
            text.push('\n');
        }
    }
    if !got.theme.is_empty() {
        text.push_str("\n[theme]\n");
        for line in &got.theme {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.push_str(&got.keys);
    text.push_str(&got.lsp);
    text.push_str(&not_carried(&got.skipped));
    Ok(Imported { text, carried: got.carried, skipped: got.skipped.len() })
}

/// helix's names for the servers yumete starts, where the name is not the
/// program (helix `languages.toml` of 2026-07-23).
const HELIX_SERVER_COMMANDS: &[(&str, &str)] =
    &[("jedi", "jedi-language-server"), ("r", "R"), ("ada-language-server", "ada_language_server")];

/// **helix's `languages.toml`: each server's `config`** (2026-11-07), for
/// the servers yumete starts. Nothing else in it is carried, and each thing
/// left is listed.
///
/// Warning: **The whole list is written, not the one entry.** A language
/// named under `[lsp.…]` replaces its list, so `[language-server.superhtml]
/// config` becomes both of `html`'s entries, the first with the factory's
/// own `config`. A server several languages start (`clangd` for `c` and
/// `cpp`) is written for each.
///
/// Warning: **The reader's `config` replaces helix's, it does not merge
/// into it.** That is what helix does: its `languages.toml` is merged three
/// tables deep, and `config` is the fourth.
fn helix_languages(source: &str, got: &mut Gathered) {
    let table: toml::Table = match toml::from_str(source) {
        Ok(table) => table,
        Err(e) => {
            got.skipped.push(format!("languages.toml: does not parse: {}", e.message()));
            return;
        }
    };
    let factory = yumete_config::factory_servers();
    // Language → (entry, config), in the language's own order.
    let mut given: std::collections::BTreeMap<&str, Vec<(usize, serde_json::Value)>> = Default::default();
    for (key, value) in &table {
        match (key.as_str(), value) {
            ("language-server", toml::Value::Table(servers)) => {
                for (name, server) in servers {
                    let toml::Value::Table(server) = server else {
                        got.skipped.push(format!("languages.toml [language-server.{name}]: not a table"));
                        continue;
                    };
                    for key in server.keys().filter(|k| *k != "config") {
                        got.skipped.push(format!(
                            "languages.toml [language-server.{name}] {key}: only config is carried over"
                        ));
                    }
                    let Some(config) = server.get("config") else { continue };
                    let command = HELIX_SERVER_COMMANDS
                        .iter()
                        .find(|(helix, _)| helix == name)
                        .map_or(name.as_str(), |(_, command)| command);
                    let Ok(config) = serde_json::to_value(config) else { continue };
                    let mut found = false;
                    for (language, list) in &factory {
                        for (at, _) in list.iter().enumerate().filter(|(_, s)| s.command == command) {
                            given.entry(language.as_str()).or_default().push((at, config.clone()));
                            found = true;
                        }
                    }
                    match found {
                        true => got.carried += 1,
                        false => got.skipped.push(format!(
                            "languages.toml [language-server.{name}] config: yumete starts no {command} for any language; \
                             name it under [lsp.<language>] with this config"
                        )),
                    }
                }
            }
            ("language", _) => got.skipped.push(
                "languages.toml [[language]]: not read; which servers a language runs is set under \
                 [lsp.<language>] (see Language servers)"
                    .to_string(),
            ),
            _ => got.skipped.push(format!("languages.toml {key}: no counterpart in yumete")),
        }
    }
    if given.is_empty() {
        return;
    }
    got.lsp.push_str(
        "\n# Language servers' config, from helix's languages.toml. A language\n\
         # named here replaces its whole list, so each list is written out.\n",
    );
    for (language, configs) in given {
        for (at, server) in factory[language].iter().enumerate() {
            got.lsp.push_str(&format!("\n[[lsp.{}]]\ncommand = {}\n", toml_key(language), quote(&server.command)));
            if !server.args.is_empty() {
                let args: Vec<String> = server.args.iter().map(|a| quote(a)).collect();
                got.lsp.push_str(&format!("args = [{}]\n", args.join(", ")));
            }
            let config = configs.iter().rev().find(|(i, _)| *i == at).map(|(_, c)| c).or(server.config.as_ref());
            if let Some(config) = config {
                got.lsp.push_str(&format!("config = {}\n", inline_json(config)));
            }
        }
    }
}

// ---- LazyVim -------------------------------------------------------------

/// **LazyVim's keys**: the shared config, the reader's own keys and options
/// on top when their Neovim could be run (decision 2), and the list of what
/// in `nvim` it leaves alone. `sign` is the file that said LazyVim is
/// there; `own` is what the run read, or why there was none.
fn lazyvim(nvim: &Path, sign: &Path, when: &str, beside: bool, own: Result<&vim::Own, &str>) -> Imported {
    let mut skipped = Vec::new();
    if own.is_err() {
        for file in ["lua/config/keymaps.lua", "lua/config/options.lua", "lua/config/autocmds.lua"] {
            if nvim.join(file).is_file() {
                skipped.push(format!("{file}: not read, as Neovim could not be run"));
            }
        }
    }
    let plugins = std::fs::read_dir(nvim.join("lua").join("plugins"))
        .map(|dir| dir.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "lua")).count())
        .unwrap_or(0);
    if plugins > 0 {
        skipped.push(format!(
            "lua/plugins/ ({plugins} file{}): plugins and their options; yumete has no plugins",
            if plugins == 1 { "" } else { "s" }
        ));
    }
    let extras = std::fs::read_to_string(nvim.join("lazyvim.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| json.get("extras").and_then(|e| e.as_array()).cloned())
        .unwrap_or_default();
    // **mini.surround's extra lands on yumete's own surround** (2026-11-07):
    // its `gsa` `gsd` `gsr` are vim-surround's `ys` `ds` `cs` with another
    // prefix, and visual `gsa` is helix's `ms`. nvim's `gs` sleeps and
    // yumete's does nothing, so nothing is taken.
    let mut surround = false;
    for extra in extras.iter().filter_map(|e| e.as_str()) {
        if extra == "lazyvim.plugins.extras.coding.mini-surround" {
            surround = true;
            skipped.push(format!("LazyVim extra {extra} (lazyvim.json): gsf, gsF, gsh and gsn have no counterpart"));
            continue;
        }
        skipped.push(format!("LazyVim extra {extra} (lazyvim.json): yumete has no plugins"));
    }
    skipped.push(
        "LazyVim's keys for git staging and blame, the debugger, Trouble, notifications and the plugin managers: \
         yumete has none of these"
            .to_string(),
    );
    let mut text = header("LazyVim", "lazyvim", sign, when, beside);
    text.push_str(&format!("# The config on the Coming from LazyVim page: {LAZYVIM_PAGE}\n"));
    match own {
        Ok(own) => {
            text.push_str("# Your own keys and options come after it, read by running your Neovim\n# headless on a copy of its config.\n");
            text.push_str(&said_lines("Neovim", own));
        }
        Err(why) => text.push_str(&format!("# Your own keys and options were not read: {}.\n", one_line(why))),
    }
    text.push('\n');
    // **The reader's `[editor]` lines go inside the page's**, which ends the
    // config (a test holds it there): TOML has one table of a name. Where
    // the page sets the same key, the reader's own value takes the page's
    // line (review, 2026-11-09: a reader who turned LazyVim's flash on yank
    // off got the page's `yank_flash = true`); the same value is the page's.
    let (keys_part, editor_part) = LAZYVIM.split_at(LAZYVIM.find("\n[editor]\n").map_or(LAZYVIM.len(), |at| at + 1));
    let key_of = |line: &str| line.split(" = ").next().unwrap_or("").to_string();
    let value_of = |line: &str| line.split_once(" = ").map(|(_, v)| v.split("  #").next().unwrap_or("").trim().to_string());
    let mut editor: Vec<String> = editor_part.lines().map(str::to_string).collect();
    let mut carried_own = 0;
    let mut overlap = 0;
    if let Ok(own) = own {
        for line in &own.editor {
            match editor.iter().position(|l| key_of(l) == key_of(line)) {
                Some(at) => {
                    overlap += 1;
                    if value_of(&editor[at]) != value_of(line) {
                        editor[at] = line.clone();
                    }
                }
                None => editor.push(line.clone()),
            }
        }
        carried_own = own.carried;
    }
    let config = format!("{keys_part}{}\n", editor.join("\n"));
    let config = match surround {
        true => with_mini_surround(&config),
        false => config,
    };
    // Every line of the page that binds or sets something; a group's
    // `label` names one.
    let carried = LAZYVIM
        .lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with('#') && !l.starts_with('[') && l.contains(" = ") && !l.starts_with("label "))
        .count()
        + if surround { 4 } else { 0 };
    text.push_str(&config);
    if let Ok(own) = own {
        text.push_str(&theme_and_keys(own));
        skipped.extend(own.skipped.iter().cloned());
    }
    text.push_str(&not_carried(&skipped));
    Imported { text, carried: carried + carried_own - overlap, skipped: skipped.len() }
}

/// **What a config that stopped at an error lost**, for the header
/// (review, 2026-11-09): Neovim stops a Lua file at its first error, so
/// whatever the file set after it is neither carried nor listed.
const STOPPED_LINES: &str = "# Your config stopped at an error, so what it sets after the error was\n\
                             # not read: it is neither below nor in the list at the end.\n";

/// The same, for the terminal.
fn said_stopped(own: &vim::Own) {
    if own.stopped {
        println!("yumete: your config stopped at an error, so what it sets after the error was not read");
    }
}

/// What the editor said while starting, and the note on `@` runs, as
/// header lines.
fn said_lines(editor: &str, own: &vim::Own) -> String {
    let mut out = String::new();
    if let Some(said) = &own.said {
        out.push_str(&format!("# {editor} said while starting: {}\n", one_line(said)));
    }
    if own.stopped {
        out.push_str(STOPPED_LINES);
    }
    if own.runs {
        out.push_str(
            "# A run written with @ presses yumete's vim keys, not your own maps\n\
             # (a map that relied on another map of yours loses it).\n",
        );
    }
    out
}

/// The `[theme]` and `[keys.vim.…]` tables of a vim or Neovim import.
fn theme_and_keys(own: &vim::Own) -> String {
    let mut out = String::new();
    if !own.theme.is_empty() {
        out.push_str("\n[theme]\n");
        for line in &own.theme {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&own.keys);
    out
}

/// **A vim or Neovim import's file**: the vim keymap, the reader's
/// settings, theme and keys, and the list.
fn vim_file(what: &str, word: &str, from: &Path, version: &str, when: &str, beside: bool, own: &vim::Own) -> Imported {
    let mut text = header(what, word, from, when, beside);
    text.push_str(&format!("# Read by running {what} {version} headless on a copy of your config.\n"));
    text.push_str(&said_lines(what, own));
    text.push_str("\n[keys]\npreset = \"vim\"\n");
    if !own.editor.is_empty() {
        text.push_str("\n[editor]\n");
        for line in &own.editor {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.push_str(&theme_and_keys(own));
    text.push_str(&takes_over(&own.takes));
    text.push_str(&not_carried(&own.skipped));
    Imported { text, carried: own.carried, skipped: own.skipped.len() }
}

/// **The keys of yumete's that imported maps took**, before the list of
/// what did not come over: these did, and replaced something.
fn takes_over(takes: &[String]) -> String {
    let mut out = String::new();
    if !takes.is_empty() {
        out.push_str("\n# Takes over yumete's keys:\n");
        for line in takes {
            out.push_str("#   ");
            out.push_str(&one_line(line));
            out.push('\n');
        }
    }
    out
}

/// The LazyVim config with the mini.surround extra's keys: three lines at
/// the end of its `[keys.normal]` (TOML has one table of a name) and a
/// `[keys.select]` of its own.
fn with_mini_surround(config: &str) -> String {
    let normal = "# The mini.surround extra (lazyvim.json), on yumete's ys, ds and cs\n\
                  gsa = \"@ys\"\n\
                  gsd = \"@ds\"\n\
                  gsr = \"@cs\"\n";
    let select = "\n# Visual gsa wraps the selection, as the extra's does\n\
                  [keys.select]\n\
                  gsa = \"surround_add\"\n";
    // The `[keys.normal]` table runs to the blank line before `[editor]`.
    let at = config.find("\n[editor]").unwrap_or(config.len());
    let (keys, rest) = config.split_at(at);
    format!("{keys}{normal}{rest}{select}")
}

// ---- TOML spelling ---------------------------------------------------------

/// A TOML basic string.
fn quote(text: &str) -> String {
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A TOML key: bare when it may be, quoted otherwise.
fn toml_key(key: &str) -> String {
    match !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        true => key.to_string(),
        false => quote(key),
    }
}

/// **A JSON value as an inline TOML value**: a server's `config` on one
/// line. TOML has no `null`, and a TOML file never gives one, so one is left
/// out (and is an empty table at the top).
fn inline_json(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::Null => "{}".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => quote(s),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().filter(|v| !v.is_null()).map(inline_json).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(map) if map.is_empty() => "{}".to_string(),
        Value::Object(map) => {
            let pairs: Vec<String> = map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| format!("{} = {}", toml_key(k), inline_json(v)))
                .collect();
            format!("{{ {} }}", pairs.join(", "))
        }
    }
}

/// A value on one line, for the list of what was not carried over.
fn inline(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => quote(s),
        other => one_line(&other.to_string()),
    }
}

// ---- vim and Neovim --------------------------------------------------------

/// **`$NVIM_APPNAME`, or `nvim`**; `Err` with the name when Neovim would
/// refuse it.
///
/// Warning (review, 2026-11-09): **the name is joined onto the copy's
/// directory**, and `PathBuf::join` with an absolute path drops what it is
/// joined to. `NVIM_APPNAME=/abs/myconf` (or `~/.config/nvim`, which the
/// shell expands) made the copy's target the reader's config itself, and
/// copying each file onto itself emptied it. Neovim takes a name or a
/// relative path below its config directory and refuses the rest ("must be
/// a name or relative path": an absolute path, `..`, `.` alone, a leading
/// `~`), so the import refuses the same names before anything runs.
fn nvim_app(env: Env) -> Result<String, String> {
    use std::path::Component;
    let Some(app) = env("NVIM_APPNAME").filter(|v| !v.is_empty()) else { return Ok("nvim".to_string()) };
    let parts: Vec<Component> = Path::new(&app).components().collect();
    let plain = parts.iter().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    let named = parts.iter().any(|c| matches!(c, Component::Normal(_)));
    match plain && named && !app.starts_with('~') {
        true => Ok(app),
        false => Err(app),
    }
}

/// What the terminal is told for a `$NVIM_APPNAME` Neovim refuses.
fn bad_app(app: &str) -> String {
    format!(
        "$NVIM_APPNAME is {}, and Neovim takes only a name or a relative path below its config directory",
        quote(app)
    )
}

/// The name of Neovim's data directory: Windows adds `-data`, so that it is
/// not the config directory beside it in `%LOCALAPPDATA%`.
fn nvim_data_name(app: &str) -> String {
    match cfg!(windows) {
        true => format!("{app}-data"),
        false => app.to_string(),
    }
}

/// **Neovim's data directory**, where lazy.nvim keeps the plugins:
/// `$XDG_DATA_HOME`, `%LOCALAPPDATA%` on Windows, or `~/.local/share`.
fn nvim_data_dir(env: Env) -> Option<PathBuf> {
    let name = nvim_data_name(&nvim_app(env).ok()?);
    if let Some(dir) = dir_of(env, "XDG_DATA_HOME") {
        return Some(dir.join(name));
    }
    if cfg!(windows) {
        if let Some(dir) = dir_of(env, "LOCALAPPDATA") {
            return Some(dir.join(name));
        }
    }
    let home = dir_of(env, "HOME").or_else(|| dir_of(env, "USERPROFILE"))?;
    Some(home.join(".local").join("share").join(name))
}

/// Each link the copy left out, one line each: the config may miss it.
fn said_left_out(sandbox: &dump::Sandbox) {
    for link in &sandbox.left_out {
        println!("yumete: left out of the copy it runs on: {link}, outside the config");
    }
}

/// **Run the reader's Neovim and a baseline**, each on the same copy:
/// `--clean` for `--import nvim`, LazyVim alone for `--import lazyvim`.
fn read_nvim(program: &str, env: Env, lazyvim: bool, timeout: std::time::Duration) -> Result<(vim::Dump, vim::Dump, Option<String>), dump::Failed> {
    let app = nvim_app(env).map_err(|app| dump::Failed::Copy(bad_app(&app)))?;
    let mut sandbox = dump::Sandbox::new(timeout)?;
    if let Some(config) = nvim_dir(env) {
        let to = sandbox.config().join(&app);
        sandbox.copy(&config, &to, &[])?;
    }
    if let Some(data) = nvim_data_dir(env) {
        let to = sandbox.data().join(nvim_data_name(&app));
        sandbox.copy(&data, &to, dump::DATA_LEFT_OUT)?;
    }
    said_left_out(&sandbox);
    let script = ("dump.lua", dump::DUMP_LUA);
    let user = sandbox.run(program, &dump::nvim_args(dump::Nvim::Own, &sandbox), script, timeout)?;
    let how = if lazyvim { dump::Nvim::LazyVimBase } else { dump::Nvim::Clean };
    let base = sandbox.run(program, &dump::nvim_args(how, &sandbox), script, timeout)?;
    let mut dumped = vim::Dump::from_json(&user.json);
    dumped.stopped = user.stopped;
    Ok((dumped, vim::Dump::from_json(&base.json), user.said))
}

/// The places vim may keep a vimrc, in `:h vimrc`'s order.
fn vimrc_places(env: Env) -> Vec<PathBuf> {
    let Some(home) = dir_of(env, "HOME").or_else(|| dir_of(env, "USERPROFILE")) else { return Vec::new() };
    let xdg = dir_of(env, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
    let mut places = vec![home.join(".vimrc"), home.join(".vim").join("vimrc"), xdg.join("vim").join("vimrc")];
    if cfg!(windows) {
        places.extend([home.join("_vimrc"), home.join("vimfiles").join("vimrc")]);
    }
    places
}

/// **The reader's vimrc**, or `None`. `$VIMINIT` is a config in a variable,
/// which this import does not read.
pub fn vimrc_path(env: Env) -> Result<Option<PathBuf>, String> {
    if env("VIMINIT").is_some_and(|v| !v.is_empty()) {
        return Err("$VIMINIT is set; --import vim reads a vimrc file".to_string());
    }
    Ok(vimrc_places(env).into_iter().find(|p| p.is_file()))
}

/// **Run the reader's Vim and a baseline** (`-u NONE`) on a copy of the
/// vimrc and the directories beside it.
fn read_vim(program: &str, env: Env, vimrc: &Path, timeout: std::time::Duration) -> Result<(vim::Dump, vim::Dump, Option<String>), dump::Failed> {
    let mut sandbox = dump::Sandbox::new(timeout)?;
    let home = sandbox.home.clone();
    let config = sandbox.config();
    let sandboxed = |name: &str| -> Option<String> {
        match name {
            "HOME" | "USERPROFILE" => Some(home.display().to_string()),
            "XDG_CONFIG_HOME" => Some(config.display().to_string()),
            _ => env(name),
        }
    };
    let (from, to) = (vimrc_places(env), vimrc_places(&sandboxed));
    let Some(at) = from.iter().position(|p| p == vimrc) else {
        return Err(dump::Failed::Copy(format!("{} is not where vim looks", vimrc.display())));
    };
    // The vimrc, and the directory each place is in when it is a
    // directory of vim's own (`~/.vim`, `~/.config/vim`, `vimfiles`).
    let left_out = &["undo", "swap", "backup", "view", "sessions"];
    for (a, b) in from.iter().zip(&to) {
        match a.file_name().is_some_and(|n| n == "vimrc") {
            true => sandbox.copy(a.parent().unwrap_or(a), b.parent().unwrap_or(b), left_out)?,
            false => sandbox.copy(a, b, &[])?,
        }
    }
    said_left_out(&sandbox);
    let script = ("dump.vim", dump::DUMP_VIM);
    let user = sandbox.run(program, &dump::vim_args(Some(&to[at])), script, timeout)?;
    let base = sandbox.run(program, &dump::vim_args(None), script, timeout)?;
    Ok((vim::Dump::from_json(&user.json), vim::Dump::from_json(&base.json), user.said))
}

/// **The LazyVim import**: the page's config, and the reader's own keys and
/// options when their Neovim runs; the page's config alone, and why, when
/// it does not (decision 2).
fn import_lazyvim(env: Env, nvim: &Path, sign: &Path, when: &str, beside: bool) -> Imported {
    match read_nvim("nvim", env, true, dump::TIMEOUT) {
        Ok((user, base, said)) => {
            let mut own = vim::translate(&user, &base, vim::Source::LazyVim);
            own.said = said;
            said_stopped(&own);
            lazyvim(nvim, sign, when, beside, Ok(&own))
        }
        Err(failed) => {
            let why = failed.said(false);
            println!("yumete: your own keys and options were not read: {why}");
            lazyvim(nvim, sign, when, beside, Err(&why))
        }
    }
}

// ---- the command ---------------------------------------------------------

/// The wall clock for the header: `2026-11-06 17:30`.
fn now() -> String {
    let s = yumete_core::clock::stamp();
    format!("{}-{}-{} {}:{}", &s[0..4], &s[4..6], &s[6..8], &s[8..10], &s[10..12])
}

/// **`yumete --import <which>`.** What it says is English, as the other
/// shell commands' is (`--grep`, `--export`): it is read in a terminal by
/// someone setting up, often from a script or a dotfiles README.
pub fn run(which: &str) -> ExitCode {
    let env = |name: &str| std::env::var(name).ok();
    let dir = yumete_config::config_dir();
    let (path, beside) = target(&dir);
    let when = now();
    // Before anything runs or is copied (review, 2026-11-09; see `nvim_app`).
    if matches!(which, "lazyvim" | "nvim" | "neovim") {
        if let Err(app) = nvim_app(&env) {
            eprintln!("yumete: {}. Nothing was read or written.", bad_app(&app));
            return ExitCode::FAILURE;
        }
    }
    let imported = match which {
        "helix" => {
            let Some(from) = helix_config_path(&env) else {
                eprintln!("yumete: cannot tell where helix's config is (no HOME)");
                return ExitCode::FAILURE;
            };
            let source = match std::fs::read_to_string(&from) {
                Ok(source) => source,
                Err(_) => {
                    eprintln!("yumete: no helix config at {}", from.display());
                    return ExitCode::FAILURE;
                }
            };
            let languages = std::fs::read_to_string(from.with_file_name("languages.toml")).ok();
            match helix(&source, &from, &when, beside, languages.as_deref()) {
                Ok(imported) => imported,
                Err(why) => {
                    eprintln!("yumete: {} does not parse: {why}", from.display());
                    return ExitCode::FAILURE;
                }
            }
        }
        "lazyvim" => {
            let Some(nvim) = nvim_dir(&env) else {
                eprintln!("yumete: cannot tell where Neovim's config is (no HOME)");
                return ExitCode::FAILURE;
            };
            let Some(sign) = lazyvim_sign(&nvim) else {
                eprintln!("yumete: no LazyVim in {}", nvim.display());
                eprintln!("looked for lazyvim.json, and LazyVim in lazy-lock.json and lua/config/lazy.lua");
                return ExitCode::FAILURE;
            };
            import_lazyvim(&env, &nvim, &sign, &when, beside)
        }
        "nvim" | "neovim" => {
            let Some(nvim) = nvim_dir(&env) else {
                eprintln!("yumete: cannot tell where Neovim's config is (no HOME)");
                return ExitCode::FAILURE;
            };
            // **A LazyVim config gets the LazyVim import** (decision 3).
            if let Some(sign) = lazyvim_sign(&nvim) {
                println!("yumete: this Neovim is LazyVim, so its Space menu comes too (as --import lazyvim)");
                import_lazyvim(&env, &nvim, &sign, &when, beside)
            } else {
                if !nvim.is_dir() {
                    eprintln!("yumete: no Neovim config at {}", nvim.display());
                    return ExitCode::FAILURE;
                }
                match read_nvim("nvim", &env, false, dump::TIMEOUT) {
                    Ok((user, base, said)) => {
                        let mut own = vim::translate(&user, &base, vim::Source::Neovim);
                        own.said = said;
                        said_stopped(&own);
                        vim_file("Neovim", which, &nvim, &user.version, &when, beside, &own)
                    }
                    Err(failed) => {
                        eprintln!(
                            "yumete: --import {which} runs your Neovim to read its settings, and {}. Nothing was written.",
                            failed.said(false)
                        );
                        return ExitCode::FAILURE;
                    }
                }
            }
        }
        "vim" => {
            let vimrc = match vimrc_path(&env) {
                Ok(Some(vimrc)) => vimrc,
                Ok(None) => {
                    eprintln!("yumete: no vimrc found; looked for ~/.vimrc, ~/.vim/vimrc and ~/.config/vim/vimrc");
                    return ExitCode::FAILURE;
                }
                Err(why) => {
                    eprintln!("yumete: {why}");
                    return ExitCode::FAILURE;
                }
            };
            match read_vim("vim", &env, &vimrc, dump::TIMEOUT) {
                Ok((user, base, said)) => {
                    let mut own = vim::translate(&user, &base, vim::Source::Vim);
                    own.said = said;
                    vim_file("Vim", "vim", &vimrc, &user.version, &when, beside, &own)
                }
                Err(failed) => {
                    eprintln!(
                        "yumete: --import vim runs your Vim to read its settings, and {}. Nothing was written.",
                        failed.said(true)
                    );
                    return ExitCode::FAILURE;
                }
            }
        }
        other => {
            eprintln!("yumete: --import takes helix, vim, nvim or lazyvim, not '{other}'");
            return ExitCode::from(2);
        }
    };
    if let Err(err) = std::fs::create_dir_all(&dir).and_then(|()| write(&path, &imported.text)) {
        eprintln!("yumete: cannot write {}: {err}", path.display());
        return ExitCode::FAILURE;
    }
    println!("yumete: wrote {}", path.display());
    println!(
        "{} line{} carried over; {} item{} not, listed at the end of the file",
        imported.carried,
        if imported.carried == 1 { "" } else { "s" },
        imported.skipped,
        if imported.skipped == 1 { "" } else { "s" },
    );
    if beside {
        println!("config.toml was already there, so this went beside it; copy what you want into config.toml");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary directory of this test's own.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yumete-import-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// **What the editor would say about this config**: the loader's own
    /// checks (an error stops the file, a warning drops a line) and the
    /// front end's.
    fn complaints(text: &str) -> Vec<String> {
        let mut out: Vec<String> = yumete_config::faults::faults_in(text).into_iter().map(|f| f.said).collect();
        let (config, said) = yumete_config::Config::from_toml_saying(text);
        out.extend(said);
        out.extend(yumete_tui::settings::complaints(&config));
        out
    }

    const SAMPLE: &str = r#"
theme = "catppuccin_mocha"

[editor]
scrolloff = 8
line-number = "relative"
mouse = false
bufferline = "multiple"
rulers = [80, 100]
text-width = 72

[editor.soft-wrap]
enable = true
max-wrap = 25

[editor.indent-guides]
render = true
character = "╎"
skip-levels = 1

[editor.search]
smart-case = false

[editor.lsp]
display-messages = true

[keys.normal]
C-s = ":w"
X = "extend_line"
C-d = ["half_page_down", "align_view_center"]
"C-space" = "completion"
A-x = "dap_launch"
Q = "@<C-w>v"
R = ":set line-number relative"
W = ":sp"
esc = ["collapse_selection", "keep_primary_selection"]

[keys.normal.space]
w = ":write"
q = ":quit-all"
c = { b = ":bc" }

[keys.insert]
j = { k = "normal_mode" }
"#;

    fn sample() -> Imported {
        helix(SAMPLE, Path::new("/home/me/.config/helix/config.toml"), "2026-11-06 17:30", false, Some("[[language]]\nname = \"rust\"\n")).unwrap()
    }

    #[test]
    fn helix_keys_pass_through_and_are_renamed() {
        let out = sample().text;
        let start = out.lines().position(|l| l == "[keys]").expect("a [keys] table");
        assert_eq!(out.lines().nth(start + 1), Some("preset = \"helix\""), "{out}");
        assert!(out.lines().take(start).all(|l| l.starts_with('#') || l.is_empty()), "{out}");
        // A typable alias goes in as yumete's name for the same command.
        assert!(out.contains("\nC-s = \":write\"\n"), "{out}");
        // A renamed action, in a list with one that is the same.
        assert!(out.contains("C-d = [\"page_cursor_half_down\", \"align_view_center\"]"), "{out}");
        // helix's `:sp` is a split; yumete's `:sp` is the project search.
        assert!(out.contains("W = \":hsplit\""), "{out}");
        // `:set` is yumete's settings page, so the option becomes its command.
        assert!(out.contains("R = \":line-numbers relative\""), "{out}");
        assert!(out.contains("Q = \"@C-wv\""), "{out}");
        // Groups keep their shape.
        assert!(out.contains("[keys.normal.space]\nw = \":write\"\nq = \":quit-all\"\n"), "{out}");
        assert!(out.contains("[keys.normal.space.c]\nb = \":buffer-close\"\n"), "{out}");
        assert!(out.contains("[keys.insert.j]\nk = \"normal_mode\"\n"), "{out}");
    }

    #[test]
    fn helix_lines_yumete_cannot_take_are_listed_with_a_reason() {
        let imported = sample();
        let out = &imported.text;
        let (_, list) = out.split_once("# Not carried over:\n").expect("the list");
        assert!(list.contains("[keys.normal] X = \"extend_line\": yumete has no extend_line"), "{list}");
        assert!(list.contains("[keys.normal] A-x = \"dap_launch\": yumete has no debugger"), "{list}");
        assert!(list.contains("[keys.normal] C-space: yumete cannot bind C-space"), "{list}");
        assert!(list.contains("[keys.normal] esc: yumete cannot bind esc"), "{list}");
        assert!(list.contains("[editor] mouse: no counterpart in yumete"), "{list}");
        assert!(list.contains("[editor.soft-wrap] max-wrap: no counterpart in yumete"), "{list}");
        assert!(list.contains("[editor.lsp]: no counterpart in yumete"), "{list}");
        assert!(list.contains("rulers after the first"), "{list}");
        assert!(list.contains("languages.toml [[language]]: not read"), "{list}");
        assert!(list.lines().all(|l| l.starts_with("#   ")), "{list}");
        assert_eq!(imported.skipped, list.lines().count());
        // Nothing skipped is written as a binding.
        let body = out.split("# Not carried over:").next().unwrap();
        assert!(!body.contains("dap_launch") && !body.contains("extend_line"), "{body}");
    }

    #[test]
    fn helix_options_and_theme_are_translated() {
        let out = sample().text;
        let editor = out.split("\n[editor]\n").nth(1).unwrap().split("\n\n").next().unwrap();
        for line in [
            "scrolloff = 8",
            "line_numbers = \"relative\"",
            "tabs = \"auto\"",
            "ruler = 80",
            "hard_wrap = 72",
            "soft_wrap = true",
            "indent_guides = true",
            "indent_guides_character = \"╎\"",
            "indent_guides_skip_levels = 1",
            "smart_case = false",
        ] {
            assert!(editor.lines().any(|l| l == line), "{line} missing from:\n{editor}");
        }
        assert!(out.contains("\n[theme]\nname = \"catppuccin\"\nmode = \"dark\"\n"), "{out}");
        assert_eq!(helix_theme("catppuccin_latte"), Some(("catppuccin", Some("light"))));
        assert_eq!(helix_theme("onedark"), Some(("one-dark", None)));
        assert_eq!(helix_theme("gruvbox_material_dark_hard"), None);
        assert_eq!(helix_theme("rose_pine"), None);
        let two = helix("theme = { light = \"github_light\", dark = \"github_dark\" }\n", Path::new("x"), "t", false, None)
            .unwrap()
            .text;
        assert!(two.contains("name = \"github\"\nmode = \"auto\""), "{two}");
    }

    #[test]
    fn a_helix_import_is_a_config_the_editor_takes_without_a_word() {
        let imported = sample();
        assert_eq!(complaints(&imported.text), Vec::<String>::new(), "{}", imported.text);
        // Ten settings, the theme, and nine bindings.
        assert_eq!(imported.carried, 20);
        // An empty helix config is still a clean one.
        let empty = helix("", Path::new("x"), "t", false, None).unwrap();
        assert_eq!(complaints(&empty.text), Vec::<String>::new(), "{}", empty.text);
        // Every key table: nothing yumete reports, whatever is skipped.
        let gutters = "[editor]\ngutters = [\"diagnostics\", \"spacer\"]\nline-number = \"relative\"\nscrolloff = 99\n\
                       [editor.word-completion]\nenable = false\n[keys.normal.g]\nx = \"no_such_thing\"\n";
        let out = helix(gutters, Path::new("x"), "t", true, None).unwrap().text;
        assert_eq!(complaints(&out), Vec::<String>::new(), "{out}");
        assert!(out.contains("line_numbers = \"none\""), "{out}");
        assert!(out.contains("diff_gutter = false"), "{out}");
        assert!(out.contains("word_completion = 0"), "{out}");
        assert!(out.contains("[editor] scrolloff: 99 is outside yumete's 0–20"), "{out}");
        // A group whose every key was left out is not written as an empty one.
        assert!(!out.contains("\n[keys.normal.g]\n"), "{out}");
        assert!(out.contains("# config.toml was already there"), "{out}");
        // The note on `@` runs only when one was written.
        assert!(!out.contains("A run written with @"), "{out}");
        assert!(sample().text.contains("A run written with @"));
        let skipped_run = helix("[keys.normal]\nA-x = \"@<esc>x\"\n", Path::new("x"), "t", false, None).unwrap().text;
        assert!(!skipped_run.contains("A run written with @"), "{skipped_run}");
    }

    #[test]
    fn a_helix_config_that_does_not_parse_is_refused() {
        assert!(helix("[editor\nscrolloff = 3", Path::new("x"), "t", false, None).is_err());
    }

    /// **helix's `languages.toml` gives its servers' `config`** (2026-11-07),
    /// written as whole `[[lsp.<language>]]` lists the editor takes as they
    /// are; everything else in it is listed.
    #[test]
    fn a_helix_server_s_config_comes_over_as_a_whole_list() {
        let languages = r#"
[language-server.rust-analyzer.config]
check.command = "clippy"

[language-server.superhtml]
config = { lint = false }

[language-server.clangd]
command = "/opt/llvm/bin/clangd"
config = { fallbackFlags = ["-std=c++20"] }

[language-server.jedi.config]
diagnostics = { enable = true }

[language-server.golangci-lint-lsp]
command = "golangci-lint-langserver"
config = { command = ["golangci-lint", "run"] }

[[language]]
name = "go"
language-servers = ["gopls", "golangci-lint-lsp"]
"#;
        let imported = helix("", Path::new("x"), "t", false, Some(languages)).unwrap();
        let out = &imported.text;
        assert_eq!(complaints(out), Vec::<String>::new(), "{out}");
        assert_eq!(imported.carried, 4, "rust-analyzer, superhtml, clangd, jedi");
        let config = yumete_config::Config::from_toml(out);
        assert_eq!(config.lsp["rust"].len(), 1);
        assert_eq!(config.lsp["rust"][0].config, Some(serde_json::json!({ "check": { "command": "clippy" } })));
        // Both of html's, the first keeping the factory's own `config`.
        let html = &config.lsp["html"];
        assert_eq!(html.iter().map(|s| s.command.as_str()).collect::<Vec<_>>(), ["vscode-html-language-server", "superhtml"]);
        assert_eq!(html[0].args, ["--stdio"]);
        assert_eq!(html[0].config, Some(serde_json::json!({ "provideFormatter": true })));
        assert_eq!(html[1].config, Some(serde_json::json!({ "lint": false })));
        // One server, two languages; its own command is not carried.
        for language in ["c", "cpp"] {
            assert_eq!(config.lsp[language][0].command, "clangd", "{language}");
            assert_eq!(config.lsp[language][0].config, Some(serde_json::json!({ "fallbackFlags": ["-std=c++20"] })));
        }
        // helix's `jedi` is the program `jedi-language-server`, third of four.
        assert_eq!(config.lsp["python"].len(), 4);
        assert_eq!(config.lsp["python"][2].config, Some(serde_json::json!({ "diagnostics": { "enable": true } })));
        assert_eq!(config.lsp["python"][0].config, None);
        // A language nobody configured keeps the factory's list.
        assert_eq!(config.lsp["json"], yumete_config::factory_servers()["json"]);
        let (_, list) = out.split_once("# Not carried over:\n").expect("the list");
        assert!(list.contains("[language-server.clangd] command: only config is carried over"), "{list}");
        assert!(list.contains("[language-server.golangci-lint-lsp] config: yumete starts no golangci-lint-lsp"), "{list}");
        assert!(list.contains("languages.toml [[language]]: not read"), "{list}");
        assert_eq!(imported.skipped, list.lines().count());
        // A languages.toml that does not parse is said, and the rest still comes.
        let broken = helix("[editor]\nscrolloff = 3\n", Path::new("x"), "t", false, Some("[language-server")).unwrap();
        assert!(broken.text.contains("languages.toml: does not parse"), "{}", broken.text);
        assert!(broken.text.contains("scrolloff = 3"), "{}", broken.text);
    }

    /// **Every name the tables translate to is yumete's** — the renames go
    /// stale the day yumete renames something.
    #[test]
    fn every_rename_lands_on_something_yumete_has() {
        for (helix, yumete) in HELIX_ACTION_RENAMES {
            assert!(yumete_core::actions::action(yumete).is_some(), "{helix} → {yumete}");
        }
        for (helix, yumete) in HELIX_COMMAND_RENAMES {
            assert!(yumete_command(yumete).is_some(), "{helix} → :{yumete}");
        }
        for name in ["one-dark", "github", "monokai", "dracula", "tokyo-night", "catppuccin", "gruvbox"] {
            assert!(yumete_config::ThemeConfig::named(name).is_some(), "{name}");
        }
        for (alias, line) in [("w", ":write"), ("r x.md", ":read x.md"), ("u", ":update"), ("lang rust", ":syntax rust")] {
            assert_eq!(typable(&format!(":{alias}")).as_deref(), Ok(line), "{alias}");
        }
        assert_eq!(typable(":toggle soft-wrap.enable").as_deref(), Ok(":view-wrap"));
        assert!(typable(":buffer-close-others").is_err());
        assert_eq!(typable(":set soft-wrap.enable false").as_deref(), Ok(":view-wrap off"));
        assert_eq!(typable(":set soft-wrap.enable true").as_deref(), Ok(":view-wrap on"));
    }

    #[test]
    fn helix_macros_are_spelt_the_yumete_way() {
        assert_eq!(helix_macro("miw").as_deref(), Ok("@miw"));
        assert_eq!(helix_macro("<C-w>v").as_deref(), Ok("@C-wv"));
        assert_eq!(helix_macro("x<ret><lt>").as_deref(), Ok("@x<enter><"));
        assert_eq!(helix_macro("<S-tab><A-;>").as_deref(), Ok("@<S-tab>A-;"));
        assert!(helix_macro("<esc>miw").is_err());
        assert!(helix_macro("<C-space>").is_err());
        assert!(helix_macro("C-x").is_err());
    }

    #[test]
    fn lazyvim_is_found_without_running_anything() {
        let home = scratch("lazy-home");
        let xdg = scratch("lazy-xdg");
        let env_home = |name: &str| (name == "HOME").then(|| home.display().to_string());
        let env_xdg = |name: &str| match name {
            "HOME" => Some(home.display().to_string()),
            "XDG_CONFIG_HOME" => Some(xdg.display().to_string()),
            _ => None,
        };
        let nvim = nvim_dir(&env_home).unwrap();
        assert_eq!(nvim, home.join(".config").join("nvim"));
        assert_eq!(nvim_dir(&env_xdg).unwrap(), xdg.join("nvim"));
        let app = |name: &str| match name {
            "HOME" => Some(home.display().to_string()),
            "NVIM_APPNAME" => Some("lazy".to_string()),
            _ => None,
        };
        assert_eq!(nvim_dir(&app).unwrap(), home.join(".config").join("lazy"));
        assert_eq!(helix_config_path(&env_xdg).unwrap(), xdg.join("helix").join("config.toml"));

        // A plain Neovim config is not LazyVim.
        std::fs::create_dir_all(nvim.join("lua").join("config")).unwrap();
        std::fs::write(nvim.join("init.lua"), "vim.o.number = true\n").unwrap();
        std::fs::write(nvim.join("lazy-lock.json"), "{ \"telescope.nvim\": {} }\n").unwrap();
        assert_eq!(lazyvim_sign(&nvim), None);
        // The starter's lazy.lua names it before anything is installed.
        let lazy = nvim.join("lua").join("config").join("lazy.lua");
        std::fs::write(&lazy, "spec = { { \"LazyVim/LazyVim\", import = \"lazyvim.plugins\" } }\n").unwrap();
        assert_eq!(lazyvim_sign(&nvim), Some(lazy));
        // lazyvim.json wins, and its extras are listed.
        std::fs::write(nvim.join("lazyvim.json"), "{ \"extras\": [\"lazyvim.plugins.extras.lang.python\"] }").unwrap();
        std::fs::write(nvim.join("lua").join("config").join("keymaps.lua"), "-- mine\n").unwrap();
        std::fs::create_dir_all(nvim.join("lua").join("plugins")).unwrap();
        std::fs::write(nvim.join("lua").join("plugins").join("a.lua"), "return {}\n").unwrap();
        let sign = lazyvim_sign(&nvim).unwrap();
        assert_eq!(sign, nvim.join("lazyvim.json"));

        let imported = lazyvim(&nvim, &sign, "2026-11-06 17:30", false, Err("there is no `nvim` on PATH"));
        let out = &imported.text;
        assert!(out.contains(LAZYVIM), "{out}");
        assert!(out.contains("lua/config/keymaps.lua: not read"), "{out}");
        assert!(out.contains("lua/plugins/ (1 file)"), "{out}");
        assert!(out.contains("LazyVim extra lazyvim.plugins.extras.lang.python"), "{out}");
        assert_eq!(complaints(out), Vec::<String>::new(), "{out}");
        assert!(imported.carried > 80, "{}", imported.carried);

        // The mini.surround extra lands on yumete's own surround keys, and the
        // file still reads as one config.
        std::fs::write(nvim.join("lazyvim.json"), "{ \"extras\": [\"lazyvim.plugins.extras.coding.mini-surround\"] }").unwrap();
        let with = lazyvim(&nvim, &sign, "2026-11-07 17:30", false, Err("there is no `nvim` on PATH"));
        let out = &with.text;
        assert!(out.contains("gsa = \"@ys\"\ngsd = \"@ds\"\ngsr = \"@cs\"\n"), "{out}");
        assert!(out.contains("[keys.select]\ngsa = \"surround_add\"\n"), "{out}");
        assert!(out.contains("gsf, gsF, gsh and gsn have no counterpart"), "{out}");
        assert_eq!(complaints(out), Vec::<String>::new(), "{out}");
        assert_eq!(with.carried, imported.carried + 4);
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&xdg);
    }

    #[test]
    fn an_import_never_writes_over_a_config() {
        let dir = scratch("target");
        assert_eq!(target(&dir), (dir.join("config.toml"), false));
        // The first start's one-liner is no config of the reader's.
        std::fs::write(dir.join("config.toml"), FIRST_CONFIG).unwrap();
        assert_eq!(target(&dir), (dir.join("config.toml"), false));
        write(&dir.join("config.toml"), "[keys]\npreset = \"helix\"\n").unwrap();
        assert_eq!(target(&dir), (dir.join("config.imported.toml"), true));
        assert!(write(&dir.join("config.toml"), "x").is_err(), "a config of theirs is never written over");
        std::fs::write(dir.join("config.imported.toml"), "").unwrap();
        assert_eq!(target(&dir), (dir.join("config.imported-2.toml"), true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The docs page shows this file** (2026-11-06): the TOML blocks of
    /// `from/lazyvim.md`, one after another, are [`LAZYVIM`]. Skipped when
    /// the docs site is not checked out beside this repository.
    #[test]
    fn the_lazyvim_page_shows_the_shared_config() {
        let root = match std::env::var_os("YUMETE_DOCS") {
            Some(dir) => PathBuf::from(dir),
            None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../yumete-docs"),
        };
        let page = root.join("src").join("from").join("lazyvim.md");
        let Ok(text) = std::fs::read_to_string(&page) else {
            eprintln!("skipped: no docs site at {} (set YUMETE_DOCS)", root.display());
            return;
        };
        let mut blocks = Vec::new();
        let mut inside: Option<String> = None;
        for line in text.lines() {
            match (&mut inside, line) {
                (None, "```toml") => inside = Some(String::new()),
                (Some(block), "```") => {
                    blocks.push(std::mem::take(block));
                    inside = None;
                }
                (Some(block), line) => {
                    block.push_str(line);
                    block.push('\n');
                }
                (None, _) => {}
            }
        }
        assert_eq!(blocks.join("\n"), LAZYVIM, "{} has drifted from crates/yumete-config/src/lazyvim.toml", page.display());
    }

    // ---- vim and Neovim ----------------------------------------------------

    /// A Neovim dump with these options and maps, the rest as `--clean`.
    fn nvim_dump(leader: &str, options: &str, maps: &str, extra: &str) -> vim::Dump {
        let json = format!(
            r#"{{ "nvim": "0.12.6", "leader": "{leader}", "options": {{ "number": {{ "value": false }},
               "relativenumber": {{ "value": false }}, "tabstop": {{ "value": 8 }}, "shiftwidth": {{ "value": 8 }},
               "expandtab": {{ "value": false }}, "clipboard": {{ "value": "" }}, "runtimepath": {{ "value": "a" }}
               {options} }}, "keymaps": [ {{ "mode": "n", "lhs": "Y", "rhs": "y$" }},
               {{ "mode": "n", "lhs": "grn", "rhs": "", "callback": true }} {maps} ] {extra} }}"#
        );
        vim::Dump::from_json(&serde_json::from_str(&json).unwrap())
    }

    #[test]
    fn a_neovim_import_carries_what_the_reader_changed_and_lists_the_rest() {
        let base = nvim_dump(" ", "", "", r#", "keymaps2": {}"#);
        let user = nvim_dump(
            " ",
            r#", "number": { "value": true }, "relativenumber": { "value": true }, "tabstop": { "value": 4 },
               "shiftwidth": { "value": 0 }, "clipboard": { "value": "unnamedplus" }, "runtimepath": { "value": "b" }"#,
            r#", { "mode": "n", "lhs": " ", "rhs": "" },
               { "mode": "n", "lhs": " w", "rhs": "<Cmd>write<CR>", "desc": "Save file" },
               { "mode": "n", "lhs": " ff", "rhs": "", "callback": "telescope.builtin.find_files", "desc": "Find files" },
               { "mode": "n", "lhs": " x", "rhs": "", "callback": true, "desc": "Say hi" },
               { "mode": "n", "lhs": "<C-S>", "rhs": "<Cmd>w<CR><Esc>" },
               { "mode": "n", "lhs": "K", "rhs": "", "callback": "vim.lsp.buf.hover" },
               { "mode": "v", "lhs": "<lt>", "rhs": "<lt>gv" },
               { "mode": "v", "lhs": ">", "rhs": ">gv" },
               { "mode": "n", "lhs": "<C-D>", "rhs": "<C-D>zz", "desc": "Down\u001b[1m and centre" },
               { "mode": "i", "lhs": "jk", "rhs": "<Esc>" },
               { "mode": "o", "lhs": "ie", "rhs": ":<C-U>normal! ggVG<CR>" },
               { "mode": "n", "lhs": "grn", "rhs": "", "callback": true }"#,
            r#", "colorscheme": "tokyonight-night",
               "autocmds": [ { "event": "TextYankPost", "desc": "Highlight when yanking" }, { "event": "TextYankPost" } ]"#,
        );
        let mut own = vim::translate(&user, &base, vim::Source::Neovim);
        own.said = Some("E5113: Error while calling lua chunk".to_string());
        own.stopped = true;
        let imported = vim_file("Neovim", "nvim", Path::new("/home/me/.config/nvim"), "0.12.6", "2026-11-08 12:00", false, &own);
        let out = &imported.text;
        assert_eq!(complaints(out), Vec::<String>::new(), "{out}");
        assert!(out.contains("\n[keys]\npreset = \"vim\"\n"), "{out}");
        for line in ["line_numbers = \"relative\"", "tab_width = 4", "indent_width = 4", "yank_flash = true"] {
            assert!(out.contains(line), "{line}: {out}");
        }
        assert!(out.contains("[theme]\nname = \"tokyo-night\"\nmode = \"dark\"\n"), "{out}");
        assert!(out.contains("[keys.vim.normal.space]\nw = { run = \":write\", label = \"Save file\" }\n"), "{out}");
        assert!(out.contains("[keys.vim.normal.space.f]\nf = { run = \"file_picker\", label = \"Find files\" }\n"), "{out}");
        assert!(out.contains("C-s = \":write\"\n"), "{out}");
        // `vnoremap > >gv` keeps the selection, as yumete's `indent` does
        // (review, 2026-11-09: the run `@>gv` lost it).
        assert!(out.contains("[keys.vim.select]\n\"<\" = \"unindent\"\n\">\" = \"indent\"\n"), "{out}");
        assert!(out.contains("C-d = \"@C-dzz\"  # Down [1m and centre\n"), "a control character in a comment: {out}");
        assert!(out.contains("# Your config stopped at an error"), "{out}");
        assert!(out.contains("[keys.vim.insert]\njk = \"normal_mode\"\n"), "{out}");
        assert!(out.contains("# Neovim said while starting: E5113"), "{out}");
        assert!(out.contains("# A run written with @ presses yumete's vim keys"), "{out}");
        let (_, list) = out.split_once("# Not carried over:\n").expect("the list");
        assert!(list.contains("n <leader>x (Say hi): a Lua function"), "{list}");
        assert!(list.contains("o ie: yumete has no Operator-pending table"), "{list}");
        assert!(list.contains("TextYankPost autocommand (a Lua function with no description or group): "), "{list}");
        assert!(!list.contains("<leader>w"), "it was carried: {list}");
        let (_, takes) = out.split_once("# Takes over yumete's keys:\n").expect("the keys taken");
        let takes = takes.split("\n\n").next().unwrap();
        assert_eq!(takes, "#   <leader>f: yumete's Space f (file picker (project))\n#   <leader>w: yumete's Space w (region; the group is still on C-w)");
        assert!(list.contains("options with no counterpart in yumete: clipboard\n"), "{list}");
        assert!(list.contains("1 map already does the same in yumete: n K"), "{list}");
        // The leader on <Nop> is left without a word (decision 13), and so is
        // what the baseline has too.
        assert!(!list.contains("Space key itself") && !list.contains("grn") && !list.contains("runtimepath"), "{list}");
        assert_eq!(imported.skipped, list.lines().count());
    }

    #[test]
    fn a_deleted_neovim_y_is_vim_s_again() {
        let base = nvim_dump(" ", "", "", "");
        let json = r#"{ "nvim": "0.12.6", "options": {}, "keymaps": [ { "mode": "n", "lhs": "grn", "rhs": "", "callback": true } ] }"#;
        let user = vim::Dump::from_json(&serde_json::from_str(json).unwrap());
        let own = vim::translate(&user, &base, vim::Source::Neovim);
        assert!(own.keys.contains("[keys.vim.normal]\nY = \"@yy\"\n"), "{}", own.keys);
        assert!(own.skipped.is_empty(), "{:?}", own.skipped);
        // Lua's empty table is `{}`, Vim's booleans are numbers.
        let lenient = r#"{ "vim": "9.1", "options": { "number": { "value": 1, "script": "~/.vimrc" } }, "keymaps": {} }"#;
        let dump = vim::Dump::from_json(&serde_json::from_str(lenient).unwrap());
        let none = vim::Dump::from_json(&serde_json::from_str(r#"{ "options": { "number": { "value": 0 } } }"#).unwrap());
        let own = vim::translate(&dump, &none, vim::Source::Vim);
        assert_eq!(own.editor, ["line_numbers = \"absolute\""]);
    }

    /// **The reader's LazyVim keys and options go on the page's config**
    /// (decision 2): `[editor]` lines inside the page's own, the keys under
    /// `[keys.vim]`, and a LazyVim key the reader deleted taken away.
    #[test]
    fn lazyvim_carries_the_reader_s_own_on_top() {
        assert!(LAZYVIM.trim_end().ends_with("indent_guides = true"), "the page's config ends inside [editor]");
        let base = nvim_dump(
            " ",
            r#", "scrolloff": { "value": 4 }, "shortmess": { "value": "tlTWcFoIOC" }, "statusline": { "value": " " }"#,
            r#", { "mode": "n", "lhs": " bd", "rhs": "<Cmd>bd<CR>", "desc": "Delete Buffer" }"#,
            r#", "colorscheme": "tokyonight", "autocmds": [ { "event": "TextYankPost", "group": "lazyvim_highlight_yank" } ]"#,
        );
        let user = nvim_dump(
            " ",
            r#", "scrolloff": { "value": 10 }, "shortmess": { "value": "WTlOoCtFIc" }, "statusline": { "value": "%!v:lua.x()" }"#,
            r#", { "mode": "n", "lhs": " fz", "rhs": "<Cmd>Telescope oldfiles<CR>", "desc": "Old files" }"#,
            r#", "colorscheme": "catppuccin-mocha", "autocmds": [ { "event": "TextYankPost", "group": "lazyvim_highlight_yank" } ]"#,
        );
        let own = vim::translate(&user, &base, vim::Source::LazyVim);
        let dir = scratch("lazy-own");
        std::fs::write(dir.join("lazyvim.json"), "{ \"extras\": [\"lazyvim.plugins.extras.coding.mini-surround\"] }").unwrap();
        let imported = lazyvim(&dir, &dir.join("lazyvim.json"), "t", false, Ok(&own));
        let out = &imported.text;
        assert_eq!(complaints(out), Vec::<String>::new(), "{out}");
        assert!(out.contains("indent_guides = true\nscrolloff = 10\n"), "{out}");
        assert!(out.contains("\n[theme]\nname = \"catppuccin\"\nmode = \"dark\"\n"), "{out}");
        assert!(out.contains("[keys.vim.normal.space.b]\nd = \"no_op\"\n"), "{out}");
        assert!(out.contains("[keys.vim.normal.space.f]\nz = { run = \"global_recent_file_picker\", label = \"Old files\" }\n"), "{out}");
        assert!(out.contains("[keys.select]\ngsa = \"surround_add\""), "{out}");
        assert!(!out.contains("yank_flash = true  #"), "the page sets it already: {out}");
        assert!(!out.contains("lua/config/keymaps.lua: not read"), "{out}");
        assert!(out.contains("read by running your Neovim"), "{out}");
        // Two runs of a real LazyVim gave `shortmess`'s flags in another
        // order and lualine's statusline at another moment: neither is the
        // reader's.
        assert!(!out.contains("no counterpart in yumete: "), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every file under `dir`, with its bytes and when it was written.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>, Option<std::time::SystemTime>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(at) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&at) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let meta = std::fs::symlink_metadata(&path).unwrap();
                if meta.is_dir() {
                    stack.push(path.clone());
                }
                let bytes = if meta.is_file() { std::fs::read(&path).unwrap() } else { Vec::new() };
                out.push((path, bytes, meta.modified().ok()));
            }
        }
        out.sort();
        out
    }

    /// Whether a program runs here; the tests that need it say so and pass.
    fn installed(program: &str) -> bool {
        let found = std::process::Command::new(program).arg("--version").output().is_ok();
        if !found {
            eprintln!("skipped: no {program}");
        }
        found
    }

    /// A home with a Neovim config in it, and the environment that names it.
    fn nvim_home(name: &str, init: &str) -> PathBuf {
        let home = scratch(name);
        let config = home.join(".config").join("nvim");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("init.lua"), init).unwrap();
        home
    }

    fn env_of(home: &Path) -> impl Fn(&str) -> Option<String> + '_ {
        move |name: &str| (name == "HOME").then(|| home.display().to_string())
    }

    /// **The author's hard rule** (decision 2): an import writes nothing
    /// to the reader's Neovim config, data or state, though their config
    /// and its plugins write to all three, and to `~`, as they start.
    #[test]
    fn running_neovim_never_writes_to_the_reader_s_files() {
        if !installed("nvim") {
            return;
        }
        let home = nvim_home(
            "nvim-run",
            r#"vim.g.mapleader = " "
vim.opt.number = true
vim.opt.relativenumber = true
vim.opt.tabstop = 4
vim.opt.expandtab = true
vim.keymap.set("n", "<leader>w", "<cmd>write<cr>", { desc = "Save file" })
vim.keymap.set("n", "<C-h>", "<C-w>h")
vim.keymap.del("n", "Y")
vim.api.nvim_create_autocmd("VimEnter", { callback = function()
  vim.keymap.set("n", "<leader>sd", vim.lsp.buf.definition, { desc = "Definition" })
end })
local function put(path, text)
  vim.fn.mkdir(vim.fn.fnamemodify(path, ":h"), "p")
  local f = io.open(path, "w"); f:write(text); f:close()
end
put(vim.fn.stdpath("config") .. "/lazy-lock.json", "{ \"rewritten\": true }")
put(vim.fn.stdpath("data") .. "/lazy/old-plugin/plugin/x.lua", "-- changed")
put(vim.fn.stdpath("data") .. "/lazy/new-plugin/x.lua", "")
put(vim.fn.stdpath("state") .. "/nvim/log", "x")
put(vim.fn.expand("~/written-by-the-config"), "x")
"#,
        );
        std::fs::write(home.join(".config/nvim/lazy-lock.json"), "{ \"lazy.nvim\": { \"commit\": \"abc\" } }\n").unwrap();
        let plugin = home.join(".local/share/nvim/lazy/old-plugin/plugin");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("x.lua"), "-- as installed\n").unwrap();
        let before = snapshot(&home);
        let (user, base, said) = read_nvim("nvim", &env_of(&home), false, dump::TIMEOUT).unwrap();
        assert_eq!(snapshot(&home), before, "the reader's files are as they were");
        assert_eq!(said, None);
        let own = vim::translate(&user, &base, vim::Source::Neovim);
        let out = vim_file("Neovim", "nvim", &home, &user.version, "t", false, &own).text;
        assert_eq!(complaints(&out), Vec::<String>::new(), "{out}");
        for line in [
            "line_numbers = \"relative\"",
            "tab_width = 4",
            "tab_inserts = \"spaces\"",
            "w = { run = \":write\", label = \"Save file\" }",
            "d = { run = \"goto_definition\", label = \"Definition\" }",
            "C-h = \"jump_view_left\"",
            "Y = \"@yy\"",
        ] {
            assert!(out.contains(line), "{line}: {out}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A config that waits for input is killed; one that fails part way is
    /// read as far as it got, and what Neovim said is kept.
    #[test]
    fn a_neovim_that_hangs_or_fails_is_handled() {
        if !installed("nvim") {
            return;
        }
        // It starts a job first, and a process as lazy.nvim starts git, in
        // its own session and in Neovim's group: both go with it.
        let pids = scratch("nvim-hang-pid");
        let home = nvim_home(
            "nvim-hang",
            &format!(
                "vim.opt.tabstop = 3\nvim.fn.jobstart({{ \"sh\", \"-c\", \"echo $$ > {0}/job && exec sleep 300\" }})\n\
                 vim.uv.spawn(\"sh\", {{ args = {{ \"-c\", \"echo $$ > {0}/spawned && exec sleep 300\" }} }})\n\
                 vim.uv.spawn(\"sh\", {{ args = {{ \"-c\", \"trap '' TERM; echo $$ > {0}/deaf; while true; do sleep 1; done\" }} }})\n\
                 local f = io.open(\"{0}/tempname\", \"w\"); f:write(vim.fn.tempname()); f:close()\n\
                 local name = vim.fn.input(\"name? \")\n",
                pids.display()
            ),
        );
        let started = std::time::Instant::now();
        let failed = read_nvim("nvim", &env_of(&home), false, std::time::Duration::from_secs(2)).unwrap_err();
        assert!(matches!(failed, dump::Failed::TimedOut(_)), "{failed:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(8));
        #[cfg(unix)]
        for name in ["job", "spawned", "deaf"] {
            let pid: libc::pid_t = std::fs::read_to_string(pids.join(name)).unwrap().trim().parse().unwrap();
            // Gone, once whoever adopted it has reaped it.
            let gone = (0..100).any(|_| {
                std::thread::sleep(std::time::Duration::from_millis(20));
                // SAFETY: signal 0 only asks whether the process is there.
                unsafe { libc::kill(pid, 0) != 0 }
            });
            assert!(gone, "the {name} nvim started outlived it");
        }
        // Neovim's own temporary directory went with the copy, though the
        // run was killed (review, 2026-11-09: each one was left in $TMPDIR).
        let tempname = std::fs::read_to_string(pids.join("tempname")).unwrap();
        let nvim_tmp = Path::new(&tempname).parent().unwrap();
        assert!(!nvim_tmp.exists(), "{} outlived the run", nvim_tmp.display());
        let _ = std::fs::remove_dir_all(&pids);
        let home = nvim_home("nvim-error", "vim.opt.tabstop = 3\nrequire(\"no.such.module\")\nvim.opt.shiftwidth = 5\n");
        let (user, base, said) = read_nvim("nvim", &env_of(&home), false, dump::TIMEOUT).unwrap();
        assert!(said.as_deref().is_some_and(|s| s.contains("E5113") || s.contains("no.such.module")), "{said:?}");
        let own = vim::translate(&user, &base, vim::Source::Neovim);
        assert!(own.editor.contains(&"tab_width = 3".to_string()), "{:?}", own.editor);
        // What came after the error was lost, and the file says so (review,
        // 2026-11-09).
        assert!(!own.editor.iter().any(|l| l.starts_with("indent_width")), "{:?}", own.editor);
        assert!(own.stopped);
        let out = vim_file("Neovim", "nvim", &home, &user.version, "t", false, &own).text;
        assert!(out.contains("# Your config stopped at an error, so what it sets after the error was\n"), "{out}");
        assert_eq!(complaints(&out), Vec::<String>::new(), "{out}");
        assert!(matches!(
            read_nvim("yumete-no-such-editor", &env_of(&home), false, dump::TIMEOUT),
            Err(dump::Failed::NotInstalled)
        ));
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(scratch("nvim-hang"));
    }

    #[test]
    fn running_vim_never_writes_to_the_reader_s_files() {
        if !installed("vim") {
            return;
        }
        let home = scratch("vim-run");
        std::fs::write(
            home.join(".vimrc"),
            "let mapleader = \",\"\nset number tabstop=4 shiftwidth=2 expandtab\nnnoremap <leader>w :w<CR>\n\
             nnoremap Y y$\ninoremap jk <Esc>\ncall writefile(['x'], expand('~/.vim/written-by-the-vimrc'))\n",
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".vim/plugin")).unwrap();
        std::fs::write(home.join(".vim/plugin/p.vim"), "nnoremap <leader>p :echo 'p'<CR>\n").unwrap();
        let env = env_of(&home);
        let vimrc = vimrc_path(&env).unwrap().unwrap();
        assert_eq!(vimrc, home.join(".vimrc"));
        let before = snapshot(&home);
        let (user, base, _) = read_vim("vim", &env, &vimrc, dump::TIMEOUT).unwrap();
        assert_eq!(snapshot(&home), before, "the reader's files are as they were");
        let own = vim::translate(&user, &base, vim::Source::Vim);
        let out = vim_file("Vim", "vim", &vimrc, &user.version, "t", false, &own).text;
        assert_eq!(complaints(&out), Vec::<String>::new(), "{out}");
        for line in ["line_numbers = \"absolute\"", "indent_width = 2", "\",w\" = \":write\"", "jk = \"normal_mode\""] {
            assert!(out.contains(line), "{line}: {out}");
        }
        assert!(out.contains("n <leader>p, p.vim:1: yumete has no :echo 'p'"), "{out}");
        assert!(out.contains("already does the same in yumete: n Y"), "{out}");
        // `Y` was a run, but not one written.
        assert!(!out.contains("A run written with @"), "{out}");
        let viminit = |name: &str| match name {
            "VIMINIT" => Some("set nu".to_string()),
            _ => env(name),
        };
        assert!(vimrc_path(&viminit).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// **A link round the config is not a tree without end** (review,
    /// 2026-11-09: `lua/loop/{a,b} -> .` made 357,928 directories in 30 s).
    /// A link to a directory the copy is in comes over as a link into the
    /// copy; one to a directory outside the config is left out and named;
    /// one to a file is copied; and the config still runs.
    #[test]
    fn a_link_round_the_config_is_copied_once() {
        let home = nvim_home("nvim-loop", "vim.opt.tabstop = 3\n");
        let config = home.join(".config/nvim");
        let elsewhere = home.join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("big")).unwrap();
        std::fs::write(elsewhere.join("big/x.lua"), "-- not the config's\n").unwrap();
        std::fs::write(elsewhere.join("stowed.lua"), "vim.g.stowed = 1\n").unwrap();
        std::fs::create_dir_all(config.join("lua/loop")).unwrap();
        std::fs::write(config.join("lua/loop/init.lua"), "return 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(".", config.join("lua/loop/a")).unwrap();
            symlink(".", config.join("lua/loop/b")).unwrap();
            symlink("../..", config.join("lua/loop/up")).unwrap();
            symlink(elsewhere.join("big"), config.join("lua/big")).unwrap();
            symlink(elsewhere.join("stowed.lua"), config.join("lua/stowed.lua")).unwrap();
        }
        let before = snapshot(&home);
        let started = std::time::Instant::now();
        let mut sandbox = dump::Sandbox::new(dump::TIMEOUT).unwrap();
        let to = sandbox.config().join("nvim");
        sandbox.copy(&config, &to, &[]).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
        assert!(to.join("lua/loop/init.lua").is_file());
        #[cfg(unix)]
        {
            for (link, points_to) in [("lua/loop/a", "lua/loop"), ("lua/loop/b", "lua/loop"), ("lua/loop/up", "")] {
                let meta = std::fs::symlink_metadata(to.join(link)).unwrap();
                assert!(meta.file_type().is_symlink(), "{link} is made again as a link");
                let target = std::fs::read_link(to.join(link)).unwrap();
                assert_eq!(target.components().collect::<Vec<_>>(), to.join(points_to).components().collect::<Vec<_>>(), "{link}");
            }
            assert!(to.join("lua/loop/a/a/b/init.lua").is_file(), "the link still reads as it did");
            assert!(!to.join("lua/big").exists(), "a directory outside the config is left out");
            assert_eq!(sandbox.left_out.len(), 1, "{:?}", sandbox.left_out);
            assert!(sandbox.left_out[0].contains("lua/big"), "{:?}", sandbox.left_out);
            let stowed = to.join("lua/stowed.lua");
            assert!(!std::fs::symlink_metadata(&stowed).unwrap().file_type().is_symlink(), "a file is copied");
            assert_eq!(std::fs::read_to_string(&stowed).unwrap(), "vim.g.stowed = 1\n");
        }
        drop(sandbox);
        // Out of time: it says so, and how far it got.
        let mut sandbox = dump::Sandbox::new(std::time::Duration::ZERO).unwrap();
        let to = sandbox.config().join("nvim");
        let Err(dump::Failed::Copy(why)) = sandbox.copy(&config, &to, &[]) else { panic!("a copy out of time") };
        assert!(why.starts_with("gave up copying ") && why.contains("more than 0 s"), "{why}");
        drop(sandbox);
        if installed("nvim") {
            let (user, base, _) = read_nvim("nvim", &env_of(&home), false, dump::TIMEOUT).unwrap();
            let own = vim::translate(&user, &base, vim::Source::Neovim);
            assert!(own.editor.contains(&"tab_width = 3".to_string()), "{:?}", own.editor);
        }
        assert_eq!(snapshot(&home), before, "the reader's files are as they were");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// **git stays in the copy** (review, 2026-11-09): with the temporary
    /// directory inside a repository, a config's `git config` in its own
    /// directory, which has no `.git` in the copy, wrote the outer
    /// repository's `.git/config`.
    #[test]
    fn git_run_by_the_config_stays_in_the_copy() {
        if !installed("nvim") || !installed("git") {
            return;
        }
        let repo = scratch("nvim-git");
        let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
        assert!(git(&["init", "-q"]).status.success());
        let before = std::fs::read_to_string(repo.join(".git/config")).unwrap();
        let mut sandbox = dump::Sandbox::new_in(&repo, dump::TIMEOUT).unwrap();
        let config = sandbox.config().join("nvim");
        std::fs::create_dir_all(&config).unwrap();
        let wrote = repo.join("git-said");
        std::fs::write(
            config.join("init.lua"),
            format!(
                "local said = vim.fn.system({{ \"git\", \"-C\", vim.fn.stdpath(\"config\"), \"config\", \"user.name\", \"WRITTEN-BY-IMPORT\" }})\n\
                 local f = io.open(\"{}\", \"w\"); f:write(said); f:close()\n",
                wrote.display()
            ),
        )
        .unwrap();
        let args = dump::nvim_args(dump::Nvim::Own, &sandbox);
        sandbox.run("nvim", &args, ("dump.lua", dump::DUMP_LUA), dump::TIMEOUT).unwrap();
        drop(sandbox);
        assert!(wrote.is_file(), "the config ran git");
        assert_eq!(std::fs::read_to_string(repo.join(".git/config")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&repo);
    }

    /// **A detached job goes with the run** (review, 2026-11-09): Neovim's
    /// `jobstart(…, {detach = true})` leaves the editor's group, and wrote
    /// into the copy 6 s after the import had returned.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn a_detached_job_does_not_outlive_the_import() {
        if !installed("nvim") {
            return;
        }
        let pids = scratch("nvim-detach-pid");
        let pid_file = pids.join("job");
        let home = nvim_home(
            "nvim-detach",
            &format!(
                "vim.opt.tabstop = 3\n\
                 vim.fn.jobstart({{ \"sh\", \"-c\", \"echo $$ > {0} && exec sleep 300\" }}, {{ detach = true }})\n\
                 vim.wait(5000, function() return vim.uv.fs_stat(\"{0}\") ~= nil end, 20)\n",
                pid_file.display()
            ),
        );
        let (user, base, _) = read_nvim("nvim", &env_of(&home), false, dump::TIMEOUT).unwrap();
        let own = vim::translate(&user, &base, vim::Source::Neovim);
        assert!(own.editor.contains(&"tab_width = 3".to_string()), "{:?}", own.editor);
        let pid: libc::pid_t = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        let gone = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            // SAFETY: signal 0 only asks whether the process is there.
            unsafe { libc::kill(pid, 0) != 0 }
        });
        if !gone {
            // SAFETY: as above; it must not outlive the test either.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        assert!(gone, "the detached job outlived the import");
        let _ = std::fs::remove_dir_all(&pids);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// **A `$NVIM_APPNAME` Neovim refuses is refused before anything runs,
    /// and a copy writes nowhere but into its sandbox** (review, 2026-11-09):
    /// `NVIM_APPNAME=/abs/myconf` made the copy's target the config itself,
    /// and every file of it was emptied.
    #[test]
    fn a_copy_never_writes_to_the_config_it_copies() {
        let app = |v: &str| {
            let v = v.to_string();
            nvim_app(&move |n: &str| (n == "NVIM_APPNAME").then(|| v.clone()))
        };
        assert_eq!(app(""), Ok("nvim".to_string()));
        for good in ["lazy", "foo/bar", "./x", "a..b"] {
            assert_eq!(app(good), Ok(good.to_string()));
        }
        for bad in ["/abs/myconf", "..", "../nvim", "a/../b", ".", "~/.config/nvim", "~x"] {
            assert_eq!(app(bad), Err(bad.to_string()));
        }
        let home = nvim_home("nvim-appname", "vim.opt.tabstop = 3\n");
        let config = home.join(".config/nvim");
        std::fs::create_dir_all(config.join("lua")).unwrap();
        std::fs::write(config.join("lua/x.lua"), "return 1\n").unwrap();
        // Nor into a sandbox the config holds (made there, it changes the
        // directory's time, so before the snapshot).
        let mut sandbox = dump::Sandbox::new_in(&config, dump::TIMEOUT).unwrap();
        let to = sandbox.config().join("nvim");
        let failed = sandbox.copy(&config, &to, &[]).unwrap_err();
        assert!(matches!(&failed, dump::Failed::Copy(why) if why.contains("holds the import's own directory")), "{failed:?}");
        drop(sandbox);
        let before = snapshot(&home);
        for name in [config.display().to_string(), "../.config/nvim".to_string()] {
            let env = |n: &str| match n {
                "HOME" => Some(home.display().to_string()),
                "NVIM_APPNAME" => Some(name.clone()),
                _ => None,
            };
            assert_eq!(nvim_dir(&env), None);
            let failed = read_nvim("nvim", &env, false, dump::TIMEOUT).unwrap_err();
            assert!(matches!(&failed, dump::Failed::Copy(why) if why.contains("$NVIM_APPNAME")), "{failed:?}");
        }
        // The second guard, in the copy itself: a target outside the
        // sandbox, by name, through `..` or through a link, is refused.
        let mut sandbox = dump::Sandbox::new(dump::TIMEOUT).unwrap();
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut outside = vec![config.clone(), sandbox.config().join("../../../elsewhere")];
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&sandbox.root).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "the sandbox is the reader's alone");
            std::os::unix::fs::symlink(home.join(".config"), sandbox.config().join("out")).unwrap();
            outside.push(sandbox.config().join("out/nvim"));
        }
        for to in outside {
            let failed = sandbox.copy(&config, &to, &[]).unwrap_err();
            assert!(matches!(&failed, dump::Failed::Copy(why) if why.contains("not inside")), "{}: {failed:?}", to.display());
        }
        // A file is never copied onto itself: a hard link to it inside the
        // sandbox is the same file.
        let to = sandbox.config().join("nvim");
        std::fs::create_dir_all(&to).unwrap();
        std::fs::hard_link(config.join("init.lua"), to.join("init.lua")).unwrap();
        let failed = sandbox.copy(&config, &to, &[]).unwrap_err();
        assert!(matches!(&failed, dump::Failed::Copy(why) if why.contains("onto itself")), "{failed:?}");
        drop(sandbox);
        assert_eq!(snapshot(&home), before, "the reader's files are as they were");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// **A LazyVim reader's own "off" beats the page's "on"** (review,
    /// 2026-11-09): the page's `yank_flash = true` and `indent_guides = true`
    /// won over a config that took both away.
    #[test]
    fn a_lazyvim_reader_s_own_off_is_kept() {
        let base = nvim_dump(" ", "", "", r#", "autocmds": [ { "event": "TextYankPost", "group": "lazyvim_highlight_yank" } ], "loaded": ["ibl"]"#);
        let user = nvim_dump(" ", "", "", "");
        let own = vim::translate(&user, &base, vim::Source::LazyVim);
        let dir = scratch("lazy-off");
        let imported = lazyvim(&dir, &dir.join("lazyvim.json"), "t", false, Ok(&own));
        let out = &imported.text;
        assert_eq!(complaints(out), Vec::<String>::new(), "{out}");
        assert!(out.contains("\n[editor]\nyank_flash = false  # your config took the flash on yank away\nindent_guides = false  #"), "{out}");
        assert!(!out.contains("yank_flash = true") && !out.contains("indent_guides = true"), "{out}");
        // A value the page has already is the page's line, counted once.
        let same = vim::translate(&base, &user, vim::Source::LazyVim);
        let again = lazyvim(&dir, &dir.join("lazyvim.json"), "t", false, Ok(&same));
        assert!(again.text.contains("\n[editor]\nyank_flash = true\nindent_guides = true\n"), "{}", again.text);
        let plain = lazyvim(&dir, &dir.join("lazyvim.json"), "t", false, Ok(&vim::Own::default()));
        assert_eq!(again.carried, plain.carried);
        assert_eq!(imported.carried, plain.carried);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Odd bytes in what Neovim says, and a Neovim too old** (review,
    /// 2026-11-09): a description with a byte that is not UTF-8 failed the
    /// whole import as "stopped before it could be read", and a Neovim older
    /// than 0.7 waited out the time limit instead of saying it was too old.
    #[test]
    fn a_neovim_that_says_odd_things_is_read() {
        if !installed("nvim") {
            return;
        }
        let home = nvim_home(
            "nvim-bytes",
            "vim.g.mapleader = \",\"\n\
             vim.keymap.set(\"n\", \"<leader>z\", \"<Cmd>w<CR>\", { desc = \"caf\\233\" })\n\
             vim.keymap.set(\"n\", \"<leader>y\", \"<Cmd>q<CR>\", { desc = \"quit\\27[1m now\" })\n",
        );
        let (user, base, _) = read_nvim("nvim", &env_of(&home), false, dump::TIMEOUT).unwrap();
        let own = vim::translate(&user, &base, vim::Source::Neovim);
        let out = vim_file("Neovim", "nvim", &home, &user.version, "t", false, &own).text;
        assert_eq!(complaints(&out), Vec::<String>::new(), "{out}");
        assert!(out.contains("\",z\" = \":write\"  # caf\u{FFFD}\n") && out.contains("\",y\" = \":quit\"  # quit [1m now\n"), "{out}");
        let _ = std::fs::remove_dir_all(&home);
        // An old Neovim, played by a config that takes away what 0.7 added.
        let home = nvim_home(
            "nvim-old",
            "local has = vim.fn.has\n\
             vim.fn.has = function(f) if f == \"nvim-0.9\" then return 0 end return has(f) end\n\
             vim.api.nvim_create_autocmd = nil\n",
        );
        let started = std::time::Instant::now();
        let failed = read_nvim("nvim", &env_of(&home), false, dump::TIMEOUT).unwrap_err();
        assert!(matches!(&failed, dump::Failed::TooOld(v) if v.starts_with("0.") && !v.contains(char::is_whitespace)), "{failed:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_mark_is_one_whole_string_of_an_environment() {
        let needle = b"YUMETE_IMPORT_RUN=/t/r\0";
        assert!(dump::carries(b"A=1\0YUMETE_IMPORT_RUN=/t/r\0B=2\0", needle));
        assert!(dump::carries(b"YUMETE_IMPORT_RUN=/t/r\0", needle));
        assert!(!dump::carries(b"XYUMETE_IMPORT_RUN=/t/r\0", needle));
        assert!(!dump::carries(b"YUMETE_IMPORT_RUN=/t/r/2\0", needle));
    }
}
