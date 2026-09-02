//! Parsing of Ex-style command-line commands (the `:` commands).
//!
//! For Feature #1 only the commands needed to open a file or start a new buffer
//! are recognised. The parser is intentionally small and data-light so that
//! later features (`:w`, `:q`, `:s///`, …) can extend the [`Command`] enum and
//! the `match` in [`parse`] without disturbing existing call sites.

use std::fmt;

use crate::ruby::Dialect;
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
    Quit {
        force: bool,
    },
    /// `:wq [path]` / `:x` — save (optionally to a new path), then leave.
    WriteQuit(Option<String>),
    /// `:count` (alias `:wc`) — how much has been written.
    Count,
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
    /// `:hanging` — 句讀 in the margin rather than a square each (Feature #70).
    ToggleHanging,
    /// `:wrap` / `:nowrap` — whether a paragraph too wide for the terminal
    /// continues on the next screen row (Feature #77).
    SetSoftWrap(bool),
    /// `:buffer-next` / `:buffer-previous` (aliases `:bn` / `:bp`) — show
    /// another of the open buffers.
    NextBuffer,
    PreviousBuffer,
    /// `:ruby` — open Ruby mode on the group or selection at the cursor
    /// (Feature #65).
    Ruby,
    /// `:ruby-on` / `:ruby-off`, and `:render-ruby-<dialect>[-off]` — which
    /// ruby dialects are laid out as readings. A `None` dialect means "the one
    /// this file is written in" for `on`, and "all of them" for `off`.
    RenderRuby {
        dialect: Option<Dialect>,
        on: bool,
    },
    /// `:format-ruby-<dialect>` — rewrite every reading in the buffer into one
    /// dialect, whatever it was written in.
    FormatRuby(Dialect),
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
        "wq" | "x" => Ok(Command::WriteQuit(if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        })),
        "count" | "wc" => Ok(Command::Count),
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
        "hanging" => Ok(Command::ToggleHanging),
        "wrap" => Ok(Command::SetSoftWrap(true)),
        "nowrap" => Ok(Command::SetSoftWrap(false)),
        "buffer-next" | "bn" => Ok(Command::NextBuffer),
        "buffer-previous" | "bp" => Ok(Command::PreviousBuffer),
        "ruby" => Ok(Command::Ruby),
        "ruby-on" => Ok(Command::RenderRuby {
            dialect: None,
            on: true,
        }),
        "ruby-off" => Ok(Command::RenderRuby {
            dialect: None,
            on: false,
        }),
        // `:render-ruby-html`, `:render-ruby-typst-off`, `:format-ruby-typst`.
        other if other.starts_with("render-ruby-") || other.starts_with("format-ruby-") => {
            let (verb, rest) = other.split_at("render-ruby-".len());
            let (name, on) = match rest.strip_suffix("-off") {
                Some(name) => (name, false),
                None => (rest, true),
            };
            let dialect =
                Dialect::parse_name(name).ok_or_else(|| CommandError::InvalidArgument {
                    command: "ruby",
                    value: name.to_string(),
                })?;
            Ok(if verb.starts_with("format") {
                Command::FormatRuby(dialect)
            } else {
                Command::RenderRuby {
                    dialect: Some(dialect),
                    on,
                }
            })
        }
        "vertical" => Ok(Command::SetLayout(Some(Layout::Vertical))),
        "horizontal" => Ok(Command::SetLayout(Some(Layout::Horizontal))),
        other => Err(CommandError::Unknown(other.to_string())),
    }
}

/// One entry of the command list: what to type, and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The command word, as typed after the `:`.
    pub name: &'static str,
    /// Its shorter form, if it has one.
    pub alias: Option<&'static str>,
    /// One line saying what it does — short enough to sit beside the name.
    pub help: &'static str,
}

/// Every command, for the completion list.
///
/// This is a second copy of the names in [`parse`], and deliberately so: the
/// parser is a `match` on string literals, which cannot be enumerated. Keeping
/// the list here rather than deriving it means adding a command is two edits —
/// the price of the table being the one place that says what each one is *for*,
/// which is what the user is reading when they cannot remember the name.
pub const COMMANDS: &[Entry] = &[
    Entry {
        name: "open",
        alias: Some("o"),
        help: "open a file",
    },
    Entry {
        name: "new",
        alias: None,
        help: "start an empty buffer",
    },
    Entry {
        name: "write",
        alias: Some("w"),
        help: "save, optionally to a new path",
    },
    Entry {
        name: "wq",
        alias: Some("x"),
        help: "save, then leave",
    },
    Entry {
        name: "count",
        alias: Some("wc"),
        help: "how much has been written",
    },
    Entry {
        name: "quit",
        alias: Some("q"),
        help: "leave; ! discards changes",
    },
    Entry {
        name: "undo",
        alias: Some("u"),
        help: "undo the last change",
    },
    Entry {
        name: "redo",
        alias: Some("red"),
        help: "redo it",
    },
    Entry {
        name: "segment",
        alias: Some("seg"),
        help: "word-segmentation tint",
    },
    Entry {
        name: "layout",
        alias: Some("lay"),
        help: "flip horizontal / vertical",
    },
    Entry {
        name: "vertical",
        alias: None,
        help: "lay the text out in 縱",
    },
    Entry {
        name: "horizontal",
        alias: None,
        help: "lay the text out in lines",
    },
    Entry {
        name: "chaifen",
        alias: Some("cf"),
        help: "拆分 beside candidates",
    },
    Entry {
        name: "hanging",
        alias: None,
        help: "句讀 in the margin (標點旁置)",
    },
    Entry {
        name: "wrap",
        alias: None,
        help: "wrap long paragraphs to the next row",
    },
    Entry {
        name: "nowrap",
        alias: None,
        help: "let long paragraphs run off the edge",
    },
    Entry {
        name: "buffer-next",
        alias: Some("bn"),
        help: "show the next open file (gn)",
    },
    Entry {
        name: "buffer-previous",
        alias: Some("bp"),
        help: "show the previous one (gp)",
    },
    Entry {
        name: "ruby",
        alias: None,
        help: "edit the reading at the cursor",
    },
    Entry {
        name: "ruby-on",
        alias: None,
        help: "lay readings out",
    },
    Entry {
        name: "ruby-off",
        alias: None,
        help: "show the ruby markup",
    },
    Entry {
        name: "render-ruby-html",
        alias: None,
        help: "read <ruby> markup",
    },
    Entry {
        name: "render-ruby-typst",
        alias: None,
        help: "read #ruby() markup",
    },
    Entry {
        name: "format-ruby-html",
        alias: None,
        help: "rewrite readings as HTML",
    },
    Entry {
        name: "format-ruby-typst",
        alias: None,
        help: "rewrite readings as Typst",
    },
    Entry {
        name: "s/pat/rep/",
        alias: None,
        help: "substitute on this line (%s: all)",
    },
];

/// The commands whose name or alias starts with what has been typed.
///
/// An empty prefix lists everything, which is what makes `:` on its own a menu
/// rather than a guess. A prefix that is already a whole command still lists it,
/// so the help stays visible while the arguments are typed.
pub fn complete(prefix: &str) -> Vec<&'static Entry> {
    let prefix = prefix.trim_start_matches(':');
    // Only the command word matters; once there is a space the user has moved
    // on to arguments and the list should stop narrowing.
    let word = prefix.split_whitespace().next().unwrap_or("");
    COMMANDS
        .iter()
        .filter(|e| e.name.starts_with(word) || e.alias.is_some_and(|a| a.starts_with(word)))
        .collect()
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
    fn completion_narrows_as_the_command_is_typed() {
        assert_eq!(
            complete("").len(),
            COMMANDS.len(),
            "`:` alone lists them all"
        );
        let ruby: Vec<&str> = complete("ruby").iter().map(|e| e.name).collect();
        assert_eq!(ruby, ["ruby", "ruby-on", "ruby-off"]);
        // Aliases match too, so `:cf` finds the command it is short for.
        assert_eq!(
            complete("cf").iter().map(|e| e.name).collect::<Vec<_>>(),
            ["chaifen"]
        );
        // Once arguments start, the list stops narrowing.
        assert_eq!(complete("write draft.md").len(), 1);
        assert!(complete("zzz").is_empty());
    }

    /// The other direction of `every_listed_command_parses`: a command that
    /// works but is not in the table is invisible, and the menu is the only way
    /// most of these are ever found. The table cannot be derived from `parse`,
    /// which is a `match` on literals — so this is the thing that notices when
    /// the two drift apart.
    #[test]
    fn every_command_worth_finding_is_listed() {
        for name in [
            "open",
            "new",
            "write",
            "quit",
            "undo",
            "redo",
            "segment",
            "layout",
            "vertical",
            "horizontal",
            "wq",
            "count",
            "chaifen",
            "hanging",
            "wrap",
            "nowrap",
            "buffer-next",
            "buffer-previous",
            "ruby",
            "ruby-on",
            "ruby-off",
            "render-ruby-html",
            "render-ruby-typst",
            "format-ruby-html",
            "format-ruby-typst",
        ] {
            assert!(
                COMMANDS.iter().any(|e| e.name == name),
                "`:{name}` works but is not in the command list, so nothing shows it"
            );
        }
    }

    /// Every name in the list must actually parse, or the menu would offer
    /// commands that do not exist.
    #[test]
    fn every_listed_command_parses() {
        for entry in COMMANDS {
            let line = match entry.name {
                // These two need an argument to be well-formed.
                "open" => ":open a.md".to_string(),
                "s/pat/rep/" => ":s/a/b/".to_string(),
                name => format!(":{name}"),
            };
            assert!(parse(&line).is_ok(), "{} does not parse", entry.name);
            if let Some(alias) = entry.alias {
                let line = if alias == "o" {
                    ":o a.md".to_string()
                } else {
                    format!(":{alias}")
                };
                assert!(parse(&line).is_ok(), "alias {alias} does not parse");
            }
        }
    }

    #[test]
    fn reports_errors() {
        assert_eq!(parse(":"), Err(CommandError::Empty));
        assert_eq!(parse(":open"), Err(CommandError::MissingArgument("open")));
        assert_eq!(parse(":bogus"), Err(CommandError::Unknown("bogus".into())));
    }
}
