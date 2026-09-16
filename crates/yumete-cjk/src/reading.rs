//! Where a reading comes from — Feature #234.
//!
//! `:ruby-auto` writes 注音 into the text, and to do that it has to know how the
//! text is *read*. That knowledge is not in this crate and never will be: it is
//! a 拆分表 and a 讀音表, tens of megabytes of data a writer installs, and it
//! reaches the editor the same way word segmentation does — as a trait the
//! editor holds and `yumete-ime` implements ([`crate::Segmenter`] is the older
//! sibling of this one).
//!
//! ## Why a *word* is the unit
//!
//! 了 is `le` in 為了 and `liǎo` in 了解, and a per-character table cannot tell
//! them apart — which is exactly why Word's 拼音指南 gets it wrong. So the
//! question this trait asks is 「how is **this word** read」, and the
//! implementation is free to answer it from a 詞-level table. The answer comes
//! back one syllable per character, because that is how CJK ruby is set: each
//! 漢字 carries its own reading above it.

/// Something that knows how 漢字 are read, and which of them are rare.
///
/// The blanket answer is 「I do not know」 — [`None`] everywhere — so an editor
/// with no language data installed is a reader that says so, rather than one
/// that quietly invents readings.
pub trait Reader {
    /// How `word` is read: one syllable per character, or `None` when this
    /// reader has nothing to say about it.
    ///
    /// The returned vector is always as long as the word is in `char`s, or the
    /// answer is `None` — a reading that covers half a word is not a reading.
    fn read(&self, word: &str) -> Option<Vec<String>>;

    /// Whether `ch` is outside 通用規範漢字表 — the question `:ruby-auto rare`
    /// asks of every character.
    ///
    /// `None` means the 字集 data is not loaded, which is a different finding
    /// from 「this character is common」 and has to stay distinguishable: the
    /// first is worth a status line, the second is worth nothing.
    fn is_rare(&self, _ch: char) -> Option<bool> {
        None
    }

    /// Which 字集 carry `ch`, exactly as the 拆分表 spells it — `簡古臺-CJK`:
    /// the standards first, then the Unicode block, separated by a `-`.
    ///
    /// The field is handed over unsplit on purpose. The two halves answer two
    /// different questions — *is this character standard* and *is this
    /// character in a block the typesetter's font has* — and a check that asks
    /// the second one (#240) needs the block name to say anything useful.
    ///
    /// `None` is 「the 字集 data is not loaded」, which is not the same finding
    /// as an empty field: an empty field means the data was consulted and this
    /// character is in nothing.
    fn charset(&self, _ch: char) -> Option<String> {
        None
    }

    /// Whether this reader can answer anything at all, for the status line.
    fn available(&self) -> bool {
        false
    }
}

/// The reader an editor has before anyone installs one: it knows nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoReader;

impl Reader for NoReader {
    fn read(&self, _word: &str) -> Option<Vec<String>> {
        None
    }
}

/// The five 字集 marks a 拆分表 row can carry, in the order yume writes them:
/// 通用規範, 通規繁體, 古籍通規, 國字常用（臺）, 常用字字形表（港）.
pub const CHARSET_TAGS: [char; 5] = ['簡', '繁', '古', '臺', '港'];

/// The 字集 field split into the two questions it answers — `簡古臺-CJK-A`
/// becomes `("簡古臺", "CJK-A")`.
///
/// ⚠️ **Not `split('-')`.** A block name has hyphens of its own — `CJK-A`,
/// `CJK-B`, `假名擴展-A` — so the first `-` is the separator only when
/// something precedes it, and a character in *no* 字集 has a field that is
/// nothing but a block name. Splitting on the first `-` read `CJK-A` as the
/// tags `CJK`, which is not empty, so `:check-charset` answered 「每個字都在
/// 字集裏」 for a page of 擴展A — the one answer it exists to disprove.
///
/// So the tags are the leading run of [`CHARSET_TAGS`] and nothing else. No
/// block name begins with one of those five characters, which is what makes
/// the prefix unambiguous.
pub fn split_charset(field: &str) -> (&str, &str) {
    let end = field
        .char_indices()
        .find(|(_, c)| !CHARSET_TAGS.contains(c))
        .map_or(field.len(), |(i, _)| i);
    let (tags, rest) = field.split_at(end);
    (tags, rest.strip_prefix('-').unwrap_or(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_keeps_its_own_hyphens() {
        assert_eq!(split_charset("簡古臺-CJK-A"), ("簡古臺", "CJK-A"));
        assert_eq!(split_charset("簡-CJK"), ("簡", "CJK"));
        // No tags at all: the whole field is the block, hyphen and all.
        assert_eq!(split_charset("CJK-A"), ("", "CJK-A"));
        assert_eq!(split_charset("假名擴展-A"), ("", "假名擴展-A"));
        assert_eq!(split_charset("康熙部首"), ("", "康熙部首"));
        // Tags with no block, and a row the 拆分表 has nothing for.
        assert_eq!(split_charset("簡繁古臺港"), ("簡繁古臺港", ""));
        assert_eq!(split_charset(""), ("", ""));
    }
}
