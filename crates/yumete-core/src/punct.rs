//! 標點 — the marks a Chinese manuscript got wrong and cannot see (#238).
//!
//! Three questions, and all three are invisible to the eye that wrote the
//! sentence:
//!
//! - **半角**. A `,` in Chinese text is not a comma, it is a comma sitting on
//!   the left third of its 字身 with nothing under it. It happens because the
//!   IME was off for one keystroke, and it is the single most common thing a
//!   typesetter sends a manuscript back for.
//! - **`...`**. The Chinese ellipsis is 六點, two `…` in a row. Three ASCII
//!   dots are an English one.
//! - **Unbalanced 「」（）《》**. This is the one worth having. A missing 」
//!   does not look wrong anywhere — it silently inverts every quotation mark
//!   after it, for the rest of the chapter, and the writer proofreading the
//!   page it happened on sees nothing at all.
//!
//! **The rule everywhere is 「only where it is Chinese」.** `3.14`, `1,000`,
//! `../path`, a URL and an English sentence are all full of half-width marks
//! and all of them are right; what makes one wrong is the 漢字 next to it. So
//! a mark is only reported when the character beside it is Chinese, and code —
//! fenced blocks and `inline spans` — is skipped whole.

use yumete_cjk::is_han;

/// What is wrong with one mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A half-width mark in Chinese text, `wanted` being its full-width twin.
    HalfWidth,
    /// `...` where 中文 wants `……`.
    Ellipsis,
    /// An opener nothing closes.
    Unclosed,
    /// A closer nothing opened.
    Unopened,
}

/// One mark the manuscript should look at again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slip {
    /// Which line, counting from zero.
    pub line: usize,
    /// Which character of it, counting from zero.
    pub column: usize,
    /// What is wrong.
    pub kind: Kind,
    /// What is written here.
    pub written: String,
    /// What it should be — for an unbalanced pair, the mark that is missing.
    pub wanted: String,
}

/// The half-width marks that have a 全角 twin, and the twin.
///
/// `(`/`)` are here and `[`/`]` are not: a `[` in a Markdown manuscript is a
/// link, and telling a writer that every link should be 【 would make the
/// command useless on the first file it was run on.
const TWINS: &[(char, char)] = &[
    (',', '，'),
    ('.', '。'),
    (';', '；'),
    (':', '：'),
    ('!', '！'),
    ('?', '？'),
    ('(', '（'),
    (')', '）'),
];

/// The pairs a paragraph has to balance, opener first.
const PAIRS: &[(char, char)] = &[
    ('「', '」'),
    ('『', '』'),
    ('（', '）'),
    ('《', '》'),
    ('〈', '〉'),
    ('【', '】'),
    ('“', '”'),
    ('‘', '’'),
];

/// Whether `c` is Chinese enough to make the mark beside it a Chinese mark.
///
/// 漢字 plus the CJK marks themselves, so that 「，」 and 「。」 count as
/// company: `他說：「好。」,` has nothing but punctuation to the left of that
/// stray comma, and it is still wrong.
fn chinese(c: char) -> bool {
    is_han(c) || matches!(c as u32, 0x3000..=0x303F | 0xFF01..=0xFF60)
}

/// The line's characters, with everything inside code blanked to a space.
///
/// Blanked rather than removed so that every column reported is still the
/// column the cursor has to go to.
fn prose(line: &str, in_fence: &mut bool) -> Vec<char> {
    let chars: Vec<char> = line.chars().collect();
    let fence = line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~");
    if fence {
        *in_fence = !*in_fence;
        return vec![' '; chars.len()];
    }
    if *in_fence {
        return vec![' '; chars.len()];
    }
    // An inline span runs from a backtick to the next one; an unclosed
    // backtick closes at the end of the line, the way every Markdown renderer
    // treats it.
    let mut out = chars.clone();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] == '`' {
            let end = chars[at + 1..]
                .iter()
                .position(|&c| c == '`')
                .map(|i| at + 1 + i)
                .unwrap_or(chars.len() - 1);
            for c in out.iter_mut().take(end + 1).skip(at) {
                *c = ' ';
            }
            at = end + 1;
            continue;
        }
        at += 1;
    }
    out
}

/// Whether the nearest non-space neighbour on either side is Chinese.
fn beside_chinese(chars: &[char], at: usize, len: usize) -> bool {
    let before = chars[..at].iter().rev().find(|c| !c.is_whitespace());
    let after = chars[at + len..].iter().find(|c| !c.is_whitespace());
    before.copied().is_some_and(chinese) || after.copied().is_some_and(chinese)
}

/// The marks **one line** answers for on its own: a half-width mark among
/// 漢字, and `...` where 中文 wants `……`.
///
/// Told apart from the pairs below because these two are decidable from the
/// line itself, and so can be drawn on the page as it is typed (#248's inline
/// notes) rather than only listed after a walk of the whole manuscript. An
/// unbalanced 「 is not: whether it is wrong depends on the paragraph after it.
fn marks_in(chars: &[char], line: usize, slips: &mut Vec<Slip>) {
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        // `...` — count the whole run, so `.....` is one finding.
        if c == '.' {
            let run = chars[at..].iter().take_while(|&&c| c == '.').count();
            if run >= 3 && beside_chinese(chars, at, run) {
                slips.push(Slip {
                    line,
                    column: at,
                    kind: Kind::Ellipsis,
                    written: ".".repeat(run),
                    wanted: "……".to_string(),
                });
                at += run;
                continue;
            }
        }
        if let Some(&(_, full)) = TWINS.iter().find(|(half, _)| *half == c) {
            // A decimal point and a thousands comma are not punctuation, and
            // both sides of them are digits.
            let digits = matches!(c, '.' | ',')
                && at > 0
                && chars[at - 1].is_ascii_digit()
                && chars.get(at + 1).is_some_and(char::is_ascii_digit);
            if !digits && beside_chinese(chars, at, 1) {
                slips.push(Slip {
                    line,
                    column: at,
                    kind: Kind::HalfWidth,
                    written: c.to_string(),
                    wanted: full.to_string(),
                });
            }
        }
        at += 1;
    }
}

/// What is wrong with the marks on **one line**, asked of that line alone.
///
/// The two findings a line can answer for by itself, with `line` left at zero:
/// what `:hint` draws beside the mark as the sentence is written. The pairs —
/// 「」（）《》 — are not here, because an opener left unclosed at the end of a
/// paragraph is correct Chinese typesetting when the next paragraph opens the
/// same way, and one line cannot see that. Those stay with [`check`].
///
/// The line is read as prose: a fenced block is the caller's business (it
/// knows which lines are literal), but an `inline span` is blanked here the
/// same way [`check`] blanks it.
pub fn check_line(text: &str) -> Vec<Slip> {
    let mut slips = Vec::new();
    let mut in_fence = false;
    let chars = prose(text, &mut in_fence);
    marks_in(&chars, 0, &mut slips);
    slips
}

/// Every mark worth a second look, in reading order.
pub fn check(text: &str) -> Vec<Slip> {
    let mut slips = Vec::new();
    let mut in_fence = false;
    let lines: Vec<Vec<char>> = text
        .lines()
        .map(|l| prose(l, &mut in_fence))
        .collect::<Vec<_>>();
    for (line, chars) in lines.iter().enumerate() {
        marks_in(chars, line, &mut slips);
    }

    // The pairs, one paragraph at a time — which here is one line, because a
    // Chinese paragraph is one line (see `docs/manual.md` §四「想寫多長」).
    for (line, chars) in lines.iter().enumerate() {
        let mut stack: Vec<(usize, char)> = Vec::new();
        for (at, &c) in chars.iter().enumerate() {
            if let Some(&(_, close)) = PAIRS.iter().find(|(o, _)| *o == c) {
                stack.push((at, close));
                continue;
            }
            if PAIRS.iter().any(|(_, close)| *close == c) {
                match stack.iter().rposition(|&(_, want)| want == c) {
                    // 「（」 — everything opened after the one this closes is
                    // closed by nothing, and a later paragraph cannot rescue
                    // it the way it rescues a 「 at the end of a line.
                    Some(i) => {
                        for &(was, want) in &stack[i + 1..] {
                            slips.push(Slip {
                                line,
                                column: was,
                                kind: Kind::Unclosed,
                                written: chars[was].to_string(),
                                wanted: want.to_string(),
                            });
                        }
                        stack.truncate(i);
                    }
                    None => slips.push(Slip {
                        line,
                        column: at,
                        kind: Kind::Unopened,
                        written: c.to_string(),
                        wanted: PAIRS
                            .iter()
                            .find(|(_, close)| *close == c)
                            .map(|(o, _)| o.to_string())
                            .unwrap_or_default(),
                    }),
                }
            }
        }
        for (at, close) in stack {
            let open = chars[at];
            // **A 「 left open at the end of a paragraph may be right.** When a
            // quotation runs over several paragraphs, Chinese typesetting opens
            // 「 again at the head of each one and closes it only at the end of
            // the last. So an unclosed quote is only wrong when the paragraph
            // after it does not open the same way.
            let quoted = matches!(open, '「' | '『' | '“' | '‘');
            if quoted && next_paragraph_reopens(&lines, line, open) {
                continue;
            }
            slips.push(Slip {
                line,
                column: at,
                kind: Kind::Unclosed,
                written: open.to_string(),
                wanted: close.to_string(),
            });
        }
    }
    slips.sort_by_key(|s| (s.line, s.column));
    slips
}

/// Whether the next paragraph opens with the same mark — the 多段引文 rule.
fn next_paragraph_reopens(lines: &[Vec<char>], line: usize, open: char) -> bool {
    lines[line + 1..]
        .iter()
        .find(|l| l.iter().any(|c| !c.is_whitespace()))
        .is_some_and(|l| {
            l.iter()
                .find(|c| !c.is_whitespace() && **c != '　')
                .is_some_and(|&c| c == open)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(usize, usize, Kind, String)> {
        check(text)
            .into_iter()
            .map(|s| (s.line, s.column, s.kind, s.wanted))
            .collect()
    }

    /// The comma that got typed with the IME off.
    #[test]
    fn a_half_width_mark_between_hanzi_is_a_slip() {
        assert_eq!(
            kinds("他說,好."),
            vec![
                (0, 2, Kind::HalfWidth, "，".to_string()),
                (0, 4, Kind::HalfWidth, "。".to_string()),
            ]
        );
    }

    /// **What must not fire.** A decimal point, a thousands comma, a file name
    /// and an English sentence are all correct, and a check that reported them
    /// would be turned off on the first file it ran on.
    #[test]
    fn numbers_and_english_keep_their_own_marks() {
        assert_eq!(kinds("圓周率是 3.14，see README.md, right?"), vec![]);
    }

    /// Code is not prose — neither a fenced block nor an inline span.
    #[test]
    fn code_is_not_prose() {
        let text = "他寫的是 `a.b,c` 這個\n\n```\n他說,好.\n```\n";
        assert_eq!(kinds(text), vec![]);
    }

    /// 中文的省略號是六點，一次報一整串。
    #[test]
    fn three_dots_are_an_english_ellipsis() {
        assert_eq!(
            kinds("他不知道....."),
            vec![(0, 4, Kind::Ellipsis, "……".to_string())]
        );
    }

    /// The finding worth having: nothing on the page looks wrong.
    #[test]
    fn a_quote_that_never_closes_is_found() {
        assert_eq!(
            kinds("他說：「你回來了。\n"),
            vec![(0, 3, Kind::Unclosed, "」".to_string())]
        );
        assert_eq!(
            kinds("她問：你回來了。」\n"),
            vec![(0, 8, Kind::Unopened, "「".to_string())]
        );
    }

    /// A quotation running over paragraphs opens 「 again in each and closes it
    /// only at the end — the first two paragraphs are not missing anything.
    #[test]
    fn a_quotation_carried_over_paragraphs_is_left_alone() {
        let text = "「第一段。\n\n「第二段。\n\n「第三段。」\n";
        assert_eq!(kinds(text), vec![]);
    }

    /// A bracket cannot be carried over the way a quote can.
    #[test]
    fn a_bracket_left_open_inside_a_quote_is_found() {
        assert_eq!(
            kinds("他說「（好」"),
            vec![(0, 3, Kind::Unclosed, "）".to_string())]
        );
    }

    #[test]
    fn one_line_answers_for_its_own_marks() {
        let slips = check_line("他說,好的...");
        assert_eq!(slips.len(), 2, "{slips:?}");
        assert_eq!(slips[0].kind, Kind::HalfWidth);
        assert_eq!(slips[0].column, 2);
        assert_eq!(slips[0].wanted, "，");
        assert_eq!(slips[1].kind, Kind::Ellipsis);
        assert_eq!(slips[1].written, "...");
    }

    #[test]
    fn one_line_says_nothing_about_a_quote_it_cannot_see_the_end_of() {
        // 「 unclosed here is `check`'s question, not this one's: the next
        // paragraph may open with 「 too, and one line cannot see it.
        assert!(check_line("「他說").is_empty());
    }

    #[test]
    fn one_line_leaves_code_and_english_alone() {
        assert!(check_line("`a, b` and 3.14 in README.md").is_empty());
    }

}
