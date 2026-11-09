//! **把一項設定寫回 toml，而不動旁邊的任何一個字**。
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
//! **原樣留着。** 不把面板不認得的鍵註釋掉、挪到表頭底下：`deny_unknown_fields`
//! 固然讓一個打錯的鍵名作廢整份配置，可那樣是改了使用者没叫改的行——他只改了
//! `scrolloff`，存完 `typo_key` 被註釋掉、次序也變了。一份讀不進去的檔，面板頂上
//! 就說出來，`:w` 不寫它，要改請 `e` 打開那個檔
//! （[`crate::panel::Panel::save`]）。

use toml_edit::{DocumentMut, InlineTable, Item, Table, TableLike, Value};

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
    /// `8` or `'Songti SC'`: what `u` puts back, in the very
    /// spelling the file had.
    Written(String),
}

/// The table `tables` names, one inside the next, if the file has it —
/// written as a `[表]` of its own lines or inline as `表 = { … }`.
///
/// Warning: **An inline table is written into where it stands.** Turned
/// into a table of its own lines it would move to the end of the file and
/// lose the comment after it.
fn table_at<'a>(doc: &'a mut DocumentMut, tables: &[&str]) -> Option<&'a mut dyn TableLike> {
    let mut t = doc.get_mut(tables.first()?)?.as_table_like_mut()?;
    for name in &tables[1..] {
        t = t.get_mut(name)?.as_table_like_mut()?;
    }
    Some(t)
}

/// **How the file ends its lines and whether it opens with a BOM**:
/// `toml_edit` reads both and writes neither, and a one-value change from
/// the panel must not be a whole-file diff of a Windows config.
struct Shape {
    bom: bool,
    crlf: bool,
}

impl Shape {
    /// The shape of `text`, and the text to hand the parser: no BOM, and
    /// `\n` for `\r\n` when the file ends its lines that way — judged by
    /// its first line, the way an editor does.
    fn of(text: &str) -> (Shape, String) {
        let bom = text.starts_with('\u{feff}');
        let text = text.trim_start_matches('\u{feff}');
        let crlf = text.find('\n').is_some_and(|at| text[..at].ends_with('\r'));
        let plain = match crlf {
            true => text.replace("\r\n", "\n"),
            false => text.to_string(),
        };
        (Shape { bom, crlf }, plain)
    }

    /// `text`, written back in this shape.
    fn put(&self, text: String) -> String {
        let text = match self.crlf {
            true => text.replace("\r\n", "\n").replace('\n', "\r\n"),
            false => text,
        };
        match self.bom {
            true => format!("\u{feff}{text}"),
            false => text,
        }
    }
}

/// **一份 toml，改幾處，字節級別地不動別的。**
///
/// `text` 是檔裏原樣的那一份（空字串 ＝ 還没有這個檔）。`changes` 是
/// `("表.鍵", 怎麽改)`。回來的是新的正文；檔讀不通就回解析器的話，一個字不寫。
pub fn rewrite(text: &str, changes: &[(String, Change)]) -> Result<String, String> {
    let (shape, text) = Shape::of(text);
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;

    for (path, change) in changes {
        // `表.鍵`, or `表.子表.鍵` for a table inside a table
        // (`keys.flavor_items.esc_collapses`).
        let Some((tables, key)) = path.rsplit_once('.') else {
            return Err(format!("{path} 不是「表.鍵」"));
        };
        let tables: Vec<&str> = tables.split('.').collect();
        match change {
            Change::Drop => {
                let mut said = String::new();
                if let Some(t) = table_at(&mut doc, &tables) {
                    said = remove_keeping_comments(t, key);
                }
                if !said.is_empty() {
                    put_after_table(&mut doc, &tables, &said);
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
                    let empty = outer
                        .get(tables[depth])
                        .and_then(Item::as_table_like)
                        .is_some_and(|inner| inner.is_empty());
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
                let entry = doc.entry(tables[0]).or_insert_with(|| Item::Table(Table::new()));
                if let (true, true, Some(t)) = (fresh_outer, tables.len() > 1, entry.as_table_mut()) {
                    t.set_implicit(true);
                }
                let mut inline = entry.is_inline_table();
                let Some(mut t) = entry.as_table_like_mut() else {
                    return Err(format!("{} 不是一張表", tables[0]));
                };
                for name in &tables[1..] {
                    // A table that is not there yet is written the way the
                    // table holding it is: inside an inline one, inline too.
                    if !t.contains_key(name) {
                        t.insert(
                            name,
                            match inline {
                                true => Item::Value(Value::InlineTable(InlineTable::new())),
                                false => Item::Table(Table::new()),
                            },
                        );
                    }
                    let Some(item) = t.get_mut(name) else {
                        return Err(format!("{name} 不是一張表"));
                    };
                    inline |= item.is_inline_table();
                    let Some(inner) = item.as_table_like_mut() else {
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
                        let mut value = value;
                        // The space before an inline table's `}` belongs to its
                        // last value; it moves to the one added after it.
                        let last = t.iter().last().map(|(k, _)| k.to_string());
                        if let (true, Some(Item::Value(last))) = (inline, last.and_then(|k| t.get_mut(&k))) {
                            let space = last.decor().suffix().and_then(|s| s.as_str()).unwrap_or("").to_string();
                            if space.trim().is_empty() {
                                last.decor_mut().set_suffix("");
                                value.decor_mut().set_prefix(" ");
                                value.decor_mut().set_suffix(space);
                            }
                        }
                        t.insert(key, Item::Value(value));
                    }
                }
            }
        }
    }
    Ok(shape.put(doc.to_string()))
}

/// **Take `key` out of `t`, and keep the comment lines above it.** They
/// are the writer's and say something about the table, not only about the
/// value: they go onto the key after it. Returned when no key follows, for
/// [`put_after_table`] to place.
fn remove_keeping_comments(t: &mut dyn TableLike, key: &str) -> String {
    let said = t
        .key(key)
        .and_then(|k| k.leaf_decor().prefix())
        .and_then(|p| p.as_str())
        .map(comment_lines)
        .unwrap_or_default();
    let next = {
        let mut keys = t.iter().filter(|(_, item)| item.is_value()).map(|(k, _)| k.to_string());
        keys.by_ref().find(|k| k == key);
        keys.next()
    };
    t.remove(key);
    if said.is_empty() {
        return said;
    }
    match next.as_deref().and_then(|next| t.key_mut(next)) {
        Some(mut next) => {
            let decor = next.leaf_decor_mut();
            let had = decor.prefix().and_then(|p| p.as_str()).unwrap_or("").to_string();
            decor.set_prefix(format!("{said}{had}"));
            String::new()
        }
        None => said,
    }
}

/// The whole lines of a key's prefix, if any of them is a comment: what is
/// above the key, without the indent of the key's own line.
fn comment_lines(prefix: &str) -> String {
    match (prefix.contains('#'), prefix.rfind('\n')) {
        (true, Some(end)) => prefix[..=end].to_string(),
        _ => String::new(),
    }
}

/// Comment lines from the end of the table `tables` names, put before the
/// table that comes after it in the file, or at the end of the file.
fn put_after_table(doc: &mut DocumentMut, tables: &[&str], said: &str) {
    let at = tables
        .iter()
        .try_fold(doc.as_table(), |t, name| t.get(name)?.as_table())
        .and_then(Table::position);
    let mut all = Vec::new();
    positions(doc.as_table(), &mut Vec::new(), &mut all);
    let next = all.into_iter().filter(|(p, _)| at.is_some_and(|at| *p > at)).min().map(|(_, path)| path);
    let target = next.and_then(|path| {
        let mut t = doc.as_table_mut();
        for name in &path {
            t = t.get_mut(name)?.as_table_mut()?;
        }
        Some(t)
    });
    match target {
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

/// Every table written with a header, by where it stands in the file.
fn positions(t: &Table, at: &mut Vec<String>, out: &mut Vec<(usize, Vec<String>)>) {
    for (key, item) in t.iter() {
        if let Some(inner) = item.as_table() {
            at.push(key.to_string());
            if let (Some(p), false) = (inner.position(), inner.is_implicit()) {
                out.push((p, at.clone()));
            }
            positions(inner, at, out);
            at.pop();
        }
    }
}

/// **A top-level table with nothing left in it goes, header and all**:
/// dropping the only key of a project file does not leave `[editor]` alone
/// in it. The comment lines above its header are the
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

    /// **A key yumete does not know stays where it is, as it is**: changing
    /// `scrolloff` does not comment `typo_key` out or move it. Changing a neighbour is a byte-for-byte edit of that
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

    /// **The only key of a table dropped, the header goes too** (`[editor]` is
    /// not left alone in `.yumete/config.toml`), and the
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

    /// **A CRLF file stays CRLF, and its BOM stays**: one value changed from
    /// the panel is one line changed.
    #[test]
    fn the_line_endings_and_the_bom_are_the_files() {
        let before = "\u{feff}# mine\r\n[editor]\r\nscrolloff = 8\r\nsoft_wrap = true # yes\r\n";
        let after = rewrite(before, &[("editor.scrolloff".into(), Change::Count(9))]).unwrap();
        assert_eq!(after, before.replace("scrolloff = 8", "scrolloff = 9"));
        let after = rewrite(before, &[("editor.indent".into(), Change::Count(2))]).unwrap();
        assert!(after.starts_with('\u{feff}'), "{after:?}");
        assert!(!after.replace("\r\n", "").contains('\n'), "a new line is CRLF too: {after:?}");
        let after = rewrite("[editor]\nscrolloff = 8\n", &[("editor.scrolloff".into(), Change::Count(9))]).unwrap();
        assert_eq!(after, "[editor]\nscrolloff = 9\n", "LF stays LF, no BOM is added");
    }

    /// **`r` takes the key, not the comment over it**: the comment goes onto
    /// what follows — the next key, the next table, or the end of the file.
    #[test]
    fn dropping_a_key_keeps_the_comment_above_it() {
        let drop = |before: &str| rewrite(before, &[("editor.soft_wrap".into(), Change::Drop)]).unwrap();
        assert_eq!(
            drop("[editor]\n# about wrapping\nsoft_wrap = true\nindent = 2\n"),
            "[editor]\n# about wrapping\nindent = 2\n"
        );
        assert_eq!(
            drop("[editor]\nindent = 2\n# about wrapping\nsoft_wrap = true\n\n[theme]\nname = \"bw\"\n"),
            "[editor]\nindent = 2\n# about wrapping\n\n[theme]\nname = \"bw\"\n"
        );
        assert_eq!(
            drop("[editor]\nindent = 2\n# about wrapping\nsoft_wrap = true\n"),
            "[editor]\nindent = 2\n# about wrapping\n"
        );
        let after = drop("[editor]\n# about wrapping\nsoft_wrap = true\n");
        assert!(after.contains("# about wrapping") && !after.contains("soft_wrap"), "{after:?}");
    }

    /// **An inline table is written where it stands**, its comment kept, and
    /// a top-level `editor = { … }` can be saved into.
    #[test]
    fn an_inline_table_is_written_where_it_stands() {
        let before = "[keys]\nflavor_items = { tables = false } # trying this\npreset = \"vim\"\n\n[theme]\nname = \"bw\"\n";
        let after = rewrite(before, &[("keys.flavor_items.tables".into(), Change::Tick(true))]).unwrap();
        assert_eq!(after, before.replace("tables = false", "tables = true"));
        let after = rewrite(before, &[("keys.flavor_items.lists".into(), Change::Tick(false))]).unwrap();
        assert_eq!(after, before.replace("tables = false", "tables = false, lists = false"));
        let after = rewrite(before, &[("keys.flavor_items.tables".into(), Change::Drop)]).unwrap();
        assert_eq!(after, "[keys]\npreset = \"vim\"\n\n[theme]\nname = \"bw\"\n", "emptied, it goes");

        let before = "editor = { scrolloff = 3 }\n";
        let after = rewrite(before, &[("editor.scrolloff".into(), Change::Count(4))]).unwrap();
        assert_eq!(after, "editor = { scrolloff = 4 }\n");
        let after = rewrite(before, &[("editor.indent".into(), Change::Count(2))]).unwrap();
        let back: toml::Value = toml::from_str(&after).expect("TOML");
        assert_eq!(back["editor"]["indent"].as_integer(), Some(2), "{after}");
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
