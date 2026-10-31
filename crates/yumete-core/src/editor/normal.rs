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

/// A Normal command: its name, its method, and the key it answers to by
/// default.
pub(super) type NormalCommand = (&'static str, fn(&mut Editor, Cx), Key);

/// **Every Normal command, by name**, with its method and the key it answers
/// to by default: the vim preset's own where it has one, helix's otherwise.
///
/// Read by the tests until the keymap is built from it (pass B of step 1);
/// until then the `match` in `on_normal_key` is what dispatches.
#[allow(dead_code)]
pub(super) const NORMAL_COMMANDS: &[NormalCommand] = &[
    ("move_char_left", Editor::cmd_move_char_left, Key::Char('h')),
    ("move_char_right", Editor::cmd_move_char_right, Key::Char('l')),
    ("move_visual_line_up", Editor::cmd_move_visual_line_up, Key::Char('k')),
    ("move_visual_line_down", Editor::cmd_move_visual_line_down, Key::Char('j')),
    ("move_line_up", Editor::cmd_move_line_up, Key::Char('k')),
    ("move_line_down", Editor::cmd_move_line_down, Key::Char('j')),
    ("goto_line_start", Editor::cmd_goto_line_start, Key::Home),
    ("goto_line_end", Editor::cmd_goto_line_end, Key::End),
    ("goto_next_line_nonwhitespace", Editor::cmd_goto_next_line_nonwhitespace, Key::Enter),
    ("jump_next_word_start", Editor::cmd_jump_by_word, Key::Char('w')),
    ("jump_next_long_word_start", Editor::cmd_jump_by_word, Key::Char('W')),
    ("jump_prev_word_start", Editor::cmd_jump_by_word, Key::Char('b')),
    ("jump_prev_long_word_start", Editor::cmd_jump_by_word, Key::Char('B')),
    ("jump_next_word_end", Editor::cmd_jump_by_word, Key::Char('e')),
    ("jump_next_long_word_end", Editor::cmd_jump_by_word, Key::Char('E')),
    ("jump_next_paragraph", Editor::cmd_jump_by_word, Key::Char('}')),
    ("jump_prev_paragraph", Editor::cmd_jump_by_word, Key::Char('{')),
    ("move_next_word_start", Editor::cmd_move_next_word_start, Key::Char('w')),
    ("move_next_word_end", Editor::cmd_move_next_word_end, Key::Char('e')),
    ("move_prev_word_start", Editor::cmd_move_prev_word_start, Key::Char('b')),
    ("move_next_paragraph", Editor::cmd_move_next_paragraph, Key::Char('}')),
    ("move_prev_paragraph", Editor::cmd_move_prev_paragraph, Key::Char('{')),
    ("goto_window_top", Editor::cmd_goto_window, Key::Char('H')),
    ("goto_window_center", Editor::cmd_goto_window, Key::Char('M')),
    ("goto_window_bottom", Editor::cmd_goto_window, Key::Char('L')),
    ("jump_prev_sentence", Editor::cmd_jump_by_sentence, Key::Char('(')),
    ("jump_next_sentence", Editor::cmd_jump_by_sentence, Key::Char(')')),
    ("goto_next_sentence", Editor::cmd_goto_next_sentence, Key::Char('L')),
    ("goto_prev_sentence", Editor::cmd_goto_prev_sentence, Key::Char('H')),
    ("add_mark", Editor::cmd_add_mark, Key::Char('M')),
    ("goto_mark", Editor::cmd_goto_mark, Key::Char('\'')),
    ("view_mode", Editor::cmd_view_mode, Key::Char('z')),
    ("sticky_view_mode", Editor::cmd_sticky_view_mode, Key::Char('Z')),
    ("right_bracket_mode", Editor::cmd_bracket_mode, Key::Char(']')),
    ("left_bracket_mode", Editor::cmd_bracket_mode, Key::Char('[')),
    ("move_next_long_word_start", Editor::cmd_move_next_long_word_start, Key::Char('W')),
    ("move_next_long_word_end", Editor::cmd_move_next_long_word_end, Key::Char('E')),
    ("move_prev_long_word_start", Editor::cmd_move_prev_long_word_start, Key::Char('B')),
    ("goto_mode", Editor::cmd_goto_mode, Key::Char('g')),
    ("space_mode", Editor::cmd_space_mode, Key::Char(' ')),
    ("find_next_char", Editor::cmd_find_char, Key::Char('f')),
    ("find_prev_char", Editor::cmd_find_char, Key::Char('F')),
    ("find_till_char", Editor::cmd_find_char, Key::Char('t')),
    ("till_prev_char", Editor::cmd_find_char, Key::Char('T')),
    ("select_line_mode", Editor::cmd_select_line_mode, Key::Char('V')),
    ("select_mode", Editor::cmd_select_mode, Key::Char('v')),
    ("normal_mode", Editor::cmd_normal_mode, Key::Esc),
    ("repeat_find_char", Editor::cmd_repeat_find, Key::Char(';')),
    ("repeat_find_char_reverse", Editor::cmd_repeat_find, Key::Char(',')),
    ("collapse_selection", Editor::cmd_collapse_selection, Key::Char(';')),
    ("copy_selection_on_next_line", Editor::cmd_copy_selection_on_next_line, Key::Char('C')),
    ("copy_selection_on_prev_line", Editor::cmd_copy_selection_on_prev_line, Key::Alt('C')),
    ("keep_primary_selection", Editor::cmd_keep_primary_selection, Key::Char(',')),
    ("remove_primary_selection", Editor::cmd_remove_primary_selection, Key::Alt(',')),
    ("merge_selections", Editor::cmd_merge_selections, Key::Alt('-')),
    ("merge_consecutive_selections", Editor::cmd_merge_consecutive_selections, Key::Alt('_')),
    ("ensure_selections_forward", Editor::cmd_ensure_selections_forward, Key::Alt(':')),
    ("shrink_to_line_bounds", Editor::cmd_shrink_to_line_bounds, Key::Alt('x')),
    ("split_selection_on_newline", Editor::cmd_split_selection_on_newline, Key::Alt('s')),
    ("rotate_selections_forward", Editor::cmd_rotate_selections_forward, Key::Char(')')),
    ("rotate_selections_backward", Editor::cmd_rotate_selections_backward, Key::Char('(')),
    ("trim_selections", Editor::cmd_trim_selections, Key::Char('_')),
    ("align_selections", Editor::cmd_align_selections, Key::Char('&')),
    ("rotate_selection_contents_forward", Editor::cmd_rotate_selection_contents_forward, Key::Alt(')')),
    ("rotate_selection_contents_backward", Editor::cmd_rotate_selection_contents_backward, Key::Alt('(')),
    ("select_regex", Editor::cmd_select_regex, Key::Char('s')),
    ("split_selection", Editor::cmd_split_selection, Key::Char('S')),
    ("keep_selections", Editor::cmd_keep_selections, Key::Alt('k')),
    ("remove_selections", Editor::cmd_remove_selections, Key::Alt('K')),
    ("extend_line_below", Editor::cmd_extend_line_below, Key::Char('x')),
    ("operator_delete", Editor::cmd_vim_operator, Key::Char('d')),
    ("operator_change", Editor::cmd_vim_operator, Key::Char('c')),
    ("operator_yank", Editor::cmd_vim_operator, Key::Char('y')),
    ("operator_indent", Editor::cmd_vim_operator, Key::Char('>')),
    ("operator_unindent", Editor::cmd_vim_operator, Key::Char('<')),
    ("delete_selection", Editor::cmd_delete_or_change, Key::Char('d')),
    ("change_selection", Editor::cmd_delete_or_change, Key::Char('c')),
    ("delete_selection_noyank", Editor::cmd_delete_or_change, Key::Alt('d')),
    ("change_selection_noyank", Editor::cmd_delete_or_change, Key::Alt('c')),
    ("delete_to_line_end", Editor::cmd_delete_or_change, Key::Char('D')),
    ("yank", Editor::cmd_yank, Key::Char('y')),
    ("paste_after", Editor::cmd_paste, Key::Char('p')),
    ("paste_before", Editor::cmd_paste, Key::Char('P')),
    ("select_inner_object", Editor::cmd_select_object, Key::Char('i')),
    ("select_around_object", Editor::cmd_select_object, Key::Char('a')),
    ("insert_mode", Editor::cmd_insert_mode, Key::Char('i')),
    ("append_mode", Editor::cmd_append_mode, Key::Char('a')),
    ("insert_at_line_start", Editor::cmd_insert_at_line_start, Key::Char('I')),
    ("insert_at_line_end", Editor::cmd_insert_at_line_end, Key::Char('A')),
    ("open_below", Editor::cmd_open_below, Key::Char('o')),
    ("open_above", Editor::cmd_open_above, Key::Char('O')),
    ("undo", Editor::cmd_undo, Key::Char('u')),
    ("undo_line", Editor::cmd_undo_line, Key::Char('U')),
    ("redo", Editor::cmd_redo, Key::Char('U')),
    ("earlier", Editor::cmd_earlier, Key::Alt('u')),
    ("later", Editor::cmd_later, Key::Alt('U')),
    ("goto_column", Editor::cmd_goto_column, Key::Char('|')),
    ("shell_pipe", Editor::cmd_shell_pipe, Key::Char('|')),
    ("shell_insert_output", Editor::cmd_shell_insert_output, Key::Char('!')),
    ("shell_append_output", Editor::cmd_shell_append_output, Key::Alt('!')),
    ("shell_pipe_to", Editor::cmd_shell_pipe_to, Key::Alt('|')),
    ("search", Editor::cmd_search, Key::Char('/')),
    ("rsearch", Editor::cmd_rsearch, Key::Char('?')),
    ("search_next", Editor::cmd_search_next, Key::Char('n')),
    ("search_prev", Editor::cmd_search_prev, Key::Char('N')),
    ("command_mode", Editor::cmd_command_mode, Key::Char(':')),
    ("match_mode", Editor::cmd_match_mode, Key::Char('m')),
    ("replace", Editor::cmd_replace, Key::Char('r')),
    ("select_register", Editor::cmd_select_register, Key::Char('"')),
    ("record_macro", Editor::cmd_record_macro, Key::Char('Q')),
    ("replay_macro", Editor::cmd_replay_macro, Key::Char('q')),
    ("window_mode", Editor::cmd_window_mode, Key::Ctrl('w')),
    ("page_down", Editor::cmd_page_down, Key::Ctrl('f')),
    ("page_up", Editor::cmd_page_up, Key::Ctrl('b')),
    ("join_selections_space", Editor::cmd_join_selections_space, Key::Alt('J')),
    ("save_selection", Editor::cmd_save_selection, Key::Ctrl('s')),
    ("jump_backward", Editor::cmd_jump_backward, Key::Ctrl('o')),
    ("jump_forward", Editor::cmd_jump_forward, Key::Ctrl('i')),
    ("half_page_down", Editor::cmd_half_page_down, Key::Ctrl('d')),
    ("half_page_up", Editor::cmd_half_page_up, Key::Ctrl('u')),
    ("two_thirds_page_down", Editor::cmd_two_thirds_page_down, Key::Ctrl('n')),
    ("two_thirds_page_up", Editor::cmd_two_thirds_page_up, Key::Ctrl('p')),
    ("join_selections", Editor::cmd_join_selections, Key::Char('J')),
    ("flip_selections", Editor::cmd_flip_selections, Key::Alt(';')),
    ("select_all", Editor::cmd_select_all, Key::Char('%')),
    ("vim_redo", Editor::cmd_vim_redo, Key::Ctrl('r')),
    ("delete_char_backward", Editor::cmd_delete_char_backward, Key::Char('X')),
    ("extend_to_line_bounds", Editor::cmd_extend_to_line_bounds, Key::Char('X')),
    ("case_mode", Editor::cmd_case_mode, Key::Char('`')),
    ("switch_case", Editor::cmd_switch_case, Key::Char('~')),
    ("search_selection_detect_word_boundaries", Editor::cmd_search_selection_detect_word_boundaries, Key::Char('*')),
    ("search_selection_exact", Editor::cmd_search_selection_exact, Key::Alt('*')),
    ("search_word_backward", Editor::cmd_search_word_backward, Key::Char('#')),
    ("replace_mode", Editor::cmd_replace_mode, Key::Char('R')),
    ("replace_with_yanked", Editor::cmd_replace_with_yanked, Key::Char('R')),
    ("goto_line", Editor::cmd_goto_line, Key::Char('G')),
    ("indent", Editor::cmd_indent_or_unindent, Key::Char('>')),
    ("unindent", Editor::cmd_indent_or_unindent, Key::Char('<')),
    ("increment", Editor::cmd_increment, Key::Ctrl('a')),
    ("decrement", Editor::cmd_decrement, Key::Ctrl('x')),
    ("vim_repeat_last_change", Editor::cmd_vim_repeat_last_change, Key::Char('.')),
    ("repeat_last_change", Editor::cmd_repeat_last_change, Key::Char('.')),
    ("repeat_last_motion", Editor::cmd_repeat_last_motion, Key::Alt('.')),
];

impl Editor {
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
        let wrap = match cx.key {
            Key::Left => '<',
            Key::Backspace => 'b',
            _ => 'h',
        };
        self.repeat(cx.count, |e| e.step_sideways(motion::prev_grapheme, Some(wrap)))
    }

    pub(super) fn cmd_move_char_right(&mut self, cx: Cx) {
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

    // 全窗裏這兩個也是這一格的兩頭——同 `g h`/`g l`，見 `cell_edge`。
    #[allow(unused_variables)]
    pub(super) fn cmd_goto_line_start(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        let pos = self.cell_edge(false).unwrap_or_else(|| {
            motion::line_start(self.current_buffer().rope(), self.sel.head())
        });
        self.move_head(pos);

    }

    #[allow(unused_variables)]
    pub(super) fn cmd_goto_line_end(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        let pos = self.cell_edge(true).unwrap_or_else(|| {
            motion::line_last(self.current_buffer().rope(), self.sel.head())
        });
        self.move_head(pos);

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
    #[allow(unused_variables)]
    pub(super) fn cmd_goto_window(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        let Key::Char(one) = key else { return };
        self.go_to_screen(one, count);

    }

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
    }

    // **`Z` 是同一層，按完不收**（helix 的 sticky view mode）：`Zjjjj` 一路滾。
    #[allow(unused_variables)]
    pub(super) fn cmd_sticky_view_mode(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.pending = Pending::AimStuck;
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
        // **浮窗先收**（2026-10-01 提的）。它是屏幕上最臨時的那一樣，
        // 而 `Esc` 是人對着臨時的東西按的那一鍵。收的只是「這會兒別擋
        // 着」——挪開光標它又回來，邊欄一動不動。
        if self.hush_the_float() {
            return;
        }
        if self.other_pane().is_some() && self.live_pane() == 0 {
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
        if self.key_preset == yumete_cjk::KeyPreset::Vim && self.extend {
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
        let (anchor, head) = match self.key_preset == yumete_cjk::KeyPreset::Vim
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
        let Key::Char(c) = key else { return };
        self.paste_copies = count.clamp(1, Self::WRITING_MAX);
        self.paste(c == 'p');
        if count > Self::WRITING_MAX {
            self.status = say!("count.writing-ceiling", Self::WRITING_MAX);
        }

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
        if self.key_preset != yumete_cjk::KeyPreset::Vim && ranged && end > pos {
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
        let pos = match self.key_preset == yumete_cjk::KeyPreset::Vim && !self.extend {
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
        if self.key_preset != yumete_cjk::KeyPreset::Vim {
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
            match (written, self.key_preset == yumete_cjk::KeyPreset::Vim) {
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
    }

    // …and the keys a keyboard already has for it. `C-f`/`C-b` are
    // vi's; these are the ones a reader who has never used vi presses.
    // Warning: **浮窗開着的時候它們歸浮窗**（見 `on_key` 開頭，同 helix）。
    #[allow(unused_variables)]
    pub(super) fn cmd_page_down(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.move_page(count, false, 1.0);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_page_up(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.move_page(count, true, 1.0);
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
        self.walk_jumps(true);
    }

    // Warning: **`Tab` 在 Normal 裏也是這一件事**（2026-10-06，對齊 helix 的
    // `jump_forward`）。終端把 `Tab` 和 `C-i` 發成同一個字節，而這個編輯器
    // 分得開（`kitty` 那套協議），於是從前 `Tab` 在 Normal 裏一個 arm 都
    // 没有、靜默掉地——偏偏一個終端一換，手上那個 `C-i` 就變成了它。
    #[allow(unused_variables)]
    pub(super) fn cmd_jump_forward(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.walk_jumps(false);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_half_page_down(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.move_page(count, false, 0.5);
    }

    #[allow(unused_variables)]
    pub(super) fn cmd_half_page_up(&mut self, cx: Cx) {
        let Cx { key, count, operator_count, continuing_zong } = cx;
        self.move_page(count, true, 0.5);
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
            if self.key_preset != yumete_cjk::KeyPreset::Vim {
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
        // vim's `3~` switches three characters (`:h ~`).
        if self.key_preset == yumete_cjk::KeyPreset::Vim && !self.extend && count > 1 {
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
        if self.key_preset == yumete_cjk::KeyPreset::Vim && !self.extend {
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
        // `operator_count`, not `self.count`: the count was taken at
        // the top of this function, so asking `self.count` here always
        // said "no digits" and `1G` went to the *last* line.
        match operator_count.is_some() {
            true => {
                self.remember_jump();
                // 全窗裏數的是表格的行，見 `goto_row`。
                self.goto_grid_row(count);
            }
            false if self.key_preset == yumete_cjk::KeyPreset::Vim => {
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
        let Key::Char(c) = key else { return };
        // vim's `>>` keeps the cursor's column (nvim, `nostartofline`);
        // helix's `>` keeps the selection on its text.
        let kept = (self.key_preset == yumete_cjk::KeyPreset::Vim).then(|| {
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
        "operator_yank", "operator_indent", "operator_unindent", "delete_to_line_end",
        "select_inner_object", "select_around_object", "undo_line", "goto_column", "vim_redo",
        "delete_char_backward", "search_word_backward", "replace_mode", "vim_repeat_last_change",
    ];

    /// The bindable names written before the command list, and the command
    /// each is: a name helix spells differently stays, so no config breaks.
    const OLD_NAMES: &[(&str, &str)] = &[
        ("extend_mode", "select_mode"),
        ("select_line", "extend_line_below"),
        ("join_lines", "join_selections"),
    ];

    #[test]
    fn every_normal_command_has_one_name() {
        let mut seen = std::collections::HashSet::new();
        for (name, _, _) in NORMAL_COMMANDS {
            assert!(seen.insert(*name), "{name} twice");
            assert!(name.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{name}");
        }
        for name in VIM_ONLY {
            assert!(seen.contains(name), "{name} is not a command");
        }
    }

    /// **An action and the command on its key agree on the name** (the
    /// `outline` action pressed `空格 s` for weeks after that key became the
    /// symbol picker, and nothing said so). Every action whose keys are one
    /// top-level key is that key's helix command, by its name or an old one.
    #[test]
    fn the_actions_on_one_key_are_the_commands_on_it() {
        for action in yumete_cjk::actions::ALL {
            let yumete_cjk::actions::How::Keys(keys) = action.how else { continue };
            let key = match keys.chars().collect::<Vec<_>>()[..] {
                [yumete_cjk::actions::ALT, c] => Key::Alt(c),
                [c] => crate::editor::keys::pressed(c),
                _ => continue,
            };
            let helix: Vec<&str> = NORMAL_COMMANDS
                .iter()
                .filter(|(name, _, k)| *k == key && !VIM_ONLY.contains(name))
                .map(|(name, _, _)| *name)
                .collect();
            let wanted = OLD_NAMES.iter().find(|(old, _)| *old == action.name).map_or(action.name, |(_, new)| new);
            assert!(helix.contains(&wanted), "{} is on {keys:?}, whose command is {helix:?}", action.name);
        }
    }
}
