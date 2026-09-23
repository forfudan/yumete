//! **每一項設定，說成一行** —— 那扇設置面板讀的就是這張表（2026-09-23）。
//!
//! 作者提的：
//!
//! > 這樣的話用戶（特別是寫小説的），不需要面對 toml 和一堆 key 發呆不知道他們
//! > 都是幹啥的，也不需要查詢到底有哪些 key 可以用。
//!
//! 所以這張表要說出 `RawConfig` 說不出的三件事：**這一項叫什麽**（給人看的名
//! 字）、**它是什麽**（打勾／幾選一／數字／一段字）、**它歸哪一組**。類型與取值
//! 域**抄 `into_config`**，不是想出來的——那一支纔是真正在鉗值的地方。
//!
//! ⚠️ **一張 Rust 常量表，不是一個 toml。** Yume 那頭是 `settings_layout.toml`
//! ＋ codegen，這個倉沒有那一套；而這張表要和 `RawEditor` 的字段名對得上，
//! **編譯期對得上比運行期對得上好**——`tests/settings_ui.rs` 讀源碼逐條核，少一
//! 項就紅。
//!
//! ⚠️ **少一項是一個無聲的洞。** 加了新設定而忘了加進這張表，面板上就沒有它，
//! 而編譯照過、面板照樣顯示別的設定的對的值——記事本裏那條「全局設置加一項要動
//! 三處，漏掉第三處編譯照過」說的就是這一族。

/// **左側那張單子的一格。**
///
/// 判準是**「改了它，屏幕上哪一塊會變」**，不是 toml 的 `[表]`：`[editor]` 一張
/// 表裝了 45 個鍵（87 個裏的一半），`tab_width`（寫程序的）和 `zong_gap`（寫小
/// 説的）在同一節底下。按 section 分只會得到一個 45 行的巨面板。
///
/// 次序是**寫小説的人當天就會碰的排前面**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// 版面：字怎麽排在紙上。
    Layout,
    /// 標記：畫在正文上面的那些。
    Marks,
    /// 界面：編輯器自己那幾條邊。
    Interface,
    /// 配色。
    Colours,
    /// 輸入法。
    Ime,
    /// 編輯：縮進、存檔、搜索的脾氣。
    Editing,
    /// 文件：什麽當什麽讀，導出成什麽。
    Files,
    /// 鍵盤。
    Keys,
}

impl Group {
    /// 畫出來的次序。
    pub const ALL: [Group; 8] = [
        Group::Layout,
        Group::Marks,
        Group::Interface,
        Group::Colours,
        Group::Ime,
        Group::Editing,
        Group::Files,
        Group::Keys,
    ];

    /// 這一組的名字，給 `say!` 用。
    pub fn label(self) -> &'static str {
        match self {
            Group::Layout => "set.group.layout",
            Group::Marks => "set.group.marks",
            Group::Interface => "set.group.interface",
            Group::Colours => "set.group.colours",
            Group::Ime => "set.group.ime",
            Group::Editing => "set.group.editing",
            Group::Files => "set.group.files",
            Group::Keys => "set.group.keys",
        }
    }
}

/// 幾選一裏的一個。`word` 是寫進 toml 的那個字，`label` 是給人看的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub word: &'static str,
    pub label: &'static str,
}

/// **這一項是什麽東西** —— 決定它畫成什麽控件，也決定 `:w` 寫回去什麽類型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 打勾。`[x]` / `[ ]`。
    Tick,
    /// 一個數，**鉗在這個範圍裏**——範圍抄 `into_config`，那一支纔是真在鉗的。
    ///
    /// `low` 是允許的最小值；⚠️ 有幾項的 `0` 是「不說」而不是零（`zong_length`
    /// 的 0 是「窗口有多高就多長」），那種寫 `low: 0` 而在 `zero` 裏說明。
    Count {
        low: usize,
        high: usize,
        /// `0` 另有意思的時候，這裏是那句話的 tag；沒有就是 `None`。
        zero: Option<&'static str>,
    },
    /// 幾選一，狀態名裝在方括號裏——同搜索面板的 `[智能] 大小寫`。
    Pick(&'static [Choice]),
    /// 一段自己打的字。
    Text,
}

/// **一項設定。**
#[derive(Debug, Clone, Copy)]
pub struct Setting {
    /// toml 裏的表名：`editor`、`theme`、`panel`、`ime`、`export`、`keys`。
    pub table: &'static str,
    /// toml 裏的鍵名，**和 `RawXxx` 的字段名逐字相同**。
    pub key: &'static str,
    pub group: Group,
    pub kind: Kind,
    /// 給人看的名字。
    pub label: &'static str,
    /// 一句話說它幹什麽。
    pub hint: &'static str,
}

impl Setting {
    /// `editor.zong_length` —— 寫進 toml 的路徑，也是面板記「哪一項改過了」的鍵。
    pub fn path(&self) -> String {
        format!("{}.{}", self.table, self.key)
    }
}

const LAYOUT_WAYS: &[Choice] = &[
    Choice { word: "horizontal", label: "set.layout.horizontal" },
    Choice { word: "vertical", label: "set.layout.vertical" },
];

const MARGIN_WAYS: &[Choice] = &[
    Choice { word: "never", label: "set.margin.never" },
    Choice { word: "dense", label: "set.margin.dense" },
    Choice { word: "loose", label: "set.margin.loose" },
    Choice { word: "always", label: "set.margin.always" },
];

/// **整張表。**
///
/// ⚠️ 眼下只有「版面」一組是滿的——其餘七組在 [`LATER`] 裏掛着，一組一組填。
/// 那張一致性測試讀的是這兩張表的**並集**，所以漏一項照樣紅。
pub const SETTINGS: &[Setting] = &[
    // ---- 版面 ----------------------------------------------------------
    Setting {
        table: "editor",
        key: "layout",
        group: Group::Layout,
        kind: Kind::Pick(LAYOUT_WAYS),
        label: "set.editor.layout",
        hint: "set.editor.layout.hint",
    },
    Setting {
        table: "editor",
        key: "zong_length",
        group: Group::Layout,
        // 4..64 照 `into_config`；0 是「窗口有多高就多長」，不鉗上去。
        kind: Kind::Count { low: 0, high: 64, zero: Some("set.zero.as-tall-as-the-window") },
        label: "set.editor.zong-length",
        hint: "set.editor.zong-length.hint",
    },
    Setting {
        table: "editor",
        key: "zong_gap",
        group: Group::Layout,
        kind: Kind::Count { low: 0, high: 4, zero: None },
        label: "set.editor.zong-gap",
        hint: "set.editor.zong-gap.hint",
    },
    Setting {
        table: "editor",
        key: "bands",
        group: Group::Layout,
        kind: Kind::Count { low: 1, high: 4, zero: None },
        label: "set.editor.bands",
        hint: "set.editor.bands.hint",
    },
    Setting {
        table: "editor",
        key: "indent",
        group: Group::Layout,
        kind: Kind::Count { low: 0, high: 8, zero: Some("set.zero.no-indent") },
        label: "set.editor.indent",
        hint: "set.editor.indent.hint",
    },
    Setting {
        table: "editor",
        key: "measure",
        group: Group::Layout,
        kind: Kind::Count { low: 0, high: 400, zero: Some("set.zero.as-wide-as-the-window") },
        label: "set.editor.measure",
        hint: "set.editor.measure.hint",
    },
    Setting {
        table: "editor",
        key: "ruler",
        group: Group::Layout,
        kind: Kind::Count { low: 0, high: 400, zero: Some("set.zero.no-ruler") },
        label: "set.editor.ruler",
        hint: "set.editor.ruler.hint",
    },
    Setting {
        table: "editor",
        key: "soft_wrap",
        group: Group::Layout,
        kind: Kind::Tick,
        label: "set.editor.soft-wrap",
        hint: "set.editor.soft-wrap.hint",
    },
    Setting {
        table: "editor",
        key: "hanging_punctuation",
        group: Group::Layout,
        kind: Kind::Tick,
        label: "set.editor.hanging-punctuation",
        hint: "set.editor.hanging-punctuation.hint",
    },
    Setting {
        table: "editor",
        key: "margin",
        group: Group::Layout,
        kind: Kind::Pick(MARGIN_WAYS),
        label: "set.editor.margin",
        hint: "set.editor.margin.hint",
    },
    Setting {
        table: "editor",
        key: "tatechuyoko",
        group: Group::Layout,
        // 一格裝幾個半角字。`TATECHUYOKO_MAX` 是 8，0 是關。
        kind: Kind::Count { low: 0, high: 8, zero: Some("set.zero.off") },
        label: "set.editor.tatechuyoko",
        hint: "set.editor.tatechuyoko.hint",
    },
    Setting {
        table: "editor",
        key: "paper_ticks",
        group: Group::Layout,
        kind: Kind::Count { low: 0, high: 64, zero: Some("set.zero.off") },
        label: "set.editor.paper-ticks",
        hint: "set.editor.paper-ticks.hint",
    },
];

/// **還沒填進 [`SETTINGS`] 的那些。**
///
/// 一組一組填，這張表跟着縮短。⚠️ **它不是「不做」的名單**——那一張是
/// [`NOT_IN_THE_PANEL`]，兩者的區別是「還沒輪到」與「有理由不進去」。
pub const LATER: &[&str] = &[
    "editor.tab_width",
    "editor.indent_width",
    "editor.line_numbers",
    "editor.scrolloff",
    "editor.wheel_step",
    "editor.line_number_fill",
    "editor.diff_gutter",
    "editor.indent_hint",
    "editor.indent_symbol",
    "editor.table_rules",
    "editor.language_key",
    "editor.show_segmentation",
    "editor.word_level",
    "editor.word_mark",
    "editor.session",
    "editor.language",
    "editor.show_chaifen",
    "editor.show_ruby",
    "editor.ruby_dialects",
    "editor.usage_groups",
    "editor.code_highlight",
    "editor.autosave",
    "editor.ambiguous_width",
    "editor.detail_width",
    "editor.sidebar_width",
    "editor.tabs",
    "editor.syntax",
    "editor.char_info",
    "editor.command_line",
    "editor.smart_case",
    "editor.fuzzy_search",
    "editor.tab_inserts",
    "theme.name",
    "theme.mode",
    "theme.ground",
    "theme.status_bar",
    "theme.fill",
    // ⚠️ **二十四個色位（十二個色 × 深淺兩套）收在二級入口裏，不攤在第一層。**
    // `ThemeConfig` 自己的註釋寫着「A theme is a few numbers, not a table of
    // colours」，十個內置主題已經覆蓋；而 TUI 裏沒有取色器，只有 `#RRGGBB` 文本
    // 框——擺二十四行等於讓人在一張色號表上發呆，正是這扇面板要消滅的那種發呆。
    "theme.ink",
    "theme.paper",
    "theme.ink_light",
    "theme.paper_light",
    "theme.mark",
    "theme.mark_light",
    "theme.gold",
    "theme.gold_light",
    "theme.purple",
    "theme.purple_light",
    "theme.green",
    "theme.green_light",
    "theme.azure",
    "theme.azure_light",
    "theme.amber",
    "theme.amber_light",
    "theme.orange",
    "theme.orange_light",
    "theme.pink",
    "theme.pink_light",
    "theme.cyan",
    "theme.cyan_light",
    "theme.lime",
    "theme.lime_light",
    "panel.display",
    "panel.page_size",
    "panel.markers",
    "panel.rounded",
    "panel.ink",
    "panel.paper",
    "ime.scheme",
    "ime.start",
    "ime.commit",
    "ime.system",
    "ime.data_dirs",
    "export.page",
    "keys.preset",
];

/// **有理由不進面板的那幾個。**
///
/// ⚠️ **判準一（決定性的）：值是一條會被執行的命令行的，不進。**
/// `[editor] screenshot` 是全樹唯一一條經過 shell 的設定——本地配置寫了它會被
/// 當場拒收（`load_reporting`）。`[lsp.*]` 與 `[language.*]` 同理。理由不是
/// 「難畫」，是**一扇設置面板不該變成從菜單裏運行任意程序的入口**。
///
/// ⚠️ **判準二：鍵集開放的表是編輯器，不是設置。** `[keys.normal]` 左邊是任意鍵
/// 序列、右邊是任意動作名；`[syntax]` 的鍵是任意擴展名。要做進面板得有「捕獲任
/// 意鍵序列」加「動作名補全」兩個新控件，那是另一個功能。
///
/// ⚠️ **`segmentation_threshold` 已經退役**，留在結構裏只為了報一句話。
pub const NOT_IN_THE_PANEL: &[&str] = &[
    "editor.screenshot",
    "editor.segmentation_threshold",
    "keys.normal",
];
