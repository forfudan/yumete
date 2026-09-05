//! Integration test against the real, installed IME data (Feature #27/#32).
//!
//! Skipped automatically when the compiled tables are not present in the user
//! data directory (`~/.local/share/yumete` or `$XDG_DATA_HOME/yumete`), so it is
//! safe to run anywhere. Populate the data with `scripts/build.sh`.

use std::path::PathBuf;

use yumete_ime::{CommitStrategy, ImeSession, Scheme};

fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    let dir = base.join("yumete");
    // The layout `yume_core::data_manifest` names: schemes under `schemes/`.
    if dir.join("schemes/ling.ytab").is_file() {
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

    let mut session = ImeSession::new(Scheme::LINGMING, vec![dir]);
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
fn a_chosen_commit_method_survives_a_scheme_switch_but_never_overrules_pinyin() {
    let Some(dir) = data_dir() else {
        return;
    };
    let mut session = ImeSession::new(Scheme::LINGMING, vec![dir]);
    session.set_commit_strategy(Some(CommitStrategy::Unique));
    assert!(session.set_scheme(Scheme::PINYIN));
    // Chosen, and remembered — but 拼音 has no 碼表 to look a segment up in, so
    // what is *in force* there is 整句 whatever was asked (Feature #209).
    assert_eq!(session.commit_override(), Some(CommitStrategy::Unique));
    assert_eq!(session.commit_strategy(), CommitStrategy::Fluency);
    // …and coming back, the choice is still the writer's.
    assert!(session.set_scheme(Scheme::LINGMING));
    assert_eq!(session.commit_strategy(), CommitStrategy::Unique);
}

#[test]
fn real_pinyin_scheme_switch_stays_available() {
    let Some(dir) = data_dir() else {
        return;
    };
    let mut session = ImeSession::new(Scheme::LINGMING, vec![dir]);
    // Pinyin uses the shared fluency table, which build.sh also installs.
    assert!(session.set_scheme(Scheme::PINYIN));
    // The display name is yume's to choose — it has been 「拼音」 and is now
    // 「宇浩拼音」 — so this asks what the name is *about*, not what it is.
    let name = session.scheme_name();
    assert!(name.contains("拼音"), "{name}");
}

/// **Which dictionary actually drives `w`/`b`/`e`**, on a machine that has the
/// real data.
///
/// Yume's own language model is 繁簡混合 and is what the editor prefers; the
/// list bundled in the binary is the fallback for a machine with no data
/// installed. Asserting it here so 「the word list is simplified-only」 can
/// never again be said about the model, only about the fallback.
#[test]
fn the_yume_model_segments_both_scripts() {
    let Some(dir) = data_dir() else {
        eprintln!("skipping: no installed IME data (run scripts/build.sh)");
        return;
    };
    let session = ImeSession::new(Scheme::LINGMING, vec![dir]);
    let words = session.segmenter();
    if !words.is_available() {
        eprintln!("skipping: the language tables are not installed");
        return;
    }
    let joins = |line: &str, want: &str| {
        let chars: Vec<char> = line.chars().collect();
        let found: Vec<String> = yumete_cjk::Segmenter::segment(&words, line)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect();
        // Joined *into a word* — the model may take 「那時候」 whole, which is
        // still a word motion stepping over 時候 rather than through it.
        assert!(
            found.iter().any(|w| w.chars().count() > 1 && w.contains(want)),
            "{found:?} never joins {want}"
        );
    };
    // The same sentence in both scripts, and neither is read one 字 at a time.
    joins("那時候他抬頭看了看。", "時候");
    joins("那时候他抬头看了看。", "时候");
    joins("他說道：這裏沒有人。", "說道");
    joins("他说道：这里没有人。", "说道");
}

/// `:word level` on the real model: the knob moves the boundaries, and it moves
/// them in the direction its name promises.
#[test]
fn the_word_level_changes_how_readily_words_join() {
    let Some(dir) = data_dir() else {
        eprintln!("skipping: no installed IME data (run scripts/build.sh)");
        return;
    };
    let session = ImeSession::new(Scheme::LINGMING, vec![dir]);
    if !session.segmenter().is_available() {
        eprintln!("skipping: the language tables are not installed");
        return;
    }
    let line = "那年冬天他抬頭看了看那片天，山路已經看不見了。";
    let cut = |level| {
        let mut words = session.segmenter();
        yumete_cjk::Segmenter::set_level(&mut words, level);
        let chars: Vec<char> = line.chars().collect();
        let found: Vec<String> = yumete_cjk::Segmenter::segment(&words, line)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect();
        found
    };
    let strict = cut(yumete_cjk::WordLevel::Strict);
    let balanced = cut(yumete_cjk::WordLevel::Balanced);
    let full = cut(yumete_cjk::WordLevel::Full);
    assert!(
        strict.len() > balanced.len(),
        "strict should cut more:\n  strict   {strict:?}\n  balanced {balanced:?}"
    );
    assert!(
        full.len() < balanced.len(),
        "full should join more:\n  full     {full:?}\n  balanced {balanced:?}"
    );
    // …and none of the three reads the sentence one 字 at a time.
    assert!(strict.iter().any(|w| w.chars().count() > 1), "{strict:?}");
}

/// `:ruby auto rare` asked the 字集 column for the 簡 tag alone, and 簡 is
/// 通用規範漢字表 — a list of *simplified* standard forms. So on the 繁體 a
/// novel is written in it answered 「生僻」 for 說, 為, 這, 裏, 學 and 國, which
/// is every second character: the mode that exists to keep a novel from
/// becoming a textbook turned every novel into one. Ordinary now means carried
/// by any standard in current use.
#[test]
fn a_traditional_character_in_daily_use_is_not_a_rare_one() {
    let Some(dir) = data_dir() else { return };
    let session = ImeSession::new(Scheme::LINGMING, vec![dir]);
    let reader = session.reader();
    assert!(
        yumete_cjk::Reader::available(&reader),
        "the annotation table should be loaded"
    );
    for ch in "說為這裏學國們臺灣".chars() {
        assert_eq!(
            yumete_cjk::Reader::is_rare(&reader, ch),
            Some(false),
            "{ch} is written every day"
        );
    }
    // 龘 is in none of the four, and is what the mode is for.
    assert_eq!(yumete_cjk::Reader::is_rare(&reader, '龘'), Some(true));
    // Not a 漢字 at all — nothing to stumble on.
    assert_eq!(yumete_cjk::Reader::is_rare(&reader, 'a'), Some(false));
}

/// A probe, not an assertion: what each bias does to one real sentence.
#[test]
#[ignore]
fn probe_bias_sensitivity() {
    let Some(dir) = data_dir() else { return };
    let session = ImeSession::new(Scheme::LINGMING, vec![dir]);
    let line = "那年冬天他抬頭看了看那片天，山路已經看不見了。";
    for tenths in -40..=40 {
        if tenths % 5 != 0 {
            continue;
        }
        let bias = tenths as f64 / 10.0;
        let mut words = session.segmenter();
        words.set_bias_for_probe(bias);
        let chars: Vec<char> = line.chars().collect();
        let found: Vec<String> = yumete_cjk::Segmenter::segment(&words, line)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect();
        println!("{bias:+.1}  {}  {found:?}", found.len());
    }
}
