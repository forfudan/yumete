//! Colouring Markdown without hiding it — Feature #96.
//!
//! A renderer's job is to turn `**字**` into a bold 字 and throw the asterisks
//! away. This does the opposite half: the asterisks **stay on the page**,
//! because the file is the manuscript and a writer needs to see what is in it —
//! but the 字 between them is set bold, so a page of prose shows its own shape.
//!
//! Deliberately not a CommonMark parser, and deliberately not `pulldown-cmark`:
//!
//! - **It is per line.** Everything here is redrawn on every keystroke, and a
//!   parse of the whole document per frame is what this editor has already had
//!   to take out of word motion, the segmentation overlay and search. Emphasis
//!   in prose does not straddle a paragraph, so a paragraph is the right unit —
//!   and one paragraph's answer can be cached against its own text.
//! - **CommonMark's emphasis rules are wrong for 漢字.** They are built on the
//!   left/right-flanking of *space-delimited* words; between two 漢字 they
//!   misfire, and it is a long-standing complaint against every implementation.
//!   Here a delimiter is a delimiter.
//! - **It is flat.** No emphasis inside emphasis. On a page of prose the extra
//!   fidelity buys nothing and costs predictability.

/// What a span of a line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `**bold**` — the text between the markers.
    Strong,
    /// `*italic*` or `_italic_`.
    Emphasis,
    /// `` `code` ``.
    Code,
    /// `~~struck~~`.
    Strike,
    /// The text of a heading line, after its hashes.
    Heading,
    /// The visible text of a `[link](target)`.
    Link,
    /// The markup itself — the asterisks, the hashes, the brackets and the
    /// target. Shown, but set back, so it reads as scaffolding rather than as
    /// something the reader wrote.
    Marker,
}

/// A run of one line, in char indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

/// The marked-up runs of `line`, in order and non-overlapping.
///
/// Anything not covered is ordinary prose and gets no span.
pub fn spans(line: &str) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();

    // A heading is the whole line, so it is decided before anything else and
    // the rest of the line is still scanned for emphasis inside the title.
    let mut from = 0;
    let hashes = chars.iter().take_while(|&&c| c == '#').count();
    if hashes > 0 && hashes <= 6 && matches!(chars.get(hashes), Some(' ') | None) {
        out.push(Span {
            start: 0,
            end: hashes,
            kind: Kind::Marker,
        });
        from = hashes;
        // The title itself, under whatever emphasis it also carries.
        if from < chars.len() {
            out.push(Span {
                start: from,
                end: chars.len(),
                kind: Kind::Heading,
            });
        }
    }

    let mut at = from;
    while at < chars.len() {
        // Code first: inside a code span nothing else is markup.
        if chars[at] == '`' {
            if let Some(close) = find(&chars, at + 1, |c| c == '`') {
                mark(&mut out, at, at + 1, Kind::Marker);
                mark(&mut out, at + 1, close, Kind::Code);
                mark(&mut out, close, close + 1, Kind::Marker);
                at = close + 1;
                continue;
            }
        }
        if let Some(len) = fence(&chars, at) {
            let kind = match (chars[at], len) {
                ('*', 2) | ('_', 2) => Kind::Strong,
                ('~', 2) => Kind::Strike,
                _ => Kind::Emphasis,
            };
            if let Some(close) = closing(&chars, at + len, chars[at], len) {
                mark(&mut out, at, at + len, Kind::Marker);
                mark(&mut out, at + len, close, kind);
                mark(&mut out, close, close + len, Kind::Marker);
                at = close + len;
                continue;
            }
        }
        // `[text](target)` — the text is what the reader reads.
        if chars[at] == '[' {
            if let Some(close) = find(&chars, at + 1, |c| c == ']') {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = find(&chars, close + 2, |c| c == ')') {
                        mark(&mut out, at, at + 1, Kind::Marker);
                        mark(&mut out, at + 1, close, Kind::Link);
                        mark(&mut out, close, end + 1, Kind::Marker);
                        at = end + 1;
                        continue;
                    }
                }
            }
        }
        at += 1;
    }
    out
}

/// How many delimiter characters start at `at`, or `None` when none do.
///
/// An underscore only opens where a word does not: `snake_case` is a name, not
/// an emphasis, and in a manuscript full of file names that matters more than
/// being able to italicise with `_`.
fn fence(chars: &[char], at: usize) -> Option<usize> {
    let c = chars[at];
    if !matches!(c, '*' | '_' | '~') {
        return None;
    }
    let len = chars[at..].iter().take_while(|&&x| x == c).count().min(2);
    if c == '~' && len < 2 {
        return None;
    }
    if c == '_' {
        let before = at.checked_sub(1).and_then(|i| chars.get(i));
        if before.is_some_and(|c| c.is_alphanumeric()) {
            return None;
        }
    }
    // A delimiter with nothing after it opens nothing.
    (at + len < chars.len()).then_some(len)
}

/// Where the run of `len` `delimiter`s closing the one opened at `from` begins.
///
/// The content may not be empty and may not start with a space: `** ` in the
/// middle of a sentence is two asterisks, not the start of a bold run.
fn closing(chars: &[char], from: usize, delimiter: char, len: usize) -> Option<usize> {
    if chars.get(from) == Some(&' ') {
        return None;
    }
    let mut at = from;
    while at + len <= chars.len() {
        if chars[at..at + len].iter().all(|&c| c == delimiter)
            && chars.get(at.wrapping_sub(1)) != Some(&' ')
            && at > from
        {
            return Some(at);
        }
        at += 1;
    }
    None
}

/// The first index at or after `from` whose character satisfies `f`.
fn find(chars: &[char], from: usize, f: impl Fn(char) -> bool) -> Option<usize> {
    (from..chars.len()).find(|&i| f(chars[i]))
}

/// Push a span, dropping empty ones.
fn mark(out: &mut Vec<Span>, start: usize, end: usize, kind: Kind) {
    if end > start {
        out.push(Span { start, end, kind });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kind covering each character, for reading a line's shape at a glance.
    /// Later spans win, which is how a heading's title also shows its emphasis.
    fn shape(line: &str) -> String {
        let n = line.chars().count();
        let mut out = vec![' '; n];
        for span in spans(line) {
            let mark = match span.kind {
                Kind::Strong => 'B',
                Kind::Emphasis => 'I',
                Kind::Code => 'C',
                Kind::Strike => 'S',
                Kind::Heading => 'H',
                Kind::Link => 'L',
                Kind::Marker => '.',
            };
            for slot in out.iter_mut().take(span.end.min(n)).skip(span.start) {
                *slot = mark;
            }
        }
        out.into_iter().collect()
    }

    #[test]
    fn the_markers_stay_and_the_text_between_them_is_set() {
        assert_eq!(shape("那**年**天"), " ..B.. ");
        assert_eq!(shape("那*年*天"), " .I. ");
        assert_eq!(shape("那`碼`天"), " .C. ");
        assert_eq!(shape("那~~年~~天"), " ..S.. ");
    }

    #[test]
    fn a_heading_is_the_line_and_its_hashes_are_markup() {
        assert_eq!(shape("## 第一章"), "..HHHH");
        // And emphasis inside the title still shows.
        assert_eq!(shape("# **甲**"), ".H..B..");
        // Six is the deepest; seven hashes is just a line of hashes.
        assert_eq!(shape("####### 甲"), " ".repeat(9));
        // A run of hashes with no title is an empty heading — markup, and
        // nothing else. It contributes nothing to the outline either.
        assert_eq!(shape("###"), "...");
    }

    #[test]
    fn emphasis_between_two_漢字_works_here_even_though_commonmark_argues() {
        // The whole reason this is not `pulldown-cmark`.
        assert_eq!(shape("中**文**中"), " ..B.. ");
        assert_eq!(shape("「**甲**」"), " ..B.. ");
    }

    #[test]
    fn an_underscore_inside_a_word_is_part_of_the_word() {
        assert_eq!(shape("snake_case_name"), " ".repeat(15));
        // At a word boundary it still emphasises.
        assert_eq!(shape("那 _年_ 天"), "  .I.  ");
    }

    #[test]
    fn an_unclosed_or_empty_delimiter_is_just_a_character() {
        assert_eq!(shape("那**年"), " ".repeat(4));
        assert_eq!(shape("那 ** 年"), " ".repeat(6));
        assert_eq!(shape("2 * 3 * 4"), " ".repeat(9));
    }

    #[test]
    fn code_masks_the_markup_inside_it() {
        assert_eq!(shape("`**not bold**`"), ".CCCCCCCCCCCC.");
    }

    #[test]
    fn a_link_shows_its_text_and_sets_its_target_back() {
        assert_eq!(shape("見[附錄](a.md)"), " .LL.......");
    }

    #[test]
    fn ordinary_prose_carries_no_spans() {
        assert!(spans("那年冬天，雪下得早。").is_empty());
        assert!(spans("").is_empty());
    }
}
