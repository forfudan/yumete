//! `yumete` — the binary entry point.
//!
//! yumete parses command-line arguments, opens the given file(s) into the
//! editor (or starts a new scratch buffer when none are given), and launches the
//! interactive terminal editor. When standard output is not a terminal, or with
//! `--preview`, it instead prints a non-interactive preview of the active
//! buffer (useful for piping and for quick inspection).

use std::io::IsTerminal;
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
    let config = yumete_config::Config::load();
    editor.set_key_aliases(config.keys.normal.clone());
    // Layout (Feature #61): the config sets it, a flag overrides for one run,
    // and `:layout` switches it live.
    editor.set_layout(force_layout.unwrap_or(config.editor.layout));
    editor.set_zong_length(config.editor.zong_length);
    editor.set_indent_width(config.editor.tab_width);
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

    // The built-in Yume IME (Feature #27): load the default scheme's tables from
    // the data directory. When the data is absent the session is unavailable and
    // Insert mode simply types plain ASCII.
    let mut ime = ImeSession::from_default_dirs(Scheme::Lingming);
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

    if force_preview || !std::io::stdout().is_terminal() {
        preview(&editor, &config);
        return ExitCode::SUCCESS;
    }

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
    gg  ge    goto buffer start / last line
    gh gl gs  goto line start / end / first non-blank
    f t F T   find / till a character (forward / backward); A-. repeats it
    x  X      select the current line / extend to whole lines
    v  ;  %   select (extend) mode / collapse / select the whole file
    d  c  R   delete / change / replace the selection with the register
    y  p  P   yank / paste after / before
    i  a      insert before / after the selection
    I  A      insert at line start / end
    o  O      open a line below / above
    u  U  .   undo / redo / repeat the last insert
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
              :s/old/new/[g]  :%s/old/new/[g]  :segment
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

    // Status line.
    println!(
        "── {name}{modified} — {lines} line(s), {chars} char(s){extra} ──",
        name = buf.display_name(),
        lines = buf.line_count(),
        chars = buf.char_count(),
    );

    if buf.char_count() == 0 {
        println!("(empty buffer)");
        return;
    }

    // Vertical layout (Feature #61): print the page itself. Line numbers would
    // mean nothing here — the reading order is what there is to look at.
    if editor.layout() == Layout::Vertical {
        for line in
            yumete_core::zong::render_page(buf.rope(), editor.grid(), config.editor.zong_gap)
        {
            println!("{line}");
        }
        return;
    }

    // ropey counts a trailing "\n" as starting an extra empty line; don't print
    // that phantom final line in the preview.
    let text = buf.text();
    let mut count = buf.line_count();
    if text.ends_with('\n') {
        count -= 1;
    }

    let width = count.to_string().len().max(1);
    for i in 0..count {
        if let Some(line) = buf.line(i) {
            println!("{:>width$} │ {line}", i + 1, width = width);
        }
    }
}
