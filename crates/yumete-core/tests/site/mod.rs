//! **The site's English pages**, for the tests that hold the documents to the
//! editor (2026-10-30: the manual moved to the yumete-docs site).
//!
//! The site is its own repository, checked out beside this one as
//! `../yumete-docs`, or wherever `YUMETE_DOCS` points. Without it these tests
//! fail rather than pass on nothing: a net that reads no pages catches nothing
//! and says nothing.

use std::path::{Path, PathBuf};

/// Where the site's repository is.
pub fn root() -> PathBuf {
    match std::env::var_os("YUMETE_DOCS") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../yumete-docs"),
    }
}

/// Every English page, with its path under the site's repository. The
/// generated tables come along: they are what a reader sees on the page that
/// includes them.
pub fn pages() -> Vec<(String, String)> {
    let root = root();
    let src = root.join("src");
    assert!(
        src.is_dir(),
        "the docs site is not at {} — clone yumete-docs beside yumete, or set YUMETE_DOCS",
        root.display()
    );
    let mut found = Vec::new();
    walk(&src, &mut found);
    found.sort();
    let pages: Vec<(String, String)> = found
        .into_iter()
        .map(|path| {
            let name = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
            let text = std::fs::read_to_string(&path).expect("a page");
            (name, text)
        })
        .collect();
    assert!(pages.len() > 20, "only {} pages under {}", pages.len(), src.display());
    pages
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // The translations are not written yet; the build's own files are
            // not pages.
            if !matches!(name.as_ref(), "zht" | "zhs" | ".vitepress" | "public" | "node_modules") {
                walk(&path, found);
            }
        } else if name.ends_with(".md") {
            found.push(path);
        }
    }
}
