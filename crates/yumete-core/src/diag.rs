//! **When it goes wrong, leave something behind** (#300, first slice).
//!
//! A terminal editor that panics takes the screen with it: the alternate
//! screen is torn down, whatever the default hook printed scrolls past inside
//! a raw-mode terminal, and the reader is left with a shell prompt and no idea
//! what they pressed. Until now yumete had **no panic hook at all** — the one
//! upstream crash this project has reproduced (#344, `:yume-table` on a file
//! that is not a code table) would have ended exactly that way, taking every
//! unsaved buffer with it and saying nothing.
//!
//! So: one file, appended to, that says what happened and what was pressed
//! just before. Nothing here is on a hot path — the only per-keystroke cost is
//! [`note_key`], which pushes a `Copy` enum into a fixed ring.
//!
//! **The path comes from outside.** This crate does not know where a data
//! directory is (that is `yumete-config`'s job, and the dependency runs the
//! other way), so the front end calls [`log_to`] at startup. Nothing is
//! written before it does.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::input::Key;

/// Where the log goes. `None` until the front end says.
static LOG: Mutex<Option<PathBuf>> = Mutex::new(None);

/// How many keystrokes are kept for the report.
///
/// Enough to see the shape of what was being done — a command line being
/// typed, a count, a table walk — and small enough that the ring costs
/// nothing to keep.
const KEPT_KEYS: usize = 64;

thread_local! {
    static KEYS: RefCell<Vec<Key>> = const { RefCell::new(Vec::new()) };
}

/// Say where the log lives. Called once, at startup, by the front end.
pub fn log_to(path: PathBuf) {
    if let Ok(mut slot) = LOG.lock() {
        *slot = Some(path);
    }
}

/// Where the log lives, if anywhere.
pub fn path() -> Option<PathBuf> {
    LOG.lock().ok().and_then(|slot| slot.clone())
}

/// Remember a keystroke for the report.
///
/// The whole cost is a push into a `Vec` of a `Copy` enum, bounded at
/// [`KEPT_KEYS`] — no allocation once the ring is warm, no formatting, no
/// lock. Formatting happens when something has already gone wrong.
pub fn note_key(key: Key) {
    KEYS.with(|ring| {
        let mut ring = ring.borrow_mut();
        if ring.len() == KEPT_KEYS {
            ring.remove(0);
        }
        ring.push(key);
    });
}

/// The keys kept so far, oldest first, as one line.
pub fn recent_keys() -> String {
    KEYS.with(|ring| {
        ring.borrow()
            .iter()
            .map(|k| match k {
                Key::Char(c) => c.to_string(),
                other => format!("<{other:?}>"),
            })
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// Append one entry. **Best effort**: a log that cannot be written is not a
/// reason to interrupt the writing, so every error here is dropped.
pub fn note(head: &str, body: &str) {
    let Some(path) = path() else { return };
    let stamp = crate::clock::stamp();
    let when = match stamp.len() == 14 {
        true => format!(
            "{}-{}-{} {}:{}:{}",
            &stamp[0..4],
            &stamp[4..6],
            &stamp[6..8],
            &stamp[8..10],
            &stamp[10..12],
            &stamp[12..14]
        ),
        false => stamp,
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = writeln!(file, "── {when}  {head}  yumete {}", env!("CARGO_PKG_VERSION"));
    for line in body.lines() {
        let _ = writeln!(file, "   {line}");
    }
    let _ = writeln!(file);
}

/// Write the next panic to the log instead of over the reader's screen.
///
/// The default hook prints to stderr, which inside the alternate screen lands
/// on top of the manuscript and is gone the moment the screen is torn down.
/// This one says the same things into a file that is still there afterwards,
/// and adds the two the standard hook cannot know: **what was pressed**, and
/// a backtrace captured whether or not `RUST_BACKTRACE` was set — nobody sets
/// it before the crash they did not expect.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| note("panic", &report(&info.to_string()))));
}

/// What the hook writes, apart from the timestamp — kept out of the hook so it
/// can be read by a test without installing a hook the whole process shares.
fn report(what: &str) -> String {
    // **Captured here and nowhere else.** Symbolising a backtrace walks the
    // binary's debug info, which on a debug build of this crate is ten to
    // fifteen seconds — fine once, in the instant something has already gone
    // wrong, and ruinous anywhere a test would reach it. So the capture is the
    // only thing this function does that [`compose`] does not, and the test
    // reads `compose` (#300: the cost belongs on the failure path).
    compose(what, &std::backtrace::Backtrace::force_capture().to_string())
}

/// The report's shape, with the backtrace handed in rather than captured.
fn compose(what: &str, backtrace: &str) -> String {
    format!("{what}\nkeys: {}\n{backtrace}", recent_keys())
}

/// Where the loop was when it last said anything.
///
/// Kept as a number so a beat is one relaxed store and nothing else — this is
/// on the hot path, which is the one place a diagnostic must not be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Stage {
    /// Between turns, with nothing being asked of it.
    Idle = 0,
    /// Working out how big the page is.
    Measuring = 1,
    /// Inside `terminal.draw`.
    Drawing = 2,
    /// Waiting for the terminal to say something.
    Reading = 3,
    /// Handling a key.
    Key = 4,
    /// Swallowing a wheel gesture.
    Wheel = 5,
    /// Scrolling by what the gesture said.
    Scrolling = 6,
    /// Some other mouse event.
    Mouse = 7,
    /// A paste.
    Paste = 8,
}

impl Stage {
    fn name(n: usize) -> &'static str {
        match n {
            0 => "idle",
            1 => "measuring",
            2 => "drawing",
            3 => "reading",
            4 => "key",
            5 => "wheel",
            6 => "scrolling",
            7 => "mouse",
            8 => "paste",
            _ => "?",
        }
    }
}

static STAGE: AtomicUsize = AtomicUsize::new(0);
static BEAT: AtomicU64 = AtomicU64::new(0);
static DETAIL: AtomicU64 = AtomicU64::new(0);
static STARTED: Mutex<Option<std::time::Instant>> = Mutex::new(None);

fn millis_now() -> u64 {
    let mut slot = match STARTED.lock() {
        Ok(slot) => slot,
        Err(_) => return 0,
    };
    let began = slot.get_or_insert_with(std::time::Instant::now);
    began.elapsed().as_millis() as u64
}

/// Say where the loop is, and one number about it (a notch count, a line —
/// whatever the stage has to say for itself).
///
/// Two relaxed stores and a clock read. Nothing allocates, nothing locks on
/// the common path, and no formatting happens unless something goes wrong.
pub fn beat(stage: Stage, detail: u64) {
    STAGE.store(stage as usize, Ordering::Relaxed);
    DETAIL.store(detail, Ordering::Relaxed);
    BEAT.store(millis_now(), Ordering::Relaxed);
}

/// Watch the heartbeat from a thread of its own, and write down a stall.
///
/// **A hang leaves nothing behind** — no panic, no message, and the reader can
/// only say 「it froze」. This turns that into a line naming the stage it
/// stopped in and how long ago. Reported once per stall, so a genuinely long
/// operation does not fill the log with the same paragraph.
pub fn watch(after: std::time::Duration) {
    std::thread::spawn(move || {
        let mut said_at = 0u64;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let beat = BEAT.load(Ordering::Relaxed);
            if beat == 0 || beat == said_at {
                continue;
            }
            let stalled = millis_now().saturating_sub(beat);
            if stalled >= after.as_millis() as u64 {
                let stage = Stage::name(STAGE.load(Ordering::Relaxed));
                let detail = DETAIL.load(Ordering::Relaxed);
                note(
                    "stall",
                    &format!(
                        "no beat for {stalled} ms; last stage {stage} ({detail})\nkeys: {}",
                        recent_keys()
                    ),
                );
                said_at = beat;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_the_last_keys_and_no_more() {
        for i in 0..(KEPT_KEYS + 10) {
            note_key(Key::Char(char::from_digit((i % 10) as u32, 10).unwrap()));
        }
        let said = recent_keys();
        assert_eq!(
            said.split(' ').count(),
            KEPT_KEYS,
            "the ring is bounded: {said}"
        );
        // The oldest ten fell off the front, so it starts where they stopped.
        assert!(said.starts_with("0 1 2"), "{said}");
    }

    /// **The shape of the report, not the capture** (#300).
    ///
    /// This used to call `report`, which captures a real backtrace — 14.6
    /// seconds of symbolising, in a lib suite that is otherwise 2. One test
    /// turned the fast lane into the slow one. `compose` is everything the
    /// hook writes except the capture, which is the one line nothing here
    /// could have asserted about anyway.
    #[test]
    fn a_report_says_what_was_pressed() {
        note_key(Key::Char('往'));
        note_key(Key::Esc);
        let said = compose("panicked at 'index out of bounds'", "  0: yumete::main");
        assert!(said.contains("index out of bounds"), "{said}");
        assert!(said.contains("往"), "the keys are in it: {said}");
        assert!(said.contains("<Esc>"), "and the named ones too: {said}");
    }

    #[test]
    fn a_note_lands_in_the_file_and_a_missing_one_is_not_fatal() {
        // No path set yet in this thread's process? Then nothing is written
        // and nothing complains — the whole contract of a best-effort log.
        let dir = std::env::temp_dir().join(format!("yumete-diag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let at = dir.join("deeper").join("yumete.log");
        log_to(at.clone());
        note("panic", "something gave way\nkeys: a b c");
        note("panic", "and again");
        let said = std::fs::read_to_string(&at).expect("the log was created, parents and all");
        assert_eq!(said.matches("── ").count(), 2, "appended, not replaced: {said}");
        assert!(said.contains("something gave way"), "{said}");
        assert!(said.contains(env!("CARGO_PKG_VERSION")), "the build is named: {said}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
