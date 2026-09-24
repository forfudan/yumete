//! **那扇設置面板，畫出來** —— Feature #421（2026-09-24）。
//!
//! 狀態在 [`yumete_config::panel`]（那一頭讀得到設置表，核心讀不到）；這裏只管
//! 把它畫成一頁，和把鍵交給它。
//!
//! ```text
//!  設置                                      [全局] ~/.config/yumete/config.toml
//!  ─────────────┬──────────────────────────────────────────────────────────────
//!   版面        │  排版方向          [橫排]              出廠
//!   標記        │  每縱字數           0                  出廠
//!   界面        │  首行縮進           2                  本項目 ← 出廠 0
//!  ─────────────┴──────────────────────────────────────────────────────────────
//!   整頁的字是橫着走還是竪着走
//!   hl 換欄  jk 上下  空格 切換  i 改  d 撤掉  Tab 換存到哪一份  :w 存  q 走
//! ```
//!
//! ⚠️ **第三欄說的是「這一項現在從哪一層來」**，後面跟着被它蓋掉的那一個值。不寫
//! 出來的話，一個讀者看見「首行縮進 2」而他記得自己全局設的是 0，就只能去翻檔。

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::widgets::Clear;
use ratatui::Frame;

use yumete_config::panel::{Layer, Pane, Panel};
use yumete_config::settings_ui::{Kind, Setting, GROUPS};
use yumete_config::Config;
use yumete_core::input::Key;
use yumete_core::Editor;
use yumete_core::say;

/// **`say!`，但標籤是個變量。**
///
/// ⚠️ `say!` 只收字面量（那是有意的：`messages.rs` 那張「每個標籤都有條目」的網
/// 是讀源碼找的，一個算出來的標籤它看不見）。這一扇面板的標籤**全部**來自
/// `settings_ui` 那張表，所以它們是變量——而那張網照樣看得見，因為它們在表上寫成
/// `label:` 與 `hint:`，那兩個開口是 2026-09-24 為此開的。
fn said(tag: &str) -> String {
    yumete_core::messages::say(tag, &[])
}

use crate::put_text;

/// 左邊那張組單子有多寬（全角格）。
const GROUPS_WIDE: u16 = 14;

/// **右邊那三欄從哪裏開始**，以及它們各自到哪裏為止。
///
/// ⚠️ **量出來的，不是寫死的。** 從前是兩個常量（22 與 38），於是 56 欄以下那扇
/// 面板**只剩一列標籤**，什麽值都看不見，而窄窗口下沒有任何降級。2026-09-24 審
/// 出來的。
///
/// ⚠️ **值那一欄有右邊界。** 從前它的上限是整頁的右邊，於是一個長值直接蓋到
/// 「從哪一層來」那一欄上（`layout = "a-very-long-word"`，而 [`drawn`] 有意照原
/// 樣畫不認得的詞）。`Kind::Text` 那一族填進表之後那就是常態。
struct Columns {
    /// 名字那一欄從哪裏開始。
    name: u16,
    /// 值那一欄。
    value: u16,
    /// 「從哪一層來」那一欄；`None` ＝ 窄到擺不下，這一欄不畫。
    layer: Option<u16>,
    /// 頁的右邊。
    right: u16,
}

impl Columns {
    fn of(area: Rect) -> Columns {
        let right = area.x + area.width;
        let name = area.x + GROUPS_WIDE + 2;
        let room = right.saturating_sub(name);
        // ⚠️ **按比例算再鉗住，不是純按比例。** 純比例在寬窗口下把三欄拉得老遠
        // ——名字最長五個漢字（10 格），而四成的 110 欄是 38 格，中間攤着一片空
        // 白。鉗上去之後寬窗口是緊的，窄窗口照舊按比例縮。
        let name_wide = (room * 30 / 100).clamp(14, 24);
        let value_wide = (room * 22 / 100).clamp(12, 18);
        let value = name + name_wide;
        // 窄到擺不下第三欄就不畫它——一欄殘缺的「出廠」比沒有這一欄難讀。
        let layer = (room >= name_wide + value_wide + 16).then(|| value + value_wide);
        Columns { name, value, layer, right }
    }

    /// 值那一欄畫到哪裏為止。
    fn value_ends(&self) -> u16 {
        self.layer.unwrap_or(self.right).saturating_sub(1).max(self.value)
    }
}

/// **一鍵，交給面板。**
///
/// 回來的是「這一鍵之後面板還開着嗎」——`false` ＝ 該關了。
///
/// ⚠️ **`:` 不在這裏接。** 它要交給編輯器去開命令行，`:w` 與 `:q` 就是在那條命令
/// 行上打的（核心認得「面板開着」，於是那兩條說的是面板）。一扇面板自己再長一條
/// 命令行出來，是第二套規矩。
pub fn press(panel: &mut Panel, key: Key) -> bool {
    // 正在打字：除了 Enter 與 Esc，都是字。
    if let Some(typing) = panel.typing.as_mut() {
        match key {
            Key::Enter => panel.finish_typing(),
            Key::Esc => panel.cancel_typing(),
            Key::Backspace => {
                typing.pop();
            }
            Key::Char(c) => typing.push(c),
            _ => {}
        }
        return true;
    }
    match key {
        Key::Char('q') | Key::Esc => return false,
        Key::Char('j') | Key::Down => panel.step(true),
        Key::Char('k') | Key::Up => panel.step(false),
        Key::Char('h') | Key::Left => panel.across(false),
        Key::Char('l') | Key::Right | Key::Enter => panel.across(true),
        Key::Char(' ') => match panel.pane {
            // 左欄上的空格是「進去」，不是「切換」——那一欄沒有東西可切。
            Pane::Groups => panel.across(true),
            Pane::Settings => panel.press(),
        },
        Key::Char('i') => panel.begin_typing(),
        Key::Char('d') => panel.drop_here(),
        Key::Tab => panel.flip_into(),
        Key::Char('g') => {
            panel.row = 0;
            panel.group = match panel.pane {
                Pane::Groups => 0,
                Pane::Settings => panel.group,
            };
        }
        Key::Char('G') => {
            let last = |n: usize| n.saturating_sub(1);
            match panel.pane {
                // ⚠️ **換組要把行號歸零**，同 `step`。不歸的話，在十二行的組裏
                // `G` 到末行、`h`、`G`、`l` 進一個三行的組，`here()` 是 `None`
                // ——沒有一行高亮，空格／`i`／`d` 全沒反應，看起來像面板卡住。
                Pane::Groups => {
                    panel.group = last(GROUPS.len());
                    panel.row = 0;
                }
                Pane::Settings => panel.row = last(panel.rows().len()),
            }
        }
        _ => {}
    }
    true
}


/// **那扇面板在前端的一個座位** —— 開、收鍵、存、關，一整圈。
///
/// ⚠️ **這一支存在的理由是「別寫兩遍」。** 主循環（`run`）與 `--keys`
/// （`yumete/src/main.rs` 的 `press`）都要走同一套，而 2026-09-24 那兩份各抄一遍
/// 之後**當天就分岔了**：`--keys` 那一份漏了「有改動不許一下走」的閘，又把存盤
/// 的錯 `let _ =` 吞掉。兩份代碼一個行為，第三次分岔只是時間問題。
///
/// 連帶一件好處：面板最要緊的那條安全行為（走之前先問一句）**拍得到照片了**——
/// 從前它只在主循環裏，而這個倉審前端就是靠拍照。
#[derive(Default)]
pub struct Seat {
    /// 開着的那一扇；`None` ＝ 沒開。
    pub panel: Option<Panel>,
    /// 有改動沒存時按過一次「走」——再按一次纔算數。
    warned: bool,
}

impl Seat {
    /// **這一鍵歸面板嗎？** 歸就收下並回 `true`（呼叫方 `continue`）。
    ///
    /// ⚠️ **`:` 不歸。** 它交給編輯器去開命令行，`:w`／`:q` 就是在那條行上打的
    /// （核心認得 `settings_open`，於是那幾條說的是面板）。命令行已經開着的時候
    /// 當然也不歸，否則那條命令打不完。
    pub fn took(&mut self, editor: &mut Editor, key: Option<Key>) -> bool {
        let Some(page) = self.panel.as_mut() else { return false };
        if editor.mode() == yumete_core::input::Mode::Command {
            return false;
        }
        if key == Some(Key::Char(':')) {
            return false;
        }
        // ⚠️ **認不出的鍵也算收下了**：面板開着的時候一個 F13 不該掉進正文。
        let Some(key) = key else { return true };
        let dirty = page.dirty();
        match press(page, key) {
            true => self.warned = false,
            // ⚠️ **有改動沒存就不許一下走掉**：面板上攢的東西一個鍵都沒落到磁碟
            // 上，走了就是全丟。第一下說一句，第二下纔算數。
            false => self.leave(editor, dirty),
        }
        true
    }

    /// 核心那幾張條子：開過 `:settings` 沒有、按過 `:w` 沒有、按過 `:q` 沒有。
    pub fn settle(&mut self, editor: &mut Editor) {
        // ⚠️ **已經開着就什麽都不做。** 無條件重開會把攢着沒存的改動一聲不吭地
        // 丟掉，而 `q` 與 `:q` 都有兩段式的閘。
        if editor.take_settings_request() && self.panel.is_none() {
            let local = std::env::current_dir()
                .ok()
                .map(|cwd| yumete_config::panel::local_sheet_path(&cwd));
            self.panel = Some(Panel::open(
                Some(yumete_config::config_dir().join("config.toml")),
                local,
            ));
            self.warned = false;
            editor.set_settings_open(true);
            editor.set_status(String::new());
        }
        if editor.take_settings_save() {
            if let Some(page) = self.panel.as_mut() {
                let named = page
                    .sheet()
                    .path
                    .clone()
                    .map(|p| shorten(&p))
                    .unwrap_or_default();
                let said = match page.dirty() {
                    false => Ok(None),
                    true => page.save().map(Some),
                };
                editor.set_status(match said {
                    Ok(None) => say!("set.nothing-to-save"),
                    // 這期間那個檔被外面改過 —— 說一句，那比「有幾個鍵註釋掉了」
                    // 要緊：眼前這一頁的值可能已經不是檔裏寫的了。
                    Ok(Some(spoke)) if !spoke.changed_underneath.is_empty() => {
                        say!("set.saved-moved", named)
                    }
                    Ok(Some(spoke)) if spoke.commented_out.is_empty() => say!("set.saved", named),
                    Ok(Some(spoke)) => say!(
                        "set.saved-said",
                        named,
                        spoke.commented_out.join(&say!("label.comma"))
                    ),
                    Err(why) => say!("set.cannot-save", why),
                });
                self.warned = false;
            }
        }
        // `force` ＝ 打了 `!`（`:q!`），那時不問「有改動沒存」。
        if let Some(force) = editor.take_settings_close() {
            let dirty = self.panel.as_ref().is_some_and(|p| p.dirty()) && !force;
            self.leave(editor, dirty);
        }
    }

    /// 走 —— 有沒存的東西就先說一句，第二下纔真的走。
    fn leave(&mut self, editor: &mut Editor, dirty: bool) {
        match dirty && !self.warned {
            true => {
                self.warned = true;
                editor.set_status(say!("set.leaving-unsaved"));
            }
            false => {
                self.panel = None;
                self.warned = false;
                editor.set_settings_open(false);
                editor.set_status(String::new());
            }
        }
    }
}

/// 一個值畫成什麽樣。
///
/// ⚠️ **打勾與幾選一戴的是同一副方括號** —— 同搜索面板那條規矩（2026-09-23
/// 作者：「這樣用戶就知道這裏是可以空格切換的」）。數字與一段字不戴：它們按空格
/// 也動，但動的是「加一」而不是「換一個」，戴上會讓人以為只有兩三個值。
fn drawn(setting: &Setting, value: &str) -> String {
    match setting.kind {
        Kind::Tick => match value {
            "true" => "[x]".to_string(),
            _ => "[ ]".to_string(),
        },
        Kind::Pick(choices) => {
            let word = value.trim_matches('"');
            let label = choices
                .iter()
                .find(|c| c.word == word)
                .map(|c| said(c.label))
                // 檔裏寫了一個我們不認得的詞：照原樣畫出來，別假裝它是第一個。
                .unwrap_or_else(|| word.to_string());
            format!("[{label}]")
        }
        Kind::Count { .. } | Kind::Text => value.trim_matches('"').to_string(),
    }
}

/// **畫一頁。**
///
/// 回來的是硬件光標該站的地方 —— 只有正在打字的時候有，別的時候 `None`。
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

    frame.render_widget(Clear, area);
    let col = Columns::of(area);
    let right = col.right;
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..right {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(ground);
            }
        }
    }

    // ---- 頂上那一行：標題，和改動落到哪一份 --------------------------------
    put_text(buf, area.x + 1, area.y, right, &say!("set.title"), head);
    let sheet = panel.sheet();
    let where_to = format!(
        "[{}] {}",
        said(panel.into.layer().label()),
        match (&sheet.path, sheet.there) {
            (Some(p), true) => shorten(p),
            // ⚠️ **那個檔還沒有就明說，而路徑照畫** —— 本項目那一份多半不存在，
            // 存盤時它是被**建出來**的，而建在哪裏是一件該先看得見的事。
            (Some(p), false) => format!("{}（{}）", shorten(p), say!("set.no-file")),
            (None, _) => say!("set.no-file"),
        }
    );
    let dirty = match panel.dirty() {
        true => format!("{}  {where_to}", say!("set.unsaved")),
        false => where_to,
    };
    // ⚠️ **撞上標題就縮**。從前它只 `saturating_sub`，於是 40 欄的窗口上那條路徑
    // 直接蓋掉「設置」兩個字（`設置全局] ~/.config/…`）。先丟路徑只留 `[全局]`，
    // 再丟不下就整條不畫——標題不許被吃掉。2026-09-24 審出來的。
    let title_ends = area.x + 1 + yumete_cjk::str_width(&say!("set.title")) as u16 + 2;
    let short = format!("[{}]", said(panel.into.layer().label()));
    // ⚠️ **「改了還沒存」是最後纔丟的那一個。** 從前這裏只有兩檔——完整那一行，
    // 或者光一個 `[全局]`——於是 74 欄以下**改了東西沒有任何提示**，而那正是這一
    // 行存在的理由。中間插一檔：徽章 ＋ `[全局]`，路徑先丟。2026-09-24 審出來的。
    let badge = match panel.dirty() {
        true => format!("{}  {short}", say!("set.unsaved")),
        false => short.clone(),
    };
    let line = match right.saturating_sub(title_ends) as usize {
        room if room >= yumete_cjk::str_width(&dirty) + 1 => Some(dirty),
        room if room >= yumete_cjk::str_width(&badge) + 1 => Some(badge),
        room if room >= yumete_cjk::str_width(&short) + 1 => Some(short),
        _ => None,
    };
    if let Some(line) = line {
        let wide = yumete_cjk::str_width(&line) as u16;
        put_text(
            buf,
            right.saturating_sub(wide + 1),
            area.y,
            right,
            &line,
            match panel.dirty() {
                true => changed,
                false => quiet,
            },
        );
    }

    // ---- 兩道橫綫與一道竪綫 ------------------------------------------------
    let split = area.x + GROUPS_WIDE;
    // ⚠️ **`foot` 不許爬到上面那道橫綫之上。** 可用高度 ≤3 時它會落到 `area.y`，
    // 於是 `┬` 畫在 `┴` 下面、說明行蓋在橫綫上、標題被抹掉——整扇面板讀成一團。
    // 2026-09-24 審出來的（110×5）。
    let foot = (area.y + area.height.saturating_sub(3)).max(area.y + 2);
    let rule = |buf: &mut ratatui::buffer::Buffer, y: u16, join: &str| {
        for x in area.x..right {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol("─").set_style(quiet);
            }
        }
        if let Some(cell) = buf.cell_mut((split, y)) {
            cell.set_symbol(join).set_style(quiet);
        }
    };
    let bottom = area.y + area.height;
    if area.y + 1 < bottom {
        rule(buf, area.y + 1, "┬");
    }
    if foot < bottom && foot > area.y + 1 {
        rule(buf, foot, "┴");
    }
    for y in area.y + 2..foot {
        if let Some(cell) = buf.cell_mut((split, y)) {
            cell.set_symbol("│").set_style(quiet);
        }
    }

    // ---- 左邊：八組 --------------------------------------------------------
    for (i, named) in GROUPS.iter().enumerate() {
        let y = area.y + 2 + i as u16;
        if y >= foot {
            break;
        }
        let here = i == panel.group;
        let style = match (here, panel.pane) {
            (true, Pane::Groups) => on,
            (true, Pane::Settings) => head,
            (false, _) => text,
        };
        // 整行鋪底，否則選中的那一格只有字那麽寬，讀起來像一塊污漬。
        let name = said(named.label);
        let pad = (GROUPS_WIDE as usize).saturating_sub(yumete_cjk::str_width(&name) + 2);
        put_text(buf, area.x, y, split, &format!("  {name}{}", " ".repeat(pad)), style);
    }

    // ---- 右邊：這一組的那些項 ----------------------------------------------
    let mut caret = None;
    let rows = panel.rows();
    let from = col.name;
    if rows.is_empty() {
        put_text(buf, from, area.y + 2, right, &say!("set.group.not-yet"), quiet);
    }
    for (i, setting) in rows.iter().enumerate() {
        let y = area.y + 2 + i as u16;
        if y >= foot {
            break;
        }
        let here = i == panel.row && panel.pane == Pane::Settings;
        let shown = panel.shown(setting);
        let label = said(setting.label);
        // 名字 —— 畫到值那一欄為止，長名字截住而不是推開值。
        put_text(
            buf,
            from,
            y,
            col.value.saturating_sub(1),
            &label,
            match here {
                true => on,
                false => text,
            },
        );
        // 值 —— 正在打字的那一項畫的是框裏的字，不是算出來的值。
        let value = match (here, panel.typing.as_ref()) {
            (true, Some(typed)) => format!("{typed}▏"),
            _ => drawn(setting, &shown.value),
        };
        let value_style = match (panel.typing.is_some() && here, shown.from) {
            (true, _) => ground.fg(ink.gold()),
            (false, Layer::Factory) => text,
            // 動過的那些亮一點：一頁十二行裏哪三行是自己設的，要一眼看得出。
            (false, _) => ground.fg(ink.azure()),
        };
        // ⚠️ **切了要看得出來。** `put_text` 到了邊界就停，一聲不吭——「候選的序號」
        // 那九個圈字正好比值欄寬一格，畫出來是 `㊀㊁㊂㊃㊄㊅㊆㊇`，看着像設定裏
        // 只有八個。省略號說出「後面還有」。2026-09-24 拍圖看出來的。
        let room = col.value_ends().saturating_sub(col.value) as usize;
        let value = crate::elide(&value, room);
        put_text(buf, col.value, y, col.value_ends(), &value, value_style);
        if here && panel.typing.is_some() {
            let at = col.value + yumete_cjk::str_width(&value).saturating_sub(1) as u16;
            caret =
                Some(ratatui::layout::Position { x: at.min(col.value_ends().saturating_sub(1)), y });
        }
        // 從哪一層來，以及被蓋掉的那一個 —— 窄窗口下這一欄整個不畫。
        if let Some(at) = col.layer {
            let mut whence = said(shown.from.label());
            if let Some((under, was)) = &shown.under {
                whence.push_str(&format!("  ← {} {}", said(under.label()), drawn(setting, was)));
            }
            put_text(buf, at, y, right, &whence, quiet);
        }
    }

    // ---- 底下兩行：這一項是幹什麽的，和鍵 ----------------------------------
    let note = match (panel.pane, panel.here()) {
        (Pane::Settings, Some(setting)) => {
            let mut note = said(setting.hint);
            // `0` 另有意思的那幾項，把那句話跟在後面——它只在這一項上成立，
            // 擺在總的說明裏等於讓人在別的項上也去想 0。
            if let Kind::Count { zero: Some(tag), .. } = setting.kind {
                note.push_str(&format!("　（{}）", said(tag)));
            }
            // ⚠️ **改的那一層被蓋着就說一句。** 不說的話，一個人在全局上按空格而
            // 本項目壓着這一項，螢幕上那一行紋絲不動——看起來像鍵壞了。
            if panel.shadowed(setting) {
                note.push_str(&format!("　{}", say!("set.shadowed")));
            }
            note
        }
        _ => say!("set.pick-a-group"),
    };
    if foot + 1 < bottom {
        put_text(buf, area.x + 2, foot + 1, right, &note, quiet);
    }
    if foot + 2 < bottom {
        put_text(buf, area.x + 2, foot + 2, right, &say!("set.keys"), quiet);
    }
    caret
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

    /// 畫值那一支：打勾與幾選一戴方括號，數字不戴。
    #[test]
    fn a_tick_and_a_pick_wear_the_same_brackets() {
        let tick = yumete_config::settings_ui::SETTINGS
            .iter()
            .find(|s| s.key == "soft_wrap")
            .unwrap();
        assert_eq!(drawn(tick, "true"), "[x]");
        assert_eq!(drawn(tick, "false"), "[ ]");

        let count = yumete_config::settings_ui::SETTINGS
            .iter()
            .find(|s| s.key == "bands")
            .unwrap();
        assert_eq!(drawn(count, "3"), "3", "數字不戴括號");

        let pick = yumete_config::settings_ui::SETTINGS
            .iter()
            .find(|s| s.key == "layout")
            .unwrap();
        assert!(drawn(pick, "\"vertical\"").starts_with('['));
    }

    /// **檔裏寫了一個不認得的詞，照原樣畫**——不假裝它是第一個選項。
    ///
    /// ⚠️ 假裝的話，一個把 `layout = "vertcal"` 打錯的人會看見面板上寫着「橫排」
    /// 而檔裏寫着別的，兩邊都說不通。
    #[test]
    fn a_word_we_do_not_know_is_drawn_as_it_was_written() {
        let pick = yumete_config::settings_ui::SETTINGS
            .iter()
            .find(|s| s.key == "layout")
            .unwrap();
        assert_eq!(drawn(pick, "\"vertcal\""), "[vertcal]");
    }


    /// **畫一頁，讀回來。** 用真的緩衝區，不是拿眼睛看註釋裏那張草圖。
    fn shot(panel: &Panel, width: u16, height: u16) -> String {
        let config = Config::default();
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("a terminal");
        terminal
            .draw(|frame| {
                draw(frame, panel, &config, Rect::new(0, 0, width, height));
            })
            .expect("one frame");
        crate::buffer_to_text(terminal.backend().buffer())
    }

    /// 八個組名、十二項、兩道橫綫與一道竪綫，都在。
    #[test]
    fn the_page_has_both_columns_and_the_rules_between_them() {
        let page = shot(&Panel::open(None, None), 110, 24);
        for group in ["版面", "標記", "界面", "配色", "輸入法", "編輯", "文件", "鍵盤"] {
            assert!(page.contains(group), "少了組 {group}：\n{page}");
        }
        for row in ["排版方向", "每縱字數", "首行縮進", "標點旁置", "稿紙刻度"] {
            assert!(page.contains(row), "少了 {row}：\n{page}");
        }
        assert!(page.contains('┬') && page.contains('┴') && page.contains('│'), "{page}");
        assert!(page.contains("hl 換欄"), "底下那一行鍵：\n{page}");
    }

    /// **一項都沒設的時候，每一行都說「出廠」。**
    #[test]
    fn an_untouched_config_says_factory_on_every_row() {
        let page = shot(&Panel::open(None, None), 110, 24);
        let rows = page.lines().filter(|l| l.contains('│')).count();
        let factory = page.matches("出廠").count();
        assert!(factory >= 12, "十二項都該說出廠，只有 {factory}：\n{page}");
        assert!(rows >= 12, "{rows} 行：\n{page}");
    }

    /// **本項目蓋了全局，那一行要把被蓋掉的那個也說出來。**
    ///
    /// ⚠️ 不說的話，一個讀者看見「首行縮進 2」而他記得自己全局設的是 4，就只能去
    /// 翻檔案——而這一欄存在的全部理由就是免掉那一趟。
    #[test]
    fn a_row_that_is_overridden_names_what_it_covered() {
        let mut p = Panel::open(None, None);
        // 直接攢兩筆改動，效果和檔裏寫着一樣。
        p.into = yumete_config::panel::Which::Global;
        p.across(true);
        p.row = p.rows().iter().position(|s| s.key == "indent").unwrap();
        p.begin_typing();
        p.typing = Some("4".into());
        p.finish_typing();
        p.flip_into();
        p.begin_typing();
        p.typing = Some("2".into());
        p.finish_typing();

        let page = shot(&p, 110, 24);
        assert!(page.contains("本項目"), "贏的那一層：\n{page}");
        assert!(page.contains("← 全局 4"), "被蓋掉的那一個：\n{page}");
        assert!(page.contains("改了還沒存"), "頂上要說一句：\n{page}");
    }

    /// **矮窗口上不畫到框外面去。** 24 行的表，12 行的窗。
    #[test]
    fn a_short_window_cuts_the_list_rather_than_the_frame() {
        let mut p = Panel::open(None, None);
        p.across(true);
        let page = shot(&p, 90, 12);
        let lines: Vec<&str> = page.lines().collect();
        assert_eq!(lines.len(), 12, "畫了 {} 行：\n{page}", lines.len());
        assert!(page.contains("hl 換欄"), "底下那一行鍵還在：\n{page}");
        assert!(page.contains('┴'), "下面那道橫綫還在：\n{page}");
        // 裝不下的那幾項掉了，而不是畫到框外面。
        assert!(!page.contains("稿紙刻度"), "末一項該被切掉：\n{page}");
    }

    /// **`i` 打字的時候，畫的是框裏那段字加一個光標。**
    #[test]
    fn a_row_being_typed_into_shows_what_is_being_typed() {
        let mut p = Panel::open(None, None);
        p.across(true);
        p.row = p.rows().iter().position(|s| s.key == "zong_length").unwrap();
        p.begin_typing();
        p.typing = Some("32".into());
        let page = shot(&p, 110, 24);
        assert!(page.contains("32▏"), "框裏那段字與光標：\n{page}");
    }

    /// `q` 與 `Esc` 關掉，別的鍵不關。
    #[test]
    fn q_closes_it_and_the_walking_keys_do_not() {
        let mut p = Panel::open(None, None);
        assert!(!press(&mut p, Key::Char('q')));
        assert!(!press(&mut p, Key::Esc));
        for key in [Key::Char('j'), Key::Char('l'), Key::Char(' '), Key::Tab] {
            assert!(press(&mut p, key), "{key:?} 不該關掉它");
        }
    }

    /// **正在打字的時候 `q` 是一個字**，不是「走」。
    #[test]
    fn while_typing_q_is_a_letter() {
        let mut p = Panel::open(None, None);
        p.across(true);
        p.row = p.rows().iter().position(|s| s.key == "bands").unwrap();
        p.begin_typing();
        assert!(press(&mut p, Key::Char('q')), "還開着");
        assert_eq!(p.typing.as_deref(), Some("1q"));
        // Esc 退出打字，而不是關掉面板。
        assert!(press(&mut p, Key::Esc), "還開着");
        assert!(p.typing.is_none());
    }
}
