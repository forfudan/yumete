//! The file sidebar — Feature #94.
//!
//! A different tool from the picker, for a different question. `Space f` answers
//! "take me to the file I am thinking of"; this answers "show me the shape of
//! the book". For a novel of a hundred chapters filed under 卷一 and 卷二, the
//! tree *is* the table of contents, and a writer wants to see it while writing
//! rather than summon it and have it go away again.
//!
//! It costs columns, which in a terminal is the one thing there is never enough
//! of — and set vertically it costs them by threes, because that is what a 縱 is
//! wide. So it is off by default, its width is a setting, and opening it is one
//! keystroke.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Which side of the page a panel is docked on — Feature #293.
///
/// The page has **two slots**, and which panel lands in which is not a fact
/// about the panel: it is a setting, and until there is one every panel goes
/// left, which is where the only panel there has ever been already is. So this
/// is a slot address and nothing more — no view, no key and no drawing may ask
/// "am I the left one" and behave differently for the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Side {
    /// Columns off the left of the page.
    #[default]
    Left,
    /// Columns off the right of it.
    Right,
}

impl Side {
    /// Both slots, in screen order — what the front end draws and what a
    /// question about "any panel" walks.
    pub const BOTH: [Side; 2] = [Side::Left, Side::Right];

    /// The other slot.
    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// What a panel is showing.
///
/// Views of one question — "what is there, and where am I in it" — at three
/// scales: the project, the files open in it, and the chapter on screen. `Tab`
/// walks between them, because they answer each other. And one that is not
/// like them at all, which is what [`View::resident`] is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// The files on disk, as a tree.
    #[default]
    Explorer,
    /// The files already open.
    Buffers,
    /// The headings of the file being written.
    Outline,
    /// What the 拆分表 knows about one character — Feature #215.
    Dictionary,
}

impl View {
    /// Every view, in the order `Tab` walks them.
    pub const ALL: [View; 4] = [
        View::Explorer,
        View::Buffers,
        View::Outline,
        View::Dictionary,
    ];

    /// **Whether the view stays up on its own** — Feature #293.
    ///
    /// A resident view is about something that is always there: a directory,
    /// the open files, this document. It waits to be looked at, so `Tab`
    /// reaches it.
    ///
    /// A transient one is put up by a question and taken down when the cursor
    /// leaves what was asked about. 字典 is the one there is: cycling into it
    /// would show an empty panel most of the time, and a `Tab` that lands
    /// somewhere empty is a `Tab` nobody presses twice.
    pub fn resident(self) -> bool {
        !matches!(self, View::Dictionary)
    }

    /// The next resident view in that direction, wrapping — and from a
    /// transient one, the nearest resident one, because `Tab` out of 字典 has
    /// to go *somewhere*.
    fn step(self, back: bool) -> View {
        let residents: Vec<View> = View::ALL.into_iter().filter(|v| v.resident()).collect();
        let Some(&first) = residents.first() else {
            return self;
        };
        let Some(at) = residents.iter().position(|&v| v == self) else {
            return first;
        };
        let n = residents.len();
        residents[match back {
            true => (at + n - 1) % n,
            false => (at + 1) % n,
        }]
    }

    /// Its name, for the sidebar's header.
    pub fn title(self) -> &'static str {
        match self {
            View::Explorer => "檔案",
            View::Buffers => "緩衝區",
            View::Outline => "大綱",
            View::Dictionary => "字典",
        }
    }
}

/// What choosing a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    /// Open this file.
    File(PathBuf),
    /// Show the buffer with this index.
    Buffer(usize),
    /// Put the cursor on this line of the file being written.
    Line(usize),
    /// Open this file and put the cursor on this line of it.
    ///
    /// The outline of a book reaches past the file being written: a chapter
    /// pulled in with `#include` is one row here and a whole file there.
    FileLine(PathBuf, usize),
}

/// One line of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The file or directory this row stands for.
    pub path: PathBuf,
    /// Its own name, without the path above it.
    pub name: String,
    /// How deep under the root it sits, for the indent.
    pub depth: usize,
    /// Whether it is a directory.
    pub is_dir: bool,
    /// Whether it is a directory that is showing its contents.
    pub expanded: bool,
}

/// One heading of the 大綱, before any of it is folded away (#37).
///
/// The rows the sidebar draws cannot answer 「what is under this one」 — a
/// [`Row`] spends `depth` on the line number and keeps the indent inside its
/// name, so the nesting is a fact about the *string*. Folding needs the
/// nesting as a number, so the editor builds these first and turns them into
/// rows afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// The file it is in, empty for the one being written.
    pub path: PathBuf,
    /// Which line of that file.
    pub line: usize,
    /// How many `#` or `=` deep, counting from one.
    pub level: usize,
    /// What it says.
    pub title: String,
}

impl Heading {
    /// What the fold set remembers it by.
    pub fn key(&self) -> (PathBuf, usize) {
        (self.path.clone(), self.line)
    }
}

/// The open sidebar.
#[derive(Debug, Clone)]
pub struct Sidebar {
    root: PathBuf,
    /// The directories showing their contents. The root is always one of them.
    open: BTreeSet<PathBuf>,
    rows: Vec<Row>,
    selected: usize,
    /// Which view is showing, and where the highlight was in each of the
    /// others — so `Tab` back and forth returns to where you were, not to the
    /// top.
    view: View,
    kept: [usize; View::ALL.len()],
    /// The headings whose contents are folded away — [`Heading::key`] of each
    /// (#37).
    ///
    /// Kept here rather than beside the outline because the outline is rebuilt
    /// from the document every time the sidebar is looked at, and a fold that
    /// did not survive that would never be seen folded.
    folded: BTreeSet<(PathBuf, usize)>,
    /// Whether it is opened out wide enough to read a whole title.
    ///
    /// The ordinary width is a setting, and it is narrow on purpose — columns
    /// are what a terminal has least of. But a chapter called 「天門真境之傳家
    /// 寶扇」 does not fit in it, and cutting the name off is exactly what an
    /// outline must not do. So the width is a toggle, not a compromise.
    wide: bool,
}

/// How many entries one directory contributes before the tree gives up on it.
///
/// A directory of ten thousand files is not a manuscript, and reading it would
/// stall the keystroke that opened the sidebar.
const MAX_PER_DIR: usize = 500;

impl Sidebar {
    /// Open a sidebar rooted at `root`, with the root expanded.
    pub fn new(root: &Path) -> Sidebar {
        let mut sidebar = Sidebar {
            root: root.to_path_buf(),
            open: BTreeSet::from([root.to_path_buf()]),
            rows: Vec::new(),
            selected: 0,
            view: View::Explorer,
            kept: [0; View::ALL.len()],
            folded: BTreeSet::new(),
            wide: false,
        };
        sidebar.rebuild();
        sidebar
    }

    /// Whether it is opened out to read whole titles.
    pub fn wide(&self) -> bool {
        self.wide
    }

    /// Open it out, or fold it back.
    pub fn toggle_width(&mut self) -> bool {
        self.wide = !self.wide;
        self.wide
    }

    /// Which view is showing.
    pub fn view(&self) -> View {
        self.view
    }

    /// Show a named view, keeping the place in the one being left.
    ///
    /// `Tab` reaches only the resident views ([`View::resident`]), so asking
    /// about a character is the only way into 字典, and this is how the editor
    /// asks.
    pub fn show(&mut self, view: View) {
        self.kept[self.view as usize] = self.selected;
        self.view = view;
        self.selected = self.kept[self.view as usize];
    }

    /// Walk to the next resident view (`Tab`, or `Shift+Tab` backwards),
    /// keeping each one's place.
    ///
    /// The rows of the others are not this module's to build — buffers and
    /// headings belong to the editor — so it says which view it wants and is
    /// handed the rows for it.
    ///
    /// **Backwards is a direction, not two steps forward.** It used to be
    /// spelled `cycle(); cycle();`, which is the same thing only while there
    /// are exactly three residents — the arithmetic breaks the day a fourth
    /// joins them.
    pub fn cycle(&mut self, back: bool) -> View {
        self.kept[self.view as usize] = self.selected;
        self.view = self.view.step(back);
        self.selected = self.kept[self.view as usize];
        self.view
    }

    /// Fill the sidebar with rows the editor built (buffers, or headings).
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        self.rows = rows;
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    /// The rows to draw, top to bottom.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Which row is highlighted.
    pub fn selected(&self) -> usize {
        self.selected.min(self.rows.len().saturating_sub(1))
    }

    /// Put the highlight on row `i`, or on the last row if there is no such
    /// row — what folding does after the rows below have gone away.
    pub fn select(&mut self, i: usize) {
        self.selected = i.min(self.rows.len().saturating_sub(1));
    }

    /// Whether that heading's contents are folded away (#37).
    pub fn is_folded(&self, key: &(PathBuf, usize)) -> bool {
        self.folded.contains(key)
    }

    /// Fold that heading, or open it again. Whether anything changed.
    pub fn set_folded(&mut self, key: (PathBuf, usize), folded: bool) -> bool {
        match folded {
            true => self.folded.insert(key),
            false => self.folded.remove(&key),
        }
    }

    /// The sidebar's header: the view's name, and for the tree the directory it
    /// is rooted at.
    pub fn title(&self) -> String {
        match self.view {
            View::Explorer => {
                let root = self
                    .root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| self.root.display().to_string());
                format!("{}  {root}", self.view.title())
            }
            view => view.title().to_string(),
        }
    }

    /// Move the highlight, stopping at the ends rather than wrapping: a tree is
    /// a shape, and wrapping from the last file to the first loses the shape.
    pub fn step(&mut self, down: bool) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = if down {
            self.selected().saturating_add(1).min(last)
        } else {
            self.selected().saturating_sub(1)
        };
    }

    /// Jump to the top or the bottom of the list.
    ///
    /// `g` and `G`, as in the text: an outline of 700 chapters has two ends
    /// worth reaching in one key, and 「hold j」 is not a way to reach either.
    pub fn go_to_end(&mut self, bottom: bool) {
        self.selected = match bottom {
            true => self.rows.len().saturating_sub(1),
            false => 0,
        };
    }

    /// Enter the highlighted row.
    ///
    /// In the tree a directory opens or closes and a file is handed back to be
    /// opened; in the other two views every row is a destination.
    pub fn activate(&mut self) -> Option<Chosen> {
        let row = self.rows.get(self.selected())?.clone();
        match self.view {
            View::Buffers => return Some(Chosen::Buffer(row.depth)),
            View::Outline => {
                return Some(if row.path.as_os_str().is_empty() {
                    Chosen::Line(row.depth)
                } else {
                    Chosen::FileLine(row.path, row.depth)
                })
            }
            // A field is not a place to go — the panel is read, not walked
            // into. `Esc` and `C-w` are how a reader leaves it.
            View::Dictionary => return None,
            View::Explorer => {}
        }
        if !row.is_dir {
            return Some(Chosen::File(row.path));
        }
        if row.expanded {
            self.open.remove(&row.path);
        } else {
            self.open.insert(row.path);
        }
        self.rebuild();
        None
    }

    /// Close the highlighted directory, or step out to the one holding this row.
    ///
    /// One key for both, because "less of this" is one intention: pressing it
    /// repeatedly walks back up the tree, which is how a reader leaves a branch.
    pub fn collapse(&mut self) {
        if self.view != View::Explorer {
            return;
        }
        let Some(row) = self.rows.get(self.selected()).cloned() else {
            return;
        };
        if row.is_dir && row.expanded {
            self.open.remove(&row.path);
            self.rebuild();
            return;
        }
        let Some(parent) = row.path.parent().map(Path::to_path_buf) else {
            return;
        };
        if parent == self.root {
            return;
        }
        self.open.remove(&parent);
        self.rebuild();
        if let Some(i) = self.rows.iter().position(|r| r.path == parent) {
            self.selected = i;
        }
    }

    /// Put the highlight on `path`, if it is on the tree — so opening a file by
    /// any other means still shows where it lives.
    pub fn reveal(&mut self, path: &Path) {
        if self.view != View::Explorer {
            return;
        }
        // Expand every directory between the root and the file first.
        let mut at = path.parent();
        let mut opened = false;
        while let Some(dir) = at {
            if !dir.starts_with(&self.root) {
                break;
            }
            opened |= self.open.insert(dir.to_path_buf());
            if dir == self.root {
                break;
            }
            at = dir.parent();
        }
        if opened {
            self.rebuild();
        }
        if let Some(i) = self.rows.iter().position(|r| r.path == path) {
            self.selected = i;
        }
    }

    /// Walk the tree again, descending only into the directories that are open.
    pub(crate) fn rebuild(&mut self) {
        if self.view != View::Explorer {
            return;
        }
        let keep = self.rows.get(self.selected()).map(|r| r.path.clone());
        self.rows.clear();
        let root = self.root.clone();
        self.push_dir(&root, 0);
        // Keep the highlight on whatever it was on, if that is still shown.
        self.selected = keep
            .and_then(|path| self.rows.iter().position(|r| r.path == path))
            .unwrap_or(self.selected)
            .min(self.rows.len().saturating_sub(1));
    }

    /// Append `dir`'s children, directories first and each sorted by name.
    ///
    /// **The same question `:grep` asks** (#362), asked of the same walker:
    /// one level of [`ignore`], which reads the `.gitignore` in the directory
    /// as well as the ones above it. The tree used to skip three hard-coded
    /// names, so a repository showed its whole source next to the chapters.
    fn push_dir(&mut self, dir: &Path, depth: usize) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let walker = ignore::WalkBuilder::new(dir)
            .max_depth(Some(1))
            .follow_links(false)
            .require_git(false)
            .build();
        // `depth() > 0` rather than `skip(1)`: the walker's first answer is
        // the directory itself, and it is not always there to be skipped.
        let children = walker.flatten().filter(|e| e.depth() > 0);
        for entry in children.take(MAX_PER_DIR) {
            let name = entry.file_name().to_string_lossy().into_owned();
            // The floor under the ignore files; see `editor::walk`.
            if name == "target" || name == "node_modules" {
                continue;
            }
            match entry.file_type() {
                Some(t) if t.is_dir() => dirs.push((name, entry.into_path())),
                Some(t) if t.is_file() => files.push((name, entry.into_path())),
                _ => {}
            }
        }
        dirs.sort();
        files.sort();
        for (name, path) in dirs {
            let expanded = self.open.contains(&path);
            self.rows.push(Row {
                path: path.clone(),
                name,
                depth,
                is_dir: true,
                expanded,
            });
            if expanded {
                self.push_dir(&path, depth + 1);
            }
        }
        for (name, path) in files {
            self.rows.push(Row {
                path,
                name,
                depth,
                is_dir: false,
                expanded: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little novel on disk: two 卷, a chapter in each, and a stray note.
    fn novel() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yumete-side-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("卷一")).unwrap();
        std::fs::create_dir_all(dir.join("卷二")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("卷一/ch01.md"), "").unwrap();
        std::fs::write(dir.join("卷二/ch02.md"), "").unwrap();
        std::fs::write(dir.join("notes.md"), "").unwrap();
        dir
    }

    #[test]
    fn the_tree_shows_directories_first_and_hides_what_is_not_a_manuscript() {
        let dir = novel();
        let sidebar = Sidebar::new(&dir);
        let names: Vec<&str> = sidebar.rows().iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["卷一", "卷二", "notes.md"]);
        assert!(sidebar.rows().iter().all(|r| !r.expanded));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_directory_opens_and_closes_and_a_file_is_returned() {
        let dir = novel();
        let mut sidebar = Sidebar::new(&dir);

        // The first row is 卷一; entering it shows its chapter, indented.
        assert_eq!(
            sidebar.activate(),
            None,
            "a directory opens, it is not opened"
        );
        let names: Vec<&str> = sidebar.rows().iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["卷一", "ch01.md", "卷二", "notes.md"]);
        assert_eq!(sidebar.rows()[1].depth, 1);

        // Down onto the chapter, and entering it hands back the path to open.
        sidebar.step(true);
        assert_eq!(
            sidebar.activate(),
            Some(Chosen::File(dir.join("卷一/ch01.md")))
        );

        // Collapsing from inside walks back out to the directory.
        sidebar.collapse();
        assert_eq!(sidebar.rows()[sidebar.selected()].name, "卷一");
        assert_eq!(sidebar.rows().len(), 3, "and it is closed again");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tab_walks_the_views_and_keeps_each_ones_place() {
        let dir = novel();
        let mut sidebar = Sidebar::new(&dir);
        sidebar.step(true);
        assert_eq!(sidebar.selected(), 1);

        assert_eq!(sidebar.cycle(false), View::Buffers);
        // The other views' rows come from the editor; empty until it fills them.
        assert_eq!(sidebar.selected(), 0);
        assert_eq!(sidebar.cycle(false), View::Outline);
        assert_eq!(sidebar.cycle(false), View::Explorer);
        assert_eq!(sidebar.selected(), 1, "the tree is where it was left");
        assert!(sidebar.title().contains("檔案"));

        // …and backwards is one step back, not two forward — the same answer
        // today, and still the right one when a fourth resident view joins.
        assert_eq!(sidebar.cycle(true), View::Outline);
        assert_eq!(sidebar.cycle(true), View::Buffers);
        assert_eq!(sidebar.cycle(true), View::Explorer);

        // 字典 is transient: `Tab` never lands on it, and `Tab` out of it goes
        // to the first resident view rather than nowhere.
        sidebar.show(View::Dictionary);
        assert_eq!(sidebar.cycle(false), View::Explorer);
        sidebar.show(View::Dictionary);
        assert_eq!(sidebar.cycle(true), View::Explorer);
        assert!(View::ALL.iter().filter(|v| v.resident()).count() == 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_flat_view_carries_its_index_and_is_not_a_tree() {
        let dir = novel();
        let mut sidebar = Sidebar::new(&dir);
        sidebar.cycle(false);
        sidebar.set_rows(vec![
            Row {
                path: PathBuf::new(),
                name: "ch01.md".to_string(),
                depth: 0,
                is_dir: false,
                expanded: true,
            },
            Row {
                path: PathBuf::new(),
                name: "ch02.md +".to_string(),
                depth: 1,
                is_dir: false,
                expanded: false,
            },
        ]);
        sidebar.step(true);
        assert_eq!(sidebar.activate(), Some(Chosen::Buffer(1)));
        // Collapsing means nothing in a flat list, and does nothing.
        sidebar.collapse();
        assert_eq!(sidebar.rows().len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_highlight_stops_at_the_ends_rather_than_wrapping() {
        let dir = novel();
        let mut sidebar = Sidebar::new(&dir);
        for _ in 0..10 {
            sidebar.step(true);
        }
        assert_eq!(sidebar.selected(), 2, "the last row");
        for _ in 0..10 {
            sidebar.step(false);
        }
        assert_eq!(sidebar.selected(), 0, "the first");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn revealing_a_file_opens_the_branch_it_lives_on() {
        let dir = novel();
        let mut sidebar = Sidebar::new(&dir);
        sidebar.reveal(&dir.join("卷二/ch02.md"));
        assert_eq!(sidebar.rows()[sidebar.selected()].name, "ch02.md");
        assert!(sidebar
            .rows()
            .iter()
            .any(|r| r.name == "卷二" && r.expanded));
        std::fs::remove_dir_all(&dir).ok();
    }
}
