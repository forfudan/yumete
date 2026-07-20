//! The [`TextStore`] trait: an abstraction over the concrete text-storage
//! backend (currently a [`ropey`](https://docs.rs/ropey) rope).
//!
//! Keeping the editor core behind this trait — rather than reaching for the
//! rope directly everywhere — lets us swap the implementation later (a
//! different rope, a gap buffer, a vendored Helix core) without rewriting call
//! sites, as laid out in the yumete plan (§2, "decouple aggressively").

/// A read-only, line-addressed view over a document's text.
pub trait TextStore {
    /// The total number of lines.
    ///
    /// Following common editor semantics, an empty document still has one
    /// (empty) line, so this never returns `0`.
    fn line_count(&self) -> usize;

    /// The total number of Unicode scalar values (`char`s) in the document.
    ///
    /// Note this counts `char`s, not display cells or grapheme clusters; the
    /// CJK-aware width and grapheme layer (Feature #16 / #17) will live in the
    /// `yumete-cjk` crate.
    fn char_count(&self) -> usize;

    /// The text of a single 0-based line, without its trailing line break, or
    /// `None` if `index` is out of range.
    fn line(&self, index: usize) -> Option<String>;

    /// The entire document as one owned `String`.
    fn text(&self) -> String;
}
