//! **轉圈的那八個點**，一個模組，誰都用得上（2026-09-30 作者提的）。
//!
//! 原話：「⣾⣽⣻⢿⡿⣟⣯⣷ I love this. This can be used elsewhere (at different
//! locations to mean different things). If it is beside the NOR, it means the
//! status of LSP. It can also be somewhere else, like after a notice to show
//! 『doing』, instead of using 『...』.」
//!
//! Warning: **它不存狀態，只問時間。** 轉到第幾格是「從什麽時候開始轉」算出來
//! 的純函數——所以一個地方要用它，只要記得自己是什麽時候開始忙的，別的什麽都
//! 不必記，也不會有兩處轉得不同步。
//!
//! Warning: **不轉的時候也要占那一格。** 旁邊的字纔不會每八十毫秒跳一次；helix
//! 的註釋原話是「Even if there's no spinner; reserve its space to avoid
//! elements frequently shifting」（`ui/statusline.rs`）。所以 [`frame`] 不轉的
//! 時候回的是一個空格，不是空串。
//!
//! Warning: **那句話的後半至今沒有落點，而且理由是結構性的**（2026-10-02 查
//! 的）。「after a notice to show 『doing』」要有一件**邊轉邊等**的事，而這個
//! 編輯器裏慢的那幾件全是同步跑完的：搜索走磁碟（最長五秒，見
//! `editor::WALK_DEADLINE`）、自動認詞、`:check` 那一族——它們一跑，主循環就
//! 停在那裏，轉圈轉不動，畫出來的永遠是同一格。真要讓它轉起來，先要把那幾件搬
//! 到後臺去，那是另一件事。所以今天只有語言服務器那一處用得上它：**那一頭本來
//! 就在別的進程裏跑**。
//!
//! Warning: **要有人叫醒循環。** 這個編輯器是有事纔重畫的，不按秒重畫——所以轉
//! 圈的那一頭得告訴主循環「下一格什麽時候到」（[`TICK`]），同 `docs_due_in`／
//! `vcs_due_in` 那一族。不說的話它畫一格就睡着了。

use std::time::{Duration, Instant};

/// 八格，轉一圈。照 helix 的 `ui/spinner.rs`——那一組點在等寬字體裏每一格都是
/// 一欄寬，換格的時候不會把旁邊的字推開。
pub const FRAMES: [&str; 8] = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

/// 多久換一格。八十毫秒是 helix 的數，轉一圈正好 0.64 秒——快得看得出在動，慢
/// 得不晃眼。
pub const TICK: Duration = Duration::from_millis(80);

/// 它占幾欄。不轉的時候也占這麽寬（見模組頭上那一段）。
pub const WIDTH: u16 = 1;

/// **轉到第幾格**——`None` ＝ 沒在忙，回一個空格占位。
///
/// ```text
/// let mark = spinner::frame(servers.busy_since());
/// ```
pub fn frame(since: Option<Instant>) -> &'static str {
    let Some(since) = since else { return " " };
    let step = since.elapsed().as_millis() / TICK.as_millis().max(1);
    FRAMES[(step as usize) % FRAMES.len()]
}

/// **下一格什麽時候到**，`None` ＝ 沒在忙，不必叫醒。
///
/// 交給主循環當鬧鐘用：不給這個數，它畫完一格就睡到下一次按鍵，那八個點就成了
/// 一個不動的點。
pub fn due_in(since: Option<Instant>) -> Option<Duration> {
    let since = since?;
    let into = Duration::from_millis((since.elapsed().as_millis() % TICK.as_millis()) as u64);
    Some(TICK.saturating_sub(into))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 不忙的時候占一格空白——占住了，旁邊的字纔不會跳。
    #[test]
    fn it_holds_its_place_when_nothing_is_running() {
        assert_eq!(frame(None), " ");
        assert_eq!(yumete_cjk::str_width(frame(None)), WIDTH as usize);
        assert_eq!(due_in(None), None);
    }

    /// 每一格都是一欄寬——不然轉一圈會把整行推來推去。
    #[test]
    fn every_frame_is_one_column_wide() {
        for one in FRAMES {
            assert_eq!(yumete_cjk::str_width(one), WIDTH as usize, "{one:?}");
        }
    }

    /// 走過八十毫秒換一格，轉滿八格回到頭上。
    #[test]
    fn it_turns_one_step_per_tick_and_comes_round() {
        let now = Instant::now();
        let at = |ms: u64| frame(Some(now - Duration::from_millis(ms)));
        assert_eq!(at(0), FRAMES[0]);
        assert_eq!(at(80), FRAMES[1]);
        assert_eq!(at(79), FRAMES[0], "不到八十毫秒不換");
        assert_eq!(at(80 * 7), FRAMES[7]);
        assert_eq!(at(80 * 8), FRAMES[0], "轉回頭上");
        assert_eq!(at(80 * 9), FRAMES[1]);
    }

    /// 鬧鐘永遠在一格之內，而且不會是零——零會讓循環空轉。
    #[test]
    fn the_alarm_is_always_within_one_tick() {
        let now = Instant::now();
        for ms in [0u64, 1, 40, 79, 80, 161, 640] {
            let left = due_in(Some(now - Duration::from_millis(ms))).unwrap();
            assert!(left <= TICK, "{ms} 毫秒之後還剩 {left:?}");
        }
    }
}
