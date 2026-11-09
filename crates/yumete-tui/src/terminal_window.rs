//! **Which window on the screen is the terminal this editor runs in**, for
//! `:shot` and `:shot png`.
//!
//! Not by asking `System Events` for the frontmost window's position and size.
//! That is Accessibility, a permission a terminal is rarely given, and when it
//! is missing `osascript` fails into a pipe whose status is `tr`'s, so
//! `screencapture -R` still runs with an empty rectangle — and both of them
//! write their complaints straight onto the editor's screen.
//!
//! The window server will say which windows are on screen, whose they are and
//! where, to anybody and without a permission (`CGWindowListCopyWindowInfo`;
//! only a window's *title* is withheld without Screen Recording, and it is not
//! asked for here). The terminal is the window owned by one of this process's
//! ancestors: yumete → shell → `login` → Ghostty. `screencapture -l <id>` then
//! takes exactly that window. What is left is Screen Recording for
//! `screencapture` itself, which no way of taking a picture of another app's
//! window avoids; [`may_record_the_screen`] asks before trying, so a missing
//! permission is named rather than turned into a picture of the wallpaper.

/// One window on the screen, as the window server lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The `CGWindowID`, what `screencapture -l` takes.
    pub id: u32,
    /// The process that owns it.
    pub pid: u32,
    /// The owning application's name (`Ghostty`), for a message.
    pub owner: String,
    /// The window server's layer: 0 is an ordinary window, anything else a
    /// menu bar item, a panel, the Dock.
    pub layer: i32,
    /// Where it is, in points: `x`, `y`, `width`, `height`.
    pub bounds: (i64, i64, i64, i64),
}

impl Window {
    /// The rectangle as `screencapture -R` writes it.
    pub fn rect(&self) -> String {
        let (x, y, w, h) = self.bounds;
        format!("{x},{y},{w},{h}")
    }
}

/// The terminal's window among `windows` (front to back, as the window server
/// lists them): the frontmost ordinary window owned by one of `ancestors`,
/// or — when none of them owns one (a terminal multiplexer whose client could
/// not be asked) — the frontmost ordinary window at all, which is the
/// terminal the moment somebody has just typed `:shot` into it.
pub fn pick<'a>(windows: &'a [Window], ancestors: &[u32]) -> Option<&'a Window> {
    let ordinary = || windows.iter().filter(|w| w.layer == 0 && w.bounds.2 > 0 && w.bounds.3 > 0);
    ordinary()
        .find(|w| ancestors.contains(&w.pid))
        .or_else(|| ordinary().next())
}

/// The terminal's window, or `None` where there is no window server to ask
/// (every platform but macOS, or nothing on screen).
pub fn find() -> Option<Window> {
    let windows = backend::on_screen();
    let mut ancestors = backend::ancestors(std::process::id());
    // Inside tmux the editor descends from the tmux *server*, which was
    // started once and left running; the terminal is the client's parent.
    if std::env::var_os("TMUX").is_some() {
        if let Some(client) = tmux_client() {
            ancestors.extend(backend::ancestors(client));
        }
    }
    pick(&windows, &ancestors).cloned()
}

/// The pid of the tmux client this pane is shown in.
fn tmux_client() -> Option<u32> {
    let out = std::process::Command::new("tmux")
        .args(["display-message", "-p", "#{client_pid}"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Whether the terminal may take pictures of the screen (Screen Recording).
///
/// The answer is the terminal application's: a command-line program is
/// judged as the app it was started from, so this is Ghostty's permission
/// when the editor runs in Ghostty — the same one `screencapture`, started
/// from here, is judged by. `true` where there is no such permission.
pub fn may_record_the_screen() -> bool {
    backend::may_record_the_screen()
}

#[cfg(target_os = "macos")]
mod backend {
    use super::Window;
    use std::ffi::c_void;

    type CFTypeRef = *const c_void;

    #[repr(C)]
    #[derive(Default)]
    struct CGRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }

    const ON_SCREEN_ONLY: u32 = 1 << 0;
    const EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const SINT64: isize = 4;
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFTypeRef;
        fn CGRectMakeWithDictionaryRepresentation(dict: CFTypeRef, rect: *mut CGRect) -> bool;
        fn CGPreflightScreenCaptureAccess() -> bool;
        static kCGWindowNumber: CFTypeRef;
        static kCGWindowOwnerPID: CFTypeRef;
        static kCGWindowOwnerName: CFTypeRef;
        static kCGWindowLayer: CFTypeRef;
        static kCGWindowBounds: CFTypeRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFArrayGetCount(array: CFTypeRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
        fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn CFNumberGetValue(number: CFTypeRef, kind: isize, value: *mut c_void) -> bool;
        fn CFStringGetCString(s: CFTypeRef, buf: *mut u8, n: isize, enc: u32) -> u8;
        fn CFRelease(object: CFTypeRef);
    }

    /// A number out of a window's dictionary.
    ///
    /// # Safety
    /// `dict` is a live `CFDictionary`.
    unsafe fn number(dict: CFTypeRef, key: CFTypeRef) -> Option<i64> {
        let value = unsafe { CFDictionaryGetValue(dict, key) };
        if value.is_null() {
            return None;
        }
        let mut out: i64 = 0;
        unsafe { CFNumberGetValue(value, SINT64, (&mut out as *mut i64).cast()) }.then_some(out)
    }

    /// A string out of a window's dictionary, empty when there is none.
    ///
    /// # Safety
    /// `dict` is a live `CFDictionary`.
    unsafe fn string(dict: CFTypeRef, key: CFTypeRef) -> String {
        let value = unsafe { CFDictionaryGetValue(dict, key) };
        let mut buf = [0u8; 256];
        if value.is_null() || unsafe { CFStringGetCString(value, buf.as_mut_ptr(), buf.len() as isize, UTF8) } == 0 {
            return String::new();
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(0);
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }

    pub fn on_screen() -> Vec<Window> {
        // Safety: the array is ours (+1 from the `Copy` call) and released
        // below; the dictionaries in it are borrowed from it.
        unsafe {
            let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP_ELEMENTS, 0);
            if list.is_null() {
                return Vec::new();
            }
            let mut windows = Vec::new();
            for i in 0..CFArrayGetCount(list) {
                let dict = CFArrayGetValueAtIndex(list, i);
                if dict.is_null() {
                    continue;
                }
                let (Some(id), Some(pid)) = (number(dict, kCGWindowNumber), number(dict, kCGWindowOwnerPID))
                else {
                    continue;
                };
                let mut rect = CGRect::default();
                let bounds = CFDictionaryGetValue(dict, kCGWindowBounds);
                if bounds.is_null() || !CGRectMakeWithDictionaryRepresentation(bounds, &mut rect) {
                    continue;
                }
                windows.push(Window {
                    id: id as u32,
                    pid: pid as u32,
                    owner: string(dict, kCGWindowOwnerName),
                    layer: number(dict, kCGWindowLayer).unwrap_or(0) as i32,
                    bounds: (
                        rect.x.round() as i64,
                        rect.y.round() as i64,
                        rect.width.round() as i64,
                        rect.height.round() as i64,
                    ),
                });
            }
            CFRelease(list);
            windows
        }
    }

    /// `pid` and its parents, up to (not including) `launchd`.
    ///
    /// Warning: **the short record, not `PROC_PIDTBSDINFO`.** The terminal's
    /// child is `/usr/bin/login`, which runs as root, and the full record of a
    /// root process is refused to a user (`proc_pidinfo` answers 0) — the walk
    /// would stop one step short of Ghostty. The short one is answered for any
    /// process, as `ps` answers.
    pub fn ancestors(pid: u32) -> Vec<u32> {
        let mut chain = Vec::new();
        let mut at = pid;
        while at > 1 && chain.len() < 64 {
            chain.push(at);
            let mut info: libc::proc_bsdshortinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of::<libc::proc_bsdshortinfo>() as libc::c_int;
            // Safety: a buffer of exactly the size the flavour fills.
            let got = unsafe {
                libc::proc_pidinfo(
                    at as libc::c_int,
                    libc::PROC_PIDT_SHORTBSDINFO,
                    0,
                    (&mut info as *mut libc::proc_bsdshortinfo).cast(),
                    size,
                )
            };
            if got != size || info.pbsi_ppid == at {
                break;
            }
            at = info.pbsi_ppid;
        }
        chain
    }

    pub fn may_record_the_screen() -> bool {
        // Safety: no arguments, no side effects — `Preflight` never prompts.
        unsafe { CGPreflightScreenCaptureAccess() }
    }
}

#[cfg(not(target_os = "macos"))]
mod backend {
    use super::Window;

    pub fn on_screen() -> Vec<Window> {
        Vec::new()
    }

    pub fn ancestors(pid: u32) -> Vec<u32> {
        vec![pid]
    }

    pub fn may_record_the_screen() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u32, pid: u32, layer: i32) -> Window {
        Window { id, pid, owner: format!("app{pid}"), layer, bounds: (0, 33, 800, 600) }
    }

    #[test]
    fn the_terminal_is_the_window_an_ancestor_owns_even_behind_another() {
        let windows = [window(1, 50, 25), window(2, 60, 0), window(3, 70, 0), window(4, 70, 0)];
        // yumete 900 → zsh 800 → login 700 → terminal 70
        assert_eq!(pick(&windows, &[900, 800, 700, 70]).map(|w| w.id), Some(3), "its frontmost window");
    }

    #[test]
    fn with_no_ancestor_on_screen_the_frontmost_ordinary_window_is_taken() {
        let windows = [window(1, 50, 25), window(2, 60, 0)];
        assert_eq!(pick(&windows, &[900]).map(|w| w.id), Some(2), "not the menu bar item");
        assert_eq!(pick(&[window(1, 50, 25)], &[900]), None, "nothing ordinary, nothing picked");
    }

    #[test]
    fn a_window_with_no_area_is_never_the_terminal() {
        let mut empty = window(2, 70, 0);
        empty.bounds = (0, 0, 0, 0);
        assert_eq!(pick(&[empty, window(3, 70, 0)], &[70]).map(|w| w.id), Some(3));
    }

    #[test]
    fn the_rectangle_is_written_as_screencapture_reads_it() {
        assert_eq!(window(1, 2, 0).rect(), "0,33,800,600");
    }

    /// On this machine the walk must reach past `login` (root) to whatever
    /// started the test runner; on any machine it starts with us.
    #[test]
    fn the_ancestry_starts_here_and_climbs() {
        let chain = backend::ancestors(std::process::id());
        assert_eq!(chain.first(), Some(&std::process::id()));
        #[cfg(target_os = "macos")]
        assert!(chain.len() >= 2, "a test runner has a parent: {chain:?}");
    }
}
