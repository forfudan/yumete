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
use yumete_core::{DictionarySegmenter, Editor, TextStore};
use yumete_ime::{ImeSession, Scheme};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let mut files: Vec<String> = Vec::new();
    let mut force_preview = false;
    let mut force_layout: Option<Layout> = None;

    for arg in std::env::args().skip(1) {
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
    for file in &files {
        if let Err(err) = editor.open_file(file) {
            eprintln!("yumete: cannot open '{file}': {err}");
            return ExitCode::FAILURE;
        }
    }

    // Load global + per-project config and apply the keymap.
    let (config, config_problems) = yumete_config::Config::load_reporting();
    // Settled before anything is measured: every width question downstream —
    // wrap, gutter, cursor, the 縱 grid — asks the same global.
    yumete_core::set_ambiguous_wide(config.editor.ambiguous_wide);
    editor.set_key_aliases(config.keys.normal.clone());
    // Layout (Feature #61): the config sets it, a flag overrides for one run,
    // and `:layout` switches it live.
    editor.set_layout(force_layout.unwrap_or(config.editor.layout));
    editor.set_zong_length(config.editor.zong_length);
    // The measure a project writes to, if it has said one; `:wrap n` is the
    // same setting for one session.
    if config.editor.measure > 0 {
        editor.set_measure(Some(config.editor.measure));
    }
    editor.set_indent_width(config.editor.tab_width);
    editor.set_tatechuyoko(config.editor.tatechuyoko);
    editor.set_hanging_punctuation(config.editor.hanging_punctuation);
    editor.set_soft_wrap(config.editor.soft_wrap);
    editor.set_default_syntax(yumete_core::syntax::Syntax::parse(&config.editor.syntax));
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
    // Which ruby dialect to lay out: whatever the config names, else the one
    // the file's extension implies.
    editor
        .execute(if config.editor.show_ruby {
            ":ruby-on"
        } else {
            ":ruby-off"
        })
        .ok();
    for name in &config.editor.ruby_dialects {
        let _ = editor.execute(&format!(":render-ruby-{name}"));
    }

    // The built-in Yume IME (Feature #27): load the configured scheme's tables
    // from the data directory. When the data is absent the session is
    // unavailable and Insert mode simply types plain ASCII — which is also what
    // happens for a scheme whose tables are not installed, since only 靈明 ships
    // with yumete.
    let wanted = Scheme::from_tag(&config.ime.scheme).unwrap_or(Scheme::Lingming);
    let mut ime = ImeSession::from_default_dirs(wanted);
    if !ime.available() && wanted != Scheme::Lingming {
        eprintln!(
            "yumete: {} is not installed; falling back to 靈明",
            config.ime.scheme
        );
        ime = ImeSession::from_default_dirs(Scheme::Lingming);
    }
    ime.set_page_size(config.panel.page_size);
    editor.set_chaifen(ime.set_annotations(config.editor.show_chaifen));

    // Word segmentation, driving `w`/`b`/`e` and the overlay. Best first:
    //
    // 1. Yume's own language model (Feature #63) — 1.25M weighted entries plus
    //    the 詞彙表, already loaded above and shared by reference.
    // 2. A user `segmentation.txt` in the data directory.
    // 3. The compact list bundled with yumete, which covers common prose only.
    let threshold = config.editor.segmentation_threshold;
    let yume = ime.segmenter();
    if yume.is_available() {
        editor.set_segmenter(Box::new(yume));
    } else if let Some(dictionary) = load_segmentation_dictionary(threshold) {
        editor.set_segmenter(Box::new(dictionary));
    } else {
        editor.set_segmenter(Box::new(DictionarySegmenter::builtin(threshold)));
    }
    editor.set_segmentation_visible(config.editor.show_segmentation);

    // A config file that does not parse is worth one line: silence is how a
    // typo comes to look like a setting that does not work.
    if !config_problems.is_empty() {
        for problem in &config_problems {
            eprintln!("yumete: {problem}");
        }
        editor.set_status(config_problems.join("; "));
    }
    if force_preview || !std::io::stdout().is_terminal() {
        preview(&editor, &config);
        return ExitCode::SUCCESS;
    }

    // Recovered work outranks a config typo for the one status line there is.
    // Only the editor has one; the preview prints its own notice instead.
    editor.announce_recovery();

    match yumete_tui::run(&mut editor, &config, &mut ime) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("yumete: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Load a `word<TAB>weight` segmentation dictionary from the first
/// `segmentation.txt` found in the data search path, or `None` if none exists
/// (in which case the caller falls back to the bundled dictionary).
fn load_segmentation_dictionary(threshold: i64) -> Option<DictionarySegmenter> {
    for dir in yumete_config::data_search_dirs() {
        let path = dir.join("segmentation.txt");
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Some(DictionarySegmenter::from_text(&text, threshold));
        }
    }
    None
}

fn print_help() {
    println!(
        "yumete {VERSION} — a CJK-aware, Helix-like terminal editor with a built-in Yume IME.

USAGE:
    yumete [FILE]...

ARGS:
    FILE    One or more files to open. Each is loaded into its own buffer;
            a file that does not yet exist opens an empty buffer bound to it.
            With no FILE, yumete starts with a new, empty scratch buffer.

OPTIONS:
    -v, --vertical   Lay the text out vertically for this run (縱書), overriding
                     the config. -H / --horizontal forces the ordinary layout.
    -p, --preview    Print a non-interactive preview instead of the editor.
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
    f t F T   find / till a character (forward / backward); A-. repeats it
    J  K      forward / back half a page   (L / H for a whole one)
    x  X      select the current line / extend to whole lines
    v  ;  %   select (extend) mode / collapse / select the whole file
    d  c  R   delete / change / replace the selection with the register
    y  p  P   yank / paste after / before
    i  a      insert before / after the selection
    I  A      insert at line start / end
    o  O      open a line below / above
    u  U  .   undo / redo / repeat the last insert
    r         write the next key over every character of the selection
    \"a        use register a for the next yank / delete / paste
    q  Q      record a macro / play the last one back
    A-;       flip which end of the selection the cursor is on
    C-d C-u   half a page onward / back (down the lines, or across the 縱)
    C-f C-b   a whole page
    J         join with the line below (no space between two 全角 characters)
    ~  `      switch case / lowercase the selection (A-` uppercases)
    >  <      indent / unindent the selected lines
    C-a C-x   increment / decrement the number at the cursor
    m         match mode: mm jump to the matching bracket; mi/ma select
              inside/around a pair; ms surround, md delete, mr replace —
              「」『』（）《》【】〔〕 and the ASCII pairs
    / ? n N   search forward / backward; next / previous match
    *         search for whatever is selected
    :         command line — Tab cycles the completion, and the list of
              commands appears above it and narrows as you type
              (:w  :w <path>  :q  :q!  :o <path>  :new
              :s/re/new/[g]   :%s/re/new/[g]   regex; $1 captures, \n newline
              :segment  :wq  :count
              :scheme <tag>    lingming xingchen qingyun riyue pinyin
              :wrap  :nowrap   soft-wrap long paragraphs (on by default)
              :wq [path]       save (optionally save-as) and quit
              :42  :goto n    put the cursor on a line
              :recover[!]      load (or drop) a crash-recovery draft
              :bn  :bp  :bd  :ls   the open files (also gn / gp)
              :grep <re>  :toc      across the project / this file's headings
              :export html|typst    write it out for a typesetter
              :layout [horizontal|vertical]  :vertical  :horizontal
              :chaifen  toggle the 拆分 annotation beside candidates
              :ruby       edit the reading at the cursor, or annotate the
                          selection — opens Ruby mode in the status line
              :ruby-on / :ruby-off   lay readings out, or show the markup)

Ruby mode (`:ruby`) edits the *reading*, which with readings laid out is not on
screen to move the cursor into. It opens on the group under the cursor with its
current reading loaded, or on the selection with an empty one; Enter writes it,
an empty reading takes the annotation off, Esc leaves it alone. A reading split
by `|` into as many parts as the base has characters annotates each character
separately — `hàn|zì` over 漢字 — while `hàn zì` stays one reading over the word.

Laid out vertically, text runs top to bottom in 縱 that stack from the right
edge leftward, wrapping every 32 characters (`zong_length`). h j k l keep their
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
            for line in
                yumete_core::zong::render_page(buf.rope(), editor.grid(), config.editor.zong_gap)
            {
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
