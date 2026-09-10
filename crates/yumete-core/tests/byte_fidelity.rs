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
