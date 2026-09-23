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

// ── 出廠值那一欄 ────────────────────────────────────────────────────────
//
// `Setting::factory` 是**抄來的**字面量（面板要說「你沒設，出廠是這個」），所以
// 它一定會漂——除非有人盯着。下面兩條就是盯着的那兩隻眼睛，而且**都不寫一行
// per-key 的代碼**：表長到八十七項的那天，它們照樣管用。

/// **出廠態 —— 讀一份什麽都沒說的配置。**
///
/// ⚠️ **不是 `Config::default()`。** 那一個的 `lsp` 是空的，而 `into_config` 會替
/// 沒寫 `[lsp.*]` 的配置填上出廠的三個語言服務器（`factory_servers`）。面板眼裏的
/// 「出廠」是後者：一份空配置跑出來的樣子。
fn nothing_said() -> yumete_config::Config {
    yumete_config::Config::from_toml("")
}

/// 一項設定拼成一行 toml。
fn one(setting: &yumete_config::settings_ui::Setting, value: &str) -> String {
    format!("[{}]\n{} = {}\n", setting.table, setting.key, value)
}

/// **把整張表的出廠值拼成一份配置讀回來，必須就是出廠設定。**
///
/// 抄錯一個數當場紅。
#[test]
fn every_factory_value_really_is_the_factory_value() {
    use yumete_config::settings_ui::SETTINGS;
    use yumete_config::Config;
    for setting in SETTINGS {
        let built = Config::from_toml(&one(setting, setting.factory));
        assert_eq!(
            built,
            nothing_said(),
            "{} 的出廠值寫的是 {}，而那不是出廠設定",
            setting.path(),
            setting.factory
        );
    }
}

/// **再逐項餵一個不是出廠的值，必須真的改出點什麽來。**
///
/// ⚠️ **這一條纔是硬的**，上面那條單獨立不住：`RawConfig` 是
/// `#[serde(deny_unknown_fields)]`，而 `Config::from_toml` 讀不進去就
/// `unwrap_or_default()`——**鍵名打錯一個字母，整份被默默丟掉，於是「等於出廠
/// 設定」照樣成立**，上面那條全綠。這一條問的是反面：既然改了，就該有東西動。
#[test]
fn a_value_that_is_not_the_factory_one_actually_lands() {
    use yumete_config::settings_ui::{Kind, SETTINGS};
    use yumete_config::Config;
    for setting in SETTINGS {
        // 一個**一定不是**出廠的值，從 `Kind` 推出來——不手寫。
        let other = match setting.kind {
            Kind::Tick => match setting.factory {
                "true" => "false".to_string(),
                _ => "true".to_string(),
            },
            Kind::Count { low, high, .. } => {
                let now: usize = setting.factory.parse().unwrap_or(low);
                match now == low {
                    true => high.to_string(),
                    false => low.to_string(),
                }
            }
            Kind::Pick(choices) => {
                let now = setting.factory.trim_matches('"');
                let other = choices
                    .iter()
                    .map(|c| c.word)
                    .find(|w| *w != now)
                    .unwrap_or_else(|| panic!("{} 的選項全同名", setting.path()));
                format!("\"{other}\"")
            }
            // 一段自己打的字沒有「另一個值」可推——跳過，那一族靠上面那條。
            Kind::Text => continue,
        };
        assert_ne!(
            Config::from_toml(&one(setting, &other)),
            nothing_said(),
            "{} 設成 {other} 之後什麽都沒變——鍵名打錯了？（打錯的話整份配置被丟掉，\
             而丟掉之後正好等於出廠設定，所以另一條測試是綠的）",
            setting.path()
        );
    }
}
