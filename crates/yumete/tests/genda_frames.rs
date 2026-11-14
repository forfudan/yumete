//! **跟打** (`:genda`) as a reader sees it: the page drawn by the real
//! binary, `--keys` going through the same seat the loop uses.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A directory of its own for each picture, with an article in it.
fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yumete-genda-frames-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).expect("a directory");
    std::fs::write(dir.join("article.txt"), "abcdefghijklmnopqrstuvwxyz\n天地玄黄，宇宙洪荒。\n").expect("an article");
    dir
}

fn shot(dir: &Path, size: &str, keys: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_yumete"))
        .current_dir(dir)
        .env("HOME", dir.join("home"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .arg("--lang=zhs")
        .arg(format!("--shot={size}"))
        .arg(format!("--keys={keys}"))
        .arg("article.txt")
        .output()
        .expect("the binary to run");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn the_page_at_a_hundred_columns_has_the_three_boxes_beside_the_article() {
    let dir = fixture("wide");
    let frame = shot(&dir, "100x30", r":genda\n");
    let rows: Vec<&str> = frame.lines().collect();
    assert!(rows[0].contains("本段") && rows[0].contains("article.txt"), "{frame}");
    assert!(frame.contains("累计") && frame.contains("跟打 · "), "{frame}");
    assert!(rows[1].contains("abcdefghijklmnopqrstuvwxy"), "{frame}");
    assert!(rows[28].contains("F3 重打"), "{frame}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_narrow_window_folds_the_left_column_into_one_row() {
    let dir = fixture("narrow");
    let frame = shot(&dir, "70x24", r":genda\n");
    let rows: Vec<&str> = frame.lines().collect();
    assert!(rows[0].contains("速度") && rows[0].contains("第 1/2 段"), "{frame}");
    assert!(!frame.contains("累计"), "{frame}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_finished_segment_puts_its_score_line_on_the_command_row() {
    let dir = fixture("finish");
    let frame = shot(&dir, "100x30", r":genda\n\e:genda-length 10\nabcdefghiX\bj");
    let said = frame.lines().nth(29).unwrap_or_default();
    assert!(said.starts_with(" 第1段 速度"), "{frame}");
    assert!(said.contains("回改1") && said.contains("【首打】") && said.contains("输入法:系统"), "{frame}");
    // No key fields under an input method the editor cannot see.
    assert!(!said.contains("击键") && !said.contains("键准"), "{frame}");
    assert!(frame.contains("第 2/4 段"), "{frame}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tab_shows_the_history() {
    let dir = fixture("history");
    let frame = shot(&dir, "100x30", r":genda\n\e:genda-length 10\nabcdefghij\t");
    assert!(frame.lines().next().unwrap_or_default().contains("历史 · 1 条"), "{frame}");
    assert!(frame.contains("article.txt") && frame.contains("j k 选择"), "{frame}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_built_in_ime_composes_in_the_pad_and_its_keys_are_counted() {
    let dir = fixture("ime");
    let frame = shot(&dir, "100x30", r":genda\n\{ime}tian");
    assert!(!frame.contains("跟打 · 系统"), "{frame}");
    // The code is in the candidate box, not in the pad's text.
    assert!(frame.contains("│tian"), "{frame}");
    let speed_row = frame.lines().nth(2).unwrap_or_default();
    assert!(speed_row.contains("击键") && !speed_row.contains('—'), "{frame}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_family_needs_the_page_open() {
    let dir = fixture("closed");
    let frame = shot(&dir, "60x8", r":genda-next\n");
    assert!(frame.contains("跟打没有打开"), "{frame}");
    let frame = shot(&dir, "60x8", r":genda\n\e\e");
    assert!(frame.contains("abcdefghij") && !frame.contains("本段"), "{frame}");
    let _ = std::fs::remove_dir_all(&dir);
}
