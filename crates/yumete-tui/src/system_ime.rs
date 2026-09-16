//! 讓開 — telling the **system's** input method to keep its hands off the keys
//! yumete is about to read itself.
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
//! And the writer can already see the answer — it is the tag the status line
//! stands (`standing_language_tag`), which is exactly `available && engaged`:
//!
//! | 狀態行 | Normal | Insert／提示行 |
//! | --- | --- | --- |
//! | `[中 靈明]` | 讓開 | 讓開 |
//! | `[ABC]` | 讓開 | 讓開 |
//! | 什麽都不寫 | 讓開 | **放行** |
//!
//! In Normal every printable key is a command, so the answer is always 讓開.
//! Where text is typed, the keyboard is taken **only if yumete can do
//! something with it**: with a 碼表 loaded and yume engaged the keys are
//! codes and belong here, and the system's input method would be a second
//! one composing the same keys.
//!
//! ⚠️ **`engaged` on its own is the wrong test, and it is the mistake that
//! made the editor unusable the first time this shipped.** `[ime] start` is
//! `false` out of the box, so a fresh session is engaged with **no 碼表** —
//! and the rule took the system's input method away from a writer whose
//! yumete could not type 漢字 either: 「你把输入法切走了，我 i 进入 insert
//! 模式用什么？」 An editor that can neither type 漢字 nor let anything else
//! do it is worse than the bug this exists to fix. The marker is the test
//! precisely because it is what the writer is already reading.
//!
//! So the bottom row — no marker, whether from `:yume off` or from a 碼表
//! that was never loaded — is where the system's input method is exactly
//! what is wanted, and somebody writing Japanese in yumete lives there.
//!
//! ⚠️ **The top two rows never switch while you work.** With yume typing for
//! you the whole row reads 讓開, so `i` and `Esc` — pressed dozens of times a
//! minute — change nothing at all. It is the bottom row that follows the
//! mode, and there the menu-bar icon moving with it is feedback rather than
//! flicker: it says which input method has the keyboard, which in that state
//! is the thing you need to know.
//!
//! ## What is actually done
//!
//! The system's input source is switched, and switched back. This is what
//! `im-select` does for vim, and the reason to do it rather than to signal
//! yume directly is that it is not about yume: it keeps 搜狗, the system
//! Japanese input method and everything else off the keys too, and it needs no
//! cooperation from anything.
//!
//! **Switching back is the hard half.** What was there is remembered at the
//! moment it is first taken away, and put back by [`Drop`] — so every way out
//! of the editor's loop restores it, a panic included. The editor also gives
//! it back whenever the terminal loses focus, so that Cmd-Tab into another
//! window does not arrive in a keyboard layout yumete chose, and before `:!`
//! hands the screen to somebody else's program.
//!
//! ⚠️ **A signal is the gap**: SIGHUP from a closed terminal window, or a
//! SIGTERM, runs no destructor, and the input source is left where yumete put
//! it. That is one keystroke to undo and it is the same gap the terminal's own
//! three colours have had since #493 (OSC 110/111/112 are written on the way
//! out too) — so it is a thing the editor lacks, not a thing this lacks.
//!
//! Nothing here reports an error. A machine with no input method to switch, a
//! terminal that cannot be reached, a Linux box with neither fcitx nor ibus:
//! all of them mean 「this does not apply here」, and a writing tool that says
//! so on the status line once a mode is a writing tool nobody keeps.
//!
//! ## ⚠️ Off out of the box, and not because of anything here
//!
//! `[ime] system` ships as `keep`. 宇浩's own macOS input method does not take
//! the keyboard back after this has deselected and reselected it: the **second**
//! time it is handed back, keys arrive as plain letters and Yume's menu-bar menu
//! reads 「……」 until something else re-activates it — Cmd-Tab out and in, or the
//! globe key, both of which the system drives.
//!
//! Measured 2026-09-16, with System Events typing into a real terminal:
//!
//! | system input method | typed | landed in the buffer |
//! | --- | --- | --- |
//! | 蘋果全拼 | `ni` ␣ | 你 ✅ |
//! | 宇浩 | `j` ␣ | `j` ❌ |
//! | 蘋果全拼 | `ni` ␣ | 你 ✅ |
//!
//! Same editor, same terminal, same keystrokes; only the input method differs,
//! 2/2 against 2/2. So what this module does is right and what it meets is not.
//! Four things were measured away on the road there, listed so nobody walks it
//! twice: `TISSelectInputSource` returns in **1 ms** and the system reports the
//! new source immediately; YumeIME neither restarts nor reloads its tables (CPU
//! and RSS flat across the switch); the gap between the switch away and the
//! switch back makes no difference (0.4 s through 3.0 s all fail); and **one**
//! hand-back always works — it is the second that does not.
//!
//! The default goes back to `auto` when yume is fixed. It is one line.

use yumete_config::SystemImePolicy;

/// **A stopwatch on every switch, off unless asked for** — `YUMETE_IME_LOG=1`
/// appends to `/tmp/yumete-ime.log`.
///
/// Here because the one fault this feature has had is a *timing* fault, and a
/// timing fault cannot be reasoned about from a screenshot: 「切换太快就会这样」.
/// The line carries the wall clock, what was asked for, how long the system
/// call took, and what the system says the input source is afterwards — which
/// together say whether a switch was slow, or too close to the one before it.
fn log(line: impl FnOnce() -> String) {
    use std::io::Write as _;
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("YUMETE_IME_LOG").is_some()) {
        return;
    }
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/yumete-ime.log")
    {
        let _ = writeln!(f, "{epoch:.2} {}", line());
    }
}

/// Holds the system input source aside while yumete wants the keys.
pub struct SystemIme {
    policy: SystemImePolicy,
    /// Whether the terminal has the keyboard at all.
    ///
    /// ⚠️ **A release is not enough on its own.** [`Self::want`] runs on every
    /// turn of the main loop, so giving the input source back when focus left
    /// and nothing more would have it taken again a millisecond later — which
    /// is what happened the first time this was measured: focus out, and the
    /// layout was US again before the next window could be typed in. Losing
    /// focus has to *stop the asking*, not answer it once.
    ///
    /// Starts true, which is also the answer for a terminal that never reports
    /// focus at all.
    focused: bool,
    /// What was current when it was first taken away. `None` means 「we have
    /// not taken it away」, which is also the state after every restore — so
    /// the field is both the saved value and the flag, and they cannot drift.
    held: Option<backend::Saved>,
}

impl SystemIme {
    pub fn new(policy: SystemImePolicy) -> Self {
        SystemIme { policy, focused: true, held: None }
    }

    /// The terminal gained or lost the keyboard.
    ///
    /// Losing it gives the input source back at once: the window in front is
    /// somebody else's, and arriving there in a layout yumete chose is worse
    /// than anything this prevents. Gaining it says nothing by itself — the
    /// next [`Self::want`] is a turn of the loop away and knows the mode.
    pub fn set_focus(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.release();
        }
    }

    /// Ask for the keys, or give them back. Idempotent, and cheap when the
    /// answer has not changed — it is called on every turn of the main loop.
    pub fn want(&mut self, block: bool) {
        if self.policy == SystemImePolicy::Keep {
            return;
        }
        log(|| format!("want(block={block}) held={}", self.held.is_some()));
        match (block && self.focused, self.held.is_some()) {
            // ⚠️ **`take` cannot fail**, or rather its failures are answers:
            // 「nothing was in the way」 and 「this machine cannot be asked」
            // both come back as a `Saved` that puts nothing back. Were they a
            // `None` here, `held` would stay empty and the system would be
            // asked again on the *next turn of the loop* — a syscall a frame,
            // for as long as the mode holds.
            (true, false) => {
                let began = std::time::Instant::now();
                let saved = backend::take();
                log(|| format!("take took {:?}, now {}", began.elapsed(), backend::current_id()));
                self.held = Some(saved);
            }
            (false, true) => self.release(),
            _ => {}
        }
    }

    /// Put back what was there. Safe to call when nothing was taken.
    pub fn release(&mut self) {
        if let Some(saved) = self.held.take() {
            let began = std::time::Instant::now();
            backend::restore(saved);
            log(|| format!("restore took {:?}, now {}", began.elapsed(), backend::current_id()));
        }
    }
}

impl Drop for SystemIme {
    fn drop(&mut self) {
        self.release();
    }
}

/// **`macOS`** — Text Input Sources, the API `im-select` is built on.
///
/// Declared here rather than pulled in as a crate: it is three functions and
/// an opaque pointer, and `core-foundation` would be a dependency for that.
///
/// Measured on 2026-09-16 from a plain binary run inside a terminal — the
/// doubt worth settling, since `macism` exists because people have seen
/// `TISSelectInputSource` not take effect. It took effect:
/// `app.shurufa.inputmethod.Yume.Hans` → `com.apple.keylayout.US` → back,
/// both calls returning `noErr`.
#[cfg(target_os = "macos")]
mod backend {
    use std::ffi::c_void;

    type InputSource = *mut c_void;

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> InputSource;
        fn TISCopyCurrentASCIICapableKeyboardInputSource() -> InputSource;
        fn TISSelectInputSource(source: InputSource) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(object: *const c_void);
        fn CFEqual(a: *const c_void, b: *const c_void) -> u8;
        fn CFStringGetCString(s: *const c_void, buf: *mut u8, n: isize, enc: u32) -> u8;
    }

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn TISGetInputSourceProperty(source: InputSource, key: *const c_void) -> *const c_void;
        static kTISPropertyInputSourceID: *const c_void;
    }

    /// What the system says is selected right now, for the stopwatch log.
    pub fn current_id() -> String {
        unsafe {
            let now = TISCopyCurrentKeyboardInputSource();
            if now.is_null() {
                return "(none)".into();
            }
            let id = TISGetInputSourceProperty(now, kTISPropertyInputSourceID);
            let mut buf = [0u8; 256];
            let ok = !id.is_null()
                && CFStringGetCString(id, buf.as_mut_ptr(), buf.len() as isize, 0x0800_0100) != 0;
            CFRelease(now);
            match ok {
                true => String::from_utf8_lossy(&buf[..buf.iter().position(|b| *b == 0).unwrap_or(0)])
                    .into_owned(),
                false => "(unreadable)".into(),
            }
        }
    }

    /// `CFEqual` returns a `Boolean`, which is a `u8`.
    unsafe fn cf_equal(a: InputSource, b: InputSource) -> bool {
        CFEqual(a, b) != 0
    }

    /// The source that was current, owned (+1 from the `Copy` call) until it
    /// is put back — or nothing, when there was nothing in the way.
    pub struct Saved(Option<InputSource>);

    // The pointer never leaves the main loop's thread; the wrapper is here so
    // that a `SystemIme` can live in a struct without infecting it.
    unsafe impl Send for Saved {}

    pub fn take() -> Saved {
        unsafe {
            let ascii = TISCopyCurrentASCIICapableKeyboardInputSource();
            let was = TISCopyCurrentKeyboardInputSource();
            if ascii.is_null() || was.is_null() {
                if !ascii.is_null() {
                    CFRelease(ascii);
                }
                if !was.is_null() {
                    CFRelease(was);
                }
                return Saved(None);
            }
            // **Nothing in the way, so nothing is touched** — somebody who
            // writes only English, or who has switched to ABC by hand, never
            // has their input source selected at all, and no menu bar moves.
            //
            // The test is exact rather than a guess: `TISCopyCurrent
            // ASCIICapable…` returns *the current source itself* when the
            // current source is ASCII-capable, and the most recent ASCII one
            // otherwise. So the two being the same object **is** the question
            // 「is an input method holding the keyboard」.
            if cf_equal(ascii, was) {
                CFRelease(ascii);
                CFRelease(was);
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
}

/// **Linux** — whichever of fcitx5, fcitx or ibus is on `PATH`.
///
/// Shelled out to rather than spoken to over D-Bus, which is the same shape
/// `:shot` uses to find a screenshot program: the three tools are the
/// interface their own documentation tells people to use, and a D-Bus client
/// would be a great deal of code to arrive at the same three commands.
///
/// Untested here — this machine is macOS. The failure mode is doing nothing.
#[cfg(all(unix, not(target_os = "macos")))]
mod backend {
    use std::process::{Command, Stdio};

    pub enum Saved {
        /// Nothing was in the way, or nothing here can be asked.
        Nothing,
        /// fcitx was active and should be switched on again.
        Fcitx(&'static str),
        /// ibus was on this engine.
        Ibus(String),
    }

    fn run(program: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// The stopwatch log's counterpart to the macOS one; here the answer is
    /// whichever tool is on `PATH` and what it says its state is.
    pub fn current_id() -> String {
        for tool in ["fcitx5-remote", "fcitx-remote"] {
            if let Some(state) = run(tool, &[]) {
                return format!("{tool}={state}");
            }
        }
        run("ibus", &["engine"]).unwrap_or_else(|| "(none)".into())
    }

    pub fn take() -> Saved {
        // `fcitx5-remote` with no arguments prints the state: 2 is 「composing
        // is on」. Only then is there anything to turn off, and only then
        // should it be turned back on. Anything else — the tool is not there,
        // fcitx is not running, composing was already off — is 「nothing in
        // the way」, which is a `Nothing` and not a retry.
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
            // An `xkb:` engine is a plain keyboard layout: no input method is
            // holding the keys, so none is moved.
            Some(engine) if !engine.is_empty() && !engine.starts_with("xkb:") => {
                match run("ibus", &["engine", "xkb:us::eng"]) {
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
}

/// **Windows** — detach the console window from its input context.
///
/// ⚠️ **Whether this reaches anything depends on the terminal.** A console
/// program does not own the window its characters appear in: under Windows
/// Terminal the window belongs to another process entirely, and that process
/// runs the IME. `GetConsoleWindow` then hands back conhost's hidden window,
/// and detaching *that* from its input context changes nothing a person can
/// see. In a real conhost window it works.
///
/// Left in rather than left out because the failure is silent and symmetrical
/// — the same contract as a Linux box with neither fcitx nor ibus — and
/// because it costs nothing to be right on the terminals where it applies.
/// **Never run here**; the Windows session should check it against both
/// Windows Terminal and conhost before it is believed.
#[cfg(windows)]
mod backend {
    use std::ffi::c_void;

    type Hwnd = *mut c_void;
    type Himc = *mut c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleWindow() -> Hwnd;
    }

    #[link(name = "imm32")]
    extern "system" {
        fn ImmAssociateContext(window: Hwnd, context: Himc) -> Himc;
    }

    /// The context the window had, to be put back — or nothing, when there
    /// was no window to ask or no context on it.
    pub struct Saved(Option<(Hwnd, Himc)>);

    unsafe impl Send for Saved {}

    /// The console window's handle is all there is to say here.
    pub fn current_id() -> String {
        unsafe { format!("hwnd={:?}", GetConsoleWindow()) }
    }

    pub fn take() -> Saved {
        unsafe {
            let window = GetConsoleWindow();
            if window.is_null() {
                return Saved(None);
            }
            let had = ImmAssociateContext(window, std::ptr::null_mut());
            match had.is_null() {
                // No context was associated: nothing was in the way, and
                // re-associating a null later would say nothing either.
                true => Saved(None),
                false => Saved(Some((window, had))),
            }
        }
    }

    pub fn restore(saved: Saved) {
        if let Some((window, context)) = saved.0 {
            unsafe {
                ImmAssociateContext(window, context);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `keep` reaches no system call at all — the one guarantee the escape
    /// hatch has to make, since the reason to write it is that the switching
    /// is misbehaving.
    #[test]
    fn keep_never_takes_anything() {
        let mut ime = SystemIme::new(SystemImePolicy::Keep);
        ime.want(true);
        assert!(ime.held.is_none());
        ime.want(false);
        assert!(ime.held.is_none());
    }

    /// **Focus outranks the mode**, or the release is undone on the next turn
    /// of the loop — the way it was when this was first measured (focus out,
    /// and the layout was back a millisecond later).
    ///
    /// The state is read rather than the system: what a `take` returns depends
    /// on the machine, and the thing being asserted is that nothing is asked
    /// for at all.
    #[test]
    fn nothing_is_asked_for_without_focus() {
        let mut ime = SystemIme::new(SystemImePolicy::Auto);
        ime.set_focus(false);
        ime.want(true);
        assert!(ime.held.is_none(), "Normal mode, but the window is somebody else's");
        ime.set_focus(true);
        // Back in front, and the next turn of the loop asks again with the
        // same mode — nothing had to be remembered across the gap.
        assert!(ime.focused);
    }
}
