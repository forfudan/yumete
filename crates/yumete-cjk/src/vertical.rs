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
/// ⚠️ **Zero, and it always was on the page.** This said 1 while `:view-dense`
/// was on out of the box and forced the gap to nothing, so the factory page
/// never had one and the constant described a page nobody saw. When the gap
/// stopped being one of `dense`'s five jobs (2026-09-16, [`Margin`]) the
/// constant had to start telling the truth, or the factory page would have
/// widened by a cell a 縱 on the day it changed name. A 縱 that carries
/// something in its margin still buys that cell for itself — see `place`.
pub const DEFAULT_ZONG_GAP: usize = 0;

/// Whether each 縱 — or each row, across — keeps the narrow lane beside it for
/// what is written *about* the text: readings, hung 句讀, 着重號, 平仄 and the
/// 稿紙 ticks (`:view-margin`, 2026-09-16).
///
/// **One question, four answers, both layouts.** It replaced `:view-dense`,
/// which was two answers to a question it did not name and meant different
/// things across and down: packed down the page it hid the margin, packed
/// across it *removed air* — and in each layout it covered a different pair of
/// the four. From tightest to loosest:
///
/// | | 縱書 | 橫排 |
/// | --- | --- | --- |
/// | `never` | no lane, whatever is written | no reading row, whatever is written |
/// | `dense` | a 縱 buys the lane if **it** carries something | a row buys the row above if **it** does |
/// | `loose` | every 縱 of a paragraph buys it if **any** of them does | every row of a line, likewise |
/// | `always` | every 縱 | a row of air above every row |
///
/// `dense` is the factory setting — the first framing, 「只對存在注釋
/// 的**視覺**縱出現」. `loose` is what the page did for 着重號 before, and the
/// reason given then still holds for anyone who prefers it: a paragraph does
/// not change width as it is scrolled through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Margin {
    /// Never kept. The margin's contents are suppressed, not switched off —
    /// `:ruby` and `:view-hanging` still say what was asked for.
    Never,
    /// Kept per visual 縱 / row that carries something.
    #[default]
    Dense,
    /// Kept for every 縱 / row of a paragraph if any of it carries something.
    Loose,
    /// Kept everywhere.
    Always,
}

impl Margin {
    /// All four, tightest first — the order `:view-margin` lists them in.
    pub const ALL: [Margin; 4] = [Margin::Never, Margin::Dense, Margin::Loose, Margin::Always];

    /// The word for it, as the config and the command spell it.
    pub fn name(self) -> &'static str {
        match self {
            Margin::Never => "never",
            Margin::Dense => "dense",
            Margin::Loose => "loose",
            Margin::Always => "always",
        }
    }

    /// Read the word back. **No synonyms**: `on`/`off` belonged to the old
    /// two-way switch and would say nothing about which of the middle two.
    pub fn parse(word: &str) -> Option<Margin> {
        Margin::ALL.into_iter().find(|m| m.name() == word.trim())
    }

    /// Whether the lane exists at all — what `:view-hanging` and `:view-meter`
    /// need before they have anywhere to draw.
    pub fn shown(self) -> bool {
        self != Margin::Never
    }
}

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
            // The Chinese names too: `:layout 竪排` is what a writer of this
            // editor's own manual would type, and it was a syntax error.
            "vertical" | "vert" | "v" | "竪排" | "豎排" | "直排" | "縱書" | "纵书" => {
                Some(Layout::Vertical)
            }
            "horizontal" | "horiz" | "h" | "橫排" | "横排" => Some(Layout::Horizontal),
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

/// Whether `c` is a mark that classical typesetting hangs beside the character
/// it follows, rather than giving it a square of its own.
///
/// 。，、？！：；and the quotation brackets. Set this way the text runs
/// unbroken down the 縱 and the 句讀 sit in the margin, which is how a 古籍 is
/// punctuated — and how a page of dialogue stops looking like it is half
/// whitespace.
///
/// Dashes and ellipses are deliberately **not** here: 「——」 and 「⋯⋯」 are
/// full-width rules that carry the line onward, and hanging them in a
/// half-width margin would break the very stroke that makes them read.
pub fn hangs_in_the_margin(c: char) -> bool {
    margin_form(c).is_some()
}

/// The **half-width** glyph a mark takes when it hangs in the margin.
///
/// A hung 句讀 is not a character in the text column any more — it is a small
/// mark written beside one, the way a 古籍 is punctuated. Unicode has narrow
/// forms for exactly this purpose (`｡` `､` `｢` `｣`), and the marks that have no
/// CJK narrow form have an ASCII twin that *is* the same mark: `，` and `,` are
/// both a low comma, `？` and `?` the same question.
///
/// Half-width is not a compromise here, it is the point. The margin is one cell
/// wide; a full-width `︒` in it spills onto the 縱 to the right and hides the
/// character there, and widening the margin to two cells would cost every 縱 a
/// column — undoing the very thing 標點旁置 is for.
///
/// `None` means the mark has no narrow form and so does not hang: it keeps its
/// square in the text column. 《》【】『』〔〕 are that set. They are rarer than
/// 。、「」 in prose, and a page whose margin changed width from 縱 to 縱 would
/// not be a margin.
///
/// # Examples
///
/// ```
/// use yumete_cjk::margin_form;
/// assert_eq!(margin_form('。'), Some('｡'));
/// assert_eq!(margin_form('「'), Some('｢'));
/// assert_eq!(margin_form('，'), Some(','));
/// assert_eq!(margin_form('《'), None);
/// ```
pub fn margin_form(c: char) -> Option<char> {
    // ⚠️ `or_else`, not `or`: the argument of `or` is **evaluated either way**,
    // and the `return None` inside this match would then return it out of
    // `margin_form` for 。 and 、 — the two marks that had just been answered.
    narrow_form(c).or_else(|| Some(match c {
        // The rest borrow their ASCII twin, which is the same mark drawn narrow.
        '，' => ',',
        '？' => '?',
        '！' => '!',
        '：' => ':',
        '；' => ';',
        '（' => '(',
        '）' => ')',
        _ => return None,
    }))
}

/// **The four CJK marks Unicode gives a真 narrow form** — Feature #230.
///
/// Not the same question as [`margin_form`], which will settle for the ASCII
/// twin: that is good enough for one mark hanging alone in a half-cell margin,
/// and **not** good enough where the narrow glyph lands in the writing itself.
/// 。 and 、 are half-em glyphs in print — the right half of the em is blank,
/// which is what lets a closing bracket nest into it — while ？ and ！ fill
/// theirs, and clreq §6.3.2 separates them from the 句號 group for exactly
/// that reason.
pub fn narrow_form(c: char) -> Option<char> {
    Some(match c {
        '。' => '｡',
        '、' => '､',
        '「' => '｢',
        '」' => '｣',
        _ => return None,
    })
}

/// Whether a hanging mark belongs beside the character *after* it rather than
/// the one before.
///
/// An opening bracket introduces what follows it, so that is the character it
/// hangs against; everything else — the stops, the commas, the closing brackets
/// — belongs to what came before. Getting this backwards puts 「 beside the word
/// that ends the sentence before the speech.
pub fn opens_a_pair(c: char) -> bool {
    matches!(c, '「' | '（')
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
    fn the_marks_that_hang_are_the_ones_with_a_narrow_form() {
        for c in [
            '。', '，', '、', '？', '！', '：', '；', '「', '」', '（', '）',
        ] {
            assert!(hangs_in_the_margin(c), "{c} should hang");
            let narrow = margin_form(c).expect("a hanging mark has a narrow form");
            assert_eq!(char_width(narrow), 1, "{c} → {narrow} must be one cell");
        }
        // No narrow form, so no hanging: they keep their square rather than
        // making the margin two cells wide for every 縱 on the page.
        for c in ['《', '》', '【', '】', '『', '』', '〔', '〕'] {
            assert!(!hangs_in_the_margin(c), "{c} has no narrow form");
        }
        // A dash or an ellipsis is a full-width rule carrying the line onward;
        // hung in a half-width margin it would break the stroke that reads.
        for c in ['—', '…', '⋯', '－'] {
            assert!(!hangs_in_the_margin(c), "{c} must keep its square");
        }
        // And nothing that is not punctuation.
        for c in ['字', 'a', '1', '　'] {
            assert!(!hangs_in_the_margin(c));
        }
    }

    #[test]
    fn openers_hang_forward_and_everything_else_back() {
        // Only the openers that hang at all: this set and the hanging set are
        // now the same question asked twice, so they cannot drift apart.
        for c in ['「', '（'] {
            assert!(opens_a_pair(c), "{c} introduces what follows it");
            assert!(hangs_in_the_margin(c));
        }
        for c in ['」', '）', '。', '，', '？'] {
            assert!(!opens_a_pair(c), "{c} belongs to what came before");
        }
        for c in ['『', '《', '【', '〔'] {
            assert!(!opens_a_pair(c), "{c} does not hang, so it opens nothing");
        }
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
        // ...but a cluster carrying an IVS is left exactly as it was written.
        assert_eq!(vertical_grapheme("葛\u{E0100}"), None);
    }
}
