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
    /// `:<n>` or `:goto <n>` (alias `:g`) — put the cursor on line `n`.
    GotoLine(usize),
    /// `:recover` — load the crash-recovery draft into the buffer;
    /// `:recover!` — throw it away instead (Feature #79).
    Recover {
        discard: bool,
    },
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
    /// `:scheme <tag>` — switch the input scheme (Feature #86).
    SetScheme(String),
    /// `:hanging` — 句讀 in the margin rather than a square each (Feature #70).
    ToggleHanging,
    /// `:wrap` / `:nowrap` — whether a paragraph too wide for the terminal
    /// continues on the next screen row (Feature #77).
    SetSoftWrap(bool),
    /// `:wrap <n>` — write to a measure of `n` columns rather than to the
    /// window; `:wrap 0` gives the window back (Feature #113).
    SetMeasure(Option<usize>),
    /// `:table` / `:table off` — read the file as a grid (Feature #118).
    SetTable(bool),
    /// `:dense` / `:dense off` — pack the 縱書 page as tight as a terminal can
    /// (Feature #120).
    SetDense(bool),
    /// `:render off|on|full` — how much of the result the page shows
    /// (Features #96 / #104).
    SetRender(crate::editor::Render),
    /// `:preview` / `:preview off` — hand the file to the real typesetter and
    /// show what it makes (Feature #128).
    SetPreview(bool),
    /// `:w!` — write over a file that changed on disk since it was read.
    WriteForce(Option<String>),
    /// `:e!` — read the file again, losing what is in the buffer.
    Reread,
    /// `:yume builtin` — use the 碼表 in the binary, whatever is installed.
    BuiltinScheme,
    /// `:yume table <path>` — type with a code table of your own.
    UserTable(String),
    /// `:yume` on its own — say what the input method is doing.
    YumeStatus,
    /// `:sh <cmd>` — run it and bring the output back into a buffer, or
    /// `:!<cmd>` — step out of the way and let it use the terminal
    /// (Feature #129).
    Shell { line: String, interactive: bool },
    /// `:pipe <cmd>` (or `!`) — send the selection to a command and put what
    /// it says back in its place.
    Pipe(String),
    /// `:clipboard-yank` / `:clipboard-paste` — the system clipboard, which
    /// Helix spells the same way (Feature #109).
    Clipboard {
        yank: bool,
    },
    /// `:syntax [markdown|typst]` — which markup this file is in
    /// (Feature #106). No argument says what it was guessed to be.
    SetSyntax(Option<String>),
    /// `:buffer-next` / `:buffer-previous` (aliases `:bn` / `:bp`) — show
    /// another of the open buffers.
    NextBuffer,
    PreviousBuffer,
    /// `:buffer-close` (alias `:bd`), `:bd!` — close the active buffer.
    CloseBuffer {
        force: bool,
    },
    /// `:buffers` (alias `:ls`) — name every open buffer.
    ListBuffers,
    /// `:toc [n]` — list the headings, or go to the nth.
    Outline(Option<usize>),
    /// `:grep <pattern>` — search every file in the project.
    Grep(String),
    /// `:export html|typst [path]` — write the manuscript out for a typesetter.
    Export {
        format: String,
        path: Option<String>,
    },
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

    // `:42` is a line number, as it is in vi and in Helix. Recognised before
    // the name split so no command can ever be named a number.
    if let Ok(n) = trimmed.parse::<usize>() {
        return Ok(Command::GotoLine(n));
    }

    // `:!make` is vi's, and it takes the whole rest of the line — including
    // spaces, and including a `!` of its own — so it is read before the name
    // split, like a substitution.
    if let Some(line) = trimmed.strip_prefix('!') {
        return if line.trim().is_empty() {
            Err(CommandError::MissingArgument("!"))
        } else {
            Ok(Command::Shell {
                line: line.to_string(),
                interactive: true,
            })
        };
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let word = resolve(parts.next().unwrap());
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
        // `!` is "I know, and mine wins" — over a file that changed on disk.
        "write!" | "w!" => Ok(Command::WriteForce(if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        })),
        // `:e!` is the other half: take what is on disk and lose what is here.
        "open!" | "o!" | "edit!" | "e!" => Ok(Command::Reread),
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
        "goto" | "g" => rest
            .parse::<usize>()
            .map(Command::GotoLine)
            .map_err(|_| CommandError::MissingArgument("goto")),
        "recover" => Ok(Command::Recover { discard: false }),
        "recover!" => Ok(Command::Recover { discard: true }),
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
        // Everything about the input method under one word. `:scheme` and
        // `:chaifen` still work — a spelling somebody has learned is not worth
        // taking away — but there is now one word to remember instead of two,
        // and it lists what it takes.
        "yume" => {
            let mut parts = rest.split_whitespace();
            // On its own it is the question, not a mistake: *which* 靈明 is
            // answering, and where did it come from. A parent command with
            // nothing after it should say where you are.
            let Some(word) = parts.next() else {
                return Ok(Command::YumeStatus);
            };
            match pick(word, YUME).map(|w| w.name) {
                Some("builtin") => Ok(Command::BuiltinScheme),
                Some("table") => match parts.next() {
                    Some(path) => Ok(Command::UserTable(path.to_string())),
                    None => Err(CommandError::MissingArgument("table")),
                },
                // No name is "the one the config asked for" — `:yume s` is the
                // whole of starting to type.
                Some("scheme") => Ok(Command::SetScheme(
                    parts
                        .next()
                        .and_then(|tag| pick(tag, SCHEMES).map(|w| w.name))
                        .unwrap_or("")
                        .to_string(),
                )),
                Some("chaifen") => Ok(Command::ToggleChaifen),
                _ => Err(CommandError::InvalidArgument {
                    command: "yume",
                    value: word.to_string(),
                }),
            }
        }

        "hanging" => Ok(Command::ToggleHanging),
        "clipboard" => match rest {
            "yank" => Ok(Command::Clipboard { yank: true }),
            "paste" => Ok(Command::Clipboard { yank: false }),
            "" => Err(CommandError::MissingArgument("clipboard")),
            other => Err(CommandError::InvalidArgument {
                command: "clipboard",
                value: other.to_string(),
            }),
        },
        "syntax" | "syn" => Ok(Command::SetSyntax(if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        })),
        "pipe" => {
            if rest.is_empty() {
                Err(CommandError::MissingArgument("pipe"))
            } else {
                Ok(Command::Pipe(rest.to_string()))
            }
        }
        "sh" => {
            if rest.is_empty() {
                Err(CommandError::MissingArgument("sh"))
            } else {
                Ok(Command::Shell {
                    line: rest.to_string(),
                    interactive: false,
                })
            }
        }
        "preview" => match rest {
            "" | "on" => Ok(Command::SetPreview(true)),
            "off" => Ok(Command::SetPreview(false)),
            other => Err(CommandError::InvalidArgument {
                command: "preview",
                value: other.to_string(),
            }),
        },
        "render" => match rest {
            "" | "on" => Ok(Command::SetRender(crate::editor::Render::On)),
            "off" => Ok(Command::SetRender(crate::editor::Render::Off)),
            "full" => Ok(Command::SetRender(crate::editor::Render::Full)),
            other => Err(CommandError::InvalidArgument {
                command: "render",
                value: other.to_string(),
            }),
        },
        // `:wrap` on its own still means what it always meant — turn wrapping
        // on — and leaves the measure alone; a number sets the measure.
        "dense" => match rest {
            "" | "on" => Ok(Command::SetDense(true)),
            "off" => Ok(Command::SetDense(false)),
            other => Err(CommandError::InvalidArgument {
                command: "dense",
                value: other.to_string(),
            }),
        },
        "table" => match rest {
            "" | "on" => Ok(Command::SetTable(true)),
            "off" => Ok(Command::SetTable(false)),
            other => Err(CommandError::InvalidArgument {
                command: "table",
                value: other.to_string(),
            }),
        },
        // On its own it turns wrapping on and leaves the measure alone; a
        // number sets the measure; `0` gives the window back; `off` stops
        // wrapping altogether.
        "wrap" => match rest {
            "" | "on" => Ok(Command::SetSoftWrap(true)),
            "off" => Ok(Command::SetSoftWrap(false)),
            "0" => Ok(Command::SetMeasure(None)),
            n => match n.parse::<usize>() {
                Ok(n) => Ok(Command::SetMeasure(Some(n))),
                Err(_) => Err(CommandError::InvalidArgument {
                    command: "wrap",
                    value: n.to_string(),
                }),
            },
        },
        "buffer" => match rest {
            "next" => Ok(Command::NextBuffer),
            "previous" => Ok(Command::PreviousBuffer),
            "close" => Ok(Command::CloseBuffer { force: false }),
            "close!" => Ok(Command::CloseBuffer { force: true }),
            "list" | "" => Ok(Command::ListBuffers),
            other => Err(CommandError::InvalidArgument {
                command: "buffer",
                value: other.to_string(),
            }),
        },
        "export" | "ex" => {
            let mut parts = rest.splitn(2, char::is_whitespace);
            let format = parts.next().unwrap_or("").trim();
            if format.is_empty() {
                return Err(CommandError::MissingArgument("export"));
            }
            let path = parts
                .next()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty());
            Ok(Command::Export {
                format: format.to_string(),
                path,
            })
        }
        "grep" | "gr" => {
            if rest.is_empty() {
                Err(CommandError::MissingArgument("grep"))
            } else {
                Ok(Command::Grep(rest.to_string()))
            }
        }
        "toc" | "outline" => Ok(Command::Outline(if rest.is_empty() {
            None
        } else {
            match rest.parse::<usize>() {
                Ok(n) => Some(n),
                Err(_) => return Err(CommandError::MissingArgument("toc")),
            }
        })),
        // `:ruby` on its own is the verb — edit the reading here. With a word
        // after it, it is the setting: whether readings are laid out at all,
        // and which spellings of them count.
        "ruby" if rest.is_empty() => Ok(Command::Ruby),
        "ruby" => {
            let mut parts = rest.split_whitespace();
            let first = parts.next().unwrap_or("");
            let second = parts.next();
            let on = |word: Option<&str>| match word {
                None | Some("on") => Ok(true),
                Some("off") => Ok(false),
                Some(other) => Err(CommandError::InvalidArgument {
                    command: "ruby",
                    value: other.to_string(),
                }),
            };
            match first {
                "on" | "off" => Ok(Command::RenderRuby {
                    dialect: None,
                    on: first == "on",
                }),
                "format" => {
                    let name = second.ok_or(CommandError::MissingArgument("ruby format"))?;
                    let dialect =
                        Dialect::parse_name(name).ok_or_else(|| CommandError::InvalidArgument {
                            command: "ruby",
                            value: name.to_string(),
                        })?;
                    Ok(Command::FormatRuby(dialect))
                }
                name => {
                    let dialect =
                        Dialect::parse_name(name).ok_or_else(|| CommandError::InvalidArgument {
                            command: "ruby",
                            value: name.to_string(),
                        })?;
                    Ok(Command::RenderRuby {
                        dialect: Some(dialect),
                        on: on(second)?,
                    })
                }
            }
        }

        other => Err(CommandError::Unknown(other.to_string())),
    }
}

/// One entry of the command list: what to type, and what it does.
pub struct Entry {
    /// The command word, as typed after the `:`.
    pub name: &'static str,
    /// Every other spelling that names it — the short form first, since that
    /// is the one shown. A list rather than one slot because the parser really
    /// does accept several (`:open` is also `:o`, `:edit`, `:e`), and a table
    /// that could only declare one of them was a table that drifted from the
    /// parser without anybody noticing.
    pub aliases: &'static [&'static str],
    /// One line saying what it does — short enough to sit beside the name.
    pub help: &'static str,
    /// What may follow it.
    pub args: Args,
}

/// What may follow a command, or one of its words.
///
/// **A parent command is not a mechanism of its own.** `:yume scheme` is a
/// command whose argument happens to be a verb, and that is the whole of it:
/// implement completing an *argument* and grouping falls out for nothing, with
/// no second code path to keep honest and no second thing for a reader to
/// learn. It also means a group can be one level deep or three without the
/// completion knowing the difference.
pub enum Args {
    /// Nothing follows.
    None,
    /// One of these words — subcommands are exactly this.
    Words(&'static [Word]),
    /// A path, completed from the file system by the caller.
    Path,
    /// Anything at all; the string is the placeholder to show while typing.
    Free(&'static str),
}

/// One word a command accepts, and what may follow *it*.
pub struct Word {
    pub name: &'static str,
    pub help: &'static str,
    pub then: Args,
}

/// The command a typed word names, by prefix when it is not a whole name.
///
/// **One rule, at every level**: an unambiguous prefix names the thing. It is
/// already how subcommands work — `:yume s l` — and a command line where the
/// rule held for the second word but not the first would be a rule nobody
/// could state.
///
/// Exact names and declared aliases win over it, in that order, so `:w` stays
/// `write` even though `w` begins three commands. A prefix that names two
/// commands names neither: the word passes through unchanged and comes back as
/// "unknown", which is the truth.
fn resolve(word: &str) -> &str {
    if COMMANDS
        .iter()
        .any(|e| e.name == word || e.aliases.contains(&word))
    {
        return word;
    }
    let mut hits = COMMANDS.iter().filter(|e| e.name.starts_with(word));
    match (hits.next(), hits.next()) {
        (Some(only), None) => only.name,
        _ => word,
    }
}

/// The one word of `from` that `typed` names, exactly or by prefix.
///
/// `:yume s l` is `:yume scheme lingming` — because `s` is the only word there
/// starting with `s`, and `l` the only scheme starting with `l`. An ambiguous
/// prefix names nothing rather than guessing: `:ruby t` could be `typst` and
/// nothing else, but if a second `t` word were ever added it would stop
/// working, loudly, instead of quietly meaning the older one.
/// The shortest unambiguous way to write `name`, among `others`.
///
/// What the menu shows in parentheses, and it is *true* — the same prefix rule
/// resolves it when typed, at every level. Worked out rather than declared, so
/// it cannot promise a spelling that a later word made ambiguous.
pub fn shortest(
    name: &'static str,
    among: impl Iterator<Item = &'static str>,
) -> Option<&'static str> {
    let others: Vec<&str> = among.filter(|&o| o != name).collect();
    let mut end = 0;
    for (at, _) in name.char_indices().skip(1) {
        end = at;
        if !others.iter().any(|o| o.starts_with(&name[..at])) {
            return Some(&name[..at]);
        }
    }
    let _ = end;
    None
}

pub fn pick<'a>(typed: &str, from: &'a [Word]) -> Option<&'a Word> {
    if let Some(exact) = from.iter().find(|w| w.name == typed) {
        return Some(exact);
    }
    let mut starting = from.iter().filter(|w| w.name.starts_with(typed));
    match (starting.next(), starting.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// A word offered by completion, whether a command or an argument.
#[derive(Clone, Copy)]
pub struct Choice {
    pub name: &'static str,
    pub alias: Option<&'static str>,
    /// The shortest prefix that names this and nothing else beside it, when
    /// that is shorter than the whole word.
    ///
    /// Worked out rather than declared, so it cannot go stale: adding a second
    /// word starting with `s` lengthens what `scheme` shows, in the same edit
    /// that made it true.
    pub short: Option<&'static str>,
    pub help: &'static str,
    /// What is written before it when it is shown: `:` for a command, nothing
    /// for a word it takes. A colon on `on` would be a lie about how to type
    /// it.
    pub leading: &'static str,
}

/// The two words every switch takes.
/// `:yume` and what may follow it — the input method's own commands, under
/// the one word a reader would think of when looking for them.
const YUME: &[Word] = &[
    Word {
        name: "scheme",
        help: "開始打字：載入一個方案（不寫名字就用配置裏那個）",
        then: Args::Words(SCHEMES),
    },
    Word {
        name: "chaifen",
        help: "候選旁的拆分注解",
        then: Args::Words(ON_OFF),
    },
    Word {
        name: "builtin",
        help: "改用出廠自帶的靈明碼表，不管裝了什麼",
        then: Args::None,
    },
    Word {
        name: "table",
        help: "用你自己的碼表（Rime 的 .dict.yaml 也行）",
        then: Args::Path,
    },
];

/// How much of the result the page shows.
const RENDER: &[Word] = &[
    Word {
        name: "off",
        help: "原文，不著色",
        then: Args::None,
    },
    Word {
        name: "on",
        help: "著色，標記留在畫面上（默認）",
        then: Args::None,
    },
    Word {
        name: "full",
        help: "標記拿掉，只在光標那一處展開",
        then: Args::None,
    },
];

/// What `:clipboard` does — the system one, not a register.
const CLIPBOARD: &[Word] = &[
    Word {
        name: "yank",
        help: "選區送到系統剪貼簿",
        then: Args::None,
    },
    Word {
        name: "paste",
        help: "從系統剪貼簿貼進來",
        then: Args::None,
    },
];

/// What `:buffer` does.
const BUFFERS: &[Word] = &[
    Word {
        name: "list",
        help: "列出開着的檔案",
        then: Args::None,
    },
    Word {
        name: "next",
        help: "下一個",
        then: Args::None,
    },
    Word {
        name: "previous",
        help: "上一個",
        then: Args::None,
    },
    Word {
        name: "close",
        help: "關掉這一個（`close!` 不管改動）",
        then: Args::None,
    },
];

/// What `:wrap` takes.
const WRAP: &[Word] = &[
    Word {
        name: "on",
        help: "太寬的段落折到下一行",
        then: Args::None,
    },
    Word {
        name: "off",
        help: "讓它跑出右邊",
        then: Args::None,
    },
    Word {
        name: "0",
        help: "尺度用窗口寬（`:wrap 50` 是固定五十欄）",
        then: Args::None,
    },
];

/// The input schemes yume ships with.
const SCHEMES: &[Word] = &[
    Word {
        name: "lingming",
        help: "靈明",
        then: Args::None,
    },
    Word {
        name: "xingchen",
        help: "星陳",
        then: Args::None,
    },
    Word {
        name: "qingyun",
        help: "卿雲",
        then: Args::None,
    },
    Word {
        name: "riyue",
        help: "日月",
        then: Args::None,
    },
    Word {
        name: "pinyin",
        help: "拼音",
        then: Args::None,
    },
];

/// The languages a file may be read as.
const SYNTAXES: &[Word] = &[
    Word {
        name: "markdown",
        help: "`#` 標題、`**粗**`",
        then: Args::None,
    },
    Word {
        name: "typst",
        help: "`=` 標題、`#import`",
        then: Args::None,
    },
];

/// Which way the page runs.
const LAYOUTS: &[Word] = &[
    Word {
        name: "vertical",
        help: "竪排，縱從右往左",
        then: Args::None,
    },
    Word {
        name: "horizontal",
        help: "橫排",
        then: Args::None,
    },
];

/// `:ruby` and what may follow it — the four `render-ruby-*` and
/// `format-ruby-*` commands, folded into the one word a reader remembers.
const RUBY: &[Word] = &[
    Word {
        name: "on",
        help: "排出注音",
        then: Args::None,
    },
    Word {
        name: "off",
        help: "顯示源碼",
        then: Args::None,
    },
    Word {
        name: "html",
        help: "認 `<ruby>` 這一種",
        then: Args::Words(ON_OFF),
    },
    Word {
        name: "typst",
        help: "認 `#ruby(…)` 這一種",
        then: Args::Words(ON_OFF),
    },
    Word {
        name: "format",
        help: "把注音改寫成另一種寫法",
        then: Args::Words(&[
            Word {
                name: "html",
                help: "改寫成 `<ruby>`",
                then: Args::None,
            },
            Word {
                name: "typst",
                help: "改寫成 `#ruby(…)`",
                then: Args::None,
            },
        ]),
    },
];

const ON_OFF: &[Word] = &[
    Word {
        name: "on",
        help: "開",
        then: Args::None,
    },
    Word {
        name: "off",
        help: "關",
        then: Args::None,
    },
];

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
        aliases: &["o", "e", "edit"],
        help: "open a file",
        args: Args::Path,
    },
    Entry {
        name: "new",
        aliases: &["enew"],
        help: "start an empty buffer",
        args: Args::Path,
    },
    Entry {
        name: "write",
        aliases: &["w"],
        help: "save, optionally to a new path",
        args: Args::Path,
    },
    Entry {
        name: "wq",
        aliases: &["x"],
        help: "save, then leave",
        args: Args::Path,
    },
    Entry {
        name: "recover",
        aliases: &[],
        help: "load the recovery draft (`!` throws it away)",
        args: Args::None,
    },
    Entry {
        name: "goto",
        aliases: &["g"],
        help: "put the cursor on a line (or just `:42`)",
        args: Args::Free("<行號>"),
    },
    Entry {
        name: "count",
        aliases: &["wc"],
        help: "how much has been written",
        args: Args::None,
    },
    Entry {
        name: "quit",
        aliases: &["q"],
        help: "leave; ! discards changes",
        args: Args::None,
    },
    Entry {
        name: "undo",
        aliases: &["u"],
        help: "undo the last change",
        args: Args::None,
    },
    Entry {
        name: "redo",
        aliases: &["red"],
        help: "redo it",
        args: Args::None,
    },
    Entry {
        name: "segment",
        aliases: &["seg"],
        help: "word-segmentation tint",
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "layout",
        aliases: &["lay"],
        help: "flip horizontal / vertical",
        args: Args::Words(LAYOUTS),
    },
    Entry {
        name: "yume",
        aliases: &[],
        help: "輸入法：現在用的是哪一個；換方案、拆分注解",
        args: Args::Words(YUME),
    },
    Entry {
        name: "hanging",
        aliases: &[],
        help: "句讀 in the margin (標點旁置)",
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "syntax",
        aliases: &["syn"],
        help: "markdown 還是 typst（不給參數就說現在是哪個）",
        args: Args::Words(SYNTAXES),
    },
    Entry {
        name: "pipe",
        aliases: &[],
        help: "把選區送給一條命令，用它的輸出換掉（`!`）",
        args: Args::Free("<命令>"),
    },
    Entry {
        name: "sh",
        aliases: &[],
        help: "跑一條命令，輸出收進一個緩衝區",
        args: Args::Free("<命令>"),
    },
    Entry {
        name: "!command",
        aliases: &[],
        help: "讓出終端跑一條命令，直接看它跑（vi 的寫法）",
        args: Args::None,
    },
    Entry {
        name: "preview",
        aliases: &[],
        help: "交給真正的排版器去排，在瀏覽器裏看",
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "render",
        aliases: &[],
        help: "畫面上顯示多少「結果」：原文、著色、所見即所得",
        args: Args::Words(RENDER),
    },
    Entry {
        name: "dense",
        aliases: &[],
        help: "密排：一縱兩格，無注音、無旁置、無刻度",
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "table",
        aliases: &[],
        help: "按格子編輯：CSV 整個檔案，或游標所在的 | 表格；`:table off` 收工",
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "wrap",
        aliases: &[],
        help: "wrap long paragraphs to the next row; `:wrap 50` sets a measure",
        args: Args::Words(WRAP),
    },
    Entry {
        name: "clipboard",
        aliases: &[],
        help: "系統剪貼簿：送出去、貼進來",
        args: Args::Words(CLIPBOARD),
    },
    Entry {
        name: "buffer",
        aliases: &[],
        help: "開着的檔案：列出、切換、關掉",
        args: Args::Words(BUFFERS),
    },
    Entry {
        name: "export",
        aliases: &["ex"],
        help: "write out as html or typst, with the layout",
        args: Args::Free("<檔名>"),
    },
    Entry {
        name: "grep",
        aliases: &["gr"],
        help: "search every file in the project",
        args: Args::Free("<正則>"),
    },
    Entry {
        name: "toc",
        aliases: &["outline"],
        help: "list the headings, or `:toc 3` to go to one",
        args: Args::Free("<第幾條，不寫就列出來>"),
    },
    Entry {
        name: "ruby",
        aliases: &[],
        help: "改這裏的注音；`:ruby on|off` 是排不排",
        args: Args::Words(RUBY),
    },
    Entry {
        name: "s/pat/rep/",
        aliases: &[],
        help: "substitute on this line (%s: all)",
        args: Args::None,
    },
];

/// The commands whose name or alias starts with what has been typed.
///
/// An empty prefix lists everything, which is what makes `:` on its own a menu
/// rather than a guess. A prefix that is already a whole command still lists it,
/// so the help stays visible while the arguments are typed.
pub fn complete(prefix: &str) -> Vec<Choice> {
    complete_at(prefix).1
}

/// The same, and where in the line the offered word would go.
///
/// The offset is what lets a completion replace **the word being typed** rather
/// than the whole line: `:yume sch` has to become `:yume scheme`, not `scheme`.
pub fn complete_at(line: &str) -> (usize, Vec<Choice>) {
    let line = line.strip_prefix(':').unwrap_or(line);
    // Where each word starts, and the word itself. A line ending in a space is
    // asking about a *new* word, not still about the last one.
    let mut words: Vec<(usize, &str)> = Vec::new();
    let mut at = 0;
    for part in line.split(' ') {
        if !part.is_empty() {
            words.push((at, part));
        }
        at += part.len() + 1;
    }
    let starting_new = line.is_empty() || line.ends_with(' ');
    let (start, typed) = if starting_new {
        (line.len(), "")
    } else {
        words.pop().unwrap_or((0, ""))
    };

    // The first word names a command; every word after it walks down what that
    // command says may follow — which is the same walk whether those words are
    // arguments or subcommands, because they are the same thing.
    let choices: Vec<Choice> = match words.first() {
        None => COMMANDS
            .iter()
            .filter(|e| {
                e.name.starts_with(typed) || e.aliases.iter().any(|a| a.starts_with(typed))
            })
            .map(|e| Choice {
                name: e.name,
                alias: e.aliases.first().copied(),
                short: shortest(e.name, COMMANDS.iter().map(|c| c.name)),
                help: e.help,
                leading: ":",
            })
            .collect(),
        Some(&(_, head)) => {
            let Some(entry) = COMMANDS
                .iter()
                .find(|e| e.name == head || e.aliases.contains(&head))
            else {
                return (start, Vec::new());
            };
            let mut args = &entry.args;
            for &(_, word) in &words[1..] {
                match args {
                    Args::Words(list) => match list.iter().find(|w| w.name == word) {
                        Some(found) => args = &found.then,
                        None => return (start, Vec::new()),
                    },
                    _ => return (start, Vec::new()),
                }
            }
            match args {
                Args::Words(list) => list
                    .iter()
                    .filter(|w| w.name.starts_with(typed))
                    .map(|w| Choice {
                        name: w.name,
                        alias: None,
                        short: shortest(w.name, list.iter().map(|o| o.name)),
                        help: w.help,
                        leading: "",
                    })
                    .collect(),
                // A path or free text is the caller's business; there is
                // nothing here to offer but the placeholder, which is help
                // rather than a completion.
                Args::Free(what) => vec![Choice {
                    name: "",
                    alias: None,
                    short: None,
                    help: what,
                    leading: "",
                }],
                Args::None | Args::Path => Vec::new(),
            }
        }
    };
    (start, choices)
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
        // `:vertical` was the value wearing the setting's name; it is gone.
        assert_eq!(parse(":vertical"), Err(CommandError::Unknown("vertical".into())));
        assert_eq!(
            parse(":layout sideways"),
            Err(CommandError::InvalidArgument {
                command: "layout",
                value: "sideways".into()
            })
        );
    }

    #[test]
    fn the_input_method_lives_under_one_word() {
        assert_eq!(parse(":yume chaifen"), Ok(Command::ToggleChaifen));
        assert_eq!(
            parse(":yume scheme lingming"),
            Ok(Command::SetScheme("lingming".into()))
        );
        // The flat spellings are gone: `:chaifen` said nothing about which of
        // the editor's many parts it belonged to, and `:scheme` even less.
        assert_eq!(parse(":chaifen"), Err(CommandError::Unknown("chaifen".into())));
        assert_eq!(parse(":scheme x"), Err(CommandError::Unknown("scheme".into())));
        // On its own it is the question "which one is answering", not a
        // mistake — a parent command should say where you are.
        assert_eq!(parse(":yume"), Ok(Command::YumeStatus));
        assert_eq!(parse(":yume b"), Ok(Command::BuiltinScheme));
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
        // One name, not three: `:ruby-on` and `:ruby-off` were the setting
        // wearing the verb's name, and they are now words `:ruby` takes.
        let ruby: Vec<&str> = complete("ruby").iter().map(|e| e.name).collect();
        assert_eq!(ruby, ["ruby"]);
        // Aliases match too, so `:w` finds the command it is short for.
        assert_eq!(
            complete("wq").iter().map(|e| e.name).collect::<Vec<_>>(),
            ["wq"]
        );
        assert!(complete("zzz").is_empty());
    }

    #[test]
    fn the_short_form_the_menu_shows_is_one_that_works() {
        // The menu prints `:yume (y)`. That has to be true, or it is teaching
        // a spelling that fails — so the same prefix rule resolves the first
        // word of a command line, not only the words after it.
        assert_eq!(parse(":y"), Ok(Command::YumeStatus));
        assert_eq!(parse(":yu"), Ok(Command::YumeStatus));
        assert_eq!(parse(":ta"), Ok(Command::SetTable(true)));

        // A declared alias beats the prefix rule, so the short spellings
        // people already know keep their meanings: `w` begins `write`, `wq`
        // and `wrap`, and it is still `write`.
        assert_eq!(parse(":w"), Ok(Command::Write(None)));
        assert_eq!(parse(":e a.md"), Ok(Command::Open("a.md".into())));

        // An ambiguous prefix names nothing rather than guessing.
        assert_eq!(parse(":re"), Err(CommandError::Unknown("re".into())));
        assert_eq!(parse(":s"), Err(CommandError::Unknown("s".into())));

        // And what the menu shows is worked out from the table it is showing,
        // so a new command that collides lengthens it in the same edit.
        let short = |name: &'static str| shortest(name, COMMANDS.iter().map(|c| c.name));
        assert_eq!(short("yume"), Some("y"));
        assert_eq!(short("render"), Some("ren"), "recover and redo are in the way");
        assert_eq!(short("sh"), None, "nothing shorter than the whole word");
        for entry in COMMANDS {
            if let Some(short) = short(entry.name) {
                assert!(
                    parse(&format!(":{short}")).is_ok()
                        || matches!(
                            parse(&format!(":{short}")),
                            Err(CommandError::MissingArgument(_))
                        ),
                    "the menu offers `:{short}` for `:{}`, which does not resolve",
                    entry.name
                );
            }
        }
    }

    #[test]
    fn an_unambiguous_prefix_is_the_word_it_starts() {
        // `:yume s l` is the whole of starting to type — `s` is the only word
        // `:yume` takes that starts with `s`, and `l` the only scheme.
        assert_eq!(
            parse(":yume s l"),
            Ok(Command::SetScheme("lingming".into()))
        );
        assert_eq!(parse(":yume scheme ling"), Ok(Command::SetScheme("lingming".into())));
        assert_eq!(parse(":yume c"), Ok(Command::ToggleChaifen));
        // No name at all means the one the config asked for.
        assert_eq!(parse(":yume s"), Ok(Command::SetScheme(String::new())));
        // A prefix that names two words names neither, loudly, rather than
        // quietly meaning whichever was written first.
        assert!(matches!(
            parse(":yume x"),
            Err(CommandError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn a_space_offers_what_may_follow_the_command() {
        // The point of the whole arrangement: nobody has to remember an
        // argument, only a verb. `:dense ` says what may come next.
        let words = |line: &str| -> Vec<&str> { complete(line).iter().map(|c| c.name).collect() };
        assert_eq!(words("dense "), ["on", "off"]);
        assert_eq!(words("dense o"), ["on", "off"]);
        assert_eq!(words("dense of"), ["off"]);
        assert_eq!(words("syntax "), ["markdown", "typst"]);
        assert_eq!(words("layout v"), ["vertical"]);

        // A parent command is not a mechanism of its own — its subcommands are
        // simply the words it takes, and they go as deep as they like.
        assert_eq!(words("ruby "), ["on", "off", "html", "typst", "format"]);
        assert_eq!(words("ruby html "), ["on", "off"]);

        // Where the word being completed starts, so a completion replaces it
        // and not the whole line.
        assert_eq!(complete_at("ruby ht").0, 5);
        assert_eq!(complete_at("ruby html o").0, 10);
        assert_eq!(complete_at("dense").0, 0);

        // A command that takes free text says what it wants rather than
        // offering a list it does not have.
        let free = complete("goto ");
        assert_eq!(free.len(), 1);
        assert_eq!(free[0].name, "", "nothing to complete");
        assert!(free[0].help.contains("行號"), "but it says what to type");

        // A path is the caller's business, and a word nothing accepts is
        // nothing rather than the whole list again.
        assert!(complete("write draft.md ").is_empty());
        assert!(complete("quit ").is_empty());
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
            "write",
            "quit",
            "goto",
            "count",
            "grep",
            "toc",
            "export",
            "ruby",
            "yume",
            "buffer",
            "clipboard",
            "layout",
            "wrap",
            "table",
            "dense",
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
        /// Build a well-formed line by walking what the command says it takes.
        ///
        /// Derived rather than listed, so a command added to the table is
        /// checked without anybody remembering to add it here too — and so a
        /// subcommand that the parser does not actually accept is caught the
        /// moment it is offered by completion.
        fn sample(path: &str, args: &Args) -> String {
            match args {
                Args::None => path.to_string(),
                Args::Path => format!("{path} a.md"),
                Args::Words(list) => sample(&format!("{path} {}", list[0].name), &list[0].then),
                // A word only this command knows the shape of.
                Args::Free(_) => {
                    let word = match path {
                        p if p.ends_with("scheme") => "lingming",
                        ":export" => "html",
                        ":grep" => "x",
                        _ => "1",
                    };
                    format!("{path} {word}")
                }
            }
        }

        for entry in COMMANDS {
            let line = match entry.name {
                "s/pat/rep/" => ":s/a/b/".to_string(),
                // Listed under the shape it is typed in, not as a word.
                "!command" => ":!echo hi".to_string(),
                name => sample(&format!(":{name}"), &entry.args),
            };
            assert!(parse(&line).is_ok(), "{line} does not parse");
            // Every *word* a command offers has to parse too, not only the
            // first — completion offering a word the parser rejects is the
            // exact drift this table exists to catch.
            if let Args::Words(list) = &entry.args {
                for word in *list {
                    let line = sample(&format!(":{} {}", entry.name, word.name), &word.then);
                    assert!(parse(&line).is_ok(), "{line} does not parse");
                }
            }
            // Every spelling the table declares, not just the first: a table
            // that says `:e` opens a file and a parser that exports one is
            // exactly the drift this test exists to catch.
            for alias in entry.aliases {
                let line = match *alias {
                    "o" | "e" | "edit" => format!(":{alias} a.md"),
                    "g" => ":g 1".to_string(),
                    "gr" => ":gr x".to_string(),
                    "ex" => ":ex html".to_string(),
                    alias => format!(":{alias}"),
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
