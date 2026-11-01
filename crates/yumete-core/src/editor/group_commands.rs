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
use crate::input::Key;
use crate::motion;

/// **Every command a group's key runs**, with the key it answers to in its
/// group by default.
pub(super) const GROUP_COMMANDS: &[NormalCommand] = &[
    // ── g ──────────────────────────────────────────────────────────────
    ("goto_file_start", |e, cx| {
        // `g30g` — the group's own count — and `30gg`, vi's order.
        let line = e.sequence_span().map(|(n, _)| n).or(cx.operator_count);
        e.operator_count = None;
        if let Some(n) = line.filter(|&n| n > 0) {
            return e.goto_grid_row(n);
        }
        e.remember_jump();
        e.goto_motion(motion::Motion::FileStart);
    }, Key::Char('g')),
    ("goto_file_end", |e, _| {
        e.remember_jump();
        e.goto_motion(motion::Motion::FileEnd);
    }, Key::Char('e')),
    // `ge` and then `gh`: helix's `ge`, which this editor gives to the end
    // of the file.
    ("goto_last_line", |e, _| {
        e.remember_jump();
        e.goto_motion(motion::Motion::FileEnd);
        e.goto_motion(motion::Motion::LineStart);
    }, Key::Char('e')),
    // vim's `ge` / `gE`: back to a word's end.
    ("jump_prev_word_end", |e, cx| {
        e.remember_jump();
        e.jump_back_to_word_end(cx, false);
    }, Key::Char('e')),
    ("jump_prev_long_word_end", |e, cx| e.jump_back_to_word_end(cx, true), Key::Char('E')),
    ("goto_first_nonwhitespace", |e, _| e.goto_motion(motion::Motion::LineFirstNonBlank), Key::Char('s')),
    ("open_here", |e, _| e.goto_file_under_cursor(), Key::Char('f')),
    ("follow_link", |e, _| e.follow_link(), Key::Char('L')),
    ("goto_word", |e, _| e.start_jump(), Key::Char('w')),
    ("goto_word_by_letters", |e, _| e.start_seek(false), Key::Char('x')),
    ("goto_char_by_pinyin", |e, _| e.start_seek(true), Key::Char('z')),
    ("goto_last_accessed_file", |e, _| e.goto_last_file(), Key::Char('a')),
    ("next_buffer", |e, _| e.next_buffer(), Key::Char('n')),
    ("prev_buffer", |e, _| e.prev_buffer(), Key::Char('p')),
    ("goto_definition", |e, _| e.show_definition(false), Key::Char('d')),
    ("goto_definition_other_window", |e, _| e.show_definition(true), Key::Char('D')),
    ("goto_reference", |e, _| e.ask_references(), Key::Char('r')),
    ("search_selection", |e, _| {
        e.definition_preview = false;
        e.search_the_page(false)
    }, Key::Char('/')),
    ("search_selection_other_window", |e, _| {
        e.definition_preview = true;
        e.search_the_page(false)
    }, Key::Char('?')),
    // A vertical page under the vim preset: `gj` walks the 縱 the way the
    // writing runs, one line of the file at a time.
    ("move_next_zong", |e, cx| e.walk_zongs(cx, true), Key::Char('j')),
    ("move_prev_zong", |e, cx| e.walk_zongs(cx, false), Key::Char('k')),
    ("goto_last_modification", |e, _| e.goto_last_modification(), Key::Char('.')),
    ("reselect_last_selection", |e, _| {
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
    ("insert_at_last_insert", |e, _| {
        if let Some(at) = e.current_buffer().insert_stop() {
            e.set_cursor(at);
        }
        e.enter_insert();
    }, Key::Char('i')),
    ("goto_older_change", |e, _| e.walk_changes(true), Key::Char(';')),
    ("goto_newer_change", |e, _| e.walk_changes(false), Key::Char(',')),
    // vim's `g` operators, each waiting for a motion.
    ("operator_lowercase", |e, _| e.g_operator('u'), Key::Char('u')),
    ("operator_uppercase", |e, _| e.g_operator('U'), Key::Char('U')),
    ("operator_switch_case", |e, _| e.g_operator('~'), Key::Char('~')),
    ("operator_fill", |e, _| e.g_operator('q'), Key::Char('q')),
    ("join_selections_no_space", |e, cx| {
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
    ("file_picker", |e, _| e.open_file_picker(), Key::Char('f')),
    ("file_picker_in_current_directory", |e, _| e.open_file_picker_here(), Key::Char('F')),
    ("file_explorer", |e, _| e.open_explorer(), Key::Char('e')),
    ("file_explorer_in_current_buffer_directory", |e, _| e.open_explorer_here(), Key::Char('E')),
    ("buffer_picker", |e, _| e.open_buffer_picker(), Key::Char('b')),
    ("changed_file_picker", |e, _| e.open_changed_picker(), Key::Char('g')),
    ("copy_to_clipboard", |e, _| e.copy_to_clipboard(), Key::Char('y')),
    ("paste_from_clipboard", |e, _| e.clipboard_paste(true), Key::Char('p')),
    ("paste_clipboard_before", |e, _| e.clipboard_paste(false), Key::Char('P')),
    ("replace_selections_with_clipboard", |e, _| e.clipboard_replace(), Key::Char('R')),
    ("symbol_picker", |e, _| e.open_symbol_picker(), Key::Char('s')),
    ("workspace_symbol_picker", |e, _| e.open_project_symbol_picker(), Key::Char('S')),
    ("dictionary", |e, _| e.look_up_under_the_cursor(true), Key::Char('n')),
    ("dictionary_panel", |e, _| e.look_up_under_the_cursor(false), Key::Char('N')),
    ("hover", |e, _| {
        if !e.ask_what_this_is(true) {
            e.set_status(say!("lsp.not-code"));
        }
    }, Key::Char('k')),
    ("hover_panel", |e, _| {
        if !e.ask_what_this_is(false) {
            e.set_status(say!("lsp.not-code"));
        }
    }, Key::Char('K')),
    ("diagnostics_picker", |e, _| e.open_diagnostics_picker(false), Key::Char('d')),
    ("workspace_diagnostics_picker", |e, _| e.open_diagnostics_picker(true), Key::Char('D')),
    ("show_diagnostic", |e, _| e.show_the_problem_here(true), Key::Char('l')),
    ("show_diagnostic_panel", |e, _| e.show_the_problem_here(false), Key::Char('L')),
    ("show_change", |e, _| e.show_the_change_here(true), Key::Char('x')),
    ("show_change_panel", |e, _| e.show_the_change_here(false), Key::Char('X')),
    ("rename_symbol", |e, _| e.start_a_rename(), Key::Char('r')),
    ("code_action", |e, _| e.ask_actions(), Key::Char('a')),
    ("show_record", |e, _| e.show_the_record_here(true), Key::Char('i')),
    ("show_record_panel", |e, _| e.show_the_record_here(false), Key::Char('I')),
    ("table_mode", |e, _| e.pending = Pending::Table, Key::Char('t')),
    ("paste_menu", |e, _| e.open_paste_picker(), Key::Char('"')),
    ("comment_line", |e, _| e.toggle_comment(crate::comment::Prefer::Line), Key::Char('c')),
    ("comment_block", |e, _| e.toggle_comment(crate::comment::Prefer::Block), Key::Char('C')),
    ("conflict_mode", |e, _| e.pending = Pending::Conflict, Key::Char('m')),
    ("open_recent_mode", |e, _| e.pending = Pending::Open, Key::Char('o')),
    ("yume_mode", |e, _| e.pending = Pending::Yume, Key::Char('z')),
    ("search_panel", |e, _| e.open_search(), Key::Char('/')),
    // helix's `command_palette` (2026-11-01): every command and every key's
    // command, each with its key and what it does, searched by description
    // (`::`). It used to open an empty `:` line.
    ("command_menu", |e, _| {
        e.open_the_command_line("");
        e.mode = Mode::Lookfor;
        e.lookfor_focus = 0;
    }, Key::Char('?')),
    // ── ] and [ ────────────────────────────────────────────────────────
    ("goto_next_conflict", |e, _| e.go_to_conflict(true), Key::Char('m')),
    ("goto_prev_conflict", |e, _| e.go_to_conflict(false), Key::Char('m')),
    ("goto_next_change", |e, _| e.go_to_change(true), Key::Char('g')),
    ("goto_prev_change", |e, _| e.go_to_change(false), Key::Char('g')),
    ("goto_next_diag", |e, _| e.go_to_problem(true, false), Key::Char('d')),
    ("goto_prev_diag", |e, _| e.go_to_problem(false, false), Key::Char('d')),
    ("goto_last_diag", |e, _| e.go_to_problem(true, true), Key::Char('D')),
    ("goto_first_diag", |e, _| e.go_to_problem(false, true), Key::Char('D')),
    ("goto_next_function", |e, _| e.go_to_definition_nearby(true, true), Key::Char('f')),
    ("goto_prev_function", |e, _| e.go_to_definition_nearby(false, true), Key::Char('f')),
    ("goto_next_class", |e, _| e.go_to_definition_nearby(true, false), Key::Char('t')),
    ("goto_prev_class", |e, _| e.go_to_definition_nearby(false, false), Key::Char('t')),
    ("goto_next_comment", |e, _| e.go_to_object_nearby(true, crate::code::Object::Comment), Key::Char('c')),
    ("goto_prev_comment", |e, _| e.go_to_object_nearby(false, crate::code::Object::Comment), Key::Char('c')),
    ("goto_next_paragraph", |e, _| {
        let span = e.run_motion(motion::Motion::Paragraph { forward: true });
        e.take_span(span);
    }, Key::Char('p')),
    ("goto_prev_paragraph", |e, _| {
        let span = e.run_motion(motion::Motion::Paragraph { forward: false });
        e.take_span(span);
    }, Key::Char('p')),
    ("add_newline_below", |e, cx| {
        e.operator_count = None;
        e.add_blank_line(true, cx.count);
    }, Key::Char(' ')),
    ("add_newline_above", |e, cx| {
        e.operator_count = None;
        e.add_blank_line(false, cx.count);
    }, Key::Char(' ')),
    // ── z and Z ────────────────────────────────────────────────────────
    ("align_view_center", |e, _| e.aim_the_page(crate::editor::Aim::Middle), Key::Char('z')),
    ("align_view_top", |e, _| e.aim_the_page(crate::editor::Aim::Top), Key::Char('t')),
    ("align_view_bottom", |e, _| e.aim_the_page(crate::editor::Aim::Bottom), Key::Char('b')),
    ("scroll_down", |e, cx| e.scroll_the_page(true, cx.count), Key::Char('j')),
    ("scroll_up", |e, cx| e.scroll_the_page(false, cx.count), Key::Char('k')),
    // ── m ──────────────────────────────────────────────────────────────
    ("match_brackets", |e, _| e.acting(false, |e| e.goto_matching_bracket()), Key::Char('m')),
    // vim's `%`: off a bracket, the next one along the line.
    ("vim_match_brackets", |e, _| e.acting(true, |e| e.goto_matching_bracket()), Key::Char('m')),
    // These four wait for a pair or an object, and keep the count for it
    // (`2mi(` is the second pair out).
    ("select_textobject_inner", |e, cx| {
        e.pending = Pending::MatchPair { around: false };
        e.operator_count = cx.operator_count;
    }, Key::Char('i')),
    ("select_textobject_around", |e, cx| {
        e.pending = Pending::MatchPair { around: true };
        e.operator_count = cx.operator_count;
    }, Key::Char('a')),
    ("surround_add", |e, _| e.pending = Pending::Surround, Key::Char('s')),
    ("surround_delete", |e, cx| {
        e.pending = Pending::SurroundOff;
        e.operator_count = cx.operator_count;
    }, Key::Char('d')),
    ("surround_replace", |e, cx| {
        e.pending = Pending::SurroundFrom;
        e.operator_count = cx.operator_count;
    }, Key::Char('r')),
    // ── C-w ────────────────────────────────────────────────────────────
    ("next_region", |e, _| e.next_region(), Key::Char('w')),
    ("jump_view_left", |e, _| e.walk_to_region(crate::window::Direction::Left), Key::Char('h')),
    ("jump_view_down", |e, _| e.walk_to_region(crate::window::Direction::Down), Key::Char('j')),
    ("jump_view_up", |e, _| e.walk_to_region(crate::window::Direction::Up), Key::Char('k')),
    ("jump_view_right", |e, _| e.walk_to_region(crate::window::Direction::Right), Key::Char('l')),
    ("toggle_left_sidebar", |e, _| e.toggle_region(crate::sidebar::Side::Left), Key::Char('e')),
    ("toggle_right_sidebar", |e, _| e.toggle_region(crate::sidebar::Side::Right), Key::Char('i')),
    ("goto_left_sidebar", |e, _| e.go_to_region(super::sidebar::Region::Side(crate::sidebar::Side::Left)), Key::Char('E')),
    ("goto_right_sidebar", |e, _| e.go_to_region(super::sidebar::Region::Side(crate::sidebar::Side::Right)), Key::Char('I')),
    ("hsplit", |e, _| {
        e.split_window(crate::window::Split::Below);
    }, Key::Char('s')),
    ("vsplit", |e, _| {
        e.split_window(crate::window::Split::Beside);
    }, Key::Char('v')),
    ("hsplit_new", |e, _| {
        e.new_window(crate::window::Split::Below);
    }, Key::Char('s')),
    ("vsplit_new", |e, _| {
        e.new_window(crate::window::Split::Beside);
    }, Key::Char('v')),
    ("window_new_mode", |e, _| e.pending = Pending::RegionNew, Key::Char('n')),
    ("wclose", |e, _| e.close_this_region(), Key::Char('q')),
    ("wonly", |e, _| e.close_other_regions(), Key::Char('o')),
    // ── ` ──────────────────────────────────────────────────────────────
    ("switch_to_lowercase", |e, _| e.map_selection(|c| c.to_lowercase().collect()), Key::Char('l')),
    ("switch_to_uppercase", |e, _| e.map_selection(|c| c.to_uppercase().collect()), Key::Char('u')),
    ("convert_to_simplified", |e, _| e.convert_selection(crate::convert::Side::T, crate::convert::Side::S), Key::Char('s')),
    ("convert_to_traditional", |e, _| e.convert_selection(crate::convert::Side::S, crate::convert::Side::T), Key::Char('t')),
    ("convert_to_taiwan_traditional", |e, _| e.convert_selection(crate::convert::Side::S, crate::convert::Side::Tw), Key::Char('w')),
    ("convert_to_hong_kong_traditional", |e, _| e.convert_selection(crate::convert::Side::S, crate::convert::Side::Hk), Key::Char('k')),
    ("convert_to_mainland_traditional", |e, _| e.convert_selection(crate::convert::Side::S, crate::convert::Side::C), Key::Char('c')),
    ("convert_to_classical_traditional", |e, _| e.convert_selection(crate::convert::Side::S, crate::convert::Side::G), Key::Char('g')),
    ("convert_to_japanese", |e, _| e.convert_selection(crate::convert::Side::T, crate::convert::Side::Jp), Key::Char('j')),
    ("switch_to_halfwidth", |e, _| e.map_selection(crate::editor::to_halfwidth), Key::Char('h')),
    ("switch_to_fullwidth", |e, _| e.map_selection(crate::editor::to_fullwidth), Key::Char('f')),
    // ── Space m ────────────────────────────────────────────────────────
    ("conflict_keep_ours", |e, _| e.resolve_conflict(crate::conflict::Keep::Ours), Key::Char('o')),
    ("conflict_keep_theirs", |e, _| e.resolve_conflict(crate::conflict::Keep::Theirs), Key::Char('t')),
    ("conflict_keep_both", |e, _| e.resolve_conflict(crate::conflict::Keep::Both), Key::Char('b')),
    // ── Space o ────────────────────────────────────────────────────────
    ("recent_file_picker", |e, _| e.open_recent_picker(true), Key::Char('f')),
    ("global_recent_file_picker", |e, _| e.open_recent_picker(false), Key::Char('g')),
    ("project_picker", |e, _| e.open_project_picker(), Key::Char('p')),
    ("resume_session", |e, _| {
        let opened = e.restore_session();
        e.status = match opened {
            0 => say!("recent.nothing-to-resume"),
            n => say!("cli.picked-up-where-you-left-off", n),
        };
    }, Key::Char('r')),
    // ── Space z ────────────────────────────────────────────────────────
    ("yume_toggle", |e, _| e.ask_the_input_method("lang:toggle"), Key::Char('z')),
    ("yume_commit_mode", |e, _| e.pending = Pending::YumeValue('m'), Key::Char('m')),
    ("yume_panel_mode", |e, _| e.pending = Pending::YumeValue('p'), Key::Char('p')),
    ("yume_preedit_mode", |e, _| e.pending = Pending::YumeValue('e'), Key::Char('e')),
    ("yume_toggle_prediction", |e, _| e.run_the_line(":yume-autocompletion toggle"), Key::Char('y')),
    ("yume_pick_scheme", |e, _| e.open_the_command_line("yume-scheme "), Key::Char('i')),
    ("yume_next_scheme", |e, _| e.ask_the_input_method("scheme:next"), Key::Char('n')),
    ("yume_menu_size_3", |e, _| e.ask_the_input_method("menu:3"), Key::Char('3')),
    ("yume_menu_size_4", |e, _| e.ask_the_input_method("menu:4"), Key::Char('4')),
    ("yume_menu_size_5", |e, _| e.ask_the_input_method("menu:5"), Key::Char('5')),
    ("yume_menu_size_6", |e, _| e.ask_the_input_method("menu:6"), Key::Char('6')),
    ("yume_menu_size_7", |e, _| e.ask_the_input_method("menu:7"), Key::Char('7')),
    ("yume_menu_size_8", |e, _| e.ask_the_input_method("menu:8"), Key::Char('8')),
    ("yume_menu_size_9", |e, _| e.ask_the_input_method("menu:9"), Key::Char('9')),
    ("yume_toggle_chaifen", |e, _| e.run_the_line(":yume-chaifen toggle"), Key::Char('/')),
    ("yume_installed_table", |e, _| e.ask_the_input_method("~"), Key::Char('t')),
    ("yume_table_source", |e, _| e.ask_the_input_method("?"), Key::Char('w')),
    ("yume_data_location", |e, _| e.ask_the_input_method("where"), Key::Char('W')),
    ("yume_commit_default", |e, _| e.run_the_line(":yume-commit default"), Key::Char('0')),
    ("yume_commit_delayed", |e, _| e.run_the_line(":yume-commit delayed"), Key::Char('d')),
    ("yume_commit_unique", |e, _| e.run_the_line(":yume-commit unique"), Key::Char('u')),
    ("yume_commit_fluency", |e, _| e.run_the_line(":yume-commit fluency"), Key::Char('f')),
    ("yume_panel_box", |e, _| e.run_the_line(":yume-panel box"), Key::Char('b')),
    ("yume_panel_bottom", |e, _| e.run_the_line(":yume-panel bottom"), Key::Char('m')),
    ("yume_panel_inline", |e, _| e.run_the_line(":yume-panel inline"), Key::Char('i')),
    ("yume_preedit_off", |e, _| e.run_the_line(":yume-inline-preedit off"), Key::Char('o')),
    ("yume_preedit_composition", |e, _| e.run_the_line(":yume-inline-preedit composition"), Key::Char('c')),
    ("yume_preedit_preview", |e, _| e.run_the_line(":yume-inline-preedit preview"), Key::Char('p')),
    // ── Space t ────────────────────────────────────────────────────────
    // Each is the table code's answer to its key, which knows what kind of
    // table the cursor is in and refuses, in words, what that kind cannot do.
    ("table_source_view", |e, _| e.table_structure(Key::Char('o')), Key::Char('o')),
    ("table_basic_view", |e, _| e.table_structure(Key::Char('b')), Key::Char('b')),
    ("table_window_view", |e, _| e.table_structure(Key::Char('t')), Key::Char('t')),
    ("table_leave_window", |e, _| e.table_structure(Key::Char('q')), Key::Char('q')),
    ("goto_next_table", |e, _| e.table_structure(Key::Char(']')), Key::Char(']')),
    ("goto_prev_table", |e, _| e.table_structure(Key::Char('[')), Key::Char('[')),
    ("table_convert_mode", |e, _| e.pending = Pending::TableConvert, Key::Char('x')),
    ("table_cell_width", |e, _| e.table_structure(Key::Char('w')), Key::Char('w')),
    ("table_search_columns", |e, _| e.table_structure(Key::Char('/')), Key::Char('/')),
    ("table_search_columns_other_window", |e, _| e.table_structure(Key::Char('?')), Key::Char('?')),
    ("table_goto_cell", |e, _| e.table_structure(Key::Char('g')), Key::Char('g')),
    ("table_yank_column", |e, _| e.table_structure(Key::Char('y')), Key::Char('y')),
    ("table_replace_column", |e, _| e.table_structure(Key::Char('v')), Key::Char('v')),
    ("table_paste_column_right", |e, _| e.table_structure(Key::Char('p')), Key::Char('p')),
    ("table_paste_column_left", |e, _| e.table_structure(Key::Char('P')), Key::Char('P')),
    ("table_add_row_below", |e, _| e.table_structure(Key::Char('r')), Key::Char('r')),
    ("table_add_row_above", |e, _| e.table_structure(Key::Char('R')), Key::Char('R')),
    ("table_add_column_right", |e, _| e.table_structure(Key::Char('c')), Key::Char('c')),
    ("table_add_column_left", |e, _| e.table_structure(Key::Char('C')), Key::Char('C')),
    ("table_delete_row", |e, _| e.table_structure(Key::Char('d')), Key::Char('d')),
    ("table_delete_column", |e, _| e.table_structure(Key::Char('D')), Key::Char('D')),
    ("table_move_row_down", |e, _| e.table_structure(Key::Char('j')), Key::Char('j')),
    ("table_move_row_up", |e, _| e.table_structure(Key::Char('k')), Key::Char('k')),
    ("table_move_column_left", |e, _| e.table_structure(Key::Char('h')), Key::Char('h')),
    ("table_move_column_right", |e, _| e.table_structure(Key::Char('l')), Key::Char('l')),
    ("table_align_left", |e, _| e.table_structure(Key::Char('<')), Key::Char('<')),
    ("table_align_center", |e, _| e.table_structure(Key::Char('=')), Key::Char('=')),
    ("table_align_right", |e, _| e.table_structure(Key::Char('>')), Key::Char('>')),
    ("table_line_up", |e, _| e.table_structure(Key::Char('F')), Key::Char('F')),
    ("table_record_panel", |e, _| e.table_structure(Key::Char('i')), Key::Char('i')),
    ("table_first_row_is_data", |e, _| e.table_structure(Key::Char('H')), Key::Char('H')),
    ("table_sort_ascending", |e, _| e.table_structure(Key::Char('s')), Key::Char('s')),
    ("table_sort_descending", |e, _| e.table_structure(Key::Char('S')), Key::Char('S')),
    ("convert_table_to_pipe", |e, _| e.convert_table_key(Key::Char('p')), Key::Char('p')),
    ("paste_table_as_pipe", |e, _| e.convert_table_key(Key::Char('P')), Key::Char('P')),
    ("convert_table_to_csv", |e, _| e.convert_table_key(Key::Char('c')), Key::Char('c')),
    ("paste_table_as_csv", |e, _| e.convert_table_key(Key::Char('C')), Key::Char('C')),
    ("convert_table_to_tsv", |e, _| e.convert_table_key(Key::Char('t')), Key::Char('t')),
    ("paste_table_as_tsv", |e, _| e.convert_table_key(Key::Char('T')), Key::Char('T')),
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
