//! **那張設置表和 `RawConfig` 對不對得上** —— 讀源碼核，不靠記性。
//!
//! ⚠️ **少一項是一個無聲的洞**：加了新設定而忘了加進 `settings_ui::SETTINGS`，
//! 面板上就沒有它，而編譯照過、面板照樣顯示別的設定的對的值。記事本裏那條「全局
//! 設置加一項要動三處，漏掉第三處編譯照過、面板照樣顯示對的值」說的就是這一族。
//!
//! 讀源碼是這個倉已有的辦法（`yumete-core/tests/messages.rs` 讀 `say!`）——
//! Rust 沒有反射，而一張**手抄的**字段名單正是這條測試要防的東西。

use std::collections::BTreeSet;

/// `struct RawXxx { … }` 裏那些 `名字: 類型,` 的名字。
fn fields_of(source: &str, what: &str) -> Vec<String> {
    let start = match source.find(&format!("\nstruct {what} {{")) {
        Some(at) => at,
        None => panic!("找不到 struct {what}"),
    };
    let end = source[start..].find("\n}").map(|n| start + n).unwrap_or(source.len());
    source[start..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim_end();
            let name = line.strip_prefix("    ")?;
            // `    名字: 類型,` —— 屬性、註釋、嵌套都不是這個形狀。
            let (name, _) = name.split_once(':')?;
            let ok = !name.is_empty()
                && name.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit());
            ok.then(|| name.to_string())
        })
        .collect()
}

/// 每一個 `RawXxx` 的字段，寫成 `表.鍵`。
fn every_key() -> BTreeSet<String> {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
    )
    .expect("yumete-config/src/lib.rs");
    let mut out = BTreeSet::new();
    for (table, what) in [
        ("editor", "RawEditor"),
        ("theme", "RawTheme"),
        ("panel", "RawPanel"),
        ("ime", "RawIme"),
        ("keys", "RawKeys"),
        ("export", "RawExport"),
    ] {
        let found = fields_of(&source, what);
        assert!(!found.is_empty(), "{what} 一個字段都沒讀出來——形狀變了？");
        for key in found {
            out.insert(format!("{table}.{key}"));
        }
    }
    out
}

#[test]
fn every_setting_names_a_key_that_exists() {
    let real = every_key();
    let wrong: Vec<String> = yumete_config::settings_ui::SETTINGS
        .iter()
        .map(|s| s.path())
        .filter(|path| !real.contains(path))
        .collect();
    assert!(
        wrong.is_empty(),
        "設置表裏這幾個鍵 `RawConfig` 裏沒有——改了名字，還是打錯了？\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn every_key_is_either_in_the_panel_or_accounted_for() {
    use yumete_config::settings_ui::{LATER, NOT_IN_THE_PANEL, SETTINGS};
    let spoken: BTreeSet<String> = SETTINGS
        .iter()
        .map(|s| s.path())
        .chain(LATER.iter().map(|s| s.to_string()))
        .chain(NOT_IN_THE_PANEL.iter().map(|s| s.to_string()))
        .collect();
    let missing: Vec<String> = every_key().into_iter().filter(|k| !spoken.contains(k)).collect();
    assert!(
        missing.is_empty(),
        "這幾個設定沒人管——放進 SETTINGS（做了）、LATER（還沒輪到）或 \
         NOT_IN_THE_PANEL（有理由不做）裏的一張：\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn nothing_is_listed_twice() {
    use yumete_config::settings_ui::{LATER, NOT_IN_THE_PANEL, SETTINGS};
    let mut seen = BTreeSet::new();
    let mut twice = Vec::new();
    for path in SETTINGS
        .iter()
        .map(|s| s.path())
        .chain(LATER.iter().map(|s| s.to_string()))
        .chain(NOT_IN_THE_PANEL.iter().map(|s| s.to_string()))
    {
        if !seen.insert(path.clone()) {
            twice.push(path);
        }
    }
    assert!(twice.is_empty(), "同一個鍵列了兩遍：\n  {}", twice.join("\n  "));
}

/// 取值域是**抄 `into_config`** 的，所以至少要自洽。
#[test]
fn every_range_is_a_range() {
    use yumete_config::settings_ui::{Kind, SETTINGS};
    for setting in SETTINGS {
        if let Kind::Count { low, high, .. } = setting.kind {
            assert!(low <= high, "{} 的範圍是反的：{low}..{high}", setting.path());
        }
        if let Kind::Pick(choices) = setting.kind {
            assert!(choices.len() >= 2, "{} 只有一個選項，那不是幾選一", setting.path());
        }
    }
}
