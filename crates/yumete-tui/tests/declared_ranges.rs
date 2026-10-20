//! **一項設定的域只許有一個，而它有三個入口。**
//!
//! `settings_ui::counted` 是那一份聲明：設置面板按它畫控件，配置檔按它鉗
//! （`as_declared`，2026-10-10，理由寫在那裏：「鉗的規矩和面板畫的規矩必須是同一個
//! 事實」）。**命令行是第三個入口，而它漏了**——2026-10-20 量到兩頭都歪，而且歪的
//! 方向相反：`:wheel 999999` 一點不攔，回「滾輪：一格 999999 行」，同一個數寫進
//! `config.toml` 回的是 10；`:indent-width 12` 是拒的，而面板上調得到 12。
//!
//! 這一條住在 `yumete-tui` 是因為**只有這裏看得見兩邊**：`yumete-core` 不依賴
//! `yumete-config`，`yumete-config` 也不依賴 `yumete-core`。

/// **給一項計數設定加一個命令，就在下面的 `PAIRS` 裏加一行。**
#[test]
fn every_count_a_command_can_set_stops_where_the_panel_stops() {
    // （命令名，表，鍵）
    const PAIRS: &[(&str, &str, &str)] = &[
        ("wheel", "editor", "wheel_step"),
        ("indent-width", "editor", "indent_width"),
    ];
    let mut loose = Vec::new();
    for &(command, table, key) in PAIRS {
        let Some((low, high, zeroed)) = yumete_config::settings_ui::counted(table, key) else {
            loose.push(format!("[{table}] {key} 不是一項計數設定，`:{command}` 這一行寫錯了"));
            continue;
        };
        let takes = |n: usize| yumete_core::command::parse(&format!(":{command} {n}")).is_ok();
        for n in [low, high] {
            if !takes(n) {
                loose.push(format!(
                    "面板說 [{table}] {key} 是 {low}–{high}，而 `:{command} {n}` 是拒的"
                ));
            }
        }
        // **下界這裏不驗**，上面那一句也不是漏寫的：`:wheel 0` 是故意收的——零
        // 是「終端自己的一格」，也就是一，而配置檔對 `0` 的鉗法一模一樣。配置
        // 那一頭的下界由 `every_count_the_panel_declares_is_held_coming_in` 管。
        // 兩個 bug 都在上界上。
        let _ = zeroed;
        if takes(high + 1) {
            loose.push(format!(
                "面板說 [{table}] {key} 最大 {high}，而 `:{command} {}` 收下了",
                high + 1
            ));
        }
    }
    assert!(loose.is_empty(), "命令行和面板說的不是同一個域：\n{}", loose.join("\n"));
}
