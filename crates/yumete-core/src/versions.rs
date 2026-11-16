//! **Local history**: the versions of a manuscript its saves left behind.
//!
//! A recovery copy answers 「the editor died before I saved」; git answers
//! 「what did I commit」. Neither answers the question a writer asks most:
//! 「what did this chapter say on Tuesday, before I cut the scene」. So every
//! save keeps a copy of what it wrote, as VS Code's and JetBrains' local
//! history do, and the copies are thinned the way Time Machine thins backups:
//! all of today, one an hour for a week, one a day for two months.
//!
//! **Layout.** A *store* is a directory of *shelves*, one shelf per file:
//!
//! ```text
//! <book>/.yumete/history/          (or <data dir>/history/ outside a book)
//!   第一章.md-3fa2b1c9e0d4a6b8/
//!     index.tsv                    one line per version
//!     1763270000.zst               the whole text, zstd-compressed
//! ```
//!
//! **A shelf is named after its file and a hash of the file's path**, not
//! after the path itself. Mirroring the path (`history/卷一/第一章.md/`) reads
//! well in a file manager, and breaks in three ways: a shelf directory and a
//! later file of the same name collide (`a.md` deleted, `a.md/b.md` created),
//! an absolute path outside a book has no portable spelling (`C:` is not a
//! folder name), and a rename has to move a tree of folders instead of one.
//! The file name at the front keeps the shelf findable by eye; the path
//! itself is written on every index line.
//!
//! **Whole copies, compressed**, not deltas: a manuscript of 300 KB
//! compresses to a fraction of that, sixty-odd copies are a few megabytes,
//! and any one of them can be read without replaying the others — a damaged
//! file costs one version, never every version after it.

use std::io;
use std::path::{Path, PathBuf};

/// Saves this close to the newest version replace it instead of adding
/// another (VS Code merges within 10 seconds; a writer saves by habit, so the
/// window is longer).
pub const MERGE_WINDOW: i64 = 10 * 60;

/// Each shelf's list of versions.
const INDEX: &str = "index.tsv";

/// The index's first line: what it is, and which layout.
const HEADER: &str = "# yumete history 1\tsecs\tfile\tbytes\tchars\tdigest\tlabel\tpath";

/// How fast against how small. Level 3 is zstd's own default: a 300 KB
/// chapter in a few milliseconds, at about a third of its size.
const LEVEL: i32 = 3;

/// One kept version of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// When it was saved, in seconds since 1970.
    pub secs: i64,
    /// Its file in the shelf.
    pub file: String,
    /// Its size in UTF-8 bytes.
    pub bytes: usize,
    /// How many characters it holds. **The ± a list shows is worked out from
    /// this**, against the version before it, rather than stored: thinning
    /// takes versions out of the middle, and a stored difference would then
    /// describe a neighbour that is gone.
    pub chars: usize,
    /// [`digest`] of its text — what tells a save that changed nothing.
    pub digest: u64,
    /// The name given with `:history-label`, or empty.
    pub label: String,
    /// The file's path when it was saved: relative to the book, or absolute
    /// outside one. Kept per version, so a shelf that followed a rename still
    /// says where each version came from.
    pub path: String,
}

/// What [`Shelf::keep`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// The text is the newest version's: nothing written.
    Same,
    /// A new version.
    Added,
    /// The newest version was replaced, being inside [`MERGE_WINDOW`].
    Merged,
}

/// **A stable 64-bit hash** (FNV-1a) — the same answer on every machine and
/// every build, since it is written to disk. Not `DefaultHasher`, whose
/// algorithm the standard library is free to change.
pub fn digest(text: &str) -> u64 {
    text.bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The folder a file's versions live in, from its key (its path relative to
/// the book, or absolute): the file's own name, made safe for any file
/// system, then a hash of the whole key.
pub fn shelf_name(key: &str) -> String {
    let name = key.rsplit(['/', '\\']).next().unwrap_or(key);
    let safe: String = name
        .chars()
        .map(|c| match c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
            true => c,
            false => '_',
        })
        .take(40)
        .collect();
    format!("{safe}-{:016x}", digest(key))
}

/// One file's versions.
#[derive(Debug, Clone)]
pub struct Shelf {
    dir: PathBuf,
}

impl Shelf {
    /// The shelf for `key` in `store`, whether or not it exists yet.
    pub fn of(store: &Path, key: &str) -> Shelf {
        Shelf { dir: store.join(shelf_name(key)) }
    }

    /// The shelf's folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether anything has been kept here.
    pub fn exists(&self) -> bool {
        self.dir.join(INDEX).is_file()
    }

    /// Every version, oldest first. A line that cannot be read is skipped,
    /// not fatal: one damaged line must not hide every other version.
    pub fn versions(&self) -> Vec<Version> {
        let Ok(text) = std::fs::read_to_string(self.dir.join(INDEX)) else {
            return Vec::new();
        };
        let mut all: Vec<Version> = text.lines().filter(|l| !l.starts_with('#')).filter_map(parse_line).collect();
        all.sort_by_key(|v| v.secs);
        all
    }

    /// Where a version's text is on disk.
    pub fn path_of(&self, version: &Version) -> PathBuf {
        self.dir.join(&version.file)
    }

    /// A version's text.
    pub fn text_of(&self, version: &Version) -> io::Result<String> {
        read_text(&self.path_of(version))
    }

    /// **Keep `text` as the newest version**, then thin the shelf.
    ///
    /// Nothing is written when `text` is the newest version's already. Within
    /// [`MERGE_WINDOW`] of the newest version the new one replaces it —
    /// unless that one has a label: a name was given to *that* text, and
    /// carrying it over to later text would make the name lie.
    ///
    /// `local` turns a second into its local day and hour, for the thinning
    /// ([`thin`]).
    pub fn keep(&self, text: &str, path: &str, now: i64, local: &dyn Fn(i64) -> (i64, u32)) -> io::Result<Kept> {
        let mut all = self.versions();
        let sum = digest(text);
        if all.last().is_some_and(|v| v.digest == sum) {
            return Ok(Kept::Same);
        }
        let merge = all
            .last()
            .is_some_and(|v| v.label.is_empty() && (0..MERGE_WINDOW).contains(&(now - v.secs)));
        let fresh = self.add_file(text, path, now, &all)?;
        let replaced = match merge {
            true => all.pop(),
            false => None,
        };
        all.push(fresh);
        self.settle(all, replaced.into_iter().collect(), now, local)?;
        Ok(match merge {
            true => Kept::Merged,
            false => Kept::Added,
        })
    }

    /// **Name the newest version** (`:history-label`), keeping `text` as a
    /// version of its own first when the newest is not that text — a label
    /// is a name for exactly what the writer is looking at.
    ///
    /// Answers the version that took the name.
    pub fn label(
        &self,
        text: &str,
        path: &str,
        label: &str,
        now: i64,
        local: &dyn Fn(i64) -> (i64, u32),
    ) -> io::Result<Version> {
        let mut all = self.versions();
        let sum = digest(text);
        if all.last().is_none_or(|v| v.digest != sum) {
            let fresh = self.add_file(text, path, now, &all)?;
            all.push(fresh);
        }
        let named = flat(label);
        if let Some(last) = all.last_mut() {
            last.label = named;
        }
        let taken = all.last().cloned().ok_or_else(|| io::Error::other("no version"))?;
        self.settle(all, Vec::new(), now, local)?;
        Ok(taken)
    }

    /// Name the newest version as it stands, whatever text it holds: for a
    /// buffer with unsaved changes, where the saved text is what the name is
    /// about.
    pub fn label_last(&self, label: &str) -> io::Result<Option<Version>> {
        let mut all = self.versions();
        let Some(last) = all.last_mut() else {
            return Ok(None);
        };
        last.label = flat(label);
        let taken = last.clone();
        self.write_index(&all)?;
        Ok(Some(taken))
    }

    /// Write `text` to a file of its own in the shelf, named after `now` and
    /// unlike any name in `all`.
    fn add_file(&self, text: &str, path: &str, now: i64, all: &[Version]) -> io::Result<Version> {
        std::fs::create_dir_all(&self.dir)?;
        // **A store tells git to leave it alone.** A book kept in git and
        // committed with `git add -A` would otherwise take a compressed copy
        // of every save into the repository, for good. A sync folder still
        // carries the store along; only git is told not to.
        if let Some(store) = self.dir.parent() {
            let ignore = store.join(".gitignore");
            if !ignore.exists() {
                let _ = std::fs::write(&ignore, "*\n");
            }
        }
        let taken = |name: &str| all.iter().any(|v| v.file == name) || self.dir.join(name).exists();
        let file = (0..)
            .map(|n| match n {
                0 => format!("{now}.zst"),
                n => format!("{now}-{n}.zst"),
            })
            .find(|name| !taken(name))
            .unwrap_or_default();
        let packed = zstd::encode_all(text.as_bytes(), LEVEL)?;
        write_atomically(&self.dir.join(&file), &packed)?;
        Ok(Version {
            secs: now,
            file,
            bytes: text.len(),
            chars: text.chars().count(),
            digest: digest(text),
            label: String::new(),
            path: flat(path),
        })
    }

    /// Thin `all`, write the index, and only then delete what left it:
    /// stopped half-way, the shelf holds a file too many, never an index line
    /// pointing at nothing. Files no line names are swept up with it.
    fn settle(&self, all: Vec<Version>, mut gone: Vec<Version>, now: i64, local: &dyn Fn(i64) -> (i64, u32)) -> io::Result<()> {
        let keep = thin(&all, now, local);
        let mut kept: Vec<Version> = Vec::with_capacity(all.len());
        for (version, stays) in all.into_iter().zip(keep) {
            match stays {
                true => kept.push(version),
                false => gone.push(version),
            }
        }
        self.write_index(&kept)?;
        let named = |name: &str| kept.iter().any(|v| v.file == name);
        for version in gone.iter().filter(|v| !named(&v.file)) {
            let _ = std::fs::remove_file(self.dir.join(&version.file));
        }
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".zst") && !named(&name) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }

    fn write_index(&self, all: &[Version]) -> io::Result<()> {
        let mut out = String::from(HEADER);
        out.push('\n');
        for v in all {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{:016x}\t{}\t{}\n",
                v.secs, v.file, v.bytes, v.chars, v.digest, v.label, v.path
            ));
        }
        write_atomically(&self.dir.join(INDEX), out.as_bytes())
    }
}

/// **Which versions stay**, one flag each, `versions` oldest first.
///
/// Time Machine's rule on the writer's own calendar: every version from
/// today; for the seven days before it, the newest of each hour; for the
/// sixty days before that, the newest of each day; nothing older. Two
/// versions always stay whatever their age — the newest, which is the save
/// that is happening, and any with a label, which the writer asked to keep.
///
/// `local` answers a second's local day number and hour; a test hands it
/// UTC so the answer does not depend on the machine's zone.
pub fn thin(versions: &[Version], now: i64, local: &dyn Fn(i64) -> (i64, u32)) -> Vec<bool> {
    let today = local(now).0;
    let mut keep = vec![false; versions.len()];
    let mut stood: std::collections::HashSet<(bool, i64)> = std::collections::HashSet::new();
    // Newest first, so the first version met in an hour or a day is the
    // newest one in it.
    for (i, v) in versions.iter().enumerate().rev() {
        let (day, hour) = local(v.secs);
        let age = today - day;
        let bucket = match age {
            ..=0 => None,
            1..=7 => Some((true, day * 24 + i64::from(hour))),
            8..=60 => Some((false, day)),
            _ => None,
        };
        let first_in_bucket = bucket.is_some_and(|b| stood.insert(b));
        keep[i] = i + 1 == versions.len() || !v.label.is_empty() || age <= 0 || first_in_bucket;
    }
    keep
}

/// **The shelf of a file that has moved**: one in `store` whose file is no
/// longer where its newest version says, and whose newest text `is_it`
/// recognises. `base` is what a relative path in the index is relative to.
///
/// What lets a chapter renamed outside the editor keep its history: the
/// buffer that opened it knows what it read, and that text is the newest
/// version of the shelf the old name left behind.
pub fn find_moved(store: &Path, base: Option<&Path>, is_it: &dyn Fn(&str) -> bool) -> Option<PathBuf> {
    let entries = std::fs::read_dir(store).ok()?;
    for entry in entries.flatten() {
        let shelf = Shelf { dir: entry.path() };
        if !shelf.exists() {
            continue;
        }
        let all = shelf.versions();
        let Some(last) = all.last() else { continue };
        let was_at = match base {
            Some(base) => base.join(&last.path),
            None => PathBuf::from(&last.path),
        };
        if was_at.exists() {
            continue;
        }
        if shelf.text_of(last).is_ok_and(|text| is_it(&text)) {
            return Some(shelf.dir);
        }
    }
    None
}

/// Move a shelf found by [`find_moved`] to the name `key` gives it.
pub fn adopt(store: &Path, from: &Path, key: &str) -> io::Result<Shelf> {
    let to = Shelf::of(store, key);
    std::fs::rename(from, &to.dir)?;
    Ok(to)
}

/// One version's text from its file.
pub fn read_text(path: &Path) -> io::Result<String> {
    let packed = std::fs::read(path)?;
    let raw = zstd::decode_all(packed.as_slice())?;
    String::from_utf8(raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// A label or a path as one index cell: no tabs, no line breaks.
fn flat(text: &str) -> String {
    text.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect()
}

fn parse_line(line: &str) -> Option<Version> {
    let mut cells = line.splitn(7, '\t');
    Some(Version {
        secs: cells.next()?.parse().ok()?,
        file: cells.next()?.to_string(),
        bytes: cells.next()?.parse().ok()?,
        chars: cells.next()?.parse().ok()?,
        digest: u64::from_str_radix(cells.next()?, 16).ok()?,
        label: cells.next()?.to_string(),
        path: cells.next()?.to_string(),
    })
}

/// Write beside, then rename over: a crash leaves the old file or the new
/// one, never half of either.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let beside = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&beside, bytes)?;
    std::fs::rename(&beside, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&beside);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const HOUR: i64 = 3_600;

    /// UTC, so the buckets do not depend on the machine's zone.
    fn utc(secs: i64) -> (i64, u32) {
        (secs.div_euclid(DAY), (secs.rem_euclid(DAY) / HOUR) as u32)
    }

    fn store(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yumete-versions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn at(secs: i64) -> Version {
        Version {
            secs,
            file: format!("{secs}.zst"),
            bytes: 0,
            chars: 0,
            digest: secs as u64,
            label: String::new(),
            path: "a.md".into(),
        }
    }

    /// Midday on some day, so an hour either way stays on it.
    const NOON: i64 = 20_000 * DAY + 12 * HOUR;

    #[test]
    fn a_save_is_kept_and_reads_back() {
        let store = store("reads-back");
        let shelf = Shelf::of(&store, "卷一/第一章.md");
        let text = "第一段。\n第二段，多写一句。\n";
        assert_eq!(shelf.keep(text, "卷一/第一章.md", NOON, &utc).unwrap(), Kept::Added);
        let all = shelf.versions();
        assert_eq!(all.len(), 1);
        assert_eq!(shelf.text_of(&all[0]).unwrap(), text);
        assert_eq!(all[0].chars, text.chars().count());
        assert_eq!(all[0].path, "卷一/第一章.md");
        assert!(shelf.dir().file_name().unwrap().to_string_lossy().starts_with("第一章.md-"));
        std::fs::remove_dir_all(&store).ok();
    }

    #[test]
    fn the_same_text_twice_is_one_version() {
        let store = store("same");
        let shelf = Shelf::of(&store, "a.md");
        shelf.keep("一句话。", "a.md", NOON, &utc).unwrap();
        assert_eq!(shelf.keep("一句话。", "a.md", NOON + 2 * MERGE_WINDOW, &utc).unwrap(), Kept::Same);
        assert_eq!(shelf.versions().len(), 1);
        std::fs::remove_dir_all(&store).ok();
    }

    #[test]
    fn saves_inside_the_window_replace_the_newest() {
        let store = store("merge");
        let shelf = Shelf::of(&store, "a.md");
        shelf.keep("一", "a.md", NOON, &utc).unwrap();
        assert_eq!(shelf.keep("一二", "a.md", NOON + MERGE_WINDOW - 1, &utc).unwrap(), Kept::Merged);
        let all = shelf.versions();
        assert_eq!(all.len(), 1);
        assert_eq!(shelf.text_of(&all[0]).unwrap(), "一二");
        // The replaced file is gone from the disk too, not only from the index.
        let files = std::fs::read_dir(shelf.dir()).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "zst")).count();
        assert_eq!(files, 1);
        // Measured from the newest version, which the merge moved.
        let later = NOON + MERGE_WINDOW - 1 + MERGE_WINDOW;
        assert_eq!(shelf.keep("一二三", "a.md", later, &utc).unwrap(), Kept::Added);
        assert_eq!(shelf.versions().len(), 2);
        std::fs::remove_dir_all(&store).ok();
    }

    #[test]
    fn a_labelled_version_is_never_merged_away() {
        let store = store("label");
        let shelf = Shelf::of(&store, "a.md");
        shelf.keep("交稿的版本", "a.md", NOON, &utc).unwrap();
        let named = shelf.label("交稿的版本", "a.md", "sent\tout", NOON + 10, &utc).unwrap();
        assert_eq!(named.label, "sent out", "a tab cannot reach the index");
        assert_eq!(shelf.keep("交稿以后又改了", "a.md", NOON + 20, &utc).unwrap(), Kept::Added);
        let all = shelf.versions();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].label, "sent out");
        std::fs::remove_dir_all(&store).ok();
    }

    #[test]
    fn labelling_other_text_keeps_it_first() {
        let store = store("label-new");
        let shelf = Shelf::of(&store, "a.md");
        shelf.keep("旧的", "a.md", NOON, &utc).unwrap();
        shelf.label("新的", "a.md", "定稿", NOON + 5, &utc).unwrap();
        let all = shelf.versions();
        assert_eq!(all.len(), 2);
        assert_eq!(shelf.text_of(&all[1]).unwrap(), "新的");
        assert_eq!((all[0].label.as_str(), all[1].label.as_str()), ("", "定稿"));
        std::fs::remove_dir_all(&store).ok();
    }

    #[test]
    fn today_keeps_everything() {
        let all: Vec<Version> = (0..5).map(|i| at(NOON - i * 60)).rev().collect();
        assert!(thin(&all, NOON, &utc).iter().all(|k| *k));
    }

    #[test]
    fn the_last_week_keeps_one_an_hour() {
        // Yesterday 10:00, 10:20, 10:40 and 11:30: the newest of each hour.
        let yesterday = NOON - DAY;
        let all = vec![
            at(yesterday - 2 * HOUR),
            at(yesterday - 2 * HOUR + 1200),
            at(yesterday - 2 * HOUR + 2400),
            at(yesterday - HOUR + 1800),
            at(NOON),
        ];
        assert_eq!(thin(&all, NOON, &utc), vec![false, false, true, true, true]);
    }

    #[test]
    fn up_to_sixty_days_keeps_one_a_day_and_older_goes() {
        let all = vec![
            at(NOON - 61 * DAY),
            at(NOON - 20 * DAY - HOUR),
            at(NOON - 20 * DAY),
            at(NOON - 8 * DAY),
            at(NOON - 7 * DAY - HOUR),
            at(NOON - 7 * DAY),
            at(NOON),
        ];
        assert_eq!(thin(&all, NOON, &utc), vec![false, false, true, true, true, true, true]);
    }

    #[test]
    fn the_newest_and_the_labelled_stay_whatever_their_age() {
        let mut old = at(NOON - 400 * DAY);
        old.label = "初稿".into();
        let all = vec![old, at(NOON - 300 * DAY), at(NOON - 200 * DAY)];
        assert_eq!(thin(&all, NOON, &utc), vec![true, false, true]);
    }

    #[test]
    fn thinning_on_a_save_deletes_the_files_too() {
        let store = store("thin");
        let shelf = Shelf::of(&store, "a.md");
        // Three saves in one hour, eight days apart from the next one.
        for (i, t) in [NOON - 3 * DAY, NOON - 3 * DAY + 900, NOON - 3 * DAY + 1800].iter().enumerate() {
            shelf.keep(&"字".repeat(i + 1), "a.md", *t, &utc).unwrap();
        }
        assert_eq!(shelf.versions().len(), 3, "all of that day, on that day");
        shelf.keep("新的一天", "a.md", NOON, &utc).unwrap();
        let all = shelf.versions();
        assert_eq!(all.len(), 2, "{all:?}");
        assert_eq!(shelf.text_of(&all[0]).unwrap(), "字字字", "the newest of the hour");
        let files = std::fs::read_dir(shelf.dir()).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "zst")).count();
        assert_eq!(files, 2);
        std::fs::remove_dir_all(&store).ok();
    }

    #[test]
    fn a_moved_file_finds_its_shelf() {
        let store = store("moved");
        let book = store.join("book");
        std::fs::create_dir_all(&book).unwrap();
        let shelf = Shelf::of(&store, "旧名.md");
        shelf.keep("这一章的文字。", "旧名.md", NOON, &utc).unwrap();
        // While the old name is still there, it is not a move.
        std::fs::write(book.join("旧名.md"), "这一章的文字。").unwrap();
        assert_eq!(find_moved(&store, Some(&book), &|t| t == "这一章的文字。"), None);
        std::fs::remove_file(book.join("旧名.md")).unwrap();
        let found = find_moved(&store, Some(&book), &|t| t == "这一章的文字。").expect("the shelf");
        assert_eq!(find_moved(&store, Some(&book), &|t| t == "别的文字"), None);
        let moved = adopt(&store, &found, "新名.md").unwrap();
        assert_eq!(moved.versions().len(), 1);
        assert!(!shelf.exists());
        std::fs::remove_dir_all(&store).ok();
    }

    #[test]
    fn digests_do_not_depend_on_the_build() {
        assert_eq!(digest(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(digest("a"), 0xaf63_dc4c_8601_ec8c);
    }
}
