//! **The manual is not a development log** (2026-10-09).
//!
//! Found by the author searching his own manual and hitting a wall of
//! 「（2026-09-25 定）」: 「你这个是用户手册还是开发记录？？？？」. He was right. The cause
//! was a habit of mine — I maintained the manual append-only, adding a paragraph for
//! every behaviour change instead of editing the paragraph that describes the
//! behaviour, in the register of a code comment: date, 原話, reason.
//!
//! Measured against vim, nvim, emacs, helix, kakoune, micro, nano and zed: across
//! 212,000 words of those eight manuals there is **not one decision date**, and the five
//! modern ones quote **no user, ever**. Vim ran this same cleanup and recorded why
//! (patch 8.1.1280: 「Remarks about functionality not in Vi clutters the help」).
//!
//! Warning: **This is the only thing that stops the cleanup decaying.** Care does not
//! survive a month of feature work; a red build does.

/// Everything that marks a sentence as a log entry rather than documentation.
const BANNED: &[(&str, &str)] = &[
    (r"20\d\d-\d\d-\d\d", "a date: when a thing was decided is not the reader's business"),
    (r"\(#\d{3}\)", "an issue number: the reader cannot open it and does not care"),
    ("(decided ", "「decided …」: say what it does, not when it was settled"),
    ("(reported ", "「reported …」: the reader is not the bug tracker"),
    ("in the original words", "a quotation of the room — no manual of the eight quotes anyone"),
    ("原話", "a quotation of the room"),
];

/// **What the ban must not catch.**
///
/// Code is exempt — both fenced blocks and inline spans — because a manual about a text
/// editor shows text, and some of that text is dates:
///
/// - the screen string `出廠自帶 2026-08-28 13:08`, which the editor really prints;
/// - the `:count-progress` chart and its `.tsv`, whose rows are dated by nature;
/// - hex colours such as `#262A27`, which `(#\d{3})` would otherwise read as issue 262.
///
/// Warning: **`§` is deliberately not banned.** The ones in the manual point at clreq,
/// CommonMark and GB/T — external standards a typographic rule has to cite — and at the
/// manual's own numbered sections. Those are references a reader can follow, which is the
/// opposite of the problem.
fn without_code(line: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for ch in line.chars() {
        match ch {
            '`' => inside = !inside,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out
}

#[test]
fn the_manual_dates_nothing_and_quotes_nobody() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    // Warning: **English only.** The Chinese manuals are frozen until yumete settles
    // (2026-10-09), and a frozen document is one a test cannot help — the next cleanup
    // would turn it red and the only way to green would be to edit the very file we
    // decided to stop editing.
    let text = std::fs::read_to_string(format!("{root}/docs/manual.md")).expect("the manual");
    let mut bad: Vec<String> = Vec::new();
    let mut fenced = false;
    for (n, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let plain = without_code(line);
        for (mark, why) in BANNED {
            let hit = match mark.starts_with("20\\d") || mark.starts_with(r"\(#") {
                true => regex_lite_hit(mark, &plain),
                false => plain.contains(mark),
            };
            if hit {
                bad.push(format!("  docs/manual.md:{}  {why}\n    {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "the manual is documentation, not a log — {} line(s):\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// The two shapes above, without pulling `regex` into a test that needs two patterns.
fn regex_lite_hit(mark: &str, text: &str) -> bool {
    let bytes: Vec<char> = text.chars().collect();
    match mark {
        // `20\d\d-\d\d-\d\d`
        r"20\d\d-\d\d-\d\d" => bytes.windows(10).any(|w| {
            w[0] == '2'
                && w[1] == '0'
                && w[2..4].iter().all(char::is_ascii_digit)
                && w[4] == '-'
                && w[5..7].iter().all(char::is_ascii_digit)
                && w[7] == '-'
                && w[8..10].iter().all(char::is_ascii_digit)
        }),
        // `(#123)`
        r"\(#\d{3}\)" => bytes.windows(6).any(|w| {
            w[0] == '(' && w[1] == '#' && w[2..5].iter().all(char::is_ascii_digit) && w[5] == ')'
        }),
        _ => false,
    }
}
