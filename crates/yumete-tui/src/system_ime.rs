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
//! ⚠️ **Both halves were got wrong once, and each cost an evening.** The first
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
//! for exactly this and for exactly these editors: 「讓 helix／vim 這類模態編輯
//! 器在 normal mode 把輸入法整個支開」. Suspended, it hands every key straight
//! back to the terminal, untouched. It does **not** switch the input source, it
//! does **not** touch 中/英 (so a writer who was in ABC comes back to ABC), and
//! it records **which application asked**, so a terminal going quiet does not
//! mute the browser. All three platforms have it: a distributed notification on
//! macOS, a registered window message on Windows.
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
//! with no editor and no terminal in the picture at all**. ⚠️ 蘋果全拼 passing
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
//! ⚠️ **yumete never needs `once`.** That third signal exists because helix has
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

use yumete_config::SystemImePolicy;

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
}

impl SystemIme {
    pub fn new(policy: SystemImePolicy) -> Self {
SystemIme { policy, focused: true, suspended: false }
    }

    /// The terminal gained or lost the keyboard.
    ///
    /// ⚠️ **A resume is not enough on its own.** [`Self::want`] runs on every
    /// turn of the main loop, so lifting the suspension when focus left and
    /// nothing more would have it re-asserted a millisecond later. Losing focus
    /// has to *stop the asking*, not answer it once.
    pub fn set_focus(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.release();
        }
    }

    /// Ask for the keys, or give them back. Idempotent, and free when the
    /// answer has not changed — it is called on every turn of the main loop.
    pub fn want(&mut self, block: bool) {
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

    /// Give the keys back. Safe to call when nothing was asked for.
    pub fn release(&mut self) {
        if self.suspended {
            log(|| "-> resume".to_string());
            self.suspended = false;
            backend::resume();
        }
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
/// ⚠️ **The two names read backwards from the two commands**, and that is
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `keep` reaches no signal at all — the one guarantee the escape hatch has
    /// to make, since the reason to write it is that the signalling misbehaves.
    #[test]
    fn keep_never_asks_for_anything() {
        let mut ime = SystemIme::new(SystemImePolicy::Keep);
        ime.want(true);
        assert!(!ime.suspended);
        ime.want(false);
        assert!(!ime.suspended);
    }

    /// **Focus outranks the mode**, or the resume is undone on the next turn of
    /// the loop — the suspension is recorded against the application, and two
    /// windows of one terminal are one application.
    #[test]
    fn nothing_is_asked_for_without_focus() {
        let mut ime = SystemIme::new(SystemImePolicy::Auto);
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
        let mut ime = SystemIme::new(SystemImePolicy::Auto);
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
