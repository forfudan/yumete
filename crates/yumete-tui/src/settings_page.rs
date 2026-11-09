//! **那扇設置面板，畫出來** —— Feature #421（2026-09-24）；2026-11-08 重做。
//!
//! 狀態在 [`yumete_config::panel`]（那一頭讀得到設置表，核心讀不到）；這裏只管
//! 把它畫成一頁，和把鍵交給它。
//!
//! ```text
//!  設置  全局  本項目                            ~/.config/yumete/config.toml
//!  ────────────┬──────────────────────────────────────────────────────────────
//!   版面       │ 版面
//!   標記       │   排版方向          [橫排]                              出廠
//!   界面       │ * 每縱字數           32                                 全局
//!  ────────────┴──────────────────────────────────────────────────────────────
//!              │  一縱寫幾個字；0 ＝與窗口同高
//!              │  數字，0，或 8 到 64；出廠 0
//!   jk 上下, / 搜索, Enter 修改, ␣ 切換, r 重置, u 撤銷 …
//! ```
//!
//! **一個鍵在每一行上只有一個意思**（2026-11-08 作者定，審出來的毛病一半是同一個鍵
//! 在不同的行上做不同的事）。改一個值打開的是和 `:` 命令行同一條單行編輯器
//! （[`yumete_core::input::Mode::Value`]），每一次確認的改動當場寫進檔裏並生效，
//! `u` 反悔。

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::widgets::Clear;
use ratatui::Frame;

use yumete_config::panel::{
    all_rows, FlavorRow, FlavorSection, ItemFrom, Layer, Panel, Refusal, SaveError, Saved, Subpage, Typing, Which,
};
use yumete_config::settings_ui::{Kind, Setting, GROUPS};
use yumete_config::Config;
use yumete_core::input::Key;
use yumete_core::say;
use yumete_core::Editor;

/// **`say!`，但標籤是個變量。**
///
/// Warning: `say!` 只收字面量（那是有意的：`messages.rs` 那張「每個標籤都有條目」的網
/// 是讀源碼找的，一個算出來的標籤它看不見）。這一扇面板的標籤**全部**來自
/// `settings_ui` 那張表，所以它們是變量——而那張網照樣看得見，因為它們在表上寫成
/// `label:` 與 `hint:`。
fn said(tag: &str) -> String {
    yumete_core::messages::say(tag, &[])
}

/// **A message in every language it is written in** (2026-11-09): a
/// setting's name typed after `:settings` is found in the script it is
/// typed in, whichever script the panel is drawn in.
fn every_name(tag: &str) -> Vec<String> {
    match yumete_core::messages::table().get(tag) {
        Some(entry) => [entry.zht, entry.zhs, entry.en]
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        None => vec![said(tag)],
    }
}

/// **What closes the panel after a `:`** (2026-11-09): the habit of closing
/// with a command, which here writes nothing more — every change is written
/// already.
const CLOSING: &[&str] = &["q", "q!", "wq", "wq!", "x"];

/// **The setting a `:settings <name>` typed on the panel's `:` line names**
/// (2026-11-09, reported: it was 「not a command」), or `:set <name>` —
/// empty when none is named; `None` when it is not that command.
fn setting_named(typed: &str) -> Option<&str> {
    let (word, rest) = typed.split_once(char::is_whitespace).unwrap_or((typed, ""));
    matches!(word, "settings" | "set" | "se").then(|| rest.trim())
}

use crate::put_text;

/// 左邊那張組目録有多寬（全角格）。
const GROUPS_WIDE: u16 = 14;

/// **一段話折成幾行**，按顯示寬度（2026-10-13）。
///
/// 中文可以在任何一個字之間斷，所以貪心地裝滿一行就斷；可是**空格那一種斷點更好
/// 讀**——鍵那一行是一組一組的，斷在組裏面會把一組拆成兩半。所以裝滿之後往回找最
/// 近的那個空格（只找這一行的後四分之一），找得到就斷在那裏。
fn fold(text: &str, wide: usize) -> Vec<String> {
    if wide == 0 {
        return Vec::new();
    }
    let mut rows: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut at = 0usize;
    // Punctuation that may not open a line (禁則): it hangs at the end of
    // the line before instead (2026-11-02).
    let closes = |g: &str| matches!(g, "，" | "。" | "、" | "；" | "：" | "！" | "？" | "）" | "」" | "』" | "》" | "〉" | "】" | "," | "." | ";" | ":" | ")");
    for g in yumete_cjk::graphemes(text) {
        let w = yumete_cjk::grapheme_width(g).max(1);
        if at + w > wide && closes(g) && !line.is_empty() {
            line.push_str(g);
            rows.push(std::mem::take(&mut line));
            at = 0;
            continue;
        }
        if at + w > wide {
            match line.rfind(' ').filter(|i| *i * 4 >= wide * 3) {
                Some(cut) => {
                    let rest = line.split_off(cut);
                    rows.push(std::mem::take(&mut line));
                    line = rest.trim_start().to_string();
                    at = yumete_cjk::str_width(&line);
                }
                None => {
                    rows.push(std::mem::take(&mut line));
                    at = 0;
                }
            }
        }
        line.push_str(g);
        at += w;
    }
    if !line.is_empty() {
        rows.push(line);
    }
    rows
}

/// **右邊那幾欄從哪裏開始**。
///
/// Warning: **量出來的，不是寫死的**（2026-09-24），而且**量的是檔與出廠說的，不是
/// 正在改的**（2026-11-08，報的：長的出廠字體一換成短的，整組的值那一欄從第 44
/// 格跳到第 26 格）。
struct Columns {
    /// 名字那一欄從哪裏開始。
    name: u16,
    /// 值那一欄。
    value: u16,
    /// 頁的右邊。
    right: u16,
}

impl Columns {
    fn of(area: Rect, panel: &Panel) -> Columns {
        let right = area.x + area.width;
        let name = area.x + GROUPS_WIDE + 2;
        let room = right.saturating_sub(name).saturating_sub(SOURCE_WIDE);
        let rows = all_rows();
        let widest = |it: &mut dyn Iterator<Item = String>| it.map(|t| yumete_cjk::str_width(&t) as u16).max().unwrap_or(0);
        let want_name = widest(&mut rows.iter().map(|s| said(s.label))) + 2;
        let want_value = widest(&mut rows.iter().flat_map(|s| {
            let path = s.path();
            let said = [Some(s.factory.to_string()), panel.global.declared(&path).cloned(), panel.local.declared(&path).cloned()];
            said.into_iter().flatten().map(move |v| row_value(panel, s, &v))
        }));
        let name_wide = match want_name + want_value <= room {
            true => want_name,
            // `max` then `min`, not `clamp`: under 22 columns `room` is below
            // six, and `clamp(6, room)` panics (2026-10-30).
            false => room.saturating_sub(want_value.min(room / 2)).max(6).min(room),
        };
        Columns { name, value: name + name_wide, right }
    }
}

/// How much of a row's right end the source of its value may take.
const SOURCE_WIDE: u16 = 18;

/// What the mouse did, as the panel takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    /// The left button went down.
    Click,
    /// The wheel turned, so many notches, up (`back`) or down.
    Wheel { back: bool, notches: usize },
}

/// What a cell of the last panel drawn was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Group(usize),
    Row(usize),
    PageRow(usize),
    Choice(usize),
}

thread_local! {
    /// **Where the last panel drawn put each line it can be clicked on**:
    /// `(y, from x, to x, what)`. The layout is worked out while drawing, so a
    /// click asks what was drawn rather than working it out a second time.
    static HITS: std::cell::RefCell<Vec<(u16, u16, u16, Hit)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn hit(y: u16, from: u16, to: u16, what: Hit) {
    HITS.with(|hits| hits.borrow_mut().push((y, from, to, what)));
}

fn hit_at(x: u16, y: u16) -> Option<Hit> {
    HITS.with(|hits| {
        hits.borrow().iter().find(|(at, from, to, _)| *at == y && (*from..*to).contains(&x)).map(|(_, _, _, what)| *what)
    })
}

/// **A theme tried while its list is open**, for the loop to draw: the one
/// under the cursor, back to what it was, or kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    Show(String),
    Undo,
    Keep,
}

/// **那扇面板在前端的一個座位** —— 開、收鍵、寫、關，一整圈。
///
/// Warning: **這一支存在的理由是「別寫兩遍」。** 主循環（`run`）與 `--keys`
/// （`yumete/src/main.rs`）都走同一套。
#[derive(Default)]
pub struct Seat {
    /// 開着的那一扇；`None` ＝ 沒開。
    pub panel: Option<Panel>,
    /// **`"` and the register it named**, for the `p` after it, as in the
    /// editor: `Some(None)` is a `"` waiting for its name.
    register: Option<Option<char>>,
    /// The first `g` of `gg`.
    g_pending: bool,
    /// Where `"+p` reads the system clipboard from; `None` is the machine's
    /// own (`pbpaste` and its kin). A test hands in its own.
    pub clipboard: Option<fn() -> Option<String>>,
    /// The status line holds what the panel said about a row: it goes when
    /// the cursor or a value moves ([`Seat::say`]).
    row_said: bool,
    /// A theme to try, put back or keep, for the loop ([`Seat::take_preview`]).
    preview: Option<Preview>,
    /// A theme is being tried on the screen.
    trying: bool,
    /// **A file was found changed outside, and nobody may have read so**
    /// (2026-11-09): the key that found it opened the value line over the
    /// news. The write that follows says it instead.
    outside: bool,
}

impl Seat {
    /// **那扇窗開着嗎？** 開着的時候一個鍵都到不了正文。
    pub fn open(&self) -> bool {
        self.panel.is_some()
    }

    /// The theme to try or put back, once.
    pub fn take_preview(&mut self) -> Option<Preview> {
        self.preview.take()
    }

    /// **這一鍵歸面板嗎？** 歸就收下並回 `true`。
    ///
    /// While a value or the search is typed the keys are the command row's —
    /// the `:` line's keys and the IME — and this gives them back.
    pub fn took(&mut self, editor: &mut Editor, key: Option<Key>) -> bool {
        if self.panel.is_none() {
            return false;
        }
        let noticed = self.notice(editor);
        if editor.mode().is_prompt() {
            return false;
        }
        // Said, and the key after it came in plain sight of it.
        if !noticed {
            self.outside = false;
        }
        let Some(page) = self.panel.as_ref() else { return false };
        let before = page.spot();
        let said = self.take(editor, key);
        // The key that found a file changed outside does not take the news
        // away again by moving.
        if !(noticed && said.is_none()) {
            self.say(editor, before, said);
        }
        true
    }

    /// **A file changed outside, read again and said once** (2026-11-09):
    /// asked before every key, click and paste, and on the loop's clock
    /// while the panel stands open ([`Seat::due_in`]).
    /// Gives whether it said so, for the key that found it to leave the line.
    fn notice(&mut self, editor: &mut Editor) -> bool {
        let Some(page) = self.panel.as_mut() else { return false };
        let moved = page.refresh();
        if moved.is_empty() {
            return false;
        }
        let names = moved.iter().map(|p| shorten(p)).collect::<Vec<_>>().join(&say!("label.comma"));
        editor.set_status(say!("settings.changed-outside", names));
        self.row_said = true;
        self.outside = true;
        true
    }

    /// **The loop's clock**: how long until the files are looked at again,
    /// while the panel is open.
    pub fn due_in(&self) -> Option<std::time::Duration> {
        self.open().then_some(std::time::Duration::from_secs(1))
    }

    /// The clock struck: a file changed outside is read again.
    pub fn watch(&mut self, editor: &mut Editor) {
        let _ = self.notice(editor);
    }

    /// **What a key said, or the stale line taken away** (2026-11-08,
    /// reported: 「本層没有設這一項」 stayed after the row was set). A line
    /// the panel said about a row lasts until the cursor or a value moves.
    fn say(&mut self, editor: &mut Editor, before: yumete_config::panel::Spot, said: Option<String>) {
        let Some(page) = self.panel.as_ref() else {
            self.row_said = false;
            return;
        };
        match said {
            Some(said) => {
                editor.set_status(said);
                self.row_said = true;
            }
            None if self.row_said && page.spot() != before => {
                editor.set_status(String::new());
                self.row_said = false;
            }
            None => {}
        }
    }

    /// One key, and what the status line is to say.
    fn take(&mut self, editor: &mut Editor, key: Option<Key>) -> Option<String> {
        // Warning: **認不出的鍵也算收下了**：面板開着的時候一個 F13 不該掉進正文。
        let key = key?;
        if let (Some(None), Key::Char(name)) = (self.register, key) {
            self.register = Some(Some(name));
            return None;
        }
        let named = self.register.take();
        let g = std::mem::take(&mut self.g_pending);
        let page = self.panel.as_mut()?;
        if page.popup.is_some() {
            return self.take_in_list(editor, key);
        }
        let on_page = page.page.is_some();
        match key {
            // **No commands in the panel** (2026-11-08): its changes are
            // written as they are made, and `q` closes it. Warning: **the `:`
            // opens a line all the same** (2026-11-09, reported: `:e
            // notes.md` ran `e` as the panel's key and typed the rest into
            // the config file it opened) — what is typed after it is the
            // line's until Enter or Esc, and never a key of the panel's.
            Key::Char(':') => self.colon(editor),
            Key::Char('j') | Key::Down if on_page => page.page_step(true),
            Key::Char('k') | Key::Up if on_page => page.page_step(false),
            Key::Char('j') | Key::Down => page.step(true),
            Key::Char('k') | Key::Up => page.step(false),
            Key::Char('g') if g && on_page => page.page = Some(0),
            Key::Char('g') if g => page.first(),
            Key::Char('g') => self.g_pending = true,
            Key::Char('G') if on_page => page.page = Some(page.page_len().saturating_sub(1)),
            Key::Char('G') => page.last(),
            Key::Char('/') if !on_page => {
                page.typing = Some(Typing::Search);
                let text = page.search.clone();
                editor.open_value_line("/", &text);
            }
            Key::Enter | Key::Char(' ') if on_page => {
                page.page_press();
                return self.write(editor);
            }
            Key::Enter | Key::Char(' ') if page.types_here() => self.edit(editor, None),
            Key::Enter => {
                page.change();
                return self.write(editor);
            }
            Key::Char(' ') => {
                page.press();
                return self.write(editor);
            }
            Key::Char('h') | Key::Left if on_page => page.close_page(),
            // **`h` `l` walk the left column** (2026-11-09): a page's row
            // opens on `Enter` or `Space`, so `l` means one thing in the list.
            Key::Char('h') | Key::Left => page.step_group(false),
            Key::Char('l') | Key::Right => page.step_group(true),
            Key::Char('r') => {
                return match page.reset() {
                    Some(true) => self.write(editor),
                    Some(false) => Some(say!("settings.nothing-to-drop")),
                    None => None,
                };
            }
            Key::Char('u') => return self.replay(editor, true),
            Key::Ctrl('r') => return self.replay(editor, false),
            Key::Char('"') => self.register = Some(None),
            Key::Char('p') => return self.paste_register(editor, named.flatten()),
            Key::Char('y') => {
                let value = page.value_here()?;
                editor.yank_text(&value);
                return Some(say!("settings.yanked", value));
            }
            Key::Tab => page.flip_into(),
            Key::Char('e') => return self.open_the_file(editor),
            Key::Char('q') => self.close(editor),
            // **`Esc` steps back**: off a page, then off a search, then out.
            Key::Esc if on_page => page.close_page(),
            Key::Esc if !page.search.is_empty() => {
                page.search.clear();
                page.narrow(None);
            }
            Key::Esc => self.close(editor),
            _ => {}
        }
        None
    }

    /// A key while a choice's list is open: walk it (a theme is tried as the
    /// cursor goes), take one, or put it away.
    fn take_in_list(&mut self, editor: &mut Editor, key: Key) -> Option<String> {
        let page = self.panel.as_mut()?;
        match key {
            Key::Char('j') | Key::Down => page.popup_step(true),
            Key::Char('k') | Key::Up => page.popup_step(false),
            Key::Enter | Key::Char(' ') => {
                page.popup_choose();
                if std::mem::take(&mut self.trying) {
                    self.preview = Some(Preview::Keep);
                }
                return self.write(editor);
            }
            Key::Esc => {
                page.popup = None;
                self.untry();
                return None;
            }
            // **`q` closes the panel here as everywhere** (2026-11-09), and
            // the theme being tried goes back with it.
            Key::Char('q') => {
                self.close(editor);
                return None;
            }
            Key::Char(':') => {
                self.colon(editor);
                return None;
            }
            _ => return None,
        }
        self.try_theme();
        None
    }

    /// **`:`**: the command row opens, and takes every key up to Enter or Esc.
    fn colon(&mut self, editor: &mut Editor) {
        let Some(page) = self.panel.as_mut() else { return };
        page.typing = Some(Typing::Command);
        editor.open_value_command_line(":", "");
    }

    /// The theme under the list's cursor, tried (2026-11-08: themes preview
    /// while the list is walked).
    fn try_theme(&mut self) {
        let Some(page) = self.panel.as_ref() else { return };
        if page.here().map(|s| s.path()).as_deref() != Some("theme.name") {
            return;
        }
        if let Some(word) = page.popup_word() {
            self.preview = Some(Preview::Show(word.to_string()));
            self.trying = true;
        }
    }

    fn untry(&mut self) {
        if std::mem::take(&mut self.trying) {
            self.preview = Some(Preview::Undo);
        }
    }

    /// **Open the one-line editor on the row**: the command row, the `:`
    /// line's own keys, with the row's name before it. `text` replaces the
    /// value (a paste); else the value is there to change.
    fn edit(&mut self, editor: &mut Editor, text: Option<String>) {
        let Some(page) = self.panel.as_mut() else { return };
        let Some(setting) = page.here() else { return };
        let text = text.unwrap_or_else(|| page.typing_from());
        page.typing = Some(Typing::Value);
        editor.open_value_line(&format!("{}: ", said(setting.label)), &text);
    }

    /// **Write what the key changed**, and say where: every confirmed change
    /// is written at once and takes effect (2026-11-08).
    fn write(&mut self, editor: &mut Editor) -> Option<String> {
        let page = self.panel.as_mut()?;
        let mut wrote = page.flush();
        if let Ok(Some(saved)) = &mut wrote {
            if std::mem::take(&mut self.outside) {
                saved.changed_underneath = saved.wrote.clone();
            }
        }
        said_of_a_write(editor, wrote, false)
    }

    /// `u` or `C-r`.
    fn replay(&mut self, editor: &mut Editor, back: bool) -> Option<String> {
        let page = self.panel.as_mut()?;
        let wrote = match back {
            true => page.undo(),
            false => page.redo(),
        };
        // **A file taken away is said to be removed** (2026-11-09), not
        // written: `u` of the change that made it.
        let removed = match &wrote {
            Ok(Some(saved)) => saved.removed.iter().map(|p| shorten(p)).collect::<Vec<_>>().join(&say!("label.comma")),
            _ => String::new(),
        };
        let wrote_any = matches!(&wrote, Ok(Some(saved)) if !saved.wrote.is_empty());
        match (&wrote, back) {
            (Ok(None), true) => Some(say!("settings.nothing-to-undo")),
            (Ok(None), false) => Some(say!("settings.nothing-to-redo")),
            _ => said_of_a_write(editor, wrote, true).map(|said| {
                let mut out = match (back, wrote_any || removed.is_empty()) {
                    (true, true) => say!("settings.undone", said),
                    (false, true) => say!("settings.redone", said),
                    (true, false) => say!("settings.undone-removed", removed.clone()),
                    (false, false) => say!("settings.redone-removed", removed.clone()),
                };
                if wrote_any && !removed.is_empty() {
                    out.push_str(&say!("settings.also-removed", removed));
                }
                out
            }),
        }
    }

    /// **`p` puts a register in place of the value** (2026-11-08): the
    /// editor opens with it, and `Enter` keeps it. `"+p` is the system
    /// clipboard, as in the editor.
    fn paste_register(&mut self, editor: &mut Editor, named: Option<char>) -> Option<String> {
        let page = self.panel.as_ref()?;
        if !page.types_here() || page.page.is_some() {
            return Some(say!("settings.paste-nowhere"));
        }
        let named = named.filter(|&name| name != '"');
        let text = match named {
            Some('+' | '*') => match self.clipboard.unwrap_or(crate::read_clipboard)() {
                Some(text) => text,
                None => return Some(crate::clipboard_unreadable()),
            },
            name => editor.register_contents(name),
        };
        if text.is_empty() {
            return Some(match named {
                Some('+' | '*') => say!("edit.clipboard-empty"),
                Some(name) => say!("register.empty", name),
                None => say!("register.nothing-yanked"),
            });
        }
        self.edit(editor, Some(one_line(&text)));
        None
    }

    /// **Text pasted through the terminal** (bracketed paste, ⌘V): it takes
    /// the value's place, in the editor, for `Enter` to keep (2026-11-08).
    /// While the command row is typing, the paste is its, at the caret.
    /// Gives whether it was the panel's.
    pub fn pasted(&mut self, editor: &mut Editor, text: &str) -> bool {
        if self.panel.is_none() {
            return false;
        }
        let _ = self.notice(editor);
        let Some(page) = self.panel.as_ref() else { return false };
        if editor.mode().is_prompt() {
            return false;
        }
        let before = page.spot();
        let said = match page.types_here() && page.page.is_none() && page.popup.is_none() {
            true => {
                self.edit(editor, Some(one_line(text)));
                None
            }
            false => Some(say!("settings.paste-nowhere")),
        };
        self.say(editor, before, said);
        true
    }

    /// **`e` opens the file this page writes into, at the row's line**
    /// (2026-10-20, the line since 2026-11-08). `Tab` says which file.
    fn open_the_file(&mut self, editor: &mut Editor) -> Option<String> {
        let page = self.panel.as_ref()?;
        let path = page.sheet().path.clone()?;
        let line = page.line_here();
        self.close(editor);
        if let Err(why) = editor.open_file(&path) {
            return Some(say!("buffer.cannot-open", shorten(&path), why));
        }
        let _ = editor.execute(&format!(":{}", line + 1));
        None
    }

    /// **The mouse, while the panel is open** (2026-11-08): it used to go on
    /// to the document behind the panel. A click on a group, a row or a
    /// value in a list goes to it, the wheel walks the list, and anything
    /// else is taken and does nothing. Gives whether it was the panel's.
    pub fn mouse(&mut self, editor: &mut Editor, pointer: Pointer, x: u16, y: u16) -> bool {
        if self.panel.is_none() {
            return false;
        }
        let noticed = self.notice(editor);
        let Some(page) = self.panel.as_mut() else { return false };
        if editor.mode().is_prompt() {
            return true;
        }
        let before = page.spot();
        let mut said = None;
        // A click away from a choice's list puts it away, and the theme
        // being tried goes back as Esc puts it back (2026-11-09).
        let listed = page.popup.is_some();
        match pointer {
            Pointer::Click => match hit_at(x, y) {
                Some(Hit::Group(i)) => {
                    page.page = None;
                    page.popup = None;
                    page.to_group(i);
                    if listed {
                        self.untry();
                    }
                }
                Some(Hit::Row(i)) => {
                    page.popup = None;
                    page.row = i;
                    if listed {
                        self.untry();
                    }
                }
                Some(Hit::PageRow(i)) => page.page = Some(i),
                Some(Hit::Choice(i)) => {
                    if let Some(popup) = page.popup.as_mut() {
                        popup.at = i;
                    }
                    said = self.take_in_list(editor, Key::Enter);
                }
                None => {}
            },
            Pointer::Wheel { back, notches } => {
                for _ in 0..notches.min(64) {
                    match (page.popup.is_some(), page.page.is_some()) {
                        (true, _) => page.popup_step(!back),
                        (false, true) => page.page_step(!back),
                        (false, false) => page.step(!back),
                    }
                }
                if page.popup.is_some() {
                    self.try_theme();
                }
            }
        }
        self.register = None;
        if !(noticed && said.is_none()) {
            self.say(editor, before, said);
        }
        true
    }

    /// 核心那幾張條子：`:settings` 開這一扇（帶一個名字就停在那一項上），和命令行
    /// 交回來的那一段字。
    pub fn settle(&mut self, editor: &mut Editor) {
        let _ = self.notice(editor);
        if editor.take_settings_request() && self.panel.is_none() {
            // 本項目那一頁跟着工作路徑走（2026-10-01）。
            let local = Some(yumete_config::panel::local_sheet_path(&editor.root()));
            let mut page = Panel::open(Some(yumete_config::config_dir().join("config.toml")), local);
            // The schemes there are, as the input method reads a name.
            page.schemes = Some(yumete_config::panel::Names {
                takes: |name| yumete_ime::Scheme::from_tag(name).is_some(),
                names: yumete_ime::Scheme::all().into_iter().map(|s| s.tag().to_string()).collect(),
            });
            self.panel = Some(page);
            self.row_said = false;
            editor.set_settings_open(true);
            editor.set_status(String::new());
        }
        if let Some(name) = editor.take_settings_target() {
            if let Some(page) = self.panel.as_mut() {
                page.search.clear();
                if !page.go_to(&name, |s| every_name(s.label)) {
                    editor.set_status(say!("settings.no-such-setting", name));
                    self.row_said = true;
                }
            }
        }
        let Some(page) = self.panel.as_mut() else { return };
        match (page.typing, editor.take_value_answer()) {
            (Some(Typing::Value), Some(Some(text))) => {
                page.typing = None;
                match page.commit_value(&text) {
                    true => {
                        let said = self.write(editor);
                        if let Some(said) = said {
                            editor.set_status(said);
                            self.row_said = true;
                        }
                    }
                    // Refused: the value is typed again, the reason under
                    // the row.
                    false => self.edit(editor, Some(text)),
                }
            }
            (Some(Typing::Value), Some(None)) => {
                page.typing = None;
                page.refusal = None;
            }
            // What was typed after a `:`: the habit of closing, or a word
            // that it is not taken. Nothing of it ever reaches the panel or
            // the document.
            (Some(Typing::Command), Some(Some(text))) => {
                page.typing = None;
                let typed = text.trim().to_string();
                if CLOSING.contains(&typed.as_str()) {
                    self.close(editor);
                    return;
                }
                // `:settings <name>` (or `:set <name>`) goes to that setting,
                // as it opens the panel on it from the editor (2026-11-09).
                if let Some(name) = setting_named(&typed) {
                    match name.is_empty() || page.go_to(name, |s| every_name(s.label)) {
                        true if !name.is_empty() => page.search.clear(),
                        true => {}
                        false => {
                            editor.set_status(say!("settings.no-such-setting", name));
                            self.row_said = true;
                        }
                    }
                    return;
                }
                if !typed.is_empty() {
                    editor.set_status(say!("settings.not-a-command", typed));
                    self.row_said = true;
                }
            }
            (Some(Typing::Command), Some(None)) => page.typing = None,
            (Some(Typing::Search), Some(answer)) => {
                page.typing = None;
                page.search = answer.unwrap_or_default();
                let found = found(&page.search);
                page.narrow(found);
            }
            (Some(_), None) if editor.mode() != yumete_core::input::Mode::Value => {
                page.typing = None;
                page.refusal = None;
            }
            _ => {}
        }
        let Some(page) = self.panel.as_mut() else { return };
        // The search narrows as it is typed.
        if page.typing == Some(Typing::Search) {
            if let Some(text) = editor.value_typed() {
                if text != page.search {
                    page.search = text.to_string();
                    let found = found(&page.search);
                    page.narrow(found);
                }
            }
        }
    }

    /// 走 —— 改動早已寫下去了，没有什麼要問的（2026-11-08）。
    fn close(&mut self, editor: &mut Editor) {
        self.untry();
        editor.close_value_line();
        self.panel = None;
        self.row_said = false;
        editor.set_settings_open(false);
        editor.set_status(String::new());
    }
}

/// What to say of a write, and the reload it asks for; `None` when nothing
/// was written. `bare` gives the files alone, for `u` and `C-r` to frame.
fn said_of_a_write(editor: &mut Editor, wrote: Result<Option<Saved>, SaveError>, bare: bool) -> Option<String> {
    let names = |paths: &[std::path::PathBuf]| paths.iter().map(|p| shorten(p)).collect::<Vec<_>>().join(&say!("label.comma"));
    match wrote {
        Ok(None) => None,
        Ok(Some(saved)) => {
            // **寫完要讓編輯器重讀一遍**（2026-10-09，見 `Editor::set_config_reload`）。
            editor.set_config_reload();
            Some(match (bare, saved.changed_underneath.is_empty()) {
                (true, _) => names(&saved.wrote),
                (false, true) => say!("settings.saved", names(&saved.wrote)),
                (false, false) => say!("settings.saved-moved", names(&saved.wrote)),
            })
        }
        Err(SaveError::Unread { path, line, said }) => Some(say!("settings.cannot-save-unread", shorten(&path), line + 1, said)),
        Err(SaveError::Io(why)) => Some(say!("settings.cannot-save", why)),
        Err(SaveError::LinkToNothing(path)) => Some(say!("settings.cannot-save-link-to-nothing", shorten(&path))),
    }
}

/// **The rows a search leaves**, as places in [`all_rows`]: a setting whose
/// name, description or TOML key holds the text, any case. Empty is no
/// search.
fn found(text: &str) -> Option<Vec<usize>> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    Some(
        all_rows()
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                [said(s.label), said(s.hint), s.path()].iter().any(|t| t.to_lowercase().contains(&text))
            })
            .map(|(i, _)| i)
            .collect(),
    )
}

/// Pasted text made one line: a break is one space, with the spaces either
/// side of it, and the breaks at either end go (a whole line copied with its
/// break is the line).
fn one_line(text: &str) -> String {
    let lines: Vec<&str> = text.split(['\n', '\r']).map(str::trim).filter(|l| !l.is_empty()).collect();
    lines.join(" ").chars().map(|c| if c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).collect()
}

/// 一個值畫成什麽樣。
///
/// Warning: **打勾與幾選一戴的是同一副方括號** —— 同搜索面板那條規矩（2026-09-23
/// 原話：「這樣用戶就知道這裏是可以空格切換的」）。數字與一段字不戴：它們按空格
/// 也動，但動的是「加一」而不是「換一個」，戴上會讓人以為只有兩三個值。
/// **值那一格要的是名字，不是那一句話。**
///
/// 幾選一的說明有兩種形狀。多數本來就是名字（`[問終端]`、`[橫排]`），而主題那十個
/// 寫的是「墨香：黑墨、白金墨、金墨、紅墨，其餘的色階都算出來」——那是給 `:theme`
/// 的菜單寫的，那裏一行一個詞，整句放得下。值那一格放不下，撥一格就只換了半句話
/// 的開頭。冒號前面那一截就是名字，而這十條都是那個形狀；沒有冒號的原樣不動。
///
/// **English writes the colon `: `** (2026-11-08, reported: the English
/// theme value was the whole sentence, `[moxiang: black, white-gold, …]`),
/// so the name is cut at whichever comes first.
fn name_of(label: String) -> String {
    let cut = [label.find('：'), label.find(": ")].into_iter().flatten().min();
    match cut {
        Some(at) => label[..at].to_string(),
        None => label,
    }
}

fn drawn(setting: &Setting, value: &str) -> String {
    match setting.kind {
        Kind::Tick => match value {
            "true" => "[x]".to_string(),
            _ => "[ ]".to_string(),
        },
        Kind::Pick(choices) | Kind::PickOr(choices) => {
            // 同底下那一支：`trim_matches` 不是 toml 轉義的反函數，而字面串
            // （`layout = 'horizontal'`）它一個字都剝不掉。
            let word = yumete_config::panel::unwritten(value);
            let word = word.as_str();
            let label = choices
                .iter()
                .find(|c| c.word == word)
                .map(|c| name_of(said(c.label)))
                // 檔裏寫了一個我們不認得的詞：照原樣畫出來，別假裝它是第一個。
                .unwrap_or_else(|| word.to_string());
            format!("[{label}]")
        }
        // Warning: **`trim_matches` 不是 toml 轉義的反函數**（2026-10-01，見
        // `yumete_config::panel::unwritten`）——`"a\"b"` 畫出來是 `a\"b`。
        Kind::Count { .. } | Kind::Text => yumete_config::panel::unwritten(value),
        // The page's own row draws its counts (see [`row_value`]).
        Kind::Page => String::new(),
    }
}

/// A row's value as the list draws it: a page's row says how many items
/// are on, and that it opens.
fn row_value(panel: &Panel, setting: &Setting, value: &str) -> String {
    match setting.kind {
        Kind::Page if setting.path() == "ime.annotation_modes" => {
            format!("{}  ›", say!("settings.annotation-modes.count", panel.annotation_modes().len()))
        }
        Kind::Page => {
            let (on, of, _) = panel.flavor_counts();
            format!("{}  ›", say!("settings.flavor-items.count", on, of))
        }
        _ => drawn(setting, value),
    }
}

/// An item's keys under a keymap, as the page writes them: `gz`, `]m`,
/// `C-w e`, `空格 t`.
fn item_keys(keys: &[&str]) -> String {
    keys.iter()
        .map(|key| {
            let internal = yumete_core::actions::written(key);
            let mut presses: Vec<String> = Vec::new();
            let mut chars = internal.chars();
            while let Some(c) = chars.next() {
                let one = match c {
                    yumete_core::actions::ALT => format!("{c}{}", chars.next().map(String::from).unwrap_or_default()),
                    c => c.to_string(),
                };
                presses.push(yumete_core::actions::spell(&one).trim().to_string());
            }
            // A one-character group key runs into what follows (`gz`, `]m`);
            // a named or held one is set apart (`C-w e`, `空格 t`).
            match presses.split_first() {
                Some((first, rest)) if first.chars().count() == 1 && !rest.is_empty() => {
                    format!("{first}{}", rest.join(""))
                }
                _ => presses.join(" "),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One line of the list: a group's heading, a row, the reason under a row,
/// or a value in the list a choice opened.
enum Line {
    Heading(usize),
    Row(usize),
    Refused(String),
    Choice(usize),
}

/// **Where a row's value comes from**, at its right end: the layer, and the
/// other file when it says something too (2026-11-08, VS Code's 「Modified
/// in」).
fn source_of(panel: &Panel, setting: &Setting) -> String {
    let shown = panel.shown(setting);
    let path = setting.path();
    let other = match shown.from {
        Layer::Local => panel.global.says(&path).map(|_| Layer::Global),
        Layer::Global => panel.local.says(&path).map(|_| Layer::Local),
        Layer::Factory => None,
    };
    match other {
        Some(other) => say!("settings.from-both", said(shown.from.label()), said(other.label())),
        None => said(shown.from.label()),
    }
}

/// **畫一頁。** The caret is the command row's while a value is typed, so
/// this gives none.
pub fn draw(
    frame: &mut Frame,
    panel: &Panel,
    config: &Config,
    area: Rect,
) -> Option<ratatui::layout::Position> {
    let ink = crate::theme::Palette::of(config);
    let ground = ink.ground(yumete_config::rung::CHROME);
    let text = ground.fg(ink.text());
    let head = ground.fg(ink.gold()).add_modifier(Modifier::BOLD);
    let quiet = ground.fg(ink.quiet());
    let on = ground.fg(ink.gold()).add_modifier(Modifier::REVERSED);
    let changed = ground.fg(ink.mark());

    HITS.with(|hits| hits.borrow_mut().clear());
    // A window too short for the page and the line being typed: the line
    // keeps its row, and the page gives up everything (2026-11-09).
    if area.width == 0 || area.height == 0 {
        return None;
    }
    frame.render_widget(Clear, area);
    let rows = panel.rows();
    let col = Columns::of(area, panel);
    let right = col.right;
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..right {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(ground);
            }
        }
    }

    // ---- 頂上那一行：標題、兩個標籤（全局／本項目，`Tab` 換），右邊是那一份檔 ----
    //
    // **Both files are always there as tabs** (2026-11-08): the one written
    // into is lit. A search in force follows them.
    let mut x = area.x + 1;
    let title = say!("settings.title");
    put_text(buf, x, area.y, right, &title, head);
    x += yumete_cjk::str_width(&title) as u16 + 2;
    for which in [Which::Global, Which::Local] {
        let name = format!(" {} ", said(which.layer().label()));
        put_text(buf, x, area.y, right, &name, if which == panel.into { on } else { quiet });
        x += yumete_cjk::str_width(&name) as u16 + 1;
    }
    if panel.page.is_some() {
        let label = panel.here().map(|s| s.label).unwrap_or("settings.keys.flavor-items");
        let crumb = format!(" › {}", said(label));
        put_text(buf, x, area.y, right, &crumb, head);
        x += yumete_cjk::str_width(&crumb) as u16;
    } else if !panel.search.is_empty() {
        let search = format!(" /{}", panel.search);
        put_text(buf, x, area.y, right, &search, head);
        x += yumete_cjk::str_width(&search) as u16;
    }
    let sheet = panel.sheet();
    let where_to = match (&sheet.path, sheet.there) {
        (Some(p), true) => shorten(p),
        // **那個檔還沒有就在名字後面加一個 `+`**：寫第一項的時候它是被建出來的。
        (Some(p), false) => format!("{}+", shorten(p)),
        (None, _) => say!("settings.no-file"),
    };
    let room = right.saturating_sub(x + 2) as usize;
    if room > 4 {
        let line = crate::elide_head(&where_to, room - 1);
        let wide = yumete_cjk::str_width(&line) as u16;
        put_text(buf, right.saturating_sub(wide + 1), area.y, right, &line, text);
    }

    // ---- 底下那幾塊有多高，上面那一欄就剩多少 --------------------------------
    //
    // 從下往上排（2026-10-13 定）：鍵那一塊、一條橫線、說明那一塊（這一行是什麼、
    // 取什麼值、三層各說了什麼）。說明和鍵都可以折成幾行。
    let split = area.x + GROUPS_WIDE;
    let bottom = area.y + area.height;
    let note_wide = right.saturating_sub(split + 3) as usize;
    let note_rows: Vec<String> = note_lines(panel).iter().flat_map(|line| fold(line, note_wide)).collect();
    // **Only the keys that work now** (2026-11-08): the list, a page, a
    // choice's list, a value typed, a search typed.
    let keys = match (panel.typing, panel.popup.is_some(), panel.subpage().filter(|_| panel.page.is_some())) {
        (Some(Typing::Value), _, _) => say!("settings.keys-editing"),
        // The `:` line takes no IME, so no `C-^` (2026-11-09).
        (Some(Typing::Command), _, _) => say!("settings.keys-command"),
        (Some(Typing::Search), _, _) => say!("settings.keys-search"),
        (None, true, _) => say!("settings.keys-list"),
        (None, false, Some(Subpage::Flavor)) => say!("settings.keys-flavor-page"),
        (None, false, Some(Subpage::Annotation)) => say!("settings.keys-annotation-page"),
        // **Not a key that does nothing** (2026-11-09): with no row there is
        // nothing to change or reset, and a page's row has no value to copy
        // or put back.
        (None, false, None) if rows.is_empty() => say!("settings.keys-no-match"),
        (None, false, None) if panel.here().is_some_and(|s| s.kind == Kind::Page) => say!("settings.keys-page-row"),
        (None, false, None) if !panel.search.is_empty() => say!("settings.keys-searched"),
        (None, false, None) => say!("settings.keys"),
    };
    let keys_rows = fold(&keys, right.saturating_sub(area.x + 2) as usize);
    // Warning: **矮窗口上一塊一塊地讓**：先讓說明那一塊，鍵那一塊最後讓。
    let room = bottom.saturating_sub(area.y + 3);
    let keys_high = (keys_rows.len() as u16 + 1).min(room);
    let block = note_rows.len() as u16 + 4;
    let fits = block < room.saturating_sub(keys_high) && bottom.saturating_sub(keys_high + block + 1) >= area.y + 2 + 4;
    let block_high = if fits { block } else { 0 };
    let keys_at = bottom.saturating_sub(keys_high.saturating_sub(1));
    let foot = bottom.saturating_sub(keys_high).max(area.y + 2);
    let block_at = foot.saturating_sub(block_high);
    let note_at = block_at;
    let layers_at = block_at + note_rows.len() as u16 + 1;
    let list_ends = if block_high > 0 { block_at.saturating_sub(1) } else { foot };
    let rule = |buf: &mut ratatui::buffer::Buffer, y: u16, from: u16, join: Option<&str>| {
        for x in from..right {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol("─").set_style(quiet);
            }
        }
        if let Some(join) = join {
            if let Some(cell) = buf.cell_mut((split, y)) {
                cell.set_symbol(join).set_style(quiet);
            }
        }
    };
    if area.y + 1 < bottom {
        rule(buf, area.y + 1, area.x, Some("┬"));
    }
    if foot < bottom && foot > area.y + 1 {
        rule(buf, foot, area.x, Some("┴"));
    }
    for y in area.y + 2..foot {
        if let Some(cell) = buf.cell_mut((split, y)) {
            cell.set_symbol("│").set_style(quiet);
        }
    }
    if block_high > 0 && block_at > area.y + 2 {
        rule(buf, block_at.saturating_sub(1), split + 1, None);
        if let Some(cell) = buf.cell_mut((split, block_at.saturating_sub(1))) {
            cell.set_symbol("├").set_style(quiet);
        }
    }

    // ---- A file that cannot be read, said at the top (2026-11-08) ----------
    let banner: Vec<String> = panel
        .unread()
        .iter()
        .flat_map(|(which, (line, why))| fold(&say!("settings.unread", said(which.layer().label()), line + 1, why), note_wide))
        .collect();
    let top = (area.y + 2 + banner.len() as u16).min(list_ends);
    for (i, row) in banner.iter().enumerate() {
        let y = area.y + 2 + i as u16;
        if y >= list_ends {
            break;
        }
        put_text(buf, split + 2, y, right, row, changed);
    }

    // ---- 左邊：八組的目録，光標那一組亮着；點一下去它的第一行 -------------------
    let here_group = panel.group();
    let room_for_groups = foot.saturating_sub(area.y + 2) as usize;
    let first_group = match room_for_groups {
        0 => 0,
        _ => here_group.saturating_sub(room_for_groups - 1),
    };
    for (i, named) in GROUPS.iter().enumerate().skip(first_group) {
        let y = area.y + 2 + (i - first_group) as u16;
        if y >= foot {
            break;
        }
        let name = said(named.label);
        let pad = (GROUPS_WIDE as usize).saturating_sub(yumete_cjk::str_width(&name) + 2);
        put_text(buf, area.x, y, split, &format!("  {name}{}", " ".repeat(pad)), if i == here_group { head } else { text });
        hit(y, area.x, split, Hit::Group(i));
    }

    // ---- 右邊 --------------------------------------------------------------
    let from = col.name;
    if panel.page.is_some() {
        let (above, below) = match panel.subpage() {
            Some(Subpage::Annotation) => draw_annotation_page(buf, panel, &ink, ground, from, right, top, list_ends),
            _ => draw_flavor_page(buf, panel, &ink, ground, from, right, top, list_ends),
        };
        more_marks(buf, from, right, (area.y + 1, list_ends), (above, below), quiet, bottom);
        if block_high > 0 {
            draw_flavor_block(buf, panel, &ink, ground, split + 3, right, note_at, &note_rows, layers_at, foot);
        }
        for (i, row) in keys_rows.iter().enumerate() {
            let y = keys_at + i as u16;
            if y >= bottom {
                break;
            }
            put_text(buf, area.x + 2, y, right, row, text);
        }
        return None;
    }
    if rows.is_empty() {
        put_text(buf, from, top, right, &say!("settings.no-match", &panel.search), quiet);
    }
    // **One list, the groups its headings** (2026-11-08), and under the
    // row the reason a value was refused, or the list a choice opened.
    let mut lines: Vec<Line> = Vec::new();
    let mut cursor_line = 0;
    let mut cursor_end = 0;
    for (i, setting) in rows.iter().enumerate() {
        let group = GROUPS.iter().position(|n| n.group == setting.group).unwrap_or(0);
        if i == 0 || rows[i - 1].group != setting.group {
            lines.push(Line::Heading(group));
        }
        if i == panel.row {
            cursor_line = lines.len();
        }
        lines.push(Line::Row(i));
        if i == panel.row {
            if let (Some(Typing::Value), Some(why)) = (panel.typing, &panel.refusal) {
                lines.push(Line::Refused(refusal_said(why)));
            }
            if let Some(popup) = &panel.popup {
                for n in 0..popup.words.len() {
                    lines.push(Line::Choice(n));
                }
            }
            cursor_end = lines.len() - 1;
        }
    }
    // **光標走到哪，那一行就跳進畫面**（2026-10-09），連同它下面那幾行；它上面
    // 那一組的標題放得下就一起放。
    let room = list_ends.saturating_sub(top) as usize;
    let mut first = match room {
        0 => cursor_line,
        _ => (cursor_end + 1).saturating_sub(room).min(cursor_line),
    };
    if first == cursor_line && first > 0 && matches!(lines[first - 1], Line::Heading(_)) && cursor_end - first + 2 <= room {
        first -= 1;
    }
    let mut y = top;
    let mut last = first;
    for line in lines.iter().skip(first) {
        if y >= list_ends {
            break;
        }
        last += 1;
        match line {
            Line::Heading(group) => {
                put_text(buf, from.saturating_sub(1), y, right, &said(GROUPS[*group].label), head);
            }
            Line::Refused(why) => {
                let why = crate::elide(why, right.saturating_sub(from + 3) as usize);
                put_text(buf, from + 2, y, right, &why, changed);
            }
            Line::Choice(n) => {
                let Some(popup) = &panel.popup else { continue };
                let word = &popup.words[*n];
                let setting = panel.here();
                let line = setting.map(|s| choice_line(s, word)).unwrap_or_else(|| word.clone());
                let line = format!(" {line}");
                let line = crate::elide(&line, right.saturating_sub(col.value + 1) as usize);
                let style = if *n == popup.at { on } else { ground.fg(ink.text()) };
                put_text(buf, col.value, y, right, &line, style);
                hit(y, col.value, right, Hit::Choice(*n));
            }
            Line::Row(i) => {
                let setting = rows[*i];
                let here = *i == panel.row;
                hit(y, from, right, Hit::Row(*i));
                draw_row(buf, panel, setting, here, (&ink, ground), &col, y);
            }
        }
        y += 1;
    }
    let rows_in = |part: &[Line]| part.iter().filter(|l| matches!(l, Line::Row(_))).count();
    let above = rows_in(&lines[..first.min(lines.len())]);
    let below = rows_in(&lines[last.min(lines.len())..]);
    more_marks(buf, from, right, (area.y + 1, list_ends), (above, below), quiet, bottom);

    // ---- 右半邊底下那一塊：這一行是什麼，取什麼值，三層各說了什麼 ----------------
    if block_high > 0 {
        for (i, row) in note_rows.iter().enumerate() {
            let y = note_at + i as u16;
            if y >= foot {
                break;
            }
            put_text(buf, split + 3, y, right, row, text);
        }
        if let Some(setting) = panel.here().filter(|s| s.kind != Kind::Page) {
            draw_layers(buf, panel, setting, (&ink, ground), split + 3, right, layers_at, foot);
        }
    }
    for (i, row) in keys_rows.iter().enumerate() {
        let y = keys_at + i as u16;
        if y >= bottom {
            break;
        }
        put_text(buf, area.x + 2, y, right, row, text);
    }
    None
}

/// One row: the mark of a row set in the file switched to, the name, the
/// value, and where the value comes from.
fn draw_row(
    buf: &mut ratatui::buffer::Buffer,
    panel: &Panel,
    setting: &Setting,
    here: bool,
    (ink, ground): (&crate::theme::Palette, ratatui::style::Style),
    col: &Columns,
    y: u16,
) {
    let from = col.name;
    let right = col.right;
    let shown = panel.shown(setting);
    let band = match here {
        true => ink.ground(yumete_config::rung::HEAD),
        false => ground,
    };
    if here {
        for x in from..right {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(band);
            }
        }
    }
    // **Set in this file**: a mark in the gap before the name (2026-11-08).
    if panel.set_here(&setting.path()) {
        put_text(buf, from.saturating_sub(1), y, from, "*", ground.fg(ink.mark()));
    }
    let label = crate::elide(&said(setting.label), col.value.saturating_sub(from + 1) as usize);
    put_text(buf, from, y, col.value.saturating_sub(1), &label, band.fg(ink.text()));
    // The source at the right end, quiet for the factory's.
    let source = match setting.kind {
        Kind::Page => String::new(),
        _ => source_of(panel, setting),
    };
    let source_wide = yumete_cjk::str_width(&source) as u16;
    let source_at = right.saturating_sub(source_wide + 1).max(col.value);
    let value = row_value(panel, setting, &shown.value);
    let room = source_at.saturating_sub(col.value + 2) as usize;
    let value_style = match (here, shown.from) {
        (true, _) => band.fg(ink.gold()).add_modifier(Modifier::REVERSED),
        (false, Layer::Factory) => ground.fg(ink.text()),
        (false, _) => ground.fg(ink.azure()),
    };
    put_text(buf, col.value, y, source_at, &crate::elide(&value, room), value_style);
    if source_at > col.value + 2 {
        let style = match shown.from {
            Layer::Factory => band.fg(ink.quiet()),
            _ => band.fg(ink.azure()),
        };
        put_text(buf, source_at, y, right, &source, style);
    }
}

/// **The three layers of the row**, one a line, the one in force in gold;
/// a file that cannot be read is unknown, not unset (2026-11-08).
#[allow(clippy::too_many_arguments)]
fn draw_layers(
    buf: &mut ratatui::buffer::Buffer,
    panel: &Panel,
    setting: &Setting,
    (ink, ground): (&crate::theme::Palette, ratatui::style::Style),
    x: u16,
    right: u16,
    layers_at: u16,
    foot: u16,
) {
    let shown = panel.shown(setting);
    for (i, (layer, said_value)) in panel.layers(setting).iter().enumerate() {
        let y = layers_at + i as u16;
        if y >= foot {
            break;
        }
        let unread = match layer {
            Layer::Factory => false,
            Layer::Global => panel.global.unread.is_some(),
            Layer::Local => panel.local.unread.is_some(),
        };
        let value = match (said_value, unread) {
            (Some(v), _) => drawn(setting, v),
            (None, true) => say!("settings.layer.unknown"),
            (None, false) => say!("settings.layer.unset"),
        };
        let name = said(layer.label());
        let pad = 8usize.saturating_sub(yumete_cjk::str_width(&name));
        let mut line = format!("{name}{}{value}", " ".repeat(pad));
        let mark = format!("   {}", say!("settings.layer.in-force"));
        let room = right.saturating_sub(x) as usize;
        if *layer == shown.from && yumete_cjk::str_width(&line) + yumete_cjk::str_width(&mark) <= room {
            line.push_str(&mark);
        }
        let style = match (said_value.is_some(), *layer == shown.from) {
            (_, true) => ground.fg(ink.gold()),
            (true, _) => ground.fg(ink.azure()),
            (false, _) => ground.fg(ink.quiet()),
        };
        put_text(buf, x, y, right, &crate::elide(&line, room), style);
    }
}

/// **A value as its list shows it**: its name, then what it means — a
/// theme's description after its name, or the word the file writes when the
/// name says it all (`從不  never`). An annotation mode the config adds and a
/// scheme the input method found are their names.
fn choice_line(setting: &Setting, word: &str) -> String {
    let (Kind::Pick(choices) | Kind::PickOr(choices)) = setting.kind else { return word.to_string() };
    let label = match choices.iter().find(|c| c.word == word) {
        Some(choice) => said(choice.label),
        None if setting.path() == "ime.annotation" => crate::annotation_label(Some(word)),
        None => return word.to_string(),
    };
    let cut = [label.find('：').map(|at| (at, '：'.len_utf8())), label.find(": ").map(|at| (at, 2))]
        .into_iter()
        .flatten()
        .min();
    match cut {
        Some((at, len)) => format!("{}  {}", &label[..at], label[at + len..].trim()),
        None if label == word => label,
        None => format!("{label}  {word}"),
    }
}

/// **The description line, always there for the row under the cursor**
/// (2026-11-08): what it does, then what it is and takes — a switch, a
/// number and its range, the choices, a piece of text — and its factory
/// value, then what `0` means where it means something.
fn note_lines(panel: &Panel) -> Vec<String> {
    let mut lines = vec![note_for(panel)];
    if panel.page.is_some() {
        return lines;
    }
    let Some(setting) = panel.here() else { return lines };
    let factory = drawn(setting, setting.factory);
    let english = yumete_core::messages::language() == yumete_core::messages::Language::English;
    let about = match setting.kind {
        Kind::Tick => say!("settings.about.tick", factory),
        Kind::Count { low, high, zero } => {
            let range = match zero.is_some() && low > 0 {
                true => say!("settings.range-or-zero", low, high),
                false => say!("settings.range", low, high),
            };
            let mut about = say!("settings.about.count", range, factory);
            if let Some(tag) = zero {
                match english {
                    true => about.push_str(&format!(" ({})", said(tag))),
                    false => about.push_str(&format!("（{}）", said(tag))),
                }
            }
            about
        }
        Kind::Pick(_) | Kind::PickOr(_) => {
            let names: Vec<String> = panel.choices(setting).iter().map(|w| drawn(setting, &format!("{w:?}"))).collect();
            say!("settings.about.choice", names.join(&say!("label.comma")), factory)
        }
        Kind::Text => say!("settings.about.text", factory),
        Kind::Page => return lines,
    };
    lines.push(about);
    lines
}

/// **The annotation modes' page** (2026-11-02): each mode under its name,
/// its eight fields with their ticks, and on the right the file that set
/// a field when one did.
#[allow(clippy::too_many_arguments)]
fn draw_annotation_page(
    buf: &mut ratatui::buffer::Buffer,
    panel: &Panel,
    ink: &crate::theme::Palette,
    ground: ratatui::style::Style,
    from: u16,
    right: u16,
    top: u16,
    ends: u16,
) -> (usize, usize) {
    let text = ground.fg(ink.text());
    let quiet = ground.fg(ink.quiet());
    // One line each, headings among them: (row index, heading or field).
    let mut lines: Vec<(Option<usize>, String, Option<yumete_config::panel::AnnotationRow>)> = Vec::new();
    for (i, (heading, row)) in panel.annotation_rows().into_iter().enumerate() {
        if let Some(mode) = heading {
            let label = crate::annotation_label(Some(&mode));
            let mut head = match label == mode {
                true => mode.clone(),
                false => format!("{label}  {mode}"),
            };
            if panel.mode_fields(&mode).is_empty() {
                head.push_str(&format!("  {}", say!("settings.annotation-modes.empty")));
            }
            lines.push((None, head, None));
        }
        lines.push((Some(i), String::new(), Some(row)));
    }
    let here = panel.page.unwrap_or(0);
    let at = lines.iter().position(|(i, _, _)| *i == Some(here)).unwrap_or(0);
    let room = ends.saturating_sub(top) as usize;
    let first = match room {
        0 => 0,
        _ => at.saturating_sub(room - 1),
    };
    let shown = (ends.saturating_sub(top) as usize).min(lines.len().saturating_sub(first));
    let rows_in = |part: &[(Option<usize>, String, Option<yumete_config::panel::AnnotationRow>)]| {
        part.iter().filter(|(i, _, _)| i.is_some()).count()
    };
    let out_of_sight = (rows_in(&lines[..first]), rows_in(&lines[first + shown..]));
    for (n, (index, head, row)) in lines.iter().enumerate().skip(first) {
        let y = top + (n - first) as u16;
        if y >= ends {
            break;
        }
        let Some(row) = row else {
            put_text(buf, from, y, right, head, quiet);
            continue;
        };
        if let Some(i) = index {
            hit(y, from, right, Hit::PageRow(*i));
        }
        if panel.set_here(&format!("ime.annotation_modes.{}.{}", row.mode, row.field)) {
            put_text(buf, from.saturating_sub(1), y, from, "*", ground.fg(ink.mark()));
        }
        let on_it = *index == Some(here);
        let band = match on_it {
            true => ink.ground(yumete_config::rung::HEAD),
            false => ground,
        };
        if on_it {
            for x in from..right {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(band);
                }
            }
        }
        let lit = |style: ratatui::style::Style| match (on_it, band.bg) {
            (true, Some(bg)) => style.bg(bg),
            _ => style,
        };
        let (on, layer) = panel.field_shown(row);
        let tick = match on {
            true => "[x]",
            false => "[ ]",
        };
        let tick_style = match on_it {
            true => band.fg(ink.gold()).add_modifier(Modifier::REVERSED),
            false => text,
        };
        put_text(buf, from + 1, y, right, tick, tick_style);
        let note = match layer {
            Layer::Factory => String::new(),
            other => said(other.label()),
        };
        let note_wide = yumete_cjk::str_width(&note) as u16;
        let note_at = right.saturating_sub(note_wide + 1);
        let name_at = from + 5;
        let name = crate::elide(
            &said(yumete_config::annotation_field_label(row.field)),
            note_at.saturating_sub(name_at + 1) as usize,
        );
        put_text(buf, name_at, y, note_at, &name, lit(text));
        if !note.is_empty() && note_at > name_at {
            put_text(buf, note_at, y, right, &note, lit(ground.fg(ink.azure())));
        }
    }
    out_of_sight
}

/// The level row's own hint, shown on the page as on the list.
const SETTINGS_FLAVOR_HINT: &str = "settings.keys.flavor.hint";

/// One line of the page: a heading, or a row with its index.
enum PageLine {
    Heading(String),
    Row(usize, FlavorRow),
}

/// **The flavour's page, in the right half** (2026-11-02): the level,
/// then each item with its tick, its keys, its name, and on the right
/// where its state comes from or which binding took its keys.
#[allow(clippy::too_many_arguments)]
fn draw_flavor_page(
    buf: &mut ratatui::buffer::Buffer,
    panel: &Panel,
    ink: &crate::theme::Palette,
    ground: ratatui::style::Style,
    from: u16,
    right: u16,
    top: u16,
    ends: u16,
) -> (usize, usize) {
    let text = ground.fg(ink.text());
    let quiet = ground.fg(ink.quiet());
    let keymap = panel.flavor_keymap();
    let mut lines: Vec<PageLine> = Vec::new();
    for (i, (section, row)) in panel.flavor_rows().into_iter().enumerate() {
        if let Some(section) = section {
            lines.push(PageLine::Heading(match section {
                FlavorSection::Basic => say!("keys.flavor.basic"),
                FlavorSection::Full => say!("keys.flavor.full"),
                FlavorSection::Other(other) => say!("settings.flavor.only-under", other.name()),
            }));
        }
        lines.push(PageLine::Row(i, row));
    }
    let here = panel.page.unwrap_or(0);
    let at = lines.iter().position(|l| matches!(l, PageLine::Row(i, _) if *i == here)).unwrap_or(0);
    let room = ends.saturating_sub(top) as usize;
    let first = match room {
        0 => 0,
        _ => at.saturating_sub(room - 1),
    };
    // The keys column is as wide as the widest keys, at most 14.
    let keys_of = |item: yumete_cjk::FlavorItem| item_keys(item.keys_under(panel.item_keymap(item)));
    let keys_wide = yumete_cjk::FlavorItem::ALL
        .iter()
        .map(|i| yumete_cjk::str_width(&keys_of(*i)))
        .max()
        .unwrap_or(0)
        .min(14) as u16;
    let shown = (ends.saturating_sub(top) as usize).min(lines.len().saturating_sub(first));
    let rows_in = |part: &[PageLine]| part.iter().filter(|l| matches!(l, PageLine::Row(..))).count();
    let out_of_sight = (rows_in(&lines[..first]), rows_in(&lines[first + shown..]));
    for (n, line) in lines.iter().enumerate().skip(first) {
        let y = top + (n - first) as u16;
        if y >= ends {
            break;
        }
        match line {
            PageLine::Heading(name) => {
                put_text(buf, from, y, right, name, quiet);
            }
            PageLine::Row(i, row) => {
                hit(y, from, right, Hit::PageRow(*i));
                let path = match row {
                    FlavorRow::Level => "keys.flavor".to_string(),
                    FlavorRow::Item(item) => format!("keys.flavor_items.{}", item.id()),
                };
                if panel.set_here(&path) {
                    put_text(buf, from.saturating_sub(1), y, from, "*", ground.fg(ink.mark()));
                }
                let on_it = *i == here;
                let band = match on_it {
                    true => ink.ground(yumete_config::rung::HEAD),
                    false => ground,
                };
                if on_it {
                    for x in from..right {
                        if let Some(cell) = buf.cell_mut((x, y)) {
                            cell.set_symbol(" ").set_style(band);
                        }
                    }
                }
                let lit = |style: ratatui::style::Style| match (on_it, band.bg) {
                    (true, Some(bg)) => style.bg(bg),
                    _ => style,
                };
                match row {
                    FlavorRow::Level => {
                        let level = panel.flavor_level();
                        let name = format!("{} ", say!("settings.keys.flavor"));
                        put_text(buf, from, y, right, &name, lit(text));
                        let value = format!("[{}]", match level {
                            yumete_cjk::FlavorLevel::Off => say!("keys.flavor.off"),
                            yumete_cjk::FlavorLevel::Basic => say!("keys.flavor.basic"),
                            yumete_cjk::FlavorLevel::Full => say!("keys.flavor.full"),
                        });
                        let x = from + yumete_cjk::str_width(&name) as u16;
                        let style = match on_it {
                            true => band.fg(ink.gold()).add_modifier(Modifier::REVERSED),
                            false => text,
                        };
                        put_text(buf, x, y, right, &value, style);
                        let (on, of, named) = panel.flavor_counts();
                        let mut sum = format!("{}  {}", keymap.name(), say!("settings.flavor-items.count", on, of));
                        if named > 0 {
                            sum.push_str(&format!("，{}", say!("settings.flavor-items.named", named)));
                        }
                        let x = x + yumete_cjk::str_width(&value) as u16 + 3;
                        put_text(buf, x, y, right, &sum, lit(quiet));
                    }
                    FlavorRow::Item(item) => {
                        let (on, source) = panel.item_shown(*item);
                        let tick = match on {
                            true => "[x]",
                            false => "[ ]",
                        };
                        let other = !item.info().keymaps.contains(&keymap);
                        let body = match other {
                            true => lit(quiet),
                            false => lit(text),
                        };
                        let tick_style = match on_it {
                            true => band.fg(ink.gold()).add_modifier(Modifier::REVERSED),
                            false => body,
                        };
                        put_text(buf, from + 1, y, right, tick, tick_style);
                        let keys = crate::elide(&keys_of(*item), keys_wide as usize);
                        put_text(buf, from + 5, y, right, &keys, body);
                        let name_at = from + 5 + keys_wide + 2;
                        // The right end: who set it, or who took its keys.
                        let taken = panel.item_taken(*item);
                        let (note, note_style) = match (taken.is_empty(), source) {
                            (false, _) => (
                                say!("settings.flavor.taken", item_keys(&taken), match item.info().insert {
                                    true => "[keys.insert]",
                                    false => "[keys.normal]",
                                }),
                                lit(ground.fg(ink.mark())),
                            ),
                            (true, ItemFrom::Layer(layer)) => (said(layer.label()), lit(ground.fg(ink.azure()))),
                            (true, ItemFrom::Level) => (String::new(), body),
                        };
                        let note_wide = yumete_cjk::str_width(&note) as u16;
                        let note_at = right.saturating_sub(note_wide + 1);
                        let name = crate::elide(
                            &said(item.info().label),
                            note_at.saturating_sub(name_at + 1) as usize,
                        );
                        put_text(buf, name_at, y, note_at, &name, body);
                        if !note.is_empty() && note_at > name_at {
                            put_text(buf, note_at, y, right, &note, note_style);
                        }
                    }
                }
            }
        }
    }
    out_of_sight
}

/// The page's note and layers: what the item does and what its key does
/// with it off, then the level's answer and each file's.
#[allow(clippy::too_many_arguments)]
fn draw_flavor_block(
    buf: &mut ratatui::buffer::Buffer,
    panel: &Panel,
    ink: &crate::theme::Palette,
    ground: ratatui::style::Style,
    x: u16,
    right: u16,
    note_at: u16,
    note_rows: &[String],
    layers_at: u16,
    foot: u16,
) {
    for (i, row) in note_rows.iter().enumerate() {
        let y = note_at + i as u16;
        if y >= foot {
            break;
        }
        put_text(buf, x, y, right, row, ground.fg(ink.text()));
    }
    // A field of a mode: the factory's tick, then each file's.
    if let Some(row) = panel.annotation_here() {
        let (_, from) = panel.field_shown(&row);
        let layers = panel.field_layers(&row);
        let from = match from {
            Layer::Factory => ItemFrom::Level,
            other => ItemFrom::Layer(other),
        };
        return draw_tick_layers(buf, ink, ground, x, right, layers_at, foot, layers, from, said(Layer::Factory.label()));
    }
    let Some(FlavorRow::Item(item)) = panel.flavor_here() else { return };
    let (_, from) = panel.item_shown(item);
    draw_tick_layers(buf, ink, ground, x, right, layers_at, foot, panel.item_layers(item), from, say!("settings.layer.level"));
}

/// A tick's three layers, one a line, the one in force in gold.
#[allow(clippy::too_many_arguments)]
fn draw_tick_layers(
    buf: &mut ratatui::buffer::Buffer,
    ink: &crate::theme::Palette,
    ground: ratatui::style::Style,
    x: u16,
    right: u16,
    layers_at: u16,
    foot: u16,
    layers: [(Layer, Option<bool>); 3],
    from: ItemFrom,
    factory: String,
) {
    for (i, (layer, value)) in layers.iter().enumerate() {
        let y = layers_at + i as u16;
        if y >= foot {
            break;
        }
        let name = match layer {
            Layer::Factory => factory.clone(),
            other => said(other.label()),
        };
        let shown = match value {
            Some(true) => "[x]".to_string(),
            Some(false) => "[ ]".to_string(),
            None => say!("settings.layer.unset"),
        };
        let pad = 8usize.saturating_sub(yumete_cjk::str_width(&name));
        let mut line = format!("{name}{}{shown}", " ".repeat(pad));
        let in_force = match from {
            ItemFrom::Level => *layer == Layer::Factory,
            ItemFrom::Layer(l) => *layer == l,
        };
        if in_force {
            line.push_str(&format!("   {}", say!("settings.layer.in-force")));
        }
        let style = match (value.is_some(), in_force) {
            (_, true) => ground.fg(ink.gold()),
            (true, _) => ground.fg(ink.azure()),
            (false, _) => ground.fg(ink.quiet()),
        };
        put_text(buf, x, y, right, &line, style);
    }
}

/// **How many rows are out of sight, said on the rules above and below the
/// list** (2026-11-08, reported: at 80×24 half of 標記 was cut off with no
/// sign of it).
#[allow(clippy::too_many_arguments)]
fn more_marks(
    buf: &mut ratatui::buffer::Buffer,
    from: u16,
    right: u16,
    (up, down): (u16, u16),
    (above, below): (usize, usize),
    style: ratatui::style::Style,
    bottom: u16,
) {
    if above > 0 {
        put_text(buf, from, up, right, &format!(" {} ", say!("settings.more-above", above)), style);
    }
    if below > 0 && down < bottom && down > up {
        put_text(buf, from, down, right, &format!(" {} ", say!("settings.more-below", below)), style);
    }
}

/// Why a value was not taken, as the line under its row says it: what is
/// taken, by name or by range.
fn refusal_said(why: &Refusal) -> String {
    let range = |low: usize, high: usize, zero: bool| match zero {
        true => say!("settings.range-or-zero", low, high),
        false => say!("settings.range", low, high),
    };
    let names = |names: &[String]| names.join(&say!("label.comma"));
    match why {
        Refusal::NotANumber { low, high, zero } => {
            say!("settings.refused.not-a-number", range(*low, *high, *zero))
        }
        Refusal::OutOfRange { low, high, zero } => {
            say!("settings.refused.out-of-range", range(*low, *high, *zero))
        }
        Refusal::Empty { names: list } if list.is_empty() => say!("settings.refused.empty"),
        Refusal::Empty { names: list } => say!("settings.refused.empty-names", names(list)),
        Refusal::NotOneOf { word, names: list } => say!("settings.refused.not-one-of", word, names(list)),
        Refusal::Loader(said) => said.clone(),
    }
}

/// 底下那一句說明：這一項是幹什麽的。
fn note_for(panel: &Panel) -> String {
    // A hint marks keys and names with backticks, which the generated
    // reference draws as code; here they would show as themselves.
    note_text(panel).replace('`', "")
}

fn note_text(panel: &Panel) -> String {
    if let Some(row) = panel.annotation_here() {
        let english = yumete_core::messages::language() == yumete_core::messages::Language::English;
        let gap = if english { "  " } else { "　" };
        let mut note = format!("[ime.annotation_modes.{}] {}", row.mode, row.field);
        if panel.mode_fields(&row.mode).is_empty() {
            note.push_str(&format!("{gap}{}", say!("settings.annotation-modes.empty")));
        }
        return note;
    }
    match panel.flavor_here() {
        Some(FlavorRow::Level) => return said(SETTINGS_FLAVOR_HINT),
        Some(FlavorRow::Item(item)) => {
            let english = yumete_core::messages::language() == yumete_core::messages::Language::English;
            let gap = if english { "  " } else { "　" };
            return format!("{}{gap}{}", item.id(), said(item.info().hint));
        }
        None => {}
    }
    let Some(setting) = panel.here() else { return String::new() };
    let mut note = said(setting.hint);
    // Warning: **改的那一層被蓋着就說一句。** 不說的話，一個人在全局上按空格而
    // 本項目壓着這一項，螢幕上那一行紋絲不動——看起來像鍵壞了。
    if panel.shadowed(setting) {
        let english = yumete_core::messages::language() == yumete_core::messages::Language::English;
        let gap = if english { "  " } else { "　" };
        note.push_str(&format!("{gap}{}", say!("settings.shadowed")));
    }
    note
}

/// 家目録寫成 `~`。
pub(crate) fn shorten(path: &std::path::Path) -> String {
    let shown = path.display().to_string();
    match std::env::var("HOME").ok().filter(|h| !h.is_empty()) {
        Some(home) if shown.starts_with(&home) => format!("~{}", &shown[home.len()..]),
        _ => shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yumete_core::input::Mode;

    /// **畫一頁，讀回來。** 用真的緩衝區。
    fn shot(panel: &Panel, width: u16, height: u16) -> String {
        let config = Config::default();
        crate::theme::settle(&config, None);
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("a terminal");
        terminal.draw(|frame| {
            draw(frame, panel, &config, Rect::new(0, 0, width, height));
        })
        .expect("one frame");
        crate::buffer_to_text(terminal.backend().buffer())
    }

    /// Where `text` is on a frame drawn by [`shot`]: `(x, y)`.
    fn find_on(page: &str, text: &str) -> (u16, u16) {
        for (y, line) in page.lines().enumerate() {
            if let Some(at) = line.find(text) {
                return (yumete_cjk::str_width(&line[..at]) as u16, y as u16);
            }
        }
        panic!("no {text:?} on:\n{page}")
    }

    /// A fresh directory under the temporary one.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yumete-setpage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An editor holding a line of text, and a seat with a panel on `global`
    /// (a file in `dir`) open on `path`.
    fn seated(dir: &std::path::Path, global: &str, path: &str) -> (Editor, Seat) {
        let file = dir.join("config.toml");
        if !global.is_empty() {
            std::fs::write(&file, global).unwrap();
        }
        let mut panel = Panel::open(Some(file), Some(dir.join("local.toml")));
        assert!(panel.go_to(path, |_| Vec::new()), "{path}");
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        for c in "稿子上的一行".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
        ed.set_settings_open(true);
        (ed, Seat { panel: Some(panel), ..Default::default() })
    }

    /// Keys as the loop gives them: the panel first, the editor when the
    /// panel gives them back, and the panel settled after each.
    fn keys(seat: &mut Seat, ed: &mut Editor, keys: &[Key]) {
        for key in keys {
            if !seat.took(ed, Some(*key)) {
                ed.on_key(*key);
            }
            seat.settle(ed);
        }
    }

    fn chars(text: &str) -> Vec<Key> {
        text.chars().map(Key::Char).collect()
    }

    fn file(dir: &std::path::Path) -> String {
        std::fs::read_to_string(dir.join("config.toml")).unwrap_or_default()
    }

    fn page(seat: &Seat) -> &Panel {
        seat.panel.as_ref().expect("open")
    }

    /// **The page**: the groups down the left, one list with the groups as
    /// headings, both files as tabs, the keys of the list at the foot.
    #[test]
    fn the_page_has_its_groups_tabs_and_list() {
        let p = Panel::open(None, None);
        let page = shot(&p, 110, 34);
        for named in GROUPS {
            assert!(page.contains(&said(named.label)), "{}:\n{page}", named.label);
        }
        assert!(page.contains(&say!("settings.layer.global")) && page.contains(&say!("settings.layer.local")), "{page}");
        for row in ["排版方向", "每縱字數"] {
            assert!(page.contains(row), "{row}:\n{page}");
        }
        assert!(page.contains('┬') && page.contains('┴') && page.contains('│'), "{page}");
        assert!(page.contains(&say!("settings.keys").chars().take(8).collect::<String>()), "{page}");
        // The heading of the first group stands above its first row.
        let (_, heading) = find_on(&page.lines().skip(2).collect::<Vec<_>>().join("\n"), &said(GROUPS[0].label));
        let (_, first) = find_on(&page.lines().skip(2).collect::<Vec<_>>().join("\n"), "排版方向");
        assert_eq!(heading + 1, first, "{page}");
    }

    /// **`j` walks across groups**, `gg` and `G` go to the ends, and the
    /// left column lights the group the cursor is in.
    #[test]
    fn j_crosses_groups_and_gg_g_go_to_the_ends() {
        let dir = scratch("walk");
        let last = all_rows().iter().rposition(|s| s.group == GROUPS[0].group).unwrap();
        let (mut ed, mut seat) = seated(&dir, "", &all_rows()[last].path());
        let last_of_layout = page(&seat).row;
        assert_eq!(page(&seat).group(), 0);
        keys(&mut seat, &mut ed, &[Key::Char('j')]);
        assert_eq!((page(&seat).row, page(&seat).group()), (last_of_layout + 1, 1));
        keys(&mut seat, &mut ed, &[Key::Char('G')]);
        assert_eq!(page(&seat).row, all_rows().len() - 1);
        keys(&mut seat, &mut ed, &[Key::Char('g'), Key::Char('g')]);
        assert_eq!(page(&seat).row, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A switch is written at once, `u` writes it back, `C-r` again**
    /// (2026-11-08, save model A): no 「未保存」, nothing to ask on leaving.
    #[test]
    fn a_change_is_written_at_once_and_u_takes_it_back() {
        let dir = scratch("write");
        let (mut ed, mut seat) = seated(&dir, "# mine\n[editor]\nindent = 4\n", "editor.soft_wrap");
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert!(file(&dir).contains("soft_wrap = false") && file(&dir).contains("# mine"), "{}", file(&dir));
        assert!(ed.status().contains("config.toml"), "{}", ed.status());
        assert!(ed.take_config_reload(), "and it takes effect");
        keys(&mut seat, &mut ed, &[Key::Char(' ')]);
        assert!(file(&dir).contains("soft_wrap = true"), "Space flips it back: {}", file(&dir));
        keys(&mut seat, &mut ed, &[Key::Char('u')]);
        assert!(file(&dir).contains("soft_wrap = false"));
        assert!(ed.status().starts_with(&say!("settings.undone", "").chars().take(2).collect::<String>()), "{}", ed.status());
        keys(&mut seat, &mut ed, &[Key::Char('u'), Key::Char('u')]);
        assert_eq!(file(&dir), "# mine\n[editor]\nindent = 4\n");
        assert_eq!(ed.status(), say!("settings.nothing-to-undo"));
        keys(&mut seat, &mut ed, &[Key::Ctrl('r')]);
        assert!(file(&dir).contains("soft_wrap = false"));
        // `q` closes, with nothing to ask.
        keys(&mut seat, &mut ed, &[Key::Char('q')]);
        assert!(seat.panel.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Enter on a choice opens its list**, each value with what it means;
    /// `j` and Enter pick one, Esc puts the list away; a theme is tried as
    /// the cursor walks, put back on Esc, kept on Enter.
    #[test]
    fn enter_on_a_choice_opens_a_list_and_a_theme_is_tried() {
        let dir = scratch("choice");
        let (mut ed, mut seat) = seated(&dir, "", "editor.margin");
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert_eq!(page(&seat).popup_word(), Some("dense"));
        let drawn = shot(page(&seat), 110, 40);
        assert!(drawn.contains("loose") && drawn.contains(&said("settings.margin.loose")), "{drawn}");
        assert!(drawn.contains(&say!("settings.keys-list").chars().take(6).collect::<String>()), "{drawn}");
        keys(&mut seat, &mut ed, &[Key::Char('j'), Key::Enter]);
        assert!(file(&dir).contains("margin = \"loose\""), "{}", file(&dir));
        assert!(page(&seat).popup.is_none());

        let mut seat2 = seat;
        page_mut(&mut seat2).go_to("theme.name", |_| Vec::new());
        keys(&mut seat2, &mut ed, &[Key::Enter, Key::Char('j')]);
        assert!(matches!(seat2.take_preview(), Some(Preview::Show(name)) if name == "one-dark"));
        keys(&mut seat2, &mut ed, &[Key::Esc]);
        assert_eq!(seat2.take_preview(), Some(Preview::Undo));
        assert!(!file(&dir).contains("[theme]"));
        keys(&mut seat2, &mut ed, &[Key::Enter, Key::Char('j'), Key::Char('j'), Key::Enter]);
        let _ = seat2.take_preview();
        assert!(file(&dir).contains("name = \"github\""), "{}", file(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn page_mut(seat: &mut Seat) -> &mut Panel {
        seat.panel.as_mut().expect("open")
    }

    /// **A number is typed on the `:` line's own editor**, its name before
    /// it; Enter writes a value the config takes, and a value it refuses is
    /// typed again with the reason under the row (2026-11-08).
    #[test]
    fn a_number_is_typed_on_the_command_row_and_checked() {
        let dir = scratch("number");
        let (mut ed, mut seat) = seated(&dir, "", "editor.pages_vertical");
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert_eq!(ed.mode(), Mode::Value);
        assert_eq!(ed.value_typed(), Some("1"));
        assert!(ed.prompt_label().unwrap().contains(&say!("settings.editor.pages-vertical")));
        let drawn = shot(page(&seat), 110, 34);
        assert!(drawn.contains(&say!("settings.keys-editing").chars().take(8).collect::<String>()), "{drawn}");
        // The `:` line's own keys: C-u empties it.
        keys(&mut seat, &mut ed, &[Key::Ctrl('u')]);
        assert_eq!(ed.value_typed(), Some(""));
        keys(&mut seat, &mut ed, &chars("99"));
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert_eq!(ed.mode(), Mode::Value, "typed again");
        assert_eq!(ed.value_typed(), Some("99"));
        assert!(!file(&dir).contains("pages_vertical"));
        let drawn = shot(page(&seat), 110, 34);
        let why = say!("settings.refused.out-of-range", say!("settings.range", 1, 4));
        let (_, at) = find_on(&drawn, &why);
        let (_, row) = find_on(&drawn, &say!("settings.editor.pages-vertical"));
        assert_eq!(at, row + 1, "under the row:\n{drawn}");
        keys(&mut seat, &mut ed, &[Key::Backspace, Key::Backspace, Key::Char('3'), Key::Enter]);
        assert_eq!(ed.mode(), Mode::Normal);
        assert!(file(&dir).contains("pages_vertical = 3"), "{}", file(&dir));
        assert!(page(&seat).refusal.is_none());
        // Esc leaves the value as it is.
        keys(&mut seat, &mut ed, &[Key::Char(' '), Key::Char('4'), Key::Esc]);
        assert_eq!(ed.mode(), Mode::Normal);
        assert!(file(&dir).contains("pages_vertical = 3"));
        assert!(seat.panel.is_some(), "Esc on the editor does not close the panel");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`r` takes this file's line away**, and says so when there is none.
    #[test]
    fn r_resets_to_the_layer_below() {
        let dir = scratch("reset");
        let (mut ed, mut seat) = seated(&dir, "[editor]\nindent = 4\nmeasure = 30\n", "editor.indent");
        keys(&mut seat, &mut ed, &[Key::Char('r')]);
        assert!(!file(&dir).contains("indent"), "{}", file(&dir));
        keys(&mut seat, &mut ed, &[Key::Char('r')]);
        assert_eq!(ed.status(), say!("settings.nothing-to-drop"));
        keys(&mut seat, &mut ed, &[Key::Char('j')]);
        assert_eq!(ed.status(), "", "a line about a row goes when the cursor moves");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`p` and ⌘V put the paste in place of the value**, in the editor for
    /// Enter to keep; `y` copies the value; nowhere to paste says so.
    #[test]
    fn p_and_a_paste_replace_the_value_and_y_copies_it() {
        let dir = scratch("paste");
        let (mut ed, mut seat) = seated(&dir, "", "export.page");
        keys(&mut seat, &mut ed, &[Key::Char('y')]);
        assert_eq!(ed.register_contents(None), "a5");
        assert_eq!(ed.status(), say!("settings.yanked", "a5"));
        ed.yank_text("b6\n");
        keys(&mut seat, &mut ed, &[Key::Char('p')]);
        assert_eq!(ed.value_typed(), Some("b6"));
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert!(file(&dir).contains("page = \"b6\""), "{}", file(&dir));
        assert!(seat.pasted(&mut ed, "letter\n"));
        assert_eq!(ed.value_typed(), Some("letter"));
        // While typing, a paste is the line's, at its caret.
        assert!(!seat.pasted(&mut ed, "x"));
        ed.paste_text("-");
        assert_eq!(ed.value_typed(), Some("letter-"));
        keys(&mut seat, &mut ed, &[Key::Esc]);
        page_mut(&mut seat).go_to("editor.soft_wrap", |_| Vec::new());
        keys(&mut seat, &mut ed, &[Key::Char('p')]);
        assert_eq!(ed.status(), say!("settings.paste-nowhere"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`/` narrows the list as it is typed**, across groups; Enter keeps
    /// the search, Esc in the list clears it, Esc again closes the panel.
    #[test]
    fn slash_searches_every_setting() {
        let dir = scratch("search");
        let (mut ed, mut seat) = seated(&dir, "", "editor.layout");
        keys(&mut seat, &mut ed, &[Key::Char('/')]);
        assert_eq!(ed.mode(), Mode::Value);
        keys(&mut seat, &mut ed, &chars("font"));
        let rows: Vec<String> = page(&seat).rows().iter().map(|s| s.path()).collect();
        assert!(rows.contains(&"export.font_family".to_string()), "{rows:?}");
        assert!(!rows.contains(&"editor.layout".to_string()), "{rows:?}");
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert_eq!(page(&seat).search, "font");
        let drawn = shot(page(&seat), 110, 34);
        assert!(drawn.contains("/font"), "{drawn}");
        assert!(drawn.contains(&say!("settings.keys-searched").chars().take(6).collect::<String>()), "{drawn}");
        keys(&mut seat, &mut ed, &[Key::Esc]);
        assert!(page(&seat).search.is_empty() && page(&seat).found.is_none());
        // By name in the reader's language, and by key.
        keys(&mut seat, &mut ed, &[Key::Char('/')]);
        keys(&mut seat, &mut ed, &chars("scrolloff"));
        assert_eq!(page(&seat).here().map(|s| s.key), Some("scrolloff"));
        keys(&mut seat, &mut ed, &[Key::Esc]);
        assert!(page(&seat).found.is_none(), "Esc on the line clears it");
        keys(&mut seat, &mut ed, &[Key::Esc]);
        assert!(seat.panel.is_none(), "Esc in the list closes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`:` opens a line that swallows what follows** (2026-11-09,
    /// reported: `:e notes.md` ran `e` as the panel's key, opened the config
    /// file and typed the rest into it). Enter on a command says the panel
    /// takes none; `q` and its kin close it; nothing reaches a panel key or
    /// the document.
    #[test]
    fn a_colon_line_swallows_the_command_and_q_closes() {
        let dir = scratch("colon");
        let (mut ed, mut seat) = seated(&dir, "[editor]\nindent = 4\n", "editor.soft_wrap");
        let text = ed.current_buffer().rope().to_string();
        keys(&mut seat, &mut ed, &[Key::Char(':')]);
        assert_eq!(ed.mode(), Mode::Value);
        assert_eq!(ed.prompt_label().as_deref(), Some(":"));
        keys(&mut seat, &mut ed, &chars("e notes.md"));
        assert!(seat.panel.is_some() && ed.mode() == Mode::Value, "still the line's");
        assert_eq!(ed.value_typed(), Some("e notes.md"));
        let drawn = shot(page(&seat), 110, 34);
        assert!(drawn.contains(&say!("settings.keys-command").chars().take(8).collect::<String>()), "{drawn}");
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert!(seat.panel.is_some(), "the panel stays");
        assert_eq!(ed.status(), say!("settings.not-a-command", "e notes.md"));
        assert_eq!(file(&dir), "[editor]\nindent = 4\n", "no key of the panel's ran");
        assert_eq!(ed.current_buffer().rope().to_string(), text, "the document is not touched");
        assert!(ed.current_buffer().path().is_none_or(|p| !p.ends_with("config.toml")));
        // A space or `j` typed after `:` is the line's too.
        keys(&mut seat, &mut ed, &[Key::Char(':'), Key::Char(' '), Key::Char('j'), Key::Esc]);
        assert!(file(&dir).contains("indent = 4") && !file(&dir).contains("soft_wrap"), "{}", file(&dir));
        assert_eq!(page(&seat).here().map(|s| s.key), Some("soft_wrap"), "the cursor did not move");
        assert!(seat.panel.is_some(), "Esc on the line leaves the panel open");
        for closing in ["q", "q!", "wq", "x", "wq!"] {
            page_mut(&mut seat).typing = None;
            keys(&mut seat, &mut ed, &[Key::Char(':')]);
            keys(&mut seat, &mut ed, &chars(closing));
            keys(&mut seat, &mut ed, &[Key::Enter]);
            assert!(seat.panel.is_none(), ":{closing} closes the panel");
            assert_eq!(ed.mode(), Mode::Normal);
            seat.panel = seated(&dir, "", "editor.soft_wrap").1.panel;
            ed.set_settings_open(true);
        }
        // In a choice's list too.
        page_mut(&mut seat).go_to("editor.margin", |_| Vec::new());
        keys(&mut seat, &mut ed, &[Key::Enter, Key::Char(':'), Key::Char('j'), Key::Esc]);
        assert_eq!(page(&seat).popup_word(), Some("dense"), "j went to the line, not the list");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`:settings <name>` on the panel's `:` line goes to that setting**
    /// (2026-11-09, reported: 「not a command」), and so does `:set <name>`;
    /// a name there is not is said so. The line takes keys as they are, as
    /// the editor's own `:` does: the IME does not compose there.
    #[test]
    fn settings_on_the_colon_line_goes_to_the_setting() {
        let dir = scratch("colon-settings");
        let (mut ed, mut seat) = seated(&dir, "", "editor.soft_wrap");
        keys(&mut seat, &mut ed, &[Key::Char(':')]);
        assert!(ed.value_line_is_plain(), "a command, not a value: no IME");
        keys(&mut seat, &mut ed, &chars("settings editor.scrolloff"));
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert_eq!(page(&seat).here().map(|s| s.path()).as_deref(), Some("editor.scrolloff"));
        assert_ne!(ed.status(), say!("settings.not-a-command", "settings editor.scrolloff"));
        keys(&mut seat, &mut ed, &[Key::Char(':')]);
        keys(&mut seat, &mut ed, &chars("set editor.indent"));
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert_eq!(page(&seat).here().map(|s| s.path()).as_deref(), Some("editor.indent"));
        keys(&mut seat, &mut ed, &[Key::Char(':')]);
        keys(&mut seat, &mut ed, &chars("settings no.such"));
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert_eq!(ed.status(), say!("settings.no-such-setting", "no.such"));
        assert_eq!(page(&seat).here().map(|s| s.path()).as_deref(), Some("editor.indent"), "stays");
        assert!(seat.panel.is_some());
        // A value typed is still the IME's.
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert!(ed.mode() == Mode::Value && !ed.value_line_is_plain());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A click away from a theme's list puts the theme back** (2026-11-09),
    /// as Esc does, on a row or on a group; **`q` in the list closes the
    /// panel**, the theme put back too.
    #[test]
    fn leaving_a_themes_list_puts_the_theme_back() {
        let dir = scratch("untry");
        let (mut ed, mut seat) = seated(&dir, "", "theme.name");
        keys(&mut seat, &mut ed, &[Key::Enter, Key::Char('j')]);
        assert!(matches!(seat.take_preview(), Some(Preview::Show(_))));
        let drawn = shot(page(&seat), 110, 40);
        let above = page(&seat).rows()[page(&seat).row - 1].label;
        let (x, y) = find_on(&drawn, &said(above));
        assert!(seat.mouse(&mut ed, Pointer::Click, x, y));
        assert!(page(&seat).popup.is_none());
        assert_eq!(seat.take_preview(), Some(Preview::Undo), "a click on a row");
        page_mut(&mut seat).go_to("theme.name", |_| Vec::new());
        keys(&mut seat, &mut ed, &[Key::Enter, Key::Char('j')]);
        let _ = seat.take_preview();
        let drawn = shot(page(&seat), 110, 40);
        let (x, y) = find_on(&drawn, &format!("  {}", said(GROUPS[1].label)));
        assert!(seat.mouse(&mut ed, Pointer::Click, x + 2, y));
        assert_eq!(seat.take_preview(), Some(Preview::Undo), "a click on a group");
        page_mut(&mut seat).go_to("theme.name", |_| Vec::new());
        keys(&mut seat, &mut ed, &[Key::Enter, Key::Char('j')]);
        let _ = seat.take_preview();
        keys(&mut seat, &mut ed, &[Key::Char('q')]);
        assert!(seat.panel.is_none(), "q closes the panel from the list");
        assert_eq!(seat.take_preview(), Some(Preview::Undo));
        assert!(!file(&dir).contains("[theme]"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A file changed outside is read again and said once** (2026-11-09):
    /// the panel shows its new value, a change and its `u` start from it,
    /// and a file deleted outside gets only the new line.
    #[test]
    fn a_file_changed_outside_is_read_again_and_said_once() {
        let dir = scratch("outside");
        let (mut ed, mut seat) = seated(&dir, "[editor]\nscrolloff = 3 # a\nindent = 2\n", "editor.scrolloff");
        // A second apart, so the stamp tells it on any file system.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("config.toml"), "[editor]\nscrolloff = 9 # a\nindent = 2\nruler = 80\n").unwrap();
        seat.watch(&mut ed);
        assert_eq!(ed.status(), say!("settings.changed-outside", shorten(&dir.join("config.toml"))));
        assert_eq!(page(&seat).shown(page(&seat).here().unwrap()).value, "9");
        seat.watch(&mut ed);
        ed.set_status(String::new());
        seat.watch(&mut ed);
        assert_eq!(ed.status(), "", "once");
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert_eq!(ed.value_typed(), Some("9"), "the editor starts from the file as it is");
        keys(&mut seat, &mut ed, &[Key::Ctrl('u'), Key::Char('5'), Key::Enter]);
        assert_eq!(file(&dir), "[editor]\nscrolloff = 5 # a\nindent = 2\nruler = 80\n");
        keys(&mut seat, &mut ed, &[Key::Char('u')]);
        assert_eq!(file(&dir), "[editor]\nscrolloff = 9 # a\nindent = 2\nruler = 80\n", "back to 9, not 3");
        // Deleted outside: read as empty, said, and only the new line written.
        std::fs::remove_file(dir.join("config.toml")).unwrap();
        keys(&mut seat, &mut ed, &[Key::Char('j')]);
        assert_eq!(ed.status(), say!("settings.changed-outside", shorten(&dir.join("config.toml"))));
        assert!(!page(&seat).global.there);
        page_mut(&mut seat).go_to("editor.soft_wrap", |_| Vec::new());
        keys(&mut seat, &mut ed, &[Key::Char(' ')]);
        assert!(file(&dir).contains("soft_wrap = false") && !file(&dir).contains("scrolloff"), "{}", file(&dir));
        // Found by the key that opens the value line, which covers the news:
        // the write says it instead.
        std::fs::remove_file(dir.join("config.toml")).unwrap();
        page_mut(&mut seat).go_to("editor.ruler", |_| Vec::new());
        keys(&mut seat, &mut ed, &[Key::Enter, Key::Ctrl('u'), Key::Char('8'), Key::Char('0'), Key::Enter]);
        assert_eq!(ed.status(), say!("settings.saved-moved", shorten(&dir.join("config.toml"))));
        assert_eq!(file(&dir).trim(), "[editor]\nruler = 80");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`:settings` takes the rest of the line as the name** (2026-11-09):
    /// spaces in it, quotes or not, in any language the panel speaks — the
    /// simplified name finds the row drawn in Traditional.
    #[test]
    fn settings_takes_a_name_with_spaces_in_any_script() {
        let mut ed = Editor::new();
        let mut seat = Seat::default();
        for name in ["first-line indent", "\"first-line indent\"", "First-Line Indent", "首行缩进", "首行縮進", "'首行缩进'"] {
            ed.execute(":settings editor.scrolloff").unwrap();
            seat.settle(&mut ed);
            ed.execute(&format!(":settings {name}")).unwrap();
            seat.settle(&mut ed);
            assert_eq!(page(&seat).here().map(|s| s.label), Some("settings.editor.indent"), "{name}");
        }
        ed.execute(":settings no such thing").unwrap();
        seat.settle(&mut ed);
        assert_eq!(ed.status(), say!("settings.no-such-setting", "no such thing"));
    }

    /// **The keys at the foot are keys that do something** (2026-11-09): no
    /// `y` or `r` on a page's row, no Enter, Space or `r` when a search found
    /// nothing.
    #[test]
    fn the_foot_lists_only_keys_that_work_here() {
        let mut p = Panel::open(None, None);
        let foot = |p: &Panel| shot(p, 200, 34).lines().rev().take(4).collect::<Vec<_>>().join("\n");
        p.go_to("keys.flavor_items", |_| Vec::new());
        let drawn = foot(&p);
        assert!(drawn.contains(&say!("settings.keys-page-row")), "{drawn}");
        for gone in ["r ", "y ", "p "] {
            assert!(!say!("settings.keys-page-row").contains(&format!(", {gone}")), "{gone}");
        }
        p.go_to("ime.annotation_modes", |_| Vec::new());
        assert!(foot(&p).contains(&say!("settings.keys-page-row")));
        p.search = "zzzq".into();
        p.narrow(Some(Vec::new()));
        let drawn = foot(&p);
        assert!(drawn.contains(&say!("settings.keys-no-match")), "{drawn}");
        for gone in ["Enter", "␣", "Space", ", r "] {
            assert!(!say!("settings.keys-no-match").contains(gone), "{gone}");
        }
    }

    /// **A guide and a paragraph mark are one character** (2026-11-09,
    /// reported: `""` and `"ab"` were written), refused with the loader's own
    /// words.
    #[test]
    fn a_single_character_setting_refuses_two_or_none() {
        let dir = scratch("one-char");
        let (mut ed, mut seat) = seated(&dir, "", "editor.indent_symbol");
        for bad in ["", "ab"] {
            keys(&mut seat, &mut ed, &[Key::Enter, Key::Ctrl('u')]);
            keys(&mut seat, &mut ed, &chars(bad));
            keys(&mut seat, &mut ed, &[Key::Enter]);
            assert_eq!(ed.mode(), Mode::Value, "{bad:?} typed again");
            assert!(matches!(&page(&seat).refusal, Some(Refusal::Loader(said)) if said.contains("indent_symbol")), "{bad:?}");
            keys(&mut seat, &mut ed, &[Key::Esc]);
        }
        assert!(!file(&dir).contains("indent_symbol"));
        keys(&mut seat, &mut ed, &[Key::Enter, Key::Ctrl('u'), Key::Char('¶'), Key::Enter]);
        assert!(file(&dir).contains("indent_symbol = \"¶\""), "{}", file(&dir));
        page_mut(&mut seat).go_to("editor.indent_guides_character", |_| Vec::new());
        keys(&mut seat, &mut ed, &[Key::Enter, Key::Ctrl('u'), Key::Char('字'), Key::Enter]);
        assert_eq!(ed.mode(), Mode::Value, "a guide is one cell wide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`Tab` switches the file written into**, and the mark of a row set
    /// in that file follows it; the source says the other file too.
    #[test]
    fn tab_switches_files_and_rows_say_where_their_value_comes_from() {
        let dir = scratch("tab");
        std::fs::write(dir.join("local.toml"), "[editor]\nindent = 2\n").unwrap();
        let (mut ed, mut seat) = seated(&dir, "[editor]\nindent = 4\n", "editor.indent");
        let drawn = shot(page(&seat), 110, 34);
        let line = drawn.lines().find(|l| l.contains(&say!("settings.editor.indent"))).unwrap().to_string();
        assert!(line.contains('*'), "set in the global file: {line}");
        let both = say!("settings.from-both", say!("settings.layer.local"), say!("settings.layer.global"));
        assert!(line.contains(&both), "{line}");
        keys(&mut seat, &mut ed, &[Key::Tab]);
        assert_eq!(page(&seat).into, Which::Local);
        keys(&mut seat, &mut ed, &[Key::Char('r')]);
        assert_eq!(std::fs::read_to_string(dir.join("local.toml")).unwrap().trim(), "");
        let drawn = shot(page(&seat), 110, 34);
        let line = drawn.lines().find(|l| l.contains(&say!("settings.editor.indent"))).unwrap().to_string();
        assert!(!line.contains('*') && line.contains(&say!("settings.layer.global")), "{line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`e` opens the file at the row's line**.
    #[test]
    fn e_opens_the_file_at_the_rows_line() {
        let dir = scratch("e");
        let (mut ed, mut seat) = seated(&dir, "[editor]\nindent = 4\nscrolloff = 8\n", "editor.scrolloff");
        keys(&mut seat, &mut ed, &[Key::Char('e')]);
        assert!(seat.panel.is_none(), "the panel makes way");
        assert!(ed.current_buffer().path().unwrap().ends_with("config.toml"));
        assert_eq!(ed.cursor_line(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The mouse is the panel's** (2026-11-08): a click picks a row, a
    /// group or a value in a list, the wheel walks, the document is not
    /// touched.
    #[test]
    fn the_mouse_picks_rows_and_never_reaches_the_document() {
        let dir = scratch("mouse");
        let (mut ed, mut seat) = seated(&dir, "", "editor.layout");
        let caret = ed.cursor();
        let drawn = shot(page(&seat), 110, 34);
        let (x, y) = find_on(&drawn, "每縱字數");
        assert!(seat.mouse(&mut ed, Pointer::Click, x, y));
        assert_eq!(page(&seat).here().map(|s| s.key), Some("zong_length"));
        let marks = said(GROUPS[1].label);
        let (x, y) = find_on(&drawn, &format!("  {marks}"));
        assert!(seat.mouse(&mut ed, Pointer::Click, x + 2, y));
        assert_eq!(page(&seat).group(), 1);
        let row = page(&seat).row;
        assert!(seat.mouse(&mut ed, Pointer::Wheel { back: false, notches: 3 }, 0, 0));
        assert_eq!(page(&seat).row, row + 3);
        // A value in a list.
        page_mut(&mut seat).go_to("editor.margin", |_| Vec::new());
        keys(&mut seat, &mut ed, &[Key::Enter]);
        let drawn = shot(page(&seat), 110, 40);
        let (x, y) = find_on(&drawn, " always");
        assert!(seat.mouse(&mut ed, Pointer::Click, x + 1, y));
        assert!(file(&dir).contains("margin = \"always\""), "{}", file(&dir));
        assert!(seat.mouse(&mut ed, Pointer::Click, 109, 0), "taken, though nothing is there");
        assert_eq!(ed.cursor(), caret, "the document's caret did not move");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The description line is always there**: what the row does, what it
    /// takes and its factory value, what 0 means.
    #[test]
    fn the_description_says_what_a_row_takes() {
        let mut p = Panel::open(None, None);
        p.go_to("editor.zong_length", |_| Vec::new());
        let drawn = shot(&p, 120, 40);
        let range = say!("settings.range-or-zero", 8, 64);
        assert!(drawn.contains(&range), "{drawn}");
        p.go_to("editor.margin", |_| Vec::new());
        let drawn = shot(&p, 120, 40);
        assert!(drawn.contains(&name_of(said("settings.margin.loose"))), "{drawn}");
        p.go_to("editor.soft_wrap", |_| Vec::new());
        let lines = note_lines(&p);
        assert_eq!(lines[1], say!("settings.about.tick", "[x]"));
    }

    /// **`:settings <name>` opens the panel on that setting**; a name there
    /// is not says so.
    #[test]
    fn settings_with_a_name_opens_on_it() {
        let mut ed = Editor::new();
        let mut seat = Seat::default();
        ed.execute(":settings editor.scrolloff").unwrap();
        seat.settle(&mut ed);
        assert_eq!(page(&seat).here().map(|s| s.key), Some("scrolloff"));
        ed.execute(":settings nosuch").unwrap();
        seat.settle(&mut ed);
        assert_eq!(ed.status(), say!("settings.no-such-setting", "nosuch"));
        assert_eq!(page(&seat).here().map(|s| s.key), Some("scrolloff"), "where it was");
    }

    /// **A file that cannot be read is said at the top**, its layer is
    /// unknown rather than unset, and a change to it writes nothing and says
    /// why (2026-11-08).
    #[test]
    fn an_unreadable_file_is_said_and_not_written() {
        let dir = scratch("broken");
        let (mut ed, mut seat) = seated(&dir, "[editor]\nscrolloff = 8\nlayout = \n", "editor.soft_wrap");
        let drawn = shot(page(&seat), 140, 34);
        assert!(drawn.contains("第 3 行"), "{drawn}");
        assert!(drawn.contains(&say!("settings.layer.unknown")), "{drawn}");
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert!(ed.status().contains("config.toml") && ed.status().contains("第 3 行"), "{}", ed.status());
        assert_eq!(file(&dir), "[editor]\nscrolloff = 8\nlayout = \n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A list cut off says how many rows are out of sight** (2026-11-08).
    #[test]
    fn a_list_cut_off_says_how_many_more() {
        let mut p = Panel::open(None, None);
        let drawn = shot(&p, 80, 24);
        assert!(drawn.contains('↓') && !drawn.contains('↑'), "{drawn}");
        p.last();
        let drawn = shot(&p, 80, 24);
        assert!(drawn.contains('↑') && !drawn.contains('↓'), "{drawn}");
    }

    /// **The value column stays put while a value changes** (2026-11-08).
    #[test]
    fn the_value_column_does_not_move_with_a_value() {
        let dir = scratch("column");
        let (mut ed, mut seat) = seated(&dir, "", "export.font_family");
        let column = |p: &Panel| {
            let drawn = shot(p, 120, 34);
            let line = drawn.lines().find(|l| l.contains(&say!("settings.export.page"))).unwrap().to_string();
            line.find("a5").map(|at| yumete_cjk::str_width(&line[..at]))
        };
        let before = column(page(&seat));
        keys(&mut seat, &mut ed, &[Key::Enter, Key::Ctrl('u')]);
        keys(&mut seat, &mut ed, &chars("serif"));
        assert_eq!(column(page(&seat)), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A page opens from its row and Esc comes back**, its keys at the foot.
    #[test]
    fn a_page_opens_from_its_row_and_esc_comes_back() {
        let dir = scratch("page");
        let (mut ed, mut seat) = seated(&dir, "", "keys.flavor_items");
        keys(&mut seat, &mut ed, &[Key::Enter]);
        assert_eq!(page(&seat).page, Some(0));
        let drawn = shot(page(&seat), 110, 34);
        assert!(drawn.contains(&said("keys.flavor.item.tables")), "{drawn}");
        assert!(drawn.contains(&say!("settings.keys-flavor-page").chars().take(6).collect::<String>()), "{drawn}");
        keys(&mut seat, &mut ed, &[Key::Esc]);
        assert_eq!(page(&seat).page, None);
        assert!(seat.panel.is_some(), "back on the row, not out");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`h` `l` go to the group before and after** (2026-11-09), as the left
    /// column lists them, and a group a search left empty is passed over.
    #[test]
    fn h_and_l_walk_the_groups() {
        let dir = scratch("groups");
        let (mut ed, mut seat) = seated(&dir, "", "editor.scrolloff");
        let at = page(&seat).group();
        keys(&mut seat, &mut ed, &[Key::Char('l')]);
        assert_eq!(page(&seat).group(), at + 1);
        let first = page(&seat).rows().iter().position(|s| s.group == GROUPS[at + 1].group);
        assert_eq!(Some(page(&seat).row), first, "on the group's first row");
        keys(&mut seat, &mut ed, &[Key::Char('h'), Key::Char('h')]);
        assert_eq!(page(&seat).group(), at.saturating_sub(1));
        keys(&mut seat, &mut ed, &[Key::Char('g'), Key::Char('g'), Key::Left]);
        assert_eq!(page(&seat).group(), 0, "the first group stays");
        keys(&mut seat, &mut ed, &[Key::Char('G'), Key::Right]);
        assert_eq!(page(&seat).group(), GROUPS.len() - 1, "the last group stays");
        // A search that leaves two groups: `l` goes from one to the other.
        let all = all_rows();
        let a = all.iter().position(|s| s.group == GROUPS[0].group).unwrap();
        let b = all.iter().position(|s| s.group == GROUPS[GROUPS.len() - 1].group).unwrap();
        seat.panel.as_mut().unwrap().narrow(Some(vec![a, b]));
        keys(&mut seat, &mut ed, &[Key::Char('g'), Key::Char('g'), Key::Char('l')]);
        assert_eq!(page(&seat).group(), GROUPS.len() - 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A theme's value is its name in English too** (2026-11-08).
    #[test]
    fn a_name_is_cut_at_either_colon() {
        assert_eq!(name_of("墨香：黑、白金".into()), "墨香");
        assert_eq!(name_of("moxiang: black, white-gold, gold and red ink; every other shade computed".into()), "moxiang");
        assert_eq!(name_of("横排".into()), "横排");
    }

    /// **No line opens on a comma** (禁則, 2026-11-02).
    #[test]
    fn a_folded_line_never_opens_on_punctuation() {
        for wide in 4..30 {
            for row in fold("在 helix/vim 键位之上的改动。基本：看得见的改动，如菜单、新键；完整：再加。", wide) {
                assert!(!row.starts_with(['，', '。', '、', '；', '：']), "{wide}: {row:?}");
            }
        }
    }

    /// 打勾與幾選一戴方括號，數字不戴；不認得的詞照原樣畫。
    #[test]
    fn a_tick_and_a_pick_wear_brackets() {
        let find = |key: &str| yumete_config::settings_ui::SETTINGS.iter().find(|s| s.key == key).unwrap();
        assert_eq!(drawn(find("soft_wrap"), "true"), "[x]");
        assert_eq!(drawn(find("pages_vertical"), "3"), "3");
        assert_eq!(drawn(find("layout"), "\"vertcal\""), "[vertcal]");
    }

    /// Pasted text made one line.
    #[test]
    fn a_paste_is_made_one_line() {
        assert_eq!(one_line("a,\n  b"), "a, b");
        assert_eq!(one_line("a5\r\n"), "a5");
        assert_eq!(one_line("a\tb\u{7}"), "a b");
    }
}
