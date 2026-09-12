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
//! So this counts votes rather than parsing — and **a file that votes for
//! neither is neither** (#410). A `.txt` is a novel until it says otherwise:
//! reading one as Markdown makes the writer's own `*` and `#` flicker at them,
//! offers a `<!-- -->` to a file that has no comments, and hides the `第三章`
//! outline behind a `#` rule the file never used. Guessing wrong is worse than
//! not guessing, because in 所見即所得 the guess decides what comes off the page.
//!
//! ⚠️ **The bar is evidence, not a majority.** One weak signal in two hundred
//! lines — a stray `**`, one `//` — is prose that happens to contain a
//! character, not a declaration; [`ENOUGH`] says how much it takes.

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

/// How much evidence a guess needs before it is made.
///
/// **One strong signal, or two weak ones.** A heading (`# 一` or `= 一`) and a
/// Typst keyword are each strong enough alone — no manuscript writes those by
/// accident. `**`, a fence, a link, a `//` are worth 2 apiece: one of them
/// anywhere in two hundred lines is a character the prose happened to contain,
/// two of them is a habit.
const ENOUGH: i32 = 3;

/// Guess what `text` is written in.
///
/// Votes, because a single line can be either language: `- 項` is a list in
/// both, `// note` is a comment in Typst and prose in Markdown. **A file that
/// clears [`ENOUGH`] for neither is [`Syntax::Text`]** — plain prose is the
/// commonest thing a `.txt` holds, and it is the reading that adds nothing to
/// the page. A tie above the bar goes to Markdown, which is the friendlier of
/// the two to be wrong about.
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
        // `| --- | --- |` under a header row. A file of pipe tables is
        // Markdown even when it never writes a heading, and it is the one
        // place `t f` and the padding depend on the guess.
        if trimmed.matches('|').count() >= 2
            && trimmed.contains("--")
            && trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
        {
            markdown += 3;
        }
        // A call, a code block, or maths — all of them Typst.
        if trimmed.starts_with("#[") || trimmed.starts_with("//") || trimmed.starts_with("$") {
            typst += 2;
        }
        // `#name(` or `#name[` mid-line is a Typst call, and as good as the
        // keywords: `#` followed by a letter is never Markdown, whose heading
        // takes a space. **The bracket must touch the name.** Asking only
        // whether the line holds one somewhere later reads Markdown's own
        // `見 #註 的說明（[附錄](a.md)）` as a call.
        if let Some((_, rest)) = trimmed.split_once('#') {
            let name: String = rest
                .chars()
                .take_while(|&c| c.is_alphanumeric() || c == '_' || c == '-')
                .collect();
            let opens = matches!(rest[name.len()..].chars().next(), Some('(' | '['));
            if opens && rest.starts_with(char::is_alphabetic) {
                typst += 3;
            }
        }
    }
    // Nobody made a claim on this file, so nothing gets to interpret it.
    if typst < ENOUGH && markdown < ENOUGH {
        return Syntax::Text;
    }
    match typst > markdown {
        true => Syntax::Typst,
        false => Syntax::Markdown,
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

    /// **A `.txt` is a novel until it says otherwise** (#410).
    #[test]
    fn plain_prose_is_not_markup_at_all() {
        assert_eq!(
            sniff("那年冬天，雪下得比往常都早。\n屋簷下掛滿了冰稜。\n"),
            Syntax::Text
        );
        assert_eq!(sniff(""), Syntax::Text);
        // A list is a list in both, and settles nothing.
        assert_eq!(sniff("- 阿寧\n- 冬天\n"), Syntax::Text);
        // Neither does a manuscript that uses `*` for a scene break and `#`
        // for a note to self — which is the file this rule is for.
        assert_eq!(sniff("　　*　*　*\n\n#記：這一段要重寫\n"), Syntax::Text);
    }

    /// One character is not a declaration; two is a habit.
    #[test]
    fn one_weak_signal_is_prose_and_two_are_a_language() {
        assert_eq!(sniff("雪下得**很早**。\n"), Syntax::Text, "one bold, no more");
        assert_eq!(sniff("雪下得**很早**。\n見**附錄**。\n"), Syntax::Markdown);
        // The same on the other side: one `//` is a line of prose that begins
        // with two slashes, two of them is Typst.
        assert_eq!(sniff("// 待補\n"), Syntax::Text);
        assert_eq!(sniff("// 待補\n// 再補\n"), Syntax::Typst);
    }

    /// A file of tables never writes a heading, and it is the one place the
    /// guess decides whether `t f` opens at all.
    #[test]
    fn a_pipe_tables_dashed_row_is_a_spelling_nothing_else_has() {
        assert_eq!(sniff("| 字 | 碼 |\n| --- | --- |\n| 甲 | ab |\n"), Syntax::Markdown);
        // Without that row it is somebody's ASCII drawing, not a table.
        assert_eq!(sniff("| 字 | 碼 |\n| 甲 | ab |\n"), Syntax::Text);
    }

    #[test]
    fn a_call_mid_line_is_typst_because_markdowns_hash_takes_a_space() {
        assert_eq!(sniff("那年#emph[冬天]雪下得早。\n"), Syntax::Typst);
        // …and a hash with a space after it is a heading, whatever follows.
        assert_eq!(sniff("# 第一章 (一)\n"), Syntax::Markdown);
        // ⚠️ The bracket has to touch the name. This line is Markdown — a
        // hash tag, then a link — and asking only whether a `[` turns up
        // later on the line called it Typst.
        assert_eq!(sniff("見 #註 的說明（[附錄](a.md)）\n"), Syntax::Text);
    }
}
