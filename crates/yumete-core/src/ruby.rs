//! 振假名 / 注音 markup — Feature #65.
//!
//! The buffer is a plain text file, so a reading has to be written into it — and
//! *how* is a property of the document, not of yumete. A Markdown file wants
//! HTML ruby, which is what the Yuhao documentation is already written in and
//! what a browser renders unchanged; a Typst file wants Typst's call, which its
//! own compiler will typeset. So the markup is a [`Dialect`], the editor renders
//! whichever ones a file uses — several at once, if it mixes them — and
//! [`markup`] writes the one it is told to.
//!
//! Nothing here decides how a reading is *drawn* — see [`crate::zong`], which
//! spaces the base characters out to make room for it. This module only answers
//! "where in this line are the ruby groups, and what is in them".

/// A way of writing a reading into the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// `<ruby>漢字<rt>hàn zì</rt></ruby>` — HTML, and therefore Markdown.
    Html,
    /// `#ruby("漢字", "hàn zì")` — Typst, which compiles it straight to type.
    Typst,
}

impl Dialect {
    /// Every dialect, in the order they are searched.
    pub const ALL: [Dialect; 2] = [Dialect::Html, Dialect::Typst];

    /// The name used in commands and config (`html`, `typst`).
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Html => "html",
            Dialect::Typst => "typst",
        }
    }

    /// Parse a dialect name, as a `:render-ruby-…` command or a config value.
    pub fn parse_name(value: &str) -> Option<Dialect> {
        match value.trim().to_ascii_lowercase().as_str() {
            "html" | "markdown" | "md" => Some(Dialect::Html),
            "typst" | "typ" => Some(Dialect::Typst),
            _ => None,
        }
    }

    /// The dialect a file with this extension is written in.
    ///
    /// This is the whole of yumete's file-type knowledge for now. When a real
    /// language binding arrives it subsumes this table rather than sitting
    /// beside it.
    pub fn for_extension(ext: &str) -> Option<Dialect> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "md" | "markdown" | "html" | "htm" | "txt" => Some(Dialect::Html),
            "typ" | "typst" => Some(Dialect::Typst),
            _ => None,
        }
    }

    /// The tags this dialect brackets a group with: opening, the separator
    /// between base and reading, and the closing.
    fn parts(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Dialect::Html => ("<ruby>", "<rt>", "</rt></ruby>"),
            Dialect::Typst => ("#ruby(\"", "\", \"", "\")"),
        }
    }

    /// Write `base` read as `reading` in this dialect.
    pub fn write(self, base: &str, reading: &str) -> String {
        let (open, mid, close) = self.parts();
        format!("{open}{base}{mid}{reading}{close}")
    }

    /// The first group of this dialect at or after `from`.
    fn next(self, chars: &[char], from: usize) -> Option<Ruby> {
        let (open, mid, close) = self.parts();
        let mut i = from;
        while i < chars.len() {
            let start = find(chars, open, i)?;
            let base_start = start + open.chars().count();
            // A second opening tag before the separator means the first was
            // never closed; restart there rather than letting the base swallow
            // it, so unterminated markup costs only itself.
            if let Some(inner) = find(chars, open, base_start) {
                if find(chars, mid, base_start).is_none_or(|m| inner < m) {
                    i = inner;
                    continue;
                }
            }
            let parsed = find(chars, mid, base_start).and_then(|m| {
                let reading_start = m + mid.chars().count();
                let end = find(chars, close, reading_start)?;
                Some(Ruby {
                    dialect: self,
                    start,
                    end: end + close.chars().count(),
                    base: (base_start, m),
                    reading: (reading_start, end),
                })
            });
            match parsed {
                Some(group) => return Some(group),
                None => i = base_start,
            }
        }
        None
    }
}

/// The set of dialects being rendered.
///
/// A set rather than a choice, because a document may mix them and because
/// turning rendering off is the same as rendering none — one value covers
/// `:ruby-off`, one dialect, and several at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Dialects(pub(crate) u8);

impl Dialects {
    /// No dialects: the markup is shown as the text it is.
    pub const NONE: Dialects = Dialects(0);

    /// Just this one.
    pub fn only(dialect: Dialect) -> Dialects {
        let mut set = Dialects::NONE;
        set.insert(dialect);
        set
    }

    fn bit(dialect: Dialect) -> u8 {
        1 << (dialect as u8)
    }

    pub fn contains(self, dialect: Dialect) -> bool {
        self.0 & Dialects::bit(dialect) != 0
    }

    pub fn insert(&mut self, dialect: Dialect) {
        self.0 |= Dialects::bit(dialect);
    }

    pub fn remove(&mut self, dialect: Dialect) {
        self.0 &= !Dialects::bit(dialect);
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The dialects in the set, in [`Dialect::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = Dialect> {
        Dialect::ALL.into_iter().filter(move |&d| self.contains(d))
    }

    /// The one a new reading is written in: the first enabled, else HTML.
    pub fn writer(self) -> Dialect {
        self.iter().next().unwrap_or(Dialect::Html)
    }
}

/// One ruby group, in **character** offsets within a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ruby {
    /// Which dialect wrote it.
    pub dialect: Dialect,
    /// Where `<ruby>` starts — the first character the group covers.
    pub start: usize,
    /// One past `</ruby>` — the first character after the group.
    pub end: usize,
    /// The base text: what is actually read.
    pub base: (usize, usize),
    /// The reading, annotated beside the base.
    pub reading: (usize, usize),
}

impl Ruby {
    /// The base text of this group, given the line it came from.
    pub fn base_text<'a>(&self, chars: &'a [char]) -> &'a [char] {
        &chars[self.base.0..self.base.1]
    }

    /// The reading, given the line it came from.
    pub fn reading_text<'a>(&self, chars: &'a [char]) -> &'a [char] {
        &chars[self.reading.0..self.reading.1]
    }
}

/// Every well-formed group of any enabled dialect, in order and non-overlapping.
///
/// A group missing any of its tags is not a group — the text is left exactly as
/// written, so half-typed markup shows as the characters it is rather than
/// swallowing the rest of the paragraph.
pub fn groups(chars: &[char], dialects: Dialects) -> Vec<Ruby> {
    let mut found = Vec::new();
    let mut at = 0usize;
    while at < chars.len() {
        // Whichever dialect matches earliest from here wins the next group; a
        // file that mixes them is read in document order, not dialect order.
        let Some(next) = dialects
            .iter()
            .filter_map(|d| d.next(chars, at))
            .min_by_key(|g| g.start)
        else {
            break;
        };
        at = next.end;
        found.push(next);
    }
    found
}

/// The group covering `col`, if the cursor is inside one.
///
/// Searched across *every* dialect, not just the rendered ones: an annotation
/// can be edited whether or not the file's own dialect is being laid out.
pub fn group_at(chars: &[char], col: usize) -> Option<Ruby> {
    all_groups(chars)
        .into_iter()
        .find(|g| g.start <= col && col < g.end)
}

/// Every group in `chars`, whatever dialect wrote it.
pub fn all_groups(chars: &[char]) -> Vec<Ruby> {
    let mut every = Dialects::NONE;
    for d in Dialect::ALL {
        every.insert(d);
    }
    groups(chars, every)
}

/// The separator that splits one reading into per-character readings.
///
/// HTML ruby expresses both granularities already — one `<ruby>` over a word, or
/// one per character sitting side by side — so the only open question is which
/// the *command* writes. A bar says it outright: `hàn|zì` over 漢字 makes two
/// groups, `hàn zì` makes one. A space cannot do that job, because a space is a
/// legitimate part of a reading.
pub const SPLIT: char = '|';

/// Write the markup for `base` read as `reading`.
///
/// A reading split by [`SPLIT`] into exactly as many parts as `base` has
/// characters becomes one group per character (mono-ruby); anything else stays a
/// single group over the whole base.
pub fn markup(base: &[char], reading: &str, dialect: Dialect) -> String {
    let parts: Vec<&str> = reading.split(SPLIT).collect();
    if parts.len() > 1 && parts.len() == base.len() {
        return base
            .iter()
            .zip(parts)
            .map(|(c, r)| dialect.write(&c.to_string(), r))
            .collect();
    }
    let base: String = base.iter().collect();
    dialect.write(&base, reading)
}

/// Rewrite every group in `text` into `dialect`, whatever it was written in.
///
/// Returns `None` when nothing would change, so a no-op `:format-ruby-…` does
/// not push an undo step.
pub fn reformat(text: &str, dialect: Dialect) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let groups = all_groups(&chars);
    if groups.iter().all(|g| g.dialect == dialect) {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    for group in &groups {
        out.extend(&chars[at..group.start]);
        let base: String = group.base_text(&chars).iter().collect();
        let reading: String = group.reading_text(&chars).iter().collect();
        out.push_str(&dialect.write(&base, &reading));
        at = group.end;
    }
    out.extend(&chars[at..]);
    Some(out)
}

/// The char offset of `needle` in `chars` at or after `from`.
fn find(chars: &[char], needle: &str, from: usize) -> Option<usize> {
    let len = needle.chars().count();
    (from..=chars.len().checked_sub(len)?).find(|&i| starts_with(chars, needle, i))
}

/// Whether `chars` reads `needle` starting at `at`.
fn starts_with(chars: &[char], needle: &str, at: usize) -> bool {
    let mut wanted = needle.chars();
    let mut got = chars[at.min(chars.len())..].iter().copied();
    loop {
        match (wanted.next(), got.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(a), Some(b)) if a != b => return false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Most of these read HTML, the dialect the Yuhao documents use.
    const HTML: Dialects = Dialects(1 << (Dialect::Html as u8));

    fn parse(text: &str) -> Vec<(String, String)> {
        let chars: Vec<char> = text.chars().collect();
        groups(&chars, HTML)
            .iter()
            .map(|g| {
                (
                    g.base_text(&chars).iter().collect(),
                    g.reading_text(&chars).iter().collect(),
                )
            })
            .collect()
    }

    #[test]
    fn reads_a_single_group() {
        assert_eq!(
            parse("他說<ruby>口<rt>kǒu</rt></ruby>很難"),
            [("口".to_string(), "kǒu".to_string())]
        );
    }

    #[test]
    fn reads_several_groups_and_multi_character_bases() {
        assert_eq!(
            parse("<ruby>漢字<rt>hàn zì</rt></ruby>和<ruby>囗<rt>wéi</rt></ruby>"),
            [
                ("漢字".to_string(), "hàn zì".to_string()),
                ("囗".to_string(), "wéi".to_string()),
            ]
        );
    }

    #[test]
    fn spans_cover_the_whole_group() {
        let text = "甲<ruby>口<rt>kǒu</rt></ruby>乙";
        let chars: Vec<char> = text.chars().collect();
        let g = &groups(&chars, HTML)[0];
        assert_eq!(g.start, 1, "starts at `<`");
        assert_eq!(chars[g.end], '乙', "ends past `</ruby>`");
    }

    /// Half-typed markup is just text — it must not swallow the paragraph.
    #[test]
    fn incomplete_markup_is_not_a_group() {
        assert!(parse("<ruby>口<rt>kǒu</rt>").is_empty(), "no closing tag");
        assert!(parse("<ruby>口</ruby>").is_empty(), "no reading");
        assert!(parse("<ruby>口<rt>kǒu").is_empty(), "unterminated reading");
        assert!(parse("口<rt>kǒu</rt></ruby>").is_empty(), "no opening tag");
        // …and a later, complete group is still found after a broken one.
        assert_eq!(
            parse("<ruby>甲<ruby>口<rt>kǒu</rt></ruby>"),
            [("口".to_string(), "kǒu".to_string())]
        );
    }

    #[test]
    fn an_empty_reading_is_still_a_group() {
        assert_eq!(
            parse("<ruby>口<rt></rt></ruby>"),
            [("口".to_string(), String::new())]
        );
    }

    #[test]
    fn finds_the_group_under_the_cursor() {
        let chars: Vec<char> = "甲<ruby>口<rt>kǒu</rt></ruby>乙".chars().collect();
        assert!(group_at(&chars, 0).is_none(), "before the group");
        assert!(group_at(&chars, 1).is_some(), "on the opening tag");
        assert!(group_at(&chars, 7).is_some(), "on the base");
        assert!(group_at(&chars, chars.len() - 1).is_none(), "after it");
    }

    #[test]
    fn a_bar_splits_a_reading_across_the_characters() {
        let base: Vec<char> = "漢字".chars().collect();
        // One reading over the word.
        assert_eq!(
            markup(&base, "hàn zì", Dialect::Html),
            "<ruby>漢字<rt>hàn zì</rt></ruby>"
        );
        // One per character.
        assert_eq!(
            markup(&base, "hàn|zì", Dialect::Html),
            "<ruby>漢<rt>hàn</rt></ruby><ruby>字<rt>zì</rt></ruby>"
        );
        // A count that does not line up stays one group rather than guessing.
        assert_eq!(
            markup(&base, "hàn|zì|le", Dialect::Html),
            "<ruby>漢字<rt>hàn|zì|le</rt></ruby>"
        );
    }

    #[test]
    fn reads_typst_readings_too() {
        let text = "讀#ruby(\"漢字\", \"hàn zì\")了";
        let chars: Vec<char> = text.chars().collect();
        let found = all_groups(&chars);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].dialect, Dialect::Typst);
        assert_eq!(
            found[0].base_text(&chars).iter().collect::<String>(),
            "漢字"
        );
        assert_eq!(
            found[0].reading_text(&chars).iter().collect::<String>(),
            "hàn zì"
        );
    }

    #[test]
    fn a_mixed_document_is_read_in_document_order() {
        let text = "<ruby>甲<rt>a</rt></ruby>#ruby(\"乙\", \"b\")<ruby>丙<rt>c</rt></ruby>";
        let chars: Vec<char> = text.chars().collect();
        let readings: Vec<String> = all_groups(&chars)
            .iter()
            .map(|g| g.reading_text(&chars).iter().collect())
            .collect();
        assert_eq!(readings, ["a", "b", "c"], "not grouped by dialect");
    }

    /// A dialect that is not being rendered is not found — that is how
    /// `:render-ruby-typst` changes what the page shows.
    #[test]
    fn only_enabled_dialects_are_read() {
        let chars: Vec<char> = "#ruby(\"漢\", \"hàn\")".chars().collect();
        assert!(groups(&chars, HTML).is_empty());
        assert_eq!(groups(&chars, Dialects::only(Dialect::Typst)).len(), 1);
        assert!(groups(&chars, Dialects::NONE).is_empty());
    }

    #[test]
    fn reformat_rewrites_between_dialects() {
        let html = "讀<ruby>漢<rt>hàn</rt></ruby>";
        let typst = "讀#ruby(\"漢\", \"hàn\")";
        assert_eq!(reformat(html, Dialect::Typst).as_deref(), Some(typst));
        assert_eq!(reformat(typst, Dialect::Html).as_deref(), Some(html));
        // Nothing to change reports nothing to change.
        assert_eq!(reformat(html, Dialect::Html), None);
        assert_eq!(reformat("no ruby here", Dialect::Html), None);
    }

    #[test]
    fn a_dialect_is_guessed_from_the_extension() {
        assert_eq!(Dialect::for_extension("md"), Some(Dialect::Html));
        assert_eq!(Dialect::for_extension(".typ"), Some(Dialect::Typst));
        assert_eq!(Dialect::for_extension("rs"), None);
    }

    #[test]
    fn plain_text_has_no_groups() {
        assert!(parse("那年冬天，雪下得比往常都早。").is_empty());
    }
}
