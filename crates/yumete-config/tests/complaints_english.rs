//! **A complaint in English, when the reader speaks English.**
//!
//! Its own binary on purpose: which language the complaints come in is a
//! process-wide switch (`yumete_cjk::set_speaks_english`), and a test that
//! flips it inside the library's test binary turns a neighbour red whenever
//! the scheduler runs the two side by side — `a_misspelled_key_is_reported…`
//! was that neighbour on 2026-10-30.

use yumete_config::faults::faults_in;

#[test]
fn the_complaint_lists_the_names_in_english() {
    yumete_cjk::set_speaks_english(true);
    let problems: Vec<String> = faults_in("[editor]\ntabs = \"maybe\"\n[keys]\npreset = \"emacs\"\n")
        .into_iter()
        .map(|f| f.said)
        .collect();
    assert_eq!(
        problems,
        [
            "[editor] tabs = \"maybe\": only auto, always, or never",
            "[keys] preset = \"emacs\": only helix or vim",
        ]
    );
}
