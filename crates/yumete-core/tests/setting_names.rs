//! `:settings` completes a setting by the name the panel shows as well as by
//! its TOML path.
//!
//! Its own test binary: [`command::set_setting_names`] writes a process-wide
//! `OnceLock`, and the language is a global too.

use yumete_core::command::{self, complete_at, parse, Command};
use yumete_core::messages::{self, Language};

#[test]
fn a_setting_completes_by_its_name_on_the_panel() {
    command::set_setting_names(&[
        ("editor.indent", "settings.editor.indent"),
        ("editor.indent_guides", "settings.editor.indent-guides"),
        ("editor.scrolloff", "settings.editor.scrolloff"),
    ]);
    let offered = |line: &str| -> (usize, Vec<(String, Option<String>)>) {
        let (at, choices) = complete_at(line);
        (at, choices.iter().map(|c| (c.name.to_string(), c.note.map(str::to_string))).collect())
    };

    messages::set_language(Language::Simplified);
    // The name in the language in force, its path beside it; the one it
    // begins first.
    let (at, rows) = offered(":settings 缩进");
    assert_eq!(at, ":settings ".len());
    assert_eq!(
        rows,
        [("缩进参考线".to_string(), Some("editor.indent_guides".to_string())), ("首行缩进".to_string(), Some("editor.indent".to_string()))]
    );
    // Typed in the other script, offered in this one.
    assert_eq!(offered(":settings 首行縮").1, [("首行缩进".to_string(), Some("editor.indent".to_string()))]);
    // The path still completes, and a path is not offered twice by a name.
    assert_eq!(offered(":settings editor.scr").1, [("editor.scrolloff".to_string(), None)]);

    messages::set_language(Language::English);
    // A name with a space in it is one argument, typed on past the space.
    let (at, rows) = offered(":settings first-line in");
    assert_eq!(at, ":settings ".len());
    assert_eq!(rows, [("first-line indent".to_string(), Some("editor.indent".to_string()))]);
    assert_eq!(offered(":set FIRST").1, [("first-line indent".to_string(), Some("editor.indent".to_string()))]);
    // What the completion writes is what the panel goes to.
    assert_eq!(parse(":settings first-line indent"), Ok(Command::Settings(Some("first-line indent".to_string()))));

    messages::set_language(Language::Traditional);
}
