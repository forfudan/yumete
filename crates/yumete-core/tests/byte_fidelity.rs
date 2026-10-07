//! What the file was, coming back (#309, #310).
//!
//! Reading a file and writing it straight back was already exact — CRLF, a
//! missing final newline, even mixed endings, byte for byte. Two things broke
//! it: a byte-order mark was dropped on the way *in* and never restored, so
//! opening and saving lost three bytes with nothing edited; and every line the
//! editor added was a literal `\n`, so one `o` left a CRLF manuscript with two
//! kinds of line in it.

use yumete_core::{editor::Editor, Key};

/// A directory of its own for every call — two tests naming the same file
/// share a process, and so would share the directory.
static NTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn open(name: &str, bytes: &[u8]) -> (Editor, std::path::PathBuf) {
    let nth = NTH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("yumete-fidelity-{}-{nth}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    let mut ed = Editor::new();
    ed.open_file(&path).unwrap();
    (ed, path)
}

#[test]
fn opening_and_saving_changes_not_one_byte() {
    for (name, bytes) in [
        ("crlf.md", &b"line one\r\nline two\r\n"[..]),
        ("no-final-newline.md", &b"line one\r\nline two"[..]),
        ("mixed.md", &b"one\r\ntwo\nthree\r\n"[..]),
        // Three bytes nobody would miss in prose — and the thing Excel reads
        // to decide a `.csv` is UTF-8. Without them 田中 comes back mojibake
        // in a file its writer never touched.
        ("bom.md", "\u{feff}甲乙\n".as_bytes()),
        ("bom.csv", "\u{feff}a,b\n".as_bytes()),
        ("bom-crlf.csv", "\u{feff}a,b\r\n".as_bytes()),
    ] {
        let (mut ed, path) = open(name, bytes);
        ed.execute("write").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{name}");
    }
}

#[test]
fn a_line_the_editor_adds_ends_the_way_the_file_does() {
    for (name, bytes, want) in [
        (
            "crlf.md",
            &b"one\r\ntwo\r\n"[..],
            &b"one\r\ntwo\r\nx\r\n"[..],
        ),
        ("lf.md", &b"one\ntwo\n"[..], &b"one\ntwo\nx\n"[..]),
        // Mixed already: the commonest wins, and a tie goes to CRLF, because
        // somebody's tool has mixed them and the only question is which to
        // join.
        (
            "mixed.md",
            &b"one\r\ntwo\r\nthree\n"[..],
            &b"one\r\ntwo\r\nx\r\nthree\n"[..],
        ),
    ] {
        let (mut ed, path) = open(name, bytes);
        ed.execute("2").unwrap();
        ed.on_key(Key::Char('o'));
        ed.on_key(Key::Char('x'));
        ed.on_key(Key::Esc);
        ed.execute("write").unwrap();
        assert_eq!(
            String::from_utf8(std::fs::read(&path).unwrap()).unwrap(),
            String::from_utf8(want.to_vec()).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn enter_in_insert_mode_ends_the_line_the_same_way() {
    let (mut ed, path) = open("crlf.md", b"one\r\ntwo\r\n");
    ed.execute("1").unwrap();
    ed.on_key(Key::Char('A'));
    ed.on_key(Key::Enter);
    ed.on_key(Key::Char('x'));
    ed.on_key(Key::Esc);
    ed.execute("write").unwrap();
    assert_eq!(
        String::from_utf8(std::fs::read(&path).unwrap()).unwrap(),
        "one\r\nx\r\ntwo\r\n"
    );
}

/// **A line break is one break, even when it is two characters** (丟字第二輪,
/// 2026-10-07).
///
/// `J` and `gJ` deleted one character at [`line_end`], and on a CRLF file that
/// character is the `\r`: the two lines were not joined at all, the `\n` stayed
/// behind, and the manuscript came back with one line in LF and the rest in
/// CRLF. Nothing said so — the page looked the same, because the view strips
/// `\r` either way. A writer joining lines down a chapter would have converted
/// its endings one line at a time.
///
/// [`line_end`]: yumete_core::motion::line_end
#[test]
fn joining_two_lines_takes_the_whole_break_however_long_it_is() {
    // helix `J`: the seam between two 漢字 is empty, so this is the join alone.
    let (mut ed, path) = open("crlf-join.md", "甲\r\n乙\r\n丙\r\n".as_bytes());
    ed.on_key(Key::Char('J'));
    ed.execute("write").unwrap();
    assert_eq!(
        String::from_utf8(std::fs::read(&path).unwrap()).unwrap(),
        "甲乙\r\n丙\r\n",
        "helix J on a CRLF file"
    );

    // vim `gJ`: 「換行符拿掉，別的一個字節不動」 — and the whole break is the
    // line break.
    let (mut ed, path) = open("crlf-join-raw.md", "甲\r\n乙\r\n丙\r\n".as_bytes());
    ed.execute("keymap vim").unwrap();
    ed.on_key(Key::Char('g'));
    ed.on_key(Key::Char('J'));
    ed.execute("write").unwrap();
    assert_eq!(
        String::from_utf8(std::fs::read(&path).unwrap()).unwrap(),
        "甲乙\r\n丙\r\n",
        "vim gJ on a CRLF file"
    );

    // **The seam is still worked out**, and a CRLF file must not read its own
    // `\n` as 「the next line is blank」: between two Latin words the space is
    // kept, and a genuinely blank next line still gets no space.
    let (mut ed, path) = open("crlf-seam.md", "hello\r\nworld\r\n".as_bytes());
    ed.on_key(Key::Char('J'));
    ed.execute("write").unwrap();
    assert_eq!(
        String::from_utf8(std::fs::read(&path).unwrap()).unwrap(),
        "hello world\r\n"
    );

    let (mut ed, path) = open("crlf-blank.md", "hello\r\n\r\nworld\r\n".as_bytes());
    ed.on_key(Key::Char('J'));
    ed.execute("write").unwrap();
    assert_eq!(
        String::from_utf8(std::fs::read(&path).unwrap()).unwrap(),
        "hello\r\nworld\r\n",
        "no space from a blank line"
    );
}
