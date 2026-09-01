//! Vertical-layout presentation forms — Feature #61.
//!
//! A terminal draws one glyph per cell and never applies OpenType's `vert` /
//! `vrt2` features, so the punctuation substitution that a real typesetter gets
//! for free has to be done by hand: set vertically, the comma, the full stop and
//! every bracket pair must be *rotated or repositioned*, not merely stacked.
//!
//! Unicode already encodes those rotated shapes as the "presentation forms for
//! vertical" (U+FE10–U+FE19 and U+FE30–U+FE48), so [`vertical_form`] is a plain
//! codepoint mapping. It is applied at **render time only** — the buffer, the
//! file on disk, and anything yanked or searched keep the ordinary horizontal
//! characters, so a vertically-rendered document is still a normal text file.
//!
//! Only fullwidth and CJK punctuation is mapped. ASCII punctuation is left
//! alone: it is halfwidth, it has no vertical form, and in a CJK buffer it is
//! almost always part of a Latin run that the reader wants to see unchanged.
//!
//! Font coverage is the one thing yumete cannot control. The mapped codepoints
//! are present in the Source Han / Noto CJK families and in CJK-aware monospace
//! fonts such as Sarasa Gothic; a Latin-only programming font will render them
//! as tofu. See the crate docs on terminal font configuration.

/// The default number of graphemes in one 縱 (Feature #61).
///
/// 縱 (*zong*) is what a line of text is called in vertical layout, where "line"
/// and "column" would each mean two things. Novels are typeset at 24–32
/// characters per 縱: past that the eye loses the return sweep to the top of the
/// next one. 32 is the comfortable upper end; the renderer shortens it further
/// when the terminal is not tall enough.
pub const DEFAULT_ZONG_LENGTH: usize = 32;

/// The default gap between two 縱, in half-width cells.
///
/// One half-width cell against the two-cell 縱 gives a spacing of half an em —
/// enough to keep them apart without the airy feel of a full em.
pub const DEFAULT_ZONG_GAP: usize = 1;

/// The layout the editor arranges text in (Feature #61).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// Lines run left to right and stack downward. The default.
    #[default]
    Horizontal,
    /// Text runs top to bottom in 縱 that stack from the right edge leftward.
    Vertical,
}

impl Layout {
    /// The other layout.
    pub fn toggled(self) -> Layout {
        match self {
            Layout::Horizontal => Layout::Vertical,
            Layout::Vertical => Layout::Horizontal,
        }
    }

    /// Parse a layout from a config value or a `:layout` argument.
    pub fn parse(value: &str) -> Option<Layout> {
        match value.trim().to_ascii_lowercase().as_str() {
            "vertical" | "vert" | "v" => Some(Layout::Vertical),
            "horizontal" | "horiz" | "h" => Some(Layout::Horizontal),
            _ => None,
        }
    }

    /// A short label for the status line.
    pub fn label(self) -> &'static str {
        match self {
            Layout::Horizontal => "horizontal",
            Layout::Vertical => "vertical",
        }
    }
}

/// The vertical presentation form of `c`, or `None` when `c` is drawn the same
/// way in both writing directions (which is the case for every 漢字, kana, and
/// Latin letter).
///
/// # Examples
///
/// ```
/// use yumete_cjk::vertical_form;
/// assert_eq!(vertical_form('。'), Some('︒'));
/// assert_eq!(vertical_form('「'), Some('﹁'));
/// assert_eq!(vertical_form('字'), None);
/// ```
pub fn vertical_form(c: char) -> Option<char> {
    let mapped = match c {
        // Commas, stops and other separators (U+FE10–U+FE19).
        '，' => '︐',
        '、' => '︑',
        '。' => '︒',
        '：' => '︓',
        '；' => '︔',
        '！' => '︕',
        '？' => '︖',
        '〖' => '︗',
        '〗' => '︘',
        // Both the ellipsis and the midline ellipsis take the vertical ellipsis.
        '…' | '⋯' => '︙',
        // Leaders, dashes and the low line (U+FE30–U+FE33).
        '‥' => '︰',
        // EM DASH and HORIZONTAL BAR share one vertical form; 破折號 is usually
        // written as a doubled em dash, which stacks into an unbroken rule.
        '—' | '―' => '︱',
        '–' => '︲',
        '＿' => '︳',
        // Bracket pairs (U+FE35–U+FE44, U+FE47–U+FE48).
        '（' => '︵',
        '）' => '︶',
        '｛' => '︷',
        '｝' => '︸',
        '〔' => '︹',
        '〕' => '︺',
        '【' => '︻',
        '】' => '︼',
        '《' => '︽',
        '》' => '︾',
        '〈' => '︿',
        '〉' => '﹀',
        '「' => '﹁',
        '」' => '﹂',
        '『' => '﹃',
        '』' => '﹄',
        '［' => '﹇',
        '］' => '﹈',
        _ => return None,
    };
    Some(mapped)
}

/// Rewrite a grapheme cluster for vertical rendering.
///
/// Returns `None` when the cluster is unchanged, so callers can keep borrowing
/// the original text in the common case. Only single-character clusters are
/// substituted: a base character carrying combining marks or an ideographic
/// variation selector keeps the variation, which is what the reader chose.
pub fn vertical_grapheme(g: &str) -> Option<char> {
    let mut chars = g.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    vertical_form(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::char_width;

    #[test]
    fn layout_parses_its_names() {
        for v in ["vertical", "Vert", "v", " VERTICAL "] {
            assert_eq!(Layout::parse(v), Some(Layout::Vertical), "{v}");
        }
        for v in ["horizontal", "horiz", "h"] {
            assert_eq!(Layout::parse(v), Some(Layout::Horizontal), "{v}");
        }
        assert_eq!(Layout::parse("sideways"), None);
        assert_eq!(Layout::default(), Layout::Horizontal);
        assert_eq!(Layout::Horizontal.toggled(), Layout::Vertical);
    }

    #[test]
    fn maps_stops_and_commas() {
        assert_eq!(vertical_form('。'), Some('︒'));
        assert_eq!(vertical_form('、'), Some('︑'));
        assert_eq!(vertical_form('，'), Some('︐'));
        assert_eq!(vertical_form('？'), Some('︖'));
    }

    #[test]
    fn maps_bracket_pairs() {
        assert_eq!(vertical_form('「'), Some('﹁'));
        assert_eq!(vertical_form('」'), Some('﹂'));
        assert_eq!(vertical_form('《'), Some('︽'));
        assert_eq!(vertical_form('（'), Some('︵'));
    }

    #[test]
    fn leaves_han_kana_and_ascii_alone() {
        for c in ['字', '中', 'あ', 'a', 'Z', '1', '(', ',', ' '] {
            assert_eq!(vertical_form(c), None, "{c} should be unchanged");
        }
    }

    /// Every vertical form must still occupy two cells, or the 纵 grid would
    /// break apart at the first quotation mark.
    #[test]
    fn every_vertical_form_is_two_cells_wide() {
        let sources = [
            '，', '、', '。', '：', '；', '！', '？', '〖', '〗', '…', '⋯', '‥', '—', '―', '–',
            '＿', '（', '）', '｛', '｝', '〔', '〕', '【', '】', '《', '》', '〈', '〉', '「',
            '」', '『', '』', '［', '］',
        ];
        for c in sources {
            let v = vertical_form(c).expect("mapped");
            assert_eq!(char_width(v), 2, "U+{:04X} is not wide", v as u32);
        }
    }

    #[test]
    fn grapheme_form_keeps_variation_sequences() {
        // A lone stop is substituted...
        assert_eq!(vertical_grapheme("。"), Some('︒'));
        // ...but a cluster carrying an IVS is left as the author wrote it.
        assert_eq!(vertical_grapheme("葛\u{E0100}"), None);
    }
}
