//! **`yumete --import helix` and `yumete --import lazyvim`** (2026-11-06):
//! another editor's configuration, written once into yumete's `config.toml`.
//!
//! Decided that day, after the survey in `local/survey-0.20/import.md`:
//!
//! - **Nothing runs.** helix's `config.toml` is read as TOML; LazyVim is only
//!   *recognised* (`lazyvim.json`, `lazy-lock.json`, `lua/config/lazy.lua`),
//!   and what is written is the config on the docs site's LazyVim page. The
//!   reader's own Lua is not read: importing vim and Neovim settings needs
//!   Neovim itself to say what they are, and that is a later version's work.
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
pub fn nvim_dir(env: Env) -> Option<PathBuf> {
    let app = env("NVIM_APPNAME").filter(|v| !v.is_empty()).unwrap_or_else(|| "nvim".to_string());
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

/// The header's first lines, shared by both imports. `from` is the file
/// read (helix) or the file that showed LazyVim is there.
fn header(what: &str, from: &Path, when: &str, beside: bool) -> String {
    let label = match what {
        "LazyVim" => "LazyVim found in",
        _ => "Source",
    };
    let mut out = format!(
        "# Imported from {what} by `yumete --import {}` on {when}.\n# {label}: {}\n",
        what.to_ascii_lowercase(),
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

/// A comment cannot hold a line break; a path or a value might.
fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
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
        (true, Some(("soft-wrap", "enable")), Some("true")) => ":view-wrap on".to_string(),
        (true, Some(("soft-wrap", "enable")), Some("false")) => ":view-wrap off".to_string(),
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
/// `languages` says whether a `languages.toml` sits beside it, which this
/// does not read: yumete's language servers are a candidate list each
/// (`[lsp.<language>]`), and helix's are written for all of them at once.
pub fn helix(source: &str, from: &Path, when: &str, beside: bool, languages: bool) -> Result<Imported, String> {
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
    if languages {
        got.skipped.push(
            "languages.toml: not read; a language server is set under [lsp.<language>] (see Language servers)"
                .to_string(),
        );
    }
    let mut text = header("helix", from, when, beside);
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
    text.push_str(&not_carried(&got.skipped));
    Ok(Imported { text, carried: got.carried, skipped: got.skipped.len() })
}

// ---- LazyVim -------------------------------------------------------------

/// **LazyVim's keys**: the shared config, and the list of what in `nvim` it
/// leaves alone. `sign` is the file that said LazyVim is there.
pub fn lazyvim(nvim: &Path, sign: &Path, when: &str, beside: bool) -> Imported {
    let mut skipped = Vec::new();
    for file in ["lua/config/keymaps.lua", "lua/config/options.lua", "lua/config/autocmds.lua"] {
        if nvim.join(file).is_file() {
            skipped.push(format!(
                "{file}: not read; yumete runs no Lua, and reading Neovim's own settings is for a later version"
            ));
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
    let mut text = header("LazyVim", sign, when, beside);
    text.push_str(&format!(
        "# The config on the Coming from LazyVim page: {LAZYVIM_PAGE}\n\
         # Nothing in your Neovim config was run.\n\n"
    ));
    let config = match surround {
        true => with_mini_surround(LAZYVIM),
        false => LAZYVIM.to_string(),
    };
    text.push_str(&config);
    text.push_str(&not_carried(&skipped));
    // Every line that binds or sets something; a group's `label` names one.
    let carried = config
        .lines()
        .map(str::trim_start)
        .filter(|l| !l.starts_with('#') && !l.starts_with('[') && l.contains(" = ") && !l.starts_with("label "))
        .count();
    Imported { text, carried, skipped: skipped.len() }
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

/// A value on one line, for the list of what was not carried over.
fn inline(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => quote(s),
        other => one_line(&other.to_string()),
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
            let languages = from.with_file_name("languages.toml").is_file();
            match helix(&source, &from, &when, beside, languages) {
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
            lazyvim(&nvim, &sign, &when, beside)
        }
        "vim" | "nvim" | "neovim" => {
            eprintln!("yumete: importing vim and Neovim settings is not there yet; --import takes helix or lazyvim");
            return ExitCode::from(2);
        }
        other => {
            eprintln!("yumete: --import takes helix or lazyvim, not '{other}'");
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
        helix(SAMPLE, Path::new("/home/me/.config/helix/config.toml"), "2026-11-06 17:30", false, true).unwrap()
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
        assert!(list.contains("languages.toml: not read"), "{list}");
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
            "smart_case = false",
        ] {
            assert!(editor.lines().any(|l| l == line), "{line} missing from:\n{editor}");
        }
        assert!(out.contains("\n[theme]\nname = \"catppuccin\"\nmode = \"dark\"\n"), "{out}");
        assert_eq!(helix_theme("catppuccin_latte"), Some(("catppuccin", Some("light"))));
        assert_eq!(helix_theme("onedark"), Some(("one-dark", None)));
        assert_eq!(helix_theme("gruvbox_material_dark_hard"), None);
        assert_eq!(helix_theme("rose_pine"), None);
        let two = helix("theme = { light = \"github_light\", dark = \"github_dark\" }\n", Path::new("x"), "t", false, false)
            .unwrap()
            .text;
        assert!(two.contains("name = \"github\"\nmode = \"auto\""), "{two}");
    }

    #[test]
    fn a_helix_import_is_a_config_the_editor_takes_without_a_word() {
        let imported = sample();
        assert_eq!(complaints(&imported.text), Vec::<String>::new(), "{}", imported.text);
        // Seven settings, the theme, and nine bindings.
        assert_eq!(imported.carried, 17);
        // An empty helix config is still a clean one.
        let empty = helix("", Path::new("x"), "t", false, false).unwrap();
        assert_eq!(complaints(&empty.text), Vec::<String>::new(), "{}", empty.text);
        // Every key table: nothing yumete reports, whatever is skipped.
        let gutters = "[editor]\ngutters = [\"diagnostics\", \"spacer\"]\nline-number = \"relative\"\nscrolloff = 99\n\
                       [editor.word-completion]\nenable = false\n[keys.normal.g]\nx = \"no_such_thing\"\n";
        let out = helix(gutters, Path::new("x"), "t", true, false).unwrap().text;
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
        let skipped_run = helix("[keys.normal]\nA-x = \"@<esc>x\"\n", Path::new("x"), "t", false, false).unwrap().text;
        assert!(!skipped_run.contains("A run written with @"), "{skipped_run}");
    }

    #[test]
    fn a_helix_config_that_does_not_parse_is_refused() {
        assert!(helix("[editor\nscrolloff = 3", Path::new("x"), "t", false, false).is_err());
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
        assert_eq!(typable(":set soft-wrap.enable false").as_deref(), Ok(":view-wrap off"));
        assert!(typable(":buffer-close-others").is_err());
        assert!(typable(":toggle soft-wrap.enable").is_err());
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

        let imported = lazyvim(&nvim, &sign, "2026-11-06 17:30", false);
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
        let with = lazyvim(&nvim, &sign, "2026-11-07 17:30", false);
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
}
