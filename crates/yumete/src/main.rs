//! `yumete` — the binary entry point.
//!
//! yumete parses command-line arguments, opens the given file(s) into the
//! editor (or starts a new scratch buffer when none are given), and launches the
//! interactive terminal editor. When standard output is not a terminal, or with
//! `--preview`, it instead prints a non-interactive preview of the active
//! buffer (useful for piping and for quick inspection).

use std::io::IsTerminal;
use std::process::ExitCode;

use yumete_core::{Editor, TextStore};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let mut files: Vec<String> = Vec::new();
    let mut force_preview = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "-v" | "--version" => {
                println!("yumete {VERSION}");
                return ExitCode::SUCCESS;
            }
            "-p" | "--preview" => force_preview = true,
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

    if force_preview || !std::io::stdout().is_terminal() {
        preview(&editor);
        return ExitCode::SUCCESS;
    }

    match yumete_tui::run(&mut editor, &config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("yumete: {err}");
            ExitCode::FAILURE
        }
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
            With no FILE, yumete starts with a new, empty scratch buffer.

OPTIONS:
    -p, --preview    Print a non-interactive preview instead of the editor.
    -h, --help       Print this help and exit.
    -v, --version    Print the version and exit.

KEYS (Normal mode, Helix-style):
    h j k l   move by grapheme / line (CJK-width aware)
    w b e     next / prev word start, word end (W B E for WORDs)
    gg  ge    goto buffer start / last line
    gh gl gs  goto line start / end / first non-blank
    f t F T   find / till a character (forward / backward)
    x         select the current line (repeat to extend)
    v  ;      select (extend) mode / collapse the selection
    d  c      delete / change the selection
    y  p  P   yank / paste after / before
    i  a      insert before / after the selection
    I  A      insert at line start / end
    o  O      open a line below / above
    u  U      undo / redo
    / ? n N   search forward / backward; next / previous match
    :         command line (:w  :w <path>  :q  :q!  :o <path>  :new
              :s/old/new/[g]  :%s/old/new/[g])

In Insert mode, type to insert; Esc returns to Normal."
    );
}

/// Print a status line and the contents of the active buffer, with line numbers.
fn preview(editor: &Editor) {
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
