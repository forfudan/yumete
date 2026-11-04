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

/// **Yume's 字集 lanes are on by default** (2026-11-04): a 拼音 sentence stays
/// in one script, and the other script comes as a sentence of its own. With
/// no lanes this came out as 數学教授在講臺上讲课.
#[test]
fn a_pinyin_sentence_keeps_to_one_script() {
    if !carry() {
        return;
    }
    let mut ime = ImeSession::new(Scheme::PINYIN, nowhere());
    assert_eq!(ime.active_lanes(), ["tonggui", "tongfan", "tai"]);
    for key in "shuxuejiaoshouzaijiangtaishangjiangke".chars() {
        ime.input(key);
    }
    let candidates: Vec<String> = ime.page_candidates().into_iter().map(|c| c.text).collect();
    assert_eq!(candidates.first().map(String::as_str), Some("数学教授在讲台上讲课"), "{candidates:?}");
    assert!(candidates.iter().any(|c| c == "數學教授在講臺上講課"), "{candidates:?}");

    // The first lane decides: 繁 first puts the traditional sentence on top.
    let mut charsets = ime.charsets().clone();
    charsets.put_first("tongfan");
    ime.set_charsets(charsets);
    ime.escape();
    for key in "shuxuejiaoshouzaijiangtaishangjiangke".chars() {
        ime.input(key);
    }
    assert_eq!(ime.top_candidate(), "數學教授在講臺上講課");
}

/// **The word list the binary carries reads both scripts** (2026-11-04). It
/// replaces the compact bundled list, whose test this was: `w` must not step
/// one 漢字 at a time through a 繁體 manuscript, nor through a 简体 one.
#[test]
fn the_carried_words_join_in_both_scripts() {
    if !carry() {
        return;
    }
    let ime = ImeSession::language_only(Scheme::PINYIN);
    let seg = ime.segmenter();
    for (line, word) in [
        ("他抬頭看了看那片天，山路已經看不見了。", "已經"),
        ("他抬头看了看那片天，山路已经看不见了。", "已经"),
    ] {
        let chars: Vec<char> = line.chars().collect();
        let words: Vec<String> = yumete_cjk::Segmenter::segment(&seg, line)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect();
        assert!(words.contains(&word.to_string()), "{words:?} never joins {word}");
    }
}

/// **An installed file this build cannot read is passed over** (2026-11-04
/// review): an older Yume's `pinyin.yflb` used to take 宇浩拼音 down, and the
/// editor started in 靈明 with nothing said.
#[test]
fn an_installed_file_this_build_refuses_falls_back_to_the_carried_one() {
    if !carry() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("yumete-refused-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(dir.join("data/pinyin.yflb"), b"YFL3 not this build's format").unwrap();
    let mut ime = ImeSession::new(Scheme::PINYIN, vec![dir.clone()]);
    assert!(ime.available(), "{:?}", ime.problems());
    for key in "nihao".chars() {
        ime.input(key);
    }
    assert_eq!(ime.top_candidate(), "你好");
    let _ = std::fs::remove_dir_all(&dir);
}
