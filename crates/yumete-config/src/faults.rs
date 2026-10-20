//! **What is wrong with a config file, line by line** (2026-10-20 作者定).
//!
//! The start-up line has said 「{本地|全局}配置檔出錯，使用出廠設置」 since
//! 2026-10-10, and that is the whole of what a reader got: one sentence, at
//! launch, about a file they were not looking at. 原話 that day, written down
//! as asked-for-and-not-built (§5.155 ②): 「LSP-shaped diagnostics over that
//! file — an *error* on the line that makes the whole file invalid, a
//! *warning* on a key or value that parses but is never applied.」
//!
//! This is that, and it needs no language server: the checks already exist
//! ([`Config::from_toml_saying`]), the editor already draws other people's
//! complaints ([`yumete_core::editor::Editor::set_problems`]), and the only
//! thing missing was **which line** each one is about.
//!
//! Warning: **the line comes from the document, not from the sentence.** Every
//! complaint this crate writes opens with the key it is about, spelt the way
//! the file spells it (`[editor] zong_length = …`), so the address is read off
//! the front of the sentence and then *looked up* in the parsed document —
//! `toml_edit` keeps spans, which is why it is a dependency. A sentence whose
//! key is not in the file lands on line 0, which is where 「this whole file」
//! belongs anyway.

use crate::{Config, RawConfig};

/// One thing wrong with a config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// Which line, 0 起算.
    pub line: usize,
    /// **An error stops the whole file**, a warning is one key that parses and
    /// is then never read. The difference is the one a reader needs: TOML is
    /// all-or-nothing, so the first kind silently takes forty other settings
    /// with it.
    pub fatal: bool,
    /// The sentence, as the loader would have said it at start-up.
    pub said: String,
}

/// Everything wrong with this config text.
///
/// **At most one fatal fault**, because `serde` stops at the first thing it
/// cannot read — and a file that does not parse has no keys to check, so the
/// warnings are not reached at all. That is not a gap in this walk; it is what
/// the loader does with the file.
pub fn faults_in(text: &str) -> Vec<Fault> {
    match toml::from_str::<RawConfig>(text) {
        Err(err) => {
            let line = match err.span() {
                Some(at) => line_at(text, at.start),
                None => 0,
            };
            let detail = err.message().lines().next().unwrap_or_default().to_string();
            vec![Fault { line, fatal: true, said: detail }]
        }
        Ok(raw) => {
            // **Both halves of the loader's own checking.** `complain_about`
            // reads the values as written; `into_config_saying` says what it
            // had to clamp or drop while building the `Config`. The start-up
            // line runs both, so this must too, or the file would look
            // cleaner open in the editor than it did at launch.
            let mut said = Vec::new();
            Config::complain_about(&raw, &mut said);
            let _ = raw.into_config_saying(&mut said);
            said.into_iter()
                .map(|said| Fault { line: line_for(text, &said), fatal: false, said })
                .collect()
        }
    }
}

/// **Which line a complaint is about**, by the key it opens with.
///
/// Public because the four checks that need `yumete-core`'s parsers live in
/// the front end (`yumete_tui::settings::complaints`) and are written in the
/// same shape; they would otherwise have to guess.
pub fn line_for(text: &str, said: &str) -> usize {
    let Some((table, key)) = addressed(said) else { return 0 };
    // Warning: **`ImDocument`, not `DocumentMut`** (measured 2026-10-20): the mutable
    // document drops every span on the way in, so `Key::span()` is `None` for
    // all of them and every warning would land on line 0 — which looks like
    // 「this whole file」 and is the one thing a warning must not say.
    let Ok(doc) = toml_edit::ImDocument::parse(text) else { return 0 };
    let mut item = doc.as_item();
    for part in table.split('.') {
        let Some(next) = item.get(part) else { return 0 };
        item = next;
    }
    // The key itself, and failing that the table's own header — 「somewhere in
    // this table」 beats 「the top of the file」.
    let at = item
        .as_table()
        .and_then(|t| t.get_key_value(&key))
        .and_then(|(k, _)| k.span())
        .or_else(|| item.span());
    match at {
        Some(at) => line_at(text, at.start),
        None => 0,
    }
}

/// `[editor] zong_length = …` → `("editor", "zong_length")`.
///
/// Warning: **this reads the front of our own sentence**, which is only sound
/// because every complaint in this crate is written that way — and
/// `tests/faults.rs` is what keeps it true: it feeds a file with one mistake
/// per checkable key and asserts every complaint found a line.
fn addressed(said: &str) -> Option<(String, String)> {
    let (table, rest) = said.strip_prefix('[')?.split_once(']')?;
    let key: String = rest
        .trim_start()
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '=' && *c != '：' && *c != ':')
        .collect();
    (!key.is_empty() && !table.is_empty()).then_some((table.to_string(), key))
}

/// Which line a byte offset is on, 0 起算.
fn line_at(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].matches('\n').count()
}
