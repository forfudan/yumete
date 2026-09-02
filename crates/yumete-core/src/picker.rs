//! The picker behind `Space f` and `Space b` — Feature #90.
//!
//! A novel is a hundred files. Cycling through them with `gn` is not a way to
//! reach chapter 63; typing its whole path is not either. Helix's answer is a
//! picker: a list, a line to narrow it with, and Enter. This is that, with the
//! matching kept deliberately plain — a **subsequence** match, scored so that
//! letters found together and letters at the start of a word count for more.
//! Nobody needs a better algorithm to find `ch63` among a hundred chapters, and
//! a scoring function nobody can predict is worse than one that is merely
//! adequate.

/// What a picker offers, and what choosing it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A file to open, by path.
    File(String),
    /// One of the open buffers, by index.
    Buffer(usize, String),
}

impl Item {
    /// The text shown in the list, and matched against.
    pub fn label(&self) -> &str {
        match self {
            Item::File(path) => path,
            Item::Buffer(_, name) => name,
        }
    }
}

/// An open picker.
#[derive(Debug, Clone)]
pub struct Picker {
    /// What it is picking, for the prompt.
    pub title: &'static str,
    /// Everything it could offer, in the order it was gathered.
    items: Vec<Item>,
    /// What has been typed to narrow it.
    query: String,
    /// Which of the *matching* items is highlighted.
    selected: usize,
}

impl Picker {
    /// Open a picker over `items`.
    pub fn new(title: &'static str, items: Vec<Item>) -> Picker {
        Picker {
            title,
            items,
            query: String::new(),
            selected: 0,
        }
    }

    /// What has been typed so far.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// How many items there are in all.
    pub fn total(&self) -> usize {
        self.items.len()
    }

    /// The items matching the query, best first.
    ///
    /// Recomputed on each call rather than cached: a picker holds a few hundred
    /// paths, and being always right about what is on screen is worth more here
    /// than saving a scan.
    pub fn matches(&self) -> Vec<&Item> {
        if self.query.is_empty() {
            return self.items.iter().collect();
        }
        let mut scored: Vec<(i64, usize, &Item)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| score(item.label(), &self.query).map(|s| (s, i, item)))
            .collect();
        // Best score first; ties keep the order they were gathered in, which for
        // files is alphabetical and so is chapter order.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, _, item)| item).collect()
    }

    /// Which match is highlighted, clamped to what there is.
    pub fn selected(&self) -> usize {
        let count = self.matches().len();
        self.selected.min(count.saturating_sub(1))
    }

    /// The highlighted item, if the query matched anything.
    pub fn chosen(&self) -> Option<Item> {
        let matches = self.matches();
        matches.get(self.selected()).map(|&item| item.clone())
    }

    /// Add a character to the query. The highlight goes back to the top,
    /// because the list under it is a different list.
    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
    }

    /// Remove the last character, returning `false` when there was none — which
    /// is how Backspace on an empty query closes the picker.
    pub fn backspace(&mut self) -> bool {
        self.selected = 0;
        self.query.pop().is_some()
    }

    /// Move the highlight, wrapping at both ends.
    pub fn step(&mut self, down: bool) {
        let count = self.matches().len();
        if count == 0 {
            return;
        }
        let at = self.selected();
        self.selected = if down {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
    }
}

/// How well `label` matches `query`, or `None` when it does not.
///
/// A subsequence match: every character of the query must appear in the label,
/// in order. The score rewards two things a reader's eye also rewards — letters
/// found next to each other, and letters found at the start of a path segment —
/// so `c63` finds `卷二/ch63.md` above `ch6/notes3.md`.
fn score(label: &str, query: &str) -> Option<i64> {
    let haystack: Vec<char> = label.chars().flat_map(|c| c.to_lowercase()).collect();
    let needle: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    if needle.is_empty() {
        return Some(0);
    }
    let mut score = 0i64;
    let mut at = 0usize;
    let mut previous: Option<usize> = None;
    for want in needle {
        let found = (at..haystack.len()).find(|&i| haystack[i] == want)?;
        score += 1;
        if previous == Some(found.saturating_sub(1)) {
            // Adjacent to the last match: the query is a run, not a scatter.
            score += 8;
        }
        let starts_segment = found == 0
            || matches!(
                haystack[found - 1],
                '/' | '\\' | '_' | '-' | '.' | ' ' | '　'
            );
        if starts_segment {
            score += 4;
        }
        previous = Some(found);
        at = found + 1;
    }
    // A short label containing the query is a better answer than a long one.
    Some(score * 100 - haystack.len() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> Picker {
        Picker::new(
            "檔案",
            paths.iter().map(|p| Item::File(p.to_string())).collect(),
        )
    }

    #[test]
    fn an_empty_query_offers_everything_in_order() {
        let picker = files(&["ch01.md", "ch02.md"]);
        assert_eq!(picker.matches().len(), 2);
        assert_eq!(picker.chosen(), Some(Item::File("ch01.md".to_string())));
    }

    #[test]
    fn a_run_of_letters_beats_a_scatter() {
        let mut picker = files(&["卷二/ch63.md", "chapters/six/three.md"]);
        for c in "ch63".chars() {
            picker.push(c);
        }
        assert_eq!(
            picker.chosen(),
            Some(Item::File("卷二/ch63.md".to_string())),
            "the run should win"
        );
    }

    #[test]
    fn a_query_that_is_not_a_subsequence_matches_nothing() {
        let mut picker = files(&["ch01.md"]);
        for c in "zz".chars() {
            picker.push(c);
        }
        assert!(picker.matches().is_empty());
        assert_eq!(picker.chosen(), None);
    }

    #[test]
    fn the_highlight_wraps_and_survives_a_narrowing_query() {
        let mut picker = files(&["a.md", "b.md", "c.md"]);
        picker.step(true);
        assert_eq!(picker.selected(), 1);
        picker.step(false);
        picker.step(false);
        assert_eq!(picker.selected(), 2, "wraps at the top");

        // Typing narrows the list, and the highlight goes back to its head —
        // the item that was under it is not the item that is there now.
        picker.push('a');
        assert_eq!(picker.selected(), 0);
        assert_eq!(picker.chosen(), Some(Item::File("a.md".to_string())));
    }

    #[test]
    fn backspace_says_when_the_query_was_already_empty() {
        let mut picker = files(&["a.md"]);
        picker.push('a');
        assert!(picker.backspace());
        assert!(!picker.backspace(), "nothing left to delete");
    }

    #[test]
    fn cjk_paths_match_by_their_own_characters() {
        let mut picker = files(&["卷一/初雪.md", "卷二/驚蟄.md"]);
        picker.push('驚');
        assert_eq!(
            picker.chosen(),
            Some(Item::File("卷二/驚蟄.md".to_string()))
        );
    }
}
