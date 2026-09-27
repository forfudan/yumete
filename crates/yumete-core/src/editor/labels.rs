//! **`gw` — 一眼跳到屏幕上任何地方**（#406）。
//!
//! 按 `gw`，屏幕上每個落腳點的頭一個字被兩個字母蓋住；打那兩個字母，光標飛過去。
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
//! 了，這一跳就白跳。⚠️ **代價是英文下會糊**：`was` 剩一個 `s`。中文不糊——一個漢字
//! 正好兩格，標籤蓋掉的就是那一個字。
//!
//! # 和 helix 有意不同的兩處（2026-09-28 讀它的源碼對出來的）
//!
//! ⚠️ **一、蓋一個字素，不是兩個。** `helix-term/src/commands.rs` 的 `jump_to_label`
//! 在 `range.from()` 和 `next_grapheme_boundary(from)` 各放一個 overlay——兩個字母
//! 蓋掉**兩個**字素。西文剛好（兩個字母蓋兩個字母），中文就是拿 2 格蓋掉 4 格，
//! **那一行當場縮短兩格、整段重排**。我們蓋一個：一個漢字正好兩格。
//!
//! **二、標籤不是一律兩個字母。** helix 永遠讀兩鍵（`on_next_key` 套兩層）。落腳點
//! 不到一個字母表那麼多的時候，第二鍵沒有分辨力，白按。所以這裏一屏之內要麼全是
//! 一個字母、要麼全是兩個——打完第一個不會不知道還要不要打第二個。
//!
//! **抄過來的兩條**：落腳點上限是字母表長度的平方（多出來的畫不出來也按不到）；
//! 跳之前先 `remember_jump()`，`C-o` 回得來（helix 的 `push_jump`）。

use super::*;

/// 一個落腳點和它的標籤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jump {
    /// 落腳點在檔裏的字符下標。
    pub at: usize,
    /// 打進去就去那裏的那一兩個字母。
    pub label: String,
}

/// 標籤用的字母，**home row 在前**。
///
/// 落腳點按檔裏的次序拿標籤，所以前面那幾個拿到的是手指不用挪的那幾個鍵。
const ALPHABET: &[u8] = b"asdfghjklqwertyuiopzxcvbnm";

impl Editor {
    /// **這一頁畫了哪一段**（字符下標），前端每幀交過來一次。
    ///
    /// 同 [`Editor::set_page_top`]：畫的時候纔定得下來，所以到這裏的是上一幀的答案
    /// ——而落腳點只在按下 `gw` 的那一刻算一次，那時候上一幀就是眼前這一幀。
    pub fn set_page_span(&mut self, from: usize, to: usize) {
        self.page_span = (from.min(to), to);
    }

    /// 屏幕上的落腳點，按檔裏的次序。`room` 是一個標籤要佔幾格。
    fn jump_spots(&self, room: usize) -> Vec<usize> {
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
                // ⚠️ **蓋不下就不標。** 標籤是蓋在那個單位頭上的，蓋過了頭就吃掉
                // 它後面那個東西——英文的 `a` 只有一格，兩個字母蓋下去連它後面的
                // 空格一起沒了，於是前後兩個詞黏在一起。helix 也跳過短詞（它的
                // 說法是「two or more characters」），這裏按**格數**算，因為中文
                // 一個字就有兩格：`a` 不標，而單獨一個「我」標得了。
                let wide: usize = rope
                    .slice(a..b)
                    .chars()
                    .map(|c| yumete_cjk::char_width(c))
                    .sum();
                if wide < room {
                    continue;
                }
                out.push(a);
            }
        }
        out
    }

    /// **記一筆「欠着一次 `gw`」。** 按鍵那一支叫的是這個。
    ///
    /// ⚠️ **落腳點只算屏幕上的，而「屏幕上」是畫的那一方纔知道的事**——所以不能
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
        // ⚠️ **一個雞生蛋的小結**：標籤幾個字母，看落腳點有幾個；而蓋不下的單位
        // 不算落腳點，所以得先知道標籤幾個字母。先按一個字母數一遍——數出來超過
        // 一個字母表，就按兩格再數一遍。第二遍只會更少（門檻更高），所以不會來回
        // 震盪。
        let mut spots = self.jump_spots(1);
        let wide = spots.len() > ALPHABET.len();
        if wide {
            spots = self.jump_spots(2);
        }
        if spots.is_empty() {
            self.status = say!("jump.nowhere");
            return;
        }
        let letter = |n: usize| ALPHABET[n % ALPHABET.len()] as char;
        self.labels = spots
            .into_iter()
            .take(ALPHABET.len() * ALPHABET.len())
            .enumerate()
            .map(|(i, at)| Jump {
                at,
                label: match wide {
                    false => letter(i).to_string(),
                    true => format!("{}{}", letter(i / ALPHABET.len()), letter(i)),
                },
            })
            .collect();
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

    /// 把標籤收掉。
    pub(super) fn cancel_jump(&mut self) {
        self.labels.clear();
        self.jump_typed.clear();
    }

    /// 標籤亮着的時候，這一鍵怎麼算。
    ///
    /// ⚠️ **不認得的鍵一律收掉標籤並吃掉它自己。** 「按錯一個鍵就跳到別處去」比
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
                // 記進跳轉表，`C-o` 回得來——這一跳和 `30G` 一樣是「去了別處」。
                self.remember_jump();
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
