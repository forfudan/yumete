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

    // **And what a scheme is called, in the same test.** `set_schemes` is a
    // `OnceLock`: a process can be told once, so two tests in one binary cannot
    // both hold — the one that ran second found the list already replaced, and
    // the failure landed on whichever lost the race rather than on the cause.
    // Three intermittent reds before that was chased down; one test cannot
    // race itself.


    let offered = complete(":yume scheme ");
    let notes: Vec<Option<&str>> = offered.iter().map(|c| c.note).collect();
    assert_eq!(notes, [Some("冰雪四拼"), Some("冰雪三拼")]);

    // Searchable by the note: 「四拼」 is not a prefix of `snow-sipin` and not
    // in it at all, and it is what the reader knows the scheme as.
    let by_name: Vec<&str> = complete(":yume scheme 四拼")
        .iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(by_name, ["snow-sipin"]);

    // …and what Tab writes is still the tag.
    assert_eq!(
        complete(":yume scheme 四拼")[0].written(),
        "snow-sipin",
        "the note is how you find it, not what you type"
    );

    // Nothing else in the tree grows one: a note repeating the word beside it
    // would be noise on every row to save one.
    assert!(
        complete(":view ").iter().all(|c| c.note.is_none()),
        "only an identifier needs a name beside it"
    );
}
