//! Finding a command by what it does — Feature #224.
//!
//! 「你不可能打出一個你忘了名字的命令」，and every name in this editor is an
//! English word while the reader is thinking 「竖排模式」. `::` therefore
//! searches the **descriptions**, which already exist in three languages: the
//! `help` tag of every command and every word it takes is a `messages.toml`
//! entry with `zht` / `zhs` / `en`.
//!
//! ## Why two kinds of scoring rather than one distance
//!
//! One edit distance over the whole query is wrong for both halves of it. An
//! ASCII query is typed as an *abbreviation* — `lyt` for `layout`, `tbs` for
//! `table sort` — which is a subsequence, not a near-spelling; a CJK query is
//! typed in full and its unit is the **word**, which in a language with no
//! spaces means the character bigram (this is what `pg_trgm` and every CJK
//! analyser do). So the query is cut into runs by script and each run is
//! scored the way that script is typed, and the runs are added up.
//!
//! ## Why IDF
//!
//! 「模式」「命令」「the」「a」 are in half the descriptions and say nothing
//! about which one the reader wants; 「竖排」 is in three and says everything.
//! Both are two characters long, so length cannot tell them apart and a raw
//! overlap count would let the common word outvote the rare one. The corpus is
//! the table itself — 221 entries — so the weight is knowable exactly rather
//! than guessed.

use std::collections::HashMap;

/// Which field a match was found in, and what that is worth.
///
/// A name is what the reader will type next, so a hit there is the strongest
/// evidence; a `find` word was written *to be searched for* and is worth more
/// than a sentence that happens to contain the same characters.
pub const WEIGHT_NAME: f32 = 3.0;
pub const WEIGHT_FIND: f32 = 2.0;
pub const WEIGHT_HELP: f32 = 1.0;

/// Everything one row offers to a search.
#[derive(Debug, Clone, Copy, Default)]
pub struct Row<'a> {
    /// What has to be typed to run it — `layout vertical`.
    pub name: &'a str,
    /// Words searched but never shown: 「直排 縱書 tategaki columns」.
    pub find: &'a str,
    /// The description **in all three languages**, the one in force first.
    ///
    /// Searched in all of them, shown in the first. The table is already
    /// written three times over, and a reader who types 简体 into a 繁體
    /// editor — or the English name of a thing they know in Chinese — is
    /// asking a question this table can answer. 「竖排」 and 「竪排」 are two
    /// spellings of one word and only one of them is in the `zht` line.
    pub help: [&'a str; 3],
}

/// A run of the query, in one script.
#[derive(Debug, Clone, PartialEq)]
enum Run {
    /// Latin letters and digits, lower-cased: an abbreviation.
    Ascii(String),
    /// 漢字 and kana: whole words, scored by bigram.
    Wide(String),
}

/// Cut a query into runs by script, dropping the punctuation between them.
///
/// `竖排 layout` is two runs, and so is `竖排layout` — a reader typing Chinese
/// into a Latin word does not press space first.
fn runs(query: &str) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    // **A space ends a run.** 「排序 滾輪」 is two questions, and joining them
    // into one word would put 「序滾」 in the middle — a bigram nobody typed,
    // and the whole query would then be answered by a row that holds only its
    // first half.
    let mut broke = true;
    for ch in query.chars() {
        let wide = is_wide(ch);
        if !(wide || ch.is_alphanumeric()) {
            broke = true;
            continue;
        }
        let ch = match wide {
            true => ch,
            false => ch.to_ascii_lowercase(),
        };
        match out.last_mut() {
            Some(Run::Wide(s)) if wide && !broke => s.push(ch),
            Some(Run::Ascii(s)) if !wide && !broke => s.push(ch),
            _ => out.push(match wide {
                true => Run::Wide(ch.to_string()),
                false => Run::Ascii(ch.to_string()),
            }),
        }
        broke = false;
    }
    out
}

/// A character written full-width: 漢字, kana, and the CJK punctuation between
/// them. Everything else — Latin, digits, Cyrillic — is scored as a word.
fn is_wide(ch: char) -> bool {
    matches!(ch as u32,
        0x2E80..=0x303F | 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF | 0xFF00..=0xFF60 | 0x20000..=0x3FFFF)
}

/// The searchable tokens of a text: ASCII words and CJK bigrams.
///
/// The two kinds live in one bag because they are counted for the same
/// purpose — how many of the 221 rows say this — and nothing ever compares an
/// ASCII token with a wide one.
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for run in runs(text) {
        match run {
            Run::Ascii(word) => out.push(word),
            Run::Wide(word) => {
                let chars: Vec<char> = word.chars().collect();
                match chars.len() {
                    // A lone character is its own token: 「表」 in a query is
                    // still a question, and dropping it would answer nothing.
                    1 => out.push(word),
                    _ => out.extend(chars.windows(2).map(|w| w.iter().collect::<String>())),
                }
            }
        }
    }
    out
}

/// How rare each token is across the rows, as `ln(N / df)`.
#[derive(Debug, Default, Clone)]
pub struct Idf {
    weight: HashMap<String, f32>,
    /// What an unseen token is worth — the weight of a token in one row only,
    /// because that is what a token nobody has written would be.
    unseen: f32,
}

impl Idf {
    /// Count the corpus. Every row is one document, its three fields joined.
    pub fn of<'a>(rows: impl IntoIterator<Item = Row<'a>>) -> Idf {
        let mut df: HashMap<String, usize> = HashMap::new();
        let mut n = 0usize;
        for row in rows {
            n += 1;
            let text = format!("{} {} {}", row.name, row.find, row.help.join(" "));
            let mut seen: Vec<String> = tokens(&text);
            seen.sort();
            seen.dedup();
            for token in seen {
                *df.entry(token).or_default() += 1;
            }
        }
        let n = n.max(1) as f32;
        Idf {
            weight: df.iter().map(|(k, &d)| (k.clone(), (n / d as f32).ln().max(0.0))).collect(),
            unseen: n.ln(),
        }
    }

    /// What this token is worth. Never zero — a token in *every* row still has
    /// to carry a little, or a query made only of common words would score the
    /// whole table at nothing and answer with the first row alphabetically.
    fn at(&self, token: &str) -> f32 {
        self.weight.get(token).copied().unwrap_or(self.unseen).max(0.05)
    }
}

/// The score of one row against one query, and `None` when it does not match.
///
/// **Every run has to land somewhere.** A reader who typed 「竖排 sort」 asked
/// about both, and a row that only answers one of them is not an answer.
pub fn score(query: &str, row: Row<'_>, idf: &Idf) -> Option<f32> {
    let runs = runs(query);
    if runs.is_empty() {
        return None;
    }
    let mut total = 0.0;
    for run in &runs {
        let fields = [(row.name, WEIGHT_NAME), (row.find, WEIGHT_FIND)]
            .into_iter()
            .chain(row.help.iter().map(|&h| (h, WEIGHT_HELP)));
        let best = fields
            .filter(|(text, _)| !text.is_empty())
            .filter_map(|(text, weight)| run_score(run, text, idf).map(|s| s * weight))
            .fold(0.0f32, f32::max);
        if best <= 0.0 {
            return None;
        }
        total += best;
    }
    Some(total)
}

/// One run against one field.
fn run_score(run: &Run, text: &str, idf: &Idf) -> Option<f32> {
    match run {
        Run::Ascii(word) => ascii_score(word, text, idf),
        Run::Wide(word) => wide_score(word, text, idf),
    }
}

/// An abbreviation against a text: fzf's shape, kept small.
///
/// Every character of the run has to appear, in order. What is earned on top
/// of that is where they appeared: at the start of a word (`lyt` → **l**a**y**
/// ou**t** is worth less than `lay` → **lay**out), and next to each other.
///
/// **Then divided by how far it had to reach.** fzf matches paths; this
/// matches sentences, and a sentence is long enough that six letters find six
/// different words to start — `layout` "matched" the English 「a **l**esson:
/// the text is copied into **a** file of **you**r own, and you learn by edi**t**ing
/// it」 on word starts alone and put `:tutor` above `:layout` itself. So the
/// bonuses are scaled by the run's length against the number of words it
/// crossed: an initialism (`lv` → **l**ayout **v**ertical) crosses one word
/// per character and keeps all of it, a scatter across a whole sentence keeps
/// a fraction.
fn ascii_score(word: &str, text: &str, idf: &Idf) -> Option<f32> {
    let hay: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let needle: Vec<char> = word.chars().collect();
    let mut at = 0usize;
    let mut earned = 0.0f32;
    let mut previous: Option<usize> = None;
    let mut reach: Option<usize> = None;
    for &want in &needle {
        let found = (at..hay.len()).find(|&i| hay[i] == want)?;
        earned += 1.0;
        reach.get_or_insert(found);
        if previous == Some(found.saturating_sub(1)) && found > 0 {
            earned += 0.8;
        }
        let starts = found == 0 || !hay[found - 1].is_alphanumeric();
        if starts {
            earned += 1.0;
        }
        previous = Some(found);
        at = found + 1;
    }
    // Normalised so a long run cannot outscore a short one merely by being
    // long, then weighted by how rare the whole word is — `t` is in nearly
    // every description, `tategaki` in one.
    let full = needle.len() as f32 * 2.8;
    let words = match (reach, previous) {
        (Some(first), Some(last)) => (first..=last)
            .filter(|&i| i == 0 || !hay[i - 1].is_alphanumeric())
            .count()
            .max(1),
        _ => 1,
    };
    let dense = (needle.len() as f32 / words as f32).min(1.0);
    Some(earned / full * dense * idf.at(word).max(0.3))
}

/// A CJK run against a text: the overlap of their bigrams, plus a bonus for
/// the longest stretch they actually share.
///
/// The overlap alone cannot tell 「竖排模式」 in one order from another, and a
/// reader who typed a whole word means the word: the substring bonus is what
/// puts an exact 「竖排」 above a row that merely holds 竖 and 排 apart.
fn wide_score(word: &str, text: &str, idf: &Idf) -> Option<f32> {
    let wanted = tokens(word);
    if wanted.is_empty() {
        return None;
    }
    let held: Vec<String> =
        tokens(text).into_iter().filter(|t| t.chars().next().is_some_and(is_wide)).collect();
    let mut shared = 0.0f32;
    let mut asked = 0.0f32;
    for token in &wanted {
        let weight = idf.at(token);
        asked += weight;
        if held.iter().any(|h| h == token) {
            shared += weight;
        }
    }
    if shared <= 0.0 {
        return None;
    }
    let overlap = shared / asked.max(f32::EPSILON);
    let run: Vec<char> = word.chars().collect();
    let hay: Vec<char> = text.chars().collect();
    let (longest, at) = longest_common(&run, &hay);
    let base = overlap * 0.7 + longest as f32 / run.len() as f32 * 0.3;
    // **Rarity is the query's, not the row's.** Every row that holds the word
    // holds it equally, so within one search this is a constant — it is there
    // so that in 「表格 竖排」 the run nobody else says outweighs the run half
    // the table says, which is the whole reason for counting the corpus.
    let rarity = asked / wanted.len() as f32;
    // What tells two rows that both hold the word apart: **how much of the row
    // is the word** (「滾輪一格走多遠」 is about 滾輪; a sentence that mentions
    // it in passing is not), and **how early it comes**.
    let field = hay.len().max(1) as f32;
    let covered = 1.0 + 0.3 * (longest as f32 / field).min(1.0);
    let early = 1.0 - 0.25 * (at as f32 / field);
    Some(base * rarity * covered * early)
}

/// The longest run of characters the two share: how long, and where in `b` it
/// starts.
fn longest_common(a: &[char], b: &[char]) -> (usize, usize) {
    let (mut best, mut at) = (0usize, 0usize);
    let mut row = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        let mut previous = 0usize;
        for j in 1..=b.len() {
            let here = row[j];
            row[j] = match a[i - 1] == b[j - 1] {
                true => previous + 1,
                false => 0,
            };
            if row[j] > best {
                best = row[j];
                at = j - row[j];
            }
            previous = here;
        }
    }
    (best, at)
}

/// Damerau-Levenshtein, capped: `Some(distance)` while it is at most `cap`.
///
/// For the **name alone**, and for a typo alone: `:laoyut` is a slip of the
/// fingers, and it is the one case a subsequence match cannot catch, because
/// the letters are no longer in order.
pub fn typo(query: &str, name: &str, cap: usize) -> Option<usize> {
    let a: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let b: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    if a.len().abs_diff(b.len()) > cap {
        return None;
    }
    let mut grid = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, cell) in grid.iter_mut().enumerate() {
        cell[0] = i;
    }
    for j in 0..=b.len() {
        grid[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (grid[i - 1][j] + 1).min(grid[i][j - 1] + 1).min(grid[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(grid[i - 2][j - 2] + 1);
            }
            grid[i][j] = best;
        }
    }
    Some(grid[a.len()][b.len()]).filter(|&d| d <= cap)
}

/// One command the `::` line turned up, and how well it answered.
#[derive(Debug, Clone)]
pub struct Hit {
    /// The command itself — what ⇥ writes back into the `:` line.
    pub choice: crate::command::Choice,
    pub score: f32,
}

/// What a typo on the name alone is worth.
///
/// Below a row that really holds the word — a name match runs to 7 or 8 — and
/// above the scatter. Being one letter from a command's **name** is a strong
/// thing to be: the branch only runs when nothing in the row matched at all,
/// on a Latin query of three characters or more. It was 0.6 once, which put
/// `:layout` below every sentence that happened to hold `l…a…o…y…u…t` in
/// order — the one row the reader meant, last.
fn typo_score(distance: usize) -> f32 {
    3.0 - 0.8 * distance as f32
}

/// Whether a query is even the kind of thing that can be a typo of a name.
///
/// **Latin, and at least three characters of it.** 「竖排」 is two characters
/// and every two-letter command name is two edits away from it, so the tail of
/// a perfectly good Chinese search filled up with `:sh` and `:wq` — names it
/// has nothing to do with. A Chinese query is not a misspelling of an English
/// word; it is a different question, and the bigram scorer is the one that
/// answers it.
fn worth_a_typo_check(query: &str) -> bool {
    query.chars().count() >= 3 && query.chars().all(|c| c.is_ascii() && !c.is_whitespace())
}

/// How many hits the `::` line will show.
///
/// The scorer will rank all 221 of them and the tail is noise — a reader who
/// has not found it in thirty rows types another character instead of
/// scrolling.
pub const SHOWN: usize = 30;

/// Every command and every word they take, scored against `query`, best first.
///
/// The whole table on every keystroke: 221 rows against a query of a few
/// characters is microseconds, so there is no index to build, keep or
/// invalidate. The IDF over that corpus **is** cached, because it does not
/// depend on the query.
pub fn look(query: &str) -> Vec<Hit> {
    let choices = crate::command::all_choices();
    let table = crate::messages::table();
    // The description in all three languages, the one in force first: searched
    // in all of them, shown in the first. A reader who types 简体 into a 繁體
    // editor, or the English name of a thing they know in Chinese, is asking a
    // question this table can answer.
    let said = |tag: &'static str| -> [&'static str; 3] {
        let Some(e) = table.get(tag) else {
            return ["", "", ""];
        };
        match crate::messages::language() {
            crate::messages::Language::Traditional => [e.zht, e.zhs, e.en],
            crate::messages::Language::Simplified => [e.zhs, e.zht, e.en],
            crate::messages::Language::English => [e.en, e.zht, e.zhs],
        }
    };
    let names: Vec<String> = choices.iter().map(|c| c.written()).collect();
    let rows: Vec<Row<'_>> = choices
        .iter()
        .zip(&names)
        .map(|(c, name)| Row {
            name,
            find: table.get(c.help).map(|e| e.find).unwrap_or_default(),
            help: said(c.help),
        })
        .collect();
    let idf = idf_of(&rows);
    let mut hits: Vec<Hit> = choices
        .iter()
        .zip(&rows)
        .filter_map(|(choice, row)| {
            let score = score(query, *row, &idf)
                .or_else(|| match worth_a_typo_check(query) {
                    true => typo(query, row.name, 2).map(typo_score),
                    false => None,
                })
                .filter(|s| *s > 0.0)?;
            Some(Hit { choice: choice.clone(), score })
        })
        .collect();
    // Descending, and **by name** where two rows tie: a list that reshuffles
    // itself between two keystrokes that scored the same is a list nobody can
    // point at.
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.choice.written().cmp(&b.choice.written()))
    });
    hits.truncate(SHOWN);
    hits
}

/// The corpus weights, counted once.
///
/// The rows are the command table, which is compiled in — it cannot change
/// while the editor runs, and counting it on every keystroke of a search is
/// the one part of this that is not free.
fn idf_of(rows: &[Row<'_>]) -> &'static Idf {
    static ONCE: std::sync::OnceLock<Idf> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| Idf::of(rows.iter().copied()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Vec<Row<'static>> {
        vec![
            Row { name: "layout vertical", find: "直排 縱書 tategaki columns", help: ["把整頁轉成竖排模式，一行一行從右往左", "", ""] },
            Row { name: "layout horizontal", find: "橫排 yokogaki", help: ["橫排模式，照舊從左往右", "", ""] },
            Row { name: "table sort", find: "排序 order", help: ["照這幾欄排；t1a2d8as 是鍵盤上的同一件事", "", ""] },
            Row { name: "table new", find: "", help: ["寫一張空表，首行是欄名", "", ""] },
            Row { name: "wheel", find: "滾輪", help: ["滾輪一格走多遠", "", ""] },
            Row { name: "quit", find: "", help: ["關掉這一個", "", ""] },
        ]
    }

    #[test]
    fn a_chinese_query_finds_the_command_whose_name_is_english() {
        let rows = corpus();
        let idf = Idf::of(rows.clone());
        let mut ranked: Vec<(f32, &str)> = rows
            .iter()
            .filter_map(|r| score("竖排", *r, &idf).map(|s| (s, r.name)))
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        assert_eq!(ranked.first().map(|r| r.1), Some("layout vertical"), "{ranked:?}");
    }

    #[test]
    fn a_word_written_only_to_be_searched_for_counts() {
        let rows = corpus();
        let idf = Idf::of(rows.clone());
        for query in ["直排", "縱書", "tategaki"] {
            let mut ranked: Vec<(f32, &str)> = rows
                .iter()
                .filter_map(|r| score(query, *r, &idf).map(|s| (s, r.name)))
                .collect();
            ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
            assert_eq!(ranked.first().map(|r| r.1), Some("layout vertical"), "{query}: {ranked:?}");
        }
    }

    #[test]
    fn an_abbreviation_finds_the_word_it_abbreviates() {
        let rows = corpus();
        let idf = Idf::of(rows.clone());
        let best = |q: &str| {
            let mut ranked: Vec<(f32, &str)> = rows
                .iter()
                .filter_map(|r| score(q, *r, &idf).map(|s| (s, r.name)))
                .collect();
            ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
            ranked.first().map(|r| r.1).unwrap_or("—")
        };
        assert_eq!(best("lyt"), "layout vertical");
        assert_eq!(best("tbsrt"), "table sort");
    }

    #[test]
    fn every_run_of_the_query_has_to_land() {
        let rows = corpus();
        let idf = Idf::of(rows.clone());
        // 「排序」 is in one row and 「滾輪」 in another: nothing answers both.
        assert!(rows.iter().all(|r| score("排序 滾輪", *r, &idf).is_none()));
        // …but both halves of one row's own words do.
        let sort = rows[2];
        assert!(score("排序 table", sort, &idf).is_some());
    }

    #[test]
    fn a_common_word_does_not_outvote_a_rare_one() {
        let rows = corpus();
        let idf = Idf::of(rows.clone());
        // 「模式」 is in two rows, 「滾輪」 in one: the rare one has to weigh more.
        assert!(idf.at("滾輪") > idf.at("模式"), "{} vs {}", idf.at("滾輪"), idf.at("模式"));
    }

    #[test]
    fn a_typo_in_a_name_is_still_that_name() {
        assert_eq!(typo("laoyut", "layout", 2), Some(1), "a transposition is one slip, not two");
        assert_eq!(typo("tbale", "table", 2), Some(1));
        assert_eq!(typo("quit", "wheel", 2), None);
    }

    #[test]
    fn an_empty_query_answers_nothing() {
        let idf = Idf::of(corpus());
        assert!(score("", corpus()[0], &idf).is_none());
        assert!(score("  ", corpus()[0], &idf).is_none());
    }
}
