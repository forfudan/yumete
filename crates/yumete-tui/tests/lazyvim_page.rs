//! **The "Coming from LazyVim" page's config is one a reader can paste**:
//! it loads with nothing to say about it, and its keys reach
//! what the page says they reach.
//!
//! A binary of its own: applying a config sets process-wide things (the
//! interface language, the ambiguous width).

use yumete_config::Config;
use yumete_core::editor::Editor;
use yumete_core::input::Key;

/// The page's `toml` blocks, joined: the menu, then the keys outside it.
fn the_page_config() -> String {
    let root = match std::env::var_os("YUMETE_DOCS") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../yumete-docs"),
    };
    let page = std::fs::read_to_string(root.join("src/from/lazyvim.md"))
        .expect("the docs site beside yumete, with from/lazyvim.md");
    let mut out = String::new();
    let mut taking = false;
    for line in page.lines() {
        match (taking, line.trim()) {
            (false, "```toml") => taking = true,
            (true, "```") => taking = false,
            (true, _) => {
                out.push_str(line);
                out.push('\n');
            }
            _ => {}
        }
    }
    out
}

#[test]
fn the_lazyvim_config_loads_clean_and_its_keys_work() {
    let text = the_page_config();
    let faults: Vec<String> = yumete_config::faults::faults_in(&text).into_iter().map(|f| f.said).collect();
    assert!(faults.is_empty(), "the page's config has faults:\n{}", faults.join("\n"));
    let (config, said) = Config::from_toml_saying(&text);
    assert!(said.is_empty(), "the page's config is not clean:\n{}", said.join("\n"));
    assert!(text.contains("[keys.normal.space.f]"), "the menu groups are there");

    let mut editor = Editor::new();
    yumete_tui::settings::apply(&config, &mut editor, None, Some(yumete_core::messages::Language::English));
    let press = |editor: &mut Editor, keys: &str| {
        for c in keys.chars() {
            editor.on_key(Key::Char(c));
        }
    };
    // `Space Space` and `Space f f`: the file picker, as LazyVim's.
    for keys in ["  ", " ff"] {
        press(&mut editor, keys);
        assert!(editor.picker().is_some(), "{keys:?} opens a picker");
        editor.on_key(Key::Esc);
    }
    // `Space ,`: the open files.
    press(&mut editor, " ,");
    assert!(editor.picker().is_some(), "Space , opens a picker");
    editor.on_key(Key::Esc);
}
