//! **把一份配置推進編輯器** —— 一處，兩個人叫（2026-09-23）。
//!
//! 從前這件事只發生在 `main.rs` 啓動那一段裏，攤成一百多行 `editor.set_…`。
//! 那沒有錯，直到有人想**改完配置不重啓**：
//!
//! > 「下次啓動生效或者一個應用（重載）設置命令來生效。」——2026-09-23
//!
//! 於是它要有名字。啓動叫一次，`:config-reload` 叫一次，將來那扇設置面板存完盤
//! 再叫一次——**同一支函數**，所以不會出現「啓動時認這個設定、重載時忘了它」。
//!
//! ⚠️ **只放「推設定」，不放「開東西」。** 恢復會話、開檔、造輸入法、認 `--flag`
//! 都留在 `main.rs`：它們一趟只能做一次，而這一支要能叫第二次。判準是
//! **冪等** —— 叫兩遍和叫一遍一樣。
//!
//! ⚠️ **這裏少一項就是一個無聲的洞。** 加了新設定而忘了加進來，啓動時它照舊生效
//! （因為 `main.rs` 那一段被這一支取代了），可 `:config-reload` 之後它會**退回
//! 舊值**——而屏幕上什麼都不說。`the_apply_pass_covers_every_setting` 釘着這件事。

use yumete_config::Config;
use yumete_core::editor::Editor;

/// **每一項設定，推一遍。** 冪等——見本檔開頭。
///
/// `layout` 收一個覆蓋：`-v`／`--horizontal` 是這一趟的答案，不是配置的。重載的
/// 時候傳 `None`，配置說什麼就是什麼。
pub fn apply(config: &Config, editor: &mut Editor, layout: Option<yumete_core::zong::Layout>) {
    // 進程級的那幾個：不屬於某一個 editor，但同樣是配置說了算。
    if !config.ime.data_dirs.is_empty() {
        yumete_config::set_data_dirs(config.ime.data_dirs.clone());
    }
    editor.set_key_aliases(config.keys.normal.clone());
    editor.set_key_preset(config.keys.preset);
    editor.set_layout(layout.unwrap_or(config.editor.layout));
    // ⚠️ **這四個是「寫了纔算」**，不是「寫了零就當零」：出廠值在 `Editor` 那一
    // 頭，而配置裏的 0 是「沒說」。照抄啓動時的判準，一個字都不改。
    if config.editor.indent > 0 {
        editor.set_indent(config.editor.indent);
    }
    if config.editor.bands > 1 {
        editor.set_bands(config.editor.bands);
    }
    if config.editor.zong_length > 0 {
        editor.set_zong_length(config.editor.zong_length);
    }
    if config.editor.measure > 0 {
        editor.set_measure(Some(config.editor.measure));
    }
    editor.set_paper(config.export.page.0, config.export.page.1);
    editor.set_indent_width(config.editor.indent_width);
    editor.set_tab_spaces(config.editor.tab_spaces);
    editor.set_tab_stop(config.editor.tab_width);
    editor.set_tatechuyoko(config.editor.tatechuyoko);
    editor.set_code_colours(config.editor.code_highlight);
    editor.set_hanging_punctuation(config.editor.hanging_punctuation);
    editor.set_soft_wrap(config.editor.soft_wrap);
    // ⚠️ `--syntax` 是**這一趟**的答案，比配置大——所以 `main.rs` 在這一支之後
    // 再設一次。這裏設的是配置說的那一個，重載纔認得出它改了。
    editor.set_default_syntax(yumete_core::syntax::Syntax::parse(&config.editor.syntax));
    editor.set_syntax_by_name(
        config
            .syntax
            .by_name
            .iter()
            .filter_map(|(name, language)| {
                yumete_core::syntax::Syntax::parse(language).map(|s| (name.clone(), s))
            })
            .collect(),
    );
    editor.set_autosave(config.editor.autosave);
    editor.set_smart_case(config.editor.smart_case);
    editor.set_fuzzy_search(config.editor.fuzzy_search);
    editor.set_wheel_step(config.editor.wheel_step);
    // A name or a word nobody knows is skipped rather than guessed at — a typo
    // here moves the whole page.
    for (name, side) in &config.sidebar.side {
        if let (Some(panel), Some(side)) = (
            yumete_core::sidebar::Panel::parse(name),
            yumete_core::sidebar::Side::parse(side),
        ) {
            editor.set_side(panel, side);
        }
    }
    editor.set_margin(config.editor.margin);
    editor.set_word_level(config.editor.word_level);
    editor.set_segmentation_visible(config.editor.show_segmentation);
    editor.set_word_mark(config.editor.word_mark);
    if let Some(rules) = yumete_core::table::Rules::parse(&config.editor.table_rules) {
        editor.set_table_rules(rules);
    }
    editor.set_number_fill(config.editor.line_number_fill);
    editor.set_diff_gutter(config.editor.diff_gutter);
    if let Some(hint) = yumete_core::zong::IndentHint::parse(&config.editor.indent_hint) {
        editor.set_indent_hint(hint, Some(config.editor.indent_symbol.clone()));
    }
    editor.set_chaifen(config.editor.show_chaifen);
    editor.set_usage_groups(config.editor.usage_groups.clone());
}

/// **The settings the input method keeps**, for the same reason as [`apply`].
///
/// ⚠️ **碼表不在這裏。** `[ime] scheme` 換一個要讀幾十兆的表，那是一件看得見的
/// 事（`:yume-scheme` 自己會報進度），不是一次重載順手做的。重載改得動的是面板
/// 那幾格——它們是純設定，不碰磁盤。
pub fn apply_ime(config: &Config, ime: &mut crate::ImeSession) {
    ime.set_page_size(config.panel.page_size);
    ime.set_panel_display(config.panel.display);
}
