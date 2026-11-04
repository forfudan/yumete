//! 讓開 — asking the **system's** input method to stand down while yumete is
//! reading the keys itself.
//!
//! ## The problem the terminal creates
//!
//! yumete carries yume. So a writer typing 漢字 in it is served by the input
//! method *inside* the editor, and the one the operating system runs is not
//! wanted — but the operating system does not know that. It sees a terminal
//! window, and a terminal window is a place people type; the system input
//! method takes the keystroke first and yumete never sees it. In Normal mode
//! that means `j` opens a candidate list instead of moving the cursor, and
//! with yume engaged it means two input methods composing the same keys.
//!
//! ## The rule
//!
//! It is one question, not two: **is yumete going to type this key for you?**
//! Both halves of it are already answered elsewhere in this crate, and the
//! rule is the two of them put together —
//! `讓開 ⟺ !composes_here(editor) || (available && engaged)`:
//!
//! | 狀態行 | 鍵是命令 | 打字的地方 |
//! | --- | --- | --- |
//! | `[中 靈明]` | 讓開 | 讓開 |
//! | `[ABC]` | 讓開 | 讓開 |
//! | 什麽都不寫 | 讓開 | **放行** |
//!
//! The column is `composes_here` — the same gate the preedit, the candidate
//! panel and the lone-Shift tap light up with, so 「where 中文 belongs」 has one
//! answer in this file and not four. The row is the tag the status line stands
//! (`standing_language_tag`), which is exactly `available && engaged`.
//!
//! Warning: **Both halves were got wrong once, and each cost an evening.** The first
//! version asked only `engaged`: `[ime] start` is `false` out of the box, so a
//! fresh session is engaged with **no 碼表**, and the rule took the system's
//! input method away from a writer whose yumete could not type 漢字 either —
//! 「你把输入法切走了，我 i 进入 insert 模式用什么？」 The second asked
//! `mode != Normal`, which hands the keyboard back on the **command line** too
//! — and there the command *name* is ASCII, so `:w` had its `w` composed by
//! the system's input method and the file was never written.
//!
//! So the bottom row — no marker, whether from `:yume off` or from a 碼表 that
//! was never loaded — is where the system's input method is exactly what is
//! wanted, and somebody writing Japanese in yumete lives there.
//!
//! ## What is actually done — and what was tried first
//!
//! yume's own input method already has 模態掛起 (`ModalSuspend.swift`), built
//! for exactly this and for exactly these editors: 「讓 helix/vim 這類模態編輯
//! 器在 normal mode 把輸入法整個支開」. Suspended, it hands every key straight
//! back to the terminal, untouched. It does **not** switch the input source, it
//! does **not** touch 中/英 (so a writer who was in ABC comes back to ABC), and
//! it records **which application asked**, so a terminal going quiet does not
//! mute the browser. Two of the three platforms can be told: a distributed
//! notification on macOS, a registered window message on Windows. **Linux
//! cannot** — the fcitx5 addon has no 模態掛起 upstream yet, so there is no
//! signal to send and this module does nothing there (see `backend` below).
//!
//! **The first version of this module switched the input source instead**
//! (`TISSelectInputSource`, the way `im-select` does for vim) on the theory
//! that it would cover every input method rather than only yume. It is left
//! written down here because the measurement cost a whole evening: **that road
//! does not work on macOS.** A background process re-selecting an **IMK** input
//! method fails to re-engage roughly one time in three — the keys then arrive
//! in the buffer as plain letters. Measured with System Events typing into a
//! real window, 漢字 = engaged, letter = not:
//!
//! ```text
//! yumete＋Ghostty   宇浩 [及] [j ] [及]     鼠鬚管 [什么] [什么] [j ]
//! yumete＋Ghostty   宇浩 [及] [及] [j ]     鼠鬚管 [什么] [j ] [什么]
//! TextEdit, 無 yumete 宇浩 及|j |及|及       鼠鬚管 什么|什么|j |什么
//! ```
//!
//! Two IMK input methods, the same failure rate, a random round, and **it fails
//! with no editor and no terminal in the picture at all**. Warning: 蘋果全拼 passing
//! that same test proves nothing — SCIM is built in and does not go through the
//! IMK session protocol, which is why it was the control that pointed the
//! finger at the wrong place for an hour.
//!
//! **And the terminal cannot help.** The kitty keyboard protocol's
//! `REPORT_ALL_KEYS_AS_ESCAPE_CODES` is not a bypass: with it on, the input
//! method still swallows the key *presses* and only the *releases* leak through
//! as escape codes, while the commit is destroyed (that is the 「上屏出來一排
//! 空格」 the manual has always warned about). A control run with a plain
//! ASCII layout received both press and release for every key, so the reading
//! is clean. Zed can do this — stay on the Chinese input method and simply not
//! route keys to it — because Zed owns its NSView. A program inside a terminal
//! does not.
//!
//! So the signal is the mechanism, and it is **fast**: measured 0.2–0.8 ms per
//! post, against **85 ms** for spawning `yume-mode` (which is the whole 12 MB
//! YumeIME binary, loading the Swift runtime to send one notification). Five
//! suspend/resume pairs 0.3 s apart, every one of them exact.
//!
//! Warning: **yumete never needs `once`.** That third signal exists because helix has
//! entries with no exit event — a `/` prompt accepted with Enter, an `r`/`f`/`t`
//! waiting on one character — so the input method has to notice the commit and
//! put itself back. yumete *is* the editor: `Mode::Search`, `Mode::Command` and
//! `Mode::Field` are its own state and it knows the moment each one ends.
//!
//! **A signal used to be a gap, and no longer is.** The resume is sent by
//! [`Drop`], so every ordinary way out of the editor's loop restores it, a
//! panic included — but SIGHUP from a closed terminal window runs no
//! destructor. So the suspend carries `{"pid": …}`, and yume drops a suspension
//! whose owner has gone (upstream `18b43cc`, asked for and built the same
//! evening). The same upstream commit also stopped the **first** suspend being
//! swallowed: it used to be discarded without a word when the terminal's input
//! session had not been handed over yet, which is a race yumete wins at every
//! start-up, and yumete then believed it was suspended and never said so again.

use yumete_config::{OtherImePolicy, SystemImePolicy};

/// A stopwatch on the signal, off unless asked for — `YUMETE_IME_LOG=1` appends
/// to `/tmp/yumete-ime.log`.
///
/// Kept because every fault this feature has had has been one of **timing or
/// order**, and neither can be read off a screenshot.
fn log(line: impl FnOnce() -> String) {
    use std::io::Write as _;
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("YUMETE_IME_LOG").is_some()) {
        return;
    }
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/yumete-ime.log")
    {
        let _ = writeln!(f, "{at:.2} {}", line());
    }
}

/// Asks the system's input method to stand down while yumete wants the keys.
pub struct SystemIme {
    policy: SystemImePolicy,
    /// Whether the terminal has the keyboard at all.
    ///
    /// Losing focus lifts the suspension: the suspension is recorded against
    /// the *application* that asked for it (yume keeps a bundle id, not a
    /// boolean, so that one terminal going quiet does not mute the browser),
    /// and two windows of the same terminal share that identity. Giving it
    /// back when the window is not in front is what keeps the other window
    /// typing.
    focused: bool,
    /// Whether a suspension is outstanding. Only ever changed alongside the
    /// signal that causes it, so the two cannot drift.
    suspended: bool,
    /// **Other input methods** (2026-11-04): whether to switch them to
    /// English, and back. See [`OtherImePolicy`].
    others: OtherImePolicy,
    /// What was switched away from, while it is: `Some` from the switch until
    /// the switch back (or until it is given up, under `aside`).
    switched: Option<source::Saved>,
}

impl SystemIme {
    pub fn new(policy: SystemImePolicy, others: OtherImePolicy) -> Self {
        SystemIme { policy, focused: true, suspended: false, others: here(others), switched: None }
    }

    /// `[ime] others` read again (`:reload-config`): what the old answer
    /// switched aside is let back by the old answer first.
    pub fn set_others(&mut self, others: OtherImePolicy) {
        let others = here(others);
        if others != self.others {
            self.let_others_back();
            self.others = others;
        }
    }

    /// The terminal gained or lost the keyboard.
    ///
    /// Warning: **A resume is not enough on its own.** [`Self::want`] runs on every
    /// turn of the main loop, so lifting the suspension when focus left and
    /// nothing more would have it re-asserted a millisecond later. Losing focus
    /// has to *stop the asking*, not answer it once.
    ///
    /// **On Linux another input method is left as it is while the focus is
    /// away**: fcitx and ibus answer for the window that has the focus, so
    /// switching back then would open the input method of the window just
    /// gone to. The terminal's stays switched aside until Insert asks for it.
    pub fn set_focus(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            match source::PER_WINDOW {
                true => self.resume(),
                false => self.release(),
            }
        }
    }

    /// Ask for the keys, or give them back. Idempotent, and free when the
    /// answer has not changed — it is called on every turn of the main loop.
    pub fn want(&mut self, block: bool) {
        if self.focused || !source::PER_WINDOW {
            self.want_others_aside(block && self.focused);
        }
        if self.policy == SystemImePolicy::Keep {
            return;
        }
        match (block && self.focused, self.suspended) {
            (true, false) => {
                log(|| "-> suspend".to_string());
                backend::suspend();
                self.suspended = true;
            }
            (false, true) => self.release(),
            _ => {}
        }
    }

    /// **說一遍不算數的時候，再說一遍**（2026-09-22 報的）。
    ///
    /// 報的情形：在別的窗口用系統輸入法打了字，切回 yumete（Normal 模式），輸入
    /// 法還開着，於是鍵被吞掉。
    ///
    /// 根子在 [`Self::suspended`] 是一個**信念**：想要鍵而且還没挂起，纔發信號。
    /// 輸入法在別處自己恢復了，這一頭卻還記着「已經挂着了」——於是它再也不說第二
    /// 次。Warning: **同一族的坑源碼裏記着**（見本檔開頭：開機那一次的挂起被丟掉，
    /// yumete 就此相信自己挂着了）。
    ///
    /// 信念會漂，就得有一條重新聲明的路。抹掉信念就夠了：下一輪 [`Self::want`]
    /// 自己會把信號重發一次，而它本來就是冪等的。
    ///
    /// Warning: **不做成「焦點回來自動重發」**——那條保險絲的反面（焦點離開自動解挂）
    /// 試過、撤了，代價是一天幾十次。由人按一下，該重發的時候纔重發。
    pub fn say_it_again(&mut self) {
        log(|| "-> say it again".to_string());
        self.suspended = false;
        // The same for another input method switched back on by hand while
        // the keys were commands: switched aside again now. What comes back
        // in Insert is the one switched from now, when there was one, and
        // the one kept from before when the source was English already (a
        // second Esc out of habit must not lose it).
        if self.switched.is_some() {
            let again = source::take();
            match source::holds(&again) {
                true => {
                    if let Some(old) = self.switched.replace(again) {
                        source::forget(old);
                    }
                }
                false => source::forget(again),
            }
        }
    }

    /// **Other input methods, aside or back** (2026-11-04).
    ///
    /// Aside: whatever is current, unless it is 宇浩 (its own signal above
    /// serves it, and selecting 宇浩 by input source is the road that fails)
    /// or already a plain layout, is switched to the English one. Back: under
    /// `aside-and-back` the one that was on is selected again; under `aside`
    /// it is left to the writer, so the half that fails is never tried.
    fn want_others_aside(&mut self, aside: bool) {
        if self.others == OtherImePolicy::Off {
            return;
        }
        match (aside, self.switched.is_some()) {
            (true, false) => {
                let saved = source::take();
                log(|| format!("-> others aside: {}", source::describe(&saved)));
                self.switched = Some(saved);
            }
            (false, true) => self.let_others_back(),
            _ => {}
        }
    }

    fn let_others_back(&mut self) {
        if let Some(saved) = self.switched.take() {
            match self.others {
                OtherImePolicy::AsideAndBack => {
                    log(|| "-> others back".to_string());
                    source::restore(saved);
                }
                _ => source::forget(saved),
            }
        }
    }

    /// Give the keys back. Safe to call when nothing was asked for.
    pub fn release(&mut self) {
        self.let_others_back();
        self.resume();
    }

    /// The signal half of [`Self::release`]: 宇浩's suspension lifted.
    fn resume(&mut self) {
        if self.suspended {
            log(|| "-> resume".to_string());
            self.suspended = false;
            backend::resume();
        }
    }
}

/// `others` on this machine: **never over SSH**, where the input method is
/// the remote host's, or nothing, and no terminal sequence reaches the local
/// one.
fn here(others: OtherImePolicy) -> OtherImePolicy {
    let remote = std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
    match remote {
        true => OtherImePolicy::Off,
        false => others,
    }
}

impl Drop for SystemIme {
    fn drop(&mut self) {
        self.release();
    }
}

/// **`macOS`** — the distributed notification `ModalSuspend.swift` installs.
///
/// Posted directly rather than by running `yume-mode off`, which is the same
/// signal through a 12 MB binary: 0.2 ms against 85 ms, and the editor's loop
/// is not a place to spend 85 ms on a keystroke.
///
/// Warning: **The two names read backwards from the two commands**, and that is
/// upstream's own note: `yume-mode off` — 「讓輸入法走開」 — posts
/// `modalSuspend.**on**`, because the name says which way the *suspension* is
/// switched, not which way the input method is.
///
/// Nothing here creates an `NSApplication`, which matters: yume records **who**
/// asked by looking at whoever is frontmost, so a sender that stole focus would
/// record itself. A terminal's child process steals nothing.
#[cfg(target_os = "macos")]
mod backend {
    use std::ffi::c_void;

    const SUSPEND: &str = "app.shurufa.inputmethod.Yume.modalSuspend.on";
    const RESUME: &str = "app.shurufa.inputmethod.Yume.modalSuspend.off";
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFNotificationCenterGetDistributedCenter() -> *mut c_void;
        fn CFNotificationCenterPostNotification(
            center: *mut c_void,
            name: *const c_void,
            object: *const c_void,
            user_info: *const c_void,
            deliver_immediately: u8,
        );
        fn CFStringCreateWithBytes(
            allocator: *const c_void,
            bytes: *const u8,
            length: isize,
            encoding: u32,
            is_external: u8,
        ) -> *const c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(object: *const c_void);
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }

    fn string(text: &str) -> *const c_void {
        unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                text.as_ptr(),
                text.len() as isize,
                UTF8,
                0,
            )
        }
    }

    /// `{"pid": "<ours>"}` — the owner, so the suspension lapses on its own.
    ///
    /// A distributed notification carries no sender, so without this a yumete
    /// killed by SIGHUP (the terminal window closed) would leave yume standing
    /// down forever: `Drop` never runs, the resume is never sent, and the only
    /// way back is ⌥Space by hand. Given a pid, yume polls `kill(pid, 0)` once
    /// a second while suspended and releases itself when the owner is gone.
    ///
    /// **Property-list values only** — a distributed notification's dictionary
    /// crosses a process boundary — so the number goes as a string.
    fn owner() -> *const c_void {
        let key = string("pid");
        let value = string(&std::process::id().to_string());
        if key.is_null() || value.is_null() {
            unsafe {
                if !key.is_null() {
                    CFRelease(key);
                }
                if !value.is_null() {
                    CFRelease(value);
                }
            }
            return std::ptr::null();
        }
        unsafe {
            let dict = CFDictionaryCreate(
                std::ptr::null(),
                [key].as_ptr(),
                [value].as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            );
            // The dictionary retains both; ours are spent either way.
            CFRelease(key);
            CFRelease(value);
            dict
        }
    }

    fn post(name: &str, user_info: *const c_void) {
        unsafe {
            let center = CFNotificationCenterGetDistributedCenter();
            let cf = string(name);
            if center.is_null() || cf.is_null() {
                if !cf.is_null() {
                    CFRelease(cf);
                }
                if !user_info.is_null() {
                    CFRelease(user_info);
                }
                return;
            }
            // `deliver_immediately`: the whole point is that the next keystroke
            // is already on its way.
            CFNotificationCenterPostNotification(center, cf, std::ptr::null(), user_info, 1);
            CFRelease(cf);
            if !user_info.is_null() {
                CFRelease(user_info);
            }
        }
    }

    pub fn suspend() {
        post(SUSPEND, owner());
    }

    pub fn resume() {
        post(RESUME, std::ptr::null());
    }
}

/// **Windows** — the registered window message `yume-mode.exe` broadcasts.
///
/// Each text service keeps a message-only window of class `YumeShiftMsgWnd`;
/// the message number is whatever `RegisterWindowMessage` hands both sides for
/// the agreed name. `wparam` is 1 to suspend and 0 to resume, and the TSF build
/// has one such window per host process, so every one that can be found is
/// posted to — exactly what `tools/YumeMode.cpp` does.
///
/// **Never run here**; this machine is macOS. Posting to a window of a process
/// at a higher integrity level is refused by UIPI, which upstream also notes —
/// the failure is silent and means 「this does not apply here」.
#[cfg(windows)]
mod backend {
    use std::ffi::c_void;

    type Hwnd = *mut c_void;
    /// `HWND_MESSAGE`, the parent every message-only window hangs under.
    const HWND_MESSAGE: isize = -3;

    #[link(name = "user32")]
    extern "system" {
        fn RegisterWindowMessageW(name: *const u16) -> u32;
        fn FindWindowExW(parent: Hwnd, after: Hwnd, class: *const u16, title: *const u16) -> Hwnd;
        fn PostMessageW(window: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn broadcast(suspend: bool) {
        unsafe {
            let msg = RegisterWindowMessageW(wide("YumeModalSuspend").as_ptr());
            if msg == 0 {
                return;
            }
            let class = wide("YumeShiftMsgWnd");
            let mut found: Hwnd = std::ptr::null_mut();
            loop {
                found = FindWindowExW(
                    HWND_MESSAGE as Hwnd,
                    found,
                    class.as_ptr(),
                    std::ptr::null(),
                );
                if found.is_null() {
                    return;
                }
                PostMessageW(found, msg, usize::from(suspend), 0);
            }
        }
    }

    pub fn suspend() {
        broadcast(true);
    }

    pub fn resume() {
        broadcast(false);
    }
}

/// **Linux** — nothing to send yet.
///
/// The fcitx5 addon has no 模態掛起 of its own (the `yume-mode-*` names in
/// `Actions.cpp` are menu ids for 輸入模式, a different thing), so there is no
/// signal to post and this does nothing. When one lands upstream it goes here
/// and the rest of this file is already right.
#[cfg(all(unix, not(target_os = "macos")))]
mod backend {
    pub fn suspend() {}
    pub fn resume() {}
}

/// **Other input methods: the input source itself** (2026-11-04).
///
/// The code is the first version of this module (`24116fdc`, 2026-09-16),
/// which switched the input source for 宇浩 too and was replaced by the
/// signal two hours later. For 宇浩 the signal is the better road and stays;
/// for every other input method this is the only road there is, and it is
/// taken only when `[ime] others` asks for it.
///
/// Warning: **On macOS the switch to English is reliable and the switch back is
/// not**: re-selecting an IMK input method from a background process fails
/// to re-engage about one time in three, and `TISSelectInputSource` reports
/// success either way. That is why `aside` never switches back.
#[cfg(target_os = "macos")]
mod source {
    use std::ffi::c_void;

    type InputSource = *mut c_void;

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> InputSource;
        fn TISCopyCurrentASCIICapableKeyboardInputSource() -> InputSource;
        fn TISSelectInputSource(source: InputSource) -> i32;
        fn TISGetInputSourceProperty(source: InputSource, key: *const c_void) -> *const c_void;
        static kTISPropertyInputSourceID: *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(object: *const c_void);
        fn CFEqual(a: *const c_void, b: *const c_void) -> u8;
        fn CFStringGetCString(s: *const c_void, buf: *mut u8, n: isize, enc: u32) -> u8;
    }

    /// The source that was current, owned (+1 from the `Copy` call) until it
    /// is put back or forgotten — or nothing, when there was nothing in the
    /// way.
    pub struct Saved(Option<InputSource>);

    /// The input source is the machine's, not one window's.
    pub const PER_WINDOW: bool = false;

    /// Whether anything was switched aside.
    pub fn holds(saved: &Saved) -> bool {
        saved.0.is_some()
    }

    fn id_of(source: InputSource) -> String {
        unsafe {
            let id = TISGetInputSourceProperty(source, kTISPropertyInputSourceID);
            let mut buf = [0u8; 256];
            let ok = !id.is_null()
                && CFStringGetCString(id, buf.as_mut_ptr(), buf.len() as isize, 0x0800_0100) != 0;
            match ok {
                true => {
                    let end = buf.iter().position(|b| *b == 0).unwrap_or(0);
                    String::from_utf8_lossy(&buf[..end]).into_owned()
                }
                false => String::new(),
            }
        }
    }

    pub fn take() -> Saved {
        unsafe {
            let ascii = TISCopyCurrentASCIICapableKeyboardInputSource();
            let was = TISCopyCurrentKeyboardInputSource();
            let release = |a: InputSource, b: InputSource| {
                if !a.is_null() {
                    CFRelease(a);
                }
                if !b.is_null() {
                    CFRelease(b);
                }
            };
            if ascii.is_null() || was.is_null() {
                release(ascii, was);
                return Saved(None);
            }
            // **Nothing in the way, so nothing is touched**: `…ASCIICapable…`
            // is the current source itself when that is a plain layout, so the
            // two being equal is exactly 「no input method holds the keys」.
            // **And 宇浩 is never switched by source**: its own signal serves it.
            if CFEqual(ascii, was) != 0 || id_of(was).starts_with("app.shurufa.inputmethod.Yume") {
                release(ascii, was);
                return Saved(None);
            }
            let ok = TISSelectInputSource(ascii) == 0;
            CFRelease(ascii);
            match ok {
                true => Saved(Some(was)),
                false => {
                    CFRelease(was);
                    Saved(None)
                }
            }
        }
    }

    pub fn restore(saved: Saved) {
        if let Some(source) = saved.0 {
            unsafe {
                TISSelectInputSource(source);
                CFRelease(source);
            }
        }
    }

    pub fn forget(saved: Saved) {
        if let Some(source) = saved.0 {
            unsafe { CFRelease(source) };
        }
    }

    pub fn describe(saved: &Saved) -> String {
        match saved.0 {
            Some(source) => id_of(source),
            None => "nothing in the way".into(),
        }
    }
}

/// **Linux**: whichever of fcitx5, fcitx or ibus is on `PATH`, through the
/// commands their own documentation gives. `fcitx5-remote -c` also closes
/// 宇浩's own fcitx5 add-on, which has no signal of its own yet.
#[cfg(all(unix, not(target_os = "macos")))]
mod source {
    use std::process::{Command, Stdio};

    pub enum Saved {
        /// Nothing was in the way, or nothing here can be asked.
        Nothing,
        /// fcitx was composing and is closed.
        Fcitx(&'static str),
        /// ibus was on this engine.
        Ibus(String),
    }

    /// fcitx and ibus answer for the window that has the focus.
    pub const PER_WINDOW: bool = true;

    /// Whether anything was switched aside.
    pub fn holds(saved: &Saved) -> bool {
        !matches!(saved, Saved::Nothing)
    }

    /// ibus's settings schema.
    const IBUS_GENERAL: &str = "org.freedesktop.ibus.general";

    /// The writer's own keyboard layout in ibus: the first `xkb:` engine
    /// of the ones set up, so a German keyboard stays German in Normal.
    fn plain_layout() -> String {
        run("gsettings", &["get", IBUS_GENERAL, "preload-engines"])
            .and_then(|list| {
                list.split(['\'', '"'])
                    .find(|engine| engine.starts_with("xkb:"))
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "xkb:us::eng".to_string())
    }

    /// Run one of the input method's commands, **for half a second at
    /// most**: it runs on the editor's loop, and a D-Bus that does not
    /// answer would otherwise freeze the editor with it.
    fn run(program: &str, args: &[&str]) -> Option<String> {
        use std::io::Read;
        use std::time::{Duration, Instant};
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let deadline = BUDGET.with(|b| b.get()).unwrap_or_else(|| Instant::now() + Duration::from_millis(500));
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut out = String::new();
                    child.stdout.take()?.read_to_string(&mut out).ok()?;
                    return status.success().then(|| out.trim().to_string());
                }
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
    }

    thread_local! {
        /// One switch's whole budget: `take` may run five commands, and a
        /// D-Bus that does not answer costs the switch half a second, not
        /// five halves (review, 2026-11-04).
        static BUDGET: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    }

    pub fn take() -> Saved {
        let half = std::time::Duration::from_millis(500);
        BUDGET.with(|b| b.set(Some(std::time::Instant::now() + half)));
        let saved = take_within();
        BUDGET.with(|b| b.set(None));
        saved
    }

    fn take_within() -> Saved {
        // `fcitx5-remote` with no arguments prints the state: 2 is 「composing
        // is on」. Only then is there anything to close.
        for tool in ["fcitx5-remote", "fcitx-remote"] {
            if let Some(state) = run(tool, &[]) {
                if state != "2" || run(tool, &["-c"]).is_none() {
                    return Saved::Nothing;
                }
                return Saved::Fcitx(match tool {
                    "fcitx5-remote" => "fcitx5-remote",
                    _ => "fcitx-remote",
                });
            }
        }
        match run("ibus", &["engine"]) {
            // An `xkb:` engine is a plain layout: nothing holds the keys.
            Some(engine) if !engine.is_empty() && !engine.starts_with("xkb:") => {
                match run("ibus", &["engine", &plain_layout()]) {
                    Some(_) => Saved::Ibus(engine),
                    None => Saved::Nothing,
                }
            }
            _ => Saved::Nothing,
        }
    }

    pub fn restore(saved: Saved) {
        match saved {
            Saved::Nothing => {}
            Saved::Fcitx(tool) => {
                run(tool, &["-o"]);
            }
            Saved::Ibus(engine) => {
                run("ibus", &["engine", &engine]);
            }
        }
    }

    pub fn forget(_saved: Saved) {}

    pub fn describe(saved: &Saved) -> String {
        match saved {
            Saved::Nothing => "nothing in the way".into(),
            Saved::Fcitx(tool) => format!("{tool} closed"),
            Saved::Ibus(engine) => format!("ibus {engine}"),
        }
    }
}

/// **Windows and the rest**: nothing yet. Windows' terminal owns the input
/// context, in another process; reaching it is left to the Windows machine.
#[cfg(not(unix))]
mod source {
    pub struct Saved;
    pub const PER_WINDOW: bool = false;
    pub fn holds(_saved: &Saved) -> bool {
        false
    }
    pub fn take() -> Saved {
        Saved
    }
    pub fn restore(_saved: Saved) {}
    pub fn forget(_saved: Saved) {}
    pub fn describe(_saved: &Saved) -> String {
        "not on this platform".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `keep` reaches no signal at all — the one guarantee the escape hatch has
    /// to make, since the reason to write it is that the signalling misbehaves.
    #[test]
    fn keep_never_asks_for_anything() {
        let mut ime = SystemIme::new(SystemImePolicy::Keep, OtherImePolicy::Off);
        ime.want(true);
        assert!(!ime.suspended);
        ime.want(false);
        assert!(!ime.suspended);
    }

    /// **Other input methods are left alone by default** (2026-11-04): with
    /// `[ime] others` off, wanting the keys never touches the input source.
    /// The switch itself is not run here, since it would move this machine's.
    #[test]
    fn other_input_methods_are_left_alone_by_default() {
        let mut ime = SystemIme::new(SystemImePolicy::Keep, OtherImePolicy::Off);
        ime.want(true);
        assert!(ime.switched.is_none());
        assert_eq!(OtherImePolicy::from_config("aside"), OtherImePolicy::Aside);
        assert_eq!(OtherImePolicy::from_config("aside-and-back"), OtherImePolicy::AsideAndBack);
        assert_eq!(OtherImePolicy::from_config("asid"), OtherImePolicy::Off, "a typo switches nothing");
    }

    /// **Focus outranks the mode**, or the resume is undone on the next turn of
    /// the loop — the suspension is recorded against the application, and two
    /// windows of one terminal are one application.
    #[test]
    fn nothing_is_asked_for_without_focus() {
        let mut ime = SystemIme::new(SystemImePolicy::Auto, OtherImePolicy::Off);
        ime.set_focus(false);
        ime.want(true);
        assert!(!ime.suspended, "Normal mode, but the window is somebody else's");
        ime.set_focus(true);
        assert!(ime.focused);
    }

    /// Asking twice sends one signal, and so does giving it back twice: `want`
    /// runs on every turn of the loop, and a notification per frame would be a
    /// notification per frame.
    #[test]
    fn the_signal_is_sent_on_the_change_and_not_on_the_turn() {
        let mut ime = SystemIme::new(SystemImePolicy::Auto, OtherImePolicy::Off);
        ime.want(true);
        assert!(ime.suspended);
        ime.want(true);
        assert!(ime.suspended);
        ime.want(false);
        assert!(!ime.suspended);
        ime.want(false);
        assert!(!ime.suspended);
    }
}
