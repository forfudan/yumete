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
        let (base, reading) = (self.escape(base), self.escape(reading));
        format!("{open}{base}{mid}{reading}{close}")
    }

    /// The text as it has to be written **inside** this dialect's markup.
    ///
    /// Typst's call takes two **string literals**, where `"` ends the string
    /// and `\` opens an escape; HTML's tags hold text and take it as it is.
    /// So `say "hi"` is written `"say \"hi\""` there and `say "hi"` here, and
    /// before #332 it was written the same both ways — which compiled in
    /// neither direction and read back cut in the wrong place.
    ///
    /// ⚠️ **This is the string literal's escaping, not Typst's markup
    /// escaping.** Inside `"…"` a `*` is a star and `\*` is an error; outside,
    /// the reverse. `export.rs` had the markup one here, so a bold base came
    /// out as `#ruby("\*永和\*", …)` and the export did not compile.
    pub fn escape(self, text: &str) -> String {
        match self {
            Dialect::Html => text.to_string(),
            Dialect::Typst => {
                let mut out = String::with_capacity(text.len());
                for c in text.chars() {
                    if matches!(c, '\\' | '"') {
                        out.push('\\');
                    }
                    out.push(c);
                }
                out
            }
        }
    }

    /// The text a group holds, with this dialect's escaping taken back off —
    /// what to hand to anything that is not writing this dialect again.
    ///
    /// **Only `\"` and `\\`.** Typst knows `\n` and `\u{…}` too, and this
    /// leaves them exactly as written: turning `\n` into a newline would put a
    /// line break inside a reading that [`Dialect::escape`] cannot write back,
    /// and a round trip that does not return what it was given is worse than
    /// one that does nothing.
    pub fn unescape(self, text: &str) -> String {
        if self == Dialect::Html || !text.contains('\\') {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, chars.peek()) {
                ('\\', Some('"' | '\\')) => out.push(chars.next().expect("peeked")),
                _ => out.push(c),
            }
        }
        out
    }

    /// Every group of the **next element** of this dialect at or after `from`.
    ///
    /// A list, because one `<ruby>` may hold more than one: 熟語振假名 writes a
    /// compound with a reading per character —
    /// `<ruby>漢<rt>かん</rt>字<rt>じ</rt></ruby>` — and that is the shape the
    /// standard recommends for exactly the words a Japanese manuscript is
    /// full of. Read as one base and one reading, the second character and
    /// the tags around it were swallowed into the first's reading, and
    /// `:ruby-format typst` wrote `#ruby("漢", "かん</rt>字<rt>じ")`: 字 stopped
    /// being text at all (#330).
    fn element(self, chars: &[char], from: usize) -> Vec<Ruby> {
        match self {
            Dialect::Html => self.html_element(chars, from),
            Dialect::Typst => self.call(chars, from).into_iter().collect(),
        }
    }

    /// One `<ruby>…</ruby>`, as the pairs inside it.
    ///
    /// **The tags are matched loosely and the text strictly.** `<RUBY>`,
    /// `<ruby lang="ja">` and the `<rp>(</rp>` fallback the W3C recommends are
    /// all ordinary HTML that a manuscript may well arrive with, and a reader
    /// that cannot see them leaves them in the file while reporting that the
    /// file was converted (#331). What it will not do is guess: a `<ruby>`
    /// inside a `<ruby>`, or one that never closes, yields nothing at all
    /// rather than a group with somebody else's text in it.
    fn html_element(self, chars: &[char], from: usize) -> Vec<Ruby> {
        let mut at = from;
        while let Some(open) = tag(chars, "ruby", at) {
            let Some(close) = tag(chars, "/ruby", open.1) else {
                return Vec::new();
            };
            // A second `<ruby>` before this one closes is markup nobody can
            // read; start again there, so it costs only itself.
            if let Some(inner) = tag(chars, "ruby", open.1) {
                if inner.0 < close.0 {
                    at = inner.0;
                    continue;
                }
            }
            let mut out: Vec<Ruby> = Vec::new();
            let mut i = open.1;
            let mut base = open.1;
            let mut began = open.0;
            while i < close.0 {
                let Some(rt) = tag(chars, "rt", i) else { break };
                if rt.0 >= close.0 {
                    break;
                }
                let Some(shut) = tag(chars, "/rt", rt.1) else {
                    return Vec::new();
                };
                // **The base ends where the annotation begins**, and that may
                // be an `<rp>` rather than the `<rt>`: the parenthesis a
                // reader without ruby support falls back to is neither base
                // nor reading, and dropping it is the whole of reading it.
                let ends = match tag(chars, "rp", i) {
                    Some(rp) if rp.0 < rt.0 => rp.0,
                    _ => rt.0,
                };
                out.push(Ruby {
                    dialect: self,
                    start: began,
                    end: shut.1,
                    base: (base, ends),
                    reading: (rt.1, shut.0),
                });
                // …and the closing parenthesis, if the writer left one.
                let mut after = shut.1;
                if let Some(rp) = tag(chars, "rp", after) {
                    if rp.0 == after {
                        if let Some(shut) = tag(chars, "/rp", rp.1) {
                            after = shut.1;
                        }
                    }
                }
                i = after;
                began = after;
                base = after;
            }
            match out.last_mut() {
                // The last pair carries the closing tag, so that everything
                // between the bases is off the page and nothing is left over.
                Some(last) => {
                    last.end = close.1;
                    return out;
                }
                None => at = open.1,
            }
        }
        Vec::new()
    }

    /// One `#ruby("base", "reading")`, which is a function call and is read
    /// as one.
    ///
    /// **Read as two string literals, not as two runs of text between fixed
    /// tags** (#332): `"` closes a string unless a `\` opens it, so
    /// `#ruby("桜", "say \"hi\"")` is one group and not a base of `桜` with
    /// the reading cut at the first inner quote.
    ///
    /// Loose in one place, exactly as the HTML reader is: any number of spaces
    /// after the comma, so a call written by hand parses too.
    fn call(self, chars: &[char], from: usize) -> Option<Ruby> {
        let (open, _, _) = self.parts();
        let mut i = from;
        while i < chars.len() {
            let start = find(chars, open, i)?;
            let base_start = start + open.chars().count();
            // An unterminated string is not a group. Starting again *inside*
            // it is what finds the next call — including one opened before
            // this one closed, which is why that case needs no arm of its own.
            let parsed = (|| {
                let base_end = string_end(chars, base_start)?;
                let mut at = base_end + 1;
                if chars.get(at) != Some(&',') {
                    return None;
                }
                at += 1;
                while chars.get(at) == Some(&' ') {
                    at += 1;
                }
                if chars.get(at) != Some(&'"') {
                    return None;
                }
                let reading_start = at + 1;
                let reading_end = string_end(chars, reading_start)?;
                if chars.get(reading_end + 1) != Some(&')') {
                    return None;
                }
                Some(Ruby {
                    dialect: self,
                    start,
                    end: reading_end + 2,
                    base: (base_start, base_end),
                    reading: (reading_start, reading_end),
                })
            })();
            match parsed {
                Some(group) => return Some(group),
                None => i = base_start,
            }
        }
        None
    }

}

/// Where the string opened at `from` closes: the index of its `"`.
///
/// `\` escapes the next character, so `\"` is a quote in the string and not
/// the end of it. A string that reaches the end of the line never closes — a
/// call is written on one line, and swallowing the next paragraph looking for
/// a quote is how a missing one turns into a lost page.
fn string_end(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '\n' => return None,
            '\\' => i += 2,
            '"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Where the HTML tag `name` opens and closes, at or after `from`: the index
/// of its `<` and one past its `>`.
///
/// **Loose about the tag and exact about the name** (#331): the case is
/// ignored, because `<RUBY>` is the same element; attributes are allowed,
/// because `<ruby lang="ja">` is ordinary and a reader that cannot see it
/// leaves it in the file while reporting the file converted; and the name has
/// to end where the tag does, so `<rtc>` is not an `<rt>`.
fn tag(chars: &[char], name: &str, from: usize) -> Option<(usize, usize)> {
    let want: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    let mut i = from;
    while i < chars.len() {
        if chars[i] != '<' {
            i += 1;
            continue;
        }
        let after = i + 1;
        let same = want
            .iter()
            .enumerate()
            .all(|(k, w)| chars.get(after + k).is_some_and(|c| c.to_ascii_lowercase() == *w));
        let ends = chars.get(after + want.len());
        if same && matches!(ends, Some('>' | ' ' | '\t' | '/')) {
            let close = (after + want.len()..chars.len()).find(|&k| chars[k] == '>')?;
            return Some((i, close + 1));
        }
        i += 1;
    }
    None
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

    /// The set as a number, for a cache key.
    pub fn bits(self) -> u8 {
        self.0
    }

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
        // Whole elements, because one of them may hold several groups.
        let Some(next) = dialects
            .iter()
            .map(|d| d.element(chars, at))
            .filter(|e| !e.is_empty())
            .min_by_key(|e| e[0].start)
        else {
            break;
        };
        at = next.last().map_or(at + 1, |g| g.end);
        found.extend(next);
    }
    found
}

/// How many annotations of **another** dialect are still in `text` that this
/// module could not read (#331).
///
/// Asked after a rewrite, so that 「已改寫為 typst」 is only said about a file
/// that really is. What is counted is an opening tag of the other dialect that
/// no group covers: a `<ruby>` inside a `<ruby>`, one that never closes, an
/// `<rtc>`'s second annotation. Those are left exactly as they stand — the
/// right answer, since guessing at them is how #330 wrote 字 into a reading —
/// and this is the sentence that says so.
pub fn unread(text: &str, into: Dialect) -> usize {
    let other = match into {
        Dialect::Html => Dialect::Typst,
        Dialect::Typst => Dialect::Html,
    };
    let chars: Vec<char> = text.chars().collect();
    let read = groups(&chars, Dialects::only(other));
    let opens = match other {
        Dialect::Html => {
            let mut at = 0;
            let mut found = Vec::new();
            while let Some((start, after)) = tag(&chars, "ruby", at) {
                found.push(start);
                at = after;
            }
            found
        }
        Dialect::Typst => {
            let mut at = 0;
            let mut found = Vec::new();
            while let Some(start) = find(&chars, "#ruby(\"", at) {
                found.push(start);
                at = start + 1;
            }
            found
        }
    };
    opens
        .into_iter()
        .filter(|&at| !read.iter().any(|g| g.start <= at && at < g.end))
        .count()
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
        // Off with the dialect it was written in, on with the one it is going
        // into: `#ruby("say \"hi\"", …)` becomes `<ruby>say "hi"<rt>…`, and
        // back again (#332).
        let base: String = group.base_text(&chars).iter().collect();
        let reading: String = group.reading_text(&chars).iter().collect();
        let (base, reading) = (group.dialect.unescape(&base), group.dialect.unescape(&reading));
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

    /// #332: a quote in an English annotation used to write a Typst file
    /// that did not compile, and read back cut in the wrong place.
    #[test]
    fn a_typst_call_holds_a_quote_as_a_string_literal() {
        let written = Dialect::Typst.write("桜", r#"say "hi""#);
        assert_eq!(written, r#"#ruby("桜", "say \"hi\"")"#);
        // …and reads back as the one group it is, not as two strings cut at
        // the first inner quote.
        let chars: Vec<char> = written.chars().collect();
        let found = all_groups(&chars);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].end, chars.len(), "the whole call is the group");
        let reading: String = found[0].reading_text(&chars).iter().collect();
        assert_eq!(reading, r#"say \"hi\""#, "as written");
        assert_eq!(
            Dialect::Typst.unescape(&reading),
            r#"say "hi""#,
            "as it reads"
        );
        // A quote in the *base* was the other half of it.
        let written = Dialect::Typst.write(r#""x""#, "くお");
        assert_eq!(written, r#"#ruby("\"x\"", "くお")"#);
        let chars: Vec<char> = written.chars().collect();
        assert_eq!(all_groups(&chars).len(), 1);

        // A backslash of the writer's own survives the round trip whole.
        let round = Dialect::Typst.write(r"C:\書", "みち");
        assert_eq!(round, r#"#ruby("C:\\書", "みち")"#);
        let chars: Vec<char> = round.chars().collect();
        let base: String = all_groups(&chars)[0].base_text(&chars).iter().collect();
        assert_eq!(Dialect::Typst.unescape(&base), r"C:\書");
        // What Typst knows and we do not is left exactly as written, rather
        // than becoming a newline nothing can write back.
        assert_eq!(Dialect::Typst.unescape(r"a\nb"), r"a\nb");

        // Between dialects, off with one and on with the other.
        let html = r#"<ruby>桜<rt>say "hi"</rt></ruby>"#;
        let typst = r#"#ruby("桜", "say \"hi\"")"#;
        assert_eq!(reformat(html, Dialect::Typst).as_deref(), Some(typst));
        assert_eq!(reformat(typst, Dialect::Html).as_deref(), Some(html));

        // An unterminated string is not a group, and does not eat the line
        // after it looking for its quote.
        let chars: Vec<char> = "#ruby(\"漢\", \"hàn)\n下一行\n".chars().collect();
        assert!(all_groups(&chars).is_empty());
        // Spaces after the comma are the writer's business.
        let chars: Vec<char> = r#"#ruby("漢","hàn")"#.chars().collect();
        assert_eq!(all_groups(&chars).len(), 1, "written by hand, no space");
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
