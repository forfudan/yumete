//! **`ye --export` writes a file out and says where** (2026-10-30).

use std::process::Command;

fn fixture(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("yumete-export-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a fixture dir");
    root
}

fn ye() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_yumete"));
    // Nothing of this machine's own: not its config, not its data directory.
    cmd.env("HOME", std::env::temp_dir().join("yumete-export-cli-home"));
    cmd.env("XDG_CONFIG_HOME", std::env::temp_dir().join("yumete-export-cli-home/.config"));
    cmd.env("XDG_DATA_HOME", std::env::temp_dir().join("yumete-export-cli-home/.local/share"));
    cmd
}

#[test]
fn html_and_typst_land_beside_the_file_and_the_path_is_printed() {
    let root = fixture("html");
    let md = root.join("ch1.md");
    std::fs::write(&md, "# 第一章\n\n那年冬天。\n").unwrap();
    for (format, extension, mark) in [("html", "html", "<html"), ("typst", "typ", "第一章")] {
        let out = ye().args(["--export", format]).arg(&md).output().expect("the binary to run");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let said = String::from_utf8_lossy(&out.stdout);
        let written = md.with_extension(extension);
        assert_eq!(said.trim(), written.display().to_string());
        assert!(std::fs::read_to_string(&written).unwrap().contains(mark), "{format}");
    }
    // Written over, as a compiler's output is.
    let out = ye().args(["--export", "html"]).arg(&md).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // `--out` names the place.
    let elsewhere = root.join("out").join("page.html");
    std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
    let out = ye().args(["--export", "html", "--out"]).arg(&elsewhere).arg(&md).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(elsewhere.is_file());
    let _ = std::fs::remove_dir_all(&root);
}

/// **`--export epub` writes a book** (2026-11-08): a zip whose first entry is
/// the stored `mimetype`, and a vertical one that turns right to left; and
/// the HTML of a vertical page draws its formula with RaTeX. Vertical is the
/// export's word, not the editor's `--vertical`, which is how the draft is
/// shown.
#[test]
fn epub_is_a_book_and_html_draws_formulas() {
    let root = fixture("epub");
    let md = root.join("ch1.md");
    std::fs::write(&md, "# 第一章\n\n1997年，从 $x^2$ 起。\n").unwrap();
    let out = ye().args(["--vertical", "--export", "epub"]).arg(&md).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&std::fs::read(md.with_extension("epub")).unwrap()).to_string();
    assert!(!text.contains("page-progression-direction"), "the editor's layout is not the export's");
    let out = ye().args(["--export", "epub", "vertical"]).arg(&md).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let book = std::fs::read(md.with_extension("epub")).unwrap();
    assert_eq!(&book[..4], b"PK\x03\x04");
    assert_eq!(&book[30..58], b"mimetypeapplication/epub+zip");
    let text = String::from_utf8_lossy(&book);
    assert!(text.contains("page-progression-direction=\"rtl\""));
    assert!(text.contains("images/f1.png"));
    let out = ye().args(["--export", "html", "jlreq", "page=A4", "vertical"]).arg(&md).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let page = std::fs::read_to_string(md.with_extension("html")).unwrap();
    assert!(page.contains("<span class=\"yoko\">1997</span>年"), "{page}");
    assert!(page.contains("@page { size: 210mm 297mm;"), "{page}");
    assert!(page.contains("<img class=\"math turned\" alt=\"x^2\""), "{page}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_table_file_exports_as_tsv() {
    let root = fixture("tsv");
    let csv = root.join("t.csv");
    std::fs::write(&csv, "a,b\n1,\"x, y\"\n").unwrap();
    let out = ye().args(["--export", "tsv"]).arg(&csv).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read_to_string(csv.with_extension("tsv")).unwrap(), "a\tb\n1\tx, y\n");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_bad_format_and_a_missing_file_say_so() {
    let root = fixture("bad");
    let out = ye().args(["--export", "docx", "x.md"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("docx"));
    let out = ye().args(["--export", "html"]).arg(root.join("nope.md")).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a file"));
    // A word misspelt is said, not looked for as a file.
    let out = ye().args(["--export", "html", "verticle", "x.md"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("verticle"));
    let _ = std::fs::remove_dir_all(&root);
}

/// Only where `typst` is installed; the machine without it is told what to get.
#[test]
fn pdf_goes_through_typst() {
    let have_typst = Command::new("typst").arg("--version").output().is_ok_and(|o| o.status.success());
    let root = fixture("pdf");
    let md = root.join("ch1.md");
    std::fs::write(&md, "# 第一章\n\n那年冬天。\n").unwrap();
    let typ = root.join("book.typ");
    std::fs::write(&typ, "= 第一章\n\n那年冬天。\n").unwrap();
    for source in [&md, &typ] {
        let out = ye().args(["--export", "pdf"]).arg(source).output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        match have_typst {
            true => {
                assert!(out.status.success(), "{stderr}");
                let pdf = source.with_extension("pdf");
                assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"), "{}", pdf.display());
            }
            false => assert!(stderr.contains("typst"), "{stderr}"),
        }
    }
    assert!(!root.join("ch1.yumete-export.typ").exists(), "the draft is removed");
    let _ = std::fs::remove_dir_all(&root);
}

/// **Nothing in one run is written over another file named in it**:
/// `ye --export typst *` in a folder with `ch1.md` and a finished `ch1.typ`
/// leaves the `.typ` as it is, and `a.md` beside `a.typ` do not both go to
/// `a.html`. Both clashes are refused before anything is written; the rest
/// of the batch goes ahead.
#[test]
fn one_run_writes_nothing_over_a_file_it_names() {
    let root = fixture("batch");
    std::fs::write(root.join("ch1.md"), "# 第一章\n\n那年冬天。\n").unwrap();
    std::fs::write(root.join("ch1.typ"), "= 定稿\n").unwrap();
    for order in [["ch1.md", "ch1.typ"], ["ch1.typ", "ch1.md"]] {
        let out = ye().current_dir(&root).args(["--export", "typst"]).args(order).output().unwrap();
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("ch1.md: not exported"), "{stderr}");
        assert_eq!(std::fs::read_to_string(root.join("ch1.typ")).unwrap(), "= 定稿\n");
    }
    std::fs::write(root.join("a.md"), "# 甲\n").unwrap();
    std::fs::write(root.join("a.typ"), "= 乙\n").unwrap();
    std::fs::write(root.join("c.md"), "# 丙\n").unwrap();
    let out = ye().current_dir(&root).args(["--export", "html", "a.md", "a.typ", "c.md"]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("a.md: not exported: it and a.typ"), "{stderr}");
    assert!(stderr.contains("a.typ: not exported: it and a.md"), "{stderr}");
    assert!(!root.join("a.html").exists(), "neither of the two is written");
    assert!(root.join("c.html").is_file(), "the rest of the batch is");
    let _ = std::fs::remove_dir_all(&root);
}

/// **`--out` through a link onto the source is refused for PDF too**:
/// `typst compile` writes where it is told, so the check is made before it
/// is asked.
#[cfg(unix)]
#[test]
fn pdf_is_not_written_through_a_link_onto_the_source() {
    let have_typst = Command::new("typst").arg("--version").output().is_ok_and(|o| o.status.success());
    if !have_typst {
        return;
    }
    let root = fixture("pdf-link");
    let md = root.join("ch1.md");
    std::fs::write(&md, "# 第一章\n\n那年冬天。\n").unwrap();
    std::os::unix::fs::symlink(&md, root.join("out.pdf")).unwrap();
    let out = ye().args(["--export", "pdf", "--out"]).arg(root.join("out.pdf")).arg(&md).output().unwrap();
    assert!(!out.status.success());
    assert_eq!(std::fs::read_to_string(&md).unwrap(), "# 第一章\n\n那年冬天。\n");
    let _ = std::fs::remove_dir_all(&root);
}

/// **What Typst cannot set is said, once**, and `page=` sets its paper.
#[test]
fn a_typst_export_says_what_it_cannot_set_and_takes_the_paper() {
    let root = fixture("typst-words");
    let md = root.join("ch1.md");
    std::fs::write(&md, "# 第一章\n\n那年冬天。\n").unwrap();
    let out = ye().args(["--lang=en", "--export", "typst", "vertical", "page=A6"]).arg(&md).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.matches("vertical does not apply to a Typst export").count(), 1, "{stderr}");
    let typ = std::fs::read_to_string(md.with_extension("typ")).unwrap();
    assert!(typ.contains("#set page(width: 105mm, height: 148mm)"), "{typ}");
    let out = ye().args(["--export", "html", "vertical"]).arg(&md).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
