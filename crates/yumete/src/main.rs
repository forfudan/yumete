//! `yumete` — the binary entry point.
//!
//! yumete parses command-line arguments, opens the given file(s) into the
//! editor (or starts a new scratch buffer when none are given), and launches the
//! interactive terminal editor. When standard output is not a terminal, or with
//! `--preview`, it instead prints a non-interactive preview of the active
//! buffer (useful for piping and for quick inspection).

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use yumete_config::Layout;
use yumete_core::input::Key;
use yumete_core::search_panel::{Case, Where};
use yumete_core::{Editor, TextStore};
use yumete_ime::{CommitStrategy, ImeSession, Scheme};

/// What this binary is, exactly — stamped by `build.rs` at compile time.
///
/// `0.1.0-dev.20260905123000+bbc1485.dirty`: the version, when it was built,
/// the commit it was built from, and whether that commit was the whole truth.
/// A release build (`YUMETE_RELEASE=1`) is the bare `0.1.0`.
const VERSION: &str = env!("YUMETE_VERSION");

fn main() -> ExitCode {
    let mut files: Vec<String> = Vec::new();
    let mut force_preview = false;
    let mut force_layout: Option<Layout> = None;
    let mut force_table = false;
    // `--timing` answers「開個檔案怎麼要三秒」 without guessing: it runs the
    // whole of a launch and prints what each part of it cost.
    let mut timing = false;
    // Which markup the files named on this line are written in. The config's
    // `[syntax]` says it for a project and the extension says it for a file;
    // this says it for one run, which is what you want when the file is
    // called `.txt` and you know what is in it.
    let mut force_syntax: Option<String> = None;
    // `--shot` prints one frame — the page as it would be drawn — and exits.
    // `:shot` needs a window, a GUI session and a person; this is the same
    // picture for a headless machine, a bug report, or a reviewer who has to
    // *see* the layout rather than read an assertion about it.
    let mut shot: Option<(u16, u16)> = None;
    // Text, or the same frame with its colours as HTML — a theme is judged on
    // what it looks like, and a terminal is not always there to look at.
    let mut shot_html = false;
    // `--readonly` opens the session locked (Feature #213): a reference 碼表,
    // somebody else's manuscript, a file you came to read. Every buffer this
    // run opens is locked, not only the ones named here — `:open` opens them
    // too, and a session started to read should not go writable at the second
    // file.
    let mut readonly = false;
    // `--new` says no to picking up where you left off. Opening the editor to
    // jot one thing down and getting five chapters back is the same annoyance
    // as the reverse, in the other direction.
    let mut fresh = false;
    // `-c`: reopen last time's files even though `session` is off (the default).
    let mut resume = false;
    let mut tutor = false;
    // Keys to press before the picture is taken. A panel that only opens after
    // three keystrokes — the `:` menu, `::`, which-key, the候選 list — could
    // not be looked at without a terminal and a pair of hands, and 「動了前端
    // 就出一張圖看看」 is the rule that catches what no assertion does.
    let mut keys: Option<String> = None;
    // **The language the editor says things in, for one run** (#494). The
    // config's `[editor] language` is the standing answer; this is the flag,
    // because 「打開編輯器，然後去找哪個設定能改語言」 is exactly the loop a
    // reader who cannot read the current language is stuck in.
    let mut force_language: Option<yumete_core::messages::Language> = None;
    // **管道那一邊**（2026-10-03 作者定：「先做管道吧」）。`ye --grep 霜` 把
    // `檔:行:列:文字` 印到 stdout 就退出——找到回 0，一處都沒有回 1，所以
    // `if ye --grep …` 在腳本裏是一句話。
    let mut grep: Option<String> = None;
    // `ye --files jia` 列出名字配得上的檔——fd 那一半，而它也認拼音。
    let mut listing: Option<String> = None;
    let mut g = Grep::default();
    // **還欠着一個值的那個旗標**（2026-10-03 一輪審查報來的）。
    //
    // Warning: **值收不到要當場死**，而且三處從前都不會。`want_syntax`／`want_grep` 那
    // 幾個閂只在下一輪循環裏看一眼，走完就沒人問了：`ye --grep` 於是悄悄落空，
    // 退出碼 0，而且**開起了編輯器**。這一格記着「誰在等」，循環走完還在等就是
    // 使用者少打了一個詞。
    let mut owed: Option<&'static str> = None;
    // **這幾個開關只有配上 `--grep`／`--files` 纔有意義。** 單獨給是打錯了，不是
    // 「沒關係」——`ye --hidden` 從前悄悄開了編輯器。
    let mut modifiers: Vec<&'static str> = Vec::new();

    for arg in std::env::args().skip(1) {
        if let Some(which) = owed.take() {
            // Warning: **值不許長得像旗標。** `ye --grep --hidden alpha` 從前把
            // `--hidden` 當成了要找的詞，再把 `alpha` 當成目錄——然後說
            // 「alpha 不是一個目錄」。少打一個詞的人看不懂那句話。
            if arg.starts_with('-') && arg != "-" {
                eprintln!("yumete: {which} wants a word, not another option ({arg:?})");
                eprintln!("try 'yumete --help'");
                return ExitCode::from(2);
            }
            match which {
                "--syntax" => force_syntax = Some(arg),
                "--grep" => grep = Some(arg),
                "--files" => listing = Some(arg),
                _ => unreachable!("every owed flag is named here"),
            }
            continue;
        }
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("yumete {VERSION}");
                return ExitCode::SUCCESS;
            }
            // Warning: **舊的那八個短名撤了**（2026-10-03 作者定：「舊的讓位……等到
            // 全部弄好了再看哪些值得 short alias」）。字母留給了搜索那一邊：今天
            // 有 `-G`（`--grep`）與 `-O`（`--open`），串得起來（見底下那一條分支），
            // 加上 `-h`、`-V` 這兩個所有命令行的通例。
            "--preview" => force_preview = true,
            "--table" => force_table = true,
            "--readonly" => readonly = true,
            "--new" => fresh = true,
            "--continue" => resume = true,
            // **管道那一邊。** 開關的名字照 rg，因為那是肌肉記憶所在。
            "--grep" => owed = Some("--grep"),
            s if s.starts_with("--grep=") => grep = Some(s["--grep=".len()..].to_string()),
            "--files" => owed = Some("--files"),
            s if s.starts_with("--files=") => listing = Some(s["--files=".len()..].to_string()),
            "--ignore-case" => {
                g.case = Some(Case::Insensitive);
                modifiers.push("--ignore-case");
            }
            "--case-sensitive" => {
                g.case = Some(Case::Sensitive);
                modifiers.push("--case-sensitive");
            }
            "--word" => {
                g.word = true;
                modifiers.push("--word");
            }
            "--regex" => {
                g.regex = true;
                modifiers.push("--regex");
            }
            "--fixed" => {
                g.regex = false;
                modifiers.push("--fixed");
            }
            "--fuzzy" => {
                g.fuzzy = true;
                modifiers.push("--fuzzy");
            }
            // yumete 把「隱藏檔」和「`.gitignore` 裏的」放在**同一個開關**上
            // （面板那一格寫着「不搜 [隱藏+忽略]」），所以 rg 那兩個名字都撥它。
            "--hidden" | "--no-ignore" => {
                g.hidden = true;
                modifiers.push("--hidden");
            }
            s if s.starts_with("--glob=") => {
                g.include = s["--glob=".len()..].to_string();
                modifiers.push("--glob");
            }
            s if s.starts_with("--exclude=") => {
                g.exclude = s["--exclude=".len()..].to_string();
                modifiers.push("--exclude");
            }
            s if s.starts_with("--chinese=") => match s["--chinese=".len()..].as_ref() {
                "off" => {
                    g.chinese = (false, false);
                    modifiers.push("--chinese");
                }
                "glyphs" => {
                    g.chinese = (true, false);
                    modifiers.push("--chinese");
                }
                "pinyin" => {
                    g.chinese = (false, true);
                    modifiers.push("--chinese");
                }
                "both" => {
                    g.chinese = (true, true);
                    modifiers.push("--chinese");
                }
                other => {
                    eprintln!("yumete: --chinese: no such setting {other:?}");
                    eprintln!("try off, glyphs, pinyin or both");
                    return ExitCode::from(2);
                }
            },
            // **缺省搜當前目錄**，和 rg 一樣；這一個往上搜到項目的根。
            "--project" => {
                g.project = true;
                modifiers.push("--project");
            }
            "--open" => {
                g.open = true;
                modifiers.push("--open");
            }
            "--heading" => {
                g.heading = Some(true);
                modifiers.push("--heading");
            }
            "--no-heading" => {
                g.heading = Some(false);
                modifiers.push("--heading");
            }
            s if s.starts_with("--color=") => match s["--color=".len()..].as_ref() {
                "always" => {
                    g.colour = Some(true);
                    modifiers.push("--color");
                }
                "never" => {
                    g.colour = Some(false);
                    modifiers.push("--color");
                }
                "auto" => {
                    g.colour = None;
                    modifiers.push("--color");
                }
                other => {
                    eprintln!("yumete: --color: no such setting {other:?}");
                    eprintln!("try auto, always or never");
                    return ExitCode::from(2);
                }
            },
            // **`-t` is already the table**, so this one is long only. Worth a
            // flag at all because the lesson is what a first run wants, and
            // 「open the editor, then find out how to ask for the lesson」 is
            // the loop it exists to break.
            "--tutor" => tutor = true,
            "--timing" => timing = true,
            "--shot" => shot = Some((100, 30)),
            s if s.starts_with("--shot=") => match parse_size(&s["--shot=".len()..]) {
                Ok(size) => shot = Some(size),
                Err(why) => {
                    eprintln!("yumete: {why}");
                    eprintln!("try 'yumete --help'");
                    return ExitCode::from(2);
                }
            },
            "--html" => shot_html = true,
            s if s.starts_with("--keys=") => keys = Some(s["--keys=".len()..].to_string()),
            "--syntax" => owed = Some("--syntax"),
            s if s.starts_with("--syntax=") => {
                force_syntax = Some(s["--syntax=".len()..].to_string())
            }
            s if s.starts_with("--lang=") => {
                let want = &s["--lang=".len()..];
                match yumete_core::messages::Language::parse(want) {
                    Some(language) => force_language = Some(language),
                    None => {
                        eprintln!("yumete: --lang: no language called {want:?}");
                        eprintln!("try zh (繁體), zhs (简体) or en");
                        return ExitCode::from(2);
                    }
                }
            }
            "--vertical" => force_layout = Some(Layout::Vertical),
            "--horizontal" => force_layout = Some(Layout::Horizontal),
            // Warning: **「要一個值」和「不認得」是兩句話**（2026-10-03 一輪審查報來的）。
            // `ye --grep 霜 --chinese off` 從前答「unknown option '--chinese'」
            // ——那個旗標認得，不認得的是這個寫法。連倉裏的文檔自己都寫過
            // `--chinese off`。
            "--chinese" | "--glob" | "--exclude" | "--color" | "--lang" | "--keys" => {
                eprintln!("yumete: {arg} is written with an equals sign, as in {arg}=…");
                eprintln!("try 'yumete --help'");
                return ExitCode::from(2);
            }
            // 今天撤掉的那八個短名——說出它變成了什麼，別只說「不認得」。
            "-t" | "-v" | "-R" | "-c" | "-n" | "-p" | "-s" | "-H" => {
                let long = match arg.as_str() {
                    "-t" => "--table",
                    "-v" => "--vertical",
                    "-R" => "--readonly",
                    "-c" => "--continue",
                    "-n" => "--new",
                    "-p" => "--preview",
                    "-s" => "--syntax",
                    _ => "--horizontal",
                };
                eprintln!("yumete: '{arg}' is gone; it is spelt '{long}' now");
                eprintln!("single letters are being saved for the search flags");
                return ExitCode::from(2);
            }
            // **短名串得起來，帶值的那個把值留給下一個詞**（2026-10-03 作者定）。
            //
            // `-G zhongguo` 找詞、`-GO zhongguo` 找完直接開編輯器。能這麼串，靠的
            // 正是作者定的那條「參數必須空一格」：getopt 的規矩裏值可以貼在字母
            // 後面，於是 `-GO x` 只能讀成「`-G`，值是 `O`」；**貼寫這條路一堵死，
            // `O` 就只可能是另一個字母**，`-GO` 再沒有第二種讀法。代價兩個，都在
            // 下面當場報錯：貼着寫（`-Gzhongguo`），以及一串裏兩個帶值的。
            //
            // Warning: **入口用大寫，小寫那一整排留着**（2026-10-03 作者定）。
            // `-g` 在 rg 與 fd 裏都是 `--glob`、`-o` 在 rg 裏是 `--only-matching`
            // ——花掉它們，將來這幾個開關就沒有天然的字母了。這兩個又不是開關：
            // 它們決定整個程序問哪一個問題。大寫把這個區別寫在臉上，將來
            // `ye -Giw zhongguo` 裏大寫是入口、小寫是開關，一眼讀得出來。
            // `--files` 的字母先空着，等它的名字定下來再挑。
            s if s.len() > 1 && s.starts_with('-') && !s.starts_with("--") => {
                let letters: Vec<char> = s[1..].chars().collect();
                let mut wants: Option<&'static str> = None;
                for (i, c) in letters.iter().enumerate() {
                    let long = match c {
                        'G' => "--grep",
                        'O' => "--open",
                        _ => {
                            // **第一個字母帶值、後面卻不是認得的字母**——那多半不是
                            // 「不認得的開關」，而是把要找的詞貼在了後面。說清楚是
                            // 哪一件事，別讓人對着 `unknown option '-z'` 發愣。
                            if i > 0 && letters[0] == 'G' {
                                let word = &s[2..];
                                eprintln!(
                                    "yumete: -G wants its word in the next argument: -G {word}"
                                );
                            } else if matches!(c, 'g' | 'o') {
                                let up = c.to_ascii_uppercase();
                                eprintln!("yumete: '-{c}' is spelt '-{up}' here");
                                eprintln!(
                                    "the lower-case letters are kept for the switches rg spells the same way"
                                );
                            } else if *c == 'f' {
                                eprintln!("yumete: --files has no short name yet");
                                eprintln!("try 'yumete --help'");
                            } else {
                                eprintln!("yumete: unknown option '-{c}' (in '{s}')");
                                eprintln!("try 'yumete --help'");
                            }
                            return ExitCode::from(2);
                        }
                    };
                    match long {
                        "--open" => {
                            g.open = true;
                            modifiers.push("--open");
                        }
                        _ => match wants {
                            Some(already) if already == long => {}
                            Some(already) => {
                                eprintln!(
                                    "yumete: {already} and {long} each want a word; one to a bundle"
                                );
                                eprintln!("try 'yumete --help'");
                                return ExitCode::from(2);
                            }
                            None => wants = Some(long),
                        },
                    }
                }
                if let Some(long) = wants {
                    owed = Some(long);
                }
            }
            // Reject unknown flags, but treat a lone "-" as a filename.
            s if s.starts_with('-') && s != "-" => {
                eprintln!("yumete: unknown option '{s}'");
                eprintln!("try 'yumete --help'");
                return ExitCode::from(2);
            }
            s => files.push(s.to_string()),
        }
    }

    // **參數層的四道閘**（2026-10-03 一輪審查報來的，四條都真按得出來）。
    if let Some(which) = owed {
        eprintln!("yumete: {which} wants a word after it");
        eprintln!("try 'yumete --help'");
        return ExitCode::from(2);
    }
    if grep.is_some() && listing.is_some() {
        eprintln!("yumete: --grep and --files ask two different questions; pick one");
        return ExitCode::from(2);
    }
    if grep.as_ref().or(listing.as_ref()).is_some_and(|p| p.trim().is_empty()) {
        // 空的詞從前一邊回「什麼都沒有」、一邊回「每一個檔」——兩種答案，都不是答案。
        eprintln!("yumete: the pattern is empty");
        return ExitCode::from(2);
    }
    if grep.is_none() && listing.is_none() {
        if let Some(stray) = modifiers.first() {
            eprintln!("yumete: {stray} only means something with --grep or --files");
            eprintln!("try 'yumete --help'");
            return ExitCode::from(2);
        }
    }
    if listing.is_some() {
        // Warning: **`--files` 只認得走檔那幾個。** 別的是給式子用的，而檔名那一邊用
        // 的是挑選器的模糊匹配，根本沒有式子。從前它們**悄悄不生效**，而
        // `--help` 說「下面每一個都還管用」——說了假話比少一個功能壞。
        const FOR_GREP_ONLY: [&str; 8] = [
            "--ignore-case",
            "--case-sensitive",
            "--word",
            "--regex",
            "--fixed",
            "--fuzzy",
            "--chinese",
            "--heading",
        ];
        if let Some(stray) = modifiers.iter().find(|m| FOR_GREP_ONLY.contains(m)) {
            eprintln!("yumete: {stray} shapes a pattern, and --files matches names by feel");
            eprintln!("it belongs to --grep");
            return ExitCode::from(2);
        }
    }

    // **`--open` 之外纔印到 stdout 就退出。** 帶了 `--open` 的那一條往下走，等編輯
    // 器建好、配置載好、檔開好之後再把面板擺出來（底下那一處）。
    if !g.open {
        // **管道那一邊說英文**（2026-10-03 作者定：「cli 搜索结果我建议用英文而不是
        // 中文……和 rg 稍微对齐一下，这样比较方便（pipe 的 tool 比较容易复用）」）。
        //
        // 編輯器裏的話是給寫書的人看的；這裏印出來的多半是給**另一個程序**看的，
        // 或者給一個正在拼管道的人看的——那一群詞的通用語是英文。明寫了
        // `--lang` 的人照他說的來。
        yumete_core::messages::set_language(
            force_language.unwrap_or(yumete_core::messages::Language::English),
        );
        if let Some(pattern) = &grep {
            return run_grep(pattern, &files, &g);
        }
        if let Some(pattern) = &listing {
            return run_files(pattern, &files, &g);
        }
    }

    let mut editor = Editor::new();
    let launch = std::time::Instant::now();
    let mut marks: Vec<(&str, std::time::Duration)> = Vec::new();
    // Each mark is the time *since the launch*; the deltas are worked out
    // when they are printed, so a mark cannot drift.
    let mark = |what: &'static str, marks: &mut Vec<(&str, std::time::Duration)>| {
        marks.push((what, launch.elapsed()));
    };

    // Load global + per-project config and apply the keymap.
    //
    // Warning: **從命令行說的那個地方往上找項目配置，不從 shell 站的地方**
    // （2026-10-01）。從前它無條件走 `current_dir()`，於是在 `/tmp` 敲
    // `ye ~/書/三體/卷一/第一章.md`，編輯器的根算出來是 `~/書/三體`，而
    // `.yumete/config.toml` 一個都沒載——配置與根各認各的地方。
    // Warning: **只有「給了一個文件夾」纔算**（2026-10-01 撤回當天那個 deviation
    // 之後）。給一個檔不動工作路徑——照 helix。見 `Editor::set_root`。
    let said_where = files.first().and_then(|first| {
        let at = std::path::PathBuf::from(yumete_config::expand_tilde(first));
        let full = match at.is_absolute() {
            true => at,
            false => std::env::current_dir().unwrap_or_default().join(at),
        };
        let here = full.is_dir().then_some(full)?;
        // Warning: **規範化**（2026-10-01 審出來的）。`Editor::set_root` 規範化而
        // 這裏從前不——於是 `ye ../書/一.md` 與 `ye /repo/書/一.md` 指同一個項
        // 目，卻哈希出兩份會話，正是這一改要消滅的那件事。
        Some(std::fs::canonicalize(&here).unwrap_or(here))
    });
    // 遞進去的是**項目根**，不是工作路徑——同 helix 的
    // `find_workspace().0.join(".helix")`。
    let config_root = said_where
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .map(|from| yumete_core::editor::book_root_of(&from));
    let (mut config, config_problems) = match &config_root {
        Some(root) => yumete_config::Config::load_reporting_from(root),
        None => yumete_config::Config::load_reporting(),
    };
    // Where the reader says the 宇浩 data is, before anything asks. Set once
    // here rather than passed down, because the places that build an IME
    // session are several and none of them carries a config.
    if !config.ime.data_dirs.is_empty() {
        yumete_config::set_data_dirs(config.ime.data_dirs.clone());
    }
    // Settled before anything is measured: every width question downstream —
    // wrap, gutter, cursor, the 縱 grid — asks the same global.
    //
    // `auto` asks the terminal, because the terminal is the only thing that
    // knows what its font does with `—` and `…`, and a writer cannot be
    // expected to diagnose a caret two cells off the character as a setting.
    // A terminal that will not answer is taken to draw them narrow: that is
    // what the page itself does, so at least the caret sits on the character.
    yumete_core::set_ambiguous_wide(match config.editor.ambiguous_width {
        yumete_config::Ambiguity::Wide => true,
        yumete_config::Ambiguity::Narrow => false,
        yumete_config::Ambiguity::Auto => {
            yumete_tui::ambiguous::ask_the_terminal_about_width().unwrap_or(false)
        }
    });
    // …and the language everything says itself in, before anything says
    // anything: a config error is a message too.
    //
    // The flag wins over the config, the way `-v` wins over `layout` — it was
    // typed for this run, and the config was typed once.
    if let Some(language) = force_language
        .or_else(|| yumete_core::messages::Language::parse(&config.editor.language))
    {
        yumete_core::messages::set_language(language);
    }
    // **每一項設定推一遍，一處**（`yumete_tui::settings::apply`）。從前這裏攤着
    // 一百多行 `editor.set_…`；抽出去是因為 `:config-reload` 要叫同一支——否則
    // 「啓動時認這個設定、重載時忘了它」是遲早的事。
    yumete_tui::settings::apply(&config, &mut editor, force_layout);
    // `--syntax` outranks the config and the extension both: it is this run's
    // answer about these files, and there is nothing further to guess from.
    let named_syntax = force_syntax
        .as_deref()
        .and_then(yumete_core::syntax::Syntax::parse);
    if let (Some(name), None) = (&force_syntax, named_syntax) {
        eprintln!("yumete: unknown syntax '{name}' — markdown, typst or text");
        return ExitCode::from(2);
    }
    editor.set_default_syntax(
        named_syntax.or_else(|| yumete_core::syntax::Syntax::parse(&config.editor.syntax)),
    );
    // Somewhere for a buffer with no file to keep its recovery copy. Only the
    // front end knows where the data directory is.
    // Where `:word-list global` writes, and where a reader's own dictionary is
    // read from — the front end's answer, because only it knows XDG.
    editor.keep_word_list_in(yumete_config::data_dir());
    let drafts = yumete_config::data_dir().join("drafts");
    if std::fs::create_dir_all(&drafts).is_ok() {
        editor.keep_drafts_in(drafts);
    }
    // …and somewhere to remember which files were open. **Keyed by the
    // project**, so a novel and a codebase do not share one.
    //
    // **Always kept, whatever `session` says** (2026-09-17). `session` only
    // decides whether a bare `yumete` reopens it; `-c` reopens it on the day
    // it is off, and it can only do that if the files were written down.
    //
    // Warning: **按項目，不按 cwd**（2026-10-01）。從前它拿 `current_dir()` 做
    // 鑰匙，而工作區是命令行參數定的——**同一個項目從兩個目録打開得到兩份會
    // 話，兩個項目從同一個目録打開共用一份**。
    // Warning: **沒給路徑的那一支也要走 `book_root_of`**（2026-10-01 審出來
    // 的）。只有一支走、另一支用生的 cwd 的話，`cd 卷一 && ye` 和 `ye .` 還是
    // 兩份會話。
    let session_key = config_root.clone();
    if let Some(here) = session_key {
        editor.keep_session_in(yumete_config::data_dir().join("sessions"), &here);
    }
    // Before the files, so the files come up locked rather than being locked a
    // moment after they are on screen.
    if readonly {
        editor.set_readonly_default(true);
    }
    mark("config", &mut marks);

    // The files, *after* the session's settings. Opening one may turn the page
    // horizontal — a file a schema calls a table is read across — and a setting
    // applied afterwards would be silently refused, so which layout you got
    // would depend on the order you named your files in.
    // **工作路徑在這裏定一次。** 照 helix：命令行給了一個**文件夾**就是它，給
    // 一個檔不動；什麼都沒給就是你敲 `ye` 的那個目録。項目路徑不另存——
    // `Editor::root()` 當場從工作路徑往上算（`.yumete`，然後 `.git`）。
    //
    // 於是它不跟着你翻到哪兒走：`gd` 跳進 homebrew 或 rustup 裏的源碼，`空格 f`
    // 照舊是這個項目。想搜別處是 `空格 F`（工作路徑）或者先 `:cd`。
    let mut opened_a_folder = None;
    let first_dir = files
        .first()
        .map(|first| std::path::PathBuf::from(yumete_config::expand_tilde(first)))
        .filter(|at| at.is_dir());
    match first_dir {
        Some(at) => {
            editor.set_root(&at);
            opened_a_folder = Some(editor.root());
        }
        None => {
            if let Ok(here) = std::env::current_dir() {
                editor.set_root(&here);
            }
        }
    }
    for file in &files {
        // **`ye 稿/` 開的是那本書，不是一個檔**（2026-09-27）。從前它走到
        // `Buffer::open` 上報「不是一個檔案」，而「打開一本書」正是這個編輯器
        // 最常做的事。文件夾只定根，不開緩衝——文件樹會把它攤開。
        if std::path::Path::new(&yumete_config::expand_tilde(file)).is_dir() {
            continue;
        }
        if let Err(err) = editor.open_file(file) {
            eprintln!("yumete: cannot open '{file}': {err}");
            return ExitCode::FAILURE;
        }
    }
    // **`ye 稿/` 開的是那本書，所以開完要看得見它。** 沒有一個檔可開的時候，
    // 那扇文件樹就是這一趟的答案（2026-09-27）。
    if let Some(root) = &opened_a_folder {
        editor.open_sidebar_at(root);
    }
    mark("open", &mut marks);
    // **What opening the file said, kept out of the wipe's way** (#388 again).
    // `set_status(String::new())` below clears the installation chatter so
    // none of it reaches the first frame, and it cannot tell that apart from
    // the one thing the open itself had to say — 「本檔案格式似乎是 Tab 分欄」
    // for a `.txt` that turned out to be a grid (#380). Captured here, put
    // back after the wipe, and still outranked by everything that speaks
    // later: `-t`, a broken config, a recovered draft.
    let opening_said = editor.take_open_notice();
    // With no file named, open what was open last time — five `:open`s every
    // morning is five too many. Only then: somebody who said which file they
    // wanted gets that file, and nothing else.
    // …and not when printing: `yumete -p` and `yumete | cat` are a look at one
    // thing, not a return to work.
    let printing = force_preview || !std::io::stdout().is_terminal();
    let wants_last_time = config.editor.session || resume;
    let restored = if files.is_empty() && !printing && !fresh && wants_last_time {
        editor.restore_session()
    } else {
        0
    };
    mark("session", &mut marks);
    // Which ruby dialect to lay out: whatever the config names, else the one
    // the file's extension implies.
    editor
        .execute(if config.editor.show_ruby {
            ":ruby full"
        } else {
            // 中階, not `off`: a reading nobody asked to see is still a
            // reading, so the word count knows 「錢塘」 is two 字 and the
            // tags are none of them (#283).
            ":ruby basic"
        })
        .ok();
    for name in &config.editor.ruby_dialects {
        let _ = editor.execute(&format!(":ruby {name}"));
    }
    // The reader's own 用字 groups (#233) — a novel's names, which no built-in
    // 異體字表 can hold.


    // The built-in Yume IME (Feature #27): load the configured scheme's tables
    // from the data directory. When the data is absent the session is
    // unavailable and Insert mode simply types plain ASCII — which is also what
    // happens for a scheme whose tables are not installed, since only 靈明 ships
    // with yumete.
    // Two different things live in yume's data, and they deserve different
    // answers.
    //
    // The **language model** — word frequencies and the 詞彙表, both of them
    // scheme-independent — is what `w`, `b` and `e` walk by, and it is what
    // makes this editor's word motion better than an editor's has any right to
    // be. Twenty-seven milliseconds, and everybody gets it.
    //
    // The **碼表** is another hundred and nine, and it is only of use to
    // somebody who came here to type 宇浩. Loading it for everyone means every
    // launch pays for an input method most of them did not ask for. So it
    // waits for `:yume-scheme`.
    mark("ruby", &mut marks);
    // **What is installed is what the editor offers** (#169). A directory scan,
    // not a compiled-in list: `schemes/*.toml` under any of the data
    // directories names a scheme, and yume-core sorts what it is given into
    // menu order. It reads a few kilobytes of TOML and touches no table, so it
    // is cheap enough to do before anything is drawn — and it has to be, since
    // `--shot` and `--timing` both run the whole launch and would otherwise
    // draw a menu of schemes this build does not have. Nothing found leaves the
    // built-in five standing, which is every install today.
    if yumete_ime::discover(&yumete_config::data_search_dirs()) > 0 {
        let found: Vec<(&str, &str)> = Scheme::all()
            .into_iter()
            .map(|s| (s.tag(), s.found_name()))
            .collect();
        yumete_core::command::set_schemes(&found);
    }
    mark("方案", &mut marks);
    let wanted = Scheme::from_tag(&config.ime.scheme).unwrap_or_else(Scheme::first);
    let page_size = config.panel.page_size;
    let chaifen = config.editor.show_chaifen;
    let start_scheme = config.ime.start;
    // 上屏方式 (Feature #209). `None` is 「whatever the scheme's own default is」
    // and has to stay a `None` all the way down, or a config that says nothing
    // would pin 拼音's 整句 onto every 形碼 scheme.
    let commit = config
        .ime
        .commit
        .as_deref()
        .and_then(CommitStrategy::from_str_tag);
    // 候選面板 (Feature #211) — the other axis, and a plain value: `full` is a
    // real answer, not「ask the scheme」.
    let panel = config.panel.display;
    // 正在打的那一段寫在哪——面板畫不畫之外的另一格（2026-09-27）。
    let preedit = config.panel.preedit;
    // Read after the first frame rather than before it — see `yumete_tui::
    // Deferred`. Everything it loads is a keystroke away; the page is not.
    let load = move |ime: &mut ImeSession| -> String {
        *ime = ImeSession::language_only(wanted);
        yumete_tui::want_page_size(page_size);
        ime.set_page_size(page_size);
        let said = match start_scheme {
            // …unless the config says this is a session for writing 漢字,
            // which for anybody who writes an input method it usually is.
            true => switch_scheme_at_startup(ime, wanted, (page_size, chaifen)),
            false => String::new(),
        };
        // After, not before: the line above may have replaced the session
        // wholesale with one built from the installed tables.
        ime.set_commit_strategy(commit);
        ime.set_panel_display(panel);
        ime.set_preedit(preedit);
        said
    };
    // A preview prints and exits: there is no frame to be after, so it is read
    // now. So does `--timing`, which is measuring exactly this.
    let printing_now = force_preview || (!std::io::stdout().is_terminal() && shot.is_none()) || timing;
    let mut ime = ImeSession::empty(wanted);
    let deferred: Option<yumete_tui::Deferred> = match printing_now {
        true => {
            let said = load(&mut ime);
            if !said.is_empty() {
                editor.set_status(said);
            }
            None
        }
        false => Some(Box::new(load)),
    };

    mark("輸入法", &mut marks);
    // Word segmentation, driving `w`/`b`/`e` and the overlay. Best first:
    //
    // 1. Yume's own language model (Feature #63) — 1.25M weighted entries plus
    //    the 詞彙表, already loaded above and shared by reference.
    // 2. A user `segmentation.txt` in the data directory.
    // 3. The compact list bundled with yumete, which covers common prose only.
    // Yume's own model is the best of the three and is already loaded above,
    // whether or not the 碼表 is.
    // One answer, in `yumete_tui::choose_words`, so `:word-list reload` cannot
    // pick a different dictionary from the one start-up picked.
    editor.set_segmenter(yumete_tui::choose_words(&ime, config.editor.word_level));
    // …and the readings, for `:ruby-auto`. Same source, different question:
    // the segmenter asks where a word ends, the reader asks how it is read.
    editor.set_reader(Box::new(ime.reader()));
    // …and the book's own words on top of whichever of the three it was. The
    // name on every page is the one word no dictionary has.
    editor.reload_project_words();
    // Warning: **`-v` on a program file is not an error, and not silent either.**
    // The setting is kept — the next buffer may well be a manuscript — but the
    // page in front of the reader is across, and a flag that appears to do
    // nothing is worse than one that says why (2026-09-21).
    if force_layout == Some(Layout::Vertical) && editor.writes_code() {
        editor.set_status(yumete_core::say!("layout.code-is-horizontal"));
    } else {
        editor.set_status(opening_said.unwrap_or_default());
    }
    mark("分詞", &mut marks);
    if timing {
        let mut last = std::time::Duration::default();
        for (what, at) in &marks {
            println!("{what:>8}  {:.1?}", *at - last);
            last = *at;
        }
        println!("{:>8}  {last:.1?}", "合計");
        return ExitCode::SUCCESS;
    }

    // `-t` is the writer saying "this is a table" about a file no schema names.
    //
    // **Here, not up with the other setup.** It used to run right after the
    // files were opened — and `enter_table` answers on the status line, which
    // `set_status(String::new())` above wipes on the way out of setup so none
    // of the installation chatter reaches the first frame. So a file `-t`
    // refused («「x.csv」不像表格——每行要有同樣多的欄») had its refusal wiped
    // before anything was drawn: `-t` looked like it had simply ignored you,
    // while `t t` on the same file explained itself. Same message, same code
    // path, and one of the two callers could never be heard (#388).
    if force_table {
        editor.enter_table();
    }

    // A config file that does not parse is worth one line: silence is how a
    // typo comes to look like a setting that does not work. **After `-t`**,
    // because a broken config outranks a file that is not a grid.
    if !config_problems.is_empty() {
        for problem in &config_problems {
            eprintln!("yumete: {problem}");
        }
        editor.set_status(config_problems.join(&yumete_core::say!("label.comma")));
    }
    // **恢復那一問要在按鍵之前擺出來**，因為它就是開檔那一刻的事（2026-10-02
    // 作者定）。
    //
    // Warning: **而且要在 `--shot` 那一支之前**。從前這一句在下面第 680 行附近，
    // 而 `--shot` 在第 520 行就把圖印出來走人了——於是離屏**永遠拍不到**開檔時
    // 的任何通知，救命稿那一問、「接着上次」那一句都在圖外。§5.55 記過同一個形
    // 狀：拍照的工具看不見人看得見的東西，比 bug 還糟。
    if restored > 0 {
        editor.set_status(yumete_core::say!("cli.picked-up-where-you-left-off", restored));
    }
    // Recovered work outranks a config typo for the one status line there is.
    // Only the editor has one; the preview prints its own notice instead.
    editor.announce_recovery();
    // Every picture from here on — `--shot`, `--html`, and `:shot` inside the
    // editor — carries the build under it, so a shot in a bug report says what
    // it is a picture of.
    yumete_tui::set_build(VERSION);
    let mut settings_page = None;
    if let Some(pressed) = &keys {
        // Warning: **`\{ime}` 之後那幾個鍵走輸入法**（2026-09-28）。從前 `--keys` 一律直接叫
        // `editor.on_key`，於是**離屏拍不到任何 preedit**——而輸入法是這個編輯器最不一樣
        // 的那一塊，這個倉的審查方法又是截圖。共用記事本上記過同一個形狀的坑：離屏
        // `--figure` 繞開控制器，於是「前端在把鍵交給引擎之前派掉的那幾個鍵」那一整族
        // bug 一個都拍不到。
        //
        // Warning: **數據要先載進來。** `--shot` 下它本來是推遲到第一幀之後的（見
        // `yumete_tui::Deferred`），而 `--keys` 跑在那之前。只有真的要用輸入法的時候纔
        // 載，不然每一張圖都要多等那幾百毫秒。
        if pressed.contains("\\{ime}") {
            // Warning: **要的是碼表，不只是語言層。** `load` 那一支建的是
            // `ImeSession::language_only`，只有在配置說「這一趟是寫漢字的」的時候纔往下
            // 載碼表；不載的話 `input()` 只是把字母堆起來，空格一按原樣上屏（實測打
            // `wo` 空格出來的就是 `wo`）。
            let said = switch_scheme_at_startup(&mut ime, wanted, (page_size, chaifen));
            if !said.is_empty() {
                editor.set_status(said);
            }
            // Warning: **讀音表要跟着換過來**（2026-10-02 一輪掃查報來的）。
            // `switch_scheme_at_startup` 是整個 `*ime = full` 換掉的，而
            // `Editor` 手上那份 reader 是**上一個會話的快照**（`ime.reader()`
            // 拷的是注解表）。不補這一句，`available()` 永遠是 false：狀態欄寫着
            // `[中 靈明]`、十四兆也真的載進來了，而 `:view-meter` 旁邊永遠空着、
            // `:ruby-auto` 永遠說「拆分表沒裝」——**於是平仄和注音這兩塊離屏根本
            // 審不了**，而這個倉審前端就是靠拍照。
            //
            // 互動那一支在它自己那次延遲載入之後緊跟着就補了（`yumete-tui`
            // 的 `editor.set_reader(Box::new(ime.reader()))`），這裏照抄。
            editor.set_reader(Box::new(ime.reader()));
            // Warning: **那三個配置也要補上**（2026-10-03 一輪審查報來的）。`load` 那一支
            // 在換完方案之後緊跟着撥這三格，而這裏是另一條路——不撥的話
            // `[ime] commit`、`[panel] display`、`[panel] preedit` 離屏整個不生
            // 效：九種 `preedit × display` 的組合畫出來**逐字節相同**，而真機各
            // 畫各的。配置那一族的畫面從此離屏審不了。
            ime.set_commit_strategy(commit);
            ime.set_panel_display(panel);
            ime.set_preedit(preedit);
            // Warning: **還要告訴編輯器輸入法在不在。** 這一格是 `Need::Scheme` 那一道閘
            // 問的（`words.rs`），不補的話離屏下 `:yume-panel`／`:yume-preedit`／
            // `:chaifen` 一律答「還不行，需要：載入碼表」，而同一幀的狀態欄已經
            // 寫着 `[中 靈明]`。互動那一支每一輪都撥一次。
            editor.set_ime_available(ime.available());
        }
        // Warning: **按鍵之前先把頁夾好**（#516，2026-10-02）。互動的循環是
        // 「畫一幀、讀一個鍵、再畫一幀」，所以按鍵那一刻用的是上一幀量出來的高
        // 度；離屏這一支沒有循環，`--keys` 是在任何一幀之前跑完的。不補這一下，
        // `--keys='…wo3'` 在六行的窗口上照樣上屏那個從沒畫出來的字——**而那正
        // 是要驗的那個 bug**。
        //
        // Warning: **不能靠「先白畫一幀」把高度量出來。** 試過，`多選區` 那張金
        // 樣當場變了：`frame_to_text` 收的是 `&mut Editor`，白畫那一幀給了編輯
        // 器一個它本來沒有的視口，於是後面那幾個動作落在了别的地方。算出來，别
        // 畫出來。
        if let Some((w, h)) = shot {
            yumete_tui::fit_the_page(&mut ime, &mut editor, &config, w, h);
        }
        settings_page = press(&mut editor, pressed, &config, &mut ime, shot);
    }
    // Asked for on the command line, and run the same way `:tutor` runs it.
    if tutor {
        let _ = editor.execute(":tutor");
    }
    // **`--grep … --open` / `--files … --open`**：詞在命令行上已經打過了，進去就
    // 該是「已經找完」的那個樣子（2026-10-03 作者定）。
    if g.open {
        let scope = match g.project {
            true => Where::Project,
            false => Where::Working,
        };
        if let Some(pattern) = &grep {
            {
                let s = editor.search_mut();
                s.case = g.case.unwrap_or_default();
                s.whole = g.word;
                s.regex = g.regex;
                s.fuzzy = g.fuzzy;
                s.hidden = g.hidden;
                s.include = g.include.clone();
                s.exclude = g.exclude.clone();
                (s.glyphs, s.pinyin) = g.chinese;
            }
            editor.open_search_with(pattern, scope);
        }
        if let Some(pattern) = &listing {
            let root = match g.project {
                true => editor.project_root(),
                false => editor.working_dir(),
            };
            editor.open_file_picker_with(pattern, root);
        }
    }
    if let Some((width, height)) = shot {
        // **一幀也要算一次改動條。** 互動的循環是停手 300 毫秒纔算，離屏出圖没有
        // 那個循環——不補這一句，`--keys` 打進去的改動在圖上永遠看不見，而所有
        // 的驗證都是看圖。
        editor.vcs_tick();
        // **Say what this picture cannot show.** `--keys` presses into the core
        // (`press` calls `editor.on_key`), and the IME lives one crate up in
        // the front end: composing, the candidate panel, and every `:yume`
        // command are handled by the interactive loop, which `--shot` never
        // enters. A `:yume …` leaves its request on the editor for that loop to
        // pick up — so a request still sitting here is one nobody will ever
        // act on, and the frame below is of a session where the command did
        // nothing at all. It used to be drawn without a word, which is how a
        // reviewer comes away certain the input method is simply blank (#385).
        //
        // Standard error, not standard out: the picture itself stays a picture.
        //
        // And it is a **family**, not one case: the core answers nine kinds of
        // request by leaving them on the editor for the front end, and this
        // path picks up none of them. Naming the ones actually left is cheap
        // and does not go stale — a tenth added tomorrow shows up here the day
        // somebody uses it.
        // Warning: **Two of them this path can honour, and should** (#470). The
        // colours are globals settled before anything is drawn, not state the
        // loop owns — so `:theme`, `:theme-mode` and `:theme-fill` work here
        // exactly as they do in the editor, and a reviewer asking 「what does
        // `:theme-fill on` look like」 gets the answer instead of a warning.
        // The ones below genuinely need the loop: they hold an input method, a
        // child process, the clipboard or the window system.
        if let Some((name, mood)) = editor.take_theme_request() {
            let said = yumete_tui::set_theme(&config, name, mood);
            editor.set_status(said);
        }
        if let Some(on) = editor.take_fill_request() {
            let on = yumete_tui::theme::set_fill(on, &config);
            editor.set_status(match on {
                true => yumete_core::say!("theme.fill-on"),
                false => yumete_core::say!("theme.fill-off"),
            });
        }
        let mut unheard: Vec<String> = Vec::new();
        if let Some(asked) = editor.take_scheme_request() {
            // Warning: **印出去的要是打得出來的那一行**（2026-10-02 一輪掃查報來的）。
            // `asked` 是核心和前端之間的暗號，不是命令：`:yume-which` 傳的是 `?`，
            // 於是這裏印「`:yume ?` did nothing」，而 `:yume ?` 這一行輸進去是被
            // 拒的。這一族 2026-09 加了連字符（`:yume-which`、`:yume-installed`
            // ……），而暗號留在了那之前。
            //
            // 認得出的就還原成真名字，認不出的（方案標籤）本來就是 `:yume-scheme`
            // 的參數。
            let line = match asked.as_str() {
                "?" => ":yume-which".to_string(),
                "~" => ":yume-installed".to_string(),
                "!" => ":yume-builtin".to_string(),
                "where" => ":yume-where".to_string(),
                rest if rest.starts_with("lang:") => ":yume".to_string(),
                rest if rest.starts_with("commit:") => ":yume-commit".to_string(),
                rest if rest.starts_with("panel:") => ":yume-panel".to_string(),
                rest if rest.starts_with("preedit:") => ":yume-preedit".to_string(),
                rest => match rest.strip_prefix('=') {
                    Some(path) => format!(":yume-table {path}"),
                    None => format!(":yume-scheme {rest}"),
                },
            };
            unheard.push(line);
        }
        if editor.take_chaifen_request().is_some() {
            unheard.push(":chaifen".into());
        }
        if editor.take_preview_request().is_some() {
            unheard.push(":preview".into());
        }
        if editor.take_open_request().is_some() {
            unheard.push("gx".into());
        }
        if editor.take_shell_request().is_some() {
            unheard.push("a shell command".into());
        }
        if editor.take_clipboard_request().is_some() {
            unheard.push("the system clipboard".into());
        }
        if editor.take_screenshot_request().is_some() {
            unheard.push(":shot".into());
        }
        // 語言服務器是循環裏的一個進程，一幀畫不出它來。2026-09-23 補：這兩個
        // 從前不在名單上，於是 `空格 k` 拍出來永遠是「問問這是什麽……」，看着
        // 像功能壞了。字典不在這裏——它是 `frame_to` 自己答得了的。
        if editor.take_hover_query().is_some() {
            unheard.push("空格 k / 空格 K".into());
        }
        if editor.take_definition_query().is_some() {
            unheard.push("gd".into());
        }
        if editor.take_completion_query().is_some() {
            unheard.push("C-n".into());
        }
        // Warning: **四個 2026-10-02 補的**（一輪掃查報來的）。上面那句話說「a tenth
        // added tomorrow shows up here the day somebody uses it」——沒有，它們就這
        // 麼悄悄做了無事可做的事。看得最清楚的是剪貼板：`␣y`（寫）印了那句提示，
        // `␣p`（讀）什麼都不印，同一塊板子兩種待遇。
        if editor.take_language_run().is_some() {
            unheard.push(":format / :run".into());
        }
        if editor.take_config_reload() {
            unheard.push(":reload config".into());
        }
        if editor.take_clipboard_read().is_some() {
            unheard.push("␣p / ␣P".into());
        }
        if editor.take_words_request() {
            unheard.push(":word-list-reload".into());
        }
        if !unheard.is_empty() {
            eprintln!(
                "yumete: --shot draws one frame and never enters the editor's loop, so {} did nothing here.",
                unheard.join(", ")
            );
            eprintln!(
                "        The input method in particular is the front end's: composing and the"
            );
            eprintln!(
                "        candidate panel need a real terminal (a pty), not this."
            );
        }
        yumete_tui::fit_the_page(&mut ime, &mut editor, &config, width, height);
        // The layout the flags asked for, before the picture is taken.
        let picture = match shot_html {
            true => yumete_tui::frame_to_html(
                &mut editor,
                &config,
                &ime,
                width,
                height,
                settings_page.as_ref(),
            ),
            false => yumete_tui::frame_to_text(
                &mut editor,
                &config,
                &ime,
                width,
                height,
                settings_page.as_ref(),
            ),
        };
        print!("{picture}");
        return ExitCode::SUCCESS;
    }
    if printing {
        preview(&editor, &config);
        return ExitCode::SUCCESS;
    }

    // **Somewhere to say what went wrong** (#300). The log lives beside the
    // drafts and the sessions in the data directory, not in `~/.config`: that
    // one holds what the reader writes for the program, this is what the
    // program writes for the reader.
    yumete_core::diag::log_to(yumete_config::data_dir().join("yumete.log"));
    yumete_core::diag::install_panic_hook();
    // **A hang leaves nothing behind** — no panic, no message, and the reader
    // can only say 「it froze」. A thread watching the loop's heartbeat turns
    // that into a line naming the stage it stopped in.
    yumete_core::diag::watch(std::time::Duration::from_secs(2));
    // **A panic must not be the end of the manuscript.** Caught here rather
    // than left to unwind out of `main`, because on this side of the call the
    // editor is reachable again: the screen goes back to normal, every dirty
    // buffer gets a recovery copy, and the reader is told where to look
    // instead of watching a backtrace scroll through an alternate screen that
    // is being torn down under it.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        yumete_tui::run(&mut editor, &mut config, &mut ime, deferred)
    }));
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(_) => {
            yumete_tui::restore_terminal();
            let saved = editor.rescue_drafts();
            editor.save_session();
            let log = yumete_core::diag::path()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            eprintln!("{}", yumete_core::say!("cli.crashed", saved, log));
            return ExitCode::from(101);
        }
    };
    // Written on the way out, whichever way out it was: an editor that only
    // remembered a clean exit would forget the session you most wanted back.
    editor.save_session();
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("yumete: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Press `keys` on the editor, for `--keys`.
///
/// The escapes are the ones a keyboard has and a string does not: `\e` Esc,
/// `\t` Tab, `\n` Enter, `\b` Backspace, `\u` `\d` `\l` `\r` the arrows,
/// and `\\` a backslash. Everything else is the character itself — including
/// 漢字, which arrive here the way the IME hands them over: while `r` is armed
/// a run of them is gathered and **committed**, because a commit is a string
/// and the difference from a key is the whole of what `r` does with it.
///
/// Warning: **`:settings` 之後的鍵歸那扇面板**，和主循環一樣。不接這一下，
/// `--shot --keys=':settings\nljj '` 拍到的永遠是面板剛開的樣子——而這個倉審前端
/// 就是靠拍照，一扇按不動的面板等於一扇沒法審的面板。回來的是那扇面板（要畫它）。
fn press(
    editor: &mut Editor,
    keys: &str,
    config: &yumete_config::Config,
    ime: &mut ImeSession,
    shot: Option<(u16, u16)>,
) -> Option<yumete_config::panel::Panel> {
    let mut settings = yumete_tui::settings_page::Seat::default();
    // **`\{ime}` 撥一下這一格**：往後那幾個鍵交給輸入法，再按一次撥回來。
    let mut composing = false;
    let mut chars = keys.chars().peekable();
    while let Some(c) = chars.next() {
        // **兩個鍵之間補上主循環會畫的那一幀**（2026-10-02 查出來的）。
        //
        // 走磁碟的那一趟搜索是**欠着的**：`Enter` 只記一筆，主循環先畫一幀
        // 「正在找…」再去跑（`run_owed_search`）。而 `--keys` 是一口氣餵完的，
        // 從前整串鍵都跑在那一趟之前——於是 `--keys='\{space}/霜\n66jjjj'` 裏
        // 的每一個 `j` 走的都是**本文件那一檔的舊名單**，走完自己這一份就到頭，
        // 別的檔一處也去不了；而畫的時候那一趟已經跑完了，照片上名單是全的。
        // 看起來像「名單走不動」，其實是這支工具自己落後了一幀。
        //
        // 和底下 `gw` 那一處同一族（#406），也和這支函數開頭說的 `:settings`
        // 同一族：**主循環在兩個鍵之間做的事，這裏一件都不許少**，不然拍到的
        // 面板和人按出來的面板不是同一扇。只是這一件便宜，不必真畫一幀。
        if editor.take_owed_search() {
            editor.run_owed_search();
        }
        // **全部替換也是欠着做的**（2026-10-03）：主循環一批八十毫秒、中間畫一幀
        // 報進度，而這裏沒有循環可畫——一口氣做完就是。不補這一下，`--shot` 拍到
        // 的是「按了 `y`，什麼都沒發生」。
        while editor.replacing_a_batch() {
            editor.run_a_batch_of_replacing();
        }
        editor.refresh_the_edited_file();
        let key = match c {
            '\\' => match chars.next() {
                Some('e') => Key::Esc,
                Some('t') => Key::Tab,
                Some('n') => Key::Enter,
                Some('b') => Key::Backspace,
                Some('u') => Key::Up,
                Some('d') => Key::Down,
                Some('l') => Key::Left,
                Some('r') => Key::Right,
                // **`\^x` is Control-x**, caret notation, the way a terminal
                // has written it since teletypes. Added 2026-09-11: without it
                // a whole family of keys both tutors teach — `C-o` `C-i` `C-r`
                // `C-w` `C-a` `C-d` — could not be pressed offscreen at all,
                // and #382's `End` bug slipped through review for exactly that
                // reason: nobody could send the key.
                Some('^') => match chars.next() {
                    Some(c) => Key::Ctrl(c.to_ascii_lowercase()),
                    None => break,
                },
                // `\{name}` for the keys with names rather than letters.
                Some('{') => {
                    let mut name = String::new();
                    for c in chars.by_ref() {
                        if c == '}' {
                            break;
                        }
                        name.push(c);
                    }
                    // **`\{ime}` 撥一下「往後走不走輸入法」**（2026-09-28）。
                    if name == "ime" {
                        composing = !composing;
                        // Warning: **撥開關的時候順手把它打開。** `:yume on` 走的是請求／回應
                        // 那條路，而那條路要互動循環來服務——`--shot` 沒有循環，工具自己
                        // 早就印過一句話說這件事。這裏直接撥會話上那兩格。
                        if composing {
                            ime.set_engaged(true);
                            if !ime.is_chinese() {
                                ime.toggle_language();
                            }
                        }
                        continue;
                    }
                    // `\{alt-.}` for the Meta chords. helix's tutor leans on
                    // them — `Alt-s` `Alt-.` `Alt-,` `Alt-;` `Alt-\`` — and
                    // without this none of that family could be pressed
                    // offscreen (2026-09-11).
                    if let Some(rest) = name.strip_prefix("alt-") {
                        match rest.chars().next() {
                            Some(c) if rest.chars().count() == 1 => Key::Alt(c),
                            _ => {
                                eprintln!("yumete: --keys: alt- wants one character, got {rest:?}");
                                std::process::exit(2);
                            }
                        }
                    } else if let Some(rest) = name.strip_prefix("ctrl-") {
                        // `\{ctrl-w}` and its family. `C-w` walks the regions
                        // (#293), and without this the one key that reaches a
                        // side panel from the writing could not be pressed
                        // offscreen — which is where the panels are looked at.
                        match rest.chars().next() {
                            Some(c) if rest.chars().count() == 1 => Key::Ctrl(c),
                            _ => {
                                eprintln!("yumete: --keys: ctrl- wants one character, got {rest:?}");
                                std::process::exit(2);
                            }
                        }
                    } else {
                    match name.as_str() {
                        "home" => Key::Home,
                        "end" => Key::End,
                        "pgup" => Key::PageUp,
                        "pgdn" => Key::PageDown,
                        "del" => Key::Delete,
                        "backtab" => Key::BackTab,
                        // **四個方向鍵與 `Tab`、`Enter`、`Esc`、退格**（2026-10-03 補）。
                        // 這張表從前缺它們，而 `Key` 一直有——於是離屏驗不了「面板
                        // 裏按下去會怎樣」那一整族：面板的提示行寫着「↑ ↓ 上下移
                        // 動」，而寫腳本的人（包括我）伸手去拿 `\{down}` 拿不到。
                        // `\n` `\e` `\b` 一直認得，這裏多給一個寫得出名字的拼法。
                        "up" => Key::Up,
                        "down" => Key::Down,
                        "left" => Key::Left,
                        "right" => Key::Right,
                        "tab" => Key::Tab,
                        "enter" => Key::Enter,
                        "esc" => Key::Esc,
                        "bs" => Key::Backspace,
                        // **`\{space}` 也認**（2026-10-02）。字面的空格一直管用，
                        // 可一串鍵裏的空格看不見，所以寫腳本的人（包括我）伸手就
                        // 去拿這個名字——而它從前不存在。
                        "space" => Key::Char(' '),
                        // Warning: **認不得的名字就地死掉，不是打一行就往下走**
                        // （2026-10-02 修）。從前它 `break`，於是這一串鍵**從這裏
                        // 整個截斷**而退出碼仍然是 0——`frames.sh` 裏兩個場景因此
                        // 拍了不知多久的錯誤信息，而金樣比對一直是綠的（它比的是
                        // 「和上次一樣嗎」，上次也可以是錯的）。
                        other => {
                            eprintln!("yumete: --keys: no key called {other:?}");
                            std::process::exit(2);
                        }
                    }
                    }
                }
                Some(other) => Key::Char(other),
                None => break,
            },
            other => Key::Char(other),
        };
        // A commit is not a key, and with `r` armed that difference is the
        // whole behaviour (§5.2.3 ②): the IME hands the editor a *string*,
        // which may be two 字 long. Gather the run and commit it, so a picture
        // of `r` 打中文 shows what a reader would actually see.
        if editor.takes_a_character() {
            if let Key::Char(c) = key {
                let mut text = String::from(c);
                while let Some(&next) = chars.peek() {
                    if next.is_ascii() {
                        break;
                    }
                    text.push(next);
                    chars.next();
                }
                editor.insert_committed(&text);
                continue;
            }
        }
        // 面板開着：鍵歸它（`:` 除外——那是命令行，`:w`／`:q` 在那上面打）。
        // Warning: **和主循環同一支** `Seat`，不是抄一遍：抄本當天就分岔過（那一份漏了
        // 「有改動不許一下走」的閘，又把存盤的錯 `let _ =` 吞掉）。
        if settings.took(editor, Some(key)) {
            continue;
        }
        // **走輸入法的那一段**（`\{ime}` 之間，2026-09-28）。
        //
        // Warning: **這裏不許自己寫一份派發**（2026-10-03 一輪審查報來的，改掉了）。從前
        // 這一段是手抄的「打字要用的那幾個鍵」——碼、空格、Enter、退格、選重
        // 數字、Esc——而真正那一支一直在長。分岔出來的是：離屏按 `;` 出「；」而
        // 真機出「辶」（靈明的選二）、`=` 離屏出 `=` 而真機翻頁、方向鍵離屏把光標
        // 移走並把字上到別處、`\t` 離屏插一個製表符而真機開面板。**這個倉審前端
        // 靠的就是這張照片，而它在輸入法這一塊照的是另一個程序。**
        //
        // 現在叫的是互動循環叫的那一支（`yumete_tui::offline_ime_key`），連
        // 「這一鍵歸不歸輸入法」那三個判準都是同一份。
        if composing && yumete_tui::offline_ime_key(ime, editor, key) {
            continue;
        }
        editor.on_key(key);
        settings.settle(editor);
        // Warning: **`gw` 的標籤要等一幀纔算得出來**（#406）：落腳點只算屏幕上的，而哪一
        // 段在屏幕上是**畫的時候**量的。一批鍵是一次餵完的，所以餵到欠着的那一刻
        // 先畫一幀丟掉——不然 `--keys='gwf'` 裏那個 `f` 落到正文上當成別的鍵，而
        // 這個倉審前端就是靠拍照。
        if editor.owes_a_jump() {
            if let Some((w, h)) = shot {
                let _ = yumete_tui::frame_to_text(editor, config, ime, w, h, settings.panel.as_ref());
            }
        }
    }
    // **最後一個鍵欠下的那一批也要做完**：迴圈頂上那一下只補得了鍵**之間**的，
    // 而 `R y` 正是按在最後。不補這一下，照片上是「按了 `y`，什麼都沒發生」。
    while editor.replacing_a_batch() {
        editor.run_a_batch_of_replacing();
    }
    settings.panel
}

/// **管道那一邊的那幾個開關**（`ye --grep`，2026-10-03 作者定）。
///
/// 出廠值就是面板的出廠值，只有一處不同：**名單不封頂**。面板封在 500 條，因為
/// 名單是拿來走的；管道印給別的程序看，少印一條就是錯一條。
struct Grep {
    case: Option<Case>,
    word: bool,
    regex: bool,
    fuzzy: bool,
    hidden: bool,
    include: String,
    exclude: String,
    /// **繁簡、拼音**。出廠兩個都開——那是這個工具存在的理由
    /// （2026-10-03 作者定：`ye --grep zhongguo` 開箱就搜得到「中國」）。
    chinese: (bool, bool),
    /// 往上搜到項目的根，而不是當前目錄。
    project: bool,
    /// **染不染色**：`None` ＝ 接着終端機就染、進管道就不染（rg 的 `auto`）。
    colour: Option<bool>,
    /// **按檔分組印，還是一行一條**：`None` ＝ 跟着終端機走，同 `colour`。
    ///
    /// 作者 2026-10-03 看過平鋪那一版之後定：「能不能像 rg 这样按照文件分组？看
    /// 起来好看多了」。分組是**給人看的排版**，所以進了管道照樣一行一條自足的
    /// `檔:行:列:文字`——`xargs`、`awk` 那一頭讀的還是原來那個形狀。
    heading: Option<bool>,
    /// **開編輯器，別印到 stdout**（2026-10-03 作者定）。`--grep … --open` 開起來
    /// 面板已經在跑，`--files … --open` 開起來挑選器已經打好。
    ///
    /// 做成一個修飾旗標而不是另一對名字：所有別的開關（`--hidden`、`--project`、
    /// `--chinese=`…）白拿，只有一套詞彙要記。作者原話：「其实 open 更好。未来
    /// 可以 -go 来 grep + open，短别名是可以连缀的」。
    open: bool,
}

impl Default for Grep {
    fn default() -> Grep {
        Grep {
            case: None,
            word: false,
            regex: false,
            fuzzy: false,
            hidden: false,
            include: String::new(),
            exclude: String::new(),
            // Warning: **不是 `(false, false)`。** 繁簡和拼音出廠都開着——那是這個工具
            // 存在的理由，`derive(Default)` 給的那一對正好是反的。
            chinese: (true, true),
            project: false,
            colour: None,
            heading: None,
            open: false,
        }
    }
}

/// **`ye --grep`**：照這些開關搜一趟，把 `檔:行:列:文字` 印出來就退出。
///
/// 退出碼照 grep 的規矩：找到 0、一處都沒有 1、說不通 2。
///
/// Warning: **這裏不另寫一個 grep。** 它撥的是面板撥的那一份狀態、跑的是面板跑的那一
/// 支（`Editor::run_the_search`）。另寫一份一定會和面板分岔——同一天上午剛修過
/// 一個：離屏拍照那一支手抄了一份輸入法派發，照出來的是另一個程序。
///
/// Warning: **正文那一欄是命中前後各六十個字，不是整行。** 小說的一行是一整段，動輒幾
/// 千字——rg 印整行是因為代碼的一行是一行。要整行的話那是另一個開關的事。
/// **把命令行給的那幾個地方變成「搜哪裏」。**
///
/// 一個都沒給就是當前目錄——命令行的整個模型就是「我站在哪」，而
/// `ls`／`grep`／`rg`／`fd` 沒有一個例外。
///
/// Warning: **給了幾個就搜幾個**（2026-10-03 一輪審查報來的）。從前只看 `first()`，
/// 後面的一聲不吭地丟掉——`ye --grep alpha d1 d2` 只搜了 `d1`。
///
/// Warning: **`~` 要展開。** 倉裏別處都走 `expand_tilde`，就這兩支沒走，於是
/// `ye --grep 霜 '~/書'` 答「`~` 不是一個目錄」。
///
/// Warning: **給一個檔也算數。** `rg pat file` 是最常見的用法。檔案交給它所在的目錄
/// 加一條只放它進來的 glob——同一套機器，不另開一條路。
fn places(what: &str, given: &[String], cwd: &std::path::Path) -> Result<Vec<(std::path::PathBuf, Option<String>)>, ExitCode> {
    if given.is_empty() {
        return Ok(vec![(cwd.to_path_buf(), None)]);
    }
    let mut out = Vec::with_capacity(given.len());
    for one in given {
        let path = std::path::PathBuf::from(yumete_config::expand_tilde(one));
        if path.is_dir() {
            out.push((path, None));
            continue;
        }
        if path.is_file() {
            let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(cwd);
            let name = path.file_name().map(|n| n.to_string_lossy().to_string());
            out.push((dir.to_path_buf(), name));
            continue;
        }
        eprintln!("yumete: {what}: no such file or directory: {}", path.display());
        return Err(ExitCode::from(2));
    }
    Ok(out)
}

/// **問題本身說不通**——一個字都不印，退出碼 2。
///
/// Warning: **「安安靜靜地說沒找到」是這支工具唯一不許犯的錯**（2026-10-03 一輪審查
/// 報來的）：式子寫壞從前交退出碼 1，和「真的一處都沒有」分不開，腳本於是分不出
/// 打錯字和沒結果。
fn the_question_will_not_parse(editor: &Editor, what: &str) -> Option<ExitCode> {
    let search = editor.search();
    if search.broken {
        eprintln!("yumete: {what}: that pattern will not compile");
        return Some(ExitCode::from(2));
    }
    if search.bad_glob {
        eprintln!("yumete: {what}: {}", editor.status());
        return Some(ExitCode::from(2));
    }
    None
}

/// **有沒有東西是沒看的**——說在 stderr 上，答案照印。
///
/// 四兆以上的檔跳過、走檔走到上限停了，都屬於這一種。
///
/// Warning: **這不改退出碼，除非一處都沒找到。** 照 rg 的分寸：答案拿得到就是 0，讀
/// 不了的那幾個檔在 stderr 上說一聲——跳過一個大檔就讓整趟失敗，比沉默還糟。
/// 可是**一處都沒有、而且確實沒看全**的時候，「沒找到」就是一句假話，那時交 2。
fn what_was_not_looked_at(editor: &Editor, what: &str) -> bool {
    let search = editor.search();
    let mut incomplete = false;
    if search.skipped > 0 {
        eprintln!("yumete: {what}: {} file(s) too big to read were skipped", search.skipped);
        incomplete = true;
    }
    if search.cut {
        eprintln!("yumete: {what}: the walk stopped early; this is not the whole answer");
        incomplete = true;
    }
    incomplete
}

fn run_grep(pattern: &str, where_: &[String], g: &Grep) -> ExitCode {
    let ink = Ink(g.colour.unwrap_or_else(|| std::io::stdout().is_terminal()));
    let cwd = std::env::current_dir().unwrap_or_default();
    let roots = match places("--grep", where_, &cwd) {
        Ok(roots) => roots,
        Err(code) => return code,
    };
    let mut sink = std::io::stdout().lock();
    let mut found = 0usize;
    let mut incomplete = false;
    let given_a_place = !where_.is_empty();
    // **分組是給人看的，管道裏仍是一行一條。** rg 自己就是這個規矩：接着終端機
    // 的時候檔名自成一行、底下只寫行列與正文，兩個檔之間空一行；一旦 stdout 不
    // 是終端機，每一行都要自己說得出自己是哪個檔的。
    let grouped = g.heading.unwrap_or_else(|| std::io::stdout().is_terminal());
    let mut heading_shown: Option<String> = None;
    for (at, only) in roots {
        let mut editor = Editor::new();
        editor.set_root(&at);
        {
            let s = editor.search_mut();
            s.query = pattern.to_string();
            s.scope = match g.project {
                true => Where::Project,
                false => Where::Working,
            };
            s.uncapped = true;
            s.case = g.case.unwrap_or_default();
            s.whole = g.word;
            s.regex = g.regex;
            s.fuzzy = g.fuzzy;
            s.hidden = g.hidden;
            // 指名一個檔的時候，那條 glob 就是「只要它」。
            s.include = match &only {
                Some(name) => name.clone(),
                None => g.include.clone(),
            };
            s.exclude = g.exclude.clone();
            (s.glyphs, s.pinyin) = g.chinese;
        }
        editor.run_the_search();
        if let Some(code) = the_question_will_not_parse(&editor, "--grep") {
            return code;
        }
        incomplete |= what_was_not_looked_at(&editor, "--grep");
        // **印出來的路徑照你給的那個拼法**（2026-10-03 一輪審查報來的）。搜索的
        // 根是 canonicalize 過的，照它拼出來 `ye --grep x alias` 會答
        // `realdir/f.md`——問的是 `alias`，拿回來的是別的名字，`cd` 過去落在別處。
        // `--project` 沒有「你給的拼法」可依，那時纔退回真路徑。
        let root = match (g.project, given_a_place) {
            (false, true) => at.clone(),
            _ => editor.search().root.clone().unwrap_or_else(|| at.clone()),
        };
        // Warning: **讀的人半路走了不算出錯。** `ye --grep 霜 | head -2` 關掉管道那一頭，
        // 而 Rust 的 `println!` 遇上 EPIPE 是 **panic**——六千條命中的時候它當場吐
        // 一段堆棧。每一個 Unix 工具在這裏都是安安靜靜地收攤。
        for hit in &editor.search().hits {
            let Some(file) = hit.file.as_ref() else { continue };
            // **印得出來的路徑是相對於你站的地方的**，所以「搜了哪裏」一眼看得出：
            // `--project` 爬上去過的話，印出來就會帶 `../`。
            let shown = pathdiff(&root.join(file), &cwd);
            // **命中那幾個字自己染**：`mark` 說它們落在摘錄的哪一段（按字計）。
            let marked: String = {
                let chars: Vec<char> = hit.excerpt.chars().collect();
                let cut = |a: usize, b: usize| -> String {
                    chars.get(a.min(chars.len())..b.min(chars.len())).unwrap_or(&[]).iter().collect()
                };
                format!(
                    "{}{}{}",
                    cut(0, hit.mark.start),
                    ink.hit(&cut(hit.mark.start, hit.mark.end)),
                    cut(hit.mark.end, chars.len())
                )
            };
            let name = shown.display().to_string();
            let mut rows: Vec<String> = Vec::with_capacity(3);
            let place = format!(
                "{}:{}",
                ink.number(&(hit.line + 1).to_string()),
                ink.number(&(hit.column + 1).to_string())
            );
            if grouped {
                if heading_shown.as_deref() != Some(name.as_str()) {
                    if heading_shown.is_some() {
                        rows.push(String::new());
                    }
                    rows.push(ink.path(&name));
                    heading_shown = Some(name);
                }
                rows.push(format!("{place}:{marked}"));
            } else {
                rows.push(format!("{}:{place}:{marked}", ink.path(&name)));
            }
            for row in rows {
                match writeln!(sink, "{row}") {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                        return ExitCode::SUCCESS;
                    }
                    Err(e) => {
                        eprintln!("yumete: --grep: {e}");
                        return ExitCode::from(2);
                    }
                }
            }
            found += 1;
        }
    }
    match (found, incomplete) {
        // 一處都沒有、而且確實沒看全——「沒找到」在這裏是假話。
        (0, true) => ExitCode::from(2),
        (0, false) => ExitCode::from(1),
        _ => ExitCode::SUCCESS,
    }
}

/// **`ye --files`**：名字配得上的檔，一行一個，最配的在前。
///
/// fd 的那一半，而它認拼音：`ye --files jia` 找得到 `甲.md`。
///
/// Warning: **匹配器是挑選器那一個**（`Editor::files_matching`），所以這裏和編輯器裏
/// `空格 f` 打同樣幾個字母永遠是同一份答案。
fn run_files(pattern: &str, where_: &[String], g: &Grep) -> ExitCode {
    let ink = Ink(g.colour.unwrap_or_else(|| std::io::stdout().is_terminal()));
    let cwd = std::env::current_dir().unwrap_or_default();
    let roots = match places("--files", where_, &cwd) {
        Ok(roots) => roots,
        Err(code) => return code,
    };
    let mut sink = std::io::stdout().lock();
    let mut printed = 0usize;
    for (at, only) in roots {
        let mut editor = Editor::new();
        editor.set_root(&at);
        let root = match g.project {
            true => editor.project_root(),
            false => at,
        };
        // 同 `run_grep`：給了拼法就照拼法印。
        let shown_root = match (g.project, !where_.is_empty()) {
            (false, true) => root.clone(),
            _ => root.clone(),
        };
        // **走檔那三個開關真的生效**（2026-10-03 修）：從前這裏寫死
        // `Sieve::default()`，於是 `--hidden`／`--glob=`／`--exclude=` 全是死的。
        let sieve = yumete_core::editor::Sieve {
            hidden: g.hidden,
            include: match &only {
                Some(name) => name.clone(),
                None => g.include.clone(),
            },
            exclude: g.exclude.clone(),
        };
        for name in editor.files_matching(&root, pattern, &sieve) {
            let shown = pathdiff(&shown_root.join(&name), &cwd);
            match writeln!(sink, "{}", ink.path(&shown.display().to_string())) {
                Ok(()) => printed += 1,
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("yumete: --files: {e}");
                    return ExitCode::from(2);
                }
            }
        }
    }
    match printed {
        0 => ExitCode::from(1),
        _ => ExitCode::SUCCESS,
    }
}

/// **管道那一邊的顏色**（2026-10-03 作者定：「染色这一块也可以做一下」）。
///
/// 照 rg 的缺省配色，因為肌肉記憶和眼睛的習慣都在那兒：路徑洋紅、行號綠、命中的
/// 那幾個字紅而且粗。`auto` 的判準也和它一樣——**接着終端機就染，進管道就不染**，
/// 不然 `| cut -d:` 拿到的第一欄裏裹着轉義序列。
struct Ink(bool);

impl Ink {
    fn path(&self, text: &str) -> String {
        self.wrap("35", text)
    }
    fn number(&self, text: &str) -> String {
        self.wrap("32", text)
    }
    fn hit(&self, text: &str) -> String {
        self.wrap("1;31", text)
    }
    fn wrap(&self, how: &str, text: &str) -> String {
        match self.0 {
            true => format!("\u{1b}[{how}m{text}\u{1b}[0m"),
            false => text.to_string(),
        }
    }
}

/// `full` 相對於 `from` 怎麼寫——走不到就原樣交絕對路徑。
fn pathdiff(full: &std::path::Path, from: &std::path::Path) -> std::path::PathBuf {
    // Warning: **不許 canonicalize 要印的那一個**（2026-10-03 一輪審查報來的）。
    // `ye --grep alpha alias` 從前印 `realdir/f.md`——使用者問的是 `alias`，拿回
    // 來的是別的名字，`cd` 過去落在別處；連着的目錄指到樹外面的話還會印出一串
    // `../../..`。**問什麼就答什麼。**
    let a = full.to_path_buf();
    let b = std::fs::canonicalize(from).unwrap_or_else(|_| from.to_path_buf());
    let mut ours = a.components().peekable();
    let mut theirs = b.components().peekable();
    // `full` 本身可能是相對的（搜當前目錄的時候就是），那就已經是答案了。
    if a.is_relative() {
        return a;
    }
    while ours.peek().is_some() && ours.peek() == theirs.peek() {
        ours.next();
        theirs.next();
    }
    // Warning: **只有在底下纔寫成相對的**（2026-10-03 改）。爬出去的那一種寫成
    // `../` 本來是想讓人看見「它上去過」，可從一個深目錄搜 `~` 印出來的是七層
    // `../../../..`——那不是提示，是噪音。rg 在這裏印的是你給它的那個路徑，所以
    // 不在底下就原樣交絕對路徑：一眼看得出搜到了別處去。
    match theirs.next().is_none() {
        true => ours.collect(),
        false => full.to_path_buf(),
    }
}

/// `WIDTHxHEIGHT`, for `--shot`. Anything unreadable is the default page.
/// `WxH` for `--shot`, or why it is not that.
///
/// **It used to fall back rather than refuse** — every one of the three steps
/// below silently produced the default instead — so `--shot=40,10` drew a
/// 100×30 frame and said nothing, exit code and all. That is a bad trade for a
/// diagnostic tool: a picture that is quietly of the wrong thing is worse than
/// no picture, and in 2026-09-11's review it wasted a whole afternoon of six
/// reviewers' terminal-size findings before anyone noticed (#389).
///
/// A comma is taken as well as an `x`, because it is what everyone tries first.
fn parse_size(text: &str) -> Result<(u16, u16), String> {
    let text = text.trim();
    let Some((w, h)) = text.split_once(['x', 'X', '*', ',']) else {
        return Err(format!(
            "--shot wants a size like 100x30, not {text:?} (`x`, `X`, `*` or `,` between them)"
        ));
    };
    // Warning: **Both ends are refused, and both used to get through.** Zero is not
    // a small terminal, it is no terminal: `--shot=100x0` panicked in the
    // renderer (`index outside of buffer: the area is Rect { width: 100,
    // height: 0 }`) and `--shot=0x30` subtracted past zero drawing a menu. And
    // the old ceiling was `u16::MAX`, which is not a size either —
    // `--shot=20000x20000` allocated **56 GB** before it got anywhere, and
    // `65535x65535` never came back. A real terminal cannot reach `MAX_CELLS`;
    // a typo can.
    const MAX_CELLS: u32 = 2_000;
    let read = |part: &str, which: &str| -> Result<u16, String> {
        let part = part.trim();
        // Two different complaints, because they are two different mistakes:
        // `40x` is a typo, `40x999999` is a number nobody meant.
        match part.parse::<u32>() {
            Err(_) => Err(format!("--shot: {which} is not a number: {part:?}")),
            Ok(0) => Err(format!("--shot: {which} is 0, and nothing can be drawn in it")),
            Ok(n) if n > MAX_CELLS => Err(format!(
                "--shot: {which} is {n}, and a terminal is at most {MAX_CELLS} cells across"
            )),
            Ok(n) => Ok(n as u16),
        }
    };
    Ok((read(w, "the width")?, read(h, "the height")?))
}


/// Load a scheme's 碼表 at startup, for a config that asked for one.
fn switch_scheme_at_startup(
    ime: &mut ImeSession,
    wanted: Scheme,
    (page_size, chaifen): (usize, bool),
) -> String {
    let mut full = ImeSession::from_default_dirs(wanted);
    // The first installed scheme, not 靈明: on a build that ships only 冰雪
    // there is no 靈明 to retreat to.
    let fallback = Scheme::first();
    if !full.available() && wanted != fallback {
        full = ImeSession::from_default_dirs(fallback);
    }
    full.set_page_size(page_size);
    full.set_annotations(chaifen);
    let name = full.scheme_name().to_string();
    let ok = full.available();
    *ime = full;
    if ok {
        yumete_core::say!("cli.scheme", name)
    } else {
        yumete_core::say!("cli.scheme-table-not-installed", name)
    }
}

fn print_help() {
    // **What is installed, not what was compiled in** (#169). A `--help` run
    // stops before the launch reaches the scan, so the scan happens here — it
    // is idempotent, and the reader who runs `--help` to find out what to pass
    // `:yume-scheme` is asking exactly this question. The config's own
    // `data_dirs` are read first for the same reason: a scheme installed
    // somewhere only the config knows about is still installed.
    let (config, _) = yumete_config::Config::load_reporting();
    if !config.ime.data_dirs.is_empty() {
        yumete_config::set_data_dirs(config.ime.data_dirs.clone());
    }
    yumete_ime::discover(&yumete_config::data_search_dirs());
    let schemes: String = yumete_ime::Scheme::all()
        .iter()
        .map(|s| s.tag())
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "yumete {VERSION} — a CJK-aware, Helix-like terminal editor with a built-in Yume IME.

USAGE:
    yumete [FILE]...

ARGS:
    FILE    One or more files to open. Each is loaded into its own buffer;
            a file that does not yet exist opens an empty buffer bound to it.
            With no FILE, an empty scratch buffer; --continue (or `[editor]
            session = true`) opens again what was open last time in this
            directory, each at the line it was left on.

OPTIONS:
    Single-letter flags are gone for now, -h and -V aside: they are the
    scarcest thing a command line has, and which ones are worth spending is
    a question to answer once the search flags below have settled.

        --table      Read the file as a grid. A schema in .yumete/tables/ names
                     the columns; without one, the file's own header row does.
                     A grid is always horizontal, so this overrides --vertical.
        --vertical   Lay the text out vertically for this run (縱書), overriding
                     the config. --horizontal forces the ordinary layout.
        --readonly   Open locked: nothing this run opens can be typed into.
                     `:readonly off` unlocks the one you are looking at.
        --continue   Reopen the files that were open last time here.
        --new        Start on an empty buffer even when `[editor] session` is
                     on.
        --tutor      Open the lesson (the same as `:tutor` inside the editor).
        --lang=LANG  Which language the editor says things in for this run:
                     zh (繁體, the default), zhs (简体) or en. `[editor]
                     language` in the config is the standing answer, and
                     `:language` switches it without restarting.
        --syntax     Which markup these files are written in: markdown, typst
                     or text. Outranks both the extension and the config.
        --preview    Print a non-interactive preview instead of the editor.

SEARCHING FROM THE SHELL:
    -G, --grep PAT   Print every place PAT is, as `file:line:column:text`, and
                     exit — 0 if anything was found, 1 if nothing was, 2 if the
                     question would not parse. The editor never opens.

                     The word goes in its own argument, never stuck to the
                     letter: `-G zhongguo`, not `-Gzhongguo`. That is what
                     lets the short names be strung together — `-GO` can only
                     be read one way.

                     **拼音 and 繁簡 are on**, which is the whole point:
                     `ye --grep zhongguo` finds 中國 and 中国 both, and no
                     other grep on the machine can. `--chinese=off` turns it
                     off; `glyphs` and `pinyin` take one half each.

                     The text column is sixty characters either side of the
                     match, not the whole line: a line of a novel is a
                     paragraph.

                     Searches the current directory, as every shell tool does.
                     Name a directory to search that one instead, or
                     --project to search up to the book's root (the nearest
                     .yumete or .git above you).

                     A path is printed the way you spelt it — name a symlinked
                     directory and you get it back under that name. With no
                     directory named, paths are relative to where you stand;
                     anything outside that is printed in full.

                     Several directories may be named, and a FILE may be named
                     instead of a directory.

        --files PAT  Print every file whose name matches PAT, best first, and
                     exit — fd's half, and it reads 拼音 too: `ye --files jia`
                     finds 甲.md. Same matcher as 空格 f inside the editor.

                     Letters and 漢字 mix in one query, which is how a real
                     one is written: `di120` finds 第120章.md, and
                     `juan01/di120` finds exactly that chapter. Files only —
                     a directory is never an answer here, unlike fd. --grep
                     reads a query the same way: `zhongguo很大`, `zhong国` and
                     `中guo` all find 中國很大.

                     --files takes --hidden, --glob=, --exclude= and --project.
                     The switches that shape a *pattern* belong to --grep, and
                     --files refuses them rather than ignoring them: a name is
                     matched by feel here, and there is no pattern to shape.

    -O, --open       Open the editor on the answer instead of printing it:
                     --grep with --open comes up with the panel already run and
                     the keys on the first hit; --files with --open comes up
                     with the picker open and the query already typed. This is
                     what the search is for: `ye -GO zhongguo`.

                     The two short names are upper case on purpose. They say
                     *which question is being asked*, where every switch below
                     only shapes one search — and the lower-case letters are
                     rg's and fd's (`-g` is --glob to both of them, `-o` is
                     rg's --only-matching), so taking them would fight a habit
                     rather than borrow it. --files has no short name yet.

        --project          Search up to the project root, not here.
        --ignore-case      Case never matters. (Default: a capital in the
        --case-sensitive   pattern is how you ask for case to matter.)
        --word             Whole words only — Latin words; 漢語 has no spaces.
        --regex            Read the pattern as a regular expression.
        --fixed            Read it literally. (The default.)
        --fuzzy            These characters, nearly in a row.
        --chinese=WHICH    off / glyphs (繁簡) / pinyin / both. Default: both.
        --hidden           Search hidden files and the ones .gitignore names.
        --no-ignore        The same switch: yumete keeps them together.
        --glob=G           Only files matching these globs (comma-separated).
        --exclude=G        Never these.
        --color=WHICH      auto (the default: colour on a terminal, plain in a
                           pipe) / always / never. rg's palette, because that
                           is where the eye's habits are: path magenta, line
                           and column green, the match itself bold red.
        --heading          Group the hits under the file they are in, the way
        --no-heading       rg does, with a blank line between files. On a
                           terminal that is the default; in a pipe the default
                           is one self-contained `file:line:column:text` line
                           per hit, so awk and xargs still read it.

                     What --grep and --files say for themselves — refusals,
                     diagnostics — is **English**, whatever `[editor] language`
                     is set to: it is read by other programs and by whoever is
                     assembling the pipe. `--lang` still wins if you ask.
        --shot[=WxH] Draw one frame — the page exactly as the editor would set
                     it — to standard output and exit. 100x30 by default.
                     `:shot` inside the editor draws the same picture into a
                     file; this is it without opening the editor at all.
        --keys=KEYS  Press these before the picture is taken, so a panel that
                     opens on the third keystroke can be looked at:
                     `\\e` Esc, `\\t` Tab, `\\n` Enter, `\\b` Backspace,
                     `\\^x` Control-x, `\\{{home}}` `\\{{end}}` `\\{{pgup}}`
                     `\\{{pgdn}}` `\\{{del}}` `\\{{backtab}}` `\\{{space}}`
                     `\\{{alt-d}}` `\\{{ctrl-w}}`, `\\u\\d\\l\\r` the arrows.
                     Every one of those also has a spelt-out name, so a script
                     need not mix the two styles: `\\{{up}}` `\\{{down}}`
                     `\\{{left}}` `\\{{right}}` `\\{{tab}}` `\\{{enter}}`
                     `\\{{esc}}` `\\{{bs}}`.
                     A name this list does not hold is an error, not a warning.
                     `--keys='::竖排'` opens the command search with that in it.
        --html       With --shot: the frame **with its colours**, as one
                     self-contained HTML <pre>. What a theme is judged on.
        --timing     Print how long each part of starting up took, and exit.
    -h, --help       Print this help and exit.
    -V, --version    Print the version and exit.

KEYS (Normal mode, Helix-style):
    3w 10j    a digit prefix repeats the motion or edit that follows
    h j k l   move by grapheme / line (CJK-width aware)
    w b e     next / prev word start, word end (W B E for WORDs)
    gg  ge    goto buffer start / last line (10gg goes to line 10)
    gn  gp    show the next / previous open file
    gf        open the file:line named on this line (`:search-project` results)
    Space     menu: o outline, f files, b buffers, / search, ? commands,
              d 字典 (how the character under the cursor is written), y copy,
              r 旁注 (edit the reading here)
    C-w       move between the sidebar and the text
    gh gl gs  goto line start / end / first non-blank
    {{ }}     previous / next paragraph — here a paragraph is a logical line
    M a  ' a  name this place / go back to it, across files
    C-o C-i   the jump list: back to where a jump came from, and forward
    f F       find a character, forward / backward; A-. repeats it
              (**no t / T** — that letter is the table mode's, all of it)
    H  L      previous / next sentence — 。！？ and the mark that closes after
    J  K      forward / back half a page   (C-f / C-b for a whole one)
    x  X      select the current line / extend to whole lines
    v  ;  %   select (extend) mode / collapse / select the whole file
    d  c  R   delete / change / replace the selection with the register
    y  p  P   yank / paste after / before
    i  a      insert before / after the selection
    I  A      insert at line start / end
    o  O      open a line below / above
    u  U  .   undo / redo / **repeat the last change** (r, d, c…Esc, ms(, a paste)
    r         write the next key over every character of the selection —
              **中文 too**: the panel opens, and what you choose is what the
              selection becomes (a word replaces it rather than filling it)
    \"a        use register a for the next yank / delete / paste
    Q  q      record a macro / play the last one back
    A-;       flip which end of the selection the cursor is on
    C-d C-u   half a page onward / back (down the lines, or across the 縱)
    C-f C-b   a whole page
    gJ        join with the line below (no space between two 全角 characters)
    `         letter case: `l lower, `u upper, `` switch
    >  <      indent / unindent the selected lines
    C-a C-x   increment / decrement the number at the cursor
    m         match mode: mm jump to the matching bracket; mi/ma select
              inside/around a pair; ms surround, md delete, mr replace —
              「」『』（）《》【】〔〕 and the ASCII pairs
    / ? n N   search forward / backward; next / previous match
    g/ g?     find what is selected: jump to it / show it in the other pane
    t/ t?     the same, down a table's columns (t2-10? names the columns)
    :         command line — Tab cycles the completion, and the list of
              commands appears above it and narrows as you type
              (:w  :w <path>  :q  :q!  :o <path>  :new
              :word  :wq  :count
              :yume-scheme <tag>   {schemes}
              :view-wrap [on|off|<n>]  soft-wrap; a number is a fixed measure
              :wq [path]       save (optionally save-as) and quit
              :42  :goto n    put the cursor on a line
              :recover[!]      load (or drop) a crash-recovery draft
              :buffer|next|previous|close   the open files (gn / gp)
              :search-project <re>  :toc  across the project / this file's headings
              :export html|typst    write it out for a typesetter
              :layout [horizontal|vertical]      :view-margin [never|dense|loose|always]
              :yume-chaifen   the 拆分 annotation beside candidates
              :yume-where     the six places 碼表 and 字料 are looked
                              for, and what each one holds
              :ruby       edit the reading at the cursor, or annotate the
                          selection — opens Ruby mode in the status line
              :s/re/new/[ginfc]  regex; $1 captures, \\n a newline. The delimiter
                          is whatever follows the s (:s#a/b#c#); ranges are
                          :%s :1-40s :1,5,9s :.-$s; flags g i, f is literal,
                          c asks at each match, and n counts only
              :replace <new>   change what the last :grep found, everywhere
              :write-all       save every file that changed
              :indent off|basic|full|<n>   first-line indent — a Chinese
                          paragraph's mark; full also folds the blank line
              :view-bands [n|off]   段組: divide the 縱書 page into bands
              :word-list reload   reread .yumete/words.txt — this book's own names
              :word-habit      口頭禪: what this one says far more than prose does
              :table [off|basic|full|check]   edit as a grid; check looks the
                          whole over
              :table-jump <char>  the row a table names by that character
              :ruby [off|basic|full|<dialect>]  full lays the readings out,
                          basic keeps the markup on the page and still counts
                          a reading as no 字, off is the source)

In Insert mode C-w takes back a word and C-u the line. The `:` and `/` prompts
are editable: arrows, Home/End, C-w, C-u, and Up/Down through what you typed
before. A key that means something in another editor and nothing here says what
this one calls it — `$` answers 「行尾是 gl」 rather than doing nothing.

Ruby mode (`:ruby`) edits the *reading*, which with readings laid out is not on
screen to move the cursor into. It opens on the group under the cursor with its
current reading loaded, or on the selection with an empty one; Enter writes it,
an empty reading takes the annotation off, Esc leaves it alone. A reading split
by `|` into as many parts as the base has characters annotates each character
separately — `hàn|zì` over 漢字 — while `hàn zì` stays one reading over the word.

Laid out vertically, text runs top to bottom in 縱 that stack from the right
edge leftward, wrapping at whatever the window allows (`zong_length`). h j k l keep their
screen meaning: j and k read down and up a 縱, h and l step to the 縱 on the
left and on the right. CJK punctuation is drawn in its vertical form; the file
on disk is unchanged.

In Insert mode, type to insert; Esc returns to Normal. If the Yume IME data is
installed, Insert mode composes CJK: type a code to see candidates, Space or
1–9 to select, -/= to page, Backspace to edit, Esc to cancel; tap Shift to
toggle 中/英 (needs a terminal with the Kitty keyboard protocol)."
    );
}

/// Print a status line and the contents of the active buffer, with line numbers
/// — or, in vertical layout, the page as it would be drawn.
fn preview(editor: &Editor, config: &yumete_config::Config) {
    let buf = editor.current_buffer();
    let modified = if buf.is_modified() { " [+]" } else { "" };
    let extra = if editor.buffer_count() > 1 {
        format!("  ({} buffers open)", editor.buffer_count())
    } else {
        String::new()
    };

    // Written through a locked handle rather than `println!`, which *panics*
    // when the reader goes away: `yumete -p 稿.md | less`, then `q`, used to
    // print a Rust backtrace. `?` here means "the pipe closed", and the only
    // right answer to that is to stop.
    let mut out = io::stdout().lock();

    // Status line.
    let _ = (|| -> io::Result<()> {
        writeln!(
            out,
            "── {name}{modified} — {lines} line(s), {chars} char(s){extra} ──",
            name = buf.display_name(),
            lines = buf.line_count(),
            chars = buf.char_count(),
        )?;

        // A recovery draft is the one thing a reader must be told about before
        // they trust what follows (Feature #79).
        if buf.recovered_draft().is_some() {
            writeln!(
                out,
                "!! a newer draft was recovered — :recover to load it in the editor"
            )?;
        }

        if buf.char_count() == 0 {
            writeln!(out, "(empty buffer)")?;
            return Ok(());
        }

        // Vertical layout (Feature #61): print the page itself. Line numbers
        // would mean nothing here — the reading order is what there is to look
        // at.
        if editor.layout() == Layout::Vertical {
            let hidden = |line: usize| editor.markup_hidden_on_line(line);
            let folded = |line: usize| editor.line_is_folded(line);
            let drawn = |line: usize| editor.drawn_runs_on_line(line);
            let turned = |line: usize| editor.line_is_table_row(line);
            let grid = editor.grid_with(&hidden, &folded, &drawn, &turned);
            for line in yumete_core::zong::render_page(buf.rope(), grid, config.editor.zong_gap) {
                writeln!(out, "{line}")?;
            }
            return Ok(());
        }
        write_wrapped(&mut out, buf, config)
    })();
}

/// Print the buffer as numbered, soft-wrapped rows.
fn write_wrapped(
    out: &mut impl Write,
    buf: &yumete_core::Buffer,
    config: &yumete_config::Config,
) -> io::Result<()> {
    // ropey counts a trailing "\n" as starting an extra empty line; don't print
    // that phantom final line in the preview.
    let text = buf.text();
    let mut count = buf.line_count();
    if text.ends_with('\n') {
        count -= 1;
    }

    // The gutter is sized from the buffer's own line count, the way the editor
    // sizes it, so the preview wraps at the width the editor would.
    let width = buf.line_count().max(1).to_string().len();
    // ` │ ` between the number and the text, against the editor's single space.
    let gutter = width + 3;
    let wrap = if config.editor.soft_wrap {
        terminal_width().saturating_sub(gutter)
    } else {
        usize::MAX / 2
    }
    .max(yumete_core::wrap::MIN_WRAP_WIDTH);
    for i in 0..count {
        let Some(line) = buf.line(i) else { continue };
        let chars: Vec<char> = line.chars().collect();
        for (row, (start, end)) in yumete_core::wrap::line_rows(&line, wrap)
            .into_iter()
            .enumerate()
        {
            let text: String = chars[start..end].iter().collect();
            // The number labels the paragraph, so only its first row carries one.
            if row == 0 {
                writeln!(out, "{:>width$} │ {text}", i + 1, width = width)?;
            } else {
                writeln!(out, "{:>width$} │ {text}", "", width = width)?;
            }
        }
    }
    Ok(())
}

/// How wide the preview wraps at: the terminal, or the 80 columns a terminal
/// has had since the punched card when there is none to ask — a pipe into
/// `less`, or output redirected to a file.
fn terminal_width() -> usize {
    yumete_tui::terminal_width().unwrap_or(80)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **`--keys` 在兩個鍵之間要補上主循環那一幀**，否則走磁碟的名單走不動。
    ///
    /// 2026-10-02 一輪黑盒審查報來的：`--keys='␣/霜\n66jjjj'` 在工作路徑那一檔
    /// 只走得到眼前這一份自己的那幾處，往後按多少下 `j` 都不動。核心裏同一串鍵
    /// 走得好好的——差的是這支工具：那一趟搜索是欠着的，而整串鍵都餵在它還清
    /// 之前，於是每一個 `j` 走的都是本文件那一檔的舊名單。
    #[test]
    fn keys_pressed_offscreen_wait_for_the_search_the_way_a_frame_would() {
        let dir = std::env::temp_dir().join(format!("yumete-press-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("卷一")).unwrap();
        std::fs::write(dir.join(".yumete"), "").unwrap();
        std::fs::write(dir.join("甲.md"), "霜一\n霜二\n").unwrap();
        std::fs::write(dir.join("卷一/乙.md"), "霜三\n").unwrap();

        let config = yumete_config::Config::default();
        let mut ime = ImeSession::empty(yumete_ime::Scheme::LINGMING);
        let mut editor = Editor::new();
        editor.set_root(&dir);
        editor.open_file(dir.join("甲.md")).unwrap();

        // 開面板、打一個字、Enter，再把範圍轉到工作路徑，然後一路往下走。
        // `6` 兩下是 本文件 → 緩衝區 → 工作路徑；`j` 四下到名單，再四下走到底。
        press(&mut editor, "\\{space}/霜\\n66jjjjjjjj", &config, &mut ime, Some((100, 30)));

        assert_eq!(
            editor.current_buffer().display_name(),
            "乙.md",
            "走到別的檔上去了——走不到就是這支工具落後了一幀"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// **A size it cannot read is a refusal, not a default** (#389).
    ///
    /// Every step of this used to fall back: an unrecognised separator, a
    /// width that is not a number, a height that is not a number. So
    /// `--shot=40,10` drew 100×30 and said nothing — and a picture quietly of
    /// the wrong thing is worse for a diagnostic tool than no picture at all.
    #[test]
    fn a_shot_size_is_read_or_refused_never_guessed() {
        assert_eq!(parse_size("40x10"), Ok((40, 10)));
        assert_eq!(parse_size("40X10"), Ok((40, 10)));
        assert_eq!(parse_size("40*10"), Ok((40, 10)));
        // A comma, because it is what everyone tries first — and what the
        // review's own instructions told six people to type.
        assert_eq!(parse_size("40,10"), Ok((40, 10)));
        assert_eq!(parse_size(" 40 x 10 "), Ok((40, 10)));

        // Refusals, and each one says which half is wrong.
        for bad in ["", "abc", "40", "x", "40x", "x10"] {
            assert!(parse_size(bad).is_err(), "{bad:?} should be refused");
        }
        // `split_once` takes the first separator, so the rest is the height —
        // and `10x2` is not a height.
        assert!(parse_size("40x10x2").is_err());
        let too_wide = parse_size("999999x1").unwrap_err();
        assert!(too_wide.contains("2000"), "{too_wide}");
        assert!(parse_size("40xzz").unwrap_err().contains("the height"));
        assert!(parse_size("zzx10").unwrap_err().contains("the width"));

        // **Zero is refused at both ends.** `--shot=100x0` panicked in the
        // renderer and `--shot=0x30` subtracted past zero drawing a menu;
        // neither is a terminal a reader can have, but both are a typo away.
        assert!(parse_size("100x0").unwrap_err().contains("the height"));
        assert!(parse_size("0x30").unwrap_err().contains("the width"));
        // And so is a size that is a memory bill rather than a screen:
        // `20000x20000` reached 56 GB before it drew anything.
        assert!(parse_size("20000x20000").is_err());
        assert!(parse_size("65535x65535").is_err());
        // The ceiling is generous — wider than any terminal, well short of
        // a number nobody meant.
        assert_eq!(parse_size("2000x2000"), Ok((2000, 2000)));
    }
}
