//! **vim's and Neovim's settings and keys, as yumete's**,
//! from what the editor itself said it had ([`super::dump`]).
//!
//! Each run is read against a baseline run of the same editor with nothing
//! of the reader's (`nvim --clean`, `vim -u NONE`, LazyVim alone), so what
//! is left is what the reader changed (decision 4). The rule for the rest:
//! **carry only what is certain; less is better than wrong**.
//! Whatever is not certain goes to the list at the end, one line a map.
//!
//! - **Maps go to `[keys.vim.normal|select|insert]`** (decision 1): a run
//!   written with `@` presses the keys of the keymap in force, and these are
//!   vim's.
//! - **A right side is carried when it says one thing yumete has**: a key
//!   run, an Ex line with a known counterpart, a plugin command or a Lua
//!   function from the tables below. A Lua function of the reader's own,
//!   an expression or a `<Plug>` key is listed.
//! - **The Space menu stays** (decision 13): the leader on `<Nop>` is
//!   skipped without a word, and a map of the Space key itself is listed.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{one_line, quote, toml_key, THEME_NOT_CARRIED};

/// Which editor a dump came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Source {
    Neovim,
    Vim,
    LazyVim,
}

/// One map, as the dump gave it.
#[derive(Debug, Clone)]
pub(super) struct Map {
    mode: String,
    lhs: String,
    rhs: String,
    desc: Option<String>,
    /// A known function's name, or `""` for any other function.
    callback: Option<String>,
    expr: bool,
    script: Option<String>,
    line: i64,
}

/// One autocommand, as the dump gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Autocmd {
    event: String,
    pattern: String,
    desc: String,
    group: String,
    command: String,
}

/// **What one run said**, read leniently: Lua's empty table is `{}` and
/// Vim's booleans are numbers.
#[derive(Debug, Default)]
pub(super) struct Dump {
    pub version: String,
    /// A Lua file or function of the config stopped at an error
    /// ([`super::dump::Dumped::stopped`]).
    pub stopped: bool,
    leader: Option<String>,
    colorscheme: Option<String>,
    /// Vim's `$VIMRUNTIME`: options and maps set from under it are Vim's.
    runtime: Option<String>,
    options: BTreeMap<String, (Value, Option<String>)>,
    maps: Vec<Map>,
    autocmds: Vec<Autocmd>,
    loaded: Vec<String>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn items<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(Value::as_array).into_iter().flatten()
}

/// A Lua boolean or a Vim number.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().is_some_and(|n| n != 0),
        _ => false,
    }
}

impl Dump {
    pub(super) fn from_json(json: &Value) -> Dump {
        let options = json
            .get("options")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .map(|(name, opt)| {
                        (name.clone(), (opt.get("value").cloned().unwrap_or(Value::Null), text(opt, "script")))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let maps = items(json, "keymaps")
            .map(|m| Map {
                mode: text(m, "mode").unwrap_or_default(),
                lhs: text(m, "lhs").unwrap_or_default(),
                rhs: text(m, "rhs").unwrap_or_default(),
                desc: text(m, "desc").filter(|d| !d.is_empty()),
                callback: match m.get("callback") {
                    Some(Value::String(name)) => Some(name.clone()),
                    Some(v) if truthy(v) => Some(String::new()),
                    _ => None,
                },
                expr: m.get("expr").is_some_and(truthy),
                script: text(m, "script"),
                line: m.get("line").and_then(Value::as_i64).unwrap_or(0),
            })
            .collect();
        let autocmds = items(json, "autocmds")
            .map(|a| Autocmd {
                event: text(a, "event").unwrap_or_default(),
                pattern: text(a, "pattern").unwrap_or_default(),
                desc: text(a, "desc").unwrap_or_default(),
                group: text(a, "group").unwrap_or_default(),
                command: text(a, "command").unwrap_or_default(),
            })
            .collect();
        Dump {
            version: text(json, "nvim").or_else(|| text(json, "vim")).unwrap_or_default(),
            stopped: false,
            leader: text(json, "leader"),
            colorscheme: text(json, "colorscheme"),
            runtime: text(json, "runtime").filter(|r| !r.is_empty()),
            options,
            maps,
            autocmds,
            loaded: items(json, "loaded").filter_map(|v| v.as_str().map(str::to_string)).collect(),
        }
    }

    fn value(&self, name: &str) -> Option<&Value> {
        self.options.get(name).map(|(v, _)| v)
    }

    fn number(&self, name: &str) -> i64 {
        self.value(name).and_then(Value::as_i64).unwrap_or(0)
    }

    fn on(&self, name: &str) -> bool {
        self.value(name).is_some_and(truthy)
    }

    fn string(&self, name: &str) -> String {
        self.value(name).and_then(Value::as_str).unwrap_or("").to_string()
    }

    /// Set from a file of Vim's own (`$VIMRUNTIME`), or from nowhere.
    fn vims_own(&self, script: Option<&str>) -> bool {
        match (script, &self.runtime) {
            (None, _) => true,
            (Some(script), Some(runtime)) => script.starts_with(runtime.as_str()),
            (Some(_), None) => false,
        }
    }
}

/// What a translation gathered, before it is written out.
#[derive(Debug, Default)]
pub(super) struct Own {
    /// `[editor]` lines, `key = value`, a comment after some.
    pub editor: Vec<String>,
    /// The `[keys.vim.…]` sections, ready to write.
    pub keys: String,
    pub skipped: Vec<String>,
    /// Keys of yumete's that an imported map took, with what they did.
    pub takes: Vec<String>,
    pub carried: usize,
    /// A run written with `@`.
    pub runs: bool,
    /// The editor's first word on stderr while starting.
    pub said: Option<String>,
    /// The config stopped part way at an error ([`Dump::stopped`]).
    pub stopped: bool,
}

// ---- options -------------------------------------------------------------

/// Options that differ between two runs for reasons not the reader's, or
/// that the theme speaks for: never listed.
const NOISE: &[&str] = &[
    "runtimepath", "packpath", "loadplugins", "shadafile", "shada", "viminfo", "viminfofile", "updatecount",
    "scroll", "helplang", "grepformat", "grepprg", "backupdir", "directory", "viewdir", "undodir", "maxmem",
    "maxmemtot", "syntax", "filetype", "modified", "buftype", "background", "encoding", "compatible", "cpoptions",
    "lines", "columns", "window", "term", "ttytype", "termencoding", "fileformat", "modelineexpr",
    // Drawn by plugins (lualine, bufferline) on a timer of their own, so two
    // runs of one config can differ (as on a real LazyVim).
    "statusline", "tabline", "winbar", "statuscolumn",
];

/// Options whose value is a set of one-letter flags: `shortmess` can read
/// `WTlOoCtFIc` from one run and `tlTWcFoIOC` from the other (real LazyVim).
const FLAGS: &[&str] = &["shortmess", "cpoptions", "formatoptions", "mouse", "guioptions", "complete"];

/// Whether two values of `name` say the same: lists compared as sets.
fn same(name: &str, a: &Value, b: &Value) -> bool {
    match (a.as_str(), b.as_str()) {
        (Some(a), Some(b)) if FLAGS.contains(&name) => {
            a.chars().collect::<BTreeSet<_>>() == b.chars().collect::<BTreeSet<_>>()
        }
        (Some(a), Some(b)) if a.contains(',') || b.contains(',') => {
            a.split(',').collect::<BTreeSet<_>>() == b.split(',').collect::<BTreeSet<_>>()
        }
        _ => a == b,
    }
}

/// **The options yumete has**, from what the reader changed.
fn options(user: &Dump, base: &Dump, from: Source, own: &mut Own) {
    let changed = |name: &str| -> bool {
        let Some((value, script)) = user.options.get(name) else { return false };
        if from == Source::Vim && user.vims_own(script.as_deref()) {
            return false;
        }
        !base.value(name).is_some_and(|b| same(name, value, b))
    };
    let mut handled: BTreeSet<&str> = BTreeSet::new();
    let set = |own: &mut Own, line: String| {
        own.editor.push(line);
        own.carried += 1;
    };
    // Line numbers: what the two say, when they differ.
    handled.extend(["number", "relativenumber"]);
    if changed("number") || changed("relativenumber") {
        let how = |d: &Dump| match (d.on("number"), d.on("relativenumber")) {
            (_, true) => "relative",
            (true, false) => "absolute",
            (false, false) => "none",
        };
        if how(user) != how(base) {
            set(own, format!("line_numbers = {}", quote(how(user))));
        }
    }
    let ranged = |own: &mut Own, name: &str, key: &str, n: i64, low: i64, high: i64| match (low..=high).contains(&n) {
        true => {
            own.editor.push(format!("{key} = {n}"));
            own.carried += 1;
        }
        false => own.skipped.push(format!("{name}={n}: outside yumete's {low}–{high} for {key}")),
    };
    handled.extend(["tabstop", "shiftwidth", "expandtab", "textwidth", "scrolloff", "pumheight"]);
    if changed("tabstop") {
        ranged(own, "tabstop", "tab_width", user.number("tabstop"), 1, 16);
    }
    // `shiftwidth=0` is the tab stop.
    let sw = user.number("shiftwidth");
    if changed("shiftwidth") || (sw == 0 && changed("tabstop")) {
        let width = if sw == 0 { user.number("tabstop") } else { sw };
        ranged(own, "shiftwidth", "indent_width", width, 1, 16);
    }
    if changed("expandtab") {
        set(own, format!("tab_inserts = {}", quote(if user.on("expandtab") { "spaces" } else { "tab" })));
    }
    let tw = user.number("textwidth");
    if changed("textwidth") && tw > 0 {
        ranged(own, "textwidth", "hard_wrap", tw, 1, 400);
    }
    if changed("scrolloff") {
        ranged(own, "scrolloff", "scrolloff", user.number("scrolloff"), 0, 20);
    }
    if changed("pumheight") && user.number("pumheight") > 0 {
        ranged(own, "pumheight", "completion_page_size", user.number("pumheight"), 1, 20);
    }
    // colorcolumn: the first is the ruler; `+N` counts from textwidth.
    handled.insert("colorcolumn");
    if changed("colorcolumn") {
        let columns = user.string("colorcolumn");
        let mut all = columns.split(',').filter(|c| !c.is_empty());
        if let Some(first) = all.next() {
            let at = match first.strip_prefix('+').or(first.strip_prefix('-').map(|_| first)) {
                Some(rel) if tw > 0 => rel.parse::<i64>().ok().map(|n| tw + n),
                Some(_) => None,
                None => first.parse::<i64>().ok(),
            };
            match at {
                Some(n) => ranged(own, "colorcolumn", "ruler", n, 1, 400),
                None => own.skipped.push(format!("colorcolumn={columns}: yumete's ruler is one column number")),
            }
            if all.next().is_some() {
                own.skipped.push(format!("colorcolumn={columns}: yumete draws one ruler, the first"));
            }
        }
    }
    for (name, key, yes, no) in [
        ("wrap", "soft_wrap", "true", "false"),
        ("fixendofline", "insert_final_newline", "true", "false"),
        ("autoread", "reload_auto", "true", "false"),
        ("modeline", "modeline", "true", "false"),
    ] {
        handled.insert(name);
        if changed(name) {
            set(own, format!("{key} = {}", if user.on(name) { yes } else { no }));
        }
    }
    handled.insert("modelines");
    if changed("modelines") && user.number("modelines") == 0 && !changed("modeline") {
        set(own, "modeline = false".to_string());
    }
    handled.extend(["ignorecase", "smartcase"]);
    if changed("ignorecase") || changed("smartcase") {
        let smart = |d: &Dump| d.on("ignorecase") && d.on("smartcase");
        match (user.on("ignorecase"), user.on("smartcase")) {
            (true, true) if !smart(base) => set(own, "smart_case = true".to_string()),
            (true, false) => own
                .skipped
                .push("ignorecase without smartcase: yumete ignores case only as smart_case does".to_string()),
            _ => {}
        }
    }
    handled.insert("whichwrap");
    if changed("whichwrap") {
        let ww = user.string("whichwrap");
        let letters: String =
            ww.split(',').filter(|w| ["b", "h", "l", "<", ">", "~"].contains(w)).collect::<Vec<_>>().concat();
        set(own, format!("which_wrap = {}", quote(&letters)));
        if ww.split(',').any(|w| w == "[" || w == "]") {
            own.skipped.push("whichwrap [ and ]: yumete's Insert-mode arrows do not cross lines".to_string());
        }
    }
    handled.insert("ambiwidth");
    if changed("ambiwidth") {
        match user.string("ambiwidth").as_str() {
            "double" => set(own, format!("ambiguous_width = {}", quote("wide"))),
            "single" => set(own, format!("ambiguous_width = {}", quote("narrow"))),
            _ => {}
        }
    }
    handled.insert("mousescroll");
    if changed("mousescroll") {
        let ms = user.string("mousescroll");
        match ms.split(',').find_map(|p| p.strip_prefix("ver:")).and_then(|n| n.parse::<i64>().ok()) {
            Some(n) => ranged(own, "mousescroll", "wheel_step", n, 1, 10),
            None => own.skipped.push(format!("mousescroll={ms}: no ver: step")),
        }
    }
    if user.colorscheme.is_some() && user.colorscheme != base.colorscheme {
        let name = user.colorscheme.clone().unwrap_or_default();
        own.skipped.push(format!("colorscheme {name}: {THEME_NOT_CARRIED}"));
    }
    // A highlight on yank (decision 10), and indent-blankline.
    let new = |a: &&Autocmd| !base.autocmds.contains(a);
    let flashes = |a: &Autocmd| {
        a.event == "TextYankPost"
            && [&a.desc, &a.group, &a.command].iter().any(|t| t.to_ascii_lowercase().contains("yank"))
    };
    let yank = user.autocmds.iter().filter(new).any(&flashes);
    let ibl = |d: &Dump| d.loaded.iter().any(|m| m == "ibl");
    if yank {
        set(own, "yank_flash = true  # from your TextYankPost autocommand".to_string());
    } else if base.autocmds.iter().any(flashes) && !user.autocmds.iter().any(flashes) {
        // **The reader's own "off"**: LazyVim flashes a yank, and a reader
        // who takes its autocommand away wants none.
        set(own, "yank_flash = false  # your config took the flash on yank away".to_string());
    }
    if ibl(user) && !ibl(base) {
        set(own, "indent_guides = true  # indent-blankline was loaded".to_string());
    } else if ibl(base) && !ibl(user) {
        set(own, "indent_guides = false  # your config left indent-blankline out".to_string());
    }
    // **A TextYankPost autocommand that says nothing of a yank** is
    // listed: kickstart's has a description, but one with a bare Lua
    // function cannot be told from any other.
    for a in user.autocmds.iter().filter(new).filter(|a| a.event == "TextYankPost" && !flashes(a)) {
        let what = [&a.desc, &a.group, &a.command]
            .into_iter()
            .find(|t| !t.trim().is_empty())
            .map_or_else(|| "a Lua function with no description or group".to_string(), |t| one_line(t.trim()));
        own.skipped.push(format!(
            "TextYankPost autocommand ({what}): yumete cannot tell what it does; its own flash on yank is yank_flash = true"
        ));
    }
    for a in user.autocmds.iter().filter(new) {
        let command = a.command.trim_start();
        if a.event == "FileType" && (command.starts_with("setl") || command.starts_with("set ")) {
            own.skipped.push(format!(
                "FileType {} ({}): yumete has no per-language settings",
                a.pattern,
                one_line(command)
            ));
        }
    }
    let rest: Vec<&str> = user
        .options
        .keys()
        .map(String::as_str)
        .filter(|n| !handled.contains(n) && !NOISE.contains(n) && changed(n))
        .collect();
    if !rest.is_empty() {
        own.skipped.push(format!("options with no counterpart in yumete: {}", rest.join(", ")));
    }
}

// ---- keys ----------------------------------------------------------------

/// Where a map lands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Table {
    Normal,
    Select,
    Insert,
}

impl Table {
    fn name(self) -> &'static str {
        match self {
            Table::Normal => "normal",
            Table::Select => "select",
            Table::Insert => "insert",
        }
    }
}

/// The tables a vim mode lands in, or why none.
fn tables(mode: &str) -> Result<&'static [Table], &'static str> {
    match mode {
        "n" => Ok(&[Table::Normal]),
        // `:map` is Normal, Visual and Operator-pending; a motion bound in
        // Normal mode is that motion after an operator too.
        " " => Ok(&[Table::Normal, Table::Select]),
        // Visual, and Visual and Select (`:vmap`): yumete's Select is vim's Visual.
        "x" | "v" => Ok(&[Table::Select]),
        "i" | "!" => Ok(&[Table::Insert]),
        "s" => Err("Select mode (snippets): yumete has no Select mode of vim's kind"),
        "o" => Err("yumete has no Operator-pending table; a motion bound in Normal mode is one after an operator too"),
        "c" => Err("yumete's command line takes [keys.prompt] lines, one key a command"),
        "t" => Err("yumete has no terminal"),
        _ => Err("a mode yumete does not have"),
    }
}

/// One key in vim's notation, split from a map's side.
fn tokens(side: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = side;
    while let Some(c) = rest.chars().next() {
        if c == '<' {
            if let Some(end) = rest.find('>') {
                let inside = &rest[1..end];
                let modded = inside.len() > 2 && inside.as_bytes()[1] == b'-';
                let named = !inside.is_empty()
                    && inside.len() <= 20
                    && (inside.chars().all(|c| c.is_ascii_alphanumeric()) || modded)
                    && !inside.contains(' ');
                if named {
                    out.push(rest[..=end].to_string());
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        out.push(c.to_string());
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// **One vim key in yumete's spelling** (`<C-S>` → `C-s`, `<CR>` →
/// `<enter>`), or why not.
fn vim_key(token: &str) -> Result<String, String> {
    let Some(name) = token.strip_prefix('<').and_then(|t| t.strip_suffix('>')).filter(|_| token.len() > 2) else {
        return Ok(token.to_string());
    };
    let cannot = || Err(format!("yumete cannot press {token}"));
    let lower = name.to_ascii_lowercase();
    let named = match lower.as_str() {
        "cr" | "enter" | "return" => "<enter>",
        "tab" => "<tab>",
        "s-tab" => "<S-tab>",
        "bs" | "backspace" => "<backspace>",
        "del" | "delete" => "<delete>",
        "up" => "<up>",
        "down" => "<down>",
        "left" => "<left>",
        "right" => "<right>",
        "home" => "<home>",
        "end" => "<end>",
        "pageup" => "<pageup>",
        "pagedown" => "<pagedown>",
        "lt" => "<",
        "bar" => "|",
        "bslash" => "\\",
        "space" => " ",
        // Neovim gives `<C-J>` back as the byte it is.
        "nl" => "C-j",
        _ => {
            let chord = |prefix: &str| -> Option<char> {
                let rest = &name[prefix.len()..];
                let mut chars = rest.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Some(c),
                    _ => None,
                }
            };
            return match lower.get(..2) {
                Some("c-") => match chord("C-") {
                    Some(c) if c.is_ascii_alphabetic() => Ok(format!("C-{}", c.to_ascii_lowercase())),
                    _ => cannot(),
                },
                Some("m-" | "a-") => match lower.get(..4) {
                    Some("m-s-" | "a-s-") => match chord("M-S-") {
                        Some(c) if c.is_ascii_alphabetic() => Ok(format!("A-{}", c.to_ascii_uppercase())),
                        _ => cannot(),
                    },
                    _ => match chord("M-") {
                        Some(c) if !c.is_whitespace() => Ok(format!("A-{c}")),
                        _ => cannot(),
                    },
                },
                _ => cannot(),
            };
        }
    };
    Ok(named.to_string())
}

/// **A side as yumete's keys**: each press spelt, and the presses in the
/// internal spelling. The round trip through `written` is the check: a
/// literal `C-` or `<enter>` in vim's keys would be read as one key.
fn presses(side: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mut spelt = Vec::new();
    for token in tokens(side) {
        spelt.push(vim_key(&token)?);
    }
    let internal: Vec<String> = spelt.iter().map(|s| yumete_core::actions::written(s)).collect();
    if yumete_core::actions::written(&spelt.concat()) != internal.concat() {
        return Err("its keys would be read as other keys by yumete".to_string());
    }
    Ok((spelt, internal))
}

/// helix's Ex names that vim shares with yumete, and vim's that yumete
/// spells another way. Read before anything else: `:sp` is yumete's
/// project search, `:r` its replace.
const VIM_EX: &[(&[&str], &str)] = &[
    (&["w", "write"], ":write"),
    (&["wa", "wall"], ":write-all"),
    (&["wq"], ":write-quit"),
    (&["wqa", "wqall", "xa", "xall"], ":write-quit-all"),
    (&["x", "xit", "exi", "exit"], ":exit"),
    (&["up", "update"], ":update"),
    (&["q", "quit"], ":quit"),
    (&["qa", "qall", "quita", "quitall"], ":quit-all"),
    (&["ene", "enew"], ":new"),
    (&["sp", "split"], ":hsplit"),
    (&["vs", "vsplit"], ":vsplit"),
    (&["new"], ":hnew"),
    (&["vne", "vnew"], ":vnew"),
    (&["on", "only"], ":quit-others"),
    (&["clo", "close"], ":quit-window"),
    (&["bn", "bnext"], ":buffer-next"),
    (&["bp", "bprevious", "bN", "bNext"], ":buffer-previous"),
    (&["bd", "bdelete", "bw", "bwipeout"], ":buffer-close"),
    (&["ls", "buffers", "files"], "buffer_picker"),
    (&["u", "undo"], ":undo"),
    (&["red", "redo"], ":redo"),
    (&["r", "read"], ":read"),
    (&["cd"], ":cd"),
    (&["pw", "pwd"], ":pwd"),
    (&["Ex", "Explore"], "file_explorer_in_current_buffer_directory"),
    (&["Lex", "Lexplore"], ":sidebar-explorer"),
];

/// Commands that take nothing after them here.
const BARE: &[&str] = &[
    ":write-all", ":write-quit-all", ":quit-all", ":new", ":hnew", ":vnew", ":quit-others", ":quit-window",
    ":buffer-next", ":buffer-previous", ":undo", ":redo", ":pwd", ":sidebar-explorer",
];

/// **Plugin commands**, the whole line, to yumete's (decision 6: the file
/// trees are `:sidebar-explorer`).
const PLUGIN_COMMANDS: &[(&str, &str)] = &[
    ("Telescope find_files", "file_picker"),
    ("Telescope git_files", "file_picker"),
    ("FzfLua files", "file_picker"),
    ("FzfLua git_files", "file_picker"),
    ("Telescope live_grep", ":search-project"),
    ("FzfLua live_grep", ":search-project"),
    ("Telescope buffers", "buffer_picker"),
    ("FzfLua buffers", "buffer_picker"),
    ("Telescope oldfiles", "global_recent_file_picker"),
    ("FzfLua oldfiles", "global_recent_file_picker"),
    ("Telescope help_tags", ":help"),
    ("FzfLua help_tags", ":help"),
    ("Telescope keymaps", ":keymap-actions"),
    ("FzfLua keymaps", ":keymap-actions"),
    ("Telescope commands", "command_palette"),
    ("FzfLua commands", "command_palette"),
    ("Telescope diagnostics", "workspace_diagnostics_picker"),
    ("FzfLua diagnostics_workspace", "workspace_diagnostics_picker"),
    ("Trouble diagnostics toggle", "workspace_diagnostics_picker"),
    ("Telescope diagnostics bufnr=0", "diagnostics_picker"),
    ("FzfLua diagnostics_document", "diagnostics_picker"),
    ("Trouble diagnostics toggle filter.buf=0", "diagnostics_picker"),
    ("Telescope lsp_document_symbols", "lsp_or_syntax_symbol_picker"),
    ("Telescope lsp_dynamic_workspace_symbols", "lsp_or_syntax_workspace_symbol_picker"),
    ("Telescope lsp_workspace_symbols", "lsp_or_syntax_workspace_symbol_picker"),
    ("Telescope lsp_references", "goto_reference"),
    ("Telescope lsp_definitions", "goto_definition"),
    ("Telescope lsp_implementations", "goto_implementation"),
    ("Telescope lsp_type_definitions", "goto_type_definition"),
    ("Telescope jumplist", "jumplist_picker"),
    ("FzfLua jumps", "jumplist_picker"),
    ("Telescope resume", "last_picker"),
    ("FzfLua resume", "last_picker"),
    ("Telescope git_status", "changed_file_picker"),
    ("FzfLua git_status", "changed_file_picker"),
    ("Telescope registers", "paste_menu"),
    ("FzfLua registers", "paste_menu"),
    ("Neotree toggle", ":sidebar-explorer"),
    ("NvimTreeToggle", ":sidebar-explorer"),
    ("Neotree", ":sidebar-explorer on"),
    ("Neotree focus", ":sidebar-explorer on"),
    ("Neotree reveal", ":sidebar-explorer on"),
    ("NvimTreeOpen", ":sidebar-explorer on"),
    ("NvimTreeFocus", ":sidebar-explorer on"),
    ("NvimTreeFindFile", ":sidebar-explorer on"),
    ("Oil", "file_explorer_in_current_buffer_directory"),
    ("AerialToggle", ":sidebar-outline"),
    ("SymbolsOutline", ":sidebar-outline"),
    ("Outline", ":sidebar-outline"),
    ("Trouble symbols toggle", ":sidebar-outline"),
    ("LazyGit", ":!lazygit"),
    ("Gitsigns preview_hunk", "show_change"),
    ("Gitsigns next_hunk", "goto_next_change"),
    ("Gitsigns prev_hunk", "goto_prev_change"),
    ("Gitsigns diffthis", ":git-diff"),
    ("Gdiffsplit", ":git-diff"),
    ("Gvdiffsplit", ":git-diff"),
    ("ZenMode", ":view-typewriter"),
    ("Twilight", ":view-focus"),
    ("LspInfo", ":info"),
];

/// **Functions a map may call**, by the name the dump found for them.
const KNOWN_FUNCTIONS: &[(&str, &str)] = &[
    ("vim.lsp.buf.definition", "goto_definition"),
    ("vim.lsp.buf.declaration", "goto_declaration"),
    ("vim.lsp.buf.implementation", "goto_implementation"),
    ("vim.lsp.buf.type_definition", "goto_type_definition"),
    ("vim.lsp.buf.references", "goto_reference"),
    ("vim.lsp.buf.hover", "hover"),
    ("vim.lsp.buf.rename", "rename_symbol"),
    ("vim.lsp.buf.code_action", "code_action"),
    ("vim.lsp.buf.document_symbol", "lsp_or_syntax_symbol_picker"),
    ("vim.lsp.buf.workspace_symbol", "lsp_or_syntax_workspace_symbol_picker"),
    ("vim.diagnostic.open_float", "show_diagnostic"),
    ("vim.diagnostic.goto_next", "goto_next_diag"),
    ("vim.diagnostic.goto_prev", "goto_prev_diag"),
    ("vim.diagnostic.setloclist", "diagnostics_picker"),
    ("telescope.builtin.find_files", "file_picker"),
    ("telescope.builtin.git_files", "file_picker"),
    ("telescope.builtin.live_grep", ":search-project"),
    ("telescope.builtin.buffers", "buffer_picker"),
    ("telescope.builtin.oldfiles", "global_recent_file_picker"),
    ("telescope.builtin.help_tags", ":help"),
    ("telescope.builtin.keymaps", ":keymap-actions"),
    ("telescope.builtin.commands", "command_palette"),
    ("telescope.builtin.diagnostics", "workspace_diagnostics_picker"),
    ("telescope.builtin.lsp_references", "goto_reference"),
    ("telescope.builtin.lsp_definitions", "goto_definition"),
    ("telescope.builtin.lsp_implementations", "goto_implementation"),
    ("telescope.builtin.lsp_type_definitions", "goto_type_definition"),
    ("telescope.builtin.lsp_document_symbols", "lsp_or_syntax_symbol_picker"),
    ("telescope.builtin.lsp_dynamic_workspace_symbols", "lsp_or_syntax_workspace_symbol_picker"),
    ("telescope.builtin.resume", "last_picker"),
    ("telescope.builtin.jumplist", "jumplist_picker"),
    ("telescope.builtin.git_status", "changed_file_picker"),
    ("telescope.builtin.registers", "paste_menu"),
    ("fzf-lua.files", "file_picker"),
    ("fzf-lua.live_grep", ":search-project"),
    ("fzf-lua.buffers", "buffer_picker"),
    ("gitsigns.preview_hunk", "show_change"),
    ("gitsigns.next_hunk", "goto_next_change"),
    ("gitsigns.prev_hunk", "goto_prev_change"),
    ("oil.open", "file_explorer_in_current_buffer_directory"),
];

/// **One Ex line as yumete's**: an action's name or a `:` line, or why not.
fn vim_ex(line: &str) -> Result<String, String> {
    let mut line = line.trim();
    loop {
        let before = line;
        line = line.trim_start_matches(':').trim_start();
        for word in ["silent!", "silent "] {
            if let Some(rest) = line.strip_prefix(word) {
                line = rest.trim_start();
            }
        }
        if line == before {
            break;
        }
    }
    if let Some(call) = line.strip_prefix("lua ") {
        let call = call.trim();
        return call
            .strip_suffix("()")
            .and_then(|name| KNOWN_FUNCTIONS.iter().find(|(f, _)| *f == name && f.starts_with("vim.")))
            .map(|(_, to)| to.to_string())
            .ok_or_else(|| format!("a Lua call yumete cannot read ({call})"));
    }
    let words = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((_, to)) = PLUGIN_COMMANDS.iter().find(|(from, _)| *from == words) {
        return Ok(to.to_string());
    }
    // **vim reads `%`, `#` and `!` in a line as something else**: `%` and
    // `#` are the file's and the other file's names, `!` in a shell command
    // is the last one, and `:w !cmd` and `:r !cmd` pipe through a shell.
    // yumete takes them as written: `:w !sudo tee %` would make a file named
    // `!sudo tee %`, and `:e #` look for a file named `#`.
    if let Some(shell) = line.strip_prefix('!') {
        if shell.contains(['%', '#', '!']) {
            return Err(format!("vim reads % # and ! in :{line} as file names and the last command; yumete does not"));
        }
        return checked(format!(":!{}", shell.trim()), line);
    }
    let (word, rest) = match line.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (line, ""),
    };
    if rest.starts_with('!') {
        return Err(format!("vim's :{line} goes through a shell command; yumete's does not"));
    }
    if rest.contains(['%', '#']) {
        return Err(format!("vim reads % and # in :{line} as file names; yumete does not"));
    }
    let (base, bang) = match word.strip_suffix('!') {
        Some(base) if !base.is_empty() => (base, "!"),
        _ => (word, ""),
    };
    // `:e` alone reloads the file; with a name it opens one.
    if matches!(base, "e" | "edit") {
        return match rest.is_empty() {
            true => checked(format!(":reload{bang}"), line),
            false => checked(format!(":open {rest}"), line),
        };
    }
    if matches!(base, "h" | "help") && rest.is_empty() {
        return checked(":help".to_string(), line);
    }
    match VIM_EX.iter().find(|(names, _)| names.contains(&base)) {
        Some((_, to)) if !to.starts_with(':') => match rest.is_empty() && bang.is_empty() {
            true => Ok(to.to_string()),
            false => Err(format!("yumete's {to} takes nothing after it")),
        },
        Some((_, to)) if BARE.contains(to) && !rest.is_empty() => Err(format!("yumete's {to} takes nothing after it")),
        Some((_, to)) => match rest.is_empty() {
            true => checked(format!("{to}{bang}"), line),
            false => checked(format!("{to}{bang} {rest}"), line),
        },
        None => Err(format!("yumete has no :{line}")),
    }
}

/// A `:` line yumete's own parser takes, or why not.
fn checked(out: String, line: &str) -> Result<String, String> {
    match yumete_core::command::parse(&out) {
        Ok(_) => Ok(out),
        Err(_) => Err(format!("yumete's {} does not take :{line}", out.split_whitespace().next().unwrap_or(&out))),
    }
}

/// The Ex lines a right side is made of (`:w<CR>`, `<Cmd>w<CR><Esc>`, two in
/// a row), `None` when it is not one.
fn ex_lines(rhs: &str) -> Option<Result<Vec<String>, String>> {
    let starts = |s: &str, p: &str| s.len() >= p.len() && s.is_char_boundary(p.len()) && s[..p.len()].eq_ignore_ascii_case(p);
    let mut rest = rhs;
    let mut lines = Vec::new();
    while !rest.is_empty() {
        let body = if let Some(after) = rest.strip_prefix(':') {
            after
        } else if starts(rest, "<cmd>") {
            &rest[5..]
        } else if lines.is_empty() {
            return None;
        } else {
            return Some(Err("mixes keys and an Ex command line".to_string()));
        };
        let body = if starts(body, "<c-u>") { &body[5..] } else { body };
        let Some(end) = body.to_ascii_lowercase().find("<cr>") else {
            return Some(Err(match lines.is_empty() {
                true => "leaves a command line half typed".to_string(),
                false => "mixes keys and an Ex command line".to_string(),
            }));
        };
        let mut line = String::new();
        for token in tokens(&body[..end]) {
            match token.to_ascii_lowercase().as_str() {
                "<space>" => line.push(' '),
                "<lt>" => line.push('<'),
                "<bar>" => line.push('|'),
                "<bslash>" => line.push('\\'),
                t if t.starts_with('<') && t.len() > 2 => {
                    return Some(Err(format!("its Ex line holds {token}")));
                }
                _ => line.push_str(&token),
            }
        }
        if line.contains('|') {
            return Some(Err("runs several Ex commands in one line".to_string()));
        }
        lines.push(line);
        rest = &body[end + 4..];
        if starts(rest, "<esc>") {
            rest = &rest[5..];
        }
    }
    Some(Ok(lines))
}

/// What yumete's vim keys already do: each press run of each action, and
/// the top-level keys that run keys (vim's `Y` is `@y$`).
struct Keymap {
    commands: Vec<yumete_core::editor::DocCommand>,
    normal: Vec<(Vec<String>, String)>,
    select: Vec<(Vec<String>, String)>,
    /// The keys of yumete's Space menu, as its rows spell them, and what
    /// the row says, in English as the file is.
    space: Vec<(String, String)>,
}

impl Keymap {
    fn new() -> Keymap {
        let docs = yumete_core::Editor::keymap_for_docs(yumete_config::KeyPreset::Vim, yumete_config::Flavor::default());
        let english = |tag: &str| {
            yumete_core::messages::table().get(tag).map(|e| e.en).filter(|en| !en.is_empty()).unwrap_or(tag).to_string()
        };
        let space = docs
            .menus
            .iter()
            .filter(|m| m.title == "hint.space.title")
            .flat_map(|m| m.rows.iter())
            .flat_map(|(keys, says)| keys.split(" / ").map(|k| (k.to_string(), english(says))).collect::<Vec<_>>())
            .collect();
        Keymap {
            commands: yumete_core::Editor::commands_for_docs(),
            normal: docs.normal.into_iter().map(|k| (k.keys, k.run)).collect(),
            select: docs.select.into_iter().map(|k| (k.keys, k.run)).collect(),
            space,
        }
    }

    /// The one action these presses reach in `table`, if one does.
    fn action(&self, table: Table, keys: &[String]) -> Option<&'static str> {
        let mut found = self.commands.iter().filter(|(_, _, _, vim, insert)| match table {
            Table::Insert => insert.iter().any(|k| k == keys),
            _ => vim.iter().any(|k| k == keys),
        });
        match (found.next(), found.next()) {
            (Some((name, ..)), None) => Some(name),
            // The window group reads `C-h` after `C-w` as `h`, as vim does.
            _ if keys.len() == 2 && keys[0] == yumete_core::actions::written("C-w") => {
                let plain = ('a'..='z').find(|l| yumete_core::actions::written(&format!("C-{l}")) == keys[1])?;
                self.action(table, &[keys[0].clone(), plain.to_string()])
            }
            _ => None,
        }
    }

    /// Whether `lhs` already does `value` in yumete.
    fn already(&self, table: Table, lhs: &[String], value: &str) -> bool {
        if self.action(table, lhs) == Some(value) {
            return true;
        }
        let top = match table {
            Table::Normal => &self.normal,
            Table::Select => &self.select,
            Table::Insert => return false,
        };
        top.iter().any(|(keys, run)| keys == lhs && run == value)
    }
}

/// **A right side as yumete's**: one command, or a list of them.
fn vim_rhs(map: &Map, table: Table, keymap: &Keymap) -> Result<Vec<String>, String> {
    if let Some(name) = &map.callback {
        return KNOWN_FUNCTIONS
            .iter()
            .find(|(f, _)| f == name)
            .map(|(_, to)| vec![to.to_string()])
            .ok_or_else(|| match name.is_empty() {
                true => "a Lua function".to_string(),
                false => format!("a Lua function yumete has nothing like ({name})"),
            });
    }
    if map.expr {
        return Err("an expression map".to_string());
    }
    let rhs = map.rhs.as_str();
    if rhs.is_empty() || rhs.eq_ignore_ascii_case("<nop>") {
        return Ok(vec!["no_op".to_string()]);
    }
    let lower = rhs.to_ascii_lowercase();
    if lower.contains("<plug>") || lower.contains("<sid>") || lower.contains("<snr>") {
        return Err("a plugin's own key".to_string());
    }
    if table == Table::Insert && lower == "<esc>" {
        return Ok(vec!["normal_mode".to_string()]);
    }
    if rhs == ":" && table != Table::Insert {
        return Ok(vec!["command_mode".to_string()]);
    }
    if let Some(lines) = ex_lines(rhs) {
        return lines?.iter().map(|l| vim_ex(l)).collect();
    }
    let (spelt, internal) = presses(rhs)?;
    if let Some(name) = keymap.action(table, &internal) {
        return Ok(vec![name.to_string()]);
    }
    // **`vnoremap > >gv`, one of the commonest vimrc lines**: as the run
    // `@>gv` it would indent once and then say "nothing has been selected
    // yet", as the run's `gv` does not find the selection its own `>` just
    // left. yumete's `indent` and `unindent` keep the selection and Visual
    // mode, which is what the map is for.
    if table == Table::Select {
        match spelt.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
            [">", "g", "v"] => return Ok(vec!["indent".to_string()]),
            ["<", "g", "v"] => return Ok(vec!["unindent".to_string()]),
            _ => {}
        }
    }
    if table == Table::Insert {
        return Err("keys in Insert mode, which yumete types rather than presses".to_string());
    }
    Ok(vec![format!("@{}", spelt.concat())])
}

/// A right side as TOML.
fn rendered(run: &[String]) -> String {
    match run {
        [one] => quote(one),
        many => format!("[{}]", many.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ")),
    }
}

/// One table's lines: the key, and what follows ` = `.
type Lines = Vec<(String, String)>;

/// The keys written so far, table by table, in the order first written.
#[derive(Default)]
struct Written {
    tables: Vec<(Vec<String>, Lines)>,
    /// Every key path written, a group's included, to keep TOML's one
    /// definition of each.
    leaves: BTreeSet<Vec<String>>,
    groups: BTreeSet<Vec<String>>,
}

impl Written {
    /// Put `line` at `key` in the table `path`, unless the place is taken;
    /// whether it went in.
    fn put(&mut self, path: Vec<String>, key: String, line: String) -> bool {
        let mut full = path.clone();
        full.push(key.clone());
        if self.leaves.contains(&full) || self.groups.contains(&full) {
            return false;
        }
        // A key that would also be a group above this one.
        if (1..path.len()).any(|n| self.leaves.contains(&path[..=n])) {
            return false;
        }
        for n in 1..path.len() {
            self.groups.insert(path[..=n].to_vec());
        }
        self.leaves.insert(full);
        match self.tables.iter_mut().find(|(p, _)| *p == path) {
            Some((_, lines)) => lines.push((key, line)),
            None => self.tables.push((path, vec![(key, line)])),
        }
        true
    }

    fn text(&self) -> String {
        let mut out = String::new();
        let mut tables: Vec<_> = self.tables.iter().collect();
        let rank = |mode: &str| ["normal", "select", "insert"].iter().position(|m| *m == mode);
        tables.sort_by(|(a, _), (b, _)| (rank(&a[0]), &a[1..]).cmp(&(rank(&b[0]), &b[1..])));
        for (path, lines) in tables {
            let header: Vec<String> = path.iter().map(|p| toml_key(p)).collect();
            out.push_str(&format!("\n[keys.vim.{}]\n", header.join(".")));
            for (key, line) in lines {
                out.push_str(&format!("{} = {line}\n", toml_key(key)));
            }
        }
        out
    }
}

/// A map's mode and left side as the reader wrote them, `<leader>` put back.
fn left(map: &Map, leader: Option<&str>) -> String {
    // Vim's `maplist()` writes a Space leader `<Space>`.
    let lhs = match leader.filter(|l| !l.is_empty()) {
        Some(leader) if map.lhs.starts_with(leader) => format!("<leader>{}", &map.lhs[leader.len()..]),
        Some(" ") if map.lhs.starts_with("<Space>") => format!("<leader>{}", &map.lhs["<Space>".len()..]),
        _ => map.lhs.clone(),
    };
    format!("{} {}", if map.mode == " " { "nvo" } else { map.mode.as_str() }, lhs.replace(' ', "<Space>"))
}

/// A map as the list at the end names it: its keys, its description, and
/// the file and line when a vimscript file gave them (Lua's are always 0).
fn shown(map: &Map, leader: Option<&str>) -> String {
    let mut out = left(map, leader);
    if let Some(desc) = &map.desc {
        out.push_str(&format!(" ({desc})"));
    }
    if let (Some(script), true) = (&map.script, map.line > 0) {
        let file = std::path::Path::new(script).file_name().map(|f| f.to_string_lossy().to_string());
        out.push_str(&format!(", {}:{}", file.unwrap_or_default(), map.line));
    }
    out
}

/// What identifies a map between two runs.
fn identity(m: &Map) -> (String, String, String, Option<String>, Option<String>, bool) {
    (m.mode.clone(), m.lhs.clone(), m.rhs.clone(), m.desc.clone(), m.callback.clone(), m.expr)
}

/// The LazyVim config's own Normal-mode keys, internal spelling, Space
/// for the Space group.
fn lazyvim_keys() -> BTreeSet<String> {
    fn walk(table: &toml::Table, at: &str, out: &mut BTreeSet<String>) {
        for (key, value) in table {
            if matches!(key.as_str(), "label" | "run" | "sticky" | "inherit") && !at.is_empty() {
                continue;
            }
            let Ok(key) = yumete_config::helix_key(key) else { continue };
            let run = format!("{at}{}", yumete_core::actions::written(&key));
            match value {
                toml::Value::Table(inner) if !inner.contains_key("run") => walk(inner, &run, out),
                _ => {
                    out.insert(run);
                }
            }
        }
    }
    let mut out = BTreeSet::new();
    let table: toml::Table = toml::from_str(yumete_config::LAZYVIM).unwrap_or_default();
    if let Some(normal) = table.get("keys").and_then(|k| k.get("normal")).and_then(toml::Value::as_table) {
        walk(normal, "", &mut out);
    }
    out
}

/// **The reader's own keys**, from `user` less `base`.
fn keys(user: &Dump, base: &Dump, from: Source, own: &mut Own) {
    let keymap = Keymap::new();
    let leader = user.leader.as_deref();
    let space_leader = leader == Some(" ");
    let in_base: BTreeSet<_> = base.maps.iter().map(identity).collect();
    let mut mine: Vec<&Map> = user
        .maps
        .iter()
        .filter(|m| !in_base.contains(&identity(m)))
        .filter(|m| !m.lhs.starts_with("<Plug>") && !m.lhs.starts_with("<SNR>"))
        .filter(|m| from != Source::Vim || !user.vims_own(m.script.as_deref()))
        .collect();
    mine.sort_by(|a, b| (&a.mode, &a.lhs).cmp(&(&b.mode, &b.lhs)));
    let mut written = Written::default();
    let mut already: Vec<String> = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for map in mine {
        let say = |own: &mut Own, why: &str| own.skipped.push(format!("{}: {why}", shown(map, leader)));
        let targets = match tables(&map.mode) {
            Ok(targets) => targets,
            Err(why) => {
                say(own, why);
                continue;
            }
        };
        let (spelt, internal) = match presses(&map.lhs) {
            Ok(keys) => keys,
            Err(why) => {
                say(own, &why.replace("cannot press", "cannot bind"));
                continue;
            }
        };
        // The Space key itself: the Space menu stays (decision 13). Asked
        // of the key, not the spelling: Neovim gives it back as " ", Vim's
        // `maplist()` as `<Space>`, and either written as Space's binding
        // would switch the whole menu off without a word.
        if spelt == [" "] {
            let nop = map.callback.is_none() && (map.rhs.is_empty() || map.rhs.eq_ignore_ascii_case("<nop>"));
            if !(nop && space_leader) {
                say(own, "the Space key itself: yumete keeps its Space menu there");
            }
            continue;
        }
        // yumete's left side is read by its own reader: the spelling must
        // come back as these keys.
        let flat = spelt.concat();
        if !yumete_config::helix_key(&flat).is_ok_and(|w| yumete_core::actions::written(&w) == internal.concat()) {
            say(own, "yumete would read these keys as others");
            continue;
        }
        for &table in targets {
            let run = match vim_rhs(map, table, &keymap) {
                Ok(run) => run,
                Err(why) => {
                    say(own, &why);
                    break;
                }
            };
            if let [one] = run.as_slice() {
                if keymap.already(table, &internal, one) {
                    already.push(left(map, leader));
                    continue;
                }
            }
            let value = rendered(&run);
            let nested = space_leader && table != Table::Insert && spelt.len() > 1 && spelt[0] == " ";
            let ok = match nested {
                true => {
                    let mut path = vec![table.name().to_string(), "space".to_string()];
                    path.extend(spelt[1..spelt.len() - 1].iter().cloned());
                    let line = match &map.desc {
                        // A control character drawn in the menu would
                        // reach the terminal.
                        Some(desc) => format!("{{ run = {value}, label = {} }}", quote(&one_line(desc))),
                        None => value,
                    };
                    // A key of yumete's Space menu, taken (decision 16): said
                    // once, with what yumete had there.
                    let had = keymap.space.iter().find(|(k, _)| *k == spelt[1]);
                    if let (true, Some((key, says))) = (from != Source::LazyVim, had) {
                        if taken.insert(key.clone()) {
                            let still = if key == "w" { "; the group is still on C-w" } else { "" };
                            own.takes.push(format!("<leader>{key}: yumete's Space {key} ({says}{still})"));
                        }
                    }
                    written.put(path, spelt[spelt.len() - 1].clone(), line)
                }
                false => {
                    let line = match &map.desc {
                        Some(desc) => format!("{value}  # {}", one_line(desc)),
                        None => value,
                    };
                    written.put(vec![table.name().to_string()], flat.clone(), line)
                }
            };
            match ok {
                true => own.carried += 1,
                false => say(own, "its keys are also the start of another map's, and a yumete group has no key of its own"),
            }
        }
    }
    // Defaults the reader deleted (decision 15): only Neovim's `Y` comes
    // back, as vim's; LazyVim's keys come back as no key at all.
    let in_user: BTreeSet<_> = user.maps.iter().map(|m| (m.mode.clone(), m.lhs.clone())).collect();
    let preset = if from == Source::LazyVim { lazyvim_keys() } else { BTreeSet::new() };
    for gone in base.maps.iter().filter(|m| !in_user.contains(&(m.mode.clone(), m.lhs.clone()))) {
        match from {
            Source::Neovim if gone.mode == "n" && gone.lhs == "Y" => {
                if written.put(vec!["normal".to_string()], "Y".to_string(), quote("@yy")) {
                    own.carried += 1;
                }
            }
            Source::Neovim if !gone.lhs.starts_with("<Plug>") => {
                own.skipped.push(format!("{}: you removed Neovim's default; yumete's key, if any, stays", shown(gone, None)))
            }
            Source::LazyVim if gone.mode == "n" => {
                let Ok((spelt, internal)) = presses(&gone.lhs) else { continue };
                if !preset.contains(&internal.concat()) {
                    continue;
                }
                let ok = match spelt.first().map(String::as_str) {
                    Some(" ") if spelt.len() > 1 => {
                        let mut path = vec!["normal".to_string(), "space".to_string()];
                        path.extend(spelt[1..spelt.len() - 1].iter().cloned());
                        written.put(path, spelt[spelt.len() - 1].clone(), quote("no_op"))
                    }
                    _ => written.put(vec!["normal".to_string()], spelt.concat(), quote("no_op")),
                };
                if ok {
                    own.carried += 1;
                }
            }
            _ => {}
        }
    }
    if !already.is_empty() {
        own.skipped.push(format!(
            "{} map{} already do{} the same in yumete: {}",
            already.len(),
            if already.len() == 1 { "" } else { "s" },
            if already.len() == 1 { "es" } else { "" },
            already.join(", ")
        ));
    }
    own.keys = written.text();
    // Only a run that was written, not one that was already so.
    own.runs = own.keys.contains("\"@");
}

/// **The reader's own settings and keys**: `user` less `base`.
pub(super) fn translate(user: &Dump, base: &Dump, from: Source) -> Own {
    let mut own = Own { stopped: user.stopped, ..Own::default() };
    options(user, base, from, &mut own);
    keys(user, base, from, &mut own);
    own
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(mode: &str, lhs: &str, rhs: &str) -> Map {
        Map {
            mode: mode.to_string(),
            lhs: lhs.to_string(),
            rhs: rhs.to_string(),
            desc: None,
            callback: None,
            expr: false,
            script: None,
            line: 0,
        }
    }

    fn rhs(table: Table, side: &str) -> Result<Vec<String>, String> {
        vim_rhs(&map("n", "x", side), table, &Keymap::new())
    }

    fn ok(one: &str) -> Result<Vec<String>, String> {
        Ok(vec![one.to_string()])
    }

    #[test]
    fn vim_keys_are_spelt_the_yumete_way() {
        let spelt = |side: &str| presses(side).map(|(s, _)| s.concat());
        assert_eq!(spelt("<C-S>").as_deref(), Ok("C-s"));
        assert_eq!(spelt("<M-j>").as_deref(), Ok("A-j"));
        assert_eq!(spelt("<lt>").as_deref(), Ok("<"));
        assert_eq!(spelt("<S-Tab>").as_deref(), Ok("<S-tab>"));
        assert_eq!(spelt("<NL>").as_deref(), Ok("C-j"));
        assert_eq!(spelt(" ff").as_deref(), Ok(" ff"));
        assert!(spelt("<Esc>").is_err());
        assert!(spelt("<F5>").is_err());
        assert!(spelt("<C-^>").is_err());
        // Written out, these would be read as one key by yumete.
        assert!(spelt("C-x").is_err());
        assert!(spelt("<lt>enter>").is_err());
        // `>gv` and `<gv` keep the selection, as `indent` and `unindent` do;
        // the run `@>gv` would lose it.
        assert_eq!(rhs(Table::Select, "<lt>gv"), ok("unindent"));
        assert_eq!(rhs(Table::Select, "<gv"), ok("unindent"));
        assert_eq!(rhs(Table::Select, ">gv"), ok("indent"));
        assert_eq!(rhs(Table::Normal, ">gv"), ok("@>gv"));
        assert_eq!(rhs(Table::Normal, "y$"), ok("@y$"));
        // One key of yumete's with a name: the name.
        assert_eq!(rhs(Table::Normal, "<C-W>h"), ok("jump_view_left"));
        assert_eq!(rhs(Table::Normal, "<C-W><C-H>"), ok("jump_view_left"));
        assert_eq!(rhs(Table::Normal, "<C-W><NL>"), ok("jump_view_down"));
        assert_eq!(rhs(Table::Insert, "<Right>"), ok("move_char_right"));
        assert_eq!(rhs(Table::Insert, "<Esc>"), ok("normal_mode"));
        assert!(rhs(Table::Insert, "<C-o>dd").is_err(), "Insert mode takes no runs");
        assert_eq!(rhs(Table::Normal, "<Nop>"), ok("no_op"));
        assert!(rhs(Table::Normal, "<Plug>(x)").is_err());
        let mut function = map("n", "x", "");
        function.callback = Some("vim.lsp.buf.definition".to_string());
        assert_eq!(vim_rhs(&function, Table::Normal, &Keymap::new()), ok("goto_definition"));
        function.callback = Some(String::new());
        assert!(vim_rhs(&function, Table::Normal, &Keymap::new()).is_err());
        let mut expr = map("n", "j", "v:count == 0 ? 'gj' : 'j'");
        expr.expr = true;
        assert!(vim_rhs(&expr, Table::Normal, &Keymap::new()).is_err());
    }

    #[test]
    fn ex_lines_go_in_by_vim_s_names() {
        let one = |side: &str| rhs(Table::Normal, side);
        assert_eq!(one("<Cmd>w<CR><Esc>"), ok(":write"));
        assert_eq!(one(":w<CR>"), ok(":write"));
        // vim's `:sp` is a split; yumete's is the project search.
        assert_eq!(one(":sp<CR>"), ok(":hsplit"));
        assert_eq!(one(":bd<CR>"), ok(":buffer-close"));
        assert_eq!(one(":e<CR>"), ok(":reload"));
        assert_eq!(one(":Neotree toggle<CR>"), ok(":sidebar-explorer"));
        assert_eq!(one("<Cmd>Telescope find_files<CR>"), ok("file_picker"));
        assert_eq!(one("<Cmd>lua vim.lsp.buf.hover()<CR>"), ok("hover"));
        assert_eq!(one(":w<CR>:q<CR>"), Ok(vec![":write".to_string(), ":quit".to_string()]));
        assert_eq!(one(":"), ok("command_mode"));
        assert!(one(":ls<CR>:b ").is_err());
        assert!(one(":%s/").is_err());
        assert!(one(":Today<CR>").is_err());
        assert!(one(":m .+1<CR>==").is_err());
        assert!(one(":noh<CR>").is_err());
        assert!(one(":ls<CR>x").is_err());
        assert!(one(":bn<Bar>bd #<CR>").is_err());
        // vim reads `%`, `#` and `!` as names and shell commands, which
        // yumete does not: these are listed, not carried.
        for side in [":w !sudo tee %<CR>", "<Cmd>!chmod +x %<CR>", ":e #<CR>", ":r !date<CR>", ":cd %:p:h<CR>", ":!!<CR>"] {
            assert!(one(side).is_err(), "{side}: {:?}", one(side));
        }
        assert_eq!(one(":!make<CR>"), ok(":!make"));
        assert_eq!(one(":e notes.md<CR>"), ok(":open notes.md"));
    }

    /// **Vim's Space key is the Space key**: Vim's `maplist()` gives it as
    /// `<Space>`, Neovim as " ", and either leaves the Space menu where it
    /// is.
    #[test]
    fn a_map_of_vim_s_space_key_leaves_the_space_menu() {
        let vim_dump = |leader: &str, rhs: &str| {
            let json = format!(
                r#"{{ "vim": "9.1", "leader": "{leader}", "options": {{}},
                   "keymaps": [ {{ "mode": "n", "lhs": "<Space>", "rhs": "{rhs}", "script": "/home/me/.vimrc", "line": 3 }},
                                {{ "mode": "n", "lhs": "<Space>w", "rhs": ":w<CR>", "script": "/home/me/.vimrc", "line": 4 }} ] }}"#
            );
            Dump::from_json(&serde_json::from_str(&json).unwrap())
        };
        let none = Dump::from_json(&serde_json::from_str(r#"{ "vim": "9.1", "options": {}, "keymaps": [] }"#).unwrap());
        let own = translate(&vim_dump(" ", "za"), &none, Source::Vim);
        assert!(!own.keys.contains("\" \" ="), "{}", own.keys);
        assert_eq!(own.skipped, ["n <leader>, .vimrc:3: the Space key itself: yumete keeps its Space menu there"]);
        assert!(own.keys.contains("[keys.vim.normal.space]\nw = \":write\"\n"), "{}", own.keys);
        // The leader on `<Nop>` is left without a word (decision 13).
        let own = translate(&vim_dump(" ", "<Nop>"), &none, Source::Vim);
        assert!(!own.keys.contains("\" \" ="), "{}", own.keys);
        assert!(own.skipped.is_empty(), "{:?}", own.skipped);
    }

    /// **Every name the tables translate to is yumete's.**
    #[test]
    fn every_vim_table_lands_on_something_yumete_has() {
        // `:read` and `:cd` want a name after them.
        let has = |to: &str| match to.starts_with(':') {
            true => yumete_core::command::parse(to).is_ok() || yumete_core::command::parse(&format!("{to} x.md")).is_ok(),
            false => yumete_core::actions::action(to).is_some(),
        };
        for (names, to) in VIM_EX {
            assert!(has(to), "{names:?} → {to}");
        }
        for (from, to) in PLUGIN_COMMANDS.iter().chain(KNOWN_FUNCTIONS) {
            assert!(has(to), "{from} → {to}");
        }
        for to in BARE {
            assert!(has(to), "{to}");
        }
    }
}
