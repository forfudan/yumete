//! **Word files both ways**: `ye --export docx` writes a file other
//! programs read back with its structure, and `ye --import-word` reads one
//! into Markdown without guessing at what it does not say.
//!
//! The readers are the ones on the machine: pandoc and macOS's `textutil`.
//! A test whose tool is missing says so and passes, as the PDF tests do for
//! Typst.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("yumete-word-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a fixture dir");
    root
}

fn ye() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_yumete"));
    cmd.env("HOME", std::env::temp_dir().join("yumete-word-cli-home"));
    cmd.env("XDG_CONFIG_HOME", std::env::temp_dir().join("yumete-word-cli-home/.config"));
    cmd.env("XDG_DATA_HOME", std::env::temp_dir().join("yumete-word-cli-home/.local/share"));
    cmd.env("LANG", "en_US.UTF-8");
    cmd
}

fn has(tool: &str, probe: &[&str]) -> bool {
    let found = Command::new(tool).args(probe).output().is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("{tool} is not installed; this part is not checked");
    }
    found
}

fn run(cmd: &mut Command) -> String {
    let out = cmd.output().expect("the program to run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const MANUSCRIPT: &str = "---
title: 样稿
author: 某人
date: 2024-03-01
---

# 第一章

春天来了，**花开了**，*风*也~~冷~~暖了。见[链接](https://example.com)。

这里有{漢字|hàn zì}注音，脚注在此[^1]。

## 列表

- 第一项
- 第二项
  1. 子项一
  2. 子项二

> 引文一段。

| 名 | 数 |
|----|----|
| 甲 | 1 |

```
code line
```

[^1]: 这是脚注。
";

/// pandoc and textutil both read the exported file, and pandoc's Markdown
/// of it has the manuscript's structure back.
#[test]
fn an_exported_docx_reads_back_with_its_structure() {
    let root = fixture("export");
    let md = root.join("ch.md");
    std::fs::write(&md, MANUSCRIPT).unwrap();
    let said = run(ye().args(["--export", "docx"]).arg(&md));
    let docx = md.with_extension("docx");
    assert_eq!(said.trim(), docx.display().to_string());
    assert_eq!(&std::fs::read(&docx).unwrap()[..2], b"PK");
    if has("pandoc", &["--version"]) {
        let gfm = run(Command::new("pandoc").args(["-f", "docx", "-t", "gfm", "--wrap=none"]).arg(&docx));
        for piece in [
            "# 第一章",
            "## 列表",
            "**花开了**",
            "*风*",
            "~~冷~~",
            "[链接](https://example.com)",
            "[^1]: 这是脚注。",
            "- 第一项",
            "1.  子项一",
            "> 引文一段。",
            "| 甲 ",
            "code line",
        ] {
            assert!(gfm.contains(piece), "{piece:?} in\n{gfm}");
        }
        assert!(!gfm.contains("title:") && !gfm.contains("样稿"), "front matter is not printed:\n{gfm}");
    }
    if Path::new("/usr/bin/textutil").exists() {
        let html = run(Command::new("textutil").args(["-convert", "html", "-stdout"]).arg(&docx));
        assert!(html.contains("<title>样稿</title>"), "the title property:\n{html}");
        assert!(html.contains("<meta name=\"Author\" content=\"某人\">"), "{html}");
        assert!(html.contains("2024-03-01T00:00:00Z"), "{html}");
        // The phonetic guide: its reading set small over 漢字.
        assert!(html.contains("hàn zì</span>漢字"), "{html}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A `.docx` built by hand, the way Word writes the things a converter is
/// tempted to guess at.
fn handmade_docx(path: &Path) {
    use std::io::Write;
    const W: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"";
    let para = |style: Option<&str>, ppr: &str, runs: &str| {
        let style = style.map(|s| format!("<w:pStyle w:val=\"{s}\"/>")).unwrap_or_default();
        format!("<w:p><w:pPr>{style}{ppr}</w:pPr>{runs}</w:p>")
    };
    let run = |rpr: &str, text: &str| format!("<w:r><w:rPr>{rpr}</w:rPr><w:t xml:space=\"preserve\">{text}</w:t></w:r>");
    let body = [
        para(Some("Title"), "", &run("", "A Title Paragraph")),
        para(None, "", &run("<w:b/>", "A bold first line")),
        para(None, "", &run("", "Short line one")),
        para(None, "", &run("", "Short line two")),
        para(None, "<w:ind w:left=\"720\"/>", &run("", "An indented paragraph")),
        para(None, "", &run("<w:b/><w:sz w:val=\"48\"/>", "Large bold type")),
        para(Some("Heading2"), "", &run("", "A real heading")),
        para(Some("Quote"), "", &run("", "A quotation by style")),
        para(
            None,
            "",
            "<w:r><w:ruby><w:rubyPr/><w:rt><w:r><w:t>hàn zì</w:t></w:r></w:rt><w:rubyBase><w:r><w:t>漢字</w:t></w:r></w:rubyBase></w:ruby></w:r>",
        ),
    ]
    .concat();
    let styles = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:styles {W}>\
         <w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/></w:style></w:styles>"
    );
    let files = [
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
             <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
             <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
             <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
             <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
             <Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/></Types>"
                .to_string(),
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
             <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>"
                .to_string(),
        ),
        (
            "word/_rels/document.xml.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>"
                .to_string(),
        ),
        ("word/document.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document {W}><w:body>{body}</w:body></w:document>")),
        ("word/styles.xml", styles),
        (
            "docProps/core.xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
             xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
             <dc:title>The Property Title</dc:title><dc:creator>A. Writer</dc:creator><dc:subject></dc:subject>\
             <dcterms:created xsi:type=\"dcterms:W3CDTF\">2008-05-06T07:08:00Z</dcterms:created></cp:coreProperties>"
                .to_string(),
        ),
    ];
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, text) in files {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

/// Each Word paragraph stays one paragraph, a heading is only a Heading
/// style, and the file's properties are the front matter.
#[test]
fn an_imported_docx_guesses_nothing() {
    if !has("pandoc", &["--version"]) {
        return;
    }
    let root = fixture("import");
    let docx = root.join("letter.docx");
    handmade_docx(&docx);
    let before = std::fs::read(&docx).unwrap();
    let said = run(ye().arg("--import-word").arg(&docx));
    let md = root.join("letter.md");
    assert_eq!(said.trim(), md.display().to_string());
    let text = std::fs::read_to_string(&md).unwrap();
    let expected = "---
title: \"The Property Title\"
author: \"A. Writer\"
created: 2008-05-06T07:08:00Z
source: \"letter.docx\"
---

A Title Paragraph

**A bold first line**

Short line one

Short line two

An indented paragraph

**Large bold type**

## A real heading

> A quotation by style

{漢字|hàn zì}
";
    assert_eq!(text, expected);
    assert_eq!(std::fs::read(&docx).unwrap(), before, "the source is only read");

    // Not written over without the flag; written over with it.
    std::fs::write(&md, "mine\n").unwrap();
    let out = ye().arg("--import-word").arg(&docx).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--overwrite"), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read_to_string(&md).unwrap(), "mine\n");
    run(ye().args(["--import-word", "--overwrite"]).arg(&docx));
    assert_eq!(std::fs::read_to_string(&md).unwrap(), expected);
    let _ = std::fs::remove_dir_all(&root);
}

/// A folder converts every Word file under it into the same tree under
/// `--out`, and a `.doc` goes through a converter first.
#[test]
fn a_folder_converts_into_a_mirrored_tree() {
    if !has("pandoc", &["--version"]) {
        return;
    }
    let root = fixture("folder");
    std::fs::create_dir_all(root.join("in/part")).unwrap();
    handmade_docx(&root.join("in/one.docx"));
    let md = root.join("in/part/two.md");
    std::fs::write(&md, "# 第二篇\n\n正文一段。\n").unwrap();
    run(ye().args(["--export", "docx"]).arg(&md));
    std::fs::remove_file(&md).unwrap();
    let doc = cfg!(target_os = "macos") && Path::new("/usr/bin/textutil").exists();
    if doc {
        run(Command::new("textutil").args(["-convert", "doc", "-output"]).arg(root.join("in/part/three.doc")).arg(root.join("in/one.docx")));
    }
    let out = root.join("md");
    let said = run(ye().arg("--import-word").arg(root.join("in")).arg("--out").arg(&out));
    assert!(out.join("one.md").is_file(), "{said}");
    let two = std::fs::read_to_string(out.join("part/two.md")).unwrap();
    assert!(two.contains("source: \"two.docx\"") && two.contains("# 第二篇\n\n正文一段。"), "{two}");
    assert!(!root.join("in/one.md").exists(), "nothing is written beside the sources with --out");
    if doc {
        let three = std::fs::read_to_string(out.join("part/three.md")).unwrap();
        assert!(three.contains("source: \"three.doc\"") && three.contains("Short line one"), "{three}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn overwrite_belongs_to_import_word() {
    let out = ye().arg("--overwrite").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--import-word"));
}
