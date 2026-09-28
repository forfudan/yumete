//! **選區——複數的那一個**（#405，方案在 `docs/development.md §5.13`）。
//!
//! 語義逐條照 helix（`helix-core/src/selection.rs`，基準 commit `079a789e8`），只抄語義，
//! 不依賴它那個 crate——理由在 §5.13.5：`helix-core` 會拖進 `helix-loader`、tree-sitter，
//! 以及一個**釘死在舊版上的 `unicode-width`**，而這個倉有一條規矩是「一個概念一處權威：
//! 字素、寬度、分詞」，寬度那一處是 `yumete-cjk`。
//!
//! # 進度
//!
//! **Phase 0**（做完了）把單數包起來：`Editor` 的 `cursor`／`anchor` 兩個欄位換成這裏一
//! 個 `Selections`，行為一字不差——驗收條件是 `scripts/frames.sh` 那二十幀逐字節不變。
//!
//! **Phase 1**（在做）讓它真的裝得下多段：[`Selections::normalize`] 那一套不變式、造出
//! 第二段的鍵、移動與編輯作用在每一段上。
//!
//! ⚠️ **`goal_column` 和 `goal_slot` 都搬進 [`Range`] 了**（2026-09-28，那個「真的需要
//! 各有一份」的時候到了：N 個光標一起按 `j`／`h`，得各記各的目標）。兩個量兩格——一個是
//! 橫排的顯示列，一個是竪排的槽位，共用一格的話換一次版面就會拿列當槽位用。
//!
//! **`zong_motion`、`extend`、`vim_lines` 還在 `Editor` 上**，只有一段的時候行為一樣，
//! 等同一個理由出現再搬。

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
    /// **`j`／`k` 瞄準的那一列**，`None` 是「還不知道，用時現算」。
    ///
    /// ⚠️ **每一段各記一份，不是整個編輯器一份**（2026-09-28）。從前它是 `Editor` 上的
    /// 一個 `goal_column`，只有一個光標的時候那和放在這裏一模一樣；N 段一起按 `j` 就不
    /// 是了——N 段會一起瞄準主選區那一列。helix 也把它放在 `Range` 裏
    /// （`old_visual_position`）。
    ///
    /// ⚠️ **它進 `PartialEq`**，所以兩段兩端相同而目標列不同的選區不相等。`normalize`
    /// 靠值把主選區認回來，而排序不改值、合併會把合出來的那一段整個交回去，所以認得住。
    pub goal: Option<usize>,
    /// **竪排那一半的同一件事**：`h`／`l` 跨縱的時候瞄準第幾格。
    ///
    /// ⚠️ **和 [`Range::goal`] 是兩個量，所以是兩格。** 一個是橫排的顯示列，一個是竪排
    /// 的槽位；共用一格的話換一次版面就會拿列當槽位用。
    pub goal_slot: Option<usize>,
}

impl Range {
    /// 一段塌在 `at` 上的選區——也就是一個光標。
    pub fn at(at: usize) -> Range {
        Range { anchor: at, head: at, goal: None, goal_slot: None }
    }

    /// 兩端給定的一段，目標列還不知道。
    pub fn new(anchor: usize, head: usize) -> Range {
        Range { anchor, head, goal: None, goal_slot: None }
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
    /// ⚠️ **永遠至少一段。** 沒有「沒有光標」這個狀態。排好序、互不重疊，由
    /// [`Selections::normalize`] 保證。
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

    /// 一組只有這一段的選區。
    pub fn one(one: Range) -> Selections {
        Selections { ranges: vec![one], primary: 0 }
    }

    /// 從一串重新裝一組，`primary` 說哪一段是主的。⚠️ **裝完就整理**（排序、合併），
    /// 回傳併掉了幾段。`ranges` 空着的話退回一段塌在 0 上的——沒有「沒有光標」這個狀態。
    pub fn rebuild(&mut self, ranges: Vec<Range>, primary: usize) -> usize {
        if ranges.is_empty() {
            *self = Selections::at(0);
            return 0;
        }
        self.primary = primary.min(ranges.len() - 1);
        self.ranges = ranges;
        self.normalize()
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

    /// 主選區以外的那幾段——畫面要把它們和主選區分開對待，所以單開一支。
    pub fn secondaries(&self) -> impl Iterator<Item = &Range> {
        let primary = self.primary;
        self.ranges
            .iter()
            .enumerate()
            .filter(move |(which, _)| *which != primary)
            .map(|(_, one)| one)
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

    /// 主選區瞄準的那一列。
    pub fn goal(&self) -> Option<usize> {
        self.primary().goal
    }

    /// 記下主選區瞄準的那一列，`None` 是「忘掉，下次現算」。
    pub fn set_goal(&mut self, goal: Option<usize>) {
        self.primary_mut().goal = goal;
    }

    /// 主選區在竪排下瞄準的那一格。
    pub fn goal_slot(&self) -> Option<usize> {
        self.primary().goal_slot
    }

    /// 記下它。
    pub fn set_goal_slot(&mut self, goal: Option<usize>) {
        self.primary_mut().goal_slot = goal;
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

    // ---- 複數 -----------------------------------------------------------

    /// 加一段，然後**整理**（見 [`Selections::normalize`]）。回傳被併掉幾段。
    ///
    /// 新加的那一段成為主選區——`C` 往下複製的時候，讀者的注意力就在新長出來的那一個上。
    pub fn push(&mut self, one: Range) -> usize {
        self.ranges.push(one);
        self.primary = self.ranges.len() - 1;
        self.normalize()
    }

    /// 主選區是第幾段（從 0 數）。
    pub fn primary_index(&self) -> usize {
        self.primary
    }

    /// **換一段當主選區**，往後或者往前，到頭繞回去。⚠️ 一段都不動。
    pub fn turn(&mut self, forward: bool) {
        let count = self.ranges.len();
        if count < 2 {
            return;
        }
        self.primary = match forward {
            true => (self.primary + 1) % count,
            false => (self.primary + count - 1) % count,
        };
    }

    /// 只留主選區（`,`）。回傳去掉了幾段。
    pub fn keep_primary(&mut self) -> usize {
        let gone = self.ranges.len() - 1;
        let it = self.primary();
        self.ranges.clear();
        self.ranges.push(it);
        self.primary = 0;
        gone
    }

    /// 把每一段各自換一個樣子。⚠️ 換完會整理，所以**段數可能變少**。
    pub fn map(&mut self, mut f: impl FnMut(Range) -> Range) -> usize {
        for one in &mut self.ranges {
            *one = f(*one);
        }
        self.normalize()
    }

    /// **排好序、併掉疊在一起的。** 回傳併掉了幾段。
    ///
    /// 照 helix 的 `normalize`（`helix-core/src/selection.rs:557`）：按起點排序、合併重疊、
    /// 重新找到主選區的新下標。
    ///
    /// ⚠️ **兩段「相鄰」不算重疊。** 區間是左閉右開的，所以 `[0,5)` 和 `[5,9)` 各歸各的；
    /// 而一個**塌着的**光標（零寬）和另一段的左邊界重合**算**重疊——這是 helix 專門為零寬
    /// 加的一條（它的 `overlaps` 第一項就是 `from() == other.from()`），不然兩個光標停在
    /// 同一個地方會變成兩個。
    ///
    /// ⚠️ **合併是靜默地少掉一段**，所以呼叫方拿到的這個回傳值是要說給讀者聽的
    /// （2026-09-28 定：命令行 murmur 三秒）。helix 不說，而中文更常撞上——打一個字要按
    /// 好幾下，相鄰的兩個光標很容易在中途撞到一起。
    pub fn normalize(&mut self) -> usize {
        if self.ranges.len() < 2 {
            return 0;
        }
        let was = self.ranges.len();
        // 主選區靠**位置**認回來，不靠下標——排序會把下標打亂。
        let primary = self.primary();
        let mut sorted: Vec<Range> = std::mem::take(&mut self.ranges);
        sorted.sort_by_key(|r| r.span());
        let mut out: Vec<Range> = Vec::with_capacity(sorted.len());
        for one in sorted {
            match out.last_mut() {
                Some(prev) if overlaps(*prev, one) => {
                    // 排過序了，所以 `prev` 的起點一定不在 `one` 後面——併起來就是
                    // 「`prev` 的起點，到兩者較遠的那個終點」。
                    let lo = prev.span().0;
                    let hi = prev.span().1.max(one.span().1);
                    // **方向跟着先來的那一個。** 方向是要留的信息（`;` 塌向哪一端看它），
                    // 而先來的那一段是讀者先造出來的。
                    // 目標列也跟着先來的那一個，理由同方向：它是讀者先造出來的那一段。
                    let (goal, goal_slot) = (prev.goal, prev.goal_slot);
                    *prev = match prev.anchor <= prev.head {
                        true => Range { anchor: lo, head: hi, goal, goal_slot },
                        false => Range { anchor: hi, head: lo, goal, goal_slot },
                    };
                }
                _ => out.push(one),
            }
        }
        // 主選區：原來那一段還在就用它，被併掉了就用蓋住它的那一段。
        self.primary = out
            .iter()
            .position(|r| *r == primary)
            .or_else(|| out.iter().position(|r| overlaps(*r, primary)))
            .unwrap_or(0);
        self.ranges = out;
        was - self.ranges.len()
    }
}

/// 兩段疊在一起沒有。
///
/// ⚠️ 第一項 `from == from` 是專門給**零寬**那一種的：`[5,5)` 和 `[5,9)` 算疊着，而
/// `[0,5)` 和 `[5,9)` 不算。照搬「區間相交」的標準定義會在「光標剛好停在某段起點」時
/// 行為不同（helix 的坑 2）。
fn overlaps(a: Range, b: Range) -> bool {
    let (a0, a1) = a.span();
    let (b0, b1) = b.span();
    a0 == b0 || (a1 > b0 && b1 > a0)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn at(a: usize, h: usize) -> Range {
        Range::new(a, h)
    }

    /// **不變式：排好序、不重疊、至少一段。**
    #[test]
    fn normalize_sorts_and_merges_and_never_empties() {
        let mut s = Selections::at(10);
        s.push(at(0, 3));
        s.push(at(20, 25));
        assert_eq!(s.len(), 3);
        // 排好序了——照加進去的次序是 10、0、20。
        let spans: Vec<(usize, usize)> = s.iter().map(|r| r.span()).collect();
        assert_eq!(spans, vec![(0, 3), (10, 10), (20, 25)], "按起點排好");

        // 疊上去的那一段被併掉。
        let merged = s.push(at(1, 12));
        assert_eq!(merged, 2, "它同時蓋住了 [0,3) 和那個塌着的 10");
        assert_eq!(s.len(), 2, "而 [20,25) 沒被蓋住，還在");
        assert_eq!(s.primary().span(), (0, 12));

        // 只留主選區之後還是至少一段。
        s.push(at(30, 31));
        assert_eq!(s.len(), 3);
        assert_eq!(s.keep_primary(), 2, "去掉了兩段");
        assert_eq!(s.len(), 1, "永遠至少一段");
    }

    /// ⚠️ **相鄰不算疊，而零寬碰到邊界算。**
    ///
    /// 區間是左閉右開的，所以 `[0,5)` 和 `[5,9)` 各歸各的——兩段挨着的整行選區不該被併成
    /// 一段。而一個塌着的光標停在另一段的起點上**算**疊着（helix 專門為零寬加的一條），
    /// 不然兩個光標停在同一個地方會變成兩個。
    #[test]
    fn touching_is_not_overlapping_but_a_caret_on_an_edge_is() {
        let mut s = Selections::at(0);
        s.map(|_| at(0, 5));
        s.push(at(5, 9));
        assert_eq!(s.len(), 2, "相鄰的兩段各歸各的");

        let mut s = Selections::at(5);
        s.push(at(5, 9));
        assert_eq!(s.len(), 1, "塌在起點上的光標被併進去了");

        let mut s = Selections::at(7);
        s.push(at(7, 7));
        assert_eq!(s.len(), 1, "同一個地方的兩個光標是一個");
    }

    /// **主選區不會在整理之後丟。**
    ///
    /// 排序會把下標打亂，所以它是靠位置認回來的；而被併掉的時候，認蓋住它的那一段。
    #[test]
    fn the_primary_survives_being_sorted_and_merged() {
        let mut s = Selections::at(50);
        s.push(at(10, 12));
        s.push(at(30, 33));
        // 最後加進去的是主選區。
        assert_eq!(s.primary().span(), (30, 33));
        s.map(|r| r);
        assert_eq!(s.primary().span(), (30, 33), "整理過還是它");

        // 一段大的把它蓋住——主選區跟着變成那一段。
        s.push(at(0, 100));
        assert_eq!(s.len(), 1);
        assert_eq!(s.primary().span(), (0, 100), "被併掉就認蓋住它的那一段");
    }

    /// **方向是要留的信息。** 併起來的那一段，方向跟着先來的那一個。
    #[test]
    fn a_merged_range_keeps_the_direction_of_the_one_that_came_first() {
        let mut s = Selections::at(0);
        s.map(|_| at(9, 2)); // 反向：head 在前
        s.push(at(5, 15));
        assert_eq!(s.len(), 1);
        let it = s.primary();
        assert_eq!(it.span(), (2, 15));
        assert!(it.head < it.anchor, "先來的那一段是反向的，併完還是：{it:?}");
    }
}
