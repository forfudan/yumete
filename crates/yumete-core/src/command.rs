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

/// Which of the three states the input method is in (#290).
///
/// **`Ascii` and `Off` look identical at the keyboard** — the keys type what
/// they say, either way — and they are still two states, because only one of
/// them is yume holding the keyboard. The front end may hold the terminal flag
/// that reports a bare Shift only while yume does; that same flag stops the
/// *system's* input method from composing, so ASCII-because-yume-is-passing-
/// it-through and ASCII-because-yume-is-gone have to be told apart.
///
/// The lone-Shift tap crosses between [`Self::Chinese`] and [`Self::Ascii`];
/// [`Self::Off`] is reached by `:yume off` and left by `:yume on` — no key
    /// of its own, settled (#290). The way out of [`Self::Off`] has
/// to be pressable while a *system* input method holds the keyboard, and the
/// two chords tried (`C-Space`, `Shift+Space`) are both spent there already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engagement {
    /// 中文: yume has the keyboard and composes with it.
    Chinese,
    /// ABC: yume has the keyboard and passes what the keys say straight
    /// through — a lone Shift tap is a keystroke away from 中文.
    Ascii,
    /// 關: yume has handed the keyboard back. Whatever the system does with a
    /// keystroke — its own input method, most of all — it does again.
    Off,
}

/// What `:word` was asked about — 分詞邊界, from every side.
///
/// **One subject, one command.** Where a word ends is decided by a dictionary,
/// shown by a colour, tuned by a level, mined out of the book itself, and read
/// back as 口頭禪; those were `:words`, `:segment` and a config key nobody could
/// see, and nothing said they were the same question.
// ⚠️ **Not `Copy` since #452.** The scope a 認詞 reads is [`Where`], which
// carries a `PathBuf` in one of its arms — the same type `:search` uses, and
// sharing it is the point: 「這一篇／這個資料夾／這個倉」 is one idea.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordCommand {
    /// `:word` — which dictionary is in force, and how many words this book adds.
    Report,
    /// `:word-show on|off` — the colour that says where the boundaries fell.
    /// `None` flips it.
    Show(Option<bool>),
    /// `:word-show tint|ink` — which of the two ways it is drawn: under the
    /// writing, or in the writing (Feature #278). Turns the overlay on.
    Mark(yumete_cjk::WordMark),
    /// `:word-list` — the same report, from the list's side.
    List,
    /// `:word-list reload` — read this book's list and the global one again.
    Reload,
    /// `:word-list edit` — open this book's `.yumete/words.txt`, existing or not.
    Edit,
    /// `:word-list global` — open the global `segmentation.txt`, existing or not.
    Global,
    /// `:word-discover` and its three wider spellings — **how far to read**
    /// (Feature #239). Mines the words no dictionary has and writes the list
    /// to `.yumete/discovered_words.txt`, overwriting it.
    ///
    /// The same four scopes `:search` has, and for the same reason: 「這一篇／
    /// 這個資料夾／這個倉／打開的那個目錄」 is one idea, and a reader who has
    /// learnt it once should not have to learn it twice.
    Discover(crate::search_panel::Where),
    /// `:word-habit` — the words this manuscript leans on, by surprisal
    /// against a 詞頻表 rather than by count (Feature #242). English writing
    /// calls these *crutch words*; the manual calls them 口頭禪.
    Habit,
    /// `:word-level off|strict|balanced|full` — how readily characters join into
    /// words. `None` says which it is.
    Level(Option<yumete_cjk::WordLevel>),
}

/// A parsed command-line command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `:open <path>` (aliases `:o`, `:edit`, `:e`) — open a file into a buffer.
    Open(String),
    /// `:open` with no path — the file picker, which is where a Chinese file
    /// name is typed (2026-09-16).
    ///
    /// The command line takes no 中文 anywhere on it, so a path with 漢字 in it
    /// has to be named somewhere that does. The picker is that somewhere, and
    /// it was the better way to open a file before this was true: it completes,
    /// it cannot be mistyped, and it shows what is actually there.
    OpenPicker,
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
    /// `:wq [path]` — save (optionally to a new path), then leave.
    WriteQuit(Option<String>),
    /// `:exit` (aliases `:x`, `:xit`) — **save only if the file changed**, then
    /// leave. vi's `:x` and helix's `:exit`, which `:x` used to be spelled as
    /// here without the one thing that tells the two apart.
    Exit(Option<String>),
    /// `:update` (alias `:up`) — **save only if the file changed**, and stay.
    /// `:write` with the same gate: the timestamp of a file nobody touched is
    /// what `make`, rsync and a sync folder all read as 「this changed」.
    Update,
    /// `:count` (alias `:wc`) — how much has been written.
    Count,
    /// `:count-progress` — 寫作進度: what was written today, and every day before
    /// (Feature #244).
    Progress,
    /// `:count-target <字>` — how many 字 a day; `None` is `:count-target off`.
    Target(Option<usize>),
    /// `:check-usage` — which of two spellings the manuscript settled on, and
    /// where it slipped (Feature #233).
    CheckUsage,
    /// `:check-punct` — half-width marks in Chinese text, `...` for ……, and
    /// the 「 nothing closes (Feature #238).
    CheckPunct,
    /// `:check-charset` — the characters no current standard carries, before
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
        /// `f`: **照字面** — the pattern is the characters typed, not a regex,
        /// and `$` in the replacement is a `$` rather than a capture.
        literal: bool,
        /// `c`: **逐處確認** — stop at each match and ask before writing.
        confirm: bool,
        /// `n`: say how many there are and change nothing, as vi's `n` means.
        count_only: bool,
        /// `t`: yes, this changes how many cells a row has — 表格的欄數也改.
        /// Without it a substitution that would reshape a grid is refused, and
        /// that refusal has to name a way through or it is a wall.
        reshape: bool,
        /// Which lines it touches.
        rows: Rows,
    },
    /// `:write-all` — save every buffer that has changed.
    WriteAll,
    /// `:undo` (alias `:u`) — undo the last change.
    Undo,
    /// `:redo` (alias `:red`) — redo the last undone change.
    Redo,
    /// `:word …` — everything about **where one word ends and the next
    /// begins**, which is one subject and used to be several commands
    /// (`:segment` coloured the boundaries, `:words` weighed them).
    Word(WordCommand),
    /// `:wiki [edit|global|reload]` — 作品百科 (#287). Bare, the report.
    Wiki(Option<String>),
    /// `:layout [horizontal|vertical]` (aliases `:horizontal`, `:vertical`) —
    /// choose the layout (Feature #61). `None` toggles between the two.
    SetLayout(Option<Layout>),
    /// `:yume-chaifen [on|off]` — the 拆分 annotation beside candidates
    /// (Feature #66). `None` is the bare word, which toggles.
    SetChaifen(Option<bool>),
    /// `:scheme <tag>` — switch the input scheme (Feature #86).
    SetScheme(String),
    /// `:view-hanging [on|off]` — 句讀 in the margin rather than a square each
    /// (Feature #70). `None` is the bare word, which toggles.
    SetHanging(Option<bool>),
    /// `:view-wrap` / `:view-wrap off` — whether a paragraph too wide for the terminal
    /// continues on the next screen row (Feature #77).
    SetSoftWrap(bool),
    /// `:view-wrap <n>` — write to a measure of `n` columns rather than to the
    /// window; `:view-wrap 0` gives the window back (Feature #113).
    SetMeasure(Option<usize>),
    /// `:wheel <n>` — how far one notch of the mouse wheel moves, in whichever
    /// unit the page is set in; `None` only reports (Feature #222).
    SetWheelStep(Option<usize>),
    /// `:table` / `:table off` — read the file as a grid (Feature #118).
    /// `:table` — the door: read the table the cursor is in.
    EnterTable,
    /// `:table off|basic|full` — how much of a table is drawn (#283).
    SetTableLevel(crate::editor::TableLevel),
    /// `:table-rules …` — how the columns are told apart (Feature #157).
    /// `None` only reports.
    SetTableRules(Option<crate::table::Rules>),
    /// `:format`, `:run <名字>` — a command this language declares in the
    /// config, run by the front end.
    Language(String),
    /// `:language [zh|zhs|en]` — the language the **editor** says things in
    /// (#494). `None` only reports. Not to be confused with [`Self::Language`]
    /// above, which runs a *programming* language's formatter.
    SayIn(Option<crate::messages::Language>),
    /// `:markdown …` — write a piece of Markdown at the cursor.
    Markdown(MarkdownBit),
    /// `:view-typewriter [on|off]` — the cursor's row stays in the middle.
    SetTypewriter(Option<bool>),
    /// `:view-focus [on|off]` — everything but the 段 being written stands back.
    SetFocus(Option<bool>),
    /// `:view-meter [on|off]` — 平仄 and 韻腳 in the margin.
    SetMeter(Option<bool>),
    /// `:view-punct [on|off]` — the mark that is wrong, named on the page beside it.
    SetNote(Option<bool>),
    /// `:view-code [on|off]` — fenced code in its own grammar's colours (#420).
    SetCode(Option<bool>),
    /// `:table-numbers on|off` — the row of column numbers above the header.
    SetTableNumbers(bool),
    /// `:search` — open the search panel (#419).
    ///
    /// ⚠️ **It takes no pattern, only a place.** What to look for is typed in
    /// the panel's own box, so that `:search 卵` can mean 「the folder called
    /// 卵」 without anybody having to guess. `-cd` is this file's folder,
    /// `-wd` the one yumete was opened in, `-gd` the nearest git project —
    /// that last one being the useful one, since nobody has to count how many
    /// levels up it was.
    OpenSearch(crate::search_panel::Where),
    /// `:replace [folder]` and its `-cd`/`-wd`/`-gd` — the same panel with the
    /// replace row already showing (#419).
    OpenReplace(crate::search_panel::Where),
    /// `:sidebar-show-left|right [panel]` — which side a panel lives on
    /// (#293). No name means the one holding the keys.
    ShowSidebarAt(crate::sidebar::Side, Option<crate::sidebar::Panel>),
    /// `:table-header [on|off]` — whether the grid's first row names the
    /// columns or is a row like any other (Feature #217). `None` flips it.
    SetTableHeader(Option<bool>),
    /// `:table-schema` — the schema file in the other work area, written next
    /// to the data first if none claims it yet (Feature #218).
    OpenTableSchema,
    /// `:table-new 3 4` — an empty `|` table of this shape, blank lines around
    /// it, the cursor typing in its first heading (Feature #276). `rows`
    /// counts the heading; the rule row is not a row.
    NewTable { rows: usize, columns: usize },
    /// `:table-detail [on|off]` — the panel; `None` toggles.
    ShowDetail(Option<bool>),
    /// `:table-detail 40` — how wide it is.
    SetDetailWidth(usize),
    /// `:table-sort 1 a 2 d` — put the rows in order by these columns, in this
    /// order. Empty sorts by the column the cursor is in.
    SortTable(Vec<(usize, bool)>),
    /// `:table-pipe [分隔]` — the delimited block under the cursor becomes a
    /// `|` table (Feature #227). `None` guesses the delimiter.
    TableToPipe(Option<char>),
    /// `:table-csv [分隔]` — the `|` table under the cursor becomes delimited
    /// lines (Feature #227). The delimiter defaults to a comma.
    TableToDelimited(char),
    /// `:view-numbers-fill` — whether the line-number band has a ground of its
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
    /// `:indent-hint color` — what, if anything, is drawn in the opening
    /// squares.
    SetIndentHint(crate::zong::IndentHint),
    /// `:indent-tab [spaces|tab]` — what Tab types in Insert mode; bare reports.
    SetTabInserts(Option<bool>),
    /// `:indent-width [n]` — how many columns Tab and `>` move; bare reports.
    SetIndentWidth(Option<usize>),
    /// `:view-bands 2` — how many bands the 縱書 page is divided into (段組).
    SetBands(usize),

    /// `:table-find row|column <pattern>` — the two directions a search can run.
    Search { pattern: String, by: Axis },
    /// `:table-check` — look the whole table over and list what is wrong.
    CheckTable,
    /// `:view-margin never|dense|loose|always` — whether a 縱 (or a row, across)
    /// keeps the lane beside it for readings, hung 句讀, 着重號, 平仄 and ticks.
    /// Bare, it says which is in force. Replaced `:view-dense` (2026-09-16); see
    /// [`yumete_cjk::Margin`].
    SetMargin(Option<yumete_cjk::Margin>),
    /// One 句 to a 縱 — a view of the page, not a change to the file.
    SetSentences(bool),
    /// `:render off|on|full` — how much of the result the page shows
    /// (Features #96 / #104).
    SetRender(crate::editor::Render),
    /// `:render` with no argument — say which level all four dimensions are on.
    ReportRender,
    /// `:view-hud off|basic|full` — how loudly the editor says, beside the caret,
    /// what you have typed (Feature #284).
    SetHud(crate::editor::Hud),
    /// `:view-hud` with no argument — say which of the three it is on.
    ReportHud,
    /// `:view-preview` / `:view-preview off` — hand the file to the real typesetter and
    /// show what it makes (Feature #128).
    SetPreview(bool),
    /// `:w!` — write over a file that changed on disk since it was read.
    WriteForce(Option<String>),
    /// `:reload` / `:reload!` — read the file again. The `!` throws away
    /// unsaved changes; without it a dirty buffer is refused (Feature #214).
    Reload { force: bool },
    /// `:reload-auto on|off` — re-read a **clean** buffer by itself when the
    /// file changes on disk. `None` asks which it is (Feature #214).
    ReloadAuto(Option<bool>),
    /// `:readonly on|off` — lock this buffer against editing. `None` asks
    /// (Feature #213).
    SetReadonly(Option<bool>),
    /// `:yume-builtin` — use the 碼表 in the binary, whatever is installed.
    BuiltinScheme,
    /// `:yume-table <path>` — type with a code table of your own.
    UserTable(String),
    /// `:yume` on its own — say what the input method is doing.
    YumeStatus,
    /// `:yume-where` — the six places 碼表 and 字料 are looked for, and what
    /// each of them holds. Opens as a buffer; see `yumete_tui::where_report`.
    YumeWhere,
    /// `:yume-commit delayed|unique|fluency` — 上屏方式: when a finished code
    /// goes to the page. `None` asks which one is in force (Feature #209).
    YumeCommit(Option<String>),
    /// `:yume-panel full|bare` — 候選面板: the bordered list, or the first
    /// candidate drawn into the sentence. `None` asks which one is in force.
    ///
    /// Independent of [`Command::YumeCommit`]: **when** a word lands on the
    /// page and **where** you read the candidate are two questions, and the
    /// nine combinations are all sensible (Feature #211).
    YumePanel(Option<String>),
    /// `:theme-fill [on|off]` — whether a 品色 run also gets a ground.
    ///
    /// Off: the backtick and the `>` are drawn already, so the run's extent is
    /// on the page and a ground behind it says it a second time. `None`
    /// toggles.
    ThemeFill(Option<bool>),
    /// `:yume-menu-size 1-9` — how many candidates a page of the panel holds.
    /// `None` asks how many it holds now.
    ///
    /// 宇浩's own front ends ship **6**, and a reader who knows where 「第七個
    /// 候選」 is on one of them should find it in the same place here.
    YumeMenuSize(Option<usize>),
    /// `:yume-autocompletion [on|off]` — 輸入預測: whether a code that is not
    /// finished is answered with the candidates that would finish it. `None`
    /// toggles, as `:yume-chaifen` does.
    YumeAutocompletion(Option<bool>),
    /// `:yume on` / `:yume abc` / `:yume off` — which of the three states the
    /// input method is in (#290).
    ///
    /// **Two of them are yume holding the keyboard** and one is not, and the
    /// difference is not cosmetic: the terminal flag that makes a bare Shift
    /// visible is the same flag that stops the *system's* input method from
    /// composing, so it can be held exactly while yume has the keys.
    /// `on` also loads the 碼表 when it has not been loaded, which is the
    /// whole of starting to write in Chinese.
    YumeLanguage(Engagement),
    /// `:yume-installed` — the 碼表 the *system* has, which is `builtin`'s
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
    /// `:keymap [helix|vim]` — lay a shipped keymap under the reader's own
    /// aliases; bare reports which (#428).
    SetKeymap(Option<yumete_cjk::KeyPreset>),
    /// `:write-as <path>` (and `:write-as!`) — write this buffer to another file
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
    /// `:table-jump 木` — go to the row this table names by that character.
    GotoRow(String),
    /// `:check-merge` — the merge conflicts in this file, as a results buffer
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
    /// `:ruby-auto` — write the readings in by word; `rare` keeps only the
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
    /// A subcommand written with a space where this tree uses a hyphen.
    ///
    /// ⚠️ Only where obeying the typo would be **destructive**. `:write` takes
    /// a path, so `:write all` was a perfectly legal request to copy the
    /// manuscript into a file called `all` — and it did, silently, in the
    /// working directory, while the writer believed every buffer had been
    /// saved. The manual taught the spelling, which is how it got typed.
    MeansTheHyphenatedOne {
        command: &'static str,
        word: String,
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
            CommandError::Unknown(word) => match moved_to(word) {
                Some(now) => write!(f, "{}", crate::say!("cmd.no-such-command-but", word, now)),
                None => write!(f, "{}", crate::say!("cmd.no-such-command", word)),
            },
            CommandError::MissingArgument(what) => {
                write!(f, "{}", crate::say!("cmd.needs-an-argument", what))
            }
            CommandError::InvalidArgument { command, value } => {
                write!(f, "{}", crate::say!("cmd.not-one-of-its-values", command, value))
            }
            CommandError::MeansTheHyphenatedOne { command, word } => {
                write!(f, "{}", crate::say!("cmd.means-the-hyphenated-one", command, word))
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
/// // …and with nothing to open, the picker, where 中文 can be typed.
/// assert_eq!(parse(":open"), Ok(Command::OpenPicker));
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
    // `:table-check` growing a `csv` sibling turned the menu's `check (ch)`
    // into 「不認得」 — and `:render f`, `:buffer n`, `:clipboard y`, `:help t`
    // and a dozen more had never worked at all.
    // **The table first** (#368). A command that declares what it takes is
    // read by the one walk every other one is read by; what is left below is
    // the families that have not been moved over yet.
    if let Some(entry) = entry_named(word) {
        if let Some(answer) = read_params(entry, rest, word.ends_with('!')) {
            return answer;
        }
    }

    // **Nothing is left below** (#368). Every command is read by the one
    // walk, from its own declaration; what stands here is the answer for a
    // word that names none of them.
    Err(CommandError::Unknown(word.to_string()))
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
    /// `:convert-opencc install` / `:convert-opencc update`.
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
    /// What may follow it, in the order it is typed (#368).
    pub params: &'static [Param],
    /// **What it means** — the one place a command's meaning is written down.
    ///
    /// `None` while a family still goes through the hand-written arms of
    /// [`parse`]; when the last one is gone, so is the `Option` and so are
    /// they.
    pub build: Option<fn(&Parsed) -> Result<Command, CommandError>>,
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

/// One parameter a command takes, in the order it is typed (#368).
///
/// **A command's name is one word and its parameters follow it**, the way a
/// function's name is one word and its arguments follow it — `:view-wrap on`
/// is `view_wrap(on)`. The tree this replaced could not say that: `:view-wrap`
/// and `:write-all` were the same shape as `:write 第三章.md`, so a word list
/// that swallowed a path had to be spelled `Args::PathOr` and 中文 file names
/// were nearly lost to it (#225).
#[derive(Clone, Copy)]
pub enum Param {
    /// One of these words.
    ///
    /// `default` is what the command means with the parameter left off, and
    /// **a value list should nearly always have one**: a setting that cannot
    /// answer 「which am I now」 is a setting with a hole in it. `None` makes
    /// the parameter required, which is right for `:open`'s path and wrong for
    /// `:render`'s level.
    Words {
        of: &'static [Word],
        default: Option<&'static str>,
    },
    /// One of these words, **or anything else** — `or` is the placeholder to
    /// show for the anything.
    ///
    /// Not a mixture for its own sake: `:view-wrap` takes `on｜off｜0` and it
    /// also takes a measure, and a reader who cannot see the three words has
    /// to be told them. This is what `Args::PathOr` was, said once for every
    /// command that needs it rather than once for `:write`.
    WordsOr {
        of: &'static [Word],
        default: Option<&'static str>,
        or: &'static str,
    },
    /// A path, completed from the file system by the caller.
    Path,
    /// One of the input schemes — **whichever ones are installed** (#169).
    Schemes,
    /// Anything at all; the string is the placeholder to show while typing.
    /// Takes the whole of what is left, so it may hold spaces.
    Free(&'static str),
}

impl Param {
    /// The words this parameter may be, as they stand now — `None` when it is
    /// not a word list at all.
    ///
    /// **Every reader of a word list goes through here**, and that is the point
    /// rather than a convenience: `Param::Schemes` is answered from a registry
    /// that a `match` arm on `Param::Words` would silently skip, and skipping
    /// it means a scheme that is installed does not appear.
    pub fn words(&self) -> Option<&'static [Word]> {
        match self {
            Param::Words { of, .. } | Param::WordsOr { of, .. } => Some(of),
            Param::Schemes => Some(schemes()),
            _ => None,
        }
    }

    /// What this parameter defaults to when it is left off, if anything.
    pub fn default(&self) -> Option<&'static str> {
        match self {
            Param::Words { default, .. } | Param::WordsOr { default, .. } => *default,
            _ => None,
        }
    }

    /// Whether it takes the whole of what is left rather than one word.
    fn takes_the_rest(&self) -> bool {
        matches!(self, Param::Path | Param::Free(_))
    }

    /// What may follow, for a listing: `<檔名>`, `on|off`, or a placeholder.
    pub fn hint(&self) -> String {
        if let Param::WordsOr { of, or, .. } = self {
            let mut out: String = of.iter().map(|w| w.name).collect::<Vec<_>>().join("｜");
            out.push('｜');
            out.push_str(or);
            return out;
        }
        match self.words() {
            Some(words) => words
                .iter()
                .map(|w| w.name)
                .collect::<Vec<_>>()
                .join("｜"),
            None => match self {
                Param::Path => say!("cmd.arg.path"),
                Param::Free(what) => (*what).to_string(),
                _ => String::new(),
            },
        }
    }
}

/// A command line after the walk has read it: which command, and what stood in
/// each of its declared parameters.
///
/// Handed to [`Entry::build`], which is the **only** place a command's meaning
/// is written down. What a parameter may be is declared once, checked once,
/// and refused once — so `:view-wrap sideways` is answered in the same words
/// as `:render sideways`, which forty-four hand-written arms never managed.
pub struct Parsed<'a> {
    /// The command's own name, as the table spells it.
    pub name: &'static str,
    /// Whether a `!` was typed after the name.
    pub force: bool,
    /// One entry per declared parameter, written out in full — a word list's
    /// own spelling, or the text as it was typed.
    args: Vec<&'a str>,
    /// Whatever was left when the declared parameters ran out. Empty for
    /// every regular command; the irregular few read it themselves.
    pub rest: &'a str,
}

impl<'a> Parsed<'a> {
    /// What stood in parameter `i`, or `None` if it was left off.
    pub fn arg(&self, i: usize) -> Option<&'a str> {
        self.args.get(i).copied().filter(|a| !a.is_empty())
    }

    /// What stood in parameter `i`, refusing to guess if it was left off.
    pub fn need(&self, i: usize) -> Result<&'a str, CommandError> {
        self.arg(i).ok_or(CommandError::MissingArgument(self.name))
    }

    /// Parameter `i` as a number, in the command's own name if it is not one.
    pub fn number(&self, i: usize) -> Result<usize, CommandError> {
        let text = self.need(i)?;
        text.parse().map_err(|_| CommandError::InvalidArgument {
            command: self.name,
            value: text.to_string(),
        })
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
        })
        .collect();
    let _ = FOUND_SCHEMES.set(Box::leak(words.into_boxed_slice()));
}

/// The schemes `:yume-scheme` offers: what was found, else the built-in five.
pub fn schemes() -> &'static [Word] {
    FOUND_SCHEMES.get().copied().unwrap_or(SCHEMES)
}

/// The note that goes beside a word of `args`, when there is one (#291).
///
/// **Only the scheme registry answers**, and only when it has been filled: the
/// rows [`set_schemes`] builds keep the scheme's *name* in `help` — a name and
/// not a message key — because the front end was handed both and a tag says
/// nothing on its own. The built-in five fall back to `SCHEMES`, whose `help`
/// **is** a key, and a raw key beside a row is worse than no note at all.
///
/// Everything else in the tree is a word that says what it is, and a note
/// repeating it would be noise on every row to save one.
fn note_for(param: &Param, word: &Word) -> Option<&'static str> {
    match matches!(param, Param::Schemes) && FOUND_SCHEMES.get().is_some() {
        true => Some(word.help),
        false => None,
    }
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
    },
    Word {
        name: "png",
        help: "cmd.shot.png",
        needs: &[],
    },
    Word {
        name: "html",
        help: "cmd.shot.html",
        needs: &[],
    },
    Word {
        name: "txt",
        help: "cmd.shot.txt",
        needs: &[],
    },
];

/// …and the one word a footnote takes.
const FOOTNOTE_KINDS: &[Word] = &[Word {
    name: "inline",
    help: "cmd.footnote-kinds.inline",
    needs: &[],
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
    Word { name: "chinese", help: "help.chinese.section-summary", needs: &[] },
    Word { name: "vertical", help: "help.vertical.title", needs: &[] },
    Word { name: "table", help: "help.table.section-summary", needs: &[] },
    Word { name: "commands", help: "help.common.every-command", needs: &[] },
];

/// One word a command accepts, and what may follow *it*.
pub struct Word {
    pub name: &'static str,
    pub help: &'static str,
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
    /// The margin exists — `:view-margin never` leaves nowhere to draw this.
    Margin,
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
            Need::Margin => say!("need.margin"),
            Need::Table => say!("need.table"),
            Need::Scheme => say!("need.scheme"),
        }
    }

    /// The command that brings it about, for the message and for `force`.
    pub fn how(self) -> &'static str {
        match self {
            Need::Vertical => ":layout vertical",
            Need::Margin => ":view-margin dense",
            Need::Table => ":table",
            Need::Scheme => ":yume-scheme",
        }
    }
}

/// Read a line against what its command says it takes (#368).
///
/// **One walk for every command**, so that what a parameter may be is declared
/// once, checked once and refused once. Forty-four hand-written arms each
/// invented their own answer to 「that is not one of the words」, and each one
/// was a place the table could drift away from without anybody noticing — the
/// menu printed `check (ch)` beside words `:table ch` did not accept, and had
/// done for a year.
///
/// `None` when the command has not been moved over yet.
/// `on｜off｜toggle` as the editor's own `Option<bool>`: **the other way** is
/// 「I am not saying which, turn it round」, which is `None`.
fn switched(p: &Parsed) -> Result<Option<bool>, CommandError> {
    Ok(match p.need(0)? {
        "toggle" => None,
        word => Some(word == "on"),
    })
}

fn read_params(
    entry: &'static Entry,
    rest: &str,
    force: bool,
) -> Option<Result<Command, CommandError>> {
    let build = entry.build?;
    let mut left = rest.trim();
    let mut args: Vec<&str> = Vec::with_capacity(entry.params.len());
    for param in entry.params {
        // **The last kind of parameter eats the line.** A path may hold
        // spaces, and so may the text of a `:grep`; a word never does.
        if param.takes_the_rest() {
            args.push(left);
            left = "";
            continue;
        }
        let (head, tail) = match left.split_once(char::is_whitespace) {
            Some((head, tail)) => (head, tail.trim_start()),
            None => (left, ""),
        };
        if head.is_empty() {
            // Left off: the default if it has one, and otherwise nothing —
            // `build` decides whether it could do without it.
            args.push(param.default().unwrap_or(""));
            continue;
        }
        match (param.words(), matches!(param, Param::WordsOr { .. })) {
            // **The prefix rule is the walk's, not the parser's.** The menu
            // prints the shortest unambiguous spelling of every word; that
            // parenthesis is a promise, and this is where it is kept.
            (Some(list), open) => match pick(head, list) {
                Some(word) => args.push(word.name),
                None if open => args.push(head),
                None => {
                    return Some(Err(CommandError::InvalidArgument {
                        command: entry.name,
                        value: head.to_string(),
                    }))
                }
            },
            (None, _) => args.push(head),
        }
        left = tail;
    }
    // ⚠️ **Text nobody asked for is a typo, not a decoration.** Every regular
    // command has read what it declared by now, and `rest` is read by nothing
    // in the tree — so anything still standing here was silently dropped.
    // `:word show off` printed the report and left the tinting on; `:buffer
    // next` opened the picker and swallowed `next`; both look like the command
    // worked. The manual taught those spellings, which is how they got typed.
    if !left.is_empty() {
        // When the leftover's first word is this command's own subcommand
        // spelled with a space, say the spelling instead of just「多了」——
        // that is the mistake the manual taught, and the reader is one
        // hyphen from what they meant.
        let head = left.split_whitespace().next().unwrap_or(left);
        let hyphenated = format!("{}-{head}", entry.name);
        if COMMANDS.iter().any(|e| e.name == hyphenated) {
            return Some(Err(CommandError::MeansTheHyphenatedOne {
                command: entry.name,
                word: head.to_string(),
            }));
        }
        return Some(Err(CommandError::TakesNoArgument {
            command: entry.name,
            value: left.to_string(),
        }));
    }
    Some(build(&Parsed {
        name: entry.name,
        force,
        args,
        rest: left,
    }))
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
        let banged: Vec<&Entry> = COMMANDS
            .iter()
            .filter(|e| {
                (e.name.starts_with(stem) || e.aliases.iter().any(|a| a.starts_with(stem)))
                    && forceable(e.name).is_some()
            })
            .collect();
        if let [only] = banged.as_slice() {
            return forceable(only.name).unwrap_or(word);
        }
        // **And the head wins here too** (#368): `:qu!` reaches `quit!` and
        // `quit-all!`, and the same hierarchy settles it — a spelling that
        // reaches a family's head reaches the head.
        if let Some(head) = banged.iter().find(|e| {
            banged.iter().all(|o| {
                o.name == e.name
                    || (o.name.len() > e.name.len() + 1
                        && o.name.starts_with(e.name)
                        && o.name.as_bytes()[e.name.len()] == b'-')
            })
        }) {
            return forceable(head.name).unwrap_or(word);
        }
        return word;
    }
    let hits: Vec<&Entry> = COMMANDS.iter().filter(|e| e.name.starts_with(word)).collect();
    if let [only] = hits.as_slice() {
        return only.name;
    }
    // **A head is not ambiguous with its own family** (#368). `:tab` matches
    // `table` and the twelve `table-…` beside it, and it has always meant the
    // first of them: the hyphen says the others are *under* it — 「命令雖然
    // 現在變成了 hyphen 連接的詞，但本質上還是有級別的」 (2026-09-10)
    // — so a spelling that reaches the head reaches the head.
    let child_of = |head: &str, name: &str| {
        name.len() > head.len() + 1 && name.starts_with(head) && name.as_bytes()[head.len()] == b'-'
    };
    match hits
        .iter()
        .find(|e| hits.iter().all(|o| o.name == e.name || child_of(e.name, o.name)))
    {
        Some(stem) => stem.name,
        None => word,
    }
}

/// What `:export` writes, and what follows the format — **the file name comes
/// second** (§5.2.2 fault 7).
///
/// The commands that take a `!`, so a prefix of one can too — spelled the way
/// `parse` reads them back, because that spelling is the other half of the
/// answer and a second list of it goes stale.
///
/// It did: `recover` was in neither, so `:recover!` worked and `:rec!`
/// `:recov!` `:recove!` were all 「沒有這個命令」 while `:rec` was fine. One
/// list cannot drift from itself.
const FORCEABLE: &[&str] = &[
    // Spelled as a whole line, because the fold moved this bang off the head
    // and onto a word: `:bclose!` is now `:buffer-close!`, and a bang belongs
    // to the line it ends, not to `buffer`.
    "buffer-close!",
    "write!",
    "quit-all!",
    "reload!",
    "quit!",
    "export!",
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
/// who says `:table-pipe " "` has looked at their data and decided.
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
    // **A name's own family is not competition** (#368). `y` reaches `yume`
    // and the eight `yume-…` under it, and `resolve` settles that the same way
    // a reader would: a spelling that reaches the head reaches the head. So
    // the shortest spelling is measured against everything *except* what is
    // beneath this name.
    let others: Vec<&str> = among
        .filter(|&o| o != name)
        .filter(|o| !(o.len() > name.len() + 1 && o.starts_with(name) && o.as_bytes()[name.len()] == b'-'))
        .collect();
    for (at, _) in name.char_indices().skip(1) {
        if !others.iter().any(|o| o.starts_with(&name[..at])) {
            return Some(&name[..at]);
        }
    }
    None
}

/// The one word of `from` that `typed` names, exactly or by prefix.
///
/// `:yume s l` is `:yume-scheme lingming` — because `s` is the only word there
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
    /// How many commands stand **under** this one, folded away (#369).
    ///
    /// `Some(12)` on the row that stands for the whole `view-…` family while
    /// nothing has narrowed it. The hyphen says a name has levels — 「`view-aaaa`
    /// 就是 `view` 的二級命令」 — and a menu is glanced at rather than read, so
    /// the levels are what it folds along. Nothing is *declared* for this: it
    /// is read off the names, which is why there is still one place a command
    /// says what it is called.
    pub family: Option<usize>,
    pub name: &'static str,
    /// The other spellings of this same command, joined for the brackets.
    ///
    /// **All of them, not the first.** `write-quit` answers to `:wq` and to
    /// vi's `:x`, and `x` appears nowhere else in the menu — it is an alias of
    /// an entry rather than a line of its own — so a bracket showing only the
    /// first would be the only place it could have been found, showing half.
    pub alias: Option<String>,
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
    /// A word beside the name, for the rows whose name does not say what they
    /// are (#291).
    ///
    /// **Not every row wants one** — 「on」 beside 「on」 is noise, and the
    /// footer already explains whatever is highlighted. The row that needs one
    /// is the row whose name is an *identifier*: a scheme somebody installed
    /// is `custom.6947b838` on the line and 「冰雪清韻」 in their head, and a
    /// list of eight hexadecimal names is a list you have to Tab through to
    /// read. The tag stays the value — it is stable and it cannot collide —
    /// and the name comes along beside it, in the quiet ink.
    pub note: Option<&'static str>,
}

impl Choice {
    /// What Tab writes for this choice, and what the menu shows.
    ///
    /// A promoted child carries its parent with it: picking `scheme` out of
    /// `:yume`'s list has to leave `:yume-scheme` on the line, not `:scheme`.
    pub fn written(&self) -> String {
        match self.under.is_empty() {
            true => self.name.to_string(),
            false => format!("{} {}", self.under, self.name),
        }
    }

    /// The **other** spellings worth printing beside this one, if any.
    ///
    /// Not all of them (2026-09-10): a menu is read, and a column of
    /// parentheses holding `(rec)` `(rel)` `(red)` `(lay)` `(sho)` is a column
    /// of noise. Two rules, and between them they keep only what a reader
    /// could not have worked out:
    ///
    /// * **Two characters or fewer** is worth knowing however it was arrived
    ///   at — `(w)` `(q)` `(th)` — because nobody wants to type the other
    ///   five letters and nothing else says they need not.
    /// * **A different word** is worth knowing at any length: `wc` is 「word
    ///   count」, `ro` is not how `readonly` begins, `outline` is what `:toc`
    ///   is called elsewhere, and `fmt` is a contraction somebody chose. None
    ///   of these could be guessed from the name.
    ///
    /// What that leaves out is a **clipped name** longer than two — `syn`,
    /// `red`, `rel` — which says only 「the first three letters work」, and the
    /// prefix rule already says that about every command.
    pub fn spelt(&self) -> Option<String> {
        let worth = |spelling: &str| spelling.chars().count() <= 2 || !self.name.starts_with(spelling);
        let declared: Vec<&str> = self
            .alias
            .iter()
            .flat_map(|all| all.split(' '))
            .filter(|a| worth(a))
            .collect();
        match declared.is_empty() {
            false => Some(declared.join(" ")),
            true => self.short.filter(|s| worth(s)).map(str::to_string),
        }
    }

    /// The whole row, **as a menu draws it** (#369).
    ///
    /// Told apart from [`Choice::written`] because Tab writes one of those
    /// onto the command line and a menu draws one of these: `:table  (ta) +12`
    /// says what is there, and is not something anybody can type.
    pub fn shown(&self) -> String {
        let mut out = self.written();
        if let Some(spelt) = self.spelt() {
            out.push_str(&format!(" ({spelt})"));
        }
        if let Some(more) = self.family {
            out.push_str(&format!(" +{more}"));
        }
        out
    }
}

/// The words a command takes, shown under the command itself.
///
/// Typing `:yume` used to answer with one entry — `:yume` — and a reader had
/// no way to find out from there that the input method's whole set of commands
/// lives under that word. The list says so: the command, and then everything
/// it takes, spelled the way you would type it.
fn children(under: &'static str, param: &Param) -> Vec<Choice> {
    match param.words() {
        Some(list) => list
            .iter()
            .map(|w| Choice {
                name: w.name,
                family: None,
                needs: w.needs,
                alias: None,
                // The short form of a word is only short beside its siblings;
                // spelled out under its parent it would be a second way to
                // read the same row.
                short: None,
                help: w.help,
                leading: "",
                under: under.to_string(),
                note: note_for(param, w),
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
    param: &'static Param,
    path: &[&'static str],
    typed: &str,
    leading: &'static str,
    out: &mut Vec<Choice>,
) {
    // The `Param` rather than the list it holds, because a note is a property
    // of *where the words came from* and not of the words (#291) — `note_for`
    // has to be able to ask.
    let Some(list) = param.words() else {
        return;
    };
    for w in list {
        let note = note_for(param, w);
        if w.name.starts_with(typed) || note.is_some_and(|n| n.contains(typed)) {
            out.push(Choice {
                name: w.name,
                family: None,
                needs: w.needs,
                alias: None,
                // Same reason as `children`: a short form is only short beside
                // its siblings, and this row is being read beside its path.
                short: None,
                help: w.help,
                leading,
                under: path.join(" "),
                note,
            });
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
        // **Every parameter, not only the first.** `:vert` reaches
        // `:layout vertical` because `vertical` is what `:layout`'s one
        // parameter may be; a mood is what `:theme`'s *second* may be, and a
        // reader typing `dark` means that just as plainly.
        let before = out.len();
        for param in e.params {
            deep(param, &[e.name], typed, ":", &mut out);
        }
        // `:convert` reads a side into both of its slots, so the same word
        // stands in two of them — and it is still one word.
        let mut seen: Vec<&str> = Vec::new();
        let tail: Vec<Choice> = out
            .split_off(before)
            .into_iter()
            .filter(|c| match seen.contains(&c.name) {
                true => false,
                false => {
                    seen.push(c.name);
                    true
                }
            })
            .collect();
        out.extend(tail);
    }
    let about = |c: &Choice| c.under.starts_with("help");
    out.sort_by(|a, b| {
        (about(a), a.under.split(' ').count(), &a.under, a.name)
            .cmp(&(about(b), b.under.split(' ').count(), &b.under, b.name))
    });
    out
}

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
    },
    Word {
        name: "column",
        help: "cmd.axis.column",
        needs: &[],
    },
];

/// How much of the result the page shows.
/// How loudly the editor talks beside the caret (Feature #284).
const HUD: &[Word] = &[
    Word {
        name: "off",
        help: "cmd.hud.off",
        needs: &[],
    },
    Word {
        name: "basic",
        help: "cmd.hud.basic",
        needs: &[],
    },
    Word {
        name: "full",
        help: "cmd.hud.full",
        needs: &[],
    },
];

const RENDER: &[Word] = &[
    Word {
        name: "off",
        help: "cmd.render.off",
        needs: &[],
    },
    Word {
        name: "basic",
        help: "cmd.render.basic",
        needs: &[],
    },
    Word {
        name: "full",
        help: "cmd.render.full",
        needs: &[],
    },
];

/// What `:view-wrap` takes.
const WRAP: &[Word] = &[
    Word {
        name: "on",
        help: "cmd.wrap.on",
        needs: &[],
    },
    Word {
        name: "off",
        help: "cmd.wrap.off",
        needs: &[],
    },
    Word {
        name: "0",
        help: "cmd.wrap.0",
        needs: &[],
    },
];

/// The input schemes yume ships with.
/// The three 上屏方式 — *when* a finished code goes to the page.
///
/// The names are yume's own tags (`CommitStrategy::from_str_tag`), so that a
/// setting written here means the same thing in the input method's own panel
/// on macOS and Windows. `auto` is taken as 唯一 because that is what the
/// habit calls it — 自動上屏.
/// How much of 靈明 is answering — `:yume` on its own means the first of them.
const ENGAGEMENT: &[Word] = &[
    Word { name: "on", help: "cmd.yume.on", needs: &[] },
    Word { name: "abc", help: "cmd.yume.abc", needs: &[] },
    Word { name: "off", help: "cmd.yume.off", needs: &[] },
];

const COMMITS: &[Word] = &[
    Word {
        name: "delayed",
        help: "cmd.commits.delayed",
        needs: &[],
    },
    Word {
        name: "unique",
        help: "cmd.commits.unique",
        needs: &[],
    },
    Word {
        name: "fluency",
        help: "cmd.commits.fluency",
        needs: &[],
    },
];

const PANELS: &[Word] = &[
    Word {
        name: "full",
        help: "cmd.panels.full",
        needs: &[],
    },
    Word {
        name: "bare",
        help: "cmd.panels.bare",
        needs: &[],
    },
];

const SCHEMES: &[Word] = &[
    Word {
        name: "lingming",
        help: "cmd.schemes.lingming",
        needs: &[],
    },
    Word {
        name: "xingchen",
        help: "cmd.schemes.xingchen",
        needs: &[],
    },
    Word {
        name: "qingyun",
        help: "cmd.schemes.qingyun",
        needs: &[],
    },
    Word {
        name: "riyue",
        help: "cmd.schemes.riyue",
        needs: &[],
    },
    Word {
        name: "pinyin",
        help: "cmd.schemes.pinyin",
        needs: &[],
    },
];

/// The languages a file may be read as.
const SYNTAXES: &[Word] = &[
    Word {
        name: "markdown",
        help: "cmd.syntaxes.markdown",
        needs: &[],
    },
    Word {
        name: "typst",
        help: "cmd.syntaxes.typst",
        needs: &[],
    },
    Word {
        name: "text",
        help: "cmd.syntaxes.text",
        needs: &[],
    },
    // A file that is code (#420): coloured by its grammar, no markup.
    Word {
        name: "python",
        help: "cmd.syntaxes.code",
        needs: &[],
    },
    Word {
        name: "javascript",
        help: "cmd.syntaxes.code",
        needs: &[],
    },
    Word {
        name: "json",
        help: "cmd.syntaxes.code",
        needs: &[],
    },
    Word {
        name: "yaml",
        help: "cmd.syntaxes.code",
        needs: &[],
    },
    Word {
        name: "toml",
        help: "cmd.syntaxes.code",
        needs: &[],
    },
    Word {
        name: "html",
        help: "cmd.syntaxes.code",
        needs: &[],
    },
    Word {
        name: "css",
        help: "cmd.syntaxes.code",
        needs: &[],
    },
];

/// Which way the page runs.
/// `:theme` and what may follow it.
///
/// What is drawn in a paragraph's opening squares.
/// How much 首行縮進 is drawn — the same three levels `:render` has.
const INDENT_LEVELS: &[Word] = &[
    Word { name: "off", help: "cmd.indent.off", needs: &[] },
    Word { name: "basic", help: "cmd.indent.basic", needs: &[] },
    Word { name: "full", help: "cmd.indent.full", needs: &[] },
];

/// The shipped keymaps (#428).
const KEYMAPS: &[Word] = &[
    Word { name: "helix", help: "cmd.keymaps.helix", needs: &[] },
    Word { name: "vim", help: "cmd.keymaps.vim", needs: &[] },
];

/// What Tab types in Insert mode.
const TAB_INSERTS: &[Word] = &[
    Word { name: "spaces", help: "cmd.tabs.spaces", needs: &[] },
    Word { name: "tab", help: "cmd.tabs.tab", needs: &[] },
];

const HINTS: &[Word] = &[
    Word {
        name: "none",
        help: "cmd.hints.none",
        needs: &[],
    },
    Word {
        name: "color",
        help: "cmd.hints.color",
        needs: &[],
    },
    Word {
        name: "symbol",
        help: "cmd.hints.symbol",
        needs: &[],
    },
];

/// How a table's columns are told apart.
/// How much of a `|` table is drawn — the same three levels `:render` has,
/// in this dimension's own words.
const TABLE_LEVELS: &[Word] = &[
    Word { name: "off", help: "cmd.table.off", needs: &[] },
    Word { name: "basic", help: "cmd.table.basic", needs: &[] },
    Word { name: "full", help: "cmd.table.full", needs: &[] },
];

const RULES: &[Word] = &[
    Word {
        name: "off",
        help: "cmd.rules.off",
        needs: &[],
    },
    Word {
        name: "color",
        help: "cmd.rules.color",
        needs: &[],
    },
    Word {
        name: "line",
        help: "cmd.rules.line",
        needs: &[],
    },
];

/// Which line `:table-rules line` draws.
const STROKES: &[Word] = &[
    Word {
        name: "solid",
        help: "cmd.strokes.solid",
        needs: &[],
    },
    Word {
        name: "dash",
        help: "cmd.strokes.dash",
        needs: &[],
    },
    Word {
        name: "double",
        help: "cmd.strokes.double",
        needs: &[],
    },
];

/// Dark, light, or the terminal's answer.
///
/// **A theme and an appearance are two questions**, and mixing them in one
/// list made `:theme` offer 「moxiang、heibai、system、dark、light」 as though
/// they were five of a kind. They are not: one says which set of inks, the
/// other says which way round they go. `:theme` asks the second on its
/// own, and `:theme moxiang dark` still asks both in one line.
/// The mood words sit at both levels, so `:theme dark` and `:theme moxiang
/// dark` are both sentences — the theme's name is worth saying and worth
/// leaving out, and neither should be a special case.
const THEMES: &[Word] = &[
    // ⚠️ **The three moods are not here** (#449). They were, because `:theme`
    // took them as well — and then 「主題」 meant both 「哪一套墨」 and
    // 「深還是淺」 on one menu. They live under `:theme-mode` now ([`MOODS`]).
    Word {
        // 墨香
        name: "ink",
        help: "cmd.themes.ink",
        needs: &[],
    },
    Word {
        // 黑白
        name: "bw",
        help: "cmd.themes.bw",
        needs: &[],
    },
    Word {
        // 藍曬
        name: "cyanotype",
        help: "cmd.themes.cyanotype",
        needs: &[],
    },
    Word {
        // 琥珀
        name: "amber",
        help: "cmd.themes.amber",
        needs: &[],
    },
    Word {
        // 莫高
        name: "mogao",
        help: "cmd.themes.mogao",
        needs: &[],
    },
    Word {
        // 莫蘭迪
        name: "morandi",
        help: "cmd.themes.morandi",
        needs: &[],
    },
    Word {
        // 夜螢
        name: "firefly",
        help: "cmd.themes.firefly",
        needs: &[],
    },
    Word {
        // 明度階
        name: "meridian",
        help: "cmd.themes.meridian",
        needs: &[],
    },
    Word {
        // 陶窯
        name: "kiln",
        help: "cmd.themes.kiln",
        needs: &[],
    },
    Word {
        // 靛橘
        name: "complement",
        help: "cmd.themes.complement",
        needs: &[],
    },
];

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
    },
    Word {
        name: "typst",
        help: "cmd.export.typst",
        needs: &[],
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
    },
    Word {
        name: "tsv",
        help: "cmd.export.tsv",
        needs: &[],
    },
];

const MOODS: &[Word] = &[
    Word {
        name: "system",
        help: "cmd.moods.system",
        needs: &[],
    },
    Word {
        name: "dark",
        help: "cmd.moods.dark",
        needs: &[],
    },
    Word {
        name: "light",
        help: "cmd.moods.light",
        needs: &[],
    },
];

/// The three languages the editor speaks (#494).
const LANGUAGES: &[Word] = &[
    Word {
        name: "zh",
        help: "cmd.languages.zh",
        needs: &[],
    },
    Word {
        name: "zhs",
        help: "cmd.languages.zhs",
        needs: &[],
    },
    Word {
        name: "en",
        help: "cmd.languages.en",
        needs: &[],
    },
];

const LAYOUTS: &[Word] = &[
    Word {
        name: "vertical",
        help: "cmd.layouts.vertical",
        needs: &[],
    },
    Word {
        name: "horizontal",
        help: "cmd.layouts.horizontal",
        needs: &[],
    },
];

/// What `:word-show` may be given — **four words, not two**.
///
/// The parser has always taken `tint` and `ink` here (and 底色／字色, which
/// `WordMark::parse` reads), while the table declared `ON_OFF`: so the two
/// drawings ran, and the menu that exists to say what may follow `show` never
/// mentioned them. Same shape as §5.2.2 fault 7 — a word the editor accepts
/// and no reader can find is a word only its author has.
/// How much of a reading is drawn — the same three levels `:render` has.
const RUBY_LEVELS: &[Word] = &[
    Word { name: "off", help: "cmd.ruby.off", needs: &[] },
    Word { name: "basic", help: "cmd.ruby.basic", needs: &[] },
    Word { name: "full", help: "cmd.ruby.full", needs: &[] },
];

/// What `:ruby-auto` may be asked for beyond the common readings.
const RARE: &[Word] = &[
    Word { name: "rare", help: "cmd.ruby.auto.rare", needs: &[] },
];

/// The dialects `:ruby-format` writes every reading into.
const DIALECTS: &[Word] = &[
    Word { name: "html", help: "cmd.ruby.format.html", needs: &[] },
    Word { name: "typst", help: "cmd.ruby.format.typst", needs: &[] },
];

const WORD_SHOW: &[Word] = &[
    Word {
        name: "on",
        help: "cmd.on-off.on",
        needs: &[],
    },
    Word {
        name: "off",
        help: "hint.close",
        needs: &[],
    },
    Word {
        name: "tint",
        help: "cmd.word-show.tint",
        needs: &[],
    },
    Word {
        name: "ink",
        help: "cmd.word-show.ink",
        needs: &[],
    },
    Word {
        name: "color",
        help: "cmd.word-show.color",
        needs: &[],
    },
    Word {
        name: "line",
        help: "cmd.word-show.line",
        needs: &[],
    },
];

const WORD_LISTS: &[Word] = &[
    Word {
        name: "reload",
        help: "cmd.word-lists.reload",
        needs: &[],
    },
    Word {
        name: "edit",
        help: "cmd.word-lists.edit",
        needs: &[],
    },
    Word {
        name: "global",
        help: "cmd.word-lists.global",
        needs: &[],
    },
];

/// What `:wiki` takes.
const WIKI_WORDS: &[Word] = &[
    Word { name: "edit", help: "cmd.wikis.edit", needs: &[] },
    Word { name: "global", help: "cmd.wikis.global", needs: &[] },
    Word { name: "panel", help: "cmd.wikis.panel", needs: &[] },
    Word { name: "reload", help: "cmd.wikis.reload", needs: &[] },
];

const WORD_LEVELS: &[Word] = &[
    Word {
        name: "off",
        help: "cmd.word-levels.off",
        needs: &[],
    },
    Word {
        name: "strict",
        help: "cmd.word-levels.strict",
        needs: &[],
    },
    Word {
        name: "balanced",
        help: "cmd.word-levels.balanced",
        needs: &[],
    },
    Word {
        name: "full",
        help: "cmd.word-levels.full",
        needs: &[],
    },
];

/// What `:check` can be asked to look over (Feature #233).
///
/// What `:convert-opencc` can be asked to do.
/// The two halves of a 簡繁 conversion — which writing it is now, and which
/// it should become.
const SIDES: &[Word] = &[
    Word { name: "s", help: "cmd.convert.s", needs: &[] },
    Word { name: "t", help: "cmd.convert.t", needs: &[] },
    Word { name: "tw", help: "cmd.convert.tw", needs: &[] },
    Word { name: "hk", help: "cmd.convert.hk", needs: &[] },
    Word { name: "jp", help: "cmd.convert.jp", needs: &[] },
    Word { name: "c", help: "cmd.convert.c", needs: &[] },
    Word { name: "g", help: "cmd.convert.g", needs: &[] },
];

/// The word that says 「yes, the whole file, I mean it」.
const FORCE: &[Word] = &[
    Word { name: "force", help: "cmd.convert.force", needs: &[] },
];

const OPENCC: &[Word] = &[
    Word {
        name: "install",
        help: "cmd.opencc.install",
        needs: &[],
    },
    Word {
        name: "update",
        help: "cmd.opencc.update",
        needs: &[],
    },
];

/// The side `:convert` starts from.
///
/// `on｜off`, and **the other way** — which the parser has always accepted
/// and no word list ever said (#368). A word the menu cannot show is a word
/// only the code knows about, which is the drift this table exists to stop.
const SWITCH: &[Word] = &[
    Word {
        name: "on",
        help: "cmd.on-off.on",
        needs: &[],
    },
    Word {
        name: "off",
        help: "hint.close",
        needs: &[],
    },
    Word {
        name: "toggle",
        help: "cmd.on-off.toggle",
        needs: &[],
    },
];

/// Which panel a `:sidebar-show-*` names, or `None` for 「the one I am in」.
///
/// A word nobody knows is an error rather than 「the one I am in」: silently
/// moving the wrong panel is worse than saying the name is not one.
fn named_panel(
    command: &'static str,
    word: Option<&str>,
) -> Result<Option<crate::sidebar::Panel>, CommandError> {
    match word {
        None => Ok(None),
        Some(word) => {
            crate::sidebar::Panel::parse(word)
                .map(Some)
                .ok_or_else(|| CommandError::InvalidArgument {
                    command,
                    value: word.to_string(),
                })
        }
    }
}

/// The two words every switch takes — read back by [`switch`].
/// The four margins, tightest first (`:view-margin`).
const MARGINS: &[Word] = &[
    Word { name: "never", help: "cmd.margins.never", needs: &[] },
    Word { name: "dense", help: "cmd.margins.dense", needs: &[] },
    Word { name: "loose", help: "cmd.margins.loose", needs: &[] },
    Word { name: "always", help: "cmd.margins.always", needs: &[] },
];

const ON_OFF: &[Word] = &[
    Word {
        name: "on",
        help: "cmd.on-off.on",
        needs: &[],
    },
    Word {
        name: "off",
        help: "hint.close",
        needs: &[],
    },
];

/// The five panels a slot can hold, for `:sidebar-show-*` — Feature #293.
const SIDEBAR_PANELS: &[Word] = &[
    Word { name: "files", help: "label.panel.files", needs: &[] },
    Word { name: "buffers", help: "label.panel.buffers", needs: &[] },
    Word { name: "outline", help: "label.panel.outline", needs: &[] },
    Word { name: "dictionary", help: "label.panel.dictionary", needs: &[] },
    Word { name: "detail", help: "label.panel.detail", needs: &[] },
    Word { name: "wiki", help: "label.panel.wiki", needs: &[] },
];

/// Every command, for the completion list.
///
/// This is a second copy of the names in [`parse`], and deliberately so: the
/// parser is a `match` on string literals, which cannot be enumerated. Keeping
/// the list here rather than deriving it means adding a command is two edits —
/// the price of the table being the one place that says what each one is *for*,
/// which is what the user is reading when they cannot remember the name.
/// What `:write` takes besides a path — the two that had to join it without
/// `:w 第三章.md` ceasing to be a path (#225).
pub const WRITE: &[Word] = &[
    Word {
        name: "all",
        help: "cmd.write.all",
        needs: &[],
    },
    Word {
        name: "as",
        help: "cmd.write.as",
        needs: &[],
    },
];

pub const COMMANDS: &[Entry] = &[
    Entry {
        name: "open",
        aliases: &["o", "e", "edit"],
        help: "cmd.commands.open",
        needs: &[],
        params: &[Param::Path],
        build: Some(|p| {
            Ok(match p.arg(0) {
                Some(path) => Command::Open(path.to_string()),
                None => Command::OpenPicker,
            })
        }),
    },
    Entry {
        name: "new",
        aliases: &["enew"],
        help: "cmd.commands.new",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::NewBuffer)),
    },
    Entry {
        name: "write",
        aliases: &["w"],
        help: "cmd.commands.write",
        needs: &[],
        params: &[Param::Path],
        build: Some(|p| {
            // **A path, full stop** (#368). `Args::PathOr` existed only so
            // that `:write-all` could sit beside `:write 第三章.md`, and a
            // word list that swallowed a path nearly took 中文 file names away
            // from the one command that most needs them (#225). With `all`
            // and `as` commands of their own, the ambiguity is gone and so is
            // the shape that carried it.
            // ⚠️ `:write all` is not a path. `WRITE` lists the two words
            // that became commands of their own, and a bare one of them here
            // used to be obeyed as a file name: a copy of the manuscript
            // appeared in the working directory under the name `all`, the
            // status line said 「抄了一份到 all」, and every other modified
            // buffer stayed unsaved. Naming a file `./all` still works.
            if let Some(word) = p.arg(0) {
                if WRITE.iter().any(|w| w.name == word) {
                    return Err(CommandError::MeansTheHyphenatedOne {
                        command: "write",
                        word: word.to_string(),
                    });
                }
            }
            Ok(match p.force {
                true => Command::WriteForce(p.arg(0).map(|s| s.to_string())),
                false => Command::Write(p.arg(0).map(|s| s.to_string())),
            })
        }),
    },
    Entry {
        name: "write-all",
        aliases: &["wa"],
        help: "cmd.write.all",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::WriteAll)),
    },
    Entry {
        name: "write-as",
        aliases: &[],
        help: "cmd.write.as",
        needs: &[],
        params: &[Param::Path],
        build: Some(|p| {
            Ok(Command::SaveAs {
                path: p.need(0)?.to_string(),
                force: p.force,
            })
        }),
    },
    Entry {
        name: "recover",
        aliases: &[],
        help: "cmd.commands.recover",
        needs: &[],
        params: &[],
        build: Some(|p| Ok(Command::Recover { discard: p.force })),
    },
    Entry {
        name: "reload",
        aliases: &[],
        help: "cmd.commands.reload",
        needs: &[],
        params: &[],
        build: Some(|p| Ok(Command::Reload { force: p.force })),
    },
    Entry {
        name: "reload-auto",
        aliases: &[],
        help: "cmd.reload.auto",
        needs: &[],
        params: &[Param::Words { of: ON_OFF, default: None }],
        build: Some(|p| {
            // The bang answers 「throw this buffer's changes away」, which is
            // not a question `auto` asks — so `:reload-auto! on` is a typo,
            // not a setting, and saying so beats obeying half of it.
            match p.force {
                true => Err(CommandError::InvalidArgument {
                    command: "reload-auto",
                    value: "!".to_string(),
                }),
                false => Ok(Command::ReloadAuto(p.arg(0).map(|w| w == "on"))),
            }
        }),
    },
    Entry {
        // 只讀 (Feature #213).
        name: "readonly",
        aliases: &["ro"],
        help: "cmd.commands.readonly",
        needs: &[],
        params: &[Param::Words {
            of: ON_OFF,
            default: None,
        }],
        build: Some(|p| Ok(Command::SetReadonly(p.arg(0).map(|w| w == "on")))),
    },
    Entry {
        name: "goto",
        aliases: &["g"],
        help: "cmd.commands.goto",
        needs: &[],
        params: &[Param::Free("<行號>")],
        build: Some(|p| {
            p.arg(0)
                .and_then(|n| n.parse().ok())
                .map(Command::GotoLine)
                .ok_or(CommandError::MissingArgument("goto"))
        }),
    },
    Entry {
        name: "count",
        aliases: &["wc"],
        help: "cmd.commands.count",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Count)),
    },
    Entry {
        name: "count-progress",
        aliases: &[],
        help: "cmd.count.progress",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Progress)),
    },
    Entry {
        name: "count-target",
        aliases: &[],
        help: "cmd.count.target",
        needs: &[],
        params: &[Param::Free("<字數>｜off")],
        build: Some(|p| {
            // A bare `:count-target` is the question, not half a command:
            // somebody who types it wants to know what the target is and how
            // far off it is, and that is what `progress` answers.
            match p.arg(0) {
                None => Ok(Command::Progress),
                Some("off" | "none" | "0") => Ok(Command::Target(None)),
                Some(_) => Ok(Command::Target(Some(p.number(0)?))),
            }
        }),
    },
    Entry {
        name: "check-usage",
        aliases: &[],
        help: "cmd.check.usage",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::CheckUsage)),
    },
    Entry {
        name: "check-charset",
        aliases: &[],
        help: "cmd.check.charset",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::CheckCharset)),
    },
    Entry {
        name: "check-merge",
        aliases: &[],
        help: "cmd.check.merge",
        needs: &[],
        params: &[],
        build: Some(|_| {
            // Was `:conflicts` — the subject is a merge, `:check` is the verb
            // (§5.2.3 ④).
            Ok(Command::Conflicts)
        }),
    },
    Entry {
        name: "check-punct",
        aliases: &[],
        help: "cmd.check.punct",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::CheckPunct)),
    },
    Entry {
        name: "convert",
        aliases: &[],
        help: "cmd.commands.convert",
        needs: &[],
        params: &[Param::Words { of: SIDES, default: None }, Param::Words { of: SIDES, default: None }, Param::Words { of: FORCE, default: None }],
        build: Some(|p| {
            // 簡繁 (Feature #241). Two sides and an optional `force`: the pair
            // names an opencc config (`s tw` is `s2tw`), so what the editor
            // runs is readable from what was typed. With neither, it explains
            // which ways it can go.
            let (Some(from), Some(to)) = (p.arg(0), p.arg(1)) else {
                return Ok(Command::Convert(ConvertAsk::Explain));
            };
            match (Side::parse(from), Side::parse(to)) {
                (Some(from), Some(to)) => Ok(Command::Convert(ConvertAsk::Run {
                    from,
                    to,
                    force: p.arg(2).is_some(),
                })),
                (None, _) => Err(CommandError::InvalidArgument {
                    command: "convert",
                    value: from.to_string(),
                }),
                (_, None) => Err(CommandError::InvalidArgument {
                    command: "convert",
                    value: to.to_string(),
                }),
            }
        }),
    },
    Entry {
        name: "convert-opencc",
        aliases: &[],
        help: "cmd.convert.opencc",
        needs: &[],
        params: &[Param::Words { of: OPENCC, default: None }],
        build: Some(|p| {
            Ok(Command::Convert(match p.arg(0) {
                None => ConvertAsk::Explain,
                Some("install") => ConvertAsk::Opencc { update: false },
                _ => ConvertAsk::Opencc { update: true },
            }))
        }),
    },
    Entry {
        name: "quit",
        aliases: &["q"],
        help: "cmd.commands.quit",
        needs: &[],
        params: &[],
        build: Some(|p| Ok(Command::Quit { force: p.force })),
    },
    Entry {
        name: "quit-all",
        aliases: &["qa"],
        help: "cmd.quit.all",
        needs: &[],
        params: &[],
        build: Some(|p| Ok(Command::QuitAll { force: p.force })),
    },
    Entry {
        name: "write-quit",
        aliases: &["wq"],
        help: "cmd.commands.write-quit",
        needs: &[],
        params: &[Param::Path],
        build: Some(|p| Ok(Command::WriteQuit(p.arg(0).map(|s| s.to_string())))),
    },
    // **`:x` is not `:wq`**, and it was an alias of it here. In vi and in helix
    // both, `:x` writes *only when the file changed* — a file opened, read and
    // left alone keeps its timestamp, which is what `make`, rsync and a sync
    // folder go by. `:update` is the same gate without the leaving.
    Entry {
        name: "exit",
        aliases: &["x", "xit"],
        help: "cmd.commands.exit",
        needs: &[],
        params: &[Param::Path],
        build: Some(|p| Ok(Command::Exit(p.arg(0).map(|s| s.to_string())))),
    },
    Entry {
        name: "update",
        aliases: &["up"],
        help: "cmd.commands.update",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Update)),
    },

    Entry {
        name: "undo",
        aliases: &["u"],
        help: "cmd.commands.undo",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Undo)),
    },
    Entry {
        name: "redo",
        aliases: &["red"],
        help: "cmd.commands.redo",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Redo)),
    },
    Entry {
        name: "word",
        aliases: &["wd"],
        help: "cmd.commands.word",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Word(WordCommand::Report))),
    },
    Entry {
        name: "word-show",
        aliases: &[],
        help: "cmd.word-topics.show",
        needs: &[],
        params: &[Param::WordsOr {
            of: WORD_SHOW,
            default: None,
            or: "字色｜墨色",
        }],
        build: Some(|p| {
            // `:word-show 字色` is the same question as `:word-show on` — how
            // this is drawn — so it lives under the same command rather than
            // making the writer learn a second one.
            Ok(Command::Word(match p.arg(0) {
                None => WordCommand::Show(None),
                Some("on") => WordCommand::Show(Some(true)),
                Some("off") => WordCommand::Show(Some(false)),
                Some(how) => match yumete_cjk::WordMark::parse(how) {
                    Some(mark) => WordCommand::Mark(mark),
                    None => {
                        return Err(CommandError::InvalidArgument {
                            command: "word-show",
                            value: how.to_string(),
                        })
                    }
                },
            }))
        }),
    },
    Entry {
        name: "wiki",
        aliases: &[],
        help: "cmd.commands.wiki",
        needs: &[],
        params: &[Param::Words { of: WIKI_WORDS, default: None }],
        build: Some(|p| Ok(Command::Wiki(p.arg(0).map(str::to_string)))),
    },
    Entry {
        name: "word-list",
        aliases: &[],
        help: "cmd.word-topics.list",
        needs: &[],
        params: &[Param::Words { of: WORD_LISTS, default: None }],
        build: Some(|p| {
            Ok(Command::Word(match p.arg(0) {
                None => WordCommand::List,
                Some("reload") => WordCommand::Reload,
                Some("edit") => WordCommand::Edit,
                _ => WordCommand::Global,
            }))
        }),
    },
    Entry {
        name: "word-level",
        aliases: &[],
        help: "cmd.word-topics.level",
        needs: &[],
        params: &[Param::Words { of: WORD_LEVELS, default: None }],
        build: Some(|p| {
            Ok(Command::Word(WordCommand::Level(match p.arg(0) {
                None => None,
                Some(name) => match yumete_cjk::WordLevel::parse(name) {
                    Some(level) => Some(level),
                    None => {
                        return Err(CommandError::InvalidArgument {
                            command: "word-level",
                            value: name.to_string(),
                        })
                    }
                },
            })))
        }),
    },
    Entry {
        name: "word-discover",
        aliases: &[],
        help: "cmd.word-topics.discover",
        needs: &[],
        params: &[],
        build: Some(|_| {
            Ok(Command::Word(WordCommand::Discover(
                crate::search_panel::Where::Buffer,
            )))
        }),
    },
    Entry {
        name: "word-discover-cd",
        aliases: &[],
        help: "cmd.word-topics.discover-cd",
        needs: &[],
        params: &[],
        build: Some(|_| {
            Ok(Command::Word(WordCommand::Discover(
                crate::search_panel::Where::Folder,
            )))
        }),
    },
    Entry {
        name: "word-discover-gd",
        aliases: &[],
        help: "cmd.word-topics.discover-gd",
        needs: &[],
        params: &[],
        build: Some(|_| {
            Ok(Command::Word(WordCommand::Discover(
                crate::search_panel::Where::Project,
            )))
        }),
    },
    Entry {
        name: "word-discover-wd",
        aliases: &[],
        help: "cmd.word-topics.discover-wd",
        needs: &[],
        params: &[],
        build: Some(|_| {
            Ok(Command::Word(WordCommand::Discover(
                crate::search_panel::Where::Workspace,
            )))
        }),
    },
    Entry {
        name: "word-habit",
        aliases: &[],
        help: "cmd.word-topics.habit",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Word(WordCommand::Habit))),
    },
    Entry {
        name: "layout",
        aliases: &["lay"],
        help: "cmd.commands.layout",
        needs: &[],
        params: &[Param::WordsOr { of: LAYOUTS, default: None, or: "<竪排｜横排>" }],
        build: Some(|p| {
            // `:layout` alone flips it; the two long forms name the layout
            // outright, in either language.
            match p.arg(0) {
                None => Ok(Command::SetLayout(None)),
                Some(word) => Layout::parse(word)
                    .map(|l| Command::SetLayout(Some(l)))
                    .ok_or_else(|| CommandError::InvalidArgument {
                        command: "layout",
                        value: word.to_string(),
                    }),
            }
        }),
    },
    Entry {
        name: "theme",
        aliases: &[],
        help: "cmd.commands.theme",
        needs: &[],
        params: &[Param::WordsOr { of: THEMES, default: None, or: "<主題名>" }],
        build: Some(|p| {
            // 「墨香」 and `moxiang` are the same word. Which names exist is the
            // front end's business — it is the one holding the colours — so an
            // unknown one is answered there, in a sentence, rather than refused
            // here.
            //
            // ⚠️ **The mood is not here any more** (#449). `:theme light` used
            // to work, and `:theme moxiang dark` too, which made 「主題」 mean
            // two things on one line: the inks, and which way round they go.
            // 「防止和其他的主题混淆」 — so the mood moved out to
            // `:theme-mode`, and this command names a theme and nothing else.
            Ok(Command::Theme { name: p.arg(0).map(|w| w.to_string()), mood: None })
        }),
    },
    Entry {
        name: "theme-mode",
        aliases: &[],
        help: "cmd.commands.theme-mode",
        needs: &[],
        params: &[Param::Words { of: MOODS, default: None }],
        build: Some(|p| {
            let mood = match p.arg(0) {
                Some("system") => Some(Mood::System),
                Some("dark") => Some(Mood::Dark),
                Some("light") => Some(Mood::Light),
                // Bare, it is the question — the front end says which is on.
                _ => None,
            };
            Ok(Command::Theme { name: None, mood })
        }),
    },
    Entry {
        // **Not an alias of anything** (#494). `:run` is the *programming*
        // language's formatter; this is the language the editor speaks, which
        // is the one a reader who cannot read the current one needs to reach —
        // and `yumete --lang=en` is the same answer for a reader who cannot
        // reach the command line either.
        name: "language",
        aliases: &["lang"],
        help: "cmd.commands.language",
        needs: &[],
        params: &[Param::Words { of: LANGUAGES, default: None }],
        build: Some(|p| {
            // Bare, it is the question — the status line says which is on.
            Ok(Command::SayIn(p.arg(0).and_then(crate::messages::Language::parse)))
        }),
    },
    Entry {
        name: "keymap",
        aliases: &[],
        help: "cmd.commands.keymap",
        needs: &[],
        params: &[Param::Words { of: KEYMAPS, default: None }],
        build: Some(|p| Ok(Command::SetKeymap(p.arg(0).and_then(yumete_cjk::KeyPreset::parse)))),
    },
    Entry {
        name: "theme-fill",
        aliases: &[],
        help: "cmd.commands.theme-fill",
        needs: &[],
        params: &[Param::Words { of: ON_OFF, default: None }],
        build: Some(|p| Ok(Command::ThemeFill(p.arg(0).map(|w| w == "on")))),
    },
    Entry {
        name: "shot",
        aliases: &[],
        help: "cmd.commands.shot",
        needs: &[],
        params: &[
            Param::Words {
                of: SHOT,
                default: None,
            },
            Param::Path,
        ],
        build: Some(|p| {
            let named = p.arg(1).map(|path| path.to_string());
            let file = |how| Shot::File { how, path: named };
            let shot = match p.arg(0) {
                // **Bare `:shot` is the screen.** 「截圖」 means a picture you
                // can paste; the drawn page is the specialist and says so.
                None => Shot::Screen,
                // A name after `screen` is refused rather than dropped: the
                // clipboard has nowhere to put one, and a path that quietly
                // did nothing would be worse than a sentence saying so.
                Some("screen") => match p.arg(1) {
                    None => Shot::Screen,
                    Some(path) => {
                        return Err(CommandError::TakesNoArgument {
                            command: "shot screen",
                            value: path.to_string(),
                        })
                    }
                },
                Some("png") => file(ShotFormat::Png),
                Some("html") => file(ShotFormat::Html),
                _ => file(ShotFormat::Text),
            };
            Ok(Command::Screenshot {
                shot,
                force: p.force,
            })
        }),
    },
    Entry {
        name: "yume",
        aliases: &[],
        help: "cmd.commands.yume",
        needs: &[],
        params: &[Param::Words { of: ENGAGEMENT, default: Some("on") }],
        build: Some(|p| {
            // **The bare word turns it on** (#368). It used to report which
            // 靈明 was answering, which `:yume-which` says better; 「開中文」
            // is what a hand reaching for this command nearly always means,
            // and `:y` is then the whole of it.
            Ok(Command::YumeLanguage(match p.need(0)? {
                "on" => Engagement::Chinese,
                "abc" => Engagement::Ascii,
                _ => Engagement::Off,
            }))
        }),
    },
    Entry {
        name: "yume-where",
        aliases: &[],
        help: "cmd.yume.where",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::YumeWhere)),
    },
    Entry {
        name: "yume-which",
        aliases: &[],
        help: "cmd.yume.which",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::YumeStatus)),
    },
    Entry {
        name: "yume-scheme",
        aliases: &[],
        help: "cmd.yume.scheme",
        needs: &[],
        params: &[Param::Schemes],
        build: Some(|p| {
            // No name is 「the one the config asked for」 — `:yume-scheme s`
            // is the whole of starting to type.
            Ok(Command::SetScheme(p.arg(0).unwrap_or("").to_string()))
        }),
    },
    Entry {
        name: "yume-chaifen",
        aliases: &[],
        help: "cmd.yume.chaifen",
        needs: &[Need::Scheme],
        params: &[Param::Words { of: ON_OFF, default: None }],
        build: Some(|p| Ok(Command::SetChaifen(p.arg(0).map(|w| w == "on")))),
    },
    Entry {
        name: "yume-commit",
        aliases: &[],
        help: "cmd.yume.commit",
        needs: &[Need::Scheme],
        params: &[Param::WordsOr {
            of: COMMITS,
            default: None,
            or: "auto",
        }],
        build: Some(|p| {
            // On its own it is the question — which of the three is answering.
            // `auto` is 唯一 under the name people use for it — declared, so
            // the menu can show it, rather than known only to the parser.
            Ok(Command::YumeCommit(match p.arg(0) {
                None => None,
                Some("auto") => Some("unique".to_string()),
                Some(name) if COMMITS.iter().any(|w| w.name == name) => Some(name.to_string()),
                Some(other) => {
                    return Err(CommandError::InvalidArgument {
                        command: "yume-commit",
                        value: other.to_string(),
                    })
                }
            }))
        }),
    },
    Entry {
        name: "yume-panel",
        aliases: &[],
        help: "cmd.yume.panel",
        needs: &[Need::Scheme],
        params: &[Param::Words { of: PANELS, default: None }],
        build: Some(|p| {
            // Likewise a question on its own — and one word for both halves of
            // it, because「面板」is what a reader calls the thing whether it is
            // drawn or not.
            Ok(Command::YumePanel(p.arg(0).map(|w| w.to_string())))
        }),
    },
    Entry {
        name: "yume-menu-size",
        aliases: &[],
        help: "cmd.yume.menu-size",
        needs: &[Need::Scheme],
        params: &[Param::Free("<1–9>")],
        build: Some(|p| {
            // On its own it is the question, like `:yume-panel`.
            let Some(word) = p.arg(0) else {
                return Ok(Command::YumeMenuSize(None));
            };
            match word.parse::<usize>() {
                // **The panel's own limit, not an opinion about taste.** The
                // selection keys are `1`–`9`, so a tenth candidate on the page
                // would have no key to pick it with.
                Ok(n) if (1..=9).contains(&n) => Ok(Command::YumeMenuSize(Some(n))),
                _ => Err(CommandError::InvalidArgument {
                    command: "yume-menu-size",
                    value: word.to_string(),
                }),
            }
        }),
    },
    Entry {
        name: "yume-autocompletion",
        aliases: &[],
        help: "cmd.yume.autocompletion",
        needs: &[Need::Scheme],
        params: &[Param::Words { of: ON_OFF, default: None }],
        build: Some(|p| Ok(Command::YumeAutocompletion(p.arg(0).map(|w| w == "on")))),
    },
    Entry {
        name: "yume-installed",
        aliases: &[],
        help: "cmd.yume.installed",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::InstalledScheme)),
    },
    Entry {
        name: "yume-builtin",
        aliases: &[],
        help: "cmd.yume.builtin",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::BuiltinScheme)),
    },
    Entry {
        name: "yume-table",
        aliases: &[],
        help: "cmd.yume.table",
        needs: &[],
        params: &[Param::Path],
        build: Some(|p| Ok(Command::UserTable(p.need(0)?.to_string()))),
    },
    Entry {
        name: "syntax",
        aliases: &["syn"],
        help: "cmd.commands.syntax",
        needs: &[],
        params: &[Param::WordsOr { of: SYNTAXES, default: None, or: "<語法>" }],
        build: Some(|p| Ok(Command::SetSyntax(p.arg(0).map(|w| w.to_string())))),
    },
    Entry {
        name: "pipe",
        aliases: &[],
        help: "cmd.commands.pipe",
        needs: &[],
        params: &[Param::Free("<命令>")],
        build: Some(|p| {
            match p.arg(0) {
                None => Err(CommandError::MissingArgument("pipe")),
                Some(line) => Ok(Command::Pipe(line.to_string())),
            }
        }),
    },
    Entry {
        name: "sh",
        aliases: &[],
        help: "cmd.commands.sh",
        needs: &[],
        params: &[Param::Free("<命令>")],
        build: Some(|p| {
            match p.arg(0) {
                None => Err(CommandError::MissingArgument("sh")),
                Some(line) => Ok(Command::Shell {
                    line: line.to_string(),
                    interactive: false,
                }),
            }
        }),
    },
    Entry {
        name: "!command",
        aliases: &[],
        help: "cmd.commands.shell",
        needs: &[],
        params: &[],
        build: None,
    },
    Entry {
        name: "view-wrap",
        aliases: &[],
        help: "cmd.view.wrap",
        needs: &[],
        params: &[Param::WordsOr {
            of: WRAP,
            default: Some("on"),
            or: "<幾欄>",
        }],
        build: Some(|p| match p.need(0)? {
            "on" => Ok(Command::SetSoftWrap(true)),
            "off" => Ok(Command::SetSoftWrap(false)),
            "0" => Ok(Command::SetMeasure(None)),
            _ => Ok(Command::SetMeasure(Some(p.number(0)?))),
        }),
    },
    Entry {
        name: "view-margin",
        aliases: &[],
        help: "cmd.view.margin",
        needs: &[],
        params: &[Param::Words {
            of: MARGINS,
            default: None,
        }],
        build: Some(|p| {
            Ok(Command::SetMargin(
                p.arg(0).and_then(yumete_cjk::Margin::parse),
            ))
        }),
    },
    Entry {
        name: "view-bands",
        aliases: &[],
        help: "cmd.view.bands",
        needs: &[Need::Vertical],
        params: &[Param::WordsOr {
            of: ON_OFF,
            default: Some("on"),
            or: "<幾條，1–4>",
        }],
        build: Some(|p| match p.need(0)? {
            "on" => Ok(Command::SetBands(2)),
            "off" => Ok(Command::SetBands(1)),
            _ => match p.number(0)? {
                n @ 1..=4 => Ok(Command::SetBands(n)),
                n => Err(CommandError::InvalidArgument {
                    command: "view-bands",
                    value: n.to_string(),
                }),
            },
        }),
    },
    Entry {
        name: "view-sentence",
        aliases: &[],
        help: "cmd.view.sentence",
        needs: &[Need::Vertical],
        params: &[Param::Words {
            of: ON_OFF,
            default: Some("on"),
        }],
        build: Some(|p| Ok(Command::SetSentences(p.need(0)? == "on"))),
    },
    Entry {
        name: "view-hanging",
        aliases: &[],
        help: "cmd.view.hanging",
        needs: &[Need::Vertical, Need::Margin],
        params: &[Param::Words {
            of: ON_OFF,
            default: None,
        }],
        build: Some(|p| Ok(Command::SetHanging(p.arg(0).map(|w| w == "on")))),
    },
    Entry {
        // `:view-numbers-fill` on its own said nothing about *what* of the numbers
        // and refused; there was only ever one thing, so it is the command.
        name: "view-numbers-fill",
        aliases: &[],
        help: "cmd.numbers.fill",
        needs: &[],
        params: &[Param::Words {
            of: ON_OFF,
            default: None,
        }],
        build: Some(|p| Ok(Command::SetNumberFill(p.arg(0).map(|w| w == "on")))),
    },
    Entry {
        name: "view-typewriter",
        aliases: &[],
        help: "cmd.view.typewriter",
        needs: &[],
        params: &[Param::Words {
            of: SWITCH,
            default: Some("on"),
        }],
        build: Some(|p| Ok(Command::SetTypewriter(switched(p)?))),
    },
    Entry {
        name: "view-focus",
        aliases: &[],
        help: "cmd.view.focus",
        needs: &[],
        params: &[Param::Words {
            of: SWITCH,
            default: Some("on"),
        }],
        build: Some(|p| Ok(Command::SetFocus(switched(p)?))),
    },
    Entry {
        name: "view-meter",
        aliases: &[],
        help: "cmd.view.meter",
        needs: &[Need::Margin],
        params: &[Param::Words {
            of: SWITCH,
            default: Some("on"),
        }],
        build: Some(|p| Ok(Command::SetMeter(switched(p)?))),
    },
    Entry {
        // Was `:note`, which was never about footnotes (§5.2.3 ④).
        name: "view-punct",
        aliases: &[],
        help: "cmd.view.punct",
        needs: &[],
        params: &[Param::Words {
            of: SWITCH,
            default: Some("on"),
        }],
        build: Some(|p| Ok(Command::SetNote(switched(p)?))),
    },
    Entry {
        name: "view-code",
        aliases: &[],
        help: "cmd.view.code",
        needs: &[],
        params: &[Param::Words {
            of: SWITCH,
            default: Some("on"),
        }],
        build: Some(|p| Ok(Command::SetCode(switched(p)?))),
    },
    Entry {
        // The bare word reports, the same shape `:render` has: with three
        // levels, 「which one am I on」 is a better use of the word than a
        // fourth spelling of the middle one.
        name: "view-hud",
        aliases: &[],
        help: "cmd.view.hud",
        needs: &[],
        params: &[Param::Words {
            of: HUD,
            default: None,
        }],
        build: Some(|p| {
            Ok(match p.arg(0) {
                None => Command::ReportHud,
                Some("off") => Command::SetHud(crate::editor::Hud::Off),
                Some("basic") => Command::SetHud(crate::editor::Hud::Basic),
                _ => Command::SetHud(crate::editor::Hud::Full),
            })
        }),
    },
    Entry {
        name: "view-preview",
        aliases: &[],
        help: "cmd.view.preview",
        needs: &[],
        params: &[Param::Words {
            of: ON_OFF,
            default: Some("on"),
        }],
        build: Some(|p| Ok(Command::SetPreview(p.need(0)? == "on"))),
    },
    Entry {
        name: "render",
        aliases: &[],
        help: "cmd.commands.render",
        needs: &[],
        params: &[Param::Words { of: RENDER, default: None }],
        build: Some(|p| {
            // **The bare word reports** (#283): with three levels — and three
            // more dimensions taking their level from this one — 「which level
            // am I on」 is the better use of the word.
            Ok(match p.arg(0) {
                None => Command::ReportRender,
                Some("off") => Command::SetRender(crate::editor::Render::Off),
                Some("basic") => Command::SetRender(crate::editor::Render::Basic),
                _ => Command::SetRender(crate::editor::Render::Full),
            })
        }),
    },
    Entry {
        name: "indent",
        aliases: &[],
        help: "cmd.commands.indent",
        needs: &[],
        params: &[Param::WordsOr { of: INDENT_LEVELS, default: None, or: "<幾格，至多 8>" }],
        build: Some(|p| {
            // A number is the *width*; a word is the level. `:indent 4` on a
            // page at 中階 widens the indent and still does not fold, because
            // those are two questions and the reader answered one of them.
            // The bare word reports, for the reason bare `:render` reports.
            Ok(match p.arg(0) {
                None => Command::ReportIndent,
                Some("full") => Command::SetIndentLevel(crate::editor::Render::Full),
                Some("basic") => Command::SetIndentLevel(crate::editor::Render::Basic),
                Some("off" | "0") => Command::SetIndentLevel(crate::editor::Render::Off),
                Some(_) => match p.number(0)? {
                    n @ 0..=8 => Command::SetIndent(n),
                    n => {
                        return Err(CommandError::InvalidArgument {
                            command: "indent",
                            value: n.to_string(),
                        })
                    }
                },
            })
        }),
    },
    Entry {
        name: "indent-tab",
        aliases: &[],
        help: "cmd.indent.tab",
        needs: &[],
        params: &[Param::Words { of: TAB_INSERTS, default: None }],
        build: Some(|p| {
            Ok(Command::SetTabInserts(match p.arg(0) {
                None => None,
                Some("spaces") => Some(true),
                Some("tab") => Some(false),
                Some(other) => {
                    return Err(CommandError::InvalidArgument {
                        command: "indent-tab",
                        value: other.to_string(),
                    })
                }
            }))
        }),
    },
    Entry {
        name: "indent-width",
        aliases: &[],
        help: "cmd.indent.width",
        needs: &[],
        params: &[Param::Free("<幾格，1–8>")],
        build: Some(|p| {
            Ok(Command::SetIndentWidth(match p.arg(0) {
                None => None,
                Some(_) => match p.number(0)? {
                    n @ 1..=8 => Some(n),
                    n => {
                        return Err(CommandError::InvalidArgument {
                            command: "indent-width",
                            value: n.to_string(),
                        })
                    }
                },
            }))
        }),
    },
    Entry {
        name: "indent-hint",
        aliases: &[],
        help: "cmd.indent.hint",
        needs: &[],
        params: &[Param::Words { of: HINTS, default: None }],
        build: Some(|p| {
            match crate::zong::IndentHint::parse(p.arg(0).unwrap_or("")) {
                Some(hint) => Ok(Command::SetIndentHint(hint)),
                None => Err(CommandError::InvalidArgument {
                    command: "indent-hint",
                    value: p.arg(0).unwrap_or("").to_string(),
                }),
            }
        }),
    },
    Entry {
        name: "table",
        aliases: &[],
        help: "cmd.commands.table",
        needs: &[],
        params: &[Param::Words { of: TABLE_LEVELS, default: None }],
        build: Some(|p| {
            Ok(match p.arg(0) {
                // **The bare word is the door; a level is a level.** `:table`
                // reads the table the cursor is in — that is what it has
                // always meant and it is not a surface.
                None => Command::EnterTable,
                Some("off") => Command::SetTableLevel(crate::editor::TableLevel::Off),
                Some("basic") => Command::SetTableLevel(crate::editor::TableLevel::Basic),
                _ => Command::SetTableLevel(crate::editor::TableLevel::Full),
            })
        }),
    },
    Entry {
        name: "table-new",
        aliases: &[],
        help: "cmd.table.new",
        needs: &[],
        params: &[Param::Free("<行>x<欄>")],
        build: Some(|p| {
            // 「迅速在 markdown 中插入一個三行四列表格」 (#276). Two numbers,
            // 行 then 欄. **`rows` counts the heading**: the rule row
            // underneath is punctuation, and nobody means it when they say
            // three.
            let spec = p.arg(0).unwrap_or("");
            let mut numbers = spec.split(['x', 'X', '×', ' ', '\t']).filter(|w| !w.is_empty());
            let rows = numbers.next().unwrap_or("3");
            let columns = numbers.next().unwrap_or("3");
            match (rows.parse::<usize>(), columns.parse::<usize>()) {
                (Ok(r), Ok(c)) if (1..=200).contains(&r) && (1..=32).contains(&c) => {
                    Ok(Command::NewTable { rows: r, columns: c })
                }
                _ => Err(CommandError::InvalidArgument {
                    command: "table-new",
                    value: spec.to_string(),
                }),
            }
        }),
    },
    Entry {
        name: "table-check",
        aliases: &[],
        help: "cmd.table.check",
        needs: &[Need::Table],
        params: &[],
        build: Some(|_| Ok(Command::CheckTable)),
    },
    Entry {
        name: "table-rules",
        aliases: &[],
        help: "cmd.table.rules",
        needs: &[Need::Table],
        params: &[
            Param::WordsOr {
                of: RULES,
                default: None,
                or: "<樣式>",
            },
            Param::Words {
                of: STROKES,
                default: None,
            },
        ],
        build: Some(|p| {
            let Some(first) = p.arg(0) else {
                return Ok(Command::SetTableRules(None));
            };
            let said = match p.arg(1) {
                Some(stroke) => format!("{first} {stroke}"),
                None => first.to_string(),
            };
            match crate::table::Rules::parse(&said) {
                Some(rules) => Ok(Command::SetTableRules(Some(rules))),
                None => Err(CommandError::InvalidArgument {
                    command: "table-rules",
                    value: said,
                }),
            }
        }),
    },
    Entry {
        name: "table-sort",
        aliases: &[],
        help: "cmd.table.sort",
        needs: &[Need::Table],
        params: &[Param::Free("<欄> a｜d …")],
        build: Some(|p| {
            // `:table-sort 1 a 2 d 4 a` — column, direction, column,
            // direction. Nothing at all sorts by the column you are in.
            let mut keys = Vec::new();
            let mut words = p.arg(0).unwrap_or("").split_whitespace();
            while let Some(word) = words.next() {
                let Ok(column) = word.parse::<usize>() else {
                    return Err(CommandError::InvalidArgument {
                        command: "table-sort",
                        value: word.to_string(),
                    });
                };
                let down = match words.next() {
                    None | Some("a") | Some("asc") => false,
                    Some("d") | Some("desc") => true,
                    Some(other) => {
                        return Err(CommandError::InvalidArgument {
                            command: "table-sort",
                            value: other.to_string(),
                        })
                    }
                };
                keys.push((column, down));
            }
            Ok(Command::SortTable(keys))
        }),
    },
    Entry {
        name: "replace",
        aliases: &[],
        help: "cmd.commands.replace",
        needs: &[],
        params: &[Param::Path],
        build: Some(|p| {
            Ok(Command::OpenReplace(match p.arg(0) {
                None => crate::search_panel::Where::Buffer,
                Some(path) => crate::search_panel::Where::Named(path.into()),
            }))
        }),
    },
    Entry {
        name: "replace-cd",
        aliases: &[],
        help: "cmd.commands.replace-cd",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::OpenReplace(crate::search_panel::Where::Folder))),
    },
    Entry {
        name: "replace-gd",
        aliases: &[],
        help: "cmd.commands.replace-gd",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::OpenReplace(crate::search_panel::Where::Project))),
    },
    Entry {
        name: "replace-wd",
        aliases: &[],
        help: "cmd.commands.replace-wd",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::OpenReplace(crate::search_panel::Where::Workspace))),
    },
    Entry {
        name: "search",
        aliases: &[],
        help: "cmd.commands.search",
        needs: &[],
        params: &[Param::Path],
        build: Some(|p| {
            Ok(Command::OpenSearch(match p.arg(0) {
                None => crate::search_panel::Where::Buffer,
                Some(path) => crate::search_panel::Where::Named(path.into()),
            }))
        }),
    },
    Entry {
        name: "search-cd",
        aliases: &[],
        help: "cmd.commands.search-cd",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::OpenSearch(crate::search_panel::Where::Folder))),
    },
    Entry {
        name: "search-gd",
        aliases: &[],
        help: "cmd.commands.search-gd",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::OpenSearch(crate::search_panel::Where::Project))),
    },
    Entry {
        name: "search-wd",
        aliases: &[],
        help: "cmd.commands.search-wd",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::OpenSearch(crate::search_panel::Where::Workspace))),
    },
    Entry {
        name: "sidebar-show-left",
        aliases: &[],
        help: "cmd.sidebar.left",
        needs: &[],
        params: &[Param::Words { of: SIDEBAR_PANELS, default: None }],
        build: Some(|p| {
            Ok(Command::ShowSidebarAt(
                crate::sidebar::Side::Left,
                named_panel(p.name, p.arg(0))?,
            ))
        }),
    },
    Entry {
        name: "sidebar-show-right",
        aliases: &[],
        help: "cmd.sidebar.right",
        needs: &[],
        params: &[Param::Words { of: SIDEBAR_PANELS, default: None }],
        build: Some(|p| {
            Ok(Command::ShowSidebarAt(
                crate::sidebar::Side::Right,
                named_panel(p.name, p.arg(0))?,
            ))
        }),
    },
    Entry {
        name: "table-detail",
        aliases: &[],
        help: "cmd.table.detail",
        needs: &[Need::Table],
        params: &[Param::WordsOr { of: ON_OFF, default: None, or: "<寬>" }],
        build: Some(|p| {
            Ok(match p.arg(0) {
                None => Command::ShowDetail(None),
                Some("on") => Command::ShowDetail(Some(true)),
                Some("off") => Command::ShowDetail(Some(false)),
                _ => Command::SetDetailWidth(p.number(0)?),
            })
        }),
    },
    Entry {
        name: "table-numbers",
        aliases: &[],
        help: "cmd.table.numbers",
        needs: &[Need::Table],
        params: &[Param::Words { of: ON_OFF, default: Some("on") }],
        build: Some(|p| Ok(Command::SetTableNumbers(p.need(0)? == "on"))),
    },
    Entry {
        name: "table-header",
        aliases: &[],
        help: "cmd.table.header",
        needs: &[Need::Table],
        params: &[Param::Words { of: ON_OFF, default: None }],
        build: Some(|p| {
            // 「這一行是欄名還是資料」 — on its own it flips, because that is
            // the question a 碼表 asks once and never again (#217).
            Ok(Command::SetTableHeader(p.arg(0).map(|w| w == "on")))
        }),
    },
    Entry {
        name: "table-schema",
        aliases: &[],
        help: "cmd.table.schema",
        needs: &[Need::Table],
        params: &[],
        build: Some(|_| {
            // 「這張表到底是怎麼讀的」 — the answer is a file, so the command
            // opens it rather than printing it (#218).
            Ok(Command::OpenTableSchema)
        }),
    },
    Entry {
        name: "table-jump",
        aliases: &[],
        help: "cmd.table.jump",
        needs: &[Need::Table],
        params: &[Param::Free("<列名>")],
        build: Some(|p| {
            // Was `:row` (§5.2.3 ④): `row` under `:table` would have meant
            // the axis, and the axis is what `find` takes.
            Ok(Command::GotoRow(p.need(0)?.to_string()))
        }),
    },
    Entry {
        name: "table-find",
        aliases: &[],
        help: "cmd.table.find",
        needs: &[],
        params: &[
            Param::WordsOr {
                of: AXIS,
                default: None,
                or: "<字符串>",
            },
            Param::Free("<字符串>"),
        ],
        build: Some(|p| {
            // Was `:search`, whose two words *are* the axis (§5.2.3 ④). With
            // no direction it is a row search, because that is what a search
            // is anywhere but a table.
            let (by, pattern) = match (p.arg(0), p.arg(1)) {
                (Some("column"), rest) => (Axis::Column, rest.unwrap_or("").to_string()),
                (Some("row"), rest) => (Axis::Row, rest.unwrap_or("").to_string()),
                (Some(word), Some(rest)) => (Axis::Row, format!("{word} {rest}")),
                (Some(word), None) => (Axis::Row, word.to_string()),
                (None, _) => (Axis::Row, String::new()),
            };
            match pattern.is_empty() {
                true => Err(CommandError::MissingArgument("table-find")),
                false => Ok(Command::Search { pattern, by }),
            }
        }),
    },
    Entry {
        name: "table-pipe",
        aliases: &[],
        help: "cmd.table.pipe",
        needs: &[],
        params: &[Param::Free("<分隔>")],
        build: Some(|p| match p.arg(0) {
            None => Ok(Command::TableToPipe(None)),
            Some(word) => match delimiter_named(word) {
                Some(c) => Ok(Command::TableToPipe(Some(c))),
                None => Err(CommandError::InvalidArgument {
                    command: "table-pipe",
                    value: word.to_string(),
                }),
            },
        }),
    },
    Entry {
        name: "table-csv",
        aliases: &[],
        help: "cmd.table.csv",
        needs: &[],
        params: &[Param::Free("<分隔>")],
        build: Some(|p| match p.arg(0) {
            None => Ok(Command::TableToDelimited(',')),
            Some(word) => match delimiter_named(word) {
                Some(c) => Ok(Command::TableToDelimited(c)),
                None => Err(CommandError::InvalidArgument {
                    command: "table-csv",
                    value: word.to_string(),
                }),
            },
        }),
    },
    Entry {
        name: "wheel",
        aliases: &[],
        help: "cmd.commands.wheel",
        needs: &[],
        params: &[Param::Free("<幾行>")],
        build: Some(|p| {
            Ok(Command::SetWheelStep(match p.arg(0) {
                None => None,
                Some(_) => Some(p.number(0)?),
            }))
        }),
    },
    Entry {
        name: "clipboard-yank",
        aliases: &[],
        help: "cmd.clipboard.yank",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Clipboard { yank: true })),
    },
    Entry {
        name: "clipboard-paste",
        aliases: &[],
        help: "cmd.clipboard.paste",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Clipboard { yank: false })),
    },
    Entry {
        // The bare word lists them, which is what `:buffer` was: a
        // command whose own meaning is the useful one needs no word after it.
        name: "buffer",
        aliases: &[],
        help: "cmd.commands.buffer",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::ListBuffers)),
    },
    Entry {
        name: "buffer-next",
        aliases: &["bn"],
        help: "cmd.buffers.next",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::NextBuffer)),
    },
    Entry {
        name: "buffer-previous",
        aliases: &["bp"],
        help: "cmd.buffers.previous",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::PreviousBuffer)),
    },
    Entry {
        name: "buffer-close",
        aliases: &["bc"],
        help: "cmd.buffers.close",
        needs: &[],
        params: &[],
        build: Some(|p| Ok(Command::CloseBuffer { force: p.force })),
    },
    Entry {
        name: "format",
        aliases: &["fmt"],
        help: "cmd.commands.format",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Language("format".to_string()))),
    },
    Entry {
        name: "run",
        aliases: &[],
        help: "cmd.commands.run",
        needs: &[],
        params: &[Param::Free("<名字>")],
        build: Some(|p| {
            match p.arg(0) {
                None => Err(CommandError::MissingArgument("run")),
                Some(name) => Ok(Command::Language(name.to_string())),
            }
        }),
    },
    Entry {
        name: "markdown-footnote",
        aliases: &[],
        help: "cmd.markdown-bits.footnote",
        needs: &[],
        params: &[Param::Words { of: FOOTNOTE_KINDS, default: None }],
        build: Some(|p| {
            Ok(Command::Markdown(match p.arg(0) {
                None => MarkdownBit::Footnote,
                _ => MarkdownBit::InlineNote,
            }))
        }),
    },
    Entry {
        name: "tutor",
        aliases: &[],
        help: "cmd.commands.tutor",
        needs: &[],
        params: &[],
        build: Some(|_| Ok(Command::Tutor)),
    },
    Entry {
        name: "help",
        aliases: &[],
        help: "cmd.commands.help",
        needs: &[],
        params: &[Param::WordsOr { of: HELP_SECTIONS, default: None, or: "<題目>" }],
        build: Some(|p| Ok(Command::Help(p.arg(0).map(|w| w.to_string())))),
    },
    Entry {
        name: "export",
        aliases: &["ex"],
        help: "cmd.commands.export",
        needs: &[],
        params: &[Param::Words { of: EXPORT_FORMATS, default: None }, Param::Path],
        build: Some(|p| {
            Ok(Command::Export {
                format: p.need(0)?.to_string(),
                path: p.arg(1).map(|s| s.to_string()),
                force: p.force,
            })
        }),
    },
    Entry {
        name: "diff",
        aliases: &[],
        help: "cmd.commands.diff",
        needs: &[],
        params: &[Param::Path],
        build: Some(|p| Ok(Command::Diff(p.arg(0).map(|s| s.to_string())))),
    },
    Entry {
        name: "toc",
        aliases: &["outline"],
        help: "cmd.commands.toc",
        needs: &[],
        params: &[Param::Free("<幾級>")],
        build: Some(|p| {
            Ok(Command::Outline(match p.arg(0) {
                None => None,
                Some(_) => Some(p.number(0).map_err(|_| CommandError::MissingArgument("toc"))?),
            }))
        }),
    },
    Entry {
        name: "ruby",
        aliases: &[],
        help: "cmd.commands.ruby",
        needs: &[],
        params: &[Param::Words { of: RUBY_LEVELS, default: None }],
        build: Some(|p| {
            // **The bare word annotates the selection.** It is the one level
            // setting whose own name is also a verb — 注音 is something you
            // *do* to a word — and that reading of it wins, which is why
            // `:ruby` alone does not report the way `:render` does.
            Ok(match p.arg(0) {
                None => Command::Ruby,
                Some("off") => Command::SetRubyLevel(crate::editor::Render::Off),
                Some("basic") => Command::SetRubyLevel(crate::editor::Render::Basic),
                _ => Command::SetRubyLevel(crate::editor::Render::Full),
            })
        }),
    },
    Entry {
        name: "ruby-auto",
        aliases: &[],
        help: "cmd.ruby.auto",
        needs: &[],
        params: &[Param::Words { of: RARE, default: None }],
        build: Some(|p| Ok(Command::AutoRuby { rare: p.arg(0).is_some() })),
    },
    Entry {
        name: "ruby-html",
        aliases: &[],
        help: "cmd.ruby.html",
        needs: &[],
        params: &[Param::Words { of: ON_OFF, default: Some("on") }],
        build: Some(|p| {
            Ok(Command::RenderRuby {
                dialect: Dialect::Html,
                on: p.need(0)? == "on",
            })
        }),
    },
    Entry {
        name: "ruby-typst",
        aliases: &[],
        help: "cmd.ruby.typst",
        needs: &[],
        params: &[Param::Words { of: ON_OFF, default: Some("on") }],
        build: Some(|p| {
            Ok(Command::RenderRuby {
                dialect: Dialect::Typst,
                on: p.need(0)? == "on",
            })
        }),
    },
    Entry {
        name: "ruby-format",
        aliases: &[],
        help: "cmd.ruby.format",
        needs: &[],
        params: &[Param::Words { of: DIALECTS, default: None }],
        build: Some(|p| {
            let name = p.need(0)?;
            let dialect = Dialect::parse_name(name).ok_or(CommandError::InvalidArgument {
                command: "ruby-format",
                value: name.to_string(),
            })?;
            Ok(Command::FormatRuby(dialect))
        }),
    },
    Entry {
        name: "s/pat/rep/",
        aliases: &[],
        help: "cmd.commands.substitute",
        needs: &[],
        params: &[],
        build: None,
    },
];

/// The seven commands the fold **renamed**, and where they went (§5.2.3 ④).
///
/// Only seven, and only because these seven could not be computed. A command
/// that merely moved under a parent kept its spelling, so [`moved_to`] finds
/// it by looking — and goes on finding it if it moves again. `:conflicts` did
/// not keep its spelling: it became `:check-merge`, because the subject is a
/// merge and `:check` is the verb, and no amount of looking turns one string
/// into the other. Aliases are listed beside their names for the same reason
/// the names are: `:bc` was a real thing to type and stopped being one.
/// A short spelling, and the whole line it stands for (#363).
///
/// **The rule**: a full command is folded and says what it does
/// — `:buffer-close`, `:write-quit` — and a shorthand is **its initials**,
/// nothing else. So `:bc` yes, `:bclose` no: a half-short, half-long spelling
/// is neither one thing nor the other, and it is the shape the fold of §5.2.4
/// went to the trouble of removing.
///
/// Kept as an expansion rather than as six more arms in `parse`, so a
/// shorthand cannot drift from the command it is short for: there is one
/// definition of what `:bc` does, and it is `:buffer-close`. The bang comes
/// along for free (`:bc!` is `:buffer-close!`), and so does anything the long
/// form ever grows.
///
/// **Only the ones that name a *line*.** A short spelling of a one-word
/// command is what `Entry::aliases` is for — `:wq` and `:x` are `write-quit`'s
/// aliases and shown in its brackets like every other alias. This table is for
/// the ones that stand for two words, which no alias can express.




/// **`bc` and `wa` came back** (2026-09-10). The fold retired them along with
/// `bclose` and `wall`, on the rule that a leaf keeps its name and the tree
/// says the rest. That is right for the twenty commands a reader meets once;
/// it was wrong for the four typed all day, whose short spellings are in every
/// vi user's fingers. The long spellings still work and still say what they do
/// — these are a second way in, not a rename back.
// ⚠️ `search` was here (it pointed at `table-find`) and is gone from it:
// #419 gave the name to a live command, and a signpost may only point
// *away* from a word nobody can type any more.
const RENAMED: &[(&str, &str)] = &[
    ("appearance", "theme"),
    ("bclose", "buffer-close"),
    ("conflicts", "check-merge"),
    // Two answers to one question it did not name; four answers now (2026-09-16).
    ("dense", "view-margin"),
    ("note", "view-punct"),
    ("row", "table-jump"),
    ("sav", "write-as"),
    ("saveas", "write-as"),
    ("view-dense", "view-margin"),
    ("wall", "write-all"),
];

/// Where a word that is no longer a command went, if it went anywhere (#283).
///
/// **The point of folding a table into a tree is that the leaves keep their
/// names**, so most of this is a search rather than a record: `bands` is still
/// spelled `bands`, one level down, and the way to find that out is to look
/// for it. What cannot be looked up is a rename, and those are in [`RENAMED`].
///
/// A prefix is tried only when nothing matches whole — `:prog` was a real
/// spelling of a real command until the fold, and answering it with
/// `:count-progress` is the difference between a signpost and a shrug. Three
/// answers at most: past that the word is too vague to be pointing anywhere.
fn moved_to(word: &str) -> Option<String> {
    if let Some((_, to)) = RENAMED.iter().find(|(from, _)| *from == word) {
        return Some(format!("`:{to}`"));
    }
    let under = |matches: fn(&str, &str) -> bool| -> Vec<String> {
        let mut found = Vec::new();
        for entry in COMMANDS {
            // **A name's own segments are looked in too** (#368). `dense` is
            // not a word after a command any more, it is half of one —
            // `:view-dense` — and 「where did `:dense` go」 is the same
            // question it always was.
            if entry.name.split('-').skip(1).any(|part| matches(part, word)) {
                found.push(format!("`:{}`", entry.name));
                continue;
            }
            for param in entry.params {
                for w in param.words().unwrap_or_default() {
                    if matches(w.name, word) {
                        found.push(format!("`:{} {}`", entry.name, w.name));
                    }
                }
            }
        }
        found
    };
    let mut found = under(|name, word| name == word);
    if found.is_empty() {
        found = under(|name, word| name.starts_with(word));
    }
    if found.is_empty() || found.len() > 3 {
        return None;
    }
    Some(found.join(" "))
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
/// than the whole line: `:yume sch` has to become `:yume-scheme`, not `scheme`.
pub fn complete_at(line: &str) -> (usize, Vec<Choice>) {
    complete_within(line, true)
}

/// The same, saying whether the families are folded (#369).
///
/// **The menu folds and the search does not.** `::` looks for a word anywhere
/// in the list, and a list that had hidden eleven of twelve `view-…` behind
/// their head would answer 「沒有『密排』這種東西」 about a command that is
/// three keystrokes away.
fn complete_within(line: &str, folding: bool) -> (usize, Vec<Choice>) {
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

    // Fold each family into its head until something has narrowed it (#369).
    //
    // **Derived, never declared.** A command's name says which family it is
    // in — `view-dense` is under `view` because that is how it is spelled —
    // so the menu reads the levels off the names and there is still exactly
    // one place a command says what it is called. Typing as far as a family's
    // head opens it; `:view-dense` typed straight through never sees a fold
    // at all, because nothing about parsing changes.
    fn fold(typed: &str, mut choices: Vec<Choice>) -> Vec<Choice> {
        fn head_of(name: &str) -> &str {
            name.split('-').next().unwrap_or(name)
        }
        fn child_of(head: &str, name: &str) -> bool {
            name.len() > head.len() + 1
                && name.starts_with(head)
                && name.as_bytes()[head.len()] == b'-'
        }
        // **Only the names at the top level.** What the deep fallback finds
        // is a *word* under some command — `:vert` reaches `layout vertical`
        // and `help vertical`, two different things that happen to share a
        // spelling — and folding those together would answer one question
        // with the other.
        let mut counted: Vec<(&str, usize)> = Vec::new();
        for c in choices.iter().filter(|c| c.under.is_empty()) {
            let head = head_of(c.name);
            match counted.iter_mut().find(|(h, _)| *h == head) {
                Some((_, n)) => *n += 1,
                None => counted.push((head, 1)),
            }
        }
        // **A fold is for choosing between families.** Once what is typed
        // has narrowed the list to one of them there is nothing left to
        // choose, so `:b` opens `buffer-…` rather than making a reader type
        // the other five letters to see what they already know is there.
        //
        // **A stem with one command under it is still a stem**
        // (2026-09-10: 「未來 markdown 肯定還有別的命令」). `markdown-` names a
        // group whether or not it has grown one yet, and a list that spelled
        // it out today would change shape the day it does.
        let a_family = |head: &str, n: usize| {
            // **A family member is `head-…`, not merely a name that begins
            // the same way**: `shot` starts with `sh` and is no relation.
            n > 1 || COMMANDS.iter().any(|e| child_of(head, e.name))
        };
        let folded: Vec<&str> = counted
            .iter()
            .filter(|(head, n)| a_family(head, *n) && counted.len() > 1 && !typed.starts_with(*head))
            .map(|(head, _)| *head)
            .collect();
        if folded.is_empty() {
            return choices;
        }
        // Which families have a head that is a command in its own right —
        // asked before the walk, because it decides how the row is spelled.
        let heads: Vec<&str> = choices
            .iter()
            .filter(|c| c.under.is_empty() && folded.contains(&head_of(c.name)) && c.name == head_of(c.name))
            .map(|c| c.name)
            .collect();
        let mut out: Vec<Choice> = Vec::with_capacity(choices.len());
        let mut standing: Vec<&str> = Vec::new();
        for c in choices.drain(..) {
            let head = head_of(c.name);
            if !c.under.is_empty() || !folded.contains(&head) {
                out.push(c);
                continue;
            }
            if standing.contains(&head) {
                continue;
            }
            let under = counted.iter().find(|(h, _)| *h == head).map_or(0, |(_, n)| *n);
            // **A family whose head is a command of its own stands as itself**:
            // `:table` is a command, and a row saying 「and twelve more」 beside
            // it is more use than one that says only 「twelve」. A head that is
            // merely a prefix keeps its hyphen, because `view` alone is not
            // something anybody can type.
            let its_own = heads.contains(&head);
            match c.name == head {
                true => {
                    standing.push(head);
                    out.push(Choice {
                        family: Some(under - 1),
                        ..c
                    });
                }
                false if its_own => out.push(c),
                false => {
                    standing.push(head);
                    out.push(Choice {
                        name: &c.name[..head.len() + 1],
                        family: Some(under),
                        // **A stem is nobody's name**, so it carries nobody's
                        // spellings: `view-w` is `view-wrap`'s shortest, and
                        // printing it beside `view-` would offer it for the
                        // twelve.
                        alias: None,
                        short: None,
                        ..c
                    });
                }
            }
        }
        // **A folded family still shows the spellings worth reaching for**
        // (2026-09-10: 「wq, bc 這種重要的別名可以單開一行」). An alias
        // is another name for the command, not a shortcut to it, so it belongs
        // in the list beside the names — and once its command is folded away,
        // this row is the only place it can be seen. Named by the first of
        // them, since that is what a reader would type.
        let mut with_aliases: Vec<(&str, Choice)> = Vec::new();
        for row in &out {
            let head = head_of(row.name);
            if row.family.is_none() || !folded.contains(&head) {
                continue;
            }
            for e in COMMANDS.iter().filter(|e| head_of(e.name) == head) {
                let Some((first, rest)) = e.aliases.split_first() else {
                    continue;
                };
                // The row standing for the family is not folded away.
                if e.name == row.name {
                    continue;
                }
                with_aliases.push((head, Choice {
                    name: first,
                    family: None,
                    needs: e.needs,
                    alias: (!rest.is_empty()).then(|| rest.join(" ")),
                    short: None,
                    help: e.help,
                    leading: ":",
                    under: String::new(),
                    note: None,
                }));
            }
        }
        // Each one under the family it belongs to, which is where a reader
        // looking at that family will find it. **Placed by that family and not
        // by its own name**: `wa` begins with a `w` and belongs to `write`
        // because of what it is short for, not because of how it is spelled.
        let mut placed: Vec<Choice> = Vec::with_capacity(out.len() + with_aliases.len());
        for row in out.drain(..) {
            let head = head_of(row.name);
            placed.push(row);
            for (of, alias) in with_aliases.iter() {
                if *of == head {
                    placed.push(alias.clone());
                }
            }
        }
        let out = placed;
        out
    }

    // The first word names a command; every word after it walks down what that
    // command says may follow — which is the same walk whether those words are
    // arguments or subcommands, because they are the same thing.
    // One row of the top level, however it was reached.
    let row = |e: &'static Entry| Choice {
        name: e.name,
        family: None,
        needs: e.needs,
        alias: (!e.aliases.is_empty()).then(|| e.aliases.join(" ")),
        // **Among the aliases too.** `:table-jump` has no alias of its own and
        // no other *name* starts with `ro`, so the shortest walk over names
        // alone offered `(ro)` — while `ro` is `:readonly`'s declared alias,
        // and an exact alias beats a prefix in `resolve`. The menu was
        // promising a spelling that did something else.
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
        note: None,
    };
    let mut choices: Vec<Choice> = match words.first() {
        None => COMMANDS
            .iter()
            .filter(|e| {
                e.name.starts_with(typed) || e.aliases.iter().any(|a| a.starts_with(typed))
            })
            .map(row)
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
            Some(param) => match param.words() {
                Some(list) => list
                    .iter()
                    // **The note is searched too** (#291): the tag is what gets
                    // written, and 「冰雪」 is what the reader knows. Typing it
                    // needs 中文 on the command line, which a Shift tap gives.
                    .filter(|w| {
                        w.name.starts_with(typed)
                            || note_for(param, w).is_some_and(|n| n.contains(typed))
                    })
                    .map(|w| Choice {
                        name: w.name,
                        family: None,
                        needs: w.needs,
                        alias: None,
                        short: shortest(w.name, list.iter().map(|o| o.name)),
                        help: w.help,
                        leading: "",
                        under: String::new(),
                        note: note_for(param, w),
                    })
                    .collect(),
                // A path or free text is the caller's business; there is
                // nothing here to offer but the placeholder, which is help
                // rather than a completion.
                None => match param {
                    Param::Free(what) => vec![Choice {
                        name: "",
                        family: None,
                        needs: &[],
                        alias: None,
                        short: None,
                        help: what,
                        leading: "",
                        under: String::new(),
                        note: None,
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
    let reached_for = choices.is_empty() && !typed.is_empty();
    if reached_for {
        choices = match words.first() {
            None => {
                // **A name's later segments are looked in too** (#373).
                // `discover` is not a word after a command any more, it is the
                // second half of one — `:word-discover` — and the reader who
                // typed it named the thing just as plainly as when it was a
                // word. Flattening the tree (#368) moved most of what this
                // fallback used to find into names, and took it out of reach.
                let mut out: Vec<Choice> = COMMANDS
                    .iter()
                    .filter(|e| {
                        e.name
                            .split('-')
                            .skip(1)
                            .any(|part| part.starts_with(typed))
                    })
                    .map(row)
                    .collect();
                out.extend(deep_from_root(typed));
                out
            }
            // **A command's own parameters are all there is below it.** The
            // levels that used to hang here are in the names now, so what
            // `:vert` reaches from inside a command is that command's words
            // and nothing further down (#368).
            Some(_) => Vec::new(),
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
                .and_then(|e| e.params.first().map(|p| children(e.name, p))),
            // **A word is a value now, and a value has nothing under it.**
            // The levels a finished word used to open are in the names.
            Some(_) => None,
        };
        choices.extend(below.unwrap_or_default());
    }
    // Only the top level folds: below it the walk is already inside one
    // command, and what it offers is that command's own words.
    // **What the fallback found is not a family listing** (#373): `:punct`
    // reaches `check-punct` and `view-punct`, two particular commands that
    // happen to share a word, and folding them into `check-` and `view-` would
    // answer the question with the two families they came from.
    if folding && !reached_for && words.first().is_none() {
        choices = fold(typed, choices);
    }
    (start, choices)
}

/// Every command and every word under it, as one flat list (#224).
///
/// `complete` answers *what starts with this*; `::` asks *what does this*, and
/// that question has no prefix to walk down — the reader typed 排序 and the
/// answer is `:table-sort`, which shares not one letter with it. So the search
/// needs the whole tree at once, each row carrying the path that has to be
/// typed to reach it, and ranks it by what it says rather than how it is
/// spelled.
///
/// The top level first, then everything below it shallowest-first, so a tie in
/// the ranking falls out as the shorter command.
pub fn all_choices() -> Vec<Choice> {
    let mut out = complete_within("", false).1;
    out.extend(deep_from_root(""));
    out
}

/// What the command on this line needs before it can do anything.
///
/// The *deepest* word that says so: `:table-rules` needs a table because
/// `rules` says it, and `:yume-chaifen` needs a 碼表 because `chaifen` does.
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
    // The **deepest** word that says so, which is now a parameter's rather
    // than a subcommand's: `:table-rules` needs a table because the command
    // says it, and a value may add one of its own.
    for (at, word) in words[1..].iter().enumerate() {
        let Some(list) = entry.params.get(at).and_then(Param::words) else {
            break;
        };
        let Some(found) = pick(word, list) else { break };
        if !found.needs.is_empty() {
            needs = found.needs;
        }
    }
    needs
}

/// What may follow the words already on the line, or `None` if they name
/// nothing.
fn walk(words: &[(usize, &str)]) -> Option<&'static Param> {
    let (_, head) = words.first()?;
    let entry = entry_named(head)?;
    // **A command's parameters are read by position** (#368): the words after
    // `:theme` are its first and its second, not a path down a tree, so what
    // may stand in the next slot is what the next slot declares.
    entry.params.get(words.len() - 1)
}

/// Does a line the **documents** print name something the editor has?
///
/// Not `parse`: no argument is evaluated and nothing is run, so a line that is
/// merely impossible at this moment — `:w` with no file, `:table-rules` with no
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
    for (at, word) in words.enumerate() {
        let Some(list) = entry.params.get(at).and_then(Param::words) else {
            return Ok(());
        };
        // A bang ends a **line**, not a word, so it is looked for where the
        // line is spelled out: `:buffer-close!` is in `FORCEABLE` and
        // `:buffer!` is not.
        let (word, banged) = match word.strip_suffix('!') {
            Some(stem) => (stem, true),
            None => (word, false),
        };
        if pick(word, list).is_none() {
            // A parameter that also takes anything lets its own text through:
            // `:theme 墨香` names a theme this table has never heard of, and
            // the front end is the one holding the colours.
            return match matches!(entry.params.get(at), Some(Param::WordsOr { .. })) {
                true => Ok(()),
                false => Err(word.to_string()),
            };
        }
        if banged && !FORCEABLE.contains(&format!("{}!", entry.name).as_str()) {
            return Err(format!("{word}!"));
        }
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
///
/// **`-` is a span, `,` is a list** — the one range convention this editor has
/// (§5.7), which `t2-10/` and `t1,5,9s` already spelled. `:s` was the last
/// place where `,` meant a span, and a reader who had learnt the one had
/// learnt the wrong thing about the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rows {
    /// No range written: the lines the **selection** covers.
    Selection,
    /// `%` — every line.
    All,
    /// `1-40`, `.-$`, and a bare `5` — the two ends and everything between.
    Span(Bound, Bound),
    /// `1,5,9` — these lines and no others.
    List(Vec<Bound>),
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

/// A range that is neither one span nor one list — `1-5,9` mixes the joints,
/// `1-5-9` puts a line inside a span. Nobody has said what either means.
///
/// The whole sequence is eaten either way, so the caller can go on parsing and
/// decide whether this input was even meant to be a `:s`.
pub struct BadRange;

/// Split the leading range off a `:s` line.
///
/// **The two joints do not mix.** `1-40` is a span and `1,5,9` is a list, and
/// one range is one of them or the other — `1-5,9` is refused rather than
/// guessed at, exactly as a `t`/`g` sequence refuses it.
fn parse_rows(input: &str) -> (Result<Rows, BadRange>, &str) {
    if let Some(rest) = input.strip_prefix('%') {
        return (Ok(Rows::All), rest);
    }
    let Some((first, took)) = parse_bound(input) else {
        return (Ok(Rows::Selection), input);
    };
    let mut bounds = vec![first];
    let mut rest = &input[took..];
    let mut joint: Option<char> = None;
    let mut mixed = false;
    // Every `-` or `,` that is followed by another bound belongs to the range;
    // the first one that is not ends it (`:1s-a-b-` — a `-` delimiter).
    while let Some(next) = rest.chars().next().filter(|c| *c == '-' || *c == ',') {
        let Some((bound, n)) = parse_bound(&rest[1..]) else {
            break;
        };
        mixed |= joint.is_some_and(|first| first != next);
        joint.get_or_insert(next);
        bounds.push(bound);
        rest = &rest[1 + n..];
    }
    if mixed {
        return (Err(BadRange), rest);
    }
    let rows = match (joint, bounds.len()) {
        // A bare number is one line, the way `:40s` reads in vi.
        (None, _) => Rows::Span(first, first),
        (Some('-'), 2) => Rows::Span(bounds[0], bounds[1]),
        // `1-5-9` is a span with a middle, which is nothing: refuse it the way
        // a mixture is refused rather than quietly dropping the `5`.
        (Some('-'), _) => return (Err(BadRange), rest),
        (Some(_), _) => Rows::List(bounds),
    };
    (Ok(rows), rest)
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
    if let Some(bad) = flags.chars().find(|c| !"ginctf".contains(*c)) {
        return Some(Err(CommandError::InvalidArgument {
            command: "substitute",
            value: say!("substitute.unknown-flag", bad),
        }));
    }
    // Only now — once this is certainly a substitution and not `:set` or a
    // line number — is a range worth complaining about.
    let Ok(rows) = rows else {
        return Some(Err(CommandError::InvalidArgument {
            command: "substitute",
            value: say!("substitute.range-not-one-thing"),
        }));
    };
    Some(Ok(Command::Substitute {
        pattern: fields[0].clone(),
        replacement: fields[1].clone(),
        global: flags.contains('g'),
        ignore_case: flags.contains('i'),
        literal: flags.contains('f'),
        confirm: flags.contains('c'),
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

    /// `:open` with nothing to open is the picker, not an error (2026-09-16).
    ///
    /// The command line takes no 中文 anywhere on it, and a chapter is called
    /// 「第三章.md」, so the path has to be named somewhere that composes. Every
    /// alias gets it, because they are the same command.
    #[test]
    fn open_with_no_path_is_the_picker() {
        for cmd in [":open", ":o", ":e", ":edit", "open", "e"] {
            assert_eq!(parse(cmd), Ok(Command::OpenPicker), "{cmd}");
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
            let Some(list) = entry.params.first().and_then(Param::words) else {
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
    /// `:view-hanging off` turned hanging punctuation **on**: `COMMANDS` declared
    /// `Args::Words(ON_OFF)`, the hint printed it, the menu offered it, and
    /// `parse` was `"hanging" => Ok(Command::ToggleHanging)` with `rest`
    /// never read. `every_listed_command_parses` cannot see it — `:hanging
    /// off` *parses* — so the question has to be whether the two answers
    /// differ.
    #[test]
    fn a_command_that_offers_on_and_off_reads_them_back() {
        /// The list is 「on｜off」 — asked by what it holds rather than by
        /// which constant it is, so a second, hand-rolled pair is caught too.
        fn is_a_switch(param: &Param) -> bool {
            matches!(param.words(), Some(list)
                if list.len() >= 2 && list[0].name == "on" && list[1].name == "off")
        }
        let mut paths = Vec::new();
        for entry in COMMANDS {
            if entry.params.first().is_some_and(is_a_switch) {
                paths.push(entry.name.to_string());
            }
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
            // make `:view-hanging of` an error under a menu that prints `of`.
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
            parse(":word-show on"),
            Ok(Command::Word(WordCommand::Show(Some(true))))
        );
        assert_eq!(parse(":word-list"), Ok(Command::Word(WordCommand::List)));
        assert_eq!(
            parse(":word-list reload"),
            Ok(Command::Word(WordCommand::Reload))
        );
        assert_eq!(
            parse(":word-level strict"),
            Ok(Command::Word(WordCommand::Level(Some(
                yumete_cjk::WordLevel::Strict
            ))))
        );
        // 認詞 reads **this file** unless told otherwise, and the three wider
        // spellings are the ones `:search` already taught (#452).
        use crate::search_panel::Where;
        for (line, want) in [
            (":word-discover", Where::Buffer),
            (":word-discover-cd", Where::Folder),
            (":word-discover-gd", Where::Project),
            (":word-discover-wd", Where::Workspace),
        ] {
            assert_eq!(parse(line), Ok(Command::Word(WordCommand::Discover(want))), "{line}");
        }
        assert_eq!(parse(":word-habit"), Ok(Command::Word(WordCommand::Habit)));
        // A level nobody defined is refused by name, not silently taken.
        assert!(parse(":word-level 中等").is_err());
        // The commands it replaced are gone — `:words` was 口頭禪 and is
        // `:word-habit`, one subject and one command.
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
        assert_eq!(parse(":yume-chaifen"), Ok(Command::SetChaifen(None)));
        assert_eq!(
            parse(":yume-scheme lingming"),
            Ok(Command::SetScheme("lingming".into()))
        );
        // The flat spellings are gone: `:chaifen` said nothing about which of
        // the editor's many parts it belonged to, and `:scheme` even less.
        assert_eq!(parse(":chaifen"), Err(CommandError::Unknown("chaifen".into())));
        assert_eq!(parse(":scheme x"), Err(CommandError::Unknown("scheme".into())));
        // **On its own it turns it on** (#368). It used to be the question
        // 「which one is answering」, which `:yume-which` asks better; 「開中文」
        // is what a hand reaching for this command nearly always means.
        assert_eq!(
            parse(":yume"),
            Ok(Command::YumeLanguage(Engagement::Chinese))
        );
        assert_eq!(parse(":yume-which"), Ok(Command::YumeStatus));
        assert_eq!(
            parse(":yume on"),
            Ok(Command::YumeLanguage(Engagement::Chinese))
        );
        assert_eq!(
            parse(":yume abc"),
            Ok(Command::YumeLanguage(Engagement::Ascii))
        );
        assert_eq!(parse(":yume off"), Ok(Command::YumeLanguage(Engagement::Off)));
        assert_eq!(parse(":yume-installed"), Ok(Command::InstalledScheme));
        assert_eq!(parse(":yume-builtin"), Ok(Command::BuiltinScheme));
        assert_eq!(parse(":yume-b"), Ok(Command::BuiltinScheme));
        // 上屏方式 (Feature #209): the three are yume's own tags, `auto` is
        // 唯一 under the name the habit uses, and no argument is the question.
        assert_eq!(parse(":yume-commit"), Ok(Command::YumeCommit(None)));
        assert_eq!(
            parse(":yume-commit delayed"),
            Ok(Command::YumeCommit(Some("delayed".into())))
        );
        assert_eq!(
            parse(":yume-commit u"),
            Ok(Command::YumeCommit(Some("unique".into())))
        );
        assert_eq!(
            parse(":yume-commit auto"),
            Ok(Command::YumeCommit(Some("unique".into())))
        );
        assert_eq!(
            parse(":yume-commit fluency"),
            Ok(Command::YumeCommit(Some("fluency".into())))
        );
        // #211: 候選面板 is the other axis, and it parses by prefix too.
        assert_eq!(parse(":yume-panel"), Ok(Command::YumePanel(None)));
        assert_eq!(
            parse(":yume-panel bare"),
            Ok(Command::YumePanel(Some("bare".into())))
        );
        assert_eq!(
            parse(":yume-p f"),
            Ok(Command::YumePanel(Some("full".into())))
        );
        assert!(matches!(
            parse(":yume-panel invisible"),
            Err(CommandError::InvalidArgument {
                command: "yume-panel",
                ..
            })
        ));
        // Not silently the first of the three.
        assert!(matches!(
            parse(":yume-commit slow"),
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
            literal: false,
            confirm: false,
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
                literal: false,
                confirm: false,
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
            literal: false,
            confirm: false,
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
                literal: false,
                confirm: false,
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
        // `f` — 照字面, the pattern is characters and not a regex.
        assert!(matches!(
            parse(":%s/a.b/c/f"),
            Ok(Command::Substitute { literal: true, .. })
        ));
        assert!(matches!(
            parse(":%s/a.b/c/g"),
            Ok(Command::Substitute { literal: false, .. })
        ));
        // `c` — 逐處確認, ask before each one.
        assert!(matches!(
            parse(":%s/a/b/c"),
            Ok(Command::Substitute { confirm: true, .. })
        ));
        // A flag that is not implemented says so rather than being dropped.
        assert!(parse(":%s/a/b/z").is_err());
    }

    /// **`-` is a span, `,` is a list** — everywhere, `:s` included (§5.7).
    ///
    /// `:1,40s` was vi's span and is now the two lines 1 and 40. The editor
    /// had already taught `t2-10/` and `t1,5,9s`; leaving `:s` on vi's reading
    /// meant the one convention was true in three places out of four.
    #[test]
    fn a_substitution_takes_a_line_range() {
        assert!(matches!(
            parse(":1-40s/a/b/"),
            Ok(Command::Substitute {
                rows: Rows::Span(Bound::Line(1), Bound::Line(40)),
                ..
            })
        ));
        assert!(matches!(
            parse(":.-$s/a/b/"),
            Ok(Command::Substitute {
                rows: Rows::Span(Bound::Cursor, Bound::Last),
                ..
            })
        ));
        // A bare number is one line, the way `:40s` reads in vi.
        assert!(matches!(
            parse(":40s/a/b/"),
            Ok(Command::Substitute {
                rows: Rows::Span(Bound::Line(40), Bound::Line(40)),
                ..
            })
        ));
        // `,` names the lines it lists — three of them here, not thirty-two.
        let list = |input: &str| match parse(input) {
            Ok(Command::Substitute { rows: Rows::List(bounds), .. }) => bounds,
            other => panic!("{input} is not a list: {other:?}"),
        };
        assert_eq!(list(":1,5,9s/a/b/"), vec![
            Bound::Line(1),
            Bound::Line(5),
            Bound::Line(9)
        ]);
        // **`:1,40s` is two lines now**, which is the whole break: in vi it
        // was forty.
        assert_eq!(list(":1,40s/a/b/"), vec![Bound::Line(1), Bound::Line(40)]);
        assert_eq!(list(":.,$s/a/b/"), vec![Bound::Cursor, Bound::Last]);
        // Neither one span nor one list: refused, not guessed at.
        assert!(parse(":1-5,9s/a/b/").is_err());
        assert!(parse(":1,5-9s/a/b/").is_err());
        assert!(parse(":1-5-9s/a/b/").is_err());
        // …and a range is only worth complaining about once the line really
        // is a substitution. `:1-5,9` alone is some other unknown command.
        assert!(!matches!(
            parse(":1-5,9"),
            Err(CommandError::InvalidArgument { command: "substitute", .. })
        ));
    }

    /// 主題與深淺是兩條命令，不是一條命令的兩個位置（#449）。
    ///
    /// `:theme light` used to work, and so did `:theme moxiang dark` — which
    /// made 「主題」 name two things on one line. 「現在的 :theme
    /// dark/light/system 其實應該改成 :theme-mode，防止和其他的主題混淆。」
    #[test]
    fn a_theme_and_a_mood_are_two_questions() {
        use Mood::*;
        assert_eq!(
            parse(":theme heibai"),
            Ok(Command::Theme { name: Some("heibai".into()), mood: None })
        );
        assert_eq!(
            parse(":theme-mode light"),
            Ok(Command::Theme { name: None, mood: Some(Light) })
        );
        // **The mood is no longer a theme's name.** `light` is a word
        // `:theme` will now hand on to the front end as a theme nobody has,
        // which is the right answer: there is no theme called 「淺」.
        assert_eq!(
            parse(":theme light"),
            Ok(Command::Theme { name: Some("light".into()), mood: None })
        );
        // …and a line that asks both is a line that asks one too many.
        assert!(matches!(
            parse(":theme moxiang dark"),
            Err(CommandError::TakesNoArgument { command: "theme", .. })
        ));

        // The menu lists the themes under one and the moods under the other.
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
        let moods: Vec<String> = complete("theme-mode ").iter().map(Choice::written).collect();
        assert_eq!(moods, ["system", "dark", "light"]);
        // The shortest spelling the menu offers has to work.
        let short = shortest("theme", COMMANDS.iter().map(|c| c.name));
        assert!(parse(&format!(":{} mogao", short.unwrap_or("theme"))).is_ok());
        // And the name it lost is a signpost now, not a silence.
        assert_eq!(
            CommandError::Unknown("appearance".into()).to_string(),
            crate::say!("cmd.no-such-command-but", "appearance", "`:theme`")
        );
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
            }
        }
        for entry in COMMANDS {
            // Every parameter, at the position it stands in: the words of the
            // second slot are reached by writing something in the first.
            for (at, param) in entry.params.iter().enumerate() {
                let Some(list) = param.words() else { continue };
                let ahead: String = entry.params[..at]
                    .iter()
                    .map(|p| match p.words().and_then(|l| l.first()) {
                        Some(w) => format!(" {}", w.name),
                        None => " x".to_string(),
                    })
                    .collect();
                check(&format!(":{}{ahead}", entry.name), list);
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

    /// **A subcommand written with a space is refused, not swallowed.**
    ///
    /// This tree spells its subcommands with a hyphen. Text left over after a
    /// command has read every parameter it declared used to be dropped in
    /// silence, and two of those looked exactly like success: `:word show off`
    /// printed the report with the tinting still on, and `:buffer next` opened
    /// the picker having eaten `next`. The manual taught both spellings, which
    /// is how they got typed.
    ///
    /// ⚠️ `:write all` is the one that wrote a file. `:write` takes a path, so
    /// `all` was a legal one: a copy of the manuscript appeared in the working
    /// directory called `all`, the status line said 「抄了一份到 all」, and
    /// every other modified buffer stayed unsaved.
    #[test]
    fn a_subcommand_written_with_a_space_says_the_spelling() {
        for (line, name, word) in [
            (":word show off", "word", "show"),
            (":buffer next", "buffer", "next"),
            (":write all", "write", "all"),
            (":write as", "write", "as"),
        ] {
            match parse(line) {
                Err(CommandError::MeansTheHyphenatedOne { command, word: got }) => {
                    assert_eq!((command, got.as_str()), (name, word), "{line}");
                }
                other => panic!("{line} should name the spelling, got {other:?}"),
            }
        }
        // A leftover that is not a subcommand still says it is extra.
        assert!(matches!(
            parse(":word zzz"),
            Err(CommandError::TakesNoArgument { value, .. }) if value == "zzz"
        ));
        // …and the hyphenated spellings themselves go through.
        assert!(parse(":word-show off").is_ok());
        assert!(parse(":buffer-next").is_ok());
        // A file really called `all` is still writable, spelled as a path.
        assert!(matches!(parse(":write ./all"), Ok(Command::Write(Some(p))) if p == "./all"));
    }

    #[test]
    fn a_table_says_how_its_columns_are_told_apart() {
        use crate::table::{Rules, Stroke};
        let rules = |line: &str| match parse(line) {
            Ok(Command::SetTableRules(r)) => r,
            other => panic!("{line}: {other:?}"),
        };
        assert_eq!(rules(":table-rules"), None, "bare, it only reports");
        assert_eq!(rules(":table-rules off"), Some(Rules::Off));
        assert_eq!(rules(":table-rules color"), Some(Rules::Colour));
        assert_eq!(rules(":table-rules colour"), Some(Rules::Colour));
        for line in [":table-rules line", ":table-rules line solid"] {
            assert_eq!(rules(line), Some(Rules::Line(Stroke::Solid)), "{line}");
        }
        assert_eq!(rules(":table-rules line dash"), Some(Rules::Line(Stroke::Dash)));
        assert_eq!(
            rules(":table-rules line double"),
            Some(Rules::Line(Stroke::Double))
        );
        assert!(parse(":table-rules squiggly").is_err());
        // …and the menu lists them, the way a finished word does.
        let words: Vec<String> = complete("table-rules ").iter().map(Choice::written).collect();
        assert_eq!(words, ["off", "color", "line"]);
    }

    /// Either command on its own is 「哪一套／哪個深淺」, asked rather than set.
    #[test]
    fn a_theme_is_two_questions_and_either_may_be_left_out() {
        use Mood::*;
        let theme = |line: &str| parse(line).unwrap();
        // Neither half: the way to *ask* where things stand.
        assert_eq!(theme(":theme"), Command::Theme { name: None, mood: None });
        assert_eq!(theme(":theme-mode"), Command::Theme { name: None, mood: None });
        // A name the words know is canonicalised; anything else is passed on
        // as typed, because which themes exist is the front end's business.
        for line in [":theme ink", ":theme in"] {
            assert_eq!(
                theme(line),
                Command::Theme { name: Some("ink".into()), mood: None },
                "{line}"
            );
        }
        // A name is ASCII — a command line is typed with the IME off — and the
        // pinyin answers for the hand that thinks in Chinese.
        assert_eq!(
            theme(":theme moxiang"),
            Command::Theme { name: Some("moxiang".into()), mood: None }
        );
        assert_eq!(
            theme(":theme heibai"),
            Command::Theme { name: Some("heibai".into()), mood: None }
        );
        for (line, want) in [
            (":theme-mode light", Light),
            (":theme-mode system", System),
            (":theme-mode l", Light),
        ] {
            assert_eq!(
                theme(line),
                Command::Theme { name: None, mood: Some(want) },
                "{line}"
            );
        }
        // An unknown name is not a *syntax* error: it is answered where the
        // colours are.
        assert_eq!(
            theme(":theme solarized"),
            Command::Theme { name: Some("solarized".into()), mood: None }
        );
        // And the ground behind 品色 is its own switch, off by default.
        assert_eq!(parse(":theme-fill on"), Ok(Command::ThemeFill(Some(true))));
        assert_eq!(parse(":theme-fill"), Ok(Command::ThemeFill(None)));
    }

    /// What a menu row says, and what it leaves out (2026-09-10).
    ///
    /// Three rules, and every one of them is about what a reader could not
    /// have worked out for themselves. A column of `(rec)` `(rel)` `(red)`
    /// `(lay)` `(sho)` says only 「the first three letters work」, which the
    /// prefix rule already says about every command in the list.
    #[test]
    fn a_row_prints_the_spellings_a_reader_could_not_have_guessed() {
        let row = |typed: &str, name: &str| -> String {
            complete(typed)
                .into_iter()
                .find(|c| c.name == name)
                .map(|c| format!("{}{}", c.leading, c.shown()))
                .unwrap_or_else(|| panic!("`{name}` is not in the list for `{typed}`"))
        };

        // A head, its declared short spelling, and how many stand behind it —
        // the count last, where it reads as a remark about the row rather than
        // part of the name.
        assert_eq!(row("", "write"), ":write (w) +3");
        // A family with no head of its own keeps the hyphen, which is what
        // says it is a family and not a command, and nothing else: `view-w` is
        // `view-wrap`'s spelling, not the thirteen's.
        assert_eq!(row("", "view-"), ":view- +13");
        assert_eq!(row("", "check-"), ":check- +4");
        // …and one command under a stem is still a stem: `markdown-` names a
        // group whether or not it has grown a second one yet.
        assert_eq!(row("", "markdown-"), ":markdown- +1");
        // …while a name that merely begins the same way is no relation:
        // `shot` starts with `sh` and belongs to nobody.
        assert_eq!(row("", "sh"), ":sh");
        // **A clipped name longer than two is left out**, declared or not:
        // `red` is `:redo`'s own alias and says nothing `:re` did not.
        assert_eq!(row("", "redo"), ":redo");
        assert_eq!(row("", "recover"), ":recover");
        // …and two characters is worth knowing however it was arrived at.
        // `:theme` grew two children — `-mode` and `-fill`.
        assert_eq!(row("", "theme"), ":theme (th) +2");
        assert_eq!(row("", "quit"), ":quit (q) +1");
        // A **different word** is worth knowing at any length: none of these
        // could be read off the name.
        assert_eq!(row("", "open"), ":open (o e edit)");
        assert_eq!(row("", "toc"), ":toc (outline)");
        assert_eq!(row("", "count"), ":count (wc) +2");
        assert_eq!(row("", "format"), ":format (fmt)");
        // A folded command with a name of its own gets a row of it, under the
        // family it came from — otherwise the fold would have hidden the one
        // spelling anybody types.
        assert_eq!(row("", "wq"), ":wq");
        // `:x` is **not** `:wq` — it is `:exit`, which writes only what changed.
        assert_eq!(row("", "exit"), ":exit (x xit)");
        assert_eq!(row("", "bc"), ":bc");
        let listed: Vec<&str> = complete("").iter().map(|c| c.name).collect();
        let write = listed.iter().position(|n| *n == "write").unwrap();
        assert_eq!(
            &listed[write..write + 3],
            ["write", "wa", "wq"],
            "each one under the family it belongs to"
        );
    }

    /// **Half a name still finds the command** (#373). A reader who remembers
    /// 「discover」 and not 「word」 has named the thing, and the menu owes them
    /// the thing — the same debt #223 paid when `discover` was a word under
    /// `:word` rather than the second half of `:word-discover`.
    ///
    /// Only when nothing matched at the top level, which is what keeps `:d`
    /// about `:diff` rather than about every command with a `d` in it.
    #[test]
    fn half_a_name_finds_the_command_it_is_half_of() {
        let found = |typed: &str| -> Vec<String> {
            complete(typed).iter().map(|c| c.written()).collect()
        };
        // …and a word shared by a whole family answers with the family (#452).
        assert_eq!(
            found("discover"),
            ["word-discover", "word-discover-cd", "word-discover-gd", "word-discover-wd"]
        );
        assert_eq!(found("close"), ["buffer-close"]);
        assert_eq!(found("footnote"), ["markdown-footnote"]);
        // Two commands can share a word, and then both are the answer —
        // **not** the two families they came from, which is what a fold would
        // have made of them.
        assert_eq!(found("punct"), ["check-punct", "view-punct"]);
        // A word that is still a word — a parameter — is reached the way it
        // always was.
        assert_eq!(found("vert"), ["layout vertical", "help vertical"]);
        // …and a prefix that names something at the top level is answered
        // about that, with nothing dredged up from below it.
        assert_eq!(found("diff"), ["diff"]);
    }

    /// Print the menu the way it is drawn, for looking at.
    #[test]
    #[ignore = "a listing, not a test"]
    fn the_menu_as_it_is_drawn() {
        for c in complete("") {
            println!("{}{}", c.leading, c.shown());
        }
    }

    #[test]
    fn completion_narrows_as_the_command_is_typed() {
        // **Every command, and every spelling that is one** (#363). An alias
        // that names a whole line is in the list beside the names, because an
        // alias is another name for the command and not a shortcut to it.
        // **Folded into families** (#369). `:` alone is glanced at rather
        // than read, so the twelve `view-…` stand as one row until something
        // narrows them — and the whole list is still there for `::` to search.
        assert!(
            complete("").len() < COMMANDS.len(),
            "`:` alone folds the families: {}",
            complete("").len()
        );
        assert!(
            complete("view").len() >= 12,
            "typing the head opens the family: {}",
            complete("view").len()
        );
        assert_eq!(
            all_choices()
                .iter()
                .filter(|c| c.under.is_empty() && c.family.is_none())
                .count(),
            COMMANDS.len(),
            "and every one of them is still findable"
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
                "ruby-auto",
                "ruby-html",
                "ruby-typst",
                "ruby-format",
                "ruby off",
                "ruby basic",
                "ruby full",
            ]
        );
        // Half a word narrows to the one family, and a family with nothing
        // to choose against opens (#369).
        let rub: Vec<String> = complete("rub").iter().map(Choice::written).collect();
        assert_eq!(
            rub,
            ["ruby", "ruby-auto", "ruby-html", "ruby-typst", "ruby-format"]
        );
        // The words a finished command takes, under the command itself.
        let format: Vec<String> = complete("ruby-format").iter().map(Choice::written).collect();
        assert_eq!(format, ["ruby-format", "ruby-format html", "ruby-format typst"]);
        // **An alias is another name, not a shortcut** (#363) — and once a
        // command's name is one word, that is all a short spelling has to be
        // (#368): `bc` is `buffer-close`'s declared alias, exactly as `wq` is
        // `write-quit`'s, and the table that used to hold the two-word lines
        // separately is gone.
        assert_eq!(
            complete("wq").iter().map(|e| e.name).collect::<Vec<_>>(),
            ["write-quit"]
        );
        assert_eq!(
            complete("bc").iter().map(|e| e.name).collect::<Vec<_>>(),
            ["buffer-close"]
        );
        // …and `:b` keeps the family together.
        let under_b: Vec<&str> = complete("b").iter().map(|e| e.name).collect();
        for want in ["buffer", "buffer-close", "buffer-next", "buffer-previous"] {
            assert!(want == "buffer" || under_b.contains(&want), "{want} missing from {under_b:?}");
        }
        assert!(complete("zzz").is_empty());
    }

    #[test]
    fn the_short_form_the_menu_shows_is_one_that_works() {
        // The menu prints `:yume (y)`. That has to be true, or it is teaching
        // a spelling that fails — so the same prefix rule resolves the first
        // word of a command line, not only the words after it. And with the
        // family flattened, `y` still reaches the head rather than being torn
        // between it and the eight `yume-…` beside it (#368).
        assert_eq!(parse(":y"), Ok(Command::YumeLanguage(Engagement::Chinese)));
        assert_eq!(parse(":yu"), Ok(Command::YumeLanguage(Engagement::Chinese)));
        // `:ta` again: `:target` took the two-letter prefix away when it
        // arrived, and gave it back when the fold put it under `:count`
        // (§5.2.3 ③). Ten abbreviations got shorter that way.
        assert_eq!(parse(":tab"), Ok(Command::EnterTable));
        assert_eq!(parse(":ta"), Ok(Command::EnterTable));

        // A declared alias beats the prefix rule, so the short spellings
        // people already know keep their meanings: `w` begins `write`, `wq`,
        // `word` and `wheel`, and it is still `write`.
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
        // have been offered for `:table-jump`, while `ro` is `:readonly`'s declared
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
        // so `:table-jump` is offered with no short form at all, the way `:sh` is.
        assert_eq!(short("row"), None, "`ro` is `:readonly`'s");
        // Over what `complete` actually hands the menu, not over a second
        // derivation of it — the menu prints `Choice::short`, so that is the
        // string this has to hold to account.
        for choice in complete("") {
            // A folded family is not an entry and has no spelling of its own
            // (#369) — it stands for the twelve behind it.
            if choice.name.ends_with('-') {
                continue;
            }
            // A folded family is a prefix rather than a command (#369), and a
            // row named for a folded command's alias is that command under
            // another of its names — neither has a spelling of its own to be
            // held to account for.
            if choice.name.ends_with('-') {
                continue;
            }
            let Some(entry) = COMMANDS.iter().find(|e| e.name == choice.name) else {
                assert!(
                    COMMANDS.iter().any(|e| e.aliases.contains(&choice.name)),
                    "`{}` is in the list and is neither a command nor one's alias",
                    choice.name
                );
                assert_eq!(parse(&format!(":{}", choice.name)).is_ok(), true);
                continue;
            };
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
        // `:yume-s l` is the whole of starting to type — `yume-s` names only
        // `yume-scheme`, and `l` only 靈明.
        assert_eq!(
            parse(":yume-s l"),
            Ok(Command::SetScheme("lingming".into()))
        );
        assert_eq!(parse(":yume-scheme ling"), Ok(Command::SetScheme("lingming".into())));
        // `chaifen` and `commit` both start with `c`, so `yume-c` alone names
        // neither — the menu shows both, which is the answer.
        assert_eq!(parse(":yume-ch"), Ok(Command::SetChaifen(None)));
        assert!(matches!(parse(":yume-c"), Err(CommandError::Unknown(_))));
        // No name at all means the one the config asked for.
        assert_eq!(parse(":yume-s"), Ok(Command::SetScheme(String::new())));
        // A prefix that names two words names neither, loudly, rather than
        // quietly meaning whichever was written first.
        assert!(matches!(parse(":yume-x"), Err(CommandError::Unknown(_))));
    }

    #[test]
    fn a_space_offers_what_may_follow_the_command() {
        // The point of the whole arrangement: nobody has to remember an
        // argument, only a verb. `:view-margin ` says what may come next.
        let words = |line: &str| -> Vec<&str> { complete(line).iter().map(|c| c.name).collect() };
        assert_eq!(words("view-margin "), ["never", "dense", "loose", "always"]);
        assert_eq!(words("view-margin l"), ["loose"]);
        assert_eq!(
            words("syntax "),
            ["markdown", "typst", "text", "python", "javascript", "json", "yaml", "toml", "html", "css"]
        );
        assert_eq!(words("layout v"), ["vertical"]);

        // A parent command is not a mechanism of its own — its subcommands are
        // simply the words it takes, and they go as deep as they like.
        assert_eq!(
            words("ruby "),
            ["off", "basic", "full"]
        );
        assert_eq!(words("ruby-html "), ["on", "off"]);

        // Where the word being completed starts, so a completion replaces it
        // and not the whole line.
        assert_eq!(complete_at("ruby ht").0, 5);
        assert_eq!(complete_at("ruby-html o").0, 10);
        // One word now, so the completion replaces the whole of it (#368).
        assert_eq!(complete_at("view-margin").0, 0);

        // A command that takes free text says what it wants rather than
        // offering a list it does not have.
        let free = complete("goto ");
        assert_eq!(free.len(), 1);
        assert_eq!(free[0].name, "", "nothing to complete");
        assert!(free[0].help.contains("行號"), "but it says what to type");

        // A path is the caller's business, and a word nothing accepts is
        // nothing rather than the whole list again.
        assert!(complete("write draft.md ").is_empty());
        // …and `:quit` takes nothing at all now: 「全部關掉」 is a command of
        // its own (#368), so what `:quit` offers is the family beside it.
        assert!(complete("quit ").is_empty());
        assert_eq!(
            complete("quit").iter().map(|e| e.name).collect::<Vec<_>>(),
            ["quit", "quit-all"]
        );
        // **`:qa` is in the list as itself** (#363): an alias is another name
        // for the command, not a shortcut to it, so it narrows like a name.
        assert_eq!(
            complete("qa").iter().map(|e| e.name).collect::<Vec<_>>(),
            ["quit-all"]
        );
        // …and `:q` keeps the family together — `qa` being `quit-all`'s
        // declared alias rather than a line of its own (#368).
        let under_q: Vec<&str> = complete("q").iter().map(|e| e.name).collect();
        assert!(under_q.contains(&"quit"), "{under_q:?}");
        assert!(under_q.contains(&"quit-all"), "{under_q:?}");
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
            written("lingming").contains(&"yume-scheme lingming".to_string()),
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
        assert!(
            shallow.iter().all(|w| !w.trim_end_matches(|c: char| !c.is_whitespace()).contains(' ')
                || w.contains('…')),
            "{shallow:?}"
        );
        assert!(shallow.len() > 1, "several commands start with t");

        // Deeper down it works the same way: `:yume` knows no word `ling`, so
        // the offer comes from below it — and carries only the part still
        // missing, because `yume ` is already on the line.
        let under_yume: Vec<String> = complete("yume-scheme ling")
            .iter()
            .map(|c| c.written())
            .collect();
        assert!(
            under_yume.contains(&"lingming".to_string()),
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
            "table-sort",
            "table-new",
            "view-wrap",
            "layout vertical",
            "yume-scheme lingming",
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
            "check-usage",
            "search",
            "diff",
            "toc",
            "export",
            "ruby",
            "yume",
            "buffer",
            "clipboard-yank",
            "layout",
            "view-wrap",
            "wheel",
            "table",
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
        fn sample(name: &str, params: &[Param]) -> String {
            let mut line = format!(":{name}");
            for param in params {
                let word = match param {
                    _ if param.words().is_some_and(|l| !l.is_empty()) => {
                        let list = param.words().unwrap_or_default();
                        // `screen` refuses a file name on purpose, so the
                        // sample takes the one that wants one.
                        match name == "shot" {
                            true => "png".to_string(),
                            false => list[0].name.to_string(),
                        }
                    }
                    Param::Path => "a.md".to_string(),
                    Param::Schemes => "lingming".to_string(),
                    // A word only this command knows the shape of.
                    Param::Free(_) => match name {
                        "grep" | "run" | "pipe" | "sh" | "replace" => "x",
                        "table-new" => "3x4",
                        _ => "1",
                    }
                    .to_string(),
                    Param::Words { .. } | Param::WordsOr { .. } => String::new(),
                };
                if word.is_empty() {
                    break;
                }
                line.push(' ');
                line.push_str(&word);
            }
            line
        }

        for entry in COMMANDS {
            let line = match entry.name {
                "s/pat/rep/" => ":s/a/b/".to_string(),
                // Listed under the shape it is typed in, not as a word.
                "!command" => ":!echo hi".to_string(),
                name => sample(name, entry.params),
            };
            assert!(parse(&line).is_ok(), "{line} does not parse");
            // Every *word* a command offers has to parse too, not only the
            // first — completion offering a word the parser rejects is the
            // exact drift this table exists to catch.
            for word in entry.params.first().and_then(Param::words).unwrap_or_default() {
                // **Either shape, because a value may have rules of its own.**
                // `:table-find row` is a search with nothing to search for and
                // `:shot screen a.md` is a picture the clipboard has nowhere to
                // put — each is refused, and each has a sibling spelling that
                // is not. What this holds the table to is that every word it
                // offers stands in *some* line the parser accepts.
                let filled = sample(entry.name, &entry.params[1..]);
                let tail = filled
                    .strip_prefix(&format!(":{}", entry.name))
                    .unwrap_or("");
                let bare = format!(":{} {}", entry.name, word.name);
                let whole = format!("{bare}{tail}");
                assert!(
                    parse(&bare).is_ok() || parse(&whole).is_ok(),
                    "neither `{bare}` nor `{whole}` parses"
                );
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
        // ⚠️ **`:open` is not in this list any more** (2026-09-16): with no
        // path it is the picker, not a missing argument. `:write-as` stands in
        // for the shape — a command that really does need one.
        assert_eq!(parse(":write-as"), Err(CommandError::MissingArgument("write-as")));
        assert_eq!(parse(":bogus"), Err(CommandError::Unknown("bogus".into())));
    }
    /// §5.2.3 ④: the signpost tells the truth on both sides.
    ///
    /// `RENAMED` is the one part of `moved_to` that is written down rather
    /// than computed — nothing in the tree remembers that `punct` used to be
    /// `note` — so it is the one part that can go stale. Both halves are
    /// checked: the name it disowns must really be gone, and the line it
    /// sends the reader to must really run.
    #[test]
    fn the_signpost_names_a_command_that_exists_and_one_that_does_not() {
        for (was, is) in RENAMED {
            assert!(
                matches!(parse(&format!(":{was}")), Err(CommandError::Unknown(_))),
                "`:{was}` still parses, so the signpost is pointing away from a live command"
            );
            assert!(
                names_something(is).is_ok(),
                "the signpost sends `:{was}` to `:{is}`, which is not a command"
            );
            assert_eq!(
                moved_to(was).as_deref(),
                Some(format!("`:{is}`").as_str()),
                "`{was}` should be answered from RENAMED, not by the walk"
            );
        }
    }

    /// The other half of the signpost, the half that cannot go stale.
    ///
    /// Every word the fold moved is reachable by its own name from the parent
    /// it moved under, so a reader who types the old top-level spelling is
    /// told where it went without anyone having written it down.
    #[test]
    fn a_word_that_moved_under_a_parent_says_where_it_went() {
        for (word, sent_to) in [
            ("bands", "`:view-bands`"),
            ("hanging", "`:view-hanging`"),
            ("preview", "`:view-preview`"),
            ("progress", "`:count-progress`"),
            ("merge", "`:check-merge`"),
        ] {
            assert_eq!(moved_to(word).as_deref(), Some(sent_to));
        }
        // Two parents, two answers — ④'s rule, said out loud.
        assert_eq!(
            moved_to("punct").as_deref(),
            Some("`:check-punct` `:view-punct`")
        );
        // A word nothing owns is not answered at all: a wrong signpost is
        // worse than none.
        assert_eq!(moved_to("zzz"), None);
    }

}
