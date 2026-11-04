//! **Normal mode's keys, each a named command** (2026-10-31, step 1 of
//! the keymap plan: every key a command, as in helix).
//!
//! Each top-level Normal key's code lives here as a method, `cmd_` and the
//! command's name: helix's name where helix has the same command
//! (`move_char_left`, `extend_line_below`, `select_regex`), and the same
//! style for yumete's own. [`NORMAL_COMMANDS`] lists them with the key each
//! answers to by default. `on_normal_key`'s `match` only says which key calls
//! which; the vim preset's keys that do something else are commands of their
//! own (`move_line_down` beside helix's `move_visual_line_down`), so neither
//! command asks which preset is in use.
//!
//! What the key said before it got here — a count, a register, the pending
//! groups, the table and vertical-layout layers — is still decided in
//! `on_normal_key`, ahead of the `match`; [`Cx`] carries what a command reads.

use super::*;
use yumete_cjk::actions::NormalCmd;
use yumete_cjk::FlavorItem;
use super::groups::Only;

/// What a Normal command is handed: the key it was reached by (several
/// commands share one body and tell their keys apart), the count, the count
/// as typed (`None` when none was), and whether a vertical-page motion is
/// continuing.
#[derive(Debug, Clone, Copy)]
pub(super) struct Cx {
    pub key: Key,
    pub count: usize,
    pub operator_count: Option<usize>,
    pub continuing_zong: bool,
}

impl Cx {
    /// **The same moment, with the key a command's own row names** (2026-11-01,
    /// the review's decoupling): a body shared by several commands (`f`
    /// `F` `t` `T` are one) tells them apart by that key, and it must be
    /// the row's, never whichever key reached the command — a binding, a
    /// group, a key a later keymap adds. Only `move_char_*` read the key
    /// pressed, for vim's `whichwrap`, which is about that key.
    pub(super) fn with_key(self, key: Key) -> Cx {
        Cx { key, ..self }
    }
}

/// A Normal command: its name, its method, and the key it answers to by
/// default.
pub(super) type NormalCommand = (NormalCmd, fn(&mut Editor, Cx), Key);

/// **Every Normal command, by name**, with its method and the key it answers
/// to by default: the vim preset's own where it has one, helix's otherwise.
///
pub(super) const NORMAL_COMMANDS: &[NormalCommand] = &[
    (NormalCmd::MoveCharLeft, Editor::cmd_move_char_left, Key::Char('h')),
    (NormalCmd::MoveCharRight, Editor::cmd_move_char_right, Key::Char('l')),
    (NormalCmd::MoveVisualLineUp, Editor::cmd_move_visual_line_up, Key::Char('k')),
    (NormalCmd::MoveVisualLineDown, Editor::cmd_move_visual_line_down, Key::Char('j')),
    (NormalCmd::MoveLineUp, Editor::cmd_move_line_up, Key::Char('k')),
    (NormalCmd::MoveLineDown, Editor::cmd_move_line_down, Key::Char('j')),
    (NormalCmd::GotoLineStart, Editor::cmd_goto_line_start, Key::Home),
    (NormalCmd::GotoLineEnd, |e, cx| e.acting(false, |e| e.cmd_goto_line_end(cx)), Key::End),
    (NormalCmd::VimGotoLineEnd, |e, cx| e.acting(true, |e| e.cmd_goto_line_end(cx)), Key::End),
    (NormalCmd::GotoNextLineNonwhitespace, Editor::cmd_goto_next_line_nonwhitespace, Key::Enter),
    (NormalCmd::JumpNextWordStart, |e, cx| e.cmd_jump_by_word(cx.with_key(Key::Char('w'))), Key::Char('w')),
    (NormalCmd::JumpNextLongWordStart, |e, cx| e.cmd_jump_by_word(cx.with_key(Key::Char('W'))), Key::Char('W')),
    (NormalCmd::JumpPrevWordStart, |e, cx| e.cmd_jump_by_word(cx.with_key(Key::Char('b'))), Key::Char('b')),
    (NormalCmd::JumpPrevLongWordStart, |e, cx| e.cmd_jump_by_word(cx.with_key(Key::Char('B'))), Key::Char('B')),
    (NormalCmd::JumpNextWordEnd, |e, cx| e.cmd_jump_by_word(cx.with_key(Key::Char('e'))), Key::Char('e')),
    (NormalCmd::JumpNextLongWordEnd, |e, cx| e.cmd_jump_by_word(cx.with_key(Key::Char('E'))), Key::Char('E')),
    // vim's `{` `}` `(` `)` are jumps (`:h jump-motions`), noted without
    // moving the page, as vim's.
    (NormalCmd::JumpNextParagraph, |e, cx| {
        e.note_where_we_came_from();
        e.cmd_jump_by_word(cx.with_key(Key::Char('}')))
    }, Key::Char('}')),
    (NormalCmd::JumpPrevParagraph, |e, cx| {
        e.note_where_we_came_from();
        e.cmd_jump_by_word(cx.with_key(Key::Char('{')))
    }, Key::Char('{')),
    (NormalCmd::MoveNextWordStart, Editor::cmd_move_next_word_start, Key::Char('w')),
    (NormalCmd::MoveNextWordEnd, Editor::cmd_move_next_word_end, Key::Char('e')),
    (NormalCmd::MovePrevWordStart, Editor::cmd_move_prev_word_start, Key::Char('b')),
    (NormalCmd::MoveNextParagraph, Editor::cmd_move_next_paragraph, Key::Char('}')),
    (NormalCmd::MovePrevParagraph, Editor::cmd_move_prev_paragraph, Key::Char('{')),
    (NormalCmd::GotoWindowTop, |e, cx| e.go_to_screen('H', cx.count), Key::Char('H')),
    (NormalCmd::GotoWindowCenter, |e, cx| e.go_to_screen('M', cx.count), Key::Char('M')),
    (NormalCmd::GotoWindowBottom, |e, cx| e.go_to_screen('L', cx.count), Key::Char('L')),
    (NormalCmd::JumpPrevSentence, |e, cx| {
        e.note_where_we_came_from();
        e.cmd_jump_by_sentence(cx.with_key(Key::Char('(')))
    }, Key::Char('(')),
    (NormalCmd::JumpNextSentence, |e, cx| {
        e.note_where_we_came_from();
        e.cmd_jump_by_sentence(cx.with_key(Key::Char(')')))
    }, Key::Char(')')),
    (NormalCmd::GotoNextSentence, Editor::cmd_goto_next_sentence, Key::Char('L')),
    (NormalCmd::GotoPrevSentence, Editor::cmd_goto_prev_sentence, Key::Char('H')),
    (NormalCmd::AddMark, Editor::cmd_add_mark, Key::Char('M')),
    (NormalCmd::GotoMark, Editor::cmd_goto_mark, Key::Char('\'')),
    (NormalCmd::ViewMode, Editor::cmd_view_mode, Key::Char('z')),
    (NormalCmd::StickyViewMode, Editor::cmd_sticky_view_mode, Key::Char('Z')),
    (NormalCmd::RightBracketMode, |e, cx| e.cmd_bracket_mode(cx.with_key(Key::Char(']'))), Key::Char(']')),
    (NormalCmd::LeftBracketMode, |e, cx| e.cmd_bracket_mode(cx.with_key(Key::Char('['))), Key::Char('[')),
    (NormalCmd::MoveNextLongWordStart, Editor::cmd_move_next_long_word_start, Key::Char('W')),
    (NormalCmd::MoveNextLongWordEnd, Editor::cmd_move_next_long_word_end, Key::Char('E')),
    (NormalCmd::MovePrevLongWordStart, Editor::cmd_move_prev_long_word_start, Key::Char('B')),
    (NormalCmd::GotoMode, Editor::cmd_goto_mode, Key::Char('g')),
    (NormalCmd::SpaceMode, Editor::cmd_space_mode, Key::Char(' ')),
    (NormalCmd::FindNextChar, |e, cx| e.cmd_find_char(cx.with_key(Key::Char('f'))), Key::Char('f')),
    (NormalCmd::FindPrevChar, |e, cx| e.cmd_find_char(cx.with_key(Key::Char('F'))), Key::Char('F')),
    (NormalCmd::FindTillChar, |e, cx| e.cmd_find_char(cx.with_key(Key::Char('t'))), Key::Char('t')),
    (NormalCmd::TillPrevChar, |e, cx| e.cmd_find_char(cx.with_key(Key::Char('T'))), Key::Char('T')),
    (NormalCmd::SelectLineMode, Editor::cmd_select_line_mode, Key::Char('V')),
    (NormalCmd::SelectMode, Editor::cmd_select_mode, Key::Char('v')),
    (NormalCmd::NormalMode, |e, cx| e.acting(false, |e| e.cmd_normal_mode(cx)), Key::Esc),
    (NormalCmd::VimNormalMode, |e, cx| e.acting(true, |e| e.cmd_normal_mode(cx)), Key::Esc),
    (NormalCmd::RepeatFindChar, |e, cx| e.cmd_repeat_find(cx.with_key(Key::Char(';'))), Key::Char(';')),
    (NormalCmd::RepeatFindCharReverse, |e, cx| e.cmd_repeat_find(cx.with_key(Key::Char(','))), Key::Char(',')),
    (NormalCmd::CollapseSelection, Editor::cmd_collapse_selection, Key::Char(';')),
    (NormalCmd::CopySelectionOnNextLine, Editor::cmd_copy_selection_on_next_line, Key::Char('C')),
    (NormalCmd::CopySelectionOnPrevLine, Editor::cmd_copy_selection_on_prev_line, Key::Alt('C')),
    (NormalCmd::KeepPrimarySelection, Editor::cmd_keep_primary_selection, Key::Char(',')),
    (NormalCmd::RemovePrimarySelection, Editor::cmd_remove_primary_selection, Key::Alt(',')),
    (NormalCmd::MergeSelections, Editor::cmd_merge_selections, Key::Alt('-')),
    (NormalCmd::MergeConsecutiveSelections, Editor::cmd_merge_consecutive_selections, Key::Alt('_')),
    (NormalCmd::EnsureSelectionsForward, Editor::cmd_ensure_selections_forward, Key::Alt(':')),
    (NormalCmd::ShrinkToLineBounds, Editor::cmd_shrink_to_line_bounds, Key::Alt('x')),
    (NormalCmd::SplitSelectionOnNewline, Editor::cmd_split_selection_on_newline, Key::Alt('s')),
    (NormalCmd::RotateSelectionsForward, Editor::cmd_rotate_selections_forward, Key::Char(')')),
    (NormalCmd::RotateSelectionsBackward, Editor::cmd_rotate_selections_backward, Key::Char('(')),
    (NormalCmd::TrimSelections, Editor::cmd_trim_selections, Key::Char('_')),
    (NormalCmd::AlignSelections, Editor::cmd_align_selections, Key::Char('&')),
    (NormalCmd::RotateSelectionContentsForward, Editor::cmd_rotate_selection_contents_forward, Key::Alt(')')),
    (NormalCmd::RotateSelectionContentsBackward, Editor::cmd_rotate_selection_contents_backward, Key::Alt('(')),
    (NormalCmd::SelectRegex, Editor::cmd_select_regex, Key::Char('s')),
    (NormalCmd::SplitSelection, Editor::cmd_split_selection, Key::Char('S')),
    (NormalCmd::KeepSelections, Editor::cmd_keep_selections, Key::Alt('k')),
    (NormalCmd::RemoveSelections, Editor::cmd_remove_selections, Key::Alt('K')),
    (NormalCmd::ExtendLineBelow, Editor::cmd_extend_line_below, Key::Char('x')),
    (NormalCmd::OperatorDelete, |e, cx| e.cmd_vim_operator(cx.with_key(Key::Char('d'))), Key::Char('d')),
    (NormalCmd::OperatorChange, |e, cx| e.cmd_vim_operator(cx.with_key(Key::Char('c'))), Key::Char('c')),
    (NormalCmd::OperatorYank, |e, cx| e.cmd_vim_operator(cx.with_key(Key::Char('y'))), Key::Char('y')),
    (NormalCmd::OperatorIndent, |e, cx| e.cmd_vim_operator(cx.with_key(Key::Char('>'))), Key::Char('>')),
    (NormalCmd::OperatorUnindent, |e, cx| e.cmd_vim_operator(cx.with_key(Key::Char('<'))), Key::Char('<')),
    (NormalCmd::DeleteSelection, |e, cx| e.cmd_delete_or_change(cx.with_key(Key::Char('d'))), Key::Char('d')),
    (NormalCmd::ChangeSelection, |e, cx| e.cmd_delete_or_change(cx.with_key(Key::Char('c'))), Key::Char('c')),
    (NormalCmd::DeleteSelectionNoyank, |e, cx| e.cmd_delete_or_change(cx.with_key(Key::Alt('d'))), Key::Alt('d')),
    (NormalCmd::ChangeSelectionNoyank, |e, cx| e.cmd_delete_or_change(cx.with_key(Key::Alt('c'))), Key::Alt('c')),
    // `D` and `A-D` (the flavour's `d_to_line_end`): the rest of the line,
    // into the register or not as `d` and `A-d` are.
    (NormalCmd::DeleteToLineEnd, |e, cx| e.delete_to_line_end_as(cx.with_key(Key::Char('d'))), Key::Char('D')),
    (NormalCmd::DeleteToLineEndNoyank, |e, cx| e.delete_to_line_end_as(cx.with_key(Key::Alt('d'))), Key::Alt('D')),
    // vim's `x`: what the cursor stands on, `count` characters of it, and
    // never the line break (`Rules::delete_stays_in_line`).
    (NormalCmd::VimDeleteCharForward, |e, cx| {
        e.sel.set_anchor(e.sel.head());
        e.cmd_delete_or_change(cx.with_key(Key::Char('d')));
    }, Key::Char('x')),
    (NormalCmd::ExtendLineAbove, |e, cx| e.repeat(cx.count, |e| e.select_line_above()), Key::Char('X')),
    (NormalCmd::MoveSelectedLinesDown, |e, cx| e.move_selected_lines(cx.count, true), Key::Alt('j')),
    (NormalCmd::MoveSelectedLinesUp, |e, cx| e.move_selected_lines(cx.count, false), Key::Alt('k')),
    // `G`: a count's line, as helix's; none, the last line's start, as `ge`.
    (NormalCmd::GotoLineOrLastLine, |e, cx| match cx.operator_count.is_some() {
        true => e.acting(false, |e| e.cmd_goto_line(cx)),
        false => e.run_normal_cmd(cx, NormalCmd::GotoLastLine),
    }, Key::Char('G')),
    // `Esc` (the flavour's `esc_collapses`): helix's, and with no select
    // mode to leave, down to the primary cursor as well.
    (NormalCmd::NormalModeAndCollapse, |e, cx| {
        let idle = e.mode == Mode::Normal && !e.extend;
        e.acting(false, |e| e.cmd_normal_mode(cx));
        if idle {
            e.cmd_keep_primary_selection(cx);
            e.sel.set_anchor(e.sel.head());
        }
    }, Key::Esc),
    (NormalCmd::Quit, |e, _| e.run_bound_command("quit"), Key::Char('q')),
    (NormalCmd::Yank, Editor::cmd_yank, Key::Char('y')),
    (NormalCmd::PasteAfter, |e, cx| e.acting(false, |e| e.cmd_paste(cx.with_key(Key::Char('p')))), Key::Char('p')),
    (NormalCmd::VimPasteAfter, |e, cx| e.acting(true, |e| e.cmd_paste(cx.with_key(Key::Char('p')))), Key::Char('p')),
    (NormalCmd::PasteBefore, |e, cx| e.acting(false, |e| e.cmd_paste(cx.with_key(Key::Char('P')))), Key::Char('P')),
    (NormalCmd::VimPasteBefore, |e, cx| e.acting(true, |e| e.cmd_paste(cx.with_key(Key::Char('P')))), Key::Char('P')),
    (NormalCmd::VimPasteAfterIndented, |e, cx| e.acting(true, |e| e.paste_indented(cx.with_key(Key::Char('p')))), Key::Char('p')),
    (NormalCmd::VimPasteBeforeIndented, |e, cx| e.acting(true, |e| e.paste_indented(cx.with_key(Key::Char('P')))), Key::Char('P')),
    (NormalCmd::SelectInnerObject, |e, cx| e.cmd_select_object(cx.with_key(Key::Char('i'))), Key::Char('i')),
    (NormalCmd::SelectAroundObject, |e, cx| e.cmd_select_object(cx.with_key(Key::Char('a'))), Key::Char('a')),
    (NormalCmd::InsertMode, |e, cx| e.acting(false, |e| e.cmd_insert_mode(cx)), Key::Char('i')),
    (NormalCmd::VimInsertMode, |e, cx| e.acting(true, |e| e.cmd_insert_mode(cx)), Key::Char('i')),
    (NormalCmd::AppendMode, |e, cx| e.acting(false, |e| e.cmd_append_mode(cx)), Key::Char('a')),
    (NormalCmd::VimAppendMode, |e, cx| e.acting(true, |e| e.cmd_append_mode(cx)), Key::Char('a')),
    (NormalCmd::InsertAtLineStart, |e, cx| e.acting(false, |e| e.cmd_insert_at_line_start(cx)), Key::Char('I')),
    (NormalCmd::VimInsertAtLineStart, |e, cx| e.acting(true, |e| e.cmd_insert_at_line_start(cx)), Key::Char('I')),
    (NormalCmd::InsertAtLineEnd, |e, cx| e.acting(false, |e| e.cmd_insert_at_line_end(cx)), Key::Char('A')),
    (NormalCmd::VimInsertAtLineEnd, |e, cx| e.acting(true, |e| e.cmd_insert_at_line_end(cx)), Key::Char('A')),
    (NormalCmd::OpenBelow, |e, cx| e.acting(false, |e| e.cmd_open_below(cx)), Key::Char('o')),
    (NormalCmd::VimOpenBelow, |e, cx| e.acting(true, |e| e.cmd_open_below(cx)), Key::Char('o')),
    (NormalCmd::OpenAbove, |e, cx| e.acting(false, |e| e.cmd_open_above(cx)), Key::Char('O')),
    (NormalCmd::VimOpenAbove, |e, cx| e.acting(true, |e| e.cmd_open_above(cx)), Key::Char('O')),
    (NormalCmd::Undo, Editor::cmd_undo, Key::Char('u')),
    (NormalCmd::UndoLine, Editor::cmd_undo_line, Key::Char('U')),
    (NormalCmd::Redo, Editor::cmd_redo, Key::Char('U')),
    (NormalCmd::Earlier, Editor::cmd_earlier, Key::Alt('u')),
    (NormalCmd::Later, Editor::cmd_later, Key::Alt('U')),
    (NormalCmd::GotoColumn, Editor::cmd_goto_column, Key::Char('|')),
    (NormalCmd::ShellPipe, Editor::cmd_shell_pipe, Key::Char('|')),
    (NormalCmd::ShellInsertOutput, Editor::cmd_shell_insert_output, Key::Char('!')),
    (NormalCmd::ShellAppendOutput, Editor::cmd_shell_append_output, Key::Alt('!')),
    (NormalCmd::ShellPipeTo, Editor::cmd_shell_pipe_to, Key::Alt('|')),
    (NormalCmd::ShellKeepPipe, |e, _| e.open_the_command_line("keep-pipe "), Key::Char('$')),
    (NormalCmd::Search, Editor::cmd_search, Key::Char('/')),
    (NormalCmd::Rsearch, Editor::cmd_rsearch, Key::Char('?')),
    (NormalCmd::SearchNext, |e, cx| e.acting(false, |e| e.cmd_search_next(cx)), Key::Char('n')),
    (NormalCmd::VimSearchNext, |e, cx| e.acting(true, |e| e.cmd_search_next(cx)), Key::Char('n')),
    (NormalCmd::SearchPrev, |e, cx| e.acting(false, |e| e.cmd_search_prev(cx)), Key::Char('N')),
    (NormalCmd::VimSearchPrev, |e, cx| e.acting(true, |e| e.cmd_search_prev(cx)), Key::Char('N')),
    (NormalCmd::CommandMode, Editor::cmd_command_mode, Key::Char(':')),
    (NormalCmd::Suspend, |e, _| e.suspend_request = true, Key::Ctrl('z')),
    (NormalCmd::FormatSelections, |e, _| e.ask_format(), Key::Char('=')),
    (NormalCmd::ExpandSelection, |e, _| e.tree_select(super::tree_select::TreeStep::Expand), Key::Alt('o')),
    (NormalCmd::ShrinkSelection, |e, _| e.tree_select(super::tree_select::TreeStep::Shrink), Key::Alt('i')),
    (NormalCmd::SelectNextSibling, |e, _| e.tree_select(super::tree_select::TreeStep::NextSibling), Key::Alt('n')),
    (NormalCmd::SelectPrevSibling, |e, _| e.tree_select(super::tree_select::TreeStep::PrevSibling), Key::Alt('p')),
    (NormalCmd::SelectAllSiblings, |e, _| e.tree_select(super::tree_select::TreeStep::AllSiblings), Key::Alt('a')),
    (NormalCmd::SelectAllChildren, |e, _| e.tree_select(super::tree_select::TreeStep::AllChildren), Key::Alt('I')),
    (NormalCmd::MoveParentNodeEnd, |e, _| e.tree_node_bound(true), Key::Alt('e')),
    (NormalCmd::MoveParentNodeStart, |e, _| e.tree_node_bound(false), Key::Alt('b')),
    (NormalCmd::MatchMode, Editor::cmd_match_mode, Key::Char('m')),
    (NormalCmd::Replace, Editor::cmd_replace, Key::Char('r')),
    (NormalCmd::SelectRegister, Editor::cmd_select_register, Key::Char('"')),
    (NormalCmd::RecordMacro, Editor::cmd_record_macro, Key::Char('Q')),
    (NormalCmd::ReplayMacro, Editor::cmd_replay_macro, Key::Char('q')),
    // vim's `q{a-z}` and `@{a-z}`: the letter names the register the macro
    // is recorded into or played from (2026-11-03). `q` while recording
    // stops, as in vim.
    (NormalCmd::VimRecordMacro, |e, cx| match e.recording.is_some() {
        true => e.stop_recording(),
        false => e.pending = Pending::MacroRegister { record: true, count: cx.count },
    }, Key::Char('q')),
    // vim's `Q`: the register last recorded into, played (Neovim's).
    (NormalCmd::VimReplayLastMacro, |e, cx| match e.last_recorded {
        Some(name) => e.play_register(name, cx.count),
        None => e.status = say!("macro.nothing-recorded"),
    }, Key::Char('Q')),
    (NormalCmd::VimReplayMacro, |e, cx| e.pending = Pending::MacroRegister { record: false, count: cx.count }, Key::Char('@')),
    // vim's `_`: the first non-blank character, count \u{2212} 1 lines down.
    (NormalCmd::VimGotoFirstNonwhitespace, |e, cx| {
        let rope = e.current_buffer().rope();
        let last = rope.len_lines().saturating_sub(1);
        let line = (rope.char_to_line(e.sel.head().min(rope.len_chars())) + cx.count.max(1) - 1).min(last);
        e.goto_line(line + 1);
        e.goto_motion(motion::Motion::LineFirstNonBlank);
    }, Key::Char('_')),
    (NormalCmd::WindowMode, Editor::cmd_window_mode, Key::Ctrl('w')),
    (NormalCmd::PageDown, Editor::cmd_page_down, Key::Ctrl('f')),
    (NormalCmd::PageUp, Editor::cmd_page_up, Key::Ctrl('b')),
    (NormalCmd::JoinSelectionsSpace, Editor::cmd_join_selections_space, Key::Alt('J')),
    (NormalCmd::SaveSelection, Editor::cmd_save_selection, Key::Ctrl('s')),
    (NormalCmd::JumpBackward, Editor::cmd_jump_backward, Key::Ctrl('o')),
    (NormalCmd::JumpForward, Editor::cmd_jump_forward, Key::Ctrl('i')),
    (NormalCmd::PageCursorHalfDown, Editor::cmd_half_page_down, Key::Ctrl('d')),
    (NormalCmd::PageCursorHalfUp, Editor::cmd_half_page_up, Key::Ctrl('u')),
    (NormalCmd::TwoThirdsPageDown, Editor::cmd_two_thirds_page_down, Key::Ctrl('n')),
    (NormalCmd::TwoThirdsPageUp, Editor::cmd_two_thirds_page_up, Key::Ctrl('p')),
    (NormalCmd::JoinSelections, |e, cx| e.acting(false, |e| e.cmd_join_selections(cx.with_key(Key::Char('J')))), Key::Char('J')),
    (NormalCmd::VimJoinSelections, |e, cx| e.acting(true, |e| e.cmd_join_selections(cx.with_key(Key::Char('J')))), Key::Char('J')),
    (NormalCmd::FlipSelections, Editor::cmd_flip_selections, Key::Alt(';')),
    (NormalCmd::SelectAll, Editor::cmd_select_all, Key::Char('%')),
    (NormalCmd::VimRedo, Editor::cmd_vim_redo, Key::Ctrl('r')),
    (NormalCmd::DeleteCharBackward, Editor::cmd_delete_char_backward, Key::Char('X')),
    (NormalCmd::ExtendToLineBounds, Editor::cmd_extend_to_line_bounds, Key::Char('X')),
    (NormalCmd::CaseMode, Editor::cmd_case_mode, Key::Char('`')),
    (NormalCmd::SwitchCase, |e, cx| e.acting(false, |e| e.cmd_switch_case(cx)), Key::Char('~')),
    (NormalCmd::VimSwitchCase, |e, cx| e.acting(true, |e| e.cmd_switch_case(cx)), Key::Char('~')),
    (NormalCmd::SearchSelectionDetectWordBoundaries, Editor::cmd_search_selection_detect_word_boundaries, Key::Char('*')),
    (NormalCmd::SearchSelectionExact, Editor::cmd_search_selection_exact, Key::Alt('*')),
    (NormalCmd::SearchWordBackward, Editor::cmd_search_word_backward, Key::Char('#')),
    (NormalCmd::ReplaceMode, Editor::cmd_replace_mode, Key::Char('R')),
    (NormalCmd::ReplaceWithYanked, Editor::cmd_replace_with_yanked, Key::Char('R')),
    (NormalCmd::GotoLine, |e, cx| e.acting(false, |e| e.cmd_goto_line(cx)), Key::Char('G')),
    (NormalCmd::VimGotoLine, |e, cx| e.acting(true, |e| e.cmd_goto_line(cx)), Key::Char('G')),
    (NormalCmd::Indent, |e, cx| e.acting(false, |e| e.cmd_indent_or_unindent(cx.with_key(Key::Char('>')))), Key::Char('>')),
    (NormalCmd::VimIndent, |e, cx| e.acting(true, |e| e.cmd_indent_or_unindent(cx.with_key(Key::Char('>')))), Key::Char('>')),
    (NormalCmd::Unindent, |e, cx| e.acting(false, |e| e.cmd_indent_or_unindent(cx.with_key(Key::Char('<')))), Key::Char('<')),
    (NormalCmd::VimUnindent, |e, cx| e.acting(true, |e| e.cmd_indent_or_unindent(cx.with_key(Key::Char('<')))), Key::Char('<')),
    (NormalCmd::Increment, |e, cx| e.acting(false, |e| e.cmd_increment(cx)), Key::Ctrl('a')),
    (NormalCmd::VimIncrement, |e, cx| e.acting(true, |e| e.cmd_increment(cx)), Key::Ctrl('a')),
    (NormalCmd::Decrement, |e, cx| e.acting(false, |e| e.cmd_decrement(cx)), Key::Ctrl('x')),
    (NormalCmd::VimDecrement, |e, cx| e.acting(true, |e| e.cmd_decrement(cx)), Key::Ctrl('x')),
    (NormalCmd::VimRepeatLastChange, |e, cx| e.acting(true, |e| e.cmd_vim_repeat_last_change(cx)), Key::Char('.')),
    // vim's Visual mode's own (`:h v_o`, `:h v_u`, `:h v_y`), and its
    // blockwise one (`C-v`).
    (NormalCmd::VimFlipSelections, |e, _| e.vim_visual_key('o'), Key::Char('o')),
    (NormalCmd::VimSwitchToLowercase, |e, _| e.vim_visual_key('u'), Key::Char('u')),
    (NormalCmd::VimSwitchToUppercase, |e, _| e.vim_visual_key('U'), Key::Char('U')),
    (NormalCmd::VimYank, |e, _| e.vim_visual_key('y'), Key::Char('y')),
    (NormalCmd::VimVisualBlockMode, |e, _| e.toggle_block(), Key::Ctrl('v')),
    (NormalCmd::RepeatLastChange, |e, cx| e.acting(false, |e| e.cmd_repeat_last_change(cx)), Key::Char('.')),
    (NormalCmd::RepeatLastMotion, Editor::cmd_repeat_last_motion, Key::Alt('.')),
    // helix's: bound to a key, it unbinds it, and the key leaves its menu.
    (NormalCmd::NoOp, |_, _| {}, Key::Esc),
];

/// A condition a vim entry needs besides the preset; when it does not hold,
/// helix's entry for the key answers, as the `match` it replaced fell through.
pub(super) type When = fn(&Editor, &Cx) -> bool;

/// **The default keymap, helix's** (2026-10-31): each key and the command it
/// runs. The vim preset reads [`VIM_NORMAL`] first.
pub(super) const HELIX_NORMAL: &[(Key, NormalCmd)] = &[
    (Key::Char('h'), NormalCmd::MoveCharLeft),
    (Key::Left, NormalCmd::MoveCharLeft),
    (Key::Char('l'), NormalCmd::MoveCharRight),
    (Key::Right, NormalCmd::MoveCharRight),
    (Key::Char('k'), NormalCmd::MoveVisualLineUp),
    (Key::Up, NormalCmd::MoveVisualLineUp),
    (Key::Char('j'), NormalCmd::MoveVisualLineDown),
    (Key::Down, NormalCmd::MoveVisualLineDown),
    (Key::Home, NormalCmd::GotoLineStart),
    (Key::End, NormalCmd::GotoLineEnd),
    (Key::Char('w'), NormalCmd::MoveNextWordStart),
    (Key::Char('e'), NormalCmd::MoveNextWordEnd),
    (Key::Char('b'), NormalCmd::MovePrevWordStart),
    (Key::Char('}'), NormalCmd::MoveNextParagraph),
    (Key::Char('{'), NormalCmd::MovePrevParagraph),
    (Key::Char('z'), NormalCmd::ViewMode),
    (Key::Char('Z'), NormalCmd::StickyViewMode),
    (Key::Char(']'), NormalCmd::RightBracketMode),
    (Key::Char('['), NormalCmd::LeftBracketMode),
    (Key::Char('W'), NormalCmd::MoveNextLongWordStart),
    (Key::Char('E'), NormalCmd::MoveNextLongWordEnd),
    (Key::Char('B'), NormalCmd::MovePrevLongWordStart),
    (Key::Char('g'), NormalCmd::GotoMode),
    (Key::Char(' '), NormalCmd::SpaceMode),
    (Key::Char('f'), NormalCmd::FindNextChar),
    (Key::Char('F'), NormalCmd::FindPrevChar),
    (Key::Char('t'), NormalCmd::FindTillChar),
    (Key::Char('T'), NormalCmd::TillPrevChar),
    (Key::Char('v'), NormalCmd::SelectMode),
    (Key::Esc, NormalCmd::NormalMode),
    (Key::Char(';'), NormalCmd::CollapseSelection),
    (Key::Char('C'), NormalCmd::CopySelectionOnNextLine),
    (Key::Alt('C'), NormalCmd::CopySelectionOnPrevLine),
    (Key::Char(','), NormalCmd::KeepPrimarySelection),
    (Key::Alt(','), NormalCmd::RemovePrimarySelection),
    (Key::Alt('-'), NormalCmd::MergeSelections),
    (Key::Alt('_'), NormalCmd::MergeConsecutiveSelections),
    (Key::Alt(':'), NormalCmd::EnsureSelectionsForward),
    (Key::Alt('x'), NormalCmd::ShrinkToLineBounds),
    (Key::Alt('s'), NormalCmd::SplitSelectionOnNewline),
    (Key::Char(')'), NormalCmd::RotateSelectionsForward),
    (Key::Char('('), NormalCmd::RotateSelectionsBackward),
    (Key::Char('_'), NormalCmd::TrimSelections),
    (Key::Char('&'), NormalCmd::AlignSelections),
    (Key::Alt(')'), NormalCmd::RotateSelectionContentsForward),
    (Key::Alt('('), NormalCmd::RotateSelectionContentsBackward),
    (Key::Char('s'), NormalCmd::SelectRegex),
    (Key::Char('S'), NormalCmd::SplitSelection),
    (Key::Alt('K'), NormalCmd::RemoveSelections),
    (Key::Char('x'), NormalCmd::ExtendLineBelow),
    (Key::Char('d'), NormalCmd::DeleteSelection),
    (Key::Char('c'), NormalCmd::ChangeSelection),
    (Key::Alt('d'), NormalCmd::DeleteSelectionNoyank),
    (Key::Alt('c'), NormalCmd::ChangeSelectionNoyank),
    (Key::Char('y'), NormalCmd::Yank),
    (Key::Char('p'), NormalCmd::PasteAfter),
    (Key::Char('P'), NormalCmd::PasteBefore),
    (Key::Char('i'), NormalCmd::InsertMode),
    (Key::Char('a'), NormalCmd::AppendMode),
    (Key::Char('I'), NormalCmd::InsertAtLineStart),
    (Key::Char('A'), NormalCmd::InsertAtLineEnd),
    (Key::Char('o'), NormalCmd::OpenBelow),
    (Key::Char('O'), NormalCmd::OpenAbove),
    (Key::Char('u'), NormalCmd::Undo),
    (Key::Char('U'), NormalCmd::Redo),
    (Key::Alt('u'), NormalCmd::Earlier),
    (Key::Alt('U'), NormalCmd::Later),
    (Key::Char('|'), NormalCmd::ShellPipe),
    (Key::Char('!'), NormalCmd::ShellInsertOutput),
    (Key::Alt('!'), NormalCmd::ShellAppendOutput),
    (Key::Alt('|'), NormalCmd::ShellPipeTo),
    (Key::Char('$'), NormalCmd::ShellKeepPipe),
    (Key::Char('/'), NormalCmd::Search),
    (Key::Char('?'), NormalCmd::Rsearch),
    (Key::Char('n'), NormalCmd::SearchNext),
    (Key::Char('N'), NormalCmd::SearchPrev),
    (Key::Char(':'), NormalCmd::CommandMode),
    (Key::Ctrl('z'), NormalCmd::Suspend),
    (Key::Char('='), NormalCmd::FormatSelections),
    (Key::Alt('o'), NormalCmd::ExpandSelection),
    (Key::Alt('i'), NormalCmd::ShrinkSelection),
    (Key::Alt('I'), NormalCmd::SelectAllChildren),
    (Key::Alt('p'), NormalCmd::SelectPrevSibling),
    (Key::Alt('n'), NormalCmd::SelectNextSibling),
    (Key::Alt('e'), NormalCmd::MoveParentNodeEnd),
    (Key::Alt('b'), NormalCmd::MoveParentNodeStart),
    (Key::Alt('a'), NormalCmd::SelectAllSiblings),
    (Key::Char('m'), NormalCmd::MatchMode),
    (Key::Char('r'), NormalCmd::Replace),
    (Key::Char('"'), NormalCmd::SelectRegister),
    (Key::Char('Q'), NormalCmd::RecordMacro),
    (Key::Char('q'), NormalCmd::ReplayMacro),
    (Key::Ctrl('w'), NormalCmd::WindowMode),
    (Key::Ctrl('f'), NormalCmd::PageDown),
    (Key::PageDown, NormalCmd::PageDown),
    (Key::Ctrl('b'), NormalCmd::PageUp),
    (Key::PageUp, NormalCmd::PageUp),
    (Key::Alt('J'), NormalCmd::JoinSelectionsSpace),
    (Key::Ctrl('s'), NormalCmd::SaveSelection),
    (Key::Ctrl('o'), NormalCmd::JumpBackward),
    (Key::Ctrl('i'), NormalCmd::JumpForward),
    (Key::Tab, NormalCmd::JumpForward),
    (Key::Ctrl('d'), NormalCmd::PageCursorHalfDown),
    (Key::Ctrl('u'), NormalCmd::PageCursorHalfUp),
    (Key::Char('J'), NormalCmd::JoinSelections),
    (Key::Char('K'), NormalCmd::KeepSelections),
    (Key::Alt(';'), NormalCmd::FlipSelections),
    (Key::Char('%'), NormalCmd::SelectAll),
    (Key::Char('X'), NormalCmd::ExtendToLineBounds),
    (Key::Char('`'), NormalCmd::SwitchToLowercase),
    (Key::Alt('`'), NormalCmd::SwitchToUppercase),
    (Key::Char('~'), NormalCmd::SwitchCase),
    (Key::Char('*'), NormalCmd::SearchSelectionDetectWordBoundaries),
    (Key::Alt('*'), NormalCmd::SearchSelectionExact),
    (Key::Char('R'), NormalCmd::ReplaceWithYanked),
    (Key::Char('G'), NormalCmd::GotoLine),
    (Key::Char('>'), NormalCmd::Indent),
    (Key::Char('<'), NormalCmd::Unindent),
    (Key::Ctrl('a'), NormalCmd::Increment),
    (Key::Ctrl('c'), NormalCmd::ToggleComments),
    (Key::Ctrl('x'), NormalCmd::Decrement),
    (Key::Char('.'), NormalCmd::RepeatLastChange),
    (Key::Alt('.'), NormalCmd::RepeatLastMotion),
];

/// **The flavour's keys** (2026-11-02): yumete's own keys at the top level,
/// each held while its item is on (`yumete_cjk::FlavorItem`) and where its
/// condition holds, before either keymap's. A key whose item belongs to one
/// keymap only is said by the item, not by the row.
pub(super) const FLAVOR_NORMAL: &[(Key, NormalCmd, FlavorItem, super::groups::Only)] = &[
    (Key::Char('`'), NormalCmd::CaseMode, FlavorItem::CaseMenu, Only::Helix),
    (Key::Char('L'), NormalCmd::GotoNextSentence, FlavorItem::SentencesOnHL, Only::Always),
    (Key::Char('H'), NormalCmd::GotoPrevSentence, FlavorItem::SentencesOnHL, Only::Always),
    (Key::Char('M'), NormalCmd::AddMark, FlavorItem::MarksOnMQuote, Only::Always),
    (Key::Char('\''), NormalCmd::GotoMark, FlavorItem::MarksOnMQuote, Only::Always),
    (Key::Ctrl('n'), NormalCmd::TwoThirdsPageDown, FlavorItem::TwoThirdsPage, Only::Always),
    (Key::Ctrl('p'), NormalCmd::TwoThirdsPageUp, FlavorItem::TwoThirdsPage, Only::Always),
    (Key::Char('X'), NormalCmd::ExtendLineAbove, FlavorItem::XSelectsUpward, Only::Always),
    (Key::Char('G'), NormalCmd::GotoLineOrLastLine, FlavorItem::BareGLastLine, Only::Always),
    (Key::Alt('j'), NormalCmd::MoveSelectedLinesDown, FlavorItem::MoveLines, Only::Always),
    (Key::Alt('k'), NormalCmd::MoveSelectedLinesUp, FlavorItem::MoveLines, Only::Always),
    (Key::Char('0'), NormalCmd::GotoLineStart, FlavorItem::LineStartKeys, Only::Always),
    (Key::Char('^'), NormalCmd::GotoFirstNonwhitespace, FlavorItem::LineStartKeys, Only::Always),
    (Key::Char('D'), NormalCmd::DeleteToLineEnd, FlavorItem::DToLineEnd, Only::Always),
    (Key::Alt('D'), NormalCmd::DeleteToLineEndNoyank, FlavorItem::DToLineEnd, Only::Always),
    (Key::Esc, NormalCmd::NormalModeAndCollapse, FlavorItem::EscCollapses, Only::Always),
    (Key::Char(' '), NormalCmd::SpaceMode, FlavorItem::VimSpaceMenu, Only::Vim),
];

/// **helix's keys that vim leaves free** (2026-11-02): under vim they reach
/// helix's table only while `helix_keys_under_vim` is on. A key a binding or
/// the preset plays is helix's either way: the preset is spelt in them.
const HELIX_KEYS_UNDER_VIM: &[Key] = &[
    Key::Char('C'),
    Key::Alt('C'),
    Key::Alt(','),
    Key::Alt('-'),
    Key::Alt('_'),
    Key::Alt(':'),
    Key::Alt('x'),
    Key::Alt('s'),
    Key::Alt('('),
    Key::Alt(')'),
    Key::Alt('K'),
    Key::Alt('d'),
    Key::Alt('c'),
    Key::Alt('u'),
    Key::Alt('U'),
    Key::Alt('!'),
    Key::Alt('|'),
    Key::Alt('J'),
    Key::Alt(';'),
    Key::Alt('*'),
    Key::Alt('.'),
    Key::Alt('`'),
    Key::Ctrl('s'),
    Key::Char('Z'),
    Key::Alt('o'),
    Key::Alt('i'),
    Key::Alt('I'),
    Key::Alt('p'),
    Key::Alt('n'),
    Key::Alt('e'),
    Key::Alt('b'),
    Key::Alt('a'),
    // vim's `;` and `,` with no `f` to repeat do nothing; helix's collapse
    // and keep the primary selection.
    Key::Char(';'),
    Key::Char(','),
];

/// **The vim preset's own keys**, before helix's: the commands that do
/// something else under vim, each with the condition it needs, if any.
pub(super) const VIM_NORMAL: &[(Key, NormalCmd, Option<When>)] = &[
    // **vim's own meanings where helix's table used to answer** (2026-11-01):
    // a key this table did not name fell through to helix's, so vim's `C-n`
    // turned two-thirds of a page and `_` trimmed the selection. Keys whose
    // vim meaning yumete has no feature for do nothing (`&`, `!`).
    (Key::Ctrl('n'), NormalCmd::MoveLineDown, None),
    (Key::Ctrl('p'), NormalCmd::MoveLineUp, None),
    (Key::Char('_'), NormalCmd::VimGotoFirstNonwhitespace, None),
    (Key::Char('&'), NormalCmd::NoOp, None),
    // vim's `=` re-indents over a motion, which yumete does not have; helix's
    // formats through the server (2026-11-02).
    (Key::Char('='), NormalCmd::NoOp, None),
    (Key::Char('!'), NormalCmd::NoOp, None),
    (Key::Char('q'), NormalCmd::VimRecordMacro, None),
    (Key::Char('@'), NormalCmd::VimReplayMacro, None),
    // `C-^` is the previous file in Normal mode; while typing it switches
    // 中/英, vim's own Insert meaning (`i_CTRL-^`), and the front end takes it
    // there. A terminal sends it as `C-^` or as `C-6`.
    (Key::Ctrl('^'), NormalCmd::GotoLastAccessedFile, None),
    (Key::Ctrl('6'), NormalCmd::GotoLastAccessedFile, None),
    // vim's `Space` moves right, and `\` (vim's leader, which vim leaves
    // free) opens the Space menu: the flavour's `vim_space_menu` puts the
    // menu back on `Space` (2026-11-01).
    (Key::Char(' '), NormalCmd::MoveCharRight, None),
    // vim's `C-e` `C-y` scroll a line; its `C-c` in Normal mode does
    // nothing, where helix's comments (2026-11-02).
    (Key::Ctrl('e'), NormalCmd::ScrollDown, None),
    (Key::Ctrl('y'), NormalCmd::ScrollUp, None),
    (Key::Ctrl('c'), NormalCmd::NoOp, None),
    (Key::Char('\\'), NormalCmd::SpaceMode, None),
    (Key::Backspace, NormalCmd::MoveCharLeft, None),
    (Key::Char('k'), NormalCmd::MoveLineUp, None),
    (Key::Up, NormalCmd::MoveLineUp, None),
    (Key::Char('j'), NormalCmd::MoveLineDown, None),
    (Key::Down, NormalCmd::MoveLineDown, None),
    (Key::Enter, NormalCmd::GotoNextLineNonwhitespace, None),
    (Key::Char('w'), NormalCmd::JumpNextWordStart, Some(when::vim_word_motion)),
    (Key::Char('W'), NormalCmd::JumpNextLongWordStart, Some(when::vim_word_motion)),
    (Key::Char('b'), NormalCmd::JumpPrevWordStart, Some(when::vim_word_motion)),
    (Key::Char('B'), NormalCmd::JumpPrevLongWordStart, Some(when::vim_word_motion)),
    (Key::Char('e'), NormalCmd::JumpNextWordEnd, Some(when::vim_word_motion)),
    (Key::Char('E'), NormalCmd::JumpNextLongWordEnd, Some(when::vim_word_motion)),
    (Key::Char('{'), NormalCmd::JumpPrevParagraph, Some(when::vim_word_motion)),
    (Key::Char('}'), NormalCmd::JumpNextParagraph, Some(when::vim_word_motion)),
    (Key::Char('H'), NormalCmd::GotoWindowTop, None),
    (Key::Char('M'), NormalCmd::GotoWindowCenter, None),
    (Key::Char('L'), NormalCmd::GotoWindowBottom, None),
    (Key::Char('('), NormalCmd::JumpPrevSentence, None),
    (Key::Char(')'), NormalCmd::JumpNextSentence, None),
    (Key::Char('m'), NormalCmd::AddMark, Some(when::typed)),
    (Key::Char('V'), NormalCmd::SelectLineMode, None),
    (Key::Char(';'), NormalCmd::RepeatFindChar, Some(when::a_find_to_repeat)),
    (Key::Char(','), NormalCmd::RepeatFindCharReverse, Some(when::a_find_to_repeat)),
    (Key::Char('d'), NormalCmd::OperatorDelete, Some(when::not_extending)),
    (Key::Char('c'), NormalCmd::OperatorChange, Some(when::not_extending)),
    (Key::Char('y'), NormalCmd::OperatorYank, Some(when::not_extending)),
    (Key::Char('>'), NormalCmd::OperatorIndent, Some(when::not_extending)),
    (Key::Char('<'), NormalCmd::OperatorUnindent, Some(when::not_extending)),
    (Key::Char('>'), NormalCmd::VimIndent, None),
    (Key::Char('<'), NormalCmd::VimUnindent, None),
    (Key::Char('i'), NormalCmd::SelectInnerObject, Some(when::extending)),
    (Key::Char('a'), NormalCmd::SelectAroundObject, Some(when::extending)),
    // **vim's own versions of helix's commands** (2026-11-01, step 8 of the
    // keymap plan): the same keys, the `vim_` twin of each command, so
    // `insert_mode` means helix's `i` under either keymap.
    // vim's Visual mode: these act on the whole selection and leave it.
    (Key::Char('o'), NormalCmd::VimFlipSelections, Some(when::typed_extending)),
    (Key::Char('u'), NormalCmd::VimSwitchToLowercase, Some(when::typed_extending)),
    (Key::Char('U'), NormalCmd::VimSwitchToUppercase, Some(when::typed_extending)),
    (Key::Char('y'), NormalCmd::VimYank, Some(when::typed_extending)),
    (Key::Char('x'), NormalCmd::DeleteSelection, Some(when::typed_extending)),
    (Key::Char('s'), NormalCmd::ChangeSelection, Some(when::typed_extending)),
    (Key::Ctrl('v'), NormalCmd::VimVisualBlockMode, Some(when::typed)),
    (Key::Char('i'), NormalCmd::VimInsertMode, None),
    (Key::Char('a'), NormalCmd::VimAppendMode, None),
    (Key::Char('I'), NormalCmd::VimInsertAtLineStart, None),
    (Key::Char('A'), NormalCmd::VimInsertAtLineEnd, None),
    (Key::Char('o'), NormalCmd::VimOpenBelow, None),
    (Key::Char('O'), NormalCmd::VimOpenAbove, None),
    (Key::Esc, NormalCmd::VimNormalMode, None),
    (Key::End, NormalCmd::VimGotoLineEnd, None),
    (Key::Char('J'), NormalCmd::VimJoinSelections, None),
    (Key::Char('~'), NormalCmd::VimSwitchCase, None),
    (Key::Char('G'), NormalCmd::VimGotoLine, None),
    (Key::Char('p'), NormalCmd::VimPasteAfter, None),
    (Key::Char('P'), NormalCmd::VimPasteBefore, None),
    (Key::Ctrl('a'), NormalCmd::VimIncrement, None),
    (Key::Ctrl('x'), NormalCmd::VimDecrement, None),
    (Key::Char('n'), NormalCmd::VimSearchNext, None),
    (Key::Char('N'), NormalCmd::VimSearchPrev, None),
    (Key::Char('U'), NormalCmd::UndoLine, None),
    (Key::Char('|'), NormalCmd::GotoColumn, None),
    (Key::Ctrl('r'), NormalCmd::VimRedo, None),
    (Key::Char('X'), NormalCmd::DeleteCharBackward, None),
    (Key::Char('`'), NormalCmd::GotoMark, None),
    // helix has no `'`; vim's is the mark's line (helix's table held it as
    // a yumete key until the flavour took that one, 2026-11-02).
    (Key::Char('\''), NormalCmd::GotoMark, None),
    (Key::Char('#'), NormalCmd::SearchWordBackward, None),
    (Key::Char('R'), NormalCmd::ReplaceMode, Some(when::not_extending)),
    (Key::Char('.'), NormalCmd::VimRepeatLastChange, None),
];

/// The conditions [`VIM_NORMAL`] names.
mod when {
    use super::*;

    /// Pressed by a person, not replayed from a run of keys: the vim
    /// preset's own table plays helix's keys, and those mean helix's thing.
    pub(super) fn typed(e: &Editor, _: &Cx) -> bool {
        !e.expanding_alias
    }

    /// A vim word or paragraph motion that has a caret reading.
    pub(super) fn vim_word_motion(e: &Editor, cx: &Cx) -> bool {
        let Key::Char(c) = cx.key else { return false };
        !e.expanding_alias && crate::vim::step_for(&c.to_string(), e.word_grain(), None).is_some()
    }

    /// There is an `f`/`t` to repeat, and a person pressed the key.
    pub(super) fn a_find_to_repeat(e: &Editor, _: &Cx) -> bool {
        e.last_find.is_some() && !e.expanding_alias
    }

    pub(super) fn not_extending(e: &Editor, _: &Cx) -> bool {
        !e.extend
    }

    pub(super) fn extending(e: &Editor, _: &Cx) -> bool {
        e.extend
    }

    /// Extending, by a key typed rather than played by a binding.
    pub(super) fn typed_extending(e: &Editor, _: &Cx) -> bool {
        e.extend && !e.expanding_alias
    }

}

/// **Select mode: the keys that extend instead of move** (2026-10-31,
/// step 4 of the keymap plan; helix's `select` keymap is its `normal` one
/// with these keys changed, `keymap/default.rs`). Looked up while a
/// selection is being extended, after the vim preset's own keys and before
/// helix's; the `g` group's select keys are its own rows (`groups.rs`).
///
/// A `move_*` command follows the mode (it extends in select mode, as it
/// always has here, and as helix's `goto_line_start` does); an `extend_*`
/// command extends in either mode, so a binding can say which it means.
pub(super) const SELECT_NORMAL: &[(Key, NormalCmd)] = &[
    (Key::Char('h'), NormalCmd::ExtendCharLeft),
    (Key::Left, NormalCmd::ExtendCharLeft),
    (Key::Char('j'), NormalCmd::ExtendVisualLineDown),
    (Key::Down, NormalCmd::ExtendVisualLineDown),
    (Key::Char('k'), NormalCmd::ExtendVisualLineUp),
    (Key::Up, NormalCmd::ExtendVisualLineUp),
    (Key::Char('l'), NormalCmd::ExtendCharRight),
    (Key::Right, NormalCmd::ExtendCharRight),
    (Key::Char('w'), NormalCmd::ExtendNextWordStart),
    (Key::Char('b'), NormalCmd::ExtendPrevWordStart),
    (Key::Char('e'), NormalCmd::ExtendNextWordEnd),
    (Key::Char('W'), NormalCmd::ExtendNextLongWordStart),
    (Key::Char('B'), NormalCmd::ExtendPrevLongWordStart),
    (Key::Char('E'), NormalCmd::ExtendNextLongWordEnd),
    (Key::Char('n'), NormalCmd::ExtendSearchNext),
    (Key::Char('N'), NormalCmd::ExtendSearchPrev),
    (Key::Char('t'), NormalCmd::ExtendTillChar),
    (Key::Char('f'), NormalCmd::ExtendNextChar),
    (Key::Char('T'), NormalCmd::ExtendTillPrevChar),
    (Key::Char('F'), NormalCmd::ExtendPrevChar),
    (Key::Home, NormalCmd::ExtendToLineStart),
    (Key::End, NormalCmd::ExtendToLineEnd),
    (Key::Alt('e'), NormalCmd::ExtendParentNodeEnd),
    (Key::Alt('b'), NormalCmd::ExtendParentNodeStart),
];

/// **The `extend_*` commands**: each is the `move_*` (or `goto_*`) command
/// beside it, run extending the selection. The key is the one its twin
/// answers to.
pub(super) const EXTEND_COMMANDS: &[NormalCommand] = &[
    (NormalCmd::ExtendCharLeft, |e, cx| e.extending(cx, NormalCmd::MoveCharLeft), Key::Char('h')),
    (NormalCmd::ExtendCharRight, |e, cx| e.extending(cx, NormalCmd::MoveCharRight), Key::Char('l')),
    (NormalCmd::ExtendVisualLineUp, |e, cx| e.extending(cx, NormalCmd::MoveVisualLineUp), Key::Char('k')),
    (NormalCmd::ExtendVisualLineDown, |e, cx| e.extending(cx, NormalCmd::MoveVisualLineDown), Key::Char('j')),
    (NormalCmd::ExtendLineUp, |e, cx| e.extending(cx, NormalCmd::MoveLineUp), Key::Char('k')),
    (NormalCmd::ExtendLineDown, |e, cx| e.extending(cx, NormalCmd::MoveLineDown), Key::Char('j')),
    (NormalCmd::ExtendNextWordStart, |e, cx| e.extending(cx, NormalCmd::MoveNextWordStart), Key::Char('w')),
    (NormalCmd::ExtendPrevWordStart, |e, cx| e.extending(cx, NormalCmd::MovePrevWordStart), Key::Char('b')),
    (NormalCmd::ExtendNextWordEnd, |e, cx| e.extending(cx, NormalCmd::MoveNextWordEnd), Key::Char('e')),
    (NormalCmd::ExtendNextLongWordStart, |e, cx| e.extending(cx, NormalCmd::MoveNextLongWordStart), Key::Char('W')),
    (NormalCmd::ExtendPrevLongWordStart, |e, cx| e.extending(cx, NormalCmd::MovePrevLongWordStart), Key::Char('B')),
    (NormalCmd::ExtendNextLongWordEnd, |e, cx| e.extending(cx, NormalCmd::MoveNextLongWordEnd), Key::Char('E')),
    // helix's: with a selection being extended, the next match is added as
    // a selection of its own.
    (NormalCmd::ExtendSearchNext, |e, cx| e.extending(cx, NormalCmd::SearchNext), Key::Char('n')),
    (NormalCmd::ExtendSearchPrev, |e, cx| e.extending(cx, NormalCmd::SearchPrev), Key::Char('N')),
    (NormalCmd::ExtendTillChar, |e, cx| e.extending(cx, NormalCmd::FindTillChar), Key::Char('t')),
    (NormalCmd::ExtendNextChar, |e, cx| e.extending(cx, NormalCmd::FindNextChar), Key::Char('f')),
    (NormalCmd::ExtendTillPrevChar, |e, cx| e.extending(cx, NormalCmd::TillPrevChar), Key::Char('T')),
    (NormalCmd::ExtendPrevChar, |e, cx| e.extending(cx, NormalCmd::FindPrevChar), Key::Char('F')),
    (NormalCmd::ExtendToLineStart, |e, cx| e.extending(cx, NormalCmd::GotoLineStart), Key::Home),
    (NormalCmd::ExtendToLineEnd, |e, cx| e.extending(cx, NormalCmd::GotoLineEnd), Key::End),
    (NormalCmd::ExtendParentNodeEnd, |e, cx| e.extending(cx, NormalCmd::MoveParentNodeEnd), Key::Alt('e')),
    (NormalCmd::ExtendParentNodeStart, |e, cx| e.extending(cx, NormalCmd::MoveParentNodeStart), Key::Alt('b')),
    (NormalCmd::ExtendToFileStart, |e, cx| e.extending(cx, NormalCmd::GotoFileStart), Key::Char('g')),
    (NormalCmd::ExtendToFileEnd, |e, cx| e.extending(cx, NormalCmd::GotoFileEnd), Key::Char('e')),
    (NormalCmd::ExtendToLastLine, |e, cx| e.extending(cx, NormalCmd::GotoLastLine), Key::Char('e')),
    (NormalCmd::ExtendToWord, |e, cx| e.extending(cx, NormalCmd::GotoWord), Key::Char('w')),
];

/// The method behind a command's name.
pub(super) fn normal_command(name: impl AsRef<str>) -> Option<&'static NormalCommand> {
    let name = name.as_ref();
    NORMAL_COMMANDS
        .iter()
        .chain(super::group_commands::GROUP_COMMANDS)
        .chain(EXTEND_COMMANDS)
        .chain(super::insert::INSERT_COMMANDS)
        .find(|(n, _, _)| n.name() == name)
}

/// The method behind a command.
pub(super) fn normal_cmd(cmd: NormalCmd) -> Option<&'static NormalCommand> {
    normal_command(cmd.name())
}

/// **Where a command is in the default keymap**: the group it is in
/// (`Pending::None` for the top level) and its key there, under the preset
/// `at` says: the vim preset's own entry first, helix's after, then the
/// groups'. `None` when no key reaches it.
pub(super) fn place_of(name: &str, at: &super::groups::Context) -> Option<(Pending, Key)> {
    let name = normal_command(name)?.0;
    let flavored = FLAVOR_NORMAL
        .iter()
        .find(|(_, n, item, only)| *n == name && at.has(Some(*item)) && at.holds(*only));
    if let Some((key, _, _, _)) = flavored {
        return Some((Pending::None, *key));
    }
    // A key the flavour gives to something else is not this command's.
    let flavor_took = |key: &Key| {
        FLAVOR_NORMAL.iter().any(|(k, _, item, only)| k == key && at.has(Some(*item)) && at.holds(*only))
    };
    if at.vim {
        if let Some((key, _, _)) = VIM_NORMAL.iter().find(|(_, n, _)| *n == name) {
            return Some((Pending::None, *key));
        }
    }
    // A helix key the vim preset gives to something else is not this
    // command's under vim (`j` is `move_line_down` there).
    let taken = |key: &Key| {
        flavor_took(key) || (at.vim && VIM_NORMAL.iter().any(|(k, _, when)| k == key && when.is_none()))
    };
    if let Some((key, _)) = HELIX_NORMAL.iter().find(|(key, n)| *n == name && !taken(key)) {
        return Some((Pending::None, *key));
    }
    if let Some((key, _)) = SELECT_NORMAL.iter().find(|(key, n)| *n == name && !taken(key)) {
        return Some((Pending::None, *key));
    }
    // A group's select-mode rows hold only while extending; their command
    // is there to be found either way.
    let selecting = super::groups::Context { extend: true, ..*at };
    [*at, selecting].iter().find_map(|at| {
        super::groups::GROUPS.iter().find_map(|group| {
            group
                .binds()
                .find(|bind| {
                    bind.run == name
                        && at.reaches(bind)
                        && !bind.says.is_empty()
                        // The row its key reaches, not one a row before it
                        // shadows; nor keys the vim preset plays itself.
                        && group.answers_with(bind, at)
                        && !(at.vim
                            && super::groups::group_opened_by(group, true).is_some_and(|path| {
                                let keys = format!("{}{}", path.concat(), super::keys::key_internal(bind.key));
                                super::groups::preset_took_path(&keys)
                            }))
                })
                .map(|bind| (group.pending, bind.key))
        })
    })
}

/// **The keys that reach a command**, in a binding's spelling (`gh`,
/// `" f"`): the group's keys and the command's own.
pub(super) fn keys_of(place: (Pending, Key)) -> String {
    let (pending, key) = place;
    let path = pending.group().map_or("", |group| group.path);
    format!("{path}{}", super::keys::key_internal(key))
}

/// **The keys a command is on in helix's keymap**, written as a person
/// writes them (`f`, `A-d`, `C-o`, `<space>f`): what `::` shows beside a
/// command, which has no editor to ask which preset is in use.
pub(crate) fn default_key_written(name: &str) -> Option<String> {
    let place = place_of(name, &super::groups::Context::for_docs())?;
    Some(yumete_cjk::actions::spell(&keys_of(place)))
}

impl Editor {
    /// **Run a Normal command by name**, as a binding or `::` does
    /// (2026-10-31): the count typed before the key that asked, and the key
    /// the command answers to by default (commands sharing one body tell
    /// their names apart by it). `false` when no command has the name.
    pub(super) fn run_normal(&mut self, name: &str) -> bool {
        let Some(&(cmd, run, key)) = normal_command(name) else { return false };
        let operator_count = self.count;
        let count = self.take_count();
        // In a table's grid a name means its action done to a cell.
        if self.takes_by_name(cmd, count) {
            return true;
        }
        self.before_a_motion(cmd);
        run(self, Cx { key, count, operator_count, continuing_zong: false });
        true
    }

    /// **The keys a command is on now**, in a binding's spelling, under the
    /// preset in use. `None` when it has none.
    pub(super) fn normal_keys_for(&self, name: &str) -> Option<String> {
        place_of(name, &self.group_context()).map(keys_of)
    }

    /// **The key that runs `name` now, as the palette and `:keymap-actions`
    /// write it** (2026-11-01): the keymap in use's own key while the
    /// reader has not bound it to something else, else the reader's own
    /// key for the command. They used to show helix's key under vim's
    /// keymap, and the default even where the reader had moved it.
    pub fn keys_now(&self, name: &str) -> Option<String> {
        self.run_now(name).map(|run| yumete_cjk::actions::spell(&run))
    }

    /// [`Self::keys_now`] in the internal spelling.
    ///
    /// A key the preset or the reader binds is not the default's any more
    /// (vim's `x` `s` `%` `K` `Q`), a key a flavour item has switched off
    /// does nothing, and with `vim_space_menu` off the Space menu is on `\`
    /// (found in review, 2026-11-02: all three were shown as they are under
    /// helix).
    pub(super) fn run_now(&self, name: &str) -> Option<String> {
        let aliases = &self.key_aliases;
        let space_moved = self.vim_keymap() && !self.flavor_on(FlavorItem::VimSpaceMenu);
        let shown = |run: String| match space_moved && run.starts_with(' ') {
            true => format!("\\{}", &run[1..]),
            false => run,
        };
        // A key of its own (not a group's) must do something now: a flavour
        // item switched off leaves it doing nothing. (Not "this command":
        // vim's `;` repeats an `f` only after one.)
        let reaches = |run: &str| match super::keys::pressed_key(run) {
            Some(key) => {
                let cx = Cx { key, count: 1, operator_count: None, continuing_zong: false };
                self.normal_command_for(&cx).is_some()
            }
            None => true,
        };
        if let Some(run) = self.normal_keys_for(name).filter(|run| !aliases.contains_key(run) && reaches(run)) {
            return Some(shown(run));
        }
        let mut mine: Vec<String> = aliases
            .iter()
            .filter(|(_, run)| run.len() == 1 && run[0] == name)
            .map(|(k, _)| k.clone())
            .collect();
        mine.sort();
        mine.into_iter().next().map(shown)
    }

    /// **A hint row's keys for Normal commands, as they are bound now**
    /// (2026-11-01): each slot is one or more commands, alternatives in the
    /// order to try (a vim twin before its helix one), and names the first
    /// that has a key; slots without one are left out, and the row is
    /// `None` when none has.
    pub(super) fn normal_hint(&self, slots: &[&[NormalCmd]]) -> Option<String> {
        let mut shown: Vec<String> = Vec::new();
        for slot in slots {
            let Some(run) = slot.iter().find_map(|cmd| self.run_now(cmd.name())) else { continue };
            let one = super::places::hint_run(&run);
            if !shown.contains(&one) {
                shown.push(one);
            }
        }
        (!shown.is_empty()).then(|| shown.join(" "))
    }

    /// **The command `cx.key` runs in Normal mode**, by the default keymap:
    /// the flavour's key when its item is on, then the vim preset's own entry
    /// when it has one whose condition holds, helix's otherwise.
    pub(super) fn normal_command_for(&self, cx: &Cx) -> Option<&'static str> {
        let at = self.group_context();
        let flavored = FLAVOR_NORMAL
            .iter()
            .find(|(key, _, item, only)| *key == cx.key && at.has(Some(*item)) && at.holds(*only));
        if let Some((_, name, _, _)) = flavored {
            return Some(name.name());
        }
        if self.vim_keymap() {
            let vim = VIM_NORMAL
                .iter()
                .find(|(key, _, when)| *key == cx.key && when.is_none_or(|holds| holds(self, cx)));
            if let Some((_, name, _)) = vim {
                return Some(name.name());
            }
        }
        if self.extend {
            if let Some((_, name)) = SELECT_NORMAL.iter().find(|(key, _)| *key == cx.key) {
                return Some(name.name());
            }
        }
        if self.vim_keymap()
            && (!self.expanding_alias || self.replaying_held)
            && HELIX_KEYS_UNDER_VIM.contains(&cx.key)
            && !self.flavor_on(FlavorItem::HelixKeysUnderVim)
        {
            return None;
        }
        HELIX_NORMAL.iter().find(|(key, _)| *key == cx.key).map(|(_, name)| name.name())
    }

    /// Run one Normal command by its variant, with its own row's key.
    pub(super) fn run_normal_cmd(&mut self, cx: Cx, cmd: NormalCmd) {
        if let Some(&(_, run, key)) = normal_cmd(cmd) {
            run(self, cx.with_key(key));
        }
    }

    /// **`D` / `A-D`**: from the cursor to the line's end, its break kept,
    /// done as `key` (`d` or `A-d`) would do it to a selection.
    pub(super) fn delete_to_line_end_as(&mut self, cx: Cx) {
        let rope = self.current_buffer().rope();
        let at = self.sel.head().min(rope.len_chars());
        let line = rope.char_to_line(at);
        let end = rope.line_to_char(line) + crate::zong::line_chars(rope, line).len();
        if end <= at {
            return;
        }
        let head = motion::prev_grapheme(rope, end).max(at);
        self.sel.set_anchor(at);
        self.sel.set_head(head);
        self.cmd_delete_or_change(Cx { count: 1, ..cx });
    }

    /// **Run a command extending the selection**, in either mode: what an
    /// `extend_*` command is. A command that waits for a character or a
    /// label (`f`, `g w`) extends when it arrives.
    pub(super) fn extending(&mut self, cx: Cx, cmd: NormalCmd) {
        let Some(&(_, run, _)) = normal_cmd(cmd) else { return };
        let was = self.extend;
        self.extend = true;
        if !self.takes_by_name(cmd, cx.count) {
            run(self, cx);
        }
        if self.pending != Pending::None || self.jumping() {
            self.motion_extend = Some(true);
        }
        if self.extend {
            self.extend = was;
        }
    }

    /// **The vim keymap is in use**: what vim's rules of the cursor and the
    /// registers ask (the cursor never sits on a line break, `f` stays on
    /// its line, `"1`–`"9` keep the deletes). The one place a rule asks.
    pub(super) fn vim_keymap(&self) -> bool {
        self.keymap_rules().vim_tables
    }

    /// **What the keymap in use does**, rule by rule
    /// ([`yumete_cjk::Rules`]): the cursor, the motions, the registers. Ask
    /// the rule a behaviour is, not which keymap this is.
    pub(super) fn keymap_rules(&self) -> yumete_cjk::Rules {
        self.key_preset.rules()
    }

    /// **Whether the command running is a `vim_` twin**, its helix twin, or
    /// (outside both, as in the paste menu) the keymap in use.
    pub(super) fn as_vim(&self) -> bool {
        self.acting_as_vim.unwrap_or_else(|| self.vim_keymap())
    }

    /// Run a command as one twin or the other ([`Self::as_vim`]).
    pub(super) fn acting(&mut self, vim: bool, run: impl FnOnce(&mut Self)) {
        let was = self.acting_as_vim.replace(vim);
        run(self);
        self.acting_as_vim = was;
    }

    /// Run the motion a wait was for, extending or not as the command that
    /// began the wait asked ([`Self::extending`]).
    pub(super) fn as_asked(&mut self, motion: impl FnOnce(&mut Self)) {
        let Some(asked) = self.motion_extend.take() else { return motion(self) };
        let was = self.extend;
        self.extend = asked;
        motion(self);
        if self.extend == asked {
            self.extend = was;
        }
    }

    // **Across the break.** A line is not a wall: `l` off the end of one
    // sentence lands on the start of the next, and `h` walks back over the
    // break the same way. The line-bound pair is still what measures a line;
    // it is not what a reader walking a page wants.
    // **四個鍵分開寫，因為 vim 分開管**：`whichwrap` 給 `h`/`l` 和兩個方向鍵各一個
    // 字母（`h` `l` `<` `>`），所以 `which_wrap = "h"` 只放開 `h`，方向鍵照舊停在行
    // 頭。綁在一起的話那一格就只剩「全放開」一檔。
    // **普通模式的退格是「左一格」**（`:h <BS>`；2026-10-10 補的）：`whichwrap` 的
    // `b` 管的就是它。helix 不綁它，所以只在 vim 鍵位下。
    pub(super) fn cmd_move_char_left(&mut self, cx: Cx) {
        if self.mode == Mode::Insert {
            return self.move_horizontal(motion::prev_grapheme);
        }
        let wrap = match cx.key {
            Key::Left => '<',
            Key::Backspace => 'b',
            _ => 'h',
        };
        self.repeat(cx.count, |e| e.step_sideways(motion::prev_grapheme, Some(wrap)))
    }

    pub(super) fn cmd_move_char_right(&mut self, cx: Cx) {
        if self.mode == Mode::Insert {
            return self.move_horizontal(motion::next_grapheme);
        }
        let wrap = match cx.key {
            Key::Right => '>',
            _ => 'l',
        };
        self.repeat(cx.count, |e| e.step_sideways(motion::next_grapheme, Some(wrap)))
    }

    // **`j`/`k` 是 vim 和 helix 真的答不一樣的一處**（2026-10-10 量出來的）。
    // vim 的 `j` 走**文件的行**（一段是一行，所以是下一段），`gj` 纔走屏幕的行；
    // helix 正好反過來——它的 `j` 綁的是 `move_visual_line_down`。兩個參考實現都
    // 實測過（nvim `--clean`：`j` 到第 2 行、`gj` 到第 1 行第 41 欄；helix 的
    // `default.rs:10` 是 `"j" | "down" => move_visual_line_down`、`:59` 的 `g` 層纔
    // 是 `move_line_down`）。So they are two commands, and each preset binds its own.
    pub(super) fn cmd_move_visual_line_up(&mut self, cx: Cx) {
        self.repeat(cx.count, |e| e.move_vertical(true))
    }

    pub(super) fn cmd_move_visual_line_down(&mut self, cx: Cx) {
        self.repeat(cx.count, |e| e.move_vertical(false))
    }

    pub(super) fn cmd_move_line_up(&mut self, cx: Cx) {
        self.repeat(cx.count, |e| e.move_textual_line(true))
    }

    pub(super) fn cmd_move_line_down(&mut self, cx: Cx) {
        self.repeat(cx.count, |e| e.move_textual_line(false))
    }

    // **`Home`/`End` and `g h`/`g l` are one command each**, as in helix
    // (2026-10-31). In a table these are the cell's two ends, and where the
    // cell is the unit, the record's (`goto_motion`).
    pub(super) fn cmd_goto_line_start(&mut self, _: Cx) {
        if self.mode == Mode::Insert {
            let pos = motion::line_start(self.current_buffer().rope(), self.sel.head());
            return self.set_cursor(pos);
        }
        self.goto_motion(motion::Motion::LineStart);
    }

    // vim's `$` remembers that it went to the end, so `j` keeps to line
    // ends; under the vim preset this does too.
    pub(super) fn cmd_goto_line_end(&mut self, _: Cx) {
        self.goto_motion(motion::Motion::LineEnd);
        if self.as_vim() {
            self.sel.set_goal(Some(super::keys::LINE_END_GOAL));
        }
    }

    // **`Enter`, `+` and `-`** — vim's three ways of saying 「the next
    // line, where its writing begins」 (B4, 2026-09-20). `Enter` is
    // unbound in Normal here, and `+`/`-` are in the preset's table;
    // this is the one that cannot be spelled there, because the table
    // holds characters.
    #[allow(unused_variables)]
    pub(super) fn cmd_goto_next_line_nonwhitespace(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat(count, |e| {
            let down = e.run_motion(motion::Motion::Line { down: true });
            e.jump_to(down);
            let home = e.run_motion(motion::Motion::LineFirstNonBlank);
            e.jump_to(home);
        });
    }

    // **A standalone motion, read vim's way** (B3, 2026-09-20).
    //
    // 2026-09-20：「vim 的 `w` 獨立的時候是跳轉，在命令中是選詞。
    // helix 就是將跳轉和選擇兩個 `w` 合一了。」 So under the vim preset
    // a bare motion asks for the **caret** reading: it goes to the
    // primitive — the next word's first character — and paints
    // nothing. Extend mode still extends, because `jump_to` moves the
    // head and leaves the anchor where it is: that is vim's visual
    // mode, for free.
    // Warning: **`H`/`L` 2026-10-06 從這張表上撤了**：它們在 vim 裏是屏幕的
    // 頂/底，而句子是 `(`/`)`（今早照 vim 綁的）。上面那一支接住了裸
    // 鍵；帶算子的 `dH`/`dL` 暫時答「不是一個動作」——要做得先有一個
    // 看得見視口的動作，記在 §5.95。
    #[allow(unused_variables)]
    pub(super) fn cmd_jump_by_word(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        let Key::Char(c) = key else { return };
        let grain = self.word_grain();
        let step = crate::vim::step_for(&c.to_string(), grain, None)
            .expect("the guard just asked");
        self.repeat(count, |e| {
            let span = e.read_motion(step.motion, motion::Reading::Caret);
            // **An empty line is a word to vim** (`:h word`, measured):
            // `w` and `b` stop on one; `e` passes it by.
            let span = match c {
                'w' | 'W' => e.stop_on_an_empty_line(span, true),
                'b' | 'B' => e.stop_on_an_empty_line(span, false),
                _ => span,
            };
            e.jump_to(span);
        });
    }

    // Word motions (Helix `w`/`b`/`e`, and WORD `W`/`B`/`E`).
    #[allow(unused_variables)]
    pub(super) fn cmd_move_next_word_start(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat(count, |e| e.select_word_forward(false));
    }

    // **`e` is coarse whatever the dictionary says** (#304). Chinese
    // has no spaces, so an `e` that consulted the dictionary would do
    // very nearly what `w` does; left coarse it runs to the next
    // punctuation instead — `w` takes a word, `e` takes a clause.
    //
    // And it sets **both** ends: a span anchored at the old caret
    // would give, from a word's last character, 「that character ＋ the
    // next word」 — the punctuation between them riding along in both
    // directions.
    #[allow(unused_variables)]
    pub(super) fn cmd_move_next_word_end(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
    self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::WordEnd(motion::Grain::Coarse));
                e.take_span(span);
            });
    }

    // **`b` is `e`'s partner, not `w`'s** (#304). helix's tutor teaches
    // 「to select the word under cursor, combine `e` and `b`」, and the
    // two only compose if they read the same grain. Ours are coarse, so
    // the pair takes a **clause** — which is the more useful unit here:
    // an English word is many letters, a Chinese word is two and a half
    // characters, and the thing a writer wants to grab is the run
    // between two 標點.
    //
    // The cost, taken deliberately: **there is no 「back one word」**.
    // `w` is the only key on the dictionary's grain, and it only goes
    // forward. `C-w` in Insert still deletes a *word* — it says why in
    // its own doc, and deleting a clause there would be brutal.
    #[allow(unused_variables)]
    pub(super) fn cmd_move_prev_word_start(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
    self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::WordBack(motion::Grain::Coarse));
                e.take_span(span);
            });
    }

    // A paragraph is a logical line here, and with soft wrap on `j`
    // and `k` move by visual row — so these are the keys that move by
    // what a writer calls a paragraph, and nothing else does.
    #[allow(unused_variables)]
    pub(super) fn cmd_move_next_paragraph(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
    self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::Paragraph { forward: true });
                e.take_span(span);
            });
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_move_prev_paragraph(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
    self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::Paragraph { forward: false });
                e.take_span(span);
            });
    }

    // 。！？ and the closing mark that follows one. The unit a person
    // proofreads in, and the one the manual already teaches by telling you
    // to break the file on 。 with `:%s`.
    //
    // **On `H`/`L` since 2026-09-12** (#404), for two reasons at once.
    // They were on `(`/`)`, which Helix spends on cycling the primary
    // selection — a multi-cursor key we will want when #405 lands, and
    // squatting on it now would mean moving twice. And the pair they
    // moved to reads better than the one they left: capitals are the
    // bigger unit along the same axis, so
    //
    // ```text
    // h  l   one character      j  k   one row
    // H  L   one sentence       J  K   half a page
    // ```
    //
    // Warning: `H`/`L` used to be whole-page paging. Nothing was lost:
    // `C-f` and `C-b` still do it, and the pair a reader actually
    // wears out is the *half* page on `J`/`K`.
    // **vim 鍵位下 `H`/`M`/`L` 是屏幕的頂/中/底**（2026-10-06 定）。
    //
    // Warning: **這一條 10-05 定過「不讓」，10-06 翻了。** 當時的理由是「句子是
    // 寫小說按得最多的單位」——而那不是理由（§5.94：兼容第一，讓位的另找
    // 空鍵）。這一次一個鍵都没丟：vim 的句子動作本來就是 `(`/`)`，今早照
    // vim 綁上去了。
    //
    // 屏幕畫了哪一段是前端每幀交過來的（`set_page_span`，`gw` 也靠它）。

    // **vim 的句子動作是 `(`/`)`**（2026-10-06）。
    //
    // Warning: **不能寫成鍵位表上的 `("(", "H")`**（寫的時候當場撞的）。單字符
    // 的別名是**替換**不是播放——`expanding_alias` 不會為它撥上，於是換出
    // 來的 `H` 又被上面那一支接住，`)` 跳到屏幕底下去了。自己一支最清楚。
    #[allow(unused_variables)]
    pub(super) fn cmd_jump_by_sentence(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        let Key::Char(one) = key else { return };
        self.repeat(count, |e| {
            // Warning: **`Reading::Caret`, not the selection reading** (2026-10-18).
            // vim's `)` *goes to* the next sentence's first character; this
            // editor's sentence motion *selects a sentence* (#404), which is
            // helix's reading and what `H`/`L` keep. Asking the selection
            // reading here left the caret on the full stop that ends the
            // sentence before, one cell short, and every operator built on it
            // inherited the error: against nvim, `d)` differed in 148 of 184
            // cases and `))d(` in all 184.
            //
            // **The two readings already exist** (B3) — `w` has the same split,
            // 「vim 的 `w` 獨立的時候是跳轉，在命令中是選詞」 — so this is the
            // question being put the way vim puts it, not a new behaviour. The
            // sentence *object* and helix's `H`/`L` are untouched, which is why
            // this did not have to wait on #404.
            let span = e.read_motion(
                motion::Motion::Sentence { forward: one == ')' },
                motion::Reading::Caret,
            );
            e.take_span(span);
        });
    }

    // Warning: **上面那一支要排在這兩個前面**（寫的時候當場撞的，當天第二
    // 次）。`match` 從上往下挑，句子那兩個在前面就把它整個遮住了。
    #[allow(unused_variables)]
    pub(super) fn cmd_goto_next_sentence(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
    self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::Sentence { forward: true });
                e.take_span(span);
            });
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_goto_prev_sentence(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
    self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::Sentence { forward: false });
                e.take_span(span);
            });
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_add_mark(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Mark;
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_goto_mark(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Recall;
    }

    // 「下一個這種東西」, which is where Helix keeps it too.
    // **`z` 那一層**：把光標這一行挪到屏幕的頂/中/底（2026-09-28）。
    // Warning: `zc` 也是居中，同 helix（它的 `zc` 是 align_view_center）。
    #[allow(unused_variables)]
    pub(super) fn cmd_view_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Aim;
        // `20zt`, `3zj`, `3zF`: the count is the key's after `z`.
        self.operator_count = operator_count;
    }

    // **`Z` 是同一層，按完不收**（helix 的 sticky view mode）：`Zjjjj` 一路滾。
    #[allow(unused_variables)]
    pub(super) fn cmd_sticky_view_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::AimStuck;
        // helix drops a count typed before `Z` (`3Zjj` scrolls two).
        self.operator_count = None;
    }

    // **`3]空格` 加三條空行**，所以這一族要把計數帶過去（同 `g`、`f`/`t`
    // 那幾個等第二鍵的前綴）。
    #[allow(unused_variables)]
    pub(super) fn cmd_bracket_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Hop { forward: key == Key::Char(']') };
        self.operator_count = operator_count;
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_move_next_long_word_start(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat(count, |e| e.select_word_forward(true));
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_move_next_long_word_end(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
    self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::WordEnd(motion::Grain::Big));
                e.take_span(span);
            });
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_move_prev_long_word_start(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
    self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::WordBack(motion::Grain::Big));
                e.take_span(span);
            });
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_goto_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Goto;
        self.operator_count = operator_count;
        // **開一個序列不算走了一步**（2026-10-13）：這一支開頭就把
        // `zong_motion` 取走了，而 `gj gj` 要和 `h h` 一樣守着同一個目標
        // 格——下一鍵纔是那個動作。
        self.zong_motion = continuing_zong;
    }

    // Helix's Space menu: the things that are not motions.
    #[allow(unused_variables)]
    pub(super) fn cmd_space_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Space;
    }

    // In-line character search — **all four of vi's**, `f` `F` `t` `T`.
    //
    // Warning: **`t`/`T` were the table group until 2026-09-21**, retired as
    // till-keys on the reasoning that a verb-last editor puts till
    // 「one keystroke away from find and no more」. Two things undid
    // that: the vim preset puts the verb **first** (`dt,`), and
    // **helix's own `t` is `find_till_char`**
    // (`helix-term/src/keymap/default.rs:14`) — so one key was costing
    // *both* hands their muscle memory to save one keystroke in the
    // one group that can afford to be a keystroke longer. 「表格操作
    // 並不是特別頻繁。」 The table group is `空格 t` now.
    #[allow(unused_variables)]
    pub(super) fn cmd_find_char(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Find(match key {
            Key::Char('f') => FindKind::Forward,
            Key::Char('F') => FindKind::Backward,
            Key::Char('t') => FindKind::Till,
            _ => FindKind::TillBack,
        });
        self.operator_count = operator_count;
    }

    // Select (extend) mode and collapse (Helix `v` / `;`).
    // **vim's `V` is linewise visual**, not 「select this line」
    // (B3, 2026-09-20): the selection grows by *lines* as the caret
    // moves, and a verb takes those lines entire. The old translation
    // (`V` → `x`) could take three lines only if told `3V`; a vim hand
    // types `Vjj`.
    #[allow(unused_variables)]
    pub(super) fn cmd_select_line_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.vim_line_column = {
            let rope = self.current_buffer().rope();
            let head = self.sel.head().min(rope.len_chars());
            head - rope.line_to_char(rope.char_to_line(head))
        };
        self.select_line();
        self.extend = true;
        self.vim_lines = true;
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_select_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.extend = !self.extend;
        self.vim_lines = false;
    }

    // Esc is every modal editor's way out. It leaves select mode and
    // collapses the selection — and, when the page is showing you
    // something in the other work area, it dismisses that first: a
    // preview is the transient thing on the screen, and Esc is the key
    // every reader presses at a transient thing.
    #[allow(unused_variables)]
    pub(super) fn cmd_normal_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        // In Insert mode it is `Esc` there (a binding's `jk`, as helix's).
        if self.mode == Mode::Insert {
            return self.leave_insert();
        }
        // **浮窗先收**（2026-10-01 提的）。它是屏幕上最臨時的那一樣，
        // 而 `Esc` 是人對着臨時的東西按的那一鍵。收的只是「這會兒別擋
        // 着」——挪開光標它又回來，邊欄一動不動。
        if self.hush_the_float() {
            return;
        }
        // Only a window opened to show something: Esc closed the reader's own
        // splits too, from the first window (review, 2026-11-03).
        if self.other_pane().is_some_and(|pane| pane.preview) && self.live_pane() == 0 {
            // **The window goes; the search stays.** Esc dismissed the
            // pane, and the reader who then presses `n` means the same
            // 「下一處」 they meant a moment ago — so `n` walks to the
            // next hit and brings the pane back with it. Throwing the
            // list away here handed `n` to `/` instead, which answered
            // about some older pattern, or about nothing at all, and
            // never said which.
            self.close_split();
            return;
        }
        // **Esc 没有別的事可做的時候，就是「把輸入法的挂起再說一
        // 遍」**（2026-09-22）。收窗口輪不到這一件；而說不說得成，看的是
        // 這一下有没有別的事做。
        //
        // Warning: **選區不收**（2026-10-06，對齊 helix）。helix 的
        // `exit_select_mode` 只改 mode，`enter_normal_mode` 從不動選區
        // （`helix-view/src/editor.rs`）。我們從前連錨點一起拉到頭上，於是
        // `v w w Esc d` 只刪掉一個空格，而 helix 刪掉 `alpha beta `——
        // 選中了一段、退出延伸、再動手，是這個模型裏最順的一條路，而
        // `Esc` 從前把中間那一步變成了「全丟掉」。
        //
        // 「退一層」那個意思留着：收的是**延伸**這一層，不是選區。要把
        // 選區收成一點有自己的鍵（`;`）。
        // Warning: **只問延伸還開不開着**（2026-10-06）。從前還要求選區是塌
        // 的，因為那一下 Esc 順手把它收了；現在它不收，於是「還有選區」
        // 不再是 Esc 有事可做——不改這一句的話，選區留着的時候 Esc 永遠
        // 說不出那一句。
        // **vim's Esc leaves Visual with nothing selected and the
        // cursor where it stood** (2026-10-30, as decided; `:h v_<Esc>`).
        // After `V` the cursor goes back to the column it was in.
        if self.as_vim() && self.extend {
            let at = match self.vim_lines {
                true => {
                    let rope = self.current_buffer().rope();
                    let line = rope.char_to_line(self.sel.head().min(rope.len_chars()));
                    self.column_on_line(line, self.vim_line_column)
                }
                false => self.sel.head(),
            };
            self.extend = false;
            self.vim_lines = false;
            self.sel.set_anchor(at);
            self.sel.set_head(at);
            self.clamp_cursor();
            self.refresh_goal_column();
            return;
        }
        let idle = !self.extend;
        self.extend = false;
        if idle {
            self.say_it_again = true;
        }
    }

    // `;` collapses the selection but leaves select mode standing —
    // Helix's own behaviour, and the reason it looks broken to a
    // reader who has just pressed `v`: the very next motion grows the
    // selection again. So it says which of the two happened.
    // **vim repeats a find with `;` and `,`** (#428, 2026-09-18) —
    // here that is `A-.`, which no vim hand will ever press. `,` is
    // the same search the other way round.
    // Warning: **…but not while an alias is being played** (2026-09-19).
    // The preset writes `;` into its own expansions to mean 「collapse
    // first」 (`x` is `;{n}D`), and once a find had happened that `;`
    // repeated the find instead — so after `f,` the preset's `x` cut
    // everything the `f` had selected rather than one character.
    #[allow(unused_variables)]
    pub(super) fn cmd_repeat_find(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        let Key::Char(c) = key else { return };
        if let Some((kind, ch)) = self.last_find {
            let kind = match c {
                ',' => kind.flipped(),
                _ => kind,
            };
            // 數目是「第 n 個」，重複要往前挪一個——兩件事都在
            // `find_nth_char` 裏，不靠把它叫 n 遍。
            self.find_nth_char(kind, ch, count, true);
        }
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_collapse_selection(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.sel.set_anchor(self.sel.head());
        if self.extend {
            self.status = say!("selection.collapsed-in-extend");
        }
    }

    // **多選區**（#405）：`C`/`A-C` 往下/往上再加一段，`,` 只留主選區。
    // 三個都是 helix 的拼法。Warning: vim 預設下 `C` 被別名成 `c$`，`,` 是「反向重複
    // 剛纔那個 f」，所以那一端按不到這裏——多選區是原生鍵位的東西。
    #[allow(unused_variables)]
    pub(super) fn cmd_copy_selection_on_next_line(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.copy_selection_on_row(true, count);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_copy_selection_on_prev_line(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.copy_selection_on_row(false, count);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_keep_primary_selection(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.keep_primary_selection();
    }

    // **`,` 的反面，和兩種合併，和「全部轉成正向」**（2026-10-06）。
    // §5.13.6 自己把這四個列成「空着的，直接拿來用」，而四期都没排到。
    #[allow(unused_variables)]
    pub(super) fn cmd_remove_primary_selection(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.remove_primary_selection();
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_merge_selections(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.merge_selections(false);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_merge_consecutive_selections(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.merge_selections(true);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_ensure_selections_forward(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.face_them_forward();
    }

    // **`X` 的反面**（helix 的 `shrink_to_line_bounds`）。`x`/`X` 一直都在，
    // 缺的是往回收的那一個（2026-10-06）。
    #[allow(unused_variables)]
    pub(super) fn cmd_shrink_to_line_bounds(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.shrink_to_line_bounds();
    }

    // **`A-s` 把每一段選區按行切開**（helix 的 `split_selection_on_newline`）。
    // Warning: 它是這一族裏唯一不用打正則的，所以不開提示行。
    #[allow(unused_variables)]
    pub(super) fn cmd_split_selection_on_newline(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.split_on_newline();
    }

    // **正則那四個**（#405 Phase 2）。它們開的是**搜索那一扇**提示行，於是拼音、
    // 簡繁、模糊、正則四個開關一起管用，中文也照打——helix 的 `s` 只認正則。
    // **`(` `)` 換主選區，`_` 去兩端空白**（#405 Phase 3）。Warning: 前兩個一段都不
    // 動，動的只是「哪一段是主的」——選了二十處要一處一處看過去靠的就是它。
    // 帶數字走幾格——helix 的 `rotate_selections` 吃計數
    // （`helix-term/src/commands.rs:5649`），從前這裏 `3)` 只走一格
    // （2026-10-06）。
    #[allow(unused_variables)]
    pub(super) fn cmd_rotate_selections_forward(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat(count, |e| e.rotate_primary(true));
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_rotate_selections_backward(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat(count, |e| e.rotate_primary(false));
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_trim_selections(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.trim_selections();
    }

    // **`&` 把每一段的開頭對齊到同一列**（helix 的 `align_selections`）。
    // Warning: 算的是顯示寬度，一個漢字兩格。
    #[allow(unused_variables)]
    pub(super) fn cmd_align_selections(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.align_selections();
    }

    // **`A-(` `A-)` 輪轉的是裝在選區裏的字**，邊界不動（helix 的
    // `rotate_selection_contents_*`）。上面那一對換的是「哪一段是主的」。
    #[allow(unused_variables)]
    pub(super) fn cmd_rotate_selection_contents_forward(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.rotate_contents(true);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_rotate_selection_contents_backward(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.rotate_contents(false);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_select_regex(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.open_sift(multi::Sift::Select);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_split_selection(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.open_sift(multi::Sift::Split);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_keep_selections(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.open_sift(multi::Sift::Keep);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_remove_selections(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.open_sift(multi::Sift::Drop);
    }

    // Selection + changes (Helix: `x` selects the line, `d` deletes the
    // selection, `c` changes it).
    //
    // Warning: **這一族 2026-09-28 翻過一次，現在是 helix 的那一條**（#405）。
    //
    // #492 當初把它定成「小寫刪、大寫剪」，原話是「d 作为剪切功能会污染
    // register。这是我觉得 helix 最不好的地方」——剪貼板只有一個，而編輯器裏
    // 按得最多的那個鍵會花掉它。依據之一是「helix 的 `C`/`D` 都空着」，而**那
    // 句話錯了一半**：helix 的 `C` 是 `copy_selection_on_next_line`，佔着，
    // 只有頂層的 `D` 是空的。多選區（#405）要的正是 `C`。
    //
    // 現在：`d`/`c` 進寄存器（同 helix），`A-d`/`A-c` 不進（同 helix），
    // **`D` 是 `A-d` 的別名**——#492 那個需求（刪掉這段，別蓋掉我剛複製的）
    // 保下來了，只是從最好按的鍵換成了第二好按的。`A-c` 没有對應的別名，
    // 因為 `C` 讓給了多選區。
    #[allow(unused_variables)]
    pub(super) fn cmd_extend_line_below(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat(count, |e| e.select_line());
    }

    // **`d`, `c`, `y` are operators under the vim preset** (#429,
    // 2026-09-18): they wait for a motion. With something already
    // selected they act at once, which is what vim's visual mode does
    // — and it is also what makes `v3wd` go on working for a hand that
    // learnt this editor's own way round.
    #[allow(unused_variables)]
    pub(super) fn cmd_vim_operator(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        let Key::Char(op) = key else { return };
        self.pending = Pending::VimOperator { op, first: None };
        self.count = operator_count;
    }

    // Warning: **2026-09-28 整族換成了 helix 的拼法**（#405 要 `C` 這個鍵）。從前是
    // 「小寫刪、大寫剪」，四個鍵一條規矩；現在是 helix 的那一條：
    //
    // - `d` `c`：刪/改，**進寄存器**（helix 一樣）
    // - `A-d` `A-c`：刪/改，**不進**（helix 一樣）
    //
    // Warning: **`D` 在 helix 鍵位下有意不綁**（2026-10-07 覆查時定的）。這一段
    // 從前寫着「`D` 是 `A-d` 的別名，留一個好按的大寫鍵給它」——**代碼裏
    // 從來没有那個別名**（下面那道閘 `|| key_preset == Vim` 就是把 `D` 擋在
    // 外面的），按下去是一句提示。查證的那一天順便把帳算清了：多一個入口
    // 就多一份要記的事，而 `A-d` 本來就是 helix 的講法。
    //
    // 旁證：helix 那邊 2024 年起有一個九十多人點讚的討論（#10361）要把
    // `d` 改成不進寄存器、`A-d` 改成進——至今没改。這一頭不替他們改。
    #[allow(unused_variables)]
    pub(super) fn cmd_delete_or_change(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        // A selection reaching into a closed fold takes the fold whole, as
        // vim's operators do (2026-11-03): `x` on a fold deletes it.
        if self.select_over_closed_folds() {
            // Whole lines, so vim's `x` is not held to its line.
            self.vim_lines = true;
            let cx = Cx { count: 1, ..cx };
            return self.cmd_delete_or_change_now(cx);
        }
        self.cmd_delete_or_change_now(cx)
    }

    #[allow(unused_variables)]
    fn cmd_delete_or_change_now(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.snapshot();
        // A count deletes that many graphemes when there is nothing
        // selected, the way `3x` does in vim; with a selection it is
        // the selection that goes, once.
        if self.span().0 == self.span().1 && count > 1 {
            self.extend_by_graphemes(count);
        }
        // **Through the one door** (B2, 2026-09-20): helix hands the
        // verb its own selection, vim will hand it a motion's span,
        // and neither knows the other exists.
        // **`V` means whole lines**, however far along one the caret
        // stopped (B3).
        let whole_lines = self.vim_lines;
        if self.vim_lines {
            self.extend_to_line_bounds();
            self.vim_lines = false;
        }
        // Warning: **`D` 只有 vim 預設下走得到這裏**，而那一端它要進寄存器：那頭的
        // `D` 被別名成 `d$`（`yumete-cjk/src/keymap.rs`），所以原生的 `D` 只在
        // 別名展開裏出現，而 vim 的 `x` 展開成 `;{n}D`，vim 的 `x` 就是進寄存
        // 器的。
        // **開關關掉就對調**（`:yank-on-delete`，2026-10-08）：`d`/`c` 刪
        // 而不進，`A-d`/`A-c` 變成剪的那一對。寫了 `[keys.normal]` 那四行
        // 的人走不到這裏——別名那一層在前面就把鍵換掉了。
        let alt = matches!(key, Key::Alt('d') | Key::Alt('c'));
        let cut = alt != self.yank_on_delete();
        let op = match matches!(key, Key::Char('c') | Key::Alt('c')) {
            true => motion::Operator::Change { cut },
            false => match cut {
                true => motion::Operator::Cut,
                false => motion::Operator::Delete,
            },
        };
        let (anchor, head) = self.span();
        // Warning: **vim 的 `x` 不吃換行符**（2026-10-09 審出來的，`:h x`
        // 「does not delete a line break」）。這一支在 vim 鍵位下是 `x` 與 `s`
        // 走的那一條（它們拼成 `;{n}D`／`;{n}Di`），而 helix 的光標是一格寬的
        // 選區——一行的字用完了，那一格罩着的就是換行符。於是 `abcde` 上按第
        // 六下 `x` 把兩行併了，第七下開始啃下一段：**按住 `x` 走過整篇稿子，
        // 而屏幕上一處都不停**。nvim 第六下什麽都不做。`20x`、`DD`、`D.`、
        // `9s` 都是同一條（`D` 在那一端拼成 `d$`）。
        //
        // 只在 vim 鍵位下夾，而且只夾到**本行末**：helix 的 `d` 本來就取光標
        // 那一格（真 helix 逐格對過，一樣），`dd`／`V` 那一路是整行的
        // （`whole_lines`），而 `dw`／`d}` 那一路不經過這一支。
        //
        // Warning: **可視模式不夾**（`vim_visual_mode_acts_on_the_whole_selection`
        // 攔下來的）。vim 的可視模式裏 `x` 刪的是**整段選區**，跨幾行就跨幾行；
        // 那一支把 `x` 當成 `d` 轉進來（`vim_visual_key`），`extend` 還掛着，
        // 問它就分得開。不夾的只是這一種：Normal 的 `x` 從來没有真選區。
        let (anchor, head) = match self.keymap_rules().delete_stays_in_line
            && !whole_lines
            && !self.extend
        {
            false => (anchor, head),
            true => {
                let rope = self.current_buffer().rope();
                let line = rope.char_to_line(anchor.min(rope.len_chars()));
                let breaks_at =
                    rope.line_to_char(line) + motion::line_char_len(rope, line);
                // 這一行的字已經用完了（空行，或者光標停在換行符上）。
                if anchor >= breaks_at {
                    return;
                }
                (anchor, head.min(breaks_at - 1))
            }
        };
        self.apply(op, motion::Span::Over { anchor, head });
    }

    // Yank / paste (Helix `y` / `p` / `P`).
    #[allow(unused_variables)]
    pub(super) fn cmd_yank(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        if operator_count.is_none() && self.info_afloat() == Some(crate::sidebar::Info::Change) && self.change_here().is_some() {
            let text = self.change_here().map(|run| run.was.join("\n"));
            self.clipboard_request = text;
        } else {
            if self.vim_lines {
                self.extend_to_line_bounds();
                self.vim_lines = false;
            }
            let (anchor, head) = self.span();
            self.apply(motion::Operator::Yank, motion::Span::Over { anchor, head });
        }
    }

    // A count is that many copies pasted as one piece, selected
    // together (helix) or with the cursor where vim puts it.
    #[allow(unused_variables)]
    pub(super) fn cmd_paste(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        // vim's: a block taken is a block pasted (`:h v_b_p`), and Visual
        // `p` writes the register over the selection.
        if self.as_vim() && self.register_block && self.pending_register.is_none() {
            return self.paste_block(key == Key::Char('p'));
        }
        if self.as_vim() && self.extend && !self.expanding_alias && key == Key::Char('p') {
            return self.vim_visual_key('p');
        }
        let Key::Char(c) = key else { return };
        self.paste_copies = count.clamp(1, Self::WRITING_MAX);
        self.paste(c == 'p');
        if count > Self::WRITING_MAX {
            self.status = say!("count.writing-ceiling", Self::WRITING_MAX);
        }
    }

    /// vim's `]p` and `[p`: a paste whose lines take the cursor line's
    /// indent. In Visual it is the plain replacing `p`.
    pub(super) fn paste_indented(&mut self, cx: Cx) {
        self.paste_fixes_indent = true;
        self.cmd_paste(cx);
        self.paste_fixes_indent = false;
    }

    // Insert (`i` before the selection, `a` after it, `I`/`A` line ends).
    // **vim 預設下，可視模式裏的 `i`/`a` 是物件前綴**（2026-09-28 收到的反饋：
    // 「vi* 进入选择/高亮模式（目前是因为 i 键的关系，进入了插入模式）」）。
    //
    // Warning: **只改 vim 那一端。** helix 的 select 模式是整份繼承 normal 的
    // （`keymap/default.rs:342` 的 `select = normal.clone()`，`i` 没有被蓋掉），
    // 所以 helix 按 `vi` 也是進插入——原生鍵位現在的行為就是 helix 的行為，
    // 原生那一端要取物件是 `mi(`。vim 的 `vi(` 纔是選中括號裏那一段。
    //
    // 代價是 vim 預設下不能再從可視模式按 `i` 進插入。vim 自己也不能：那一端
    // 的 `i` 在可視模式裏就是物件前綴。要進插入先按 `v` 或 `Esc` 出來。
    #[allow(unused_variables)]
    pub(super) fn cmd_select_object(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        let Key::Char(key) = key else { return };
        self.pending = Pending::MatchPair { around: key == 'a' };
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_insert_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.snapshot();
        let (pos, end) = self.selection();
        let ranged = self.has_selection();
        self.set_cursor(pos);
        self.enter_insert();
        // **`i` keeps the selection, after what you type** (2026-10-27,
        // helix's: `insert_mode` flips the range and keeps it). Until
        // then it dropped to one character, which the manual said on
        // purpose. The vim keymap's `i` leaves a cursor, as vim's does.
        //
        // Warning: **only for a real selection.** A bare cursor is not one
        // here (`has_selection`), and keeping it would paint the
        // character after the caret as selected all through the typing;
        // `Esc` lands on that character either way, as in helix.
        if !self.as_vim() && ranged && end > pos {
            self.sel.set_anchor(end);
            self.insert_keeps_tail = true;
            self.keep_anchor_while_inserting = true;
        }
        self.type_it_again(count, None);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_append_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.snapshot();
        // **錨點留在選區的開頭**（2026-10-10，照 helix）：`a` 是「在選區
        // 後面插入」，所以打進去的字和原來那一段一起成為新的選區。
        let from = self.selection().0;
        let pos = self.append_position();
        // vim's `a` on an empty line types on that line (`:h a`,
        // measured: `aZ` on an empty first line gives `Z`); helix's
        // appends after the line break, on the next line.
        let pos = match self.as_vim() && !self.extend {
            true => {
                let rope = self.current_buffer().rope();
                let head = self.sel.head();
                match head < rope.len_chars() && matches!(rope.char(head), '\n' | '\r') {
                    true => head,
                    false => pos,
                }
            }
            false => pos,
        };
        self.set_cursor(pos);
        self.enter_insert();
        // **`a` 出門的時候要退回那一個字上**（2026-10-09，照 helix）。
        // `enter_insert` 剛把這一格清了，所以擺在它後面。
        self.restore_caret = true;
        // Warning: **只有 helix 鍵位。** vim 的 `a` 打完 `Esc` 只剩一個字的
        // 光標；兩套在這一條上真的不一樣，而 vim 那邊 `x` ＝ `;D`，`;`
        // 先把選區塌掉——所以「看起來對」掩得住這個錯，要問選區纔問得出來。
        if !self.as_vim() {
            self.sel.set_anchor(from);
            self.keep_anchor_while_inserting = true;
        }
        self.type_it_again(count, None);
    }

    // **全窗裏這兩個也是這一格的兩頭**（2026-10-17，審查找出來的）。
    // 「這一行」在全窗裏就是「這一格」——`g h`/`g l`/`Home`/`End` 早
    // 就這樣了，而這兩個還對準着行的兩端，也就是那兩道牆：`A` 在行尾
    // 那道牆後面打字，打出了第四格；`I` 在行首那道牆前面打字，那一行
    // 當場不再是一行記錄。現在它們問 `cell_edge`。
    #[allow(unused_variables)]
    pub(super) fn cmd_insert_at_line_start(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.snapshot();
        // **After the indent**, as helix's `insert_at_line_start` and
        // vim's `I` both put it (2026-10-27, measured: `  ab` + `IX`
        // gave `X  ab` here). A line of blanks only: helix goes to its
        // start, vim to its end.
        let pos = self.cell_edge(false).unwrap_or_else(|| {
            let rope = self.current_buffer().rope();
            let head = self.sel.head();
            let (start, end) = (motion::line_start(rope, head), motion::line_end(rope, head));
            let first = motion::line_first_non_blank(rope, head);
            let written = first < end && !rope.char(first).is_whitespace();
            match (written, self.as_vim()) {
                (true, _) => first,
                (false, true) => end,
                (false, false) => start,
            }
        });
        self.set_cursor(pos);
        self.enter_insert();
        self.type_it_again(count, None);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_insert_at_line_end(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.snapshot();
        // 格尾那一頭追加，所以要的是內容的開區間末端，不是末一個字。
        let pos = match self.cell_the_caret_is_in() {
            Some((_, to)) => to,
            None => motion::line_end(self.current_buffer().rope(), self.sel.head()),
        };
        self.set_cursor(pos);
        self.enter_insert();
        self.type_it_again(count, None);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_open_below(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.snapshot();
        self.open_line_below();
        self.type_it_again(count, Some(true));
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_open_above(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.snapshot();
        self.open_line_above();
        self.type_it_again(count, Some(false));
    }

    // **改動的帶子開着的時候，`u`／`y` 歸它**（2026-10-11）。
    //
    // Warning: **浮窗不收鍵是通則，這一種是例外。** 定下來的形狀是
    // 「The hint about restore and and copy the previous version will be
    // displayed in the command line」——底邊那一行寫着 `u 還原此處改動`，
    // 而那一行是人照着按的。少了這兩條，那句提示是空話：按下去走的是撤銷
    // 與複製選區，和它說的兩件事都不是。
    //
    // 帶子只在按過 `空格 x` 之後的那一會兒開着，走出那幾行或 `Esc` 就收，
    // 所以這一下搶不到別的時候的 `u`；而數字開頭（`3u`）明說了是撤銷三步，
    // 讓給撤銷。還原本身記進歷史，所以按錯了 `u` 再按一次就退得回來。
    //
    // Warning: **問的是「帶子開着**而且**這裏真有一塊」**。還原完那一塊
    // 就不在了，而 `info_asked` 還記着——光問帶子，下一下 `u` 會落進
    // 「本行無任何改動」，把人的撤銷吃掉。
    // Undo/redo (Helix: `u` / `U`).
    #[allow(unused_variables)]
    pub(super) fn cmd_undo(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        if operator_count.is_none() && self.info_afloat() == Some(crate::sidebar::Info::Change) && self.change_here().is_some() {
            self.restore_the_change_here();
            // 問題答完了，帶子收起來——它畫的那幾行此刻已經是另一份文本。
            self.info_asked = None;
        } else {
            self.repeat_writing(count, |e| e.undo());
        }
    }

    // **vim 鍵位下 `U` 是「撤完這一行」**（2026-10-06，`:h U`）。重做在那一端
    // 是 `C-r`（鍵位表上一直有），所以這個鍵讓得出來。
    #[allow(unused_variables)]
    pub(super) fn cmd_undo_line(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat_writing(1, |e| e.undo_this_line());
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_redo(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat_writing(count, |e| e.redo());
    }

    // **`A-u`／`A-U` 也是撤銷／重做**（2026-10-06，helix 的 `earlier`／`later`）。
    //
    // Warning: **helix 那兩個走的是一棵樹**：撤銷之後再改一筆，舊的那一支在它
    // 那裏還回得去，`A-u` 是「往前一個時刻」而不是「撤銷一步」。這個編輯器
    // 的歷史是一條線，重做過的那一支改一筆就没了——所以這裏兩個鍵是同一件
    // 事。鍵按下去有它該有的方向，只是回不到另一支上。
    #[allow(unused_variables)]
    pub(super) fn cmd_earlier(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat_writing(count, |e| e.undo());
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_later(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat_writing(count, |e| e.redo());
    }

    // Search (`/` forward, `?` backward, `n`/`N` repeat).
    // `!` is what it is in vi: send this through a command and take
    // what comes back. It opens the command line with the verb already
    // typed, so the key is a shortcut and not a second mechanism —
    // and so a reader who presses it by accident can see what it was
    // about to do and press Esc.
    // **helix 的 shell 四件**（2026-10-06 對齊）。從前只有 `!`，而它做的是
    // helix 的 `|`（用輸出換掉選區）——一個 helix 的手按 `!date⏎` 以為在
    // 插一行，實際把選中那段換掉了。**這張單子上唯一會丟字的一條。**
    // **vim 的 `|`：跳到第 N 格**（2026-10-06 定「按格，同 vim」）。
    //
    // Warning: **格，不是字。** 這個倉對「欄」有兩個答案——`cursor_column` 數的
    // 是第幾個**字**（狀態欄那個「列」就是它），`cursor_visual_column`
    // 數的是第幾**格**，一個漢字兩格。vim 的 `|` 數的是屏幕欄，而在中文
    // 稿子裏只有「格」跟尺規（`:rules`）和夾行對得上。反查那一支
    // （`pos_at_visual_column`）本來就有，縱向移動一直在用它。
    //
    // helix 鍵位下 `|` 是 shell 那一族（今天剛對齊），所以這一條只在
    // vim 鍵位下。
    #[allow(unused_variables)]
    pub(super) fn cmd_goto_column(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        let goal = count.saturating_sub(1);
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.sel.head());
        let at = motion::pos_at_visual_column(rope, line, goal);
        self.sel.collapse_to(at);
        self.clamp_cursor();
        self.refresh_goal_column();
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_shell_pipe(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.open_the_command_line("pipe ");
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_shell_insert_output(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.open_the_command_line("pipe-before ");
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_shell_append_output(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.open_the_command_line("pipe-after ");
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_shell_pipe_to(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.open_the_command_line("pipe-to ");
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_search(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.mode = Mode::Search;
        self.search_forward = true;
        self.command_line.clear();
        self.command_caret = self.command_line.chars().count();
        // A new search takes `n` back from the table's.
        self.hits = None;
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_rsearch(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.mode = Mode::Search;
        self.search_forward = false;
        self.command_line.clear();
        self.command_caret = self.command_line.chars().count();
        self.hits = None;
    }

    // In a table with a search open, `n` walks *its* answers: they are
    // the last search that happened, which is what `n` has always
    // meant. A plain `/` clears them and takes the key back.
    #[allow(unused_variables)]
    pub(super) fn cmd_search_next(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        if self.walking_hits() {
            self.repeat(count, |e| {
                e.walk_table_hits(true);
            });
        } else {
            if self.vim_keymap() {
                self.remember_jump();
            }
            self.repeat(count, |e| e.repeat_search(e.search_forward));
        }
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_search_prev(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        if self.walking_hits() {
            self.repeat(count, |e| {
                e.walk_table_hits(false);
            });
        } else {
            if self.vim_keymap() {
                self.remember_jump();
            }
            self.repeat(count, |e| e.repeat_search(!e.search_forward));
        }
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_command_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.open_the_command_line("");
    }

    // Match mode (Helix `m`): matching bracket, textobjects, surround.
    // The count goes with it (2026-10-27): `2mi(` and `2md(` are the
    // second pair out, as in helix.
    #[allow(unused_variables)]
    pub(super) fn cmd_match_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Match;
        self.operator_count = operator_count;
    }

    // Overwrite every character of the selection with the next key.
    #[allow(unused_variables)]
    pub(super) fn cmd_replace(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Replace;
        // vim's `3rX` writes over three characters (`:h r`).
        self.operator_count = operator_count;
    }

    // Name the register the next yank, delete or paste will use.
    #[allow(unused_variables)]
    pub(super) fn cmd_select_register(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Register;
    }

    // Record a macro, and play the last one back.
    // **`Q` records, `q` replays** — Helix's way round, and ours since
    // 2026-09-12. It was the other way, and both keys explained
    // themselves, but Warning: **a swapped pair is the worst kind of
    // divergence**: the hand does not read, it just presses, and the
    // wrong one of these two is not a no-op — it starts recording over
    // what you meant to play back (#404).
    #[allow(unused_variables)]
    pub(super) fn cmd_record_macro(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.toggle_recording();
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_replay_macro(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.replay_macro(count);
    }

    // A page, and half of one, in the direction the text is read.
    // The next region — the panels and the work areas, in the order
    // they sit on the screen. vi spells window motions `C-w`, and this
    // is the one it spells `C-w w` (#293).
    // **`C-w` 是那一組的另一扇門**（2026-09-30 定，同 helix：
    // `default.rs:193` 是 `C-w`，`:260` 是 `空格 w`，兩處同一組）。
    // 走下一區因此是 `C-w w`——vi 本來就是這麽拼的。
    #[allow(unused_variables)]
    pub(super) fn cmd_window_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Region;
        // vim's `2 C-w w` and `3 C-w x` name a window.
        self.operator_count = operator_count;
    }

    // …and the keys a keyboard already has for it. `C-f`/`C-b` are
    // vi's; these are the ones a reader who has never used vi presses.
    // Warning: **浮窗開着的時候它們歸浮窗**（見 `on_key` 開頭，同 helix）。
    #[allow(unused_variables)]
    pub(super) fn cmd_page_down(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        if self.layout() == Layout::Vertical || !self.scroll_a_page(true, count) {
            self.move_page(count, false, 1.0);
        }
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_page_up(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        if self.layout() == Layout::Vertical || !self.scroll_a_page(false, count) {
            self.move_page(count, true, 1.0);
        }
    }

    // Back and forward through the places jumps came from, as in vi
    // and in Helix. Under the Kitty protocol `C-i` and Tab are told
    // apart; without it a terminal sends the same byte for both, and
    // `C-i` simply does whatever Tab does.
    // **`C-s` 把這個選區記進跳轉表**（2026-10-06，helix 的 `save_selection`）。
    //
    // Warning: **終端本來把 `C-s` 當 XOFF**（停止輸出）。這個編輯器跑在 raw mode
    // 裏，流控是關着的，所以它到得了這裏。作者 2026-10-06 定「Who cares
    // about terminal. Just be aligned」——鍵位對齊優先（§5.94）。
    // **`A-J`：併行，並且選中補出來的那一格**（2026-10-06，helix 的
    // `join_selections_space`）。接縫是算出來的，選中它纔看得見算成了什麼。
    // A selection over several lines joins all of them, as `J` does
    // (2026-10-30: it joined the first pair only).
    #[allow(unused_variables)]
    pub(super) fn cmd_join_selections_space(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        let n = self.join_count(operator_count);
        self.select_the_seam = true;
        self.repeat_writing(n, |e| e.join_lines());
        self.select_the_seam = false;
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_save_selection(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.remember_jump();
        self.status = say!("jump.selection-saved");
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_jump_backward(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.fold_stays_closed = true;
        self.walk_jumps(true, count);
    }

    // Warning: **`Tab` 在 Normal 裏也是這一件事**（2026-10-06，對齊 helix 的
    // `jump_forward`）。終端把 `Tab` 和 `C-i` 發成同一個字節，而這個編輯器
    // 分得開（`kitty` 那套協議），於是從前 `Tab` 在 Normal 裏一個 arm 都
    // 没有、靜默掉地——偏偏一個終端一換，手上那個 `C-i` 就變成了它。
    #[allow(unused_variables)]
    pub(super) fn cmd_jump_forward(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.fold_stays_closed = true;
        self.walk_jumps(false, count);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_half_page_down(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.half_page(false, operator_count);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_half_page_up(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.half_page(true, operator_count);
    }

    /// `C-d` `C-u`: vim's scroll the view and take a count as 'scroll';
    /// helix's take no count (`page_cursor_half_down` reads none).
    fn half_page(&mut self, back: bool, count: Option<usize>) {
        if self.vim_keymap() && self.layout() == Layout::Horizontal && !self.sel.is_plural() && self.vim_half_page(!back, count) {
            return;
        }
        let count = match self.vim_keymap() {
            true => count.unwrap_or(1),
            false => 1,
        };
        self.move_page(count, back, 0.5);
    }

    // **三分之二頁**（2026-10-01 定）。起因是 `C-u`/`C-d` 在 Mac 上
    // 不好按——那塊鍵盤只有一個 Ctrl，而 `u` 和 `d` 都在左手這一邊；
    // `n` 和 `p` 在右手，左 Ctrl 配右手字母順得多。
    //
    // Warning: **二三頁不是哪一家的慣例。** 兩種慣例是「整頁留一兩行」
    // （vim 的 `C-f`、Emacs 的 `next-screen-context-lines`）與「半頁」
    // （vim 的 `C-d`）。推理和 vim 留那兩行是同一個：留一截做
    // 錨。只是留得大方——「前一页留 1/3 不会断，2/3 的新页增加阅读效
    // 率」。三種都留着，用幾天見分曉。
    #[allow(unused_variables)]
    pub(super) fn cmd_two_thirds_page_down(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.move_page(count, false, 2.0 / 3.0);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_two_thirds_page_up(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.move_page(count, true, 2.0 / 3.0);
    }

    // **`J` 合併行、`K` 按式子篩選區**——helix 出廠就是這兩個
    // （`default.rs:166`/`:168`），2026-10-06 對齊。
    //
    // Warning: **這一格從前是「前進/後退半頁」**，而且 2026-10-05 專門定過不讓
    // （理由是「讀一本小說按得最多的一對，不該是和弦」）。10-06 翻案，
    // 原話：「既然對齊就干脆點……JK 本质上也是和弦（上档）。因此 Ctrl n / p
    // 并不比 JK 更难按。」——翻半頁現在走 `C-n`/`C-p`（⅔）、`C-d`/`C-u`
    // （半頁）、`C-f`/`C-b`（整頁），六個鍵都在。
    //
    // vim 鍵位各跟各的：那邊 `J` 也是合併行（一樣），`K` 是查詞（`空格 k`）。
    #[allow(unused_variables)]
    pub(super) fn cmd_join_selections(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        if self.as_vim() && self.extend && !self.expanding_alias {
            return self.vim_visual_lines(key, 'J');
        }
        if self.joining_welds_a_grid() {
            self.status = say!("table.join-would-change-columns");
        } else {
            let n = self.join_count(operator_count);
            let kept = self.sel.primary();
            let rope = self.current_buffer().rope();
            let (len, seam) = (rope.len_chars(), motion::line_end(rope, kept.span().0));
            self.repeat_writing(n, |e| e.join_lines());
            // **The selection stays on its text** (2026-10-27, helix's
            // `join_selections` only maps it through the change; measured:
            // `J` on `ab` left the cursor on `a` there and on the seam here).
            // vim's `J` puts the cursor on the seam, and keeps doing so.
            if !self.as_vim() {
                let now = self.current_buffer().rope().len_chars();
                let moved = |p: usize| match p <= seam {
                    true => p,
                    false => (p + now).saturating_sub(len).max(seam).min(now.saturating_sub(1)),
                };
                self.sel.set_anchor(moved(kept.anchor));
                self.sel.set_head(moved(kept.head));
                self.clamp_cursor();
            }
        }
    }

    // Swap which end of the selection the cursor is on.
    #[allow(unused_variables)]
    pub(super) fn cmd_flip_selections(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.flip_selection();
    }

    // Whole file, and extending the selection to whole lines.
    #[allow(unused_variables)]
    pub(super) fn cmd_select_all(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.select_all();
    }

    // vim's `X` under the vim preset (#428): the character *before*
    // the cursor, cut, and never across the start of the line — which
    // is why it is an action and not a line in the preset's table: a
    // translation into `h` would walk over the break and join two
    // lines, and could not carry `3X` to the cut.
    // **vim's redo is `C-r`** (#428, 2026-09-18): here it is `U`, and
    // a vim hand pressing `C-r` got a line of prose pointing at it.
    // Under the preset it simply redoes.
    #[allow(unused_variables)]
    pub(super) fn cmd_vim_redo(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat(count, |e| e.redo());
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_delete_char_backward(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        // Insert mode's Backspace (pairs, overwrite, the server's list).
        if self.mode == Mode::Insert {
            return self.insert_backspace();
        }
        self.snapshot();
        self.cut_before_cursor(count);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_extend_to_line_bounds(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.extend_to_line_bounds();
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_case_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::Case;
    }

    // **`~` is `` ` `` `` ` ``, and `*` is `g/`** (#404, 2026-09-11).
    // Both were hints pointing at where the thing had moved to, which
    // is the right answer for a key we deliberately spell differently
    // — but these two are not spelled differently, they are the *same*
    // action under the name vi and Helix both give it. A hint that
    // could have just done it is a hint that costs a keystroke and
    // teaches nothing.
    //
    // Warning: `~` maps to the group's **third** member, not to the group:
    // `` ` `` opens 「小寫/大寫/互換」, and `~` has always meant the
    // last of those on its own.
    #[allow(unused_variables)]
    pub(super) fn cmd_switch_case(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        // vim's Visual `~`: the whole selection, then Normal mode.
        if self.as_vim() && self.extend && !self.expanding_alias {
            return self.vim_visual_key('~');
        }
        // vim's `3~` switches three characters (`:h ~`).
        if self.as_vim() && !self.extend && count > 1 {
            let rope = self.current_buffer().rope();
            let last = motion::line_last(rope, self.sel.head());
            let mut head = self.sel.head();
            for _ in 1..count {
                let next = motion::next_grapheme(rope, head);
                if next > last {
                    break;
                }
                head = next;
            }
            // Head at the start, so `u` lands there (nvim: `w3~u`).
            let start = self.sel.head();
            self.sel.set_anchor(head);
            self.sel.set_head(start);
        }
        self.map_selection(switch_case);
        // **vim 的 `~` 按完往前走一格**（`:h ~`），所以 `~~~` 在 `alpha`
        // 上給 `ALPha`；從前這裏三下都換同一個字，給的是 `Alpha`。
        // helix 的 `~` 不走（它作用在整個選區上），兩邊各跟各的。
        if self.as_vim() && !self.extend {
            let rope = self.current_buffer().rope();
            let at = self.selection().1.min(motion::line_end(rope, self.sel.head()));
            self.sel.set_head(at);
            self.sel.set_anchor(at);
            self.clamp_cursor();
        }
    }

    // **`*` 只記下要找什麼，光標不動**（2026-10-06，對齊 helix）。
    //
    // Warning: **§5.80「不改的兩條」把這一格寫成了「和真 helix 一致」，那句記錄
    // 是錯的**（2026-10-06 夜審查出來的）。真 helix 的
    // `search_selection_impl`（`helix-term/src/commands.rs:2463`）唯一的副
    // 作用是 `registers.push`——它不移動，也不開命中列表。我們從前跳到下
    // 一處，那是 vim 的 `*`。
    //
    // `*` 補 `\b`，`A-*` 不補——helix 那兩個鍵正是這麼分的
    // （`search_selection_detect_word_boundaries` 與 `search_selection`）。
    // vim 鍵位下 `*` 另有去處（鍵位表映到 `g/`，那個照舊跳），所以這一條
    // 只動 helix 那一端。
    #[allow(unused_variables)]
    pub(super) fn cmd_search_selection_detect_word_boundaries(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.remember_the_search(true);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_search_selection_exact(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.remember_the_search(false);
    }

    // **vim 的 `#`：往回找光標下這個詞**（2026-10-05 定）。
    //
    // Warning: **只在 vim 鍵位下，而且不走鍵位表。** 那張表是「鍵 → 一串
    // 鍵」，而往回找在 helix 那一套上没有鍵（`g/` 往前、`g?` 是在副編輯區
    // 給你看，没有一支是往回），所以表裏没有東西可以映——這一條直接寫在
    // 鍵上，同 `d`/`c`/`y` 那幾個操作符。helix 鍵位下不給鍵，要的人在配
    // 置裏繫得上。
    #[allow(unused_variables)]
    pub(super) fn cmd_search_word_backward(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.definition_preview = false;
        self.search_the_page(true);
    }

    // Replacing the selection with the register. Joining is on `gJ`:
    // `J` turns the page, which a reader presses a hundred times for
    // every once they join two lines.
    // **vim 的 `R` 是覆寫模式**（2026-10-06 定：「做，只在 vim 鍵位下」）。
    // 進去之後打一個字蓋一個字，`Esc` 出來；退格把蓋掉的還回去。helix 没有
    // 這個模式，那一端 `R` 照舊是「用寄存器換掉選區」。
    #[allow(unused_variables)]
    pub(super) fn cmd_replace_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.snapshot();
        self.enter_insert();
        self.overwriting = true;
        // `2RXY<Esc>` writes `XY` twice, over the text (nvim: `XYXYef`).
        self.insert_again = count.clamp(1, Self::WRITING_MAX) - 1;
    }

    // `2R`: two copies, as helix does it.
    #[allow(unused_variables)]
    pub(super) fn cmd_replace_with_yanked(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.paste_copies = count.clamp(1, Self::WRITING_MAX);
        self.replace_with_register();
    }

    // Search for whatever is selected (Helix `*`).
    // **`30G` goes to line 30 under either preset.** A bare `G` is the
    // vim preset's alone: there it is the last line, which is what `G`
    // means in vi. **Under the helix preset a bare `G` does nothing**,
    // because that is what helix does — `goto_line_impl` opens with
    // `if cx.count.is_some()` (`helix-term/src/commands.rs`), and
    // measured on helix 25.07.1 (`scripts/oracle/hx_keys.py`) `jG`
    // leaves the cursor on line 2, it does not go to line 1.
    //
    // Warning: **this used to go to the last line under both presets**, on the
    // reasoning that going somewhere useful beats going nowhere — and
    // the manual was written to say so, which made the manual no
    // evidence at all. 2026-10-13: 「是缺陷，因为这是你一个人自说自话的
    // 决定。手册也是你写的。手册从来没说过这是『设计的偏离』。请你和
    // helix 对齐。」 A divergence from the reference is the author's to
    // make, not something a comment can grant itself.
    #[allow(unused_variables)]
    pub(super) fn cmd_goto_line(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.fold_stays_closed = true;
        // `operator_count`, not `self.count`: the count was taken at
        // the top of this function, so asking `self.count` here always
        // said "no digits" and `1G` went to the *last* line.
        match operator_count.is_some() {
            true => {
                self.remember_jump();
                // 全窗裏數的是表格的行，見 `goto_row`。
                self.goto_grid_row(count);
            }
            false if self.as_vim() => {
                self.remember_jump();
                // **Inside a cell the cell is the document**, so its
                // far end is what this means there (設計 §03 的「格尾」,
                // 2026-10-22). Two ways it was wrong: the buffer's end
                // is outside the cell, and this arm moves the head
                // itself rather than going through `run_motion`, so the
                // clamp in `Editor::walled` — which is what gives
                // helix's `ge` the same answer — never saw it and `G`
                // walked onto the record below. And where the buffer's
                // last *line* happens to be a line of this very cell,
                // clamping was not enough either: the answer was the
                // start of the cell's last written line, not its end.
                let at = match self.cell_far_wall() {
                    Some(at) => at,
                    None => {
                        let rope = self.current_buffer().rope();
                        motion::buffer_end(rope, self.sel.head())
                    }
                };
                self.move_head(at);
            }
            // helix 鍵位：不帶數的 `G` 不動——`ge` 纔是末行。
            false => {}
        }
    }

    // Indent / unindent the selected lines.
    #[allow(unused_variables)]
    pub(super) fn cmd_indent_or_unindent(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        if let Key::Char(c @ ('>' | '<')) = key {
            if self.as_vim() && self.extend && !self.expanding_alias {
                return self.vim_visual_lines(key, c);
            }
        }
        let Key::Char(c) = key else { return };
        // vim's `>>` keeps the cursor's column (nvim, `nostartofline`);
        // helix's `>` keeps the selection on its text.
        let kept = (self.as_vim()).then(|| {
            let rope = self.current_buffer().rope();
            let head = self.sel.head();
            let line = rope.char_to_line(head);
            (line, head - rope.line_to_char(line))
        });
        self.repeat_writing(count, |e| e.indent(c == '>'));
        if let Some((line, column)) = kept {
            let to = self.column_on_line(line, column);
            self.sel.set_anchor(to);
            self.sel.set_head(to);
            self.refresh_goal_column();
        }
    }

    // Increment / decrement the number at the cursor. **The count is
    // the amount**, as in helix and vim (2026-10-29, review: repeating
    // stopped after one pass whenever the number kept its width, so
    // `3 C-a` on `7` gave `8` and on a time stepped one minute).
    #[allow(unused_variables)]
    pub(super) fn cmd_increment(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.bump_number(count as i64);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_decrement(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.bump_number(-(count as i64));
    }

    // Repeat the last insert, and the last `f`/`t`.
    // **vim's `3.` replaces the count the change was made with**
    // (`:h .`, nvim: `rX` then `3.` writes three), and the next bare
    // `.` keeps the new one. Repeating the whole change three times
    // wrote `X` over the same character three times.
    #[allow(unused_variables)]
    pub(super) fn cmd_vim_repeat_last_change(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        // vim's `.`: a count replaces the one the change was made with.
        if operator_count.is_none() {
            return self.repeat_writing(count, |e| e.repeat_edit());
        }
        self.recount_the_last_edit(count.min(Self::WRITING_MAX));
        self.repeat_edit();
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_repeat_last_change(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.repeat_writing(count, |e| e.repeat_edit());
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_repeat_last_motion(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        if let Some((kind, c)) = self.last_find {
            self.find_nth_char(kind, c, count, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The commands the vim preset binds in place of helix's on the same key.
    const VIM_ONLY: &[&str] = &[
        "move_line_up", "move_line_down", "goto_next_line_nonwhitespace",
        "jump_next_word_start", "jump_next_long_word_start", "jump_prev_word_start",
        "jump_prev_long_word_start", "jump_next_word_end", "jump_next_long_word_end",
        "jump_next_paragraph", "jump_prev_paragraph", "goto_window_top", "goto_window_center",
        "goto_window_bottom", "jump_prev_sentence", "jump_next_sentence", "select_line_mode",
        "repeat_find_char", "repeat_find_char_reverse", "operator_delete", "operator_change",
        "operator_yank", "operator_indent", "operator_unindent",
        "select_inner_object", "select_around_object", "undo_line", "goto_column", "vim_redo",
        "delete_char_backward", "search_word_backward", "replace_mode", "vim_repeat_last_change",
    ];

    #[test]
    fn every_normal_command_has_one_name() {
        let mut seen = std::collections::HashSet::new();
        for (name, _, _) in NORMAL_COMMANDS {
            assert!(seen.insert(*name), "{name} twice");
            assert!(name.name().chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "{name}");
        }
        for name in VIM_ONLY {
            assert!(seen.iter().any(|one| one.name() == *name), "{name} is not a command");
        }
    }

    /// **Every Normal action is a command, and every command a bindable
    /// name** (2026-10-31): the config reads its names from
    /// `yumete_cjk::actions`, and the code is here; the two lists say the
    /// same names.
    #[test]
    fn the_bindable_names_and_the_commands_agree() {
        let normal: Vec<&str> = yumete_cjk::actions::ALL
            .iter()
            .filter(|a| a.how == yumete_cjk::actions::How::Normal)
            .map(|a| a.name)
            .collect();
        for name in &normal {
            assert!(normal_command(name).is_some(), "the action {name} has no command");
        }
        // Every key in either table names a command.
        for (key, name) in HELIX_NORMAL {
            assert!(normal_command(name).is_some(), "{key:?} names {name}, which is no command");
        }
        for (key, name, _) in VIM_NORMAL {
            assert!(normal_command(name).is_some(), "{key:?} names {name}, which is no command");
        }
    }

    /// **Every name written anywhere a command is looked up is a command**
    /// (2026-11-01, the review's guard rails): a misspelt name in these
    /// tables compiled and then did nothing, because each lookup ends in an
    /// `if let` or a `_ => {}`.
    #[test]
    fn every_name_a_table_holds_is_a_command() {
        for (key, name) in SELECT_NORMAL {
            assert!(normal_command(name).is_some(), "select mode's {key:?} names {name}, which is no command");
        }
        for (key, name, _) in super::super::insert::INSERT_KEYS {
            assert!(normal_command(name).is_some(), "Insert's {key:?} names {name}, which is no command");
        }
        for (name, _) in super::super::keys::MOTION_STEPS {
            assert!(normal_command(name).is_some(), "MOTION_STEPS names {name}, which is no command");
        }
        // The commands an `extend_*` runs are `NormalCmd` variants: the
        // compiler checks those names, and `normal_cmd` finds each.
        for cmd in yumete_cjk::actions::NormalCmd::ALL {
            assert!(normal_cmd(*cmd).is_some(), "{cmd} is declared and has no method");
        }
    }

    /// **Every command is a bindable name**, so the config, the palette and
    /// the generated docs know it (the other half of
    /// `the_bindable_names_and_the_commands_agree`).
    #[test]
    fn every_command_is_a_bindable_name() {
        let tables = [NORMAL_COMMANDS, super::super::group_commands::GROUP_COMMANDS, EXTEND_COMMANDS, super::super::insert::INSERT_COMMANDS];
        for (name, _, _) in tables.iter().flat_map(|t| t.iter()) {
            assert!(yumete_cjk::actions::action(name.name()).is_some(), "the command {name} cannot be bound");
        }
    }
}
