//! `discover()` makes the directory the list of schemes (#169).
//!
//! Its own test binary, and it must stay that way: [`yumete_ime::discover`]
//! writes a `OnceLock` and hands yume-core a factory list that no later call
//! can take back — both of them process-wide, on purpose, since a scheme list
//! that changed under a running editor would leave the menu and the engine
//! disagreeing about what `lingming` means. A test that set it inside the
//! crate's own test binary would therefore set it for every other test there.

use std::path::PathBuf;

use yumete_ime::Scheme;

/// A scheme file with nothing in it but what makes it one.
///
/// yume-core's parser is not a TOML parser — it reads `key = value` lines and
/// the few table arrays it knows — so this is deliberately the minimum that
/// yields a scheme: a tag, a name to show, and the two keys that put it in a
/// place in the menu.
fn scheme_file(tag: &str, name: &str, index: i32) -> String {
    format!("tag = \"{tag}\"\nname = \"{name}\"\nseries = \"試\"\nindex = {index}\n")
}

#[test]
fn what_is_in_the_directory_is_what_the_editor_offers() {
    let dir = std::env::temp_dir().join(format!("yumete-schemes-{}", std::process::id()));
    let schemes = dir.join("schemes");
    std::fs::create_dir_all(&schemes).expect("temp dir");
    // Written out of order, and with the index that says the order: the
    // directory has none, and the menu has to have one.
    std::fs::write(schemes.join("zzz.toml"), scheme_file("zzz", "後面那個", 2)).unwrap();
    std::fs::write(schemes.join("aaa.toml"), scheme_file("aaa", "前面那個", 1)).unwrap();
    // Not a scheme file, and not fatal: a `README` in the directory must not
    // stop the two that are.
    std::fs::write(schemes.join("notes.txt"), "not a scheme").unwrap();

    assert_eq!(yumete_ime::discover(&[dir.clone()]), 2, "both taken");
    let tags: Vec<&str> = Scheme::all().iter().map(|s| s.tag()).collect();
    assert_eq!(tags, ["aaa", "zzz"], "the file's own index orders the menu");
    assert_eq!(Scheme::from_tag("aaa").map(|s| s.found_name()), Some("前面那個"));

    // **A scheme that is not installed is not offered.** This is the half that
    // makes the feature worth having: a build carrying only these two must not
    // put 靈明 on the menu, because picking it would load 靈明's `Schema` over
    // whatever 碼表 was actually there.
    assert_eq!(Scheme::from_tag("lingming"), None);
    assert_eq!(Scheme::from_tag("ling"), None, "an alias is no way in either");

    // Cycling stays inside what was found.
    let first = Scheme::from_tag("aaa").expect("found");
    assert_eq!(first.next().tag(), "zzz");
    assert_eq!(first.next().next().tag(), "aaa");

    // A second scan is ignored — the list is settled for the process.
    assert_eq!(yumete_ime::discover(&[PathBuf::from("/no/such/dir")]), 2);

    let _ = std::fs::remove_dir_all(&dir);
}
