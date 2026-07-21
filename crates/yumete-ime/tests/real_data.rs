//! Integration test against the real, installed IME data (Feature #27/#32).
//!
//! Skipped automatically when the compiled tables are not present in the user
//! data directory (`~/.local/share/yumete` or `$XDG_DATA_HOME/yumete`), so it is
//! safe to run anywhere. Populate the data with `scripts/build.sh`.

use std::path::PathBuf;

use yumete_ime::{ImeSession, Scheme};

fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    let dir = base.join("yumete");
    if dir.join("ling.ytab").is_file() {
        Some(dir)
    } else {
        None
    }
}

#[test]
fn real_lingming_data_loads_and_produces_candidates() {
    let Some(dir) = data_dir() else {
        eprintln!("skipping: no installed IME data (run scripts/build.sh)");
        return;
    };

    let mut session = ImeSession::new(Scheme::Lingming, vec![dir]);
    assert!(session.available(), "Lingming tables should load");
    assert_eq!(session.scheme_name(), "靈明");

    // Some single letter must yield at least one candidate from the real table.
    let mut produced = false;
    for letter in 'a'..='y' {
        session.escape();
        session.input(letter);
        if !session.page_candidates().is_empty() {
            produced = true;
            break;
        }
    }
    assert!(
        produced,
        "no candidates for any single letter — table not loaded?"
    );
}

#[test]
fn real_pinyin_scheme_switch_stays_available() {
    let Some(dir) = data_dir() else {
        return;
    };
    let mut session = ImeSession::new(Scheme::Lingming, vec![dir]);
    // Pinyin uses the shared fluency table, which build.sh also installs.
    assert!(session.set_scheme(Scheme::Pinyin));
    assert_eq!(session.scheme_name(), "拼音");
}
