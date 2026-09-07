//! 自定義方案 are found where yume put them, not where the manifest looks
//! (author, 2026-09-08: 「yume 現在重構之後，分爲內置方案和自定義方案兩個文件
//! 夾……只要掃到這兩個文件夾就能得到這些方案的碼表和設置信息了」).
//!
//! The built-in half of that is `found_schemes.rs`: a `schemes/<tag>.toml` any
//! data directory holds. This is the other half, and it is found a different
//! way on purpose — a slot has no scheme file, its parameters are in a
//! `custom.yscm` beside its tables, and `yume_core::data_manifest` answers
//! nothing about it because the manifest is the *factory* data set.
//!
//! Its own test binary, and it must stay that way, for the reason
//! `found_schemes.rs` gives: `discover` writes `OnceLock`s that no later call
//! can take back.

use std::path::{Path, PathBuf};

use yume_core::CodeTable;
use yumete_ime::{ImeSession, Scheme};

/// Lay out one slot the way yume's own 方案管理 lays it out: a directory named
/// by eight hex digits, holding the compiled 碼表 and the manifest.
///
/// Two files, because two files are what `scheme_slots::list` tests for — the
/// `.yzg`, the derived `.ycdv` and the writer's `source.txt` are all optional,
/// and a slot without them is a scheme that types with no 拆分 to show.
fn slot(root: &Path, id: &str, name: &str, codes: &str) {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).expect("slot dir");
    let mut table = CodeTable::new();
    table.load_text(codes);
    table
        .write_binary(&dir.join("custom.ytab").to_string_lossy())
        .expect("碼表");
    // The manifest's own `key = value` shape. Only these two are required; the
    // rest of what yume writes reads as 「nothing hidden, nothing recorded」.
    std::fs::write(
        dir.join("custom.yscm"),
        format!("# 宇夢自定義方案\nname = {name}\nmax_code_length = 4\n"),
    )
    .expect("manifest");
}

#[test]
fn a_slot_yume_compiled_is_a_scheme_yumete_can_type() {
    let dir = std::env::temp_dir().join(format!("yumete-slots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // Where macOS keeps them today. Windows still writes `data/custom`, and
    // both are scanned; one of them is enough to prove the scan.
    slot(&dir.join("installed"), "a1b2c3d4", "試驗方案", "a 啊\nb 吧 八\n");
    // Not a slot: a directory whose name is not eight hex digits, and a slot
    // with no 碼表 in it. Neither may reach the menu — the second is what a
    // half-finished import looks like, and it would show as a nameless row.
    std::fs::create_dir_all(dir.join("installed").join("notes")).unwrap();
    std::fs::create_dir_all(dir.join("installed").join("deadbeef")).unwrap();

    assert_eq!(yumete_ime::discover(&[dir.clone()]), 1, "one slot, one scheme");

    let tags: Vec<&str> = Scheme::all().iter().map(|s| s.tag()).collect();
    assert_eq!(
        tags,
        [
            "lingming",
            "xingchen",
            "qingyun",
            "riyue",
            "pinyin",
            "custom.a1b2c3d4",
        ],
        "the writer's own scheme comes last, and takes nothing off the list",
    );

    let scheme = Scheme::from_tag("custom.a1b2c3d4").expect("the tag names it");
    assert_eq!(
        scheme.found_name(),
        "試驗方案",
        "a menu row cannot say `custom.a1b2c3d4`",
    );
    assert!(!scheme.is_phonetic(), "a 自定義方案 is compiled from a 碼表");

    let mut ime = ImeSession::new(scheme, vec![dir.clone()]);
    assert!(ime.available(), "the slot's 碼表 is what makes it typable");
    // The manifest reached the engine, not just the menu: the 方案名 the panel
    // shows comes off the `Schema` the manifest built.
    assert_eq!(ime.scheme_name(), "試驗方案");
    ime.input('b');
    assert_eq!(ime.inline_candidate(), "吧");

    // A second scan is ignored, the same way the factory list is.
    assert_eq!(yumete_ime::discover(&[PathBuf::from("/no/such/dir")]), 1);

    let _ = std::fs::remove_dir_all(&dir);
}
