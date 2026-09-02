//! `yumete-cjk` — CJK-aware text metrics for yumete.
//!
//! Two things that a CJK-first editor must get right, kept in one small,
//! dependency-light crate so the rest of yumete can rely on them:
//!
//! - **Display width** ([`width`], Feature #16): how many terminal cells a
//!   character or string occupies. Han characters, kana, and most fullwidth
//!   forms take two cells; combining marks take zero. The editor uses this for
//!   cursor columns, alignment, and soft-wrap, never assuming one character is
//!   one cell.
//! - **Grapheme clusters** ([`grapheme`], Feature #17): where the user-perceived
//!   "characters" actually begin and end. Combining sequences, ideographic
//!   variation sequences (IVS), and emoji ZWJ sequences are single graphemes, so
//!   cursor motion and deletion move over them as a unit.
//!
//! ## Reusing the ecosystem, not reinventing it
//!
//! The underlying algorithms come from mature crates — the same ones Helix uses:
//! [`unicode-width`](https://docs.rs/unicode-width) (Unicode Annex #11) for
//! width, and [`unicode-segmentation`](https://docs.rs/unicode-segmentation)
//! (Unicode Annex #29) for grapheme clusters. This crate only adds thin,
//! editor-specific helpers on top: [`grapheme_width`](width::grapheme_width) and
//! [`tab_width_at`](width::tab_width_at). Rope-aware cursor navigation over large
//! files will use `unicode-segmentation`'s incremental `GraphemeCursor` directly
//! over the rope's chunks in `yumete-core`, rather than materializing whole lines.
//!
//! ## CJK in the terminal (Feature #18)
//!
//! yumete guarantees correct *width and motion math* regardless of the host
//! terminal. Whether a given glyph actually *renders* — and which fallback font
//! is used for rare Han characters, IVS, or PUA roots — depends on the terminal
//! emulator and its font configuration, which yumete cannot control. For the
//! Yuhao schemes, configure the terminal's font (or its fallback chain) to
//! include a wide-coverage CJK font such as the bundled `Yuniversus.ttf`.

pub mod blocks;
pub mod grapheme;
pub mod segment;
pub mod vertical;
pub mod width;
pub mod word;

pub use grapheme::{
    grapheme_count, graphemes, next_grapheme_boundary, nth_grapheme, prev_grapheme_boundary,
};
pub use segment::{CategorySegmenter, DictionarySegmenter, Segmenter};
pub use vertical::{
    hangs_in_the_margin, margin_form, opens_a_pair, vertical_form, vertical_grapheme, Layout,
    DEFAULT_ZONG_GAP, DEFAULT_ZONG_LENGTH,
};
pub use width::{
    ambiguous_is_wide, char_width, grapheme_width, set_ambiguous_wide, str_width, tab_width_at,
};
pub use word::{word_ranges, word_ranges_big};
