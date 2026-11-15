//! **A complaint in 简体, when the reader asked for 简体.**
//!
//! Its own binary for the reason `complaints_english.rs` gives: the language
//! is a process-wide switch, and flipping it inside the library's tests turns
//! a neighbour red.

use yumete_config::faults::faults_in;

#[test]
fn the_complaint_is_in_the_script_the_reader_asked_for() {
    let said = || -> Vec<String> {
        faults_in("[keys.normal]\n\"<space>q\" = 3\n[theme]\nname = \"nope\"\n")
            .into_iter()
            .map(|f| f.said)
            .collect()
    };
    yumete_cjk::set_speaks_simplified(true);
    let simplified = said();
    yumete_cjk::set_speaks_simplified(false);
    let traditional = said();
    assert!(
        simplified.iter().any(|p| p.contains("不是主题的名字")),
        "{simplified:?}"
    );
    assert!(
        traditional.iter().any(|p| p.contains("不是主題的名字")),
        "{traditional:?}"
    );
    // Not one Traditional-only character in the Simplified lines.
    for line in &simplified {
        assert!(!line.contains(['題', '這', '寫', '過', '鍵']), "{line}");
    }
}
