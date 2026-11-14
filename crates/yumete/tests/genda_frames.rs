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
fn a_finished_segment_says_a_short_line_and_adds_a_row_to_the_results() {
    let dir = fixture("finish");
    let frame = shot(&dir, "100x30", r":genda\n\e:genda-length 10\nabcdefghiX\bjklmnoXqrst");
    let rows: Vec<&str> = frame.lines().collect();
    // The next segment took the keys at once, so `y` waits for Esc.
    assert_eq!(rows[29].trim_end(), " 第2段 速度166.67/333.33 · Esc y 复制成绩", "{frame}");
    let head = rows.iter().position(|r| r.contains("╭ 成绩")).expect("the results panel") + 1;
    assert!(rows[head].contains("段  速度") && rows[head].contains("回改"), "{frame}");
    // Newest first; no key fields under an input method the editor cannot see.
    assert!(rows[head + 1].contains("│ 2   166.67/333.33  —"), "{frame}");
    assert!(rows[head + 2].contains("│ 1   272.73 ") && rows[head + 2].contains("0:02.200  1 "), "{frame}");
    assert!(frame.contains("第 3/4 段"), "{frame}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn esc_clears_a_code_then_pauses_and_only_q_closes() {
    let dir = fixture("esc");
    let frame = shot(&dir, "100x30", r":genda\n\{ime}tian\e");
    assert!(!frame.contains("│tian") && !frame.contains("已暂停"), "{frame}");
    let frame = shot(&dir, "100x30", r":genda\n\{ime}tian\e\e\e");
    assert!(frame.contains("已暂停") && frame.contains("Enter 继续  q 关闭"), "{frame}");
    let frame = shot(&dir, "100x30", r":genda\n\{ime}tian\e\eq");
    assert!(!frame.contains("本段"), "{frame}");
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
fn yumes_keys_give_the_equivalence_and_with_auto_off_a_finished_segment_stays() {
    let dir = fixture("yume");
    std::fs::write(dir.join("article.txt"), "天地玄黄，宇宙洪荒。日月盈昃，辰宿列张。\n").expect("an article");
    let typing = r":genda\n\e:genda-length 10\n\e:genda-auto off\n\n\{ime}tiandi xuanhuang ,yuzhou honghuang .";
    let frame = shot(&dir, "100x30", typing);
    let rows: Vec<&str> = frame.lines().collect();
    assert_eq!(rows[29].trim_end(), " 第1段 速度85.71 · y 复制成绩", "{frame}");
    assert!(rows[4].contains("当量") && !rows[4].contains('—'), "{frame}");
    // The results row has the key fields the built-in IME counted.
    let head = rows.iter().position(|r| r.contains("╭ 成绩")).expect("the results panel") + 1;
    assert!(rows[head].contains("当量") && rows[head + 1].contains("│ 1 ") && !rows[head + 1].contains('—'), "{frame}");
    // Waiting for Enter: the segment typed is still in the pad, and the article box in hand.
    assert!(frame.contains("│ 天地玄黄，宇宙洪荒。") && frame.contains("第 1/2 段"), "{frame}");
    assert!(rows[28].contains("Enter 继续"), "{frame}");
    let frame = shot(&dir, "100x30", &format!(r"{typing}\t"));
    assert!(frame.lines().next().unwrap_or_default().contains("今日 10字"), "{frame}");
    assert!(frame.contains("当量 1.") && frame.contains("常用键 N×5") && frame.contains("峰值 ") && frame.contains("停留 ["), "{frame}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_family_needs_the_page_open() {
    let dir = fixture("closed");
    let frame = shot(&dir, "60x8", r":genda-next\n");
    assert!(frame.contains("跟打没有打开"), "{frame}");
    let frame = shot(&dir, "60x8", r":genda\n\e\e");
    assert!(frame.contains("已暂停") && frame.contains("速度"), "{frame}");
    let frame = shot(&dir, "60x8", r":genda\n\e\eq");
    assert!(frame.contains("abcdefghij") && !frame.contains("本段"), "{frame}");
    let _ = std::fs::remove_dir_all(&dir);
}
