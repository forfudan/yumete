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

/// **那六張表就是 `RawConfig` 裏所有結構體形狀的表** —— 再加一張，這條紅。
///
/// ⚠️ [`every_key`] 走的是一張**手抄**的表名單子，而那個模組的文檔自己寫着「一張
/// 手抄的字段名單正是這條測試要防的東西」。手抄的表名單子是同一件事，只是矮了一層：
/// 加一個 `RawFoo` 進 `RawConfig` 而忘了加進那張單子，它底下每一個鍵都沒人管，而
/// 「每個設定都有着落」那條照樣綠。2026-09-24 審出來的。
///
/// ⚠️ **鍵集開放的表不算**（`syntax`／`sidebar`／`language`／`lsp`，都是
/// `HashMap`）：它們的鍵是擴展名、面板名、語言名——任意的，數不出一張名單來，所以
/// 「每個鍵都有着落」對它們沒有意義。`NOT_IN_THE_PANEL` 的判準二說的就是這一族。
#[test]
fn the_six_tables_are_every_table_that_has_a_shape() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
    )
    .expect("yumete-config/src/lib.rs");
    let start = source.find("\nstruct RawConfig {").expect("struct RawConfig");
    let end = source[start..].find("\n}").map(|n| start + n).unwrap_or(source.len());
    let shaped: Vec<String> = source[start..end]
        .lines()
        .filter_map(|line| {
            let (name, ty) = line.trim().trim_end_matches(',').split_once(": ")?;
            // `RawEditor` 有形狀；`HashMap<…>` 沒有。
            ty.starts_with("Raw").then(|| name.to_string())
        })
        .collect();
    assert!(shaped.len() >= 5, "一個帶形狀的表都沒讀出來——`RawConfig` 的寫法變了？");
    let walked = ["editor", "theme", "panel", "ime", "keys", "export"];
    let missed: Vec<&String> = shaped.iter().filter(|t| !walked.contains(&t.as_str())).collect();
    assert!(
        missed.is_empty(),
        "`RawConfig` 多了帶形狀的表而 `every_key` 沒走它：{missed:?}\n         走它（把表名加進 `every_key`），那張表底下的每個鍵纔算有人管。"
    );
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
            // ⚠️ **每一個詞都要試，不是只試第一個。** 從前是 `find(|w| *w != now)`
            // ——取排在最前面的那個非出廠詞，於是十六組幾選一裏**四十六個詞只有
            // 十六個被試過**。而這條測試逮到的那一次（`ime.system` 寫了兩個不存在
            // 的詞）純屬運氣：壞詞正好排第二。排第三就逃掉了。
            // 2026-09-24 審出來的。
            Kind::Pick(choices) => {
                let now = setting.factory.trim_matches('"');
                for choice in choices.iter().filter(|c| c.word != now) {
                    assert_ne!(
                        Config::from_toml(&one(setting, &format!("\"{}\"", choice.word))),
                        nothing_said(),
                        "{} 設成 {:?} 之後什麽都沒變——那個詞 `into_config` 不認得，\
                         而面板照樣讓人選、照樣寫進 toml",
                        setting.path(),
                        choice.word
                    );
                }
                continue;
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

/// **面板停得到的每一個值，編輯器都得原樣收下。**
///
/// ⚠️ **這一條是「面板顯示的值不是編輯器用的值」那一族的網。** `into_config` 有幾
/// 處的域是**斷的**——`zong_length` 是「0，或者 4 到 64」，`tatechuyoko` 是「0，或者
/// 2 到 8」。把 `low` 寫成 0，面板就停得到 1／2／3，寫進檔裏而編輯器按 4 排版，**編譯
/// 照過、別的測試照綠**。2026-09-24 審出來的，兩項都中。
///
/// 判準不用反射也說得出來：**域裏相鄰的兩個值，讀回來的 `Config` 必須不同**。
/// 一樣就說明其中一個被鉗掉了——它不在域裏，而面板讓人停在了那裏。
#[test]
fn every_value_the_panel_can_set_is_its_own() {
    use yumete_config::settings_ui::{Kind, SETTINGS};
    use yumete_config::Config;
    for setting in SETTINGS {
        let Kind::Count { low, high, zero } = setting.kind else { continue };
        // 域：`low..=high`，再加上 0 —— **只有 `low > 0` 的時候**。
        //
        // ⚠️ **`zero` 有兩種意思，這裏要分開。** `zong_length` 的 0 在域**外面**
        // 另成一檔（域是 4..64）；`indent` 的 0 就在域裏（0..8），`zero` 只是替
        // 它取了個名字（「不縮進」）。判準是 `low`：大於 0 纔說明 0 是另一檔。
        let mut domain: Vec<usize> = Vec::new();
        if zero.is_some() && low > 0 {
            domain.push(0);
        }
        domain.extend(low..=high);
        for pair in domain.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert_ne!(
                Config::from_toml(&one(setting, &a.to_string())),
                Config::from_toml(&one(setting, &b.to_string())),
                "{} 設成 {a} 和設成 {b} 讀回來一模一樣——其中一個被 into_config 鉗掉了，\
                 而面板讓人停在那裏（域是斷的？`low` 該是最小的**非零**值）",
                setting.path()
            );
        }
    }
}

/// **出廠值那一欄要是一個讀得出來的 toml 值，而且類型對得上。**
///
/// ⚠️ **上面那兩條堵不住引號。** `Pick` 的 `factory` 少寫一對引號（`horizontal`
/// 而不是 `"horizontal"`）→ toml 解析失敗 → `from_toml` 走 `unwrap_or_default()`
/// → **和空配置逐位元組相同** → 第一條綠；第二條自己拼 `format!("\"{other}\"")`，
/// 永遠帶引號 → 也綠。而面板會拿 `horizontal` 去和 `written()` 出來的
/// `"horizontal"` 比，`settle` 從此判不出「轉回原值」。2026-09-24 審出來的。
#[test]
fn every_factory_value_parses_as_the_kind_it_says_it_is() {
    use yumete_config::settings_ui::{Kind, SETTINGS};
    for setting in SETTINGS {
        let one = format!("v = {}\n", setting.factory);
        let read: toml::Value = toml::from_str(&one)
            .unwrap_or_else(|e| panic!("{} 的出廠值 {} 讀不出來：{e}", setting.path(), setting.factory));
        let v = read.get("v").expect("v");
        let fits = match setting.kind {
            Kind::Tick => v.is_bool(),
            Kind::Count { .. } => v.is_integer(),
            Kind::Pick(_) | Kind::Text => v.is_str(),
        };
        assert!(
            fits,
            "{} 說自己是 {:?}，而出廠值 {} 讀出來是 {}",
            setting.path(),
            setting.kind,
            setting.factory,
            v.type_str()
        );
        // 幾選一的出廠值還得真的在那張選項單子上。
        if let Kind::Pick(choices) = setting.kind {
            let word = v.as_str().unwrap_or_default();
            assert!(
                choices.iter().any(|c| c.word == word),
                "{} 的出廠值 {word:?} 不在選項裏：{:?}",
                setting.path(),
                choices.iter().map(|c| c.word).collect::<Vec<_>>()
            );
        }
    }
}
