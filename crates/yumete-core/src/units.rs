//! **What one counts as** (2026-10-29): Word's 字數. Each 漢字, kana or
//! Hangul syllable is one, and each run of letters or digits in any other
//! script is one word — so a Chinese chapter counts as it always did, an
//! English one stops reading 0, and `我用 Rust 寫` is four. The live count on
//! the status line, `:count`, and the daily target all use it.
//!
//! An apostrophe or a hyphen between two letters keeps the word going
//! (`don't`, `well-known`), as Word counts them.

use yumete_cjk::is_han;

/// A count that can be fed a piece at a time: a word that runs across two
/// pieces (two chunks of a rope, a ruby group and its neighbour) is still one.
#[derive(Default, Clone, Copy)]
pub struct Units {
    total: usize,
    in_word: bool,
    /// A joiner was the last thing seen inside a word: one more letter keeps
    /// the word going, anything else ends it.
    joining: bool,
}

impl Units {
    pub fn feed(&mut self, text: &str) {
        for c in text.chars() {
            self.feed_char(c);
        }
    }

    pub fn feed_char(&mut self, c: char) {
        if east_asian(c) {
            self.total += 1;
            self.in_word = false;
            self.joining = false;
        } else if c.is_alphanumeric() {
            if !self.in_word {
                self.total += 1;
                self.in_word = true;
            }
            self.joining = false;
        } else if self.in_word && !self.joining && matches!(c, '\'' | '\u{2019}' | '-') {
            self.joining = true;
        } else {
            self.in_word = false;
            self.joining = false;
        }
    }

    pub fn total(&self) -> usize {
        self.total
    }
}

/// How many `text` counts as.
pub fn units(text: &str) -> usize {
    let mut n = Units::default();
    n.feed(text);
    n.total()
}

/// A character that counts on its own, the way Word counts 「中文字符和朝鮮語
/// 單詞」: 漢字, kana, and Hangul.
fn east_asian(c: char) -> bool {
    is_han(c)
        || matches!(c as u32,
            0x3041..=0x309F | 0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9D
            | 0xAC00..=0xD7A3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_han_and_each_latin_word_is_one() {
        assert_eq!(units("那年冬天"), 4);
        assert_eq!(units("Hello world."), 2);
        assert_eq!(units("他說 OK 吧。"), 4);
        assert_eq!(units("我用 Rust 寫"), 4);
        assert_eq!(units("don't stop, well-known 3.14"), 5, "joiners keep a word; the dot does not");
        assert_eq!(units("かんじ カタカナ 한국"), 9);
        assert_eq!(units("  — , 。"), 0);
    }

    #[test]
    fn a_word_split_across_pieces_is_still_one() {
        let mut n = Units::default();
        n.feed("hel");
        n.feed("lo wor");
        n.feed("ld");
        assert_eq!(n.total(), 2);
    }
}
