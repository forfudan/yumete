//! **那扇設置面板的狀態** —— Feature #421（2026-09-24）。
//!
//! 原話：
//!
//! > 這樣的話用戶（特別是寫小説的），不需要面對 toml 和一堆 key 發呆不知道他們
//! > 都是幹啥的，也不需要查詢到底有哪些 key 可以用。
//!
//! 整頁一扇：一張單子，八組的名字是單子裏的分隔標題，左邊那一欄是它們的目録。
//! 鍵在每一行上只有一個意思（2026-11-08 作者定）：`jk` `gg` `G` 走、`/` 搜、
//! `Enter` 改、空格撥、`r` 回到下一層、`u` / `C-r` 撤銷與重做、`Tab` 換寫哪一份、
//! `q` 關。
//!
//! # 為什麽在 `yumete-config` 而不是 `yumete-core`
//!
//! 別的面板（搜索、大綱、字典）狀態都在核心裏，這一扇不行：它讀
//! [`crate::settings_ui`] 那張表、調 [`crate::write_back`] 存盤，而
//! **`yumete-core` 不依賴 `yumete-config`**（看 `Cargo.toml`，那一頭只有
//! `yumete-cjk`）。把表和寫回搬進核心是倒着接依賴；把狀態放在這裏，整件事就都在
//! 一個 crate 裏，前端只管畫。
//!
//! # 三層，而畫出來的是「這一項現在從哪一層來」
//!
//! 出廠 → 全局 → 本項目，後面的蓋前面的。面板右邊那一欄說的是**贏的那一層**，
//! 再後面跟着**被蓋掉的那一個值**——不寫出來的話，一個讀者看見「首行縮進 2」而
//! 他記得自己全局設的是 0，就只能去翻檔案。
//!
//! # 即改即存，`u` 反悔
//!
//! 2026-09-23 定的是 `:w`（「即時生效不好，改錯了都没辦法反悔」）；2026-11-08 改成
//! **每一次確認的改動當場寫進檔裏並生效**，而反悔是 `u`：撤銷把檔寫回去。「改了」
//! 和「存了」從此是同一件事，没有「未保存」，也没有走的時候那一問——審出來的毛病
//! 有一半是那兩個狀態分不清。改動先落在 [`Sheet::edits`]，[`Panel::flush`] 隨即
//! 寫下去，落完由 `:reload-config` 那條同一支推進編輯器。

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::settings_ui::{Kind, Setting, GROUPS, SETTINGS};
use yumete_cjk::{Flavor, FlavorItem, FlavorLevel, KeyPreset};
use crate::write_back::{rewrite, Change};

/// 一個值是從哪一層來的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// 誰都没說，用的是出廠那一個。
    Factory,
    /// 全局 `~/.config/yumete/config.toml`。
    Global,
    /// 本項目最近的 `.yumete/config.toml`。
    Local,
}

/// 一層，和它的名字。
///
/// Warning: **寫成 `label:` 字段，不是一支 `match`** —— 同 [`crate::settings_ui::Named`]，
/// 同一個理由，而這一處 2026-09-24 就照着 `match` 寫過一遍、當場被
/// `messages.rs` 那張網逮住（三則文案「没人說」）。那張網讀源碼找 `label:`/
/// `hint:`/`say!(`，**看不見 `match` 的返回值**。
///
/// Warning: 更壞的不是紅，是它**紅得像另一件事**：那條 assert 說的是「a renamed or
/// deleted tag」，照着做就把三則好好的文案刪了，面板第三欄從此逐行寫着
/// `set.layer.factory`。
pub struct LayerName {
    pub layer: Layer,
    pub label: &'static str,
}

/// 三層，出廠在最底下。
pub const LAYERS: &[LayerName] = &[
    LayerName { layer: Layer::Factory, label: "settings.layer.factory" },
    LayerName { layer: Layer::Global, label: "settings.layer.global" },
    LayerName { layer: Layer::Local, label: "settings.layer.local" },
];

impl Layer {
    /// 這一層的名字，給 `say!` 用。
    pub fn label(self) -> &'static str {
        LAYERS
            .iter()
            .find(|n| n.layer == self)
            .map(|n| n.label)
            .unwrap_or("settings.layer.factory")
    }
}

/// 改動落到哪一份檔上 —— 頂上那一行切着的那個。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Global,
    Local,
}

impl Which {
    /// Tab 切到另一份。
    pub fn other(self) -> Which {
        match self {
            Which::Global => Which::Local,
            Which::Local => Which::Global,
        }
    }

    /// 它是哪一層。
    pub fn layer(self) -> Layer {
        match self {
            Which::Global => Layer::Global,
            Which::Local => Layer::Local,
        }
    }
}

/// **一份配置檔**：它在哪、正文長什麽樣、以及改了還没存的那幾項。
#[derive(Debug, Clone, Default)]
pub struct Sheet {
    /// 檔的位置。
    ///
    /// Warning: **本項目那一份多半還不存在，而路徑照樣給得出來** —— 面板頂上寫着
    /// 「還没有這個文件——存盤時建」，那句話得兌現。給 `None` 的話 `save()` 會
    /// 回一句 `Local 那一層没有檔`，一個**没翻譯的 Rust Debug 字串**出現在界面
    /// 上，而改動還掛着。2026-09-24 審出來的。
    pub path: Option<PathBuf>,
    /// 那個檔此刻在不在磁碟上。面板拿它決定說路徑還是說「存盤時建」。
    pub there: bool,
    /// 檔裏原樣的那一份。存盤時 [`rewrite`] 改的就是它，所以**一個字節都不能動**。
    pub text: String,
    /// 改了還没存的：`表.鍵` → 怎麽改。
    pub edits: BTreeMap<String, Change>,
    /// 檔裏此刻寫着的那些值，`表.鍵` → 照 toml 的寫法。
    ///
    /// 從 [`text`](Self::text) 算出來的，開面板時算一次。Warning: **不是每一幀重算**：
    /// 一幀要問幾十次，而 toml 解析一次是幾十微秒——乘起來就是打字時看得見的頓。
    declared: BTreeMap<String, String>,
    /// **The loader cannot read this file** (2026-11-08): the line, 0 起算,
    /// and the parser's message. A TOML error, or a name or a type the
    /// config does not take — either way yumete runs as if the file were not
    /// there, so what it says is not in force and the panel does not show it
    /// as if it were; `:w` will not write it.
    pub unread: Option<(usize, String)>,
    /// **When the file was last seen and how long it was** (2026-11-09):
    /// another time or length on the disk is a change made outside, and the
    /// file is read again ([`Panel::refresh`]). `None` ＝ not there.
    stamp: Option<(std::time::SystemTime, u64)>,
}

/// A file's time and length, or `None` when it is not there.
fn stamp(path: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// **What the disk holds now**: the text, `None` when the file is not there
/// (2026-11-09, reported: a file deleted outside was written back whole from
/// the text the panel had read), an error when it cannot be read.
fn read_disk(path: &std::path::Path) -> Result<Option<String>, SaveError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(SaveError::Io(format!("{}: {e}", path.display()))),
    }
}

/// **Write `text` as the file, or take the file away for `None`** — what
/// `u` puts back when the file was not there before.
fn put(path: &std::path::Path, text: Option<&str>) -> Result<(), SaveError> {
    match text {
        Some(text) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| SaveError::Io(format!("{}: {e}", parent.display())))?;
            }
            write_atomically(path, text).map_err(|e| SaveError::Io(format!("{}: {e}", path.display())))
        }
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(SaveError::Io(format!("{}: {e}", path.display())))
            }
            _ => Ok(()),
        },
    }
}

impl Sheet {
    /// 讀一份檔。讀不到就是空的那一份（要存的時候會建出來）。
    pub fn read(path: Option<PathBuf>) -> Sheet {
        let read = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok());
        let there = read.is_some();
        let text = read.unwrap_or_default();
        let seen = path.as_deref().and_then(stamp);
        Sheet { stamp: seen, ..Sheet::of_text(path, there, text) }
    }

    /// **What the disk holds now, taken in**: the text, whether it is there,
    /// what it says, and the stamp the next change outside is told by.
    fn took(&mut self, read: Option<String>) {
        let edits = std::mem::take(&mut self.edits);
        let fresh = Sheet::of_text(self.path.clone(), read.is_some(), read.unwrap_or_default());
        let seen = self.path.as_deref().and_then(stamp);
        *self = Sheet { edits, stamp: seen, ..fresh };
    }

    /// A sheet of `text`, as if read from `path`.
    pub fn of_text(path: Option<PathBuf>, there: bool, text: String) -> Sheet {
        let unread = crate::faults::unread(&text);
        Sheet {
            // What a file the loader does not read says is not in force.
            declared: match unread {
                Some(_) => BTreeMap::new(),
                None => declared_in(&text),
            },
            unread,
            path,
            there,
            text,
            edits: BTreeMap::new(),
            stamp: None,
        }
    }

    /// 這一份此刻說 `path` 是什麽 —— 改了還没存的算數。
    pub fn says(&self, path: &str) -> Option<String> {
        match self.edits.get(path) {
            Some(Change::Drop) => None,
            Some(change) => Some(written(change)),
            None => self.declared.get(path).cloned(),
        }
    }

    /// What the file on disk says of `path`, unsaved edits left out.
    pub fn declared(&self, path: &str) -> Option<&String> {
        self.declared.get(path)
    }

    /// 這一份有没有改了還没存的東西。
    pub fn dirty(&self) -> bool {
        !self.edits.is_empty()
    }

    /// Whether `path`, or a key under it (a page's row), has an unsaved edit.
    pub fn edited(&self, path: &str) -> bool {
        let under = format!("{path}.");
        self.edits.keys().any(|k| k == path || k.starts_with(&under))
    }
}

/// 一份 toml 裏寫着的每一項，`表.鍵` → 照它寫的那個樣子。
///
/// Warning: **值取的是「寫出來的字」而不是解析出來的東西**：面板要照原樣顯示，而
/// `"dense"` 與 `dense` 在螢幕上是兩件事——前者是使用者打的，後者是他打錯的。
fn declared_in(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return out;
    };
    for (table, item) in doc.iter() {
        if let Some(t) = item.as_table() {
            declare(&mut out, table, t);
        }
    }
    out
}

/// One table's values, and the tables inside it, all the way down
/// (2026-11-02): `[keys.flavor_items]` and `[ime.annotation_modes.double]`
/// are pages of the panel, and `[keys.normal]` says which keys a binding
/// took.
fn declare(out: &mut BTreeMap<String, String>, at: &str, t: &toml_edit::Table) {
    for (key, value) in t.iter() {
        match (value.as_table(), value.as_inline_table()) {
            (Some(inner), _) => declare(out, &format!("{at}.{key}"), inner),
            // `flavor_items = { tables = false }` is the same table, written
            // on one line (found in review, 2026-11-02).
            (_, Some(inline)) => {
                let inner = inline.clone().into_table();
                declare(out, &format!("{at}.{key}"), &inner);
            }
            _ => {
                if let Some(v) = value.as_value() {
                    out.insert(format!("{at}.{key}"), bare(v));
                }
            }
        }
    }
}

/// A value as written, without the comment after it: `false # off for now`
/// is `false`.
fn bare(v: &toml_edit::Value) -> String {
    let mut v = v.clone();
    v.decor_mut().clear();
    v.to_string().trim().to_string()
}

/// 一個 [`Change`] 寫進 toml 會是什麽樣子。
fn written(change: &Change) -> String {
    match change {
        Change::Tick(on) => on.to_string(),
        Change::Count(n) => n.to_string(),
        // Warning: **要轉義。** 對面 [`declared_in`] 讀的是 `toml_edit` 吐出來的字串，
        // 那一頭轉義過；這一頭不轉，值裏帶一個 `"` 或 `\` 兩邊就永遠對不上，
        // `settle` 判不出「改回原值」，那一筆從此掛着「改了還没存」。
        Change::Text(s) => toml_edit::Value::from(s.as_str()).to_string().trim().to_string(),
        // 撤掉那一項的人看見的不是一個值，[`Sheet::says`] 先接住了。
        Change::Drop => String::new(),
        Change::Written(repr) => repr.clone(),
    }
}

/// 兩個寫法說的是不是同一個值。
///
/// Warning: **不能逐字節比寫法**（2026-10-01）。同一個值 toml 有好幾種寫法，而兩
/// 頭挑的不一樣：`toml_edit::Value::from("a\"b\\c")` 挑的是**字面串**
/// `'a"b\c'`，而檔裏人手寫的多半是**基本串** `"a\"b\\c"`。`settle` 從前拿
/// [`written`] 的結果去比 `declared_in` 的原樣寫法，於是一旦值裏有 `"` 或
/// `\`，「改回原值」就永遠判不出來——那一行掛着「改了還没存」，每存一次還
/// 往檔裏重寫一遍。
///
/// 寫法不同不算不同；解析不出來的（面板不認得的寫法）退回逐字節比。
fn same_value(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (a.parse::<toml_edit::Value>(), b.parse::<toml_edit::Value>()) {
        (Ok(x), Ok(y)) => match (x.as_str(), y.as_str()) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
        _ => false,
    }
}

/// 檔裏那個寫法還原成一段字——[`written`] 的反函數。
///
/// Warning: **`trim_matches('"')` 不是它的反函數**（2026-10-01）。這一頭一直在用
/// 那一句，而 [`declared_in`] 交出來的是 `toml_edit` 的**原樣寫法**（「面板要照
/// 原樣顯示」），於是：
///
/// | 檔裏 | 從前畫成/編輯成 | 存回去變成 |
/// | --- | --- | --- |
/// | `"a\"b"` | `a\"b` | `"a\\\"b"`，反斜線每存一次多一根 |
/// | `"a\\b"` | `a\\b` | `"a\\\\b"`，同上 |
/// | `"一\n二"` | 字面的兩個字符 | `"一\\n二"` |
/// | `'a"b'`（字面串） | `'a"b'`，連引號一起 | `"'a\"b'"` |
///
/// 而 `settle` 拿 `written(改)` 去比 `declared_in` 那一份，一旦值裏有 `"` 或
/// `\` 就永遠比不上：那一行從此掛着「改了還没存」，每存一次再多一根反斜線。
/// 兩頭都走 `toml_edit` 就對得上了——`unwritten(written(s)) == s`。
pub fn unwritten(repr: &str) -> String {
    repr.parse::<toml_edit::Value>()
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        // 不是一段字（數字、真假），或者檔裏那一行本來就寫壞了：照原樣。
        .unwrap_or_else(|| repr.trim_matches('"').to_string())
}

/// **一項此刻長什麽樣** —— 面板一行畫的就是這個。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// 贏的那個值，照 toml 的寫法（`0`、`true`、`"dense"`）。
    pub value: String,
    /// 它從哪一層來。
    pub from: Layer,
    /// **被它蓋掉的那一個** —— 没有就是 `None`（贏的已經是出廠那一層）。
    pub under: Option<(Layer, String)>,
}

/// **那扇面板。**
#[derive(Debug, Clone)]
pub struct Panel {
    pub global: Sheet,
    pub local: Sheet,
    /// 改動落到哪一份上。
    pub into: Which,
    /// **The rows a search left**, as places in [`Panel::all_rows`]: `None`
    /// is every row (2026-11-08). The front end works it out, because the
    /// names a search reads are in the reader's language.
    pub found: Option<Vec<usize>>,
    /// The row the cursor is on, of [`Panel::rows`].
    pub row: usize,
    /// **A page is open**, on this row of it (2026-11-02). Which page is the
    /// row it was opened from ([`Panel::subpage`]). `None` is the ordinary
    /// list.
    pub page: Option<usize>,
    /// **The list a choice is picked from** (2026-11-08): `Enter` on a choice
    /// opens it, each value with what it means.
    pub popup: Option<Popup>,
    /// **Why the value typed was not taken** (2026-11-08): `Enter` on a value
    /// the config would refuse writes nothing, and this is drawn under the
    /// row while the value is typed again.
    pub refusal: Option<Refusal>,
    /// The schemes there are, from the input method, which this crate cannot
    /// see: `None` takes any name.
    pub schemes: Option<Names>,
    /// **What is being typed on the command row for the panel**, if anything:
    /// a value, or the search (2026-11-08). The front end opens the line and
    /// says so here, so the page can draw what goes with it.
    pub typing: Option<Typing>,
    /// The search in force, or being typed: empty is none.
    pub search: String,
    /// What `u` takes back, the last first; what `C-r` puts back.
    undo: Vec<Step>,
    redo: Vec<Step>,
}

/// **One change as written**, for `u` and `C-r`: each file it wrote.
type Step = Vec<Stepped>;

/// **One file's part of a change** (2026-11-09, reported: `u` after `r`
/// lost the comment after the line, and a table emptied and taken back came
/// back bare). The file as it was just before and just after, whole, so
/// taking the change back writes back exactly what was there; and the keys,
/// for a file changed outside since, where writing the whole text back
/// would undo that change too.
#[derive(Debug, Clone)]
struct Stepped {
    which: Which,
    path: PathBuf,
    /// The text read from the disk just before the change; `None` ＝ not
    /// there.
    before: Option<String>,
    /// The text written.
    after: Option<String>,
    /// `表.鍵`, what it said before and after (`None` ＝ not written).
    keys: Vec<(String, Option<String>, Option<String>)>,
}

/// What the command row is typing for the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typing {
    /// The value of the row under the cursor.
    Value,
    /// The search: the list narrows as it is typed.
    Search,
    /// **What follows a `:`** (2026-11-09, reported: the keys of `:e
    /// notes.md` ran as the panel's own, and `e` opened the config file and
    /// took the rest as typing). The line takes everything up to Enter or
    /// Esc; `q` and its kin close the panel, and anything else is said not
    /// to be taken.
    Command,
}

/// The list `Enter` opens on a choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    /// The values, as the config writes them.
    pub words: Vec<String>,
    /// The one the cursor is on.
    pub at: usize,
}

/// **Names a row takes that only the front end knows**: the schemes.
#[derive(Debug, Clone)]
pub struct Names {
    /// Whether a name is one of them, as the input method reads a name
    /// (`靈明` is `lingming`).
    pub takes: fn(&str) -> bool,
    /// Them, to say.
    pub names: Vec<String>,
}

/// **Why a value cannot be set** — the config's own rules, checked on
/// `Enter` (2026-11-08). Reported: `nosuch` as a theme and `ZZZ` as a page
/// were saved and said to be in force, and only the next start said they
/// were not; `99` became `64` and `abc` vanished without a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not a number. The domain is `low` to `high`, and `0` beside it when
    /// `zero` (0 means something of its own, outside the range).
    NotANumber { low: usize, high: usize, zero: bool },
    /// A number outside the domain.
    OutOfRange { low: usize, high: usize, zero: bool },
    /// Empty where a value is needed; the names it takes, when it takes names.
    Empty { names: Vec<String> },
    /// Not one of the names it takes.
    NotOneOf { word: String, names: Vec<String> },
    /// What the config loader says of the value, in the language it speaks.
    Loader(String),
}

/// **What a write did**: every file written, and which of them had been
/// changed outside since the panel read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    pub wrote: Vec<PathBuf>,
    /// Files taken away: `u` of a change that made the file.
    pub removed: Vec<PathBuf>,
    pub changed_underneath: Vec<PathBuf>,
}

/// **What one file of a step will hold after `u` or `C-r`** ([`Panel::replan`]):
/// the text written (`None` ＝ the file taken away), the text the step goes
/// back to after that, and whether it had been changed outside since.
struct Replanned {
    want: Option<String>,
    other: Option<String>,
    moved: bool,
}

/// **Why a change was not written.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The file is one the loader cannot read (a TOML error, or a name it
    /// does not take): writing into it would claim settings that are not in
    /// force. The line is 0 起算; the message is the parser's.
    Unread { path: PathBuf, line: usize, said: String },
    /// The disk said no.
    Io(String),
}

/// **Where the panel stands**, to tell whether a key moved anything
/// (2026-11-08): a message about a row goes stale when this changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    row: usize,
    page: Option<usize>,
    into: Which,
    popup: Option<Popup>,
    found: Option<Vec<usize>>,
    typing: Option<Typing>,
    texts: (String, String),
}

/// The pages a row can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subpage {
    /// `[keys.flavor_items]`.
    Flavor,
    /// `[ime.annotation_modes]`.
    Annotation,
}

/// One row of the annotation page: a field of a mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationRow {
    /// The mode's name, as the config writes it (`multi`, `double`, or one
    /// the config adds).
    pub mode: String,
    /// Out of [`crate::ANNOTATION_FIELDS`].
    pub field: &'static str,
}

/// One row of the flavour's page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlavorRow {
    /// The level, the same setting as the row in the list (`[keys] flavor`).
    Level,
    Item(FlavorItem),
}

/// The heading a run of item rows sits under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlavorSection {
    Basic,
    Full,
    /// Items for the other keymap, kept for when you switch.
    Other(KeyPreset),
}

/// Where an item's state comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemFrom {
    /// The level gives it.
    Level,
    /// Written by name in this layer.
    Layer(Layer),
}

/// **本項目那一份在哪** —— 找得到就是它，找不到就是「要建的話建在這裏」。
///
/// Warning: **不回 `None`。** 頂上那一行寫着要建的那一個，而寫的時候
/// `create_dir_all`；少了這一支，那句承諾就是空頭的。
pub fn local_sheet_path(cwd: &std::path::Path) -> PathBuf {
    crate::local_config_path(cwd).unwrap_or_else(|| cwd.join(".yumete").join("config.toml"))
}

/// **Every row, in the order the list draws them**: the groups in their
/// order, and in each its settings in [`SETTINGS`]'s order.
pub fn all_rows() -> Vec<&'static Setting> {
    GROUPS
        .iter()
        .flat_map(|named| SETTINGS.iter().filter(move |s| s.group == named.group))
        .collect()
}

impl Panel {
    /// 開一扇。
    pub fn open(global: Option<PathBuf>, local: Option<PathBuf>) -> Panel {
        Panel {
            global: Sheet::read(global),
            // 本項目那一份多半還不存在；路徑照樣給得出來（[`local_sheet_path`]），
            // 而它在不在寫在 `Sheet::there` 上，面板照着說。
            local: Sheet::read(local),
            into: Which::Global,
            found: None,
            row: 0,
            page: None,
            popup: None,
            refusal: None,
            schemes: None,
            typing: None,
            search: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Where the panel stands now: see [`Spot`].
    pub fn spot(&self) -> Spot {
        Spot {
            row: self.row,
            page: self.page,
            into: self.into,
            popup: self.popup.clone(),
            found: self.found.clone(),
            typing: self.typing,
            texts: (self.global.text.clone(), self.local.text.clone()),
        }
    }

    /// Whether the file switched to sets `path`, or a key under it (a page's
    /// row): the row gets a mark.
    pub fn set_here(&self, path: &str) -> bool {
        let under = format!("{path}.");
        self.sheet().declared.keys().any(|k| k == path || k.starts_with(&under))
    }

    /// The files that cannot be read, global first: the panel says so at the
    /// top.
    pub fn unread(&self) -> Vec<(Which, &(usize, String))> {
        [(Which::Global, &self.global), (Which::Local, &self.local)]
            .into_iter()
            .filter_map(|(which, sheet)| sheet.unread.as_ref().map(|u| (which, u)))
            .collect()
    }

    /// The rows shown: every row, or what a search left.
    pub fn rows(&self) -> Vec<&'static Setting> {
        let all = all_rows();
        match &self.found {
            None => all,
            Some(found) => found.iter().filter_map(|i| all.get(*i).copied()).collect(),
        }
    }

    /// 光標此刻站的那一項。
    pub fn here(&self) -> Option<&'static Setting> {
        self.rows().get(self.row).copied()
    }

    /// **A search's rows** (indices into [`all_rows`]), or every row again
    /// with `None`. The cursor stays on the row it was on when that row is
    /// still there, and goes to the first otherwise.
    pub fn narrow(&mut self, found: Option<Vec<usize>>) {
        let was = self.here().map(|s| s.path());
        self.found = found;
        self.row = was
            .and_then(|path| self.rows().iter().position(|s| s.path() == path))
            .unwrap_or(0);
    }

    /// **Go to a setting by name** (`:settings editor.scrolloff`): its TOML
    /// path, its key alone when only one setting has it, or its name as the
    /// panel shows it — `names` gives it in every language the front end
    /// has, so 首行缩进 finds the row whichever script the panel is drawn in
    /// (2026-11-09). Quotes around the name are let go, and case is not
    /// asked. Gives whether it was found; a search in force is let go.
    pub fn go_to(&mut self, name: &str, names: impl Fn(&Setting) -> Vec<String>) -> bool {
        let name = name.trim();
        let name = ['"', '\'']
            .iter()
            .find_map(|q| name.strip_prefix(*q).and_then(|n| n.strip_suffix(*q)))
            .unwrap_or(name)
            .trim();
        let all = all_rows();
        let by_path = all.iter().position(|s| s.path() == name);
        let by_key = || {
            let keyed: Vec<usize> = (0..all.len()).filter(|i| all[*i].key == name).collect();
            (keyed.len() == 1).then(|| keyed[0])
        };
        let folded = name.to_lowercase();
        let by_label = || all.iter().position(|s| names(s).iter().any(|n| n.trim().to_lowercase() == folded));
        match by_path.or_else(by_key).or_else(by_label) {
            Some(at) => {
                self.found = None;
                self.page = None;
                self.popup = None;
                self.row = at;
                true
            }
            None => false,
        }
    }

    /// **三層各自說了什麼** —— 出廠、全局、本項目，照面板上那一塊的次序（2026-10-13）。
    ///
    /// `None` ＝ 那一層沒有說話。出廠那一層永遠有話說，所以它那一格永遠是 `Some`。
    pub fn layers(&self, setting: &Setting) -> [(Layer, Option<String>); 3] {
        let path = setting.path();
        let said = |raw: Option<String>| raw.map(|v| Self::as_the_editor_takes_it(setting, v));
        [
            (Layer::Factory, Some(setting.factory.to_string())),
            (Layer::Global, said(self.global.says(&path))),
            (Layer::Local, said(self.local.says(&path))),
        ]
    }

    /// 一項此刻長什麽樣：贏的那一層，和被它蓋掉的那一個。
    pub fn shown(&self, setting: &Setting) -> Shown {
        let path = setting.path();
        let factory = (Layer::Factory, setting.factory.to_string());
        let global = self.global.says(&path).map(|v| (Layer::Global, v));
        let local = self.local.says(&path).map(|v| (Layer::Local, v));
        // 本項目蓋全局，全局蓋出廠。
        let mut stack = vec![factory];
        if let Some(g) = global {
            stack.push(g);
        }
        if let Some(l) = local {
            stack.push(l);
        }
        let (from, value) = stack.pop().expect("出廠那一層永遠在");
        Shown {
            value: Self::as_the_editor_takes_it(setting, value),
            from,
            under: stack.pop(),
        }
    }

    /// **面板顯示的要是編輯器真用的那個值**（2026-10-09 作者定）。
    ///
    /// 檔是人手改的：`zong_length = 2` 寫進去，`into_config` 按下界排版——面板顯示
    /// 的值不是編輯器用的值，而這扇面板存在的理由就是終結這件事。鉗的依據是這一項
    /// **自己 declare 的域**，所以它不會和 `into_config` 分岔。`[panel] markers`
    /// 空的時候 `into_config` 留出廠那一串，所以空的就畫出廠那一串。
    fn as_the_editor_takes_it(setting: &Setting, value: String) -> String {
        if setting.table == "panel" && setting.key == "markers" {
            let bare = value.trim().trim_matches('"');
            if bare.is_empty() {
                return setting.factory.to_string();
            }
            return value;
        }
        let Kind::Count { low, high, zero } = setting.kind else { return value };
        let Ok(n) = value.trim().parse::<usize>() else { return value };
        if n == 0 && zero.is_some() {
            return value;
        }
        n.clamp(low, high).to_string()
    }

    /// 改動要落到的那一份。
    fn sheet_mut(&mut self) -> &mut Sheet {
        self.sheet_mut_of(self.into)
    }

    /// 指名那一份，要改的。
    fn sheet_mut_of(&mut self, which: Which) -> &mut Sheet {
        match which {
            Which::Global => &mut self.global,
            Which::Local => &mut self.local,
        }
    }

    /// **A file changed outside since the panel last looked** (2026-11-09,
    /// reported: changed outside from 3 to 9, then set to 5 in the panel,
    /// `u` wrote 3): read again, so the panel shows what it says now and a
    /// change starts from it. A file gone is an empty one. Gives the files
    /// read again, for the front end to say once.
    pub fn refresh(&mut self) -> Vec<PathBuf> {
        let mut moved = Vec::new();
        for which in [Which::Global, Which::Local] {
            let sheet = self.sheet_mut_of(which);
            let Some(path) = sheet.path.clone() else { continue };
            if stamp(&path) == sheet.stamp {
                continue;
            }
            // Not readable just now: asked again next time.
            let Ok(read) = read_disk(&path) else { continue };
            let same = read.as_deref().unwrap_or("") == sheet.text && read.is_some() == sheet.there;
            sheet.took(read);
            if !same {
                moved.push(path);
            }
        }
        if !moved.is_empty() {
            // An annotation mode may have come or gone with the file.
            if let Some(at) = self.page {
                self.page = Some(at.min(self.page_len().saturating_sub(1)));
            }
        }
        moved
    }

    /// 改動要落到的那一份（只讀）。
    pub fn sheet(&self) -> &Sheet {
        self.sheet_of(self.into)
    }

    /// 指名那一份。
    pub fn sheet_of(&self, which: Which) -> &Sheet {
        match which {
            Which::Global => &self.global,
            Which::Local => &self.local,
        }
    }

    /// **切着的那一份此刻替這一項出的是什麽** —— 檔裏寫着的，没寫就是它**下面**
    /// 那幾層給的（全局底下只有出廠；本項目底下是全局，再底下纔是出廠）。
    fn without_mine(&self, setting: &Setting) -> String {
        let path = setting.path();
        let mine = self.sheet().declared.get(&path).cloned();
        let under = match self.into {
            Which::Global => None,
            Which::Local => self.global.says(&path),
        };
        mine.or(under).unwrap_or_else(|| setting.factory.to_string())
    }

    /// **切着的那一份此刻替這一項出的是什麽**：按空格與編輯的起點。
    ///
    /// Warning: **不是畫出來的那個值。** 本項目蓋着全局的時候你編輯全局，起點要是全局
    /// 自己那個值，不然「空格」會從別人的值往下數。
    pub fn mine_now(&self, setting: &Setting) -> String {
        self.sheet()
            .says(&setting.path())
            .unwrap_or_else(|| self.without_mine(setting))
    }

    /// **切着的那一份被上面那一層蓋着嗎**：蓋着的時候改這一項，螢幕上那一行不會
    /// 動，所以底下那一行要說一句。
    pub fn shadowed(&self, setting: &Setting) -> bool {
        self.into == Which::Global && self.local.says(&setting.path()).is_some()
    }

    /// **攢一筆改動 —— 除非改完和不改一樣**：轉一圈回到原來那個值，或者把一項設成
    /// 下面那幾層本來就給的，都不寫。
    fn settle(&mut self, setting: &Setting, change: Change) {
        let path = setting.path();
        match same_value(&written(&change), &self.without_mine(setting)) {
            true => {
                self.sheet_mut().edits.remove(&path);
            }
            false => {
                self.sheet_mut().edits.insert(path, change);
            }
        }
    }

    // ---- 走 -------------------------------------------------------------

    /// `j` / `k`: one row, across groups — a group's name is a heading, not a
    /// stop.
    pub fn step(&mut self, down: bool) {
        let many = self.rows().len();
        if many == 0 {
            return;
        }
        self.row = match down {
            true => (self.row + 1).min(many - 1),
            false => self.row.saturating_sub(1),
        };
    }

    /// `gg`.
    pub fn first(&mut self) {
        self.row = 0;
    }

    /// `G`.
    pub fn last(&mut self) {
        self.row = self.rows().len().saturating_sub(1);
    }

    /// The group's first row: a click on its name in the left column.
    pub fn to_group(&mut self, group: usize) {
        let Some(named) = GROUPS.get(group) else { return };
        if let Some(at) = self.rows().iter().position(|s| s.group == named.group) {
            self.row = at;
            self.page = None;
            self.popup = None;
        }
    }

    /// `h` / `l` —— **the group before or after** the cursor's, as the left
    /// column lists them (2026-11-09, asked for: "previously it was hl"): its
    /// first row, skipping a group a search left empty. At either end it
    /// stays.
    pub fn step_group(&mut self, down: bool) {
        let rows = self.rows();
        let here = self.group();
        let has = |g: usize| rows.iter().any(|s| s.group == GROUPS[g].group);
        let next = match down {
            true => (here + 1..GROUPS.len()).find(|&g| has(g)),
            false => (0..here).rev().find(|&g| has(g)),
        };
        if let Some(group) = next {
            self.to_group(group);
        }
    }

    /// The group the cursor is in, as a place in [`GROUPS`].
    pub fn group(&self) -> usize {
        self.here()
            .and_then(|s| GROUPS.iter().position(|n| n.group == s.group))
            .unwrap_or(0)
    }

    /// `Tab` —— 換寫到哪一份。
    pub fn flip_into(&mut self) {
        self.popup = None;
        self.refusal = None;
        self.into = self.into.other();
    }

    // ---- 改 -------------------------------------------------------------

    /// **Whether this row's value is typed**: a number or a piece of text.
    /// `Enter`, Space and `p` open the one-line editor on it.
    pub fn types_here(&self) -> bool {
        self.here().is_some_and(|s| matches!(s.kind, Kind::Count { .. } | Kind::Text))
    }

    /// What the one-line editor starts from: this file's value, or what the
    /// layers below give.
    pub fn typing_from(&self) -> String {
        self.here().map(|s| unwritten(&self.mine_now(s))).unwrap_or_default()
    }

    /// **Space**: a tick flips, a choice moves to the next, a page opens. A
    /// value that is typed is the front end's ([`Panel::types_here`]).
    pub fn press(&mut self) {
        let Some(setting) = self.here() else { return };
        let now = self.mine_now(setting);
        let next = match setting.kind {
            Kind::Page => return self.open_page(),
            Kind::Tick => Change::Tick(now != "true"),
            Kind::Pick(_) | Kind::PickOr(_) => {
                let words = self.choices(setting);
                if words.is_empty() {
                    return;
                }
                let word = unwritten(&now);
                // **一個不認得的名字撥到第一個，不是第二個。**
                let next = match words.iter().position(|w| *w == word) {
                    Some(at) => (at + 1) % words.len(),
                    None => 0,
                };
                Change::Text(words[next].clone())
            }
            Kind::Count { .. } | Kind::Text => return,
        };
        self.settle(setting, next);
    }

    /// **Enter**: a tick flips, a choice opens its list, a page opens. A value
    /// that is typed is the front end's ([`Panel::types_here`]).
    pub fn change(&mut self) {
        let Some(setting) = self.here() else { return };
        match setting.kind {
            Kind::Pick(_) | Kind::PickOr(_) => {
                let words = self.choices(setting);
                let now = unwritten(&self.mine_now(setting));
                let at = words.iter().position(|w| *w == now).unwrap_or(0);
                if !words.is_empty() {
                    self.popup = Some(Popup { words, at });
                }
            }
            _ => self.press(),
        }
    }

    /// **The values a choice takes**, as the config writes them: its own
    /// list; for a scheme, the ones installed when the front end said; for
    /// an annotation mode, `off` and every mode either file writes.
    pub fn choices(&self, setting: &Setting) -> Vec<String> {
        let (Kind::Pick(choices) | Kind::PickOr(choices)) = setting.kind else { return Vec::new() };
        match (setting.path().as_str(), &self.schemes) {
            ("ime.scheme", Some(schemes)) => schemes.names.clone(),
            ("ime.annotation", _) => std::iter::once("off".to_string()).chain(self.annotation_modes()).collect(),
            _ => choices.iter().map(|c| c.word.to_string()).collect(),
        }
    }

    /// `j` / `k` in the list a choice opened.
    pub fn popup_step(&mut self, down: bool) {
        if let Some(popup) = self.popup.as_mut() {
            let last = popup.words.len().saturating_sub(1);
            popup.at = match down {
                true => (popup.at + 1).min(last),
                false => popup.at.saturating_sub(1),
            };
        }
    }

    /// The value the list's cursor is on.
    pub fn popup_word(&self) -> Option<&str> {
        let popup = self.popup.as_ref()?;
        popup.words.get(popup.at).map(String::as_str)
    }

    /// `Enter` in the list: that value, for the row.
    pub fn popup_choose(&mut self) {
        let Some(popup) = self.popup.take() else { return };
        let (Some(setting), Some(word)) = (self.here(), popup.words.get(popup.at)) else { return };
        self.settle(setting, Change::Text(word.clone()));
    }

    /// **The value typed, on `Enter`** (2026-11-08): taken when the config
    /// would take it; otherwise nothing is set and [`Panel::refusal`] says
    /// why, naming what is taken. A number out of range is refused, not
    /// pulled into it, and a word that is not a number is refused, not
    /// dropped. Gives whether it was taken.
    pub fn commit_value(&mut self, typed: &str) -> bool {
        let Some(setting) = self.here() else { return false };
        match self.check(setting, typed) {
            Ok(change) => {
                self.refusal = None;
                self.settle(setting, change);
                true
            }
            Err(refusal) => {
                self.refusal = Some(refusal);
                false
            }
        }
    }

    /// **The value `typed` would be for `setting`, by the config's rules** —
    /// the domain a number declares (the one `into_config` holds it to),
    /// the names a theme, a scheme or an annotation mode takes, and for a
    /// piece of text what the loader itself says of it.
    pub fn check(&self, setting: &Setting, typed: &str) -> Result<Change, Refusal> {
        let word = typed.trim();
        match setting.kind {
            Kind::Count { low, high, zero } => {
                let apart = zero.is_some() && low > 0;
                let Ok(n) = word.parse::<i64>() else {
                    return Err(Refusal::NotANumber { low, high, zero: apart });
                };
                let inside = (n >= low as i64 && n <= high as i64) || (apart && n == 0);
                match inside {
                    true => Ok(Change::Count(n)),
                    false => Err(Refusal::OutOfRange { low, high, zero: apart }),
                }
            }
            Kind::PickOr(_) | Kind::Pick(_) => {
                let names = self.choices(setting);
                let takes = match setting.path().as_str() {
                    "theme.name" => crate::ThemeConfig::named(word).is_some(),
                    "ime.scheme" => self.schemes.as_ref().is_none_or(|s| (s.takes)(word)),
                    _ => names.iter().any(|n| n == word),
                };
                match (word.is_empty(), takes) {
                    (true, _) => Err(Refusal::Empty { names }),
                    (false, false) => Err(Refusal::NotOneOf { word: word.to_string(), names }),
                    (false, true) => Ok(Change::Text(word.to_string())),
                }
            }
            Kind::Tick => match word {
                "true" => Ok(Change::Tick(true)),
                "false" => Ok(Change::Tick(false)),
                _ => Err(Refusal::NotOneOf { word: word.to_string(), names: vec!["true".into(), "false".into()] }),
            },
            Kind::Text | Kind::Page => {
                // An empty list the loader quietly takes as no list at all,
                // the factory one standing: setting it would change nothing.
                let wants_some = matches!(setting.path().as_str(), "export.font_family" | "panel.markers");
                if wants_some && word.is_empty() {
                    return Err(Refusal::Empty { names: Vec::new() });
                }
                let said = crate::faults::said_of(setting.table, setting.key, &toml_edit::Value::from(typed));
                match said.into_iter().next() {
                    Some(said) => Err(Refusal::Loader(said)),
                    None => Ok(Change::Text(typed.to_string())),
                }
            }
        }
    }

    /// **`r` —— 把這一項從切着的那一份裏拿掉，回去跟下面那一層**（2026-11-08：
    /// 從前叫 `d`，而它有三個名字）。
    ///
    /// Gives whether anything was there to take (2026-11-08). On a row this
    /// file does not set, the row already follows the layer below, and the
    /// front end says so rather than leaving a key to do nothing on screen.
    pub fn reset(&mut self) -> Option<bool> {
        if self.page.is_some() {
            return self.page_drop();
        }
        let setting = self.here().filter(|s| s.kind != Kind::Page)?;
        Some(self.drop_path(setting.path()))
    }

    /// Drop `path` from the file switched to: whether that changed anything.
    fn drop_path(&mut self, path: String) -> bool {
        let sheet = self.sheet_mut();
        let was = sheet.edits.get(&path).cloned();
        // Warning: **問的是「檔裏寫了没有」，不是「這一份此刻說什麽」**：没寫的鍵拿不掉。
        match sheet.declared.contains_key(&path) {
            true => sheet.edits.insert(path.clone(), Change::Drop),
            false => sheet.edits.remove(&path),
        };
        was != sheet.edits.get(&path).cloned()
    }

    /// The value of the row as the editor takes it, for `y`.
    pub fn value_here(&self) -> Option<String> {
        let setting = self.here().filter(|s| s.kind != Kind::Page)?;
        Some(unwritten(&self.shown(setting).value))
    }

    /// **Where this row's key is in the file switched to**, 0 起算: its line,
    /// or its table's header, or the top (`e` opens the file there).
    pub fn line_here(&self) -> usize {
        let path = match (self.page, self.subpage()) {
            (Some(_), Some(Subpage::Flavor)) => match self.flavor_here() {
                Some(FlavorRow::Item(item)) => Self::item_path(item),
                _ => "keys.flavor".to_string(),
            },
            (Some(_), Some(Subpage::Annotation)) => match self.annotation_here() {
                Some(row) => Self::field_path(&row),
                None => "ime.annotation_modes".to_string(),
            },
            _ => match self.here() {
                Some(s) => s.path(),
                None => return 0,
            },
        };
        let (table, key) = path.rsplit_once('.').unwrap_or((path.as_str(), ""));
        crate::faults::line_for(&self.sheet().text, &format!("[{table}] {key}"))
    }

    // ---- The flavour's page (2026-11-02) ---------------------------------

    fn setting(path: &str) -> &'static Setting {
        SETTINGS.iter().find(|s| s.path() == path).expect("a row the panel has")
    }

    /// The level in force, as the panel shows it (unsaved edits count).
    pub fn flavor_level(&self) -> FlavorLevel {
        let shown = self.shown(Self::setting("keys.flavor"));
        FlavorLevel::parse(&unwritten(&shown.value)).unwrap_or_default()
    }

    /// The keymap in force, as the panel shows it.
    pub fn flavor_keymap(&self) -> KeyPreset {
        let shown = self.shown(Self::setting("keys.preset"));
        KeyPreset::parse(&unwritten(&shown.value)).unwrap_or_default()
    }

    /// The keymap an item is shown for: the one in force when it applies
    /// there, else its own.
    pub fn item_keymap(&self, item: FlavorItem) -> KeyPreset {
        let now = self.flavor_keymap();
        let keymaps = item.info().keymaps;
        match keymaps.contains(&now) {
            true => now,
            false => keymaps.first().copied().unwrap_or(now),
        }
    }

    fn item_path(item: FlavorItem) -> String {
        format!("keys.flavor_items.{}", item.id())
    }

    /// What the level alone makes of an item.
    pub fn item_by_level(&self, item: FlavorItem) -> bool {
        Flavor { level: self.flavor_level(), items: Default::default() }.on(item, self.item_keymap(item))
    }

    fn said_bool(raw: Option<String>) -> Option<bool> {
        raw.and_then(|v| v.trim().parse::<bool>().ok())
    }

    /// The three layers of an item: the level's answer, then each file's.
    pub fn item_layers(&self, item: FlavorItem) -> [(Layer, Option<bool>); 3] {
        let path = Self::item_path(item);
        [
            (Layer::Factory, Some(self.item_by_level(item))),
            (Layer::Global, Self::said_bool(self.global.says(&path))),
            (Layer::Local, Self::said_bool(self.local.says(&path))),
        ]
    }

    /// An item as it is now, and where that comes from. A name in either
    /// file beats the level, wherever the level is set.
    pub fn item_shown(&self, item: FlavorItem) -> (bool, ItemFrom) {
        let [(_, level), (_, global), (_, local)] = self.item_layers(item);
        match (local, global) {
            (Some(on), _) => (on, ItemFrom::Layer(Layer::Local)),
            (None, Some(on)) => (on, ItemFrom::Layer(Layer::Global)),
            (None, None) => (level.unwrap_or(false), ItemFrom::Level),
        }
    }

    /// What the item would be without the sheet being edited.
    fn item_below(&self, item: FlavorItem) -> bool {
        let path = Self::item_path(item);
        match self.into {
            Which::Global => self.item_by_level(item),
            Which::Local => Self::said_bool(self.global.says(&path)).unwrap_or_else(|| self.item_by_level(item)),
        }
    }

    /// **The keys of an item a binding has taken**, in `[keys.normal]` (or
    /// `[keys.insert]`) of either file: that line wins over the item.
    pub fn item_taken(&self, item: FlavorItem) -> Vec<&'static str> {
        let table = match item.info().insert {
            true => "keys.insert.",
            false => "keys.normal.",
        };
        let written = |k: &str| yumete_cjk::actions::written(k);
        item.keys_under(self.item_keymap(item))
            .iter()
            .copied()
            .filter(|key| {
                [&self.global, &self.local].iter().any(|sheet| {
                    sheet.declared.keys().any(|path| {
                        path.strip_prefix(table)
                            .is_some_and(|bound| written(bound.trim_matches('"')) == written(key))
                    })
                })
            })
            .collect()
    }

    /// The page's rows, each with the heading that starts at it: the level,
    /// then this keymap's basic items, its full items, and the other
    /// keymap's.
    pub fn flavor_rows(&self) -> Vec<(Option<FlavorSection>, FlavorRow)> {
        let keymap = self.flavor_keymap();
        let mut rows = vec![(None, FlavorRow::Level)];
        let mut push = |section: FlavorSection, items: Vec<FlavorItem>| {
            for (n, item) in items.into_iter().enumerate() {
                rows.push(((n == 0).then_some(section), FlavorRow::Item(item)));
            }
        };
        let here = |level: FlavorLevel| -> Vec<FlavorItem> {
            FlavorItem::ALL
                .iter()
                .copied()
                .filter(|i| i.info().keymaps.contains(&keymap) && i.info().level == level)
                .collect()
        };
        push(FlavorSection::Basic, here(FlavorLevel::Basic));
        push(FlavorSection::Full, here(FlavorLevel::Full));
        for other in KeyPreset::ALL.into_iter().filter(|k| *k != keymap) {
            let items: Vec<FlavorItem> = FlavorItem::ALL
                .iter()
                .copied()
                .filter(|i| !i.info().keymaps.contains(&keymap) && i.info().keymaps.contains(&other))
                .collect();
            push(FlavorSection::Other(other), items);
        }
        rows
    }

    /// How many of this keymap's items are on, of how many; and how many
    /// are written by name in either file.
    pub fn flavor_counts(&self) -> (usize, usize, usize) {
        let keymap = self.flavor_keymap();
        let here: Vec<FlavorItem> =
            FlavorItem::ALL.iter().copied().filter(|i| i.info().keymaps.contains(&keymap)).collect();
        let on = here.iter().filter(|i| self.item_shown(**i).0).count();
        let named = FlavorItem::ALL
            .iter()
            .filter(|i| self.item_shown(**i).1 != ItemFrom::Level)
            .count();
        (on, here.len(), named)
    }

    /// The page's row the cursor is on.
    pub fn flavor_here(&self) -> Option<FlavorRow> {
        let at = self.page.filter(|_| self.subpage() == Some(Subpage::Flavor))?;
        self.flavor_rows().get(at).map(|(_, row)| *row)
    }

    // ---- Pages -----------------------------------------------------------

    /// The page the row under the cursor opens, if it opens one.
    pub fn subpage(&self) -> Option<Subpage> {
        let setting = self.here().filter(|s| s.kind == Kind::Page)?;
        match setting.path().as_str() {
            "ime.annotation_modes" => Some(Subpage::Annotation),
            _ => Some(Subpage::Flavor),
        }
    }

    /// How many rows the open page has.
    pub fn page_len(&self) -> usize {
        match self.subpage() {
            Some(Subpage::Flavor) => self.flavor_rows().len(),
            Some(Subpage::Annotation) => self.annotation_rows().len(),
            None => 0,
        }
    }

    /// Go in (`l`, Space, Enter, `i` on the entry row).
    pub fn open_page(&mut self) {
        self.popup = None;
        self.refusal = None;
        self.page = Some(0);
    }

    /// Come back to the entry row (`h`, Esc).
    pub fn close_page(&mut self) {
        self.page = None;
    }

    /// `j` / `k` on the page.
    pub fn page_step(&mut self, down: bool) {
        let many = self.page_len();
        if let Some(at) = self.page.as_mut() {
            *at = match down {
                true => (*at + 1).min(many.saturating_sub(1)),
                false => at.saturating_sub(1),
            };
        }
    }

    /// Space, Enter, `i` on the page.
    pub fn page_press(&mut self) {
        match self.subpage() {
            Some(Subpage::Flavor) => self.flavor_press(),
            Some(Subpage::Annotation) => self.annotation_press(),
            None => {}
        }
    }

    /// `d` on the page: whether anything was there to drop, as
    /// [`Panel::drop_here`].
    pub fn page_drop(&mut self) -> Option<bool> {
        let path = match self.subpage()? {
            Subpage::Flavor => match self.flavor_here()? {
                FlavorRow::Level => "keys.flavor".to_string(),
                FlavorRow::Item(item) => Self::item_path(item),
            },
            Subpage::Annotation => Self::field_path(&self.annotation_here()?),
        };
        Some(self.drop_path(path))
    }

    /// **A tick on a page flips.** Flipped to what the layers below give,
    /// the line goes from the file, so the tick follows them again.
    fn flip(&mut self, path: String, below: bool) {
        let now = Self::said_bool(self.sheet().says(&path)).unwrap_or(below);
        let want = !now;
        let declared = Self::said_bool(self.sheet().declared.get(&path).cloned());
        let sheet = self.sheet_mut();
        match (want == below, declared) {
            (true, Some(_)) => {
                sheet.edits.insert(path, Change::Drop);
            }
            (true, None) => {
                sheet.edits.remove(&path);
            }
            (false, Some(was)) if was == want => {
                sheet.edits.remove(&path);
            }
            (false, _) => {
                sheet.edits.insert(path, Change::Tick(want));
            }
        }
    }

    // ---- The annotation page (2026-11-02) ----------------------------------

    fn field_path(row: &AnnotationRow) -> String {
        format!("ime.annotation_modes.{}.{}", row.mode, row.field)
    }

    /// The modes, as the panel shows them: yume's two, then every name
    /// either file writes, in order.
    pub fn annotation_modes(&self) -> Vec<String> {
        let mut names: Vec<String> =
            crate::FACTORY_ANNOTATION_MODES.iter().map(|(n, _)| n.to_string()).collect();
        let mut more = std::collections::BTreeSet::new();
        for sheet in [&self.global, &self.local] {
            for path in sheet.declared.keys().chain(sheet.edits.keys()) {
                let Some(rest) = path.strip_prefix("ime.annotation_modes.") else { continue };
                if let Some((name, _)) = rest.rsplit_once('.') {
                    if crate::annotation_name_usable(name) && !names.iter().any(|n| n == name) {
                        more.insert(name.to_string());
                    }
                }
            }
        }
        names.extend(more);
        names
    }

    /// The page's rows: each mode's eight fields, the mode's name heading
    /// its first.
    pub fn annotation_rows(&self) -> Vec<(Option<String>, AnnotationRow)> {
        let mut rows = Vec::new();
        for mode in self.annotation_modes() {
            for (n, field) in crate::ANNOTATION_FIELDS.iter().enumerate() {
                rows.push(((n == 0).then(|| mode.clone()), AnnotationRow { mode: mode.clone(), field }));
            }
        }
        rows
    }

    /// The page's row the cursor is on.
    pub fn annotation_here(&self) -> Option<AnnotationRow> {
        let at = self.page.filter(|_| self.subpage() == Some(Subpage::Annotation))?;
        self.annotation_rows().into_iter().nth(at).map(|(_, row)| row)
    }

    /// What the factory says of a field: yume's two modes have their ticks,
    /// a mode the config adds has none.
    fn field_by_factory(row: &AnnotationRow) -> bool {
        crate::FACTORY_ANNOTATION_MODES
            .iter()
            .any(|(name, fields)| *name == row.mode && fields.contains(&row.field))
    }

    /// The three layers of a field.
    pub fn field_layers(&self, row: &AnnotationRow) -> [(Layer, Option<bool>); 3] {
        let path = Self::field_path(row);
        [
            (Layer::Factory, Some(Self::field_by_factory(row))),
            (Layer::Global, Self::said_bool(self.global.says(&path))),
            (Layer::Local, Self::said_bool(self.local.says(&path))),
        ]
    }

    /// A field as it is now, and the layer that says so.
    pub fn field_shown(&self, row: &AnnotationRow) -> (bool, Layer) {
        let [(_, factory), (_, global), (_, local)] = self.field_layers(row);
        match (local, global) {
            (Some(on), _) => (on, Layer::Local),
            (None, Some(on)) => (on, Layer::Global),
            (None, None) => (factory.unwrap_or(false), Layer::Factory),
        }
    }

    /// How many fields a mode shows.
    pub fn mode_fields(&self, mode: &str) -> Vec<&'static str> {
        crate::ANNOTATION_FIELDS
            .iter()
            .copied()
            .filter(|field| self.field_shown(&AnnotationRow { mode: mode.to_string(), field }).0)
            .collect()
    }

    fn annotation_press(&mut self) {
        let Some(row) = self.annotation_here() else { return };
        let path = Self::field_path(&row);
        let below = match self.into {
            Which::Global => Self::field_by_factory(&row),
            Which::Local => Self::said_bool(self.global.says(&path)).unwrap_or_else(|| Self::field_by_factory(&row)),
        };
        self.flip(path, below);
    }

    /// Space, Enter, `i`: the level moves on, an item flips. **Flipped to
    /// what the layers below give, the line goes from the file**, so the
    /// item follows the level again and a level change later moves it.
    fn flavor_press(&mut self) {
        match self.flavor_here() {
            Some(FlavorRow::Level) => {
                let level = Self::setting("keys.flavor");
                let now = unwritten(&self.mine_now(level));
                let at = FlavorLevel::NAMES.iter().position(|n| *n == now).unwrap_or(0);
                let next = FlavorLevel::NAMES[(at + 1) % FlavorLevel::NAMES.len()];
                self.settle(level, Change::Text(next.to_string()));
            }
            Some(FlavorRow::Item(item)) => {
                let below = self.item_below(item);
                self.flip(Self::item_path(item), below);
            }
            None => {}
        }
    }

    // ---- 寫 -------------------------------------------------------------

    /// Whether a change is waiting to be written.
    pub fn dirty(&self) -> bool {
        self.global.dirty() || self.local.dirty()
    }

    /// **Write what was just changed** (2026-11-08, the save model the
    /// author chose: every confirmed change is written at once and takes
    /// effect; `u` takes it back). `Ok(None)` when there was nothing to
    /// write. A change that cannot be written is let go, so the panel shows
    /// the files as they are.
    pub fn flush(&mut self) -> Result<Option<Saved>, SaveError> {
        self.write()
    }

    /// **`u`**: the last change written, written back. `Ok(None)` when
    /// nothing is left to take back.
    pub fn undo(&mut self) -> Result<Option<Saved>, SaveError> {
        self.replay(true)
    }

    /// **`C-r`**: the last change `u` took back, written again.
    pub fn redo(&mut self) -> Result<Option<Saved>, SaveError> {
        self.replay(false)
    }

    /// **One step of `u` (`back`) or `C-r`**, written and handed to the
    /// other stack (2026-11-09). A file the disk still holds as that step
    /// left it gets its other text back exactly, comments and order and all;
    /// a file changed outside since gets the step's keys set again on what
    /// it holds now, so the change outside stays.
    fn replay(&mut self, back: bool) -> Result<Option<Saved>, SaveError> {
        let popped = match back {
            true => self.undo.pop(),
            false => self.redo.pop(),
        };
        let Some(mut step) = popped else { return Ok(None) };
        let keep = |panel: &mut Panel, step: Step| match back {
            true => panel.undo.push(step),
            false => panel.redo.push(step),
        };
        // Every file is asked before any is written.
        let plan = match self.replan(&step, back) {
            Ok(plan) => plan,
            Err(err) => {
                keep(self, step);
                return Err(err);
            }
        };
        let mut saved = Saved::default();
        for (entry, planned) in step.iter_mut().zip(plan) {
            let Replanned { want, other, moved } = planned;
            if let Err(err) = put(&entry.path, want.as_deref()) {
                keep(self, step);
                return Err(err);
            }
            // The step now says what this write did, so the way back is exact
            // again, a change made outside meanwhile included.
            match back {
                true => (entry.before, entry.after) = (want.clone(), other),
                false => (entry.before, entry.after) = (other, want.clone()),
            }
            if moved {
                saved.changed_underneath.push(entry.path.clone());
            }
            match want.is_some() {
                true => saved.wrote.push(entry.path.clone()),
                false => saved.removed.push(entry.path.clone()),
            }
            self.sheet_mut_of(entry.which).took(want);
        }
        match back {
            true => self.redo.push(step),
            false => self.undo.push(step),
        }
        Ok(Some(saved))
    }

    /// What each file of a step will hold after `u` (`back`) or `C-r`, and
    /// what the step will take it back to next.
    ///
    /// Warning: **a file changed outside since gets the step's keys set on
    /// what it holds now, and so does the way back** (2026-11-09, reported:
    /// set to 5, changed outside to 7, `u` wrote 3 over the 7 and `C-r` then
    /// wrote the 7 back, not the 5). The way back was the text the disk held
    /// before `u`, which is the change outside, not the step's; it is now
    /// what `u` wrote with the step's keys set again — and a file deleted
    /// outside, taken back by `u`, comes back from `C-r` with the change in
    /// it rather than gone again.
    fn replan(&self, step: &Step, back: bool) -> Result<Vec<Replanned>, SaveError> {
        let keyed = |entry: &Stepped, text: &str, back: bool| -> Result<String, SaveError> {
            let changes: Vec<(String, Change)> = entry
                .keys
                .iter()
                .map(|(key, before, after)| {
                    let to = if back { before } else { after };
                    (key.clone(), to.clone().map_or(Change::Drop, Change::Written))
                })
                .collect();
            rewrite(text, &changes).map_err(SaveError::Io)
        };
        step.iter()
            .map(|entry| {
                let disk = read_disk(&entry.path)?;
                let (from, want) = match back {
                    true => (&entry.after, &entry.before),
                    false => (&entry.before, &entry.after),
                };
                if disk == *from {
                    return Ok(Replanned { want: want.clone(), other: disk, moved: false });
                }
                let now = disk.as_deref().unwrap_or("");
                if let Some((line, said)) = crate::faults::unread(now) {
                    return Err(SaveError::Unread { path: entry.path.clone(), line, said });
                }
                let fresh = keyed(entry, now, back)?;
                // A file gone, and nothing to put in it: it stays gone.
                let want = match disk.is_none() && fresh.trim().is_empty() {
                    true => None,
                    false => Some(fresh),
                };
                let other = keyed(entry, want.as_deref().unwrap_or(""), !back)?;
                let other = match want.is_none() && other.trim().is_empty() {
                    true => None,
                    false => Some(other),
                };
                Ok(Replanned { want, other, moved: true })
            })
            .collect()
    }

    /// **兩份各寫各的，寫完把改動清掉**，記下這一步給 `u`。
    ///
    /// Warning: **A file the loader cannot read is not written** (2026-11-08),
    /// and then nothing is: both are asked before either is touched. Writing
    /// into it would leave it as unread as before and say the settings were
    /// in force (reported: a TOML error came back as a raw, truncated English
    /// line inside the Chinese panel).
    fn write(&mut self) -> Result<Option<Saved>, SaveError> {
        if !self.dirty() {
            return Ok(None);
        }
        let planned = self.plan();
        let plan = match planned {
            Ok(plan) => plan,
            Err(err) => {
                self.global.edits.clear();
                self.local.edits.clear();
                return Err(err);
            }
        };
        let mut saved = Saved::default();
        let mut step: Step = Vec::new();
        for (which, path, now, fresh) in plan {
            if let Err(err) = put(&path, Some(&fresh)) {
                self.global.edits.clear();
                self.local.edits.clear();
                return Err(err);
            }
            let sheet = self.sheet_mut_of(which);
            // Warning: **"before" is what the disk held just now**, not what the
            // panel read at opening (2026-11-09, reported: changed outside from
            // 3 to 9, set to 5 here, and `u` wrote 3).
            let was = declared_in(now.as_deref().unwrap_or(""));
            let keys: Vec<(String, Option<String>, Option<String>)> = std::mem::take(&mut sheet.edits)
                .into_iter()
                .map(|(key, change)| {
                    let after = match change {
                        Change::Drop => None,
                        other => Some(written(&other)),
                    };
                    (key.clone(), was.get(&key).cloned(), after)
                })
                .filter(|(_, before, after)| before != after)
                .collect();
            let moved = now.as_deref().unwrap_or("") != sheet.text || now.is_some() != sheet.there;
            if now.as_deref() != Some(fresh.as_str()) {
                step.push(Stepped { which, path: path.clone(), before: now, after: Some(fresh.clone()), keys });
            }
            sheet.took(Some(fresh));
            if moved {
                saved.changed_underneath.push(path.clone());
            }
            saved.wrote.push(path);
        }
        if !step.is_empty() {
            self.undo.push(step);
            self.redo.clear();
        }
        Ok(Some(saved))
    }

    /// What each file with a change will say, read afresh from the disk:
    /// the file, what the disk holds now (`None` ＝ not there), and the new
    /// text.
    #[allow(clippy::type_complexity)]
    fn plan(&self) -> Result<Vec<(Which, PathBuf, Option<String>, String)>, SaveError> {
        let mut plan = Vec::new();
        for which in [Which::Global, Which::Local] {
            let sheet = self.sheet_of(which);
            if sheet.edits.is_empty() {
                continue;
            }
            let Some(path) = sheet.path.clone() else {
                return Err(SaveError::Io(format!("{which:?}")));
            };
            // Warning: **照磁碟上此刻那一份改，不是照開面板那一刻那一份。** 面板開着的
            // 這段時間裏那個檔可能被外面改了，拿舊正文改完整份寫回去會把人家改的
            // 別的鍵一起頂掉。改的那幾項是「鍵 → 新值」，套在哪一份正文上都成立。
            //
            // Warning: **A file deleted outside is an empty one** (2026-11-09,
            // reported: it was written back whole from the text the panel had
            // read, every line the deletion took coming back). One that cannot
            // be read is not written at all.
            let now = read_disk(&path)?;
            let text = now.as_deref().unwrap_or("");
            if let Some((line, said)) = crate::faults::unread(text) {
                return Err(SaveError::Unread { path, line, said });
            }
            let changes: Vec<(String, Change)> =
                sheet.edits.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let fresh = rewrite(text, &changes).map_err(SaveError::Io)?;
            plan.push((which, path, now, fresh));
        }
        Ok(plan)
    }

    /// Whether `u` / `C-r` have anything to do.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

/// **寫一份檔，要麽整份是新的，要麽整份是舊的。**
///
/// 寫暫存檔 → `sync_all` → `rename`，同 `yumete-core` 存稿子那一支
/// （`buffer.rs::write_atomically`，那一頭不能借：核心不依賴這個 crate，反過來
/// 更不行）。
///
/// Warning: **直接 `fs::write` 有一個真的窗口**：它先把檔截成 0 再寫。寫到一半斷電、
/// 磁碟滿、程序被殺——使用者的 `config.toml` 就剩半份，而那半份多半讀不通，於是
/// **整份設定作廢**（`deny_unknown_fields` 之外的第二種全丟法）。一個人只是想
/// 把縮進改成 2。
///
/// Warning: **`rename` 換的是目録項，所以要連目録一起 `sync`**，否則斷電後目録可能還
/// 指着舊的那個 inode——新內容落了盤，而没有人找得到它。
fn write_atomically(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    // Warning: **穿過鏈接，不是蓋在它上面。** 同 `yumete-core` 存稿子那一支
    // （`buffer.rs:1324`，那裏的註釋寫着同一句）。`~/.config/yumete/config.toml`
    // 常常是一條指向 dotfiles 的軟鏈（chezmoi/stow/yadm），而 `rename` 換的是
    // **目録項**——不穿過去的話，存一次就把那條鏈接換成一個普通檔，dotfiles 裏
    // 那一份從此原封不動地被甩掉，而 `git status` 裏什麽都看不見。
    // 2026-09-24 審出來的，實測。
    let path = &std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // The same shape as yumete-core's temporary files (2026-10-31), so one
    // ignore line, `*.yumete-tmp`, covers a crash in the middle of either.
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{}.{}-{}.yumete-tmp", name.trim_start_matches('.'), std::process::id(), nanos));
    let written = (|| -> std::io::Result<()> {
        // Warning: **只讀的檔就不動它。** 同 `buffer.rs:1337`：`rename` 換的是目録項，
        // 檔自己的權限攔不住——`chmod 444 config.toml` 是有人說「這份別動」。
        if let Ok(from) = std::fs::metadata(path) {
            if from.permissions().readonly() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "read-only",
                ));
            }
        }
        let mut file = std::fs::File::create(&tmp)?;
        // 原來那份檔的權限，留着：`File::create` 吃 umask，於是一份 0600 的配置
        // 存一次就成了 0644。
        if let Ok(from) = std::fs::metadata(path) {
            let _ = file.set_permissions(from.permissions());
        }
        file.write_all(text.as_bytes())?;
        file.flush()?;
        // flush 只清掉本行程的緩衝。rename 落了盤而數據塊没落，斷電後是一份全零。
        file.sync_all()
    })();
    // 每一條出路都把目録還原成它來時的樣子——磁碟滿的那一次不留一個
    // `….yumete-tmp` 在人家的 `.config` 裏。
    if let Err(err) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(global: &str, local: &str) -> Panel {
        let mut p = Panel::open(None, None);
        p.global = Sheet::of_text(Some(PathBuf::from("/nowhere/config.toml")), true, global.to_string());
        p.local = Sheet::of_text(Some(PathBuf::from("/nowhere/.yumete/config.toml")), true, local.to_string());
        p
    }

    fn find(path: &str) -> &'static Setting {
        SETTINGS.iter().find(|s| s.path() == path).expect(path)
    }

    /// A panel on `path`, the cursor on it.
    fn on(mut p: Panel, path: &str) -> Panel {
        assert!(p.go_to(path, |_| Vec::new()), "{path}");
        p
    }

    /// A fresh directory under the temporary one, for files the test writes.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yumete-set-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 三層，後面的蓋前面的，**而被蓋掉的那一個說得出來**。
    #[test]
    fn the_layer_that_wins_is_named_and_so_is_the_one_under_it() {
        let p = panel("[editor]\nindent = 4\n", "[editor]\nindent = 2\n");
        let shown = p.shown(find("editor.indent"));
        assert_eq!(shown.value, "2");
        assert_eq!(shown.from, Layer::Local);
        assert_eq!(shown.under, Some((Layer::Global, "4".to_string())));
        let bare = panel("", "");
        let shown = bare.shown(find("editor.indent"));
        assert_eq!((shown.value.as_str(), shown.from, shown.under), ("0", Layer::Factory, None));
    }

    /// **面板顯示的要是編輯器真用的那個值**（2026-10-09 作者定）。
    #[test]
    fn the_panel_shows_what_the_editor_takes_not_what_the_file_says() {
        let p = panel("[editor]\nzong_length = 2\npages_vertical = 99\n\n[panel]\nmarkers = \"\"\n", "");
        assert_eq!(p.shown(find("editor.zong_length")).value, "8", "域是「0，或者 8 到 64」");
        assert_eq!(p.shown(find("editor.pages_vertical")).value, "4", "上限夾住");
        assert_eq!(p.shown(find("panel.markers")).value, find("panel.markers").factory, "空的留出廠那一串");
        let zero = panel("[editor]\nzong_length = 0\n", "");
        assert_eq!(zero.shown(find("editor.zong_length")).value, "0", "0 ＝與窗口同高");
    }

    /// **One list, the groups its headings** (2026-11-08): `j` walks from
    /// the last row of a group to the first of the next, `gg` and `G` go to
    /// the ends, and a click on a group's name goes to its first row.
    #[test]
    fn the_rows_are_one_list_across_the_groups() {
        let mut p = panel("", "");
        let all = all_rows();
        assert_eq!(all.len(), SETTINGS.len(), "every setting, once");
        let first_of_marks = all.iter().position(|s| s.group == crate::settings_ui::Group::Marks).unwrap();
        p.row = first_of_marks - 1;
        assert_eq!(p.group(), 0);
        p.step(true);
        assert_eq!((p.row, p.group()), (first_of_marks, 1), "into the next group");
        p.last();
        assert_eq!(p.row, all.len() - 1);
        p.step(true);
        assert_eq!(p.row, all.len() - 1, "stops at the end");
        p.first();
        p.step(false);
        assert_eq!(p.row, 0);
        p.to_group(4);
        assert_eq!(p.group(), 4);
        assert_eq!(p.here().map(|s| s.group), Some(GROUPS[4].group));
    }

    /// **A search narrows the list and keeps the cursor on its row** when
    /// that row is left; with no search every row is back.
    #[test]
    fn a_search_narrows_the_rows() {
        let mut p = on(panel("", ""), "editor.scrolloff");
        let all = all_rows();
        let scrolloff = all.iter().position(|s| s.key == "scrolloff").unwrap();
        let wheel = all.iter().position(|s| s.key == "wheel_step").unwrap();
        p.narrow(Some(vec![wheel, scrolloff]));
        assert_eq!(p.rows().len(), 2);
        assert_eq!(p.here().map(|s| s.key), Some("scrolloff"), "still on it");
        p.narrow(Some(vec![wheel]));
        assert_eq!((p.row, p.here().map(|s| s.key)), (0, Some("wheel_step")));
        p.narrow(None);
        assert_eq!(p.rows().len(), all.len());
        assert_eq!(p.here().map(|s| s.key), Some("wheel_step"));
    }

    /// **`:settings <name>`** goes to a setting by its TOML path, by its key
    /// when only one setting has that key, or by its shown name.
    #[test]
    fn go_to_finds_a_setting_by_path_key_or_name() {
        let mut p = panel("", "");
        assert!(p.go_to("export.page", |_| Vec::new()));
        assert_eq!(p.here().map(|s| s.path()), Some("export.page".into()));
        assert!(p.go_to("scrolloff", |_| Vec::new()));
        assert_eq!(p.here().map(|s| s.key), Some("scrolloff"));
        assert!(!p.go_to("editor", |_| Vec::new()), "a table is not a setting");
        assert!(p.go_to("光標邊距", |s| if s.key == "scrolloff" { vec!["光標邊距".into()] } else { Vec::new() }));
        assert!(!p.go_to("nosuch", |_| Vec::new()));
    }

    /// Space: a tick flips, a choice moves on; a number waits for the editor.
    #[test]
    fn a_press_moves_each_kind_the_way_that_kind_moves() {
        let mut p = on(panel("", ""), "editor.soft_wrap");
        p.press();
        assert_eq!(p.shown(find("editor.soft_wrap")).value, "false");
        let mut p = on(p, "editor.layout");
        p.press();
        assert_eq!(p.shown(find("editor.layout")).value, "\"vertical\"");
        p.press();
        assert_eq!(p.shown(find("editor.layout")).value, "\"horizontal\"", "轉回來");
        let mut p = on(p, "editor.pages_vertical");
        assert!(p.types_here());
        p.press();
        assert_eq!(p.shown(find("editor.pages_vertical")).value, "1", "typed, not stepped");
        assert_eq!(p.typing_from(), "1");
    }

    /// **Enter on a choice opens its list**, on the value in force; `j` and
    /// `Enter` pick another.
    #[test]
    fn enter_on_a_choice_opens_its_list() {
        let mut p = on(panel("[editor]\nmargin = \"loose\"\n", ""), "editor.margin");
        p.change();
        let popup = p.popup.clone().expect("a list");
        assert_eq!(popup.words, ["never", "dense", "loose", "always"]);
        assert_eq!(p.popup_word(), Some("loose"));
        p.popup_step(true);
        p.popup_step(true);
        assert_eq!(p.popup_word(), Some("always"), "stops at the end");
        p.popup_choose();
        assert!(p.popup.is_none());
        assert_eq!(p.shown(find("editor.margin")).value, "\"always\"");
        // The theme's list is the themes; a tick has none.
        let mut p = on(p, "theme.name");
        p.change();
        assert_eq!(p.popup.as_ref().map(|l| l.words.len()), Some(10));
        let mut p = on(p, "editor.soft_wrap");
        p.popup = None;
        p.change();
        assert!(p.popup.is_none(), "a tick flips instead");
        assert_eq!(p.shown(find("editor.soft_wrap")).value, "false");
    }

    /// **`r` takes this file's line away**, back to the layer below, and
    /// says when there was none.
    #[test]
    fn reset_falls_back_to_the_layer_below() {
        let mut p = on(panel("[editor]\nindent = 4\n", "[editor]\nindent = 2\n"), "editor.indent");
        p.into = Which::Local;
        assert_eq!(p.reset(), Some(true));
        let shown = p.shown(find("editor.indent"));
        assert_eq!((shown.value.as_str(), shown.from), ("4", Layer::Global));
        let mut p = on(panel("[editor]\nindent = 4\n", ""), "editor.indent");
        p.into = Which::Local;
        assert_eq!(p.reset(), Some(false), "this file never said it");
        assert!(!p.dirty());
    }

    /// **檔裏帶引號或反斜線的一段字，來回一趟要一個字不差**（2026-10-01）。
    #[test]
    fn a_quote_in_a_setting_survives_the_round_trip() {
        // One character each since 2026-11-09, when a paragraph mark became
        // one character: a quote, and a backslash.
        for (file, value) in [("\"\\\"\"", "\""), ("\"\\\\\"", "\\")] {
            let mut p = on(panel(&format!("[editor]\nindent_symbol = {file}\n"), ""), "editor.indent_symbol");
            assert_eq!(p.typing_from(), value, "還原成值本身，不是它的寫法");
            assert!(p.commit_value(value));
            assert!(!p.dirty(), "一個字没改，不算改過：{:?}", p.global.edits);
        }
    }

    /// **轉一圈回到原來那個值，就不算改過**；檔裏寫着的值撥回來也不算。
    #[test]
    fn cycling_all_the_way_back_is_not_a_change() {
        let mut p = on(panel("", ""), "editor.margin");
        for _ in 0..4 {
            p.press();
        }
        assert!(!p.dirty(), "{:?}", p.global.edits);
        let mut p = on(panel("[editor]\nmargin = \"loose\"\n", ""), "editor.margin");
        p.press();
        assert!(p.dirty());
        for _ in 0..3 {
            p.press();
        }
        assert!(!p.dirty(), "{:?}", p.global.edits);
    }

    /// **「把全局設成和本項目一樣」是一次真的改動**，而被本項目蓋着說得出來。
    #[test]
    fn matching_what_the_project_says_still_changes_the_global_file() {
        let mut p = on(panel("", "[editor]\nlayout = \"vertical\"\n"), "editor.layout");
        assert!(p.shadowed(find("editor.layout")));
        p.press();
        assert_eq!(p.global.says("editor.layout").as_deref(), Some("\"vertical\""));
        p.into = Which::Local;
        assert!(!p.shadowed(find("editor.layout")));
    }

    /// **Every change is written at once**, a byte-for-byte edit of its own
    /// line: comments, order and keys the panel does not know stay, and no
    /// temporary file is left (2026-11-08).
    #[test]
    fn a_change_is_written_at_once_and_only_its_line() {
        let dir = scratch("write");
        let path = dir.join("config.toml");
        let before = "# my config\n[editor]\nscrolloff = 8  # I like 8\nindent = 4\n\n[keys.normal]\n\"C-s\" = \":w\"\n";
        std::fs::write(&path, before).unwrap();
        // A key the config does not take makes the file unread, so this one
        // is known; the test of an unknown key is below.
        let mut p = on(Panel::open(Some(path.clone()), None), "editor.scrolloff");
        assert!(p.commit_value("9"));
        let saved = p.flush().expect("written").expect("something");
        assert_eq!(saved.wrote, vec![path.clone()]);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, before.replace("scrolloff = 8", "scrolloff = 9"));
        assert!(!p.dirty());
        assert_eq!(p.flush(), Ok(None), "nothing left to write");
        let litter: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != "config.toml")
            .collect();
        assert!(litter.is_empty(), "{litter:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A file changed outside meanwhile keeps that change**: the line is
    /// set on the disk's text, not the one read at opening.
    #[test]
    fn writing_keeps_what_somebody_else_changed_meanwhile() {
        let dir = scratch("moved");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[editor]\nindent = 4\n").unwrap();
        let mut p = on(Panel::open(Some(path.clone()), None), "editor.soft_wrap");
        std::fs::write(&path, "[editor]\nindent = 4\nmeasure = 30\n").unwrap();
        p.press();
        let saved = p.flush().unwrap().unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("soft_wrap = false") && after.contains("measure = 30"), "{after}");
        assert_eq!(saved.changed_underneath, vec![path]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`u` writes the file back, `C-r` writes the change again**, in the
    /// spelling the file had; a new change forgets what `C-r` held.
    #[test]
    fn undo_writes_the_file_back_and_redo_again() {
        let dir = scratch("undo");
        let path = dir.join("config.toml");
        let before = "[editor]\nlayout = 'vertical'   # mine\nindent = 4\n";
        std::fs::write(&path, before).unwrap();
        let mut p = on(Panel::open(Some(path.clone()), None), "editor.layout");
        assert!(!p.can_undo());
        p.press();
        p.flush().unwrap();
        let mut p = on(p, "editor.indent");
        assert_eq!(p.reset(), Some(true));
        p.flush().unwrap();
        let changed = std::fs::read_to_string(&path).unwrap();
        assert!(changed.contains("layout = \"horizontal\"") && !changed.contains("indent"), "{changed}");

        assert!(p.undo().unwrap().is_some());
        assert!(std::fs::read_to_string(&path).unwrap().contains("indent = 4"));
        assert!(p.undo().unwrap().is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "back as it was, quotes and note");
        assert_eq!(p.undo(), Ok(None), "nothing more");

        assert!(p.redo().unwrap().is_some());
        assert!(std::fs::read_to_string(&path).unwrap().contains("layout = \"horizontal\""));
        assert!(p.can_redo());
        let mut p = on(p, "editor.soft_wrap");
        p.press();
        p.flush().unwrap();
        assert!(!p.can_redo(), "a new change forgets the rest");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 打了個不是數的東西 —— **不收，說為什麽**（2026-11-08；從前是一聲不吭地
    /// 當没改），不是撥成 0。
    #[test]
    fn typing_something_that_is_not_a_number_is_refused() {
        let mut p = on(panel("[editor]\nindent = 4\n", ""), "editor.indent");
        assert!(!p.commit_value("兩格"));
        assert!(!p.dirty());
        assert_eq!(p.refusal, Some(Refusal::NotANumber { low: 0, high: 8, zero: false }));
        assert!(!p.commit_value(""), "empty is not a number either");
        assert!(p.commit_value("2"));
        assert_eq!(p.refusal, None, "taken, the word goes");
        assert_eq!(p.shown(find("editor.indent")).value, "2");
    }

    /// **A number out of range is refused, not pulled in** (2026-11-08,
    /// reported: `99` became `64` without a word); a `0` that means
    /// something of its own is taken.
    #[test]
    fn a_number_out_of_range_is_refused_not_pulled_in() {
        let mut p = on(panel("", ""), "editor.pages_vertical");
        assert!(!p.commit_value("99"));
        assert_eq!(p.refusal, Some(Refusal::OutOfRange { low: 1, high: 4, zero: false }));
        assert!(!p.commit_value("-1"));
        let p = on(p, "editor.zong_length");
        for (typed, ok) in [("0", true), ("4", false), ("8", true), ("64", true), ("65", false)] {
            assert_eq!(p.check(find("editor.zong_length"), typed).is_ok(), ok, "{typed}");
        }
        assert_eq!(p.check(find("editor.zong_length"), "4"), Err(Refusal::OutOfRange { low: 8, high: 64, zero: true }));
    }

    /// **A name is one the config takes** (2026-11-08, reported: `nosuch`
    /// as a theme was saved and said to be in force).
    #[test]
    fn a_theme_a_scheme_and_an_annotation_mode_must_be_ones_there_are() {
        let mut p = panel("[ime.annotation_modes.mine]\nunicode = true\n", "");
        let theme = find("theme.name");
        assert_eq!(p.check(theme, "bw"), Ok(Change::Text("bw".into())));
        assert_eq!(p.check(theme, "墨香"), Ok(Change::Text("墨香".into())), "the loader takes it");
        assert!(matches!(p.check(theme, "nosuch"), Err(Refusal::NotOneOf { ref names, .. }) if names.contains(&"ink".to_string())));
        assert!(matches!(p.check(theme, "  "), Err(Refusal::Empty { .. })));
        let scheme = find("ime.scheme");
        assert!(p.check(scheme, "custom").is_ok(), "no list, any name");
        p.schemes = Some(Names { takes: |w| w == "lingming", names: vec!["lingming".into()] });
        assert!(p.check(scheme, "lingming").is_ok());
        assert!(matches!(p.check(scheme, "custom"), Err(Refusal::NotOneOf { .. })));
        assert_eq!(p.choices(scheme), ["lingming"]);
        let annotation = find("ime.annotation");
        assert!(p.check(annotation, "mine").is_ok(), "a mode the file adds");
        assert!(matches!(p.check(annotation, "nosuch"), Err(Refusal::NotOneOf { .. })));
        assert_eq!(p.choices(annotation), ["off", "multi", "double", "mine"]);
    }

    /// **A piece of text is what the loader takes** (2026-11-08, reported:
    /// a page of `ZZZ` and an empty one were saved).
    #[test]
    fn a_piece_of_text_is_checked_by_the_loader_itself() {
        let p = panel("", "");
        let page = find("export.page");
        assert!(p.check(page, "a5").is_ok() && p.check(page, "137x195").is_ok());
        for bad in ["ZZZ", ""] {
            match p.check(page, bad) {
                Err(Refusal::Loader(said)) => assert!(said.contains("a4") && said.contains("[export] page"), "{said}"),
                other => panic!("{bad:?}: {other:?}"),
            }
        }
        assert!(matches!(p.check(find("export.font_family"), " "), Err(Refusal::Empty { .. })));
        assert!(matches!(p.check(find("panel.markers"), "ABC"), Err(Refusal::Loader(_))));
        assert!(p.check(find("panel.markers"), "１２３４５６７８９").is_ok());
        assert!(matches!(p.check(find("editor.which_wrap"), "bs"), Err(Refusal::Loader(_))));
        assert!(p.check(find("editor.which_wrap"), "").is_ok(), "no key wraps: a setting of its own");
        assert!(matches!(p.check(find("editor.language_key"), "C-"), Err(Refusal::Loader(_))));
    }

    /// **A file the loader cannot read**: what it says is not shown as in
    /// force, and a change to either file writes nothing and says why — a
    /// TOML error, or a name the config does not take (2026-11-08).
    #[test]
    fn a_file_the_loader_cannot_read_is_not_written() {
        let dir = scratch("unread");
        let global = dir.join("config.toml");
        let local = dir.join("local.toml");
        for (text, line) in [("[editor]\nscrolloff = 8\nindent 2\n", 2), ("[editor]\nscrolloff = 8\ntypo_key = true\n", 2)] {
            std::fs::write(&global, text).unwrap();
            std::fs::write(&local, "").unwrap();
            let mut p = Panel::open(Some(global.clone()), Some(local.clone()));
            let unread = p.unread();
            assert_eq!(unread.len(), 1, "{text}");
            assert_eq!(unread[0].1 .0, line, "{:?}", unread[0]);
            assert_eq!(p.shown(find("editor.scrolloff")).from, Layer::Factory, "not in force");
            let mut p2 = on(p.clone(), "editor.soft_wrap");
            p2.press();
            match p2.flush() {
                Err(SaveError::Unread { path, line: at, said }) => {
                    assert_eq!((path, at), (global.clone(), line));
                    assert!(!said.is_empty());
                }
                other => panic!("{other:?}"),
            }
            assert!(!p2.dirty(), "the change is let go");
            assert_eq!(std::fs::read_to_string(&global).unwrap(), text, "not touched");
            // The other file is written as ever.
            p = on(p, "editor.soft_wrap");
            p.into = Which::Local;
            p.press();
            assert!(p.flush().is_ok());
            assert!(std::fs::read_to_string(&local).unwrap().contains("soft_wrap = false"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The project file is made when its first line is written**, and its
    /// last line taken away takes the table with it.
    #[test]
    fn the_project_file_is_made_and_emptied() {
        let dir = scratch("local");
        let local = dir.join("p").join(".yumete").join("config.toml");
        let mut p = on(Panel::open(None, Some(local.clone())), "editor.soft_wrap");
        p.flip_into();
        assert!(!p.local.there);
        p.press();
        assert_eq!(p.flush().unwrap().map(|s| s.wrote), Some(vec![local.clone()]));
        assert!(p.local.there && p.set_here("editor.soft_wrap"));
        p.reset();
        p.flush().unwrap();
        assert_eq!(std::fs::read_to_string(&local).unwrap().trim(), "", "no `[editor]` left alone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The state a stale message is measured against** moves with the
    /// cursor and with the files, and not otherwise.
    #[test]
    fn the_spot_moves_with_the_cursor_and_the_values() {
        let dir = scratch("spot");
        let mut p = Panel::open(Some(dir.join("config.toml")), None);
        let here = p.spot();
        assert_eq!(p.spot(), here);
        p.step(true);
        assert_ne!(p.spot(), here);
        let here = p.spot();
        p.press();
        assert_eq!(p.spot(), here, "staged, not yet written");
        p.flush().unwrap();
        assert_ne!(p.spot(), here, "a value moved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **八組都有東西**：一組空着的話，單子裏只剩一個標題。
    #[test]
    fn no_group_is_empty() {
        for named in GROUPS {
            assert!(SETTINGS.iter().any(|s| s.group == named.group), "{}", named.label);
        }
    }

    /// **`e` opens the file at the row's line**, or its table's header.
    #[test]
    fn the_line_of_a_row_is_found_in_the_file() {
        let p = on(panel("# top\n[editor]\nindent = 4\nscrolloff = 8\n\n[theme]\nname = \"bw\"\n", ""), "editor.scrolloff");
        assert_eq!(p.line_here(), 3);
        let p = on(p, "theme.mode");
        assert_eq!(p.line_here(), 5, "the table's header");
        let p = on(p, "export.page");
        assert_eq!(p.line_here(), 0, "not in the file");
    }

    /// **The flavour's page** (2026-11-02): a tick written into
    /// `[keys.flavor_items]`, flipped back to what the level gives and the
    /// line goes, and a binding of one of its keys is said.
    #[test]
    fn the_flavor_page_writes_items_by_name_and_takes_them_back() {
        use yumete_cjk::FlavorItem;
        let dir = scratch("flavor");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[keys]\npreset = \"helix\"\n\n[keys.normal]\n\"A-j\" = \"page_down\"\n").unwrap();
        let mut p = on(Panel::open(Some(path.clone()), None), "keys.flavor_items");
        p.press();
        assert_eq!(p.page, Some(0), "Space on the row goes in");
        assert_eq!(p.flavor_here(), Some(FlavorRow::Level));
        let at = |p: &Panel, item: FlavorItem| p.flavor_rows().iter().position(|(_, r)| *r == FlavorRow::Item(item)).unwrap();
        p.page = Some(at(&p, FlavorItem::Tables));
        assert_eq!(p.item_shown(FlavorItem::Tables), (true, ItemFrom::Level));
        p.page_press();
        assert_eq!(p.item_shown(FlavorItem::Tables), (false, ItemFrom::Layer(Layer::Global)));
        assert_eq!(p.item_taken(FlavorItem::MoveLines), vec!["A-j"]);
        p.flush().expect("written");
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("[keys.flavor_items]") && after.contains("tables = false"), "{after}");
        assert!(p.set_here("keys.flavor_items"), "the row is marked as set here");
        p.page_press();
        p.flush().expect("written");
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(!after.contains("flavor_items"), "{after}");
        assert!(after.contains("\"A-j\" = \"page_down\""), "the binding stays: {after}");
        p.page = Some(at(&p, FlavorItem::EscCollapses));
        p.page_press();
        assert_eq!(p.item_shown(FlavorItem::EscCollapses), (true, ItemFrom::Layer(Layer::Global)));
        p.page = Some(0);
        p.page_press();
        assert_eq!(p.flavor_level(), yumete_cjk::FlavorLevel::Full);
        p.page = Some(at(&p, FlavorItem::EscCollapses));
        p.reset();
        assert_eq!(p.item_shown(FlavorItem::EscCollapses), (true, ItemFrom::Level));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The annotation modes' page** (2026-11-02): a field ticked is written
    /// under `[ime.annotation_modes.<mode>]`, ticked back the line goes and
    /// the emptied table with it, and a mode the file adds has its fields.
    #[test]
    fn the_annotation_page_writes_fields_by_mode_and_takes_them_back() {
        let dir = scratch("annotation");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[ime.annotation_modes.mine]\nunicode = true\n").unwrap();
        let mut p = on(Panel::open(Some(path.clone()), None), "ime.annotation_modes");
        p.change();
        assert_eq!(p.page, Some(0), "Enter on the row goes in");
        assert_eq!(p.annotation_modes(), ["multi", "double", "mine"]);
        assert_eq!(p.mode_fields("double"), ["chaifen", "code"]);
        let at = |p: &Panel, mode: &str, field: &str| {
            p.annotation_rows().iter().position(|(_, r)| r.mode == mode && r.field == field).unwrap()
        };
        p.page = Some(at(&p, "double", "reading"));
        p.page_press();
        p.flush().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[ime.annotation_modes.double]\nreading = true"), "{text}");
        assert_eq!(crate::Config::from_toml(&text).ime.annotation_modes[1].fields, ["chaifen", "code", "reading"]);
        p.page_press();
        p.flush().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("double") && text.contains("[ime.annotation_modes.mine]"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Found in review (2026-11-02): an item written in an inline table, or
    /// with a comment after it, is what the file says, and writing works.
    #[test]
    fn an_inline_table_and_a_trailing_comment_are_read_as_written() {
        use yumete_cjk::FlavorItem;
        let p = panel("[keys]\nflavor_items = { tables = false }\n", "");
        assert_eq!(p.item_shown(FlavorItem::Tables), (false, ItemFrom::Layer(Layer::Global)));
        let p = panel("[keys.flavor_items]\ntables = false # off for now\n", "");
        assert_eq!(p.item_shown(FlavorItem::Tables), (false, ItemFrom::Layer(Layer::Global)));
        let text = rewrite(
            "[keys]\nflavor_items = { tables = false }\n",
            &[("keys.flavor_items.move_lines".to_string(), Change::Tick(false))],
        )
        .expect("an inline table takes a key");
        let config = crate::Config::from_toml(&text);
        assert_eq!(config.keys.flavor.items.get(&FlavorItem::Tables), Some(&false), "{text}");
        assert_eq!(config.keys.flavor.items.get(&FlavorItem::MoveLines), Some(&false), "{text}");
    }

    /// **A font list with quotes in it is written and read back as typed.**
    #[test]
    fn a_font_list_is_written_and_read_back_whole() {
        let dir = scratch("font");
        let path = dir.join("config.toml");
        let mut p = on(Panel::open(Some(path.clone()), None), "export.font_family");
        let typed = "\"Iowan Old Style\", 'Songti SC', \"Noto \\ Serif\", serif";
        assert!(p.commit_value(typed));
        p.flush().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(crate::Config::from_toml(&text).export.font_family, typed, "{text}");
        let again = Panel::open(Some(path.clone()), None);
        assert_eq!(unwritten(&again.shown(find("export.font_family")).value), typed);
        assert_eq!(p.value_here().as_deref(), Some(typed), "what `y` copies");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The preview's fonts may be emptied** (2026-11-09): unlike the
    /// export's, an empty list is a value, the export's fonts, and it is
    /// written and read back as one; a list is written whole.
    #[test]
    fn the_preview_fonts_take_an_empty_list() {
        let p = panel("", "");
        let fonts = find("editor.preview_font_family");
        assert!(matches!(p.check(fonts, ""), Ok(Change::Text(t)) if t.is_empty()), "empty follows the export");
        assert!(p.check(fonts, "'Kaiti SC', serif").is_ok());
        let dir = scratch("preview-font");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[editor]\npreview_font_family = \"Kaiti, serif\"\n").unwrap();
        let mut p = on(Panel::open(Some(path.clone()), None), "editor.preview_font_family");
        assert!(p.commit_value(""));
        p.flush().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("preview_font_family = \"\""), "{text}");
        assert_eq!(crate::Config::from_toml(&text).editor.preview_font_family, "", "{text}");
        let typed = "\"LXGW WenKai Screen\", 'Songti SC', serif";
        assert!(p.commit_value(typed));
        p.flush().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(crate::Config::from_toml(&text).editor.preview_font_family, typed, "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`u` writes back the file exactly as it was** (2026-11-09, reported:
    /// `u` after `r` lost `# a` after `scrolloff = 3`, and the last key of a
    /// table taken away and taken back came back without the table's
    /// comments); `C-r` writes the change exactly again.
    #[test]
    fn undo_writes_back_the_exact_text() {
        let dir = scratch("exact");
        let path = dir.join("config.toml");
        let before = "# top\n[editor]\nscrolloff = 3 # a\nindent = 2 # b\n\n# about the theme\n[theme]\nname = \"ink\"   # dark\n";
        std::fs::write(&path, before).unwrap();
        let mut p = on(Panel::open(Some(path.clone()), None), "editor.scrolloff");
        assert_eq!(p.reset(), Some(true));
        p.flush().unwrap();
        let after_r = std::fs::read_to_string(&path).unwrap();
        assert!(!after_r.contains("scrolloff"), "{after_r}");
        p.undo().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "the comment after the line is back");
        p.redo().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after_r);
        p.undo().unwrap();
        // The last key of a table.
        let mut p = on(p, "theme.name");
        assert_eq!(p.reset(), Some(true));
        p.flush().unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("[theme]"));
        p.undo().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "the table with its comments");
        assert_eq!(p.shown(find("theme.name")).value, "\"ink\"", "the panel reads it back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A file changed outside after the change**: `u` sets the key back on
    /// what the disk holds now, so the change outside stays; and **"before"
    /// is the disk's, not the panel's** (2026-11-09, reported: changed
    /// outside from 3 to 9, set to 5 here, and `u` wrote 3).
    #[test]
    fn undo_keeps_a_change_made_outside() {
        let dir = scratch("outside-undo");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[editor]\nscrolloff = 3 # a\nindent = 2\n").unwrap();
        let mut p = on(Panel::open(Some(path.clone()), None), "editor.scrolloff");
        // Changed outside, and not yet seen by the panel.
        std::fs::write(&path, "[editor]\nscrolloff = 9 # a\nindent = 2\n").unwrap();
        assert!(p.commit_value("5"));
        let saved = p.flush().unwrap().unwrap();
        assert_eq!(saved.changed_underneath, vec![path.clone()]);
        p.undo().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[editor]\nscrolloff = 9 # a\nindent = 2\n");
        // Set again, then a line added outside: `u` keeps it.
        assert!(p.commit_value("4"));
        p.flush().unwrap();
        std::fs::write(&path, "[editor]\nscrolloff = 4 # a\nindent = 2\nruler = 72 # mine\n").unwrap();
        let saved = p.undo().unwrap().unwrap();
        assert_eq!(saved.changed_underneath, vec![path.clone()]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[editor]\nscrolloff = 9 # a\nindent = 2\nruler = 72 # mine\n");
        p.redo().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[editor]\nscrolloff = 4 # a\nindent = 2\nruler = 72 # mine\n", "exact again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`C-r` after a `u` that had to take in a change made outside** puts
    /// the step's change back on what `u` wrote (2026-11-09, reported: set
    /// to 5, changed outside to 7, `u` wrote 3 and `C-r` wrote the 7 back).
    /// A file deleted outside comes back from `C-r` with the change in it.
    #[test]
    fn redo_after_a_merged_undo_puts_the_change_back() {
        let dir = scratch("outside-redo");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[editor]\nscrolloff = 3 # a\nindent = 2\n").unwrap();
        let mut p = on(Panel::open(Some(path.clone()), None), "editor.scrolloff");
        assert!(p.commit_value("5"));
        p.flush().unwrap();
        std::fs::write(&path, "[editor]\nscrolloff = 7 # a\nindent = 2\n").unwrap();
        p.undo().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[editor]\nscrolloff = 3 # a\nindent = 2\n");
        let saved = p.redo().unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[editor]\nscrolloff = 5 # a\nindent = 2\n", "the step's 5, not the 7");
        assert!(saved.removed.is_empty() && saved.wrote == vec![path.clone()]);
        p.undo().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[editor]\nscrolloff = 3 # a\nindent = 2\n", "and back, exactly");

        // Deleted outside after the change.
        let mut p = on(p, "editor.indent");
        assert!(p.commit_value("4"));
        p.flush().unwrap();
        std::fs::remove_file(&path).unwrap();
        p.undo().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "[editor]\nindent = 2");
        let saved = p.redo().unwrap().unwrap();
        assert!(saved.removed.is_empty(), "written, not removed");
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "[editor]\nindent = 4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A file changed outside is read again** (2026-11-09) — another value,
    /// or the file gone — and a file deleted outside is written with the
    /// new line only, not back from the text read before.
    #[test]
    fn a_file_changed_or_deleted_outside_is_read_again() {
        let dir = scratch("refresh");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[editor]\nscrolloff = 3\nindent = 2\n").unwrap();
        let mut p = on(Panel::open(Some(path.clone()), None), "editor.scrolloff");
        assert!(p.refresh().is_empty(), "nothing changed");
        std::fs::write(&path, "[editor]\nscrolloff = 9\nindent = 2\nruler = 80\n").unwrap();
        assert_eq!(p.refresh(), vec![path.clone()]);
        assert_eq!(p.shown(find("editor.scrolloff")).value, "9");
        assert!(p.refresh().is_empty(), "said once");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(p.refresh(), vec![path.clone()]);
        assert!(!p.global.there && p.shown(find("editor.ruler")).from == Layer::Factory);
        assert!(p.commit_value("7"));
        p.flush().unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after.trim(), "[editor]\nscrolloff = 7", "only the new line");
        // Deleted and not yet seen: the same.
        std::fs::remove_file(&path).unwrap();
        let mut p = on(p, "editor.indent");
        assert!(p.commit_value("4"));
        let saved = p.flush().unwrap().unwrap();
        assert_eq!(saved.changed_underneath, vec![path.clone()]);
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "[editor]\nindent = 4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A file the panel made is taken away again by `u`**, exactly as it
    /// was: not there.
    #[test]
    fn undo_takes_away_a_file_the_panel_made() {
        let dir = scratch("made");
        let local = dir.join(".yumete").join("config.toml");
        let mut p = on(Panel::open(None, Some(local.clone())), "editor.soft_wrap");
        p.flip_into();
        p.press();
        p.flush().unwrap();
        assert!(local.exists());
        let saved = p.undo().unwrap().unwrap();
        assert!(!local.exists() && !p.local.there);
        assert_eq!((saved.wrote, saved.removed), (Vec::new(), vec![local.clone()]), "removed, not written");
        p.redo().unwrap();
        assert!(std::fs::read_to_string(&local).unwrap().contains("soft_wrap = false"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `:settings` finds a name with its quotes off, in any case.
    #[test]
    fn go_to_lets_quotes_and_case_go() {
        let names = |s: &Setting| match s.key == "indent" && s.table == "editor" {
            true => vec!["首行縮進".to_string(), "首行缩进".to_string(), "first-line indent".to_string()],
            false => Vec::new(),
        };
        for typed in ["\"first-line indent\"", "'首行缩进'", "First-Line Indent ", "首行縮進"] {
            let mut p = panel("", "");
            assert!(p.go_to(typed, names), "{typed}");
            assert_eq!(p.here().map(|s| s.key), Some("indent"));
        }
    }
}
