//! 用字 — which spelling a manuscript settled on, and where it slipped
//! (Feature #233).
//!
//! Not a spell-checker, and not a 簡繁 converter. A manuscript written over a
//! year has 裏 four hundred times and 裡 three times, and nothing tells the
//! writer: Word checks 病句, Grammarly is English, and every spell-checker
//! there is tokenizes on spaces and sees one word from the first character to
//! the last. The three are not errors — both spellings are correct 漢字 — they
//! are an *inconsistency*, which is a different question and one only the
//! document itself can answer.
//!
//! So the rule is the document's own: **a group is only reported when the
//! manuscript writes more than one of its spellings**, and the one it writes
//! most is the one it meant. A book that says 裡 throughout is never asked
//! about it.

/// Spellings that mean the same thing, one group to a line, the traditional
/// or more usual one first.
///
/// First is only a tie-break — the manuscript's own count decides — but when
/// 二 and 二 are written the same number of times, the first is what it is
/// asked to settle on.
///
/// **What is not here matters as much as what is.** 面/麵, 谷/穀, 困/睏 and
/// 仿/彷 are pairs in the dictionaries and would fire on almost every page:
/// 上面 is not a bowl of noodles and 困難 is not sleepy. A group earns its
/// place by being a choice a writer actually makes, not by being a pair a
/// 異體字表 lists.
static GROUPS: &[&str] = &[
    "裏 裡",
    "為 爲",
    "臺 台",
    "著 着",
    "麼 麽 么",
    "才 纔",
    "群 羣",
    "峰 峯",
    "祕 秘",
    "線 綫",
    "沉 沈",
    "佔 占",
    "布 佈",
    "啟 啓",
    "決 决",
    "溫 温",
    "強 强",
    "產 産",
    "戶 户",
    "內 内",
    "曬 晒",
    "污 汙",
    "讚 贊",
    "慾 欲",
    "濕 溼",
    "牀 床",
    "遊 游",
    "盡 儘",
    "蹟 跡 迹",
    "掛 挂",
    "沖 冲",
    "傑 杰",
    "即 卽",
    "既 旣",
    "研 硏",
    "青 靑",
    "嘩 譁",
    "鋪 舖",
    "鏽 銹",
    "繡 綉",
    "妝 粧",
    "眾 衆",
    "冊 册",
    "插 揷",
    "敘 敍 叙",
    "幹 榦",
    "匯 滙",
    "毀 燬",
    "兇 凶",
    "拚 拼",
    "喫 吃",
    "擡 抬",
    "杯 盃",
    "煙 菸 烟",
    "慄 栗",
    "嘗 嚐",
    "姊 姐",
    "牠 它",
    "妳 你",
    "甚麼 什麼",
    "身分 身份",
];

/// One place the manuscript wrote a spelling other than its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slip {
    /// Which line, counting from zero.
    pub line: usize,
    /// Which character of it, counting from zero.
    pub column: usize,
    /// What was written here.
    pub written: String,
    /// What the rest of the manuscript writes.
    pub instead: String,
    /// How many times each, whole document.
    pub written_count: usize,
    /// …and the majority spelling's own count, which is what makes it one.
    pub instead_count: usize,
}

/// One spelling of one group, as the scanner holds it.
struct Word {
    group: usize,
    text: &'static str,
}

/// The same, for a group the reader wrote in their config.
struct Own {
    group: usize,
    text: String,
}

/// Read 「甲 乙 丙」 lines into groups of spellings.
///
/// Blank entries and one-word groups are dropped rather than refused: a group
/// with nothing to choose between is not an error in a config file, it is a
/// line somebody is still writing.
fn groups_of<S: AsRef<str>>(lines: &[S]) -> Vec<Vec<String>> {
    lines
        .iter()
        .map(|l| {
            l.as_ref()
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<String>>()
        })
        .filter(|g| g.len() > 1)
        .collect()
}

/// Every place a minority spelling is written, in reading order.
///
/// `extra` is the reader's own list in the same 「甲 乙」 spelling — a novel's
/// proper names are the whole reason it exists, because 阿嬌 and 阿姣 are not
/// in anybody's 異體字表 and are exactly the pair a manuscript slips on.
pub fn check(text: &str, extra: &[String]) -> Vec<Slip> {
    let mine = groups_of(extra);
    // The built-in groups first, so a reader's own group is numbered after
    // them and the two never share an index.
    let base = GROUPS.len();
    let mut words: Vec<Word> = Vec::new();
    for (g, line) in GROUPS.iter().enumerate() {
        for text in line.split_whitespace() {
            words.push(Word { group: g, text });
        }
    }
    let mut owned: Vec<Own> = Vec::new();
    for (g, group) in mine.iter().enumerate() {
        for text in group {
            owned.push(Own {
                group: base + g,
                text: text.clone(),
            });
        }
    }
    // **Longest first.** 什麼 holds a 麼, and without this the two groups both
    // fire on one word: the manuscript would be told it is inconsistent about
    // 麼 by a scan that had already counted the same character as 什麼.
    //
    // **The reader's own groups first among equals.** `sort_by_key` is stable,
    // so whichever side is chained first wins a spelling the two share — and
    // the built-ins winning meant a config of `裡 裏` could never be honoured:
    // both spellings landed in the built-in group, which is written `裏 裡`, so
    // the scan reported the opposite of what the reader had asked for. The
    // group numbering is untouched by this; only which group claims a hit is.
    let mut all: Vec<(usize, &str)> = owned
        .iter()
        .map(|w| (w.group, w.text.as_str()))
        .chain(words.iter().map(|w| (w.group, w.text)))
        .collect();
    all.sort_by_key(|(_, t)| std::cmp::Reverse(t.chars().count()));

    // Where each spelling was written, and how often.
    let mut hits: Vec<(usize, usize, usize, String)> = Vec::new(); // line, column, group, text
    for (line, body) in text.lines().enumerate() {
        let chars: Vec<char> = body.chars().collect();
        let mut at = 0;
        while at < chars.len() {
            let rest: String = chars[at..].iter().collect();
            match all.iter().find(|(_, t)| rest.starts_with(*t)) {
                Some(&(group, t)) => {
                    hits.push((line, at, group, t.to_string()));
                    at += t.chars().count();
                }
                None => at += 1,
            }
        }
    }

    // The manuscript's own answer, per group: what it writes most.
    let mut counts: std::collections::HashMap<(usize, String), usize> =
        std::collections::HashMap::new();
    for (_, _, group, text) in &hits {
        *counts.entry((*group, text.clone())).or_default() += 1;
    }
    let order = |group: usize, text: &str| -> usize {
        let line = match group < base {
            true => GROUPS[group].split_whitespace().map(str::to_string).collect(),
            false => mine[group - base].clone(),
        };
        line.iter().position(|w| w == text).unwrap_or(usize::MAX)
    };
    let mut winner: std::collections::HashMap<usize, (String, usize)> =
        std::collections::HashMap::new();
    for ((group, text), &n) in &counts {
        let better = match winner.get(group) {
            None => true,
            // More wins; on a tie the group's own order decides, which is why
            // the traditional spelling is written first in the table.
            Some((was, was_n)) => n > *was_n || (n == *was_n && order(*group, text) < order(*group, was)),
        };
        if better {
            winner.insert(*group, (text.clone(), n));
        }
    }

    let mut out: Vec<Slip> = Vec::new();
    for (line, column, group, text) in hits {
        let Some((instead, instead_count)) = winner.get(&group) else {
            continue;
        };
        if *instead == text {
            continue;
        }
        let written_count = counts.get(&(group, text.clone())).copied().unwrap_or(0);
        out.push(Slip {
            line,
            column,
            written: text,
            instead: instead.clone(),
            written_count,
            instead_count: *instead_count,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manuscript_that_never_wavers_is_never_asked() {
        let text = "那裏很冷。\n他站在裏面。\n";
        assert!(check(text, &[]).is_empty());
    }

    #[test]
    fn the_minority_spelling_is_the_one_reported() {
        let text = "那裏很冷。\n他站在裏面。\n她走進屋裡。\n";
        let slips = check(text, &[]);
        assert_eq!(slips.len(), 1, "{slips:?}");
        assert_eq!(slips[0].line, 2);
        assert_eq!(slips[0].written, "裡");
        assert_eq!(slips[0].instead, "裏");
        assert_eq!(slips[0].written_count, 1);
        assert_eq!(slips[0].instead_count, 2);
    }

    /// 什麼 holds a 麼, and the longer spelling is the one that counts.
    #[test]
    fn a_longer_spelling_wins_the_character_it_contains() {
        let text = "你說什麼？\n他說什麼。\n我說甚麼？\n";
        let slips = check(text, &[]);
        assert_eq!(slips.len(), 1, "{slips:?}");
        assert_eq!(slips[0].written, "甚麼");
        assert_eq!(slips[0].instead, "什麼");
    }

    /// A novel's own names are the whole point, and they come from the config.
    #[test]
    fn the_readers_own_names_are_checked_too() {
        let text = "阿嬌笑了。\n阿嬌走了。\n阿姣回頭。\n";
        let slips = check(text, &["阿嬌 阿姣".to_string()]);
        assert_eq!(slips.len(), 1, "{slips:?}");
        assert_eq!(slips[0].written, "阿姣");
        assert_eq!(slips[0].instead, "阿嬌");
        assert_eq!(slips[0].line, 2);
    }

    /// A tie is settled by the group's own order, not by whichever the hash
    /// map happened to hand back first.
    #[test]
    fn a_tie_goes_to_the_spelling_the_table_names_first() {
        let text = "那裏。\n那裡。\n";
        let slips = check("那裏。\n那裡。\n", &[]);
        assert_eq!(slips.len(), 1, "{text}: {slips:?}");
        assert_eq!(slips[0].instead, "裏");
        assert_eq!(slips[0].written, "裡");
    }

    /// The built-in table writes 「裏 裡」. A reader who prefers 裡 says so in
    /// the config, and *their* order is the one that must decide — with the
    /// built-ins scanned first this was silently impossible, and the scan told
    /// a 裡-writing manuscript to write 裏.
    #[test]
    fn the_readers_own_group_wins_a_spelling_the_table_also_holds() {
        let text = "那裡。\n那裡。\n那裏。\n";
        let slips = check(text, &["裡 裏".to_string()]);
        assert_eq!(slips.len(), 1, "{slips:?}");
        assert_eq!(slips[0].written, "裏");
        assert_eq!(slips[0].instead, "裡");
        assert_eq!(slips[0].line, 2);
    }

    /// …and a spelling only the reader's group names still lands in it, rather
    /// than being pulled into a built-in group that shares its other half.
    #[test]
    fn a_readers_group_keeps_a_spelling_a_built_in_group_shares() {
        let text = "檯燈。\n檯燈。\n台燈。\n";
        let slips = check(text, &["檯 台".to_string()]);
        assert_eq!(slips.len(), 1, "{slips:?}");
        assert_eq!(slips[0].written, "台");
        assert_eq!(slips[0].instead, "檯");
        assert_eq!(slips[0].line, 2);
    }

    /// A one-word group in a config is a line somebody is still writing.
    #[test]
    fn a_group_with_nothing_to_choose_between_is_dropped() {
        assert!(check("阿嬌阿嬌", &["阿嬌".to_string(), "".to_string()]).is_empty());
    }
}
