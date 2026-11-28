//! **Who last wrote the cursor's line** (`:view-blame`): their name, how long
//! ago and the commit's summary, drawn dim after the end of the line.
//!
//! ## One `git blame` per answer, never one per key
//!
//! It is the same bargain as [`crate::vcs`]: `git` is asked when the switch is
//! turned on, when the file is opened, read again or saved, and once for a
//! buffer the switch finds without an answer. Drawing only looks the answer
//! up. Between two answers the lines move with the edits
//! ([`Blame::shift`], fed by the buffer's line edits as the diagnostics are),
//! and each line keeps a fingerprint of the text it was blamed for, so a line
//! typed into since reads as not committed without asking again.
//!
//! ## The buffer, not the file on disk
//!
//! `git blame --porcelain --contents -` is handed the buffer's text, in the
//! encoding the file is written in: the answer's line numbers are the
//! buffer's own, unsaved edits included. Blaming the file on disk would pin
//! every answer to the saved lines, and the first line inserted above the
//! cursor would put the next commit's name on the wrong line. Lines git has
//! no commit for (the buffer's own) come back under the all-zero hash and are
//! kept as not committed.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;

use crate::say;

/// One commit, as the porcelain introduces it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub author: String,
    /// With its angle brackets, as git writes it.
    pub mail: String,
    /// Seconds since 1970.
    pub time: i64,
    /// The message's first line.
    pub summary: String,
}

/// What one line says: committed by one of [`Blame::commits`], or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Line {
    /// The fingerprint of the text git was asked about; `None` for a line
    /// that came into the buffer after the answer.
    text: Option<u64>,
    /// An index into `commits`; `None` for a line git has no commit for.
    commit: Option<usize>,
}

/// What a line's blame comes to, for drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Said<'a> {
    Committed(&'a Commit),
    NotCommitted,
}

/// A file's blame, line by line, in the buffer's own line numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blame {
    commits: Vec<Commit>,
    lines: Vec<Line>,
}

/// The fingerprint a line is compared by: its text without the line break.
pub fn fingerprint(line: &str) -> u64 {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

impl Blame {
    /// Ask git about `path`, handing it `bytes` as the file's contents and
    /// `text` (the same, decoded) for the fingerprints. `None` when git has
    /// nothing to say: outside a repository, a file it does not track, a
    /// repository with no commit yet, or no git on the machine.
    pub fn read(path: &Path, bytes: &[u8], text: &str) -> Option<Blame> {
        use std::io::Write;
        let here = Path::new(".");
        let dir = match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => here,
        };
        let mut child = std::process::Command::new("git")
            .args(["--no-optional-locks", "blame", "--porcelain", "--contents", "-", "--"])
            .arg(path.file_name()?)
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut stdin = child.stdin.take()?;
        // Written beside the read: a long file fills the pipe both ways, and
        // git refusing early closes it, which is not an error here.
        let out = std::thread::scope(|scope| {
            scope.spawn(move || {
                let _ = stdin.write_all(bytes);
            });
            child.wait_with_output()
        })
        .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(Blame::parse(&String::from_utf8_lossy(&out.stdout), text))
    }

    /// `git blame --porcelain` read against `text`, the contents it was asked
    /// about. A commit's headers come only the first time it is named; every
    /// line is a header line `<hash> <old> <new> [<count>]` and the line itself
    /// after a tab.
    pub fn parse(porcelain: &str, text: &str) -> Blame {
        let mut lines: Vec<Line> =
            text.split('\n').map(|l| Line { text: Some(fingerprint(l)), commit: None }).collect();
        let mut commits: Vec<Commit> = Vec::new();
        let mut known: HashMap<String, Option<usize>> = HashMap::new();
        let mut current: Option<usize> = None;
        let mut target: Option<usize> = None;
        for row in porcelain.lines() {
            if row.starts_with('\t') {
                if let Some(line) = target.take().and_then(|at| lines.get_mut(at)) {
                    line.commit = current;
                }
                continue;
            }
            let mut words = row.split(' ');
            let head = words.next().unwrap_or("");
            let is_hash = matches!(head.len(), 40 | 64) && head.bytes().all(|b| b.is_ascii_hexdigit());
            if is_hash {
                let new_line = words.nth(1).and_then(|n| n.parse::<usize>().ok());
                if let Some(new_line) = new_line {
                    current = *known.entry(head.to_string()).or_insert_with(|| {
                        (!head.bytes().all(|b| b == b'0')).then(|| {
                            commits.push(Commit { hash: head.to_string(), ..Commit::default() });
                            commits.len() - 1
                        })
                    });
                    target = new_line.checked_sub(1);
                    continue;
                }
            }
            let Some(commit) = current.and_then(|i| commits.get_mut(i)) else { continue };
            let rest = row.get(head.len() + 1..).unwrap_or("");
            match head {
                "author" => commit.author = rest.to_string(),
                "author-mail" => commit.mail = rest.to_string(),
                "author-time" => commit.time = rest.parse().unwrap_or(0),
                "summary" => commit.summary = rest.to_string(),
                _ => {}
            }
        }
        // `text` ending in a line break has an empty last line git never
        // names: it is the buffer's, not a commit's.
        if text.ends_with('\n') {
            if let Some(last) = lines.last_mut() {
                last.text = None;
            }
        }
        Blame { commits, lines }
    }

    /// What line `line` says, given the text it holds now. A line edited
    /// since the answer, or added since, is not committed; `None` past the
    /// lines the answer knows.
    pub fn at(&self, line: usize, now: &str) -> Option<Said<'_>> {
        let entry = self.lines.get(line)?;
        match (entry.text, entry.commit) {
            (Some(text), Some(commit)) if text == fingerprint(now) => {
                self.commits.get(commit).map(Said::Committed)
            }
            _ => Some(Said::NotCommitted),
        }
    }

    /// Move the lines through an edit, in the buffer's line-edit terms: from
    /// `line` (or the line after it, when the edit began inside it) `taken`
    /// lines became `put`. The lines put are new, so not committed.
    pub fn shift(&mut self, line: usize, taken: usize, put: usize, at_start: bool) {
        let first = (line + usize::from(!at_start)).min(self.lines.len());
        let end = (first + taken).min(self.lines.len());
        let new = Line { text: None, commit: None };
        self.lines.splice(first..end, std::iter::repeat_n(new, put));
    }
}

/// The words drawn after the line: `author, 3 days ago • summary`, or that
/// nobody has committed it. `now` is seconds since 1970.
pub fn words(said: Said<'_>, now: i64) -> String {
    match said {
        Said::Committed(c) => say!("blame.line", c.author, ago(c.time, now), c.summary),
        Said::NotCommitted => say!("blame.not-committed"),
    }
}

/// Seconds since 1970, now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// How long ago `then` was, at `now` (both seconds since 1970), in words.
pub fn ago(then: i64, now: i64) -> String {
    let seconds = (now - then).max(0);
    let (minute, hour, day) = (60, 3600, 86_400);
    match seconds {
        s if s < minute => say!("blame.just-now"),
        s if s < hour => say!("blame.minutes-ago", s / minute),
        s if s < day => say!("blame.hours-ago", s / hour),
        s if s < 30 * day => say!("blame.days-ago", s / day),
        s if s < 365 * day => say!("blame.months-ago", s / (30 * day)),
        s => say!("blame.years-ago", s / (365 * day)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "1111111111111111111111111111111111111111";
    const TWO: &str = "2222222222222222222222222222222222222222";
    const NONE: &str = "0000000000000000000000000000000000000000";

    fn porcelain() -> String {
        [
            format!("{ONE} 1 1 1"),
            "author Lin Qiu".to_string(),
            "author-mail <qiu@example.org>".to_string(),
            "author-time 1700000000".to_string(),
            "author-tz +0800".to_string(),
            "summary Plant the orchard".to_string(),
            "filename notes.md".to_string(),
            "\tapples by the wall".to_string(),
            format!("{NONE} 2 2 1"),
            "author Not Committed Yet".to_string(),
            "summary Version of notes.md from -".to_string(),
            "filename notes.md".to_string(),
            "\tpears, not yet".to_string(),
            format!("{TWO} 2 3 1"),
            "author Mo Shan".to_string(),
            "author-mail <shan@example.org>".to_string(),
            "author-time 1710000000".to_string(),
            "summary Water the plums".to_string(),
            "filename notes.md".to_string(),
            "\tplums in the shade".to_string(),
            format!("{ONE} 3 4"),
            "\tcherries at the gate".to_string(),
        ]
        .join("\n")
    }

    const TEXT: &str = "apples by the wall\npears, not yet\nplums in the shade\ncherries at the gate\n";

    #[test]
    fn porcelain_is_read_line_by_line() {
        let blame = Blame::parse(&porcelain(), TEXT);
        let Some(Said::Committed(first)) = blame.at(0, "apples by the wall") else { panic!() };
        assert_eq!((first.author.as_str(), first.summary.as_str()), ("Lin Qiu", "Plant the orchard"));
        assert_eq!((first.mail.as_str(), first.time), ("<qiu@example.org>", 1_700_000_000));
        assert_eq!(blame.at(1, "pears, not yet"), Some(Said::NotCommitted));
        let Some(Said::Committed(third)) = blame.at(2, "plums in the shade") else { panic!() };
        assert_eq!(third.author, "Mo Shan");
        // A commit named a second time comes without its headers.
        let Some(Said::Committed(fourth)) = blame.at(3, "cherries at the gate") else { panic!() };
        assert_eq!(fourth.hash, ONE);
        // The empty line after the last break, and nothing past it.
        assert_eq!(blame.at(4, ""), Some(Said::NotCommitted));
        assert_eq!(blame.at(5, ""), None);
    }

    #[test]
    fn a_line_typed_into_is_not_committed() {
        let blame = Blame::parse(&porcelain(), TEXT);
        assert_eq!(blame.at(0, "apples by the old wall"), Some(Said::NotCommitted));
        // The line break and a CRLF are not the text.
        assert!(matches!(blame.at(0, "apples by the wall\r\n"), Some(Said::Committed(_))));
    }

    #[test]
    fn lines_move_with_the_edits() {
        let mut blame = Blame::parse(&porcelain(), TEXT);
        // A line opened above the third: it is new, and the plums move down.
        blame.shift(2, 0, 1, true);
        assert_eq!(blame.at(2, "a new row"), Some(Said::NotCommitted));
        assert!(matches!(blame.at(3, "plums in the shade"), Some(Said::Committed(c)) if c.author == "Mo Shan"));
        // The first line taken out: everything comes up one.
        blame.shift(0, 1, 0, true);
        assert_eq!(blame.at(0, "pears, not yet"), Some(Said::NotCommitted));
        assert!(matches!(blame.at(2, "plums in the shade"), Some(Said::Committed(_))));
        // A line split in the middle: the line after it is new.
        blame.shift(2, 0, 1, false);
        assert!(matches!(blame.at(4, "cherries at the gate"), Some(Said::Committed(_))));
        assert_eq!(blame.at(3, "in the shade"), Some(Said::NotCommitted));
    }

    #[test]
    fn how_long_ago_picks_the_unit() {
        let day = 86_400;
        assert_eq!(ago(100, 130), say!("blame.just-now"));
        assert_eq!(ago(0, 120), say!("blame.minutes-ago", 2));
        assert_eq!(ago(0, 3600), say!("blame.hours-ago", 1));
        assert_eq!(ago(0, 3 * day), say!("blame.days-ago", 3));
        assert_eq!(ago(0, 65 * day), say!("blame.months-ago", 2));
        assert_eq!(ago(0, 800 * day), say!("blame.years-ago", 2));
        // A commit dated ahead of the clock is not in the future.
        assert_eq!(ago(500, 100), say!("blame.just-now"));
    }
}
