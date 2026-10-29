//! **The Unicode alarm** (#252, 2026-10-29): characters a reader cannot see
//! or cannot tell from a space, which arrive by pasting and do their harm
//! silently — a zero-width space splitting `price` so that a search misses
//! it, a bidi control reordering the text after it (the "Trojan Source"
//! trick), a no-break space that wraps differently.
//!
//! Two kinds, because a terminal draws them two ways: a [`Alarm::Hidden`] one
//! takes no cell, so the cell after it is tinted and the line never moves;
//! a [`Alarm::Space`] one takes a cell that looks blank, and that cell is
//! tinted.
//!
//! **Left out on purpose**: U+3000 (a Chinese paragraph's indent), the
//! zero-width joiner (emoji are built with it), and the variation selectors,
//! U+E0100 among them (a 漢字's exact form is chosen with them).

/// Which kind of character it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alarm {
    /// No width at all: invisible and bidi controls.
    Hidden,
    /// A space that is not U+0020.
    Space,
}

/// Whether `c` is one of them, and which kind.
pub fn alarm(c: char) -> Option<Alarm> {
    name(c).map(|_| match c {
        '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3164}' | '\u{FFA0}' => {
            Alarm::Space
        }
        _ => Alarm::Hidden,
    })
}

/// Its Unicode name, for the status line.
pub fn name(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{00A0}' => "NO-BREAK SPACE",
        '\u{00AD}' => "SOFT HYPHEN",
        '\u{061C}' => "ARABIC LETTER MARK",
        '\u{180E}' => "MONGOLIAN VOWEL SEPARATOR",
        '\u{2000}' => "EN QUAD",
        '\u{2001}' => "EM QUAD",
        '\u{2002}' => "EN SPACE",
        '\u{2003}' => "EM SPACE",
        '\u{2004}' => "THREE-PER-EM SPACE",
        '\u{2005}' => "FOUR-PER-EM SPACE",
        '\u{2006}' => "SIX-PER-EM SPACE",
        '\u{2007}' => "FIGURE SPACE",
        '\u{2008}' => "PUNCTUATION SPACE",
        '\u{2009}' => "THIN SPACE",
        '\u{200A}' => "HAIR SPACE",
        '\u{200B}' => "ZERO WIDTH SPACE",
        '\u{200C}' => "ZERO WIDTH NON-JOINER",
        '\u{200E}' => "LEFT-TO-RIGHT MARK",
        '\u{200F}' => "RIGHT-TO-LEFT MARK",
        '\u{202A}' => "LEFT-TO-RIGHT EMBEDDING",
        '\u{202B}' => "RIGHT-TO-LEFT EMBEDDING",
        '\u{202C}' => "POP DIRECTIONAL FORMATTING",
        '\u{202D}' => "LEFT-TO-RIGHT OVERRIDE",
        '\u{202E}' => "RIGHT-TO-LEFT OVERRIDE",
        '\u{202F}' => "NARROW NO-BREAK SPACE",
        '\u{205F}' => "MEDIUM MATHEMATICAL SPACE",
        '\u{2060}' => "WORD JOINER",
        '\u{2061}' => "FUNCTION APPLICATION",
        '\u{2062}' => "INVISIBLE TIMES",
        '\u{2063}' => "INVISIBLE SEPARATOR",
        '\u{2064}' => "INVISIBLE PLUS",
        '\u{2066}' => "LEFT-TO-RIGHT ISOLATE",
        '\u{2067}' => "RIGHT-TO-LEFT ISOLATE",
        '\u{2068}' => "FIRST STRONG ISOLATE",
        '\u{2069}' => "POP DIRECTIONAL ISOLATE",
        '\u{3164}' => "HANGUL FILLER",
        '\u{FEFF}' => "ZERO WIDTH NO-BREAK SPACE",
        '\u{FFA0}' => "HALFWIDTH HANGUL FILLER",
        _ => return None,
    })
}

/// **Which cells of a row to tint**: for each alarming character, its own
/// cell (a space) or the next one (a hidden one; the one before it at the end
/// of the row). Indices into `chars`.
pub fn cells(chars: &[char]) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        match alarm(c) {
            Some(Alarm::Space) => out.push(i),
            Some(Alarm::Hidden) => {
                // The next character that takes a cell, past a run of hidden ones.
                let next = (i + 1..chars.len()).find(|&j| alarm(chars[j]) != Some(Alarm::Hidden));
                if let Some(j) = next.or(i.checked_sub(1)) {
                    out.push(j);
                }
            }
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_ones_tint_the_next_cell_and_spaces_their_own() {
        let chars: Vec<char> = "pri\u{200B}ce a\u{00A0}b x\u{202E}".chars().collect();
        assert_eq!(cells(&chars), vec![4, 8, 11]);
        assert_eq!(name('\u{200B}'), Some("ZERO WIDTH SPACE"));
    }

    #[test]
    fn what_cjk_and_emoji_need_is_left_alone() {
        for c in ['\u{3000}', '\u{200D}', '\u{FE0F}', '\u{E0100}', '，', '　'] {
            assert_eq!(alarm(c), None, "{c:?}");
        }
    }
}
