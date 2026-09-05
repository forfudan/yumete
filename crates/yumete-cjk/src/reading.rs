//! Where a reading comes from — Feature #234.
//!
//! `:ruby auto` writes 注音 into the text, and to do that it has to know how the
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

    /// Whether `ch` is outside 通用規範漢字表 — the question `:ruby auto rare`
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
