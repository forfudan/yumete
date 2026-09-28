//! **選區——複數的那一個**（#405，方案在 `docs/development.md §5.13`）。
//!
//! 語義逐條照 helix（`helix-core/src/selection.rs`，基準 commit `079a789e8`），只抄語義，
//! 不依賴它那個 crate——理由在 §5.13.5：`helix-core` 會拖進 `helix-loader`、tree-sitter，
//! 以及一個**釘死在舊版上的 `unicode-width`**，而這個倉有一條規矩是「一個概念一處權威：
//! 字素、寬度、分詞」，寬度那一處是 `yumete-cjk`。
//!
//! # 這是 Phase 0
//!
//! ⚠️ **現在永遠只裝一段。** 這一期的全部目的是「把單數包起來」，一個可見的變化都沒有——
//! 驗收條件是 `scripts/frames.sh` 那二十幀逐字節不變。裝得下多段是 Phase 1 的事。
//!
//! 所以下面刻意**還沒有**：排序、合併重疊、`normalize`、偏移記帳。那些是有了第二段之後
//! 纔有意義的東西，現在寫進來只是沒人跑得到的代碼。
//!
//! ⚠️ **也刻意還沒有把 `goal_column`／`goal_slot`／`zong_motion`／`extend`／`vim_lines`
//! 搬進 [`Range`]。** §5.13.9 原本把它們排在 Phase 0，但只有一段選區的時候，它們在
//! `Editor` 上還是在 `Range` 上行為完全一樣——搬過來是純粹的攪動。等 Phase 1 真有第二段、
//! 它們真的需要各有一份的時候再搬。

/// 一段選區。**光標就是一段一個字素寬的選區**——這個倉早就是這麼想的
/// （`editor/modes.rs` 的 `selection()`：「The grapheme the cursor sits on is *inside*
/// the selection, as it is in Helix」）。
///
/// 兩端都是**字符下標**（不是字節、不是字素），和這個倉別處一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Range {
    /// 撐開的時候**不動**的那一端。
    pub anchor: usize,
    /// 撐開的時候**動**的那一端。
    pub head: usize,
}

impl Range {
    /// 一段塌在 `at` 上的選區——也就是一個光標。
    pub fn at(at: usize) -> Range {
        Range { anchor: at, head: at }
    }

    /// 兩端按先後排好。⚠️ **方向是要留的信息**（`d` 之後光標落在哪、`;` 塌向哪一端都看
    /// 它），所以這一支只回答「哪個在前」，不改 `Range` 自己。
    pub fn span(self) -> (usize, usize) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    /// 兩端是不是同一個地方——「這只是一個光標，沒有選中東西」。
    pub fn is_caret(self) -> bool {
        self.anchor == self.head
    }
}

/// 編輯器手上的那一組選區，以及其中哪一段是**主選區**。
///
/// 主選區是那些「只能有一個」的東西要挑的那一段：終端的硬件光標、跟着光標跑的輸入法候選
/// 面板、頁面滾動跟誰、LSP 問哪一處。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selections {
    /// ⚠️ **Phase 0：長度恆為 1。** 不變式（排序、不重疊、至少一段）等 Phase 1。
    ranges: Vec<Range>,
    /// 主選區在 `ranges` 裏的下標。⚠️ 永遠 `< ranges.len()`。
    primary: usize,
}

impl Default for Selections {
    fn default() -> Selections {
        Selections::at(0)
    }
}

impl Selections {
    /// 一組只有一段、塌在 `at` 上的選區。
    pub fn at(at: usize) -> Selections {
        Selections { ranges: vec![Range::at(at)], primary: 0 }
    }

    /// 主選區。
    pub fn primary(&self) -> Range {
        self.ranges[self.primary]
    }

    fn primary_mut(&mut self) -> &mut Range {
        let which = self.primary;
        &mut self.ranges[which]
    }

    /// 一共幾段。
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// 有沒有多於一段——`jumping()` 那一類謂詞將來問的就是它。
    pub fn is_plural(&self) -> bool {
        self.ranges.len() > 1
    }

    /// 從頭到尾。
    pub fn iter(&self) -> impl Iterator<Item = &Range> {
        self.ranges.iter()
    }

    // ---- 主選區的兩端 ---------------------------------------------------
    //
    // 這四支是 Phase 0 的全部門面：從前直接讀寫 `editor.cursor` / `editor.anchor` 的那
    // 三百多處，現在走這裏。⚠️ 它們問的**永遠是主選區**——所以 Phase 1 加進第二段的時候，
    // 這些呼叫點一個都不用改，它們本來就只該管主選區（比如狀態行上的「行 1, 列 1」）。
    // 要作用在**全部**選區上的那些（移動、編輯），Phase 1 會換成別的入口。

    /// 主選區的 head。
    pub fn head(&self) -> usize {
        self.primary().head
    }

    /// 主選區的 anchor。
    pub fn anchor(&self) -> usize {
        self.primary().anchor
    }

    /// 挪主選區的 head，anchor 不動——「撐開」。
    pub fn set_head(&mut self, at: usize) {
        self.primary_mut().head = at;
    }

    /// 挪主選區的 anchor。
    pub fn set_anchor(&mut self, at: usize) {
        self.primary_mut().anchor = at;
    }

    /// 兩端一起放到 `at`——「塌成一個光標」。
    pub fn collapse_to(&mut self, at: usize) {
        *self.primary_mut() = Range::at(at);
    }

    /// 主選區的兩端對調（`A-;`）。
    pub fn flip(&mut self) {
        let it = self.primary_mut();
        std::mem::swap(&mut it.anchor, &mut it.head);
    }
}
