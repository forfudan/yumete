//! **把一項設定寫回 toml，而不動旁邊的任何一個字**（2026-09-23）。
//!
//! 那扇設置面板要存盤，而使用者的 `config.toml` 是**他自己寫的**：帶着他的註釋、
//! 他排的次序、他留的空行，而且多半在 git 裏。所以判準只有一條：
//!
//! **改的那一行改掉，別的一個字節都不許動。**
//!
//! Warning: **`toml` 這個 crate 做不到。** 它只導出 `from_str`/`to_string`，註釋活在
//! `toml_edit::Decor` 裏，一趟 round-trip 丟掉的不只是註釋——鍵的次序、空行、行
//! 內註釋、引號的寫法全丟。一份手寫的配置存一次就被抹平成一份機器寫的。
//!
//! Warning: **而 `toml_edit` 加進來是零成本**：`toml 0.8` 的 `default = ["parse",
//! "display"]` 兩個 feature 都是 `dep:toml_edit`，`Cargo.lock` 裏就躺着
//! `toml_edit 0.22.27`。寫一行依賴，**新增 crate 0 個、編譯時間增量 0**——同
//! `serde_json`（#53）與 `libc` 的準入理由逐字相同：**it was already in the
//! lock file**。
//!
//! # 不認得的鍵
//!
//! **原樣留着**（2026-11-08）。從前存盤會把面板不認得的鍵註釋掉、挪到表頭底下，
//! 理由是 `deny_unknown_fields` 讓一個打錯的鍵名作廢整份配置；可那是改了使用者没
//! 叫改的行——他只改了 `scrolloff`，存完 `typo_key` 被註釋掉、次序也變了。現在
//! 一份讀不進去的檔面板頂上就說出來，`:w` 不寫它，要改請 `e` 打開那個檔
//! （[`crate::panel::Panel::save`]）。

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
    /// Warning: 這件事**整份重寫做不到**：刪一個鍵還要留着它周圍的註釋，只有保留格式
    /// 的那條路走得通。這是 `toml_edit` 的第二個硬理由。
    Drop,
    /// …the value **as it was written**, a TOML value such as `"dense"`,
    /// `8` or `'Songti SC'` (2026-11-08): what `u` puts back, in the very
    /// spelling the file had.
    Written(String),
}

/// The table `tables` names, one inside the next, if the file has it.
fn table_at<'a>(doc: &'a mut DocumentMut, tables: &[&str]) -> Option<&'a mut toml_edit::Table> {
    let mut t = doc.get_mut(tables.first()?)?.as_table_mut()?;
    for name in &tables[1..] {
        unfold(t, name);
        t = t.get_mut(name)?.as_table_mut()?;
    }
    Some(t)
}

/// A table written inline (`flavor_items = { tables = false }`) becomes a
/// table of its own lines, so a key can be set in it or taken out of it.
fn unfold(t: &mut toml_edit::Table, name: &str) {
    if let Some(item) = t.get_mut(name) {
        if let Some(inline) = item.as_inline_table() {
            *item = Item::Table(inline.clone().into_table());
        }
    }
}

/// **一份 toml，改幾處，字節級別地不動別的。**
///
/// `text` 是檔裏原樣的那一份（空字串 ＝ 還没有這個檔）。`changes` 是
/// `("表.鍵", 怎麽改)`。回來的是新的正文；檔讀不通就回解析器的話，一個字不寫。
pub fn rewrite(text: &str, changes: &[(String, Change)]) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;

    for (path, change) in changes {
        // `表.鍵`, or `表.子表.鍵` for a table inside a table (2026-11-02:
        // `keys.flavor_items.esc_collapses`).
        let Some((tables, key)) = path.rsplit_once('.') else {
            return Err(format!("{path} 不是「表.鍵」"));
        };
        let tables: Vec<&str> = tables.split('.').collect();
        match change {
            Change::Drop => {
                if let Some(t) = table_at(&mut doc, &tables) {
                    t.remove(key);
                }
                // A table inside a table that has nothing left goes too, from
                // the innermost out: an empty `[keys.flavor_items]` or
                // `[ime.annotation_modes.double]` says nothing.
                let mut emptied = true;
                for depth in (1..tables.len()).rev() {
                    let Some(outer) = table_at(&mut doc, &tables[..depth]) else {
                        emptied = false;
                        break;
                    };
                    let empty = matches!(outer.get(tables[depth]), Some(Item::Table(inner)) if inner.is_empty());
                    if !empty {
                        emptied = false;
                        break;
                    }
                    outer.remove(tables[depth]);
                }
                // …and so does the outermost one.
                if emptied {
                    drop_if_empty(&mut doc, tables[0]);
                }
            }
            _ => {
                // Warning: **`or_insert` 一張 implicit table 會寫成 `[表]` 一行**，
                // 而那正是我們要的：檔裏没有 `[editor]` 的時候補一個出來。一張只為
                // 裝子表而補出來的外層表是 implicit 的，不另寫一行空的 `[keys]`。
                let fresh_outer = doc.get(tables[0]).is_none();
                let entry = doc
                    .entry(tables[0])
                    .or_insert_with(|| Item::Table(toml_edit::Table::new()));
                let Some(mut t) = entry.as_table_mut() else {
                    return Err(format!("{} 不是一張表", tables[0]));
                };
                if fresh_outer && tables.len() > 1 {
                    t.set_implicit(true);
                }
                for name in &tables[1..] {
                    unfold(t, name);
                    let inner = t.entry(name).or_insert_with(|| Item::Table(toml_edit::Table::new()));
                    let Some(inner) = inner.as_table_mut() else {
                        return Err(format!("{name} 不是一張表"));
                    };
                    t = inner;
                }
                let value: Value = match change {
                    Change::Tick(on) => (*on).into(),
                    Change::Count(n) => (*n).into(),
                    Change::Text(s) => s.as_str().into(),
                    Change::Written(repr) => repr.parse::<Value>().unwrap_or_else(|_| repr.as_str().into()),
                    Change::Drop => unreachable!("上面那一支接住了"),
                };
                match t.get_mut(key) {
                    // Warning: **只換值，不換這一行。** `t[key] = value!(…)` 會把這一
                    // 行連 decor 一起換掉——行尾那句 `# 我為什麽這麽設` 就没了。
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
    Ok(doc.to_string())
}

/// **A top-level table with nothing left in it goes, header and all**
/// (2026-11-08, reported: dropping the only key of a project file left
/// `[editor]` alone in it). The comment lines above its header are the
/// writer's and stay: they move to the table after it, or to the end of the
/// file when it was the last.
fn drop_if_empty(doc: &mut DocumentMut, name: &str) {
    let Some(Item::Table(t)) = doc.get(name) else { return };
    if !t.is_empty() {
        return;
    }
    let said = t.decor().prefix().and_then(|p| p.as_str()).unwrap_or("").to_string();
    let at = t.position();
    doc.remove(name);
    if said.trim().is_empty() {
        return;
    }
    let next = doc
        .iter()
        .filter_map(|(key, item)| item.as_table().and_then(|t| t.position()).map(|p| (p, key.to_string())))
        .filter(|(p, _)| at.is_none_or(|at| *p > at))
        .min()
        .map(|(_, key)| key);
    match next.and_then(|key| doc.get_mut(&key)).and_then(Item::as_table_mut) {
        Some(after) => {
            let had = after.decor().prefix().and_then(|p| p.as_str()).unwrap_or("").to_string();
            after.decor_mut().set_prefix(format!("{said}{had}"));
        }
        None => {
            let had = doc.trailing().as_str().unwrap_or("").to_string();
            doc.set_trailing(format!("{had}{said}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let after = rewrite(before, &[("editor.indent".into(), Change::Count(2))]).unwrap();
        for kept in [
            "# 這是我自己寫的，別動",
            "# 行號要絕對的",
            "line_numbers = \"absolute\"",
            "# 行尾這一句也是我寫的",
        ] {
            assert!(after.contains(kept), "掉了 {kept:?}：\n{after}");
        }
        assert!(after.contains("indent = 2"), "新的那一項寫進去了：\n{after}");
    }

    /// 改一個已經在的鍵，**行尾那句註釋要留着**。
    #[test]
    fn changing_a_value_keeps_the_note_beside_it() {
        let before = "[editor]\nindent = 2    # 中文慣例\n";
        let after = rewrite(before, &[("editor.indent".into(), Change::Count(4))]).unwrap();
        assert!(after.contains("indent = 4"), "{after}");
        assert!(after.contains("# 中文慣例"), "行尾那句没了：\n{after}");
    }

    /// `d` 撤掉本項目的覆蓋 —— 鍵没了，周圍的註釋還在。
    #[test]
    fn dropping_a_key_leaves_its_neighbours_alone() {
        let before = "[editor]\n# 上面這一句\nline_numbers = \"absolute\"\nsoft_wrap = true\n";
        let after = rewrite(before, &[("editor.soft_wrap".into(), Change::Drop)]).unwrap();
        assert!(!after.contains("soft_wrap"), "撤掉了：\n{after}");
        assert!(after.contains("# 上面這一句"), "{after}");
        assert!(after.contains("line_numbers"), "{after}");
    }

    /// **A key yumete does not know stays where it is, as it is**
    /// (2026-11-08, reported: changing `scrolloff` commented `typo_key` out
    /// and moved it). Changing a neighbour is a byte-for-byte edit of that
    /// one line; everything else, order and comments included, is the file's.
    #[test]
    fn a_key_the_panel_does_not_know_is_left_as_it_is() {
        let before = "\
# my config
[editor]
scrolloff = 8  # I like 8
typo_key = true

[keys.normal]
\"C-s\" = \":w\"

[editr]
indent = 4

[theme]
name = \"paper\"
";
        let after = rewrite(before, &[("editor.scrolloff".into(), Change::Count(9))]).unwrap();
        assert_eq!(after, before.replace("scrolloff = 8", "scrolloff = 9"), "only that value moved");
    }

    /// **The only key of a table dropped, the header goes too** (2026-11-08,
    /// reported: `[editor]` was left alone in `.yumete/config.toml`), and the
    /// comment above it stays, on the table after it or at the end.
    #[test]
    fn dropping_the_last_key_of_a_table_drops_its_header() {
        let after = rewrite("[editor]\nscrolloff = 8\n", &[("editor.scrolloff".into(), Change::Drop)]).unwrap();
        assert_eq!(after.trim(), "", "nothing left: {after:?}");

        let before = "# why\n[editor]\nscrolloff = 8\n\n[theme]\nname = \"bw\"\n";
        let after = rewrite(before, &[("editor.scrolloff".into(), Change::Drop)]).unwrap();
        assert!(!after.contains("[editor]"), "{after}");
        assert!(after.contains("# why") && after.contains("[theme]\nname = \"bw\""), "{after}");

        let before = "[theme]\nname = \"bw\"\n\n# why\n[editor]\nscrolloff = 8\n";
        let after = rewrite(before, &[("editor.scrolloff".into(), Change::Drop)]).unwrap();
        assert!(!after.contains("[editor]") && after.contains("# why"), "{after}");
        let _: DocumentMut = after.parse().expect("still TOML");

        // A table that still holds a table of its own stays.
        let before = "[keys]\nflavor = \"full\"\n\n[keys.normal]\n\"C-s\" = \":w\"\n";
        let after = rewrite(before, &[("keys.flavor".into(), Change::Drop)]).unwrap();
        assert!(after.contains("[keys.normal]"), "{after}");
    }

    /// 檔裏還没有這一張表，就補一張。
    #[test]
    fn a_table_that_is_not_there_yet_is_written() {
        let after = rewrite("", &[("editor.soft_wrap".into(), Change::Tick(true))]).unwrap();
        assert!(after.contains("[editor]"), "{after}");
        assert!(after.contains("soft_wrap = true"), "{after}");
    }

    /// 寫回去的東西要**讀得回來** —— 這一條是整條路的驗收。
    #[test]
    fn what_goes_out_parses_back_in() {
        let before = "# 頭上一句\n[editor]\nindent = 2\n";
        let after = rewrite(
            before,
            &[
                ("editor.indent".into(), Change::Count(4)),
                ("editor.soft_wrap".into(), Change::Tick(false)),
            ],
        )
        .unwrap();
        let back: toml::Value = toml::from_str(&after).expect("讀得回來");
        let editor = back.get("editor").and_then(|t| t.as_table()).expect("[editor]");
        assert_eq!(editor.get("indent").and_then(|v| v.as_integer()), Some(4));
        assert_eq!(editor.get("soft_wrap").and_then(|v| v.as_bool()), Some(false));
    }

    /// 壞掉的 toml 不許被「修」成別的東西——原樣報錯，一個字不寫。
    #[test]
    fn a_broken_file_is_refused_rather_than_rewritten() {
        assert!(rewrite("[editor\nindent = 2\n", &[]).is_err());
    }
}
