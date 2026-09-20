//! **專名寫錯了一次** — a name from the book's own 百科, one character off.
//!
//! [`crate::usage`] answers 「which spelling did this manuscript settle on」 and
//! it works because both spellings are in a list somebody wrote down. A proper
//! name is not in anybody's list: 返塵亭 is this book's, and the one place it
//! came out 返塵停 is a typo no dictionary and no 異體字表 can find. The book's
//! 百科 does know the name, though — that is what it is for — so the question
//! can be asked of it.
//!
//! ## Why a shared reading is required
//!
//! 「one character different」 is not a finding in Chinese: 返塵亭 is one
//! character from 返塵路, and a novel is full of those. What makes a slip a
//! slip is that it is the *same sound* — an IME took the wrong candidate, or a
//! hand wrote the homophone. 亭 and 停 are both `ting`; 塵 and 尘 are both
//! `chen`. So this asks the reading table, and where there is no reading table
//! it reports nothing at all rather than a page of noise.
//!
//! ## How the scan finds them without reading the book N times
//!
//! Two indexes, by **first** character and by **last**, each with the name's
//! length. A single-character difference can move at most one of those two, so
//! every candidate is in one index or the other — and the lists under a key
//! are a handful of names, not the whole 百科.

use std::collections::{BTreeSet, HashMap};

/// One place the manuscript wrote a name one character off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slip {
    /// Which line, counting from zero.
    pub line: usize,
    /// Which character of it, counting from zero.
    pub column: usize,
    /// What is written here.
    pub written: String,
    /// The name it is one character from.
    pub name: String,
    /// The character written, and the one the name has there.
    pub wrong: char,
    pub right: char,
}

/// Every place a 百科 name is written with one homophone out of place.
///
/// `alike` says whether two characters are read the same — the caller's, so
/// that this module holds no opinion about where a reading comes from and the
/// test can hand it a table of two.
///
/// ⚠️ **A name written correctly is never a slip**, including a name that is
/// itself one character from another name (a book of brothers 李明 and 李朋):
/// an exact match has no mismatched character to ask about.
pub fn check(text: &str, names: &[String], alike: impl Fn(char, char) -> bool) -> Vec<Slip> {
    let names: Vec<Vec<char>> = names
        .iter()
        .map(|n| n.chars().collect::<Vec<char>>())
        .filter(|n| n.len() >= 2)
        .collect();
    if names.is_empty() {
        return Vec::new();
    }
    let mut lengths: BTreeSet<usize> = BTreeSet::new();
    let mut by_head: HashMap<(char, usize), Vec<usize>> = HashMap::new();
    let mut by_tail: HashMap<(char, usize), Vec<usize>> = HashMap::new();
    for (i, name) in names.iter().enumerate() {
        let n = name.len();
        lengths.insert(n);
        by_head.entry((name[0], n)).or_default().push(i);
        by_tail.entry((name[n - 1], n)).or_default().push(i);
    }

    let mut out = Vec::new();
    for (line, body) in text.lines().enumerate() {
        let chars: Vec<char> = body.chars().collect();
        let mut at = 0usize;
        while at < chars.len() {
            let mut found: Option<Slip> = None;
            // Longest first: a slip in a four-character name is that name's,
            // not two overlapping findings about the three-character one
            // inside it.
            for &n in lengths.iter().rev() {
                if at + n > chars.len() || found.is_some() {
                    continue;
                }
                let here = &chars[at..at + n];
                let mut seen: Vec<usize> = Vec::new();
                for key in [(here[0], n), (here[n - 1], n)] {
                    for list in [by_head.get(&key), by_tail.get(&key)].into_iter().flatten() {
                        for &i in list {
                            if !seen.contains(&i) {
                                seen.push(i);
                            }
                        }
                    }
                }
                for i in seen {
                    let name = &names[i];
                    let mut wrong = None;
                    let mut ok = true;
                    for (a, b) in here.iter().zip(name.iter()) {
                        if a == b {
                            continue;
                        }
                        // Two characters out is a different word, not a slip.
                        if wrong.is_some() {
                            ok = false;
                            break;
                        }
                        wrong = Some((*a, *b));
                    }
                    let Some((a, b)) = wrong.filter(|_| ok) else { continue };
                    if !alike(a, b) {
                        continue;
                    }
                    found = Some(Slip {
                        line,
                        column: at,
                        written: here.iter().collect(),
                        name: name.iter().collect(),
                        wrong: a,
                        right: b,
                    });
                    break;
                }
            }
            match found {
                // Past the slip, so one wrong character in a sentence is one
                // finding however many names it is nearly.
                Some(slip) => {
                    at += slip.written.chars().count();
                    out.push(slip);
                }
                None => at += 1,
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 亭/停 `ting`, 塵/尘 `chen`, 明/名 `ming` — the table a test needs.
    fn alike(a: char, b: char) -> bool {
        let sound = |c: char| match c {
            '亭' | '停' => "ting",
            '塵' | '尘' => "chen",
            '明' | '名' => "ming",
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
        let found = check("他到返塵停。\n", &names(&["返塵亭"]), alike);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].written, "返塵停");
        assert_eq!(found[0].name, "返塵亭");
        assert_eq!((found[0].wrong, found[0].right), ('停', '亭'));
        assert_eq!((found[0].line, found[0].column), (0, 2));
    }

    /// The whole reason the reading is asked for: without it, every name in a
    /// novel is one character from a dozen ordinary words.
    #[test]
    fn one_character_out_but_a_different_sound_is_not_a_slip() {
        assert!(check("他到返塵路。\n", &names(&["返塵亭"]), alike).is_empty());
    }

    #[test]
    fn the_name_written_right_is_never_a_slip() {
        assert!(check("他到返塵亭。\n", &names(&["返塵亭"]), alike).is_empty());
        // …and neither is a second name that is itself one character away.
        assert!(check("李明和李朋。\n", &names(&["李明", "李朋"]), alike).is_empty());
    }

    /// A single-character difference can fall anywhere, the first character
    /// included — which is why there are two indexes and not one.
    #[test]
    fn the_wrong_character_may_be_the_first_one() {
        assert!(check("尘塵亭。\n", &names(&["返塵亭"]), alike).is_empty(), "尘 and 返 are not one sound");
        let found = check("塵塵亭。\n", &names(&["返塵亭"]), |a, b| a == '塵' && b == '返');
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].column, 0);
    }

    #[test]
    fn two_characters_out_is_a_different_word() {
        assert!(check("他到尘塵停。\n", &names(&["返塵亭"]), alike).is_empty());
    }

    #[test]
    fn a_one_character_name_is_never_asked_about() {
        assert!(check("墨。\n", &names(&["墨"]), alike).is_empty());
    }
}
