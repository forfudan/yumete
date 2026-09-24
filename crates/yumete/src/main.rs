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
    let mut want_syntax = false;
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

    for arg in std::env::args().skip(1) {
        if want_syntax {
            force_syntax = Some(arg);
            want_syntax = false;
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
            "-p" | "--preview" => force_preview = true,
            "-t" | "--table" => force_table = true,
            "-R" | "--readonly" => readonly = true,
            "-n" | "--new" => fresh = true,
            "-c" | "--continue" => resume = true,
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
            "-s" | "--syntax" => want_syntax = true,
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
            "-v" | "--vertical" => force_layout = Some(Layout::Vertical),
            "-H" | "--horizontal" => force_layout = Some(Layout::Horizontal),
            // Reject unknown flags, but treat a lone "-" as a filename.
            s if s.starts_with('-') && s != "-" => {
                eprintln!("yumete: unknown option '{s}'");
                eprintln!("try 'yumete --help'");
                return ExitCode::from(2);
            }
            s => files.push(s.to_string()),
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
    let (mut config, config_problems) = yumete_config::Config::load_reporting();
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
    // …and somewhere to remember which files were open. Keyed by the working
    // directory, so a novel and a codebase do not share one.
    //
    // **Always kept, whatever `session` says** (2026-09-17). `session` only
    // decides whether a bare `yumete` reopens it; `-c` reopens it on the day
    // it is off, and it can only do that if the files were written down.
    if let Ok(here) = std::env::current_dir() {
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
    for file in &files {
        if let Err(err) = editor.open_file(file) {
            eprintln!("yumete: cannot open '{file}': {err}");
            return ExitCode::FAILURE;
        }
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
    // Read after the first frame rather than before it — see `yumete_tui::
    // Deferred`. Everything it loads is a keystroke away; the page is not.
    let load = move |ime: &mut ImeSession| -> String {
        *ime = ImeSession::language_only(wanted);
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
    // ⚠️ **`-v` on a program file is not an error, and not silent either.**
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
    // Every picture from here on — `--shot`, `--html`, and `:shot` inside the
    // editor — carries the build under it, so a shot in a bug report says what
    // it is a picture of.
    yumete_tui::set_build(VERSION);
    let mut settings_page = None;
    if let Some(pressed) = &keys {
        settings_page = press(&mut editor, pressed);
    }
    // Asked for on the command line, and run the same way `:tutor` runs it.
    if tutor {
        let _ = editor.execute(":tutor");
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
        // ⚠️ **Two of them this path can honour, and should** (#470). The
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
            unheard.push(format!(":yume {asked}"));
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

    // Restoring five chapters without saying so leaves a person wondering
    // what they are looking at.
    if restored > 0 {
        editor.set_status(yumete_core::say!("cli.picked-up-where-you-left-off", restored));
    }
    // Recovered work outranks a config typo for the one status line there is.
    // Only the editor has one; the preview prints its own notice instead.
    editor.announce_recovery();

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
/// ⚠️ **`:settings` 之後的鍵歸那扇面板**，和主循環一樣。不接這一下，
/// `--shot --keys=':settings\nljj '` 拍到的永遠是面板剛開的樣子——而這個倉審前端
/// 就是靠拍照，一扇按不動的面板等於一扇沒法審的面板。回來的是那扇面板（要畫它）。
fn press(editor: &mut Editor, keys: &str) -> Option<yumete_config::panel::Panel> {
    let mut settings = yumete_tui::settings_page::Seat::default();
    let mut chars = keys.chars().peekable();
    while let Some(c) = chars.next() {
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
                    // `\{alt-.}` for the Meta chords. helix's tutor leans on
                    // them — `Alt-s` `Alt-.` `Alt-,` `Alt-;` `Alt-\`` — and
                    // without this none of that family could be pressed
                    // offscreen (2026-09-11).
                    if let Some(rest) = name.strip_prefix("alt-") {
                        match rest.chars().next() {
                            Some(c) if rest.chars().count() == 1 => Key::Alt(c),
                            _ => {
                                eprintln!("yumete: --keys: alt- wants one character, got {rest:?}");
                                break;
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
                                break;
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
                        // A name nobody knows is worth saying so: a silent
                        // fallback here is #389 all over again.
                        other => {
                            eprintln!("yumete: --keys: no key called {other:?}");
                            break;
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
        // ⚠️ **和主循環同一支** `Seat`，不是抄一遍：抄本當天就分岔過（那一份漏了
        // 「有改動不許一下走」的閘，又把存盤的錯 `let _ =` 吞掉）。
        if settings.took(editor, Some(key)) {
            continue;
        }
        editor.on_key(key);
        settings.settle(editor);
    }
    settings.panel
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
    // ⚠️ **Both ends are refused, and both used to get through.** Zero is not
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
            With no FILE, an empty scratch buffer; -c (or `[editor] session =
            true`) opens again what was open last time in this directory,
            each at the line it was left on.

OPTIONS:
    -t, --table      Read the file as a grid. A schema in .yumete/tables/ names
                     the columns; without one, the file's own header row does.
                     A grid is always horizontal, so this overrides -v.
    -v, --vertical   Lay the text out vertically for this run (縱書), overriding
                     the config. -H / --horizontal forces the ordinary layout.
    -R, --readonly   Open locked: nothing this run opens can be typed into.
                     `:readonly off` unlocks the one you are looking at.
    -c, --continue   Reopen the files that were open last time here.
    -n, --new        Start on an empty buffer even when `[editor] session` is
                     on.
        --tutor      Open the lesson (the same as `:tutor` inside the editor).
        --lang=LANG  Which language the editor says things in for this run:
                     zh (繁體, the default), zhs (简体) or en. `[editor]
                     language` in the config is the standing answer, and
                     `:language` switches it without restarting.
    -s, --syntax     Which markup these files are written in: markdown, typst
                     or text. Outranks both the extension and the config.
    -p, --preview    Print a non-interactive preview instead of the editor.
        --shot[=WxH] Draw one frame — the page exactly as the editor would set
                     it — to standard output and exit. 100x30 by default.
                     `:shot` inside the editor draws the same picture into a
                     file; this is it without opening the editor at all.
        --keys=KEYS  Press these before the picture is taken, so a panel that
                     opens on the third keystroke can be looked at:
                     `\\e` Esc, `\\t` Tab, `\\n` Enter, `\\b` Backspace,
                     `\\^x` Control-x, `\\{{home}}` `\\{{end}}` `\\{{pgup}}`
                     `\\{{pgdn}}` `\\{{del}}` `\\{{backtab}}` `\\{{alt-d}}` `\\{{ctrl-w}}`,
                     `\\u\\d\\l\\r` the arrows. `--keys='::竖排'` opens the
                     command search with that in it.
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
    gf        open the file:line named on this line (`:search-gd` results)
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
              :search-gd <re>  :toc  across the project / this file's headings
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
