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
//! Warning: **只放「推設定」，不放「開東西」。** 恢復會話、開檔、造輸入法、認 `--flag`
//! 都留在 `main.rs`：它們一趟只能做一次，而這一支要能叫第二次。判準是
//! **冪等** —— 叫兩遍和叫一遍一樣。
//!
//! Warning: **這裏少一項就是一個無聲的洞。** 加了新設定而忘了加進來，啓動時它照舊生效
//! （因為 `main.rs` 那一段被這一支取代了），可 `:config-reload` 之後它會**退回
//! 舊值**——而屏幕上什麼都不說。`the_apply_pass_covers_every_setting` 釘着這件事。

use yumete_config::Config;
use yumete_core::editor::Editor;

/// **每一項設定，推一遍。** 冪等——見本檔開頭。
///
/// `layout` 與 `language` 各收一個覆蓋：`-v`/`--horizontal` 和 `--language` 是這一
/// 趟啓動的答案，不是配置的。重載的時候兩個都傳 `None`，配置說什麼就是什麼。
/// **打錯的值要出聲，而這幾格的解析器在核心裏**（2026-10-10 報的）。
///
/// `yumete-config` 自己那一支（`into_config_saying`）已經盯住它看得見的每一格，
/// 可 `syntax`／`table_rules`／`indent_hint`／`[sidebar]` 的判準寫在 `yumete-core`
/// 裏，而那兩個 crate 是兄弟、互不依賴——於是這幾格打錯只是 `parse` 回 `None`、
/// 悄悄退回出廠值，和「這個設定不管用」在屏幕上長得一模一樣。這一支站在兩邊都看
/// 得見的地方補上那一半。
///
/// Warning: **允許值是數出來的，不是抄的**（`Panel::ALL`）。抄一份下來，下次加一
/// 扇面板這句話就開始撒謊。
pub fn complaints(config: &Config) -> Vec<String> {
    let mut said = Vec::new();
    let panels = || {
        yumete_core::sidebar::Panel::ALL
            .iter()
            .map(|p| p.key())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let word = config.editor.syntax.trim();
    if !word.is_empty() && yumete_core::syntax::Syntax::parse(word).is_none() {
        said.push(format!(
            "[editor] syntax = \"{word}\" 不是一種語法——:syntax 按一下 Tab 看有哪些"
        ));
    }
    for (name, language) in &config.syntax.by_name {
        if yumete_core::syntax::Syntax::parse(language).is_none() {
            said.push(format!(
                "[syntax] {name} = \"{language}\" 不是一種語法——:syntax 按一下 Tab 看有哪些"
            ));
        }
    }
    let word = config.editor.table_rules.trim();
    if !word.is_empty() && yumete_core::table::Rules::parse(word).is_none() {
        said.push(format!(
            "[editor] table_rules = \"{word}\" 只能是 off、color、line、line dash、line double"
        ));
    }
    let word = config.editor.indent_hint.trim();
    if !word.is_empty() && yumete_core::zong::IndentHint::parse(word).is_none() {
        said.push(format!(
            "[editor] indent_hint = \"{word}\" 只能是 none、color、symbol"
        ));
    }
    for (name, side) in &config.sidebar.side {
        if yumete_core::sidebar::Panel::parse(name).is_none() {
            said.push(format!(
                "[sidebar] \"{name}\" 不是面板的名字——只能是 {}",
                panels()
            ));
        } else if yumete_core::sidebar::Side::parse(side).is_none() {
            said.push(format!(
                "[sidebar] {name} = \"{side}\" 只能是 left 或 right"
            ));
        }
    }
    said
}

pub fn apply(
    config: &Config,
    editor: &mut Editor,
    layout: Option<yumete_core::zong::Layout>,
    language: Option<yumete_core::messages::Language>,
) {
    // 進程級的那幾個：不屬於某一個 editor，但同樣是配置說了算。
    if !config.ime.data_dirs.is_empty() {
        yumete_config::set_data_dirs(config.ime.data_dirs.clone());
    }
    // **這三格從前只有啓動那一趟推**（2026-10-07 審出來的）。面板裏改了界面語言、
    // 歧義寬度或顯示注音，存盤，`:reload-config`——屏幕上寫着「設定重讀了」，而那
    // 三樣原封不動。這一支的開頭就寫着「每一項設定，推一遍」，漏掉的那幾項正是
    // 「加一個設定要動三處，漏了第三處」那一族。
    yumete_core::set_ambiguous_wide(match config.editor.ambiguous_width {
        yumete_config::Ambiguity::Wide => true,
        yumete_config::Ambiguity::Narrow => false,
        // 只有終端自己知道它的字體把 `—` 和 `…` 畫成幾格。問不出來就當窄的：
        // 正文本來就是那麼排的，至少光標落在字上。
        yumete_config::Ambiguity::Auto => {
            crate::ambiguous::ask_the_terminal_about_width().unwrap_or(false)
        }
    });
    if let Some(language) =
        language.or_else(|| yumete_core::messages::Language::parse(&config.editor.language))
    {
        yumete_core::messages::set_language(language);
    }
    // 注音畫多少，以及認哪幾種寫法。
    //
    // Warning: **方言那一行從前是 `:ruby {name}`**，而 `:ruby` 從來不收這個詞——它收的
    // 是三個級別。所以 `ruby_dialects` 這一格**一直是死的**（叫的人還 `let _ =`
    // 把錯吞了）。真正的命令是 `:ruby-html on` / `:ruby-typst on`。
    editor
        .execute(match config.editor.show_ruby {
            true => ":ruby-render full",
            // 中階，不是 `off`：沒人要看的讀音也還是讀音，所以字數知道「錢塘」是
            // 兩個字而標記一個都不是（#283）。
            false => ":ruby-render basic",
        })
        .ok();
    for name in &config.editor.ruby_dialects {
        let _ = editor.execute(&format!(":ruby-{name} on"));
    }
    editor.set_key_aliases(config.keys.normal.clone());
    editor.set_key_preset(config.keys.preset);
    editor.set_which_wrap(config.editor.which_wrap.clone());
    // **`d`/`c` 進不進寄存器**（`:yank-on-delete`，2026-10-08）。和上面兩句同一族：
    // 鍵位那一層的事，所以擺在一起。
    editor.set_yank_on_delete(config.editor.yank_on_delete);
    editor.set_layout(layout.unwrap_or(config.editor.layout));
    // Warning: **這四個是「寫了纔算」**，不是「寫了零就當零」：出廠值在 `Editor` 那一
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
    // Warning: `--syntax` 是**這一趟**的答案，比配置大——所以 `main.rs` 在這一支之後
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
/// Warning: **碼表不在這裏。** `[ime] scheme` 換一個要讀幾十兆的表，那是一件看得見的
/// 事（`:yume-scheme` 自己會報進度），不是一次重載順手做的。重載改得動的是面板
/// 那幾格——它們是純設定，不碰磁盤。
pub fn apply_ime(config: &Config, ime: &mut crate::ImeSession) {
    ime.set_page_size(config.panel.page_size);
    ime.set_panel_display(config.panel.display);
    ime.set_preedit(config.panel.preedit);
    // **上屏方式也在這裏推**（2026-10-07 審出來的）。從前只有啓動那一趟推它
    // （`main.rs` 的 `load`），於是面板裏改了上屏方式、存了、`:reload-config`，
    // 屏幕上說「設定重讀了」而那一格原封不動。
    //
    // `None` 要一路傳下去，不能折成一個值：配置沒說話的時候，意思是「這個方案
    // 自己說了算」，而不是「拿拼音的整句去套每一個形碼方案」。
    ime.set_commit_strategy(
        config.ime.commit.as_deref().and_then(yumete_ime::CommitStrategy::from_str_tag),
    );
}

#[cfg(test)]
mod every_setting_is_pushed_here {
    /// **`:reload-config` 推得到的，和啓動時推的，要是同一批**（2026-10-07 審出
    /// 來的）。
    ///
    /// 這一支開頭就寫着「每一項設定，推一遍」，而四樣東西從前只有 `main.rs` 的
    /// 啓動那一段推：界面語言、歧義寬度、顯示注音＋注音寫法、輸入法上屏方式。
    /// 面板裏改了、存了、`:reload-config`——屏幕上寫着「設定重讀了」，那四樣原封
    /// 不動。這是「加一個設定要動三處，漏了第三處」那一族。
    ///
    /// 釘的是**源碼**：那幾個推送器只許在這個檔裏叫，不許散回啓動那一段去。源碼
    /// 比行為好釘——這四樣是進程級的全局，測試裏撥一下會弄紅鄰居。
    #[test]
    fn the_startup_path_pushes_nothing_this_one_does_not() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let main = std::fs::read_to_string(format!("{root}/crates/yumete/src/main.rs"))
            .expect("the front end's main");
        let here = std::fs::read_to_string(format!(
            "{root}/crates/yumete-tui/src/settings.rs"
        ))
        .expect("this file");
        for (what, needle) in [("顯示注音", ":ruby-render"), ("注音寫法", "ruby_dialects")] {
            assert!(here.contains(needle), "{what} 要在這個檔裏推：{needle}");
            assert!(
                !main.contains(needle),
                "{what} 又跑回啓動那一段了（{needle}）——那樣 `:reload-config` 就推不到它"
            );
        }
        // 這三樣兩頭都有，而且必須。前兩個要趕在第一幀／第一句話之前；上屏方式
        // 是因為啓動那一趟會把整個輸入法會話**換掉**（`ImeSession::language_only`），
        // 換完的那一個沒聽過 `apply_ime`。所以釘的是「這個檔裏也有」，不是「那邊
        // 沒有」。
        for needle in ["set_ambiguous_wide", "messages::set_language", "set_commit_strategy"] {
            assert!(here.contains(needle), "{needle} 要在這個檔裏也推一遍");
        }
    }
}
