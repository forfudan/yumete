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
/// Every key is a command now (2026-10-31, step 3 of the keymap plan), so
/// no action presses keys any more: the last of them, the keys inside the
/// groups (`gh`, `空格 f`), became commands of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// Run this command line, without the `:`.
    Command(&'static str),
    /// **A Normal command, by this action's own name** (2026-10-31): the
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

/// **Alt 的內部拼法**：`A-d` 存成這個字 ＋ `d`（2026-10-08）。
///
/// 一串按鍵在這個倉裏是一個 `String`——控制鍵用的是真的控制字節（`C-o` 就是
/// `\u{f}`），而 Alt 没有這樣一個字節。所以借一個私用區的碼位當前綴：存着的時候
/// 一個鍵仍然是一兩個 `char`，`HashMap<String, String>` 那一層一個字都不用改。
///
/// Warning: **不許寫進任何人看得見的地方。** 進來由 [`written`] 翻，出去由 [`spell`]
/// 翻，中間那一段纔是這個字。
pub const ALT: char = '\u{e000}';

/// **有名字的那幾個鍵**：配置裏寫 `<tab>`，表裏存成一個 `char`。
///
/// Warning: **左邊那一串是「一串按鍵」**（`"dj" = …` 是 d 跟 j 兩下），所以鍵名不能
/// 光着寫——`"tab"` 的意思是 t、a、b 三下。尖括號是 vim 的辦法，和一串按鍵永遠
/// 不會撞（2026-10-12 定）。
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
/// `[keys.normal]` 的左邊和預設表都走這一支。從前它不存在，於是配置裏寫
/// `"A-d" = …` 綁的是 **`A`、`-`、`d` 三個鍵連按**——編譯照過、配置照收，而那個鍵
/// 永遠按不出來（2026-10-08 查出來的）。
///
/// Warning: **`C-` 只翻字母。** `C-[`、`C-]` 那幾個在舊終端上和數字鍵撞車（見
/// `Config::language_key` 的註釋），這裏不替它們猜。
pub fn written(spelled: &str) -> String {
    let mut out = String::new();
    let mut rest = spelled;
    // **`<名字>` 先認**（2026-10-12）：認不得的原樣留着，於是一個真的 `<` 照舊是
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
                out.push((next.to_ascii_lowercase() as u8 - b'a' + 1) as char);
                chars = rest;
            }
            _ => out.push(c),
        }
    }
    out
}

/// **A key sequence, spelled the way the manual spells it** — `空格 f`, not a
/// space and an `f`; `C-o`, not a control byte. The inverse of [`written`].
pub fn spell(keys: &str) -> String {
    let mut out = String::new();
    let mut chars = keys.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' => out.push_str("空格 "),
            '\u{1b}' => out.push_str("Esc"),
            // 有名字的那幾個鍵寫回名字——`\t` 畫出來是一段空白，讀的人看不出
            // 那是 Tab（2026-10-12）。
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
            }
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
/// Warning: **keys used to be the *fallback*, and that is what made a typo
/// dangerous.** `x = "delete_slection"` was pressed — d, e, l, e, t, e… straight
/// into the manuscript — because an unknown name fell through to 「then it must
/// be keys」. The guard against it asked whether the name held an underscore,
/// and 2026-10-19 an audit found the hole that leaves: `"zz" = "save"` is a
/// misspelling of `save_file` with no underscore in it, so it went in and typed
/// `s`, `a`, `v`, `e` — the `s` opening the search panel and `ave` landing in
/// it. Fourteen of the eighty names have no underscore either, so the shape of
/// a word cannot answer this.
///
/// So the kind is **said, not guessed** (2026-10-19 作者定). Measured on the two
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

const fn normal(name: &'static str, help: &'static str) -> Action {
    Action { name, help, how: How::Normal }
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
    normal("move_char_left", "action.move-char-left"),
    normal("move_char_right", "action.move-char-right"),
    // **helix's names for what `k`/`j` do** (2026-10-31): these were called
    // `move_line_up`/`move_line_down`, which in helix are `gk`/`gj`, a whole
    // line of the file. The behaviour is the one they always had, on screen.
    normal("move_visual_line_up", "action.move-visual-line-up"),
    normal("move_visual_line_down", "action.move-visual-line-down"),
    normal("move_next_word_start", "action.move-next-word-start"),
    normal("move_prev_word_start", "action.move-prev-word-start"),
    normal("move_next_word_end", "action.move-next-word-end"),
    normal("move_next_long_word_start", "action.move-next-long-word-start"),
    normal("move_prev_long_word_start", "action.move-prev-long-word-start"),
    normal("move_next_long_word_end", "action.move-next-long-word-end"),
    normal("goto_line_start", "action.goto-line-start"),
    normal("goto_line_end", "action.goto-line-end"),
    normal("goto_first_nonwhitespace", "action.goto-first-nonwhitespace"),
    normal("goto_file_start", "action.goto-file-start"),
    normal("goto_file_end", "action.goto-file-end"),
    // **Not `G`** (2026-10-31): under the helix preset a bare `G` does
    // nothing, as in helix, and actions play helix's keys — so this one did
    // nothing at all. `ge` then `gh` is helix's `goto_last_line`: the last
    // line, at its start.
    normal("goto_last_line", "action.goto-last-line"),
    normal("goto_prev_sentence", "action.goto-prev-sentence"),
    normal("goto_next_sentence", "action.goto-next-sentence"),
    // helix's `[p`/`]p` (2026-10-31): these pressed `gk`/`gj`, one file line.
    normal("goto_prev_paragraph", "action.goto-prev-paragraph"),
    normal("goto_next_paragraph", "action.goto-next-paragraph"),
    normal("half_page_down", "action.half-page-down"),
    normal("half_page_up", "action.half-page-up"),
    normal("jump_backward", "action.jump-backward"),
    // ---- Selecting ------------------------------------------------------
    // helix's names (2026-11-01): these were `extend_mode`, `select_line`
    // and `join_lines`, and helix's own names could not be bound.
    normal("select_mode", "action.extend-mode"),
    normal("extend_line_below", "action.select-line"),
    normal("extend_to_line_bounds", "action.extend-to-line-bounds"),
    normal("select_all", "action.select-all"),
    // Warning: **`;`, not Esc** (2026-10-06). Esc used to collapse the selection as
    // well as leave 延伸 mode; it leaves the selection alone now, the way
    // helix's does, and `;` is the key whose whole job is collapsing.
    normal("collapse_selection", "action.collapse-selection"),
    // ---- Changing -------------------------------------------------------
    // Warning: **這四個 2026-10-08 改了名，也改了指向的鍵。**
    //
    // 從前是 `delete_selection`→`d`、`cut_selection`→`D`，而文案說的是「刪除選區
    // （不動寄存器）」「剪切選區（進寄存器）」——**和它們真做的事相反**：`d` 自
    // 2026-09-28 起就進寄存器了，而 `D` 在 helix 鍵位下根本没綁。於是這四行寫進
    // 配置一行都不生效，`::` 那張單子照着四句錯文案列了十天。
    //
    // 名字改成 helix 的（這個模組開頭那條規矩：helix 有同一個動作就用它的名字）。
    // 從前我們同名而意思相反——一個 helix 使用者寫 `d = "delete_selection"` 會得
    // 到和他家裏相反的行為，這是最壞的一種。
    normal("delete_selection", "action.delete-selection"),
    normal("delete_selection_noyank", "action.delete-selection-noyank"),
    normal("change_selection", "action.change-selection"),
    normal("change_selection_noyank", "action.change-selection-noyank"),
    normal("insert_mode", "action.insert-mode"),
    normal("append_mode", "action.append-mode"),
    normal("insert_at_line_start", "action.insert-at-line-start"),
    normal("insert_at_line_end", "action.insert-at-line-end"),
    normal("open_below", "action.open-below"),
    normal("open_above", "action.open-above"),
    normal("yank", "action.yank"),
    normal("paste_after", "action.paste-after"),
    normal("paste_before", "action.paste-before"),
    normal("replace_with_yanked", "action.replace-with-yanked"),
    normal("join_selections", "action.join-lines"),
    normal("indent", "action.indent"),
    normal("unindent", "action.unindent"),
    normal("increment", "action.increment"),
    normal("decrement", "action.decrement"),
    normal("switch_case", "action.switch-case"),
    normal("undo", "action.undo"),
    normal("redo", "action.redo"),
    normal("repeat_last_change", "action.repeat-last-change"),
    // ---- Looking for something ------------------------------------------
    normal("search", "action.search"),
    normal("rsearch", "action.rsearch"),
    normal("search_next", "action.search-next"),
    normal("search_prev", "action.search-prev"),
    normal("search_selection", "action.search-selection"),
    normal("goto_definition", "action.goto-definition"),
    normal("open_here", "action.open-here"),
    normal("follow_link", "action.follow-link"),
    // ---- Files, panels, the session --------------------------------------
    normal("file_picker", "action.file-picker"),
    normal("buffer_picker", "action.buffer-picker"),
    normal("next_buffer", "action.next-buffer"),
    normal("prev_buffer", "action.prev-buffer"),
    // `空格 s` became helix's symbol picker on 2026-10-31; the outline
    // sidebar is a command now.
    command("outline", "action.outline", "sidebar outline"),
    normal("dictionary", "action.dictionary"),
    normal("next_region", "action.next-region"),
    normal("search_panel", "action.search-panel"),
    normal("command_menu", "action.command-menu"),
    normal("paste_menu", "action.paste-menu"),
    normal("comment_line", "action.comment-line"),
    normal("comment_block", "action.comment-block"),
    normal("copy_to_clipboard", "action.copy-to-clipboard"),
    normal("paste_from_clipboard", "action.paste-from-clipboard"),
    command("ruby", "action.ruby", "ruby"),
    command("sidebar", "action.sidebar", "sidebar"),
    command("wiki_panel", "action.wiki-panel", "wiki panel"),
    command("save_file", "action.save-file", "write"),
    command("save_and_quit", "action.save-and-quit", "wq"),
    command("quit", "action.quit", "quit"),
    command("count_progress", "action.count-progress", "count-progress"),
    // ---- Every other Normal key (2026-10-31): each a command by name ----
    normal("move_line_up", "action.move-line-up"),
    normal("move_line_down", "action.move-line-down"),
    normal("goto_next_line_nonwhitespace", "action.goto-next-line-nonwhitespace"),
    normal("jump_next_word_start", "action.jump-next-word-start"),
    normal("jump_next_long_word_start", "action.jump-next-long-word-start"),
    normal("jump_prev_word_start", "action.jump-prev-word-start"),
    normal("jump_prev_long_word_start", "action.jump-prev-long-word-start"),
    normal("jump_next_word_end", "action.jump-next-word-end"),
    normal("jump_next_long_word_end", "action.jump-next-long-word-end"),
    normal("jump_next_paragraph", "action.jump-next-paragraph"),
    normal("jump_prev_paragraph", "action.jump-prev-paragraph"),
    normal("move_next_paragraph", "action.move-next-paragraph"),
    normal("move_prev_paragraph", "action.move-prev-paragraph"),
    normal("goto_window_top", "action.goto-window-top"),
    normal("goto_window_center", "action.goto-window-center"),
    normal("goto_window_bottom", "action.goto-window-bottom"),
    normal("jump_prev_sentence", "action.jump-prev-sentence"),
    normal("jump_next_sentence", "action.jump-next-sentence"),
    normal("add_mark", "action.add-mark"),
    normal("goto_mark", "action.goto-mark"),
    normal("view_mode", "action.view-mode"),
    normal("sticky_view_mode", "action.sticky-view-mode"),
    normal("right_bracket_mode", "action.right-bracket-mode"),
    normal("left_bracket_mode", "action.left-bracket-mode"),
    normal("goto_mode", "action.goto-mode"),
    normal("space_mode", "action.space-mode"),
    normal("find_next_char", "action.find-next-char"),
    normal("find_prev_char", "action.find-prev-char"),
    normal("find_till_char", "action.find-till-char"),
    normal("till_prev_char", "action.till-prev-char"),
    normal("select_line_mode", "action.select-line-mode"),
    normal("normal_mode", "action.normal-mode"),
    normal("repeat_find_char", "action.repeat-find-char"),
    normal("repeat_find_char_reverse", "action.repeat-find-char-reverse"),
    normal("copy_selection_on_next_line", "action.copy-selection-on-next-line"),
    normal("copy_selection_on_prev_line", "action.copy-selection-on-prev-line"),
    normal("keep_primary_selection", "action.keep-primary-selection"),
    normal("remove_primary_selection", "action.remove-primary-selection"),
    normal("merge_selections", "action.merge-selections"),
    normal("merge_consecutive_selections", "action.merge-consecutive-selections"),
    normal("ensure_selections_forward", "action.ensure-selections-forward"),
    normal("shrink_to_line_bounds", "action.shrink-to-line-bounds"),
    normal("split_selection_on_newline", "action.split-selection-on-newline"),
    normal("rotate_selections_forward", "action.rotate-selections-forward"),
    normal("rotate_selections_backward", "action.rotate-selections-backward"),
    normal("trim_selections", "action.trim-selections"),
    normal("align_selections", "action.align-selections"),
    normal("rotate_selection_contents_forward", "action.rotate-selection-contents-forward"),
    normal("rotate_selection_contents_backward", "action.rotate-selection-contents-backward"),
    normal("select_regex", "action.select-regex"),
    normal("split_selection", "action.split-selection"),
    normal("keep_selections", "action.keep-selections"),
    normal("remove_selections", "action.remove-selections"),
    normal("operator_delete", "action.operator-delete"),
    normal("operator_change", "action.operator-change"),
    normal("operator_yank", "action.operator-yank"),
    normal("operator_indent", "action.operator-indent"),
    normal("operator_unindent", "action.operator-unindent"),
    normal("delete_to_line_end", "action.delete-to-line-end"),
    normal("select_inner_object", "action.select-inner-object"),
    normal("select_around_object", "action.select-around-object"),
    normal("undo_line", "action.undo-line"),
    normal("earlier", "action.earlier"),
    normal("later", "action.later"),
    normal("goto_column", "action.goto-column"),
    normal("shell_pipe", "action.shell-pipe"),
    normal("shell_insert_output", "action.shell-insert-output"),
    normal("shell_append_output", "action.shell-append-output"),
    normal("shell_pipe_to", "action.shell-pipe-to"),
    normal("command_mode", "action.command-mode"),
    normal("match_mode", "action.match-mode"),
    normal("replace", "action.replace"),
    normal("select_register", "action.select-register"),
    normal("record_macro", "action.record-macro"),
    normal("replay_macro", "action.replay-macro"),
    normal("window_mode", "action.window-mode"),
    normal("page_down", "action.page-down"),
    normal("page_up", "action.page-up"),
    normal("join_selections_space", "action.join-selections-space"),
    normal("save_selection", "action.save-selection"),
    normal("jump_forward", "action.jump-forward"),
    normal("two_thirds_page_down", "action.two-thirds-page-down"),
    normal("two_thirds_page_up", "action.two-thirds-page-up"),
    normal("flip_selections", "action.flip-selections"),
    normal("vim_redo", "action.vim-redo"),
    normal("delete_char_backward", "action.delete-char-backward"),
    normal("case_mode", "action.case-mode"),
    normal("search_selection_detect_word_boundaries", "action.search-selection-detect-word-boundaries"),
    normal("search_selection_exact", "action.search-selection-exact"),
    normal("search_word_backward", "action.search-word-backward"),
    normal("replace_mode", "action.replace-mode"),
    normal("goto_line", "action.goto-line"),
    normal("vim_repeat_last_change", "action.vim-repeat-last-change"),
    normal("repeat_last_motion", "action.repeat-last-motion"),
    // ---- The groups' keys (2026-10-31): `g`, `Space`, `[` `]`, `z`, `m`,
    // `C-w`, the backtick group, `Space t` and the rest, each by name ----
    normal("jump_prev_word_end", "action.jump-prev-word-end"),
    normal("jump_prev_long_word_end", "action.jump-prev-long-word-end"),
    normal("goto_word", "hint.goto.jump-labels"),
    normal("goto_word_by_letters", "hint.goto.seek-written"),
    normal("goto_char_by_pinyin", "hint.goto.seek-reading"),
    normal("goto_last_accessed_file", "hint.goto.last-file"),
    normal("goto_definition_other_window", "hint.goto.follow-note-elsewhere"),
    normal("goto_reference", "hint.goto.references"),
    normal("search_selection_other_window", "hint.goto.search-word-elsewhere"),
    normal("move_next_zong", "action.move-next-zong"),
    normal("move_prev_zong", "action.move-prev-zong"),
    normal("goto_last_modification", "hint.goto.last-change"),
    normal("reselect_last_selection", "hint.goto.reselect"),
    normal("insert_at_last_insert", "hint.goto.insert-again"),
    normal("goto_older_change", "action.goto-older-change"),
    normal("goto_newer_change", "action.goto-newer-change"),
    normal("operator_lowercase", "action.operator-lowercase"),
    normal("operator_uppercase", "action.operator-uppercase"),
    normal("operator_switch_case", "action.operator-switch-case"),
    normal("operator_fill", "hint.goto.fill"),
    normal("join_selections_no_space", "hint.goto.join-without-a-space"),
    normal("file_picker_in_current_directory", "hint.space.open-file-here"),
    normal("file_explorer", "hint.space.explorer"),
    normal("file_explorer_in_current_buffer_directory", "hint.space.explorer-here"),
    normal("changed_file_picker", "hint.space.changed-files"),
    normal("paste_clipboard_before", "hint.space.paste-before"),
    normal("replace_selections_with_clipboard", "hint.space.replace-from-clipboard"),
    normal("symbol_picker", "hint.space.symbols"),
    normal("workspace_symbol_picker", "hint.space.project-symbols"),
    normal("dictionary_panel", "hint.space.dictionary-panel"),
    normal("hover", "hint.space.what-is-this"),
    normal("hover_panel", "hint.space.what-is-this-panel"),
    normal("diagnostics_picker", "hint.space.diagnostics"),
    normal("workspace_diagnostics_picker", "hint.space.project-diagnostics"),
    normal("show_diagnostic", "hint.space.problems"),
    normal("show_diagnostic_panel", "hint.space.problems-panel"),
    normal("show_change", "hint.space.change"),
    normal("show_change_panel", "hint.space.change-panel"),
    normal("rename_symbol", "hint.space.rename"),
    normal("code_action", "hint.space.actions"),
    normal("show_record", "hint.space.record"),
    normal("show_record_panel", "hint.space.record-panel"),
    normal("table_mode", "hint.space.table"),
    normal("conflict_mode", "hint.conflict.title"),
    normal("open_recent_mode", "hint.space.open-recent"),
    normal("yume_mode", "hint.space.yume"),
    normal("goto_next_conflict", "action.goto-next-conflict"),
    normal("goto_prev_conflict", "action.goto-prev-conflict"),
    normal("goto_next_change", "action.goto-next-change"),
    normal("goto_prev_change", "action.goto-prev-change"),
    normal("goto_next_diag", "action.goto-next-diag"),
    normal("goto_prev_diag", "action.goto-prev-diag"),
    normal("goto_last_diag", "action.goto-last-diag"),
    normal("goto_first_diag", "action.goto-first-diag"),
    normal("goto_next_function", "action.goto-next-function"),
    normal("goto_prev_function", "action.goto-prev-function"),
    normal("goto_next_class", "action.goto-next-class"),
    normal("goto_prev_class", "action.goto-prev-class"),
    normal("goto_next_comment", "action.goto-next-comment"),
    normal("goto_prev_comment", "action.goto-prev-comment"),
    normal("add_newline_below", "hint.hop.blank-below"),
    normal("add_newline_above", "hint.hop.blank-above"),
    normal("align_view_center", "action.align-view-center"),
    normal("align_view_top", "action.align-view-top"),
    normal("align_view_bottom", "action.align-view-bottom"),
    normal("scroll_down", "action.scroll-down"),
    normal("scroll_up", "action.scroll-up"),
    normal("match_brackets", "hint.match.pair"),
    normal("select_textobject_inner", "hint.match.inside"),
    normal("select_textobject_around", "hint.match.around"),
    normal("surround_add", "hint.match.surround"),
    normal("surround_delete", "hint.match.take-off"),
    normal("surround_replace", "hint.change"),
    normal("jump_view_left", "action.jump-view-left"),
    normal("jump_view_down", "action.jump-view-down"),
    normal("jump_view_up", "action.jump-view-up"),
    normal("jump_view_right", "action.jump-view-right"),
    normal("toggle_left_sidebar", "action.toggle-left-sidebar"),
    normal("toggle_right_sidebar", "action.toggle-right-sidebar"),
    normal("goto_left_sidebar", "action.goto-left-sidebar"),
    normal("goto_right_sidebar", "action.goto-right-sidebar"),
    normal("hsplit", "hint.region.split"),
    normal("vsplit", "hint.region.split-beside"),
    normal("hsplit_new", "hint.region.new-below"),
    normal("vsplit_new", "hint.region.new-beside"),
    normal("window_new_mode", "action.window-new-mode"),
    normal("wclose", "hint.region.close"),
    normal("wonly", "hint.region.only"),
    normal("switch_to_lowercase", "hint.case.lower"),
    normal("switch_to_uppercase", "hint.case.upper"),
    normal("convert_to_simplified", "hint.case.to-s"),
    normal("convert_to_traditional", "hint.case.to-t"),
    normal("convert_to_taiwan_traditional", "hint.case.to-tw"),
    normal("convert_to_hong_kong_traditional", "hint.case.to-hk"),
    normal("convert_to_mainland_traditional", "hint.case.to-c"),
    normal("convert_to_classical_traditional", "hint.case.to-g"),
    normal("convert_to_japanese", "hint.case.to-jp"),
    normal("switch_to_halfwidth", "hint.case.to-half"),
    normal("switch_to_fullwidth", "hint.case.to-full"),
    normal("conflict_keep_ours", "hint.conflict.ours"),
    normal("conflict_keep_theirs", "hint.conflict.theirs"),
    normal("conflict_keep_both", "hint.conflict.both"),
    normal("recent_file_picker", "hint.open.recent-files"),
    normal("global_recent_file_picker", "hint.open.recent-files-global"),
    normal("project_picker", "hint.open.recent-projects"),
    normal("resume_session", "hint.open.resume"),
    normal("yume_toggle", "hint.yume.toggle"),
    normal("yume_commit_mode", "hint.yume.commit"),
    normal("yume_panel_mode", "hint.yume.panel"),
    normal("yume_preedit_mode", "hint.yume.preedit"),
    normal("yume_toggle_prediction", "hint.yume.predict"),
    normal("yume_pick_scheme", "hint.yume.scheme"),
    normal("yume_next_scheme", "hint.yume.next-scheme"),
    normal("yume_menu_size_3", "action.yume-menu-size-3"),
    normal("yume_menu_size_4", "action.yume-menu-size-4"),
    normal("yume_menu_size_5", "action.yume-menu-size-5"),
    normal("yume_menu_size_6", "action.yume-menu-size-6"),
    normal("yume_menu_size_7", "action.yume-menu-size-7"),
    normal("yume_menu_size_8", "action.yume-menu-size-8"),
    normal("yume_menu_size_9", "action.yume-menu-size-9"),
    normal("yume_toggle_chaifen", "hint.yume.chaifen"),
    normal("yume_installed_table", "hint.yume.table"),
    normal("yume_table_source", "hint.yume.which"),
    normal("yume_data_location", "hint.yume.where"),
    normal("yume_commit_default", "hint.yume.commit-default"),
    normal("yume_commit_delayed", "hint.yume.commit-delayed"),
    normal("yume_commit_unique", "hint.yume.commit-unique"),
    normal("yume_commit_fluency", "hint.yume.commit-fluency"),
    normal("yume_panel_box", "hint.yume.panel-box"),
    normal("yume_panel_bottom", "hint.yume.panel-bottom"),
    normal("yume_panel_inline", "hint.yume.panel-inline"),
    normal("yume_preedit_off", "hint.yume.preedit-off"),
    normal("yume_preedit_composition", "hint.yume.preedit-composition"),
    normal("yume_preedit_preview", "hint.yume.preedit-preview"),
    normal("table_source_view", "hint.table.back-to-prose"),
    normal("table_basic_view", "hint.table.operate-it"),
    normal("table_window_view", "hint.table.whole-window"),
    normal("table_leave_window", "hint.table.leave-the-window"),
    normal("goto_next_table", "action.goto-next-table"),
    normal("goto_prev_table", "action.goto-prev-table"),
    normal("table_convert_mode", "hint.table.convert"),
    normal("table_cell_width", "hint.table.fold-wide-cells"),
    normal("table_search_columns", "hint.table.search-columns"),
    normal("table_search_columns_other_window", "hint.table.search-columns-elsewhere"),
    normal("table_goto_cell", "hint.table.go-to-cell"),
    normal("table_yank_column", "action.table-yank-column"),
    normal("table_replace_column", "hint.table.replace-column"),
    normal("table_paste_column_right", "action.table-paste-column-right"),
    normal("table_paste_column_left", "action.table-paste-column-left"),
    normal("table_add_row_below", "action.table-add-row-below"),
    normal("table_add_row_above", "action.table-add-row-above"),
    normal("table_add_column_right", "action.table-add-column-right"),
    normal("table_add_column_left", "action.table-add-column-left"),
    normal("table_delete_row", "hint.table.delete-row"),
    normal("table_delete_column", "action.table-delete-column"),
    normal("table_move_row_down", "action.table-move-row-down"),
    normal("table_move_row_up", "action.table-move-row-up"),
    normal("table_move_column_left", "action.table-move-column-left"),
    normal("table_move_column_right", "action.table-move-column-right"),
    normal("table_align_left", "action.table-align-left"),
    normal("table_align_center", "action.table-align-center"),
    normal("table_align_right", "action.table-align-right"),
    normal("table_line_up", "hint.table.line-it-up"),
    normal("table_record_panel", "hint.table.detail-panel"),
    normal("table_first_row_is_data", "hint.table.first-row-is-data"),
    normal("table_sort_ascending", "action.table-sort-ascending"),
    normal("table_sort_descending", "action.table-sort-descending"),
    normal("convert_table_to_pipe", "action.convert-table-to-pipe"),
    normal("paste_table_as_pipe", "action.paste-table-as-pipe"),
    normal("convert_table_to_csv", "action.convert-table-to-csv"),
    normal("paste_table_as_csv", "action.paste-table-as-csv"),
    normal("convert_table_to_tsv", "action.convert-table-to-tsv"),
    normal("paste_table_as_tsv", "action.paste-table-as-tsv"),
    normal("no_op", "action.no-op"),
    // ---- Select mode (2026-10-31): each a motion, extending the selection ----
    normal("extend_char_left", "action.extend-char-left"),
    normal("extend_char_right", "action.extend-char-right"),
    normal("extend_visual_line_up", "action.extend-visual-line-up"),
    normal("extend_visual_line_down", "action.extend-visual-line-down"),
    normal("extend_line_up", "action.extend-line-up"),
    normal("extend_line_down", "action.extend-line-down"),
    normal("extend_next_word_start", "action.extend-next-word-start"),
    normal("extend_prev_word_start", "action.extend-prev-word-start"),
    normal("extend_next_word_end", "action.extend-next-word-end"),
    normal("extend_next_long_word_start", "action.extend-next-long-word-start"),
    normal("extend_prev_long_word_start", "action.extend-prev-long-word-start"),
    normal("extend_next_long_word_end", "action.extend-next-long-word-end"),
    normal("extend_search_next", "action.extend-search-next"),
    normal("extend_search_prev", "action.extend-search-prev"),
    normal("extend_till_char", "action.extend-till-char"),
    normal("extend_next_char", "action.extend-next-char"),
    normal("extend_till_prev_char", "action.extend-till-prev-char"),
    normal("extend_prev_char", "action.extend-prev-char"),
    normal("extend_to_line_start", "action.extend-to-line-start"),
    normal("extend_to_line_end", "action.extend-to-line-end"),
    normal("extend_to_file_start", "action.extend-to-file-start"),
    normal("extend_to_file_end", "action.extend-to-file-end"),
    normal("extend_to_word", "action.extend-to-word"),
    // ---- Insert mode (2026-10-31): each key a command by name ----
    normal("insert_newline", "action.insert-newline"),
    normal("delete_char_forward", "action.delete-char-forward"),
    normal("delete_word_forward", "action.delete-word-forward"),
    normal("delete_word_backward", "action.delete-word-backward"),
    normal("kill_to_line_start", "action.kill-to-line-start"),
    normal("kill_to_line_end", "action.kill-to-line-end"),
    normal("insert_register", "action.insert-register"),
    normal("run_one_normal_command", "action.run-one-normal-command"),
    normal("goto_line_end_newline", "action.goto-line-end-newline"),
    normal("commit_undo_checkpoint", "action.commit-undo-checkpoint"),
    normal("insert_tab", "action.insert-tab"),
    normal("insert_other_tab", "action.insert-other-tab"),
    // ---- vim's own versions (2026-11-01): the vim keymap's keys run these ----
    normal("vim_normal_mode", "action.vim-normal-mode"),
    normal("vim_goto_line_end", "action.vim-goto-line-end"),
    normal("vim_insert_mode", "action.vim-insert-mode"),
    normal("vim_append_mode", "action.vim-append-mode"),
    normal("vim_insert_at_line_start", "action.vim-insert-at-line-start"),
    normal("vim_insert_at_line_end", "action.vim-insert-at-line-end"),
    normal("vim_open_below", "action.vim-open-below"),
    normal("vim_open_above", "action.vim-open-above"),
    normal("vim_join_selections", "action.vim-join-selections"),
    normal("vim_switch_case", "action.vim-switch-case"),
    normal("vim_goto_line", "action.vim-goto-line"),
    normal("vim_indent", "action.vim-indent"),
    normal("vim_unindent", "action.vim-unindent"),
    normal("vim_paste_after", "action.vim-paste-after"),
    normal("vim_paste_before", "action.vim-paste-before"),
    normal("vim_increment", "action.vim-increment"),
    normal("vim_decrement", "action.vim-decrement"),
    normal("vim_search_next", "action.vim-search-next"),
    normal("vim_search_prev", "action.vim-search-prev"),
    normal("vim_match_brackets", "action.vim-match-brackets"),
    normal("vim_kill_to_line_start", "action.vim-kill-to-line-start"),
    normal("vim_kill_to_line_end", "action.vim-kill-to-line-end"),
    normal("vim_flip_selections", "action.vim-flip-selections"),
    normal("vim_switch_to_lowercase", "action.vim-switch-to-lowercase"),
    normal("vim_switch_to_uppercase", "action.vim-switch-to-uppercase"),
    normal("vim_yank", "action.vim-yank"),
    normal("vim_visual_block_mode", "action.vim-visual-block-mode"),
];

/// **The three places that take keys and are not modes of the text**
/// (2026-11-01, step 9 of the keymap plan): a prompt (`:`, `/`, `::`, a
/// reading, and a picker's query), a picker's list (and the `::` palette's),
/// and a sidebar. helix hard-codes these keys; here each place has a table
/// in the config (`[keys.prompt]`, `[keys.picker]`, `[keys.sidebar]`) and
/// commands of its own.
///
/// A name the text's keymap has too (`move_char_left`, `page_down`) is
/// reused where it means the same thing in that place, so one name is one
/// thing to do wherever it is run. A picker takes the prompt's keys as well,
/// its own winning where both name a key (helix's rule). Nothing comes over
/// from `[keys.normal]` or `[keys.insert]`: a sidebar's `J` pages and its `g`
/// goes to the top at once, so the two keymaps already differ.
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

/// **Every command of the four places, named once** (2026-11-01, the
/// review's decoupling): the config and the docs read the names, the
/// editor matches on the variants, so a misspelt name in a handler no
/// longer compiles into an arm that never runs.
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

    /// **No two actions share a name, or a command line** (2026-10-30, when
    /// `next_region` pressed `outline`'s keys and binding it opened the
    /// outline: a second name for one thing is a slip, never a feature).
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
    /// own names (2026-11-01).
    #[test]
    fn every_place_action_is_its_own() {
        for place in Place::ALL {
            let mut names = std::collections::HashSet::new();
            for one in place.actions() {
                assert!(names.insert(one.name), "`{}` twice in {}", one.name, place.name());
            }
        }
        assert_eq!(Place::Picker.bound("save_file"), None, "a Normal command is not a picker's");
        // Every command the enum declares is some place's, so none is dead.
        for cmd in PlaceCmd::ALL {
            let used = Place::ALL.iter().any(|p| p.actions().iter().any(|a| a.name == cmd.name()));
            assert!(used, "{cmd:?} is in no place");
        }
        assert!(Place::Sidebar.bound("no_op").is_some());
        assert!(Place::Prompt.bound(":write").is_some());
    }
}
