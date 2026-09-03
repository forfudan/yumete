//! Which markup language a file is written in — Feature #106.
//!
//! A `.md` is Markdown and a `.typ` is Typst; the question is only about the
//! files that say nothing, which in practice means `.txt`. A novel written in
//! Typst but filed as `.txt` is an ordinary thing to have, and reading it as
//! Markdown makes `#import` look like a heading and `*粗*` look like nothing.
//!
//! The two languages are easy to tell apart because their signals are
//! *disjoint*, not merely different:
//!
//! - `#let` `#import` `#show` `#set` open Typst code. Markdown has no such
//!   thing — in Markdown they are headings, but a heading's `#` is followed by
//!   a space, and Typst's is followed by an identifier. The two spellings
//!   cannot both be right.
//! - `**粗**` is Markdown's bold; Typst's is a single `*粗*`, and a doubled one
//!   there is an empty strong plus a stray asterisk.
//! - `= 第一章` at the head of a line is a Typst heading. Markdown has no use
//!   for it.
//!
//! So this counts votes rather than parsing, and **a tie goes to Markdown**:
//! guessing wrong is worse than not guessing, because in 所見即所得 the guess
//! decides what comes off the page.

/// The markup a file is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Syntax {
    /// Markdown, and the extensions a manuscript uses.
    #[default]
    Markdown,
    /// Typst.
    Typst,
    /// No markup at all: the file is writing and nothing in it means anything
    /// else.
    ///
    /// A manuscript that uses `*` for a scene break and `#` for a note to
    /// self is not Markdown, and colouring it as Markdown makes the writer's
    /// own punctuation flicker at them. `:syntax text` says so.
    Text,
}

impl Syntax {
    /// Parse a `:syntax` argument or a config value.
    pub fn parse(name: &str) -> Option<Syntax> {
        match name.trim().to_ascii_lowercase().as_str() {
            "markdown" | "md" => Some(Syntax::Markdown),
            "typst" | "typ" => Some(Syntax::Typst),
            "text" | "txt" | "raw" | "none" | "plain" => Some(Syntax::Text),
            _ => None,
        }
    }

    /// Its name, for the status line.
    pub fn name(self) -> &'static str {
        match self {
            Syntax::Markdown => "markdown",
            Syntax::Typst => "typst",
            Syntax::Text => "text",
        }
    }
}

/// What a file's *name* says, when it says anything.
pub fn from_extension(name: &str) -> Option<Syntax> {
    let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
    match extension.as_str() {
        "md" | "markdown" | "mdown" => Some(Syntax::Markdown),
        "typ" | "typst" => Some(Syntax::Typst),
        _ => None,
    }
}

/// How many lines are read before the guess is made.
///
/// A Typst manuscript declares itself at the top — the imports and the `#let`s
/// come before the prose — and a file that has said nothing in two hundred
/// lines is prose either way.
const SAMPLE: usize = 200;

/// Guess what `text` is written in.
///
/// Votes, because a single line can be either language: `- 項` is a list in
/// both, `// note` is a comment in Typst and prose in Markdown. A tie is
/// Markdown, since that is the one a file of plain prose should be read as.
pub fn sniff(text: &str) -> Syntax {
    let mut typst = 0i32;
    let mut markdown = 0i32;
    for line in text.lines().take(SAMPLE) {
        let trimmed = line.trim_start();
        // Typst code, which Markdown has no spelling for at all.
        if ["#import", "#include", "#let", "#set", "#show"]
            .iter()
            .any(|k| trimmed.starts_with(k))
        {
            typst += 4;
            continue;
        }
        // A heading, spelled one way or the other.
        if trimmed.starts_with("# ") || trimmed.starts_with("## ") {
            markdown += 3;
            continue;
        }
        let equals = trimmed.chars().take_while(|&c| c == '=').count();
        if equals > 0 && equals <= 6 && trimmed.chars().nth(equals) == Some(' ') {
            typst += 3;
            continue;
        }
        // Emphasis: two asterisks are Markdown's bold, one is Typst's.
        if trimmed.contains("**") {
            markdown += 2;
        }
        if trimmed.contains("```") || trimmed.contains("~~") || trimmed.contains("](") {
            markdown += 2;
        }
        // A call, a code block, or maths — all of them Typst.
        if trimmed.starts_with("#[") || trimmed.starts_with("//") || trimmed.starts_with("$") {
            typst += 2;
        }
        // `#name(` or `#name[` mid-line is a Typst call. `#` followed by a
        // letter is never Markdown, whose heading takes a space.
        if let Some(rest) = trimmed.split_once('#').map(|(_, r)| r) {
            let mut chars = rest.chars();
            if chars.next().is_some_and(|c| c.is_alphabetic()) && rest.contains(['(', '[']) {
                typst += 2;
            }
        }
    }
    if typst > markdown {
        Syntax::Typst
    } else {
        Syntax::Markdown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_says_so_is_believed() {
        assert_eq!(from_extension("ch01.md"), Some(Syntax::Markdown));
        assert_eq!(from_extension("ch01.typ"), Some(Syntax::Typst));
        // The ones that say nothing are what the sniffing is for.
        assert_eq!(from_extension("ch01.txt"), None);
        assert_eq!(from_extension("ch01"), None);
    }

    #[test]
    fn a_typst_manuscript_declares_itself_at_the_top() {
        let text = "#import \"lib.typ\": chapter\n\n= 第一章\n\n那年冬天，雪下得*很早*。\n";
        assert_eq!(sniff(text), Syntax::Typst);
    }

    #[test]
    fn a_markdown_manuscript_does_too() {
        let text = "# 第一章\n\n那年冬天，雪下得**很早**。見[附錄](a.md)。\n";
        assert_eq!(sniff(text), Syntax::Markdown);
    }

    #[test]
    fn plain_prose_is_read_as_markdown() {
        // A tie goes to Markdown: guessing wrong is worse than not guessing,
        // because the guess decides what comes off the page.
        assert_eq!(
            sniff("那年冬天，雪下得比往常都早。\n屋簷下掛滿了冰稜。\n"),
            Syntax::Markdown
        );
        assert_eq!(sniff(""), Syntax::Markdown);
        // A list is a list in both, and settles nothing.
        assert_eq!(sniff("- 阿寧\n- 冬天\n"), Syntax::Markdown);
    }

    #[test]
    fn a_call_mid_line_is_typst_because_markdowns_hash_takes_a_space() {
        assert_eq!(sniff("那年#emph[冬天]雪下得早。\n"), Syntax::Typst);
        // …and a hash with a space after it is a heading, whatever follows.
        assert_eq!(sniff("# 第一章 (一)\n"), Syntax::Markdown);
    }
}
