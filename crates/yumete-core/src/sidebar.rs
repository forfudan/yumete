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

    /// Parse a config value. An unknown word is `None`, and the caller keeps
    /// the default rather than guessing: a typo here moves the whole page.
    pub fn parse(value: &str) -> Option<Side> {
        match value.trim().to_ascii_lowercase().as_str() {
            "left" | "左" => Some(Side::Left),
            "right" | "右" => Some(Side::Right),
            _ => None,
        }
    }

    /// The other slot.
    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// **What is in a slot right now** — one panel, never two (2026-09-22 定：
/// 「臨時面板直接廢除，以後只有左右邊欄」)。
///
/// 從前這裏是兩層（`Layer::Top`／`Bottom`）：常駐的一層在上、光標算出來的一層在
/// 下，同時畫兩個。取消它的理由是**用起來幾乎不會兩個一起看**，而代價一直在付：
/// 三個正交的問題（誰決定它消失、畫在哪、收不收鍵）被那兩層絞成一團，逐個東西定
/// 規矩，記不住。
///
/// 現在的模型一句話：**邊欄是容器，面板是內容。** 容器只由人開由人關，空了就空
/// 着；內容一次一個。
///
/// ⚠️ **「臨時」只剩一個屬性**：光標放上去的那幾種（[`Transient`]）在它成立的時候
/// **頂掉**常駐的那一個，不成立了就自己下去——而常駐那一個**一直存着没動過**，所以
/// 「有前任還給前任，没有就空着」是白拿的，不必記什麼。
/// **A panel the cursor puts there**, if any — Feature #293.
///
/// 這幾種**不存狀態**：每一幀從光標在哪算出來，不成立就不畫。它們頂掉常駐那一個，
/// 而常駐那一個原封不動地留着（見上面那一段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transient {
    /// What the cursor is standing in, field by field — Feature #296.
    Detail,
    /// What the 拆分表 knows about one character — Feature #215.
    Dictionary,
    /// **What the language server says this name is** (`空格 K`, #53 ③).
    ///
    /// ⚠️ **Shares 字典's slot on purpose.** Both answer 「光標底下這個東西是什
    /// 麽」—— one about a 字, one about a name —— and they can never both want
    /// the slot: 2026-09-21 定的那條，「寫代碼的時候才需要 lsp 錯誤，寫普通文章
    /// 才需要查字典。這兩個場景是很少耦合的」。A setting of its own would be a
    /// knob nobody ever turns.
    Hover,
}

impl Transient {
    /// **Whether `C-w` stops here** — Feature #293。
    ///
    /// 一律停。2026-09-22 定的規矩只有一條：**浮窗不收鍵，邊欄裏的收**——而
    /// [`Transient`] 描述的就是「擺在邊欄裏的那一個」，所以三個都收。
    ///
    /// ⚠️ 從前這裏是逐個東西定的（`Detail => false`），理由是詳情欄自己會跟着
    /// 光標滾、不必走。理由沒錯，可它是**多記一條例外**：新規矩買的正是「不必
    /// 逐個記」，留一個例外等於沒換（2026-09-23 審出來的，`development.md`
    /// §5.12.16 那張「從前／現在」表早就這麽寫了）。走得動不妨礙跟着滾——`C-w`
    /// 進去之前它照舊跟着光標。
    /// 這個 [`Panel`] 擺在邊欄裏的時候是哪一種臨時面板，沒有就是 `None`。
    pub fn of(panel: Panel) -> Option<Transient> {
        match panel {
            Panel::Dictionary => Some(Transient::Dictionary),
            Panel::Detail => Some(Transient::Detail),
            _ => None,
        }
    }

    pub fn takes_keys(self) -> bool {
        match self {
            // 服務器說的話可以有十幾行，一份 28 欄的拆分表行更長——讀得到底纔算數。
            Transient::Detail | Transient::Dictionary | Transient::Hover => true,
        }
    }
}

/// **Every panel a slot can hold** — Feature #293.
///
/// One name per thing that can be put in a slot, and the only question it
/// answers is *which side is it on*. The resident ones are also a [`View`]
/// (they share a slot and `Tab` walks between them); the transient ones are
/// also a [`Transient`]. This enum is neither of those: it
/// is the address a setting writes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    /// The files on disk.
    Files,
    /// The files already open.
    Buffers,
    /// The headings of the file being written.
    Outline,
    /// Look for a pattern, and what it found.
    Search,
    /// The 拆分表 on one character.
    Dictionary,
    /// What the cursor is standing in, field by field.
    Detail,
    /// The wiki entry the cursor is standing on (#287).
    Wiki,
}

impl Panel {
    /// Every one of them, in the order a setting file lists them.
    pub const ALL: [Panel; 7] = [
        Panel::Files,
        Panel::Buffers,
        Panel::Outline,
        Panel::Search,
        Panel::Dictionary,
        Panel::Detail,
        Panel::Wiki,
    ];

    /// Its name in the config file and on the command line.
    pub fn key(self) -> &'static str {
        match self {
            Panel::Files => "files",
            Panel::Buffers => "buffers",
            Panel::Outline => "outline",
            Panel::Search => "search",
            Panel::Dictionary => "dictionary",
            Panel::Detail => "detail",
            Panel::Wiki => "wiki",
        }
    }

    /// Read that name back.
    pub fn parse(value: &str) -> Option<Panel> {
        let value = value.trim().to_ascii_lowercase();
        Panel::ALL.into_iter().find(|p| p.key() == value)
    }

    /// **What to call it, as a message tag rather than a word.**
    ///
    /// ⚠️ Not a `&'static str` of Chinese like [`View::title`]: that one is a
    /// panel's own header, drawn as it is, while this one is dropped into
    /// sentences (`sidebar.moved`) — and a Chinese word in an English sentence
    /// is a sentence half translated.
    pub fn tag(self) -> &'static str {
        match self {
            Panel::Files => "label.panel.files",
            Panel::Buffers => "label.panel.buffers",
            Panel::Outline => "label.panel.outline",
            Panel::Search => "label.panel.search",
            Panel::Dictionary => "label.panel.dictionary",
            Panel::Detail => "label.panel.detail",
            Panel::Wiki => "label.panel.wiki",
        }
    }
}

impl From<View> for Panel {
    fn from(view: View) -> Panel {
        match view {
            View::Explorer => Panel::Files,
            View::Buffers => Panel::Buffers,
            View::Outline => Panel::Outline,
            View::Search => Panel::Search,
            View::Wiki => Panel::Wiki,
        }
    }
}

impl From<Transient> for Panel {
    fn from(kind: Transient) -> Panel {
        match kind {
            // hover 與字典共用同一個位置，所以也共用那一格設定。
            Transient::Dictionary | Transient::Hover => Panel::Dictionary,
            Transient::Detail => Panel::Detail,
        }
    }
}

/// What a panel is showing.
///
/// Views of one question — "what is there, and where am I in it" — at three
/// scales: the project, the files open in it, and the chapter on screen. `Tab`
/// walks between them, because they answer each other.
///
/// **Every view here is resident**, and that is now a fact about the layer
/// rather than about the view: what is put up by the cursor and taken down
/// again is a [`Transient`], not a `View`. 字典 used to be the exception in
/// this enum and is one of those now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// The files on disk, as a tree.
    #[default]
    Explorer,
    /// The files already open.
    Buffers,
    /// The headings of the file being written.
    Outline,
    /// Look for a pattern, and what it found — Feature #419.
    Search,
    /// The wiki entry under the cursor, kept on the page (#287). While it is
    /// open the floating panel does not show the entry too: one place at a
    /// time.
    Wiki,
}

impl View {
    /// Every view, in the order `Tab` walks them.
    pub const ALL: [View; 5] = [
        View::Explorer,
        View::Buffers,
        View::Outline,
        View::Search,
        View::Wiki,
    ];

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
    pub fn show(&mut self, view: View) {
        self.kept[self.view as usize] = self.selected;
        self.view = view;
        self.selected = self.kept[self.view as usize];
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
    /// **這一格的主語**，接在框畫的標題後面——文件樹接的是它扎根的那個目錄
    /// （「文件樹  yumete」）。別的視圖沒有主語，回空。
    ///
    /// ⚠️ **名字不在這裏**（2026-09-26 定，原話：「我希望你让这些面板都一致，不要
    /// 搞特殊化」）：五扇面板的標題一律由 `sidebar_shell` 畫，源頭是 `Panel::tag()`
    /// 那一張表。從前這裏另有一張寫死的繁體短名表，於是同一扇面板有兩個名字，而
    /// 英文界面上這一行照樣是中文。
    pub fn subject(&self) -> String {
        match self.view {
            View::Explorer => self
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.root.display().to_string()),
            _ => String::new(),
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
            // Never reached: the search panel has a store of its own and
            // never fills these rows (`refresh_panel`).
            View::Search | View::Wiki => return None,
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

    /// Every view keeps its own place, so walking away and back does not go
    /// to the top. **Which view is next is not asked here** — that depends on
    /// which views share this slot, which is the editor's question (#293).
    #[test]
    fn each_view_keeps_its_own_place() {
        let dir = novel();
        let mut sidebar = Sidebar::new(&dir);
        sidebar.step(true);
        assert_eq!(sidebar.selected(), 1);

        sidebar.show(View::Buffers);
        // The other views' rows come from the editor; empty until it fills them.
        assert_eq!(sidebar.selected(), 0);
        sidebar.show(View::Outline);
        sidebar.show(View::Explorer);
        assert_eq!(sidebar.selected(), 1, "the tree is where it was left");
        // 標題歸容器畫（`sidebar_shell`）；這一格自己只交出主語——根目錄名。
        assert_eq!(sidebar.subject(), dir.file_name().unwrap().to_string_lossy());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_flat_view_carries_its_index_and_is_not_a_tree() {
        let dir = novel();
        let mut sidebar = Sidebar::new(&dir);
        sidebar.show(View::Buffers);
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
