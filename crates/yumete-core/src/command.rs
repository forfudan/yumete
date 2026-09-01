//! Parsing of Ex-style command-line commands (the `:` commands).
//!
//! For Feature #1 only the commands needed to open a file or start a new buffer
//! are recognised. The parser is intentionally small and data-light so that
//! later features (`:w`, `:q`, `:s///`, …) can extend the [`Command`] enum and
//! the `match` in [`parse`] without disturbing existing call sites.

use std::fmt;

use crate::zong::Layout;

/// A parsed command-line command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `:open <path>` (aliases `:o`, `:edit`, `:e`) — open a file into a buffer.
    Open(String),
    /// `:new` (alias `:enew`) — create a new, empty scratch buffer.
    NewBuffer,
    /// `:write [path]` (alias `:w`) — save the current buffer, optionally to a
    /// new path (save-as). `None` saves to the buffer's bound file.
    Write(Option<String>),
    /// `:quit` (alias `:q`) or `:quit!` / `:q!` — leave the editor. `force`
    /// skips the unsaved-changes check.
    Quit { force: bool },
    /// `:s/pattern/replacement/[g]` (optionally `:%s/...` for the whole file) —
    /// substitute text. `global` replaces every match on a line; `whole_file`
    /// applies to every line rather than just the cursor's line.
    Substitute {
        pattern: String,
        replacement: String,
        global: bool,
        whole_file: bool,
    },
    /// `:undo` (alias `:u`) — undo the last change.
    Undo,
    /// `:redo` (alias `:red`) — redo the last undone change.
    Redo,
    /// `:segment` (alias `:seg`) — toggle the word-segmentation overlay
    /// (Feature #24).
    ToggleSegmentation,
    /// `:layout [horizontal|vertical]` (aliases `:horizontal`, `:vertical`) —
    /// choose the layout (Feature #61). `None` toggles between the two.
    SetLayout(Option<Layout>),
    /// `:chaifen` (alias `:cf`) — toggle the 拆分 annotation beside candidates
    /// (Feature #66).
    ToggleChaifen,
}

/// An error produced while parsing a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// The command line was empty (nothing after the `:`).
    Empty,
    /// The command word was not recognised.
    Unknown(String),
    /// The command requires an argument that was not supplied.
    MissingArgument(&'static str),
    /// The command's argument was not one of the values it accepts.
    InvalidArgument {
        command: &'static str,
        value: String,
    },
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::Empty => write!(f, "empty command"),
            CommandError::Unknown(word) => write!(f, "unknown command: {word}"),
            CommandError::MissingArgument(what) => {
                write!(f, "{what} requires an argument")
            }
            CommandError::InvalidArgument { command, value } => {
                write!(f, "{command}: unknown value '{value}'")
            }
        }
    }
}

impl std::error::Error for CommandError {}

/// Parse a single command line, with or without a leading `:`.
///
/// # Examples
///
/// ```
/// use yumete_core::command::{parse, Command};
/// assert_eq!(parse(":open notes.md"), Ok(Command::Open("notes.md".into())));
/// assert_eq!(parse("e 日記.txt"), Ok(Command::Open("日記.txt".into())));
/// assert_eq!(parse(":new"), Ok(Command::NewBuffer));
/// ```
pub fn parse(input: &str) -> Result<Command, CommandError> {
    let trimmed = input.trim().trim_start_matches(':').trim_start();
    if trimmed.is_empty() {
        return Err(CommandError::Empty);
    }

    // Substitution (`s/.../.../` or `%s/.../.../`) is recognised before the
    // whitespace split, since its argument contains no spaces to split on.
    if let Some(cmd) = parse_substitution(trimmed) {
        return cmd;
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let word = parts.next().unwrap();
    let rest = parts.next().unwrap_or("").trim();

    match word {
        "open" | "o" | "edit" | "e" => {
            if rest.is_empty() {
                Err(CommandError::MissingArgument("open"))
            } else {
                Ok(Command::Open(rest.to_string()))
            }
        }
        "new" | "enew" => Ok(Command::NewBuffer),
        "write" | "w" => Ok(Command::Write(if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        })),
        "quit" | "q" => Ok(Command::Quit { force: false }),
        "quit!" | "q!" => Ok(Command::Quit { force: true }),
        "undo" | "u" => Ok(Command::Undo),
        "redo" | "red" => Ok(Command::Redo),
        "segment" | "seg" => Ok(Command::ToggleSegmentation),
        // Layout (Feature #61): `:layout` alone flips it, the two long forms
        // name the layout outright.
        "layout" | "lay" => {
            if rest.is_empty() {
                Ok(Command::SetLayout(None))
            } else {
                Layout::parse(rest)
                    .map(|l| Command::SetLayout(Some(l)))
                    .ok_or_else(|| CommandError::InvalidArgument {
                        command: "layout",
                        value: rest.to_string(),
                    })
            }
        }
        "chaifen" | "cf" => Ok(Command::ToggleChaifen),
        "vertical" => Ok(Command::SetLayout(Some(Layout::Vertical))),
        "horizontal" => Ok(Command::SetLayout(Some(Layout::Horizontal))),
        other => Err(CommandError::Unknown(other.to_string())),
    }
}

/// Try to parse a substitution command (`s/pat/rep/flags`, `%s/pat/rep/flags`).
///
/// Returns `None` when the input is not a substitution, or `Some(Err(..))` when
/// it looks like one but is malformed. Only `/` is supported as the delimiter.
fn parse_substitution(input: &str) -> Option<Result<Command, CommandError>> {
    let (whole_file, rest) = match input.strip_prefix('%') {
        Some(r) => (true, r),
        None => (false, input),
    };
    let rest = rest.strip_prefix('s')?;
    // The character right after `s` must be the `/` delimiter.
    let body = rest.strip_prefix('/')?;

    let fields: Vec<&str> = body.split('/').collect();
    // Expect at least "pattern/replacement" (flags optional): 2 or 3 fields.
    if fields.len() < 2 || fields.len() > 3 {
        return Some(Err(CommandError::MissingArgument("substitute")));
    }
    let pattern = fields[0];
    if pattern.is_empty() {
        return Some(Err(CommandError::MissingArgument("substitute")));
    }
    let replacement = fields[1];
    let flags = fields.get(2).copied().unwrap_or("");

    Some(Ok(Command::Substitute {
        pattern: pattern.to_string(),
        replacement: replacement.to_string(),
        global: flags.contains('g'),
        whole_file,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_open_and_aliases() {
        for cmd in [":open a.md", "open a.md", ":o a.md", ":edit a.md", "e a.md"] {
            assert_eq!(parse(cmd), Ok(Command::Open("a.md".into())), "{cmd}");
        }
    }

    #[test]
    fn keeps_paths_with_spaces() {
        assert_eq!(
            parse(":open my novel.md"),
            Ok(Command::Open("my novel.md".into()))
        );
    }

    #[test]
    fn parses_new_buffer() {
        assert_eq!(parse(":new"), Ok(Command::NewBuffer));
        assert_eq!(parse("enew"), Ok(Command::NewBuffer));
    }

    #[test]
    fn parses_write_with_optional_path() {
        assert_eq!(parse(":w"), Ok(Command::Write(None)));
        assert_eq!(parse(":write"), Ok(Command::Write(None)));
        assert_eq!(
            parse(":w draft.md"),
            Ok(Command::Write(Some("draft.md".into())))
        );
    }

    #[test]
    fn parses_quit_and_force_quit() {
        assert_eq!(parse(":q"), Ok(Command::Quit { force: false }));
        assert_eq!(parse("quit"), Ok(Command::Quit { force: false }));
        assert_eq!(parse(":q!"), Ok(Command::Quit { force: true }));
        assert_eq!(parse(":quit!"), Ok(Command::Quit { force: true }));
    }

    #[test]
    fn parses_undo_and_redo() {
        assert_eq!(parse(":undo"), Ok(Command::Undo));
        assert_eq!(parse(":u"), Ok(Command::Undo));
        assert_eq!(parse(":redo"), Ok(Command::Redo));
    }

    #[test]
    fn parses_segment_toggle() {
        assert_eq!(parse(":segment"), Ok(Command::ToggleSegmentation));
        assert_eq!(parse(":seg"), Ok(Command::ToggleSegmentation));
    }

    #[test]
    fn parses_layout_commands() {
        assert_eq!(parse(":layout"), Ok(Command::SetLayout(None)));
        assert_eq!(
            parse(":layout vertical"),
            Ok(Command::SetLayout(Some(Layout::Vertical)))
        );
        assert_eq!(
            parse(":layout h"),
            Ok(Command::SetLayout(Some(Layout::Horizontal)))
        );
        assert_eq!(
            parse(":vertical"),
            Ok(Command::SetLayout(Some(Layout::Vertical)))
        );
        assert_eq!(
            parse(":horizontal"),
            Ok(Command::SetLayout(Some(Layout::Horizontal)))
        );
        assert_eq!(
            parse(":layout sideways"),
            Err(CommandError::InvalidArgument {
                command: "layout",
                value: "sideways".into()
            })
        );
    }

    #[test]
    fn parses_chaifen_toggle() {
        assert_eq!(parse(":chaifen"), Ok(Command::ToggleChaifen));
        assert_eq!(parse(":cf"), Ok(Command::ToggleChaifen));
    }

    #[test]
    fn parses_substitution() {
        assert_eq!(
            parse(":s/foo/bar/"),
            Ok(Command::Substitute {
                pattern: "foo".into(),
                replacement: "bar".into(),
                global: false,
                whole_file: false,
            })
        );
        assert_eq!(
            parse(":%s/foo/bar/g"),
            Ok(Command::Substitute {
                pattern: "foo".into(),
                replacement: "bar".into(),
                global: true,
                whole_file: true,
            })
        );
        // Empty replacement (a deletion) is allowed.
        assert_eq!(
            parse(":s/foo//"),
            Ok(Command::Substitute {
                pattern: "foo".into(),
                replacement: "".into(),
                global: false,
                whole_file: false,
            })
        );
        // Empty pattern is rejected.
        assert!(parse(":s//bar/").is_err());
    }

    #[test]
    fn reports_errors() {
        assert_eq!(parse(":"), Err(CommandError::Empty));
        assert_eq!(parse(":open"), Err(CommandError::MissingArgument("open")));
        assert_eq!(parse(":bogus"), Err(CommandError::Unknown("bogus".into())));
    }
}
