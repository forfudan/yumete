//! **`gw` — 一眼跳到屏幕上任何地方**（#406）。
//!
//! 按 `gw`，屏幕上每個落腳點的頭一個字被兩個字母蓋住；打那兩個字母，光標飛過去。
//! 永遠兩個字母、兩格，正好是一個漢字的寬度。
//! vim 那邊叫 easymotion／leap／flash，helix 把它做進了核心。
//!
//! 三件定下來的事（2026-09-27）：
//!
//! **一、落腳點是 `e` 的單位，不是每一個詞。** helix 把標籤貼在空白分隔的詞首上，
//! 那是英文的樣子——中文一屏六百個詞，滿屏字母，原文就讀不出來了。原話：「句首的
//! 定义用 e 的那个，也就是说空格、逗号、句号都算」。`e` 走的是 [`Grain::Coarse`]
//! ——「`w` 取一個詞，`e` 取一個小句」——所以「那年冬天，雪下得早。」是兩個落腳點
//! 而不是六個，一屏六十個而不是六百個。
//!
//! **二、標點自成一段的不給標籤。** `e` 把「，」也算一段，可給它一個標籤等於把標籤
//! 數翻一倍，而標點本來就只有一格、兩個字母蓋不下。
//!
//! **三、標籤蓋住那個字，不推開版面。** 插進去的話一行憑空長出幾十格，後面的字全
//! 往右擠、整屏重排——而按 `gw` 之前眼睛已經鎖定了要去的地方，版面一動那個地方就跑
//! 了，這一跳就白跳。Warning: **代價是英文下會糊**：`was` 剩一個 `s`。中文不糊——一個漢字
//! 正好兩格，標籤蓋掉的就是那一個字。
//!
//! # 和 helix 有意不同的一處（2026-09-28 讀它的源碼對出來的）
//!
//! Warning: **蓋一個字素，不是兩個。** `helix-term/src/commands.rs` 的 `jump_to_label`
//! 在 `range.from()` 和 `next_grapheme_boundary(from)` 各放一個 overlay——兩個字母
//! 蓋掉**兩個**字素。西文剛好（兩個字母蓋兩個字母），中文就是拿 2 格蓋掉 4 格，
//! **那一行當場縮短兩格、整段重排**。我們蓋一個：一個漢字正好兩格。
//!
//! **抄過來的三條**：**標籤固定兩個字母**（見 [`LABEL`]）；落腳點上限是字母表長度
//! 的平方（多出來的畫不出來也按不到）；跳之前先記一筆，`C-o` 回得來（helix 的
//! `push_jump`）。
//!
//! Warning: **記一筆，可是不挪版面**（2026-10-04 修）。從前走的是 `remember_jump()`，
//! 而那一支順帶把落腳行挪到屏幕正中——居中是給「跳到看不見的地方」的（`n`、`gd`、
//! 搜索結果），而這裏那個地方**本來就在屏幕上、你正盯着它**，挪走它這一跳就白跳。
//! helix 也不挪（`jump_to_label` 跳完只有 `set_selection`，沒有 `align_view`）。
//! 現在走 `note_where_we_came_from()`。

use super::*;

/// 一個落腳點和它的標籤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jump {
    /// 落腳點在檔裏的字符下標。
    pub at: usize,
    /// 打進去就去那裏的那一兩個字母。
    pub label: String,
}

/// 標籤用的字母。
///
/// **`a`–`z`，不是主鍵位行在前**（2026-09-28 定）。一度寫成
/// `asdfghjklqwertyuiopzxcvbnm`——vim 那邊跳轉插件的排法，理由是前面那幾個落腳點拿
/// 到的是手指不用挪的鍵。原話：「我觉得 a-z 好。」helix 的出廠也是 `('a'..='z')`
/// （`helix-view/src/editor.rs`），它還把這一串做成了配置項。
///
/// 兩種都說得通，而 `a`–`z` 贏在**猜得到**：看見一屏標籤，心裏知道第幾個大概是哪
/// 個字母；`a s d f g h j k l q` 要一個一個認。
const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz";

/// **把一串落腳點發成標籤** —— `gw` 和 `go`／`gu` 共用這一支（2026-10-04）。
///
/// 永遠兩個字母（見 [`LABEL`]），所以兩邊的手指學的是同一件事；字母表也是同一張，
/// 不然同一個位置在兩個鍵下拿到不同的號碼，而讀者記的是「第幾個大概是哪個字母」。
pub(super) fn label_them(spots: impl IntoIterator<Item = usize>) -> Vec<Jump> {
    let letter = |n: usize| ALPHABET[n % ALPHABET.len()] as char;
    let spots: Vec<usize> = spots.into_iter().take(ALPHABET.len().pow(LABEL as u32)).collect();
    // **夠得上就一個字母**（見 [`LABEL`]）。全屏一致：要麼全是一個，要麼全是兩個
    // ——所以不會有「某個單字母標籤正好是某個雙字母標籤的頭一個字母」那種撞法，
    // avy／easymotion 為混長度專門做的那套前綴樹這裏一行都不需要。
    let one = spots.len() <= ALPHABET.len();
    spots
        .into_iter()
        .enumerate()
        .map(|(i, at)| Jump {
            at,
            label: match one {
                true => letter(i).to_string(),
                false => format!("{}{}", letter(i / ALPHABET.len()), letter(i)),
            },
        })
        .collect()
}

/// **一個標籤最多兩個字母**，也就是最多兩格——落腳點不到二十六個就只用一個。
///
/// # 2026-09-28 定過「固定兩個」，2026-10-04 改回可長可短
///
/// 當時的原話：「我觉得就应该固定两个字母，因为很少情况能在 26 个落点内。」**那句話
/// 到今天仍然對，可它說的只是 `gw`**——那時候這個倉裏只有 `gw`。`go`／`gu` 是另一個
/// 分佈。十二屏正文量出來的（`measure::how_many_places_does_one_screen_have`）：
///
/// ```text
///        中位   最大   ≤26 的佔
/// gw      155    239        0%     ← 單字母那一檔一次都輪不到
/// go        1     11      100%     ← 每一次都是單字母
/// gu        2     45       99%
/// ```
///
/// 所以這一改**對 `gw` 等於沒改**（它永遠落在兩個字母那一檔），對 `go`／`gu` 是按鍵數
/// 直接少一半。
///
/// 另一條當時的理由是肌肉記憶——「永遠兩個，手指學會加兩下」。2026-10-04 撤了：
/// 「之前我觉得单字母跳转是怕影响肌肉记忆，现在想来似乎还好，因为是
/// interactive 的，用户不存在肌肉记忆。」標籤本來就得看了才知道打什麼。
///
/// Warning: **沒做大寫那一檔。** 提過「超過 26×26 就動用大寫，湊成 52×52」。量下來
/// 最大 239，離 676 還差一半還多——那會是一段永遠跑不到的代碼，而大寫要按 Shift，
/// 在「越快越好」這件事上是負的。
///
/// Warning: **全屏一致，所以沒有前綴問題。** 要麼全是一個字母、要麼全是兩個，不會出現
/// 「`a` 和 `ab` 同時是標籤」。avy／easymotion 為混長度做的那套前綴樹這裏不需要。
///
/// 塌掉的兩處一併刪了：全角標籤（兩個半角字母本來就正好兩格，一個縱、一個漢字都填
/// 得滿），以及「先按一格數一遍再按兩格數一遍」那個兩趟。
const LABEL: usize = 2;

impl Editor {
    /// **這一頁畫了哪一段**（字符下標），前端每幀交過來一次。
    ///
    /// 同 [`Editor::set_page_top`]：畫的時候纔定得下來，所以到這裏的是上一幀的答案
    /// ——而落腳點只在按下 `gw` 的那一刻算一次，那時候上一幀就是眼前這一幀。
    pub fn set_page_span(&mut self, from: usize, to: usize) {
        self.page_span = (from.min(to), to);
    }

    /// 屏幕上的落腳點，按檔裏的次序。
    fn jump_spots(&self) -> Vec<usize> {
        let rope = self.current_buffer().rope();
        let (from, to) = self.page_span;
        let to = to.min(rope.len_chars());
        if from >= to {
            return Vec::new();
        }
        let seg = self.segmenter.as_ref();
        let mut out = Vec::new();
        for line in rope.char_to_line(from)..=rope.char_to_line(to.saturating_sub(1)) {
            for (a, b) in motion::line_words(rope, line, motion::Grain::Coarse, seg) {
                if a < from || a >= to {
                    continue;
                }
                // **標點自成一段的不給標籤。** 一個字母數字都沒有就是那一種——
                // 漢字在 Unicode 眼裏是 alphanumeric，「，」不是。
                if !rope.slice(a..b).chars().any(char::is_alphanumeric) {
                    continue;
                }
                // Warning: **兩格都蓋不下就不標。** 標籤是蓋在那個單位頭上的，蓋過了頭
                // 就吃掉它後面那個東西——英文的 `a` 只有一格，兩個字母蓋下去連它
                // 後面的空格一起沒了，於是前後兩個詞黏在一起。helix 也跳過短詞
                // （它的說法是「two or more characters」），這裏按**格數**算，因為
                // 中文一個字就有兩格：`a` 不標，而單獨一個「我」標得了。
                let wide: usize = rope
                    .slice(a..b)
                    .chars()
                    .map(yumete_cjk::char_width)
                    .sum();
                if wide < LABEL {
                    continue;
                }
                out.push(a);
            }
        }
        out
    }

    /// **記一筆「欠着一次 `gw`」。** 按鍵那一支叫的是這個。
    ///
    /// Warning: **落腳點只算屏幕上的，而「屏幕上」是畫的那一方纔知道的事**——所以不能
    /// 在按鍵這一刻算：`--shot` 只畫一幀，按鍵跑在那一幀之前，那時候
    /// [`Editor::set_page_span`] 還沒被餵過，落腳點一個都找不到。前端畫完一幀、
    /// 把這一頁的範圍交過來，再回頭把它跑掉（[`Editor::run_owed_jump`]）。
    ///
    /// 同欠着那一趟搜索的辦法（`owed_search`），也同載入碼表那一處。
    pub(super) fn start_jump(&mut self) {
        self.owed_jump = true;
    }

    /// 欠着沒有——問一句，不取走。
    ///
    /// `--shot` 那條路用它：一批鍵是一次餵完的，而 `gw` 之後那一鍵打的是屏幕上發
    /// 的號碼，所以餵到欠着的那一刻要先畫一幀把號碼算出來。
    pub fn owes_a_jump(&self) -> bool {
        self.owed_jump
    }

    /// 前端畫完一幀、交了範圍之後，回頭問一句欠着沒有。
    pub fn take_owed_jump(&mut self) -> bool {
        std::mem::take(&mut self.owed_jump)
    }

    /// 亮起標籤。
    pub fn run_owed_jump(&mut self) {
        let spots = self.jump_spots();
        if spots.is_empty() {
            self.status = say!("jump.nowhere");
            return;
        }
        self.labels = label_them(spots);
        self.jump_typed.clear();
    }

    /// 標籤亮着沒有。
    pub fn jumping(&self) -> bool {
        !self.labels.is_empty()
    }

    /// **還畫得出來的那幾個標籤**，以及每個標籤已經被打掉的字母數。
    ///
    /// 打了一個字母之後只剩以它開頭的那些——別的當場滅掉，屏幕上剩下的就是還能去
    /// 的地方。
    pub fn jump_labels(&self) -> Vec<(usize, &str)> {
        self.labels
            .iter()
            .filter(|j| j.label.starts_with(&self.jump_typed))
            .map(|j| (j.at, &j.label[self.jump_typed.len()..]))
            .collect()
    }

    /// **屏幕上亮着的那些標籤** —— `gw` 與 `go`／`gu` 共用的一問。
    ///
    /// 兩種機制、同一種畫法（兩個字母蓋一個字素），所以畫的那一邊不該問「是哪一
    /// 種」——橫排一處、竪排一處，問兩次就是四處，而它們早晚會分岔。
    pub fn labels_on_the_page(&self) -> Vec<(usize, &str)> {
        match self.jumping() {
            true => self.jump_labels(),
            false => self.seek_labels(),
        }
    }

    /// 有沒有標籤亮着（哪一種都算）。
    pub fn showing_labels(&self) -> bool {
        self.jumping() || !self.seek_labels().is_empty()
    }

    /// 把標籤收掉。
    pub(super) fn cancel_jump(&mut self) {
        self.labels.clear();
        self.jump_typed.clear();
    }

    /// 標籤亮着的時候，這一鍵怎麼算。
    ///
    /// Warning: **不認得的鍵一律收掉標籤並吃掉它自己。** 「按錯一個鍵就跳到別處去」比
    /// 「按錯一個鍵什麼都沒發生」壞得多——而標籤亮着的時候整個鍵盤都是標籤，讀者
    /// 心裏清楚自己在一個臨時的狀態裏。
    pub(super) fn jump_key(&mut self, key: Key) -> bool {
        let Key::Char(c) = key else {
            self.cancel_jump();
            return true;
        };
        if !c.is_ascii_alphabetic() {
            self.cancel_jump();
            return true;
        }
        let mut typed = self.jump_typed.clone();
        typed.push(c.to_ascii_lowercase());
        match self.labels.iter().find(|j| j.label == typed) {
            Some(j) => {
                let at = j.at;
                self.cancel_jump();
                // 記進跳轉表，`C-o` 回得來。
                //
                // Warning: **不用 `remember_jump`——那一支會把落腳行挪到屏幕正中**
                // （2026-10-04 修）。居中是給「跳到看不見的地方」的；這裏那個地方
                // 本來就在屏幕上、你正盯着它，挪走它等於這一跳白跳。helix 也不挪。
                self.note_where_we_came_from();
                self.move_head(at);
            }
            // 還有標籤以它開頭：留着，等下一個字母。
            None if self.labels.iter().any(|j| j.label.starts_with(&typed)) => {
                self.jump_typed = typed;
            }
            None => self.cancel_jump(),
        }
        true
    }
}

#[cfg(test)]
mod measure {
    use super::*;

    /// **量一量：一屏上到底有幾個落腳點。**
    ///
    /// 2026-09-28 把標籤定為固定兩個字母，靠的就是一個數：一屏中文正文約一百四十個
    /// 落腳點，「一個字母那一檔只在屏幕幾乎空着的時候出現」。2026-10-04 重提單字母
    /// 標籤，**那個數只說得了 `gw`**——`go`／`gu` 是另一個分佈。留着是為了重跑得了。
    #[test]
    #[ignore = "量數用的，不是斷言"]
    fn how_many_places_does_one_screen_have() {
        let Some(all) = ["../../docs/manual_tc.md", "docs/manual_tc.md"]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
        else {
            println!("找不到 docs/manual_tc.md，跳過");
            return;
        };
        let lines: Vec<&str> = all.lines().collect();
        let (mut gw, mut go, mut gu) = (Vec::new(), Vec::new(), Vec::new());
        let mut too_narrow = 0usize;
        for start in (0..lines.len()).step_by(400).take(12) {
            let screen: String = lines[start..(start + 40).min(lines.len())].join("\n");
            let mut ed = Editor::new();
            ed.current_buffer_mut().replace(0..0, &screen).unwrap();
            let n = ed.current_buffer().rope().len_chars();
            if n == 0 {
                continue;
            }
            ed.set_page_span(0, n);
            ed.start_jump();
            assert!(ed.take_owed_jump());
            ed.run_owed_jump();
            gw.push(ed.labels_on_the_page().len());
            // 今天**標不了**的那些：一格寬，兩個字母蓋下去會吃掉後面那個東西。
            let rope = ed.current_buffer().rope();
            for line in 0..rope.len_lines() {
                for (a, b) in motion::line_words(rope, line, motion::Grain::Coarse, ed.segmenter.as_ref()) {
                    let wide: usize =
                        rope.slice(a..b).chars().map(yumete_cjk::char_width).sum();
                    let has = rope.slice(a..b).chars().any(char::is_alphanumeric);
                    if has && wide < 2 {
                        too_narrow += 1;
                    }
                }
            }
            for q in ["th", "de", "in", "an", "zh", "sh", "ji", "do"] {
                let found = crate::written::targets(&screen, q);
                if !found.latin.is_empty() {
                    go.push(found.latin.len());
                }
                for (_, at) in &found.han {
                    gu.push(at.len());
                }
            }
        }
        let show = |name: &str, v: &mut Vec<usize>| {
            if v.is_empty() {
                println!("{name:4} 沒有樣本");
                return;
            }
            v.sort_unstable();
            println!(
                "{name:4} 樣本 {:4}　中位 {:3}　最大 {:4}　≤26 的佔 {:3.0}%　≤676 的佔 {:3.0}%",
                v.len(),
                v[v.len() / 2],
                v[v.len() - 1],
                100.0 * v.iter().filter(|&&n| n <= 26).count() as f64 / v.len() as f64,
                100.0 * v.iter().filter(|&&n| n <= 676).count() as f64 / v.len() as f64,
            );
        };
        show("gw", &mut gw);
        show("go", &mut go);
        show("gu", &mut gu);
        println!("今天因為「只有一格」而標不了的單位，十二屏合計 {too_narrow} 個");
    }
}
