//! **What a key can be bound to, by name** (#429).
//!
//! Layer one (#428) let `[keys.normal]` say 「when I press this, press that
//! instead」 — an alias from one key sequence to another. It is what the vim
//! preset is made of, and it has one flaw that matters: the right-hand side is
//! *keys*, so a reader has to know the editor's default keys in order to
//! rebind them, and a binding breaks the day a default changes.
//!
//! Layer two is what helix does: **every bindable thing has a name**, and a
//! keymap binds keys to names. `x = "delete_selection"` says what it means
//! without the reader knowing that `d` happens to delete today.
//!
//! Warning: **The names are the surface; the defaults still live in the `match`.**
//! An action here says what it *is* and how it is carried out right now — by
//! playing the keys the editor already answers, or by running a command. What
//! this buys today is a stable vocabulary for the config file and the manual;
//! what it does not yet buy is a table the default keys are read from. That is
//! the next tranche, and it can land action by action without any config
//! written against these names having to change.
//!
//! Names follow helix's wherever helix has the same action, so that a reader
//! coming from that editor can guess, and a reader leaving it can search.

/// How an action is carried out.
///
/// Every key is a command (step 3 of the keymap plan), so no action presses
/// keys: the keys inside the groups (`gh`, `空格 f`) are commands of their
/// own too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// Run this command line, without the `:`.
    Command(&'static str),
    /// **A Normal command, by this action's own name**: the
    /// editor's command list (`yumete-core`'s `editor/normal.rs`) holds the
    /// code, and the key it is on is the keymap's to say.
    Normal,
}

/// One bindable action: its name, what it does in one line, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Action {
    /// What `[keys.normal]` writes on the right-hand side.
    pub name: &'static str,
    /// The message tag that says what it does, for `:keymap` and the manual.
    pub help: &'static str,
    pub how: How,
}

/// **Alt 的內部拼法**：`A-d` 存成這個字 ＋ `d`。
///
/// 一串按鍵在這個倉裏是一個 `String`——控制鍵用的是真的控制字節（`C-o` 就是
/// `\u{f}`），而 Alt 没有這樣一個字節。所以借一個私用區的碼位當前綴：存着的時候
/// 一個鍵仍然是一兩個 `char`，`HashMap<String, String>` 那一層一個字都不用改。
///
/// Warning: **不許寫進任何人看得見的地方。** 進來由 [`written`] 翻，出去由 [`spell`]
/// 翻，中間那一段纔是這個字。
pub const ALT: char = '\u{e000}';

/// **`C-j` and `C-i` have code points of their own**: their control bytes
/// are `\n` and `\t`, which are Enter and Tab here, so with those bytes
/// `[keys.picker] "C-j" = "next_entry"` would take Enter too. A terminal
/// sends `C-j` apart from Enter; the table keeps them apart.
pub const CTRL_J: char = '\u{e00c}';
pub const CTRL_I: char = '\u{e00d}';

/// The internal spelling of `C-` and a letter.
pub fn ctrl(letter: char) -> char {
    match letter.to_ascii_lowercase() {
        'j' => CTRL_J,
        'i' => CTRL_I,
        c => (c as u8 - b'a' + 1) as char,
    }
}

/// **有名字的那幾個鍵**：配置裏寫 `<tab>`，表裏存成一個 `char`。
///
/// Warning: **左邊那一串是「一串按鍵」**（`"dj" = …` 是 d 跟 j 兩下），所以鍵名不能
/// 光着寫——`"tab"` 的意思是 t、a、b 三下。尖括號是 vim 的辦法，和一串按鍵永遠
/// 不會撞。
///
/// 有自己字節的就用那個字節（Tab 是 `\t`、Enter 是 `\n`、空格就是空格），没有的
/// 借私用區一個碼位，同 [`ALT`]。於是一個鍵在表裏仍然是一兩個 `char`，查表那一層
/// 一個字都不用改。
///
/// Warning: **`Esc` 不在這張表上。** 它是「鬆手」那一下——按鍵序列按了一半要靠它
/// 退出來（`keys.rs` 的 `alias_held`）。綁掉它就没有退路了。
///
/// `S-` 是 vim／helix／kakoune 三家共同的 Shift 寫法；字母的 Shift 本來就寫成大寫
/// 本身，所以 `S-` 只出現在有名字的鍵前面。
pub const NAMED: &[(&str, char)] = &[
    ("tab", '\t'),
    ("S-tab", '\u{e001}'),
    ("enter", '\n'),
    ("space", ' '),
    ("backspace", '\u{e002}'),
    ("delete", '\u{e003}'),
    ("up", '\u{e004}'),
    ("down", '\u{e005}'),
    ("left", '\u{e006}'),
    ("right", '\u{e007}'),
    ("home", '\u{e008}'),
    ("end", '\u{e009}'),
    ("pageup", '\u{e00a}'),
    ("pagedown", '\u{e00b}'),
];

/// `<tab>` 裏那個名字 → 它的 `char`，大小寫不計。
pub fn named_key(name: &str) -> Option<char> {
    NAMED
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|&(_, c)| c)
}

/// 反過來：一個 `char` → 寫在 `:keymap` 上的那個名字。
fn key_name(c: char) -> Option<&'static str> {
    NAMED.iter().find(|&&(_, k)| k == c).map(|&(n, _)| n)
}

/// **把人寫的那一串翻成內部拼法**：`A-d` → [`ALT`] ＋ `d`，`C-o` → `\u{f}`。
///
/// `[keys.normal]` 的左邊和預設表都走這一支。Warning: **繞過它就錯**——配置裏寫
/// `"A-d" = …` 會綁成 **`A`、`-`、`d` 三個鍵連按**：編譯照過、配置照收，而那個鍵
/// 永遠按不出來。
///
/// Warning: **`C-` 只翻字母。** `C-[`、`C-]` 那幾個在舊終端上和數字鍵撞車（見
/// `Config::language_key` 的註釋），這裏不替它們猜。
pub fn written(spelled: &str) -> String {
    let mut out = String::new();
    let mut rest = spelled;
    // **`<名字>` 先認**：認不得的原樣留着，於是一個真的 `<` 照舊是
    // 它自己。
    while let Some(at) = rest.find('<') {
        let Some(shut) = rest[at..].find('>').map(|n| at + n) else { break };
        let Some(key) = named_key(&rest[at + 1..shut]) else {
            out.push_str(&rest[..=at]);
            rest = &rest[at + 1..];
            continue;
        };
        out.push_str(&modifiers(&rest[..at]));
        out.push(key);
        rest = &rest[shut + 1..];
    }
    out.push_str(&modifiers(rest));
    out
}

/// `A-`／`C-` 那一層，[`written`] 的另一半。
fn modifiers(spelled: &str) -> String {
    let mut out = String::new();
    let mut chars = spelled.chars().peekable();
    while let Some(c) = chars.next() {
        let modified = matches!(c, 'A' | 'C') && chars.peek() == Some(&'-');
        if !modified {
            out.push(c);
            continue;
        }
        // `A-` / `C-` 後面要真有一個字，不然那兩個字就是它們自己。
        let mut rest = chars.clone();
        rest.next();
        match rest.next() {
            Some(next) if c == 'A' => {
                out.push(ALT);
                out.push(next);
                chars = rest;
            }
            Some(next) if next.is_ascii_alphabetic() => {
                out.push(ctrl(next));
                chars = rest;
            }
            _ => out.push(c),
        }
    }
    out
}

/// **A key sequence, spelled the way the manual spells it** — `Space f`, not
/// a space and an `f`; `C-o`, not a control byte. The inverse of [`written`].
/// Keys are named in English in every interface language, as `Esc`, `Tab`
/// and `C-o` are.
pub fn spell(keys: &str) -> String {
    let mut out = String::new();
    let mut chars = keys.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' => out.push_str("Space "),
            CTRL_J => out.push_str("C-j"),
            CTRL_I => out.push_str("C-i"),
            '\u{1b}' => out.push_str("Esc"),
            // 有名字的那幾個鍵寫回名字——`\t` 畫出來是一段空白，讀的人看不出
            // 那是 Tab。
            c if key_name(c).is_some() => {
                out.push('<');
                out.push_str(key_name(c).unwrap_or_default());
                out.push('>');
            }
            ALT => {
                out.push_str("A-");
                if let Some(next) = chars.next() {
                    out.push(next);
                }
                // **A chord is a word of its own**: `C-w w`,
                // not `C-ww`, which reads as one chord with two letters.
                if chars.clone().next().is_some() {
                    out.push(' ');
                }
            }
            c if (c as u32) < 32 => {
                out.push_str("C-");
                out.push((b'a' + c as u8 - 1) as char);
                if chars.clone().next().is_some() {
                    out.push(' ');
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Look one up by name.
pub fn action(name: &str) -> Option<&'static Action> {
    ALL.iter().find(|a| a.name == name)
}

/// **What the right-hand side of a binding is**, told by its own spelling.
///
/// Three kinds, each with a mark of its own: `:write` is the command line,
/// `@xxd` is keys pressed as if by hand, and a bare word is an action by name.
/// `None` is 「none of the three」 — a name the editor does not know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound<'a> {
    /// `:write` — the line, without its colon.
    Command(&'a str),
    /// `@xxd` — the keys, without the `@`, still in the written spelling.
    Keys(&'a str),
    /// `delete_selection` — this action.
    Named(&'static Action),
}

/// Read a right-hand side, or `None` when it names nothing (#429).
///
/// Warning: **keys are never the *fallback*, or a typo is dangerous.** If an
/// unknown name fell through to 「then it must be keys」, `x = "delete_slection"`
/// would be pressed — d, e, l, e, t, e… straight into the manuscript. Guessing
/// by shape does not help: `"zz" = "save"` is a misspelling of `save_file` with
/// no underscore in it, and it would type `s`, `a`, `v`, `e` — the `s` opening
/// the search panel and `ave` landing in it. Fourteen of the eighty names have
/// no underscore either, so the shape of a word cannot answer this.
///
/// So the kind is **said, not guessed**. Measured on the two
/// editors this one follows: helix takes a bare string as a command name only —
/// `X = "xxd"` is refused with 「No command named 'xxd'」 — and writes keys with
/// an `@`, which is the spelling borrowed here; vim has no name kind at all, so
/// its right-hand side is always keys and there is nothing to typo. yumete was
/// the only one of the three guessing between them.
pub fn bound(rhs: &str) -> Option<Bound<'_>> {
    if let Some(line) = rhs.strip_prefix(':') {
        return Some(Bound::Command(line));
    }
    if let Some(keys) = rhs.strip_prefix('@') {
        return Some(Bound::Keys(keys));
    }
    action(rhs).map(Bound::Named)
}

/// A Normal command and what it does (help first, so the messages test
/// finds the tag right after `normal(`).
const fn normal(help: &'static str, cmd: NormalCmd) -> Action {
    Action { name: cmd.name(), help, how: How::Normal }
}

/// **Every Normal command, named once**: the config reads the names, the
/// editor's tables hold the variants, so a misspelt name in a keymap or a
/// group row does not compile. The order is the manual's, as [`ALL`] lists them.
macro_rules! normal_commands {
    ($($variant:ident = $name:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum NormalCmd {
            $($variant,)*
        }

        impl NormalCmd {
            pub const ALL: &'static [NormalCmd] = &[$(NormalCmd::$variant,)*];

            /// What the config writes.
            pub const fn name(self) -> &'static str {
                match self {
                    $(NormalCmd::$variant => $name,)*
                }
            }

            /// The command a config line names, if there is one.
            pub fn from_name(name: &str) -> Option<NormalCmd> {
                NormalCmd::ALL.iter().copied().find(|one| one.name() == name)
            }
        }

        impl std::fmt::Display for NormalCmd {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.name())
            }
        }

        impl AsRef<str> for NormalCmd {
            fn as_ref(&self) -> &str {
                self.name()
            }
        }
    };
}

normal_commands! {
    MoveCharLeft = "move_char_left",
    MoveCharRight = "move_char_right",
    MoveVisualLineUp = "move_visual_line_up",
    MoveVisualLineDown = "move_visual_line_down",
    MoveNextWordStart = "move_next_word_start",
    MovePrevWordStart = "move_prev_word_start",
    MoveNextWordEnd = "move_next_word_end",
    MoveNextLongWordStart = "move_next_long_word_start",
    MovePrevLongWordStart = "move_prev_long_word_start",
    MoveNextLongWordEnd = "move_next_long_word_end",
    GotoLineStart = "goto_line_start",
    GotoLineEnd = "goto_line_end",
    GotoFirstNonwhitespace = "goto_first_nonwhitespace",
    GotoFileStart = "goto_file_start",
    VimGotoFileStart = "vim_goto_file_start",
    GotoFileEnd = "goto_file_end",
    GotoLastLine = "goto_last_line",
    GotoPrevSentence = "goto_prev_sentence",
    GotoNextSentence = "goto_next_sentence",
    GotoPrevParagraph = "goto_prev_paragraph",
    GotoNextParagraph = "goto_next_paragraph",
    PageCursorHalfDown = "page_cursor_half_down",
    PageCursorHalfUp = "page_cursor_half_up",
    JumpBackward = "jump_backward",
    SelectMode = "select_mode",
    ExtendLineBelow = "extend_line_below",
    ExtendToLineBounds = "extend_to_line_bounds",
    SelectAll = "select_all",
    CollapseSelection = "collapse_selection",
    DeleteSelection = "delete_selection",
    DeleteSelectionNoyank = "delete_selection_noyank",
    ChangeSelection = "change_selection",
    ChangeSelectionNoyank = "change_selection_noyank",
    InsertMode = "insert_mode",
    AppendMode = "append_mode",
    InsertAtLineStart = "insert_at_line_start",
    InsertAtLineEnd = "insert_at_line_end",
    OpenBelow = "open_below",
    OpenAbove = "open_above",
    Yank = "yank",
    PasteAfter = "paste_after",
    PasteBefore = "paste_before",
    ReplaceWithYanked = "replace_with_yanked",
    JoinSelections = "join_selections",
    Indent = "indent",
    Unindent = "unindent",
    Increment = "increment",
    Decrement = "decrement",
    SwitchCase = "switch_case",
    Undo = "undo",
    Redo = "redo",
    RepeatLastChange = "repeat_last_change",
    Search = "search",
    Rsearch = "rsearch",
    SearchNext = "search_next",
    SearchPrev = "search_prev",
    SearchSelection = "search_selection",
    GotoDefinition = "goto_definition",
    GotoFile = "goto_file",
    FollowLink = "follow_link",
    OpenLink = "open_link",
    GotoDeclaration = "goto_declaration",
    GotoTypeDefinition = "goto_type_definition",
    GotoImplementation = "goto_implementation",
    GotoLastModifiedFile = "goto_last_modified_file",
    InsertCharAbove = "insert_char_above",
    InsertCharBelow = "insert_char_below",
    FilePicker = "file_picker",
    BufferPicker = "buffer_picker",
    GotoNextBuffer = "goto_next_buffer",
    GotoPreviousBuffer = "goto_previous_buffer",
    Dictionary = "dictionary",
    RotateView = "rotate_view",
    GlobalSearch = "global_search",
    CommandPalette = "command_palette",
    PasteMenu = "paste_menu",
    ToggleComments = "toggle_comments",
    ToggleBlockComments = "toggle_block_comments",
    ToggleLineComments = "toggle_line_comments",
    YankToClipboard = "yank_to_clipboard",
    YankMainSelectionToClipboard = "yank_main_selection_to_clipboard",
    JumplistPicker = "jumplist_picker",
    GotoFirstChange = "goto_first_change",
    GotoLastChange = "goto_last_change",
    Suspend = "suspend",
    SelectReferencesToSymbolUnderCursor = "select_references_to_symbol_under_cursor",
    FormatSelections = "format_selections",
    ExpandSelection = "expand_selection",
    ShrinkSelection = "shrink_selection",
    SelectNextSibling = "select_next_sibling",
    SelectPrevSibling = "select_prev_sibling",
    SelectAllSiblings = "select_all_siblings",
    SelectAllChildren = "select_all_children",
    MoveParentNodeEnd = "move_parent_node_end",
    MoveParentNodeStart = "move_parent_node_start",
    ExtendParentNodeEnd = "extend_parent_node_end",
    ExtendParentNodeStart = "extend_parent_node_start",
    LastPicker = "last_picker",
    PasteClipboardAfter = "paste_clipboard_after",
    MoveLineUp = "move_line_up",
    MoveLineDown = "move_line_down",
    GotoNextLineNonwhitespace = "goto_next_line_nonwhitespace",
    JumpNextWordStart = "jump_next_word_start",
    JumpNextLongWordStart = "jump_next_long_word_start",
    JumpPrevWordStart = "jump_prev_word_start",
    JumpPrevLongWordStart = "jump_prev_long_word_start",
    JumpNextWordEnd = "jump_next_word_end",
    JumpNextLongWordEnd = "jump_next_long_word_end",
    JumpNextParagraph = "jump_next_paragraph",
    JumpPrevParagraph = "jump_prev_paragraph",
    MoveNextParagraph = "move_next_paragraph",
    MovePrevParagraph = "move_prev_paragraph",
    GotoWindowTop = "goto_window_top",
    GotoWindowCenter = "goto_window_center",
    GotoWindowBottom = "goto_window_bottom",
    JumpPrevSentence = "jump_prev_sentence",
    JumpNextSentence = "jump_next_sentence",
    AddMark = "add_mark",
    GotoMark = "goto_mark",
    GotoMarkExact = "goto_mark_exact",
    ViewMode = "view_mode",
    StickyViewMode = "sticky_view_mode",
    RightBracketMode = "right_bracket_mode",
    LeftBracketMode = "left_bracket_mode",
    GotoMode = "goto_mode",
    SpaceMode = "space_mode",
    FindNextChar = "find_next_char",
    FindPrevChar = "find_prev_char",
    FindTillChar = "find_till_char",
    TillPrevChar = "till_prev_char",
    SelectLineMode = "select_line_mode",
    NormalMode = "normal_mode",
    RepeatFindChar = "repeat_find_char",
    RepeatFindCharReverse = "repeat_find_char_reverse",
    CopySelectionOnNextLine = "copy_selection_on_next_line",
    CopySelectionOnPrevLine = "copy_selection_on_prev_line",
    KeepPrimarySelection = "keep_primary_selection",
    RemovePrimarySelection = "remove_primary_selection",
    MergeSelections = "merge_selections",
    MergeConsecutiveSelections = "merge_consecutive_selections",
    EnsureSelectionsForward = "ensure_selections_forward",
    ShrinkToLineBounds = "shrink_to_line_bounds",
    SplitSelectionOnNewline = "split_selection_on_newline",
    RotateSelectionsForward = "rotate_selections_forward",
    RotateSelectionsBackward = "rotate_selections_backward",
    TrimSelections = "trim_selections",
    AlignSelections = "align_selections",
    RotateSelectionContentsForward = "rotate_selection_contents_forward",
    RotateSelectionContentsBackward = "rotate_selection_contents_backward",
    SelectRegex = "select_regex",
    SplitSelection = "split_selection",
    KeepSelections = "keep_selections",
    RemoveSelections = "remove_selections",
    OperatorDelete = "operator_delete",
    OperatorChange = "operator_change",
    OperatorYank = "operator_yank",
    OperatorIndent = "operator_indent",
    OperatorUnindent = "operator_unindent",
    DeleteToLineEnd = "delete_to_line_end",
    DeleteToLineEndNoyank = "delete_to_line_end_noyank",
    VimDeleteCharForward = "vim_delete_char_forward",
    ExtendLineAbove = "extend_line_above",
    MoveSelectedLinesDown = "move_selected_lines_down",
    MoveSelectedLinesUp = "move_selected_lines_up",
    GotoLineOrLastLine = "goto_line_or_last_line",
    NormalModeAndCollapse = "normal_mode_and_collapse",
    Quit = "quit",
    SelectInnerObject = "select_inner_object",
    SelectAroundObject = "select_around_object",
    UndoLine = "undo_line",
    Earlier = "earlier",
    Later = "later",
    GotoColumn = "goto_column",
    ShellPipe = "shell_pipe",
    ShellInsertOutput = "shell_insert_output",
    ShellAppendOutput = "shell_append_output",
    ShellPipeTo = "shell_pipe_to",
    ShellKeepPipe = "shell_keep_pipe",
    CommandMode = "command_mode",
    VimCommandMode = "vim_command_mode",
    MatchMode = "match_mode",
    Replace = "replace",
    SelectRegister = "select_register",
    RecordMacro = "record_macro",
    ReplayMacro = "replay_macro",
    WindowMode = "window_mode",
    PageDown = "page_down",
    PageUp = "page_up",
    JoinSelectionsSpace = "join_selections_space",
    SaveSelection = "save_selection",
    JumpForward = "jump_forward",
    TwoThirdsPageDown = "two_thirds_page_down",
    TwoThirdsPageUp = "two_thirds_page_up",
    FlipSelections = "flip_selections",
    VimRedo = "vim_redo",
    DeleteCharBackward = "delete_char_backward",
    CaseMode = "case_mode",
    SearchSelectionDetectWordBoundaries = "search_selection_detect_word_boundaries",
    SearchSelectionExact = "search_selection_exact",
    SearchWordBackward = "search_word_backward",
    ReplaceMode = "replace_mode",
    GotoLine = "goto_line",
    VimRepeatLastChange = "vim_repeat_last_change",
    RepeatLastMotion = "repeat_last_motion",
    JumpPrevWordEnd = "jump_prev_word_end",
    JumpPrevLongWordEnd = "jump_prev_long_word_end",
    GotoWord = "goto_word",
    GotoWordByLetters = "goto_word_by_letters",
    SplitJoin = "split_join",
    GotoCharByPinyin = "goto_char_by_pinyin",
    GotoLastAccessedFile = "goto_last_accessed_file",
    GotoReference = "goto_reference",
    MoveNextZong = "move_next_zong",
    MovePrevZong = "move_prev_zong",
    GotoLastModification = "goto_last_modification",
    ReselectLastSelection = "reselect_last_selection",
    InsertAtLastInsert = "insert_at_last_insert",
    GotoOlderChange = "goto_older_change",
    GotoNewerChange = "goto_newer_change",
    OperatorLowercase = "operator_lowercase",
    OperatorUppercase = "operator_uppercase",
    OperatorSwitchCase = "operator_switch_case",
    OperatorFill = "operator_fill",
    OperatorComment = "operator_comment",
    OperatorFillKeepCursor = "operator_fill_keep_cursor",
    JoinSelectionsNoSpace = "join_selections_no_space",
    FilePickerInCurrentDirectory = "file_picker_in_current_directory",
    FileExplorer = "file_explorer",
    FileExplorerInCurrentBufferDirectory = "file_explorer_in_current_buffer_directory",
    ChangedFilePicker = "changed_file_picker",
    PasteClipboardBefore = "paste_clipboard_before",
    ReplaceSelectionsWithClipboard = "replace_selections_with_clipboard",
    LspOrSyntaxSymbolPicker = "lsp_or_syntax_symbol_picker",
    LspOrSyntaxWorkspaceSymbolPicker = "lsp_or_syntax_workspace_symbol_picker",
    DictionaryPanel = "dictionary_panel",
    Hover = "hover",
    HoverPanel = "hover_panel",
    DiagnosticsPicker = "diagnostics_picker",
    WorkspaceDiagnosticsPicker = "workspace_diagnostics_picker",
    ShowDiagnostic = "show_diagnostic",
    ShowDiagnosticPanel = "show_diagnostic_panel",
    ShowChange = "show_change",
    ShowChangePanel = "show_change_panel",
    RenameSymbol = "rename_symbol",
    CodeAction = "code_action",
    ShowRecord = "show_record",
    ShowRecordPanel = "show_record_panel",
    TableMode = "table_mode",
    ConflictMode = "conflict_mode",
    OpenRecentMode = "open_recent_mode",
    YumeMode = "yume_mode",
    GotoNextConflict = "goto_next_conflict",
    GotoPrevConflict = "goto_prev_conflict",
    GotoNextChange = "goto_next_change",
    GotoPrevChange = "goto_prev_change",
    GotoNextDiag = "goto_next_diag",
    GotoPrevDiag = "goto_prev_diag",
    GotoLastDiag = "goto_last_diag",
    GotoFirstDiag = "goto_first_diag",
    GotoNextFunction = "goto_next_function",
    GotoPrevFunction = "goto_prev_function",
    GotoNextClass = "goto_next_class",
    GotoPrevClass = "goto_prev_class",
    GotoNextComment = "goto_next_comment",
    GotoPrevComment = "goto_prev_comment",
    GotoNextParameter = "goto_next_parameter",
    GotoPrevParameter = "goto_prev_parameter",
    GotoNextEntry = "goto_next_entry",
    GotoPrevEntry = "goto_prev_entry",
    GotoNextTest = "goto_next_test",
    GotoPrevTest = "goto_prev_test",
    GotoNextXmlElement = "goto_next_xml_element",
    GotoPrevXmlElement = "goto_prev_xml_element",
    AddNewlineBelow = "add_newline_below",
    AddNewlineAbove = "add_newline_above",
    AlignViewCenter = "align_view_center",
    AlignViewTop = "align_view_top",
    AlignViewBottom = "align_view_bottom",
    ScrollDown = "scroll_down",
    ScrollUp = "scroll_up",
    MatchBrackets = "match_brackets",
    SelectTextobjectInner = "select_textobject_inner",
    SelectTextobjectAround = "select_textobject_around",
    SurroundAdd = "surround_add",
    SurroundDelete = "surround_delete",
    SurroundReplace = "surround_replace",
    JumpViewLeft = "jump_view_left",
    JumpViewDown = "jump_view_down",
    JumpViewUp = "jump_view_up",
    JumpViewRight = "jump_view_right",
    ToggleLeftSidebar = "toggle_left_sidebar",
    ToggleRightSidebar = "toggle_right_sidebar",
    GotoLeftSidebar = "goto_left_sidebar",
    GotoRightSidebar = "goto_right_sidebar",
    Hsplit = "hsplit",
    Vsplit = "vsplit",
    HsplitAbove = "hsplit_above",
    VsplitLeft = "vsplit_left",
    HsplitNew = "hsplit_new",
    VsplitNew = "vsplit_new",
    WindowNewMode = "window_new_mode",
    NewWindowLeft = "new_window_left",
    NewWindowDown = "new_window_down",
    NewWindowUp = "new_window_up",
    NewWindowRight = "new_window_right",
    NewWindowLeftPick = "new_window_left_pick",
    NewWindowDownPick = "new_window_down_pick",
    NewWindowUpPick = "new_window_up_pick",
    NewWindowRightPick = "new_window_right_pick",
    Wclose = "wclose",
    Wonly = "wonly",
    TransposeView = "transpose_view",
    SwapViewLeft = "swap_view_left",
    SwapViewDown = "swap_view_down",
    SwapViewUp = "swap_view_up",
    SwapViewRight = "swap_view_right",
    GotoFileHsplit = "goto_file_hsplit",
    GotoFileVsplit = "goto_file_vsplit",
    GotoFileHsplitAbove = "goto_file_hsplit_above",
    CreateFold = "create_fold",
    VimCreateFold = "vim_create_fold",
    CreateFoldLines = "create_fold_lines",
    OpenFold = "open_fold",
    OpenFoldsRecursively = "open_folds_recursively",
    CloseFold = "close_fold",
    CloseFoldsRecursively = "close_folds_recursively",
    ToggleFold = "toggle_fold",
    ToggleFoldsRecursively = "toggle_folds_recursively",
    OpenFoldsToCursor = "open_folds_to_cursor",
    OpenAllFolds = "open_all_folds",
    CloseAllFolds = "close_all_folds",
    DeleteFold = "delete_fold",
    DeleteFoldsRecursively = "delete_folds_recursively",
    DeleteAllFolds = "delete_all_folds",
    GotoNextFold = "goto_next_fold",
    GotoPrevFold = "goto_prev_fold",
    DisableFolds = "disable_folds",
    EnableFolds = "enable_folds",
    ToggleFoldsEnabled = "toggle_folds_enabled",
    FoldMore = "fold_more",
    FoldLess = "fold_less",
    GotoFoldStart = "goto_fold_start",
    GotoFoldEnd = "goto_fold_end",
    ReapplyFoldLevelAndView = "reapply_fold_level_and_view",
    ReapplyFoldLevel = "reapply_fold_level",
    VimAlignViewTop = "vim_align_view_top",
    VimAlignViewCenter = "vim_align_view_center",
    VimAlignViewBottom = "vim_align_view_bottom",
    VimAlignViewBelowTop = "vim_align_view_below_top",
    VimAlignViewAboveBottom = "vim_align_view_above_bottom",
    ScrollViewLeft = "scroll_view_left",
    ScrollViewRight = "scroll_view_right",
    ScrollViewHalfLeft = "scroll_view_half_left",
    ScrollViewHalfRight = "scroll_view_half_right",
    AlignViewCursorLeft = "align_view_cursor_left",
    AlignViewCursorRight = "align_view_cursor_right",
    AlignViewMiddle = "align_view_middle",
    RotateViewReverse = "rotate_view_reverse",
    JumpViewFirst = "jump_view_first",
    JumpViewLast = "jump_view_last",
    JumpViewLastAccessed = "jump_view_last_accessed",
    ExchangeView = "exchange_view",
    RotateViewsForward = "rotate_views_forward",
    RotateViewsBackward = "rotate_views_backward",
    MoveViewFarLeft = "move_view_far_left",
    MoveViewFarDown = "move_view_far_down",
    MoveViewFarUp = "move_view_far_up",
    MoveViewFarRight = "move_view_far_right",
    VimCloseWindow = "vim_close_window",
    GotoLastAccessedFileHsplit = "goto_last_accessed_file_hsplit",
    SwitchToLowercase = "switch_to_lowercase",
    SwitchToUppercase = "switch_to_uppercase",
    ConvertToSimplified = "convert_to_simplified",
    ConvertToTraditional = "convert_to_traditional",
    ConvertToTaiwanTraditional = "convert_to_taiwan_traditional",
    ConvertToHongKongTraditional = "convert_to_hong_kong_traditional",
    ConvertToMainlandTraditional = "convert_to_mainland_traditional",
    ConvertToClassicalTraditional = "convert_to_classical_traditional",
    ConvertToJapanese = "convert_to_japanese",
    SwitchToHalfwidth = "switch_to_halfwidth",
    SwitchToFullwidth = "switch_to_fullwidth",
    ConflictKeepOurs = "conflict_keep_ours",
    ConflictKeepTheirs = "conflict_keep_theirs",
    ConflictKeepBoth = "conflict_keep_both",
    RecentFilePicker = "recent_file_picker",
    GlobalRecentFilePicker = "global_recent_file_picker",
    ProjectPicker = "project_picker",
    ResumeSession = "resume_session",
    YumeToggle = "yume_toggle",
    YumeCommitMode = "yume_commit_mode",
    YumePanelMode = "yume_panel_mode",
    YumePreeditMode = "yume_preedit_mode",
    YumeTogglePrediction = "yume_toggle_prediction",
    YumePickScheme = "yume_pick_scheme",
    YumeNextScheme = "yume_next_scheme",
    YumeMenuSize3 = "yume_menu_size_3",
    YumeMenuSize4 = "yume_menu_size_4",
    YumeMenuSize5 = "yume_menu_size_5",
    YumeMenuSize6 = "yume_menu_size_6",
    YumeMenuSize7 = "yume_menu_size_7",
    YumeMenuSize8 = "yume_menu_size_8",
    YumeMenuSize9 = "yume_menu_size_9",
    YumeNextAnnotation = "yume_next_annotation",
    YumeInstalledTable = "yume_installed_table",
    YumeTableSource = "yume_table_source",
    YumeDataLocation = "yume_data_location",
    YumeCommitDefault = "yume_commit_default",
    YumeCommitDelayed = "yume_commit_delayed",
    YumeCommitUnique = "yume_commit_unique",
    YumeCommitFluency = "yume_commit_fluency",
    YumePanelBox = "yume_panel_box",
    YumePanelBottom = "yume_panel_bottom",
    YumePanelInline = "yume_panel_inline",
    YumePreeditOff = "yume_preedit_off",
    YumePreeditComposition = "yume_preedit_composition",
    YumePreeditPreview = "yume_preedit_preview",
    YumeCharsetMode = "yume_charset_mode",
    YumeSortMode = "yume_sort_mode",
    YumeLanesMode = "yume_lanes_mode",
    YumeBuiltinMode = "yume_builtin_mode",
    YumeCharsetAll = "yume_charset_all",
    YumeCharsetCommon = "yume_charset_common",
    YumeCharsetTonggui = "yume_charset_tonggui",
    YumeCharsetHarmonic = "yume_charset_harmonic",
    YumeSortOriginal = "yume_sort_original",
    YumeSortCommon = "yume_sort_common",
    YumeSortTonggui = "yume_sort_tonggui",
    YumeSortHarmonic = "yume_sort_harmonic",
    YumeLaneTonggui = "yume_lane_tonggui",
    YumeLaneTongfan = "yume_lane_tongfan",
    YumeLaneTai = "yume_lane_tai",
    YumeBuiltinPinyin = "yume_builtin_pinyin",
    YumeBuiltinLingming = "yume_builtin_lingming",
    YumeBuiltinXingchen = "yume_builtin_xingchen",
    TableSourceView = "table_source_view",
    TableBasicView = "table_basic_view",
    TableWindowView = "table_window_view",
    TableLeaveWindow = "table_leave_window",
    GotoNextTable = "goto_next_table",
    GotoPrevTable = "goto_prev_table",
    TableConvertMode = "table_convert_mode",
    TableCellWidth = "table_cell_width",
    TableSearchColumns = "table_search_columns",
    TableGotoCell = "table_goto_cell",
    TableEnterCell = "table_enter_cell",
    TableLeaveCell = "table_leave_cell",
    TableNextCell = "table_next_cell",
    TablePrevCell = "table_prev_cell",
    TableYankColumn = "table_yank_column",
    TableReplaceColumn = "table_replace_column",
    TablePasteColumnRight = "table_paste_column_right",
    TablePasteColumnLeft = "table_paste_column_left",
    TableAddRowBelow = "table_add_row_below",
    TableAddRowAbove = "table_add_row_above",
    TableAddColumnRight = "table_add_column_right",
    TableAddColumnLeft = "table_add_column_left",
    TableDeleteRow = "table_delete_row",
    TableDeleteColumn = "table_delete_column",
    TableMoveRowDown = "table_move_row_down",
    TableMoveRowUp = "table_move_row_up",
    TableMoveColumnLeft = "table_move_column_left",
    TableMoveColumnRight = "table_move_column_right",
    TableAlignLeft = "table_align_left",
    TableAlignCenter = "table_align_center",
    TableAlignRight = "table_align_right",
    TableLineUp = "table_line_up",
    TableRecordPanel = "table_record_panel",
    TableFirstRowIsData = "table_first_row_is_data",
    TableSortAscending = "table_sort_ascending",
    TableSortDescending = "table_sort_descending",
    ConvertTableToPipe = "convert_table_to_pipe",
    PasteTableAsPipe = "paste_table_as_pipe",
    ConvertTableToCsv = "convert_table_to_csv",
    PasteTableAsCsv = "paste_table_as_csv",
    ConvertTableToTsv = "convert_table_to_tsv",
    PasteTableAsTsv = "paste_table_as_tsv",
    NoOp = "no_op",
    ExtendCharLeft = "extend_char_left",
    ExtendCharRight = "extend_char_right",
    ExtendVisualLineUp = "extend_visual_line_up",
    ExtendVisualLineDown = "extend_visual_line_down",
    ExtendLineUp = "extend_line_up",
    ExtendLineDown = "extend_line_down",
    ExtendNextWordStart = "extend_next_word_start",
    ExtendPrevWordStart = "extend_prev_word_start",
    ExtendNextWordEnd = "extend_next_word_end",
    ExtendNextLongWordStart = "extend_next_long_word_start",
    ExtendPrevLongWordStart = "extend_prev_long_word_start",
    ExtendNextLongWordEnd = "extend_next_long_word_end",
    ExtendSearchNext = "extend_search_next",
    ExtendSearchPrev = "extend_search_prev",
    ExtendTillChar = "extend_till_char",
    ExtendNextChar = "extend_next_char",
    ExtendTillPrevChar = "extend_till_prev_char",
    ExtendPrevChar = "extend_prev_char",
    ExtendToLineStart = "extend_to_line_start",
    ExtendToLineEnd = "extend_to_line_end",
    ExtendToFileStart = "extend_to_file_start",
    ExtendToFileEnd = "extend_to_file_end",
    ExtendToWord = "extend_to_word",
    ExtendToLastLine = "extend_to_last_line",
    InsertNewline = "insert_newline",
    DeleteCharForward = "delete_char_forward",
    DeleteWordForward = "delete_word_forward",
    DeleteWordBackward = "delete_word_backward",
    KillToLineStart = "kill_to_line_start",
    KillToLineEnd = "kill_to_line_end",
    InsertRegister = "insert_register",
    RunOneNormalCommand = "run_one_normal_command",
    GotoLineEndNewline = "goto_line_end_newline",
    CommitUndoCheckpoint = "commit_undo_checkpoint",
    InsertTab = "insert_tab",
    InsertOtherTab = "insert_other_tab",
    VimNormalMode = "vim_normal_mode",
    VimGotoLineEnd = "vim_goto_line_end",
    VimInsertMode = "vim_insert_mode",
    VimAppendMode = "vim_append_mode",
    VimInsertAtLineStart = "vim_insert_at_line_start",
    VimInsertAtLineEnd = "vim_insert_at_line_end",
    VimOpenBelow = "vim_open_below",
    VimOpenAbove = "vim_open_above",
    VimJoinSelections = "vim_join_selections",
    VimSwitchCase = "vim_switch_case",
    VimGotoLine = "vim_goto_line",
    VimIndent = "vim_indent",
    VimUnindent = "vim_unindent",
    VimPasteAfter = "vim_paste_after",
    VimPasteBefore = "vim_paste_before",
    VimReplayLastMacro = "vim_replay_last_macro",
    VimPasteAfterIndented = "vim_paste_after_indented",
    VimPasteBeforeIndented = "vim_paste_before_indented",
    VimIncrement = "vim_increment",
    VimDecrement = "vim_decrement",
    VimSearchNext = "vim_search_next",
    VimSearchPrev = "vim_search_prev",
    VimMatchBrackets = "vim_match_brackets",
    VimKillToLineStart = "vim_kill_to_line_start",
    VimKillToLineEnd = "vim_kill_to_line_end",
    VimFlipSelections = "vim_flip_selections",
    VimSwitchToLowercase = "vim_switch_to_lowercase",
    VimSwitchToUppercase = "vim_switch_to_uppercase",
    VimYank = "vim_yank",
    VimVisualBlockMode = "vim_visual_block_mode",
    VimRecordMacro = "vim_record_macro",
    VimReplayMacro = "vim_replay_macro",
    VimGotoFirstNonwhitespace = "vim_goto_first_nonwhitespace",
}

const fn command(name: &'static str, help: &'static str, c: &'static str) -> Action {
    Action { name, help, how: How::Command(c) }
}

/// **Every action a key may be bound to**, in the order the manual lists them.
///
/// Every Normal key is here by name, the keys inside the groups (`g h`,
/// `空格 f`, `C-w s`) as well as the keys that open them (`goto_mode`).
pub const ALL: &[Action] = &[
    // ---- Moving ---------------------------------------------------------
    normal("action.move-char-left", NormalCmd::MoveCharLeft),
    normal("action.move-char-right", NormalCmd::MoveCharRight),
    // **helix's names for what `k`/`j` do**: one line on screen. They are not
    // `move_line_up`/`move_line_down`, which in helix are `gk`/`gj`, a whole
    // line of the file.
    normal("action.move-visual-line-up", NormalCmd::MoveVisualLineUp),
    normal("action.move-visual-line-down", NormalCmd::MoveVisualLineDown),
    normal("action.move-next-word-start", NormalCmd::MoveNextWordStart),
    normal("action.move-prev-word-start", NormalCmd::MovePrevWordStart),
    normal("action.move-next-word-end", NormalCmd::MoveNextWordEnd),
    normal("action.move-next-long-word-start", NormalCmd::MoveNextLongWordStart),
    normal("action.move-prev-long-word-start", NormalCmd::MovePrevLongWordStart),
    normal("action.move-next-long-word-end", NormalCmd::MoveNextLongWordEnd),
    normal("action.goto-line-start", NormalCmd::GotoLineStart),
    normal("action.goto-line-end", NormalCmd::GotoLineEnd),
    normal("action.goto-first-nonwhitespace", NormalCmd::GotoFirstNonwhitespace),
    normal("action.goto-file-start", NormalCmd::GotoFileStart),
    normal("action.vim-goto-file-start", NormalCmd::VimGotoFileStart),
    normal("action.goto-file-end", NormalCmd::GotoFileEnd),
    // **Not `G`**: under the helix preset a bare `G` does nothing, as in
    // helix. `ge` then `gh` is helix's `goto_last_line`: the last line, at
    // its start.
    normal("action.goto-last-line", NormalCmd::GotoLastLine),
    normal("action.goto-prev-sentence", NormalCmd::GotoPrevSentence),
    normal("action.goto-next-sentence", NormalCmd::GotoNextSentence),
    // helix's `[p`/`]p`: a paragraph, not `gk`/`gj`'s one file line.
    normal("action.goto-prev-paragraph", NormalCmd::GotoPrevParagraph),
    normal("action.goto-next-paragraph", NormalCmd::GotoNextParagraph),
    normal("action.half-page-down", NormalCmd::PageCursorHalfDown),
    normal("action.half-page-up", NormalCmd::PageCursorHalfUp),
    normal("action.jump-backward", NormalCmd::JumpBackward),
    // ---- Selecting ------------------------------------------------------
    // helix's names, so helix's own names can be bound.
    normal("action.extend-mode", NormalCmd::SelectMode),
    normal("action.select-line", NormalCmd::ExtendLineBelow),
    normal("action.extend-to-line-bounds", NormalCmd::ExtendToLineBounds),
    normal("action.select-all", NormalCmd::SelectAll),
    // Warning: **`;`, not Esc.** Esc leaves 延伸 mode and leaves the selection
    // alone, the way helix's does; `;` is the key whose whole job is collapsing.
    normal("action.collapse-selection", NormalCmd::CollapseSelection),
    // ---- Changing -------------------------------------------------------
    // Warning: **這四個的名字與意思都是 helix 的**（這個模組開頭那條規矩：helix
    // 有同一個動作就用它的名字）：`delete_selection` 進寄存器，`_noyank` 那兩個
    // 不進。同名而意思相反是最壞的一種——一個 helix 使用者
    // 寫 `d = "delete_selection"` 會得到和他家裏相反的行為。
    normal("action.delete-selection", NormalCmd::DeleteSelection),
    normal("action.delete-selection-noyank", NormalCmd::DeleteSelectionNoyank),
    normal("action.change-selection", NormalCmd::ChangeSelection),
    normal("action.change-selection-noyank", NormalCmd::ChangeSelectionNoyank),
    normal("action.insert-mode", NormalCmd::InsertMode),
    normal("action.append-mode", NormalCmd::AppendMode),
    normal("action.insert-at-line-start", NormalCmd::InsertAtLineStart),
    normal("action.insert-at-line-end", NormalCmd::InsertAtLineEnd),
    normal("action.open-below", NormalCmd::OpenBelow),
    normal("action.open-above", NormalCmd::OpenAbove),
    normal("action.yank", NormalCmd::Yank),
    normal("action.paste-after", NormalCmd::PasteAfter),
    normal("action.paste-before", NormalCmd::PasteBefore),
    normal("action.replace-with-yanked", NormalCmd::ReplaceWithYanked),
    normal("action.join-lines", NormalCmd::JoinSelections),
    normal("action.indent", NormalCmd::Indent),
    normal("action.unindent", NormalCmd::Unindent),
    normal("action.increment", NormalCmd::Increment),
    normal("action.decrement", NormalCmd::Decrement),
    normal("action.switch-case", NormalCmd::SwitchCase),
    normal("action.undo", NormalCmd::Undo),
    normal("action.redo", NormalCmd::Redo),
    normal("action.repeat-last-change", NormalCmd::RepeatLastChange),
    // ---- Looking for something ------------------------------------------
    normal("action.search", NormalCmd::Search),
    normal("action.rsearch", NormalCmd::Rsearch),
    normal("action.search-next", NormalCmd::SearchNext),
    normal("action.search-prev", NormalCmd::SearchPrev),
    normal("action.search-selection", NormalCmd::SearchSelection),
    normal("action.goto-definition", NormalCmd::GotoDefinition),
    normal("action.open-here", NormalCmd::GotoFile),
    normal("action.follow-link", NormalCmd::FollowLink),
    // `gx`, vim's: the link under the cursor, handed to the system.
    normal("action.open-link", NormalCmd::OpenLink),
    // helix's `gD` `gy` `gi` `gm`.
    normal("hint.goto.declaration", NormalCmd::GotoDeclaration),
    normal("hint.goto.type-definition", NormalCmd::GotoTypeDefinition),
    normal("hint.goto.implementation", NormalCmd::GotoImplementation),
    normal("hint.goto.last-modified-file", NormalCmd::GotoLastModifiedFile),
    // vim's Insert `C-y` `C-e`.
    normal("action.insert-char-above", NormalCmd::InsertCharAbove),
    normal("action.insert-char-below", NormalCmd::InsertCharBelow),
    // ---- Files, panels, the session --------------------------------------
    normal("action.file-picker", NormalCmd::FilePicker),
    normal("action.buffer-picker", NormalCmd::BufferPicker),
    normal("action.next-buffer", NormalCmd::GotoNextBuffer),
    normal("action.prev-buffer", NormalCmd::GotoPreviousBuffer),
    // `空格 s` is helix's symbol picker; the outline sidebar is a command.
    command("outline", "action.outline", "sidebar-outline"),
    normal("action.dictionary", NormalCmd::Dictionary),
    normal("action.next-region", NormalCmd::RotateView),
    normal("action.search-panel", NormalCmd::GlobalSearch),
    normal("action.command-menu", NormalCmd::CommandPalette),
    normal("action.paste-menu", NormalCmd::PasteMenu),
    normal("action.comment-line", NormalCmd::ToggleComments),
    normal("action.comment-block", NormalCmd::ToggleBlockComments),
    normal("hint.space.comment-lines-only", NormalCmd::ToggleLineComments),
    normal("action.copy-to-clipboard", NormalCmd::YankToClipboard),
    normal("hint.space.copy-main-to-clipboard", NormalCmd::YankMainSelectionToClipboard),
    normal("hint.space.jumplist", NormalCmd::JumplistPicker),
    normal("action.goto-first-change", NormalCmd::GotoFirstChange),
    normal("action.goto-last-change", NormalCmd::GotoLastChange),
    normal("action.suspend", NormalCmd::Suspend),
    normal("hint.space.select-references", NormalCmd::SelectReferencesToSymbolUnderCursor),
    normal("action.format-selections", NormalCmd::FormatSelections),
    normal("action.expand-selection", NormalCmd::ExpandSelection),
    normal("action.shrink-selection", NormalCmd::ShrinkSelection),
    normal("action.select-next-sibling", NormalCmd::SelectNextSibling),
    normal("action.select-prev-sibling", NormalCmd::SelectPrevSibling),
    normal("action.select-all-siblings", NormalCmd::SelectAllSiblings),
    normal("action.select-all-children", NormalCmd::SelectAllChildren),
    normal("action.move-parent-node-end", NormalCmd::MoveParentNodeEnd),
    normal("action.move-parent-node-start", NormalCmd::MoveParentNodeStart),
    normal("action.extend-parent-node-end", NormalCmd::ExtendParentNodeEnd),
    normal("action.extend-parent-node-start", NormalCmd::ExtendParentNodeStart),
    normal("hint.space.last-picker", NormalCmd::LastPicker),
    normal("action.paste-from-clipboard", NormalCmd::PasteClipboardAfter),
    command("ruby", "action.ruby", "ruby"),
    command("save_file", "action.save-file", "write"),
    command("save_and_quit", "action.save-and-quit", "wq"),
    command("count_progress", "action.count-progress", "count-progress"),
    // ---- Every other Normal key: each a command by name ----------------
    normal("action.move-line-up", NormalCmd::MoveLineUp),
    normal("action.move-line-down", NormalCmd::MoveLineDown),
    normal("action.goto-next-line-nonwhitespace", NormalCmd::GotoNextLineNonwhitespace),
    normal("action.jump-next-word-start", NormalCmd::JumpNextWordStart),
    normal("action.jump-next-long-word-start", NormalCmd::JumpNextLongWordStart),
    normal("action.jump-prev-word-start", NormalCmd::JumpPrevWordStart),
    normal("action.jump-prev-long-word-start", NormalCmd::JumpPrevLongWordStart),
    normal("action.jump-next-word-end", NormalCmd::JumpNextWordEnd),
    normal("action.jump-next-long-word-end", NormalCmd::JumpNextLongWordEnd),
    normal("action.jump-next-paragraph", NormalCmd::JumpNextParagraph),
    normal("action.jump-prev-paragraph", NormalCmd::JumpPrevParagraph),
    normal("action.move-next-paragraph", NormalCmd::MoveNextParagraph),
    normal("action.move-prev-paragraph", NormalCmd::MovePrevParagraph),
    normal("action.goto-window-top", NormalCmd::GotoWindowTop),
    normal("action.goto-window-center", NormalCmd::GotoWindowCenter),
    normal("action.goto-window-bottom", NormalCmd::GotoWindowBottom),
    normal("action.jump-prev-sentence", NormalCmd::JumpPrevSentence),
    normal("action.jump-next-sentence", NormalCmd::JumpNextSentence),
    normal("action.add-mark", NormalCmd::AddMark),
    normal("action.goto-mark", NormalCmd::GotoMark),
    normal("action.goto-mark-exact", NormalCmd::GotoMarkExact),
    normal("action.view-mode", NormalCmd::ViewMode),
    normal("action.sticky-view-mode", NormalCmd::StickyViewMode),
    normal("action.right-bracket-mode", NormalCmd::RightBracketMode),
    normal("action.left-bracket-mode", NormalCmd::LeftBracketMode),
    normal("action.goto-mode", NormalCmd::GotoMode),
    normal("action.space-mode", NormalCmd::SpaceMode),
    normal("action.find-next-char", NormalCmd::FindNextChar),
    normal("action.find-prev-char", NormalCmd::FindPrevChar),
    normal("action.find-till-char", NormalCmd::FindTillChar),
    normal("action.till-prev-char", NormalCmd::TillPrevChar),
    normal("action.select-line-mode", NormalCmd::SelectLineMode),
    normal("action.normal-mode", NormalCmd::NormalMode),
    normal("action.repeat-find-char", NormalCmd::RepeatFindChar),
    normal("action.repeat-find-char-reverse", NormalCmd::RepeatFindCharReverse),
    normal("action.copy-selection-on-next-line", NormalCmd::CopySelectionOnNextLine),
    normal("action.copy-selection-on-prev-line", NormalCmd::CopySelectionOnPrevLine),
    normal("action.keep-primary-selection", NormalCmd::KeepPrimarySelection),
    normal("action.remove-primary-selection", NormalCmd::RemovePrimarySelection),
    normal("action.merge-selections", NormalCmd::MergeSelections),
    normal("action.merge-consecutive-selections", NormalCmd::MergeConsecutiveSelections),
    normal("action.ensure-selections-forward", NormalCmd::EnsureSelectionsForward),
    normal("action.shrink-to-line-bounds", NormalCmd::ShrinkToLineBounds),
    normal("action.split-selection-on-newline", NormalCmd::SplitSelectionOnNewline),
    normal("action.rotate-selections-forward", NormalCmd::RotateSelectionsForward),
    normal("action.rotate-selections-backward", NormalCmd::RotateSelectionsBackward),
    normal("action.trim-selections", NormalCmd::TrimSelections),
    normal("action.align-selections", NormalCmd::AlignSelections),
    normal("action.rotate-selection-contents-forward", NormalCmd::RotateSelectionContentsForward),
    normal("action.rotate-selection-contents-backward", NormalCmd::RotateSelectionContentsBackward),
    normal("action.select-regex", NormalCmd::SelectRegex),
    normal("action.split-selection", NormalCmd::SplitSelection),
    normal("action.keep-selections", NormalCmd::KeepSelections),
    normal("action.remove-selections", NormalCmd::RemoveSelections),
    normal("action.operator-delete", NormalCmd::OperatorDelete),
    normal("action.operator-change", NormalCmd::OperatorChange),
    normal("action.operator-yank", NormalCmd::OperatorYank),
    normal("action.operator-indent", NormalCmd::OperatorIndent),
    normal("action.operator-unindent", NormalCmd::OperatorUnindent),
    normal("action.delete-to-line-end", NormalCmd::DeleteToLineEnd),
    normal("action.delete-to-line-end-noyank", NormalCmd::DeleteToLineEndNoyank),
    // vim's `x`: the character under the cursor, never the line break.
    normal("action.vim-delete-char-forward", NormalCmd::VimDeleteCharForward),
    // ---- Flavour commands: what the flavour's keys run -----------------
    normal("action.extend-line-above", NormalCmd::ExtendLineAbove),
    normal("action.move-selected-lines-down", NormalCmd::MoveSelectedLinesDown),
    normal("action.move-selected-lines-up", NormalCmd::MoveSelectedLinesUp),
    normal("action.goto-line-or-last-line", NormalCmd::GotoLineOrLastLine),
    normal("action.normal-mode-and-collapse", NormalCmd::NormalModeAndCollapse),
    // `:quit` as a Normal command, so a group's row can run it (the
    // flavour's `Space q`); it was a command-line action before.
    normal("action.quit", NormalCmd::Quit),
    normal("action.select-inner-object", NormalCmd::SelectInnerObject),
    normal("action.select-around-object", NormalCmd::SelectAroundObject),
    normal("action.undo-line", NormalCmd::UndoLine),
    normal("action.earlier", NormalCmd::Earlier),
    normal("action.later", NormalCmd::Later),
    normal("action.goto-column", NormalCmd::GotoColumn),
    normal("action.shell-pipe", NormalCmd::ShellPipe),
    normal("action.shell-insert-output", NormalCmd::ShellInsertOutput),
    normal("action.shell-append-output", NormalCmd::ShellAppendOutput),
    normal("action.shell-pipe-to", NormalCmd::ShellPipeTo),
    normal("cmd.commands.keep-pipe", NormalCmd::ShellKeepPipe),
    normal("action.command-mode", NormalCmd::CommandMode),
    normal("action.vim-command-mode", NormalCmd::VimCommandMode),
    normal("action.match-mode", NormalCmd::MatchMode),
    normal("action.replace", NormalCmd::Replace),
    normal("action.select-register", NormalCmd::SelectRegister),
    normal("action.record-macro", NormalCmd::RecordMacro),
    normal("action.replay-macro", NormalCmd::ReplayMacro),
    normal("action.window-mode", NormalCmd::WindowMode),
    normal("action.page-down", NormalCmd::PageDown),
    normal("action.page-up", NormalCmd::PageUp),
    normal("action.join-selections-space", NormalCmd::JoinSelectionsSpace),
    normal("action.save-selection", NormalCmd::SaveSelection),
    normal("action.jump-forward", NormalCmd::JumpForward),
    normal("action.two-thirds-page-down", NormalCmd::TwoThirdsPageDown),
    normal("action.two-thirds-page-up", NormalCmd::TwoThirdsPageUp),
    normal("action.flip-selections", NormalCmd::FlipSelections),
    normal("action.vim-redo", NormalCmd::VimRedo),
    normal("action.delete-char-backward", NormalCmd::DeleteCharBackward),
    normal("action.case-mode", NormalCmd::CaseMode),
    normal("action.search-selection-detect-word-boundaries", NormalCmd::SearchSelectionDetectWordBoundaries),
    normal("action.search-selection-exact", NormalCmd::SearchSelectionExact),
    normal("action.search-word-backward", NormalCmd::SearchWordBackward),
    normal("action.replace-mode", NormalCmd::ReplaceMode),
    normal("action.goto-line", NormalCmd::GotoLine),
    normal("action.vim-repeat-last-change", NormalCmd::VimRepeatLastChange),
    normal("action.repeat-last-motion", NormalCmd::RepeatLastMotion),
    // ---- The groups' keys: `g`, `Space`, `[` `]`, `z`, `m`,
    // `C-w`, the backtick group, `Space t` and the rest, each by name ----
    normal("action.jump-prev-word-end", NormalCmd::JumpPrevWordEnd),
    normal("action.jump-prev-long-word-end", NormalCmd::JumpPrevLongWordEnd),
    normal("hint.goto.jump-labels", NormalCmd::GotoWord),
    normal("hint.goto.seek-written", NormalCmd::GotoWordByLetters),
    normal("hint.goto.split-join", NormalCmd::SplitJoin),
    normal("hint.goto.seek-reading", NormalCmd::GotoCharByPinyin),
    normal("hint.goto.last-file", NormalCmd::GotoLastAccessedFile),
    normal("hint.goto.references", NormalCmd::GotoReference),
    normal("action.move-next-zong", NormalCmd::MoveNextZong),
    normal("action.move-prev-zong", NormalCmd::MovePrevZong),
    normal("hint.goto.last-change", NormalCmd::GotoLastModification),
    normal("hint.goto.reselect", NormalCmd::ReselectLastSelection),
    normal("hint.goto.insert-again", NormalCmd::InsertAtLastInsert),
    normal("action.goto-older-change", NormalCmd::GotoOlderChange),
    normal("action.goto-newer-change", NormalCmd::GotoNewerChange),
    normal("action.operator-lowercase", NormalCmd::OperatorLowercase),
    normal("action.operator-uppercase", NormalCmd::OperatorUppercase),
    normal("action.operator-switch-case", NormalCmd::OperatorSwitchCase),
    normal("action.operator-comment", NormalCmd::OperatorComment),
    normal("hint.goto.fill", NormalCmd::OperatorFill),
    // vim's `gw`: `gq`, and the cursor stays where it was.
    normal("action.operator-fill-keep-cursor", NormalCmd::OperatorFillKeepCursor),
    normal("hint.goto.join-without-a-space", NormalCmd::JoinSelectionsNoSpace),
    normal("hint.space.open-file-here", NormalCmd::FilePickerInCurrentDirectory),
    normal("hint.space.explorer", NormalCmd::FileExplorer),
    normal("hint.space.explorer-here", NormalCmd::FileExplorerInCurrentBufferDirectory),
    normal("hint.space.changed-files", NormalCmd::ChangedFilePicker),
    normal("hint.space.paste-before", NormalCmd::PasteClipboardBefore),
    normal("hint.space.replace-from-clipboard", NormalCmd::ReplaceSelectionsWithClipboard),
    normal("hint.space.symbols", NormalCmd::LspOrSyntaxSymbolPicker),
    normal("hint.space.project-symbols", NormalCmd::LspOrSyntaxWorkspaceSymbolPicker),
    normal("hint.space.dictionary-panel", NormalCmd::DictionaryPanel),
    normal("hint.space.what-is-this", NormalCmd::Hover),
    normal("hint.space.what-is-this-panel", NormalCmd::HoverPanel),
    normal("hint.space.diagnostics", NormalCmd::DiagnosticsPicker),
    normal("hint.space.project-diagnostics", NormalCmd::WorkspaceDiagnosticsPicker),
    normal("hint.space.problems", NormalCmd::ShowDiagnostic),
    normal("hint.space.problems-panel", NormalCmd::ShowDiagnosticPanel),
    normal("hint.space.change", NormalCmd::ShowChange),
    normal("hint.space.change-panel", NormalCmd::ShowChangePanel),
    normal("hint.space.rename", NormalCmd::RenameSymbol),
    normal("hint.space.actions", NormalCmd::CodeAction),
    normal("hint.space.record", NormalCmd::ShowRecord),
    normal("hint.space.record-panel", NormalCmd::ShowRecordPanel),
    normal("hint.space.table", NormalCmd::TableMode),
    normal("hint.conflict.title", NormalCmd::ConflictMode),
    normal("hint.space.open-recent", NormalCmd::OpenRecentMode),
    normal("hint.space.yume", NormalCmd::YumeMode),
    normal("action.goto-next-conflict", NormalCmd::GotoNextConflict),
    normal("action.goto-prev-conflict", NormalCmd::GotoPrevConflict),
    normal("action.goto-next-change", NormalCmd::GotoNextChange),
    normal("action.goto-prev-change", NormalCmd::GotoPrevChange),
    normal("action.goto-next-diag", NormalCmd::GotoNextDiag),
    normal("action.goto-prev-diag", NormalCmd::GotoPrevDiag),
    normal("action.goto-last-diag", NormalCmd::GotoLastDiag),
    normal("action.goto-first-diag", NormalCmd::GotoFirstDiag),
    normal("action.goto-next-function", NormalCmd::GotoNextFunction),
    normal("action.goto-prev-function", NormalCmd::GotoPrevFunction),
    normal("action.goto-next-class", NormalCmd::GotoNextClass),
    normal("action.goto-prev-class", NormalCmd::GotoPrevClass),
    normal("action.goto-next-comment", NormalCmd::GotoNextComment),
    normal("action.goto-prev-comment", NormalCmd::GotoPrevComment),
    normal("action.goto-next-parameter", NormalCmd::GotoNextParameter),
    normal("action.goto-prev-parameter", NormalCmd::GotoPrevParameter),
    normal("action.goto-next-entry", NormalCmd::GotoNextEntry),
    normal("action.goto-prev-entry", NormalCmd::GotoPrevEntry),
    normal("action.goto-next-test", NormalCmd::GotoNextTest),
    normal("action.goto-prev-test", NormalCmd::GotoPrevTest),
    normal("action.goto-next-xml-element", NormalCmd::GotoNextXmlElement),
    normal("action.goto-prev-xml-element", NormalCmd::GotoPrevXmlElement),
    normal("hint.hop.blank-below", NormalCmd::AddNewlineBelow),
    normal("hint.hop.blank-above", NormalCmd::AddNewlineAbove),
    normal("action.align-view-center", NormalCmd::AlignViewCenter),
    normal("action.align-view-top", NormalCmd::AlignViewTop),
    normal("action.align-view-bottom", NormalCmd::AlignViewBottom),
    normal("action.scroll-down", NormalCmd::ScrollDown),
    normal("action.scroll-up", NormalCmd::ScrollUp),
    normal("hint.match.pair", NormalCmd::MatchBrackets),
    normal("hint.match.inside", NormalCmd::SelectTextobjectInner),
    normal("hint.match.around", NormalCmd::SelectTextobjectAround),
    normal("hint.match.surround", NormalCmd::SurroundAdd),
    normal("hint.match.take-off", NormalCmd::SurroundDelete),
    normal("hint.change", NormalCmd::SurroundReplace),
    normal("action.jump-view-left", NormalCmd::JumpViewLeft),
    normal("action.jump-view-down", NormalCmd::JumpViewDown),
    normal("action.jump-view-up", NormalCmd::JumpViewUp),
    normal("action.jump-view-right", NormalCmd::JumpViewRight),
    normal("action.toggle-left-sidebar", NormalCmd::ToggleLeftSidebar),
    normal("action.toggle-right-sidebar", NormalCmd::ToggleRightSidebar),
    normal("action.goto-left-sidebar", NormalCmd::GotoLeftSidebar),
    normal("action.goto-right-sidebar", NormalCmd::GotoRightSidebar),
    normal("hint.region.split", NormalCmd::Hsplit),
    normal("hint.region.split-above", NormalCmd::HsplitAbove),
    normal("hint.region.split-left", NormalCmd::VsplitLeft),
    normal("hint.region.split-beside", NormalCmd::Vsplit),
    normal("hint.region.new-below", NormalCmd::HsplitNew),
    normal("hint.region.new-beside", NormalCmd::VsplitNew),
    normal("action.window-new-mode", NormalCmd::WindowNewMode),
    normal("action.new-window-left", NormalCmd::NewWindowLeft),
    normal("action.new-window-down", NormalCmd::NewWindowDown),
    normal("action.new-window-up", NormalCmd::NewWindowUp),
    normal("action.new-window-right", NormalCmd::NewWindowRight),
    normal("action.new-window-left-pick", NormalCmd::NewWindowLeftPick),
    normal("action.new-window-down-pick", NormalCmd::NewWindowDownPick),
    normal("action.new-window-up-pick", NormalCmd::NewWindowUpPick),
    normal("action.new-window-right-pick", NormalCmd::NewWindowRightPick),
    normal("hint.region.close", NormalCmd::Wclose),
    normal("hint.region.only", NormalCmd::Wonly),
    normal("hint.region.transpose", NormalCmd::TransposeView),
    normal("action.swap-view-left", NormalCmd::SwapViewLeft),
    normal("action.swap-view-down", NormalCmd::SwapViewDown),
    normal("action.swap-view-up", NormalCmd::SwapViewUp),
    normal("action.swap-view-right", NormalCmd::SwapViewRight),
    normal("hint.region.file-below", NormalCmd::GotoFileHsplit),
    normal("hint.region.file-beside", NormalCmd::GotoFileVsplit),
    normal("hint.region.file-above", NormalCmd::GotoFileHsplitAbove),
    normal("hint.fold.create-selection", NormalCmd::CreateFold),
    normal("hint.fold.create", NormalCmd::VimCreateFold),
    normal("hint.fold.create-lines", NormalCmd::CreateFoldLines),
    normal("hint.fold.open", NormalCmd::OpenFold),
    normal("hint.fold.open-all-here", NormalCmd::OpenFoldsRecursively),
    normal("hint.fold.close", NormalCmd::CloseFold),
    normal("hint.fold.close-all-here", NormalCmd::CloseFoldsRecursively),
    normal("hint.fold.toggle", NormalCmd::ToggleFold),
    normal("hint.fold.toggle-all-here", NormalCmd::ToggleFoldsRecursively),
    normal("hint.fold.view", NormalCmd::OpenFoldsToCursor),
    normal("hint.fold.open-every", NormalCmd::OpenAllFolds),
    normal("hint.fold.close-every", NormalCmd::CloseAllFolds),
    normal("hint.fold.delete", NormalCmd::DeleteFold),
    normal("hint.fold.delete-all-here", NormalCmd::DeleteFoldsRecursively),
    normal("hint.fold.delete-every", NormalCmd::DeleteAllFolds),
    normal("hint.fold.next", NormalCmd::GotoNextFold),
    normal("hint.fold.prev", NormalCmd::GotoPrevFold),
    normal("hint.fold.off", NormalCmd::DisableFolds),
    normal("hint.fold.on", NormalCmd::EnableFolds),
    normal("hint.fold.on-off", NormalCmd::ToggleFoldsEnabled),
    normal("hint.fold.more", NormalCmd::FoldMore),
    normal("hint.fold.less", NormalCmd::FoldLess),
    normal("hint.fold.start", NormalCmd::GotoFoldStart),
    normal("hint.fold.end", NormalCmd::GotoFoldEnd),
    normal("hint.fold.reapply", NormalCmd::ReapplyFoldLevelAndView),
    normal("hint.fold.reapply-only", NormalCmd::ReapplyFoldLevel),
    normal("hint.aim.top-past-indent", NormalCmd::VimAlignViewTop),
    normal("hint.aim.middle-past-indent", NormalCmd::VimAlignViewCenter),
    normal("hint.aim.bottom-past-indent", NormalCmd::VimAlignViewBottom),
    normal("hint.aim.below-to-top", NormalCmd::VimAlignViewBelowTop),
    normal("hint.aim.above-to-bottom", NormalCmd::VimAlignViewAboveBottom),
    normal("hint.aim.view-left", NormalCmd::ScrollViewLeft),
    normal("hint.aim.view-right", NormalCmd::ScrollViewRight),
    normal("hint.aim.view-half-left", NormalCmd::ScrollViewHalfLeft),
    normal("hint.aim.view-half-right", NormalCmd::ScrollViewHalfRight),
    normal("hint.aim.cursor-left-edge", NormalCmd::AlignViewCursorLeft),
    normal("hint.aim.cursor-right-edge", NormalCmd::AlignViewCursorRight),
    normal("hint.aim.middle-sideways", NormalCmd::AlignViewMiddle),
    normal("hint.region.previous", NormalCmd::RotateViewReverse),
    normal("hint.region.first", NormalCmd::JumpViewFirst),
    normal("hint.region.last", NormalCmd::JumpViewLast),
    normal("hint.region.last-accessed", NormalCmd::JumpViewLastAccessed),
    normal("hint.region.exchange", NormalCmd::ExchangeView),
    normal("hint.region.rotate", NormalCmd::RotateViewsForward),
    normal("hint.region.rotate-back", NormalCmd::RotateViewsBackward),
    normal("action.move-view-far-left", NormalCmd::MoveViewFarLeft),
    normal("action.move-view-far-down", NormalCmd::MoveViewFarDown),
    normal("action.move-view-far-up", NormalCmd::MoveViewFarUp),
    normal("action.move-view-far-right", NormalCmd::MoveViewFarRight),
    normal("hint.region.close-not-last", NormalCmd::VimCloseWindow),
    normal("hint.region.last-file-above", NormalCmd::GotoLastAccessedFileHsplit),
    normal("hint.case.lower", NormalCmd::SwitchToLowercase),
    normal("hint.case.upper", NormalCmd::SwitchToUppercase),
    normal("hint.case.to-s", NormalCmd::ConvertToSimplified),
    normal("hint.case.to-t", NormalCmd::ConvertToTraditional),
    normal("hint.case.to-tw", NormalCmd::ConvertToTaiwanTraditional),
    normal("hint.case.to-hk", NormalCmd::ConvertToHongKongTraditional),
    normal("hint.case.to-c", NormalCmd::ConvertToMainlandTraditional),
    normal("hint.case.to-g", NormalCmd::ConvertToClassicalTraditional),
    normal("hint.case.to-jp", NormalCmd::ConvertToJapanese),
    normal("hint.case.to-half", NormalCmd::SwitchToHalfwidth),
    normal("hint.case.to-full", NormalCmd::SwitchToFullwidth),
    normal("hint.conflict.ours", NormalCmd::ConflictKeepOurs),
    normal("hint.conflict.theirs", NormalCmd::ConflictKeepTheirs),
    normal("hint.conflict.both", NormalCmd::ConflictKeepBoth),
    normal("hint.open.recent-files", NormalCmd::RecentFilePicker),
    normal("hint.open.recent-files-global", NormalCmd::GlobalRecentFilePicker),
    normal("hint.open.recent-projects", NormalCmd::ProjectPicker),
    normal("hint.open.resume", NormalCmd::ResumeSession),
    normal("hint.yume.toggle", NormalCmd::YumeToggle),
    normal("hint.yume.commit", NormalCmd::YumeCommitMode),
    normal("hint.yume.panel", NormalCmd::YumePanelMode),
    normal("hint.yume.preedit", NormalCmd::YumePreeditMode),
    normal("hint.yume.predict", NormalCmd::YumeTogglePrediction),
    normal("hint.yume.scheme", NormalCmd::YumePickScheme),
    normal("hint.yume.next-scheme", NormalCmd::YumeNextScheme),
    normal("action.yume-menu-size-3", NormalCmd::YumeMenuSize3),
    normal("action.yume-menu-size-4", NormalCmd::YumeMenuSize4),
    normal("action.yume-menu-size-5", NormalCmd::YumeMenuSize5),
    normal("action.yume-menu-size-6", NormalCmd::YumeMenuSize6),
    normal("action.yume-menu-size-7", NormalCmd::YumeMenuSize7),
    normal("action.yume-menu-size-8", NormalCmd::YumeMenuSize8),
    normal("action.yume-menu-size-9", NormalCmd::YumeMenuSize9),
    normal("hint.yume.annotation", NormalCmd::YumeNextAnnotation),
    normal("hint.yume.table", NormalCmd::YumeInstalledTable),
    normal("hint.yume.which", NormalCmd::YumeTableSource),
    normal("hint.yume.where", NormalCmd::YumeDataLocation),
    normal("hint.yume.commit-default", NormalCmd::YumeCommitDefault),
    normal("hint.yume.commit-delayed", NormalCmd::YumeCommitDelayed),
    normal("hint.yume.commit-unique", NormalCmd::YumeCommitUnique),
    normal("hint.yume.commit-fluency", NormalCmd::YumeCommitFluency),
    normal("hint.yume.panel-box", NormalCmd::YumePanelBox),
    normal("hint.yume.panel-bottom", NormalCmd::YumePanelBottom),
    normal("hint.yume.panel-inline", NormalCmd::YumePanelInline),
    normal("hint.yume.preedit-off", NormalCmd::YumePreeditOff),
    normal("hint.yume.preedit-composition", NormalCmd::YumePreeditComposition),
    normal("hint.yume.preedit-preview", NormalCmd::YumePreeditPreview),
    normal("hint.yume.charset", NormalCmd::YumeCharsetMode),
    normal("hint.yume.sort", NormalCmd::YumeSortMode),
    normal("hint.yume.lanes", NormalCmd::YumeLanesMode),
    normal("hint.yume.builtin", NormalCmd::YumeBuiltinMode),
    normal("ime.charset.all", NormalCmd::YumeCharsetAll),
    normal("ime.charset.common", NormalCmd::YumeCharsetCommon),
    normal("ime.charset.tonggui", NormalCmd::YumeCharsetTonggui),
    normal("ime.charset.harmonic", NormalCmd::YumeCharsetHarmonic),
    normal("ime.sort.original", NormalCmd::YumeSortOriginal),
    normal("ime.sort.common", NormalCmd::YumeSortCommon),
    normal("ime.sort.tonggui", NormalCmd::YumeSortTonggui),
    normal("ime.sort.harmonic", NormalCmd::YumeSortHarmonic),
    normal("ime.lane.tonggui", NormalCmd::YumeLaneTonggui),
    normal("ime.lane.tongfan", NormalCmd::YumeLaneTongfan),
    normal("ime.lane.tai", NormalCmd::YumeLaneTai),
    normal("cmd.schemes.pinyin", NormalCmd::YumeBuiltinPinyin),
    normal("cmd.schemes.lingming", NormalCmd::YumeBuiltinLingming),
    normal("cmd.schemes.xingchen", NormalCmd::YumeBuiltinXingchen),
    normal("hint.table.back-to-prose", NormalCmd::TableSourceView),
    normal("hint.table.operate-it", NormalCmd::TableBasicView),
    normal("hint.table.whole-window", NormalCmd::TableWindowView),
    normal("hint.table.leave-the-window", NormalCmd::TableLeaveWindow),
    normal("action.goto-next-table", NormalCmd::GotoNextTable),
    normal("action.goto-prev-table", NormalCmd::GotoPrevTable),
    normal("hint.table.convert", NormalCmd::TableConvertMode),
    normal("hint.table.fold-wide-cells", NormalCmd::TableCellWidth),
    normal("hint.table.search-columns", NormalCmd::TableSearchColumns),
    normal("hint.table.go-to-cell", NormalCmd::TableGotoCell),
    // ---- A table's grid: the four keys whose table meaning is
    // its own; every other key there is its Normal command, done to a cell.
    normal("action.table-enter-cell", NormalCmd::TableEnterCell),
    normal("action.table-leave-cell", NormalCmd::TableLeaveCell),
    normal("action.table-next-cell", NormalCmd::TableNextCell),
    normal("action.table-prev-cell", NormalCmd::TablePrevCell),
    normal("action.table-yank-column", NormalCmd::TableYankColumn),
    normal("hint.table.replace-column", NormalCmd::TableReplaceColumn),
    normal("action.table-paste-column-right", NormalCmd::TablePasteColumnRight),
    normal("action.table-paste-column-left", NormalCmd::TablePasteColumnLeft),
    normal("action.table-add-row-below", NormalCmd::TableAddRowBelow),
    normal("action.table-add-row-above", NormalCmd::TableAddRowAbove),
    normal("action.table-add-column-right", NormalCmd::TableAddColumnRight),
    normal("action.table-add-column-left", NormalCmd::TableAddColumnLeft),
    normal("hint.table.delete-row", NormalCmd::TableDeleteRow),
    normal("action.table-delete-column", NormalCmd::TableDeleteColumn),
    normal("action.table-move-row-down", NormalCmd::TableMoveRowDown),
    normal("action.table-move-row-up", NormalCmd::TableMoveRowUp),
    normal("action.table-move-column-left", NormalCmd::TableMoveColumnLeft),
    normal("action.table-move-column-right", NormalCmd::TableMoveColumnRight),
    normal("action.table-align-left", NormalCmd::TableAlignLeft),
    normal("action.table-align-center", NormalCmd::TableAlignCenter),
    normal("action.table-align-right", NormalCmd::TableAlignRight),
    normal("hint.table.line-it-up", NormalCmd::TableLineUp),
    normal("hint.table.detail-panel", NormalCmd::TableRecordPanel),
    normal("hint.table.first-row-is-data", NormalCmd::TableFirstRowIsData),
    normal("action.table-sort-ascending", NormalCmd::TableSortAscending),
    normal("action.table-sort-descending", NormalCmd::TableSortDescending),
    normal("action.convert-table-to-pipe", NormalCmd::ConvertTableToPipe),
    normal("action.paste-table-as-pipe", NormalCmd::PasteTableAsPipe),
    normal("action.convert-table-to-csv", NormalCmd::ConvertTableToCsv),
    normal("action.paste-table-as-csv", NormalCmd::PasteTableAsCsv),
    normal("action.convert-table-to-tsv", NormalCmd::ConvertTableToTsv),
    normal("action.paste-table-as-tsv", NormalCmd::PasteTableAsTsv),
    normal("action.no-op", NormalCmd::NoOp),
    // ---- Select mode: each a motion, extending the selection ----
    normal("action.extend-char-left", NormalCmd::ExtendCharLeft),
    normal("action.extend-char-right", NormalCmd::ExtendCharRight),
    normal("action.extend-visual-line-up", NormalCmd::ExtendVisualLineUp),
    normal("action.extend-visual-line-down", NormalCmd::ExtendVisualLineDown),
    normal("action.extend-line-up", NormalCmd::ExtendLineUp),
    normal("action.extend-line-down", NormalCmd::ExtendLineDown),
    normal("action.extend-next-word-start", NormalCmd::ExtendNextWordStart),
    normal("action.extend-prev-word-start", NormalCmd::ExtendPrevWordStart),
    normal("action.extend-next-word-end", NormalCmd::ExtendNextWordEnd),
    normal("action.extend-next-long-word-start", NormalCmd::ExtendNextLongWordStart),
    normal("action.extend-prev-long-word-start", NormalCmd::ExtendPrevLongWordStart),
    normal("action.extend-next-long-word-end", NormalCmd::ExtendNextLongWordEnd),
    normal("action.extend-search-next", NormalCmd::ExtendSearchNext),
    normal("action.extend-search-prev", NormalCmd::ExtendSearchPrev),
    normal("action.extend-till-char", NormalCmd::ExtendTillChar),
    normal("action.extend-next-char", NormalCmd::ExtendNextChar),
    normal("action.extend-till-prev-char", NormalCmd::ExtendTillPrevChar),
    normal("action.extend-prev-char", NormalCmd::ExtendPrevChar),
    normal("action.extend-to-line-start", NormalCmd::ExtendToLineStart),
    normal("action.extend-to-line-end", NormalCmd::ExtendToLineEnd),
    normal("action.extend-to-file-start", NormalCmd::ExtendToFileStart),
    normal("action.extend-to-file-end", NormalCmd::ExtendToFileEnd),
    normal("action.extend-to-last-line", NormalCmd::ExtendToLastLine),
    normal("action.extend-to-word", NormalCmd::ExtendToWord),
    // ---- Insert mode: each key a command by name ----
    normal("action.insert-newline", NormalCmd::InsertNewline),
    normal("action.delete-char-forward", NormalCmd::DeleteCharForward),
    normal("action.delete-word-forward", NormalCmd::DeleteWordForward),
    normal("action.delete-word-backward", NormalCmd::DeleteWordBackward),
    normal("action.kill-to-line-start", NormalCmd::KillToLineStart),
    normal("action.kill-to-line-end", NormalCmd::KillToLineEnd),
    normal("action.insert-register", NormalCmd::InsertRegister),
    normal("action.run-one-normal-command", NormalCmd::RunOneNormalCommand),
    normal("action.goto-line-end-newline", NormalCmd::GotoLineEndNewline),
    normal("action.commit-undo-checkpoint", NormalCmd::CommitUndoCheckpoint),
    normal("action.insert-tab", NormalCmd::InsertTab),
    normal("action.insert-other-tab", NormalCmd::InsertOtherTab),
    // ---- vim's own versions: the vim keymap's keys run these ----
    normal("action.vim-normal-mode", NormalCmd::VimNormalMode),
    normal("action.vim-goto-line-end", NormalCmd::VimGotoLineEnd),
    normal("action.vim-insert-mode", NormalCmd::VimInsertMode),
    normal("action.vim-append-mode", NormalCmd::VimAppendMode),
    normal("action.vim-insert-at-line-start", NormalCmd::VimInsertAtLineStart),
    normal("action.vim-insert-at-line-end", NormalCmd::VimInsertAtLineEnd),
    normal("action.vim-open-below", NormalCmd::VimOpenBelow),
    normal("action.vim-open-above", NormalCmd::VimOpenAbove),
    normal("action.vim-join-selections", NormalCmd::VimJoinSelections),
    normal("action.vim-switch-case", NormalCmd::VimSwitchCase),
    normal("action.vim-goto-line", NormalCmd::VimGotoLine),
    normal("action.vim-indent", NormalCmd::VimIndent),
    normal("action.vim-unindent", NormalCmd::VimUnindent),
    normal("action.vim-paste-after", NormalCmd::VimPasteAfter),
    normal("action.vim-paste-before", NormalCmd::VimPasteBefore),
    normal("action.vim-paste-after-indented", NormalCmd::VimPasteAfterIndented),
    normal("action.vim-paste-before-indented", NormalCmd::VimPasteBeforeIndented),
    normal("action.vim-increment", NormalCmd::VimIncrement),
    normal("action.vim-decrement", NormalCmd::VimDecrement),
    normal("action.vim-search-next", NormalCmd::VimSearchNext),
    normal("action.vim-search-prev", NormalCmd::VimSearchPrev),
    normal("action.vim-match-brackets", NormalCmd::VimMatchBrackets),
    normal("action.vim-kill-to-line-start", NormalCmd::VimKillToLineStart),
    normal("action.vim-kill-to-line-end", NormalCmd::VimKillToLineEnd),
    normal("action.vim-flip-selections", NormalCmd::VimFlipSelections),
    normal("action.vim-switch-to-lowercase", NormalCmd::VimSwitchToLowercase),
    normal("action.vim-switch-to-uppercase", NormalCmd::VimSwitchToUppercase),
    normal("action.vim-yank", NormalCmd::VimYank),
    normal("action.vim-visual-block-mode", NormalCmd::VimVisualBlockMode),
    normal("action.vim-record-macro", NormalCmd::VimRecordMacro),
    normal("action.vim-replay-macro", NormalCmd::VimReplayMacro),
    normal("action.vim-replay-last-macro", NormalCmd::VimReplayLastMacro),
    normal("action.vim-goto-first-nonwhitespace", NormalCmd::VimGotoFirstNonwhitespace),
];

/// **The places that take keys and are not modes of the text** (step 9 of
/// the keymap plan): a prompt (`:`, `/`, `::`, a reading, and a
/// picker's query), a picker's list (and the `::` palette's), a sidebar, and
/// a box of a panel's form. helix hard-codes these keys; here each place has
/// a table in the config (`[keys.prompt]`, `[keys.picker]`, `[keys.sidebar]`,
/// `[keys.field]`) and commands of its own.
///
/// A name the text's keymap has too (`move_char_left`, `page_down`) is
/// reused where it means the same thing in that place, so one name is one
/// thing to do wherever it is run. A picker and a field take the prompt's
/// keys as well ([`Place::chain`]), their own winning where both name a key
/// (helix's rule). The defaults of `[keys.normal]` and `[keys.insert]` never
/// come over (a sidebar's `J` pages and its `g` goes to the top at once), but
/// the reader's own one-key lines do while `[keys] inherit` is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Prompt,
    Picker,
    Sidebar,
    /// A box of a panel's form (the search panel's 找什麼, 換成什麼…), which
    /// takes the prompt's keys too, its own winning.
    Field,
}

impl Place {
    pub const ALL: [Place; 4] = [Place::Prompt, Place::Picker, Place::Sidebar, Place::Field];

    /// The table's name in the config: `[keys.prompt]`.
    pub fn name(self) -> &'static str {
        match self {
            Place::Prompt => "prompt",
            Place::Picker => "picker",
            Place::Sidebar => "sidebar",
            Place::Field => "field",
        }
    }

    /// The place a `[keys.…]` table is for.
    pub fn of(table: &str) -> Option<Place> {
        Place::ALL.into_iter().find(|place| place.name() == table)
    }

    /// The commands a key in this place may be bound to.
    pub fn actions(self) -> &'static [Action] {
        match self {
            Place::Prompt => PROMPT,
            Place::Picker => PICKER,
            Place::Sidebar => SIDEBAR,
            Place::Field => FIELD,
        }
    }

    /// The places whose keys this one reads, its own first: a picker and a
    /// field read the prompt's after their own.
    pub fn chain(self) -> &'static [Place] {
        match self {
            Place::Prompt => &[Place::Prompt],
            Place::Picker => &[Place::Picker, Place::Prompt],
            Place::Sidebar => &[Place::Sidebar],
            Place::Field => &[Place::Field, Place::Prompt],
        }
    }

    /// One of them by name. `no_op` is in every place, and a picker or a
    /// field runs a prompt's commands too, on its own text.
    pub fn action(self, name: &str) -> Option<&'static Action> {
        self.chain()
            .iter()
            .find_map(|place| place.actions().iter().find(|a| a.name == name))
            .or_else(|| action(name).filter(|a| a.name == "no_op"))
    }

    /// Read a right-hand side in this place: a command line and keys as
    /// anywhere ([`bound`]), a name only from this place's list.
    pub fn bound(self, rhs: &str) -> Option<Bound<'_>> {
        match rhs.starts_with([':', '@']) {
            true => bound(rhs),
            false => self.action(rhs).map(Bound::Named),
        }
    }
}

/// **Every command of the four places, named once**: the config and the
/// docs read the names, the editor matches on the variants, so a misspelt
/// name in a handler does not compile into an arm that never runs.
macro_rules! place_commands {
    ($($variant:ident = $name:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum PlaceCmd {
            $($variant,)*
        }

        impl PlaceCmd {
            pub const ALL: &'static [PlaceCmd] = &[$(PlaceCmd::$variant,)*];

            /// What the config writes.
            pub const fn name(self) -> &'static str {
                match self {
                    $(PlaceCmd::$variant => $name,)*
                }
            }

            /// The command a config line names, if a place has it.
            pub fn from_name(name: &str) -> Option<PlaceCmd> {
                PlaceCmd::ALL.iter().copied().find(|one| one.name() == name)
            }
        }
    };
}

place_commands! {
    NormalMode = "normal_mode",
    Confirm = "confirm",
    CompletionNext = "completion_next",
    CompletionPrev = "completion_prev",
    HistoryPrev = "history_prev",
    HistoryNext = "history_next",
    MoveCharLeft = "move_char_left",
    MoveCharRight = "move_char_right",
    GotoLineStart = "goto_line_start",
    GotoLineEnd = "goto_line_end",
    DeleteCharBackward = "delete_char_backward",
    DeleteCharForward = "delete_char_forward",
    DeleteWordBackward = "delete_word_backward",
    KillToLineStart = "kill_to_line_start",
    ToggleWideSearch = "toggle_wide_search",
    NextEntry = "next_entry",
    PrevEntry = "prev_entry",
    PageDown = "page_down",
    PageUp = "page_up",
    FirstEntry = "first_entry",
    LastEntry = "last_entry",
    CycleHiddenFiles = "cycle_hidden_files",
    MoveLineDown = "move_line_down",
    MoveLineUp = "move_line_up",
    GotoFileStart = "goto_file_start",
    GotoLastLine = "goto_last_line",
    CollapseEntry = "collapse_entry",
    ExpandEntry = "expand_entry",
    OpenEntry = "open_entry",
    ToggleLinksGraph = "toggle_links_graph",
    RestoreChange = "restore_change",
    YankOldText = "yank_old_text",
    RefreshSidebar = "refresh_sidebar",
    CycleSidebarWidth = "cycle_sidebar_width",
    NextSidebarView = "next_sidebar_view",
    PrevSidebarView = "prev_sidebar_view",
    LeaveSidebar = "leave_sidebar",
    CommandMode = "command_mode",
    SpaceMode = "space_mode",
    WindowMode = "window_mode",
    NextField = "next_field",
    PrevField = "prev_field",
}

/// A place's command and what it does there (help first, so the messages
/// test finds the tag right after `place(`).
const fn place(help: &'static str, cmd: PlaceCmd) -> Action {
    Action { name: cmd.name(), help, how: How::Normal }
}

/// A prompt's commands: `:`, `/`, `::`, a reading, and a picker's query.
pub const PROMPT: &[Action] = &[
    place("action.close-prompt", PlaceCmd::NormalMode),
    place("action.confirm", PlaceCmd::Confirm),
    place("action.completion-next", PlaceCmd::CompletionNext),
    place("action.completion-prev", PlaceCmd::CompletionPrev),
    place("action.history-prev", PlaceCmd::HistoryPrev),
    place("action.history-next", PlaceCmd::HistoryNext),
    place("action.move-char-left", PlaceCmd::MoveCharLeft),
    place("action.move-char-right", PlaceCmd::MoveCharRight),
    place("action.goto-line-start", PlaceCmd::GotoLineStart),
    place("action.goto-line-end", PlaceCmd::GotoLineEnd),
    place("action.delete-char-backward", PlaceCmd::DeleteCharBackward),
    place("action.delete-char-forward", PlaceCmd::DeleteCharForward),
    place("action.delete-word-backward", PlaceCmd::DeleteWordBackward),
    place("action.kill-to-line-start", PlaceCmd::KillToLineStart),
    place("action.toggle-wide-search", PlaceCmd::ToggleWideSearch),
];

/// A picker's list, and the `::` palette's.
pub const PICKER: &[Action] = &[
    place("action.next-entry", PlaceCmd::NextEntry),
    place("action.prev-entry", PlaceCmd::PrevEntry),
    place("action.page-down", PlaceCmd::PageDown),
    place("action.page-up", PlaceCmd::PageUp),
    place("action.first-entry", PlaceCmd::FirstEntry),
    place("action.last-entry", PlaceCmd::LastEntry),
    place("action.cycle-hidden-files", PlaceCmd::CycleHiddenFiles),
];

/// A panel's box: the next and the previous box of its form.
pub const FIELD: &[Action] = &[
    place("action.next-field", PlaceCmd::NextField),
    place("action.prev-field", PlaceCmd::PrevField),
    place("action.field-below", PlaceCmd::MoveLineDown),
    place("action.field-above", PlaceCmd::MoveLineUp),
];

/// A sidebar's: its lists, its article, and what every panel has.
pub const SIDEBAR: &[Action] = &[
    place("action.next-row", PlaceCmd::MoveLineDown),
    place("action.prev-row", PlaceCmd::MoveLineUp),
    place("action.page-down", PlaceCmd::PageDown),
    place("action.page-up", PlaceCmd::PageUp),
    place("action.first-row", PlaceCmd::GotoFileStart),
    place("action.last-row", PlaceCmd::GotoLastLine),
    place("action.collapse-entry", PlaceCmd::CollapseEntry),
    place("action.expand-entry", PlaceCmd::ExpandEntry),
    place("action.open-entry", PlaceCmd::OpenEntry),
    place("action.toggle-links-graph", PlaceCmd::ToggleLinksGraph),
    place("action.restore-change", PlaceCmd::RestoreChange),
    place("action.yank-old-text", PlaceCmd::YankOldText),
    place("action.refresh-sidebar", PlaceCmd::RefreshSidebar),
    place("action.cycle-sidebar-width", PlaceCmd::CycleSidebarWidth),
    place("action.next-sidebar-view", PlaceCmd::NextSidebarView),
    place("action.prev-sidebar-view", PlaceCmd::PrevSidebarView),
    place("action.sidebar-normal-mode", PlaceCmd::NormalMode),
    place("action.leave-sidebar", PlaceCmd::LeaveSidebar),
    place("action.command-mode", PlaceCmd::CommandMode),
    place("action.space-mode", PlaceCmd::SpaceMode),
    place("action.window-mode", PlaceCmd::WindowMode),
];

#[cfg(test)]
mod no_two_alike {
    use super::*;

    /// **No two actions share a name, or a command line**: a second name for
    /// one thing is a slip, never a feature (`next_region` pressing
    /// `outline`'s keys would make binding it open the outline).
    #[test]
    fn every_action_is_its_own() {
        let mut names = std::collections::HashSet::new();
        let mut lines = std::collections::HashSet::new();
        for action in ALL {
            assert!(names.insert(action.name), "`{}` twice", action.name);
            if let How::Command(line) = action.how {
                assert!(lines.insert(line), "`{}` runs `{line}`, as another action does", action.name);
            }
        }
    }

    /// The same in each of the three places, and a place reads only its
    /// own names.
    #[test]
    fn every_place_action_is_its_own() {
        for place in Place::ALL {
            let mut names = std::collections::HashSet::new();
            for one in place.actions() {
                assert!(names.insert(one.name), "`{}` twice in {}", one.name, place.name());
            }
        }
        assert_eq!(Place::Picker.bound("save_file"), None, "a Normal command is not a picker's");
        // Every Normal command the enum declares is in `ALL` once, so the
        // config, the palette and the docs know it.
        for cmd in NormalCmd::ALL {
            let n = ALL.iter().filter(|a| a.name == cmd.name()).count();
            assert_eq!(n, 1, "{cmd} is in ALL {n} times");
        }
        // Every command the enum declares is some place's, so none is dead.
        for cmd in PlaceCmd::ALL {
            let used = Place::ALL.iter().any(|p| p.actions().iter().any(|a| a.name == cmd.name()));
            assert!(used, "{cmd:?} is in no place");
        }
        assert!(Place::Sidebar.bound("no_op").is_some());
        assert!(Place::Prompt.bound(":write").is_some());
    }
}

#[cfg(test)]
mod control_keys_of_their_own {
    use super::*;

    /// `C-j` is not Enter and `C-i` is not Tab in a binding table, and each
    /// spells back as written.
    #[test]
    fn c_j_and_c_i_are_their_own_keys() {
        assert_ne!(written("C-j"), written("<enter>"));
        assert_ne!(written("C-i"), written("<tab>"));
        assert_eq!(spell(&written("C-j")), "C-j");
        assert_eq!(spell(&written("C-o")), "C-o");
    }
}
