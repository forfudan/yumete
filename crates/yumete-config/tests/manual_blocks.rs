//! **Every `toml` block the manual prints is one a reader can paste.**
//!
//! The manual teaches the config file by showing it, and a block that does not
//! parse is worse than a wrong sentence: TOML is all or nothing, so one
//! un-copyable line in an example voids the whole file of whoever copies it,
//! and every other setting they had silently stops applying.
//!
//! Found by this walk on 2026-10-20, in the section on rebinding prefix keys:
//!
//! ```text
//! "<0x08>" = "@h"  # backspace moves left (a control character, written as its code point)
//! ```
//!
//! — a **raw** control character inside a basic string, which TOML forbids
//! (`invalid basic string`); the comment claiming it was 「written as its code
//! point」 was describing something the line did not do; and four paragraphs
//! above, the manual had already said that a named key goes in angle brackets.
//! Measured both ways: the raw form takes the whole file down, and
//! `"<backspace>" = "@h"` binds and works.

include!("../../../docs/test_support.rs");

/// The blocks that are **not** `config.toml` and must not be read as one.
///
/// Warning: **by their first table, and each one needs a reason.** Skipping a block
/// because it fails to parse would skip exactly the bug this test is for.
///
/// - `[table]` — a table schema, `.yumete/tables/*.toml`, a different file
///   with a different shape.
const NOT_THE_CONFIG: &[&str] = &["[table]"];

/// Every ```toml fence in the manual, with the line it starts on.
fn blocks(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut line = 0;
    let mut taking: Option<(usize, String)> = None;
    for one in text.lines() {
        line += 1;
        match (&mut taking, one.trim_end()) {
            (None, "```toml") => taking = Some((line + 1, String::new())),
            (Some(_), "```") => {
                let (at, body) = taking.take().expect("open");
                out.push((at, body));
            }
            (Some((_, body)), _) => {
                body.push_str(one);
                body.push('\n');
            }
            _ => {}
        }
    }
    out
}

#[test]
fn every_config_block_in_the_manual_parses() {
    let text = manual_text();
    let found = blocks(&text);
    assert!(found.len() > 10, "only {} toml blocks read out of the manual", found.len());

    let mut wrong = Vec::new();
    let mut checked = 0;
    for (at, body) in &found {
        if NOT_THE_CONFIG.iter().any(|head| body.trim_start().starts_with(head)) {
            continue;
        }
        checked += 1;
        for fault in yumete_config::faults::faults_in(body) {
            // **Only the fatal ones.** A block showing `zong_length = 100000`
            // to explain what is out of range would be a warning, and the
            // manual is allowed to print a value it is warning about; a block
            // that does not parse is never on purpose.
            if fault.fatal {
                wrong.push(format!(
                    "  docs/manual.md:{}: {}",
                    at + fault.line,
                    fault.said
                ));
            }
        }
    }
    assert!(checked > 8, "only {checked} config blocks checked");
    assert!(
        wrong.is_empty(),
        "{} block(s) in the manual cannot be pasted into a config:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// **The count the manual prints is the count the panel draws.**
///
/// 「Eight groups, fifty-four items」 — it was fifty-nine on 2026-10-20, and
/// nothing had ever compared the two.
#[test]
fn the_manual_counts_the_settings_the_panel_has() {
    let text = manual_text();
    let said = |n: usize| -> String {
        // Only the spellings this sentence has worn; a number this test cannot
        // spell is a number it must not claim to have checked.
        let words = [
            (54, "fifty-four"),
            (55, "fifty-five"),
            (56, "fifty-six"),
            (57, "fifty-seven"),
            (58, "fifty-eight"),
            (59, "fifty-nine"),
            (60, "sixty"),
            (61, "sixty-one"),
            (62, "sixty-two"),
            (63, "sixty-three"),
            (64, "sixty-four"),
            (65, "sixty-five"),
        ];
        words
            .iter()
            .find(|(k, _)| *k == n)
            .map(|(_, w)| (*w).to_string())
            .unwrap_or_else(|| panic!("spell {n} out in this test, then in the manual"))
    };
    let want = format!(
        "**Eight groups, {} items**",
        said(yumete_config::settings_ui::SETTINGS.len())
    );
    assert!(
        text.contains(&want),
        "the manual should say 「{want}」 — {} groups, {} items",
        yumete_config::settings_ui::GROUPS.len(),
        yumete_config::settings_ui::SETTINGS.len()
    );
}
