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
        ime.set_learn(true);
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
    assert_eq!(commits, Some(7), "the app's 5 and yumete's 2, summed");
    let own_freq = std::fs::read_to_string(own.join("lingming/freq.txt")).unwrap();
    assert!(own_freq.contains("df\t她\t2\t"), "only yumete's two: {own_freq}");
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

// ---- Two layers ----------------------------------------------------------

/// How many times `code` gave `text`, as the engine holds it.
fn count(ime: &ImeSession, code: &str, text: &str) -> Option<u32> {
    ime.engine.user_freq().rows().find(|((c, t), _)| c == code && t == text).map(|(_, r)| r.commits)
}

/// A file's modification time moved on, so a re-read notices it.
fn touch_later(path: &Path) {
    let later = SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(later).unwrap();
}

#[test]
fn a_change_made_in_the_app_shows_through_yumetes_own() {
    let app = scratch("layers-app");
    let own = scratch("layers-own");
    let freq = put(&app, "schemes/lingming/freq.txt", "#tick\t10\ndf\t她\t5\t2.0e0\t10\n");
    put(&own, "lingming/freq.txt", "#tick\t4\ndf\t她\t2\t1.0e0\t4\n");
    let mut ime = session();
    ime.set_user_word_roots(roots(&app, &own));
    assert_eq!(count(&ime, "df", "她"), Some(7), "5 and 2, summed");
    std::fs::write(&freq, "#tick\t12\ndf\t她\t9\t3.0e0\t12\n").unwrap();
    touch_later(&freq);
    assert!(ime.refresh_user_words());
    assert_eq!(count(&ime, "df", "她"), Some(11), "the app's 9 shows, yumete's 2 stays");
    let kept = std::fs::read_to_string(own.join("lingming/freq.txt")).unwrap();
    assert!(kept.contains("df\t她\t2\t"), "yumete's file is its own: {kept}");
}

#[test]
fn without_yumetes_files_the_apps_rows_stand_alone() {
    let app = scratch("alone-app");
    let own = scratch("alone-own");
    put(&app, "data/lingming_user_dict.txt", "你们\tjk\n");
    put(&app, "schemes/lingming/freq.txt", "#tick\t10\ndf\t她\t5\t2.0e0\t10\n");
    put(&app, "schemes/lingming/pins.txt", "df\t她\t1\n");
    let mut app_only = session();
    app_only.set_user_word_roots(UserWordRoots { app: Some(app.clone()), own: None });
    put(&own, "lingming/user_dict.txt", "他们\tdf\n");
    put(&own, "lingming/freq.txt", "#tick\t3\ndf\t她\t3\t1.0e0\t3\n");
    put(&own, "lingming/pins.txt", "jk\t你好\t1\n");
    let mut ime = session();
    ime.set_user_word_roots(roots(&app, &own));
    assert!(typed(&mut ime, "df").contains(&"他们".to_string()));
    assert_eq!(typed(&mut ime, "jk").first().map(String::as_str), Some("你好"));
    std::fs::remove_dir_all(own.join("lingming")).unwrap();
    assert!(ime.refresh_user_words(), "a file gone is a change");
    for code in ["jk", "df"] {
        assert_eq!(typed(&mut ime, code), typed(&mut app_only, code), "{code}: as the app alone");
    }
    assert_eq!(count(&ime, "df", "她"), Some(5));
    assert_eq!(ime.engine.records(yume_core::user_records::Kind::Pin).len(), 1);
}

#[test]
fn learning_off_writes_nothing_and_empties_the_queues() {
    let app = scratch("off-app");
    let own = scratch("off-own");
    put(&app, "schemes/lingming/freq.txt", "#tick\t10\ndf\t她\t5\t2.0e0\t10\n");
    put(&own, "lingming/user_dict.txt", "他们\tdf\n");
    let before = snapshot(&own);
    let mut ime = session();
    assert!(!ime.learn(), "off by default");
    ime.set_user_word_roots(roots(&app, &own));
    assert!(typed(&mut ime, "df").contains(&"他们".to_string()), "yumete's own files are read when off");
    ime.engine.set_adaptive(true);
    ime.engine.user_freq_mut().note("df", "她");
    ime.engine.pin_dict_mut().set("jk", "你好", 1);
    ime.input('j');
    ime.input('k');
    ime.space();
    ime.take_committed();
    ime.save_learning();
    assert!(ime.engine.take_new_words().is_empty());
    assert!(ime.engine.take_new_codes().is_empty());
    assert!(ime.engine.take_new_pinyin_words().is_empty());
    assert!(count(&ime, "df", "她") >= Some(6), "the engine still reorders for the session");
    assert_eq!(snapshot(&own), before, "nothing is written");
    // On from here: what is learned from here on is kept, and only that.
    ime.set_learn(true);
    ime.engine.user_freq_mut().note("df", "她");
    ime.save_learning();
    let kept = std::fs::read_to_string(own.join("lingming/freq.txt")).unwrap();
    assert!(kept.contains("df\t她\t1\t"), "one choice, made while learning: {kept}");
}

#[test]
fn learning_on_writes_only_what_yumete_learned() {
    let app = scratch("on-app");
    let own = scratch("on-own");
    put(&app, "data/lingming_user_dict.txt", "你们\tjk\n");
    put(&app, "schemes/lingming/freq.txt", "#tick\t10\ndf\t她\t5\t2.0e0\t10\njk\t你好\t3\t1.0e0\t9\n");
    put(&app, "schemes/lingming/pins.txt", "df\t她\t1\n");
    let app_before = snapshot(&app);
    let mut ime = session();
    ime.set_learn(true);
    ime.set_user_word_roots(roots(&app, &own));
    ime.engine.user_freq_mut().note("df", "他");
    ime.engine.pin_dict_mut().set("jk", "你好", 1);
    ime.take_committed();
    ime.save_learning();
    let freq = std::fs::read_to_string(own.join("lingming/freq.txt")).unwrap();
    let rows: Vec<&str> = freq.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(rows.len(), 1, "{freq}");
    assert!(rows[0].starts_with("df\t他\t1\t"), "{freq}");
    assert_eq!(std::fs::read_to_string(own.join("lingming/pins.txt")).unwrap(), "jk\t你好\t1\n");
    assert!(!own.join("lingming/user_dict.txt").exists(), "no word learned, no word file");
    assert_eq!(snapshot(&app), app_before, "the app's files are never written");
    // A pin of the app's taken away in yumete: yumete's own row says so.
    ime.engine.pin_dict_mut().remove("df", "她");
    ime.take_committed();
    let pins = std::fs::read_to_string(own.join("lingming/pins.txt")).unwrap();
    assert_eq!(pins, "jk\t你好\t1\ndf\t她\t0\n");
    assert_eq!(snapshot(&app), app_before);
}

#[test]
fn deleting_and_putting_back_each_list() {
    let app = scratch("page-app");
    let own = scratch("page-own");
    put(&app, "data/lingming_user_dict.txt", "你们\tjk\n");
    put(&app, "schemes/lingming/freq.txt", "#tick\t10\ndf\t她\t5\t2.0e0\t10\n");
    put(&app, "schemes/lingming/pins.txt", "jk\t你\t2\n");
    put(&own, "lingming/user_dict.txt", "# mine\n他们\tdf\n你们\tjk\n他们\tdff\n");
    put(&own, "lingming/freq.txt", "#tick\t4\ndf\t她\t2\t1.0e0\t4\ndf\t他\t1\t1.0e0\t3\n");
    put(&own, "lingming/pins.txt", "df\t她\t1\n");
    let app_before = snapshot(&app);
    let mut ime = session();
    ime.set_user_word_roots(roots(&app, &own));
    assert_eq!(ime.word_lists(), vec![WordList::Words, WordList::Freq, WordList::Pins]);

    // 用户詞庫: a word only yumete has, a word both have.
    let words = ime.word_rows(WordList::Words);
    let held: Vec<(&str, &str, Held)> = words.iter().map(|r| (r.word.as_str(), r.code.as_str(), r.held)).collect();
    assert_eq!(held, [("他们", "df dff", Held::Own), ("你们", "jk", Held::Both)]);
    assert_eq!(words[0].line, Some(2), "the line in yumete's file");
    let both = words[1].clone();
    let gone = ime.delete_own_row(WordList::Words, &both).expect("yumete's copy");
    let after = ime.word_rows(WordList::Words);
    assert_eq!(after.iter().find(|r| r.word == "你们").map(|r| r.held), Some(Held::App), "stays, as the app's");
    assert!(typed(&mut ime, "jk").contains(&"你们".to_string()), "the app still has it");
    let mine = after[0].clone();
    let gone_too = ime.delete_own_row(WordList::Words, &mine).unwrap();
    assert!(!typed(&mut ime, "df").contains(&"他们".to_string()), "gone at once");
    assert_eq!(std::fs::read_to_string(own.join("lingming/user_dict.txt")).unwrap(), "# mine\n");
    assert!(ime.restore_own_row(gone_too));
    assert!(ime.restore_own_row(gone));
    assert_eq!(
        std::fs::read_to_string(own.join("lingming/user_dict.txt")).unwrap(),
        "# mine\n他们\tdf\n你们\tjk\n他们\tdff\n",
        "back where they stood"
    );
    assert!(typed(&mut ime, "df").contains(&"他们".to_string()));

    // 調頻: summed; deleting yumete's row leaves the app's count.
    let freq = ime.word_rows(WordList::Freq);
    let row = freq.iter().find(|r| r.word == "她").unwrap().clone();
    assert_eq!((row.own, row.app, row.held), (Some(2), Some(5), Held::Both));
    assert_eq!(count(&ime, "df", "她"), Some(7));
    let gone = ime.delete_own_row(WordList::Freq, &row).unwrap();
    assert_eq!(count(&ime, "df", "她"), Some(5), "back to the app's");
    let file = std::fs::read_to_string(own.join("lingming/freq.txt")).unwrap();
    assert!(!file.contains("她") && file.contains("他"), "{file}");
    assert!(ime.restore_own_row(gone));
    assert_eq!(count(&ime, "df", "她"), Some(7));

    // 固頂.
    assert_eq!(typed(&mut ime, "df").first().map(String::as_str), Some("她"));
    let pin = ime.word_rows(WordList::Pins).into_iter().find(|r| r.held == Held::Own).unwrap();
    let gone = ime.delete_own_row(WordList::Pins, &pin).unwrap();
    assert_eq!(std::fs::read_to_string(own.join("lingming/pins.txt")).unwrap(), "");
    assert!(ime.engine.records(yume_core::user_records::Kind::Pin).iter().all(|r| r.text != "她"), "unpinned at once");
    assert!(ime.restore_own_row(gone));
    assert_eq!(typed(&mut ime, "df").first().map(String::as_str), Some("她"));

    // The app's rows are not deleted here.
    let theirs = ime.word_rows(WordList::Pins).into_iter().find(|r| r.held == Held::App).unwrap();
    assert!(ime.delete_own_row(WordList::Pins, &theirs).is_none());
    assert_eq!(snapshot(&app), app_before, "the app's files are never written");
}

#[test]
fn a_deletion_belongs_to_its_scheme() {
    let own = scratch("other-scheme");
    put(&own, "lingming/user_dict.txt", "他们\tdf\n");
    let mut ime = session();
    ime.set_user_word_roots(UserWordRoots { app: None, own: Some(own.clone()) });
    let row = ime.word_rows(WordList::Words).remove(0);
    let gone = ime.delete_own_row(WordList::Words, &row).unwrap();
    assert_eq!(gone.tag(), "lingming");
    let mut other = ImeSession::from_table_text(Scheme::XINGCHEN, TABLE);
    other.set_user_word_roots(UserWordRoots { app: None, own: Some(own.clone()) });
    assert!(!other.restore_own_row(gone), "another scheme's lists refuse it");
}
