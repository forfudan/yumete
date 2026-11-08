//! **那扇設置面板的狀態** —— Feature #421（2026-09-24）。
//!
//! 原話：
//!
//! > 這樣的話用戶（特別是寫小説的），不需要面對 toml 和一堆 key 發呆不知道他們
//! > 都是幹啥的，也不需要查詢到底有哪些 key 可以用。
//!
//! 整頁一扇，左邊八組、右邊那一組的項；`hl` 換欄、`jk` 上下、空格切換、`i` 改、
//! `d` 撤掉、`Tab` 換存到哪一份、`:w` 存盤、`q` 走。
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
//! # `:w`，不是即時生效
//!
//! 2026-09-23 定，原話：「需要 :w，即時生效不好，改錯了都没辦法反悔。」所以改動
//! 攢在 [`Sheet::edits`] 裏，存盤那一下纔落到檔上，落完由 `:reload-config` 那條
//! 同一支推進編輯器。

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

/// 鍵在左邊那張組單子上，還是右邊那些項上。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    #[default]
    Groups,
    Settings,
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
}

impl Sheet {
    /// 讀一份檔。讀不到就是空的那一份（要存的時候會建出來）。
    pub fn read(path: Option<PathBuf>) -> Sheet {
        let read = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok());
        let there = read.is_some();
        let text = read.unwrap_or_default();
        Sheet::of_text(path, there, text)
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

/// Where a paste goes in a value that is not being typed yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pasting {
    /// `p`: after the value.
    After,
    /// `P`: before it.
    Before,
    /// A paste from the terminal: in place of it.
    Replace,
}

/// Pasted text made one line: see [`Panel::paste`].
fn one_line(text: &str) -> String {
    // A terminal hands a pasted break over as `\r` as often as `\n`.
    let lines: Vec<&str> = text.split(['\n', '\r']).collect();
    let last = lines.len() - 1;
    let joined = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line = if i > 0 { line.trim_start() } else { line };
            if i < last { line.trim_end() } else { line }
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    joined
        .chars()
        .map(|c| match c {
            '\t' => ' ',
            c => c,
        })
        .filter(|c| !c.is_control())
        .collect()
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
    pub pane: Pane,
    /// 左邊那張單子上的第幾組（[`GROUPS`] 的下標）。
    pub group: usize,
    /// 右邊那一組裏的第幾項。
    pub row: usize,
    /// `i` 進來改的那一段字；`None` ＝ 不在改。
    pub typing: Option<String>,
    /// **How many characters of [`Panel::typing`] stand after the caret**
    /// (2026-11-08). Counted from the end, so `0` is the end of the text
    /// and a value set whole (`typing = Some(..)`) has its caret after it,
    /// where `i` puts it. A paste lands at the caret, and `P` puts it
    /// before the value.
    pub tail: usize,
    /// **A page is open**, on this row of it (2026-11-02). Which page is the
    /// row it was opened from ([`Panel::subpage`]). `None` is the ordinary
    /// list.
    pub page: Option<usize>,
    /// **Why the value typed was not taken** (2026-11-08): `Enter` on a value
    /// the config would refuse leaves it in the field, and this is drawn
    /// under the row.
    pub refusal: Option<Refusal>,
    /// The schemes there are, from the input method, which this crate cannot
    /// see: `None` takes any name.
    pub schemes: Option<Names>,
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

/// **What `:w` did** (2026-11-08): every file it wrote, and which of them
/// had been changed outside while the panel was open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    pub wrote: Vec<PathBuf>,
    pub changed_underneath: Vec<PathBuf>,
}

/// **Why `:w` wrote nothing.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The file is one the loader cannot read (a TOML error, or a name it
    /// does not take): saving over it would claim settings that are not in
    /// force. The line is 0 起算; the message is the parser's.
    Unread { path: PathBuf, line: usize, said: String },
    /// The disk said no.
    Io(String),
}

/// **Where the panel stands**, to tell whether a key moved anything
/// (2026-11-08): a message about a row goes stale when this changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    pane: Pane,
    group: usize,
    row: usize,
    page: Option<usize>,
    into: Which,
    typing: Option<String>,
    edits: (BTreeMap<String, Change>, BTreeMap<String, Change>),
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
/// Warning: **不回 `None`。** 面板明寫着「存盤時建」，而 `save()` 自己會
/// `create_dir_all`；少了這一支，那句承諾就是空頭的。要建哪一個看得見：頂上那
/// 一行寫的就是它。
pub fn local_sheet_path(cwd: &std::path::Path) -> PathBuf {
    crate::local_config_path(cwd).unwrap_or_else(|| cwd.join(".yumete").join("config.toml"))
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
            pane: Pane::Groups,
            group: 0,
            row: 0,
            typing: None,
            tail: 0,
            page: None,
            refusal: None,
            schemes: None,
        }
    }

    /// Where the panel stands now: see [`Spot`].
    pub fn spot(&self) -> Spot {
        Spot {
            pane: self.pane,
            group: self.group,
            row: self.row,
            page: self.page,
            into: self.into,
            typing: self.typing.clone(),
            edits: (self.global.edits.clone(), self.local.edits.clone()),
        }
    }

    /// Whether `path` has an unsaved edit in either file: the row gets a mark.
    pub fn edited(&self, path: &str) -> bool {
        self.global.edited(path) || self.local.edited(path)
    }

    /// The files that cannot be read, global first: the panel says so at the
    /// top.
    pub fn unread(&self) -> Vec<(Which, &(usize, String))> {
        [(Which::Global, &self.global), (Which::Local, &self.local)]
            .into_iter()
            .filter_map(|(which, sheet)| sheet.unread.as_ref().map(|u| (which, u)))
            .collect()
    }

    /// 這一組底下的那些項，畫出來的次序就是 [`SETTINGS`] 的次序。
    pub fn rows(&self) -> Vec<&'static Setting> {
        let Some(named) = GROUPS.get(self.group) else {
            return Vec::new();
        };
        SETTINGS.iter().filter(|s| s.group == named.group).collect()
    }

    /// 光標此刻站的那一項。
    pub fn here(&self) -> Option<&'static Setting> {
        self.rows().get(self.row).copied()
    }

    /// **The row a change goes to: none on the group list** (2026-11-08).
    /// There no row is highlighted, and [`Panel::here`] is only the group's
    /// first: reported, `i` on 版面 flipped 排版方向 unseen and `c` opened a
    /// field nobody could see, its Enter setting 光標邊距.
    fn chosen(&self) -> Option<&'static Setting> {
        self.here().filter(|_| self.pane == Pane::Settings)
    }

    /// 一項此刻長什麽樣。
    /// **三層各自說了什麼** —— 出廠、全局、本項目，照面板上那一塊的次序（2026-10-13）。
    ///
    /// `None` ＝ 那一層沒有說話。出廠那一層永遠有話說，所以它那一格永遠是 `Some`。
    ///
    /// Warning: **和 [`Panel::shown`] 問的是同一份東西，答的是不同的問題**：那一支答
    /// 「贏的是誰」，這一支答「三個人各說了什麼」——面板底下那一塊要的是後者，而從前
    /// 只有前者，於是「全局設了但被本項目蓋住」這件事看不見。
    pub fn layers(&self, setting: &Setting) -> [(Layer, Option<String>); 3] {
        let path = setting.path();
        let said = |raw: Option<String>| raw.map(|v| Self::as_the_editor_takes_it(setting, v));
        [
            (Layer::Factory, Some(setting.factory.to_string())),
            (Layer::Global, said(self.global.says(&path))),
            (Layer::Local, said(self.local.says(&path))),
        ]
    }

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
    /// 面板**撥**不到域外的數（`Kind::Count` 的 `low`／`high` 就是為這個寫的，
    /// `tests/settings_ui.rs::every_value_the_panel_can_set_is_its_own` 盯着），可
    /// 檔是人手改的：`zong_length = 2` 寫進去，面板畫 2 而 `into_config` 按 4 排版
    /// ——**面板顯示的值不是編輯器用的值**，而這扇面板存在的理由就是終結這件事。
    ///
    /// 鉗的依據是這一項**自己declare 的域**，不是另抄一份數字，所以它不會和
    /// `into_config` 分岔。`zero` 那一格的意思是「0 另有意思」，所以 0 照過。
    ///
    /// ⚠ 非數字那一族只有一個：`[panel] markers` 空的時候 `into_config` 留出廠那一
    /// 串（它那一句註釋寫着「keep the default rather than silently taking the
    /// numbers away」），所以空的就畫出廠那一串。
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
        match self.into {
            Which::Global => &mut self.global,
            Which::Local => &mut self.local,
        }
    }

    /// 改動要落到的那一份（只讀）。
    pub fn sheet(&self) -> &Sheet {
        self.sheet_of(self.into)
    }

    /// 指名那一份。
    fn sheet_of(&self, which: Which) -> &Sheet {
        match which {
            Which::Global => &self.global,
            Which::Local => &self.local,
        }
    }

    /// **切着的那一份此刻替這一項出的是什麽** —— 檔裏寫着的，没寫就是它**下面**
    /// 那幾層給的。
    ///
    /// Warning: **看的是下面，不是「畫出來的那個值」。** 編輯全局而本項目蓋着它的時候，
    /// 畫出來的是本項目那個值——拿它作比，「把全局設成和本項目一樣」就被判成「没
    /// 改」，而那明明改變了**別的項目**裏的行為。全局底下只有出廠；本項目底下是
    /// 全局，再底下纔是出廠。
    fn without_mine(&self, setting: &Setting) -> String {
        let path = setting.path();
        let mine = self.sheet().declared.get(&path).cloned();
        let under = match self.into {
            // 全局底下只有出廠。
            Which::Global => None,
            // 本項目底下是全局（連它没存的改動一起算：那是同一次存盤落下去的）。
            Which::Local => self.global.says(&path),
        };
        mine.or(under).unwrap_or_else(|| setting.factory.to_string())
    }

    /// **切着的那一份此刻替這一項出的是什麽，連没存的改動一起算。**
    ///
    /// Warning: **按空格與 `i` 的起點是這個，不是畫出來的那個值。** 畫出來的是三層裏
    /// **贏**的那一個；本項目蓋着全局的時候你編輯全局，起點要是全局自己那個值。
    /// 拿畫出來的值當起點，「空格」會從別人的值往下數——按一下 1 變 3。
    /// 2026-09-24 一條測試逮到的。
    pub fn mine_now(&self, setting: &Setting) -> String {
        self.sheet()
            .says(&setting.path())
            .unwrap_or_else(|| self.without_mine(setting))
    }

    /// **切着的那一份被上面那一層蓋着嗎。**
    ///
    /// Warning: 蓋着的時候改這一項，**螢幕上那一行不會動**（贏的還是上面那個），看起來
    /// 像按了没反應。所以底下那一行要說一句。
    pub fn shadowed(&self, setting: &Setting) -> bool {
        self.into == Which::Global && self.local.says(&setting.path()).is_some()
    }

    /// **攢一筆改動 —— 除非改完和不改一樣。**
    ///
    /// Warning: 不問這一句，按空格轉一圈回到原來那個值**還算「改了」**：面板頂上寫着
    /// 「改了還没存」，而 `:w` 會往檔裏寫一行跟原來一模一樣的東西。同一族的還有
    /// 「把一項設成和出廠一樣」——那一行寫進去純屬噪音。2026-09-24 拍圖看出來的。
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

    /// `j` / `k`。
    pub fn step(&mut self, down: bool) {
        let deep = self.rows().len();
        let (at, many) = match self.pane {
            Pane::Groups => (&mut self.group, GROUPS.len()),
            Pane::Settings => (&mut self.row, deep),
        };
        if many == 0 {
            return;
        }
        *at = match down {
            true => (*at + 1).min(many - 1),
            false => at.saturating_sub(1),
        };
        // 換了組，右邊那一欄要從頭數。
        if self.pane == Pane::Groups {
            self.row = 0;
        }
    }

    /// `h` / `l`。
    ///
    /// Warning: **空的那一組進不去**：七組還没填，`l` 進去會停在一張空表上，而那一刻
    /// `j`/`k`/空格全部没有反應——看起來像面板卡住了。
    pub fn across(&mut self, right: bool) {
        self.cancel_typing();
        self.pane = match (right, self.rows().is_empty()) {
            (true, false) => Pane::Settings,
            (true, true) => Pane::Groups,
            (false, _) => Pane::Groups,
        };
    }

    /// `Tab` —— 換存到哪一份。
    pub fn flip_into(&mut self) {
        self.cancel_typing();
        self.into = self.into.other();
    }

    // ---- 改 -------------------------------------------------------------

    /// 空格 —— 打勾翻面、幾選一走下一個、數字加一（到頂回到底）。
    ///
    /// Warning: **一段自己打的字没有「下一個」**，空格對它就是 `i`。
    pub fn press(&mut self) {
        let Some(setting) = self.chosen() else { return };
        if setting.kind == Kind::Page {
            return self.open_page();
        }
        let now = self.mine_now(setting);
        let next = match setting.kind {
            Kind::Tick => Change::Tick(now != "true"),
            // Warning: **`0` 另算一檔，而 `low` 是「最小的非零值」。** `zong_length` 真
            // 正的域是「0，或者 4 到 64」（`into_config` 寫着
            // `if length == 0 {0} else {length.clamp(4,64)}`），`tatechuyoko` 是
            // 「0，或者 2 到 8」。從前 `low` 寫 0，於是面板停得到 1/2/3，寫進
            // 檔裏而**編輯器按 4 排版**——面板顯示的值不是編輯器用的值，這個倉最
            // 怕的那一族。2026-09-24 審出來的。
            Kind::Count { low, high, zero } => {
                let n: usize = now.parse().unwrap_or(low);
                // Warning: **`0` 只在 `low > 0` 的時候是域外那一檔。** `indent` 的
                // `zero` 只是替域裏的 0 取了個名字（「不縮進」，域是 0..8）；
                // 不分這一下，`indent` 在 0 上按空格會回到 `low` ＝ 0，那一格死住。
                let apart = zero.is_some() && low > 0;
                let next = match () {
                    _ if apart && n == 0 => low,
                    _ if n >= high && apart => 0,
                    _ if n >= high => low,
                    _ => (n + 1).max(low),
                };
                Change::Count(next as i64)
            }
            Kind::Pick(choices) | Kind::PickOr(choices) => {
                let word = unwritten(now.as_str());
                let word = word.as_str();
                let next = match choices.iter().position(|c| c.word == word) {
                    Some(at) => (at + 1) % choices.len(),
                    // **一個不認得的名字撥到第一個，不是第二個。** `PickOr` 裝
                    // 得下讀者自己的主題或碼表，而從那裏邁出一步該落在一個名字
                    // 上，不該跳過一個。檔裏拼錯了的 `Pick` 同理。
                    None => 0,
                };
                Change::Text(choices[next].word.to_string())
            }
            Kind::Text => {
                self.typing = Some(unwritten(now.as_str()));
                self.tail = 0;
                return;
            }
            // Gone in above.
            Kind::Page => return,
        };
        self.settle(setting, next);
    }

    /// `i` —— 進去打字。數字與一段字纔有得打。
    /// **`Enter`：改這一項，按這一項自己的改法**（2026-10-09 作者定）。
    ///
    /// 從前改一項要看它是哪一種：撥的用空格，打字的用 `i`，而兩個鍵各自在另一種
    /// 上**一聲不吭地什麽都不做**——`i` 停在一個勾上没有反應，看起來像面板壞了。
    /// 原話：「the way to use it is a little bit fuzzy」。兩個鍵都留着，`Enter` 是
    /// 那個不必先判斷種類的。
    pub fn change_here(&mut self) {
        let Some(setting) = self.chosen() else { return };
        match setting.kind {
            // 撥得動的：撥一格。
            Kind::Tick | Kind::Pick(_) | Kind::Page => self.press(),
            // **`PickOr` 走打字那一支**（2026-10-10）。這一種兩條路都有，所以
            // 兩個鍵各守各的本義：空格是「撥一格」，`i` 是「我自己打」——讀者
            // 自己裝的主題或碼表只有這一條路進得來。`Tick` 和 `Pick` 没得打，
            // 那時 `i` 纔替人撥（2026-10-09 定的那一條，原話「`i` 停在一個勾上
            // 一聲不吭」）。
            // 數和字：打進去。數也打字而不是一格一格撥——`zong_length` 的域是 4 到
            // 64，按空格按到 40 不是一個人會做的事。
            _ => self.begin_typing(),
        }
    }

    pub fn begin_typing(&mut self) {
        let Some(setting) = self.chosen() else { return };
        // **`PickOr` 不在這裏**：它收得下表外的名字，而 `i` 就是打那個名字用的。
        if matches!(setting.kind, Kind::Tick | Kind::Pick(_) | Kind::Page) {
            return;
        }
        // Warning: **`PickOr` 從空的開始**（2026-10-10 報的）。從前這裏一律把現在
        // 那個值放進去、光標擺在末尾，於是在主題那一行按 `i` 再打字得到的是
        // `ink我的主題`——要先按三下退格纔打得出自己的名字。`Count` 和 `Text` 照舊
        // 帶着現在那個值（把 16 改成 12 是改，不是重打），而 `PickOr` 的本意就是
        // 「打一個表裏没有的名字」，現在那個名字永遠不是你要的那一個。
        self.typing = Some(match setting.kind {
            Kind::PickOr(_) => String::new(),
            _ => unwritten(&self.mine_now(setting)),
        });
        self.tail = 0;
    }

    /// Whether this row takes typed text: a number, a piece of text, or a
    /// name of one's own. A tick, a choice and a page have nothing to type.
    pub fn takes_text(&self) -> bool {
        self.pane == Pane::Settings
            && self.here().is_some_and(|s| !matches!(s.kind, Kind::Tick | Kind::Pick(_) | Kind::Page))
    }

    /// The caret, as a count of characters from the start of the text.
    pub fn caret(&self) -> usize {
        let many = self.typing.as_ref().map_or(0, |t| t.chars().count());
        many - self.tail.min(many)
    }

    /// The byte where the caret stands in the text being typed.
    fn caret_byte(&self) -> usize {
        let at = self.caret();
        let typed = self.typing.as_deref().unwrap_or("");
        typed.char_indices().nth(at).map_or(typed.len(), |(i, _)| i)
    }

    /// A character typed at the caret.
    pub fn type_char(&mut self, c: char) {
        let at = self.caret_byte();
        self.tail = self.tail.min(self.typing.as_ref().map_or(0, |t| t.chars().count()));
        if let Some(typed) = self.typing.as_mut() {
            typed.insert(at, c);
        }
    }

    /// `Backspace`: the character before the caret.
    pub fn delete_back(&mut self) {
        let at = self.caret();
        if at == 0 {
            return;
        }
        self.tail = self.typing.as_ref().map_or(0, |t| t.chars().count()) - at;
        if let Some(typed) = self.typing.as_mut() {
            let (i, _) = typed.char_indices().nth(at - 1).expect("a character before the caret");
            typed.remove(i);
        }
    }

    /// `Delete`: the character after the caret.
    pub fn delete_forward(&mut self) {
        let at = self.caret_byte();
        let typed = self.typing.as_deref().unwrap_or("");
        if at >= typed.len() {
            return;
        }
        self.tail = typed[at..].chars().count() - 1;
        if let Some(typed) = self.typing.as_mut() {
            typed.remove(at);
        }
    }

    /// `←` / `→`, one character; `Home` / `End`, the whole way.
    pub fn move_caret(&mut self, to: usize) {
        let many = self.typing.as_ref().map_or(0, |t| t.chars().count());
        self.tail = many - to.min(many);
    }

    /// **Text arriving whole** (2026-11-08): a paste from the terminal, or a
    /// register by `p` / `P`. Typing, it lands at the caret. Not typing, on
    /// a row that takes text, the row opens as `i` opens it, and the text
    /// goes after the value (`p`), before it (`P`), or in place of it (a
    /// paste, which in Normal mode replaces what is selected, and the value
    /// is drawn selected). The row stays open, so `Enter` keeps what it
    /// shows and `Esc` leaves the value as it was.
    ///
    /// The value is one line. **A line break becomes one space**, with the
    /// spaces either side of it, and the breaks at either end go: a font
    /// list or a page copied over two lines reads `a, b`, and a whole line
    /// copied with its break (what an editor puts on the clipboard for a
    /// copy with nothing selected) is the line. Other control characters
    /// go, as they do from a paste into the `:` line.
    ///
    /// Gives how many characters went in, or `None` when there is nowhere
    /// for them to go: a tick, a choice, a page, the group list.
    pub fn paste(&mut self, text: &str, how: Pasting) -> Option<usize> {
        if self.typing.is_none() {
            if !self.takes_text() {
                return None;
            }
            self.begin_typing();
            match how {
                Pasting::Replace => {
                    self.typing = Some(String::new());
                    self.tail = 0;
                }
                Pasting::After => self.tail = 0,
                Pasting::Before => self.move_caret(0),
            }
        }
        let text = one_line(text);
        for c in text.chars() {
            self.type_char(c);
        }
        Some(text.chars().count())
    }

    /// 打完了 —— `Enter`。
    ///
    /// **A value the config would refuse is not taken** (2026-11-08): it
    /// stays in the field, and [`Panel::refusal`] says why, naming what is
    /// taken. A number out of range is refused, not pulled into it — the
    /// one typed is not the one that would be saved — and a word that is
    /// not a number is refused, not dropped. Gives whether it was taken.
    pub fn finish_typing(&mut self) -> bool {
        let Some(typed) = self.typing.clone() else { return false };
        let Some(setting) = self.chosen() else { return false };
        let change = match self.check(setting, &typed) {
            Ok(change) => change,
            Err(refusal) => {
                self.refusal = Some(refusal);
                return false;
            }
        };
        self.cancel_typing();
        self.settle(setting, change);
        true
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
            Kind::PickOr(choices) => {
                let mut names: Vec<String> = choices.iter().map(|c| c.word.to_string()).collect();
                let takes = match setting.path().as_str() {
                    "theme.name" => crate::ThemeConfig::named(word).is_some(),
                    "ime.scheme" => match &self.schemes {
                        Some(schemes) => {
                            names = schemes.names.clone();
                            (schemes.takes)(word)
                        }
                        None => true,
                    },
                    "ime.annotation" => {
                        names = std::iter::once("off".to_string()).chain(self.annotation_modes()).collect();
                        names.iter().any(|n| n == word)
                    }
                    _ => true,
                };
                match (word.is_empty(), takes) {
                    (true, _) => Err(Refusal::Empty { names }),
                    (false, false) => Err(Refusal::NotOneOf { word: word.to_string(), names }),
                    (false, true) => Ok(Change::Text(word.to_string())),
                }
            }
            _ => {
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

    /// `Esc` —— 不改了。
    pub fn cancel_typing(&mut self) {
        self.typing = None;
        self.tail = 0;
        self.refusal = None;
    }

    /// `d` —— 把這一項從**切着的那一份**裏撤掉，回去跟下面那一層。
    ///
    /// Gives whether anything was there to drop (2026-11-08). On a row this
    /// file does not set, the row already follows the layer below and `d`
    /// has nothing to do — the front end says so, rather than leaving a key
    /// the hint line names to do nothing on screen. Not on the group list:
    /// no row is chosen there, and the one `here()` gives is only the
    /// group's first.
    pub fn drop_here(&mut self) -> Option<bool> {
        if self.pane != Pane::Settings {
            return None;
        }
        let setting = self.here().filter(|s| s.kind != Kind::Page)?;
        Some(self.drop_path(setting.path()))
    }

    /// Drop `path` from the file switched to: whether that changed anything.
    fn drop_path(&mut self, path: String) -> bool {
        let sheet = self.sheet_mut();
        let was = sheet.edits.get(&path).cloned();
        // Warning: **問的是「檔裏寫了没有」，不是「這一份此刻說什麽」。** 後者把還没存
        // 的改動也算進去，於是「改了一項，又按 `d` 反悔」會攢下一條刪一個根本不
        // 存在的鍵的指令——面板頂上從此寫着「改了還没存」，而其實什麽都不用存。
        // 2026-09-24 拍圖看出來的。
        match sheet.declared.contains_key(&path) {
            true => sheet.edits.insert(path.clone(), Change::Drop),
            false => sheet.edits.remove(&path),
        };
        was != sheet.edits.get(&path).cloned()
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
        self.cancel_typing();
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

    // ---- 存 -------------------------------------------------------------

    /// 有没有改了還没存的。
    pub fn dirty(&self) -> bool {
        self.global.dirty() || self.local.dirty()
    }

    /// **`:w`** —— 兩份各寫各的，寫完把改動清掉。
    ///
    /// 回來的是寫了哪幾個檔，和其中哪幾個在面板開着的時候被外面改過。
    ///
    /// Warning: **A file the loader cannot read is not written** (2026-11-08),
    /// and then nothing is: both are asked before either is touched. Saving
    /// into it would leave it as unread as before and say the settings were
    /// in force (reported: a TOML error came back as a raw, truncated
    /// English line inside the Chinese panel).
    pub fn save(&mut self) -> Result<Saved, SaveError> {
        let mut plan = Vec::new();
        for which in [Which::Global, Which::Local] {
            let sheet = self.sheet_of(which);
            if sheet.edits.is_empty() {
                continue;
            }
            let Some(path) = sheet.path.clone() else {
                return Err(SaveError::Io(format!("{which:?}")));
            };
            // Warning: **照磁碟上此刻那一份改，不是照開面板那一刻那一份。**
            //
            // `sheet.text` 是開面板時讀的。面板開着的這段時間裏那個檔可能被外面
            // 改了——另一個 yumete、另一臺機器上同步下來的、`git checkout` ——而
            // 拿舊正文改完整份寫回去，**會把人家改的別的鍵一起頂掉**。改的那幾項
            // 是「鍵 → 新值」，套在哪一份正文上都成立，所以重讀一次就没有這件事。
            //
            // Warning: 讀不到就用手上這一份（檔剛被刪掉/權限没了）：那時 `write_atomically`
            // 自己會報，不必在這裏先攔一道。
            let now = std::fs::read_to_string(&path).unwrap_or_else(|_| sheet.text.clone());
            if let Some((line, said)) = crate::faults::unread(&now) {
                return Err(SaveError::Unread { path, line, said });
            }
            let changes: Vec<(String, Change)> =
                sheet.edits.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let fresh = rewrite(&now, &changes).map_err(SaveError::Io)?;
            plan.push((which, path, now != sheet.text, fresh));
        }
        let mut saved = Saved::default();
        for (which, path, moved, fresh) in plan {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| SaveError::Io(format!("{}: {e}", parent.display())))?;
            }
            write_atomically(&path, &fresh).map_err(|e| SaveError::Io(format!("{}: {e}", path.display())))?;
            let sheet = match which {
                Which::Global => &mut self.global,
                Which::Local => &mut self.local,
            };
            sheet.text = fresh;
            sheet.declared = declared_in(&sheet.text);
            sheet.there = true;
            sheet.edits.clear();
            if moved {
                saved.changed_underneath.push(path.clone());
            }
            saved.wrote.push(path);
        }
        Ok(saved)
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

    /// 三層，後面的蓋前面的，**而被蓋掉的那一個說得出來**。
    #[test]
    fn the_layer_that_wins_is_named_and_so_is_the_one_under_it() {
        let p = panel("[editor]\nindent = 4\n", "[editor]\nindent = 2\n");
        let shown = p.shown(find("editor.indent"));
        assert_eq!(shown.value, "2");
        assert_eq!(shown.from, Layer::Local);
        assert_eq!(shown.under, Some((Layer::Global, "4".to_string())));

        // 誰都没說 —— 出廠那一層贏，下面没有東西了。
        let bare = panel("", "");
        let shown = bare.shown(find("editor.indent"));
        assert_eq!((shown.value.as_str(), shown.from, shown.under), ("0", Layer::Factory, None));
    }

    /// **面板顯示的要是編輯器真用的那個值**（2026-10-09 作者定）。
    ///
    /// 面板自己撥不到域外的數，可檔是人手改的——`zong_length = 2` 寫進去，從前面板
    /// 畫 2 而 `into_config` 按下界排版。`markers` 空的時候 `into_config` 留出廠那一
    /// 串，而面板畫的是空白，候選欄照舊 １２３。
    ///
    /// Warning: **下界 2026-10-09 從 4 擡到 8**，和 `:view-wrap-height` 對齊（那一頭用
    /// 的是 `wrap::MIN_WRAP_WIDTH`）。從前面板說 4、命令說 8、檔案說 4。
    #[test]
    fn the_panel_shows_what_the_editor_takes_not_what_the_file_says() {
        let p = panel("[editor]\nzong_length = 2\npages_vertical = 99\n\n[panel]\nmarkers = \"\"\n", "");
        assert_eq!(p.shown(find("editor.zong_length")).value, "8", "域是「0，或者 8 到 64」");
        assert_eq!(p.shown(find("editor.pages_vertical")).value, "4", "上限夾住");
        assert_eq!(
            p.shown(find("panel.markers")).value,
            find("panel.markers").factory,
            "空的留出廠那一串"
        );
        // `0` 另有意思的那幾項照過，不許夾到 `low`。
        let zero = panel("[editor]\nzong_length = 0\n", "");
        assert_eq!(zero.shown(find("editor.zong_length")).value, "0", "0 ＝與窗口同高");
    }

    /// 空格：打勾翻面、幾選一走下一個、數字加一。
    #[test]
    fn a_press_moves_each_kind_the_way_that_kind_moves() {
        let mut p = panel("", "");
        p.pane = Pane::Settings;

        // 打勾（soft_wrap 出廠 true）。
        p.row = p.rows().iter().position(|s| s.key == "soft_wrap").unwrap();
        p.press();
        assert_eq!(p.shown(find("editor.soft_wrap")).value, "false");

        // 幾選一（layout 出廠 "horizontal"）。
        p.row = p.rows().iter().position(|s| s.key == "layout").unwrap();
        p.press();
        assert_eq!(p.shown(find("editor.layout")).value, "\"vertical\"");
        p.press();
        assert_eq!(p.shown(find("editor.layout")).value, "\"horizontal\"", "轉回來");

        // 數字（pages_vertical 出廠 1，範圍 1..4）。
        p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
        p.press();
        assert_eq!(p.shown(find("editor.pages_vertical")).value, "2");
    }

    /// 數字到頂回到底 —— 不然按到 4 之後那一格就死了。
    #[test]
    fn a_count_at_its_ceiling_comes_back_to_the_floor() {
        let mut p = panel("[editor]\npages_vertical = 4\n", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
        p.press();
        assert_eq!(p.shown(find("editor.pages_vertical")).value, "1");
    }

    /// `d` 撤掉這一層，回去跟下面那一層。
    #[test]
    fn dropping_this_layer_falls_back_to_the_one_under_it() {
        let mut p = panel("[editor]\nindent = 4\n", "[editor]\nindent = 2\n");
        p.into = Which::Local;
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "indent").unwrap();
        p.drop_here();
        let shown = p.shown(find("editor.indent"));
        assert_eq!((shown.value.as_str(), shown.from), ("4", Layer::Global));
    }

    /// **改了一項又按 `d` 反悔，該乾乾淨淨**——不是攢一條刪一個不存在的鍵的指令。
    ///
    /// Warning: 攢下來的話，面板頂上從此寫着「改了還没存」而其實什麽都不用存，走的時候
    /// 還要多問一句「有改動没存」。2026-09-24 拍圖看出來的。
    #[test]
    fn changing_a_row_and_then_dropping_it_leaves_nothing_behind() {
        let mut p = panel("", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "measure").unwrap();
        p.begin_typing();
        p.typing = Some("36".to_string());
        p.finish_typing();
        assert!(p.dirty(), "改過了");
        p.drop_here();
        assert!(!p.dirty(), "反悔之後乾淨了：{:?}", p.global.edits);
        assert_eq!(p.shown(find("editor.measure")).value, "0");
    }

    /// **檔裏帶引號或反斜線的一段字，來回一趟要一個字不差**（2026-10-01）。
    ///
    /// Warning: 從前這一頭用 `trim_matches('"')` 還原，而 [`declared_in`] 交出來的是
    /// `toml_edit` 的原樣寫法（轉義過的）。於是 `"a\"b"` 進面板成了 `a\"b`，寫
    /// 回去成了 `"a\\\"b"`——反斜線每存一次多一根，而 `settle` 從此判不出
    /// 「改回原值」，那一行永遠掛着「改了還没存」。
    #[test]
    fn a_quote_in_a_setting_survives_the_round_trip() {
        // 檔裏是 toml 的寫法：值本身是 `a"b\c`。
        let mut p = panel("[editor]\nindent_symbol = \"a\\\"b\\\\c\"\n", "");
        p.pane = Pane::Settings;
        // `indent_symbol` 在「標記」那一組，不是默認那一組。
        p.group = GROUPS.iter().position(|n| n.group == crate::settings_ui::Group::Marks).unwrap();
        p.row = p.rows().iter().position(|s| s.key == "indent_symbol").unwrap();
        // 進去打字，一個字不改就出來——這不是一次改動。
        p.begin_typing();
        assert_eq!(p.typing.as_deref(), Some("a\"b\\c"), "還原成值本身，不是它的寫法");
        p.finish_typing();
        assert!(!p.dirty(), "一個字没改，不算改過：{:?}", p.global.edits);
    }

    /// **轉一圈回到原來那個值，就不算改過。**
    ///
    /// Warning: 算的話，`:w` 會往檔裏寫一行跟原來一模一樣的東西，而面板頂上一直亮着
    /// 「改了還没存」。
    #[test]
    fn cycling_all_the_way_back_is_not_a_change() {
        let mut p = panel("", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
        for _ in 0..4 {
            p.press();
        }
        assert_eq!(p.shown(find("editor.pages_vertical")).value, "1", "1→2→3→4→1");
        assert!(!p.dirty(), "轉回來就不算改過：{:?}", p.global.edits);
    }

    /// 檔裏寫着 3，改成別的再改回 3 —— 同理，不算改過。
    #[test]
    fn setting_a_row_back_to_what_the_file_says_is_not_a_change() {
        let mut p = panel("[editor]\npages_vertical = 3\n", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
        p.press();
        assert!(p.dirty(), "3→4");
        p.press();
        p.press();
        p.press();
        assert_eq!(p.shown(find("editor.pages_vertical")).value, "3", "4→1→2→3");
        assert!(!p.dirty(), "回到檔裏那個值：{:?}", p.global.edits);
    }

    /// **「把全局設成和本項目一樣」是一次真的改動**，不是「没改」。
    ///
    /// Warning: 判成没改的話，一個想「以後每個項目都這樣」的人按了空格、看見值對了、
    /// `:w` 存了，而全局那個檔一個字没動——換一個目録打開就打回原形。
    #[test]
    fn matching_what_the_project_says_still_changes_the_global_file() {
        let mut p = panel("", "[editor]\npages_vertical = 2\n");
        p.into = Which::Global;
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
        // 出廠 1 → 按一下是 2，和本項目那個值一樣。
        p.press();
        assert!(p.dirty(), "全局那一份真的改了：{:?}", p.global.edits);
        assert_eq!(p.global.says("editor.pages_vertical").as_deref(), Some("2"));
    }

    /// **被上面那一層蓋着的時候，說得出來。**
    #[test]
    fn editing_a_layer_that_is_covered_says_so() {
        let mut p = panel("", "[editor]\npages_vertical = 2\n");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
        p.into = Which::Global;
        assert!(p.shadowed(find("editor.pages_vertical")), "本項目蓋着全局");
        p.into = Which::Local;
        assert!(!p.shadowed(find("editor.pages_vertical")), "本項目自己在最上面");
    }

    /// **存盤是原子的**：換的是目録項，寫到一半的東西不會出現在那個路徑上。
    ///
    /// Warning: 直接 `fs::write` 先把檔截成 0 再寫——斷電或被殺就剩半份，而半份 toml
    /// 多半讀不通，於是整份設定作廢。一個人只是想把縮進改成 2。
    #[test]
    fn saving_replaces_the_file_whole_and_leaves_no_litter() {
        let dir = std::env::temp_dir().join(format!("yumete-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "# 我寫的\n[editor]\nindent = 4    # 理由\n").unwrap();

        let mut p = Panel::open(Some(path.clone()), None);
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
        p.press();
        p.save().expect("存得下去");

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# 我寫的") && after.contains("# 理由"), "{after}");
        assert!(after.contains("pages_vertical = 2"), "{after}");
        // 暫存檔不許留在人家的目録裏。
        let litter: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != "config.toml")
            .collect();
        assert!(litter.is_empty(), "留了垃圾：{litter:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **面板開着的時候檔被外面改了，`:w` 不許把人家改的頂掉。**
    ///
    /// Warning: 拿開面板那一刻的正文改完整份寫回去，別人改的**別的鍵**就没了。改的那
    /// 幾項是「鍵 → 新值」，套在哪一份正文上都成立，所以存盤前重讀一次。
    #[test]
    fn saving_keeps_what_somebody_else_changed_meanwhile() {
        let dir = std::env::temp_dir().join(format!("yumete-set-2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[editor]\nindent = 4\n").unwrap();

        let mut p = Panel::open(Some(path.clone()), None);
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
        p.press();

        // 面板開着的這段時間裏，外面動了**別的**鍵。
        std::fs::write(&path, "[editor]\nindent = 4\nsoft_wrap = false\n").unwrap();

        let said = p.save().expect("存得下去");
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("pages_vertical = 2"), "我改的那一項落地了：\n{after}");
        assert!(after.contains("soft_wrap = false"), "人家改的那一項没被頂掉：\n{after}");
        assert_eq!(said.changed_underneath.len(), 1, "而且說了一句：{said:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 這一份本來就没說過的，`d` 什麽都不做——而不是攢一條「刪掉」。
    #[test]
    fn dropping_what_this_layer_never_said_writes_nothing() {
        let mut p = panel("[editor]\nindent = 4\n", "");
        p.into = Which::Local;
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "indent").unwrap();
        p.drop_here();
        assert!(!p.dirty(), "空的改動表：{:?}", p.local.edits);
    }

    /// 打了個不是數的東西 —— **不收，留在框裏說為什麽**（2026-11-08；從前是
    /// 一聲不吭地當没改），不是撥成 0。
    #[test]
    fn typing_something_that_is_not_a_number_is_refused_and_kept() {
        let mut p = panel("[editor]\nindent = 4\n", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "indent").unwrap();
        p.begin_typing();
        p.typing = Some("兩格".to_string());
        assert!(!p.finish_typing());
        assert!(!p.dirty());
        assert_eq!(p.typing.as_deref(), Some("兩格"), "still in the field");
        assert_eq!(p.refusal, Some(Refusal::NotANumber { low: 0, high: 8, zero: false }));
        assert_eq!(p.shown(find("editor.indent")).value, "4");
        // Empty is not a number either.
        p.typing = Some(String::new());
        assert!(!p.finish_typing());
        // Fixed, it is taken, and the word goes.
        p.typing = Some("2".to_string());
        assert!(p.finish_typing());
        assert_eq!((p.typing.as_deref(), p.refusal.as_ref()), (None, None));
        assert_eq!(p.shown(find("editor.indent")).value, "2");
    }

    /// **A number out of range is refused, not pulled in** (2026-11-08,
    /// reported: `99` became `64` without a word) — and a `0` that means
    /// something of its own is taken.
    #[test]
    fn a_number_out_of_range_is_refused_not_pulled_in() {
        let mut p = panel("", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
        p.begin_typing();
        p.typing = Some("99".to_string());
        assert!(!p.finish_typing());
        assert_eq!(p.refusal, Some(Refusal::OutOfRange { low: 1, high: 4, zero: false }));
        assert_eq!(p.shown(find("editor.pages_vertical")).value, "1", "nothing set");
        p.typing = Some("-1".to_string());
        assert!(!p.finish_typing());
        assert!(matches!(p.refusal, Some(Refusal::OutOfRange { .. })), "a number, out of range");
        p.cancel_typing();
        assert_eq!(p.refusal, None, "Esc takes the word away with the field");

        p.row = p.rows().iter().position(|s| s.key == "zong_length").unwrap();
        for (typed, ok) in [("0", true), ("4", false), ("8", true), ("64", true), ("65", false)] {
            p.begin_typing();
            p.typing = Some(typed.to_string());
            assert_eq!(p.finish_typing(), ok, "{typed}");
            p.cancel_typing();
        }
        assert_eq!(
            p.check(find("editor.zong_length"), "4"),
            Err(Refusal::OutOfRange { low: 8, high: 64, zero: true })
        );
    }

    /// **A name is one the config takes** (2026-11-08, reported: `nosuch`
    /// as a theme was saved and said to be in force). Empty is no name; a
    /// scheme is asked of the front end when it says which there are.
    #[test]
    fn a_theme_a_scheme_and_an_annotation_mode_must_be_ones_there_are() {
        let mut p = panel("[ime.annotation_modes.mine]\nunicode = true\n", "");
        let theme = find("theme.name");
        assert_eq!(p.check(theme, "bw"), Ok(Change::Text("bw".into())));
        assert_eq!(p.check(theme, "墨香"), Ok(Change::Text("墨香".into())), "the loader takes it");
        assert!(matches!(p.check(theme, "nosuch"), Err(Refusal::NotOneOf { ref word, ref names }) if word == "nosuch" && names.contains(&"ink".to_string())));
        assert!(matches!(p.check(theme, "  "), Err(Refusal::Empty { .. })));

        let scheme = find("ime.scheme");
        assert!(p.check(scheme, "custom").is_ok(), "no list, any name");
        assert!(matches!(p.check(scheme, ""), Err(Refusal::Empty { .. })));
        p.schemes = Some(Names { takes: |w| w == "lingming", names: vec!["lingming".into()] });
        assert!(p.check(scheme, "lingming").is_ok());
        assert!(matches!(p.check(scheme, "custom"), Err(Refusal::NotOneOf { ref names, .. }) if names == &["lingming"]));

        let annotation = find("ime.annotation");
        assert!(p.check(annotation, "mine").is_ok(), "a mode the file adds");
        assert!(p.check(annotation, "off").is_ok());
        assert!(matches!(p.check(annotation, "nosuch"), Err(Refusal::NotOneOf { .. })));
    }

    /// **A piece of text is what the loader takes** (2026-11-08, reported:
    /// a page of `ZZZ` and an empty one were saved): the loader's own
    /// sentence is the reason, and an empty font list — which the loader
    /// takes as no list — is refused.
    #[test]
    fn a_piece_of_text_is_checked_by_the_loader_itself() {
        let p = panel("", "");
        let page = find("export.page");
        assert!(p.check(page, "a5").is_ok());
        assert!(p.check(page, "137x195").is_ok());
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

    /// **On the group list nothing is edited** (2026-11-08, reported: `i` on
    /// 版面 flipped 排版方向, which was not highlighted).
    #[test]
    fn nothing_is_changed_from_the_group_list() {
        let mut p = panel("", "");
        assert_eq!(p.pane, Pane::Groups);
        p.press();
        p.change_here();
        p.begin_typing();
        assert!(p.typing.is_none() && !p.dirty(), "{:?}", p.global.edits);
        assert_eq!(p.drop_here(), None);
    }

    /// **A file the loader cannot read**: what it says is not shown as in
    /// force, `:w` writes nothing and says why — a TOML error, or a name the
    /// config does not take (2026-11-08).
    #[test]
    fn a_file_the_loader_cannot_read_is_not_written() {
        let dir = std::env::temp_dir().join(format!("yumete-set-unread-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
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
            // An edit to the other file goes in, and the unread one blocks
            // the save of both.
            p.into = Which::Local;
            p.pane = Pane::Settings;
            p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
            p.press();
            p.into = Which::Global;
            p.press();
            match p.save() {
                Err(SaveError::Unread { path, line: at, said }) => {
                    assert_eq!((path, at), (global.clone(), line));
                    assert!(!said.is_empty());
                }
                other => panic!("{other:?}"),
            }
            assert_eq!(std::fs::read_to_string(&global).unwrap(), text, "not touched");
            assert_eq!(std::fs::read_to_string(&local).unwrap(), "", "nor the other one");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`:w` names every file it wrote** (2026-11-08, reported: it wrote
    /// both and named one).
    #[test]
    fn saving_names_every_file_it_wrote() {
        let dir = std::env::temp_dir().join(format!("yumete-set-both-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let global = dir.join("config.toml");
        let local = dir.join("p").join(".yumete").join("config.toml");
        let mut p = Panel::open(Some(global.clone()), Some(local.clone()));
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "pages_vertical").unwrap();
        p.press();
        p.flip_into();
        p.press();
        let saved = p.save().expect("saved");
        assert_eq!(saved.wrote, vec![global.clone(), local.clone()]);
        assert!(saved.changed_underneath.is_empty());
        assert!(p.local.there, "the file is there now");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The state a stale message is measured against** moves with the
    /// cursor and with the values, and not otherwise.
    #[test]
    fn the_spot_moves_with_the_cursor_and_the_values() {
        let mut p = panel("", "");
        let here = p.spot();
        assert_eq!(p.spot(), here);
        p.across(true);
        assert_ne!(p.spot(), here);
        let here = p.spot();
        p.press();
        assert_ne!(p.spot(), here, "a value moved");
        assert!(p.edited("editor.layout") && !p.edited("editor.zong_length"));
    }

    /// **空的那一組進不去。**
    ///
    /// Warning: `l` 進去會停在一張空表上，而那一刻 `j`/`k`/空格全部没有反應——看起來
    /// 像面板卡住了。
    ///
    /// Warning: **這一條不許挑「碰巧空着的那一組」。** 從前它挑的是鍵盤組（那時那一組
    /// 還没填），2026-09-24 那一組填上之後測試當場紅——而它守的規矩一個字没變，
    /// 變的只是「哪一組是空的」。八組現在都有東西（3～14 項），所以造一個：把
    /// 下標指到 `GROUPS` 外面去，`rows()` 回空，走的是同一條路。
    #[test]
    fn a_group_with_nothing_in_it_cannot_be_walked_into() {
        let mut p = panel("", "");
        p.group = GROUPS.len();
        assert!(p.rows().is_empty(), "指到單子外面，那一組什麽都没有");
        p.across(true);
        assert_eq!(p.pane, Pane::Groups, "進不去");
    }

    /// **八組都有東西** —— 上面那一條不再有現成的空組可挑，而這一條說出為什麽。
    ///
    /// Warning: 順帶守一件事：一組空着的話，面板上那一格點進去是一張白表，而讀者不知道
    /// 是「没有這一類設定」還是「壞了」。要麽填，要麽別列。
    #[test]
    fn no_group_is_empty() {
        let p = panel("", "");
        let empty: Vec<&str> = GROUPS
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                let mut q = p.clone();
                q.group = *i;
                q.rows().is_empty()
            })
            .map(|(_, n)| n.label)
            .collect();
        assert!(empty.is_empty(), "這幾組是空的：{empty:?}");
    }

    /// 換組之後光標從頭數起 —— 不然停在一個不存在的行號上。
    #[test]
    fn moving_to_another_group_starts_at_its_first_row() {
        let mut p = panel("", "");
        p.across(true);
        p.row = 5;
        p.across(false);
        p.step(true);
        assert_eq!(p.row, 0);
    }

    /// **The flavour's page** (2026-11-02): a tick written into
    /// `[keys.flavor_items]`, flipped back to what the level gives and the
    /// line goes, and a binding of one of its keys is said.
    #[test]
    fn the_flavor_page_writes_items_by_name_and_takes_them_back() {
        use yumete_cjk::FlavorItem;
        let dir = std::env::temp_dir().join(format!("yumete-set-flavor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[keys]\npreset = \"helix\"\n\n[keys.normal]\n\"A-j\" = \"page_down\"\n").unwrap();

        let mut p = Panel::open(Some(path.clone()), None);
        p.pane = Pane::Settings;
        p.group = GROUPS.iter().position(|g| g.group == crate::settings_ui::Group::Keys).unwrap();
        p.row = p.rows().iter().position(|s| s.key == "flavor_items").unwrap();
        p.press();
        assert_eq!(p.page, Some(0), "Space on the row goes in");
        assert_eq!(p.flavor_here(), Some(FlavorRow::Level));
        let at = |p: &Panel, item: FlavorItem| {
            p.flavor_rows().iter().position(|(_, r)| *r == FlavorRow::Item(item)).unwrap()
        };

        // Off by name: a basic item at the basic level.
        p.page = Some(at(&p, FlavorItem::Tables));
        assert_eq!(p.item_shown(FlavorItem::Tables), (true, ItemFrom::Level));
        p.flavor_press();
        assert_eq!(p.item_shown(FlavorItem::Tables), (false, ItemFrom::Layer(Layer::Global)));
        // A binding of `A-j` has taken one of `move_lines`'s keys.
        assert_eq!(p.item_taken(FlavorItem::MoveLines), vec!["A-j"]);
        p.save().expect("saved");
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("[keys.flavor_items]") && after.contains("tables = false"), "{after}");

        // Flipped back to what the level gives: the line goes, and the
        // table with it.
        let mut p = Panel::open(Some(path.clone()), None);
        p.pane = Pane::Settings;
        p.group = GROUPS.iter().position(|g| g.group == crate::settings_ui::Group::Keys).unwrap();
        p.row = p.rows().iter().position(|s| s.key == "flavor_items").unwrap();
        p.page = Some(at(&p, FlavorItem::Tables));
        p.flavor_press();
        assert_eq!(p.item_shown(FlavorItem::Tables), (true, ItemFrom::Level));
        p.save().expect("saved");
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(!after.contains("flavor_items"), "{after}");
        assert!(after.contains("\"A-j\" = \"page_down\""), "the binding stays: {after}");

        // A full item at the basic level: on by name, then the level moves
        // to full and the line is only the level's own answer; `d` drops it.
        let mut p = Panel::open(Some(path.clone()), None);
        p.pane = Pane::Settings;
        p.group = GROUPS.iter().position(|g| g.group == crate::settings_ui::Group::Keys).unwrap();
        p.row = p.rows().iter().position(|s| s.key == "flavor_items").unwrap();
        p.page = Some(at(&p, FlavorItem::EscCollapses));
        p.flavor_press();
        assert_eq!(p.item_shown(FlavorItem::EscCollapses), (true, ItemFrom::Layer(Layer::Global)));
        p.page = Some(0);
        p.flavor_press();
        assert_eq!(p.flavor_level(), yumete_cjk::FlavorLevel::Full);
        p.page = Some(at(&p, FlavorItem::EscCollapses));
        p.page_drop();
        assert_eq!(p.item_shown(FlavorItem::EscCollapses), (true, ItemFrom::Level));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The annotation modes' page** (2026-11-02): a field ticked is written
    /// under `[ime.annotation_modes.<mode>]`, ticked back to the factory the
    /// line goes and the emptied table with it, and a mode the file adds
    /// gets its own fields.
    #[test]
    fn the_annotation_page_writes_fields_by_mode_and_takes_them_back() {
        let dir = std::env::temp_dir().join(format!("yumete-set-annotation-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[ime.annotation_modes.mine]\nunicode = true\n").unwrap();

        let mut p = Panel::open(Some(path.clone()), None);
        p.pane = Pane::Settings;
        p.group = GROUPS.iter().position(|g| g.group == crate::settings_ui::Group::Ime).unwrap();
        p.row = p.rows().iter().position(|s| s.key == "annotation_modes").unwrap();
        p.press();
        assert_eq!(p.page, Some(0), "Space on the row goes in");
        assert_eq!(p.annotation_modes(), ["multi", "double", "mine"]);
        assert_eq!(p.mode_fields("mine"), ["unicode"]);
        assert_eq!(p.mode_fields("double"), ["chaifen", "code"]);

        let at = |p: &Panel, mode: &str, field: &str| {
            p.annotation_rows().iter().position(|(_, r)| r.mode == mode && r.field == field).unwrap()
        };
        p.page = Some(at(&p, "double", "reading"));
        p.page_press();
        p.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[ime.annotation_modes.double]\nreading = true"), "{text}");
        assert_eq!(p.mode_fields("double"), ["chaifen", "code", "reading"]);
        assert_eq!(crate::Config::from_toml(&text).ime.annotation_modes[1].fields, ["chaifen", "code", "reading"]);

        // Back to the factory's: the line goes, and the table it was in.
        p.page_press();
        p.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("double"), "{text}");
        assert!(text.contains("[ime.annotation_modes.mine]"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Found in review (2026-11-02): an item written in an inline table, or
    /// with a comment after it, is what the file says, and saving works.
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

    /// **A font list with quotes in it is written to the file and read back
    /// as it was typed** (2026-11-08): typed, pasted over two lines into the
    /// middle, saved, and read by the config and by a new panel.
    #[test]
    fn a_font_list_typed_and_pasted_is_saved_and_read_back_whole() {
        let dir = std::env::temp_dir().join(format!("yumete-setfont-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut p = Panel::open(Some(path.clone()), None);
        p.pane = Pane::Settings;
        p.group = GROUPS.iter().position(|n| n.group == crate::settings_ui::Group::Files).unwrap();
        p.row = p.rows().iter().position(|s| s.table == "export" && s.key == "font_family").unwrap();
        p.begin_typing();
        p.typing = Some("\"Iowan Old Style\", serif".into());
        p.move_caret("\"Iowan Old Style\", ".chars().count());
        assert_eq!(p.paste("'Songti SC',\n\t\"Noto \\ Serif\",\n", Pasting::After), Some(28));
        p.type_char(' ');
        let typed = "\"Iowan Old Style\", 'Songti SC', \"Noto \\ Serif\", serif";
        assert_eq!(p.typing.as_deref(), Some(typed));
        p.finish_typing();
        p.save().expect("saved");

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(crate::Config::from_toml(&text).export.font_family, typed, "{text}");
        let again = Panel::open(Some(path.clone()), None);
        let setting = p.here().unwrap();
        assert_eq!(unwritten(&again.shown(setting).value), typed);
        assert!(!again.dirty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pasted text made one line: a break is one space, with the spaces
    /// beside it, and a break at either end goes, with the spaces beside it.
    #[test]
    fn a_paste_is_made_one_line() {
        assert_eq!(one_line("a,\n  b"), "a, b");
        assert_eq!(one_line("a5\n"), "a5");
        assert_eq!(one_line("a5\r"), "a5");
        assert_eq!(one_line("\r\n a, \r\n\r\n b \n"), "a, b");
        assert_eq!(one_line("  a"), "  a");
        assert_eq!(one_line("a\tb\u{7}"), "a b");
        assert_eq!(one_line(""), "");
    }
}
