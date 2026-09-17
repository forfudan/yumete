//! 光標與選區的顏色（#493）。
//!
//! ⚠️ **自己一個檔，因為明暗是個全局。** `theme::set_dark` 寫的是一個
//! `AtomicU8`，而 cargo 在同一個測試二進制裏是多執行緒跑的：這一族原先住在
//! `theme.rs` 的 `mod tests` 裏，於是它每翻一次頁面，隔壁正在畫圖的
//! `the_cell_ground_survives_a_callout_but_not_a_highlight` 就有一次機會量到
//! 另一種模式的顏色——紅得毫無道理，重跑又綠。整合測試各自一個行程。
//!
//! 同一條在 `yumete-core/tests/language.rs` 那邊已經記過一次：**改全局的測試
//! 不許和別人共用一個二進制。**

use yumete_tui::theme::{self, Palette};

fn rgb(colour: ratatui::style::Color) -> (u8, u8, u8) {
    match colour {
        ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
        other => panic!("不是 RGB：{other:?}"),
    }
}

/// 橫排的光標是終端機自己畫的，顏色來自使用者的終端機配置——它不知道頁面翻成了
/// 淺色。原話：「浅色模式肉眼很难找到光标的位置。」所以由頁面用 OSC 12 說。
#[test]
fn the_caret_is_the_ink_in_both_moods() {
    let config = yumete_config::Config::default();
    let sum = |(r, g, b): (u8, u8, u8)| r as u32 + g as u32 + b as u32;
    for (dark, want_light_caret) in [(true, true), (false, false)] {
        theme::set_dark(dark);
        let palette = Palette::of(&config);
        let caret = palette.caret();
        assert_eq!(
            sum(caret) > 383 * 3 / 2,
            want_light_caret,
            "深色頁要淺光標，淺色頁要深光標：{caret:?}"
        );
        // 它蓋在字上，所以要和紙分得開——正文與紙的對比就是這一條的標準。
        let apart = sum(caret).abs_diff(sum(rgb(palette.paper())));
        assert!(apart > 300, "光標與紙只差 {apart}，在紙上看不見");
    }

    // 選區反過來：它是**底色**，所以深色頁配深灰、淺色頁配淺灰，這一半本來就對
    // ——量在這裏，免得哪天有人「順手」把它也調成墨色。
    theme::set_dark(false);
    let light = sum(rgb(Palette::of(&config).selection()));
    theme::set_dark(true);
    let dark = sum(rgb(Palette::of(&config).selection()));
    assert!(light > dark, "淺色頁的選區（{light}）要比深色頁的（{dark}）亮");
}
