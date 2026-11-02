//! **The commands the key groups name** (2026-10-31, step 3 of the keymap
//! plan): what `g h`, `Space f`, `]d`, `C-w s` and the rest do, each under
//! its own name, so a binding can reach any of them without its group.
//!
//! Each is written as the body it runs, in the table, beside its name:
//! most are one call. A command reads its count from its [`Cx`], never from
//! the key that reached it, so it does the same thing whatever key a
//! configuration puts it on. The rows that say which key of which group
//! runs which command are in `groups.rs`.

use super::normal::{Cx, NormalCommand};
use super::*;
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
        if let Some(n) = line.filter(|&n| n > 0) {
            return e.goto_grid_row(n);
        }
        e.remember_jump();
        e.goto_motion(motion::Motion::FileStart);
    }, Key::Char('g')),
    (NormalCmd::GotoFileEnd, |e, _| {
        e.remember_jump();
        e.goto_motion(motion::Motion::FileEnd);
    }, Key::Char('e')),
    // `ge` and then `gh`: helix's `ge`, which this editor gives to the end
    // of the file.
    (NormalCmd::GotoLastLine, |e, _| {
        e.remember_jump();
        e.goto_motion(motion::Motion::FileEnd);
        e.goto_motion(motion::Motion::LineStart);
    }, Key::Char('e')),
    // vim's `ge` / `gE`: back to a word's end.
    (NormalCmd::JumpPrevWordEnd, |e, cx| {
        e.remember_jump();
        e.jump_back_to_word_end(cx, false);
    }, Key::Char('e')),
    (NormalCmd::JumpPrevLongWordEnd, |e, cx| e.jump_back_to_word_end(cx, true), Key::Char('E')),
    (NormalCmd::GotoFirstNonwhitespace, |e, _| e.goto_motion(motion::Motion::LineFirstNonBlank), Key::Char('s')),
    (NormalCmd::OpenHere, |e, _| e.goto_file_under_cursor(), Key::Char('f')),
    (NormalCmd::FollowLink, |e, _| e.follow_link(), Key::Char('L')),
    (NormalCmd::OpenLink, |e, _| e.open_link(), Key::Char('x')),
    (NormalCmd::GotoDeclaration, |e, _| e.ask_for_place(crate::lsp::Goto::Declaration), Key::Char('D')),
    (NormalCmd::GotoTypeDefinition, |e, _| e.ask_for_place(crate::lsp::Goto::TypeDefinition), Key::Char('y')),
    (NormalCmd::GotoImplementation, |e, _| e.ask_for_place(crate::lsp::Goto::Implementation), Key::Char('i')),
    (NormalCmd::GotoLastModifiedFile, |e, _| e.goto_last_modified_file(), Key::Char('m')),
    (NormalCmd::GotoWord, |e, _| e.start_jump(), Key::Char('w')),
    (NormalCmd::GotoWordByLetters, |e, _| e.start_seek(false), Key::Char('x')),
    (NormalCmd::GotoCharByPinyin, |e, _| e.start_seek(true), Key::Char('z')),
    (NormalCmd::GotoLastAccessedFile, |e, _| e.goto_last_file(), Key::Char('a')),
    (NormalCmd::NextBuffer, |e, _| e.next_buffer(), Key::Char('n')),
    (NormalCmd::PrevBuffer, |e, _| e.prev_buffer(), Key::Char('p')),
    (NormalCmd::GotoDefinition, |e, _| e.show_definition(), Key::Char('d')),
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
    (NormalCmd::OperatorFill, |e, _| e.g_operator('q'), Key::Char('q')),
    (NormalCmd::OperatorFillKeepCursor, |e, _| e.g_operator('w'), Key::Char('w')),
    (NormalCmd::JoinSelectionsNoSpace, |e, cx| {
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
    (NormalCmd::CopyToClipboard, |e, _| e.copy_to_clipboard(), Key::Char('y')),
    (NormalCmd::PasteFromClipboard, |e, _| e.clipboard_paste(true), Key::Char('p')),
    (NormalCmd::PasteClipboardBefore, |e, _| e.clipboard_paste(false), Key::Char('P')),
    (NormalCmd::ReplaceSelectionsWithClipboard, |e, _| e.clipboard_replace(), Key::Char('R')),
    (NormalCmd::SymbolPicker, |e, _| e.open_symbol_picker(), Key::Char('s')),
    (NormalCmd::WorkspaceSymbolPicker, |e, _| e.open_project_symbol_picker(), Key::Char('S')),
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
    (NormalCmd::CommentLine, |e, _| e.toggle_comment(crate::comment::Prefer::Line), Key::Char('c')),
    (NormalCmd::CommentBlock, |e, _| e.toggle_comment(crate::comment::Prefer::Block), Key::Char('C')),
    (NormalCmd::ConflictMode, |e, _| e.pending = Pending::Conflict, Key::Char('m')),
    (NormalCmd::OpenRecentMode, |e, _| e.pending = Pending::Open, Key::Char('o')),
    (NormalCmd::YumeMode, |e, _| e.pending = Pending::Yume, Key::Char('z')),
    (NormalCmd::SearchPanel, |e, _| e.open_search(), Key::Char('/')),
    // helix's `command_palette` (2026-11-01): every command and every key's
    // command, each with its key and what it does, searched by description
    // (`::`). It used to open an empty `:` line.
    (NormalCmd::CommandMenu, |e, _| {
        e.open_the_command_line("");
        e.mode = Mode::Lookfor;
        e.lookfor_focus = 0;
    }, Key::Char('?')),
    // ── ] and [ ────────────────────────────────────────────────────────
    (NormalCmd::GotoNextConflict, |e, _| e.go_to_conflict(true), Key::Char('m')),
    (NormalCmd::GotoPrevConflict, |e, _| e.go_to_conflict(false), Key::Char('m')),
    (NormalCmd::GotoNextChange, |e, _| e.go_to_change(true), Key::Char('g')),
    (NormalCmd::GotoPrevChange, |e, _| e.go_to_change(false), Key::Char('g')),
    (NormalCmd::GotoNextDiag, |e, _| e.go_to_problem(true, false), Key::Char('d')),
    (NormalCmd::GotoPrevDiag, |e, _| e.go_to_problem(false, false), Key::Char('d')),
    (NormalCmd::GotoLastDiag, |e, _| e.go_to_problem(true, true), Key::Char('D')),
    (NormalCmd::GotoFirstDiag, |e, _| e.go_to_problem(false, true), Key::Char('D')),
    (NormalCmd::GotoNextFunction, |e, _| e.go_to_definition_nearby(true, true), Key::Char('f')),
    (NormalCmd::GotoPrevFunction, |e, _| e.go_to_definition_nearby(false, true), Key::Char('f')),
    (NormalCmd::GotoNextClass, |e, _| e.go_to_definition_nearby(true, false), Key::Char('t')),
    (NormalCmd::GotoPrevClass, |e, _| e.go_to_definition_nearby(false, false), Key::Char('t')),
    (NormalCmd::GotoNextComment, |e, _| e.go_to_object_nearby(true, crate::code::Object::Comment), Key::Char('c')),
    (NormalCmd::GotoPrevComment, |e, _| e.go_to_object_nearby(false, crate::code::Object::Comment), Key::Char('c')),
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
    (NormalCmd::AlignViewCenter, |e, _| e.aim_the_page(crate::editor::Aim::Middle), Key::Char('z')),
    (NormalCmd::AlignViewTop, |e, _| e.aim_the_page(crate::editor::Aim::Top), Key::Char('t')),
    (NormalCmd::AlignViewBottom, |e, _| e.aim_the_page(crate::editor::Aim::Bottom), Key::Char('b')),
    (NormalCmd::ScrollDown, |e, cx| e.scroll_the_page(true, cx.count), Key::Char('j')),
    (NormalCmd::ScrollUp, |e, cx| e.scroll_the_page(false, cx.count), Key::Char('k')),
    // ── m ──────────────────────────────────────────────────────────────
    (NormalCmd::MatchBrackets, |e, _| e.acting(false, |e| e.goto_matching_bracket()), Key::Char('m')),
    // vim's `%`: off a bracket, the next one along the line.
    (NormalCmd::VimMatchBrackets, |e, _| e.acting(true, |e| e.goto_matching_bracket()), Key::Char('m')),
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
    // ── C-w ────────────────────────────────────────────────────────────
    (NormalCmd::NextRegion, |e, _| e.next_region(), Key::Char('w')),
    (NormalCmd::JumpViewLeft, |e, _| e.walk_to_region(crate::window::Direction::Left), Key::Char('h')),
    (NormalCmd::JumpViewDown, |e, _| e.walk_to_region(crate::window::Direction::Down), Key::Char('j')),
    (NormalCmd::JumpViewUp, |e, _| e.walk_to_region(crate::window::Direction::Up), Key::Char('k')),
    (NormalCmd::JumpViewRight, |e, _| e.walk_to_region(crate::window::Direction::Right), Key::Char('l')),
    (NormalCmd::ToggleLeftSidebar, |e, _| e.toggle_region(crate::sidebar::Side::Left), Key::Char('e')),
    (NormalCmd::ToggleRightSidebar, |e, _| e.toggle_region(crate::sidebar::Side::Right), Key::Char('i')),
    (NormalCmd::GotoLeftSidebar, |e, _| e.go_to_region(super::sidebar::Region::Side(crate::sidebar::Side::Left)), Key::Char('E')),
    (NormalCmd::GotoRightSidebar, |e, _| e.go_to_region(super::sidebar::Region::Side(crate::sidebar::Side::Right)), Key::Char('I')),
    (NormalCmd::Hsplit, |e, _| {
        e.split_window(crate::window::Split::Below);
    }, Key::Char('s')),
    (NormalCmd::Vsplit, |e, _| {
        e.split_window(crate::window::Split::Beside);
    }, Key::Char('v')),
    (NormalCmd::HsplitNew, |e, _| {
        e.new_window(crate::window::Split::Below);
    }, Key::Char('s')),
    (NormalCmd::VsplitNew, |e, _| {
        e.new_window(crate::window::Split::Beside);
    }, Key::Char('v')),
    (NormalCmd::WindowNewMode, |e, _| e.pending = Pending::RegionNew, Key::Char('n')),
    (NormalCmd::Wclose, |e, _| e.close_this_region(), Key::Char('q')),
    (NormalCmd::Wonly, |e, _| e.close_other_regions(), Key::Char('o')),
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
    (NormalCmd::YumeTogglePrediction, |e, _| e.run_the_line(":yume-autocompletion toggle"), Key::Char('y')),
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
    // **The grid's own four** (2026-11-01). In a table's grid they are
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
