//! **那扇設置面板的狀態** —— Feature #421（2026-09-24）。
//!
//! 作者提的：
//!
//! > 這樣的話用戶（特別是寫小説的），不需要面對 toml 和一堆 key 發呆不知道他們
//! > 都是幹啥的，也不需要查詢到底有哪些 key 可以用。
//!
//! 整頁一扇，左邊八組、右邊那一組的項；`hl` 換欄、`jk` 上下、空格切換、`i` 改、
//! `d` 撤掉、`Tab` 換存到哪一份、`:w` 存盤、`q` 走。
//!
//! # 為什麽在 `yumete-config` 而不是 `yumete-core`
//!
//! 別的面板（搜索、大綱、字典）狀態都在核心裏，這一扇不行：它讀
//! [`crate::settings_ui`] 那張表、調 [`crate::write_back`] 存盤，而
//! **`yumete-core` 不依賴 `yumete-config`**（看 `Cargo.toml`，那一頭只有
//! `yumete-cjk`）。把表和寫回搬進核心是倒着接依賴；把狀態放在這裏，整件事就都在
//! 一個 crate 裏，前端只管畫。
//!
//! # 三層，而畫出來的是「這一項現在從哪一層來」
//!
//! 出廠 → 全局 → 本項目，後面的蓋前面的。面板右邊那一欄說的是**贏的那一層**，
//! 再後面跟着**被蓋掉的那一個值**——不寫出來的話，一個讀者看見「首行縮進 2」而
//! 他記得自己全局設的是 0，就只能去翻檔案。
//!
//! # `:w`，不是即時生效
//!
//! 作者 2026-09-23 定：「需要 :w，即時生效不好，改錯了都沒辦法反悔。」所以改動
//! 攢在 [`Sheet::edits`] 裏，存盤那一下纔落到檔上，落完由 `:reload config` 那條
//! 同一支推進編輯器。

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::settings_ui::{Kind, Setting, GROUPS, LATER, NOT_IN_THE_PANEL, SETTINGS};
use crate::write_back::{rewrite, Change, Said};

/// 一個值是從哪一層來的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// 誰都沒說，用的是出廠那一個。
    Factory,
    /// 全局 `~/.config/yumete/config.toml`。
    Global,
    /// 本項目最近的 `.yumete/config.toml`。
    Local,
}

/// 一層，和它的名字。
///
/// ⚠️ **寫成 `label:` 字段，不是一支 `match`** —— 同 [`crate::settings_ui::Named`]，
/// 同一個理由，而這一處 2026-09-24 就照着 `match` 寫過一遍、當場被
/// `messages.rs` 那張網逮住（三則文案「沒人說」）。那張網讀源碼找 `label:`／
/// `hint:`／`say!(`，**看不見 `match` 的返回值**。
///
/// ⚠️ 更壞的不是紅，是它**紅得像另一件事**：那條 assert 說的是「a renamed or
/// deleted tag」，照着做就把三則好好的文案刪了，面板第三欄從此逐行寫着
/// `set.layer.factory`。
pub struct LayerName {
    pub layer: Layer,
    pub label: &'static str,
}

/// 三層，出廠在最底下。
pub const LAYERS: &[LayerName] = &[
    LayerName { layer: Layer::Factory, label: "set.layer.factory" },
    LayerName { layer: Layer::Global, label: "set.layer.global" },
    LayerName { layer: Layer::Local, label: "set.layer.local" },
];

impl Layer {
    /// 這一層的名字，給 `say!` 用。
    pub fn label(self) -> &'static str {
        LAYERS
            .iter()
            .find(|n| n.layer == self)
            .map(|n| n.label)
            .unwrap_or("set.layer.factory")
    }
}

/// 改動落到哪一份檔上 —— 頂上那一行切着的那個。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Global,
    Local,
}

impl Which {
    /// Tab 切到另一份。
    pub fn other(self) -> Which {
        match self {
            Which::Global => Which::Local,
            Which::Local => Which::Global,
        }
    }

    /// 它是哪一層。
    pub fn layer(self) -> Layer {
        match self {
            Which::Global => Layer::Global,
            Which::Local => Layer::Local,
        }
    }
}

/// 鍵在左邊那張組單子上，還是右邊那些項上。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    #[default]
    Groups,
    Settings,
}

/// **一份配置檔**：它在哪、正文長什麽樣、以及改了還沒存的那幾項。
#[derive(Debug, Clone, Default)]
pub struct Sheet {
    /// 檔的位置。
    ///
    /// ⚠️ **本項目那一份多半還不存在，而路徑照樣給得出來** —— 面板頂上寫着
    /// 「還沒有這個文件——存盤時建」，那句話得兌現。給 `None` 的話 `save()` 會
    /// 回一句 `Local 那一層沒有檔`，一個**沒翻譯的 Rust Debug 字串**出現在界面
    /// 上，而改動還掛着。2026-09-24 審出來的。
    pub path: Option<PathBuf>,
    /// 那個檔此刻在不在磁碟上。面板拿它決定說路徑還是說「存盤時建」。
    pub there: bool,
    /// 檔裏原樣的那一份。存盤時 [`rewrite`] 改的就是它，所以**一個字節都不能動**。
    pub text: String,
    /// 改了還沒存的：`表.鍵` → 怎麽改。
    pub edits: BTreeMap<String, Change>,
    /// 檔裏此刻寫着的那些值，`表.鍵` → 照 toml 的寫法。
    ///
    /// 從 [`text`](Self::text) 算出來的，開面板時算一次。⚠️ **不是每一幀重算**：
    /// 一幀要問幾十次，而 toml 解析一次是幾十微秒——乘起來就是打字時看得見的頓。
    declared: BTreeMap<String, String>,
}

impl Sheet {
    /// 讀一份檔。讀不到就是空的那一份（要存的時候會建出來）。
    pub fn read(path: Option<PathBuf>) -> Sheet {
        let read = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok());
        let there = read.is_some();
        let text = read.unwrap_or_default();
        Sheet {
            declared: declared_in(&text),
            path,
            there,
            text,
            edits: BTreeMap::new(),
        }
    }

    /// 這一份此刻說 `path` 是什麽 —— 改了還沒存的算數。
    pub fn says(&self, path: &str) -> Option<String> {
        match self.edits.get(path) {
            Some(Change::Drop) => None,
            Some(change) => Some(written(change)),
            None => self.declared.get(path).cloned(),
        }
    }

    /// 這一份有沒有改了還沒存的東西。
    pub fn dirty(&self) -> bool {
        !self.edits.is_empty()
    }
}

/// 一份 toml 裏寫着的每一項，`表.鍵` → 照它寫的那個樣子。
///
/// ⚠️ **值取的是「寫出來的字」而不是解析出來的東西**：面板要照原樣顯示，而
/// `"dense"` 與 `dense` 在螢幕上是兩件事——前者是使用者打的，後者是他打錯的。
fn declared_in(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return out;
    };
    for (table, item) in doc.iter() {
        let Some(t) = item.as_table() else { continue };
        for (key, value) in t.iter() {
            if let Some(v) = value.as_value() {
                out.insert(format!("{table}.{key}"), v.to_string().trim().to_string());
            }
        }
    }
    out
}

/// 一個 [`Change`] 寫進 toml 會是什麽樣子。
fn written(change: &Change) -> String {
    match change {
        Change::Tick(on) => on.to_string(),
        Change::Count(n) => n.to_string(),
        // ⚠️ **要轉義。** 對面 [`declared_in`] 讀的是 `toml_edit` 吐出來的字串，
        // 那一頭轉義過；這一頭不轉，值裏帶一個 `"` 或 `\` 兩邊就永遠對不上，
        // `settle` 判不出「改回原值」，那一筆從此掛着「改了還沒存」。今天沒有
        // 一項是 `Kind::Text`，所以是死代碼——Text 那一行落地那天會咬。
        Change::Text(s) => toml_edit::Value::from(s.as_str()).to_string().trim().to_string(),
        // 撤掉那一項的人看見的不是一個值，[`Sheet::says`] 先接住了。
        Change::Drop => String::new(),
    }
}

/// **一項此刻長什麽樣** —— 面板一行畫的就是這個。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// 贏的那個值，照 toml 的寫法（`0`、`true`、`"dense"`）。
    pub value: String,
    /// 它從哪一層來。
    pub from: Layer,
    /// **被它蓋掉的那一個** —— 沒有就是 `None`（贏的已經是出廠那一層）。
    pub under: Option<(Layer, String)>,
}

/// **那扇面板。**
#[derive(Debug, Clone)]
pub struct Panel {
    pub global: Sheet,
    pub local: Sheet,
    /// 改動落到哪一份上。
    pub into: Which,
    pub pane: Pane,
    /// 左邊那張單子上的第幾組（[`GROUPS`] 的下標）。
    pub group: usize,
    /// 右邊那一組裏的第幾項。
    pub row: usize,
    /// `i` 進來改的那一段字；`None` ＝ 不在改。
    pub typing: Option<String>,
    /// 上一次存盤說的話（哪幾個鍵被註釋掉了）。
    pub said: Option<Said>,
}

/// **本項目那一份在哪** —— 找得到就是它，找不到就是「要建的話建在這裏」。
///
/// ⚠️ **不回 `None`。** 面板明寫着「存盤時建」，而 `save()` 自己會
/// `create_dir_all`；少了這一支，那句承諾就是空頭的。要建哪一個看得見：頂上那
/// 一行寫的就是它。
pub fn local_sheet_path(cwd: &std::path::Path) -> PathBuf {
    crate::local_config_path(cwd).unwrap_or_else(|| cwd.join(".yumete").join("config.toml"))
}

impl Panel {
    /// 開一扇。
    pub fn open(global: Option<PathBuf>, local: Option<PathBuf>) -> Panel {
        Panel {
            global: Sheet::read(global),
            // 本項目那一份多半還不存在；路徑照樣給得出來（[`local_sheet_path`]），
            // 而它在不在寫在 `Sheet::there` 上，面板照着說。
            local: Sheet::read(local),
            into: Which::Global,
            pane: Pane::Groups,
            group: 0,
            row: 0,
            typing: None,
            said: None,
        }
    }

    /// 這一組底下的那些項，畫出來的次序就是 [`SETTINGS`] 的次序。
    pub fn rows(&self) -> Vec<&'static Setting> {
        let Some(named) = GROUPS.get(self.group) else {
            return Vec::new();
        };
        SETTINGS.iter().filter(|s| s.group == named.group).collect()
    }

    /// 光標此刻站的那一項。
    pub fn here(&self) -> Option<&'static Setting> {
        self.rows().get(self.row).copied()
    }

    /// 一項此刻長什麽樣。
    pub fn shown(&self, setting: &Setting) -> Shown {
        let path = setting.path();
        let factory = (Layer::Factory, setting.factory.to_string());
        let global = self.global.says(&path).map(|v| (Layer::Global, v));
        let local = self.local.says(&path).map(|v| (Layer::Local, v));
        // 本項目蓋全局，全局蓋出廠。
        let mut stack = vec![factory];
        if let Some(g) = global {
            stack.push(g);
        }
        if let Some(l) = local {
            stack.push(l);
        }
        let (from, value) = stack.pop().expect("出廠那一層永遠在");
        Shown {
            value,
            from,
            under: stack.pop(),
        }
    }

    /// 改動要落到的那一份。
    fn sheet_mut(&mut self) -> &mut Sheet {
        match self.into {
            Which::Global => &mut self.global,
            Which::Local => &mut self.local,
        }
    }

    /// 改動要落到的那一份（只讀）。
    pub fn sheet(&self) -> &Sheet {
        self.sheet_of(self.into)
    }

    /// 指名那一份。
    fn sheet_of(&self, which: Which) -> &Sheet {
        match which {
            Which::Global => &self.global,
            Which::Local => &self.local,
        }
    }

    /// **切着的那一份此刻替這一項出的是什麽** —— 檔裏寫着的，沒寫就是它**下面**
    /// 那幾層給的。
    ///
    /// ⚠️ **看的是下面，不是「畫出來的那個值」。** 編輯全局而本項目蓋着它的時候，
    /// 畫出來的是本項目那個值——拿它作比，「把全局設成和本項目一樣」就被判成「沒
    /// 改」，而那明明改變了**別的項目**裏的行為。全局底下只有出廠；本項目底下是
    /// 全局，再底下纔是出廠。
    fn without_mine(&self, setting: &Setting) -> String {
        let path = setting.path();
        let mine = self.sheet().declared.get(&path).cloned();
        let under = match self.into {
            // 全局底下只有出廠。
            Which::Global => None,
            // 本項目底下是全局（連它沒存的改動一起算：那是同一次存盤落下去的）。
            Which::Local => self.global.says(&path),
        };
        mine.or(under).unwrap_or_else(|| setting.factory.to_string())
    }

    /// **切着的那一份此刻替這一項出的是什麽，連沒存的改動一起算。**
    ///
    /// ⚠️ **按空格與 `i` 的起點是這個，不是畫出來的那個值。** 畫出來的是三層裏
    /// **贏**的那一個；本項目蓋着全局的時候你編輯全局，起點要是全局自己那個值。
    /// 拿畫出來的值當起點，「空格」會從別人的值往下數——按一下 1 變 3。
    /// 2026-09-24 一條測試逮到的。
    pub fn mine_now(&self, setting: &Setting) -> String {
        self.sheet()
            .says(&setting.path())
            .unwrap_or_else(|| self.without_mine(setting))
    }

    /// **切着的那一份被上面那一層蓋着嗎。**
    ///
    /// ⚠️ 蓋着的時候改這一項，**螢幕上那一行不會動**（贏的還是上面那個），看起來
    /// 像按了沒反應。所以底下那一行要說一句。
    pub fn shadowed(&self, setting: &Setting) -> bool {
        self.into == Which::Global && self.local.says(&setting.path()).is_some()
    }

    /// **攢一筆改動 —— 除非改完和不改一樣。**
    ///
    /// ⚠️ 不問這一句，按空格轉一圈回到原來那個值**還算「改了」**：面板頂上寫着
    /// 「改了還沒存」，而 `:w` 會往檔裏寫一行跟原來一模一樣的東西。同一族的還有
    /// 「把一項設成和出廠一樣」——那一行寫進去純屬噪音。2026-09-24 拍圖看出來的。
    fn settle(&mut self, setting: &Setting, change: Change) {
        let path = setting.path();
        match written(&change) == self.without_mine(setting) {
            true => {
                self.sheet_mut().edits.remove(&path);
            }
            false => {
                self.sheet_mut().edits.insert(path, change);
            }
        }
    }

    // ---- 走 -------------------------------------------------------------

    /// `j` / `k`。
    pub fn step(&mut self, down: bool) {
        let deep = self.rows().len();
        let (at, many) = match self.pane {
            Pane::Groups => (&mut self.group, GROUPS.len()),
            Pane::Settings => (&mut self.row, deep),
        };
        if many == 0 {
            return;
        }
        *at = match down {
            true => (*at + 1).min(many - 1),
            false => at.saturating_sub(1),
        };
        // 換了組，右邊那一欄要從頭數。
        if self.pane == Pane::Groups {
            self.row = 0;
        }
    }

    /// `h` / `l`。
    ///
    /// ⚠️ **空的那一組進不去**：七組還沒填，`l` 進去會停在一張空表上，而那一刻
    /// `j`／`k`／空格全部沒有反應——看起來像面板卡住了。
    pub fn across(&mut self, right: bool) {
        self.typing = None;
        self.pane = match (right, self.rows().is_empty()) {
            (true, false) => Pane::Settings,
            (true, true) => Pane::Groups,
            (false, _) => Pane::Groups,
        };
    }

    /// `Tab` —— 換存到哪一份。
    pub fn flip_into(&mut self) {
        self.typing = None;
        self.into = self.into.other();
    }

    // ---- 改 -------------------------------------------------------------

    /// 空格 —— 打勾翻面、幾選一走下一個、數字加一（到頂回到底）。
    ///
    /// ⚠️ **一段自己打的字沒有「下一個」**，空格對它就是 `i`。
    pub fn press(&mut self) {
        let Some(setting) = self.here() else { return };
        let now = self.mine_now(setting);
        let next = match setting.kind {
            Kind::Tick => Change::Tick(now != "true"),
            // ⚠️ **`0` 另算一檔，而 `low` 是「最小的非零值」。** `zong_length` 真
            // 正的域是「0，或者 4 到 64」（`into_config` 寫着
            // `if length == 0 {0} else {length.clamp(4,64)}`），`tatechuyoko` 是
            // 「0，或者 2 到 8」。從前 `low` 寫 0，於是面板停得到 1／2／3，寫進
            // 檔裏而**編輯器按 4 排版**——面板顯示的值不是編輯器用的值，這個倉最
            // 怕的那一族。2026-09-24 審出來的。
            Kind::Count { low, high, zero } => {
                let n: usize = now.parse().unwrap_or(low);
                // ⚠️ **`0` 只在 `low > 0` 的時候是域外那一檔。** `indent` 的
                // `zero` 只是替域裏的 0 取了個名字（「不縮進」，域是 0..8）；
                // 不分這一下，`indent` 在 0 上按空格會回到 `low` ＝ 0，那一格死住。
                let apart = zero.is_some() && low > 0;
                let next = match () {
                    _ if apart && n == 0 => low,
                    _ if n >= high && apart => 0,
                    _ if n >= high => low,
                    _ => (n + 1).max(low),
                };
                Change::Count(next as i64)
            }
            Kind::Pick(choices) => {
                let word = now.trim_matches('"');
                let at = choices.iter().position(|c| c.word == word).unwrap_or(0);
                let next = choices[(at + 1) % choices.len()].word;
                Change::Text(next.to_string())
            }
            Kind::Text => {
                self.typing = Some(now.trim_matches('"').to_string());
                return;
            }
        };
        self.settle(setting, next);
    }

    /// `i` —— 進去打字。數字與一段字纔有得打。
    pub fn begin_typing(&mut self) {
        let Some(setting) = self.here() else { return };
        if matches!(setting.kind, Kind::Tick | Kind::Pick(_)) {
            return;
        }
        self.typing = Some(self.mine_now(setting).trim_matches('"').to_string());
    }

    /// 打完了 —— `Enter`。打的不是個數就原樣不動。
    pub fn finish_typing(&mut self) {
        let Some(typed) = self.typing.take() else { return };
        let Some(setting) = self.here() else { return };
        let change = match setting.kind {
            Kind::Count { low, high, zero } => match typed.trim().parse::<usize>() {
                // `0` 是域外那一檔的時候，它是合法的，不鉗到 `low`。
                Ok(0) if zero.is_some() && low > 0 => Change::Count(0),
                Ok(n) => Change::Count(n.clamp(low, high) as i64),
                // ⚠️ **打了個不是數的東西就當沒改**，不寫一個 0 進去：使用者敲錯
                // 一個鍵而設定被撥成 0，比什麽都沒發生壞得多。
                Err(_) => return,
            },
            _ => Change::Text(typed),
        };
        self.settle(setting, change);
    }

    /// `Esc` —— 不改了。
    pub fn cancel_typing(&mut self) {
        self.typing = None;
    }

    /// `d` —— 把這一項從**切着的那一份**裏撤掉，回去跟下面那一層。
    pub fn drop_here(&mut self) {
        let Some(setting) = self.here() else { return };
        let path = setting.path();
        let sheet = self.sheet_mut();
        // ⚠️ **問的是「檔裏寫了沒有」，不是「這一份此刻說什麽」。** 後者把還沒存
        // 的改動也算進去，於是「改了一項，又按 `d` 反悔」會攢下一條刪一個根本不
        // 存在的鍵的指令——面板頂上從此寫着「改了還沒存」，而其實什麽都不用存。
        // 2026-09-24 拍圖看出來的。
        match sheet.declared.contains_key(&path) {
            true => sheet.edits.insert(path, Change::Drop),
            false => sheet.edits.remove(&path),
        };
    }

    // ---- 存 -------------------------------------------------------------

    /// 有沒有改了還沒存的。
    pub fn dirty(&self) -> bool {
        self.global.dirty() || self.local.dirty()
    }

    /// **`:w`** —— 兩份各寫各的，寫完把改動清掉。
    ///
    /// 回來的是「有話說的那幾句」：不認得的鍵被註釋掉了哪幾個。
    pub fn save(&mut self) -> Result<Said, String> {
        let mut said = Said::default();
        for which in [Which::Global, Which::Local] {
            let sheet = match which {
                Which::Global => &mut self.global,
                Which::Local => &mut self.local,
            };
            if sheet.edits.is_empty() {
                continue;
            }
            let Some(path) = sheet.path.clone() else {
                return Err(format!("{:?} 那一層沒有檔", which));
            };
            let changes: Vec<(String, Change)> =
                sheet.edits.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let (fresh, mut spoke) = rewrite(&sheet.text, &changes, &known)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            write_atomically(&path, &fresh).map_err(|e| format!("{}: {e}", path.display()))?;
            sheet.text = fresh;
            sheet.declared = declared_in(&sheet.text);
            sheet.edits.clear();
            said.commented_out.append(&mut spoke.commented_out);
        }
        self.said = Some(said.clone());
        Ok(said)
    }
}

/// **寫一份檔，要麽整份是新的，要麽整份是舊的。**
///
/// 寫暫存檔 → `sync_all` → `rename`，同 `yumete-core` 存稿子那一支
/// （`buffer.rs::write_atomically`，那一頭不能借：核心不依賴這個 crate，反過來
/// 更不行）。
///
/// ⚠️ **直接 `fs::write` 有一個真的窗口**：它先把檔截成 0 再寫。寫到一半斷電、
/// 磁碟滿、程序被殺——使用者的 `config.toml` 就剩半份，而那半份多半讀不通，於是
/// **整份設定作廢**（`deny_unknown_fields` 之外的第二種全丟法）。一個人只是想
/// 把縮進改成 2。
///
/// ⚠️ **`rename` 換的是目録項，所以要連目録一起 `sync`**，否則斷電後目録可能還
/// 指着舊的那個 inode——新內容落了盤，而沒有人找得到它。
fn write_atomically(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".yumete-set-{}-{}", std::process::id(), nanos));
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        // 原來那份檔的權限，留着：`File::create` 吃 umask，於是一份 0600 的配置
        // 存一次就成了 0644。
        if let Ok(from) = std::fs::metadata(path) {
            let _ = file.set_permissions(from.permissions());
        }
        file.write_all(text.as_bytes())?;
        file.flush()?;
        // flush 只清掉本行程的緩衝。rename 落了盤而數據塊沒落，斷電後是一份全零。
        file.sync_all()
    })();
    // 每一條出路都把目録還原成它來時的樣子——磁碟滿的那一次不留一個
    // `.yumete-set-…` 在人家的 `.config` 裏。
    if let Err(err) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// yumete 認得這個鍵嗎 —— [`rewrite`] 拿它決定要不要註釋掉一行。
///
/// ⚠️ **三張表的並集，不是只有 [`SETTINGS`]。** 只問畫在面板上的那些，會把
/// `[editor] screenshot`（有理由不進面板）和七十多個還沒輪到的鍵在存盤那一下
/// 全部註釋掉——使用者按了一下保存，配置被改了一半。
fn known(table: &str, key: &str) -> bool {
    let path = format!("{table}.{key}");
    SETTINGS.iter().any(|s| s.path() == path)
        || LATER.contains(&path.as_str())
        || NOT_IN_THE_PANEL.contains(&path.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(global: &str, local: &str) -> Panel {
        let mut p = Panel::open(None, None);
        p.global = Sheet {
            path: Some(PathBuf::from("/nowhere/config.toml")),
            there: true,
            text: global.to_string(),
            edits: BTreeMap::new(),
            declared: declared_in(global),
        };
        p.local = Sheet {
            path: Some(PathBuf::from("/nowhere/.yumete/config.toml")),
            there: true,
            text: local.to_string(),
            edits: BTreeMap::new(),
            declared: declared_in(local),
        };
        p
    }

    fn find(path: &str) -> &'static Setting {
        SETTINGS.iter().find(|s| s.path() == path).expect(path)
    }

    /// 三層，後面的蓋前面的，**而被蓋掉的那一個說得出來**。
    #[test]
    fn the_layer_that_wins_is_named_and_so_is_the_one_under_it() {
        let p = panel("[editor]\nindent = 4\n", "[editor]\nindent = 2\n");
        let shown = p.shown(find("editor.indent"));
        assert_eq!(shown.value, "2");
        assert_eq!(shown.from, Layer::Local);
        assert_eq!(shown.under, Some((Layer::Global, "4".to_string())));

        // 誰都沒說 —— 出廠那一層贏，下面沒有東西了。
        let bare = panel("", "");
        let shown = bare.shown(find("editor.indent"));
        assert_eq!((shown.value.as_str(), shown.from, shown.under), ("0", Layer::Factory, None));
    }

    /// 空格：打勾翻面、幾選一走下一個、數字加一。
    #[test]
    fn a_press_moves_each_kind_the_way_that_kind_moves() {
        let mut p = panel("", "");
        p.pane = Pane::Settings;

        // 打勾（soft_wrap 出廠 true）。
        p.row = p.rows().iter().position(|s| s.key == "soft_wrap").unwrap();
        p.press();
        assert_eq!(p.shown(find("editor.soft_wrap")).value, "false");

        // 幾選一（layout 出廠 "horizontal"）。
        p.row = p.rows().iter().position(|s| s.key == "layout").unwrap();
        p.press();
        assert_eq!(p.shown(find("editor.layout")).value, "\"vertical\"");
        p.press();
        assert_eq!(p.shown(find("editor.layout")).value, "\"horizontal\"", "轉回來");

        // 數字（bands 出廠 1，範圍 1..4）。
        p.row = p.rows().iter().position(|s| s.key == "bands").unwrap();
        p.press();
        assert_eq!(p.shown(find("editor.bands")).value, "2");
    }

    /// 數字到頂回到底 —— 不然按到 4 之後那一格就死了。
    #[test]
    fn a_count_at_its_ceiling_comes_back_to_the_floor() {
        let mut p = panel("[editor]\nbands = 4\n", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "bands").unwrap();
        p.press();
        assert_eq!(p.shown(find("editor.bands")).value, "1");
    }

    /// `d` 撤掉這一層，回去跟下面那一層。
    #[test]
    fn dropping_this_layer_falls_back_to_the_one_under_it() {
        let mut p = panel("[editor]\nindent = 4\n", "[editor]\nindent = 2\n");
        p.into = Which::Local;
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "indent").unwrap();
        p.drop_here();
        let shown = p.shown(find("editor.indent"));
        assert_eq!((shown.value.as_str(), shown.from), ("4", Layer::Global));
    }

    /// **改了一項又按 `d` 反悔，該乾乾淨淨**——不是攢一條刪一個不存在的鍵的指令。
    ///
    /// ⚠️ 攢下來的話，面板頂上從此寫着「改了還沒存」而其實什麽都不用存，走的時候
    /// 還要多問一句「有改動沒存」。2026-09-24 拍圖看出來的。
    #[test]
    fn changing_a_row_and_then_dropping_it_leaves_nothing_behind() {
        let mut p = panel("", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "measure").unwrap();
        p.begin_typing();
        p.typing = Some("36".to_string());
        p.finish_typing();
        assert!(p.dirty(), "改過了");
        p.drop_here();
        assert!(!p.dirty(), "反悔之後乾淨了：{:?}", p.global.edits);
        assert_eq!(p.shown(find("editor.measure")).value, "0");
    }

    /// **轉一圈回到原來那個值，就不算改過。**
    ///
    /// ⚠️ 算的話，`:w` 會往檔裏寫一行跟原來一模一樣的東西，而面板頂上一直亮着
    /// 「改了還沒存」。
    #[test]
    fn cycling_all_the_way_back_is_not_a_change() {
        let mut p = panel("", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "bands").unwrap();
        for _ in 0..4 {
            p.press();
        }
        assert_eq!(p.shown(find("editor.bands")).value, "1", "1→2→3→4→1");
        assert!(!p.dirty(), "轉回來就不算改過：{:?}", p.global.edits);
    }

    /// 檔裏寫着 3，改成別的再改回 3 —— 同理，不算改過。
    #[test]
    fn setting_a_row_back_to_what_the_file_says_is_not_a_change() {
        let mut p = panel("[editor]\nbands = 3\n", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "bands").unwrap();
        p.press();
        assert!(p.dirty(), "3→4");
        p.press();
        p.press();
        p.press();
        assert_eq!(p.shown(find("editor.bands")).value, "3", "4→1→2→3");
        assert!(!p.dirty(), "回到檔裏那個值：{:?}", p.global.edits);
    }

    /// **「把全局設成和本項目一樣」是一次真的改動**，不是「沒改」。
    ///
    /// ⚠️ 判成沒改的話，一個想「以後每個項目都這樣」的人按了空格、看見值對了、
    /// `:w` 存了，而全局那個檔一個字沒動——換一個目録打開就打回原形。
    #[test]
    fn matching_what_the_project_says_still_changes_the_global_file() {
        let mut p = panel("", "[editor]\nbands = 2\n");
        p.into = Which::Global;
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "bands").unwrap();
        // 出廠 1 → 按一下是 2，和本項目那個值一樣。
        p.press();
        assert!(p.dirty(), "全局那一份真的改了：{:?}", p.global.edits);
        assert_eq!(p.global.says("editor.bands").as_deref(), Some("2"));
    }

    /// **被上面那一層蓋着的時候，說得出來。**
    #[test]
    fn editing_a_layer_that_is_covered_says_so() {
        let mut p = panel("", "[editor]\nbands = 2\n");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "bands").unwrap();
        p.into = Which::Global;
        assert!(p.shadowed(find("editor.bands")), "本項目蓋着全局");
        p.into = Which::Local;
        assert!(!p.shadowed(find("editor.bands")), "本項目自己在最上面");
    }

    /// **存盤是原子的**：換的是目録項，寫到一半的東西不會出現在那個路徑上。
    ///
    /// ⚠️ 直接 `fs::write` 先把檔截成 0 再寫——斷電或被殺就剩半份，而半份 toml
    /// 多半讀不通，於是整份設定作廢。一個人只是想把縮進改成 2。
    #[test]
    fn saving_replaces_the_file_whole_and_leaves_no_litter() {
        let dir = std::env::temp_dir().join(format!("yumete-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "# 我寫的\n[editor]\nindent = 4    # 理由\n").unwrap();

        let mut p = Panel::open(Some(path.clone()), None);
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "bands").unwrap();
        p.press();
        p.save().expect("存得下去");

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# 我寫的") && after.contains("# 理由"), "{after}");
        assert!(after.contains("bands = 2"), "{after}");
        // 暫存檔不許留在人家的目録裏。
        let litter: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != "config.toml")
            .collect();
        assert!(litter.is_empty(), "留了垃圾：{litter:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 這一份本來就沒說過的，`d` 什麽都不做——而不是攢一條「刪掉」。
    #[test]
    fn dropping_what_this_layer_never_said_writes_nothing() {
        let mut p = panel("[editor]\nindent = 4\n", "");
        p.into = Which::Local;
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "indent").unwrap();
        p.drop_here();
        assert!(!p.dirty(), "空的改動表：{:?}", p.local.edits);
    }

    /// 打了個不是數的東西 —— **當沒改**，不是撥成 0。
    #[test]
    fn typing_something_that_is_not_a_number_changes_nothing() {
        let mut p = panel("[editor]\nindent = 4\n", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "indent").unwrap();
        p.begin_typing();
        p.typing = Some("兩格".to_string());
        p.finish_typing();
        assert!(!p.dirty());
        assert_eq!(p.shown(find("editor.indent")).value, "4");
    }

    /// 打出範圍的數鉗回來。
    #[test]
    fn a_number_out_of_range_is_pulled_back_in() {
        let mut p = panel("", "");
        p.pane = Pane::Settings;
        p.row = p.rows().iter().position(|s| s.key == "bands").unwrap();
        p.begin_typing();
        p.typing = Some("99".to_string());
        p.finish_typing();
        assert_eq!(p.shown(find("editor.bands")).value, "4");
    }

    /// **空的那一組進不去。**
    ///
    /// ⚠️ 七組還沒填，`l` 進去會停在一張空表上，而那一刻 `j`／`k`／空格全部沒有
    /// 反應——看起來像面板卡住了。
    #[test]
    fn a_group_with_nothing_in_it_cannot_be_walked_into() {
        let mut p = panel("", "");
        p.group = GROUPS.iter().position(|n| n.group == crate::settings_ui::Group::Keys).unwrap();
        assert!(p.rows().is_empty(), "鍵盤那一組還沒填");
        p.across(true);
        assert_eq!(p.pane, Pane::Groups, "進不去");
    }

    /// 換組之後光標從頭數起 —— 不然停在一個不存在的行號上。
    #[test]
    fn moving_to_another_group_starts_at_its_first_row() {
        let mut p = panel("", "");
        p.across(true);
        p.row = 5;
        p.across(false);
        p.step(true);
        assert_eq!(p.row, 0);
    }

    /// `known` 認的是**三張表的並集**。
    ///
    /// ⚠️ 只認畫在面板上的那些，存盤那一下會把七十多個還沒輪到的鍵和
    /// `screenshot` 一起註釋掉——按了一下保存，配置被改了一半。
    #[test]
    fn saving_does_not_comment_out_a_key_the_panel_has_not_got_to_yet() {
        assert!(known("editor", "indent"), "面板上的");
        assert!(known("editor", "tab_width"), "還沒輪到的");
        assert!(known("editor", "screenshot"), "有理由不進面板的");
        assert!(!known("editor", "nosuchkey"), "真不認得的");
    }
}
