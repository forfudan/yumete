//! **Asking which block one line is in must cost one lookup** (#322).
//!
//! `blocks_through(last)` hands back a copy of the block of every line from the
//! top of the buffer down to `last`. That is the right shape for a page, which
//! reads all of them; it is the wrong shape for a caller that wants one, and on
//! 資治通鑑 the wrong shape is sixty-three thousand elements allocated to index
//! one of them — every frame, because the panel that asked ran every frame.
//!
//! `block_of(line)` is the same answer out of the same cache with no copy. The
//! antipattern is invisible in review (`blocks_through(l).get(l)` reads exactly
//! like what it means) and it came back twice after being fixed once, so it is
//! read out of the source here rather than remembered.

/// Every `.rs` file of the workspace's source.
fn sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let mut out = Vec::new();
    let mut todo = vec![root.join("crates")];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).expect("a crate directory").flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                todo.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("a source file");
                out.push((path.display().to_string(), text));
            }
        }
    }
    assert!(out.len() > 20, "the source was not found: {}", out.len());
    out
}

#[test]
fn nobody_copies_every_block_to_read_one() {
    let mut found = Vec::new();
    for (name, text) in sources() {
        if name.ends_with("one_line_one_lookup.rs") {
            continue;
        }
        for (at, _) in text.match_indices("blocks_through(") {
            // The statement it stands in. A caller that binds the whole run to
            // a name is reading the whole run, which is what it is for.
            let head = text[..at].rfind('\n').map_or(0, |n| n + 1);
            if text[head..at].trim_start().starts_with("//") {
                // Prose about the antipattern, including the sentence above.
                continue;
            }
            let rest = &text[at..];
            let statement = &rest[..rest.find(';').unwrap_or(rest.len())];
            if statement.contains(".get(") {
                let line = text[..at].matches('\n').count() + 1;
                found.push(format!("{name}:{line}"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "one line's block is `block_of(line)`, not a copy of every line above \
         it — {found:?}",
    );
}
