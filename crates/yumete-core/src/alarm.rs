//! **The Unicode alarm** (#252): characters a reader cannot see
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
    match c {
        '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3164}' | '\u{FFA0}' => {
            Some(Alarm::Space)
        }
        '\u{00AD}' | '\u{180E}' | '\u{200B}' | '\u{200C}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}' => {
            Some(Alarm::Hidden)
        }
        _ if bidi_control(c) => Some(Alarm::Hidden),
        _ => None,
    }
}

/// **The bidirectional controls**, the characters "Trojan Source"
/// (CVE-2021-42574) hides behind: the embeddings, overrides and isolates
/// that reorder what follows them, and the three marks. The same twelve VS
/// Code draws as control characters (`isControlCharacter` in its
/// `viewLineRenderer.ts`). rustc's `text_direction_codepoint_in_literal` lint
/// warns on the first nine only and leaves out the marks, which reorder
/// nothing past themselves; a mark is as invisible as the rest, though, and a
/// reader asking why the caret jumps wants it named too.
pub fn bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

/// Its Unicode name, for the status line: `RIGHT-TO-LEFT OVERRIDE`.
pub fn name(c: char) -> Option<String> {
    alarm(c).and_then(|_| yumete_cjk::blocks::name(c))
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
        assert_eq!(name('\u{200B}').as_deref(), Some("ZERO WIDTH SPACE"));
    }

    /// **Trojan Source is tinted and named**: every bidi control VS Code
    /// draws, rustc's nine among them, takes no cell and has its name.
    #[test]
    fn the_bidi_controls_are_hidden_and_named() {
        let rustc = ['\u{202A}', '\u{202B}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{202C}', '\u{2069}'];
        let marks = ['\u{200E}', '\u{200F}', '\u{061C}'];
        for c in rustc.into_iter().chain(marks) {
            assert!(bidi_control(c), "{c:?}");
            assert_eq!(alarm(c), Some(Alarm::Hidden), "{c:?}");
            assert!(name(c).is_some_and(|n| !n.is_empty()), "{c:?}");
        }
        assert_eq!(name('\u{202E}').as_deref(), Some("RIGHT-TO-LEFT OVERRIDE"));
        assert_eq!(name('\u{2066}').as_deref(), Some("LEFT-TO-RIGHT ISOLATE"));
        assert_eq!(name('\u{061C}').as_deref(), Some("ARABIC LETTER MARK"));
        // The neighbours that are not controls stay plain.
        for c in ['\u{2029}', '\u{2065}', '\u{2070}', '\u{061B}', '\u{200D}'] {
            assert!(!bidi_control(c), "{c:?}");
        }
        // An override, an isolate and its closing pop: the cell after each
        // is tinted (the one before, for the pop at the end of the row), and
        // no character moves.
        let chars: Vec<char> = "a\u{202E}b\u{2066}c\u{2069}".chars().collect();
        assert_eq!(cells(&chars), vec![2, 4, 4]);
    }

    /// **Every character the alarm tints has a name** in the Unicode data,
    /// so the status line never says a bare code point for one.
    #[test]
    fn every_alarming_character_is_named() {
        for c in (0..=0x10FFFFu32).filter_map(char::from_u32).filter(|&c| alarm(c).is_some()) {
            assert!(name(c).is_some_and(|n| !n.is_empty()), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn what_cjk_and_emoji_need_is_left_alone() {
        for c in ['\u{3000}', '\u{200D}', '\u{FE0F}', '\u{E0100}', '，', '　'] {
            assert_eq!(alarm(c), None, "{c:?}");
        }
    }
}
