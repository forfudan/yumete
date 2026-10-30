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
