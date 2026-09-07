//! Parsing of Ex-style command-line commands (the `:` commands).
//!
//! For Feature #1 only the commands needed to open a file or start a new buffer
//! are recognised. The parser is intentionally small and data-light so that
//! later features (`:w`, `:q`, `:s///`, …) can extend the [`Command`] enum and
//! the `match` in [`parse`] without disturbing existing call sites.

use std::fmt;

use crate::convert::Side;
use crate::ruby::Dialect;
use crate::say;
use crate::zong::Layout;

/// What `:word` was asked about — 分詞邊界, from every side.
///
/// **One subject, one command.** Where a word ends is decided by a dictionary,
/// shown by a colour, tuned by a level, mined out of the book itself, and read
/// back as 口頭禪; those were `:words`, `:segment` and a config key nobody could
/// see, and nothing said they were the same question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordCommand {
    /// `:word` — which dictionary is in force, and how many words this book adds.
    Report,
    /// `:word show on|off` — the colour that says where the boundaries fell.
    /// `None` flips it.
    Show(Option<bool>),
    /// `:word show tint|ink` — which of the two ways it is drawn: under the
    /// writing, or in the writing (Feature #278). Turns the overlay on.
    Mark(yumete_cjk::WordMark),
    /// `:word list` — the same report, from the list's side.
    List,
    /// `:word list reload` — read this book's list and the global one again.
    Reload,
    /// `:word list edit` — open this book's `.yumete/words.txt`, existing or not.
    Edit,
    /// `:word list global` — open the global `segmentation.txt`, existing or not.
    Global,
    /// `:word discover` — mine this book for the words no dictionary has, and
    /// write them into `.yumete/words.txt` unsaved (Feature #239).
    Discover,
    /// `:word habit` — the words this manuscript leans on, by surprisal
    /// against a 詞頻表 rather than by count (Feature #242). English writing
    /// calls these *crutch words*; the manual calls them 口頭禪.
    Habit,
    /// `:word level strict|balanced|full` — how readily characters join into
    /// words. `None` says which it is.
    Level(Option<yumete_cjk::WordLevel>),
}

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
    /// `:quit` (alias `:q`) or `:quit!` / `:q!` — close **this file**, and
    /// leave the editor only when it was the last one open. Vim's rule and
    /// helix's. `force` skips the unsaved-changes check.
    Quit {
        force: bool,
    },
    /// `:quitall` (alias `:qa`) or `:quitall!` / `:qa!` — leave, however many
    /// files are open.
    QuitAll {
        force: bool,
    },
    /// `:wq [path]` / `:x` — save (optionally to a new path), then leave.
    WriteQuit(Option<String>),
    /// `:count` (alias `:wc`) — how much has been written.
    Count,
    /// `:progress` — 寫作進度: what was written today, and every day before
    /// (Feature #244).
    Progress,
    /// `:target <字>` — how many 字 a day; `None` is `:target off`.
    Target(Option<usize>),
    /// `:check usage` — which of two spellings the manuscript settled on, and
    /// where it slipped (Feature #233).
    CheckUsage,
    /// `:check punct` — half-width marks in Chinese text, `...` for ……, and
    /// the 「 nothing closes (Feature #238).
    CheckPunct,
    /// `:check charset` — the characters no current standard carries, before
    /// the typesetter's font finds out (Feature #240).
    CheckCharset,
    /// `:convert …` — 簡繁, run through opencc (Feature #241).
    Convert(ConvertAsk),
    /// `:<n>` or `:goto <n>` (alias `:g`) — put the cursor on line `n`.
    GotoLine(usize),
    /// `:recover` — load the crash-recovery draft into the buffer;
    /// `:recover!` — throw it away instead (Feature #79).
    Recover {
        discard: bool,
    },
    /// `:[range]s/pattern/replacement/[flags]` — substitute text.
    ///
    /// The delimiter is whatever character follows the `s`, so a pattern with
    /// a `/` in it — a date, a path, a URL — is written `:s#a/b#c#`.
    Substitute {
        pattern: String,
        replacement: String,
        /// `g`: every match on a line, not only the first.
        global: bool,
        /// `i`: ignore case.
        ignore_case: bool,
        /// `n`: say how many there are and change nothing, as vi's `n` means.
        count_only: bool,
        /// `t`: yes, this changes how many cells a row has — 表格的欄數也改.
        /// Without it a substitution that would reshape a grid is refused, and
        /// that refusal has to name a way through or it is a wall.
        reshape: bool,
        /// Which lines it touches.
        rows: Rows,
    },
    /// `:replace <text>` — change what the last `:grep` found, everywhere it
    /// found it. The pattern is the one you already looked at. `:replace!`
    /// goes through even where it changes how many cells a row has.
    ReplaceFound(String, bool),
    /// `:wa` — save every buffer that has changed.
    WriteAll,
    /// `:undo` (alias `:u`) — undo the last change.
    Undo,
    /// `:redo` (alias `:red`) — redo the last undone change.
    Redo,
    /// `:word …` — everything about **where one word ends and the next
    /// begins**, which is one subject and used to be several commands
    /// (`:segment` coloured the boundaries, `:words` weighed them).
    Word(WordCommand),
    /// `:layout [horizontal|vertical]` (aliases `:horizontal`, `:vertical`) —
    /// choose the layout (Feature #61). `None` toggles between the two.
    SetLayout(Option<Layout>),
    /// `:yume chaifen [on|off]` — the 拆分 annotation beside candidates
    /// (Feature #66). `None` is the bare word, which toggles.
    SetChaifen(Option<bool>),
    /// `:scheme <tag>` — switch the input scheme (Feature #86).
    SetScheme(String),
    /// `:hanging [on|off]` — 句讀 in the margin rather than a square each
    /// (Feature #70). `None` is the bare word, which toggles.
    SetHanging(Option<bool>),
    /// `:wrap` / `:nowrap` — whether a paragraph too wide for the terminal
    /// continues on the next screen row (Feature #77).
    SetSoftWrap(bool),
    /// `:wrap <n>` — write to a measure of `n` columns rather than to the
    /// window; `:wrap 0` gives the window back (Feature #113).
    SetMeasure(Option<usize>),
    /// `:wheel <n>` — how far one notch of the mouse wheel moves, in whichever
    /// unit the page is set in; `None` only reports (Feature #222).
    SetWheelStep(Option<usize>),
    /// `:table` / `:table off` — read the file as a grid (Feature #118).
    /// `:table` — the door: read the table the cursor is in.
    EnterTable,
    /// `:table off|basic|full` — how much of a table is drawn (#283).
    SetTableLevel(crate::editor::TableLevel),
    /// `:table rules …` — how the columns are told apart (Feature #157).
    /// `None` only reports.
    SetTableRules(Option<crate::table::Rules>),
    /// `:format`, `:run <名字>` — a command this language declares in the
    /// config, run by the front end.
    Language(String),
    /// `:markdown …` — write a piece of Markdown at the cursor.
    Markdown(MarkdownBit),
    /// `:typewriter [on|off]` — the cursor's row stays in the middle.
    SetTypewriter(Option<bool>),
    /// `:focus [on|off]` — everything but the 段 being written stands back.
    SetFocus(Option<bool>),
    /// `:meter [on|off]` — 平仄 and 韻腳 in the margin.
    SetMeter(Option<bool>),
    /// `:note [on|off]` — the mark that is wrong, named on the page beside it.
    SetNote(Option<bool>),
    /// `:table numbers on|off` — the row of column numbers above the header.
    SetTableNumbers(bool),
    /// `:table header [on|off]` — whether the grid's first row names the
    /// columns or is a row like any other (Feature #217). `None` flips it.
    SetTableHeader(Option<bool>),
    /// `:table schema` — the schema file in the other work area, written next
    /// to the data first if none claims it yet (Feature #218).
    OpenTableSchema,
    /// `:table new 3 4` — an empty `|` table of this shape, blank lines around
    /// it, the cursor typing in its first heading (Feature #276). `rows`
    /// counts the heading; the rule row is not a row.
    NewTable { rows: usize, columns: usize },
    /// `:table detail [on|off]` — the panel; `None` toggles.
    ShowDetail(Option<bool>),
    /// `:table detail 40` — how wide it is.
    SetDetailWidth(usize),
    /// `:table sort 1 a 2 d` — put the rows in order by these columns, in this
    /// order. Empty sorts by the column the cursor is in.
    SortTable(Vec<(usize, bool)>),
    /// `:table pipe [分隔]` — the delimited block under the cursor becomes a
    /// `|` table (Feature #227). `None` guesses the delimiter.
    TableToPipe(Option<char>),
    /// `:table csv [分隔]` — the `|` table under the cursor becomes delimited
    /// lines (Feature #227). The delimiter defaults to a comma.
    TableToDelimited(char),
    /// `:numbers fill` — whether the line-number band has a ground of its
    /// own. `None` toggles.
    SetNumberFill(Option<bool>),
    /// `:shot` — a picture of the page, drawn by the editor itself (#189).
    Screenshot { shot: Shot, force: bool },
    /// `:theme` — which theme, and whether it is dark, light or the
    /// terminal's own answer (Feature #152).
    ///
    /// Both halves are optional and either may be written alone: `:theme`
    /// says where things stand, `:theme dark` keeps the theme and changes the
    /// mood, `:theme moxiang` names the theme and leaves the mood as it is.
    Theme {
        name: Option<String>,
        mood: Option<Mood>,
    },
    /// `:indent 2` — how many squares open a paragraph; `:indent off` is none.
    SetIndent(usize),
    /// `:indent off|basic|full` — whether paragraphs are indented, and whether
    /// the blank line the indent stands in for comes off the page (#283).
    SetIndentLevel(crate::editor::Render),
    /// `:indent` on its own — which of the three levels it is on.
    ReportIndent,
    /// `:ruby off|basic|full` — whether a reading is known, and whether it is
    /// drawn beside its base (#283).
    SetRubyLevel(crate::editor::Render),
    /// `:indent hint color` — what, if anything, is drawn in the opening
    /// squares.
    SetIndentHint(crate::zong::IndentHint),
    /// `:bands 2` — how many bands the 縱書 page is divided into (段組).
    SetBands(usize),

    /// `:search row|column <pattern>` — the two directions a search can run.
    Search { pattern: String, by: Axis },
    /// `:table check` — look the whole table over and list what is wrong.
    CheckTable,
    /// `:dense` / `:dense off` — pack the 縱書 page as tight as a terminal can
    /// (Feature #120).
    SetDense(bool),
    /// One 句 to a 縱 — a view of the page, not a change to the file.
    SetSentences(bool),
    /// `:render off|on|full` — how much of the result the page shows
    /// (Features #96 / #104).
    SetRender(crate::editor::Render),
    /// `:render` with no argument — say which level all four dimensions are on.
    ReportRender,
    /// `:hud off|basic|full` — how loudly the editor says, beside the caret,
    /// what you have typed (Feature #284).
    SetHud(crate::editor::Hud),
    /// `:hud` with no argument — say which of the three it is on.
    ReportHud,
    /// `:preview` / `:preview off` — hand the file to the real typesetter and
    /// show what it makes (Feature #128).
    SetPreview(bool),
    /// `:w!` — write over a file that changed on disk since it was read.
    WriteForce(Option<String>),
    /// `:reload` / `:reload!` — read the file again. The `!` throws away
    /// unsaved changes; without it a dirty buffer is refused (Feature #214).
    Reload { force: bool },
    /// `:reload auto on|off` — re-read a **clean** buffer by itself when the
    /// file changes on disk. `None` asks which it is (Feature #214).
    ReloadAuto(Option<bool>),
    /// `:readonly on|off` — lock this buffer against editing. `None` asks
    /// (Feature #213).
    SetReadonly(Option<bool>),
    /// `:yume builtin` — use the 碼表 in the binary, whatever is installed.
    BuiltinScheme,
    /// `:yume table <path>` — type with a code table of your own.
    UserTable(String),
    /// `:yume` on its own — say what the input method is doing.
    YumeStatus,
    /// `:yume commit delayed|unique|fluency` — 上屏方式: when a finished code
    /// goes to the page. `None` asks which one is in force (Feature #209).
    YumeCommit(Option<String>),
    /// `:yume panel full|bare` — 候選面板: the bordered list, or the first
    /// candidate drawn into the sentence. `None` asks which one is in force.
    ///
    /// Independent of [`Command::YumeCommit`]: **when** a word lands on the
    /// page and **where** you read the candidate are two questions, and the
    /// nine combinations are all sensible (Feature #211).
    YumePanel(Option<String>),
    /// `:yume on` / `:yume off` — type 漢字, or type what the keys say.
    ///
    /// The 中/英 switch already exists as a lone Shift tap; this is the same
    /// switch with a name, for a hand that is already on `:` — and `on` also
    /// loads the 碼表 when it has not been loaded, which is the whole of
    /// starting to write in Chinese.
    YumeLanguage(bool),
    /// `:yume installed` — the 碼表 the *system* has, which is `builtin`'s
    /// other half: one binary, two tables, and a way back from either.
    InstalledScheme,
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
    /// `:saveas <path>` (and `:saveas!`) — write this buffer to another file
    /// **and go on editing that one**. `:w <path>` is the other half: a copy,
    /// leaving the buffer where it is.
    SaveAs {
        path: String,
        force: bool,
    },
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
    /// `:help [節]` — the keys and the commands, in a buffer.
    Help(Option<String>),
    /// `:tutor` — a lesson, written into a file of the reader's own.
    Tutor,
    /// `:toc [n]` — list the headings, or go to the nth.
    Outline(Option<usize>),
    /// `:row 木` — go to the row this table names by that character.
    GotoRow(String),
    /// `:grep <pattern>` — search every file in the project.
    Grep(String),
    /// `:conflicts` — the merge conflicts in this file, as a results buffer
    /// (Feature #249).
    Conflicts,
    /// `:diff [path]` — what changed, by 詞, against the file on disk or
    /// against another draft (Feature #235).
    Diff(Option<String>),
    /// `:export html|typst [path]`, and `:export!` over a file that is
    /// already there — write the manuscript out for a typesetter.
    Export {
        format: String,
        path: Option<String>,
        force: bool,
    },
    /// `:ruby auto` — write the readings in by word; `rare` keeps only the
    /// words holding a character outside 通用規範漢字表 (Feature #234).
    AutoRuby {
        rare: bool,
    },
    /// `:ruby` — open Ruby mode on the group or selection at the cursor
    /// (Feature #65).
    Ruby,
    /// `:ruby <dialect> [off]` — one spelling of a reading, added to or taken
    /// from the set being laid out.
    ///
    /// **An override, not a level.** The three level words say how much of the
    /// reading dimension is drawn; this says *which spelling*, and naming one
    /// is asking to see it. The bare `:ruby on` / `:ruby off` this grew out of
    /// retired with #283 — they were the two-state vocabulary the entry
    /// exists to replace.
    RenderRuby {
        dialect: Dialect,
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
    /// Something followed a word that takes nothing.
    ///
    /// Not the same complaint as [`CommandError::InvalidArgument`], which says
    /// 「不認得」 — a file name after `:shot screen` is a perfectly good file
    /// name, and telling the writer it is not recognised sends them looking
    /// for the typo in it.
    TakesNoArgument {
        command: &'static str,
        value: String,
    },
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::Empty => write!(f, "{}", crate::say!("cmd.no-command-typed")),
            CommandError::Unknown(word) => {
                write!(f, "{}", crate::say!("cmd.no-such-command", word))
            }
            CommandError::MissingArgument(what) => {
                write!(f, "{}", crate::say!("cmd.needs-an-argument", what))
            }
            CommandError::InvalidArgument { command, value } => {
                write!(f, "{}", crate::say!("cmd.not-one-of-its-values", command, value))
            }
            CommandError::TakesNoArgument { command, value } => {
                write!(f, "{}", crate::say!("cmd.takes-nothing-after-it", command, value))
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
    // The prefix rule goes on past the command name. The menu prints the
    // shortest unambiguous spelling of every word that may follow, and most of
    // those spellings used to be lies: the arms below match whole words, so
    // `:table check` growing a `csv` sibling turned the menu's `check (ch)`
    // into 「不認得」 — and `:render f`, `:buffer n`, `:clipboard y`, `:help t`
    // and a dozen more had never worked at all.
    let spelled = spell_out(word, rest);
    let rest = spelled.as_deref().unwrap_or(rest);

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
        // The other half of `:w!`: take what is on disk and lose what is here.
        // `:e!` and `:o!` used to say this; they are gone, with no alias and no
        // hint, because `open!` reads as 「open, but harder」 and what it
        // actually does is discard the afternoon.
        "reload" | "reload!" => {
            let force = word.ends_with('!');
            // The bang answers 「throw this buffer's changes away」, which is
            // not a question `auto` asks — so `:reload! auto on` is a typo,
            // not a setting, and saying so beats obeying half of it.
            match (force, rest) {
                (_, "") => Ok(Command::Reload { force }),
                (false, "auto") => Ok(Command::ReloadAuto(None)),
                (false, "auto on") => Ok(Command::ReloadAuto(Some(true))),
                (false, "auto off") => Ok(Command::ReloadAuto(Some(false))),
                (_, other) => Err(CommandError::InvalidArgument {
                    command: "reload",
                    value: other.to_string(),
                }),
            }
        }
        // 只讀 (Feature #213).
        "readonly" | "ro" => Ok(Command::SetReadonly(match rest {
            "" => None,
            word => Some(switch("readonly", word)?),
        })),
        // Rebinding, said out loud. `:w <path>` is a copy.
        "saveas" | "sav" | "saveas!" | "sav!" => {
            if rest.is_empty() {
                Err(CommandError::MissingArgument("saveas"))
            } else {
                Ok(Command::SaveAs {
                    path: rest.to_string(),
                    force: word.ends_with('!'),
                })
            }
        }
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
        // A bare `:target` is the question, not half a command: somebody who
        // types it wants to know what the target is and how far off it is, and
        // that is what `:progress` answers.
        "progress" | "prog" => Ok(Command::Progress),
        "target" => match rest {
            "" => Ok(Command::Progress),
            "off" | "none" | "0" => Ok(Command::Target(None)),
            n => match n.parse::<usize>() {
                Ok(n) => Ok(Command::Target(Some(n))),
                Err(_) => Err(CommandError::InvalidArgument {
                    command: "target",
                    value: n.to_string(),
                }),
            },
        },
        "check" => match rest {
            "usage" => Ok(Command::CheckUsage),
            "punct" => Ok(Command::CheckPunct),
            "charset" => Ok(Command::CheckCharset),
            // `:check` on its own asks what to check rather than guessing:
            // 用字 is the first of several, and the day 標點 lands a bare
            // `:check` that had quietly meant one of them would change what
            // it does under everybody who had typed it.
            "" => Err(CommandError::MissingArgument("check")),
            other => Err(CommandError::InvalidArgument {
                command: "check",
                value: other.to_string(),
            }),
        },
        // 簡繁 (Feature #241). Two words and an optional `force`: the pair
        // names an opencc config (`s tw` is `s2tw`), so what the editor runs is
        // readable from what was typed.
        "convert" => match rest.split_whitespace().collect::<Vec<_>>().as_slice() {
            [] | ["opencc"] => Ok(Command::Convert(ConvertAsk::Explain)),
            ["opencc", "install"] => Ok(Command::Convert(ConvertAsk::Opencc { update: false })),
            ["opencc", "update"] => Ok(Command::Convert(ConvertAsk::Opencc { update: true })),
            [from, to] | [from, to, "force"] => {
                let force = rest.split_whitespace().count() == 3;
                match (Side::parse(from), Side::parse(to)) {
                    (Some(from), Some(to)) => Ok(Command::Convert(ConvertAsk::Run {
                        from,
                        to,
                        force,
                    })),
                    (None, _) => Err(CommandError::InvalidArgument {
                        command: "convert",
                        value: (*from).to_string(),
                    }),
                    (_, None) => Err(CommandError::InvalidArgument {
                        command: "convert",
                        value: (*to).to_string(),
                    }),
                }
            }
            other => Err(CommandError::InvalidArgument {
                command: "convert",
                value: other.join(" "),
            }),
        },
        "goto" | "g" => rest
            .parse::<usize>()
            .map(Command::GotoLine)
            .map_err(|_| CommandError::MissingArgument("goto")),
        "recover" => Ok(Command::Recover { discard: false }),
        "recover!" => Ok(Command::Recover { discard: true }),
        // Closing *this file* rather than the editor, spelled the way helix
        // spells it (`:bc`, `:bclose`) — see the note on [`Command::Quit`].
        // Not `:Q`: one Shift away from `:q` and meaning something else is
        // exactly where a hand slips, and every other command here is
        // lowercase.
        "bclose" | "bc" => Ok(Command::CloseBuffer { force: false }),
        "bclose!" | "bc!" => Ok(Command::CloseBuffer { force: true }),
        "quit" | "q" => Ok(Command::Quit { force: false }),
        "quit!" | "q!" => Ok(Command::Quit { force: true }),
        "quitall" | "qa" => Ok(Command::QuitAll { force: false }),
        "quitall!" | "qa!" => Ok(Command::QuitAll { force: true }),
        "undo" | "u" => Ok(Command::Undo),
        "redo" | "red" => Ok(Command::Redo),
        // 分詞邊界, from four sides — see [`Word`]. `:segment` and `:words`
        // were two of them and are gone: one subject, one command.
        "word" | "wd" => match rest.split_whitespace().collect::<Vec<_>>().as_slice() {
            [] => Ok(Command::Word(WordCommand::Report)),
            ["show"] => Ok(Command::Word(WordCommand::Show(None))),
            ["show", "on"] => Ok(Command::Word(WordCommand::Show(Some(true)))),
            ["show", "off"] => Ok(Command::Word(WordCommand::Show(Some(false)))),
            // `:word show 字色` is the same question as `:word show on` — how
            // this is drawn — so it lives under the same word rather than
            // making the writer learn a second one.
            ["show", how] if yumete_cjk::WordMark::parse(how).is_some() => Ok(Command::Word(
                WordCommand::Mark(yumete_cjk::WordMark::parse(how).unwrap()),
            )),
            ["list"] => Ok(Command::Word(WordCommand::List)),
            ["list", "reload"] => Ok(Command::Word(WordCommand::Reload)),
            ["list", "edit"] => Ok(Command::Word(WordCommand::Edit)),
            ["list", "global"] => Ok(Command::Word(WordCommand::Global)),
            ["discover"] => Ok(Command::Word(WordCommand::Discover)),
            ["habit"] => Ok(Command::Word(WordCommand::Habit)),
            ["level"] => Ok(Command::Word(WordCommand::Level(None))),
            ["level", name] => match yumete_cjk::WordLevel::parse(name) {
                Some(level) => Ok(Command::Word(WordCommand::Level(Some(level)))),
                None => Err(CommandError::InvalidArgument {
                    command: "word level",
                    value: (*name).to_string(),
                }),
            },
            other => Err(CommandError::InvalidArgument {
                command: "word",
                value: other.join(" "),
            }),
        },
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
                Some("which") => Ok(Command::YumeStatus),
                Some("on") => Ok(Command::YumeLanguage(true)),
                Some("off") => Ok(Command::YumeLanguage(false)),
                Some("installed") => Ok(Command::InstalledScheme),
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
                        .and_then(|tag| pick(tag, schemes()).map(|w| w.name))
                        .unwrap_or("")
                        .to_string(),
                )),
                Some("chaifen") => Ok(Command::SetChaifen(match parts.next() {
                    None => None,
                    Some(word) => Some(switch("chaifen", word)?),
                })),
                // On its own it is the question — which of the three is
                // answering — the same as a bare `:yume`.
                Some("commit") => match parts.next() {
                    None => Ok(Command::YumeCommit(None)),
                    // `auto` is 唯一 under the name people use for it; every
                    // other spelling is picked by prefix, so `d` `u` `f` work.
                    Some("auto") => Ok(Command::YumeCommit(Some("unique".to_string()))),
                    Some(word) => match pick(word, COMMITS).map(|w| w.name) {
                        Some(name) => Ok(Command::YumeCommit(Some(name.to_string()))),
                        None => Err(CommandError::InvalidArgument {
                            command: "yume commit",
                            value: word.to_string(),
                        }),
                    },
                },
                // Likewise a question on its own — and one word for both
                // halves of it, because「面板」is what a reader calls the
                // thing whether it is drawn or not.
                Some("panel") => match parts.next() {
                    None => Ok(Command::YumePanel(None)),
                    Some(word) => match pick(word, PANELS).map(|w| w.name) {
                        Some(name) => Ok(Command::YumePanel(Some(name.to_string()))),
                        None => Err(CommandError::InvalidArgument {
                            command: "yume panel",
                            value: word.to_string(),
                        }),
                    },
                },
                _ => Err(CommandError::InvalidArgument {
                    command: "yume",
                    value: word.to_string(),
                }),
            }
        }

        // 「墨香」 and `moxiang` are the same word, and a mood may be written
        // with or without the theme's name: every combination of the two
        // halves is a sentence, because there is nothing to be gained by
        // refusing one.
        "shot" | "shot!" => {
            let force = word.ends_with('!');
            let rest = rest.trim();
            // The word, then whatever is left — which is a file name and may
            // hold spaces, so it is split once rather than by whitespace.
            let (kind, path) = match rest.split_once(char::is_whitespace) {
                Some((kind, path)) => (kind, path.trim()),
                None => (rest, ""),
            };
            let named = (!path.is_empty()).then(|| path.to_string());
            let file = |how| Shot::File { how, path: named };
            let shot = match pick(kind, SHOT).map(|w| w.name) {
                // **Bare `:shot` is the screen.** 「截圖」 means a picture you
                // can paste; the drawn page is the specialist and says so.
                _ if kind.is_empty() => Shot::Screen,
                // A name after `screen` is refused rather than dropped: the
                // clipboard has nowhere to put one, and a path that quietly
                // does nothing is how a person comes to hunt for a file that
                // was never written.
                Some("screen") if !path.is_empty() => {
                    return Err(CommandError::TakesNoArgument {
                        command: "shot screen",
                        value: path.to_string(),
                    })
                }
                Some("screen") => Shot::Screen,
                Some("png") => file(ShotFormat::Png),
                Some("html") => file(ShotFormat::Html),
                Some("txt") => file(ShotFormat::Text),
                _ => {
                    return Err(CommandError::InvalidArgument {
                        command: "shot",
                        value: kind.to_string(),
                    })
                }
            };
            Ok(Command::Screenshot { shot, force })
        }
        "numbers" => match rest {
            // On its own it says nothing about *what* of the numbers, so it
            // asks rather than guessing.
            "" => Err(CommandError::MissingArgument("numbers")),
            "fill" => Ok(Command::SetNumberFill(None)),
            "fill on" => Ok(Command::SetNumberFill(Some(true))),
            "fill off" => Ok(Command::SetNumberFill(Some(false))),
            other => Err(CommandError::InvalidArgument {
                command: "numbers",
                value: other.to_string(),
            }),
        },
        // The appearance on its own: which way round the theme's inks go.
        "appearance" => {
            let mut mood = None;
            for word in rest.split_whitespace() {
                match pick(word, MOODS).map(|w| w.name) {
                    Some("system") => mood = Some(Mood::System),
                    Some("dark") => mood = Some(Mood::Dark),
                    Some("light") => mood = Some(Mood::Light),
                    _ => {
                        return Err(CommandError::InvalidArgument {
                            command: "appearance",
                            value: word.to_string(),
                        })
                    }
                }
            }
            Ok(Command::Theme { name: None, mood })
        }

        "theme" => {
            let mut name = None;
            let mut mood = None;
            for word in rest.split_whitespace() {
                match pick(word, MOODS).map(|w| w.name) {
                    Some("system") => mood = Some(Mood::System),
                    Some("dark") => mood = Some(Mood::Dark),
                    Some("light") => mood = Some(Mood::Light),
                    // Anything else is a theme's name. Which names exist is
                    // the front end's business — it is the one holding the
                    // colours — so an unknown one is answered there, in a
                    // sentence, rather than refused here as a syntax error.
                    _ => {
                        name = Some(match pick(word, THEMES).map(|w| w.name) {
                            Some(known) => known.to_string(),
                            None => word.to_string(),
                        })
                    }
                }
            }
            Ok(Command::Theme { name, mood })
        }

        "hanging" => Ok(Command::SetHanging(match rest {
            "" => None,
            word => Some(switch("hanging", word)?),
        })),
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
        "preview" => Ok(Command::SetPreview(match rest {
            "" => true,
            word => switch("preview", word)?,
        })),
        // **The bare word reports** (#283). It used to mean 中階, back when
        // there were two levels and 「the other one」 named itself. With three
        // — and three more dimensions taking their level from this one —
        // 「which level am I on」 is the better use of the word, and it is the
        // shape `:table rules` already had.
        "render" => match rest {
            "" => Ok(Command::ReportRender),
            "basic" => Ok(Command::SetRender(crate::editor::Render::Basic)),
            "off" => Ok(Command::SetRender(crate::editor::Render::Off)),
            "full" => Ok(Command::SetRender(crate::editor::Render::Full)),
            other => Err(CommandError::InvalidArgument {
                command: "render",
                value: other.to_string(),
            }),
        },
        // The bare word reports, the same shape `:render` has — and for the
        // same reason: with three levels, 「which one am I on」 is a better use
        // of the word than a fourth spelling of the middle one.
        "hud" => match rest {
            "" => Ok(Command::ReportHud),
            "off" => Ok(Command::SetHud(crate::editor::Hud::Off)),
            "basic" => Ok(Command::SetHud(crate::editor::Hud::Basic)),
            "full" => Ok(Command::SetHud(crate::editor::Hud::Full)),
            other => Err(CommandError::InvalidArgument {
                command: "hud",
                value: other.to_string(),
            }),
        },
        // `:wrap` on its own still means what it always meant — turn wrapping
        // on — and leaves the measure alone; a number sets the measure.
        "dense" => Ok(Command::SetDense(match rest {
            "" => true,
            word => switch("dense", word)?,
        })),
        "sentence" => Ok(Command::SetSentences(match rest {
            "" => true,
            word => switch("sentence", word)?,
        })),
        "search" => {
            // `:search <pattern>` with no direction is a row search, because
            // that is what a search is anywhere but a table.
            let (by, pattern) = match rest.split_once(char::is_whitespace) {
                Some((word, rest)) if pick(word, AXIS).map(|w| w.name) == Some("column") => {
                    (Axis::Column, rest.trim())
                }
                Some((word, rest)) if pick(word, AXIS).map(|w| w.name) == Some("row") => {
                    (Axis::Row, rest.trim())
                }
                _ => (Axis::Row, rest),
            };
            if pattern.is_empty() {
                Err(CommandError::MissingArgument("search"))
            } else {
                Ok(Command::Search {
                    pattern: pattern.to_string(),
                    by,
                })
            }
        }
        "bands" => match rest {
            "" | "on" | "2" => Ok(Command::SetBands(2)),
            "off" | "1" => Ok(Command::SetBands(1)),
            n => match n.parse::<usize>() {
                Ok(n) if (1..=4).contains(&n) => Ok(Command::SetBands(n)),
                _ => Err(CommandError::InvalidArgument {
                    command: "bands",
                    value: n.to_string(),
                }),
            },
        },
        "indent" => match rest {
            // A number is the *width*; a word is the level. `:indent 4` on a
            // page at 中階 widens the indent and still does not fold, because
            // those are two questions and the reader answered one of them.
            // The bare word reports, for the reason bare `:render` reports:
            // with three levels, 「which one am I on」 is the better use of it.
            "" => Ok(Command::ReportIndent),
            "full" => Ok(Command::SetIndentLevel(crate::editor::Render::Full)),
            "basic" => Ok(Command::SetIndentLevel(crate::editor::Render::Basic)),
            "off" | "0" => Ok(Command::SetIndentLevel(crate::editor::Render::Off)),
            _ if rest.starts_with("hint") => {
                match crate::zong::IndentHint::parse(rest.trim_start_matches("hint").trim()) {
                    Some(hint) => Ok(Command::SetIndentHint(hint)),
                    None => Err(CommandError::InvalidArgument {
                        command: "indent hint",
                        value: rest.trim_start_matches("hint").trim().to_string(),
                    }),
                }
            }
            n => match n.parse::<usize>() {
                Ok(n) if n <= 8 => Ok(Command::SetIndent(n)),
                _ => Err(CommandError::InvalidArgument {
                    command: "indent",
                    value: n.to_string(),
                }),
            },
        },
        // `:markdown footnote` and friends — what Markdown is *made of*,
        // written for you. The verb is the language's, because these are things
        // only a Markdown file has.
        // **The verb is the same in every file; the config says how.** See
        // `yumete_config::Runner`.
        "format" | "fmt" => Ok(Command::Language("format".to_string())),
        "run" => match rest.is_empty() {
            true => Err(CommandError::MissingArgument("run")),
            false => Ok(Command::Language(rest.to_string())),
        },
        "markdown" | "md" => match rest {
            "" => Err(CommandError::MissingArgument("markdown")),
            "footnote" | "fn" => Ok(Command::Markdown(MarkdownBit::Footnote)),
            "footnote inline" | "fni" => Ok(Command::Markdown(MarkdownBit::InlineNote)),
            other => Err(CommandError::InvalidArgument {
                command: "markdown",
                value: other.to_string(),
            }),
        },
        "typewriter" => Ok(Command::SetTypewriter(match rest {
            "" => Some(true),
            "toggle" => None,
            word => Some(switch("typewriter", word)?),
        })),
        "focus" => Ok(Command::SetFocus(match rest {
            "" => Some(true),
            "toggle" => None,
            word => Some(switch("focus", word)?),
        })),
        "meter" => Ok(Command::SetMeter(match rest {
            "" => Some(true),
            "toggle" => None,
            word => Some(switch("meter", word)?),
        })),
        "note" => Ok(Command::SetNote(match rest {
            "" => Some(true),
            "toggle" => None,
            word => Some(switch("note", word)?),
        })),
        "table" => match rest {
            // **The bare word is the door; a level is a level.** `:table`
            // reads the table the cursor is in — that is what it has always
            // meant and it is not a surface. The three words say how much of
            // one is drawn, and `off` is both: a page with no grid on it is a
            // page you are not in.
            "" => Ok(Command::EnterTable),
            "off" => Ok(Command::SetTableLevel(crate::editor::TableLevel::Off)),
            "basic" => Ok(Command::SetTableLevel(crate::editor::TableLevel::Basic)),
            "full" => Ok(Command::SetTableLevel(crate::editor::TableLevel::Full)),
            "check" => Ok(Command::CheckTable),
            _ if rest.starts_with("detail ") => {
                match rest["detail ".len()..].trim() {
                    "off" => Ok(Command::ShowDetail(Some(false))),
                    "on" => Ok(Command::ShowDetail(Some(true))),
                    n => match n.parse::<usize>() {
                        Ok(n) => Ok(Command::SetDetailWidth(n)),
                        Err(_) => Err(CommandError::InvalidArgument {
                            command: "table detail",
                            value: n.to_string(),
                        }),
                    },
                }
            }
            "detail" => Ok(Command::ShowDetail(None)),
            _ if rest.starts_with("sort") => {
                // `:table sort 1 a 2 d 4 a` — column, direction, column,
                // direction. Nothing at all sorts by the column you are in.
                let mut keys = Vec::new();
                let mut words = rest["sort".len()..].split_whitespace();
                while let Some(word) = words.next() {
                    let Ok(column) = word.parse::<usize>() else {
                        return Err(CommandError::InvalidArgument {
                            command: "table sort",
                            value: word.to_string(),
                        });
                    };
                    let down = match words.next() {
                        None | Some("a") | Some("asc") => false,
                        Some("d") | Some("desc") => true,
                        Some(other) => {
                            return Err(CommandError::InvalidArgument {
                                command: "table sort",
                                value: other.to_string(),
                            })
                        }
                    };
                    keys.push((column, down));
                }
                Ok(Command::SortTable(keys))
            }
            "pipe" => Ok(Command::TableToPipe(None)),
            "csv" => Ok(Command::TableToDelimited(',')),
            _ if rest.starts_with("pipe ") || rest.starts_with("csv ") => {
                let (word, arg) = rest.split_once(' ').unwrap_or((rest, ""));
                match delimiter_named(arg.trim()) {
                    Some(c) if word == "pipe" => Ok(Command::TableToPipe(Some(c))),
                    Some(c) => Ok(Command::TableToDelimited(c)),
                    None => Err(CommandError::InvalidArgument {
                        command: match word {
                            "pipe" => "table pipe",
                            _ => "table csv",
                        },
                        value: arg.trim().to_string(),
                    }),
                }
            }
            "numbers" | "numbers on" => Ok(Command::SetTableNumbers(true)),
            "numbers off" => Ok(Command::SetTableNumbers(false)),
            // 「這一行是欄名還是資料」 — on its own it flips, because that is
            // the question a 碼表 asks once and never again (#217).
            // 「這張表到底是怎麼讀的」 — the answer is a file, so the
            // command opens it rather than printing it (#218).
            "schema" => Ok(Command::OpenTableSchema),
            // 「迅速在 markdown 中插入一個三行四列表格」 (#276). Two numbers,
            // 行 then 欄, the way the author said it and the way a word
            // processor's「插入表格」dialog asks. **`rows` counts the heading**:
            // the rule row underneath is punctuation, and nobody means it when
            // they say three.
            _ if rest == "new" || rest.starts_with("new ") => {
                let spec = rest["new".len()..].trim();
                let mut numbers = spec.split(['x', 'X', '×', ' ', '\t']).filter(|w| !w.is_empty());
                let rows = numbers.next().unwrap_or("3");
                let columns = numbers.next().unwrap_or("3");
                match (rows.parse::<usize>(), columns.parse::<usize>()) {
                    (Ok(r), Ok(c)) if (1..=200).contains(&r) && (1..=32).contains(&c) => {
                        Ok(Command::NewTable { rows: r, columns: c })
                    }
                    _ => Err(CommandError::InvalidArgument {
                        command: "table new",
                        value: spec.to_string(),
                    }),
                }
            }
            "header" => Ok(Command::SetTableHeader(None)),
            "header on" => Ok(Command::SetTableHeader(Some(true))),
            "header off" => Ok(Command::SetTableHeader(Some(false))),
            "rules" => Ok(Command::SetTableRules(None)),
            _ if rest.starts_with("rules ") => {
                match crate::table::Rules::parse(rest.trim_start_matches("rules ")) {
                    Some(rules) => Ok(Command::SetTableRules(Some(rules))),
                    None => Err(CommandError::InvalidArgument {
                        command: "table rules",
                        value: rest["rules ".len()..].to_string(),
                    }),
                }
            }
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
        // On its own it says what the step is; a number sets it. `0` is the
        // terminal's own step — one unit a notch.
        "wheel" => match rest {
            "" => Ok(Command::SetWheelStep(None)),
            n => match n.parse::<usize>() {
                Ok(n) => Ok(Command::SetWheelStep(Some(n))),
                Err(_) => Err(CommandError::InvalidArgument {
                    command: "wheel",
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
        "export" | "ex" | "export!" | "ex!" => {
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
                force: word.ends_with('!'),
            })
        }
        "conflicts" => Ok(Command::Conflicts),
        "grep" | "gr" => {
            if rest.is_empty() {
                Err(CommandError::MissingArgument("grep"))
            } else {
                Ok(Command::Grep(rest.to_string()))
            }
        }
        "diff" => Ok(Command::Diff(match rest.trim().is_empty() {
            true => None,
            false => Some(rest.trim().to_string()),
        })),
        "wa" | "wall" => Ok(Command::WriteAll),
        "replace" | "replace!" => {
            if rest.trim().is_empty() {
                Err(CommandError::MissingArgument("replace"))
            } else {
                Ok(Command::ReplaceFound(rest.to_string(), word.ends_with('!')))
            }
        }
        "row" => {
            if rest.is_empty() {
                Err(CommandError::MissingArgument("row"))
            } else {
                Ok(Command::GotoRow(rest.to_string()))
            }
        }
        "tutor" => Ok(Command::Tutor),
        "help" => Ok(Command::Help(match rest.is_empty() {
            true => None,
            false => Some(rest.to_string()),
        })),
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
                // Before the dialect arm below, which would read `auto` as a
                // dialect name and answer 「no such thing」.
                "auto" => match second {
                    None => Ok(Command::AutoRuby { rare: false }),
                    Some("rare") => Ok(Command::AutoRuby { rare: true }),
                    Some(other) => Err(CommandError::InvalidArgument {
                        command: "ruby auto",
                        value: other.to_string(),
                    }),
                },
                "off" => Ok(Command::SetRubyLevel(crate::editor::Render::Off)),
                "basic" => Ok(Command::SetRubyLevel(crate::editor::Render::Basic)),
                "full" => Ok(Command::SetRubyLevel(crate::editor::Render::Full)),
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
                        dialect,
                        on: on(second)?,
                    })
                }
            }
        }

        other => Err(CommandError::Unknown(other.to_string())),
    }
}

/// One entry of the command list: what to type, and what it does.
/// What was asked of `:convert`.
///
/// Three shapes rather than one because they are three different questions:
/// what can this do, do it, and 「the program it needs is not here」.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertAsk {
    /// `:convert` on its own — every pair it can do, and whether opencc is
    /// installed.
    ///
    /// **There is deliberately no default direction.** Half the manuscripts in
    /// the world want `s t` and half want `t s`; a bare `:convert` that picked
    /// one would rewrite a book the wrong way for the other half, and undo is
    /// not a good enough answer for that.
    Explain,
    /// `:convert <從> <到> [force]`.
    Run {
        from: Side,
        to: Side,
        /// Convert **words** too — opencc's `p` configs, which turn 内存 into
        /// 記憶體. Behind a word because it is a different promise: the text
        /// comes back a different length, and running the reverse config does
        /// not bring it back.
        force: bool,
    },
    /// `:convert opencc install` / `:convert opencc update`.
    Opencc {
        update: bool,
    },
}

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
    /// What has to be true before it does anything.
    pub needs: &'static [Need],
}

/// What `:shot` makes a picture of — Feature #189.
///
/// Two pictures, because there are two things a person means by 「截圖」. The
/// platform's screenshot program photographs a *window*: the title bar, the
/// tab strip, the terminal's own padding and whatever is in front of it. The
/// editor's own drawing is the **page** and nothing else, by construction —
/// no window, no scale, no chrome — because the renderer already produces the
/// frame cell by cell for `--shot`, so it works over ssh, on a headless
/// machine, and in a test.
///
/// The drawn page was the bare `:shot` for a while and should not have been
/// (2026-09-06): what a person wants nine times out of ten is a picture they
/// can paste, and the specialist is the one that should have to be named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shot {
    /// **The default.** Hand the screen to the platform's own screenshot
    /// program, which on macOS crops to the window and fills the clipboard.
    Screen,
    /// A picture kept as a file. [`ShotFormat`] says which kind, and `path`
    /// says where; without one the picture is named after the document and
    /// dated, in the downloads folder rather than beside the document.
    ///
    /// The format used to be read off the extension of a name that had to be
    /// given, which made 「plain text」 something you could only ask for by
    /// spelling out a path, and left the argument a free string with nothing
    /// to complete: `:shot ` opened an empty menu.
    File {
        how: ShotFormat,
        path: Option<String>,
    },
}

/// The three things `:shot` can leave on disk.
///
/// `Png` is not drawn by the editor at all — it is the same screenshot program
/// [`Shot::Screen`] uses, told to write a file instead of filling the
/// clipboard. So it photographs the window, while `Html` and `Text` draw the
/// page from the frame the reader is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotFormat {
    /// The page, with its colours.
    Html,
    /// The page, without them.
    Text,
    /// The window, by the platform's own program.
    Png,
}

impl ShotFormat {
    /// What the file is called after the dot.
    pub fn extension(self) -> &'static str {
        match self {
            ShotFormat::Html => "html",
            ShotFormat::Text => "txt",
            ShotFormat::Png => "png",
        }
    }
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
    /// One of the input schemes — **whichever ones are installed** (#169).
    ///
    /// The one argument in the table that is not written in the table. Every
    /// other list here is a closed set the editor defines (`on｜off`,
    /// `markdown｜typst｜text`); the schemes are a directory. A build that ships
    /// only 冰雪 should offer only 冰雪, and one that ships none of them falls
    /// back to the five this crate knows, which is what [`schemes`] answers.
    Schemes,
}

impl Args {
    /// The words this argument may be, as they stand now — `None` when it is
    /// not a word list at all.
    ///
    /// **Every reader of a word list goes through here**, and that is the point
    /// rather than a convenience: `Args::Schemes` is answered from a registry
    /// that a `match` arm on `Args::Words` would silently skip, and skipping it
    /// means a scheme that is installed does not appear. Written as one
    /// accessor so there is one place to be right.
    pub fn words(&self) -> Option<&'static [Word]> {
        match self {
            Args::Words(list) => Some(list),
            Args::Schemes => Some(schemes()),
            _ => None,
        }
    }

    /// What may follow the command, for a listing: `<檔名>`, `on|off`, or
    /// nothing at all.
    pub fn hint(&self) -> String {
        match self.words() {
            Some(words) => words
                .iter()
                .map(|w| w.name)
                .collect::<Vec<_>>()
                .join("｜"),
            None => match self {
                Args::Path => say!("cmd.arg.path"),
                Args::Free(what) => (*what).to_string(),
                _ => String::new(),
            },
        }
    }
}

/// The schemes the frontend found installed, or nothing while none has looked.
static FOUND_SCHEMES: std::sync::OnceLock<&'static [Word]> = std::sync::OnceLock::new();

/// Tell the command table what schemes are installed (#169).
///
/// Each pair is a tag and a display name. The name is shown beside the tag in
/// the `:` menu the way a `help` tag's translation is — an unknown tag renders
/// as itself, so a found scheme's 方案名 can stand in that slot directly and
/// reads correctly in every language, which is right, because 「冰雪四拼」 is
/// its name in all three.
///
/// Called once, at startup, before anything is drawn. A second call is ignored:
/// the completion table and the which-key panel are `&'static`, and a list that
/// changed under them mid-session would leak a slice per change for no gain.
pub fn set_schemes(found: &[(&str, &str)]) {
    // Before any leaking: a second call would otherwise leak a `Word` and two
    // strings per scheme and then throw the slice away at the `set`.
    if found.is_empty() || FOUND_SCHEMES.get().is_some() {
        return;
    }
    let words: Vec<Word> = found
        .iter()
        .map(|(tag, name)| Word {
            name: Box::leak(tag.to_string().into_boxed_str()),
            help: Box::leak(name.to_string().into_boxed_str()),
            needs: &[],
            then: Args::None,
        })
        .collect();
    let _ = FOUND_SCHEMES.set(Box::leak(words.into_boxed_slice()));
}

/// The schemes `:yume scheme` offers: what was found, else the built-in five.
pub fn schemes() -> &'static [Word] {
    FOUND_SCHEMES.get().copied().unwrap_or(SCHEMES)
}

/// What `:shot` makes a picture of, and what it leaves behind — see [`Shot`].
///
/// Words rather than a free string, because a free string had nothing to
/// offer: typing `:shot ` opened the command panel on an empty list, which is
/// what the writer saw and asked about. `png`, `html` and `txt` each take a
/// file name after them, and without one the picture is named after the
/// document and dated.
const SHOT: &[Word] = &[
    Word {
        name: "screen",
        help: "cmd.shot.screen",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "png",
        help: "cmd.shot.png",
        needs: &[],
        then: Args::Path,
    },
    Word {
        name: "html",
        help: "cmd.shot.html",
        needs: &[],
        then: Args::Path,
    },
    Word {
        name: "txt",
        help: "cmd.shot.txt",
        needs: &[],
        then: Args::Path,
    },
];

/// The pieces of Markdown `:markdown` can write.
const MARKDOWN_BITS: &[Word] = &[
    Word {
        name: "footnote",
        help: "cmd.markdown-bits.footnote",
        needs: &[],
        then: Args::Words(FOOTNOTE_KINDS),
    },
];

/// …and the one word a footnote takes.
const FOOTNOTE_KINDS: &[Word] = &[Word {
    name: "inline",
    help: "cmd.footnote-kinds.inline",
    needs: &[],
    then: Args::None,
}];

/// A piece of Markdown the editor can write for you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownBit {
    /// `[^3]` here, `[^3]: ` at the foot, the cursor in the note.
    Footnote,
    /// `^[…]`, the cursor inside the brackets.
    InlineNote,
}

/// The sections `:help` offers.
const HELP_SECTIONS: &[Word] = &[
    Word { name: "chinese", help: "help.chinese.section-summary", then: Args::None, needs: &[] },
    Word { name: "vertical", help: "help.vertical.title", then: Args::None, needs: &[] },
    Word { name: "table", help: "help.table.section-summary", then: Args::None, needs: &[] },
    Word { name: "commands", help: "help.common.every-command", then: Args::None, needs: &[] },
];

/// One word a command accepts, and what may follow *it*.
pub struct Word {
    pub name: &'static str,
    pub help: &'static str,
    pub then: Args,
    /// What has to be true before this word does anything (Feature #170).
    pub needs: &'static [Need],
}

/// A state a command needs before it can do what it says.
///
/// **The editor used to answer these with silence.** `:hanging on` on a
/// horizontal page set a flag that nothing read: the setting was on, the page
/// did not change, and there was nowhere to find out why. A prerequisite is
/// part of what a command *is*, so it is declared beside it — and then it can
/// be shown in the menu before the command is run, said plainly when it is,
/// and satisfied on request with `force`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// The page runs 縱書.
    Vertical,
    /// The page is not packed — 密排 drops the margin this needs.
    Loose,
    /// A table is open.
    Table,
    /// A 碼表 has been loaded.
    Scheme,
}

impl Need {
    /// What it is, in a sentence a status line can hold.
    pub fn says(self) -> String {
        match self {
            Need::Vertical => say!("need.vertical"),
            Need::Loose => say!("need.loose"),
            Need::Table => say!("need.table"),
            Need::Scheme => say!("need.scheme"),
        }
    }

    /// The command that brings it about, for the message and for `force`.
    pub fn how(self) -> &'static str {
        match self {
            Need::Vertical => ":layout vertical",
            Need::Loose => ":dense off",
            Need::Table => ":table",
            Need::Scheme => ":yume scheme",
        }
    }
}

/// The words after `command`, each written out in full, or `None` if they
/// already were.
///
/// Walks the same word lists the menu shows, so the abbreviation it offers is
/// the abbreviation that parses — one rule rather than a promise the parser
/// has to remember to keep. The walk stops the moment the list runs out (a
/// path, free text, a delimiter, a number), so nothing a writer typed for
/// themselves is ever rewritten: `:table pipe " "` still hands `" "` through
/// untouched, and an unresolvable word is left exactly as it stands for the
/// arm below to refuse by its own name.
fn spell_out(command: &str, rest: &str) -> Option<String> {
    let entry = COMMANDS.iter().find(|e| {
        let named = |w: &str| e.name == w || e.aliases.contains(&w);
        named(command) || command.strip_suffix('!').is_some_and(named)
    })?;
    let mut args = &entry.args;
    let mut out = String::with_capacity(rest.len());
    let mut left = rest;
    let mut changed = false;
    while let Some(list) = args.words() {
        let head = left.trim_start();
        if head.is_empty() {
            break;
        }
        let (head, tail) = match head.split_once(char::is_whitespace) {
            Some(split) => split,
            None => (head, ""),
        };
        let Some(found) = pick(head, list) else { break };
        changed |= found.name != head;
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(found.name);
        args = &found.then;
        left = tail;
    }
    if !changed {
        return None;
    }
    let left = left.trim_start();
    if !left.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(left);
    }
    Some(out)
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
    // **`!` composes with the prefix rule.** `:expo html` resolved and
    // `:exp! html` did not, so the shorthand a writer had settled into stopped
    // working at exactly the moment they meant「yes, overwrite it」. The bang
    // belongs to the command, not to its spelling.
    if let Some(stem) = word.strip_suffix('!') {
        // **An exact spelling that takes no bang is not a prefix of one.**
        // `e` and `o` are `open`'s own aliases and `:e!` / `:o!` are retired
        // outright (see `parse`); because `open` is not forceable they fell
        // through to the walk below and came out as `export!`, so `:e!
        // 第三章.md` — typed by a hand meaning「re-read it」 — wrote an export
        // over the chapter.
        if COMMANDS
            .iter()
            .any(|e| (e.name == stem || e.aliases.contains(&stem)) && forceable(e.name).is_none())
        {
            return word;
        }
        let mut banged = COMMANDS.iter().filter_map(|e| {
            let named = e.name.starts_with(stem) || e.aliases.iter().any(|a| a.starts_with(stem));
            named.then(|| forceable(e.name)).flatten()
        });
        if let (Some(only), None) = (banged.next(), banged.next()) {
            return only;
        }
        return word;
    }
    let mut hits = COMMANDS.iter().filter(|e| e.name.starts_with(word));
    match (hits.next(), hits.next()) {
        (Some(only), None) => only.name,
        _ => word,
    }
}

/// What `:export` writes, and what follows the format — **the file name comes
/// second** (§5.2.2 fault 7).
///
/// The table used to declare `Args::Free("<檔名>")` while `parse` read the
/// format first, so `:export 第三章.md` was answered 「沒有『第三章.md』這種
/// 格式」 by a prompt that had just asked for a file name. And because a free
/// argument has no words in it, `html` `typst` `csv` `tsv` were the only words
/// this editor accepts that appeared nowhere in `::`'s corpus: four names you
/// had to already know to find.
const EXPORT_FORMATS: &[Word] = &[
    Word {
        name: "html",
        help: "cmd.export.html",
        needs: &[],
        then: Args::Free("<檔名>"),
    },
    Word {
        name: "typst",
        help: "cmd.export.typst",
        needs: &[],
        then: Args::Free("<檔名>"),
    },
    // A grid, not a page: these come from the table under the cursor. See
    // [`crate::export::delimiter_of`] for why they are not `Format` variants.
    //
    // **No `Need::Table`**, although one would parse: away from a table the
    // export already answers with the row it wanted and the `|` it was looking
    // for, and a prerequisite would replace that with 「需要：表格模式——句末加
    // force 一併打開」 — an offer to *open* a table where there is none.
    Word {
        name: "csv",
        help: "cmd.export.csv",
        needs: &[],
        then: Args::Free("<檔名>"),
    },
    Word {
        name: "tsv",
        help: "cmd.export.tsv",
        needs: &[],
        then: Args::Free("<檔名>"),
    },
];

/// The commands that take a `!`, so a prefix of one can too — spelled the way
/// `parse` reads them back, because that spelling is the other half of the
/// answer and a second list of it goes stale.
///
/// It did: `recover` was in neither, so `:recover!` worked and `:rec!`
/// `:recov!` `:recove!` were all 「沒有這個命令」 while `:rec` was fine. One
/// list cannot drift from itself.
const FORCEABLE: &[&str] = &[
    "bclose!",
    "quitall!",
    "write!",
    "reload!",
    "quit!",
    "export!",
    "saveas!",
    "replace!",
    "recover!",
    "shot!",
];

/// The banged spelling of `name`, if the command takes a bang at all.
fn forceable(name: &str) -> Option<&'static str> {
    FORCEABLE
        .iter()
        .copied()
        .find(|banged| banged.strip_suffix('!') == Some(name))
}

/// The character a `<分隔>` argument names.
///
/// A delimiter is one character, and most of them can simply be typed. The two
/// that cannot are the tab — which the command line would never see, because
/// `Tab` completes — and the space, which is spelled out for the same reason
/// and is accepted here even though the sniffer will never guess it: a writer
/// who says `:table pipe " "` has looked at their data and decided.
fn delimiter_named(word: &str) -> Option<char> {
    match word {
        "tab" | "\\t" => Some('\t'),
        "space" | "\\s" => Some(' '),
        "comma" => Some(','),
        "semicolon" => Some(';'),
        "\" \"" | "' '" => Some(' '),
        _ => {
            let mut chars = word.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_whitespace() => Some(c),
                _ => None,
            }
        }
    }
}

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
    for (at, _) in name.char_indices().skip(1) {
        if !others.iter().any(|o| o.starts_with(&name[..at])) {
            return Some(&name[..at]);
        }
    }
    None
}

/// The one word of `from` that `typed` names, exactly or by prefix.
///
/// `:yume s l` is `:yume scheme lingming` — because `s` is the only word there
/// starting with `s`, and `l` the only scheme starting with `l`. An ambiguous
/// prefix names nothing rather than guessing: `:ruby t` could be `typst` and
/// nothing else, but if a second `t` word were ever added it would stop
/// working, loudly, instead of quietly meaning the older one.
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

/// The command a head word names — **by the parser's rule, not a stricter one**.
///
/// `resolve` for the prefix, `forceable` for the bang, and only then the
/// table. Four readers of `COMMANDS` had spelled this walk out by hand and two
/// of them stopped at the exact name, so `:tab rules` **ran** while `:tab `
/// drew nothing: the menu vanished on the abbreviation it had printed itself
/// (§5.2.2 fault 1).
///
/// The bang belongs to the command, not to its spelling — without it, the line
/// where a path is *most* likely to be Chinese, `:w! 第三章.md`, the one you
/// type because the file is already there, was the one line that refused the
/// IME. How far the bang is followed is the comment below.
fn entry_named(head: &str) -> Option<&'static Entry> {
    let spelled = |word: &str| {
        COMMANDS
            .iter()
            .find(|e| e.name == word || e.aliases.contains(&word))
    };
    let head = resolve(head);
    if let Some(entry) = spelled(head) {
        return Some(entry);
    }
    // A banged spelling `resolve` could not settle on its own. It hands back
    // `write!` for `:w!`, but leaves `:q!` exactly as it found it, because two
    // commands begin with `q` and only their **banged** spellings tell them
    // apart — which is what `parse` matches on and what a hand meaning「不存了」
    // actually types. So the stem is resolved in its own right.
    //
    // **Only where the bang is really this command's**: `:o!` and `:e!` are
    // retired outright, and an unconditional strip resurrected them — `:o!
    // 第三章.md` became `open` and was offered the IME for a line that can only
    // end in an error.
    let entry = spelled(resolve(head.strip_suffix('!')?))?;
    forceable(entry.name).map(|_| entry)
}

/// A word offered by completion, whether a command or an argument.
#[derive(Debug, Clone)]
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
    /// The words this one lives under, when it is being shown *beside* its
    /// parent rather than after it — `yume`, for the `scheme` in `:yume`'s
    /// list. Empty for everything else.
    ///
    /// A `String` rather than a `&'static str` because a deep match (#223)
    /// carries a whole path: `:lingming` is answered with `yume scheme
    /// lingming`, and `yume scheme` is two words that exist separately in the
    /// table and nowhere together.
    pub under: String,
    /// What has to be true before it does anything.
    pub needs: &'static [Need],
}

impl Choice {
    /// What Tab writes for this choice, and what the menu shows.
    ///
    /// A promoted child carries its parent with it: picking `scheme` out of
    /// `:yume`'s list has to leave `:yume scheme` on the line, not `:scheme`.
    pub fn written(&self) -> String {
        match self.under.is_empty() {
            true => self.name.to_string(),
            false => format!("{} {}", self.under, self.name),
        }
    }
}

/// The words a command takes, shown under the command itself.
///
/// Typing `:yume` used to answer with one entry — `:yume` — and a reader had
/// no way to find out from there that the input method's whole set of commands
/// lives under that word. The list says so: the command, and then everything
/// it takes, spelled the way you would type it.
fn children(under: &'static str, args: &Args) -> Vec<Choice> {
    match args.words() {
        Some(list) => list
            .iter()
            .map(|w| Choice {
                name: w.name,
                needs: w.needs,
                alias: None,
                // The short form of a word is only short beside its siblings;
                // spelled out under its parent it would be a second way to
                // read the same row.
                short: None,
                help: w.help,
                leading: "",
                under: under.to_string(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Every word deeper in the tree whose name starts with `typed`, carrying the
/// path that has to be typed to reach it (#223).
///
/// The fallback for a word that names nothing at the depth it was typed at.
/// `:vert` is not a command — but `vertical` is a word `:layout` takes, and
/// that is what the reader who typed it meant. The tree is already walked to
/// offer children; this walks the rest of it.
///
/// **Only when nothing matched at this depth.** `:t` is a question about
/// `:table`, and answering it with a dozen grandchildren called `top` and
/// `tight` would bury the answer under the guesses.
fn deep(
    list: &'static [Word],
    path: &[&'static str],
    typed: &str,
    leading: &'static str,
    out: &mut Vec<Choice>,
) {
    for w in list {
        if w.name.starts_with(typed) {
            out.push(Choice {
                name: w.name,
                needs: w.needs,
                alias: None,
                // Same reason as `children`: a short form is only short beside
                // its siblings, and this row is being read beside its path.
                short: None,
                help: w.help,
                leading,
                under: path.join(" "),
            });
        }
        if let Some(inner) = w.then.words() {
            let mut below = path.to_vec();
            below.push(w.name);
            deep(inner, &below, typed, leading, out);
        }
    }
}

/// The same, from the top: every subcommand of every command.
///
/// Sorted shallowest first, so `:layout vertical` is offered before something
/// three words down that happens to share the prefix.
///
/// **`:help`'s own words go last.** Every one of them is the name of something
/// else — that is what a help topic *is* — so in a deep match they shadow the
/// thing itself: `:vert` would be answered 「read about 竪排」 before 「switch
/// to 竪排」. The reader who typed the name of a thing meant the thing.
fn deep_from_root(typed: &str) -> Vec<Choice> {
    let mut out = Vec::new();
    for e in COMMANDS {
        if let Some(list) = e.args.words() {
            deep(list, &[e.name], typed, ":", &mut out);
        }
    }
    let about = |c: &Choice| c.under.starts_with("help");
    out.sort_by(|a, b| {
        (about(a), a.under.split(' ').count(), &a.under, a.name)
            .cmp(&(about(b), b.under.split(' ').count(), &b.under, b.name))
    });
    out
}

/// `:yume` and what may follow it — the input method's own commands, under
/// the one word a reader would think of when looking for them.
const YUME: &[Word] = &[
    Word {
        name: "scheme",
        help: "cmd.yume.scheme",
        needs: &[],
        then: Args::Schemes,
    },
    Word {
        name: "chaifen",
        help: "cmd.yume.chaifen",
        needs: &[Need::Scheme],
        then: Args::Words(ON_OFF),
    },
    Word {
        name: "commit",
        help: "cmd.yume.commit",
        needs: &[Need::Scheme],
        then: Args::Words(COMMITS),
    },
    Word {
        name: "panel",
        help: "cmd.yume.panel",
        needs: &[Need::Scheme],
        then: Args::Words(PANELS),
    },
    Word {
        name: "which",
        help: "cmd.yume.which",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "on",
        help: "cmd.yume.on",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "off",
        help: "cmd.yume.off",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "installed",
        help: "cmd.yume.installed",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "builtin",
        help: "cmd.yume.builtin",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "table",
        help: "cmd.yume.table",
        needs: &[],
        then: Args::Path,
    },
];

/// Dark, light, or whichever the terminal is.
///
/// Not a colour and not a theme: the same three anchors read one way on a dark
/// ground and another on a light one, so this says which of the two the theme
/// is being asked for — and `System` says *don't ask me, ask the terminal*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    /// Whatever the terminal answered when it was asked its background.
    System,
    Dark,
    Light,
}

/// Which way a search runs.
const AXIS: &[Word] = &[
    Word {
        name: "row",
        help: "cmd.axis.row",
        needs: &[],
        then: Args::Free("<正則>"),
    },
    Word {
        name: "column",
        help: "cmd.axis.column",
        needs: &[],
        then: Args::Free("<正則>"),
    },
];

/// What `:table` takes.
const TABLE: &[Word] = &[
    Word {
        name: "off",
        help: "cmd.table.off",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "basic",
        help: "cmd.table.basic",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "full",
        help: "cmd.table.full",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "new",
        help: "cmd.table.new",
        needs: &[],
        then: Args::Free("<行> <欄>"),
    },
    Word {
        name: "check",
        help: "cmd.table.check",
        needs: &[Need::Table],
        then: Args::None,
    },
    Word {
        name: "rules",
        help: "cmd.table.rules",
        needs: &[Need::Table],
        then: Args::Words(RULES),
    },
    Word {
        name: "sort",
        help: "cmd.table.sort",
        needs: &[Need::Table],
        then: Args::Free("<欄> a｜d …"),
    },
    Word {
        name: "detail",
        help: "cmd.table.detail",
        needs: &[Need::Table],
        then: Args::Free("on｜off｜<寬度>"),
    },
    Word {
        name: "numbers",
        help: "cmd.table.numbers",
        needs: &[Need::Table],
        then: Args::Words(ON_OFF),
    },
    Word {
        name: "header",
        help: "cmd.table.header",
        needs: &[Need::Table],
        then: Args::Words(ON_OFF),
    },
    Word {
        name: "schema",
        help: "cmd.table.schema",
        needs: &[Need::Table],
        then: Args::None,
    },
    // Neither of these needs a table to be *open*: turning a block of text into
    // one is how you get a table in the first place.
    Word {
        name: "pipe",
        help: "cmd.table.pipe",
        needs: &[],
        then: Args::Free("<分隔>"),
    },
    Word {
        name: "csv",
        help: "cmd.table.csv",
        needs: &[],
        then: Args::Free("<分隔>"),
    },
];

/// How much of the result the page shows.
/// How loudly the editor talks beside the caret (Feature #284).
const HUD: &[Word] = &[
    Word {
        name: "off",
        help: "cmd.hud.off",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "basic",
        help: "cmd.hud.basic",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "full",
        help: "cmd.hud.full",
        needs: &[],
        then: Args::None,
    },
];

const RENDER: &[Word] = &[
    Word {
        name: "off",
        help: "cmd.render.off",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "basic",
        help: "cmd.render.basic",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "full",
        help: "cmd.render.full",
        needs: &[],
        then: Args::None,
    },
];

/// What `:clipboard` does — the system one, not a register.
const CLIPBOARD: &[Word] = &[
    Word {
        name: "yank",
        help: "cmd.clipboard.yank",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "paste",
        help: "cmd.clipboard.paste",
        needs: &[],
        then: Args::None,
    },
];

/// What `:buffer` does.
const BUFFERS: &[Word] = &[
    Word {
        name: "list",
        help: "cmd.buffers.list",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "next",
        help: "cmd.buffers.next",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "previous",
        help: "cmd.buffers.previous",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "close",
        help: "cmd.buffers.close",
        needs: &[],
        then: Args::None,
    },
];

/// What `:wrap` takes.
const WRAP: &[Word] = &[
    Word {
        name: "on",
        help: "cmd.wrap.on",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "off",
        help: "cmd.wrap.off",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "0",
        help: "cmd.wrap.0",
        needs: &[],
        then: Args::None,
    },
];

/// The input schemes yume ships with.
/// The three 上屏方式 — *when* a finished code goes to the page.
///
/// The names are yume's own tags (`CommitStrategy::from_str_tag`), so that a
/// setting written here means the same thing in the input method's own panel
/// on macOS and Windows. `auto` is taken as 唯一 because that is what the
/// habit calls it — 自動上屏.
const COMMITS: &[Word] = &[
    Word {
        name: "delayed",
        help: "cmd.commits.delayed",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "unique",
        help: "cmd.commits.unique",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "fluency",
        help: "cmd.commits.fluency",
        needs: &[],
        then: Args::None,
    },
];

const PANELS: &[Word] = &[
    Word {
        name: "full",
        help: "cmd.panels.full",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "bare",
        help: "cmd.panels.bare",
        needs: &[],
        then: Args::None,
    },
];

const SCHEMES: &[Word] = &[
    Word {
        name: "lingming",
        help: "cmd.schemes.lingming",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "xingchen",
        help: "cmd.schemes.xingchen",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "qingyun",
        help: "cmd.schemes.qingyun",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "riyue",
        help: "cmd.schemes.riyue",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "pinyin",
        help: "cmd.schemes.pinyin",
        needs: &[],
        then: Args::None,
    },
];

/// The languages a file may be read as.
const SYNTAXES: &[Word] = &[
    Word {
        name: "markdown",
        help: "cmd.syntaxes.markdown",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "typst",
        help: "cmd.syntaxes.typst",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "text",
        help: "cmd.syntaxes.text",
        needs: &[],
        then: Args::None,
    },
];

/// Which way the page runs.
/// `:theme` and what may follow it.
///
/// The mood words sit at both levels, so `:theme dark` and `:theme moxiang
/// dark` are both sentences — the theme's name is worth saying and worth
/// leaving out, and neither should be a special case.
const THEMES: &[Word] = &[
    Word {
        // 墨香
        name: "ink",
        help: "cmd.themes.ink",
        needs: &[],
        then: Args::Words(MOODS),
    },
    Word {
        // 黑白
        name: "bw",
        help: "cmd.themes.bw",
        needs: &[],
        then: Args::Words(MOODS),
    },
    Word {
        // 藍曬
        name: "cyanotype",
        help: "cmd.themes.cyanotype",
        needs: &[],
        then: Args::Words(MOODS),
    },
    Word {
        // 琥珀
        name: "amber",
        help: "cmd.themes.amber",
        needs: &[],
        then: Args::Words(MOODS),
    },
    Word {
        // 莫高
        name: "mogao",
        help: "cmd.themes.mogao",
        needs: &[],
        then: Args::Words(MOODS),
    },
    Word {
        // 莫蘭迪
        name: "morandi",
        help: "cmd.themes.morandi",
        needs: &[],
        then: Args::Words(MOODS),
    },
    Word {
        // 夜螢
        name: "firefly",
        help: "cmd.themes.firefly",
        needs: &[],
        then: Args::Words(MOODS),
    },
    Word {
        // 明度階
        name: "meridian",
        help: "cmd.themes.meridian",
        needs: &[],
        then: Args::Words(MOODS),
    },
    Word {
        // 陶窯
        name: "kiln",
        help: "cmd.themes.kiln",
        needs: &[],
        then: Args::Words(MOODS),
    },
    Word {
        // 靛橘
        name: "complement",
        help: "cmd.themes.complement",
        needs: &[],
        then: Args::Words(MOODS),
    },
];

/// `:indent` and what may follow it.
const INDENT: &[Word] = &[
    Word {
        name: "off",
        help: "cmd.indent.off",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "basic",
        help: "cmd.indent.basic",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "full",
        help: "cmd.indent.full",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "hint",
        help: "cmd.indent.hint",
        needs: &[],
        then: Args::Words(HINTS),
    },
];

/// What is drawn in a paragraph's opening squares.
const HINTS: &[Word] = &[
    Word {
        name: "none",
        help: "cmd.hints.none",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "color",
        help: "cmd.hints.color",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "symbol",
        help: "cmd.hints.symbol",
        needs: &[],
        then: Args::None,
    },
];

/// `:numbers` and what may follow it.
const NUMBERS: &[Word] = &[
    Word {
        name: "fill",
        help: "cmd.numbers.fill",
        needs: &[],
        then: Args::Words(ON_OFF),
    },
];

/// How a table's columns are told apart.
const RULES: &[Word] = &[
    Word {
        name: "off",
        help: "cmd.rules.off",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "color",
        help: "cmd.rules.color",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "line",
        help: "cmd.rules.line",
        needs: &[],
        then: Args::Words(STROKES),
    },
];

/// Which line `:table rules line` draws.
const STROKES: &[Word] = &[
    Word {
        name: "solid",
        help: "cmd.strokes.solid",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "dash",
        help: "cmd.strokes.dash",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "double",
        help: "cmd.strokes.double",
        needs: &[],
        then: Args::None,
    },
];

/// Dark, light, or the terminal's answer.
///
/// **A theme and an appearance are two questions**, and mixing them in one
/// list made `:theme` offer 「moxiang、heibai、system、dark、light」 as though
/// they were five of a kind. They are not: one says which set of inks, the
/// other says which way round they go. `:appearance` asks the second on its
/// own, and `:theme moxiang dark` still asks both in one line.
const MOODS: &[Word] = &[
    Word {
        name: "system",
        help: "cmd.moods.system",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "dark",
        help: "cmd.moods.dark",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "light",
        help: "cmd.moods.light",
        needs: &[],
        then: Args::None,
    },
];

const LAYOUTS: &[Word] = &[
    Word {
        name: "vertical",
        help: "cmd.layouts.vertical",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "horizontal",
        help: "cmd.layouts.horizontal",
        needs: &[],
        then: Args::None,
    },
];

/// `:ruby` and what may follow it — the four `render-ruby-*` and
/// `format-ruby-*` commands, folded into the one word a reader remembers.
const RUBY: &[Word] = &[
    Word {
        name: "off",
        help: "cmd.ruby.off",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "basic",
        help: "cmd.ruby.basic",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "full",
        help: "cmd.ruby.full",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "auto",
        help: "cmd.ruby.auto",
        needs: &[],
        then: Args::Words(&[Word {
            name: "rare",
            help: "cmd.ruby.auto.rare",
            needs: &[],
            then: Args::None,
        }]),
    },
    Word {
        name: "html",
        help: "cmd.ruby.html",
        needs: &[],
        then: Args::Words(ON_OFF),
    },
    Word {
        name: "typst",
        help: "cmd.ruby.typst",
        needs: &[],
        then: Args::Words(ON_OFF),
    },
    Word {
        name: "format",
        help: "cmd.ruby.format",
        needs: &[],
        then: Args::Words(&[
            Word {
                name: "html",
                help: "cmd.ruby.format.html",
                needs: &[],
        then: Args::None,
            },
            Word {
                name: "typst",
                help: "cmd.ruby.format.typst",
                needs: &[],
        then: Args::None,
            },
        ]),
    },
];

/// `:word` — one subject, three sides of it.
const WORD_TOPICS: &[Word] = &[
    Word {
        name: "show",
        help: "cmd.word-topics.show",
        needs: &[],
        then: Args::Words(WORD_SHOW),
    },
    Word {
        name: "list",
        help: "cmd.word-topics.list",
        needs: &[],
        then: Args::Words(WORD_LISTS),
    },
    Word {
        name: "level",
        help: "cmd.word-topics.level",
        needs: &[],
        then: Args::Words(WORD_LEVELS),
    },
    Word {
        name: "discover",
        help: "cmd.word-topics.discover",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "habit",
        help: "cmd.word-topics.habit",
        needs: &[],
        then: Args::None,
    },
];

/// What `:word show` may be given — **four words, not two**.
///
/// The parser has always taken `tint` and `ink` here (and 底色／字色, which
/// `WordMark::parse` reads), while the table declared `ON_OFF`: so the two
/// drawings ran, and the menu that exists to say what may follow `show` never
/// mentioned them. Same shape as §5.2.2 fault 7 — a word the editor accepts
/// and no reader can find is a word only its author has.
const WORD_SHOW: &[Word] = &[
    Word {
        name: "on",
        help: "cmd.on-off.on",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "off",
        help: "hint.close",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "tint",
        help: "cmd.word-show.tint",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "ink",
        help: "cmd.word-show.ink",
        needs: &[],
        then: Args::None,
    },
];

const WORD_LISTS: &[Word] = &[
    Word {
        name: "reload",
        help: "cmd.word-lists.reload",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "edit",
        help: "cmd.word-lists.edit",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "global",
        help: "cmd.word-lists.global",
        needs: &[],
        then: Args::None,
    },
];

const WORD_LEVELS: &[Word] = &[
    Word {
        name: "strict",
        help: "cmd.word-levels.strict",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "balanced",
        help: "cmd.word-levels.balanced",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "full",
        help: "cmd.word-levels.full",
        needs: &[],
        then: Args::None,
    },
];

/// What `:check` can be asked to look over (Feature #233).
///
/// Three words, and 空格 is the fourth. This is why `:check` took a word list
/// from the first day it had one word: a command that had taken nothing at all
/// would have had to be re-shaped here, and every `:check` anybody had typed
/// would have changed meaning under them.
const CHECK: &[Word] = &[
    Word {
        name: "usage",
        help: "cmd.check.usage",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "charset",
        help: "cmd.check.charset",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "punct",
        help: "cmd.check.punct",
        needs: &[],
        then: Args::None,
    },
];

/// What `:convert opencc` can be asked to do.
const OPENCC: &[Word] = &[
    Word {
        name: "install",
        help: "cmd.opencc.install",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "update",
        help: "cmd.opencc.update",
        needs: &[],
        then: Args::None,
    },
];

/// The side `:convert` starts from.
///
/// The placeholder after each one lists **that side's** destinations rather
/// than all seven: opencc has `s2tw` and `tw2s` but no `tw2hk`, and a menu that
/// offered every pair would be offering combinations that fail at the command
/// line. 日本新字体 only goes back to 繁體, so it says so.
const CONVERT: &[Word] = &[
    Word {
        name: "s",
        help: "cmd.convert.s",
        needs: &[],
        then: Args::Free("t｜tw｜hk｜c｜g [force]"),
    },
    Word {
        name: "t",
        help: "cmd.convert.t",
        needs: &[],
        then: Args::Free("s｜tw｜hk｜jp｜c｜g"),
    },
    Word {
        name: "tw",
        help: "cmd.convert.tw",
        needs: &[],
        then: Args::Free("s｜t｜c｜g [force]"),
    },
    Word {
        name: "hk",
        help: "cmd.convert.hk",
        needs: &[],
        then: Args::Free("s｜t｜c｜g [force]"),
    },
    Word {
        name: "jp",
        help: "cmd.convert.jp",
        needs: &[],
        then: Args::Free("t｜c｜g"),
    },
    Word {
        name: "c",
        help: "cmd.convert.c",
        needs: &[],
        then: Args::Free("s｜t｜tw｜hk｜jp｜g"),
    },
    Word {
        name: "g",
        help: "cmd.convert.g",
        needs: &[],
        then: Args::Free("s｜t｜tw｜hk｜jp｜c"),
    },
    Word {
        name: "opencc",
        help: "cmd.convert.opencc",
        needs: &[],
        then: Args::Words(OPENCC),
    },
];

/// The one word `:reload` takes besides nothing at all.
const RELOAD: &[Word] = &[Word {
    name: "auto",
    help: "cmd.reload.auto",
    needs: &[],
    then: Args::Words(ON_OFF),
}];

/// What a word means to a command that offers 「on｜off」.
///
/// A function, because **the reading is the half that goes stale**. Ten arms
/// had written it out by hand and the eleventh forgot: `:hanging` declares
/// `Args::Words(ON_OFF)`, the hint prints `on｜off`, the menu offers both —
/// and `parse` was `"hanging" => Ok(Command::ToggleHanging)` with `rest` never
/// read, so `:hanging off` turned hanging punctuation **on** (§5.2.2 fault 2).
/// `every_listed_command_parses` could not see it: `:hanging off` *parses*. It
/// simply did not listen.
///
/// `pick` rather than an equality test, because that is the rule everywhere
/// else — the menu shows `of` as `off`'s shortest spelling, so `of` has to
/// mean it. **What a missing word means stays with the caller**: `:readonly`
/// on its own toggles, `:dense` on its own is 密排, and neither is this
/// function's to decide.
fn switch(command: &'static str, word: &str) -> Result<bool, CommandError> {
    match pick(word, ON_OFF).map(|w| w.name) {
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        _ => Err(CommandError::InvalidArgument {
            command,
            value: word.to_string(),
        }),
    }
}

/// The two words every switch takes — read back by [`switch`].
const ON_OFF: &[Word] = &[
    Word {
        name: "on",
        help: "cmd.on-off.on",
        needs: &[],
        then: Args::None,
    },
    Word {
        name: "off",
        help: "hint.close",
        needs: &[],
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
        help: "cmd.commands.open",
        needs: &[],
        args: Args::Path,
    },
    Entry {
        name: "new",
        aliases: &["enew"],
        help: "cmd.commands.new",
        needs: &[],
        args: Args::Path,
    },
    Entry {
        name: "write",
        aliases: &["w"],
        help: "cmd.commands.write",
        needs: &[],
        args: Args::Path,
    },
    Entry {
        name: "wq",
        aliases: &["x"],
        help: "cmd.commands.wq",
        needs: &[],
        args: Args::Path,
    },
    Entry {
        name: "recover",
        aliases: &[],
        help: "cmd.commands.recover",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "reload",
        aliases: &[],
        help: "cmd.commands.reload",
        needs: &[],
        args: Args::Words(RELOAD),
    },
    Entry {
        name: "readonly",
        aliases: &["ro"],
        help: "cmd.commands.readonly",
        needs: &[],
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "goto",
        aliases: &["g"],
        help: "cmd.commands.goto",
        needs: &[],
        args: Args::Free("<行號>"),
    },
    Entry {
        name: "count",
        aliases: &["wc"],
        help: "cmd.commands.count",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "check",
        aliases: &[],
        help: "cmd.commands.check",
        needs: &[],
        args: Args::Words(CHECK),
    },
    Entry {
        name: "convert",
        aliases: &[],
        help: "cmd.commands.convert",
        needs: &[],
        args: Args::Words(CONVERT),
    },
    Entry {
        name: "quit",
        aliases: &["q"],
        help: "cmd.commands.quit",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "quitall",
        aliases: &["qa"],
        help: "cmd.commands.quitall",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "undo",
        aliases: &["u"],
        help: "cmd.commands.undo",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "redo",
        aliases: &["red"],
        help: "cmd.commands.redo",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "word",
        aliases: &["wd"],
        help: "cmd.commands.word",
        needs: &[],
        args: Args::Words(WORD_TOPICS),
    },
    Entry {
        name: "progress",
        aliases: &["prog"],
        help: "cmd.commands.progress",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "target",
        aliases: &[],
        help: "cmd.commands.target",
        needs: &[],
        args: Args::Free("<字數>｜off"),
    },
    Entry {
        name: "layout",
        aliases: &["lay"],
        help: "cmd.commands.layout",
        needs: &[],
        args: Args::Words(LAYOUTS),
    },
    Entry {
        name: "theme",
        aliases: &[],
        help: "cmd.commands.theme",
        needs: &[],
        args: Args::Words(THEMES),
    },
    Entry {
        name: "numbers",
        aliases: &[],
        help: "cmd.commands.numbers",
        needs: &[],
        args: Args::Words(NUMBERS),
    },
    Entry {
        name: "shot",
        aliases: &[],
        help: "cmd.commands.shot",
        needs: &[],
        args: Args::Words(SHOT),
    },
    Entry {
        name: "appearance",
        aliases: &[],
        help: "cmd.commands.appearance",
        needs: &[],
        args: Args::Words(MOODS),
    },
    Entry {
        name: "yume",
        aliases: &[],
        help: "cmd.commands.yume",
        needs: &[],
        args: Args::Words(YUME),
    },
    Entry {
        name: "hanging",
        aliases: &[],
        help: "cmd.commands.hanging",
        needs: &[Need::Vertical, Need::Loose],
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "syntax",
        aliases: &["syn"],
        help: "cmd.commands.syntax",
        needs: &[],
        args: Args::Words(SYNTAXES),
    },
    Entry {
        name: "pipe",
        aliases: &[],
        help: "cmd.commands.pipe",
        needs: &[],
        args: Args::Free("<命令>"),
    },
    Entry {
        name: "sh",
        aliases: &[],
        help: "cmd.commands.sh",
        needs: &[],
        args: Args::Free("<命令>"),
    },
    Entry {
        name: "!command",
        aliases: &[],
        help: "cmd.commands.shell",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "preview",
        aliases: &[],
        help: "cmd.commands.preview",
        needs: &[],
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "render",
        aliases: &[],
        help: "cmd.commands.render",
        needs: &[],
        args: Args::Words(RENDER),
    },
    Entry {
        name: "hud",
        aliases: &[],
        help: "cmd.commands.hud",
        needs: &[],
        args: Args::Words(HUD),
    },
    Entry {
        name: "search",
        aliases: &[],
        help: "cmd.commands.search",
        needs: &[],
        args: Args::Words(AXIS),
    },
    Entry {
        name: "bands",
        aliases: &[],
        help: "cmd.commands.bands",
        needs: &[Need::Vertical],
        args: Args::Free("<幾條，1–4；不寫就是 2>"),
    },
    Entry {
        name: "sentence",
        aliases: &[],
        help: "cmd.commands.sentence",
        // 竪排 only: a 句 gets a 縱 of its own, and 橫排 has no 縱. `:sentence!`
        // turns the page for you rather than refusing.
        needs: &[Need::Vertical],
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "indent",
        aliases: &[],
        help: "cmd.commands.indent",
        needs: &[],
        args: Args::Words(INDENT),
    },
    Entry {
        name: "dense",
        aliases: &[],
        help: "cmd.commands.dense",
        // **Both layouts**: 密排 packs a 縱書 page by taking the gap between
        // 縱 away, and packs a 橫排 page by taking the row between rows away.
        // It used to need 竪排, which made 「疏排」 unsayable on the page most
        // writing is done on.
        needs: &[],
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "table",
        aliases: &[],
        help: "cmd.commands.table",
        needs: &[],
        args: Args::Words(TABLE),
    },
    Entry {
        name: "wrap",
        aliases: &[],
        help: "cmd.commands.wrap",
        needs: &[],
        args: Args::Words(WRAP),
    },
    Entry {
        name: "wheel",
        aliases: &[],
        help: "cmd.commands.wheel",
        needs: &[],
        args: Args::Free("<格數>"),
    },
    Entry {
        name: "clipboard",
        aliases: &[],
        help: "cmd.commands.clipboard",
        needs: &[],
        args: Args::Words(CLIPBOARD),
    },
    Entry {
        name: "buffer",
        aliases: &[],
        help: "cmd.commands.buffer",
        needs: &[],
        args: Args::Words(BUFFERS),
    },
    Entry {
        name: "bclose",
        aliases: &["bc"],
        help: "cmd.commands.bclose",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "format",
        aliases: &["fmt"],
        help: "cmd.commands.format",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "run",
        aliases: &[],
        help: "cmd.commands.run",
        needs: &[],
        args: Args::Free("<名字>"),
    },
    Entry {
        name: "markdown",
        aliases: &["md"],
        help: "cmd.commands.markdown",
        needs: &[],
        args: Args::Words(MARKDOWN_BITS),
    },
    Entry {
        name: "typewriter",
        aliases: &[],
        help: "cmd.commands.typewriter",
        needs: &[],
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "focus",
        aliases: &[],
        help: "cmd.commands.focus",
        needs: &[],
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "meter",
        aliases: &[],
        help: "cmd.commands.meter",
        needs: &[],
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "note",
        aliases: &[],
        help: "cmd.commands.note",
        needs: &[],
        args: Args::Words(ON_OFF),
    },
    Entry {
        name: "tutor",
        aliases: &[],
        help: "cmd.commands.tutor",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "help",
        aliases: &[],
        help: "cmd.commands.help",
        needs: &[],
        args: Args::Words(HELP_SECTIONS),
    },
    Entry {
        name: "saveas",
        aliases: &["sav"],
        help: "cmd.commands.saveas",
        needs: &[],
        args: Args::Free("<檔名>"),
    },
    Entry {
        name: "export",
        aliases: &["ex"],
        help: "cmd.commands.export",
        needs: &[],
        args: Args::Words(EXPORT_FORMATS),
    },
    Entry {
        name: "conflicts",
        aliases: &[],
        help: "cmd.commands.conflicts",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "grep",
        aliases: &["gr"],
        help: "cmd.commands.grep",
        needs: &[],
        args: Args::Free("<正則>"),
    },
    Entry {
        name: "diff",
        aliases: &[],
        help: "cmd.commands.diff",
        needs: &[],
        args: Args::Path,
    },
    Entry {
        name: "replace",
        aliases: &[],
        help: "cmd.commands.replace",
        needs: &[],
        args: Args::Free("<換成什麼>"),
    },
    Entry {
        name: "wa",
        aliases: &["wall"],
        help: "cmd.commands.wa",
        needs: &[],
        args: Args::None,
    },
    Entry {
        name: "row",
        aliases: &[],
        help: "cmd.commands.row",
        needs: &[Need::Table],
        args: Args::Free("<那一行的名字>"),
    },
    Entry {
        name: "toc",
        aliases: &["outline"],
        help: "cmd.commands.toc",
        needs: &[],
        args: Args::Free("<第幾條，不寫就列出來>"),
    },
    Entry {
        name: "ruby",
        aliases: &[],
        help: "cmd.commands.ruby",
        needs: &[],
        args: Args::Words(RUBY),
    },
    Entry {
        name: "s/pat/rep/",
        aliases: &[],
        help: "cmd.commands.substitute",
        needs: &[],
        args: Args::None,
    },
];

/// Whether what is being typed **right now** is free text or a path (#225).
///
/// A command line's *names* are ASCII — that is why the IME was kept out of it
/// altogether — but its **arguments are not**: `:s <正則> <換成什麼>` and
/// `:e`/`:w`/`:r <檔名>` are exactly the two an editor for Chinese novels wants
/// 中文 in, and until this they could only be pasted.
///
/// Three ways a line can be in text rather than in names:
///
/// * `:!…` hands the whole rest of the line to a shell, where a file name is
///   as likely to be Chinese as anywhere else;
/// * `:s/照首行/照全表/` is **one word** to a space-splitter, because a
///   substitution's delimiter is whatever follows the `s` — so it is answered
///   before the walk, or the case this feature is named after would be the one
///   case it missed;
/// * everything else: the finished words are walked, and what they lead to
///   decides. Once the walk is in `Free` or `Path`, the rest of the line is
///   too — `:grep 中文 再一個` is all pattern.
pub fn takes_text(line: &str) -> bool {
    let line = line.strip_prefix(':').unwrap_or(line);
    let (_, rest) = parse_rows(line);
    if rest.starts_with('!') {
        return true;
    }
    if let Some(after) = rest.strip_prefix('s') {
        if let Some(d) = after.chars().next() {
            if !(d.is_alphanumeric() || d.is_whitespace() || d == '\\') {
                return true;
            }
        }
    }
    // The word at the caret is the one being typed; what comes *before* it is
    // what says whether it is a name or a value. A line ending in a space is
    // already asking about the next word.
    let mut words: Vec<&str> = line.split(' ').filter(|w| !w.is_empty()).collect();
    if !line.ends_with(' ') {
        words.pop();
    }
    let Some(head) = words.first() else {
        // Still naming the command. `:層` names nothing and never will.
        return false;
    };
    let Some(entry) = entry_named(head) else {
        return false;
    };
    let mut args = &entry.args;
    for word in &words[1..] {
        match args.words() {
            // `pick`, not an exact match: it is the rule the parser walks by,
            // and a rule that only half applies is worse than either. `:yume
            // tab 詞庫.txt` runs, so it has to compose too.
            Some(list) => match pick(word, list) {
                Some(found) => args = &found.then,
                None => return false,
            },
            None => return matches!(args, Args::Free(_) | Args::Path),
        }
    }
    matches!(args, Args::Free(_) | Args::Path)
}

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
    let mut choices: Vec<Choice> = match words.first() {
        None => COMMANDS
            .iter()
            .filter(|e| {
                e.name.starts_with(typed) || e.aliases.iter().any(|a| a.starts_with(typed))
            })
            .map(|e| Choice {
                name: e.name,
                needs: e.needs,
                alias: e.aliases.first().copied(),
                // **Among the aliases too.** `:row` has no alias of its own
                // and no other *name* starts with `ro`, so the shortest walk
                // over names alone offered `(ro)` — while `ro` is `:readonly`'s
                // declared alias, and an exact alias beats a prefix in
                // `resolve`. The menu was promising a spelling that did
                // something else.
                short: shortest(
                    e.name,
                    COMMANDS
                        .iter()
                        .filter(|c| c.name != e.name)
                        .flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied())),
                ),
                help: e.help,
                leading: ":",
                under: String::new(),
            })
            .collect(),
        // **The same walk the parser walks** (§5.2.2 fault 1): `walk` resolves
        // the head by prefix and picks each word below it the same way, so the
        // menu answers `:tab ` exactly as it answers `:table `.
        //
        // And a miss is an empty list rather than a `return`, because the
        // fallback below is what answers a word this level does not know — the
        // early return took `:vert` and #223's deep search out with it.
        Some(_) => match walk(&words) {
            None => Vec::new(),
            Some(args) => match args.words() {
                Some(list) => list
                    .iter()
                    .filter(|w| w.name.starts_with(typed))
                    .map(|w| Choice {
                        name: w.name,
                        needs: w.needs,
                        alias: None,
                        short: shortest(w.name, list.iter().map(|o| o.name)),
                        help: w.help,
                        leading: "",
                        under: String::new(),
                    })
                    .collect(),
                // A path or free text is the caller's business; there is
                // nothing here to offer but the placeholder, which is help
                // rather than a completion.
                None => match args {
                    Args::Free(what) => vec![Choice {
                        name: "",
                        needs: &[],
                        alias: None,
                        short: None,
                        help: what,
                        leading: "",
                        under: String::new(),
                    }],
                    _ => Vec::new(),
                },
            },
        },
    };
    // Nothing at this depth knows that word — so look for it deeper (#223).
    // `:vert` is answered with `:layout vertical`, and `:yume ling` with
    // `scheme lingming`, because in both the reader named the leaf and not the
    // path. Guarded on an empty result rather than merged into it: a word that
    // *does* name something here has been answered already.
    if choices.is_empty() && !typed.is_empty() {
        choices = match words.first() {
            None => deep_from_root(typed),
            Some(_) => match walk(&words) {
                // The path already typed is the prefix the reader does not
                // have to type again, so the offer starts below it.
                Some(args) if args.words().is_some() => {
                    let mut out = Vec::new();
                    for w in args.words().unwrap_or_default() {
                        if let Some(inner) = w.then.words() {
                            deep(inner, &[w.name], typed, "", &mut out);
                        }
                    }
                    out
                }
                _ => Vec::new(),
            },
        };
    }

    // A word that is *finished* also says what may follow it. Nothing is
    // promoted for a half-typed word: `:yu` is still a question about which
    // command, and answering it with a list of the input method's verbs would
    // be answering a question nobody asked.
    if !typed.is_empty() {
        let below = match words.first() {
            None => COMMANDS
                .iter()
                .find(|e| e.name == typed || e.aliases.contains(&typed))
                .map(|e| children(e.name, &e.args)),
            Some(_) => choices
                .iter()
                .find(|c| c.name == typed && c.under.is_empty())
                .and_then(|c| walk(&words).map(|args| (c.name, args)))
                .and_then(|(name, args)| {
                    args.words()?
                        .iter()
                        .find(|w| w.name == name)
                        .map(|w| children(w.name, &w.then))
                }),
        };
        choices.extend(below.unwrap_or_default());
    }
    (start, choices)
}

/// Every command and every word under it, as one flat list (#224).
///
/// `complete` answers *what starts with this*; `::` asks *what does this*, and
/// that question has no prefix to walk down — the reader typed 排序 and the
/// answer is `:table sort`, which shares not one letter with it. So the search
/// needs the whole tree at once, each row carrying the path that has to be
/// typed to reach it, and ranks it by what it says rather than how it is
/// spelled.
///
/// The top level first, then everything below it shallowest-first, so a tie in
/// the ranking falls out as the shorter command.
pub fn all_choices() -> Vec<Choice> {
    let mut out = complete("");
    out.extend(deep_from_root(""));
    out
}

/// What the command on this line needs before it can do anything.
///
/// The *deepest* word that says so: `:table rules` needs a table because
/// `rules` says it, and `:yume chaifen` needs a 碼表 because `chaifen` does.
/// Walked from the same table the completion walks, so a prerequisite is
/// declared once and shown, said and satisfied from the one declaration.
pub fn needs_of(line: &str) -> &'static [Need] {
    let line = line.strip_prefix(':').unwrap_or(line);
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(head) = words.first() else {
        return &[];
    };
    // The same prefix rule the parser uses, so `:h on` is answered about
    // `hanging` and not about nothing.
    let Some(entry) = entry_named(head) else {
        return &[];
    };
    let mut needs = entry.needs;
    let mut args = &entry.args;
    for word in &words[1..] {
        let Some(list) = args.words() else { break };
        let Some(found) = pick(word, list) else { break };
        if !found.needs.is_empty() {
            needs = found.needs;
        }
        args = &found.then;
    }
    needs
}

/// What may follow the words already on the line, or `None` if they name
/// nothing.
fn walk(words: &[(usize, &str)]) -> Option<&'static Args> {
    let (_, head) = words.first()?;
    let entry = entry_named(head)?;
    let mut args = &entry.args;
    for &(_, word) in &words[1..] {
        match args.words() {
            // `pick`, not an exact match — one rule, at every level. `:tab r`
            // *runs* as `:table rules`, so it has to be answerable too.
            Some(list) => args = &pick(word, list)?.then,
            None => return None,
        }
    }
    Some(args)
}

/// Does a line the **documents** print name something the editor has?
///
/// Not `parse`: no argument is evaluated and nothing is run, so a line that is
/// merely impossible at this moment — `:w` with no file, `:table rules` with no
/// table — still answers yes. It settles the one question a manual can get
/// wrong all by itself, 「有沒有這條命令，它收不收後面這幾個字」, by walking
/// the same `resolve` / `pick` rule the parser walks.
///
/// The **first word it cannot place** comes back, so a document that has gone
/// stale names the word rather than the line. Walking stops where the words
/// stop: under a path or free text what follows is the writer's own text, not
/// this table's vocabulary.
pub fn names_something(line: &str) -> Result<(), String> {
    let line = line.strip_prefix(':').unwrap_or(line);
    let mut words = line.split_whitespace();
    let Some(head) = words.next() else {
        return Err(String::from(":"));
    };
    // The two the parser reaches **before** the name split, and so the two the
    // table cannot hold under a word: `:!make` takes the whole rest of the line,
    // and the substitution's entry is named for its shape, `s/pat/rep/`. The
    // documents call the second one `:s`, which is what a reader types.
    if head == "s" || head.starts_with("s/") || head.starts_with('!') {
        return Ok(());
    }
    let Some(entry) = entry_named(head) else {
        return Err(head.to_string());
    };
    let mut args = &entry.args;
    for word in words {
        let Some(list) = args.words() else { return Ok(()) };
        let Some(found) = pick(word, list) else {
            return Err(word.to_string());
        };
        args = &found.then;
    }
    Ok(())
}

/// Which way a search runs.
///
/// A page is read across a line and then down to the next, and `/` searches it
/// that way. A **table** has a second way of being read that a document does
/// not: down one column, then down the next. Asking 「誰用了卵」 of a 拆分表 is
/// asking about one column at a time, and the answers want to arrive in that
/// order — so it is not a different feature, it is the same verb on the other
/// axis. `Enter` in a table is its shortcut, as `/` is row's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Across a line, then the next — what `/` does, in a table or not.
    Row,
    /// Down a column, then the next. Only a table has these.
    Column,
}

/// Which lines a `:s` touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rows {
    /// No range written: the lines the **selection** covers.
    Selection,
    /// `%` — every line.
    All,
    /// `1,40`, `.,$`, `5` — from one bound to another, inclusive.
    Range(Bound, Bound),
}

/// One end of a `:s` range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// A line number, counting from 1.
    Line(usize),
    /// `.` — the line the cursor is on.
    Cursor,
    /// `$` — the last line.
    Last,
}

/// Read one end of a range, and say how much of `input` it took.
fn parse_bound(input: &str) -> Option<(Bound, usize)> {
    let mut chars = input.char_indices();
    match chars.next()? {
        (_, '.') => Some((Bound::Cursor, 1)),
        (_, '$') => Some((Bound::Last, 1)),
        (_, c) if c.is_ascii_digit() => {
            let end = input
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(input.len());
            input[..end].parse().ok().map(|n| (Bound::Line(n), end))
        }
        _ => None,
    }
}

/// Split the leading range off a `:s` line.
fn parse_rows(input: &str) -> (Rows, &str) {
    if let Some(rest) = input.strip_prefix('%') {
        return (Rows::All, rest);
    }
    let Some((first, took)) = parse_bound(input) else {
        return (Rows::Selection, input);
    };
    let rest = &input[took..];
    match rest.strip_prefix(',').and_then(|r| parse_bound(r).map(|(b, n)| (b, n, r))) {
        Some((second, n, r)) => (Rows::Range(first, second), &r[n..]),
        // A bare number is one line, the way `:40s` reads in vi.
        None => (Rows::Range(first, first), rest),
    }
}

/// Split `body` on unescaped `delim`.
fn split_escaped(body: &str, delim: char) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut escaped = false;
    for c in body.chars() {
        if escaped {
            // `\/` is the delimiter itself; every other escape is the regex's
            // or the replacement's own and is passed through untouched.
            if c != delim {
                out.last_mut().unwrap().push('\\');
            }
            out.last_mut().unwrap().push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delim {
            out.push(String::new());
        } else {
            out.last_mut().unwrap().push(c);
        }
    }
    if escaped {
        out.last_mut().unwrap().push('\\');
    }
    out
}

/// Try to parse a substitution command.
///
/// Returns `None` when the input is not a substitution, or `Some(Err(..))` when
/// it looks like one but is malformed.
///
/// **The delimiter is whatever follows the `s`**, as it is in vi and sed: a
/// pattern holding a `/` — a date, a path, a URL — is unreachable otherwise,
/// and `:s` used to answer 「substitute requires an argument」 to one.
fn parse_substitution(input: &str) -> Option<Result<Command, CommandError>> {
    let (rows, rest) = parse_rows(input);
    let rest = rest.strip_prefix('s')?;
    let delim = rest.chars().next()?;
    // Not a letter, a digit or a space: those are other commands (`:set`, and
    // `:s` on its own), and a backslash is an escape wherever it appears.
    if delim.is_alphanumeric() || delim.is_whitespace() || delim == '\\' {
        return None;
    }
    let body = &rest[delim.len_utf8()..];
    let fields = split_escaped(body, delim);
    if fields.len() < 2 || fields.len() > 3 {
        return Some(Err(CommandError::MissingArgument("substitute")));
    }
    if fields[0].is_empty() {
        return Some(Err(CommandError::MissingArgument("substitute")));
    }
    let flags = fields.get(2).cloned().unwrap_or_default();
    // Saying so beats doing the substitution the flag was meant to hold back:
    // `n` in vi means "count, change nothing", and it used to *substitute*.
    if let Some(bad) = flags.chars().find(|c| !"ginct".contains(*c)) {
        return Some(Err(CommandError::InvalidArgument {
            command: "substitute",
            value: say!("substitute.unknown-flag", bad),
        }));
    }
    if flags.contains('c') {
        return Some(Err(CommandError::InvalidArgument {
            command: "substitute",
            value: say!("substitute.confirm-not-yet"),
        }));
    }
    Some(Ok(Command::Substitute {
        pattern: fields[0].clone(),
        replacement: fields[1].clone(),
        global: flags.contains('g'),
        ignore_case: flags.contains('i'),
        count_only: flags.contains('n'),
        reshape: flags.contains('t'),
        rows,
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

    /// §5.2.2 fault 6: a bang has to survive every prefix that resolves
    /// without one.
    ///
    /// `:rec` was `recover` and `:rec!` was 「沒有『rec』這個命令」, because
    /// the name lived in one of the two lists that had to agree and not the
    /// other. There is one list now; this walks it so a tenth forceable
    /// command cannot arrive half-registered.
    #[test]
    fn a_forceable_command_takes_its_bang_on_every_prefix() {
        for banged in FORCEABLE {
            let name = banged.strip_suffix('!').expect("spelled with its bang");
            for cut in 1..=name.len() {
                let stem = &name[..cut];
                // Only where the short spelling is this command's to begin
                // with: `:q` is `quit`'s, `:qu` is not `quitall`'s.
                if resolve(stem) != name {
                    continue;
                }
                assert_eq!(
                    resolve(&format!("{stem}!")),
                    *banged,
                    "`:{stem}` is `{name}` but `:{stem}!` is not `{banged}`"
                );
            }
        }
        assert_eq!(parse(":rec!"), Ok(Command::Recover { discard: true }));
        assert_eq!(parse(":recover"), Ok(Command::Recover { discard: false }));

        // …and the other direction, which is the one that actually failed:
        // `parse` reads `recover!` by its whole name whatever `FORCEABLE`
        // says, so a command can accept a bang and never be registered as
        // taking one. Spelled in full, a command that takes no bang is the
        // only one that comes back 「不認得」.
        for entry in COMMANDS {
            // Two entries are spelled as the line they match rather than as a
            // word — `!<命令>` hands the rest to a shell and `s/pat/rep/` is
            // its own syntax — so a `!` glued to the end of them lands in an
            // argument, not on the command.
            if !entry.name.chars().all(|c| c.is_ascii_alphabetic()) {
                continue;
            }
            let takes_one = !matches!(
                parse(&format!(":{}!", entry.name)),
                Err(CommandError::Unknown(_))
            );
            assert_eq!(
                takes_one,
                forceable(entry.name).is_some(),
                "`:{}!` parses={takes_one} but FORCEABLE says otherwise",
                entry.name
            );
        }
    }

    /// §5.2.2 fault 1: the menu answers the abbreviation it printed itself.
    ///
    /// `:table ` draws twelve words with their shortest spellings — `off
    /// (of)`, `rules (r)` — and `:tab rules` **runs**, because `resolve`
    /// expands the prefix. The menu was the one reader of `COMMANDS` that did
    /// not ask it: `complete_at` looked the head up by exact name or alias and
    /// matched each word below it the same way, so the shortest spelling it
    /// had just printed drew a blank panel.
    ///
    /// Asked of every command at once, and of every word under it, because
    /// `shortest` is what the menu prints — so the two can only disagree here
    /// if they disagree in front of a reader.
    #[test]
    fn the_menu_answers_the_abbreviation_it_printed() {
        let every = || {
            COMMANDS
                .iter()
                .flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied()))
        };
        let names = |line: &str| -> Vec<&'static str> {
            complete_at(line).1.iter().map(|c| c.name).collect()
        };
        let mut asked = 0;
        for entry in COMMANDS {
            let Some(short) = shortest(entry.name, every()) else {
                continue;
            };
            let full = names(&format!("{} ", entry.name));
            if full.is_empty() {
                continue;
            }
            assert_eq!(
                names(&format!("{short} ")),
                full,
                "`:{short} ` is the spelling the menu prints for `:{}`",
                entry.name
            );
            asked += 1;

            // …and one level down, where the same rule has to hold.
            let Some(list) = entry.args.words() else {
                continue;
            };
            for word in list {
                let Some(inner) = shortest(word.name, list.iter().map(|o| o.name)) else {
                    continue;
                };
                let deep = names(&format!("{} {} ", entry.name, word.name));
                if deep.is_empty() {
                    continue;
                }
                assert_eq!(
                    names(&format!("{short} {inner} ")),
                    deep,
                    "`:{short} {inner} ` is `:{} {}`",
                    entry.name,
                    word.name
                );
                asked += 1;
            }
        }
        assert!(asked > 20, "the walk found almost nothing: {asked}");
    }

    /// Every place the menu prints 「on｜off」, asked whether the parser reads
    /// it back (§5.2.2 fault 2).
    ///
    /// The declaration and the arm that answers it sit two thousand lines
    /// apart, so nothing but a walk of the table can tell they disagree.
    /// `:hanging off` turned hanging punctuation **on**: `COMMANDS` declared
    /// `Args::Words(ON_OFF)`, the hint printed it, the menu offered it, and
    /// `parse` was `"hanging" => Ok(Command::ToggleHanging)` with `rest`
    /// never read. `every_listed_command_parses` cannot see it — `:hanging
    /// off` *parses* — so the question has to be whether the two answers
    /// differ.
    #[test]
    fn a_command_that_offers_on_and_off_reads_them_back() {
        /// The list is 「on｜off」 — asked by what it holds rather than by
        /// which constant it is, so a second, hand-rolled pair is caught too.
        fn is_a_switch(args: &Args) -> bool {
            matches!(args.words(), Some(list)
                if list.len() == 2 && list[0].name == "on" && list[1].name == "off")
        }
        fn walk_down(args: &'static Args, path: &str, out: &mut Vec<String>) {
            if is_a_switch(args) {
                out.push(path.to_string());
                return;
            }
            for word in args.words().unwrap_or_default() {
                walk_down(&word.then, &format!("{path} {}", word.name), out);
            }
        }
        let mut paths = Vec::new();
        for entry in COMMANDS {
            walk_down(&entry.args, entry.name, &mut paths);
        }
        assert!(paths.len() > 8, "the walk found almost nothing: {paths:?}");

        for path in &paths {
            let on = parse(&format!(":{path} on"));
            let off = parse(&format!(":{path} off"));
            assert!(on.is_ok(), "`:{path} on` is offered and refused: {on:?}");
            assert!(off.is_ok(), "`:{path} off` is offered and refused: {off:?}");
            assert_ne!(
                on, off,
                "`:{path} on` and `:{path} off` are the same command — the \
                 menu offers a switch the parser never reads"
            );
            // …and the abbreviation the menu shows is the one that parses.
            // `pick` is the rule everywhere else; an equality test here would
            // make `:hanging of` an error under a menu that prints `of`.
            assert_eq!(parse(&format!(":{path} of")), off, "`:{path} of`");
        }
    }

    #[test]
    fn parses_undo_and_redo() {
        assert_eq!(parse(":undo"), Ok(Command::Undo));
        assert_eq!(parse(":u"), Ok(Command::Undo));
        assert_eq!(parse(":redo"), Ok(Command::Redo));
    }

    #[test]
    fn parses_segment_toggle() {
        // 分詞邊界 is one subject and one command now.
        assert_eq!(parse(":word"), Ok(Command::Word(WordCommand::Report)));
        assert_eq!(
            parse(":word show on"),
            Ok(Command::Word(WordCommand::Show(Some(true))))
        );
        assert_eq!(parse(":word list"), Ok(Command::Word(WordCommand::List)));
        assert_eq!(
            parse(":word list reload"),
            Ok(Command::Word(WordCommand::Reload))
        );
        assert_eq!(
            parse(":word level strict"),
            Ok(Command::Word(WordCommand::Level(Some(
                yumete_cjk::WordLevel::Strict
            ))))
        );
        assert_eq!(
            parse(":word discover"),
            Ok(Command::Word(WordCommand::Discover))
        );
        assert_eq!(parse(":word habit"), Ok(Command::Word(WordCommand::Habit)));
        // A level nobody defined is refused by name, not silently taken.
        assert!(parse(":word level 中等").is_err());
        // The commands it replaced are gone — `:words` was 口頭禪 and is
        // `:word habit`, one subject and one command.
        assert!(parse(":segment").is_err());
        assert!(parse(":words").is_err());
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
        assert_eq!(parse(":yume chaifen"), Ok(Command::SetChaifen(None)));
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
        assert_eq!(parse(":yume which"), Ok(Command::YumeStatus));
        assert_eq!(parse(":yume on"), Ok(Command::YumeLanguage(true)));
        assert_eq!(parse(":yume off"), Ok(Command::YumeLanguage(false)));
        assert_eq!(parse(":yume installed"), Ok(Command::InstalledScheme));
        assert_eq!(parse(":yume builtin"), Ok(Command::BuiltinScheme));
        assert_eq!(parse(":yume b"), Ok(Command::BuiltinScheme));
        // 上屏方式 (Feature #209): the three are yume's own tags, `auto` is
        // 唯一 under the name the habit uses, and no argument is the question.
        assert_eq!(parse(":yume commit"), Ok(Command::YumeCommit(None)));
        assert_eq!(
            parse(":yume commit delayed"),
            Ok(Command::YumeCommit(Some("delayed".into())))
        );
        assert_eq!(
            parse(":yume commit u"),
            Ok(Command::YumeCommit(Some("unique".into())))
        );
        assert_eq!(
            parse(":yume commit auto"),
            Ok(Command::YumeCommit(Some("unique".into())))
        );
        assert_eq!(
            parse(":yume commit fluency"),
            Ok(Command::YumeCommit(Some("fluency".into())))
        );
        // #211: 候選面板 is the other axis, and it parses by prefix too.
        assert_eq!(parse(":yume panel"), Ok(Command::YumePanel(None)));
        assert_eq!(
            parse(":yume panel bare"),
            Ok(Command::YumePanel(Some("bare".into())))
        );
        assert_eq!(
            parse(":yume p f"),
            Ok(Command::YumePanel(Some("full".into())))
        );
        assert!(matches!(
            parse(":yume panel invisible"),
            Err(CommandError::InvalidArgument {
                command: "yume panel",
                ..
            })
        ));
        // Not silently the first of the three.
        assert!(matches!(
            parse(":yume commit slow"),
            Err(CommandError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn parses_substitution() {
        let plain = |pattern: &str, replacement: &str| Command::Substitute {
            pattern: pattern.into(),
            replacement: replacement.into(),
            global: false,
            ignore_case: false,
            count_only: false,
            reshape: false,
            rows: Rows::Selection,
        };
        assert_eq!(parse(":s/foo/bar/"), Ok(plain("foo", "bar")));
        assert_eq!(
            parse(":%s/foo/bar/g"),
            Ok(Command::Substitute {
                pattern: "foo".into(),
                replacement: "bar".into(),
                global: true,
                ignore_case: false,
                count_only: false,
            reshape: false,
                rows: Rows::All,
            })
        );
        // Empty replacement (a deletion) is allowed.
        assert_eq!(parse(":s/foo//"), Ok(plain("foo", "")));
        // Empty pattern is rejected.
        assert!(parse(":s//bar/").is_err());
    }

    #[test]
    fn the_delimiter_is_whatever_follows_the_s() {
        // A pattern holding a `/` — a date, a path, a URL — was unreachable,
        // and `:s` answered 「substitute requires an argument」 to one.
        let want = Command::Substitute {
            pattern: "2024/01".into(),
            replacement: "2025/02".into(),
            global: false,
            ignore_case: false,
            count_only: false,
            reshape: false,
            rows: Rows::Selection,
        };
        assert_eq!(parse(":s#2024/01#2025/02#"), Ok(want.clone()));
        assert_eq!(parse(":s,2024/01,2025/02,"), Ok(want.clone()));
        // …and the vi spelling still works.
        assert_eq!(parse(r":s/2024\/01/2025\/02/"), Ok(want));
        // A letter after `s` is another command, not a delimiter — whatever
        // `:set` turns out to mean, it is not a substitution.
        assert!(!matches!(parse(":set"), Ok(Command::Substitute { .. })));
    }

    #[test]
    fn substitution_flags_are_read_rather_than_swallowed() {
        assert_eq!(
            parse(":%s/a/b/gi"),
            Ok(Command::Substitute {
                pattern: "a".into(),
                replacement: "b".into(),
                global: true,
                ignore_case: true,
                count_only: false,
            reshape: false,
                rows: Rows::All,
            })
        );
        // `n` in vi means "count, change nothing" — and it used to substitute.
        assert!(matches!(
            parse(":%s/a/b/n"),
            Ok(Command::Substitute { count_only: true,
            reshape: false, .. })
        ));
        // A flag that is not implemented says so rather than being dropped.
        assert!(parse(":%s/a/b/c").is_err());
        assert!(parse(":%s/a/b/z").is_err());
    }

    #[test]
    fn a_substitution_takes_a_line_range() {
        assert!(matches!(
            parse(":1,40s/a/b/"),
            Ok(Command::Substitute {
                rows: Rows::Range(Bound::Line(1), Bound::Line(40)),
                ..
            })
        ));
        assert!(matches!(
            parse(":.,$s/a/b/"),
            Ok(Command::Substitute {
                rows: Rows::Range(Bound::Cursor, Bound::Last),
                ..
            })
        ));
        // A bare number is one line, the way `:40s` reads in vi.
        assert!(matches!(
            parse(":40s/a/b/"),
            Ok(Command::Substitute {
                rows: Rows::Range(Bound::Line(40), Bound::Line(40)),
                ..
            })
        ));
    }

    #[test]
    fn a_theme_and_an_appearance_are_two_questions() {
        use Mood::*;
        // `:theme` names the inks; `:appearance` says which way round they go.
        assert_eq!(
            parse(":theme heibai"),
            Ok(Command::Theme { name: Some("heibai".into()), mood: None })
        );
        assert_eq!(
            parse(":appearance light"),
            Ok(Command::Theme { name: None, mood: Some(Light) })
        );
        // …and one line may still ask both.
        assert_eq!(
            parse(":theme moxiang dark"),
            Ok(Command::Theme { name: Some("moxiang".into()), mood: Some(Dark) })
        );
        // The menu lists them apart, too.
        let themes: Vec<String> = complete("theme ").iter().map(Choice::written).collect();
        assert_eq!(
            themes,
            [
                "ink",
                "bw",
                "cyanotype",
                "amber",
                "mogao",
                "morandi",
                "firefly",
                "meridian",
                "kiln",
                "complement"
            ],
            "the menu lists what can be typed — ASCII, with the pinyin as an alias"
        );
        let moods: Vec<String> = complete("appearance ").iter().map(Choice::written).collect();
        assert_eq!(moods, ["system", "dark", "light"]);
        // The shortest spelling the menu offers has to work.
        let short = shortest("appearance", COMMANDS.iter().map(|c| c.name));
        assert!(parse(&format!(":{} dark", short.unwrap_or("appearance"))).is_ok());
    }

    #[test]
    fn every_short_form_the_menu_offers_actually_parses() {
        // The menu prints `check (ch)` next to every word it offers, and that
        // parenthesis is a promise: type those two letters and you get this.
        // The arms of `parse` match whole words, so for a year most of those
        // promises were broken — `:table ch`, `:render f`, `:buffer n`,
        // `:clipboard y` — and nothing said so, because the abbreviation is
        // worked out from the word list while the parser is written by hand.
        //
        // Now `spell_out` writes the words out before the arms see them, and
        // this walks every list to hold it to account: the short spelling has
        // to parse to **the same command** the whole word does. Only words
        // that need no argument can be asked outright; a word that takes one
        // is walked into instead, so `:table ru li` is covered as well.
        fn check(prefix: &str, list: &'static [Word]) {
            for word in list {
                let short = shortest(word.name, list.iter().map(|o| o.name));
                let full = format!("{prefix} {}", word.name);
                if let Some(short) = short {
                    let abbreviated = format!("{prefix} {short}");
                    assert_eq!(
                        parse(&abbreviated),
                        parse(&full),
                        "the menu offers `{short}` for `{}`",
                        word.name
                    );
                }
                if let Some(under) = word.then.words() {
                    // Down the short spelling, so a lie at either level shows.
                    let walk = match short {
                        Some(short) => format!("{prefix} {short}"),
                        None => full,
                    };
                    check(&walk, under);
                }
            }
        }
        for entry in COMMANDS {
            if let Some(list) = entry.args.words() {
                check(&format!(":{}", entry.name), list);
            }
        }
    }

    #[test]
    fn a_shot_says_which_picture_and_the_menu_can_say_it_for_you() {
        let shot = |line: &str| match parse(line) {
            Ok(Command::Screenshot { shot, .. }) => shot,
            other => panic!("{line}: {other:?}"),
        };
        // Bare is the clipboard, and `screen` is the same thing said aloud.
        assert_eq!(shot(":shot"), Shot::Screen);
        assert_eq!(shot(":shot screen"), Shot::Screen);
        assert_eq!(shot(":shot!"), Shot::Screen, "the bang is the editor's to refuse");
        let file = |how, path: Option<&str>| Shot::File {
            how,
            path: path.map(str::to_string),
        };
        assert_eq!(shot(":shot png"), file(ShotFormat::Png, None));
        assert_eq!(shot(":shot html"), file(ShotFormat::Html, None));
        assert_eq!(shot(":shot txt"), file(ShotFormat::Text, None));
        // A name may hold spaces, so the split is once and not by whitespace.
        assert_eq!(
            shot(":shot html 給編輯 二稿.html"),
            file(ShotFormat::Html, Some("給編輯 二稿.html"))
        );
        // A prefix is the word it starts, the way every other word list works.
        assert_eq!(shot(":shot p"), file(ShotFormat::Png, None));
        assert_eq!(shot(":shot h"), file(ShotFormat::Html, None));
        // A name the clipboard cannot keep is refused, not dropped — and the
        // refusal does not pretend the name was the problem.
        assert!(matches!(
            parse(":shot screen 圖.png"),
            Err(CommandError::TakesNoArgument { value, .. }) if value == "圖.png"
        ));
        // …and a word that is no picture at all names itself in the refusal.
        assert!(matches!(
            parse(":shot jpeg"),
            Err(CommandError::InvalidArgument { value, .. }) if value == "jpeg"
        ));
        // The panel the writer opens on `:shot ` — which used to be empty,
        // because the argument was a free string (2026-09-06).
        let words: Vec<String> = complete("shot ").iter().map(Choice::written).collect();
        assert_eq!(words, ["screen", "png", "html", "txt"]);
    }

    #[test]
    fn a_table_says_how_its_columns_are_told_apart() {
        use crate::table::{Rules, Stroke};
        let rules = |line: &str| match parse(line) {
            Ok(Command::SetTableRules(r)) => r,
            other => panic!("{line}: {other:?}"),
        };
        assert_eq!(rules(":table rules"), None, "bare, it only reports");
        assert_eq!(rules(":table rules off"), Some(Rules::Off));
        assert_eq!(rules(":table rules color"), Some(Rules::Colour));
        assert_eq!(rules(":table rules colour"), Some(Rules::Colour));
        for line in [":table rules line", ":table rules line solid"] {
            assert_eq!(rules(line), Some(Rules::Line(Stroke::Solid)), "{line}");
        }
        assert_eq!(rules(":table rules line dash"), Some(Rules::Line(Stroke::Dash)));
        assert_eq!(
            rules(":table rules line double"),
            Some(Rules::Line(Stroke::Double))
        );
        assert!(parse(":table rules squiggly").is_err());
        // …and the menu lists them, the way a finished word does.
        let words: Vec<String> = complete("table rules ").iter().map(Choice::written).collect();
        assert_eq!(words, ["off", "color", "line"]);
    }

    #[test]
    fn a_theme_is_two_questions_and_either_may_be_left_out() {
        use Mood::*;
        let theme = |line: &str| parse(line).unwrap();
        // Neither half: the way to *ask* where things stand.
        assert_eq!(
            theme(":theme"),
            Command::Theme {
                name: None,
                mood: None
            }
        );
        assert_eq!(
            theme(":theme moxiang"),
            Command::Theme {
                name: Some("moxiang".into()),
                mood: None
            }
        );
        // The name, and the mood with or without it. A name the words know is
        // canonicalised; anything else is passed on as typed, because which
        // themes exist is the front end's business.
        for line in [":theme ink dark", ":theme in d"] {
            assert_eq!(
                theme(line),
                Command::Theme {
                    name: Some("ink".into()),
                    mood: Some(Dark)
                },
                "{line}"
            );
        }
        // A name is ASCII — a command line is typed with the IME off — and the
        // pinyin answers for the hand that thinks in Chinese.
        assert_eq!(
            theme(":theme ink dark"),
            Command::Theme {
                name: Some("ink".into()),
                mood: Some(Dark)
            }
        );
        assert_eq!(
            theme(":theme moxiang"),
            Command::Theme {
                name: Some("moxiang".into()),
                mood: None
            }
        );
        assert_eq!(
            theme(":theme heibai"),
            Command::Theme {
                name: Some("heibai".into()),
                mood: None
            }
        );
        for (line, want) in [
            (":theme light", Light),
            (":theme system", System),
            (":theme l", Light),
        ] {
            assert_eq!(
                theme(line),
                Command::Theme {
                    name: None,
                    mood: Some(want)
                },
                "{line}"
            );
        }
        // An unknown name is not a *syntax* error: it is answered where the
        // colours are.
        assert_eq!(
            theme(":theme solarized"),
            Command::Theme {
                name: Some("solarized".into()),
                mood: None
            }
        );
    }

    #[test]
    fn completion_narrows_as_the_command_is_typed() {
        assert_eq!(
            complete("").len(),
            COMMANDS.len(),
            "`:` alone lists them all"
        );
        // One name, not three: `:ruby-on` and `:ruby-off` were the setting
        // wearing the verb's name, and they are now words `:ruby` takes — and
        // a finished word says what it takes, so they are listed under it.
        // The three levels come first because they are the setting; the
        // dialects after, because they are overrides of it (#283).
        let ruby: Vec<String> = complete("ruby").iter().map(Choice::written).collect();
        assert_eq!(
            ruby,
            [
                "ruby",
                "ruby off",
                "ruby basic",
                "ruby full",
                "ruby auto",
                "ruby html",
                "ruby typst",
                "ruby format"
            ]
        );
        // Half a word is still a question about which command.
        let rub: Vec<String> = complete("rub").iter().map(Choice::written).collect();
        assert_eq!(rub, ["ruby"]);
        // Two levels down, the same rule and the same spelling.
        let format: Vec<String> = complete("ruby format").iter().map(Choice::written).collect();
        assert_eq!(format, ["format", "format html", "format typst"]);
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
        // `:tab`, not `:ta`: `:target` arrived and took the two-letter
        // prefix away, which is the rule doing its job rather than a
        // regression — the menu lengthened what it prints in the same edit.
        assert_eq!(parse(":tab"), Ok(Command::EnterTable));
        assert_eq!(parse(":ta"), Err(CommandError::Unknown("ta".into())));

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
        //
        // Worked out over the **names and the aliases** together, because both
        // are things `resolve` matches: `ro` names no other command and would
        // have been offered for `:row`, while `ro` is `:readonly`'s declared
        // alias and an alias beats a prefix.
        let short = |name: &'static str| {
            shortest(
                name,
                COMMANDS
                    .iter()
                    .filter(|c| c.name != name)
                    .flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied())),
            )
        };
        assert_eq!(short("yume"), Some("y"));
        assert_eq!(short("render"), Some("ren"), "recover and redo are in the way");
        assert_eq!(short("sh"), None, "nothing shorter than the whole word");
        // `r` and `ro` are both taken — the second by `:readonly`'s alias —
        // so `:row` is offered with no short form at all, the way `:sh` is.
        assert_eq!(short("row"), None, "`ro` is `:readonly`'s");
        // Over what `complete` actually hands the menu, not over a second
        // derivation of it — the menu prints `Choice::short`, so that is the
        // string this has to hold to account.
        for choice in complete("") {
            let entry = COMMANDS.iter().find(|e| e.name == choice.name).unwrap();
            assert_eq!(choice.short, short(entry.name), "one rule, not two");
            if let Some(short) = choice.short {
                // It resolves, **and it resolves to this one**. Checking only
                // that it parsed let `:row (ro)` onto the menu for a year:
                // `:ro` parses, as `:readonly`.
                // Listed under the shape it is typed in rather than as a word:
                // `:!` is never a word on its own, it is the mark the shell
                // command follows.
                let named = resolve(short);
                assert!(
                    entry.name == "!command" ||
                    named == entry.name || entry.aliases.contains(&named),
                    "the menu offers `:{short}` for `:{}`, and it names `:{named}`",
                    entry.name
                );
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
        // `chaifen` and `commit` both start with `c`, so `c` alone names
        // neither — the menu on `:yume c` shows both, which is the answer.
        assert_eq!(parse(":yume ch"), Ok(Command::SetChaifen(None)));
        assert!(matches!(
            parse(":yume c"),
            Err(CommandError::InvalidArgument { .. })
        ));
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
        assert_eq!(words("syntax "), ["markdown", "typst", "text"]);
        assert_eq!(words("layout v"), ["vertical"]);

        // A parent command is not a mechanism of its own — its subcommands are
        // simply the words it takes, and they go as deep as they like.
        assert_eq!(
            words("ruby "),
            ["off", "basic", "full", "auto", "html", "typst", "format"]
        );
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

    /// A word that names nothing at the depth it was typed at is looked for
    /// **deeper** (#223).
    ///
    /// The reader who types `:vert` has not made a mistake; they have named
    /// the leaf and left out the path. Before this, the answer was nothing at
    /// all — `complete_at` looked the first word up in `COMMANDS`, missed, and
    /// returned an empty list.
    #[test]
    fn a_word_that_names_no_command_is_looked_for_deeper() {
        let written = |line: &str| -> Vec<String> {
            complete(line).iter().map(|c| c.written()).collect()
        };

        // The case that started it. The whole path is what Tab writes, so the
        // line ends up sayable rather than clever.
        // `:help` has a 竪排 topic too, and a topic is by construction named
        // after the thing it is about — so it comes second. Doing beats
        // reading about doing.
        assert_eq!(written("vert"), ["layout vertical", "help vertical"]);
        assert_eq!(written("horiz"), ["layout horizontal"]);

        // And the fallback reaches past the first level: a scheme's name is
        // two words below `:yume`.
        assert!(
            written("lingming").contains(&"yume scheme lingming".to_string()),
            "a leaf three deep is still reachable by its own name"
        );

        // What Tab replaces is the word that was typed, and the word starts at
        // the top of the line — so `:vert` becomes `:layout vertical` and not
        // `:vertlayout vertical`.
        assert_eq!(complete_at("vert").0, 0);

        // **Only when nothing matched at this depth.** `:t` names commands, so
        // it is answered about those and not buried under every grandchild of
        // the tree that happens to start with a `t`.
        let shallow = written("t");
        assert!(shallow.iter().all(|w| !w.contains(' ')), "{shallow:?}");
        assert!(shallow.len() > 1, "several commands start with t");

        // Deeper down it works the same way: `:yume` knows no word `ling`, so
        // the offer comes from below it — and carries only the part still
        // missing, because `yume ` is already on the line.
        let under_yume: Vec<String> = complete("yume ling")
            .iter()
            .map(|c| c.written())
            .collect();
        assert!(
            under_yume.contains(&"scheme lingming".to_string()),
            "{under_yume:?}"
        );

        // An empty prefix is not a question, and must never dump the tree.
        assert!(complete("nosuchcommandanywhere ").is_empty());
    }

    /// `::` searches the whole tree, so the whole tree has to be in the list
    /// (#224): every command, and every word under every command, each one
    /// carrying the path a reader would have to type to reach it.
    #[test]
    fn the_flat_list_holds_the_whole_tree() {
        let all = all_choices();
        let written: Vec<String> = all.iter().map(|c| c.written()).collect();
        for one in [
            "table",
            "table sort",
            "table new",
            "wrap",
            "layout vertical",
            "yume scheme lingming",
            "theme mogao",
        ] {
            assert!(
                written.iter().any(|w| w == one),
                "`:{one}` is not in the flat list, so `::` cannot find it:\n{written:#?}"
            );
        }
        // Nothing is listed twice: a duplicate is two rows saying the same
        // thing, which in a ranked list reads as two different answers.
        let mut seen = std::collections::BTreeSet::new();
        let twice: Vec<&String> = written.iter().filter(|w| !seen.insert(*w)).collect();
        assert!(twice.is_empty(), "listed twice: {twice:?}");
        // Every row can say what it does — the ranking has nothing to read
        // otherwise, and the menu would draw a blank line.
        for choice in &all {
            assert!(
                !choice.help.is_empty() || choice.name.is_empty(),
                "`{}` has no help, so it cannot be found by what it does",
                choice.written()
            );
        }
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
            "progress",
            "check",
            "grep",
            "conflicts",
            "diff",
            "toc",
            "export",
            "ruby",
            "yume",
            "buffer",
            "clipboard",
            "layout",
            "wrap",
            "wheel",
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
                _ if args.words().is_some_and(|l| !l.is_empty()) => {
                    let list = args.words().unwrap_or_default();
                    sample(&format!("{path} {}", list[0].name), &list[0].then)
                }
                Args::None | Args::Words(_) | Args::Schemes => path.to_string(),
                Args::Path => format!("{path} a.md"),
                // A word only this command knows the shape of.
                Args::Free(_) => {
                    let word = match path {
                        p if p.ends_with("scheme") => "lingming",
                        ":export" => "html",
                        ":grep" => "x",
                        // Both words are sides, and the pair has to be a
                        // conversion someone could actually ask for: a side
                        // converted to itself is refused on purpose.
                        ":convert t" => "s",
                        p if p.starts_with(":convert ") => "t",
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
            if let Some(list) = entry.args.words() {
                for word in list {
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
                    "sav" => ":sav a.md".to_string(),
                    "md" => ":md footnote".to_string(),
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
