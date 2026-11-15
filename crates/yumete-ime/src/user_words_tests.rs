//! 單字派 and the word lists, driven through a session over a made-up 碼表 and
//! made-up folders. Nothing here points at a real app folder: every root is a
//! fresh directory under the test's temp dir, holding entries written below.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::*;

/// `jk` is 你 and 你好 in the 碼表; `df` is 他 and 她.
const TABLE: &str = "jk 你 你好\ndf 他 她\n";

fn session() -> ImeSession {
    ImeSession::from_table_text(Scheme::LINGMING, TABLE)
}

fn typed(ime: &mut ImeSession, code: &str) -> Vec<String> {
    ime.escape();
    for ch in code.chars() {
        ime.input(ch);
    }
    let out = ime.page_candidates().into_iter().map(|c| c.text).collect();
    ime.escape();
    out
}

/// A fresh, empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yumete-user-words-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(root: &Path, rel: &str, text: &str) -> PathBuf {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

/// Every file under `root`, with its bytes and modification time.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>, SystemTime)> {
    let mut out = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else {
                let when = std::fs::metadata(&path).unwrap().modified().unwrap();
                out.push((path.clone(), std::fs::read(&path).unwrap(), when));
            }
        }
    }
    out.sort();
    out
}

fn roots(app: &Path, own: &Path) -> UserWordRoots {
    UserWordRoots { app: Some(app.to_path_buf()), own: Some(own.to_path_buf()) }
}

// ---- 單字派 ----------------------------------------------------------------

#[test]
fn strict_hides_a_two_character_word_but_keeps_the_users_own() {
    let app = scratch("strict-app");
    put(&app, "data/lingming_user_dict.txt", "你们\tjk\n");
    let mut ime = session();
    ime.set_user_word_roots(UserWordRoots { app: Some(app.clone()), own: None });
    assert!(typed(&mut ime, "jk").contains(&"你好".to_string()), "字詞同出 shows the word");
    let mut single = ime.single_chars();
    single.char_by_char = SingleCharMode::Strict;
    ime.set_single_chars(single);
    let got = typed(&mut ime, "jk");
    assert!(!got.contains(&"你好".to_string()), "嚴格單字 hides the 碼表's word: {got:?}");
    assert!(got.contains(&"你".to_string()), "{got:?}");
    assert!(got.contains(&"你们".to_string()), "the writer's own word stays: {got:?}");
}

#[test]
fn the_sentence_copy_is_its_own() {
    let mut ime = session();
    assert_eq!(ime.single_chars(), SingleChars::default());
    assert_eq!(ime.single_chars().sentence, SingleCharMode::Strict, "Yume's 整句 default");
    assert_eq!(ime.active_single_char(), SingleCharMode::Both, "延遲 reads the other copy");
    ime.set_commit_strategy(Some(CommitStrategy::Fluency));
    assert!(ime.single_char_is_sentence());
    assert_eq!(ime.active_single_char(), SingleCharMode::Strict);
    ime.set_active_single_char(SingleCharMode::Primary);
    assert_eq!(ime.single_chars().sentence, SingleCharMode::Primary);
    assert_eq!(ime.single_chars().char_by_char, SingleCharMode::Both, "untouched");
    ime.set_commit_strategy(Some(CommitStrategy::Delayed));
    assert_eq!(ime.active_single_char(), SingleCharMode::Both);
}

#[test]
fn the_setting_survives_a_scheme_switch() {
    let mut ime = session();
    let want = SingleChars {
        char_by_char: SingleCharMode::Primary,
        sentence: SingleCharMode::Both,
        word_show: WordDisplayPrefs { show_two: true, ..WordDisplayPrefs::default() },
    };
    ime.set_single_chars(want);
    ime.set_scheme(Scheme::XINGCHEN);
    assert_eq!(ime.single_chars(), want);
    assert_eq!(SingleChars::of(&ime.engine), want, "pushed into the new engine");
    let mut next = session();
    next.carry_settings_from(&mut ime);
    assert_eq!(next.single_chars(), want);
}

#[test]
fn the_config_words_read_as_yume_names() {
    let got = SingleChars::from_words("strict", "both", &["two".into(), "whitelist".into()]);
    assert_eq!(got.char_by_char, SingleCharMode::Strict);
    assert_eq!(got.sentence, SingleCharMode::Both);
    assert!(got.word_show.show_two && got.word_show.show_whitelist);
    assert!(!got.word_show.show_three && !got.word_show.show_four && !got.word_show.show_longer);
    assert_eq!(SingleChars::from_words("nonsense", "", &[]), SingleChars::default());
}

#[test]
fn the_cycle_is_yumes() {
    let mut mode = SingleCharMode::Both;
    let mut seen = Vec::new();
    for _ in 0..3 {
        mode = SingleChars::next(mode);
        seen.push(mode);
    }
    assert_eq!(seen, [SingleCharMode::Primary, SingleCharMode::Strict, SingleCharMode::Both]);
}

// ---- The app's word lists --------------------------------------------------

#[test]
fn the_apps_files_change_the_candidates() {
    let app = scratch("app-read");
    let own = scratch("app-read-own");
    // The old stem: 靈明's words saved before the rename.
    put(&app, "data/ling_user_dict.txt", "你们\tjk\n");
    put(&app, "schemes/lingming/pins.txt", "df\t她\t1\n");
    put(&app, "schemes/lingming/freq.txt", "#tick\t20\ndf\t她\t9\t5.0e0\t20\n");
    let mut ime = session();
    let before = typed(&mut ime, "df");
    assert_eq!(before.first().map(String::as_str), Some("他"), "{before:?}");
    ime.set_user_word_roots(roots(&app, &own));
    assert!(typed(&mut ime, "jk").contains(&"你们".to_string()), "the old ling stem is read");
    let pinned = typed(&mut ime, "df");
    assert_eq!(pinned.first().map(String::as_str), Some("她"), "固頂: {pinned:?}");
    assert!(ime.engine.user_freq().log_weight("df", "她").is_some(), "調頻 is read");
    let read = ime.user_files_read();
    assert_eq!(read.len(), 3, "{read:?}");
    assert!(read.iter().all(|f| !f.own && f.entries == 1), "{read:?}");
}

#[test]
fn the_shared_pinyin_dictionary_is_read() {
    let app = scratch("shared");
    put(&app, "data/snow-pinyin_user_dict.txt", "她们\tdf\n");
    put(&app, "schemes/snow-pinyin/freq.txt", "#tick\t3\nta1 men5\t她们\t2\t1.0e0\t3\n");
    let mut table = CodeTable::new();
    table.load_text(TABLE);
    let mut engine = Engine::new(table);
    engine.set_scheme(yume_core::schema::Schema {
        pinyin_user_dict: "snow-pinyin".to_string(),
        ..yume_core::schema::Schema::default()
    });
    let mut ime = ImeSession::from_engine(engine, Scheme::LINGMING);
    ime.set_user_word_roots(UserWordRoots { app: Some(app.clone()), own: None });
    assert!(typed(&mut ime, "df").contains(&"她们".to_string()));
    assert_eq!(ime.engine.pinyin_freq().len(), 1, "the shared 調頻");
    let names: Vec<_> = ime.user_files_read().iter().map(|f| f.path.clone()).collect();
    assert!(names.contains(&app.join("data/snow-pinyin_user_dict.txt")), "{names:?}");
    assert!(names.contains(&app.join("schemes/snow-pinyin/freq.txt")), "{names:?}");
}

#[test]
fn a_file_the_app_saves_is_read_again() {
    let app = scratch("mtime");
    let dict = put(&app, "data/lingming_user_dict.txt", "你们\tjk\n");
    let mut ime = session();
    ime.set_user_word_roots(UserWordRoots { app: Some(app.clone()), own: None });
    assert!(!ime.refresh_user_words(), "nothing changed");
    std::fs::write(&dict, "你们\tjk\n他们\tdf\n").unwrap();
    let later = SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options().write(true).open(&dict).unwrap().set_modified(later).unwrap();
    assert!(ime.refresh_user_words());
    assert!(typed(&mut ime, "df").contains(&"他们".to_string()));
    // A file that was not there and now is counts as a change too.
    put(&app, "schemes/lingming/pins.txt", "jk\t你好\t1\n");
    assert!(ime.refresh_user_words());
    assert_eq!(typed(&mut ime, "jk").first().map(String::as_str), Some("你好"));
}

#[test]
fn no_app_folder_is_no_failure() {
    let nowhere = scratch("none").join("not-there");
    let mut ime = session();
    ime.set_user_word_roots(UserWordRoots { app: Some(nowhere.clone()), own: None });
    assert!(ime.user_files_read().is_empty());
    assert!(!ime.refresh_user_words());
    ime.save_learning();
    assert_eq!(typed(&mut ime, "jk").first().map(String::as_str), Some("你"));
    assert!(!nowhere.exists(), "nothing is made where the app would be");
    let mut bare = session();
    bare.set_user_word_roots(UserWordRoots::default());
    assert!(bare.user_files_read().is_empty());
}

// ---- yumete's own --------------------------------------------------------

#[test]
fn a_learned_word_persists_and_loads_after_the_apps() {
    let app = scratch("own-app");
    let own = scratch("own-own");
    put(&app, "data/lingming_user_dict.txt", "你们\tjk\n");
    put(&app, "schemes/lingming/freq.txt", "#tick\t10\ndf\t她\t5\t2.0e0\t10\n");
    let before = snapshot(&app);
    {
        let mut ime = session();
        ime.set_user_word_roots(roots(&app, &own));
        let words = ime.user_words.as_mut().unwrap();
        assert!(words.add_line(false, "他们\tdf"));
        assert!(!words.add_line(false, "你们\tjk"), "the app has it already");
        words.write_own(false);
        words.install_words(&mut ime.engine);
        ime.engine.set_adaptive(true);
        // yumete's own 調頻 on a key the app also has: chosen twice more.
        for _ in 0..2 {
            ime.engine.user_freq_mut().note("df", "她");
        }
        ime.save_learning();
        assert_eq!(ime.forget_own_word("nothing-like-it"), 0);
    }
    assert_eq!(snapshot(&app), before, "the app's files are never written");
    let dict = std::fs::read_to_string(own.join("lingming/user_dict.txt")).unwrap();
    assert_eq!(dict, "他们\tdf\n");
    let mut ime = session();
    ime.set_user_word_roots(roots(&app, &own));
    assert!(typed(&mut ime, "df").contains(&"他们".to_string()), "a new session has it");
    let commits = ime
        .engine
        .user_freq()
        .rows()
        .find(|((c, t), _)| c == "df" && t == "她")
        .map(|(_, r)| r.commits);
    assert_eq!(commits, Some(7), "yumete's record of the key is the one in force");
    let read = ime.user_files_read();
    let own_files: Vec<_> = read.iter().filter(|f| f.own).collect();
    assert_eq!(own_files.len(), 2, "{read:?}");
    let first_own = read.iter().position(|f| f.own).unwrap();
    assert!(read[..first_own].iter().all(|f| !f.own), "the app's are read first: {read:?}");
    // And it can be taken back out.
    assert_eq!(ime.forget_own_word("他们"), 1);
    assert!(!typed(&mut ime, "df").contains(&"他们".to_string()));
    assert_eq!(std::fs::read_to_string(own.join("lingming/user_dict.txt")).unwrap(), "");
    ime.save_learning();
    ime.refresh_user_words();
    assert_eq!(snapshot(&app), before, "still untouched");
}

#[test]
fn a_commit_empties_the_engines_queues() {
    let own = scratch("queues");
    let mut ime = session();
    ime.set_user_word_roots(UserWordRoots { app: None, own: Some(own.clone()) });
    ime.input('j');
    ime.input('k');
    ime.space();
    assert_eq!(ime.take_committed(), "你");
    assert!(ime.engine.take_new_words().is_empty());
    assert!(ime.engine.take_new_codes().is_empty());
    assert!(ime.engine.take_new_pinyin_words().is_empty());
}
