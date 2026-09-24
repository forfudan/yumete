//! **把一項設定寫回 toml，而不動旁邊的任何一個字**（2026-09-23）。
//!
//! 那扇設置面板要存盤，而使用者的 `config.toml` 是**他自己寫的**：帶着他的註釋、
//! 他排的次序、他留的空行，而且多半在 git 裏。所以判準只有一條：
//!
//! **改的那一行改掉，別的一個字節都不許動。**
//!
//! ⚠️ **`toml` 這個 crate 做不到。** 它只導出 `from_str`／`to_string`，註釋活在
//! `toml_edit::Decor` 裏，一趟 round-trip 丟掉的不只是註釋——鍵的次序、空行、行
//! 內註釋、引號的寫法全丟。一份手寫的配置存一次就被抹平成一份機器寫的。
//!
//! ⚠️ **而 `toml_edit` 加進來是零成本**：`toml 0.8` 的 `default = ["parse",
//! "display"]` 兩個 feature 都是 `dep:toml_edit`，`Cargo.lock` 裏就躺着
//! `toml_edit 0.22.27`。寫一行依賴，**新增 crate 0 個、編譯時間增量 0**——同
//! `serde_json`（#53）與 `libc` 的準入理由逐字相同：**it was already in the
//! lock file**。
//!
//! # 不認得的鍵
//!
//! `RawConfig` 是 `#[serde(deny_unknown_fields)]`，所以**一個打錯的鍵名會扔掉整
//! 份配置**。實測（2026-09-23）：
//!
//! ```toml
//! [editor]
//! line_numbers = "absolute"
//! nosuchkey = 42
//! soft_wrap = true
//! ```
//!
//! stderr 上報一行，而同一個檔裏那兩條對的設定**一條都沒生效**。
//!
//! 所以存盤要管這件事。作者 2026-09-23 定的是「移除」，商量之後改成**註釋掉**：
//!
//! ```toml
//! # nosuchkey = 42    # yumete 不認得這個名字
//! ```
//!
//! 三個理由：**什麽都沒丟**（那可能是從新版文檔抄來的）、**檔當場能用了**、
//! **看得見發生過什麽**（刪掉是無聲的）。

use toml_edit::{DocumentMut, Item, Value};

/// 一次寫回要做的事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// 把 `表.鍵` 設成一個布爾。
    Tick(bool),
    /// …一個整數。
    Count(i64),
    /// …一段字。
    Text(String),
    /// **把這一項從這一份裏拿掉** —— 本項目那一份上的 `d`：撤掉覆蓋，回去跟全局。
    ///
    /// ⚠️ 這件事**整份重寫做不到**：刪一個鍵還要留着它周圍的註釋，只有保留格式
    /// 的那條路走得通。這是 `toml_edit` 的第二個硬理由。
    Drop,
}

/// 寫回之後，有話說的那幾句。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Said {
    /// 註釋掉了的那些鍵（`表.鍵`）。
    pub commented_out: Vec<String>,
}

/// **一份 toml，改幾處，字節級別地不動別的。**
///
/// `text` 是檔裏原樣的那一份（空字串 ＝ 還沒有這個檔）。`changes` 是
/// `("表.鍵", 怎麽改)`。回來的是新的正文與有話說的那幾句。
///
/// ⚠️ **不認得的鍵在這裏一併註釋掉**，因為它們和「存盤」是同一件事：面板存完盤
/// 之後那個檔必須是**讀得進去的**，否則使用者按了保存而設定一條都沒生效。
pub fn rewrite(
    text: &str,
    changes: &[(String, Change)],
    known: &dyn Fn(&str, &str) -> bool,
) -> Result<(String, Said), String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let mut said = Said::default();

    for (path, change) in changes {
        let Some((table, key)) = path.split_once('.') else {
            return Err(format!("{path} 不是「表.鍵」"));
        };
        match change {
            Change::Drop => {
                if let Some(Item::Table(t)) = doc.get_mut(table) {
                    t.remove(key);
                }
            }
            _ => {
                // ⚠️ **`or_insert` 一張 implicit table 會寫成 `[表]` 一行**，
                // 而那正是我們要的：檔裏沒有 `[editor]` 的時候補一個出來。
                let entry = doc
                    .entry(table)
                    .or_insert_with(|| Item::Table(toml_edit::Table::new()));
                let Some(t) = entry.as_table_mut() else {
                    return Err(format!("{table} 不是一張表"));
                };
                let value: Value = match change {
                    Change::Tick(on) => (*on).into(),
                    Change::Count(n) => (*n).into(),
                    Change::Text(s) => s.as_str().into(),
                    Change::Drop => unreachable!("上面那一支接住了"),
                };
                match t.get_mut(key) {
                    // ⚠️ **只換值，不換這一行。** `t[key] = value!(…)` 會把這一
                    // 行連 decor 一起換掉——行尾那句 `# 我為什麽這麽設` 就沒了。
                    Some(Item::Value(old)) => {
                        let decor = old.decor().clone();
                        let mut fresh = value;
                        *fresh.decor_mut() = decor;
                        *old = fresh;
                    }
                    _ => {
                        t.insert(key, Item::Value(value));
                    }
                }
            }
        }
    }

    // 不認得的鍵：註釋掉，並說出來。
    for table in ["editor", "theme", "panel", "ime", "keys", "export"] {
        let Some(Item::Table(t)) = doc.get_mut(table) else { continue };
        let strangers: Vec<String> = t
            .iter()
            .map(|(k, _)| k.to_string())
            .filter(|k| !known(table, k))
            .collect();
        for key in strangers {
            let Some(item) = t.get(&key) else { continue };
            let line = match item.as_value() {
                Some(v) => format!("{key} ={}", v.to_string().trim_end()),
                None => continue,
            };
            t.remove(&key);
            said.commented_out.push(format!("{table}.{key}"));
            // ⚠️ **掛在表頭那一行的後綴上**，所以它畫出來是在 `[editor]` **下面
            // 一行**，而不是原來那個位置。留在本節裏就夠了——挪一行註釋掉的字比
            // 「為了位置精確而動別人的行」便宜。
            //
            // ⚠️ 連帶一件事：它會擠在上一行註釋和它注的那一行中間。實測
            // （2026-09-24）：
            //
            // ```toml
            // [editor]
            // # nosuchkey = 42    # yumete does not know this name
            // # 竪排
            // layout = "vertical"
            // ```
            let decor = t.decor_mut();
            let had = decor.suffix().and_then(|s| s.as_str()).unwrap_or("").to_string();
            decor.set_suffix(format!("{had}\n# {line}    # {STRANGER}"));
        }
    }

    Ok((doc.to_string(), said))
}

/// 註釋掉那一行後面跟的那句話。
///
/// ⚠️ **不走 `say!`。** 它寫進使用者的檔裏，而那個檔明天可能被另一種語言的
/// yumete 讀到；一句跟着界面語言變的註釋會讓同一個檔在兩臺機器上長得不一樣。
const STRANGER: &str = "yumete does not know this name";

#[cfg(test)]
mod tests {
    use super::*;

    fn known(table: &str, key: &str) -> bool {
        matches!(
            (table, key),
            ("editor", "line_numbers") | ("editor", "soft_wrap") | ("editor", "indent")
        )
    }

    /// **這一條是整件事的理由**：使用者寫的東西一個字都不許掉。
    #[test]
    fn everything_the_writer_put_there_survives() {
        let before = "\
# 這是我自己寫的，別動
[editor]
# 行號要絕對的
line_numbers = \"absolute\"

soft_wrap = true    # 行尾這一句也是我寫的
";
        let (after, said) =
            rewrite(before, &[("editor.indent".into(), Change::Count(2))], &known).unwrap();
        for kept in [
            "# 這是我自己寫的，別動",
            "# 行號要絕對的",
            "line_numbers = \"absolute\"",
            "# 行尾這一句也是我寫的",
        ] {
            assert!(after.contains(kept), "掉了 {kept:?}：\n{after}");
        }
        assert!(after.contains("indent = 2"), "新的那一項寫進去了：\n{after}");
        assert!(said.commented_out.is_empty());
    }

    /// 改一個已經在的鍵，**行尾那句註釋要留着**。
    #[test]
    fn changing_a_value_keeps_the_note_beside_it() {
        let before = "[editor]\nindent = 2    # 中文慣例\n";
        let (after, _) =
            rewrite(before, &[("editor.indent".into(), Change::Count(4))], &known).unwrap();
        assert!(after.contains("indent = 4"), "{after}");
        assert!(after.contains("# 中文慣例"), "行尾那句沒了：\n{after}");
    }

    /// `d` 撤掉本項目的覆蓋 —— 鍵沒了，周圍的註釋還在。
    #[test]
    fn dropping_a_key_leaves_its_neighbours_alone() {
        let before = "[editor]\n# 上面這一句\nline_numbers = \"absolute\"\nsoft_wrap = true\n";
        let (after, _) =
            rewrite(before, &[("editor.soft_wrap".into(), Change::Drop)], &known).unwrap();
        assert!(!after.contains("soft_wrap"), "撤掉了：\n{after}");
        assert!(after.contains("# 上面這一句"), "{after}");
        assert!(after.contains("line_numbers"), "{after}");
    }

    /// **不認得的鍵註釋掉，而不是刪掉**（2026-09-23 定）。
    ///
    /// ⚠️ 留着它，`deny_unknown_fields` 會讓**整份**配置作廢——同一個檔裏那兩條
    /// 對的設定一條都不生效。所以這不是破壞，是修復。
    #[test]
    fn a_name_yumete_does_not_know_is_commented_out_not_deleted() {
        let before = "[editor]\nline_numbers = \"absolute\"\nnosuchkey = 42\nsoft_wrap = true\n";
        let (after, said) = rewrite(before, &[], &known).unwrap();
        assert_eq!(said.commented_out, vec!["editor.nosuchkey"]);
        assert!(!after.contains("\nnosuchkey"), "不再是一個活的鍵：\n{after}");
        assert!(after.contains("# nosuchkey = 42"), "但字還在：\n{after}");
        assert!(after.contains(STRANGER), "而且說了為什麽：\n{after}");
        // 那兩條對的照舊。
        assert!(after.contains("line_numbers = \"absolute\""), "{after}");
        assert!(after.contains("soft_wrap = true"), "{after}");
    }

    /// 檔裏還沒有這一張表，就補一張。
    #[test]
    fn a_table_that_is_not_there_yet_is_written() {
        let (after, _) =
            rewrite("", &[("editor.soft_wrap".into(), Change::Tick(true))], &known).unwrap();
        assert!(after.contains("[editor]"), "{after}");
        assert!(after.contains("soft_wrap = true"), "{after}");
    }

    /// 寫回去的東西要**讀得回來** —— 這一條是整條路的驗收。
    #[test]
    fn what_goes_out_parses_back_in() {
        let before = "# 頭上一句\n[editor]\nnosuchkey = 1\nindent = 2\n";
        let (after, _) = rewrite(
            before,
            &[
                ("editor.indent".into(), Change::Count(4)),
                ("editor.soft_wrap".into(), Change::Tick(false)),
            ],
            &known,
        )
        .unwrap();
        let back: toml::Value = toml::from_str(&after).expect("讀得回來");
        let editor = back.get("editor").and_then(|t| t.as_table()).expect("[editor]");
        assert_eq!(editor.get("indent").and_then(|v| v.as_integer()), Some(4));
        assert_eq!(editor.get("soft_wrap").and_then(|v| v.as_bool()), Some(false));
        assert!(editor.get("nosuchkey").is_none(), "那個名字不再是一個鍵");
    }

    /// 壞掉的 toml 不許被「修」成別的東西——原樣報錯，一個字不寫。
    #[test]
    fn a_broken_file_is_refused_rather_than_rewritten() {
        assert!(rewrite("[editor\nindent = 2\n", &[], &known).is_err());
    }
}
