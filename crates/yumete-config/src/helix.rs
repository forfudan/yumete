//! **helix's settings in yumete's words**: what the config import and
//! `:set` share, so a helix name means one thing in both.

/// What a helix `[editor]` table carries over ([`editor_table`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Carried {
    /// `key = value` lines for yumete's `[editor]`.
    pub editor: Vec<String>,
    /// What was left out, and why.
    pub skipped: Vec<String>,
}

/// **One helix option and its value as yumete's settings** (helix's
/// `:set-option key value`): each `(table, key, value)`, the value as TOML
/// writes it. The value is read as TOML (`3`, `false`, `"x"`), and as a bare
/// word where TOML does not take it (`warning`); a dotted key names a
/// sub-table (`soft-wrap.enable`). `Err(Some(why))` when helix has the option
/// and yumete cannot carry this value; `Err(None)` when yumete has nothing
/// for it.
#[allow(clippy::type_complexity)]
pub fn option(key: &str, value: &str) -> Result<Vec<(String, String, String)>, Option<String>> {
    let parsed = toml::from_str::<toml::Table>(&format!("v = {value}"))
        .ok()
        .and_then(|mut t| t.remove("v"))
        .unwrap_or_else(|| toml::Value::String(value.to_string()));
    let mut table = toml::Table::new();
    match key.split_once('.') {
        Some((outer, inner)) => {
            let mut sub = toml::Table::new();
            sub.insert(inner.to_string(), parsed);
            table.insert(outer.to_string(), toml::Value::Table(sub));
        }
        None => {
            table.insert(key.to_string(), parsed);
        }
    }
    let carried = editor_table(&table);
    if carried.editor.is_empty() {
        return Err(carried.skipped.into_iter().next().filter(|why| !why.ends_with(NO_COUNTERPART)));
    }
    Ok(carried
        .editor
        .iter()
        .filter_map(|line| line.split_once(" = "))
        .map(|(key, value)| ("editor".to_string(), key.to_string(), value.to_string()))
        .collect())
}

/// What a helix setting yumete has nothing for is said to have.
const NO_COUNTERPART: &str = "no counterpart in yumete";

/// A whole number in `low..=high`, or why not.
fn number_in(value: &toml::Value, low: i64, high: i64) -> Result<i64, String> {
    match value.as_integer() {
        Some(n) if (low..=high).contains(&n) => Ok(n),
        Some(n) => Err(format!("{n} is outside yumete's {low}–{high}")),
        None => Err("not a whole number".to_string()),
    }
}

/// One `[editor]` line carried over.
fn set(got: &mut Carried, key: &str, value: String) {
    got.editor.push(format!("{key} = {value}"));
}

/// **The level a helix `[editor.auto-pairs]` table names**: its pairs as a
/// set, when they are exactly one of yumete's levels. An empty table pairs
/// nothing; the three brackets are `minimal`; helix's own six are `basic`.
fn auto_pairs_level(pairs: &toml::Table) -> Option<&'static str> {
    let one = |s: &str| {
        let mut chars = s.chars();
        chars.next().filter(|_| chars.next().is_none())
    };
    let mut got: Vec<(char, char)> = Vec::new();
    for (open, close) in pairs {
        got.push((one(open)?, one(close.as_str()?)?));
    }
    got.sort_unstable();
    got.dedup();
    let same = |want: &[(char, char)]| {
        let mut want = want.to_vec();
        want.sort_unstable();
        got == want
    };
    let brackets = [('(', ')'), ('[', ']'), ('{', '}')];
    let helix = [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\''), ('`', '`')];
    match () {
        _ if got.is_empty() => Some("off"),
        _ if same(&brackets) => Some("minimal"),
        _ if same(&helix) => Some("basic"),
        _ => None,
    }
}

/// **helix's `[editor]`, for the settings yumete has**: each line carried
/// over, `key = value` under yumete's `[editor]`, and each one left out with
/// why. The config import reads a whole helix file through it, and `:set`
/// one helix option.
pub fn editor_table(table: &toml::Table) -> Carried {
    let mut carried = Carried::default();
    let got = &mut carried;
    let mut line_numbers: Option<String> = None;
    let mut gutters_hide_numbers = false;
    for (key, value) in table {
        let skip = |got: &mut Carried, why: &str| got.skipped.push(format!("[editor] {key}: {why}"));
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
            // helix names the least level drawn; yumete has three levels, and
            // each of helix's five lands on the one that draws what it drew.
            "end-of-line-diagnostics" => match value.as_str() {
                Some("disable") => set(got, "end_of_line_diagnostics", quote("off")),
                Some("error" | "warning") => set(got, "end_of_line_diagnostics", quote("basic")),
                Some("info" | "hint") => set(got, "end_of_line_diagnostics", quote("full")),
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
                toml::Value::Table(pairs) => match auto_pairs_level(pairs) {
                    Some(level) => set(got, "auto_pairs", quote(level)),
                    None => skip(got, "yumete's pairs are fixed: auto_pairs is off, minimal, basic or full"),
                },
                _ => skip(got, "not true, false or a table"),
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
    carried
}

/// A helix sub-table of `[editor]` (`[editor.soft-wrap]`): `take` carries
/// what it can and answers whether it did; the rest is listed.
fn sub_table(
    got: &mut Carried,
    key: &str,
    value: &toml::Value,
    take: &mut dyn FnMut(&mut Carried, &str, &toml::Value) -> bool,
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

/// A TOML basic string.
pub fn quote(text: &str) -> String {
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
