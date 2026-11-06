//! **把一份配置推進編輯器** —— 一處，兩個人叫（2026-09-23）。
//!
//! 從前這件事只發生在 `main.rs` 啓動那一段裏，攤成一百多行 `editor.set_…`。
//! 那沒有錯，直到有人想**改完配置不重啓**：
//!
//! > 「下次啓動生效或者一個應用（重載）設置命令來生效。」——2026-09-23
//!
//! 於是它要有名字。啓動叫一次，`:reload-config` 叫一次，將來那扇設置面板存完盤
//! 再叫一次——**同一支函數**，所以不會出現「啓動時認這個設定、重載時忘了它」。
//!
//! Warning: **只放「推設定」，不放「開東西」。** 恢復會話、開檔、造輸入法、認 `--flag`
//! 都留在 `main.rs`：它們一趟只能做一次，而這一支要能叫第二次。判準是
//! **冪等** —— 叫兩遍和叫一遍一樣。
//!
//! Warning: **這裏少一項就是一個無聲的洞。** 加了新設定而忘了加進來，啓動時它照舊生效
//! （因為 `main.rs` 那一段被這一支取代了），可 `:reload-config` 之後它會**退回
//! 舊值**——而屏幕上什麼都不說。`the_apply_pass_covers_every_setting` 釘着這件事。

use yumete_config::Config;
use yumete_core::editor::Editor;
use yumete_core::{say, TextStore as _};

/// **每一項設定，推一遍。** 冪等——見本檔開頭。
///
/// `layout` 與 `language` 各收一個覆蓋：`-v`/`--horizontal` 和 `--language` 是這一
/// 趟啓動的答案，不是配置的。重載的時候兩個都傳 `None`，配置說什麼就是什麼。
/// **打錯的值要出聲，而這幾格的解析器在核心裏**（2026-10-10 報的）。
///
/// `yumete-config` 自己那一支（`into_config_saying`）已經盯住它看得見的每一格，
/// 可 `syntax`／`table_rules`／`indent_hint` 的判準寫在 `yumete-core`
/// 裏，而那兩個 crate 是兄弟、互不依賴——於是這幾格打錯只是 `parse` 回 `None`、
/// 悄悄退回出廠值，和「這個設定不管用」在屏幕上長得一模一樣。這一支站在兩邊都看
/// 得見的地方補上那一半。
///
/// **The config file, checked while you are looking at it** (2026-10-20 作者定).
///
/// 原話：「為 config.toml 寫個 yumete config 專用 lsp，檢測有問題的項目？」Asked
/// on 2026-10-10 and written down then as not built (§5.155 ②); this is it.
///
/// Warning: **no server, and that is the point.** Everything it needs is already
/// here — the checks ([`yumete_config::faults::faults_in`] and the four below
/// that need `yumete-core`'s own parsers), the place to put the answer
/// ([`Editor::set_problems`]), and the gutter and 診斷 panel that draw whatever
/// is in it. A language server would have added a process, a socket and a
/// second copy of the rules.
///
/// It follows the **current** buffer, like the language servers do: a config
/// file open in the other pane and not being looked at is not where the
/// reader's attention is, and the check is cheap enough to run the moment it
/// comes back.
#[derive(Default)]
pub struct ConfigWatch {
    /// The file and the revision already answered for, so the walk happens on
    /// an edit rather than on every frame.
    seen: Option<(std::path::PathBuf, u64)>,
}

impl ConfigWatch {
    /// Whether this path is one of the files the loader actually reads.
    ///
    /// Matched by shape rather than by canonical path: a buffer may hold a
    /// relative path, and `~/.config/yumete/config.toml` through a symlinked
    /// home is the same file by two names. The three shapes are
    /// [`yumete_config::config_dir`]'s `config.toml`, a project's
    /// `.yumete/config.toml`, and a project's `.yumete.toml`.
    fn is_a_config(path: &std::path::Path) -> bool {
        let named = |what: &str| path.file_name().is_some_and(|n| n == what);
        if named(".yumete.toml") {
            return true;
        }
        if !named("config.toml") {
            return false;
        }
        let parent = path.parent();
        parent.is_some_and(|p| p.file_name().is_some_and(|n| n == ".yumete"))
            || parent == Some(yumete_config::config_dir().as_path())
    }

    /// Check the current buffer if it is a config file and has changed.
    pub fn follow(&mut self, editor: &mut Editor) {
        let Some(path) = editor.current_buffer().path().map(std::path::Path::to_path_buf) else {
            return;
        };
        if !Self::is_a_config(&path) {
            return;
        }
        let now = (path.clone(), editor.current_buffer().revision());
        if self.seen.as_ref() == Some(&now) {
            return;
        }
        self.seen = Some(now);
        let text = editor.current_buffer().text();
        editor.set_problems(path, String::from(Self::WHOSE), Self::faults(&text));
    }

    /// The name this checker signs its complaints with — it stands where a
    /// server's name stands, and `Problems` keeps one list per name.
    const WHOSE: &'static str = "yumete-config";

    /// Everything wrong with this text, as the editor draws complaints.
    fn faults(text: &str) -> Vec<yumete_core::problem::Problem> {
        use yumete_core::problem::{Problem, Severity};
        let say = |line: usize, severity: Severity, message: String| Problem {
            line,
            // **The line, not a column.** Every one of these is about a key,
            // and a key is the line it is written on; pointing at a column
            // would be pointing at the quote marks.
            utf16_column: 0,
            severity,
            message,
            source: Some(String::from(Self::WHOSE)),
            code: None,
        };
        let mut out: Vec<Problem> = yumete_config::faults::faults_in(text)
            .into_iter()
            .map(|f| {
                let loud = match f.fatal {
                    // **這一條讓整份不生效**，所以它要說出那個後果，不能只說
                    // 哪個字不認得——同啓動那一行（§5.155）。
                    true => Severity::Error,
                    false => Severity::Warn,
                };
                let said = match f.fatal {
                    true => format!("{}——{}", f.said, say!("config.whole-file-dropped")),
                    false => f.said,
                };
                say(f.line, loud, said)
            })
            .collect();
        // **The four whose rules live in `yumete-core`.** `yumete-config` is
        // its sibling and cannot see them; this crate sees both, which is why
        // `complaints` is here and not there.
        if out.iter().all(|p| p.severity != Severity::Error) {
            for said in complaints(&Config::from_toml(text)) {
                let line = yumete_config::faults::line_for(text, &said);
                out.push(say(line, Severity::Warn, said));
            }
        }
        out.sort_by_key(|p| p.line);
        out
    }
}

/// **`:check-config` — both config files, without opening either** (2026-10-20).
///
/// The editor's own checks already run at start-up and, since
/// [`ConfigWatch`], on the file while it is open. This is the third question
/// a reader asks — 「is anything wrong *right now*」 — and it is the one that
/// does not require knowing where the files are.
///
/// The listing is `path:line: 說的話`, which is the shape `gf` and `Enter`
/// already walk, so every complaint is one keystroke from the line it is
/// about. Absolute paths: the two files are in two different places and
/// neither is under the project.
pub fn answer_config_check(editor: &mut Editor) {
    if !editor.take_config_check() {
        return;
    }
    let mut lines = Vec::new();
    let mut looked = 0;
    for path in config_files(editor) {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        looked += 1;
        let mut said: Vec<(usize, String)> = yumete_config::faults::faults_in(&text)
            .into_iter()
            .map(|f| (f.line, f.said))
            .collect();
        // The four whose rules are `yumete-core`'s, located the same way.
        if !text.trim().is_empty() {
            for one in complaints(&Config::from_toml(&text)) {
                said.push((yumete_config::faults::line_for(&text, &one), one));
            }
        }
        // **Down the file, not in the order the checks happen to run.** A
        // listing is walked with `Enter`, and one that jumps 6 → 3 → 2 makes
        // a reader lose their place in their own config.
        said.sort_by_key(|(line, _)| *line);
        // **The path in full.** `~` would read better and `gf` would then have
        // to expand it; the listing's whole value is that every line is one
        // keystroke from the place it is about.
        lines.extend(said.into_iter().map(|(line, one)| {
            format!("{}:{}: {one}", path.display(), line + 1)
        }));
    }
    // The keys a table took are not a fault, but asking is when a reader
    // wants the whole list: it is said once at start-up (2026-11-06).
    let taken = editor.taken_keys_note();
    if lines.is_empty() {
        let clean = say!("config.check-clean", looked);
        editor.set_status(taken.map_or(clean.clone(), |note| format!("{clean} · {note}")));
        return;
    }
    let n = lines.len();
    lines.extend(taken);
    editor.open_report(&say!("config.check-name"), &format!("{}
", lines.join("
")));
    editor.set_status(say!("config.check-found", n));
}

/// **Whether the keys a table took are news**: they differ from the ones
/// last said, which `dir`'s `taken-keys` file keeps. Saying them every
/// start-up was saying the same thing to someone who chose it; a config that
/// takes no key forgets the file, so taking them again is said again
/// (2026-11-06).
pub fn taken_keys_are_news(taken: &[String], dir: &std::path::Path) -> bool {
    let file = dir.join("taken-keys");
    if taken.is_empty() {
        let _ = std::fs::remove_file(&file);
        return false;
    }
    let now = taken.join("\n");
    if std::fs::read_to_string(&file).is_ok_and(|was| was == now) {
        return false;
    }
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&file, now);
    true
}

/// The config files that exist, global first — the order the loader reads
/// them in, so a reader sees them the way they are applied.
fn config_files(editor: &Editor) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let global = yumete_config::config_dir().join("config.toml");
    if global.is_file() {
        out.push(global);
    }
    out.extend(yumete_config::local_config_path(&editor.root()));
    out
}

pub fn complaints(config: &Config) -> Vec<String> {
    let mut said = Vec::new();
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
    let word = config.editor.auto_pairs.trim();
    if !word.is_empty() && yumete_core::editor::AutoPairs::parse(word).is_none() {
        said.push(format!("[editor] auto_pairs = \"{word}\" 只能是 off、basic、full"));
    }
    let word = config.editor.status_count.trim();
    if !word.is_empty() && !matches!(word, "text" | "all" | "off") {
        said.push(format!("[editor] status_count = \"{word}\" 只能是 text、all、off"));
    }
    said
}

/// **How the page looks, as the `:` commands that would set it** (2026-10-28),
/// without the colon: the layout, the `:view-*` settings (not `:view-preview`,
/// which starts a server) and the theme. Order matters where one sets another:
/// `view-wrap 40` turns wrapping on, so the measure comes before the switch.
fn view_lines(editor: &Editor) -> Vec<String> {
    use yumete_core::editor::Hud;
    use yumete_core::zong::Layout;
    let on = |b: bool| if b { "on" } else { "off" };
    vec![
        // What was chosen, not what this buffer shows: a program file or a
        // drawn table is horizontal whatever the reader chose (2026-10-29,
        // review: quitting on a `.rs` kept `horizontal` over a vertical config).
        match editor.layout_chosen() {
            Layout::Vertical => "vertical".to_string(),
            Layout::Horizontal => "horizontal".to_string(),
        },
        format!("view-wrap {}", editor.measure().unwrap_or(0)),
        format!("view-wrap {}", on(editor.soft_wrap())),
        format!("view-wrap-vertical {}", editor.zong_measure().unwrap_or(0)),
        format!("view-pages-vertical {}", editor.pages_vertical()),
        format!("view-pages-horizontal {}", editor.pages_horizontal()),
        format!("view-margin {}", editor.margin().name()),
        format!("view-hanging {}", on(editor.hanging_punctuation())),
        format!("view-numbers-fill {}", on(editor.number_fill())),
        format!("line-numbers {}", editor.line_numbers().name()),
        format!("view-focus {}", on(editor.focus())),
        format!("view-typewriter {}", on(editor.typewriter())),
        format!(
            "view-hud {}",
            match editor.hud() {
                Hud::Off => "off",
                Hud::Basic => "basic",
                Hud::Full => "full",
            }
        ),
        format!("view-sentence {}", on(editor.sentences())),
        format!("view-punct {}", on(editor.notes())),
        format!("view-meter {}", on(editor.meter())),
        format!("view-diff {}", on(editor.diff_gutter())),
        format!("view-code {}", on(editor.code_colours())),
        match editor.long_line() {
            Some(upto) => format!("view-long-line {upto}"),
            None => "view-long-line off".to_string(),
        },
    ]
}

/// **What the session keeps of how the page looks** (2026-10-28): each line of
/// [`view_lines`] the config would not give, then the theme that `:theme`,
/// `:theme-mode` or `:theme-fill` chose. What the config gives is worked out on
/// a fresh editor with the config applied, so a `:reload-config` part-way
/// through is counted, and a setting put back the way the config has it is
/// not kept.
pub fn view_to_keep(editor: &Editor, config: &Config) -> Vec<String> {
    let given = view_lines(&as_configured(config));
    let lines = view_lines(editor);
    // **The wrap width and the switch go together** (2026-10-29, review): a
    // kept `view-wrap 40` without its `view-wrap off` turned wrapping on when
    // replayed, since a width turns it on.
    let wrap_differs = lines.iter().filter(|l| l.starts_with("view-wrap ")).any(|l| !given.contains(l));
    let mut keep: Vec<String> = lines
        .into_iter()
        .filter(|line| !given.contains(line) || (wrap_differs && line.starts_with("view-wrap ")))
        .collect();
    // **Not what the config says already** (2026-10-29, review): a chosen
    // theme or mood the config gives too was kept, and then outlived a later
    // change to the config.
    let name = crate::theme::name(config);
    if crate::theme::snapshot().is_some() && name != config.theme.name {
        keep.push(format!("theme {name}"));
    }
    let configured = match config.theme.mode {
        yumete_config::Mode::Auto => "system",
        yumete_config::Mode::Dark => "dark",
        yumete_config::Mode::Light => "light",
    };
    if let Some(mood) = crate::theme::chosen_mood().filter(|&mood| mood != configured) {
        keep.push(format!("theme-mode {mood}"));
    }
    if let Some(fill) = crate::theme::chosen_fill() {
        keep.push(format!("theme-fill {}", if fill { "on" } else { "off" }));
    }
    keep
}

/// A fresh editor with `config` applied: what the config gives.
///
/// Warning: **The editor half only** (2026-10-29, review): the whole of
/// [`apply`] also asks the terminal about ambiguous widths and sets the
/// process's language, and this runs at every quit, after the terminal has
/// been handed back: a 120 ms wait, and over ssh a late reply printed at the
/// shell prompt.
fn as_configured(config: &Config) -> Editor {
    let mut plain = Editor::new();
    apply_to_editor(config, &mut plain, None);
    plain
}

/// **Every view setting back the way the config has it** (2026-10-28), for
/// `:reload-config`. [`apply`] sets only what a config writes, so a page count,
/// `:view-focus` or `:view-hud` set by command used to outlive the reload that
/// was meant to undo it. The status line is the reload's to fill.
pub fn view_back_to_config(editor: &mut Editor, config: &Config) {
    let was = editor.status().to_string();
    // Asked afresh before each line (2026-10-29, review): one line can change
    // another (`view-wrap 0` turns wrapping on), so what was true before the
    // first is not true before the third. Quietly, so a setting that needs
    // 竪排 is put back on a horizontal page too.
    for line in view_lines(&as_configured(config)) {
        if !view_lines(editor).contains(&line) {
            editor.execute_quietly(&format!(":{line}"));
        }
    }
    editor.set_status(was);
}

pub fn apply(
    config: &Config,
    editor: &mut Editor,
    layout: Option<yumete_core::zong::Layout>,
    language: Option<yumete_core::messages::Language>,
) {
    apply_to_the_process(config, language);
    editor.set_config_file(yumete_config::config_dir().join("config.toml"));
    apply_to_editor(config, editor, layout);
}

/// The settings that belong to the process rather than to one editor.
fn apply_to_the_process(config: &Config, language: Option<yumete_core::messages::Language>) {
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
    // **`auto` is the locale's** (2026-10-29): an English system starts in
    // English, a Chinese one in its own script.
    let language = language.or_else(|| yumete_core::messages::Language::parse(&config.editor.language));
    let language = match config.editor.language.trim() {
        "auto" | "" => language.or_else(|| Some(the_locale())),
        _ => language,
    };
    if let Some(language) = language {
        yumete_core::messages::set_language(language);
    }
}

/// The language the locale asks for — in a test build, always 繁體.
///
/// Warning: **The language is process-wide, and the tests share a process.**
/// A test that reloads a default config would otherwise switch every test
/// after it into the machine's locale — English here, something else in CI —
/// and they assert on the text they see (2026-10-29: twenty went red at once).
#[cfg(not(test))]
fn the_locale() -> yumete_core::messages::Language {
    yumete_core::messages::locale_language()
}

#[cfg(test)]
fn the_locale() -> yumete_core::messages::Language {
    yumete_core::messages::Language::Traditional
}

/// The settings that belong to one editor.
fn apply_to_editor(config: &Config, editor: &mut Editor, layout: Option<yumete_core::zong::Layout>) {
    // **`remember = false` takes hold without a restart** (2026-10-29, review:
    // unticked in the panel, the session was still written at the next quit).
    // Only off: turning it on needs the session's place, worked out at start.
    if !config.editor.remember {
        editor.set_remembering(false);
    }
    // How much ruby is laid out. Both ruby syntaxes are always read, so
    // there is no per-dialect switch here (`ruby_dialects` went 2026-10-30).
    editor
        .execute(match config.editor.show_ruby {
            true => ":ruby-render full",
            // 中階，不是 `off`：沒人要看的讀音也還是讀音，所以字數知道「錢塘」是
            // 兩個字而標記一個都不是（#283）。
            false => ":ruby-render basic",
        })
        .ok();
    // The key trees are laid once, after every setter.
    editor.lay_keys_once(|editor| {
        editor.set_key_aliases(config.keys.normal.clone());
        editor.set_select_aliases(config.keys.select.clone());
        editor.set_key_labels(config.keys.labels.clone());
        editor.set_key_shapes(config.keys.shapes.clone());
        editor.set_insert_aliases(config.keys.insert.clone());
        for (keymap, own) in [
            (yumete_cjk::KeyPreset::Helix, &config.keys.helix),
            (yumete_cjk::KeyPreset::Vim, &config.keys.vim),
        ] {
            editor.set_keymap_aliases(keymap, own.normal.clone(), own.select.clone(), own.insert.clone());
            editor.set_keymap_labels(keymap, own.labels.clone());
            editor.set_keymap_shapes(keymap, own.shapes.clone());
        }
        editor.set_table_aliases(config.keys.table.clone());
        editor.set_key_preset(config.keys.preset);
        // **`d`/`c` 進不進寄存器**（`:yank-on-delete`，2026-10-08）。和上面兩句同一族：
        // 鍵位那一層的事，所以擺在一起。
        editor.set_flavor(config.keys.flavor.clone());
    });
    use yumete_core::actions::Place;
    editor.set_place_bindings(Place::Prompt, config.keys.prompt.clone());
    editor.set_place_bindings(Place::Picker, config.keys.picker.clone());
    editor.set_place_bindings(Place::Sidebar, config.keys.sidebar.clone());
    editor.set_place_bindings(Place::Field, config.keys.field.clone());
    editor.set_place_inheritance(config.keys.inherit);
    editor.set_which_wrap(config.editor.which_wrap.clone());
    editor.set_layout(layout.unwrap_or(config.editor.layout));
    // Warning: **這四個是「寫了纔算」**，不是「寫了零就當零」：出廠值在 `Editor` 那一
    // 頭，而配置裏的 0 是「沒說」。照抄啓動時的判準，一個字都不改。
    if config.editor.indent > 0 {
        editor.set_indent(config.editor.indent);
    }
    // Warning: **1 is a page count now, not 「unset」** (2026-10-29, review: set
    // back to 1 in the panel, two pages stayed).
    if config.editor.pages_vertical > 0 {
        editor.set_pages_vertical(config.editor.pages_vertical);
    }
    editor.set_word_completion(config.editor.word_completion);
    if config.editor.pages_horizontal > 0 {
        editor.set_pages_horizontal(config.editor.pages_horizontal);
    }
    if config.editor.zong_length > 0 {
        editor.set_zong_length(config.editor.zong_length);
    }
    if config.editor.measure > 0 {
        editor.set_measure(Some(config.editor.measure));
    }
    editor.set_hard_wrap(config.editor.hard_wrap);
    editor.set_line_numbers_baseline(match config.editor.line_numbers {
        yumete_config::LineNumbers::Absolute => yumete_core::editor::LineNumbers::Absolute,
        yumete_config::LineNumbers::Relative => yumete_core::editor::LineNumbers::Relative,
        yumete_config::LineNumbers::None => yumete_core::editor::LineNumbers::None,
    });
    editor.set_paper(config.export.page.0, config.export.page.1);
    editor.set_indent_width(config.editor.indent_width);
    editor.set_tab_spaces(config.editor.tab_spaces);
    editor.set_tab_stop(config.editor.tab_width);
    editor.set_tatechuyoko(config.editor.tatechuyoko);
    editor.set_vertical_keys(config.editor.vertical_keys);
    editor.set_code_colours(config.editor.code_highlight);
    editor.set_hanging_punctuation(config.editor.hanging_punctuation);
    editor.set_soft_wrap(config.editor.soft_wrap);
    editor.set_insert_final_newline(config.editor.insert_final_newline);
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
    editor.set_reload_auto(config.editor.reload_auto);
    editor.set_smart_case(config.editor.smart_case);
    editor.set_fuzzy_search(config.editor.fuzzy_search);
    editor.set_wheel_step(config.editor.wheel_step);
    editor.set_margin(config.editor.margin);
    editor.set_word_level(config.editor.word_level);
    editor.set_segmentation_visible(config.editor.show_segmentation);
    editor.set_word_mark(config.editor.word_mark);
    if let Some(rules) = yumete_core::table::Rules::parse(&config.editor.table_rules) {
        editor.set_table_rules(rules);
    }
    editor.set_number_fill(config.editor.line_number_fill);
    editor.set_diff_gutter(config.editor.diff_gutter);
    editor.set_modeline(config.editor.modeline);
    if let Some(level) = yumete_core::problem::level_from_word(&config.editor.end_of_line_diagnostics) {
        editor.set_end_of_line_diagnostics(level);
    }
    if let Some(hint) = yumete_core::zong::IndentHint::parse(&config.editor.indent_hint) {
        editor.set_indent_hint(hint, Some(config.editor.indent_symbol.clone()));
    }
    if let Some(level) = yumete_core::editor::AutoPairs::parse(&config.editor.auto_pairs) {
        editor.set_auto_pairs(level);
    }
    editor.set_usage_groups(config.editor.usage_groups.clone());
}

/// **The settings the input method keeps**, for the same reason as [`apply`].
///
/// Warning: **碼表不在這裏。** `[ime] scheme` 換一個要讀幾十兆的表，那是一件看得見的
/// 事（`:yume-scheme` 自己會報進度），不是一次重載順手做的。重載改得動的是面板
/// 那幾格——它們是純設定，不碰磁盤。
pub fn apply_ime(config: &Config, ime: &mut crate::ImeSession) {
    ime.set_page_size(config.panel.page_size);
    // The modes, not the mode in force: that is the writer's, moved with `/`,
    // and a reload is not a reason to move it back.
    ime.set_annotation_modes(&config.ime.annotation_modes);
    ime.set_panel_display(config.panel.display);
    ime.set_inline_preedit(config.panel.inline_preedit);
    // **上屏方式也在這裏推**（2026-10-07 審出來的）。從前只有啓動那一趟推它
    // （`main.rs` 的 `load`），於是面板裏改了上屏方式、存了、`:reload-config`，
    // 屏幕上說「設定重讀了」而那一格原封不動。
    //
    // `None` 要一路傳下去，不能折成一個值：配置沒說話的時候，意思是「這個方案
    // 自己說了算」，而不是「拿拼音的整句去套每一個形碼方案」。
    ime.set_commit_strategy(
        config.ime.commit.as_deref().and_then(yumete_ime::CommitStrategy::from_str_tag),
    );
    // 字集過濾、排序與整句字集軌, picked in the panel (2026-11-04 review).
    ime.set_charsets(yumete_ime::Charsets::from_words(
        &config.ime.charset,
        &config.ime.charset_sort,
        &config.ime.lanes,
    ));
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
        // **問的是「推」那一句，不是那個名字**（2026-10-19）。從前拿名字在整個
        // `main.rs` 上搜，而那個檔裏還有 `--help` 那一大段字——`--help` 寫上
        // `:ruby-render off|basic|full` 這一行（它本來教的是一個已經退役的拼法）
        // 當場把這一條弄紅了，而没有任何東西跑回啓動那一段。釘的是命令串，命令
        // 串只出現在真去推它的地方。
        let (what, needle) = ("顯示注音", "\":ruby-render ");
        assert!(here.contains(needle), "{what} 要在這個檔裏推：{needle}");
        assert!(
            !main.contains(needle),
            "{what} 又跑回啓動那一段了（{needle}）——那樣 `:reload-config` 就推不到它"
        );
        // 這三樣兩頭都有，而且必須。前兩個要趕在第一幀／第一句話之前；上屏方式
        // 是因為啓動那一趟會把整個輸入法會話**換掉**（`ImeSession::language_only`），
        // 換完的那一個沒聽過 `apply_ime`。所以釘的是「這個檔裏也有」，不是「那邊
        // 沒有」。
        for needle in ["set_ambiguous_wide", "messages::set_language", "set_commit_strategy", "set_charsets"] {
            assert!(here.contains(needle), "{needle} 要在這個檔裏也推一遍");
        }
    }
}

#[cfg(test)]
mod config_watch_tests {
    use super::ConfigWatch;
    use yumete_core::editor::Editor;
    use yumete_core::problem::Severity;
    use yumete_core::TextStore as _;

    /// **Which files the checker owns.** Too wide and it would complain about
    /// somebody else's `config.toml`; too narrow and the one file a reader
    /// actually edits goes unchecked.
    #[test]
    fn it_knows_a_yumete_config_from_any_other_toml() {
        // **The global one is asked for, not spelt out**: where it lives
        // depends on `XDG_CONFIG_HOME` and on the platform, and a literal
        // here would pass on one machine and fail on the next.
        let global = yumete_config::config_dir().join("config.toml");
        assert!(ConfigWatch::is_a_config(&global), "{}", global.display());
        let yes = ["/book/.yumete/config.toml", ".yumete/config.toml", "/book/.yumete.toml"];
        let no = [
            "/book/Cargo.toml",
            "/book/config.toml",
            "/book/.config/config.toml",
            "/book/.yumete/keys.toml",
            "/book/notes.md",
        ];
        for one in yes {
            assert!(ConfigWatch::is_a_config(std::path::Path::new(one)), "{one}");
        }
        for one in no {
            assert!(!ConfigWatch::is_a_config(std::path::Path::new(one)), "{one}");
        }
    }

    /// **The keys a table took are said once per change** (2026-11-06), and
    /// again after a config that took none.
    #[test]
    fn taken_keys_are_said_once_for_each_change() {
        let dir = std::env::temp_dir().join(format!("yumete-taken-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = vec!["<Space>f".to_string()];
        let fb = vec!["<Space>f".to_string(), "<Space>b".to_string()];
        assert!(super::taken_keys_are_news(&f, &dir), "the first time");
        assert!(!super::taken_keys_are_news(&f, &dir), "the same keys again");
        assert!(super::taken_keys_are_news(&fb, &dir), "one more key");
        assert!(!super::taken_keys_are_news(&[], &dir), "none taken");
        assert!(super::taken_keys_are_news(&fb, &dir), "taken again after none");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`:check-config` — both files, without opening either** (2026-10-20).
    #[test]
    fn check_config_lists_every_fault_with_its_line() {
        let dir = std::env::temp_dir().join(format!("yumete-check-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("config.toml"),
            "[editor]\nindent = 50\nsyntax = \"klingon\"\n",
        )
        .unwrap();
        let mut ed = Editor::new();
        ed.set_root(&dir);

        ed.execute(":check-config").unwrap();
        super::answer_config_check(&mut ed);
        let page = ed.current_buffer().text();
        // The global config of the machine running this is listed too, and
        // what it says is not this test's business: only the project's lines
        // are counted, and a clean project opens nothing only when the global
        // one is clean as well.
        let project = dir.file_name().unwrap().to_string_lossy().to_string();
        let global = page.lines().filter(|line| !line.contains(&project)).count();
        // One line per fault, each `path:line: what`, **down the file**.
        let lines: Vec<&str> = page.lines().filter(|line| line.contains(&project)).collect();
        assert_eq!(lines.len(), 2, "{page}");
        assert!(lines[0].contains(":2: ") && lines[0].contains("indent"), "{}", lines[0]);
        // The one whose rule is `yumete-core`'s, located the same way.
        assert!(lines[1].contains(":3: ") && lines[1].contains("syntax"), "{}", lines[1]);
        assert!(ed.status().contains(&(2 + global).to_string()), "{}", ed.status());

        // **A clean config says so and opens nothing.**
        std::fs::write(dir.join(".yumete").join("config.toml"), "[editor]\nindent = 4\n").unwrap();
        let before = ed.current_buffer().id();
        ed.execute(":check-config").unwrap();
        super::answer_config_check(&mut ed);
        if global == 0 {
            assert_eq!(ed.current_buffer().id(), before, "no second page: {}", ed.status());
        } else {
            assert!(!ed.current_buffer().text().contains(&project), "{}", ed.current_buffer().text());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The error says what it costs; a warning is one key and says so.
    #[test]
    fn the_one_that_drops_the_file_says_that_it_does() {
        let fatal = ConfigWatch::faults("[editor]\nlayout = \"vertical\"\n\n[nope]\na = 1\n");
        assert_eq!(fatal.len(), 1);
        assert_eq!(fatal[0].severity, Severity::Error);
        assert_eq!(fatal[0].line, 3);
        assert!(fatal[0].message.contains("出廠設置"), "{}", fatal[0].message);
        assert_eq!(fatal[0].source.as_deref(), Some("yumete-config"));

        // **一句都不說的那一種**：文件好好的。
        assert!(ConfigWatch::faults("[editor]\nlayout = \"vertical\"\n").is_empty());

        // 這一格的規矩在 `yumete-core` 裏，`yumete-config` 看不見它——所以它走
        // 的是這個檔自己那一支 `complaints`，而行號照樣找得到。
        let core = ConfigWatch::faults("[editor]\nsyntax = \"klingon\"\n");
        assert_eq!(core.len(), 1, "{core:?}");
        assert_eq!(core[0].severity, Severity::Warn);
        assert_eq!(core[0].line, 1);
    }

    /// A session keeps the view settings the config would not give, and none
    /// that it would (2026-10-28).
    #[test]
    fn the_session_keeps_only_what_differs_from_the_config() {
        let config = yumete_config::Config::default();
        let mut ed = yumete_core::Editor::new();
        super::apply(&config, &mut ed, None, None);
        let view = |ed: &yumete_core::Editor| -> Vec<String> {
            super::view_to_keep(ed, &config)
                .into_iter()
                .filter(|line| !line.starts_with("theme"))
                .collect()
        };
        assert!(view(&ed).is_empty(), "{:?}", view(&ed));
        ed.execute(":view-focus on").unwrap();
        ed.execute(":view-pages-vertical 2").unwrap();
        ed.execute(":vertical").unwrap();
        assert_eq!(view(&ed), ["vertical", "view-pages-vertical 2", "view-focus on"]);
        ed.execute(":view-focus off").unwrap();
        assert_eq!(view(&ed), ["vertical", "view-pages-vertical 2"], "put back, not kept");

        // `:reload-config` puts the rest back too.
        ed.execute(":view-hud off").unwrap();
        super::view_back_to_config(&mut ed, &config);
        assert!(view(&ed).is_empty(), "{:?}", view(&ed));
    }

    /// **The session and the reload after the review** (2026-10-29): the
    /// layout chosen, not the one a program file shows; the wrap width kept
    /// with its switch; a reload that puts wrapping and a 竪排-only setting
    /// back although one line changes another and the page is horizontal.
    #[test]
    fn the_session_and_the_reload_after_the_review() {
        let mut config = yumete_config::Config::default();
        config.editor.soft_wrap = false;
        let view = |ed: &yumete_core::Editor, config: &yumete_config::Config| -> Vec<String> {
            super::view_to_keep(ed, config)
                .into_iter()
                .filter(|line| !line.starts_with("theme"))
                .collect()
        };
        let fresh = |config: &yumete_config::Config| {
            let mut ed = yumete_core::Editor::new();
            super::apply_to_editor(config, &mut ed, None);
            ed
        };

        // A code file in front does not make the session say `horizontal`.
        let mut vertical = config.clone();
        vertical.editor.layout = yumete_core::zong::Layout::Vertical;
        let mut ed = fresh(&vertical);
        ed.set_syntax(yumete_core::syntax::Syntax::Code(yumete_core::code::Language::Rust));
        assert!(view(&ed, &vertical).is_empty(), "{:?}", view(&ed, &vertical));

        // The width and the switch go together.
        let mut ed = fresh(&config);
        ed.execute(":view-wrap 40").unwrap();
        ed.execute(":view-wrap off").unwrap();
        let kept = view(&ed, &config);
        assert!(kept.contains(&"view-wrap 40".to_string()) && kept.contains(&"view-wrap off".to_string()), "{kept:?}");

        // The reload puts wrapping back off after a width turned it on.
        let mut ed = fresh(&config);
        ed.execute(":view-wrap 40").unwrap();
        super::view_back_to_config(&mut ed, &config);
        assert!(!ed.soft_wrap(), "wrapping back off, as the config has it");

        // …and a 竪排-only setting, on a page the config turns horizontal.
        let mut ed = fresh(&config);
        ed.execute(":vertical").unwrap();
        ed.execute(":view-sentence on").unwrap();
        assert!(ed.sentences());
        super::view_back_to_config(&mut ed, &config);
        assert!(!ed.sentences(), "put back although it needs 竪排");
        assert!(view(&ed, &config).is_empty(), "{:?}", view(&ed, &config));
    }
}

/// **The key tree read from a config** (2026-11-05): tables are groups,
/// flat runs are held, and the menus say so.
#[cfg(test)]
mod key_tree_tests {
    use yumete_config::Config;
    use yumete_core::editor::Editor;
    use yumete_core::input::Key;
    use yumete_core::TextStore as _;

    /// An editor laid with `config`, which reads without a problem, holding
    /// `one two three` with the cursor at its start.
    fn laid(config: &str) -> Editor {
        let (config, problems) = Config::from_toml_saying(config);
        assert!(problems.is_empty(), "{problems:?}");
        let mut ed = Editor::new();
        super::apply_to_editor(&config, &mut ed, None);
        ed.on_key(Key::Char('i'));
        for c in "one two three".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
        ed.on_key(Key::Home);
        ed
    }

    fn press(ed: &mut Editor, keys: &str) {
        for c in keys.chars() {
            ed.on_key(Key::Char(c));
        }
    }

    /// The menu open now, as (title, [(keys, line)]).
    fn menu(ed: &Editor) -> (String, Vec<(String, String)>) {
        let (title, rows) = ed.pending_menu().expect("a menu");
        (title, rows.into_iter().map(|(k, v)| (k.into_owned(), v)).collect())
    }

    fn line_of<'a>(rows: &'a [(String, String)], keys: &str) -> Option<&'a str> {
        rows.iter().find(|(k, _)| k == keys).map(|(_, v)| v.as_str())
    }

    /// **`Space w` with the reader's keys under it still lists `C-w`'s**,
    /// and the reader's key is `C-w`'s too: one group, two doors.
    #[test]
    fn a_table_under_space_w_keeps_the_window_keys() {
        let mut ed = laid("[keys.normal.space.w]\nz = \"select_all\"\n");
        press(&mut ed, " w");
        let (_, rows) = menu(&ed);
        for keys in ["z", "v", "q"] {
            assert!(line_of(&rows, keys).is_some(), "{keys}: {rows:?}");
        }
        ed.on_key(Key::Esc);
        ed.on_key(Key::Ctrl('w'));
        press(&mut ed, "z");
        assert_eq!(ed.selection(), (0, 13), "C-w z is Space w z");
    }

    /// **A group moved to another key keeps the reader's keys**, a table of
    /// theirs in it included.
    #[test]
    fn a_moved_space_keeps_the_readers_tables() {
        let mut ed = laid(
            "[keys.normal]\n\";\" = \"@ \"\n\"<space>\" = \"@l\"\n\n[keys.normal.space.b]\nlabel = \"buffer\"\nd = \"select_all\"\n",
        );
        press(&mut ed, " ");
        assert_eq!(ed.cursor(), 1, "Space moves right");
        press(&mut ed, ";");
        let (_, rows) = menu(&ed);
        assert_eq!(line_of(&rows, "b"), Some("+buffer"), "{rows:?}");
        press(&mut ed, "bd");
        assert_eq!(ed.selection(), (0, 13));
    }

    /// **What a group's table gets wrong is said** (review, 2026-11-05): a
    /// `label` that is not text, one key written twice, and a table with no
    /// keys in it.
    #[test]
    fn a_groups_mistakes_are_said() {
        let (_, problems) = Config::from_toml_saying("[keys.normal.space.b]\nlabel = 3\n");
        assert!(problems.iter().any(|p| p.contains("label 只能是一段文字")), "{problems:?}");

        let twice = "[keys.normal]\n\"<space>bx\" = \"move_line_down\"\n\n[keys.normal.space.b]\nx = \"move_line_up\"\n";
        let (_, problems) = Config::from_toml_saying(twice);
        assert!(problems.iter().any(|p| p.contains("[keys.normal.space.b] x 與 [keys.normal] <space>bx 是同一個鍵")), "{problems:?}");

        let ed = laid("[keys.normal.space.b]\ne = { label = \"no run\" }\nf = { run = \":write\", label = \"save\" }\n");
        let notes = ed.key_notes().join(" ");
        assert!(notes.contains("<Space>be 是空的鍵組"), "{notes}");
        let ed = laid("[keys.normal.space.w]\nlabel = \"windows\"\n");
        assert!(!ed.key_notes().join(" ").contains("空的鍵組"), "a built-in group renamed is not hollow");
    }

    /// **A table grid's group that takes a key is said too** (review,
    /// 2026-11-06): `Q` replays a macro, and `[keys.table.Q]` takes it there.
    #[test]
    fn a_table_grids_group_says_the_key_it_took() {
        let ed = laid("[keys.table.Q]\nl = \"move_char_right\"\n");
        assert!(ed.taken_keys().iter().any(|k| k == "Q"), "{:?}", ed.taken_keys());
    }

    /// **A moved Space menu says the key that opened it**, and a named key
    /// in a menu reads as the hint row writes it (review, 2026-11-06).
    #[test]
    fn a_moved_space_echoes_its_own_key_and_tab_reads_tab() {
        let mut ed = laid("[keys.normal]\n\";\" = \"space_mode\"\n\n[keys.normal.space.tab]\nlabel = \"tabs\"\nn = \":new\"\n");
        press(&mut ed, ";");
        assert_eq!(ed.typed_so_far(), ";");
        let (title, rows) = menu(&ed);
        assert_eq!(title, ";");
        assert_eq!(line_of(&rows, "Tab"), Some("+tabs"), "{rows:?}");
        ed.on_key(Key::Tab);
        assert_eq!(ed.typed_so_far(), ";Tab");
        ed.on_key(Key::Esc);
        press(&mut ed, " ");
        assert_eq!(ed.typed_so_far(), "␣", "Space itself still says Space");
    }

    /// **A count typed in a group is in the key echo** (review, 2026-11-05):
    /// `Space a b 3` says `␣ab3`, and `Backspace` takes the 3 back.
    #[test]
    fn a_count_in_a_group_is_echoed() {
        let mut ed = laid("[keys.normal.space.a.b]\nl = \"move_char_right\"\n");
        press(&mut ed, " ab3");
        assert_eq!(ed.typed_so_far(), "␣ab3");
        ed.on_key(Key::Backspace);
        assert_eq!(ed.typed_so_far(), "␣ab");
        press(&mut ed, "2l");
        assert_eq!(ed.cursor(), 2, "Space a b 2 l");
    }

    /// **A group with nothing in it is no group** (review, 2026-11-05):
    /// emptied by `inherit = false`, or holding only `no_op`, it closes as
    /// it opens, and the Space menu does not list it.
    #[test]
    fn an_empty_group_closes_as_it_opens() {
        for config in [
            "[keys.normal.space.t]\ninherit = false\n",
            "[keys.normal.space.t]\ninherit = false\nx = \"no_op\"\n",
        ] {
            let mut ed = laid(config);
            press(&mut ed, " ");
            let (_, rows) = menu(&ed);
            assert!(line_of(&rows, "t").is_none(), "{config}: {rows:?}");
            press(&mut ed, "t");
            assert!(ed.pending_menu().is_none(), "{config}: Space t waits for nothing");
            press(&mut ed, "l");
            assert_eq!(ed.cursor(), 1, "{config}: l is l again");
        }
        let mut ed = laid("[keys.normal.Q]\nj = \"no_op\"\n");
        press(&mut ed, "Ql");
        assert_eq!(ed.cursor(), 1, "Q opened nothing, and l moved");
    }

    /// **`[keys.normal.space]` is the Space group's beside a flat
    /// `"<space>v"`** (review, 2026-11-05): with the Space menu on `\`, the
    /// flat run held the Space key, and the table's keys landed on it too.
    #[test]
    fn a_space_table_follows_the_group_beside_a_flat_space_run() {
        let mut ed = laid(
            "[keys]\npreset = \"vim\"\nflavor = \"off\"\n\n[keys.normal]\n\"<space>v\" = \"select_all\"\n\n[keys.normal.space]\nq = \"goto_line_end\"\n",
        );
        press(&mut ed, "\\");
        let (_, rows) = menu(&ed);
        assert!(line_of(&rows, "q").is_some(), "{rows:?}");
        press(&mut ed, "q");
        assert_eq!(ed.cursor(), 12, "\\ q");
        ed.on_key(Key::Home);
        press(&mut ed, " v");
        assert_eq!(ed.selection(), (0, 13), "Space v is still the flat run");
    }

    /// **Under vim with Space moving right, `\` is the Space group**, and
    /// `[keys.normal.space.b]` is reached by `\ b`.
    #[test]
    fn backslash_reaches_the_space_tables_under_vim() {
        let mut ed = laid(
            "[keys]\npreset = \"vim\"\nflavor = \"off\"\n\n[keys.normal.space.b]\nlabel = \"buffer\"\nd = \"select_all\"\n",
        );
        press(&mut ed, "\\b");
        let (title, rows) = menu(&ed);
        assert_eq!(title, "buffer");
        assert!(line_of(&rows, "d").is_some(), "{rows:?}");
        press(&mut ed, "d");
        assert!(ed.pending_menu().is_none());
        assert!(ed.selection().1 >= 12, "{:?}", ed.selection());
    }

    /// **A table takes its key, and a key its group lacks closes it**: with
    /// LazyVim's `Space f`, `Space f x` opens no picker, and the startup
    /// list says once what the table took.
    #[test]
    fn a_table_takes_its_key_and_says_so() {
        let mut ed = laid("[keys.normal.space.f]\nlabel = \"file/find\"\nn = \":new\"\n");
        press(&mut ed, " f");
        let (title, _) = menu(&ed);
        assert_eq!(title, "file/find");
        press(&mut ed, "x");
        assert!(ed.pending_menu().is_none(), "x closed the group");
        assert_eq!(ed.current_buffer().text(), "one two three", "and typed nothing");
        assert_eq!(ed.cursor(), 0, "nor moved");
        let notes = ed.key_notes();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("<Space>f"), "{notes:?}");
        // Several, and deeper ones, on one line by their whole path.
        let ed = laid("[keys.normal.space.f]\nn = \":new\"\n[keys.normal.space.b]\nd = \":buffer-close\"\n");
        let notes = ed.key_notes();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("<Space>f") && notes[0].contains("<Space>b"), "{notes:?}");
    }

    /// **A run written flat keeps the keys before it**: `"<space>bd"` holds
    /// after `Space b`, and a key that does not go on lets Space b's own
    /// meaning act, the reader's when they bound it.
    #[test]
    fn a_flat_run_holds_and_lets_go() {
        let mut ed = laid("[keys.normal]\n\"<space>bd\" = \"select_all\"\n\"<space>b\" = \"move_char_right\"\n");
        press(&mut ed, " bl");
        assert_eq!(ed.cursor(), 2, "Space b was the reader's, then l");
        press(&mut ed, " bd");
        assert_eq!(ed.selection(), (0, 13));
    }

    /// **`label` alone renames a built-in group**, on its parent's row as
    /// `+name` and as the title of its own menu.
    #[test]
    fn a_label_alone_renames_a_group() {
        let mut ed = laid("[keys.normal.space.w]\nlabel = \"windows\"\n");
        press(&mut ed, " ");
        let (_, rows) = menu(&ed);
        assert_eq!(line_of(&rows, "w"), Some("+windows"), "{rows:?}");
        press(&mut ed, "w");
        let (title, rows) = menu(&ed);
        assert_eq!(title, "windows");
        assert!(line_of(&rows, "v").is_some(), "its keys stay: {rows:?}");
    }

    /// **Backspace goes back a level** where the group has none of its own,
    /// and Space's own `⌫` still turns the page.
    #[test]
    fn backspace_goes_back_a_level() {
        let mut ed = laid("[keys.normal.space.g]\nlabel = \"git\"\n\n[keys.normal.space.g.h]\nlabel = \"hunks\"\np = \"show_change\"\n");
        press(&mut ed, " gh");
        assert_eq!(menu(&ed).0, "hunks");
        ed.on_key(Key::Backspace);
        assert_eq!(menu(&ed).0, "git");
        ed.on_key(Key::Backspace);
        assert!(menu(&ed).1.iter().any(|(k, _)| k == "f"), "back in Space");
        ed.on_key(Key::Esc);
        // Space w, then back to Space.
        press(&mut ed, " w");
        ed.on_key(Key::Backspace);
        assert!(menu(&ed).1.iter().any(|(k, v)| k == "w" && v.starts_with('+')), "{:?}", menu(&ed));
        // Esc closes every level.
        press(&mut ed, "gh");
        ed.on_key(Key::Esc);
        assert!(ed.pending_menu().is_none());
    }

    /// **The menu keeps the config's order**; a key of the reader's over a
    /// built-in one keeps that row's place, and a command row says its own
    /// text.
    #[test]
    fn the_menu_keeps_the_configs_order() {
        let mut ed = laid(
            r#"[keys.normal.space]
z = "select_all"
"," = "buffer_picker"
f = { run = ":new", label = "scratch" }
q = ":quit"
"#,
        );
        press(&mut ed, " ");
        let (_, rows) = menu(&ed);
        let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys.first(), Some(&"f"), "f keeps its place: {keys:?}");
        assert_eq!(line_of(&rows, "f"), Some("scratch"));
        let at = |k: &str| keys.iter().position(|one| *one == k).unwrap_or(usize::MAX);
        let mut plain = laid("");
        press(&mut plain, " ");
        let factory: Vec<String> = menu(&plain).1.into_iter().map(|(k, _)| k).collect();
        let was = factory.iter().position(|k| k == "z").expect("the input method's row");
        assert_eq!(at("z"), was, "z keeps the input method's place: {keys:?}");
        assert_ne!(line_of(&rows, "z"), menu(&plain).1.iter().find(|(k, _)| k == "z").map(|(_, v)| v.as_str()));
        assert!(at(",") < at("q"), "the new ones in the file's order: {keys:?}");
        assert!(at("q") == keys.len() - 1, "{keys:?}");
    }

    /// **`sticky = true` keeps a group open**, and `inherit = false` takes
    /// a built-in group's own keys away.
    #[test]
    fn sticky_and_inherit() {
        let mut ed = laid("[keys.normal.space.h]\nlabel = \"hydra\"\nsticky = true\nl = \"move_char_right\"\n");
        press(&mut ed, " hll");
        assert_eq!(ed.cursor(), 2);
        assert_eq!(menu(&ed).0, "hydra", "still open");
        ed.on_key(Key::Esc);
        assert!(ed.pending_menu().is_none());

        let mut ed = laid("[keys.normal.g]\ninherit = false\nx = \"goto_last_line\"\n");
        press(&mut ed, "g");
        let (_, rows) = menu(&ed);
        assert_eq!(rows.len(), 1, "only the reader's: {rows:?}");
        press(&mut ed, "e");
        assert!(ed.pending_menu().is_none(), "g e is gone");
    }

    /// **A group left with no keys leaves its parent's menu**: `no_op` on
    /// each key of the merge-conflict group, or on the group's own key.
    #[test]
    fn an_empty_group_leaves_the_menu() {
        let mut ed = laid("[keys.normal.space.m]\no = \"no_op\"\nt = \"no_op\"\nb = \"no_op\"\n");
        press(&mut ed, " ");
        assert!(line_of(&menu(&ed).1, "m").is_none(), "{:?}", menu(&ed).1);
        let mut ed = laid("[keys.normal.space]\nw = \"no_op\"\n");
        press(&mut ed, " ");
        assert!(line_of(&menu(&ed).1, "w").is_none());
    }

    /// **The LazyVim page's config reads without a problem**, and its groups
    /// are `+name` rows on the Space menu.
    #[test]
    fn the_lazyvim_config_reads_and_shows_its_groups() {
        // CI checks the docs out beside the repo, as the site tests read them.
        let docs = match std::env::var_os("YUMETE_DOCS") {
            Some(dir) => std::path::PathBuf::from(dir),
            None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../yumete-docs"),
        };
        let page = std::fs::read_to_string(docs.join("src/from/lazyvim.md")).expect("the docs site beside yumete, with from/lazyvim.md");
        let mut config = String::new();
        let mut inside = false;
        for line in page.lines() {
            match (inside, line.trim()) {
                (false, "```toml") => inside = true,
                (true, "```") => inside = false,
                (true, _) => {
                    config.push_str(line);
                    config.push('\n');
                }
                _ => {}
            }
        }
        let mut ed = laid(&config);
        press(&mut ed, " ");
        let (_, rows) = menu(&ed);
        for (keys, name) in [("b", "+buffer"), ("c", "+code"), ("f", "+file/find"), ("g", "+git"), ("s", "+search"), ("Tab", "+tabs")] {
            assert_eq!(line_of(&rows, keys), Some(name), "{rows:?}");
        }
        press(&mut ed, "g");
        assert_eq!(line_of(&menu(&ed).1, "h"), Some("+hunks"));
    }

    /// **A flat `"<space>…"` is the Space key**, as in 0.18: where Space is
    /// not the Space menu (vim at flavour off, `vim_space_menu = false`, a
    /// Space of the reader's) the run holds Space and waits. Only
    /// `[keys.normal.space]` is the Space group wherever it is opened.
    #[test]
    fn a_flat_space_run_is_the_space_key() {
        let flat = "[keys.normal]\n\"<space>x\" = \"select_all\"\n\"<space>ff\" = \"goto_line_end\"\n";
        for head in [
            "[keys]\npreset = \"vim\"\nflavor = \"off\"\n",
            "[keys]\npreset = \"vim\"\n[keys.flavor_items]\nvim_space_menu = false\n",
            "",
        ] {
            let mut ed = laid(&format!("{head}{flat}"));
            press(&mut ed, " x");
            assert_eq!(ed.selection(), (0, 13), "{head}");
            assert_eq!(ed.current_buffer().text(), "one two three", "nothing deleted: {head}");
            ed.on_key(Key::Esc);
            ed.on_key(Key::Home);
            press(&mut ed, " ff");
            assert!(ed.cursor() >= 12, "{head}: {}", ed.cursor());
        }
        // Under vim at flavour off `\` is the Space group, and a flat run
        // of the Space key is not in it.
        let mut ed = laid(&format!("[keys]\npreset = \"vim\"\nflavor = \"off\"\n{flat}"));
        press(&mut ed, "\\");
        let (_, rows) = menu(&ed);
        let mut helix = laid(flat);
        press(&mut helix, " ");
        let says = line_of(&menu(&helix).1, "x").expect("x in helix's Space menu").to_string();
        assert!(rows.iter().all(|(_, v)| *v != says), "{rows:?}");
        // A Space of the reader's acts first when the run does not go on.
        let mut ed = laid("[keys.normal]\n\" \" = \"move_char_right\"\n\"<space>x\" = \"goto_line_end\"\n");
        press(&mut ed, " x");
        assert!(ed.cursor() >= 12, "the run: {}", ed.cursor());
        ed.on_key(Key::Home);
        press(&mut ed, " l");
        assert_eq!(ed.cursor(), 2, "Space, then l");
        // The table is the group, and `\` reaches it.
        let mut ed = laid("[keys]\npreset = \"vim\"\nflavor = \"off\"\n[keys.normal.space]\nx = \"select_all\"\n");
        press(&mut ed, "\\x");
        assert_eq!(ed.selection(), (0, 13));
    }

    /// **Two spellings into one group** (`"C-wz"`, `"<space>wz"`): the one
    /// later in the file wins, and startup says which it replaced.
    #[test]
    fn of_two_spellings_into_one_group_the_later_wins() {
        for (config, select) in [
            ("\"<space>wz\" = \"goto_file_end\"\n\"C-wz\" = \"select_all\"\n", true),
            ("\"C-wz\" = \"select_all\"\n\"<space>wz\" = \"goto_file_end\"\n", false),
        ] {
            let mut ed = laid(&format!("[keys.normal]\n{config}"));
            ed.on_key(Key::Ctrl('w'));
            press(&mut ed, "z");
            assert_eq!(ed.selection() == (0, 13), select, "{config}");
            ed.on_key(Key::Esc);
            ed.on_key(Key::Home);
            press(&mut ed, " wz");
            assert_eq!(ed.selection() == (0, 13), select, "{config}");
            let notes = ed.key_notes();
            let (kept, lost) = match select {
                true => ("<C-w>z", "<Space>wz"),
                false => ("<Space>wz", "<C-w>z"),
            };
            assert_eq!(notes.len(), 1, "{notes:?}");
            let at = |path: &str| notes[0].find(path);
            assert!(at(kept).is_some_and(|k| at(lost).is_some_and(|l| k < l)), "{notes:?}");
        }
        // vim's two doors into Space, `\` and Space itself.
        for (config, select) in [
            ("\"<space>x\" = \"goto_line_end\"\n\"\\\\x\" = \"select_all\"\n", true),
            ("\"\\\\x\" = \"select_all\"\n\"<space>x\" = \"goto_line_end\"\n", false),
        ] {
            let mut ed = laid(&format!("[keys]\npreset = \"vim\"\n[keys.normal]\n{config}"));
            press(&mut ed, "\\x");
            let (from, to) = ed.selection();
            assert_eq!(from == 0 && to >= 12, select, "{config}: {from}..{to}");
            assert_eq!(ed.key_notes().len(), 1, "{config}");
        }
    }

    /// **A group stays the same group when the tree in force changes**:
    /// a sticky group whose key enters select mode keeps its own keys, not
    /// those of the node at its place in select mode's tree.
    #[test]
    fn a_sticky_group_survives_select_mode() {
        let mut ed = laid(
            "[keys.normal.space.h]\nlabel = \"hop\"\nsticky = true\nv = \"select_mode\"\nl = \"move_char_right\"\n\n[keys.select.\",\"]\nlabel = \"commasel\"\nx = \"select_all\"\n",
        );
        press(&mut ed, " hv");
        assert_eq!(menu(&ed).0, "hop");
        press(&mut ed, "l");
        assert_eq!(menu(&ed).0, "hop", "still the group");
        assert_eq!(ed.cursor(), 1, "l moved");
    }

    /// **A deep group's menu costs one look at each level**: twenty-four
    /// unnamed levels used to cost 2^24 menus.
    #[test]
    fn a_deep_menu_is_drawn_at_once() {
        let mut config = String::new();
        let mut path = String::from("[keys.normal.space.q");
        for _ in 0..24 {
            config.push_str(&format!("{path}]\nx = \"select_all\"\ny = \"goto_file_end\"\n"));
            path.push_str(".q");
        }
        let mut ed = laid(&config);
        let started = std::time::Instant::now();
        press(&mut ed, " ");
        let (_, rows) = menu(&ed);
        assert!(line_of(&rows, "q").is_some_and(|l| l.starts_with('+')), "{rows:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
    }

    /// **A label on vim's `Z` table titles the held `Z`'s menu.**
    #[test]
    fn a_label_on_vims_z_titles_its_menu() {
        let mut ed = laid("[keys]\npreset = \"vim\"\n[keys.normal.Z]\nlabel = \"zed\"\nx = \"goto_file_end\"\n");
        press(&mut ed, "Z");
        let (title, rows) = menu(&ed);
        assert_eq!(title, "zed");
        assert!(line_of(&rows, "x").is_some(), "{rows:?}");
    }

    /// **The startup note**: a `,` path is quoted, and a group a later
    /// line took back is not claimed.
    #[test]
    fn the_note_quotes_a_comma_and_forgets_a_group_taken_back() {
        let ed = laid("[keys.normal.\",\"]\nx = \"select_all\"\n[keys.normal.space.f]\nn = \":new\"\n");
        let notes = ed.key_notes();
        assert!(notes[0].contains("\",\", <Space>f"), "{notes:?}");
        let ed = laid("[keys]\npreset = \"vim\"\n[keys.normal.space.f]\nn = \":new\"\n[keys.vim.normal]\n\"<space>f\" = \"file_picker\"\n");
        assert!(ed.key_notes().is_empty(), "{:?}", ed.key_notes());
    }

    /// **The sign shows the whole run held** (`␣b` of `Space bd`), and
    /// Backspace takes a digit typed in a group back before the level.
    #[test]
    fn the_sign_shows_a_held_run_and_backspace_takes_a_digit() {
        let mut ed = laid("[keys.normal]\n\"<space>bd\" = \"select_all\"\n");
        press(&mut ed, " b");
        assert_eq!(ed.typed_so_far(), "␣b");
        let mut ed = laid("[keys.normal.space.g]\nlabel = \"git\"\n[keys.normal.space.g.h]\nlabel = \"hunks\"\nl = \"move_char_right\"\n");
        press(&mut ed, " gh3");
        ed.on_key(Key::Backspace);
        assert_eq!(menu(&ed).0, "hunks", "the digit went, not the level");
        press(&mut ed, "l");
        assert_eq!(ed.cursor(), 1, "and with it the count");
    }
}
