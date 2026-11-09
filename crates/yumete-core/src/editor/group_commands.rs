//! **The commands the key groups name** (step 3 of the keymap plan): what `g h`, `Space f`, `]d`, `C-w s` and the rest do, each under
//! its own name, so a binding can reach any of them without its group.
//!
//! Each is written as the body it runs, in the table, beside its name:
//! most are one call. A command reads its count from its [`Cx`], never from
//! the key that reached it, so it does the same thing whatever key a
//! configuration puts it on. The rows that say which key of which group
//! runs which command are in `groups.rs`.

use super::normal::{Cx, NormalCommand};
use super::*;
use crate::editor::Sideways;
use yumete_cjk::actions::NormalCmd;
use crate::input::Key;
use crate::motion;

/// **Every command a group's key runs**, with the key it answers to in its
/// group by default.
pub(super) const GROUP_COMMANDS: &[NormalCommand] = &[
    // ── g ──────────────────────────────────────────────────────────────
    (NormalCmd::GotoFileStart, |e, cx| {
        // `g30g` — the group's own count — and `30gg`, vi's order.
        let line = e.sequence_span().map(|(n, _)| n).or(cx.operator_count);
        e.operator_count = None;
        // vim's jumps leave a closed fold closed (its `foldopen` has no
        // `jump`, measured in nvim).
        e.fold_stays_closed = true;
        // vim's `gg` keeps the column, as `G` does (`line_jump_keeps`).
        let keep = e.vim_keymap().then(|| e.line_jump_keeps()).flatten();
        match line.filter(|&n| n > 0) {
            Some(n) => e.goto_grid_row(n),
            None => {
                e.remember_jump();
                e.goto_motion(motion::Motion::FileStart);
            }
        }
        if let Some(want) = keep {
            e.land_at_column(e.cursor_line(), want);
        }
    }, Key::Char('g')),
    (NormalCmd::GotoFileEnd, |e, _| {
        e.fold_stays_closed = true;
        e.remember_jump();
        e.goto_motion(motion::Motion::FileEnd);
    }, Key::Char('e')),
    // `ge` and then `gh`: helix's `ge`, which this editor gives to the end
    // of the file.
    (NormalCmd::GotoLastLine, |e, _| {
        e.fold_stays_closed = true;
        e.remember_jump();
        e.goto_motion(motion::Motion::FileEnd);
        e.goto_motion(motion::Motion::LineStart);
    }, Key::Char('e')),
    // vim's `ge` / `gE`: back to a word's end.
    (NormalCmd::JumpPrevWordEnd, |e, cx| e.jump_back_to_word_end(cx, false), Key::Char('e')),
    (NormalCmd::JumpPrevLongWordEnd, |e, cx| e.jump_back_to_word_end(cx, true), Key::Char('E')),
    (NormalCmd::GotoFirstNonwhitespace, |e, _| e.goto_motion(motion::Motion::LineFirstNonBlank), Key::Char('s')),
    (NormalCmd::GotoFile, |e, _| e.goto_file_under_cursor(), Key::Char('f')),
    (NormalCmd::FollowLink, |e, _| e.follow_link(), Key::Char('L')),
    (NormalCmd::OpenLink, |e, _| e.open_link(), Key::Char('x')),
    (NormalCmd::GotoDeclaration, |e, _| e.ask_for_place(crate::lsp::Goto::Declaration), Key::Char('D')),
    (NormalCmd::GotoTypeDefinition, |e, _| e.ask_for_place(crate::lsp::Goto::TypeDefinition), Key::Char('y')),
    (NormalCmd::GotoImplementation, |e, _| e.ask_for_place(crate::lsp::Goto::Implementation), Key::Char('i')),
    (NormalCmd::GotoLastModifiedFile, |e, _| e.goto_last_modified_file(), Key::Char('m')),
    (NormalCmd::GotoWord, |e, _| e.start_jump(), Key::Char('w')),
    (NormalCmd::GotoWordByLetters, |e, _| e.start_seek(false), Key::Char('x')),
    (NormalCmd::SplitJoin, |e, _| e.edit_each(|e| e.split_join_here()), Key::Char('S')),
    (NormalCmd::GotoCharByPinyin, |e, _| e.start_seek(true), Key::Char('z')),
    (NormalCmd::GotoLastAccessedFile, |e, _| e.goto_last_file(), Key::Char('a')),
    (NormalCmd::GotoNextBuffer, |e, _| e.next_buffer(), Key::Char('n')),
    (NormalCmd::GotoPreviousBuffer, |e, _| e.prev_buffer(), Key::Char('p')),
    (NormalCmd::GotoDefinition, |e, _| {
        let was = (e.current_buffer().id(), e.sel.head());
        e.show_definition();
        // helix's `gd` centres where it lands, on the page or off it; a
        // language server's answer comes later (`go_to_definition`).
        if (e.current_buffer().id(), e.sel.head()) != was {
            e.centre_here();
        }
    }, Key::Char('d')),
    (NormalCmd::GotoReference, |e, _| e.ask_references(), Key::Char('r')),
    (NormalCmd::SearchSelection, |e, _| {
        e.definition_preview = false;
        e.search_the_page(false)
    }, Key::Char('/')),
    // A vertical page under the vim preset: `gj` walks the 縱 the way the
    // writing runs, one line of the file at a time.
    (NormalCmd::MoveNextZong, |e, cx| e.walk_zongs(cx, true), Key::Char('j')),
    (NormalCmd::MovePrevZong, |e, cx| e.walk_zongs(cx, false), Key::Char('k')),
    (NormalCmd::GotoLastModification, |e, _| e.goto_last_modification(), Key::Char('.')),
    (NormalCmd::ReselectLastSelection, |e, _| {
        let Some((anchor, head)) = e.last_selection else {
            e.status = say!("selection.nothing-to-reselect");
            return;
        };
        let len = e.current_buffer().rope().len_chars();
        e.sel.set_anchor(anchor.min(len));
        e.sel.set_head(head.min(len));
        e.extend = true;
        e.clamp_cursor();
        e.refresh_goal_column();
    }, Key::Char('v')),
    (NormalCmd::InsertAtLastInsert, |e, _| {
        if let Some(at) = e.current_buffer().insert_stop() {
            e.set_cursor(at);
        }
        e.enter_insert();
    }, Key::Char('i')),
    (NormalCmd::GotoOlderChange, |e, _| e.walk_changes(true), Key::Char(';')),
    (NormalCmd::GotoNewerChange, |e, _| e.walk_changes(false), Key::Char(',')),
    // vim's `g` operators, each waiting for a motion.
    (NormalCmd::OperatorLowercase, |e, _| e.g_operator('u'), Key::Char('u')),
    (NormalCmd::OperatorUppercase, |e, _| e.g_operator('U'), Key::Char('U')),
    (NormalCmd::OperatorSwitchCase, |e, _| e.g_operator('~'), Key::Char('~')),
    // `gc` is `#` inside: the operator's own letter is taken by `c`.
    (NormalCmd::OperatorComment, |e, _| e.g_operator('#'), Key::Char('c')),
    (NormalCmd::OperatorFill, |e, _| e.g_operator('q'), Key::Char('q')),
    (NormalCmd::OperatorFillKeepCursor, |e, _| e.g_operator('w'), Key::Char('w')),
    (NormalCmd::JoinSelectionsNoSpace, |e, cx| {
        // A group's key is not seen by `before_a_motion`: on a closed fold,
        // vim's `gJ` joins the line the cursor is hidden on, as `J` does.
        if e.vim_keymap() && !e.extend {
            e.stand_where_hidden(true);
        }
        let n = match e.extend {
            true => {
                let rope = e.current_buffer().rope();
                let (start, end) = e.selection();
                let lines = rope.char_to_line(end.saturating_sub(1).max(start)) - rope.char_to_line(start);
                let seat = rope.line_to_char(rope.char_to_line(start));
                e.extend = false;
                e.vim_lines = false;
                e.sel.set_anchor(seat);
                e.sel.set_head(seat);
                lines.max(1)
            }
            false => cx.count,
        };
        e.repeat_writing(n, |e| e.join_lines_raw());
    }, Key::Char('J')),
    // ── Space ──────────────────────────────────────────────────────────
    (NormalCmd::FilePicker, |e, _| e.open_file_picker(), Key::Char('f')),
    (NormalCmd::FilePickerInCurrentDirectory, |e, _| e.open_file_picker_here(), Key::Char('F')),
    (NormalCmd::FileExplorer, |e, _| e.open_explorer(), Key::Char('e')),
    (NormalCmd::FileExplorerInCurrentBufferDirectory, |e, _| e.open_explorer_here(), Key::Char('E')),
    (NormalCmd::BufferPicker, |e, _| e.open_buffer_picker(), Key::Char('b')),
    (NormalCmd::ChangedFilePicker, |e, _| e.open_changed_picker(), Key::Char('g')),
    (NormalCmd::YankToClipboard, |e, _| e.copy_to_clipboard(true), Key::Char('y')),
    (NormalCmd::YankMainSelectionToClipboard, |e, _| e.copy_to_clipboard(false), Key::Char('Y')),
    (NormalCmd::JumplistPicker, |e, _| e.open_jumplist_picker(), Key::Char('j')),
    (NormalCmd::SelectReferencesToSymbolUnderCursor, |e, _| e.ask_highlights(), Key::Char('h')),
    (NormalCmd::LastPicker, |e, _| e.reopen_last_picker(), Key::Char('\'')),
    (NormalCmd::PasteClipboardAfter, |e, _| e.clipboard_paste(true), Key::Char('p')),
    (NormalCmd::PasteClipboardBefore, |e, _| e.clipboard_paste(false), Key::Char('P')),
    (NormalCmd::ReplaceSelectionsWithClipboard, |e, _| e.clipboard_replace(), Key::Char('R')),
    (NormalCmd::LspOrSyntaxSymbolPicker, |e, _| e.open_symbol_picker(), Key::Char('s')),
    (NormalCmd::LspOrSyntaxWorkspaceSymbolPicker, |e, _| e.open_project_symbol_picker(), Key::Char('S')),
    (NormalCmd::Dictionary, |e, _| e.look_up_under_the_cursor(true), Key::Char('n')),
    (NormalCmd::DictionaryPanel, |e, _| e.look_up_under_the_cursor(false), Key::Char('N')),
    (NormalCmd::Hover, |e, _| {
        if !e.ask_what_this_is(true) {
            e.set_status(say!("lsp.not-code"));
        }
    }, Key::Char('k')),
    (NormalCmd::HoverPanel, |e, _| {
        if !e.ask_what_this_is(false) {
            e.set_status(say!("lsp.not-code"));
        }
    }, Key::Char('K')),
    (NormalCmd::DiagnosticsPicker, |e, _| e.open_diagnostics_picker(false), Key::Char('d')),
    (NormalCmd::WorkspaceDiagnosticsPicker, |e, _| e.open_diagnostics_picker(true), Key::Char('D')),
    (NormalCmd::ShowDiagnostic, |e, _| e.show_the_problem_here(true), Key::Char('l')),
    (NormalCmd::ShowDiagnosticPanel, |e, _| e.show_the_problem_here(false), Key::Char('L')),
    (NormalCmd::ShowChange, |e, _| e.show_the_change_here(true), Key::Char('x')),
    (NormalCmd::ShowChangePanel, |e, _| e.show_the_change_here(false), Key::Char('X')),
    (NormalCmd::RenameSymbol, |e, _| e.start_a_rename(), Key::Char('r')),
    (NormalCmd::CodeAction, |e, _| e.ask_actions(), Key::Char('a')),
    (NormalCmd::ShowRecord, |e, _| e.show_the_record_here(true), Key::Char('i')),
    (NormalCmd::ShowRecordPanel, |e, _| e.show_the_record_here(false), Key::Char('I')),
    (NormalCmd::TableMode, |e, _| e.pending = Pending::Table, Key::Char('t')),
    (NormalCmd::PasteMenu, |e, _| e.open_paste_picker(), Key::Char('"')),
    (NormalCmd::ToggleComments, |e, _| e.toggle_comment(super::edits::CommentKey::Toggle), Key::Char('c')),
    (NormalCmd::ToggleLineComments, |e, _| e.toggle_comment(super::edits::CommentKey::Lines), Key::Alt('c')),
    (NormalCmd::ToggleBlockComments, |e, _| e.toggle_comment(super::edits::CommentKey::Block), Key::Char('C')),
    (NormalCmd::ConflictMode, |e, _| e.pending = Pending::Conflict, Key::Char('m')),
    (NormalCmd::OpenRecentMode, |e, _| e.pending = Pending::Open, Key::Char('o')),
    (NormalCmd::YumeMode, |e, _| e.pending = Pending::Yume, Key::Char('z')),
    (NormalCmd::GlobalSearch, |e, _| e.open_search(), Key::Char('/')),
    // helix's `command_palette`: every command and every key's command, each
    // with its key and what it does, searched by description (`::`), not an
    // empty `:` line.
    (NormalCmd::CommandPalette, |e, _| {
        e.open_the_command_line("");
        e.mode = Mode::Lookfor;
        e.lookfor_focus = 0;
    }, Key::Char('?')),
    // ── ] and [ ────────────────────────────────────────────────────────
    (NormalCmd::GotoNextConflict, |e, _| e.go_to_conflict(true), Key::Char('m')),
    (NormalCmd::GotoPrevConflict, |e, _| e.go_to_conflict(false), Key::Char('m')),
    (NormalCmd::GotoNextChange, |e, _| e.go_to_change(true), Key::Char('g')),
    (NormalCmd::GotoPrevChange, |e, _| e.go_to_change(false), Key::Char('g')),
    (NormalCmd::GotoLastChange, |e, _| e.go_to_change_at_the_end(true), Key::Char('G')),
    (NormalCmd::GotoFirstChange, |e, _| e.go_to_change_at_the_end(false), Key::Char('G')),
    (NormalCmd::GotoNextDiag, |e, _| e.go_to_problem(true, false), Key::Char('d')),
    (NormalCmd::GotoPrevDiag, |e, _| e.go_to_problem(false, false), Key::Char('d')),
    (NormalCmd::GotoLastDiag, |e, _| e.go_to_problem(true, true), Key::Char('D')),
    (NormalCmd::GotoFirstDiag, |e, _| e.go_to_problem(false, true), Key::Char('D')),
    (NormalCmd::GotoNextFunction, |e, cx| e.go_to_definition_nearby(true, true, cx.count), Key::Char('f')),
    (NormalCmd::GotoPrevFunction, |e, cx| e.go_to_definition_nearby(false, true, cx.count), Key::Char('f')),
    (NormalCmd::GotoNextClass, |e, cx| e.go_to_definition_nearby(true, false, cx.count), Key::Char('t')),
    (NormalCmd::GotoPrevClass, |e, cx| e.go_to_definition_nearby(false, false, cx.count), Key::Char('t')),
    (NormalCmd::GotoNextComment, |e, cx| e.go_to_object_nearby(true, crate::code::Object::Comment, cx.count), Key::Char('c')),
    (NormalCmd::GotoPrevComment, |e, cx| e.go_to_object_nearby(false, crate::code::Object::Comment, cx.count), Key::Char('c')),
    (NormalCmd::GotoNextParameter, |e, cx| e.go_to_object_nearby(true, crate::code::Object::Parameter, cx.count), Key::Char('a')),
    (NormalCmd::GotoPrevParameter, |e, cx| e.go_to_object_nearby(false, crate::code::Object::Parameter, cx.count), Key::Char('a')),
    (NormalCmd::GotoNextEntry, |e, cx| e.go_to_object_nearby(true, crate::code::Object::Entry, cx.count), Key::Char('e')),
    (NormalCmd::GotoPrevEntry, |e, cx| e.go_to_object_nearby(false, crate::code::Object::Entry, cx.count), Key::Char('e')),
    (NormalCmd::GotoNextTest, |e, cx| e.go_to_object_nearby(true, crate::code::Object::Test, cx.count), Key::Char('T')),
    (NormalCmd::GotoPrevTest, |e, cx| e.go_to_object_nearby(false, crate::code::Object::Test, cx.count), Key::Char('T')),
    (NormalCmd::GotoNextXmlElement, |e, cx| e.go_to_object_nearby(true, crate::code::Object::Element, cx.count), Key::Char('x')),
    (NormalCmd::GotoPrevXmlElement, |e, cx| e.go_to_object_nearby(false, crate::code::Object::Element, cx.count), Key::Char('x')),
    (NormalCmd::GotoNextParagraph, |e, _| {
        let span = e.run_motion(motion::Motion::Paragraph { forward: true });
        e.take_span(span);
    }, Key::Char('p')),
    (NormalCmd::GotoPrevParagraph, |e, _| {
        let span = e.run_motion(motion::Motion::Paragraph { forward: false });
        e.take_span(span);
    }, Key::Char('p')),
    (NormalCmd::AddNewlineBelow, |e, cx| {
        e.operator_count = None;
        e.add_blank_line(true, cx.count);
    }, Key::Char(' ')),
    (NormalCmd::AddNewlineAbove, |e, cx| {
        e.operator_count = None;
        e.add_blank_line(false, cx.count);
    }, Key::Char(' ')),
    // ── z and Z ────────────────────────────────────────────────────────
    (NormalCmd::AlignViewCenter, |e, cx| {
        e.vim_aim_at_line(cx.operator_count);
        e.aim_the_page(crate::editor::Aim::Middle);
    }, Key::Char('z')),
    (NormalCmd::AlignViewTop, |e, cx| {
        e.vim_aim_at_line(cx.operator_count);
        e.aim_the_page(crate::editor::Aim::Top);
    }, Key::Char('t')),
    (NormalCmd::AlignViewBottom, |e, cx| {
        e.vim_aim_at_line(cx.operator_count);
        e.aim_the_page(crate::editor::Aim::Bottom);
    }, Key::Char('b')),
    (NormalCmd::ScrollDown, |e, cx| e.scroll_the_page(true, cx.count), Key::Char('j')),
    (NormalCmd::ScrollUp, |e, cx| e.scroll_the_page(false, cx.count), Key::Char('k')),
    // ── m ──────────────────────────────────────────────────────────────
    (NormalCmd::MatchBrackets, |e, _| e.acting(false, |e| e.goto_matching_bracket()), Key::Char('m')),
    // vim's `%`: off a bracket, the next one along the line.
    // With a count, `{count}%` goes that far down the file (`:h N%`).
    (NormalCmd::VimMatchBrackets, |e, cx| match cx.operator_count {
        Some(n) => e.goto_percent(n, cx),
        None => e.acting(true, |e| e.goto_matching_bracket()),
    }, Key::Char('m')),
    // These four wait for a pair or an object, and keep the count for it
    // (`2mi(` is the second pair out).
    (NormalCmd::SelectTextobjectInner, |e, cx| {
        e.pending = Pending::MatchPair { around: false };
        e.operator_count = cx.operator_count;
    }, Key::Char('i')),
    (NormalCmd::SelectTextobjectAround, |e, cx| {
        e.pending = Pending::MatchPair { around: true };
        e.operator_count = cx.operator_count;
    }, Key::Char('a')),
    (NormalCmd::SurroundAdd, |e, _| e.pending = Pending::Surround, Key::Char('s')),
    (NormalCmd::SurroundDelete, |e, cx| {
        e.pending = Pending::SurroundOff;
        e.operator_count = cx.operator_count;
    }, Key::Char('d')),
    (NormalCmd::SurroundReplace, |e, cx| {
        e.pending = Pending::SurroundFrom;
        e.operator_count = cx.operator_count;
    }, Key::Char('r')),
    // ── z: vim's folds ────────────────────────────────────────────────
    // vim's an operator, helix's the selection's lines (the flavour's `zf`).
    (NormalCmd::CreateFold, |e, _| match e.vim_keymap() {
        true => e.fold_operator(),
        false => e.fold_selection(),
    }, Key::Char('f')),
    (NormalCmd::CreateFoldLines, |e, cx| { let line = e.cursor_line(); e.fold_lines(line, line + cx.count.max(1) - 1) }, Key::Char('F')),
    (NormalCmd::OpenFold, |e, cx| e.fold_open(cx.count, false), Key::Char('o')),
    (NormalCmd::OpenFoldsRecursively, |e, cx| e.fold_open(cx.count, true), Key::Char('O')),
    (NormalCmd::CloseFold, |e, cx| e.fold_close(cx.count, false), Key::Char('c')),
    (NormalCmd::CloseFoldsRecursively, |e, cx| e.fold_close(cx.count, true), Key::Char('C')),
    (NormalCmd::ToggleFold, |e, cx| e.fold_toggle(cx.count, false), Key::Char('a')),
    (NormalCmd::ToggleFoldsRecursively, |e, cx| e.fold_toggle(cx.count, true), Key::Char('A')),
    (NormalCmd::OpenFoldsToCursor, |e, _| e.fold_view_cursor(), Key::Char('v')),
    (NormalCmd::ReapplyFoldLevelAndView, |e, _| e.reapply_fold_level(true), Key::Char('x')),
    (NormalCmd::ReapplyFoldLevel, |e, _| e.reapply_fold_level(false), Key::Char('X')),
    (NormalCmd::VimAlignViewTop, |e, cx| e.vim_aim_past_indent(crate::editor::Aim::Top, cx.operator_count), Key::Enter),
    (NormalCmd::VimAlignViewCenter, |e, cx| e.vim_aim_past_indent(crate::editor::Aim::Middle, cx.operator_count), Key::Char('.')),
    (NormalCmd::VimAlignViewBottom, |e, cx| e.vim_aim_past_indent(crate::editor::Aim::Bottom, cx.operator_count), Key::Char('-')),
    (NormalCmd::VimAlignViewBelowTop, |e, cx| e.vim_turn_the_page(true, cx.operator_count), Key::Char('+')),
    (NormalCmd::VimAlignViewAboveBottom, |e, cx| e.vim_turn_the_page(false, cx.operator_count), Key::Char('^')),
    (NormalCmd::ScrollViewLeft, |e, cx| e.scroll_sideways(Sideways::Shift(-(cx.count as isize))), Key::Char('h')),
    (NormalCmd::ScrollViewRight, |e, cx| e.scroll_sideways(Sideways::Shift(cx.count as isize)), Key::Char('l')),
    (NormalCmd::ScrollViewHalfLeft, |e, cx| {
        // Half the window, its gutter counted, as vim's (measured in nvim).
        let half = ((e.side_seen.get().1 + e.side_gutter.get()) / 2).max(1);
        e.scroll_sideways(Sideways::Shift(-((cx.count * half) as isize)));
    }, Key::Char('H')),
    (NormalCmd::ScrollViewHalfRight, |e, cx| {
        let half = ((e.side_seen.get().1 + e.side_gutter.get()) / 2).max(1);
        e.scroll_sideways(Sideways::Shift((cx.count * half) as isize));
    }, Key::Char('L')),
    (NormalCmd::AlignViewCursorLeft, |e, _| e.scroll_sideways(Sideways::CursorAtLeft), Key::Char('s')),
    (NormalCmd::AlignViewCursorRight, |e, _| e.scroll_sideways(Sideways::CursorAtRight), Key::Char('e')),
    (NormalCmd::AlignViewMiddle, |e, _| e.scroll_sideways(Sideways::Center), Key::Char('m')),
    (NormalCmd::OpenAllFolds, |e, _| e.fold_all(false), Key::Char('R')),
    (NormalCmd::CloseAllFolds, |e, _| e.fold_all(true), Key::Char('M')),
    (NormalCmd::DeleteFold, |e, _| e.fold_delete(super::folds::FoldDelete::Innermost), Key::Char('d')),
    (NormalCmd::DeleteFoldsRecursively, |e, _| e.fold_delete(super::folds::FoldDelete::Here), Key::Char('D')),
    (NormalCmd::DeleteAllFolds, |e, _| e.fold_delete(super::folds::FoldDelete::All), Key::Char('E')),
    (NormalCmd::GotoNextFold, |e, cx| e.fold_step(true, cx.count), Key::Char('j')),
    (NormalCmd::GotoPrevFold, |e, cx| e.fold_step(false, cx.count), Key::Char('k')),
    (NormalCmd::DisableFolds, |e, _| e.fold_enable(Some(false)), Key::Char('n')),
    (NormalCmd::EnableFolds, |e, _| e.fold_enable(Some(true)), Key::Char('N')),
    (NormalCmd::ToggleFoldsEnabled, |e, _| e.fold_enable(None), Key::Char('i')),
    (NormalCmd::FoldMore, |e, cx| e.fold_level(false, cx.count), Key::Char('m')),
    (NormalCmd::FoldLess, |e, cx| e.fold_level(true, cx.count), Key::Char('r')),
    (NormalCmd::GotoFoldStart, |e, _| e.fold_edge(false), Key::Char('z')),
    (NormalCmd::GotoFoldEnd, |e, _| e.fold_edge(true), Key::Char('z')),
    // ── C-w ────────────────────────────────────────────────────────────
    // vim's `C-w w` `W` with a count go to that window; helix's take none.
    (NormalCmd::RotateView, |e, cx| match cx.operator_count.filter(|_| e.vim_keymap()) {
        Some(n) => e.go_to_nth_window(n),
        None => e.next_region(),
    }, Key::Char('w')),
    (NormalCmd::RotateViewReverse, |e, cx| match cx.operator_count.filter(|_| e.vim_keymap()) {
        Some(n) => e.go_to_nth_window(n),
        None => e.previous_region(),
    }, Key::Char('W')),
    (NormalCmd::JumpViewFirst, |e, _| e.go_to_nth_window(1), Key::Char('t')),
    (NormalCmd::JumpViewLast, |e, _| e.go_to_nth_window(usize::MAX), Key::Char('b')),
    (NormalCmd::JumpViewLastAccessed, |e, _| e.go_to_last_accessed_window(), Key::Char('p')),
    (NormalCmd::ExchangeView, |e, cx| e.exchange_window(cx.operator_count), Key::Char('x')),
    (NormalCmd::RotateViewsForward, |e, _| e.rotate_windows(true), Key::Char('r')),
    (NormalCmd::RotateViewsBackward, |e, _| e.rotate_windows(false), Key::Char('R')),
    (NormalCmd::MoveViewFarLeft, |e, _| e.move_window_to_edge(crate::window::Direction::Left), Key::Char('H')),
    (NormalCmd::MoveViewFarDown, |e, _| e.move_window_to_edge(crate::window::Direction::Down), Key::Char('J')),
    (NormalCmd::MoveViewFarUp, |e, _| e.move_window_to_edge(crate::window::Direction::Up), Key::Char('K')),
    (NormalCmd::MoveViewFarRight, |e, _| e.move_window_to_edge(crate::window::Direction::Right), Key::Char('L')),
    (NormalCmd::VimCloseWindow, |e, cx| e.close_region_numbered(cx.operator_count), Key::Char('c')),
    (NormalCmd::GotoLastAccessedFileHsplit, |e, _| e.goto_last_file_in_split(), Key::Char('^')),
    (NormalCmd::JumpViewLeft, |e, _| e.walk_to_region(crate::window::Direction::Left), Key::Char('h')),
    (NormalCmd::JumpViewDown, |e, _| e.walk_to_region(crate::window::Direction::Down), Key::Char('j')),
    (NormalCmd::JumpViewUp, |e, _| e.walk_to_region(crate::window::Direction::Up), Key::Char('k')),
    (NormalCmd::JumpViewRight, |e, _| e.walk_to_region(crate::window::Direction::Right), Key::Char('l')),
    (NormalCmd::ToggleLeftSidebar, |e, _| e.toggle_region(crate::sidebar::Side::Left), Key::Char('e')),
    (NormalCmd::ToggleRightSidebar, |e, _| e.toggle_region(crate::sidebar::Side::Right), Key::Char('i')),
    (NormalCmd::GotoLeftSidebar, |e, _| e.go_to_region(super::sidebar::Region::Side(crate::sidebar::Side::Left)), Key::Char('E')),
    (NormalCmd::GotoRightSidebar, |e, _| e.go_to_region(super::sidebar::Region::Side(crate::sidebar::Side::Right)), Key::Char('I')),
    // Each action is one side, whatever the keymap; a keymap chooses which
    // action its key is bound to.
    (NormalCmd::Hsplit, |e, _| {
        e.split_window_on(crate::window::Split::Below, false);
    }, Key::Char('s')),
    (NormalCmd::Vsplit, |e, _| {
        e.split_window_on(crate::window::Split::Beside, false);
    }, Key::Char('v')),
    (NormalCmd::HsplitAbove, |e, _| {
        e.split_window_on(crate::window::Split::Below, true);
    }, Key::Char('s')),
    (NormalCmd::VsplitLeft, |e, _| {
        e.split_window_on(crate::window::Split::Beside, true);
    }, Key::Char('v')),
    (NormalCmd::HsplitNew, |e, _| e.new_window_on(crate::window::Split::Below, false, false), Key::Char('s')),
    (NormalCmd::NewWindowLeft, |e, _| e.new_window_on(crate::window::Split::Beside, true, false), Key::Char('h')),
    (NormalCmd::NewWindowDown, |e, _| e.new_window_on(crate::window::Split::Below, false, false), Key::Char('j')),
    (NormalCmd::NewWindowUp, |e, _| e.new_window_on(crate::window::Split::Below, true, false), Key::Char('k')),
    (NormalCmd::NewWindowRight, |e, _| e.new_window_on(crate::window::Split::Beside, false, false), Key::Char('l')),
    (NormalCmd::NewWindowLeftPick, |e, _| e.new_window_on(crate::window::Split::Beside, true, true), Key::Char('H')),
    (NormalCmd::NewWindowDownPick, |e, _| e.new_window_on(crate::window::Split::Below, false, true), Key::Char('J')),
    (NormalCmd::NewWindowUpPick, |e, _| e.new_window_on(crate::window::Split::Below, true, true), Key::Char('K')),
    (NormalCmd::NewWindowRightPick, |e, _| e.new_window_on(crate::window::Split::Beside, false, true), Key::Char('L')),
    (NormalCmd::VsplitNew, |e, _| e.new_window_on(crate::window::Split::Beside, false, false), Key::Char('v')),
    (NormalCmd::WindowNewMode, |e, _| e.pending = Pending::RegionNew, Key::Char('n')),
    (NormalCmd::Wclose, |e, _| e.close_this_region(true), Key::Char('q')),
    (NormalCmd::Wonly, |e, _| e.close_other_regions(), Key::Char('o')),
    (NormalCmd::TransposeView, |e, _| e.transpose_window(), Key::Char('t')),
    (NormalCmd::SwapViewLeft, |e, _| e.swap_window(crate::window::Direction::Left), Key::Char('H')),
    (NormalCmd::SwapViewDown, |e, _| e.swap_window(crate::window::Direction::Down), Key::Char('J')),
    (NormalCmd::SwapViewUp, |e, _| e.swap_window(crate::window::Direction::Up), Key::Char('K')),
    (NormalCmd::SwapViewRight, |e, _| e.swap_window(crate::window::Direction::Right), Key::Char('L')),
    (NormalCmd::GotoFileHsplit, |e, _| e.goto_file_in_split(crate::window::Split::Below, false), Key::Char('f')),
    (NormalCmd::GotoFileVsplit, |e, _| e.goto_file_in_split(crate::window::Split::Beside, false), Key::Char('F')),
    (NormalCmd::GotoFileHsplitAbove, |e, _| e.goto_file_in_split(crate::window::Split::Below, true), Key::Char('f')),
    // ── ` ──────────────────────────────────────────────────────────────
    (NormalCmd::SwitchToLowercase, |e, _| e.map_selection(|c| c.to_lowercase().collect()), Key::Char('l')),
    (NormalCmd::SwitchToUppercase, |e, _| e.map_selection(|c| c.to_uppercase().collect()), Key::Char('u')),
    (NormalCmd::ConvertToSimplified, |e, _| e.convert_selection(crate::convert::Side::T, crate::convert::Side::S), Key::Char('s')),
    (NormalCmd::ConvertToTraditional, |e, _| e.convert_selection(crate::convert::Side::S, crate::convert::Side::T), Key::Char('t')),
    (NormalCmd::ConvertToTaiwanTraditional, |e, _| e.convert_selection(crate::convert::Side::S, crate::convert::Side::Tw), Key::Char('w')),
    (NormalCmd::ConvertToHongKongTraditional, |e, _| e.convert_selection(crate::convert::Side::S, crate::convert::Side::Hk), Key::Char('k')),
    (NormalCmd::ConvertToMainlandTraditional, |e, _| e.convert_selection(crate::convert::Side::S, crate::convert::Side::C), Key::Char('c')),
    (NormalCmd::ConvertToClassicalTraditional, |e, _| e.convert_selection(crate::convert::Side::S, crate::convert::Side::G), Key::Char('g')),
    (NormalCmd::ConvertToJapanese, |e, _| e.convert_selection(crate::convert::Side::T, crate::convert::Side::Jp), Key::Char('j')),
    (NormalCmd::SwitchToHalfwidth, |e, _| e.map_selection(crate::editor::to_halfwidth), Key::Char('h')),
    (NormalCmd::SwitchToFullwidth, |e, _| e.map_selection(crate::editor::to_fullwidth), Key::Char('f')),
    // ── Space m ────────────────────────────────────────────────────────
    (NormalCmd::ConflictKeepOurs, |e, _| e.resolve_conflict(crate::conflict::Keep::Ours), Key::Char('o')),
    (NormalCmd::ConflictKeepTheirs, |e, _| e.resolve_conflict(crate::conflict::Keep::Theirs), Key::Char('t')),
    (NormalCmd::ConflictKeepBoth, |e, _| e.resolve_conflict(crate::conflict::Keep::Both), Key::Char('b')),
    // ── Space o ────────────────────────────────────────────────────────
    (NormalCmd::RecentFilePicker, |e, _| e.open_recent_picker(true), Key::Char('f')),
    (NormalCmd::GlobalRecentFilePicker, |e, _| e.open_recent_picker(false), Key::Char('g')),
    (NormalCmd::ProjectPicker, |e, _| e.open_project_picker(), Key::Char('p')),
    (NormalCmd::ResumeSession, |e, _| {
        let opened = e.restore_session();
        e.status = match opened {
            0 => say!("recent.nothing-to-resume"),
            n => say!("cli.picked-up-where-you-left-off", n),
        };
    }, Key::Char('r')),
    // ── Space z ────────────────────────────────────────────────────────
    (NormalCmd::YumeToggle, |e, _| e.ask_the_input_method("lang:toggle"), Key::Char('z')),
    (NormalCmd::YumeCommitMode, |e, _| e.pending = Pending::YumeValue('m'), Key::Char('m')),
    (NormalCmd::YumePanelMode, |e, _| e.pending = Pending::YumeValue('p'), Key::Char('p')),
    (NormalCmd::YumePreeditMode, |e, _| e.pending = Pending::YumeValue('e'), Key::Char('e')),
    (NormalCmd::YumeTogglePrediction, |e, _| e.run_the_line(":yume-autocompletion"), Key::Char('y')),
    (NormalCmd::YumePickScheme, |e, _| e.open_the_command_line("yume-scheme "), Key::Char('i')),
    (NormalCmd::YumeNextScheme, |e, _| e.ask_the_input_method("scheme:next"), Key::Char('n')),
    (NormalCmd::YumeMenuSize3, |e, _| e.ask_the_input_method("menu:3"), Key::Char('3')),
    (NormalCmd::YumeMenuSize4, |e, _| e.ask_the_input_method("menu:4"), Key::Char('4')),
    (NormalCmd::YumeMenuSize5, |e, _| e.ask_the_input_method("menu:5"), Key::Char('5')),
    (NormalCmd::YumeMenuSize6, |e, _| e.ask_the_input_method("menu:6"), Key::Char('6')),
    (NormalCmd::YumeMenuSize7, |e, _| e.ask_the_input_method("menu:7"), Key::Char('7')),
    (NormalCmd::YumeMenuSize8, |e, _| e.ask_the_input_method("menu:8"), Key::Char('8')),
    (NormalCmd::YumeMenuSize9, |e, _| e.ask_the_input_method("menu:9"), Key::Char('9')),
    (NormalCmd::YumeNextAnnotation, |e, _| e.run_the_line(":yume-annotation next"), Key::Char('/')),
    (NormalCmd::YumeInstalledTable, |e, _| e.ask_the_input_method("~"), Key::Char('t')),
    (NormalCmd::YumeTableSource, |e, _| e.ask_the_input_method("?"), Key::Char('w')),
    (NormalCmd::YumeDataLocation, |e, _| e.ask_the_input_method("where"), Key::Char('W')),
    (NormalCmd::YumeCommitDefault, |e, _| e.run_the_line(":yume-commit default"), Key::Char('0')),
    (NormalCmd::YumeCommitDelayed, |e, _| e.run_the_line(":yume-commit delayed"), Key::Char('d')),
    (NormalCmd::YumeCommitUnique, |e, _| e.run_the_line(":yume-commit unique"), Key::Char('u')),
    (NormalCmd::YumeCommitFluency, |e, _| e.run_the_line(":yume-commit fluency"), Key::Char('f')),
    (NormalCmd::YumePanelBox, |e, _| e.run_the_line(":yume-panel box"), Key::Char('b')),
    (NormalCmd::YumePanelBottom, |e, _| e.run_the_line(":yume-panel bottom"), Key::Char('m')),
    (NormalCmd::YumePanelInline, |e, _| e.run_the_line(":yume-panel inline"), Key::Char('i')),
    (NormalCmd::YumePreeditOff, |e, _| e.run_the_line(":yume-inline-preedit off"), Key::Char('o')),
    (NormalCmd::YumePreeditComposition, |e, _| e.run_the_line(":yume-inline-preedit composition"), Key::Char('c')),
    (NormalCmd::YumePreeditPreview, |e, _| e.run_the_line(":yume-inline-preedit preview"), Key::Char('p')),
    (NormalCmd::YumeCharsetMode, |e, _| e.pending = Pending::YumeValue('0'), Key::Char('0')),
    (NormalCmd::YumeSortMode, |e, _| e.pending = Pending::YumeValue('s'), Key::Char('s')),
    (NormalCmd::YumeLanesMode, |e, _| e.pending = Pending::YumeValue('g'), Key::Char('g')),
    (NormalCmd::YumeBuiltinMode, |e, _| e.pending = Pending::YumeValue('b'), Key::Char('b')),
    (NormalCmd::YumeCharsetAll, |e, _| e.run_the_line(":yume-charset all"), Key::Char('a')),
    (NormalCmd::YumeCharsetCommon, |e, _| e.run_the_line(":yume-charset common"), Key::Char('c')),
    (NormalCmd::YumeCharsetTonggui, |e, _| e.run_the_line(":yume-charset tonggui"), Key::Char('s')),
    (NormalCmd::YumeCharsetHarmonic, |e, _| e.run_the_line(":yume-charset harmonic"), Key::Char('t')),
    (NormalCmd::YumeSortOriginal, |e, _| e.run_the_line(":yume-charset-sort original"), Key::Char('o')),
    (NormalCmd::YumeSortCommon, |e, _| e.run_the_line(":yume-charset-sort common"), Key::Char('c')),
    (NormalCmd::YumeSortTonggui, |e, _| e.run_the_line(":yume-charset-sort tonggui"), Key::Char('s')),
    (NormalCmd::YumeSortHarmonic, |e, _| e.run_the_line(":yume-charset-sort harmonic"), Key::Char('t')),
    (NormalCmd::YumeLaneTonggui, |e, _| e.run_the_line(":yume-lanes tonggui"), Key::Char('j')),
    (NormalCmd::YumeLaneTongfan, |e, _| e.run_the_line(":yume-lanes tongfan"), Key::Char('f')),
    (NormalCmd::YumeLaneTai, |e, _| e.run_the_line(":yume-lanes tai"), Key::Char('t')),
    (NormalCmd::YumeBuiltinPinyin, |e, _| e.ask_the_input_method("! pinyin"), Key::Char('p')),
    (NormalCmd::YumeBuiltinLingming, |e, _| e.ask_the_input_method("! lingming"), Key::Char('l')),
    (NormalCmd::YumeBuiltinXingchen, |e, _| e.ask_the_input_method("! xingchen"), Key::Char('x')),
    // ── Space t ────────────────────────────────────────────────────────
    // Each is the table code's answer to its key, which knows what kind of
    // table the cursor is in and refuses, in words, what that kind cannot do.
    (NormalCmd::TableSourceView, |e, _| e.table_structure(Key::Char('o')), Key::Char('o')),
    (NormalCmd::TableBasicView, |e, _| e.table_structure(Key::Char('b')), Key::Char('b')),
    (NormalCmd::TableWindowView, |e, _| e.table_structure(Key::Char('t')), Key::Char('t')),
    (NormalCmd::TableLeaveWindow, |e, _| e.table_structure(Key::Char('q')), Key::Char('q')),
    (NormalCmd::GotoNextTable, |e, _| e.table_structure(Key::Char(']')), Key::Char(']')),
    (NormalCmd::GotoPrevTable, |e, _| e.table_structure(Key::Char('[')), Key::Char('[')),
    (NormalCmd::TableConvertMode, |e, _| e.pending = Pending::TableConvert, Key::Char('x')),
    (NormalCmd::TableCellWidth, |e, _| e.table_structure(Key::Char('w')), Key::Char('w')),
    (NormalCmd::TableSearchColumns, |e, _| e.table_structure(Key::Char('/')), Key::Char('/')),
    (NormalCmd::TableGotoCell, |e, _| e.table_structure(Key::Char('g')), Key::Char('g')),
    // **The grid's own four**. In a table's grid they are
    // reached through `table_takes`, as its `Enter`, `Esc`, `Tab` and
    // `S-Tab`; outside one only the cell steps do anything, in a cell while
    // typing (where `Tab` is the same step).
    (NormalCmd::TableEnterCell, |_, _| {}, Key::Enter),
    (NormalCmd::TableLeaveCell, |_, _| {}, Key::Esc),
    (NormalCmd::TableNextCell, |e, _| e.table_tab(true), Key::Tab),
    (NormalCmd::TablePrevCell, |e, _| e.table_tab(false), Key::BackTab),
    (NormalCmd::TableYankColumn, |e, _| e.table_structure(Key::Char('y')), Key::Char('y')),
    (NormalCmd::TableReplaceColumn, |e, _| e.table_structure(Key::Char('v')), Key::Char('v')),
    (NormalCmd::TablePasteColumnRight, |e, _| e.table_structure(Key::Char('p')), Key::Char('p')),
    (NormalCmd::TablePasteColumnLeft, |e, _| e.table_structure(Key::Char('P')), Key::Char('P')),
    (NormalCmd::TableAddRowBelow, |e, _| e.table_structure(Key::Char('r')), Key::Char('r')),
    (NormalCmd::TableAddRowAbove, |e, _| e.table_structure(Key::Char('R')), Key::Char('R')),
    (NormalCmd::TableAddColumnRight, |e, _| e.table_structure(Key::Char('c')), Key::Char('c')),
    (NormalCmd::TableAddColumnLeft, |e, _| e.table_structure(Key::Char('C')), Key::Char('C')),
    (NormalCmd::TableDeleteRow, |e, _| e.table_structure(Key::Char('d')), Key::Char('d')),
    (NormalCmd::TableDeleteColumn, |e, _| e.table_structure(Key::Char('D')), Key::Char('D')),
    (NormalCmd::TableMoveRowDown, |e, _| e.table_structure(Key::Char('j')), Key::Char('j')),
    (NormalCmd::TableMoveRowUp, |e, _| e.table_structure(Key::Char('k')), Key::Char('k')),
    (NormalCmd::TableMoveColumnLeft, |e, _| e.table_structure(Key::Char('h')), Key::Char('h')),
    (NormalCmd::TableMoveColumnRight, |e, _| e.table_structure(Key::Char('l')), Key::Char('l')),
    (NormalCmd::TableAlignLeft, |e, _| e.table_structure(Key::Char('<')), Key::Char('<')),
    (NormalCmd::TableAlignCenter, |e, _| e.table_structure(Key::Char('=')), Key::Char('=')),
    (NormalCmd::TableAlignRight, |e, _| e.table_structure(Key::Char('>')), Key::Char('>')),
    (NormalCmd::TableLineUp, |e, _| e.table_structure(Key::Char('F')), Key::Char('F')),
    (NormalCmd::TableRecordPanel, |e, _| e.table_structure(Key::Char('i')), Key::Char('i')),
    (NormalCmd::TableFirstRowIsData, |e, _| e.table_structure(Key::Char('H')), Key::Char('H')),
    (NormalCmd::TableSortAscending, |e, _| e.table_structure(Key::Char('s')), Key::Char('s')),
    (NormalCmd::TableSortDescending, |e, _| e.table_structure(Key::Char('S')), Key::Char('S')),
    (NormalCmd::ConvertTableToPipe, |e, _| e.convert_table_key(Key::Char('p')), Key::Char('p')),
    (NormalCmd::PasteTableAsPipe, |e, _| e.convert_table_key(Key::Char('P')), Key::Char('P')),
    (NormalCmd::ConvertTableToCsv, |e, _| e.convert_table_key(Key::Char('c')), Key::Char('c')),
    (NormalCmd::PasteTableAsCsv, |e, _| e.convert_table_key(Key::Char('C')), Key::Char('C')),
    (NormalCmd::ConvertTableToTsv, |e, _| e.convert_table_key(Key::Char('t')), Key::Char('t')),
    (NormalCmd::PasteTableAsTsv, |e, _| e.convert_table_key(Key::Char('T')), Key::Char('T')),
];

impl Editor {
    /// **A goto motion**: to a line's start or end, its first character, or
    /// the file's start or end — the cell's, in a table whose cells are the
    /// unit, and the grid's rows, in a table that has the window.
    pub(super) fn goto_motion(&mut self, what: motion::Motion) {
        let step = match what {
            motion::Motion::LineStart | motion::Motion::LineFirstNonBlank => {
                Some(crate::table::layer::Step::RecordStart)
            }
            motion::Motion::LineEnd => Some(crate::table::layer::Step::RecordEnd),
            _ => None,
        };
        if let Some(step) = step.filter(|_| self.cell_is_the_unit()) {
            return self.step_cells(step, 1);
        }
        // 全窗裏 `gh`/`gl` 是這一格的兩頭（見 `cell_edge`）。
        let edge = match what {
            motion::Motion::LineStart | motion::Motion::LineFirstNonBlank => self.cell_edge(false),
            motion::Motion::LineEnd => self.cell_edge(true),
            _ => None,
        };
        if let Some(at) = edge {
            self.move_head(at);
            return;
        }
        let rows = (!self.table_layer().walled())
            .then(|| self.grid_has_the_pane().then(|| self.table_row_span()).flatten())
            .flatten();
        if let Some((first, last)) = rows {
            match what {
                motion::Motion::FileStart => return self.goto_grid_row(1),
                motion::Motion::FileEnd => return self.goto_grid_row(last - first + 1),
                _ => {}
            }
        }
        let span = self.run_motion(what);
        self.jump_to(span);
    }

    /// vim's `ge` / `gE`.
    fn jump_back_to_word_end(&mut self, cx: Cx, big: bool) {
        let grain = match big {
            true => motion::Grain::Big,
            false => self.word_grain(),
        };
        self.repeat(cx.count, |e| {
            let span = e.read_motion(motion::Motion::WordEndBack(grain), motion::Reading::Caret);
            e.jump_to(span);
        });
    }

    fn walk_zongs(&mut self, cx: Cx, onward: bool) {
        let mut first = cx.continuing_zong;
        self.repeat(cx.count, move |e| {
            e.move_zong_from(onward, first);
            first = true;
        });
    }

    /// A vim `g` operator: it waits for a motion, or acts on what Visual mode
    /// has selected.
    /// vim's `zf`: an operator over a motion, or the Visual selection's
    /// lines folded.
    fn fold_operator(&mut self) {
        self.g_operator('Z');
    }

    fn g_operator(&mut self, op: char) {
        if self.extend {
            return self.vim_visual_key(op);
        }
        self.pending = Pending::VimOperator { op, first: None };
    }

    /// The character under the cursor, looked up in the dictionary: in a
    /// float, or in the sidebar.
    fn look_up_under_the_cursor(&mut self, afloat: bool) {
        match self.grapheme_at_cursor() {
            Some(ch) => {
                match afloat {
                    true => self.look_up_afloat(&ch),
                    false => self.look_up(&ch),
                };
            }
            None => self.set_status(say!("ui.nothing-to-look-up")),
        }
    }

    /// A request for the input method, which the front end carries out.
    fn ask_the_input_method(&mut self, what: &str) {
        self.scheme_request = Some(what.to_string());
    }
}
