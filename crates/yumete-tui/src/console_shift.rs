//! A lone Shift tap on the Windows console.
//!
//! Everywhere else the tap arrives through the Kitty keyboard protocol as a
//! bare `LeftShift`/`RightShift` press and release, and [`crate::ShiftTap`]
//! answers it. The Windows console has no such protocol, and crossterm says so
//! (`supports_keyboard_enhancement` is `Ok(false)` there) — yet the console's
//! own input queue does hold a `KEY_EVENT` record for Shift going down and
//! for it coming up. crossterm reads those records and throws them away
//! (`VK_SHIFT | VK_CONTROL | VK_MENU => None` in its Windows parser).
//!
//! So on Windows the key reader looks at the **head** of the queue before
//! crossterm does:
//!
//! - a record crossterm would throw away (Shift, Ctrl or Alt by itself, a
//!   menu record) is taken off the queue here and fed to [`LoneShift`];
//! - any other record is fed to [`LoneShift`] too — a key going down taints
//!   the tap, focus going away drops it — and left for crossterm, which is
//!   then asked with a zero timeout. A zero timeout makes crossterm read
//!   **exactly one** record (its loop stops as soon as the time is up), so the
//!   record crossterm takes is the one just looked at, and every record is
//!   seen here once: none skipped, none counted twice.
//!
//! A completed tap goes into the event channel as a Shift press and release
//! ([`as_events`]), and from there takes the same road the Kitty tap takes:
//! [`crate::ShiftTap`], then `route_a_shift_tap`, so which modes answer, the
//! prompts, the settings rows and the borrowed system input method all follow
//! the one set of rules.
//!
//! The classification ([`key_record`]) and the detector are plain functions
//! over numbers, so they are tested on every platform; only the three console
//! calls are Windows code.

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode};
use yumete_ime::{FuncKey, ModifierTap};

/// Virtual-key codes, as `WinUser.h` gives them.
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_LSHIFT: u16 = 0xA0;
const VK_RSHIFT: u16 = 0xA1;
const VK_LCONTROL: u16 = 0xA2;
const VK_RCONTROL: u16 = 0xA3;
const VK_LMENU: u16 = 0xA4;
const VK_RMENU: u16 = 0xA5;

/// The right Shift's scan code. The console reports both Shifts as
/// `VK_SHIFT`; only the scan code tells them apart (the left one is `0x2A`).
const SCAN_RIGHT_SHIFT: u16 = 0x36;

/// `dwControlKeyState` bits for the modifiers that make a Shift a chord.
const RIGHT_ALT_PRESSED: u32 = 0x0001;
const LEFT_ALT_PRESSED: u32 = 0x0002;
const RIGHT_CTRL_PRESSED: u32 = 0x0004;
const LEFT_CTRL_PRESSED: u32 = 0x0008;
const CHORD: u32 = RIGHT_ALT_PRESSED | LEFT_ALT_PRESSED | RIGHT_CTRL_PRESSED | LEFT_CTRL_PRESSED;

/// One console input record, as far as the lone-Shift tap cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Record {
    /// Left or right Shift, down or up; `others` is Ctrl or Alt held with it.
    Shift { key: FuncKey, down: bool, others: bool },
    /// Ctrl or Alt by itself.
    Modifier { down: bool },
    /// Any other key, crossterm's to turn into an event.
    Key { down: bool },
    /// Focus came or went: a release may have been lost on the way.
    Focus,
    /// A menu record: the console's own, which crossterm throws away.
    Menu,
    /// The rest (mouse, resize), crossterm's.
    Other,
}

impl Record {
    /// Whether this record is taken off the queue here rather than left for
    /// crossterm — exactly the records crossterm reads and discards, so
    /// crossterm loses nothing it would have used.
    pub(crate) fn taken_here(self) -> bool {
        matches!(self, Record::Shift { .. } | Record::Modifier { .. } | Record::Menu)
    }
}

/// A `KEY_EVENT` record's fields, classified.
///
/// `ch` is the record's UTF-16 character. Alt coming **up with a character**
/// is an Alt code (Alt held while digits are typed on the keypad), which
/// crossterm turns into that character — so it stays crossterm's.
pub(crate) fn key_record(vk: u16, scan: u16, down: bool, state: u32, ch: u16) -> Record {
    match vk {
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => {
            let right = vk == VK_RSHIFT || (vk == VK_SHIFT && scan == SCAN_RIGHT_SHIFT);
            let key = if right { FuncKey::ShiftR } else { FuncKey::ShiftL };
            Record::Shift { key, down, others: state & CHORD != 0 }
        }
        VK_MENU | VK_LMENU | VK_RMENU if !down && ch != 0 => Record::Key { down },
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL | VK_MENU | VK_LMENU | VK_RMENU => Record::Modifier { down },
        _ => Record::Key { down },
    }
}

/// The lone-Shift detector over console records. The state machine is
/// yume's [`ModifierTap`], as for [`crate::ShiftTap`]: down then up of one
/// Shift, nothing else pressed in between, no Ctrl or Alt held. There is no
/// time limit, on either road: a tap is a matter of what came in between, not
/// of how long it took.
#[derive(Default)]
pub(crate) struct LoneShift(ModifierTap);

impl LoneShift {
    /// Feed the next record; `Some` is a completed tap of that Shift.
    pub(crate) fn feed(&mut self, record: Record) -> Option<FuncKey> {
        match record {
            Record::Shift { key, down, others } => self.0.modifier(key, down, others),
            // Ctrl or Alt going down taints as any key does, as on the Kitty
            // road, where `ShiftTap` sees them as other keys.
            Record::Modifier { down: true } | Record::Key { down: true } => {
                self.0.other_key();
                None
            }
            Record::Focus => {
                self.0.reset();
                None
            }
            Record::Modifier { down: false } | Record::Key { down: false } | Record::Menu | Record::Other => None,
        }
    }

    /// The release is not coming (the reader stepped off the queue for a
    /// program that had the terminal).
    pub(crate) fn reset(&mut self) {
        self.0.reset();
    }
}

/// A completed tap as the two events the Kitty protocol would have sent, so
/// it reaches [`crate::ShiftTap`] by the same door.
pub(crate) fn as_events(key: FuncKey) -> [Event; 2] {
    let code = KeyCode::Modifier(match key {
        FuncKey::ShiftR => ModifierKeyCode::RightShift,
        _ => ModifierKeyCode::LeftShift,
    });
    [KeyEventKind::Press, KeyEventKind::Release]
        .map(|kind| Event::Key(KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)))
}

/// Whether a lone Shift can be heard without the Kitty protocol: on Windows,
/// when standard input is a console. The terminal in front of the console
/// decides whether the Shift records are really there — Windows Terminal and
/// the classic console send them — and that cannot be asked.
pub(crate) fn available() -> bool {
    #[cfg(windows)]
    {
        console::Console::open().is_some()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
pub(crate) mod console {
    use super::{key_record, Record};
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, PeekConsoleInputW, ReadConsoleInputW, FOCUS_EVENT, INPUT_RECORD, KEY_EVENT,
        MENU_EVENT, STD_INPUT_HANDLE,
    };
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    /// The console's input queue, the same one crossterm reads through its
    /// own `CONIN$` handle.
    pub(crate) struct Console(HANDLE);

    // Safety: a console handle is a process-wide kernel handle, usable from
    // any thread; only the key reader's thread uses this one.
    unsafe impl Send for Console {}

    /// The head of the queue after a wait.
    pub(crate) enum Head {
        /// Nothing came within the wait.
        Empty,
        /// A record crossterm would have discarded, taken off the queue.
        Taken(Record),
        /// A record left at the head for crossterm to read next.
        Crossterm(Record),
    }

    impl Console {
        /// Standard input, if it is a console.
        pub(crate) fn open() -> Option<Console> {
            // Safety: plain queries on the process's own standard input handle.
            unsafe {
                let handle = GetStdHandle(STD_INPUT_HANDLE);
                if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                    return None;
                }
                let mut mode = 0;
                (GetConsoleMode(handle, &mut mode) != 0).then_some(Console(handle))
            }
        }

        /// Wait up to `wait` for input, then look at the head record and take
        /// it off the queue if crossterm would only discard it.
        pub(crate) fn next(&self, wait: Duration) -> Head {
            let millis = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
            // Safety: the buffers are one zeroed `INPUT_RECORD` each, and the
            // length passed is 1. Only this thread reads the queue, and
            // crossterm only when this thread asks it to, so the record read
            // is the record peeked.
            unsafe {
                if WaitForSingleObject(self.0, millis) != WAIT_OBJECT_0 {
                    return Head::Empty;
                }
                let mut buf: INPUT_RECORD = std::mem::zeroed();
                let mut count = 0u32;
                if PeekConsoleInputW(self.0, &mut buf, 1, &mut count) == 0 || count == 0 {
                    return Head::Empty;
                }
                let record = classify(&buf);
                if !record.taken_here() {
                    return Head::Crossterm(record);
                }
                let mut took = 0u32;
                ReadConsoleInputW(self.0, &mut buf, 1, &mut took);
                Head::Taken(record)
            }
        }
    }

    fn classify(buf: &INPUT_RECORD) -> Record {
        match u32::from(buf.EventType) {
            KEY_EVENT => {
                // Safety: `EventType` says which member of the union is set.
                let key = unsafe { buf.Event.KeyEvent };
                let ch = unsafe { key.uChar.UnicodeChar };
                key_record(key.wVirtualKeyCode, key.wVirtualScanCode, key.bKeyDown != 0, key.dwControlKeyState, ch)
            }
            FOCUS_EVENT => Record::Focus,
            MENU_EVENT => Record::Menu,
            _ => Record::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The records a console delivers for a key: down, then up.
    fn shift_l(down: bool) -> Record {
        key_record(VK_SHIFT, 0x2A, down, 0, 0)
    }
    fn shift_r(down: bool) -> Record {
        key_record(VK_SHIFT, SCAN_RIGHT_SHIFT, down, 0, 0)
    }
    fn letter(down: bool) -> Record {
        key_record(0x41, 0x1E, down, 0, u16::from(b'a'))
    }

    fn taps(records: &[Record]) -> Vec<FuncKey> {
        let mut tap = LoneShift::default();
        records.iter().filter_map(|r| tap.feed(*r)).collect()
    }

    #[test]
    fn a_lone_shift_is_a_tap_and_the_two_shifts_are_told_apart_by_scan_code() {
        assert_eq!(taps(&[shift_l(true), shift_l(false)]), [FuncKey::ShiftL]);
        assert_eq!(taps(&[shift_r(true), shift_r(false)]), [FuncKey::ShiftR]);
        assert_eq!(taps(&[key_record(VK_RSHIFT, 0, true, 0, 0), key_record(VK_RSHIFT, 0, false, 0, 0)]), [FuncKey::ShiftR]);
        // Held long enough to repeat, and still alone: still a tap.
        assert_eq!(taps(&[shift_l(true), shift_l(true), shift_l(true), shift_l(false)]), [FuncKey::ShiftL]);
    }

    #[test]
    fn shift_with_anything_else_is_not_a_tap() {
        // Shift + a letter: a capital, not a switch.
        assert!(taps(&[shift_l(true), letter(true), letter(false), shift_l(false)]).is_empty());
        // Ctrl+Shift, whichever goes down first.
        let ctrl = |down| key_record(VK_CONTROL, 0x1D, down, if down { LEFT_CTRL_PRESSED } else { 0 }, 0);
        assert!(taps(&[shift_l(true), ctrl(true), ctrl(false), shift_l(false)]).is_empty());
        let held = key_record(VK_SHIFT, 0x2A, true, LEFT_CTRL_PRESSED, 0);
        assert!(taps(&[ctrl(true), held, shift_l(false)]).is_empty());
        // Released while Alt is still down.
        let up_with_alt = key_record(VK_SHIFT, 0x2A, false, LEFT_ALT_PRESSED, 0);
        assert!(taps(&[shift_l(true), up_with_alt]).is_empty());
        // Left down, right down and up: the right one is the tap, as on the
        // Kitty road; the left one's release ends nothing.
        assert_eq!(taps(&[shift_l(true), shift_r(true), shift_r(false), shift_l(false)]), [FuncKey::ShiftR]);
    }

    #[test]
    fn a_key_released_while_shift_goes_down_does_not_spoil_the_tap() {
        // Fast typing: the letter's release lands after Shift is already down.
        assert_eq!(taps(&[letter(true), shift_l(true), letter(false), shift_l(false)]), [FuncKey::ShiftL]);
    }

    #[test]
    fn focus_going_away_drops_a_half_made_tap() {
        let focus = Record::Focus;
        assert!(taps(&[shift_l(true), focus, shift_l(false)]).is_empty());
        // …and does not eat the next one.
        assert_eq!(taps(&[shift_l(true), focus, letter(true), shift_l(true), shift_l(false)]), [FuncKey::ShiftL]);
    }

    #[test]
    fn only_records_crossterm_discards_are_taken_off_the_queue() {
        assert!(shift_l(true).taken_here());
        assert!(key_record(VK_CONTROL, 0x1D, true, LEFT_CTRL_PRESSED, 0).taken_here());
        assert!(key_record(VK_MENU, 0x38, true, LEFT_ALT_PRESSED, 0).taken_here());
        assert!(Record::Menu.taken_here());
        // An Alt code's release carries the character: crossterm's.
        assert!(!key_record(VK_MENU, 0x38, false, 0, 0x4E2D).taken_here());
        // `C-\` (the `language_key` default) is an ordinary key with Ctrl held.
        assert!(!key_record(0xDC, 0x2B, true, LEFT_CTRL_PRESSED, 0x1C).taken_here());
        assert!(!letter(true).taken_here());
        assert!(!Record::Focus.taken_here());
        assert!(!Record::Other.taken_here());
    }

    #[test]
    fn a_tap_reaches_the_kitty_road_as_one_tap() {
        for key in [FuncKey::ShiftL, FuncKey::ShiftR] {
            let mut shift = crate::ShiftTap::default();
            let answers: Vec<_> = as_events(key)
                .iter()
                .map(|event| match event {
                    Event::Key(k) => shift.update(k),
                    _ => unreachable!(),
                })
                .collect();
            assert!(matches!(answers[0], crate::ShiftResult::Consumed));
            assert!(matches!(answers[1], crate::ShiftResult::Tap(k) if k == key));
        }
    }
}
