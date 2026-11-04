//! **The tables the binary carries** (2026-11-04): the full 宇浩拼音, 靈明 and
//! 星陳, with the language layer.
//!
//! A binary of its own, because what is carried is set once per process: in
//! the unit tests nothing is, as on a machine with no data.
//!
//! Warning: **No tables, nothing to prove.** They are fetched at build time
//! (`yumete-tables/build.rs`); a machine that was offline on its first build
//! has none, and then each test says so and passes. A release build fails
//! instead of building without them.

use std::path::PathBuf;
use yumete_ime::{builtin_tables, builtin_version, Carried, ImeSession, Scheme};

fn carry() -> bool {
    yumete_ime::carry(Carried {
        file: yumete_tables::file,
        has: yumete_tables::has,
        names: || yumete_tables::names().collect(),
        version: yumete_tables::VERSION,
    });
    yumete_ime::discover(&[]);
    yumete_tables::VERSION.is_some()
}

/// Nothing installed: an empty search path leaves only what is carried.
fn nowhere() -> Vec<PathBuf> {
    vec![PathBuf::from("/no/such/dir")]
}

#[test]
fn each_carried_scheme_types_with_nothing_installed() {
    if !carry() {
        return;
    }
    assert_eq!(
        builtin_tables(),
        vec![Scheme::PINYIN, Scheme::LINGMING, Scheme::XINGCHEN]
    );
    for (scheme, keys, want) in [
        (Scheme::LINGMING, "e", "的"),
        (Scheme::XINGCHEN, "d", "的"),
        (Scheme::PINYIN, "nihao", "你好"),
    ] {
        let mut ime = ImeSession::new(scheme, nowhere());
        assert!(ime.available(), "{} stands on the carried tables", scheme.tag());
        assert!(ime.is_builtin(), "{} says they are the carried ones", scheme.tag());
        for key in keys.chars() {
            ime.input(key);
        }
        assert_eq!(ime.top_candidate(), want, "{} {keys}", scheme.tag());
        let source = ime.table_source().expect("a table, and where it is from");
        assert!(source.starts_with("出廠自帶"), "{source}");
    }
}

/// Pinyin's sentence model is carried too: without it the sentence below
/// came out in traditional characters (measured 2026-11-04).
#[test]
fn carried_pinyin_has_its_language_model() {
    if !carry() {
        return;
    }
    let mut ime = ImeSession::new(Scheme::PINYIN, nowhere());
    for key in "zhegewentiyijingjiejuele".chars() {
        ime.input(key);
    }
    assert!(ime.problems().iter().all(|p| !p.file.ends_with("lang.ygram")));
    assert!(ime.top_candidate().starts_with("这个问题"), "{}", ime.top_candidate());
}

/// Switching goes through the same loader as starting, so it reaches the
/// carried tables too (the gap measured 2026-10-12).
#[test]
fn switching_scheme_reaches_the_carried_tables() {
    if !carry() {
        return;
    }
    let mut s = ImeSession::new(Scheme::LINGMING, nowhere());
    assert!(s.set_scheme(Scheme::XINGCHEN));
    assert!(s.is_builtin());
    s.input('a');
    assert!(!s.page_candidates().is_empty());
    // A scheme the binary does not carry is honestly unavailable.
    assert!(!s.set_scheme(Scheme::RIYUE));
}

/// `:yume !` takes every file from the binary, whatever is installed.
#[test]
fn the_builtin_session_reads_only_the_binary() {
    if !carry() {
        return;
    }
    assert!(builtin_version().is_some());
    let s = ImeSession::builtin(Scheme::PINYIN).expect("pinyin is carried");
    assert!(s.available() && s.is_builtin());
    assert!(ImeSession::builtin(Scheme::QINGYUN).is_none());
}

/// The 拆分表 answers with no scheme loaded, from the binary.
#[test]
fn a_character_answers_from_the_carried_divisions() {
    if !carry() {
        return;
    }
    let ime = ImeSession::empty(Scheme::PINYIN);
    assert!(ime.chaifen_available());
    let got = ime.glosses("行");
    assert!(got.divided, "{got:?}");
}
