//! **`:check-spelling`: the English words the dictionary does not know.**
//!
//! A Chinese manuscript carries English too — a title, a name, a quoted
//! line — and a typo there is as easy to read past as 醉翁停 is. The question
//! is asked only when the writer asks it, over a file or a book; nothing is
//! underlined while typing and nothing runs behind.
//!
//! What is asked about is a **word of Latin letters in prose**. Whatever is not
//! prose is blanked first: code (the same masking [`crate::punct`] does),
//! maths, front matter, link targets, HTML tags, URLs and e-mail addresses. Of
//! what is left, these are let through without asking, because a dictionary
//! has no opinion about them:
//!
//! - a single letter: a variable, a list label, `I`;
//! - a word with a digit, or a capital after its first letter (`UTF8`,
//!   `JSON`, `LaTeX`, `iPhone`): acronyms and the names of things;
//! - a word joined to the next one by `.` `/` `\` `_` `@` (`config.toml`,
//!   `src/main`, `snake_case`), or opened by `\` `#` `@` `&` (`\frac`,
//!   `#set`, `@smith`, `&nbsp`): names a computer reads, not words.
//!
//! A capital at the start is a sentence's: the dictionary reads `Hello` as
//! `hello`, and `London` as itself.

use std::collections::{HashMap, HashSet};

/// The en_US dictionary, packed by build.rs.
const AFF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/en_US.aff.zst"));
const DIC: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/en_US.dic.zst"));

/// SCOWL's en_US word list (size 60), unpacked and read.
///
/// About 5 ms, so it is made each time the command runs and dropped after it,
/// rather than kept in memory for a command most sessions never type.
pub fn dictionary() -> Option<spellbook::Dictionary> {
    let unpack = |packed: &[u8]| String::from_utf8(zstd::decode_all(packed).ok()?).ok();
    spellbook::Dictionary::new(&unpack(AFF)?, &unpack(DIC)?).ok()
}

/// One word the dictionary does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Miss<L> {
    /// Where it is first written.
    pub at: L,
    /// As it is written there.
    pub word: String,
    /// How many times it is written, in any capitalisation.
    pub count: usize,
}

/// The words `known` turns down, **one per word**, at its first place.
///
/// A misspelt name written forty times is one decision, not forty rows. Words
/// are told apart by their lower-case form, so `Teh` at the start of a
/// sentence and `teh` inside one are the same slip, and a possessive `'s` is
/// left off (`Wenlan's` is a row of `Wenlan`). `known` is still asked about
/// each spelling as written, since `London` is a word and `london` is not.
pub fn misses<L: Clone>(
    words: impl IntoIterator<Item = (L, String)>,
    known: impl Fn(&str) -> bool,
) -> Vec<Miss<L>> {
    let mut asked: HashMap<String, bool> = HashMap::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut found: Vec<Miss<L>> = Vec::new();
    for (place, word) in words {
        let ok = *asked.entry(word.clone()).or_insert_with(|| known(&word));
        if ok {
            continue;
        }
        let word = word.strip_suffix("'s").filter(|w| !w.is_empty()).unwrap_or(&word).to_string();
        let key = word.to_lowercase();
        match at.get(&key) {
            Some(&i) => found[i].count += 1,
            None => {
                at.insert(key, found.len());
                found.push(Miss { at: place, word, count: 1 });
            }
        }
    }
    found
}

/// Whether the dictionary knows `word`, or the word a possessive `'s` was
/// added to: the list carries `parser` but not every `parser's`.
pub fn knows(dict: &spellbook::Dictionary, word: &str) -> bool {
    dict.check(word) || word.strip_suffix("'s").is_some_and(|base| !base.is_empty() && dict.check(base))
}

/// The words of `.yumete/words.txt`, in the form [`misses`] compares them.
///
/// An entry may be a phrase (`Jean-Luc Picard`) or carry Chinese with it;
/// every Latin word in it is accepted, in any capitalisation.
pub fn accepted<'a>(entries: impl IntoIterator<Item = &'a str>) -> HashSet<String> {
    entries
        .into_iter()
        .flat_map(|entry| {
            entry
                .split(|c: char| !(latin_letter(c) || is_apostrophe(c)))
                .map(|w| w.trim_matches(is_apostrophe).replace('’', "'").to_lowercase())
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether `word` is in the list [`accepted`] made, with or without a
/// possessive `'s`.
pub fn is_accepted(list: &HashSet<String>, word: &str) -> bool {
    let folded = word.to_lowercase();
    list.contains(&folded) || folded.strip_suffix("'s").is_some_and(|base| list.contains(base))
}

/// Every word of `text` the dictionary is asked about, with its line, in
/// reading order. `’` is written `'`, the apostrophe the dictionary spells.
pub fn words(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let front = front_matter_end(&lines);
    let mut out = Vec::new();
    let mut fence = false;
    let mut maths = false;
    for (n, line) in lines.iter().enumerate() {
        if front.is_some_and(|end| n <= end) {
            continue;
        }
        let trimmed = line.trim();
        if maths {
            if trimmed.ends_with("$$") {
                maths = false;
            }
            continue;
        }
        if !fence {
            if let Some(rest) = trimmed.strip_prefix("$$") {
                maths = !rest.contains("$$");
                continue;
            }
        }
        let mut chars = crate::punct::prose(line, &mut fence);
        if reference_definition(trimmed) {
            continue;
        }
        blank_markup(&mut chars);
        blank_addresses(&mut chars);
        for word in words_in(&chars) {
            out.push((n, word));
        }
    }
    out
}

/// The last line of a front matter block at the top: `---` to `---` or
/// `...` (YAML), `+++` to `+++` (TOML). `None` when there is none.
fn front_matter_end(lines: &[&str]) -> Option<usize> {
    let first = lines.first()?.trim_end();
    let closers: &[&str] = match first {
        "---" => &["---", "..."],
        "+++" => &["+++"],
        _ => return None,
    };
    lines.iter().skip(1).position(|l| closers.contains(&l.trim_end())).map(|i| i + 1)
}

/// `[label]: target` — a link's address on a line of its own. A footnote's
/// definition (`[^1]: …`) is prose, and is left to be read.
fn reference_definition(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix('[') else {
        return false;
    };
    if rest.starts_with('^') {
        return false;
    }
    rest.find("]:").is_some_and(|end| end > 0 && !rest[..end].contains('['))
}

fn blank(chars: &mut [char], from: usize, to: usize) {
    for c in chars.iter_mut().take(to + 1).skip(from) {
        *c = ' ';
    }
}

fn find(chars: &[char], from: usize, want: char) -> Option<usize> {
    chars.get(from..)?.iter().position(|&c| c == want).map(|i| from + i)
}

/// Inline maths, link targets and HTML tags, each blanked to spaces.
fn blank_markup(chars: &mut [char]) {
    let mut at = 0;
    while at < chars.len() {
        let next = chars.get(at + 1).copied();
        match chars[at] {
            // `$x+y$`: a `$` with text right after it, closed by one with text
            // right before it, so `$5 and $10` is left alone.
            '$' if next.is_some_and(|c| !c.is_whitespace() && c != '$') => {
                let close = (at + 2..chars.len()).find(|&j| chars[j] == '$' && !chars[j - 1].is_whitespace());
                if let Some(close) = close {
                    blank(chars, at, close);
                    at = close + 1;
                    continue;
                }
            }
            // `](target)` — the address of a link or a picture.
            ']' if next == Some('(') => {
                if let Some(close) = find(chars, at + 2, ')') {
                    blank(chars, at + 1, close);
                    at = close + 1;
                    continue;
                }
            }
            // `[[target]]`, `[[target|label]]`: the page, not the label.
            '[' if next == Some('[') => {
                let close = (at + 2..chars.len()).find(|&j| chars[j] == '|' || (chars[j] == ']' && chars.get(j + 1) == Some(&']')));
                if let Some(close) = close {
                    blank(chars, at, close);
                    at = close + 1;
                    continue;
                }
            }
            // `<tag …>`, `</tag>`, `<!-- …>`, `<https://…>`; `a < b` is not one.
            '<' if next.is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!') => {
                if let Some(close) = find(chars, at + 1, '>') {
                    blank(chars, at, close);
                    at = close + 1;
                    continue;
                }
            }
            _ => {}
        }
        at += 1;
    }
}

/// URLs and e-mail addresses: a run of non-space, non-CJK characters with
/// `://` in it, starting `www.`, or with `@` between two letters.
fn blank_addresses(chars: &mut [char]) {
    let mut at = 0;
    while at < chars.len() {
        if !in_chunk(chars[at]) {
            at += 1;
            continue;
        }
        let end = (at..chars.len()).find(|&j| !in_chunk(chars[j])).unwrap_or(chars.len());
        let chunk: String = chars[at..end].iter().collect();
        let mail = chars[at..end]
            .iter()
            .enumerate()
            .any(|(i, &c)| c == '@' && i > 0 && chars[at + i - 1].is_alphanumeric() && chars.get(at + i + 1).is_some_and(|c| c.is_alphanumeric()));
        if chunk.contains("://") || chunk.starts_with("www.") || mail {
            blank(chars, at, end - 1);
        }
        at = end;
    }
}

/// A character an address can be made of: not a space, and before the
/// general punctuation block, so Chinese text and its marks end one.
fn in_chunk(c: char) -> bool {
    !c.is_whitespace() && (c as u32) < 0x2000
}

/// A letter of a script written left to right in Latin or near it — up to
/// the general punctuation block, which keeps Han, kana and full-width
/// letters out. Greek and Cyrillic are letters here so that they end no word
/// halfway, and a word holding one is then not asked about.
fn latin_letter(c: char) -> bool {
    c.is_alphabetic() && (c as u32) < 0x2000
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '’'
}

/// The words of one masked line that the dictionary is asked about.
fn words_in(chars: &[char]) -> Vec<String> {
    let word_char = |c: char| latin_letter(c) || c.is_ascii_digit() || is_apostrophe(c);
    let alnum = |c: Option<&char>| c.is_some_and(|&c| latin_letter(c) || c.is_ascii_digit());
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if !word_char(chars[at]) {
            at += 1;
            continue;
        }
        let start = at;
        let end = (at..chars.len()).find(|&j| !word_char(chars[j])).unwrap_or(chars.len());
        at = end;
        let before = start.checked_sub(1).map(|i| chars[i]);
        let after = chars.get(end).copied();
        const JOINERS: &[char] = &['.', '/', '\\', '_', '@'];
        if before.is_some_and(|c| matches!(c, '\\' | '#' | '@' | '&')) {
            continue;
        }
        if before.is_some_and(|c| JOINERS.contains(&c)) && alnum(start.checked_sub(2).and_then(|i| chars.get(i))) {
            continue;
        }
        if after.is_some_and(|c| JOINERS.contains(&c)) && alnum(chars.get(end + 1)) {
            continue;
        }
        let word: String = chars[start..end].iter().collect();
        let word = word.trim_matches(is_apostrophe).replace('’', "'");
        let rest: Vec<char> = word.chars().skip(1).collect();
        if rest.iter().all(|&c| c == '\'') {
            continue;
        }
        if word.chars().any(|c| c.is_ascii_digit() || (c.is_alphabetic() && c as u32 >= 0x0250)) {
            continue;
        }
        if rest.iter().any(|c| c.is_uppercase()) {
            continue;
        }
        out.push(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn just_words(text: &str) -> Vec<String> {
        words(text).into_iter().map(|(_, w)| w).collect()
    }

    /// The dictionary loads, knows an ordinary word, a sentence's capital and
    /// a name, and turns a typo down.
    #[test]
    fn the_built_in_dictionary_reads() {
        let dict = dictionary().expect("the dictionary unpacks and reads");
        for good in ["receive", "Receive", "London", "don't"] {
            assert!(dict.check(good), "{good}");
        }
        for bad in ["recieve", "teh", "london"] {
            assert!(!dict.check(bad), "{bad}");
        }
        assert!(knows(&dict, "parser's") && !knows(&dict, "parsr's"));
    }

    #[test]
    fn han_text_ends_a_word_and_is_never_one() {
        assert_eq!(just_words("用Python写的recieve程序"), ["Python", "recieve"]);
        assert!(just_words("醉翁亭记。").is_empty());
    }

    #[test]
    fn code_maths_and_front_matter_are_not_prose() {
        let text = "---\ntitle: Wrold\n---\nSee `fn mian()` here.\n```\nlet teh = 1;\n```\n$$\n\\mathrm{foo}\n$$\nthe $abc + xyz$ sum\n";
        assert_eq!(just_words(text), ["See", "here", "the", "sum"]);
    }

    #[test]
    fn link_targets_tags_urls_and_mail_are_not_words() {
        let text = "a [nice lnik](chaptre-two.md) and [[Pgae|lable]]\n<span class=\"wrogn\">ok</span>\nsee https://exmaple.com/pth or www.exmaple.org and someone@exmaple.com\n[ref]: ./some/pth\n";
        assert_eq!(just_words(text), ["nice", "lnik", "and", "lable", "ok", "see", "or", "and"]);
    }

    #[test]
    fn acronyms_inner_capitals_digits_and_joined_names_pass() {
        let text = "JSON LaTeX iPhone UTF8 x86 config.toml src/main snake_case \\frac #set @smith &nbsp; I a";
        assert!(just_words(text).is_empty(), "{:?}", just_words(text));
        assert_eq!(just_words("End.Next _emph_ 'quoted' dogs' don’t"), ["emph", "quoted", "dogs", "don't"]);
    }

    #[test]
    fn one_row_per_word_counted_in_any_capitalisation() {
        let found = misses(words("Teh cat.\nteh dog, teh's end.\nLondon and london.\n"), |w| {
            !matches!(w, "Teh" | "teh" | "teh's" | "london")
        });
        assert_eq!(
            found,
            [
                Miss { at: 0, word: "Teh".to_string(), count: 3 },
                Miss { at: 2, word: "london".to_string(), count: 1 },
            ]
        );
    }

    #[test]
    fn words_txt_entries_accept_every_latin_word_in_them() {
        let list = accepted(["Jean-Luc Picard", "青石镇", "yumete"]);
        assert!(is_accepted(&list, "Picard") && is_accepted(&list, "luc") && is_accepted(&list, "Yumete's"));
        assert!(!is_accepted(&list, "Picardy"));
    }
}
