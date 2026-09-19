//! **哪幾行跟 git 那一份不一樣**（#55，#298 的第三步）。
//!
//! 行號旁邊那一格從一開始就是留給改動條的（`GUTTER_AIR`，「這個位置現在先空着，
//! 免得功能落地那天整頁的版心挪一格」）。這個模組答的就是那一格要塗什麼色：
//! 每一行是**新添**、**改過**，還是它上面**剪掉**了幾行。
//!
//! ## 為什麼是喊 `git`，不是引一個 git 庫
//!
//! `gix` 與 `git2` 都是一整條供應鏈——`git2` 底下是 libgit2 的 C 代碼，`gix` 是
//! 幾十個 crate。這個編輯器現在的依賴一隻手數得完（`docs/development.md` §6442
//! 為了「自動更新」拒過同一件事：「不要為它引一條網絡供應鏈……它要做的事 `curl`
//! 和 `unzip` 已經會了」）。這裏要的東西比那個還小：**一串 `@@ -a,b +c,d @@`**。
//! 而且喊 `git` 有一個庫給不了的好處——使用者的 `.gitattributes`、`core.autocrlf`、
//! 子模組、worktree、`GIT_DIR`，全是 git 自己的答案，抄一份只會抄出第二種答案。
//!
//! 代價是一個子行程。所以**它一趟都不許落在畫面那條路上**：只在開檔、存檔，
//! 和 `:view-diff on` 那一刻算一次，答案按 buffer 存着（[`crate::editor::Editor`]
//! 那一份快取連着算它時的 revision，revision 沒動就不再喊第二次）。
//!
//! ## 跟誰比
//!
//! `HEAD`，不是索引。作者要問的是「這一章這次坐下來動了哪裏」，而 `git add` 過
//! 的段落照樣是這次動過的。helix 也是跟 HEAD 的那一份 blob 比。
//!
//! ⚠️ **算的是磁碟上那一份**，所以還沒存的改動要到下一次 `:w` 纔進來。買到的是
//! 「一鍵一個子行程」永遠不會發生。buffer 跟磁碟之間的逐行差是 #298 的第二步，
//! 另算。

use std::ops::Range;
use std::path::Path;

/// 一段行的來歷。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// 這幾行是新添的。
    Added,
    /// 這幾行改過。
    Changed,
    /// 這一行**上面**剪掉了幾行——剪口在它和上一行之間。
    CutAbove,
    /// 剪口在檔尾：最後一行**下面**剪掉了幾行。
    CutBelow,
}

impl Change {
    /// 剪口沒有自己的行，畫法跟另外兩種不一樣。
    pub fn is_cut(self) -> bool {
        matches!(self, Change::CutAbove | Change::CutBelow)
    }
}

/// 一個檔案跟 `HEAD` 那一份的逐行差。
///
/// 存的是**區間**不是逐行：新開一個檔是一條 `@@ -0,0 +1,20000 @@`，逐行存就是兩萬
/// 條記錄，而它本來只是一句話。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// 按起點排好、互不重疊。
    runs: Vec<(Range<usize>, Change)>,
}

impl Changes {
    /// 第 `line` 行（0 起算）的來歷，沒有就是沒動過。
    pub fn at(&self, line: usize) -> Option<Change> {
        // 區間互不重疊且按起點排好，所以「起點 ≤ line 的最後一條」是唯一可能命中的。
        let i = self.runs.partition_point(|(r, _)| r.start <= line);
        let (range, change) = self.runs.get(i.checked_sub(1)?)?;
        range.contains(&line).then_some(*change)
    }

    /// 一條都沒有——沒進 git、沒動過，或者 git 不肯回答，這三件事這裏不分。
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// 喊一次 `git`，把 `path` 跟 `HEAD` 那一份比出來。
    ///
    /// `lines` 是這個檔案現在有幾行，只用來認「剪口在檔尾」那一種。
    ///
    /// `None` ＝ 這個問題問不出答案：不在 git 倉裏、這個倉還沒有第一個提交、
    /// 機器上沒有 git。三種都不是錯，畫面上都是「什麼都不畫」。
    pub fn read(path: &Path, lines: usize) -> Option<Changes> {
        // `git` 認的是「我在哪個目錄」，不是呼叫者的 cwd——編輯器開着兩個倉裏的
        // 檔案是常事。檔名自己單獨傳，免得一個叫 `-x` 的檔被當成旗標。
        let here = Path::new(".");
        let dir = match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => here,
        };
        let out = std::process::Command::new("git")
            .args([
                // 只是讀，別為它去碰 index 的鎖：另一個 git 正在跑的時候這一句
                // 是「不要跟它搶」。
                "--no-optional-locks",
                "diff",
                "--no-color",
                // 使用者自己配的 difftool 吐的不是 unified diff。
                "--no-ext-diff",
                // 要的只有 `@@` 那一行，一行上下文都不要。
                "-U0",
                "HEAD",
                "--",
            ])
            .arg(path)
            .current_dir(dir)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(Changes::from_diff(&String::from_utf8_lossy(&out.stdout), lines))
    }

    /// `git diff -U0` 的輸出 → 逐行的來歷。
    ///
    /// 跟 [`Changes::read`] 分開，因為這一半**不需要磁碟上有一個 git 倉**，而真
    /// 正會算錯的地方（`+c,0` 那個 `c` 指的是缺口**上面**那一行）全在這一半。
    /// 它也是「另一個來源」進來的門：跟磁碟上那一份比（#298 的第二步）吐的同樣
    /// 是 unified diff，到時候不必為它再開一種結構。
    pub fn from_diff(diff: &str, lines: usize) -> Changes {
        parse(diff, lines)
    }
}

/// 一條 `@@ -a,b +c,d @@` 裏用得着的三個數：舊邊幾行、新邊從第幾行起、幾行。
/// 省掉的 `,b` 照 unified diff 的規矩算 1。
fn hunk(header: &str) -> Option<(usize, usize, usize)> {
    let rest = header.strip_prefix("@@ ")?;
    let (spec, _) = rest.split_once(" @@")?;
    let (old, new) = spec.split_once(' ')?;
    let count = |s: &str, sign: char| -> Option<(usize, usize)> {
        let s = s.strip_prefix(sign)?;
        match s.split_once(',') {
            Some((at, n)) => Some((at.parse().ok()?, n.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (_, old_n) = count(old, '-')?;
    let (new_at, new_n) = count(new, '+')?;
    Some((old_n, new_at, new_n))
}

/// [`Changes::from_diff`] 的本體。
fn parse(diff: &str, lines: usize) -> Changes {
    let mut runs: Vec<(Range<usize>, Change)> = Vec::new();
    for header in diff.lines().filter(|l| l.starts_with("@@ ")) {
        let Some((old_n, new_at, new_n)) = hunk(header) else {
            continue;
        };
        // 1 起算 → 0 起算。`+0,n` 不合規矩，但飽和減一句比一個 panic 便宜。
        let from = new_at.saturating_sub(1);
        match (old_n, new_n) {
            // 一行沒去、一行沒來：git 不會寫出這種，寫得出也沒話說。
            (0, 0) => {}
            // 純添。`+c,d` 的 c 是 1 起算的第一條新行。
            (0, _) => runs.push((from..from + new_n, Change::Added)),
            // 純刪。⚠️ **`+c,0` 的 c 是缺口上面那一行**（1 起算），所以 0 起算
            // 的 `c` 正好是缺口**下面**那一行——剪口畫在它頭上。刪在檔頭時
            // c ＝ 0，0 起算的 0 就是第一行，同一條式子。
            (_, 0) => match new_at >= lines {
                // 刪在檔尾：下面沒有行了，剪口改記在最後一行的腳下。
                true => {
                    if lines > 0 {
                        runs.push((lines - 1..lines, Change::CutBelow));
                    }
                }
                false => runs.push((new_at..new_at + 1, Change::CutAbove)),
            },
            _ => runs.push((from..from + new_n, Change::Changed)),
        }
    }
    // `-U0` 的 hunk 在新檔那一側本來就互不重疊——git 把貼着的改動併成一條。
    // 這兩句是保險：剪口那一條是算出來的，算錯了寧可少畫一格，不許畫出一條
    // 起點倒退的區間讓 `at` 的二分查找答出鬼話。
    runs.sort_by_key(|(r, c)| (r.start, c.is_cut()));
    let mut end = 0usize;
    runs.retain(|(r, _)| {
        let keep = r.start >= end;
        if keep {
            end = r.end;
        }
        keep
    });
    Changes { runs }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pure_insertion_is_added() {
        // 第 3 行（1 起算）起新添兩行。
        let c = parse("@@ -2,0 +3,2 @@\n+甲\n+乙\n", 10);
        assert_eq!(c.at(1), None);
        assert_eq!(c.at(2), Some(Change::Added));
        assert_eq!(c.at(3), Some(Change::Added));
        assert_eq!(c.at(4), None);
    }

    #[test]
    fn a_hunk_with_no_comma_counts_one_line() {
        let c = parse("@@ -5 +5 @@\n-舊\n+新\n", 10);
        assert_eq!(c.at(3), None);
        assert_eq!(c.at(4), Some(Change::Changed));
        assert_eq!(c.at(5), None);
    }

    #[test]
    fn a_pure_deletion_marks_the_line_below_the_gap() {
        // 舊檔的第 5 行沒了；`+4,0` 的 4 是缺口**上面**那一行。
        let c = parse("@@ -5,1 +4,0 @@\n-沒了\n", 10);
        assert_eq!(c.at(3), None, "缺口上面那一行自己沒動過");
        assert_eq!(c.at(4), Some(Change::CutAbove));
        assert_eq!(c.at(5), None);
    }

    #[test]
    fn a_deletion_at_the_top_marks_the_first_line() {
        let c = parse("@@ -1,2 +0,0 @@\n-甲\n-乙\n", 10);
        assert_eq!(c.at(0), Some(Change::CutAbove));
        assert_eq!(c.at(1), None);
    }

    #[test]
    fn a_deletion_at_the_foot_marks_under_the_last_line() {
        // 十行的檔，尾巴上剪掉三行：缺口下面沒有行了。
        let c = parse("@@ -11,3 +10,0 @@\n-甲\n-乙\n-丙\n", 10);
        assert_eq!(c.at(9), Some(Change::CutBelow));
        assert_eq!(c.at(8), None);
    }

    #[test]
    fn several_hunks_keep_their_own_lines() {
        let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n\
                    @@ -1,0 +1,1 @@\n+新\n\
                    @@ -8,2 +9,2 @@\n-舊\n-舊\n+改\n+改\n";
        let c = parse(diff, 20);
        assert_eq!(c.at(0), Some(Change::Added));
        assert_eq!(c.at(1), None);
        assert_eq!(c.at(8), Some(Change::Changed));
        assert_eq!(c.at(9), Some(Change::Changed));
        assert_eq!(c.at(10), None);
    }

    #[test]
    fn a_whole_new_file_is_one_run_not_twenty_thousand() {
        let c = parse("@@ -0,0 +1,20000 @@\n", 20000);
        assert_eq!(c.runs.len(), 1);
        assert_eq!(c.at(19_999), Some(Change::Added));
    }

    #[test]
    fn rubbish_is_not_a_hunk() {
        assert!(parse("@@ 亂寫 @@\n@@\nhello\n", 10).is_empty());
    }
}
