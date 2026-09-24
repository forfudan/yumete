//! **每一項設定，說成一行** —— 那扇設置面板（#421）讀的就是這張表（2026-09-23）。
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

/// 一組，和它的名字。
///
/// ⚠️ **名字寫成一個 `label:` 字段，不是一支 `match`。** 兩個理由，第二個是硬的：
/// ① 次序與名字在同一張表上，加一組不會只加一半；② `messages.rs` 那張「每個標籤
/// 都有條目」的網是**讀源碼**找的，而它看得見 `label:`，看不見 `match` 的返回值
/// ——寫成 `match` 的話，八個組名可以一條條目都沒有而全樹皆綠，面板上逐行寫着
/// `set.group.layout`。
pub struct Named {
    pub group: Group,
    pub label: &'static str,
}

/// **八組，畫出來的次序。**
pub const GROUPS: &[Named] = &[
    Named { group: Group::Layout, label: "set.group.layout" },
    Named { group: Group::Marks, label: "set.group.marks" },
    Named { group: Group::Interface, label: "set.group.interface" },
    Named { group: Group::Colours, label: "set.group.colours" },
    Named { group: Group::Ime, label: "set.group.ime" },
    Named { group: Group::Editing, label: "set.group.editing" },
    Named { group: Group::Files, label: "set.group.files" },
    Named { group: Group::Keys, label: "set.group.keys" },
];

impl Group {
    /// 這一組的名字，給 `say!` 用。
    pub fn label(self) -> &'static str {
        GROUPS
            .iter()
            .find(|n| n.group == self)
            .map(|n| n.label)
            .unwrap_or("set.group.layout")
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
    /// ⚠️ **`zero` 不是 `None` 的時候，`low` 是「最小的**非零**值」**，而真正的
    /// 域是「0，或者 `low` 到 `high`」——中間沒有別的。`zong_length` 是「0，或者
    /// 4 到 64」，`tatechuyoko` 是「0，或者 2 到 8」。把 `low` 寫成 0 的話面板停
    /// 得到域外的數，寫進檔裏而 `into_config` 當場鉗掉——**面板顯示的值不是編輯器
    /// 用的值**。`tests/settings_ui.rs` 那條
    /// `every_value_the_panel_can_set_is_its_own` 盯着這件事。
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
    /// **出廠是什麽，照 toml 的寫法**：`0`、`true`、`"horizontal"`。
    ///
    /// 面板要說「這一項你沒設，出廠是這個」，而那句話得有個地方來。⚠️ 它是一個
    /// **抄來的**字面量，所以一定會漂——除非有人盯着。盯着的是
    /// `tests/settings_ui.rs` 那兩條：把整張表的 `factory` 拼成一份 toml 讀回來，
    /// 必須和 `Config::default()` 逐位元組相同；再逐項餵一個**不是**出廠的值，
    /// 必須真的改出點什麽來（不然就是鍵名打錯了，而
    /// `#[serde(deny_unknown_fields)]` 會把整份默默丟掉，兩條都綠）。
    pub factory: &'static str,
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

const EDITOR_LINE_NUMBERS_WAYS: &[Choice] = &[
    Choice { word: "absolute", label: "set.pick.line-numbers.absolute" },
    Choice { word: "relative", label: "set.pick.line-numbers.relative" },
    Choice { word: "none", label: "set.pick.line-numbers.none" },
];

const EDITOR_WORD_LEVEL_WAYS: &[Choice] = &[
    Choice { word: "strict", label: "set.pick.word-level.strict" },
    Choice { word: "balanced", label: "set.pick.word-level.balanced" },
    Choice { word: "full", label: "set.pick.word-level.full" },
];

const EDITOR_WORD_MARK_WAYS: &[Choice] = &[
    Choice { word: "color", label: "set.pick.word-mark.color" },
    Choice { word: "ink", label: "set.pick.word-mark.ink" },
    Choice { word: "tint", label: "set.pick.word-mark.tint" },
    Choice { word: "line", label: "set.pick.word-mark.line" },
];

const EDITOR_INDENT_HINT_WAYS: &[Choice] = &[
    Choice { word: "none", label: "set.pick.indent-hint.none" },
    Choice { word: "symbol", label: "set.pick.indent-hint.symbol" },
    Choice { word: "color", label: "set.pick.indent-hint.color" },
];

const EDITOR_TABLE_RULES_WAYS: &[Choice] = &[
    Choice { word: "line dash", label: "set.pick.table-rules.line-dash" },
    Choice { word: "line", label: "set.pick.table-rules.line" },
    Choice { word: "line double", label: "set.pick.table-rules.line-double" },
    Choice { word: "color", label: "set.pick.table-rules.color" },
    Choice { word: "off", label: "set.pick.table-rules.off" },
];

const EDITOR_TABS_WAYS: &[Choice] = &[
    Choice { word: "auto", label: "set.pick.tabs.auto" },
    Choice { word: "always", label: "set.pick.tabs.always" },
    Choice { word: "never", label: "set.pick.tabs.never" },
];

const PANEL_DISPLAY_WAYS: &[Choice] = &[
    Choice { word: "full", label: "set.pick.display.full" },
    Choice { word: "bare", label: "set.pick.display.bare" },
];

const THEME_MODE_WAYS: &[Choice] = &[
    Choice { word: "auto", label: "set.pick.mode.auto" },
    Choice { word: "dark", label: "set.pick.mode.dark" },
    Choice { word: "light", label: "set.pick.mode.light" },
];

const THEME_GROUND_WAYS: &[Choice] = &[
    Choice { word: "paint", label: "set.pick.ground.paint" },
    Choice { word: "terminal", label: "set.pick.ground.terminal" },
];

const THEME_STATUS_BAR_WAYS: &[Choice] = &[
    Choice { word: "sunken", label: "set.pick.status-bar.sunken" },
    Choice { word: "raised", label: "set.pick.status-bar.raised" },
];

/// ⚠️ **兩個詞，沒有同義詞**（`SystemImePolicy::parse` 自己的註釋這麽寫）：別的
/// 字一律讀成 `auto`。所以這裏不許多寫一個「看着像」的選項——面板上切過去、檔裏
/// 寫下去，而編輯器當它是 `auto`。2026-09-24 那條「改一項要真的改得動」逮到的。
const IME_SYSTEM_WAYS: &[Choice] = &[
    Choice { word: "auto", label: "set.pick.system.auto" },
    Choice { word: "keep", label: "set.pick.system.keep" },
];

const EDITOR_TAB_INSERTS_WAYS: &[Choice] = &[
    Choice { word: "spaces", label: "set.pick.tab-inserts.spaces" },
    Choice { word: "tab", label: "set.pick.tab-inserts.tab" },
];

const EDITOR_LANGUAGE_WAYS: &[Choice] = &[
    Choice { word: "zh", label: "set.pick.language.zh" },
    Choice { word: "zhs", label: "set.pick.language.zhs" },
    Choice { word: "en", label: "set.pick.language.en" },
];

const EDITOR_AMBIGUOUS_WIDTH_WAYS: &[Choice] = &[
    Choice { word: "auto", label: "set.pick.ambiguous-width.auto" },
    Choice { word: "wide", label: "set.pick.ambiguous-width.wide" },
    Choice { word: "narrow", label: "set.pick.ambiguous-width.narrow" },
];

/// **整張表。**
///
/// ⚠️ **八組都有東西了**（2026-09-24）。[`LATER`] 裏剩下的是二十四個色位（收在
/// 二級入口，見那張表自己的註釋）和幾項要新控件纔畫得出來的（列表、路徑）。
/// 那張一致性測試讀的是三張表的**並集**，所以漏一項照樣紅。
pub const SETTINGS: &[Setting] = &[
    // ---- 版面 ----------------------------------------------------------
    Setting {
        table: "editor",
        key: "layout",
        group: Group::Layout,
        kind: Kind::Pick(LAYOUT_WAYS),
        label: "set.editor.layout",
        hint: "set.editor.layout.hint",
        factory: r#""horizontal""#,
    },
    Setting {
        table: "editor",
        key: "zong_length",
        group: Group::Layout,
        // ⚠️ **`low` 是「最小的非零值」，不是「最小值」。** `into_config` 寫的是
        // `if length == 0 {0} else {length.clamp(4,64)}`——真正的域是「0，或者
        // 4 到 64」，中間那三個數不存在。寫 `low: 0` 的時候面板停得到 1／2／3，
        // 寫進檔裏而編輯器按 4 排版。2026-09-24 審出來的。
        kind: Kind::Count { low: 4, high: 64, zero: Some("set.zero.as-tall-as-the-window") },
        label: "set.editor.zong-length",
        hint: "set.editor.zong-length.hint",
        factory: "0",
    },
    Setting {
        table: "editor",
        key: "zong_gap",
        group: Group::Layout,
        kind: Kind::Count { low: 0, high: 4, zero: None },
        label: "set.editor.zong-gap",
        hint: "set.editor.zong-gap.hint",
        factory: "0",
    },
    Setting {
        table: "editor",
        key: "bands",
        group: Group::Layout,
        kind: Kind::Count { low: 1, high: 4, zero: None },
        label: "set.editor.bands",
        hint: "set.editor.bands.hint",
        factory: "1",
    },
    Setting {
        table: "editor",
        key: "indent",
        group: Group::Layout,
        kind: Kind::Count { low: 0, high: 8, zero: Some("set.zero.no-indent") },
        label: "set.editor.indent",
        hint: "set.editor.indent.hint",
        factory: "0",
    },
    Setting {
        table: "editor",
        key: "measure",
        group: Group::Layout,
        kind: Kind::Count { low: 0, high: 400, zero: Some("set.zero.as-wide-as-the-window") },
        label: "set.editor.measure",
        hint: "set.editor.measure.hint",
        factory: "0",
    },
    Setting {
        table: "editor",
        key: "ruler",
        group: Group::Layout,
        kind: Kind::Count { low: 0, high: 400, zero: Some("set.zero.no-ruler") },
        label: "set.editor.ruler",
        hint: "set.editor.ruler.hint",
        factory: "0",
    },
    Setting {
        table: "editor",
        key: "soft_wrap",
        group: Group::Layout,
        kind: Kind::Tick,
        label: "set.editor.soft-wrap",
        hint: "set.editor.soft-wrap.hint",
        factory: "true",
    },
    Setting {
        table: "editor",
        key: "hanging_punctuation",
        group: Group::Layout,
        kind: Kind::Tick,
        label: "set.editor.hanging-punctuation",
        hint: "set.editor.hanging-punctuation.hint",
        factory: "false",
    },
    Setting {
        table: "editor",
        key: "margin",
        group: Group::Layout,
        kind: Kind::Pick(MARGIN_WAYS),
        label: "set.editor.margin",
        hint: "set.editor.margin.hint",
        factory: r#""dense""#,
    },
    Setting {
        table: "editor",
        key: "tatechuyoko",
        group: Group::Layout,
        // 一格裝幾個半角字：**0（關），或者 2 到 8**。`into_config` 把 1 當成 0
        // （`at_most >= TATECHUYOKO_CLASSIC`），所以 `low` 是 2 而不是 0。
        kind: Kind::Count { low: 2, high: 8, zero: Some("set.zero.off") },
        label: "set.editor.tatechuyoko",
        hint: "set.editor.tatechuyoko.hint",
        factory: "4",
    },
    Setting {
        table: "editor",
        key: "paper_ticks",
        group: Group::Layout,
        kind: Kind::Count { low: 0, high: 64, zero: Some("set.zero.off") },
        label: "set.editor.paper-ticks",
        hint: "set.editor.paper-ticks.hint",
        factory: "0",
    },
    Setting {
        table: "editor",
        key: "line_numbers",
        group: Group::Marks,
        kind: Kind::Pick(EDITOR_LINE_NUMBERS_WAYS),
        label: "set.editor.line-numbers",
        hint: "set.editor.line-numbers.hint",
        factory: r#""absolute""#,
    },
    Setting {
        table: "editor",
        key: "line_number_fill",
        group: Group::Marks,
        kind: Kind::Tick,
        label: "set.editor.line-number-fill",
        hint: "set.editor.line-number-fill.hint",
        factory: "false",
    },
    Setting {
        table: "editor",
        key: "diff_gutter",
        group: Group::Marks,
        kind: Kind::Tick,
        label: "set.editor.diff-gutter",
        hint: "set.editor.diff-gutter.hint",
        factory: "true",
    },
    Setting {
        table: "editor",
        key: "show_segmentation",
        group: Group::Marks,
        kind: Kind::Tick,
        label: "set.editor.show-segmentation",
        hint: "set.editor.show-segmentation.hint",
        factory: "true",
    },
    Setting {
        table: "editor",
        key: "word_level",
        group: Group::Marks,
        kind: Kind::Pick(EDITOR_WORD_LEVEL_WAYS),
        label: "set.editor.word-level",
        hint: "set.editor.word-level.hint",
        factory: r#""balanced""#,
    },
    Setting {
        table: "editor",
        key: "word_mark",
        group: Group::Marks,
        kind: Kind::Pick(EDITOR_WORD_MARK_WAYS),
        label: "set.editor.word-mark",
        hint: "set.editor.word-mark.hint",
        factory: r#""color""#,
    },
    Setting {
        table: "editor",
        key: "code_highlight",
        group: Group::Marks,
        kind: Kind::Tick,
        label: "set.editor.code-highlight",
        hint: "set.editor.code-highlight.hint",
        factory: "true",
    },
    Setting {
        table: "editor",
        key: "show_ruby",
        group: Group::Marks,
        kind: Kind::Tick,
        label: "set.editor.show-ruby",
        hint: "set.editor.show-ruby.hint",
        factory: "false",
    },
    Setting {
        table: "editor",
        key: "show_chaifen",
        group: Group::Marks,
        kind: Kind::Tick,
        label: "set.editor.show-chaifen",
        hint: "set.editor.show-chaifen.hint",
        factory: "false",
    },
    Setting {
        table: "editor",
        key: "indent_hint",
        group: Group::Marks,
        kind: Kind::Pick(EDITOR_INDENT_HINT_WAYS),
        label: "set.editor.indent-hint",
        hint: "set.editor.indent-hint.hint",
        factory: r#""none""#,
    },
    Setting {
        table: "editor",
        key: "indent_symbol",
        group: Group::Marks,
        kind: Kind::Text,
        label: "set.editor.indent-symbol",
        hint: "set.editor.indent-symbol.hint",
        factory: r#""↵""#,
    },
    Setting {
        table: "editor",
        key: "table_rules",
        group: Group::Marks,
        kind: Kind::Pick(EDITOR_TABLE_RULES_WAYS),
        label: "set.editor.table-rules",
        hint: "set.editor.table-rules.hint",
        factory: r#""line dash""#,
    },
    Setting {
        table: "editor",
        key: "char_info",
        group: Group::Marks,
        kind: Kind::Tick,
        label: "set.editor.char-info",
        hint: "set.editor.char-info.hint",
        factory: "true",
    },
    Setting {
        table: "editor",
        key: "scrolloff",
        group: Group::Interface,
        kind: Kind::Count { low: 0, high: 20, zero: None },
        label: "set.editor.scrolloff",
        hint: "set.editor.scrolloff.hint",
        factory: "3",
    },
    Setting {
        table: "editor",
        key: "wheel_step",
        group: Group::Interface,
        kind: Kind::Count { low: 1, high: 10, zero: None },
        label: "set.editor.wheel-step",
        hint: "set.editor.wheel-step.hint",
        factory: "3",
    },
    Setting {
        table: "editor",
        key: "sidebar_width",
        group: Group::Interface,
        kind: Kind::Count { low: 12, high: 60, zero: None },
        label: "set.editor.sidebar-width",
        hint: "set.editor.sidebar-width.hint",
        factory: "24",
    },
    Setting {
        table: "editor",
        key: "detail_width",
        group: Group::Interface,
        kind: Kind::Count { low: 12, high: 80, zero: None },
        label: "set.editor.detail-width",
        hint: "set.editor.detail-width.hint",
        factory: "30",
    },
    Setting {
        table: "editor",
        key: "tabs",
        group: Group::Interface,
        kind: Kind::Pick(EDITOR_TABS_WAYS),
        label: "set.editor.tabs",
        hint: "set.editor.tabs.hint",
        factory: r#""auto""#,
    },
    Setting {
        table: "editor",
        key: "command_line",
        group: Group::Interface,
        kind: Kind::Tick,
        label: "set.editor.command-line",
        hint: "set.editor.command-line.hint",
        factory: "true",
    },
    Setting {
        table: "panel",
        key: "display",
        group: Group::Interface,
        kind: Kind::Pick(PANEL_DISPLAY_WAYS),
        label: "set.panel.display",
        hint: "set.panel.display.hint",
        factory: r#""full""#,
    },
    Setting {
        table: "panel",
        key: "page_size",
        group: Group::Interface,
        kind: Kind::Count { low: 1, high: 9, zero: None },
        label: "set.panel.page-size",
        hint: "set.panel.page-size.hint",
        factory: "6",
    },
    Setting {
        table: "panel",
        key: "markers",
        group: Group::Interface,
        kind: Kind::Text,
        label: "set.panel.markers",
        hint: "set.panel.markers.hint",
        factory: r#""㊀㊁㊂㊃㊄㊅㊆㊇㊈""#,
    },
    Setting {
        table: "panel",
        key: "rounded",
        group: Group::Interface,
        kind: Kind::Tick,
        label: "set.panel.rounded",
        hint: "set.panel.rounded.hint",
        factory: "true",
    },
    Setting {
        table: "theme",
        key: "name",
        group: Group::Colours,
        kind: Kind::Text,
        label: "set.theme.name",
        hint: "set.theme.name.hint",
        factory: r#""ink""#,
    },
    Setting {
        table: "theme",
        key: "mode",
        group: Group::Colours,
        kind: Kind::Pick(THEME_MODE_WAYS),
        label: "set.theme.mode",
        hint: "set.theme.mode.hint",
        factory: r#""auto""#,
    },
    Setting {
        table: "theme",
        key: "ground",
        group: Group::Colours,
        kind: Kind::Pick(THEME_GROUND_WAYS),
        label: "set.theme.ground",
        hint: "set.theme.ground.hint",
        factory: r#""paint""#,
    },
    Setting {
        table: "theme",
        key: "status_bar",
        group: Group::Colours,
        kind: Kind::Pick(THEME_STATUS_BAR_WAYS),
        label: "set.theme.status-bar",
        hint: "set.theme.status-bar.hint",
        factory: r#""sunken""#,
    },
    Setting {
        table: "theme",
        key: "fill",
        group: Group::Colours,
        kind: Kind::Tick,
        label: "set.theme.fill",
        hint: "set.theme.fill.hint",
        factory: "false",
    },
    Setting {
        table: "ime",
        key: "scheme",
        group: Group::Ime,
        kind: Kind::Text,
        label: "set.ime.scheme",
        hint: "set.ime.scheme.hint",
        factory: r#""lingming""#,
    },
    Setting {
        table: "ime",
        key: "start",
        group: Group::Ime,
        kind: Kind::Tick,
        label: "set.ime.start",
        hint: "set.ime.start.hint",
        factory: "false",
    },
    Setting {
        table: "ime",
        key: "system",
        group: Group::Ime,
        kind: Kind::Pick(IME_SYSTEM_WAYS),
        label: "set.ime.system",
        hint: "set.ime.system.hint",
        factory: r#""auto""#,
    },
    Setting {
        table: "editor",
        key: "indent_width",
        group: Group::Editing,
        kind: Kind::Count { low: 1, high: 16, zero: None },
        label: "set.editor.indent-width",
        hint: "set.editor.indent-width.hint",
        factory: "4",
    },
    Setting {
        table: "editor",
        key: "tab_width",
        group: Group::Editing,
        kind: Kind::Count { low: 1, high: 16, zero: None },
        label: "set.editor.tab-width",
        hint: "set.editor.tab-width.hint",
        factory: "8",
    },
    Setting {
        table: "editor",
        key: "tab_inserts",
        group: Group::Editing,
        kind: Kind::Pick(EDITOR_TAB_INSERTS_WAYS),
        label: "set.editor.tab-inserts",
        hint: "set.editor.tab-inserts.hint",
        factory: r#""spaces""#,
    },
    Setting {
        table: "editor",
        key: "autosave",
        group: Group::Editing,
        kind: Kind::Tick,
        label: "set.editor.autosave",
        hint: "set.editor.autosave.hint",
        factory: "true",
    },
    Setting {
        table: "editor",
        key: "smart_case",
        group: Group::Editing,
        kind: Kind::Tick,
        label: "set.editor.smart-case",
        hint: "set.editor.smart-case.hint",
        factory: "true",
    },
    Setting {
        table: "editor",
        key: "fuzzy_search",
        group: Group::Editing,
        kind: Kind::Tick,
        label: "set.editor.fuzzy-search",
        hint: "set.editor.fuzzy-search.hint",
        factory: "false",
    },
    Setting {
        table: "editor",
        key: "session",
        group: Group::Editing,
        kind: Kind::Tick,
        label: "set.editor.session",
        hint: "set.editor.session.hint",
        factory: "false",
    },
    Setting {
        table: "editor",
        key: "language",
        group: Group::Files,
        kind: Kind::Pick(EDITOR_LANGUAGE_WAYS),
        label: "set.editor.language",
        hint: "set.editor.language.hint",
        factory: r#""zh""#,
    },
    Setting {
        table: "editor",
        key: "ambiguous_width",
        group: Group::Files,
        kind: Kind::Pick(EDITOR_AMBIGUOUS_WIDTH_WAYS),
        label: "set.editor.ambiguous-width",
        hint: "set.editor.ambiguous-width.hint",
        factory: r#""auto""#,
    },
    Setting {
        table: "editor",
        key: "language_key",
        group: Group::Files,
        kind: Kind::Text,
        label: "set.editor.language-key",
        hint: "set.editor.language-key.hint",
        factory: r#""C-^""#,
    },
];

/// **還沒填進 [`SETTINGS`] 的那些。**
///
/// 一組一組填，這張表跟着縮短。⚠️ **它不是「不做」的名單**——那一張是
/// [`NOT_IN_THE_PANEL`]，兩者的區別是「還沒輪到」與「有理由不進去」。
pub const LATER: &[&str] = &[
    "editor.ruby_dialects",
    "editor.usage_groups",
    "editor.syntax",
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
    "panel.ink",
    "panel.paper",
    "ime.commit",
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
