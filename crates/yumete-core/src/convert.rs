//! 簡繁 conversion, handed to opencc — Feature #241.
//!
//! ## Why the editor does not do this itself
//!
//! 簡繁 looks like a character table and is not one. 发 is 發 in 头发 and 髮 in
//! 发现; 干 is 乾, 幹 or 干; 里 is 里 or 裏 depending on whether it is a 里 of
//! road or the inside of something. Getting those right needs a phrase
//! dictionary and the 分詞 to use it, which is a project — and one that has
//! been done, carefully, for fifteen years, by
//! [OpenCC](https://github.com/BYVoid/OpenCC). A converter written here would
//! be a worse OpenCC that a manuscript would have to trust.
//!
//! So `:convert` runs `opencc`. If it is not installed the editor says so and
//! says how to install it, and changes nothing. That is the whole design.
//!
//! ## The one thing that is ours
//!
//! OpenCC's 繁體 is 港臺 字形: 爲 説 裏 for `s2t`, 為 裡 著 for `s2tw`. Neither
//! is 大陸通規繁體 — the standard a mainland publisher sets a 繁體 book in, and
//! the one 宇浩's own 字料 is written in. The difference is **glyph shape, not
//! word choice**: 説 vs 說, 内 vs 內, 吴 vs 吳 — a few dozen characters, each
//! one a straight swap.
//!
//! That is small enough to be data. [`Side::C`] and [`Side::G`] run opencc to
//! 繁體 first and then walk the result once, one character at a time, through
//! a table copied from [GujiCC](https://github.com/forFudan/GujiCC). No
//! context, no dictionary, no ambiguity — every row is one character for one
//! character, so the text comes back exactly as long as it went in.
//!
//! ## And the way back
//!
//! opencc has never heard of 通規, so `:convert c tw` cannot be handed to it as
//! it stands — `t2tw` is written against 港臺 字形 and walks straight past 説
//! 内 吴. The route is the same table read backwards, then the ordinary opencc
//! leg: **c → (table backwards) → t → `t2tw` → tw**.
//!
//! Backwards is not free, because the standard *merges*: 蝨 and 虱 are both
//! 繁體 characters opencc uses and 通規 keeps only 虱, so a 虱 in a 通規
//! manuscript has no unique 源. Those rows are named in the data file (see its
//! tail) and travel forward only — going back leaves them alone and lets
//! opencc's own dictionaries decide. Six characters for `c`, four for `g`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

/// 大陸通規繁體 字形, copied from GujiCC (Apache-2.0).
const GLYPHS_C: &str = include_str!("glyphs_c.txt");
/// 古籍通規繁體 字形, copied from GujiCC (Apache-2.0).
const GLYPHS_G: &str = include_str!("glyphs_g.txt");

/// One side of the 簡繁 divide — or one standard *within* 繁體.
///
/// The five short ones are opencc's own config names, so `:convert s tw` reads
/// as the `s2tw` it runs. `c` and `g` are the two mainland 繁體 standards,
/// which opencc has no config for and which this crate reaches by patching
/// glyphs afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// 簡體.
    S,
    /// OpenCC's plain 繁體 — 爲 説 裏 着.
    T,
    /// 臺灣正體 — 為 裡 著.
    Tw,
    /// 香港繁體 — 為 裏 着 説.
    Hk,
    /// 日本新字体.
    Jp,
    /// 大陸通規繁體 — 爲 裏 着 説 内 吴.
    C,
    /// 古籍通規繁體.
    G,
}

impl Side {
    /// The word typed after `:convert`.
    pub fn word(self) -> &'static str {
        match self {
            Side::S => "s",
            Side::T => "t",
            Side::Tw => "tw",
            Side::Hk => "hk",
            Side::Jp => "jp",
            Side::C => "c",
            Side::G => "g",
        }
    }

    /// A side by the word that names it.
    pub fn parse(word: &str) -> Option<Side> {
        Some(match word {
            "s" => Side::S,
            "t" => Side::T,
            "tw" => Side::Tw,
            "hk" => Side::Hk,
            "jp" => Side::Jp,
            "c" => Side::C,
            "g" => Side::G,
            _ => return None,
        })
    }

    /// Every side, in the order the listing shows them.
    pub const ALL: [Side; 7] = [
        Side::S,
        Side::T,
        Side::Tw,
        Side::Hk,
        Side::Jp,
        Side::C,
        Side::G,
    ];

    /// The 字形 table that separates this side from opencc's plain 繁體, if
    /// any — walked forwards into this side, backwards out of it.
    fn glyphs(self) -> Option<&'static str> {
        match self {
            Side::C => Some(GLYPHS_C),
            Side::G => Some(GLYPHS_G),
            _ => None,
        }
    }

    /// What opencc is asked for when this side is the **source**.
    ///
    /// `c` and `g` are 繁體 written a particular way; once the table has been
    /// walked backwards they *are* opencc's 繁體, which is why every route out
    /// of them starts at [`Side::T`].
    fn as_source(self) -> Side {
        match self {
            Side::C | Side::G => Side::T,
            other => other,
        }
    }
}

/// Every pair opencc has a config for: `(from, to, plain, 詞彙)`.
///
/// The fourth column is the `p` variant, which converts **words** and not just
/// characters — `s2twp` turns 内存 into 記憶體. It is a different kind of edit
/// from the rest of this file (it changes the character count, and it is not
/// reversible), so it is behind `force` rather than being what `:convert s tw`
/// quietly does.
const ROUTES: &[(Side, Side, &str, Option<&str>)] = &[
    (Side::S, Side::T, "s2t", None),
    (Side::S, Side::Tw, "s2tw", Some("s2twp")),
    (Side::S, Side::Hk, "s2hk", Some("s2hkp")),
    (Side::T, Side::S, "t2s", None),
    (Side::T, Side::Tw, "t2tw", None),
    (Side::T, Side::Hk, "t2hk", None),
    (Side::T, Side::Jp, "t2jp", None),
    (Side::Tw, Side::S, "tw2s", Some("tw2sp")),
    (Side::Tw, Side::T, "tw2t", None),
    (Side::Hk, Side::S, "hk2s", Some("hk2sp")),
    (Side::Hk, Side::T, "hk2t", None),
    (Side::Jp, Side::T, "jp2t", None),
];

/// What `:convert` will actually do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The side whose 字形 table is walked **backwards** over the text before
    /// opencc sees it — `Some(C)` for anything starting at 大陸通規繁體, and
    /// `None` for every side opencc already understands.
    pub unpatch: Option<Side>,
    /// The opencc config to run the text through — `None` when the text is
    /// already on the right side and only the 字形 table has anything to say
    /// (`:convert t c` runs no program at all).
    pub config: Option<&'static str>,
    /// The side whose 字形 table is walked over what comes back.
    pub patch: Option<Side>,
}

/// Why a conversion cannot be planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snag {
    /// Both sides are the same one.
    Same,
    /// opencc has no route between these two.
    NoRoute,
    /// `force` was asked for on a pair with no 詞彙 variant.
    NoWords,
}

/// How to get from `from` to `to`, or why that cannot be done.
pub fn plan(from: Side, to: Side, force: bool) -> Result<Plan, Snag> {
    if from == to {
        return Err(Snag::Same);
    }
    // Leaving 通規 means putting the 字形 back the way opencc writes them,
    // because opencc's own configs cannot read them: `t2s` happens to know
    // 説 内 吴 (they come back as 说 内 吴), but `t2tw` and `t2hk` are written
    // against 港臺 字形 and walk straight past them, handing back a 臺灣正體
    // text with mainland glyphs still in it. Checked against opencc 1.4.2
    // rather than assumed.
    let unpatch = from.glyphs().is_some().then_some(from);
    let src = from.as_source();
    if to.glyphs().is_some() {
        if force {
            return Err(Snag::NoWords);
        }
        // Already 繁體: nothing to run, just the table.
        if src == Side::T {
            return Ok(Plan {
                unpatch,
                config: None,
                patch: Some(to),
            });
        }
        let (_, _, config, _) = ROUTES
            .iter()
            .find(|(f, t, _, _)| *f == src && *t == Side::T)
            .ok_or(Snag::NoRoute)?;
        return Ok(Plan {
            unpatch,
            config: Some(config),
            patch: Some(to),
        });
    }
    // `:convert c t` is the table backwards and nothing else — there is no
    // opencc leg, because 通規 *is* 繁體 once the 字形 are opencc's own.
    if src == to {
        // No 詞彙 variant of 「walk a table backwards」 exists to ask for.
        if force {
            return Err(Snag::NoWords);
        }
        return match unpatch {
            Some(_) => Ok(Plan {
                unpatch,
                config: None,
                patch: None,
            }),
            None => Err(Snag::Same),
        };
    }
    let (_, _, plain, words) = ROUTES
        .iter()
        .find(|(f, t, _, _)| *f == src && *t == to)
        .ok_or(Snag::NoRoute)?;
    let config = match force {
        false => *plain,
        true => words.ok_or(Snag::NoWords)?,
    };
    Ok(Plan {
        unpatch,
        config: Some(config),
        patch: None,
    })
}

/// One side's 字形 table, both ways round.
struct Glyphs {
    /// opencc's 繁體 → this side's.
    forward: HashMap<char, char>,
    /// This side's → opencc's, minus the rows the standard merged.
    back: HashMap<char, char>,
}

/// The 字形 table for a side, parsed once.
fn table(which: Side) -> &'static Glyphs {
    static C: OnceLock<Glyphs> = OnceLock::new();
    static G: OnceLock<Glyphs> = OnceLock::new();
    let slot = match which {
        Side::G => &G,
        _ => &C,
    };
    slot.get_or_init(|| parse_glyphs(which.glyphs().unwrap_or("")))
}

/// `舊<TAB>新` a line, `#` a comment.
///
/// Where the right-hand side lists several candidates the first one wins:
/// GujiCC writes `岳\t岳 嶽` to mean 「keep 岳, and 嶽 is the other one people
/// use」, and a converter that cannot ask has to take the recommendation.
fn parse_glyphs(text: &str) -> Glyphs {
    let mut forward = HashMap::new();
    let mut rows: Vec<(char, char)> = Vec::new();
    let mut merged: HashSet<char> = HashSet::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split('\t');
        let Some(from) = parts.next() else { continue };
        let Some(to) = parts.next() else {
            // One field: a 字形 the standard merged two 繁體 characters into.
            // It converts *into* 通規 like any other and has no way back.
            if let Some(c) = one_char(from.trim()) {
                merged.insert(c);
            }
            continue;
        };
        let Some(from) = one_char(from) else { continue };
        let Some(to) = to.split_whitespace().next().and_then(one_char) else {
            continue;
        };
        if from != to {
            forward.insert(from, to);
            rows.push((from, to));
        }
    }
    // Backwards, later rows winning: the 補 section at the foot of the file
    // exists precisely to correct an upstream row, and correcting it means
    // being the one that answers for that 字形.
    let mut back = HashMap::new();
    for (from, to) in rows {
        if !merged.contains(&to) {
            back.insert(to, from);
        }
    }
    Glyphs { forward, back }
}

/// A field that is exactly one `char`, or nothing.
fn one_char(field: &str) -> Option<char> {
    let mut chars = field.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

/// Walk `text` through a side's 字形 table.
///
/// One character out for every character in, always: this is the step that is
/// allowed to be a table precisely because it never has to decide anything.
pub fn respell(text: &str, which: Side) -> String {
    walk(text, &table(which).forward)
}

/// Walk `text` back through a side's 字形 table, into the 字形 opencc writes.
///
/// The characters the standard merged are left exactly as they are — see the
/// foot of the data file. Leaving them is not a shortcut: opencc's `t2s` and
/// `t2tw` read 虱 and 岳 perfectly well, so the one thing a guess here could
/// add is a wrong 蝨.
pub fn unrespell(text: &str, which: Side) -> String {
    walk(text, &table(which).back)
}

/// One character out for every character in, through whichever map.
fn walk(text: &str, map: &HashMap<char, char>) -> String {
    if map.is_empty() {
        return text.to_string();
    }
    text.chars()
        .map(|c| map.get(&c).copied().unwrap_or(c))
        .collect()
}

/// Where `opencc` is, if it is anywhere.
///
/// Looked up rather than spawned: `:convert` has to be able to say 「it is not
/// installed, here is how」 *before* it hands a whole manuscript to a program
/// that may not exist, and the answer to 「does this exist」 should not cost a
/// process.
pub fn opencc() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("opencc"))
        .find(|candidate| candidate.is_file())
}

/// Every destination a side can reach, and whether `force` works for it.
///
/// Asked of [`plan`] rather than read off [`ROUTES`], so the menu and the
/// command can never disagree about what is possible.
pub fn destinations(from: Side) -> Vec<(Side, bool)> {
    Side::ALL
        .into_iter()
        .filter_map(|to| {
            plan(from, to, false)
                .ok()
                .map(|_| (to, plan(from, to, true).is_ok()))
        })
        .collect()
}

/// The shell line that runs one conversion.
///
/// The program is quoted because it came from `PATH`, and a `PATH` entry is
/// whatever the reader's shell profile put there — `/Users/…/Application
/// Support/…` is a real place for a package manager to install into. The
/// config name never needs quoting: it comes from [`ROUTES`], which is a
/// literal in this file.
pub fn command_line(program: &std::path::Path, config: &str) -> String {
    let program = program.to_string_lossy().replace('\'', r"'\''");
    format!("'{program}' -c {config}.json")
}

/// The command that installs opencc on this machine.
///
/// macOS gets Homebrew, which `:convert-opencc install` can actually run.
/// Linux gets whichever package manager is there, printed rather than run: it
/// needs `sudo`, and a full-screen terminal application is the wrong place to
/// be asked for a password.
pub fn install_line() -> &'static str {
    if cfg!(target_os = "macos") {
        "brew install opencc"
    } else if cfg!(target_os = "windows") {
        "winget install BYVoid.OpenCC"
    } else {
        "sudo apt install opencc  ｜  sudo dnf install opencc  ｜  sudo pacman -S opencc"
    }
}

/// Whether `:convert-opencc install` may run the install itself.
pub fn install_runs_here() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glyph_tables_are_one_character_for_one_character() {
        for side in [Side::C, Side::G] {
            let map = &table(side).forward;
            assert!(map.len() > 60, "{} has only {} rows", side.word(), map.len());
            // The whole promise of this step: length in equals length out.
            let sample = "說爲內吳呂溫錄羣峯黃橫";
            assert_eq!(
                respell(sample, side).chars().count(),
                sample.chars().count()
            );
        }
    }

    #[test]
    fn mainland_glyphs_are_what_the_standard_asks_for() {
        // 說→説, 為→爲 is opencc's job; 爲 stays, 說 changes.
        assert_eq!(respell("說內吳呂溫黃橫", Side::C), "説内吴吕温黄横");
        // The two rows added on this side: opencc's s2t turns 奖 into 獎 and
        // 岳 into 嶽 before the table ever sees them.
        assert_eq!(respell("嶽獎", Side::C), "岳奬");
    }

    #[test]
    fn the_way_back_undoes_everything_the_standard_did_not_merge() {
        for side in [Side::C, Side::G] {
            // Read back-to-front on purpose. Forwards is many-to-one — the
            // 補 section at the foot of the file exists to override an
            // upstream row whose 字頭 is 簡體 (奖 and 奬 both point at 奬, and
            // only 獎 can ever arrive from opencc) — so the invariant that
            // means anything is that every 字形 the way back names is one the
            // way out actually produces.
            let Glyphs { forward, back } = table(side);
            for (&to, &from) in back {
                assert_eq!(forward.get(&from), Some(&to), "{from} does not make {to}");
            }
        }
        assert_eq!(unrespell("説内吴吕温録黄横奬", Side::C), "說內吳呂溫錄黃橫獎");
        // 通規 folds 蝨 into 虱 and 嶽 into 岳; going back does not guess.
        assert_eq!(unrespell("虱岳", Side::C), "虱岳");
    }

    #[test]
    fn a_route_is_planned_or_refused_but_never_guessed() {
        assert_eq!(
            plan(Side::S, Side::T, false),
            Ok(Plan {
                unpatch: None,
                config: Some("s2t"),
                patch: None
            })
        );
        assert_eq!(
            plan(Side::S, Side::C, false),
            Ok(Plan {
                unpatch: None,
                config: Some("s2t"),
                patch: Some(Side::C)
            })
        );
        // Already 繁體: the table alone, no child process.
        assert_eq!(
            plan(Side::T, Side::C, false),
            Ok(Plan {
                unpatch: None,
                config: None,
                patch: Some(Side::C)
            })
        );
        // Out of 通規: the table backwards, then opencc's ordinary route.
        assert_eq!(
            plan(Side::C, Side::S, false),
            Ok(Plan {
                unpatch: Some(Side::C),
                config: Some("t2s"),
                patch: None
            })
        );
        assert_eq!(
            plan(Side::C, Side::Tw, false),
            Ok(Plan {
                unpatch: Some(Side::C),
                config: Some("t2tw"),
                patch: None
            })
        );
        // `c t` is the table backwards and nothing else — no child process.
        assert_eq!(
            plan(Side::C, Side::T, false),
            Ok(Plan {
                unpatch: Some(Side::C),
                config: None,
                patch: None
            })
        );
        // Between the two 通規 standards: backwards out of one, forwards into
        // the other, still no opencc.
        assert_eq!(
            plan(Side::C, Side::G, false),
            Ok(Plan {
                unpatch: Some(Side::C),
                config: None,
                patch: Some(Side::G)
            })
        );
        assert_eq!(plan(Side::S, Side::S, false), Err(Snag::Same));
        assert_eq!(plan(Side::Jp, Side::S, false), Err(Snag::NoRoute));
        assert_eq!(
            plan(Side::S, Side::Tw, true),
            Ok(Plan {
                unpatch: None,
                config: Some("s2twp"),
                patch: None
            })
        );
        assert_eq!(plan(Side::S, Side::T, true), Err(Snag::NoWords));
        assert_eq!(plan(Side::S, Side::C, true), Err(Snag::NoWords));
    }

    #[test]
    fn the_menu_offers_only_what_the_command_will_do() {
        // 日本新字体 reads back into 繁體 and no further, and the menu has to
        // say so rather than offering `jp s`, which opencc has no config for.
        let from_jp: Vec<Side> = destinations(Side::Jp).into_iter().map(|(t, _)| t).collect();
        assert_eq!(from_jp, vec![Side::T, Side::C, Side::G]);
        // `force` is offered exactly where a `p` config exists.
        assert_eq!(
            destinations(Side::S),
            vec![
                (Side::T, false),
                (Side::Tw, true),
                (Side::Hk, true),
                (Side::C, false),
                (Side::G, false)
            ]
        );
        assert_eq!(
            destinations(Side::C),
            vec![
                (Side::S, false),
                (Side::T, false),
                (Side::Tw, false),
                (Side::Hk, false),
                (Side::Jp, false),
                (Side::G, false)
            ]
        );
    }

    #[test]
    fn a_path_with_a_space_in_it_is_still_one_word() {
        let line = command_line(std::path::Path::new("/opt/my tools/opencc"), "s2t");
        assert_eq!(line, "'/opt/my tools/opencc' -c s2t.json");
    }

    #[test]
    fn every_route_opencc_ships_is_reachable() {
        // The table is the contract with a program we do not control: if a
        // pair is listed here it has to be plannable, or `:convert` offers a
        // combination that fails at the command line.
        for (from, to, plain, words) in ROUTES {
            assert_eq!(plan(*from, *to, false).unwrap().config, Some(*plain));
            if let Some(p) = words {
                assert_eq!(plan(*from, *to, true).unwrap().config, Some(*p));
            }
        }
    }
}
