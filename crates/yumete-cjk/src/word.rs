//! Word segmentation for motions (`w` / `b` / `e`) — Feature #25.
//!
//! Two granularities, mirroring Helix and Vim:
//!
//! - [`word_ranges`] — "words": runs of alphanumerics (plus `_`), runs of
//!   punctuation, and **each CJK ideograph or kana as its own word** (so `w`
//!   steps through Chinese/Japanese one character at a time until a dictionary
//!   segmenter, Feature #24, upgrades it).
//! - [`word_ranges_big`] — "WORDS": runs of any non-whitespace characters.
//!
//! Both return character-index ranges `(start, end)` with whitespace skipped.

/// **What 分詞 is for** — the characters a dictionary is asked to cut, and the
/// only ones the segmentation overlay ever paints (#446).
///
/// 漢字文化圈的文字寫起來不帶空格，所以要有人告訴讀者詞在哪裏斷；拉丁文與標點
/// **自己就帶邊界**，用不着誰再說一遍。分詞只對下面這些字生效：
///
/// | 區段 | 碼位 | 例 |
/// | --- | --- | --- |
/// | CJK 基本區 | `4E00–9FFF` | 漢字 |
/// | 擴展 A | `3400–4DBF` | 㐅㑯 |
/// | 擴展 B–F、I | `20000–2A6DF`、`2A700–2EE5F` | 𠀀𪚥 |
/// | 兼容漢字補充 | `2F800–2FA1F` | 丽𠄢 |
/// | 擴展 G、H、J | `30000–3347F` | 𰀀𱍊 |
/// | 兼容漢字 | `F900–FAFF` | 﨏﨑 |
/// | 康熙部首 | `2F00–2FDF` | ⼀⽔ |
/// | 部首補充 | `2E80–2EFF` | ⺈⻌ |
/// | 〇（U+3007）、々（U+3005） | 兩個單點 | 二〇二五年、佗々 |
/// | 平假名 | `3040–309F` | ひらがな |
/// | 片假名 | `30A0–30FF`（不含 `・`） | カタカナ |
/// | 片假名音標擴展 | `31F0–31FF` | ㇰㇱ |
///
/// ⚠️ **三處有意留在外面：**
///
/// * **⿰⿱ 那一族**（`2FF0–2FFF`，表意文字描述符）不是字，是拆字用的運算符；
/// * **`・`（U+30FB）** 在日文裏本來就是詞與詞之間的那道界，算進去等於把界當字；
/// * **全角標點**（`3000–303F` 的其餘、`FF01–FF60`）從來就是看得見的邊界。
///
/// ⚠️ **輔助平面按區塊列，不按平面掃。** 這裏從前寫的是 `20000..=3FFFF`——兩個
/// 完整平面，順手把未分配的空洞（`2A6E0–2A6FF`、`2EE60–2F7FF`、`33480–3FFFF`）
/// 與兩個非字符（`2FFFE`、`2FFFF`）都算成了漢字。邊界抄自 [`crate::blocks`]，
/// 那張表是 Unicode 自己的 `Blocks.txt`，`every_han_block_is_segmentable`
/// 逐區塊比對兩邊，所以下次補一個擴展區時漏改這裏會紅。
///
/// 日文目前**沒有分詞數據**，所以假名進得來、卻只會被逐字切開——等有了詞表，
/// 這裏一個字都不用改。
pub fn is_segmentable(c: char) -> bool {
    matches!(
        c as u32,
        0x2E80..=0x2EFF      // CJK 部首補充
        | 0x2F00..=0x2FDF    // 康熙部首
        | 0x3005             // 々 疊字符
        | 0x3007             // 〇
        | 0x3040..=0x309F    // 平假名
        | 0x30A0..=0x30FA    // 片假名（・ 之前）
        | 0x30FC..=0x30FF    // 片假名（・ 之後：ー ヽ ヾ）
        | 0x31F0..=0x31FF    // 片假名音標擴展
        | 0x3400..=0x4DBF    // 擴展 A
        | 0x4E00..=0x9FFF    // 基本區
        | 0xF900..=0xFAFF    // 兼容漢字
        | 0x20000..=0x2A6DF  // 擴展 B
        | 0x2A700..=0x2EE5F  // 擴展 C D E F I（連號，中間沒有別的區塊）
        | 0x2F800..=0x2FA1F  // 兼容漢字補充
        | 0x30000..=0x3347F  // 擴展 G H J（同上）
    )
}

/// Whether `c` is a 漢字 — what a Chinese word count actually counts.
///
/// The unified blocks and their extensions, plus the compatibility ideographs
/// and the two ideographs that live outside them: 〇 (U+3007), which is how a
/// year is written — 二〇二五年 is five 字, not four — and 々 (U+3005), the
/// repetition mark, which stands for a 漢字 and is counted as one.
///
/// Kana and punctuation are deliberately out, which is what separates this from
/// [`is_segmentable`]: a 字數 is not a character count (`:count` reports both),
/// and a reading (#234) is something only a 漢字 has.
pub fn is_han(c: char) -> bool {
    matches!(c as u32,
        0x3005 | 0x3007 | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
        // 輔助平面上的那幾個區塊，與 [`is_segmentable`] 同一張表——漢字就是漢字，
        // 分詞收不收假名是另一件事。
        || (is_segmentable(c) && c as u32 >= 0x20000)
}

/// Coarse character category for grouping non-CJK runs.
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum Category {
    Word,
    Punctuation,
}

pub(crate) fn category(c: char) -> Category {
    if c.is_alphanumeric() || c == '_' {
        Category::Word
    } else {
        Category::Punctuation
    }
}

/// **Where a run of 漢字 begins and ends, and what becomes of the rest** — the
/// one implementation (#349).
///
/// Every segmenter in the project agrees about the easy half of the job and
/// differs only in the hard one: whitespace separates and is itself no word,
/// a run of CJK is handed to a dictionary, and anything else is a run of one
/// category — letters and digits together, 標點 together. Only `cut_run` is a
/// judgement about the language, and it is the only thing a segmenter here
/// supplies.
///
/// It was written out three times before this, and the copies had drifted:
/// [`YumeSegmenter`](../../yumete_ime/segment/struct.YumeSegmenter.html)
/// dropped every 標點 on the floor instead of giving it a range, so `w` and
/// the overlay behaved differently depending on which dictionary happened to
/// be loaded — which is the whole of #349 in one line.
///
/// `cut_run` is given the run's characters and answers in indices relative to
/// that run; the offsets are put back here.
pub fn ranges_around_cjk(
    s: &str,
    mut cut_run: impl FnMut(&[char]) -> Vec<(usize, usize)>,
) -> Vec<(usize, usize)> {
    let chars: Vec<char> = s.chars().collect();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if is_segmentable(c) {
            let start = i;
            while i < chars.len() && is_segmentable(chars[i]) {
                i += 1;
            }
            ranges.extend(
                cut_run(&chars[start..i])
                    .into_iter()
                    .map(|(a, b)| (start + a, start + b)),
            );
            continue;
        }
        let cat = category(c);
        let start = i;
        i += 1;
        while i < chars.len()
            && !chars[i].is_whitespace()
            && !is_segmentable(chars[i])
            && category(chars[i]) == cat
        {
            i += 1;
        }
        ranges.push((start, i));
    }
    ranges
}

/// Word ranges: alphanumeric runs, punctuation runs, and single CJK characters.
pub fn word_ranges(s: &str) -> Vec<(usize, usize)> {
    ranges_around_cjk(s, |run| (0..run.len()).map(|i| (i, i + 1)).collect())
}

/// **Coarse words: a 漢字 is a letter.**
///
/// Runs of one category — alphanumerics (plus `_`) or punctuation — with no
/// special case for CJK, so 「我們都是apple」 is one word and 「。，」 is
/// another. This is what helix does with Chinese, having no dictionary to do
/// anything else with it, and it is two things here:
///
/// * what `w` walks by when the dictionary is switched off
///   ([`WordLevel::Off`](crate::WordLevel::Off)), and
/// * what `e` **always** walks by — because Chinese has no spaces, so if both
///   keys respected the dictionary the two would do nearly the same thing.
///   Left coarse, `e` runs to the next punctuation instead: **`w` takes a word,
///   `e` takes a clause** (#304).
pub fn word_ranges_coarse(s: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = s.chars().collect();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let cat = category(chars[i]);
        let start = i;
        i += 1;
        while i < chars.len() && !chars[i].is_whitespace() && category(chars[i]) == cat {
            i += 1;
        }
        ranges.push((start, i));
    }
    ranges
}

/// WORD ranges: maximal runs of non-whitespace characters.
pub fn word_ranges_big(s: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = s.chars().collect();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        ranges.push((start, i));
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 分詞收的區段與 `Blocks.txt` 逐塊對得上（#446）。
    ///
    /// ⚠️ **這是為了「不按平面掃」而存在的。** `is_segmentable` 從前寫
    /// `20000..=3FFFF`，一句話蓋掉兩個完整平面，未分配的空洞與兩個非字符全算成
    /// 漢字。改成逐區塊之後，風險換了一種：下次 Unicode 補一個擴展區，
    /// [`crate::blocks`] 那張表更新了而這裏忘了跟。所以比對是**兩個方向**的
    /// ——該收的一個不少，不該收的一個不多。
    #[test]
    fn every_han_block_is_segmentable() {
        // 收進分詞的區塊，按 Unicode 自己的區塊名。
        const TAKEN: &[&str] = &[
            "CJK Radicals Supplement",
            "Kangxi Radicals",
            "Hiragana",
            "Katakana",
            "Katakana Phonetic Extensions",
            "CJK Unified Ideographs Extension A",
            "CJK Unified Ideographs",
            "CJK Compatibility Ideographs",
            "CJK Unified Ideographs Extension B",
            "CJK Unified Ideographs Extension C",
            "CJK Unified Ideographs Extension D",
            "CJK Unified Ideographs Extension E",
            "CJK Unified Ideographs Extension F",
            "CJK Unified Ideographs Extension I",
            "CJK Compatibility Ideographs Supplement",
            "CJK Unified Ideographs Extension G",
            "CJK Unified Ideographs Extension H",
            "CJK Unified Ideographs Extension J",
        ];
        for &(first, last, name) in crate::blocks::BLOCKS {
            let want = TAKEN.contains(&name);
            for cp in first..=last {
                let Some(c) = char::from_u32(cp) else { continue };
                // 〇 與 々 住在標點區塊裏，是兩個單點例外；・ 是片假名區塊裏
                // 唯一一個不收的。
                if matches!(cp, 0x3005 | 0x3007 | 0x30FB) {
                    continue;
                }
                assert_eq!(
                    is_segmentable(c),
                    want,
                    "U+{cp:04X} 在「{name}」裏，分詞{}收它",
                    if want { "該" } else { "不該" }
                );
            }
        }
        assert!(is_segmentable('〇') && is_segmentable('々'));
        assert!(!is_segmentable('・'));
        // 未分配的空洞不在任何區塊裏，所以上面掃不到——單點釘住。
        for cp in [0x2A6E0u32, 0x2EE60, 0x2FFFE, 0x33480, 0x3FFFF] {
            let c = char::from_u32(cp).unwrap();
            assert!(!is_segmentable(c), "U+{cp:04X} 不是任何區塊裏的字");
            assert!(!is_han(c));
        }
    }

    #[test]
    fn words_split_latin_runs_and_cjk_singles() {
        // "hello, 世界 rust" → "hello", ",", 世, 界, "rust"
        let r = word_ranges("hello, 世界 rust");
        assert_eq!(r, vec![(0, 5), (5, 6), (7, 8), (8, 9), (10, 14)]);
    }

    #[test]
    fn big_words_split_only_on_whitespace() {
        // "hello, 世界 rust" → "hello,", "世界", "rust"
        let r = word_ranges_big("hello, 世界 rust");
        assert_eq!(r, vec![(0, 6), (7, 9), (10, 14)]);
    }

    #[test]
    fn leading_and_trailing_whitespace_is_skipped() {
        assert_eq!(word_ranges("  ab  "), vec![(2, 4)]);
        assert_eq!(word_ranges_big("\tab\n"), vec![(1, 3)]);
    }
}
