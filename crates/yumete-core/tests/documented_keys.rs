//! **What the documents teach has to be what the editor has.**
//!
//! §5.2.2 found twelve faults in one night and eight of them were a name that
//! exists twice: a command taught in the manual and in the lesson and
//! implemented in neither (`t20,20g`), a key that had moved and left its old
//! spelling behind in prose (`t o`, `t f`), an argument the built-in help
//! offered that its command refuses. None of them could be found by reading the
//! code, because the code was right — the *other* copy of the name was wrong.
//!
//! So the documents are read here as if they were source. Every `` `:命令` ``
//! the manual or the lesson prints is walked down the real command table by the
//! parser's own rule, and every `` `空格 x` ``, `` `t o` ``, `` `gd` `` is
//! looked up in the union of the menus that key actually draws. A document that
//! teaches something the editor does not have fails the build, and it fails
//! naming the word.

use yumete_core::command;
use yumete_core::editor::Editor;

/// The two documents, read from the tree rather than embedded.
fn documents() -> Vec<(&'static str, String)> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    vec![
        (
            "docs/manual.md",
            std::fs::read_to_string(format!("{root}/docs/manual.md")).expect("the manual"),
        ),
        ("tutor.rs", yumete_core::tutor::LESSON.to_string()),
    ]
}

/// Every run between backticks, with the line it stands on.
fn quoted(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut rest = line;
        while let Some(open) = rest.find('`') {
            let after = &rest[open + 1..];
            match after.find('`') {
                Some(close) => {
                    out.push((n + 1, after[..close].to_string()));
                    rest = &after[close + 1..];
                }
                None => break,
            }
        }
    }
    out
}

/// The shape of a name this table can be asked about: lower-case ASCII, and a
/// `!` only at the end.
///
/// It is a **filter on the documents, not on the editor** — which is only
/// honest as long as no command is spelled any other way, and the next test
/// holds that. What it skips is the manual's own 「舊名 → 新名」 column
/// (`:ruby-on`, `:clipboard-yank`) and the line that says 「沒有 `:Q`」: names
/// printed in order to say they are *gone*, which are exactly the names this
/// test must not demand.
fn is_a_name(word: &str) -> bool {
    let word = word.strip_suffix('!').unwrap_or(word);
    !word.is_empty()
        && word.starts_with(|c: char| c.is_ascii_lowercase())
        && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

#[test]
fn no_command_is_spelled_outside_the_shape_the_documents_are_filtered_by() {
    for entry in command::COMMANDS {
        // `!command` and `s/pat/rep/` are the two the parser reaches before the
        // table — the shell escape and the substitution — and neither is looked
        // up by name.
        if entry.name == "!command" || entry.name.starts_with("s/") {
            continue;
        }
        assert!(is_a_name(entry.name), "`:{}` would be skipped", entry.name);
        for alias in entry.aliases {
            assert!(is_a_name(alias), "`:{alias}` would be skipped");
        }
    }
}

/// Names the documents print **in order to say they do not work**.
///
/// A manual that explains 「`:re` 同時是三個命令的前綴，所以它報不認識」 is
/// doing its job, and a test that demanded `:re` exist would be reading it
/// backwards. The list is short and it is checked from the other side by
/// [`each_name_the_documents_disown_really_is_missing`]: put a real command in
/// here and that test fails, so this cannot become the place a stale name goes
/// to hide.
const DISOWNED: &[&str] = &[
    // Ambiguous prefixes: `recover`, `redo`, `render` — the manual's own
    // example of 「兩個都不選」.
    ":re",
    // Ambiguous one level down, and printed for the same reason: `chaifen` and
    // `commit` both begin with `c`.
    ":yume c",
    // §「沒有了」 — the names deleted when a group was made, printed beside the
    // spelling that replaced each of them. 舊名字直接刪掉 is the philosophy the
    // table is there to state, so every one of these has to stay missing.
    ":scheme",
    ":chaifen",
    ":cf",
    ":vertical",
    ":horizontal",
    ":nowrap",
    ":markup",
    ":wysiwyg",
    ":source",
    ":buffers",
    ":ls",
    ":bn",
    ":bp",
    ":bd",
    ":cy",
    ":cp",
];

#[test]
fn each_name_the_documents_disown_really_is_missing() {
    for line in DISOWNED {
        assert!(
            command::names_something(line).is_err(),
            "`{line}` is a command the editor has — it does not belong on the list \
             of names the documents print to say they are gone"
        );
    }
}

#[test]
fn every_command_the_documents_print_is_a_command_the_editor_has() {
    let mut asked = 0;
    for (doc, text) in documents() {
        for (line, quote) in quoted(&text) {
            let Some(rest) = quote.strip_prefix(':') else { continue };
            let mut words = rest.split_whitespace();
            let Some(head) = words.next() else { continue };
            if !is_a_name(head) || DISOWNED.contains(&quote.as_str()) {
                continue;
            }
            // The words below the head are checked only as far as they are
            // written out. `:ruby full/off`, `:wrap 24`, `:e 第三章.md` and
            // `:run <名字>` all say「一個什麼」rather than naming one, and a
            // placeholder is not a word this table could ever know.
            let mut line_to_ask = String::from(head);
            for word in words {
                if !is_a_name(word) {
                    break;
                }
                line_to_ask.push(' ');
                line_to_ask.push_str(word);
            }
            asked += 1;
            if let Err(word) = command::names_something(&line_to_ask) {
                panic!("{doc}:{line} teaches `{quote}`, and `{word}` is not a word it has");
            }
        }
    }
    assert!(asked > 400, "only {asked} commands read out of the documents");
}

/// A leader and the key that follows it, as the documents write the pair.
///
/// Two spellings, because both are in the documents and both are how a reader
/// says it out loud: `` `空格 f` `` and `` `t o` `` with the space, `` `gd` ``
/// and `` `mi` `` without it. Nothing longer is read — `t1a2d8as` names three
/// columns and `mi「` names a bracket, and neither is this table's vocabulary.
fn leader_and_key(quote: &str) -> Option<(char, String)> {
    let mut words = quote.split_whitespace();
    let (Some(head), rest, None) = (words.next(), words.next(), words.next()) else {
        return None;
    };
    if let Some(key) = rest {
        // `空格` is two characters and one key.
        let lead = match head {
            "空格" => '空',
            one => one.chars().next().filter(|_| one.chars().count() == 1)?,
        };
        return Some((lead, key.to_string()));
    }
    // Both halves ASCII: `空格` on its own is two 漢字 and one key, and read
    // as a pair it asks the 空 menu for 「格」.
    let mut chars = head.chars();
    let (Some(lead), Some(key), None) = (chars.next(), chars.next(), chars.next()) else {
        return None;
    };
    (lead.is_ascii() && key.is_ascii()).then(|| (lead, key.to_string()))
}

/// Two-letter words that are **not** key sequences at all.
///
/// `md` is a file suffix and `:markdown`'s alias, `tw` is OpenCC's 臺灣正體 in
/// the `:convert` table, and `[]` is a pair of brackets being shown. Nothing
/// can be concluded from these in either direction — `m d` happens to be a real
/// sequence and `t w` is one too — so they are dropped before the question is
/// asked rather than answered.
const NOT_A_SEQUENCE: &[&str] = &["md", "tw", "[]"];

/// Sequences the documents print **in order to say the editor has not got them**.
///
/// `gt`／`gc`／`gb` (跳到屏幕頂／中／底) stand in the 「明說出來，免得你去找」
/// list beside 多光標, absent for the reason stated there. `t s` is printed to
/// say 「光按 `t s` 什麼都不會發生」: the sort is real, but it must be told a
/// column first, which is why the menu spells it `1s` and not `s`. Checked from
/// the other side by [`each_key_the_documents_disown_really_is_missing`], so
/// this cannot become the place a stale key goes to hide.
const DISOWNED_KEYS: &[&str] = &["gt", "gc", "gb", "t s"];

#[test]
fn every_key_sequence_the_documents_print_is_one_the_editor_offers() {
    let mut asked = 0;
    for (doc, text) in documents() {
        for (line, quote) in quoted(&text) {
            if NOT_A_SEQUENCE.contains(&quote.as_str())
                || DISOWNED_KEYS.contains(&quote.as_str())
            {
                continue;
            }
            let Some((lead, key)) = leader_and_key(&quote) else { continue };
            // A leader with no list is one that takes any character — `f`, `r`,
            // `"`, `M` — and every two-letter word in the manual would be one
            // of those if this did not stop here.
            let Some(offers) = Editor::keys_after(lead) else { continue };
            assert!(
                offers.iter().any(|k| *k == key),
                "{doc}:{line} teaches `{quote}`, and the {lead} menu does not offer `{key}`"
            );
            asked += 1;
        }
    }
    assert!(asked >= 150, "only {asked} key sequences read out of the documents");
}

#[test]
fn each_key_the_documents_disown_really_is_missing() {
    for quote in DISOWNED_KEYS {
        let Some((lead, key)) = leader_and_key(quote) else { continue };
        let Some(offers) = Editor::keys_after(lead) else { continue };
        assert!(
            !offers.iter().any(|k| *k == key),
            "`{quote}` is a key sequence the editor offers — it does not belong on \
             the list of pairs that only look like one"
        );
    }
}
