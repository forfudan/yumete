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

/// The open sidebar.
#[derive(Debug, Clone)]
pub struct Sidebar {
    root: PathBuf,
    /// The directories showing their contents. The root is always one of them.
    open: BTreeSet<PathBuf>,
    rows: Vec<Row>,
    selected: usize,
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
        };
        sidebar.rebuild();
        sidebar
    }

    /// The rows to draw, top to bottom.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Which row is highlighted.
    pub fn selected(&self) -> usize {
        self.selected.min(self.rows.len().saturating_sub(1))
    }

    /// The name of the directory the tree is rooted at, for the header.
    pub fn title(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
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

    /// Enter the highlighted row: a file is returned to be opened, a directory
    /// opens or closes.
    pub fn activate(&mut self) -> Option<PathBuf> {
        let row = self.rows.get(self.selected())?.clone();
        if !row.is_dir {
            return Some(row.path);
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
    fn rebuild(&mut self) {
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
    fn push_dir(&mut self, dir: &Path, depth: usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in entries.flatten().take(MAX_PER_DIR) {
            let name = entry.file_name().to_string_lossy().into_owned();
            // The same things `:grep` skips: a manuscript directory holds them,
            // and a writer never opens them.
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => dirs.push((name, entry.path())),
                Ok(t) if t.is_file() => files.push((name, entry.path())),
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
        assert_eq!(sidebar.activate(), Some(dir.join("卷一/ch01.md")));

        // Collapsing from inside walks back out to the directory.
        sidebar.collapse();
        assert_eq!(sidebar.rows()[sidebar.selected()].name, "卷一");
        assert_eq!(sidebar.rows().len(), 3, "and it is closed again");
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
