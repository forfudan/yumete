//! **The pointer resting on the interface brings up what its cell cannot
//! fit** (`[editor] mouse_hover`).
//!
//! Six places answer: the status line's language-server cell, its file
//! name and a message it had to cut, a sidebar row cut with `…`, the
//! gutter's problem sign with the diagnostic drawn after a line's end, and
//! the scrollbar's marks. Never the text itself: a panel over the words
//! being read would be in the way of the one thing on the screen that is
//! not chrome.
//!
//! What a panel says follows Emacs's help-echo: what the cell is (its
//! name, on the ring), the whole value, and what a click does where a
//! click does something. Only what the cell does not already show: a panel
//! that repeats its cell is not drawn.
//!
//! Three parts, kept apart so each can be tested alone:
//!
//! * [`Hover`], the timing: when a panel comes up and what puts it away.
//!   It lives in the front end beside the scroll positions, not in the
//!   editor, because it is about the pointer and the screen.
//! * [`begin`], [`offer`] and [`draw`], the drawing: each place that draws
//!   one of the six offers what it would say for the cells it drew, in the
//!   same pass, so the answer is about the frame on the screen. Nothing is
//!   worked out unless the pointer is on those cells.
//! * The `*_words` functions, the wording, from plain values.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::Frame;
use yumete_config::Config;
use yumete_core::say;

/// **How soon after a panel went away the next one comes up at once.**
/// Sweeping from one place to the next is reading along a row, and making
/// each of them wait the whole delay again would be making the reader stop
/// at each.
pub const AGAIN_WITHIN: Duration = Duration::from_millis(200);

/// The widest a panel is, ring included.
pub const WIDEST: u16 = 60;

/// **When a hover panel is up, and what puts it away.**
///
/// The pointer resting on one cell for the delay brings it up; the
/// pointer leaving that cell puts it away, and so does anything done: a
/// key, a click, the wheel, a drag, focus leaving the terminal, the input
/// method composing. After those, nothing comes up again until the pointer
/// moves to another cell — the pointer is still resting where it was, and
/// a panel that came back by itself the moment the typing paused would be
/// a panel that came back on every pause.
#[derive(Debug)]
pub struct Hover {
    /// `[editor] mouse` and `mouse_hover` both on.
    on: bool,
    /// `[editor] mouse_hover_delay`.
    delay: Duration,
    /// The cell the pointer is on, as far as a panel is concerned: `None`
    /// when it is over nothing that may have one (a picker, a page that
    /// takes the mouse) or nothing is known.
    at: Option<(u16, u16)>,
    /// When it came to rest there.
    since: Option<Instant>,
    /// The panel is wanted at `at`.
    shown: bool,
    /// The last frame drew it: the place had something to say.
    visible: bool,
    /// When a drawn panel last went away because the pointer left it.
    closed: Option<Instant>,
    /// Put away by something done; nothing until the pointer moves cell.
    held: bool,
    /// The input method is composing.
    composing: bool,
    /// **What the language servers are, said when the panel is wanted**:
    /// the loop holds the servers and the drawing does not, so it is
    /// worded on the loop's side ([`server_words`]).
    pub servers: Option<Said>,
}

impl Default for Hover {
    fn default() -> Hover {
        Hover {
            on: false,
            delay: Duration::from_millis(500),
            at: None,
            since: None,
            shown: false,
            visible: false,
            closed: None,
            held: false,
            composing: false,
            servers: None,
        }
    }
}

impl Hover {
    /// Take the settings: whether panels come up at all, and after how long.
    pub fn configure(&mut self, on: bool, delay: Duration) {
        self.delay = delay;
        if self.on && !on {
            self.put_away();
            self.at = None;
        }
        self.on = on;
    }

    /// The pointer is over `cell` now, or over nothing a panel is for.
    pub fn moved(&mut self, cell: Option<(u16, u16)>, now: Instant) {
        if cell == self.at {
            return;
        }
        if self.shown && self.visible {
            self.closed = Some(now);
        }
        self.at = cell;
        self.since = Some(now);
        self.visible = false;
        self.held = false;
        self.shown = self.ready()
            && cell.is_some()
            && self.closed.is_some_and(|closed| now.saturating_duration_since(closed) < AGAIN_WITHIN);
    }

    /// A press of the button, a drag or the wheel at `cell`: the panel goes,
    /// and stays away until the pointer moves to another cell.
    pub fn pressed(&mut self, cell: (u16, u16), now: Instant) {
        if self.at != Some(cell) {
            self.at = Some(cell);
            self.since = Some(now);
        }
        self.dismiss();
    }

    /// A key, a paste: the panel goes, and stays away until the pointer
    /// moves to another cell.
    pub fn dismiss(&mut self) {
        self.put_away();
        self.held = true;
    }

    /// Focus left the terminal: the pointer is somewhere else.
    pub fn lost_focus(&mut self) {
        self.put_away();
        self.at = None;
    }

    /// Whether the input method is composing; while it is, no panel.
    pub fn composing(&mut self, yes: bool) {
        if yes && !self.composing {
            self.dismiss();
        }
        self.composing = yes;
    }

    /// How long until the panel is due, while one is owed.
    pub fn due_in(&self, now: Instant) -> Option<Duration> {
        if !self.ready() || self.shown || self.at.is_none() {
            return None;
        }
        let rested = now.saturating_duration_since(self.since?);
        Some(self.delay.saturating_sub(rested))
    }

    /// The delay has run out: the panel is wanted.
    pub fn tick(&mut self, now: Instant) {
        if self.due_in(now) == Some(Duration::ZERO) {
            self.shown = true;
        }
    }

    /// The cell a panel is wanted at, if one is.
    pub fn showing(&self) -> Option<(u16, u16)> {
        self.at.filter(|_| self.shown && self.ready())
    }

    /// What the frame just drawn did: whether there was a panel to draw.
    pub fn drawn(&mut self, yes: bool) {
        self.visible = yes && self.shown;
    }

    /// Whether a panel is on the screen now, as the last frame drew it.
    pub fn on_screen(&self) -> bool {
        self.shown && self.visible
    }

    fn ready(&self) -> bool {
        self.on && !self.composing && !self.held
    }

    fn put_away(&mut self) {
        self.shown = false;
        self.visible = false;
    }
}

/// **What one place says**: its name, for the ring, and the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub title: String,
    pub body: String,
}

impl Said {
    pub fn new(title: String, body: String) -> Said {
        Said { title, body }
    }
}

/// What the frame found to say, and where.
struct Found {
    said: Said,
    at: (u16, u16),
    /// The status line's panels stand above it; everything else below the
    /// pointer, or above where there is no room.
    above: bool,
}

thread_local! {
    /// The cell a panel is wanted at, for the frame being drawn.
    static ASKED: Cell<Option<(u16, u16)>> = const { Cell::new(None) };
    /// What the place under it said, once one has.
    static FOUND: RefCell<Option<Found>> = const { RefCell::new(None) };
    /// What the loop said of the language servers ([`Hover::servers`]).
    static SERVERS: RefCell<Option<Said>> = const { RefCell::new(None) };
}

/// A frame starts: a panel is wanted at `asked`, or none is; `servers` is
/// what the language-server cell would say.
pub fn begin(asked: Option<(u16, u16)>, servers: Option<Said>) {
    ASKED.set(asked);
    FOUND.take();
    SERVERS.set(servers.filter(|_| asked.is_some()));
}

/// What the language-server cell says, for the frame being drawn.
pub fn servers() -> Option<Said> {
    SERVERS.with(|s| s.borrow().clone())
}

/// Whether the frame being drawn is to have a panel at all.
pub fn wanted() -> bool {
    ASKED.get().is_some()
}

/// **The pointer over `rect`?** Then this is the place, and `what` says
/// what it says of the cell the pointer is on — worked out only then, so a
/// frame with no panel wanted costs each place one comparison.
pub fn offer(rect: Rect, above: bool, what: impl FnOnce((u16, u16)) -> Option<Said>) {
    let Some(at) = ASKED.get() else { return };
    let inside = at.0 >= rect.x
        && at.0 < rect.x.saturating_add(rect.width)
        && at.1 >= rect.y
        && at.1 < rect.y.saturating_add(rect.height);
    if !inside || FOUND.with(|found| found.borrow().is_some()) {
        return;
    }
    if let Some(said) = what(at) {
        FOUND.set(Some(Found { said, at, above }));
    }
}

/// **Draw what was found**, in the float every panel is drawn as, at the
/// pointer: under it, or over it on the status line and where there is no
/// room below. Not interactive: it takes no keys and no focus, and the
/// next thing done puts it away.
pub fn draw(frame: &mut Frame, config: &Config, area: Rect) -> Option<Rect> {
    ASKED.set(None);
    let found = FOUND.take()?;
    // Under the pointer's row: the floor is the bottom of the screen, or
    // the pointer's own row for a panel that stands above it.
    let bottom = match found.above {
        true => found.at.1,
        false => area.y + area.height,
    };
    crate::panel::draw(frame, config, area, bottom, crate::chrome::Spot::cell(found.at), false, &crate::panel::Panel {
        stand: crate::panel::Stand::Pointer,
        accent: None,
        title: found.said.title,
        lede: None,
        entry: false,
        body: crate::panel::Body::Prose(found.said.body),
        scroll: 0,
        vertical_text: false,
        tag: None,
        pages: false,
        reading: true,
        marked: false,
    })
}

/// `name：value`, a row of a panel.
fn field(name: String, value: impl std::fmt::Display) -> String {
    say!("hover.field", name, value)
}

/// **The whole of something its cell cut short**, or nothing when it was
/// not cut: a sidebar row ended with `…`, a message the status line had to
/// shorten.
pub fn cut_words(whole: &str, shown: &str) -> Option<Said> {
    (whole.trim_end() != shown.trim_end()).then(|| Said::new(String::new(), whole.to_string()))
}

/// **The status line's file name**: where the file is, the project it is
/// in, how it is written and when it last changed — what `:info` says of
/// it that the name does not.
pub fn file_words(facts: &yumete_core::editor::FileFacts) -> Said {
    let mut rows = vec![
        field(
            say!("file.where"),
            match &facts.path {
                Some(path) => yumete_core::buffer::plain_spelling(path),
                None => say!("file.never-saved"),
            },
        ),
        field(say!("file.root"), facts.root.display()),
        field(say!("file.encoding"), &facts.encoding),
        field(say!("file.ending"), facts.ending),
    ];
    if let Some(when) = &facts.modified {
        rows.push(field(say!("file.modified-at"), when));
    }
    Said::new(say!("file.title"), rows.join("\n"))
}

/// **The language-server cell**: each server watching the file — what it
/// said it is and whether it is running, the program started and the
/// environment it runs in — or why none is, then the file's problems by
/// kind. Nothing when no server belongs here and nothing has been said.
pub fn server_words(
    standing: &crate::server::Standing,
    facts: &[crate::server::Facts],
    counts: [usize; 3],
) -> Option<Said> {
    use crate::server::Standing;
    let mut blocks: Vec<String> = Vec::new();
    match standing {
        Standing::Serving(_) if !facts.is_empty() => {
            for one in facts {
                let name = one.said.clone().unwrap_or_else(|| one.command.clone());
                let mut rows = vec![match one.ready {
                    true => say!("file.server-ready", name),
                    false => say!("file.server-starting", name),
                }];
                if let Some(path) = &one.path {
                    rows.push(field(say!("hover.command"), path.display()));
                }
                if let Some(env) = &one.environment {
                    rows.push(field(say!("hover.pixi-environment"), env));
                }
                blocks.push(rows.join("\n"));
            }
        }
        other => blocks.extend(crate::standing_in_words(other)),
    }
    let [errors, warnings, info] = counts;
    if matches!(standing, Standing::Serving(_)) || errors + warnings + info > 0 {
        blocks.push(say!("file.problem-counts", errors, warnings, info));
    }
    (!blocks.is_empty()).then(|| Said::new(say!("file.servers"), blocks.join("\n\n")))
}

/// The word a diagnostic's kind is said with, the one the problem panel
/// and the sidebar put before each message.
pub fn loudness(severity: yumete_core::problem::Severity) -> String {
    use yumete_core::problem::Severity;
    match severity {
        Severity::Error => say!("problem.error"),
        Severity::Warn => say!("problem.warn"),
        Severity::Note => say!("problem.note"),
        Severity::Hint => say!("problem.hint"),
    }
}

/// **Every diagnostic on one line**, each as the problem panel writes it,
/// for the gutter's sign and the message drawn after the line, which shows
/// only the loudest.
pub fn problem_words<'a>(said: impl IntoIterator<Item = &'a yumete_core::problem::Problem>) -> Option<Said> {
    let rows: Vec<String> = said
        .into_iter()
        .map(|p| format!("{}　{}", loudness(p.severity), p.message.trim()))
        .collect();
    (!rows.is_empty()).then(|| {
        Said::new(yumete_core::messages::say(yumete_core::sidebar::Info::Problems.tag(), &[]), rows.join("\n\n"))
    })
}

/// **What one cell of the scrollbar stands for**, counted by
/// [`crate::scrollbar::covered`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Covered {
    /// Errors, and the first one's message.
    pub errors: (usize, Option<String>),
    /// Warnings, and the first one's message.
    pub warnings: (usize, Option<String>),
    /// Lines a search hit.
    pub search: usize,
    /// Lines of the last listing.
    pub listing: usize,
    /// Runs `:view-diff` marks.
    pub changes: usize,
}

/// **A scrollbar mark**: what the lines under its cell hold, the loudest
/// first, and that a click turns the page there.
pub fn scrollbar_words(covered: &Covered) -> Option<Said> {
    let mut rows = Vec::new();
    let one = |message: &Option<String>| message.as_deref().map(str::trim).unwrap_or("").to_string();
    if covered.errors.0 > 0 {
        rows.push(say!("hover.scrollbar-errors", covered.errors.0, one(&covered.errors.1)));
    }
    if covered.warnings.0 > 0 {
        rows.push(say!("hover.scrollbar-warnings", covered.warnings.0, one(&covered.warnings.1)));
    }
    if covered.search > 0 {
        rows.push(say!("hover.scrollbar-search", covered.search));
    }
    if covered.listing > 0 {
        rows.push(say!("hover.scrollbar-listing", covered.listing));
    }
    if covered.changes > 0 {
        rows.push(say!("hover.scrollbar-changes", covered.changes));
    }
    if rows.is_empty() {
        return None;
    }
    rows.push(String::new());
    rows.push(say!("hover.scrollbar-click"));
    Some(Said::new(say!("hover.scrollbar"), rows.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn hover() -> (Hover, Instant) {
        let mut h = Hover::default();
        h.configure(true, ms(500));
        (h, Instant::now())
    }

    /// **The panel comes up after the pointer has rested the delay on one
    /// cell**, and a move to another cell starts the wait again.
    #[test]
    fn the_panel_waits_for_the_pointer_to_rest() {
        let (mut h, t) = hover();
        h.moved(Some((3, 4)), t);
        assert_eq!(h.due_in(t), Some(ms(500)));
        h.tick(t + ms(499));
        assert_eq!(h.showing(), None);
        h.moved(Some((4, 4)), t + ms(300));
        h.tick(t + ms(600));
        assert_eq!(h.showing(), None, "the move started the wait again");
        assert_eq!(h.due_in(t + ms(600)), Some(ms(200)));
        h.tick(t + ms(800));
        assert_eq!(h.showing(), Some((4, 4)));
        assert_eq!(h.due_in(t + ms(800)), None, "nothing owed while it is up");
        // Over nothing a panel is for, nothing is owed.
        h.moved(None, t + ms(900));
        assert_eq!((h.showing(), h.due_in(t + ms(900))), (None, None));
    }

    /// **Within 200 ms of a drawn panel going away, the next comes at
    /// once**, even across a cell with nothing to say; after that, the
    /// whole delay again. A panel wanted where nothing was drawn does not
    /// count as one that went away.
    #[test]
    fn a_panel_just_closed_brings_the_next_at_once() {
        let (mut h, t) = hover();
        h.moved(Some((1, 1)), t);
        h.tick(t + ms(500));
        h.drawn(true);
        h.moved(Some((2, 1)), t + ms(600));
        assert_eq!(h.showing(), Some((2, 1)), "the next cell at once");
        h.drawn(false);
        h.moved(Some((3, 1)), t + ms(750));
        assert_eq!(h.showing(), Some((3, 1)), "still within 200 ms of the one that was drawn");
        h.drawn(false);
        h.moved(Some((4, 1)), t + ms(850));
        assert_eq!(h.showing(), None, "past 200 ms");
        h.tick(t + ms(1349));
        assert_eq!(h.showing(), None);
        h.tick(t + ms(1350));
        assert_eq!(h.showing(), Some((4, 1)));
    }

    /// **Each thing done puts the panel away**, and nothing comes up again
    /// until the pointer moves to another cell — then after the delay, not
    /// at once: what put it away was not a sweep.
    #[test]
    fn each_thing_done_puts_it_away_until_the_pointer_moves() {
        type Doing = fn(&mut Hover, Instant);
        let doings: [(&str, Doing); 5] = [
            ("a key", |h, _| h.dismiss()),
            ("a click", |h, t| h.pressed((5, 5), t)),
            ("the wheel", |h, t| h.pressed((5, 5), t)),
            ("a drag", |h, t| h.pressed((5, 6), t)),
            ("composing", |h, _| h.composing(true)),
        ];
        for (what, doing) in doings {
            let (mut h, t) = hover();
            h.moved(Some((5, 5)), t);
            h.tick(t + ms(500));
            h.drawn(true);
            doing(&mut h, t + ms(600));
            assert_eq!(h.showing(), None, "{what}");
            h.composing(false);
            h.tick(t + ms(5000));
            assert_eq!(h.showing(), None, "{what}: not back while the pointer rests");
            assert_eq!(h.due_in(t + ms(5000)), None, "{what}");
            h.moved(Some((7, 7)), t + ms(5000));
            assert_eq!(h.showing(), None, "{what}: not at once");
            h.tick(t + ms(5500));
            assert_eq!(h.showing(), Some((7, 7)), "{what}: after the delay on the new cell");
        }
    }

    /// **The pointer leaving the cell puts it away**; focus leaving the
    /// terminal too, and nothing is owed until the pointer is heard again.
    #[test]
    fn leaving_the_cell_or_the_terminal_puts_it_away() {
        let (mut h, t) = hover();
        h.moved(Some((5, 5)), t);
        h.tick(t + ms(500));
        h.drawn(true);
        h.moved(None, t + ms(600));
        assert_eq!(h.showing(), None, "off every place a panel is for");
        h.moved(Some((5, 5)), t + ms(1000));
        h.tick(t + ms(1500));
        assert_eq!(h.showing(), Some((5, 5)));
        h.lost_focus();
        assert_eq!((h.showing(), h.due_in(t + ms(2000))), (None, None));
    }

    /// **After a click, nothing until the pointer moves to another cell**,
    /// even when the terminal reports the pointer on the cell it clicked.
    #[test]
    fn after_a_click_nothing_until_the_pointer_moves_cell() {
        let (mut h, t) = hover();
        h.moved(Some((2, 2)), t);
        h.pressed((2, 2), t + ms(100));
        h.moved(Some((2, 2)), t + ms(200));
        h.tick(t + ms(2000));
        assert_eq!(h.showing(), None);
        h.moved(Some((3, 2)), t + ms(2000));
        h.tick(t + ms(2500));
        assert_eq!(h.showing(), Some((3, 2)));
    }

    /// **Never while composing**: nothing owed, nothing shown, and a panel
    /// that is up goes.
    #[test]
    fn nothing_while_composing() {
        let (mut h, t) = hover();
        h.composing(true);
        h.moved(Some((1, 1)), t);
        assert_eq!(h.due_in(t), None);
        h.tick(t + ms(1000));
        assert_eq!(h.showing(), None);
        h.composing(false);
        h.tick(t + ms(1500));
        assert_eq!(h.showing(), Some((1, 1)), "the rest began on the move, which composing did not undo");
        h.composing(true);
        assert_eq!(h.showing(), None);
    }

    /// **`mouse_hover = false`**: nothing owed, nothing shown, and turning
    /// it off puts away a panel that is up.
    #[test]
    fn nothing_with_hover_off() {
        let mut h = Hover::default();
        let t = Instant::now();
        h.configure(false, ms(500));
        h.moved(Some((1, 1)), t);
        assert_eq!(h.due_in(t), None);
        h.tick(t + ms(1000));
        assert_eq!(h.showing(), None);
        h.configure(true, ms(500));
        h.moved(Some((2, 1)), t + ms(1000));
        h.tick(t + ms(1500));
        assert_eq!(h.showing(), Some((2, 1)));
        h.configure(false, ms(500));
        assert_eq!(h.showing(), None);
        assert_eq!(h.due_in(t + ms(2000)), None);
    }

    /// **A place offers only for its own cells**, the first to offer wins,
    /// and nothing is worked out when no panel is wanted.
    #[test]
    fn a_place_offers_for_its_own_cells_only() {
        let said = |_| Some(Said::new("t".into(), "b".into()));
        begin(None, None);
        offer(Rect::new(0, 0, 10, 10), false, |_| panic!("worked out with no panel wanted"));
        begin(Some((12, 3)), None);
        offer(Rect::new(0, 0, 10, 10), false, |_| panic!("worked out for a cell outside"));
        offer(Rect::new(10, 3, 5, 1), false, said);
        offer(Rect::new(10, 3, 5, 1), false, |_| panic!("worked out after a place had answered"));
        assert!(FOUND.with(|f| f.borrow().as_ref().is_some_and(|f| f.at == (12, 3))));
        begin(None, None);
    }

    /// **A cut row says its whole**; a row shown whole says nothing.
    #[test]
    fn a_cut_row_says_its_whole_and_a_whole_one_nothing() {
        assert_eq!(cut_words("天地玄黄.md", "天地玄黄.md"), None);
        assert_eq!(
            cut_words("天地玄黄宇宙洪荒.md", "天地玄黄宇…"),
            Some(Said::new(String::new(), "天地玄黄宇宙洪荒.md".to_string()))
        );
        // A message the status line shortened.
        let whole = "written /tmp/a/very/long/folder/name/chapter.md";
        assert!(cut_words(whole, "written /tmp/…/chapter.md").is_some_and(|s| s.body == whole));
    }

    /// **Every diagnostic on the line**, each with its kind, in the order
    /// they were said.
    #[test]
    fn a_line_with_three_diagnostics_says_all_three() {
        use yumete_core::problem::{Problem, Severity};
        let p = |severity, message: &str| Problem {
            line: 4,
            utf16_column: 0,
            severity,
            message: message.to_string(),
            source: None,
            code: None,
        };
        let said = [p(Severity::Warn, "unused x"), p(Severity::Error, "no such y"), p(Severity::Hint, "rename z")];
        let words = problem_words(&said).unwrap();
        let rows: Vec<&str> = words.body.split("\n\n").collect();
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert!(rows[0].ends_with("unused x") && rows[0].starts_with(&loudness(Severity::Warn)));
        assert!(rows[1].ends_with("no such y") && rows[1].starts_with(&loudness(Severity::Error)));
        assert!(rows[2].ends_with("rename z"));
        assert_eq!(problem_words(&[]), None);
    }

    /// **The file name's panel**: the whole path, the project, the
    /// encoding, the line ending and the time it changed, each by the name
    /// `:info` gives it.
    #[test]
    fn the_file_name_says_where_and_how() {
        let facts = yumete_core::editor::FileFacts {
            path: Some("/book/drafts/one.md".into()),
            root: "/book".into(),
            encoding: "GBK".to_string(),
            ending: "CRLF",
            modified: Some("2026-01-02 03:04".to_string()),
        };
        let words = file_words(&facts);
        assert_eq!(words.title, say!("file.title"));
        for part in ["/book/drafts/one.md", "/book", "GBK", "CRLF", "2026-01-02 03:04", &say!("file.root")] {
            assert!(words.body.contains(part), "{part} in {}", words.body);
        }
        let unsaved = file_words(&yumete_core::editor::FileFacts { path: None, modified: None, ..facts });
        assert!(unsaved.body.contains(&say!("file.never-saved")), "{}", unsaved.body);
    }

    /// **The language-server cell**: each server with its version, state,
    /// program and environment, then the counts; why none runs when none
    /// does; nothing when none belongs here and nothing was said.
    #[test]
    fn the_server_cell_says_each_server() {
        use crate::server::{Facts, Lost, Standing};
        let facts = vec![
            Facts {
                command: "ruff".into(),
                said: Some("ruff 0.9.1".into()),
                ready: true,
                path: Some("/p/.pixi/envs/default/bin/ruff".into()),
                environment: Some("default".into()),
            },
            Facts { command: "pylsp".into(), said: None, ready: false, path: Some("/usr/bin/pylsp".into()), environment: None },
        ];
        let standing = Standing::Serving(vec![("ruff".into(), true), ("pylsp".into(), false)]);
        let words = server_words(&standing, &facts, [1, 2, 3]).unwrap();
        assert_eq!(words.title, say!("file.servers"));
        for part in [
            say!("file.server-ready", "ruff 0.9.1"),
            say!("file.server-starting", "pylsp"),
            "/p/.pixi/envs/default/bin/ruff".to_string(),
            "/usr/bin/pylsp".to_string(),
            say!("hover.field", say!("hover.pixi-environment"), "default"),
            say!("file.problem-counts", 1, 2, 3),
        ] {
            assert!(words.body.contains(&part), "{part} in {}", words.body);
        }
        let lost = Standing::Lost(Lost::Exited("gopls".into()));
        let words = server_words(&lost, &[], [0, 0, 0]).unwrap();
        assert_eq!(words.body, say!("file.server-exited", "gopls"));
        assert_eq!(server_words(&Standing::Unserved, &[], [0, 0, 0]), None);
        let checked = server_words(&Standing::Unserved, &[], [1, 0, 0]).unwrap();
        assert_eq!(checked.body, say!("file.problem-counts", 1, 0, 0), "the config check needs no server");
    }

    /// **A scrollbar mark says what its cell holds** and that a click turns
    /// the page there; an unmarked cell says nothing.
    #[test]
    fn a_scrollbar_mark_says_what_it_holds() {
        assert_eq!(scrollbar_words(&Covered::default()), None);
        let covered = Covered {
            errors: (2, Some("expected `;`".into())),
            warnings: (0, None),
            search: 3,
            listing: 0,
            changes: 1,
        };
        let words = scrollbar_words(&covered).unwrap();
        assert!(words.body.contains(&say!("hover.scrollbar-errors", 2, "expected `;`")), "{}", words.body);
        assert!(words.body.contains(&say!("hover.scrollbar-search", 3)));
        assert!(words.body.contains(&say!("hover.scrollbar-changes", 1)));
        assert!(!words.body.contains(&say!("hover.scrollbar-listing", 0)));
        assert!(words.body.ends_with(&say!("hover.scrollbar-click")));
    }
}
