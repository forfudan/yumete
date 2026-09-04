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
use yumete_core::{Editor, TextStore};
use yumete_ime::{CommitStrategy, ImeSession, Scheme};

const VERSION: &str = env!("CARGO_PKG_VERSION");

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
            "--timing" => timing = true,
            "--shot" => shot = Some((100, 30)),
            s if s.starts_with("--shot=") => shot = Some(parse_size(&s["--shot=".len()..])),
            "--html" => shot_html = true,
            "-s" | "--syntax" => want_syntax = true,
            s if s.starts_with("--syntax=") => {
                force_syntax = Some(s["--syntax=".len()..].to_string())
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
    let (config, config_problems) = yumete_config::Config::load_reporting();
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
            yumete_tui::width::ask_the_terminal_about_width().unwrap_or(false)
        }
    });
    // …and the language everything says itself in, before anything says
    // anything: a config error is a message too.
    if let Some(language) = yumete_core::messages::Language::parse(&config.editor.language) {
        yumete_core::messages::set_language(language);
    }
    editor.set_key_aliases(config.keys.normal.clone());
    // Layout (Feature #61): the config sets it, a flag overrides for one run,
    // and `:layout` switches it live.
    editor.set_layout(force_layout.unwrap_or(config.editor.layout));
    if config.editor.indent > 0 {
        editor.set_indent(config.editor.indent);
    }
    if config.editor.bands > 1 {
        editor.set_bands(config.editor.bands);
    }
    if config.editor.zong_length > 0 {
        editor.set_zong_length(config.editor.zong_length);
    }
    // The measure a project writes to, if it has said one; `:wrap n` is the
    // same setting for one session.
    if config.editor.measure > 0 {
        editor.set_measure(Some(config.editor.measure));
    }
    editor.set_indent_width(config.editor.tab_width);
    editor.set_tatechuyoko(config.editor.tatechuyoko);
    editor.set_hanging_punctuation(config.editor.hanging_punctuation);
    editor.set_soft_wrap(config.editor.soft_wrap);
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
    // …and what it says about particular extensions or names.
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
    // Somewhere for a buffer with no file to keep its recovery copy. Only the
    // front end knows where the data directory is.
    // Where `:word list global` writes, and where a reader's own dictionary is
    // read from — the front end's answer, because only it knows XDG.
    editor.keep_word_list_in(yumete_config::data_dir());
    let drafts = yumete_config::data_dir().join("drafts");
    if std::fs::create_dir_all(&drafts).is_ok() {
        editor.keep_drafts_in(drafts);
    }
    // …and somewhere to remember which files were open. Keyed by the working
    // directory, so a novel and a codebase do not share one.
    if config.editor.session {
        if let Ok(here) = std::env::current_dir() {
            editor.keep_session_in(yumete_config::data_dir().join("sessions"), &here);
        }
    }
    editor.set_dense(config.editor.dense);
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
    // With no file named, open what was open last time — five `:open`s every
    // morning is five too many. Only then: somebody who said which file they
    // wanted gets that file, and nothing else.
    // …and not when printing: `yumete -p` and `yumete | cat` are a look at one
    // thing, not a return to work.
    let printing = force_preview || !std::io::stdout().is_terminal();
    let restored = if files.is_empty() && !printing {
        editor.restore_session()
    } else {
        0
    };
    mark("session", &mut marks);
    // `-t` is the writer saying "this is a table" about a file no schema names.
    // After the files, because it is about the file that is open.
    if force_table {
        editor.enter_table();
    }

    // Which ruby dialect to lay out: whatever the config names, else the one
    // the file's extension implies.
    editor
        .execute(if config.editor.show_ruby {
            ":ruby on"
        } else {
            ":ruby off"
        })
        .ok();
    for name in &config.editor.ruby_dialects {
        let _ = editor.execute(&format!(":ruby {name}"));
    }

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
    // waits for `:yume scheme`.
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
    editor.set_chaifen(config.editor.show_chaifen);
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
            // which for the author of an input method it usually is.
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
    // One answer, in `yumete_tui::choose_words`, so `:word list reload` cannot
    // pick a different dictionary from the one start-up picked.
    editor.set_segmenter(yumete_tui::choose_words(&ime, config.editor.word_level));
    editor.set_word_level(config.editor.word_level);
    editor.set_segmentation_visible(config.editor.show_segmentation);
    if let Some(rules) = yumete_core::table::Rules::parse(&config.editor.table_rules) {
        editor.set_table_rules(rules);
    }
    editor.set_number_fill(config.editor.line_number_fill);
    if let Some(hint) = yumete_core::zong::IndentHint::parse(&config.editor.indent_hint) {
        editor.set_indent_hint(hint, Some(config.editor.indent_symbol.clone()));
    }
    // …and the book's own words on top of whichever of the three it was. The
    // name on every page is the one word no dictionary has.
    editor.reload_project_words();
    editor.set_status(String::new());
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

    // A config file that does not parse is worth one line: silence is how a
    // typo comes to look like a setting that does not work.
    if !config_problems.is_empty() {
        for problem in &config_problems {
            eprintln!("yumete: {problem}");
        }
        editor.set_status(config_problems.join(&yumete_core::say!("label.comma")));
    }
    if let Some((width, height)) = shot {
        // The layout the flags asked for, before the picture is taken.
        let picture = match shot_html {
            true => yumete_tui::frame_to_html(&mut editor, &config, &ime, width, height),
            false => yumete_tui::frame_to_text(&mut editor, &config, &ime, width, height),
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

    let outcome = yumete_tui::run(&mut editor, &config, &mut ime, deferred);
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

/// `WIDTHxHEIGHT`, for `--shot`. Anything unreadable is the default page.
fn parse_size(text: &str) -> (u16, u16) {
    let (w, h) = text.split_once(['x', 'X', '*']).unwrap_or(("100", "30"));
    (
        w.trim().parse().unwrap_or(100),
        h.trim().parse().unwrap_or(30),
    )
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
    println!(
        "yumete {VERSION} — a CJK-aware, Helix-like terminal editor with a built-in Yume IME.

USAGE:
    yumete [FILE]...

ARGS:
    FILE    One or more files to open. Each is loaded into its own buffer;
            a file that does not yet exist opens an empty buffer bound to it.
            With no FILE, yumete opens again what was open last time in this
            directory, each at the line it was left on — or an empty scratch
            buffer the first time.

OPTIONS:
    -t, --table      Read the file as a grid. A schema in .yumete/tables/ names
                     the columns; without one, the file's own header row does.
                     A grid is always horizontal, so this overrides -v.
    -v, --vertical   Lay the text out vertically for this run (縱書), overriding
                     the config. -H / --horizontal forces the ordinary layout.
    -R, --readonly   Open locked: nothing this run opens can be typed into.
                     `:readonly off` unlocks the one you are looking at.
    -s, --syntax     Which markup these files are written in: markdown, typst
                     or text. Outranks both the extension and the config.
    -p, --preview    Print a non-interactive preview instead of the editor.
        --shot[=WxH] Draw one frame — the page exactly as the editor would set
                     it — to standard output and exit. 100x30 by default.
                     `:shot` inside the editor hands the screen to the
                     platform's screenshot program; this is the same picture on
                     a machine with no window, for a report or a review.
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
    gf        open the file:line named on this line (`:grep` results)
    Space     menu: e sidebar, o outline, f files, b buffers, / search, y copy
    C-w       move between the sidebar and the text
    gh gl gs  goto line start / end / first non-blank
    {{ }}       previous / next paragraph — here a paragraph is a logical line
    ( )       previous / next sentence — 。！？ and the mark that closes after
    M a  ' a  name this place / go back to it, across files
    C-o C-i   the jump list: back to where a jump came from, and forward
    f t F T   find / till a character (forward / backward); A-. repeats it
    J  K      forward / back half a page   (L / H for a whole one)
    x  X      select the current line / extend to whole lines
    v  ;  %   select (extend) mode / collapse / select the whole file
    d  c  R   delete / change / replace the selection with the register
    y  p  P   yank / paste after / before
    i  a      insert before / after the selection
    I  A      insert at line start / end
    o  O      open a line below / above
    u  U  .   undo / redo / **repeat the last change** (r, d, c…Esc, ms(, a paste)
    r         write the next key over every character of the selection
    \"a        use register a for the next yank / delete / paste
    q  Q      record a macro / play the last one back
    A-;       flip which end of the selection the cursor is on
    C-d C-u   half a page onward / back (down the lines, or across the 縱)
    C-f C-b   a whole page
    gJ        join with the line below (no space between two 全角 characters)
    ~  `      switch case / lowercase the selection (A-` uppercases)
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
              :segment  :wq  :count
              :yume scheme <tag>   lingming xingchen qingyun riyue pinyin
              :wrap [on|off|<n>]   soft-wrap; a number is a fixed measure
              :wq [path]       save (optionally save-as) and quit
              :42  :goto n    put the cursor on a line
              :recover[!]      load (or drop) a crash-recovery draft
              :buffer list|next|previous|close   the open files (gn / gp)
              :grep <re>  :toc      across the project / this file's headings
              :export html|typst    write it out for a typesetter
              :layout [horizontal|vertical]      :dense [on|off]
              :yume chaifen   the 拆分 annotation beside candidates
              :ruby       edit the reading at the cursor, or annotate the
                          selection — opens Ruby mode in the status line
              :s/re/new/[gin]  regex; $1 captures, \\n a newline. The delimiter
                          is whatever follows the s (:s#a/b#c#); ranges are
                          :%s :1,40s :.,$s :40s; flags g i, and n counts only
              :replace <new>   change what the last :grep found, everywhere
              :wa              save every file that changed
              :indent [n|off]  first-line indent — a Chinese paragraph's mark
              :bands [n|off]   段組: divide the 縱書 page into bands
              :words           reload .yumete/words.txt — this book's own names
              :table [off|check]   edit as a grid; check looks the whole over
              :row <char>      go to the row a table names by that character
              :ruby [on|off|<dialect>]  lay readings out, or show the markup)

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
            let ghost = |line: usize| editor.ghost_on_line(line);
            let grid = editor.grid_with(&hidden, &folded, &ghost);
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
