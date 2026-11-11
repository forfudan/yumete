//! **An action or command never asks which keymap is in use.**
//!
//! What a named action or command does is decided by its name, the rules
//! (`yumete_cjk::Rules`, the editing model a keymap brings) and the
//! settings. Which keymap is in use may only decide which key runs which
//! action: the key tables. So `vim_keymap()` is asked only in the functions
//! below, each of which routes keys; anywhere else, ask the rule the
//! behaviour is, or make the action a twin (`vim_…`).

use std::path::Path;

/// The functions that route keys, by file: where `vim_keymap()` belongs.
const ROUTING: &[(&str, &str)] = &[
    // The listing's `helix | vim` columns, the one in use in bold.
    ("editor/commands.rs", "execute"),
    // A group menu's rows, by keymap.
    ("editor/groups.rs", "group_context"),
    // Insert mode's key table.
    ("editor/insert.rs", "INSERT_KEYS"),
    // vim's `]p` is a put, helix's a paragraph move.
    ("editor/keys.rs", "fan_out"),
    // vim's `Z` waits for its second key.
    ("editor/keys.rs", "on_normal_key"),
    // Where the `Space` menu is.
    ("editor/normal.rs", "run_now"),
    // A twin's own answer, else the keymap's twin for a key it routes.
    ("editor/normal.rs", "as_vim"),
    // `:split` and `:vsplit`: which side they open on.
    ("editor/page.rs", "split_window"),
    // vim's `ZZ` and `ZQ` in a preview.
    ("editor/page_preview.rs", "on_page_window_key"),
    // Insert mode's `C-n`.
    ("editor/prompt.rs", "on_insert_key"),
    // A table's `d` `c` `y`: vim's operators.
    ("editor/tables.rs", "table_motion"),
    // The keys typed so far, as the keymap spells them.
    ("editor/wrap.rs", "typed_so_far"),
];

#[test]
fn only_the_key_tables_ask_which_keymap_is_in_use() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut asked = Vec::new();
    walk(&src, &src, &mut asked);
    let strays: Vec<String> = asked
        .iter()
        .filter(|(file, function, _)| !ROUTING.iter().any(|(f, n)| f == file && n == function))
        .map(|(file, function, line)| format!("{file}:{line} in {function}"))
        .collect();
    assert!(
        strays.is_empty(),
        "an action or command asks which keymap is in use; ask a rule, or make it a twin:\n  {}",
        strays.join("\n  ")
    );
}

/// Every `.vim_keymap()` outside tests under `dir`: its file (relative to
/// `root`), the function or table it is in, and its line.
fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String, usize)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out);
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") || path.file_name().is_some_and(|n| n == "tests.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let file = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        let mut current = String::from("?");
        for (n, line) in text.lines().enumerate() {
            if line.starts_with("#[cfg(test)]") || line.trim() == "mod tests {" {
                break;
            }
            if let Some(name) = item_name(line) {
                current = name;
            }
            if line.contains(".vim_keymap()") {
                out.push((file.clone(), current.clone(), n + 1));
            }
        }
    }
}

/// The name a line declares, when it starts a function or a constant.
fn item_name(line: &str) -> Option<String> {
    let words: Vec<&str> = line.split_whitespace().skip_while(|w| w.starts_with("pub")).collect();
    let name = match words.as_slice() {
        ["fn", name, ..] | ["const", "fn", name, ..] => name.split(['(', '<']).next().unwrap_or(name),
        ["const", name, ..] => name.trim_end_matches(':'),
        _ => return None,
    };
    Some(name.to_string())
}
