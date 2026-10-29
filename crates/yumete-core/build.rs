//! Embed the same Markdown sources used by mdBook; installed help needs no checkout.
use std::{env, fs, path::Path};
fn collect(dir: &Path, base: &Path, out: &mut String) {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect(&path, base, out);
        } else if path.extension().is_some_and(|s| s == "md") {
            let id = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let absolute = fs::canonicalize(&path).unwrap();
            let mut body = fs::read_to_string(&absolute).unwrap();
            while let Some(start) = body.find("{{#include ") {
                let end = body[start..].find("}}").unwrap() + start;
                let include = body[start + 11..end].trim();
                let included = absolute.parent().unwrap().join(include);
                println!("cargo:rerun-if-changed={}", included.display());
                let text = fs::read_to_string(included).unwrap();
                body.replace_range(start..end + 2, &text);
            }
            if id == "en/project.md" {
                body = body
                    .replace("(docs/manual.md)", "(index.md)")
                    .replace("(docs/en/", "(")
                    .replace("(CHANGELOG.md)", "(changelog.md)")
                    .replace("(LICENSE)", "(licenses.md)")
                    .replace("(licenses/)", "(licenses.md)")
                    .replace(
                        "(crates/",
                        "(https://github.com/forfudan/yumete/tree/master/crates/",
                    );
            }
            out.push_str(&format!("({id:?}, {body:?}),\n"));
        }
    }
}
fn main() {
    let root = Path::new("../../docs");
    let mut out = String::from("pub const PAGES: &[(&str, &str)] = &[\n");
    for lang in ["en", "zh-Hans", "zh-Hant"] {
        let dir = root.join(lang);
        println!("cargo:rerun-if-changed={}", dir.display());
        if dir.exists() {
            collect(&dir, root, &mut out);
        }
    }
    out.push_str("];\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("help_pages.rs"),
        out,
    )
    .unwrap();
}
