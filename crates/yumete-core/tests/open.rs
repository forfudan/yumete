//! Integration test for Feature #1: opening a real file from disk into the
//! editor and reading its contents back through the [`TextStore`] API.

use std::fs;

use yumete_core::{Editor, TextStore};

/// Build a unique temp path in the OS temp dir (avoids extra dev-dependencies).
fn temp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    p.push(format!("yumete-test-{pid}-{name}"));
    p
}

#[test]
fn opens_a_file_and_reads_its_lines() {
    let path = temp_path("open.md");
    let contents = "# 標題\n第一行\n第二行\n";
    fs::write(&path, contents).expect("write temp file");

    let mut editor = Editor::new();
    editor.open_file(&path).expect("open file");

    let buf = editor.current_buffer();
    assert_eq!(buf.path(), Some(path.as_path()));
    let expected_name = path.file_name().unwrap().to_string_lossy();
    assert_eq!(buf.display_name(), expected_name);
    assert_eq!(buf.line(0).as_deref(), Some("# 標題"));
    assert_eq!(buf.line(1).as_deref(), Some("第一行"));
    assert_eq!(buf.line(2).as_deref(), Some("第二行"));
    assert_eq!(buf.text(), contents);
    // Opening replaced the initial pristine scratch buffer.
    assert_eq!(editor.buffer_count(), 1);

    fs::remove_file(&path).ok();
}

#[test]
fn open_command_then_new_buffer_stacks() {
    let path = temp_path("stack.txt");
    fs::write(&path, "hello").expect("write temp file");

    let mut editor = Editor::new();
    editor
        .execute(&format!(":open {}", path.display()))
        .expect("open command");
    editor.new_buffer();

    // The scratch buffer added after a real file is a second buffer.
    assert_eq!(editor.buffer_count(), 2);
    assert_eq!(editor.current_buffer().display_name(), "[scratch]");

    fs::remove_file(&path).ok();
}
