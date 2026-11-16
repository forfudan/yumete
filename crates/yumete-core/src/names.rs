//! **專名寫走了樣** — a name from the book's own list, written another way.
//!
//! [`crate::usage`] answers 「which spelling did this manuscript settle on」 and
//! it works because both spellings are in a list somebody wrote down. A proper
//! name is not in anybody's list: 醉翁亭 is this book's, and the one place it
//! came out 醉翁停 is a typo no dictionary and no 異體字表 can find. The book
//! does know its names, though — its 百科 and its `words.txt` are where they
//! are written down — so the question can be asked of them.
//!
//! ## Four reasons, and what is left out
//!
//! A window of the text the same length as a name is listed when every
//! character either is the name's or stands to it in one of these:
//!
//! | reason | test | how many |
//! | --- | --- | --- |
//! | 同音 | same first reading | **one** character |
//! | 繁简 | one is the other's 簡體 (opencc `TSCharacters`) | any |
//! | 异体 | same 字形組 in [`crate::glyphs`], not 繁简 | any |
//! | 兼容字 | equal after NFKC (compatibility ideographs, 康熙部首) | any |
//!
//! 瀋鶴年 is 沈鹤年 in every character and is still the same name; 醉翁停 is
//! one character off. **The cap on 同音 is what keeps this a proofing tool**:
//! 「one character the same sound」 is a slip, 「two characters the same sound」
//! is most of the vocabulary.
//!
//! Warning: **形近 is not a reason.** 己/已/巳 look alike and are three
//! characters; a look-alike table would flag every 未/末 in a book, and the
//! IME slips this exists for are slips of sound, not of shape.
//!
//! ## Why a shared reading is required
//!
//! 「one character different」 is not a finding in Chinese: 醉翁亭 is one
//! character from 醉翁路, and a novel is full of those. What makes a slip a
//! slip is that it is the *same sound* — an IME took the wrong candidate, or a
//! hand wrote the homophone. 亭 and 停 are both `ting`. So this asks the
//! reading table, and where there is no reading table the caller reports
//! nothing at all rather than a page of noise.
//!
//! ## How the scan finds them without reading the book N times
//!
//! Two indexes, by **first** character and by **last**, each with the name's
//! length, both keyed on the character's 字形組 ([`crate::glyphs::fold`] after
//! NFKC). At most one character is off in sound, so the first or the last is
//! always in the name's own group — every candidate is under one key or the
//! other, and the lists under a key are a handful of names.

use std::collections::{BTreeSet, HashMap};
use unicode_normalization::UnicodeNormalization;

/// Why a written character may stand for the name's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// 同音: the same first reading.
    Sound,
    /// 繁简: one is the other's 簡體.
    Script,
    /// 异体: one 字形組, not 繁简.
    Variant,
    /// 兼容字: the same character after NFKC.
    Compat,
}

/// One place the manuscript wrote a name another way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slip {
    /// Which line, counting from zero.
    pub line: usize,
    /// Which character of it, counting from zero.
    pub column: usize,
    /// What is written here.
    pub written: String,
    /// The name it stands for.
    pub name: String,
    /// Each character that differs: the one written, and the name's.
    pub pairs: Vec<(char, char)>,
    /// Every reason it took, in [`Reason`]'s order, each once.
    pub reasons: Vec<Reason>,
}

/// The single character NFKC folds `ch` into, or `ch` itself.
///
/// Only one-to-one folds count: ㈱ comes apart into three characters, and a
/// name is compared a character at a time.
fn plain(ch: char) -> char {
    if ch.is_ascii() {
        return ch;
    }
    let mut out = std::iter::once(ch).nfkc();
    match (out.next(), out.next()) {
        (Some(one), None) => one,
        _ => ch,
    }
}

/// The index key of a character: its 字形組 after NFKC.
fn key(ch: char) -> char {
    crate::glyphs::fold(plain(ch))
}

/// The names, indexed once — a whole book is checked against one of these.
pub struct Names {
    names: Vec<Vec<char>>,
    lengths: BTreeSet<usize>,
    by_head: HashMap<(char, usize), Vec<usize>>,
    by_tail: HashMap<(char, usize), Vec<usize>>,
}

impl Names {
    /// Index these names. One-character names and repeats are dropped: a lone
    /// character is in every sentence.
    pub fn new<S: AsRef<str>>(names: &[S]) -> Names {
        let mut list: Vec<Vec<char>> = names
            .iter()
            .map(|n| n.as_ref().trim().chars().collect::<Vec<char>>())
            .filter(|n| n.len() >= 2)
            .collect();
        list.sort();
        list.dedup();
        let mut lengths = BTreeSet::new();
        let mut by_head: HashMap<(char, usize), Vec<usize>> = HashMap::new();
        let mut by_tail: HashMap<(char, usize), Vec<usize>> = HashMap::new();
        for (i, name) in list.iter().enumerate() {
            let n = name.len();
            lengths.insert(n);
            by_head.entry((key(name[0]), n)).or_default().push(i);
            by_tail.entry((key(name[n - 1]), n)).or_default().push(i);
        }
        Names { names: list, lengths, by_head, by_tail }
    }

    /// No name of two characters or more.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Every place a name is written another way, in reading order.
    ///
    /// `alike` says whether two characters are read the same — the caller's,
    /// so that this module holds no opinion about where a reading comes from
    /// and a test can hand it a table of two.
    ///
    /// Warning: **A name written as listed is never a slip**, including a name
    /// that is itself close to another name (brothers 李明 and 李朋, or a
    /// writer who lists both 沈鹤年 and 瀋鶴年 because the book uses both). An
    /// exact match wins the window before any near one is asked about — which
    /// is also how a deliberate variant is let through: list it.
    pub fn check(&self, text: &str, alike: impl Fn(char, char) -> bool) -> Vec<Slip> {
        let mut out = Vec::new();
        if self.names.is_empty() {
            return out;
        }
        for (line, body) in text.lines().enumerate() {
            let chars: Vec<char> = body.chars().collect();
            let mut at = 0usize;
            while at < chars.len() {
                match self.at(&chars, at, &alike) {
                    // Past the slip, so one wrong character in a sentence is
                    // one finding however many names it is nearly.
                    Found::Slip(n, pairs, reasons, name) => {
                        out.push(Slip {
                            line,
                            column: at,
                            written: chars[at..at + n].iter().collect(),
                            name: self.names[name].iter().collect(),
                            pairs,
                            reasons,
                        });
                        at += n;
                    }
                    // Past the name as well: its inside is not a fresh place to
                    // start a near match.
                    Found::Exact(n) => at += n,
                    Found::Nothing => at += 1,
                }
            }
        }
        out
    }

    /// What starts at `at`: a name as listed, a name written another way, or
    /// neither. Longest first: a slip in a four-character name is that name's,
    /// not two overlapping findings about the three-character one inside it.
    fn at(&self, chars: &[char], at: usize, alike: &impl Fn(char, char) -> bool) -> Found {
        let head = key(chars[at]);
        for &n in self.lengths.iter().rev() {
            if at + n > chars.len() {
                continue;
            }
            let here = &chars[at..at + n];
            let tail = key(here[n - 1]);
            let mut near: Option<Found> = None;
            let heads = self.by_head.get(&(head, n)).into_iter().flatten();
            let tails = self.by_tail.get(&(tail, n)).into_iter().flatten();
            for &i in heads.chain(tails) {
                let name = &self.names[i];
                if here == name.as_slice() {
                    return Found::Exact(n);
                }
                if near.is_none() {
                    if let Some((pairs, reasons)) = compare(here, name, alike) {
                        near = Some(Found::Slip(n, pairs, reasons, i));
                    }
                }
            }
            if let Some(found) = near {
                return found;
            }
        }
        Found::Nothing
    }
}

enum Found {
    Nothing,
    /// A name as listed, this long.
    Exact(usize),
    /// A name written another way: its length, the pairs, the reasons, which.
    Slip(usize, Vec<(char, char)>, Vec<Reason>, usize),
}

/// How `written` stands to `name`, character by character — `None` when some
/// character is simply another character, or more than one is off in sound.
type Compared = (Vec<(char, char)>, Vec<Reason>);

fn compare(written: &[char], name: &[char], alike: &impl Fn(char, char) -> bool) -> Option<Compared> {
    let mut pairs = Vec::new();
    let mut reasons: BTreeSet<Reason> = BTreeSet::new();
    let mut sounds = 0usize;
    for (&w, &n) in written.iter().zip(name.iter()) {
        if w == n {
            continue;
        }
        let (pw, pn) = (plain(w), plain(n));
        if pw != w || pn != n {
            reasons.insert(Reason::Compat);
        }
        if pw == pn {
            pairs.push((w, n));
            continue;
        }
        // The glyph group before the sound: 瀋 and 沈 are both `shen`, and
        // they are the same character, which says more than a shared reading
        // and is not held to the one-character cap.
        if crate::glyphs::related(pw, pn) {
            reasons.insert(match crate::glyphs::simplified_pair(pw, pn) {
                true => Reason::Script,
                false => Reason::Variant,
            });
        } else if alike(pw, pn) {
            sounds += 1;
            if sounds > 1 {
                return None;
            }
            reasons.insert(Reason::Sound);
        } else {
            return None;
        }
        pairs.push((w, n));
    }
    match pairs.is_empty() {
        true => None,
        false => Some((pairs, reasons.into_iter().collect())),
    }
}

/// [`Names::check`] for a list used once.
pub fn check(text: &str, names: &[String], alike: impl Fn(char, char) -> bool) -> Vec<Slip> {
    Names::new(names).check(text, alike)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 亭/停 `ting`, 塵/尘 `chen`, 明/名 `ming` — the table a test needs.
    fn alike(a: char, b: char) -> bool {
        let sound = |c: char| match c {
            '亭' | '停' => "ting",
            '醉' | '罪' => "zui",
            '塵' | '尘' => "chen",
            '明' | '名' => "ming",
            '年' | '念' => "nian",
            '石' | '时' => "shi",
            '路' => "lu",
            '南' => "nan",
            '返' => "fan",
            '李' => "li",
            '朋' => "peng",
            _ => "?",
        };
        sound(a) == sound(b) && sound(a) != "?"
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_homophone_one_character_out_is_found() {
        let found = check("他到醉翁停。\n", &names(&["醉翁亭"]), alike);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].written, "醉翁停");
        assert_eq!(found[0].name, "醉翁亭");
        assert_eq!(found[0].pairs, vec![('停', '亭')]);
        assert_eq!(found[0].reasons, vec![Reason::Sound]);
        assert_eq!((found[0].line, found[0].column), (0, 2));
    }

    /// The whole reason the reading is asked for: without it, every name in a
    /// novel is one character from a dozen ordinary words.
    #[test]
    fn one_character_out_but_a_different_sound_is_not_a_slip() {
        assert!(check("他到醉翁路。\n", &names(&["醉翁亭"]), alike).is_empty());
    }

    #[test]
    fn the_name_written_right_is_never_a_slip() {
        assert!(check("他到醉翁亭。\n", &names(&["醉翁亭"]), alike).is_empty());
        // …and neither is a second name that is itself one character away.
        assert!(check("李明和李名。\n", &names(&["李明", "李名"]), alike).is_empty());
    }

    /// A single-character difference can fall anywhere, the first character
    /// included — which is why there are two indexes and not one.
    #[test]
    fn the_wrong_character_may_be_the_first_one() {
        assert!(check("尘翁亭。\n", &names(&["醉翁亭"]), alike).is_empty(), "尘 and 醉 are not one sound");
        let found = check("罪翁亭。\n", &names(&["醉翁亭"]), alike);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].column, 0);
    }

    /// Two homophones in one name is a different word, however alike.
    #[test]
    fn two_homophones_out_is_a_different_word() {
        assert!(check("他到罪翁停。\n", &names(&["醉翁亭"]), alike).is_empty());
    }

    #[test]
    fn a_one_character_name_is_never_asked_about() {
        assert!(check("墨。\n", &names(&["墨"]), alike).is_empty());
    }

    /// 繁简 may differ in every character: 瀋鶴年 is 沈鹤年.
    #[test]
    fn the_traditional_spelling_of_every_character_is_the_same_name() {
        let found = check("瀋鶴年到了青石鎮。\n", &names(&["沈鹤年", "青石镇"]), alike);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].written, "瀋鶴年");
        assert_eq!(found[0].name, "沈鹤年");
        assert_eq!(found[0].pairs, vec![('瀋', '沈'), ('鶴', '鹤')]);
        assert_eq!(found[0].reasons, vec![Reason::Script]);
        assert_eq!(found[1].written, "青石鎮");
        // …and the other way round, a simplified spelling of a traditional name.
        let back = check("沈鹤年。\n", &names(&["瀋鶴年"]), alike);
        assert_eq!(back.len(), 1, "{back:?}");
        assert_eq!(back[0].reasons, vec![Reason::Script]);
    }

    /// 异体: one 字形組, neither the other's 簡體 — 説/說, 裏/裡.
    #[test]
    fn a_variant_form_is_tagged_as_one() {
        let found = check("他在説書亭。\n", &names(&["說書亭"]), alike);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].reasons, vec![Reason::Variant]);
        let both = check("瀋鶴年在説書亭。\n", &names(&["沈鶴年", "說書亭"]), alike);
        assert_eq!(both.len(), 2, "{both:?}");
    }

    /// 兼容字: a compatibility ideograph (U+F9E4 is 理) or a 康熙部首 (U+2F08
    /// is 人) is the unified character after NFKC.
    #[test]
    fn a_compatibility_character_is_the_same_character() {
        let found = check("沈\u{F9E4}年。\n", &names(&["沈理年"]), alike);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].reasons, vec![Reason::Compat]);
        let radical = check("青\u{2F08}镇。\n", &names(&["青人镇"]), alike);
        assert_eq!(radical.len(), 1, "{radical:?}");
        assert_eq!(radical[0].reasons, vec![Reason::Compat]);
    }

    /// The reasons mix, and the sound cap counts sound alone: 繁简 in two
    /// characters and one homophone is still the name.
    #[test]
    fn reasons_mix_and_only_sound_is_capped() {
        let found = check("瀋鶴念。\n", &names(&["沈鹤年"]), alike);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].reasons, vec![Reason::Sound, Reason::Script]);
    }

    /// Warning: **形近 is not a reason.** 己/已, 未/末 and 鹤/鸖-like look-alikes
    /// are other characters, and nothing here may call them the name.
    #[test]
    fn a_look_alike_is_not_reported() {
        assert!(check("沈末年。\n", &names(&["沈未年"]), alike).is_empty());
        assert!(check("青己镇。\n", &names(&["青已镇"]), alike).is_empty());
        assert!(check("沈鹤午。\n", &names(&["沈鹤年"]), alike).is_empty());
    }

    /// 發 and 髮 are two characters; 发 may be either. The 字形組 is
    /// one-sided on purpose and the check keeps it so.
    #[test]
    fn two_characters_that_merely_share_a_simplified_form_are_not_one() {
        assert!(check("頭髮亭。\n", &names(&["頭發亭"]), alike).is_empty());
        assert_eq!(check("头髮亭。\n", &names(&["头发亭"]), alike).len(), 1);
    }

    /// Listing a variant is how a writer keeps it: an exact match wins.
    #[test]
    fn a_listed_variant_is_let_through() {
        assert!(check("瀋鶴年。\n", &names(&["沈鹤年", "瀋鶴年"]), alike).is_empty());
    }
}

#[cfg(test)]
mod bench {
    #[test]
    #[ignore = "a stopwatch, not a test"]
    fn the_cost_of_checking_a_chapter() {
        let names: Vec<String> = (0..200).map(|i| format!("醉翁亭{i}")).collect();
        let line = "他到醉翁亭，天色已晚，山門之外另有客舍三十間，逢法會則不敷用。\n";
        let text: String = line.repeat(20_000);
        let at = std::time::Instant::now();
        let found = super::check(&text, &names, |a, b| a == '停' && b == '亭');
        println!(
            "{} 行、{} 個名字：{:?}，{} 處",
            20_000,
            names.len(),
            at.elapsed(),
            found.len()
        );
    }
}
