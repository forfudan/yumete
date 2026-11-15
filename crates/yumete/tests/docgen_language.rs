//! **The site's generated tables are written in each language through and
//! through**: the English command table has no Chinese placeholder in its
//! arguments column, and the Chinese pages have Chinese headers.
//!
//! Driven through the real binary, into a directory of its own, so what is
//! checked is what `ye --docgen` writes.

use std::process::Command;

/// A Han character, kana, CJK punctuation or a full-width form other than
/// `｜`, which separates a hint's choices in every language.
fn cjk(text: &str) -> bool {
    text.chars().any(|c| {
        ('\u{3000}'..='\u{9fff}').contains(&c)
            || ('\u{f900}'..='\u{faff}').contains(&c)
            || (('\u{ff01}'..='\u{ff5e}').contains(&c) && c != '｜')
    })
}

#[test]
fn generated_tables_speak_their_language() {
    let dir = std::env::temp_dir().join(format!("yumete-docgen-language-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_yumete"))
        .arg(format!("--docgen={}", dir.display()))
        .output()
        .expect("the binary to run");
    assert!(out.status.success(), "--docgen: {}", String::from_utf8_lossy(&out.stderr));
    let read = |page: &str| std::fs::read_to_string(dir.join(page)).expect("a generated page");

    let commands = read("en/commands.md");
    let rows: Vec<&str> = commands.lines().filter(|l| l.starts_with("| `:")).collect();
    assert!(rows.len() > 100, "only {} command rows", rows.len());
    let chinese: Vec<&str> = rows
        .iter()
        .copied()
        .filter(|row| row.split(" | ").nth(1).is_some_and(cjk))
        .collect();
    assert!(chinese.is_empty(), "Chinese in the English arguments column:\n{}", chinese.join("\n"));
    assert!(commands.contains("| `:goto` | `<line number>` |"), "{}", rows[..3].join("\n"));

    let zhs = read("zhs/commands.md");
    assert!(zhs.contains("| 命令 | 参数 | 作用 |"), "the Simplified table's header");
    assert!(zhs.contains("| `:goto` | `<行号>` |"));
    assert!(read("zht/commands.md").contains("| `:goto` | `<行號>` |"));
    assert!(read("zhs/keymap-helix.md").contains("## 普通模式"));
    let _ = std::fs::remove_dir_all(&dir);
}
