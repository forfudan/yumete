//! `Args::Schemes` is answered from what the frontend found (#169).
//!
//! Its own test binary: [`command::set_schemes`] writes a process-wide
//! `OnceLock`, so a test that calls it inside the crate's own test binary
//! would change what every other test there sees `:yume scheme` offer.

use yumete_core::command::{self, complete, Args};

#[test]
fn the_command_table_offers_the_schemes_that_were_found() {
    // Before anything looked, the five this crate knows.
    let built_in: Vec<&str> = command::schemes().iter().map(|w| w.name).collect();
    assert!(built_in.contains(&"lingming"), "{built_in:?}");
    assert!(built_in.contains(&"pinyin"), "{built_in:?}");

    command::set_schemes(&[("snow-sipin", "冰雪四拼"), ("snow-sanpin", "冰雪三拼")]);

    let now: Vec<&str> = command::schemes().iter().map(|w| w.name).collect();
    assert_eq!(now, ["snow-sipin", "snow-sanpin"], "found replaces built-in");

    // The completion, the hint and the deep match (#223) all read the same
    // list — the point of routing every reader through `Args::words()`.
    let offered: Vec<String> = complete(":yume scheme ")
        .iter()
        .map(|c| c.name.to_string())
        .collect();
    assert_eq!(offered, ["snow-sipin", "snow-sanpin"]);
    assert_eq!(Args::Schemes.hint(), "snow-sipin｜snow-sanpin");
    assert!(
        complete(":snow-si")
            .iter()
            .any(|c| c.written() == "yume scheme snow-sipin"),
        "a found scheme is findable by its own name alone"
    );

    // The display name stands in for a `help` tag: an unknown tag renders as
    // itself, so 「冰雪四拼」 reads correctly in all three languages.
    assert_eq!(command::schemes()[0].help, "冰雪四拼");

    // Set once. A second call is ignored rather than half-applied.
    command::set_schemes(&[("lingming", "靈明")]);
    assert_eq!(command::schemes()[0].name, "snow-sipin");
}
