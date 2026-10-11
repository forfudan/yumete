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
//! 代價是一個子進程。所以**它一趟都不許落在畫面那條路上**：只在開檔、存檔，
//! 和 `:view-diff on` 那一刻算一次，答案按 buffer 存着（[`crate::editor::Editor`]
//! 那一份快取連着算它時的 revision，revision 沒動就不再喊第二次）。
//!
//! ## 跟誰比
//!
//! `HEAD`，不是索引。要問的是「這一章這次坐下來動了哪裏」，而 `git add` 過
//! 的段落照樣是這次動過的。helix 也是跟 HEAD 的那一份 blob 比。
//!
//! Warning: **算的是磁碟上那一份**，所以還沒存的改動要到下一次 `:w` 纔進來。買到的是
//! 「一鍵一個子進程」永遠不會發生。buffer 跟磁碟之間的逐行差是 #298 的第二步，
//! 另算。

use std::ops::Range;
use std::path::{Path, PathBuf};

/// 一段行的來歷。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Change {
    /// 這幾行是新添的。
    #[default]
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
    runs: Vec<Run>,
}

/// 一段改動：它佔新檔的哪幾行、是哪一種、以及 `HEAD` 那一份原來寫的是什麼。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Run {
    /// 新檔這一側的行（0 起算）。剪口那兩種佔一行，是記號不是內容。
    pub at: Range<usize>,
    pub how: Change,
    /// **改之前這裏寫的是什麼**（2026-10-11）。
    ///
    /// 從前 `parse` 只讀 `@@` 那一行、把 `-` 那幾行扔了——畫行號槽只要知道「這幾
    /// 行動過」，而「改之前長什麼樣」要的正是被扔掉的那一半。純添的那一段是空的
    /// （本來就沒有舊文本）。
    pub was: Vec<String>,
}

impl Changes {
    /// 第 `line` 行（0 起算）的來歷，沒有就是沒動過。
    pub fn at(&self, line: usize) -> Option<Change> {
        self.run_at(line).map(|run| run.how)
    }

    /// **這一行屬於哪一段改動**，連它改之前寫的是什麼一起（2026-10-11）。
    pub fn run_at(&self, line: usize) -> Option<&Run> {
        // 區間互不重疊且按起點排好，所以「起點 ≤ line 的最後一條」是唯一可能命中的。
        let i = self.runs.partition_point(|run| run.at.start <= line);
        let run = self.runs.get(i.checked_sub(1)?)?;
        run.at.contains(&line).then_some(run)
    }

    /// **下一處/上一處改動的起點行**（`]g`/`[g`，2026-10-06）。
    ///
    /// 走的是**段**不是行：`runs` 本來就是按起點排好、互不重疊的段，所以一段五行
    /// 的改動只算一處，按五下不會在它裏面爬。
    ///
    /// 到頭**繞回去**，同 `g/`（「這個詞還在哪裏」那一支）。helix 的 `]g` 到頭就
    /// 不動也不說話；這個倉的規矩是「一個不說話的鍵看起來就是壞的」，而繞回去
    /// 既不用說話也不會讓人以為按壞了。
    pub fn hunk_from(&self, line: usize, forward: bool) -> Option<usize> {
        let starts = || self.runs.iter().map(|run| run.at.start);
        match forward {
            true => starts().find(|&s| s > line).or_else(|| starts().next()),
            false => starts().rfind(|&s| s < line).or_else(|| starts().next_back()),
        }
    }

    /// 一條都沒有——沒進 git、沒動過，或者 git 不肯回答，這三件事這裏不分。
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// **`HEAD` 裏那一份的正文**，拿來跟緩衝區比（2026-09-19）。
    ///
    /// 從前比的是**磁碟上那一份**，於是緩衝區裏剛打的行根本不在那次比較裏：在
    /// 一段上面插一行，記號還釘在原來的行號上，看着就是「下面那一段的竪綫不見
    /// 了」。helix 比的是緩衝區，這裏跟它一樣。
    ///
    /// Warning: **兩邊都把行尾的 `\r` 去掉**。倉裏開着 `core.autocrlf` 的時候，`HEAD`
    /// 裏存的是 CRLF 而緩衝區是 LF——逐字節比會說**每一行都改過**，整篇亮起來。
    /// 這正是 helix 在同一個檔案上掉進去的坑（`git diff` 自己不會，因為它兩邊都
    /// 過一遍 git 的換行轉換；`--no-index` 不過）。
    /// **`HEAD` 此刻指着哪一個提交**——不喊 git，只讀兩個檔（2026-10-11 報的）。
    ///
    /// 提交之後檔案一個字節都沒變，變的是 `HEAD`，所以按檔的 mtime 或者 buffer 的
    /// `revision` 判「要不要重算」的那一套看不見它：行號槽上的改動條一直是提交前
    /// 的樣子，直到人手動 `:reload`。原話：「the problem is not about
    /// autoreloading the file per se as the file is the same. It is about the
    /// detection of the git HEAD.」
    ///
    /// 答的是一個**指紋**，不是提交號：`.git/HEAD` 的內容（分支名，或者 detached
    /// 時就是提交號）加上它指的那個 ref 檔的內容。切分支、提交、rebase、reset
    /// 都動得了這兩者之一。
    ///
    /// Warning: **一個子進程都不生。** 這一支要在每一次停手時問，而 `git rev-parse`
    /// 一次十幾毫秒；讀兩個小檔是微秒級的。倉在哪由 `.git` 往上找，和
    /// `book_root` 同一個走法。
    pub fn head_mark(path: &Path) -> Option<String> {
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let git = dir.ancestors().map(|d| d.join(".git")).find(|d| d.exists())?;
        // 工作樹（`git worktree`）的 `.git` 是一個檔，寫着真正的 git 目錄在哪。
        let git = match git.is_dir() {
            true => git,
            false => {
                let said = std::fs::read_to_string(&git).ok()?;
                let at = said.strip_prefix("gitdir:")?.trim();
                PathBuf::from(at)
            }
        };
        let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
        let head = head.trim().to_string();
        // `ref: refs/heads/main` —— 再讀那個 ref 纔知道它此刻是哪一個提交。
        // 讀不到不要緊（packed-refs 裏的分支就讀不到），那時候分支名本身還在，
        // 切分支照樣看得出來；同一條分支上的提交由 `logs/HEAD` 兜住。
        let Some(at) = head.strip_prefix("ref:").map(str::trim) else {
            return Some(head);
        };
        let now = std::fs::read_to_string(git.join(at)).unwrap_or_default();
        // reflog 每一次 `HEAD` 動過都追加一行，所以它的長度也是指紋的一部分——
        // packed-refs 底下那個 ref 檔不存在，光靠分支名看不出提交。
        let logged = std::fs::metadata(git.join("logs/HEAD")).map(|m| m.len()).unwrap_or(0);
        Some(format!("{head}\u{1}{}\u{1}{logged}", now.trim()))
    }

    pub fn head_text(path: &Path) -> Option<String> {
        Changes::text_at(path, "HEAD").ok()
    }

    /// **同一件事，可以問別的提交**——`:git-diff HEAD~3`、`:git-diff v1.0`。
    ///
    /// `Err` 帶着 git 自己那一句話，因為問不出答案有好幾種原因（不在倉裏、這個
    /// 檔還沒提交過、沒有那個提交、機器上沒有 git），而它們要的處置各不相同。
    /// 「什麼都不畫」的那一條路（行號旁的改動條）把它丟掉就是；**開口問的那一條
    /// 路（`:git-diff`）要把它說出來**。
    ///
    /// Warning: **`<提交>:./<檔名>` 那個 `./`** 讓 git 拿**當前目錄**去解析路徑，於是
    /// 呼叫方不必先算出倉根在哪。`current_dir` 是檔案自己那一層——編輯器開着兩個
    /// 倉裏的檔是常事，呼叫者的 cwd 說了不算。
    pub fn text_at(path: &Path, commit: &str) -> Result<String, String> {
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let Some(name) = path.file_name() else {
            return Err(path.display().to_string());
        };
        let mut arg = std::ffi::OsString::from(commit);
        arg.push(":./");
        arg.push(name);
        let out = std::process::Command::new("git")
            .args(["--no-optional-locks", "show"])
            .arg(&arg)
            .current_dir(dir)
            .output()
            .map_err(|e| e.to_string())?;
        match out.status.success() {
            true => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
            // git 的抱怨自己就是最好的說明（「exists on disk, but not in
            // 'HEAD'」「not a git repository」），照搬，別另編一句。
            false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        }
    }

    /// `HEAD` 那一份與**這一份正文**的逐行差。
    ///
    /// `base` 是 [`Changes::head_text`] 取來的那一份（按 buffer 存住，不必每次
    /// 重取）；`text` 是緩衝區現在的樣子。
    pub fn against(base: &str, text: &str, lines: usize) -> Option<Changes> {
        let strip = |s: &str| {
            s.split('\n').map(|l| l.trim_end_matches('\r')).collect::<Vec<_>>().join("\n")
        };
        let dir = std::env::temp_dir().join(format!("yumete-vcs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let (a, b) = (dir.join("head"), dir.join("now"));
        std::fs::write(&a, strip(base)).ok()?;
        std::fs::write(&b, strip(text)).ok()?;
        let out = std::process::Command::new("git")
            .args(["--no-optional-locks", "diff", "--no-color", "--no-ext-diff", "-U0", "--no-index", "--"])
            .arg(&a)
            .arg(&b)
            .output()
            .ok();
        let _ = std::fs::remove_dir_all(&dir);
        let out = out?;
        // Warning: `--no-index` 退出碼 **1 ＝ 兩邊不一樣**，不是錯。只有 2 以上纔是。
        if out.status.code().is_some_and(|c| c > 1) {
            return None;
        }
        Some(Changes::from_diff(&String::from_utf8_lossy(&out.stdout), lines))
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
    let mut runs: Vec<Run> = Vec::new();
    // 一段一段地收 `-` 那幾行，和 `runs` 同一個下標。
    let mut was: Vec<Vec<String>> = Vec::new();
    let mut taken: Vec<String> = Vec::new();
    let mut seen_a_header = false;
    for row in diff.lines() {
        if !row.starts_with("@@ ") {
            // Warning: **`---` 是檔頭不是刪掉的行。** `-U0` 的輸出頭上有
            // `--- a/…`，照收就會把它當成原文的第一行。
            if seen_a_header && row.starts_with('-') && !row.starts_with("---") {
                taken.push(row[1..].to_string());
            }
            continue;
        }
        let header = row;
        if seen_a_header {
            was.push(std::mem::take(&mut taken));
        }
        seen_a_header = true;
        let Some((old_n, new_at, new_n)) = hunk(header) else {
            // 這一條讀不懂，它收的那幾行也不要——下一條 header 來的時候
            // `was` 會比 `runs` 多一格，所以這裏就把它對齊掉。
            taken.clear();
            seen_a_header = false;
            continue;
        };
        // 1 起算 → 0 起算。`+0,n` 不合規矩，但飽和減一句比一個 panic 便宜。
        let from = new_at.saturating_sub(1);
        match (old_n, new_n) {
            // 一行沒去、一行沒來：git 不會寫出這種，寫得出也沒話說。
            (0, 0) => {}
            // 純添。`+c,d` 的 c 是 1 起算的第一條新行。
            (0, _) => runs.push(Run { at: from..from + new_n, how: Change::Added, was: Vec::new() }),
            // 純刪。Warning: **`+c,0` 的 c 是缺口上面那一行**（1 起算），所以 0 起算
            // 的 `c` 正好是缺口**下面**那一行——剪口畫在它頭上。刪在檔頭時
            // c ＝ 0，0 起算的 0 就是第一行，同一條式子。
            (_, 0) => match new_at >= lines {
                // 刪在檔尾：下面沒有行了，剪口改記在最後一行的腳下。
                true => {
                    if lines > 0 {
                        runs.push(Run { at: lines - 1..lines, how: Change::CutBelow, was: Vec::new() });
                    }
                }
                false => runs.push(Run { at: new_at..new_at + 1, how: Change::CutAbove, was: Vec::new() }),
            },
            _ => runs.push(Run { at: from..from + new_n, how: Change::Changed, was: Vec::new() }),
        }
    }
    // `-U0` 的 hunk 在新檔那一側本來就互不重疊——git 把貼着的改動併成一條。
    // 這兩句是保險：剪口那一條是算出來的，算錯了寧可少畫一格，不許畫出一條
    // 起點倒退的區間讓 `at` 的二分查找答出鬼話。
    // 收尾：最後一條的 `-` 行還在手上。
    if seen_a_header {
        was.push(std::mem::take(&mut taken));
    }
    // **舊文本按它那一條的下標配回去**，要在排序之前——排完下標就不是原來那個了。
    for (run, old) in runs.iter_mut().zip(was) {
        run.was = old;
    }
    runs.sort_by_key(|run| (run.at.start, run.how.is_cut()));
    let mut end = 0usize;
    runs.retain(|run| {
        let keep = run.at.start >= end;
        if keep {
            end = run.at.end;
        }
        keep
    });
    Changes { runs }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **改之前寫的是什麼，也要留着**（2026-10-11）。
    ///
    /// `-U0` 的 `-` 那幾行從前整個扔了，而「看這一塊改前長什麼樣」要的正是它們。
    #[test]
    fn a_change_remembers_what_the_line_used_to_say() {
        let c = parse(
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-老的那一行\n+新的那一行\n@@ -5,2 +6,0 @@\n-甲\n-乙\n",
            9,
        );
        let changed = c.run_at(0).expect("第一行改過");
        assert_eq!(changed.how, Change::Changed);
        assert_eq!(changed.was, ["老的那一行"], "改之前那一行");
        // 剪掉的那兩行記在剪口上。
        let cut = c.run_at(5).or_else(|| c.run_at(6)).expect("剪口在");
        assert!(cut.how.is_cut());
        assert_eq!(cut.was, ["甲", "乙"], "剪掉的是這兩行");
        // Warning: `--- a/x` 是檔頭，不是刪掉的行。
        assert!(
            c.runs.iter().all(|run| !run.was.iter().any(|line| line.starts_with("-- a/"))),
            "檔頭混進原文了"
        );
    }

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
