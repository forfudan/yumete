//! **`空格 n` names its rows in the reader's language.**
//!
//! The lookup (`yumete-ime`) hands back message tags; the panel says them.
//! Its own binary because the language is process-wide.

use yumete_core::editor::Found;
use yumete_core::messages::{self, Language};
use yumete_ime::{ImeSession, Scheme};

/// Every row name, as each language writes it.
const NAMES: &[(&str, &str, &str)] = &[
    ("ime.lookup.division", "方案拆分", "方案拆分"),
    ("ime.lookup.code", "编码", "編碼"),
    ("ime.lookup.segmented", "分节编码", "分節編碼"),
    ("ime.lookup.full-division", "宇浩全息拆分", "宇浩全息拆分"),
    ("ime.lookup.reading", "读音", "讀音"),
    ("ime.lookup.note", "注释", "注釋"),
    ("ime.lookup.charset", "字集", "字集"),
    ("ime.lookup.codepoint", "统一码", "統一碼"),
    ("ime.lookup.block", "统一码区块", "統一碼區塊"),
    ("ime.lookup.source.mainland", "陆", "陸"),
    ("ime.lookup.source.taiwan", "台", "臺"),
    ("ime.lookup.source.hong-kong", "港", "港"),
    ("ime.lookup.source.compatible", "兼", "兼"),
];

fn has_han(text: &str) -> bool {
    text.chars().any(yumete_cjk::is_han)
}

#[test]
fn the_lookup_rows_are_named_in_the_reader_s_language() {
    // Every tag the lookup can hand back has its three forms.
    for (tag, zhs, zht) in NAMES {
        messages::set_language(Language::Simplified);
        assert_eq!(messages::say(tag, &[]), *zhs, "{tag}");
        messages::set_language(Language::Traditional);
        assert_eq!(messages::say(tag, &[]), *zht, "{tag}");
        messages::set_language(Language::English);
        let en = messages::say(tag, &[]);
        assert!(!has_han(&en) && en != *tag, "{tag}: {en}");
    }
    let lookup_tags = messages::table().keys().filter(|k| k.starts_with("ime.lookup.")).count();
    assert_eq!(lookup_tags, NAMES.len(), "a lookup tag with no line above");

    // …and a real lookup comes out that way. The code point and its block
    // need no table, so this holds on any machine.
    let ime = ImeSession::empty(Scheme::LINGMING);
    let got = ime.glosses("行");
    for (tag, _) in &got.rows {
        assert!(NAMES.iter().any(|(t, _, _)| t == tag), "{tag} is not in NAMES");
    }
    let found = Found { rows: got.rows, divided: got.divided };
    let names = |language| {
        messages::set_language(language);
        found.said_rows().into_iter().map(|(name, _)| name).collect::<Vec<_>>()
    };
    let zhs = names(Language::Simplified);
    assert!(zhs.contains(&"统一码".to_string()), "{zhs:?}");
    let zht = names(Language::Traditional);
    assert!(zht.contains(&"統一碼區塊".to_string()), "{zht:?}");
    let en = names(Language::English);
    assert!(en.iter().all(|name| !has_han(name)), "{en:?}");
    assert!(en.contains(&"Unicode block".to_string()), "{en:?}");
    messages::set_language(Language::Traditional);
}
