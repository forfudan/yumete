//! **What a key can be bound to, by name** (#429, 2026-09-18).
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
//! ⚠️ **The names are the surface; the defaults still live in the `match`.**
//! An action here says what it *is* and how it is carried out right now — by
//! playing the keys the editor already answers, or by running a command. What
//! this buys today is a stable vocabulary for the config file and the manual;
//! what it does not yet buy is a table the default keys are read from. That is
//! the next tranche, and it can land action by action without any config
//! written against these names having to change.
//!
//! Names follow helix's wherever helix has the same action, so that a reader
//! coming from that editor can guess, and a reader leaving it can search.

/// How an action is carried out, until it is a code path of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// Press these keys, as the alias layer does.
    Keys(&'static str),
    /// Run this command line, without the `:`.
    Command(&'static str),
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

/// **A key sequence, spelled the way the manual spells it** — `空格 f`, not a
/// space and an `f`; `C-o`, not a control byte.
pub fn spell(keys: &str) -> String {
    let mut out = String::new();
    for c in keys.chars() {
        match c {
            ' ' => out.push_str("空格 "),
            '\u{1b}' => out.push_str("Esc"),
            c if (c as u32) < 32 => {
                out.push_str("C-");
                out.push((b'a' + c as u8 - 1) as char);
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

const fn keys(name: &'static str, help: &'static str, k: &'static str) -> Action {
    Action { name, help, how: How::Keys(k) }
}

const fn command(name: &'static str, help: &'static str, c: &'static str) -> Action {
    Action { name, help, how: How::Command(c) }
}

/// **Every action a key may be bound to**, in the order the manual lists them.
///
/// Not every key the editor answers: a key that takes an argument (`f`, `r`,
/// `M`) or opens a group (`g`, `m`, `t`, `空格`) is a *prefix*, and binding a
/// prefix to one action would take its whole group away. Those are left out
/// until the group itself is a named thing.
pub const ALL: &[Action] = &[
    // ---- Moving ---------------------------------------------------------
    keys("move_char_left", "action.move-char-left", "h"),
    keys("move_char_right", "action.move-char-right", "l"),
    keys("move_line_up", "action.move-line-up", "k"),
    keys("move_line_down", "action.move-line-down", "j"),
    keys("move_next_word_start", "action.move-next-word-start", "w"),
    keys("move_prev_word_start", "action.move-prev-word-start", "b"),
    keys("move_next_word_end", "action.move-next-word-end", "e"),
    keys("move_next_long_word_start", "action.move-next-long-word-start", "W"),
    keys("move_prev_long_word_start", "action.move-prev-long-word-start", "B"),
    keys("move_next_long_word_end", "action.move-next-long-word-end", "E"),
    keys("goto_line_start", "action.goto-line-start", "gh"),
    keys("goto_line_end", "action.goto-line-end", "gl"),
    keys("goto_first_nonwhitespace", "action.goto-first-nonwhitespace", "gs"),
    keys("goto_file_start", "action.goto-file-start", "gg"),
    keys("goto_file_end", "action.goto-file-end", "ge"),
    keys("goto_last_line", "action.goto-last-line", "G"),
    keys("goto_prev_sentence", "action.goto-prev-sentence", "H"),
    keys("goto_next_sentence", "action.goto-next-sentence", "L"),
    keys("goto_prev_paragraph", "action.goto-prev-paragraph", "gk"),
    keys("goto_next_paragraph", "action.goto-next-paragraph", "gj"),
    keys("half_page_down", "action.half-page-down", "J"),
    keys("half_page_up", "action.half-page-up", "K"),
    keys("jump_backward", "action.jump-backward", "\u{11}"),
    // ---- Selecting ------------------------------------------------------
    keys("extend_mode", "action.extend-mode", "v"),
    keys("select_line", "action.select-line", "x"),
    keys("extend_to_line_bounds", "action.extend-to-line-bounds", "X"),
    keys("select_all", "action.select-all", "%"),
    keys("collapse_selection", "action.collapse-selection", "\u{1b}"),
    // ---- Changing -------------------------------------------------------
    keys("delete_selection", "action.delete-selection", "d"),
    keys("change_selection", "action.change-selection", "c"),
    keys("cut_selection", "action.cut-selection", "D"),
    keys("cut_and_change", "action.cut-and-change", "C"),
    keys("insert_mode", "action.insert-mode", "i"),
    keys("append_mode", "action.append-mode", "a"),
    keys("insert_at_line_start", "action.insert-at-line-start", "I"),
    keys("insert_at_line_end", "action.insert-at-line-end", "A"),
    keys("open_below", "action.open-below", "o"),
    keys("open_above", "action.open-above", "O"),
    keys("yank", "action.yank", "y"),
    keys("paste_after", "action.paste-after", "p"),
    keys("paste_before", "action.paste-before", "P"),
    keys("replace_with_yanked", "action.replace-with-yanked", "R"),
    keys("join_lines", "action.join-lines", "gJ"),
    keys("indent", "action.indent", ">"),
    keys("unindent", "action.unindent", "<"),
    keys("increment", "action.increment", "\u{1}"),
    keys("decrement", "action.decrement", "\u{18}"),
    keys("switch_case", "action.switch-case", "~"),
    keys("undo", "action.undo", "u"),
    keys("redo", "action.redo", "U"),
    keys("repeat_last_change", "action.repeat-last-change", "."),
    // ---- Looking for something ------------------------------------------
    keys("search", "action.search", "/"),
    keys("rsearch", "action.rsearch", "?"),
    keys("search_next", "action.search-next", "n"),
    keys("search_prev", "action.search-prev", "N"),
    keys("search_selection", "action.search-selection", "g/"),
    keys("goto_definition", "action.goto-definition", "gd"),
    keys("open_here", "action.open-here", "gf"),
    keys("follow_link", "action.follow-link", "gx"),
    // ---- Files, panels, the session --------------------------------------
    keys("file_picker", "action.file-picker", " f"),
    keys("buffer_picker", "action.buffer-picker", " b"),
    keys("next_buffer", "action.next-buffer", "gn"),
    keys("prev_buffer", "action.prev-buffer", "gp"),
    keys("outline", "action.outline", " o"),
    keys("dictionary", "action.dictionary", " d"),
    keys("next_region", "action.next-region", " s"),
    keys("search_panel", "action.search-panel", " /"),
    keys("command_menu", "action.command-menu", "?"),
    keys("paste_menu", "action.paste-menu", "\""),
    keys("comment_line", "action.comment-line", " c"),
    keys("comment_block", "action.comment-block", " C"),
    keys("copy_to_clipboard", "action.copy-to-clipboard", " y"),
    keys("paste_from_clipboard", "action.paste-from-clipboard", " p"),
    keys("ruby", "action.ruby", " r"),
    command("sidebar", "action.sidebar", "sidebar-left"),
    command("wiki_panel", "action.wiki-panel", "wiki panel"),
    command("save_file", "action.save-file", "write"),
    command("save_and_quit", "action.save-and-quit", "wq"),
    command("quit", "action.quit", "quit"),
    command("count_progress", "action.count-progress", "count-progress"),
];
