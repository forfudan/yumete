//! **vim modelines** (2026-11-04): `vim: set ts=2 ft=python:` in the first or
//! last five lines of a file says how that file is to be read.
//!
//! Read exactly as vim and nvim read them (measured in nvim 0.12.5): both
//! forms, the `vi:` / `vim:` / `Vim:` / `ex:` prefixes, the version forms
//! against nvim's 801, `\:` for a colon in a value, and a line given up at
//! the first option vim would refuse. Only a fixed list of plain values is
//! **taken** — the file type, the tab stop, the indent width, whether Tab
//! types spaces, the text width — and nothing is ever run: vim's two
//! modeline CVEs (2016-1248, 2019-12735) came from values that were
//! evaluated. Everything else vim knows is passed over in silence; a novel
//! that says 「For ex: see」 must not cost its writer a message.

/// How many lines at each end are read, as vim's `modelines` default.
pub const LINES: usize = 5;

/// A line longer than this is not read: a minified one-line file is not
/// sliced looking for a modeline.
const LONGEST: usize = 4096;

/// nvim's version, as `vim{N}:` compares it.
const VERSION: u32 = 801;

/// What the modelines of a file say, as far as yumete takes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Modeline {
    /// `ft=` / `syntax=`, a syntax yumete has.
    pub syntax: Option<crate::syntax::Syntax>,
    /// `ts=`, 1 to 16.
    pub tab_stop: Option<usize>,
    /// `sw=`, 0 to 16; `0` follows the tab stop, whatever sets it.
    pub indent_width: Option<usize>,
    /// `et` / `noet`.
    pub tab_spaces: Option<bool>,
    /// `tw=`, 0 to 400.
    pub text_width: Option<usize>,
    /// What was taken and what was passed over, for `:info`.
    pub said: Vec<String>,
}

impl Modeline {
    /// Whether it says anything yumete takes.
    pub fn is_empty(&self) -> bool {
        self.syntax.is_none()
            && self.tab_stop.is_none()
            && self.indent_width.is_none()
            && self.tab_spaces.is_none()
            && self.text_width.is_none()
    }
}

/// Read the modelines of `lines` (all of a file's lines; only the first and
/// last [`LINES`] are looked at).
pub fn scan(lines: &[&str]) -> Modeline {
    let mut out = Modeline::default();
    let n = lines.len();
    let mut seen: Vec<usize> = (0..n.min(LINES)).collect();
    for i in n.saturating_sub(LINES)..n {
        if !seen.contains(&i) {
            seen.push(i);
        }
    }
    for i in seen {
        let line = lines[i].trim_end_matches('\r');
        if line.len() > LONGEST {
            continue;
        }
        if !read_line(line, &mut out) {
            break;
        }
    }
    out
}

/// Where the options start on `line`, and whether it is the `set` form.
fn options_of(line: &str) -> Option<(&str, bool)> {
    let bytes = line.as_bytes();
    let mut at = 0;
    while at < line.len() {
        let rest = &line[at..];
        let blank_before = at > 0 && (bytes[at - 1] == b' ' || bytes[at - 1] == b'\t');
        let found = prefix(rest, at == 0 || blank_before, blank_before);
        if let Some((len, capital)) = found {
            let after = &rest[len..];
            let trimmed = after.trim_start();
            let set = ["set ", "se "]
                .iter()
                .find_map(|word| trimmed.strip_prefix(word));
            match set {
                Some(options) => return Some((options, true)),
                // `Vim:` counts only in the `set` form.
                None if capital => {}
                None => return Some((after, false)),
            }
        }
        at += rest.chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// The length of a modeline prefix at the start of `rest`, with whether it
/// was `Vim:`; `None` when there is none. `vi:` and `vim:` may start the
/// line; `ex:` needs a blank before it.
fn prefix(rest: &str, may_start: bool, blank_before: bool) -> Option<(usize, bool)> {
    if blank_before && rest.starts_with("ex:") {
        return Some((3, false));
    }
    if !may_start {
        return None;
    }
    if rest.starts_with("vi:") {
        return Some((3, false));
    }
    let (tail, capital) = match rest.strip_prefix("vim") {
        Some(tail) => (tail, false),
        None => (rest.strip_prefix("Vim")?, true),
    };
    if tail.starts_with(':') {
        return Some((4, capital));
    }
    // `vim{N}:`, `vim<N:`, `vim=N:`, `vim>N:` against nvim's version.
    let (op, digits_from) = match tail.chars().next()? {
        c @ ('<' | '=' | '>') => (c, 1),
        c if c.is_ascii_digit() => ('>', 0),
        _ => return None,
    };
    let digits: String = tail[digits_from..].chars().take_while(char::is_ascii_digit).collect();
    let end = digits_from + digits.len();
    if digits.is_empty() || !tail[end..].starts_with(':') {
        return None;
    }
    let wanted: u32 = digits.parse().ok()?;
    let holds = match (op, digits_from) {
        ('<', _) => VERSION < wanted,
        ('=', _) => VERSION == wanted,
        (_, 1) => VERSION > wanted,
        _ => VERSION >= wanted,
    };
    holds.then_some((3 + end + 1, capital))
}

/// Read one line into `out`. `false` when it said `nomodeline`, which stops
/// the lines after it (the rest of its own line still counts, as in nvim).
fn read_line(line: &str, out: &mut Modeline) -> bool {
    let Some((options, set_form)) = options_of(line) else {
        return true;
    };
    // The options, split as vim splits them: by blanks, and in the first
    // form by colons too; `\:` is a colon in a value. The `set` form ends at
    // its first plain colon, and without one the line says nothing.
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut closed = false;
    let mut chars = options.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&':') => {
                word.push(':');
                chars.next();
            }
            ':' if set_form => {
                closed = true;
                break;
            }
            ' ' | '\t' | ':' => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if set_form && !closed {
        return true;
    }
    if !word.is_empty() {
        words.push(word);
    }
    let mut go_on = true;
    for word in words {
        let (name, value) = match word.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (word.as_str(), None),
        };
        match (name, value) {
            ("ml" | "modeline", None) => {}
            ("noml" | "nomodeline", None) => go_on = false,
            ("ft" | "filetype" | "syn" | "syntax", Some(value)) => {
                if let Some(syntax) = syntax_named(value) {
                    out.syntax = Some(syntax);
                    out.said.push(word.clone());
                }
            }
            ("ts" | "tabstop", Some(value)) => {
                if let Some(n) = number(value, 1, 16) {
                    out.tab_stop = Some(n);
                    out.said.push(word.clone());
                }
            }
            ("sw" | "shiftwidth", Some(value)) => {
                // `sw=0` is the tab stop; it is settled once the line is read.
                if let Some(n) = number(value, 0, 16) {
                    out.indent_width = Some(n);
                    out.said.push(word.clone());
                }
            }
            ("et" | "expandtab", None) => {
                out.tab_spaces = Some(true);
                out.said.push(word.clone());
            }
            ("noet" | "noexpandtab", None) => {
                out.tab_spaces = Some(false);
                out.said.push(word.clone());
            }
            ("tw" | "textwidth", Some(value)) => {
                if let Some(n) = number(value, 0, 400) {
                    out.text_width = Some(n);
                    out.said.push(word.clone());
                }
            }
            // Anything else is passed over, and the options after it still
            // count: yumete cannot tell an option vim refuses from one it
            // only does not use, and a modeline that reached a file almost
            // always says what it meant.
            _ => {}
        }
    }
    go_on
}

/// A number in `low..=high`, or nothing: out of range is ignored, never
/// clamped.
fn number(value: &str, low: usize, high: usize) -> Option<usize> {
    value.parse().ok().filter(|n| (low..=high).contains(n))
}

/// A file type by vim's name, when yumete has that syntax. Only plain names
/// are looked up (`[A-Za-z0-9._-]`); `a.b` is its first part yumete knows.
fn syntax_named(value: &str) -> Option<crate::syntax::Syntax> {
    if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return None;
    }
    value.split('.').find_map(|one| {
        let one = match one {
            "typescriptreact" => "tsx",
            "javascriptreact" => "javascript",
            other => other,
        };
        crate::syntax::Syntax::parse(one)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::Syntax;

    fn of(text: &str) -> Modeline {
        let lines: Vec<&str> = text.lines().collect();
        scan(&lines)
    }

    #[test]
    fn both_forms_and_their_prefixes() {
        let m = of("# vim: set ts=2 sw=2 et ft=python:\nbody\n");
        assert_eq!((m.tab_stop, m.indent_width, m.tab_spaces), (Some(2), Some(2), Some(true)));
        assert_eq!(m.syntax.map(Syntax::name), Some("python"));
        let m = of("/* vim: ts=4 noet */\n");
        assert_eq!((m.tab_stop, m.tab_spaces), (Some(4), Some(false)), "the first form, up to what vim refuses");
        assert_eq!(of("<!-- vim: set ft=html ts=2: -->\n").tab_stop, Some(2));
        assert_eq!(of("vi: ts=3\n").tab_stop, Some(3));
        assert_eq!(of(" ex: ts=5\n").tab_stop, Some(5), "ex: after a blank");
        assert_eq!(of("ex: ts=5\n").tab_stop, None, "ex: at the start is not one");
        assert_eq!(of("Vim: ts=6\n").tab_stop, None, "Vim: only in the set form");
        assert_eq!(of("Vim: set ts=6:\n").tab_stop, Some(6));
    }

    #[test]
    fn the_set_form_needs_its_closing_colon() {
        assert_eq!(of("# vim: set ts=2\n").tab_stop, None);
        assert_eq!(of("# vim: set ts=2: sw=7\n").indent_width, None, "after the colon is not read");
    }

    #[test]
    fn only_the_first_and_last_five_lines() {
        let mut text: Vec<String> = (0..12).map(|i| format!("line {i}")).collect();
        text[5] = "vim: ts=2".into();
        assert_eq!(of(&text.join("\n")).tab_stop, None, "line 6 of 12 is in neither end");
        text[5] = "line 5".into();
        text[11] = "vim: ts=3".into();
        assert_eq!(of(&text.join("\n")).tab_stop, Some(3));
    }

    #[test]
    fn versions_count_against_nvims() {
        assert_eq!(of("vim600: ts=2\n").tab_stop, Some(2));
        assert_eq!(of("vim900: ts=2\n").tab_stop, None);
        assert_eq!(of("vim<900: ts=2\n").tab_stop, Some(2));
        assert_eq!(of("vim=801: ts=2\n").tab_stop, Some(2));
        assert_eq!(of("Vim600: set ts=2:\n").tab_stop, Some(2), "Vim with a version, in the set form");
        assert_eq!(of("Vim600: ts=2\n").tab_stop, None);
    }

    #[test]
    fn nothing_is_taken_that_could_be_run_or_is_out_of_range() {
        let m = of("vim: set ft=../../x ts=40 tw=5000:\n");
        assert!(m.is_empty(), "{m:?}");
        assert_eq!(of("vim: set ft=xml:\n").syntax, None, "no XML here, and no stand-in");
        let m = of("vim: set fdm=marker spell ts=2:\n");
        assert_eq!(m.tab_stop, Some(2), "options vim knows are passed over");
        for line in ["vim: set fo+=t ts=2:", "vim: set cul ts=2:", "vim: set et! ts=2:", "vim: set foo=1 ts=2:"] {
            assert_eq!(of(line).tab_stop, Some(2), "{line}: what yumete does not use is passed over");
        }
        assert_eq!(of("vim: set sw=0 ts=4:\n").indent_width, Some(0), "sw=0 follows the tab stop");
        assert_eq!(of("vim: set ft=typescriptreact:\n").syntax.map(Syntax::name), Some("tsx"));
    }

    #[test]
    fn nomodeline_stops_the_lines_after_it() {
        assert_eq!(of("vim: set noml:\nvim: ts=2\n").tab_stop, None);
        assert_eq!(of("vim: set noml ts=3:\n").tab_stop, Some(3), "its own line still counts");
        assert!(of("For ex: see the notes\n").is_empty(), "prose is not a modeline");
    }
}
