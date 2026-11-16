//! **A dotted underline** (#287, `development.md` §5.8.4) — the one thing the
//! page draws that ratatui 0.29 cannot say.
//!
//! ratatui has one underline (`UNDERLINED`, SGR 4). Terminals that came with
//! coloured underlines also draw dotted ones (SGR `4:4`), and a wiki name has
//! to be told apart from a link, which is already solidly underlined. So a
//! cell asks for it with a modifier bit ratatui does not use ([`DOTTED`]), and
//! this backend writes those cells itself; every other cell goes through the
//! crossterm backend untouched.
//!
//! `TestBackend` and `--shot` never see this file: an unknown modifier bit is
//! ignored there, so the tests draw what they always drew.

use std::io::{self, Write};

use ratatui::crossterm::cursor::MoveTo;
use ratatui::crossterm::queue;
use ratatui::crossterm::style::{
    Attribute, Color as CColor, Colors, Print, SetAttribute, SetColors, SetUnderlineColor,
};
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, Clear, EndSynchronizedUpdate};
use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::Modifier;

/// The bit a cell sets to be underlined with dots. ratatui's own modifiers
/// take bits 0–8.
pub const DOTTED: Modifier = Modifier::from_bits_retain(1 << 9);

/// **Move the cursor where the system input method should type, and keep it
/// out of sight.**
///
/// 竪排插入態 draws its own caret — a rule on the boundary the text arrives at
/// — and the terminal's cursor is then not a caret at all: it is the anchor a
/// macOS input method draws its inline preedit on. Those two want different
/// cells. The rule belongs on the boundary *above* the character the text goes
/// in front of; the preedit belongs **on that character**, because that is
/// where the text it is composing will appear: `i` on 「餅」 must not have the
/// preedit cover the 「蛋」 above it.
///
/// So the cursor goes on the character and this flag hides it, leaving the rule
/// the only caret on the page.
///
/// A process-wide flag because the backend is built in `run` and the question
/// is answered a frame at a time, deep inside the draw.
static HIDE_THE_CURSOR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the terminal's cursor is to be moved but not shown. See
/// [`HIDE_THE_CURSOR`].
pub fn keep_the_cursor_hidden(hidden: bool) {
    HIDE_THE_CURSOR.store(hidden, std::sync::atomic::Ordering::Relaxed);
}

/// The crossterm backend, with dotted underlines, and **a clear of the
/// screen written with the frame that follows it**.
///
/// ratatui clears the whole screen when the terminal changes size, and
/// `:redraw` does so on purpose; the crossterm backend writes and flushes
/// that clear at once, and the frame comes only after the page is laid out
/// again — long enough on a big vertical page to see the screen go blank.
/// Here the clear waits for the next frame's cells and goes out in the same
/// write, opened as a synchronized update (`CSI ? 2026 h`) and closed at the
/// flush, so a terminal that knows the mode shows the old screen until the
/// new one is whole, and one that does not still has no blank frame to show
/// between the two.
pub struct Dotted<W: Write> {
    inner: CrosstermBackend<W>,
    /// A clear of the whole screen asked for and not yet written.
    owed_clear: bool,
    /// The frame being written opened a synchronized update, which the
    /// flush closes.
    synchronized: bool,
}

impl<W: Write> Dotted<W> {
    pub fn new(writer: W) -> Dotted<W> {
        Dotted { inner: CrosstermBackend::new(writer), owed_clear: false, synchronized: false }
    }

    /// One dotted cell, from a clean state and back to one — the crossterm
    /// backend resets at the end of every `draw`, so the two can take turns.
    fn cell(&mut self, x: u16, y: u16, cell: &Cell) -> io::Result<()> {
        let w = self.inner.writer_mut();
        queue!(
            w,
            MoveTo(x, y),
            SetAttribute(Attribute::Reset),
            SetColors(Colors::new(cell.fg.into(), cell.bg.into())),
            SetUnderlineColor(cell.underline_color.into()),
        )?;
        for (bit, attribute) in [
            (Modifier::BOLD, Attribute::Bold),
            (Modifier::DIM, Attribute::Dim),
            (Modifier::ITALIC, Attribute::Italic),
            (Modifier::REVERSED, Attribute::Reverse),
            (Modifier::CROSSED_OUT, Attribute::CrossedOut),
        ] {
            if cell.modifier.contains(bit) {
                queue!(w, SetAttribute(attribute))?;
            }
        }
        // Warning: **Both, in this order, and the first is not redundant.** A
        // terminal that parses colon sub-parameters turns the solid line into
        // a dotted one; a terminal that does not drops the sequence it cannot
        // read and keeps the solid line it was already given. The worst case
        // is 「looks like a link」, never 「nothing at all」.
        queue!(
            w,
            SetAttribute(Attribute::Underlined),
            SetAttribute(Attribute::Underdotted),
            Print(cell.symbol()),
            SetAttribute(Attribute::Reset),
            SetColors(Colors::new(CColor::Reset, CColor::Reset)),
            SetUnderlineColor(CColor::Reset),
        )
    }
}

/// Raw writes go straight through — `:!` hands the screen over with a few
/// escape sequences of its own.
impl<W: Write> Write for Dotted<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.writer_mut().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.writer_mut().flush()
    }
}

impl<W: Write> Backend for Dotted<W> {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        if std::mem::take(&mut self.owed_clear) {
            queue!(
                self.inner.writer_mut(),
                BeginSynchronizedUpdate,
                Clear(ratatui::crossterm::terminal::ClearType::All)
            )?;
            self.synchronized = true;
        }
        // **A picture's upload goes ahead of the cells that show it**
        // (`graphics.rs`): the placeholders below are only text
        // until the terminal has the picture they name.
        let owed = crate::graphics::take_pending();
        if !owed.is_empty() {
            self.inner.writer_mut().write_all(&owed)?;
        }
        let cells: Vec<(u16, u16, &Cell)> = content.collect();
        let mut at = 0;
        while at < cells.len() {
            let dotted = cells[at].2.modifier.contains(DOTTED);
            let end = cells[at..]
                .iter()
                .position(|c| c.2.modifier.contains(DOTTED) != dotted)
                .map_or(cells.len(), |n| at + n);
            if dotted {
                for &(x, y, cell) in &cells[at..end] {
                    self.cell(x, y, cell)?;
                }
            } else {
                self.inner.draw(cells[at..end].iter().copied())?;
            }
            at = end;
        }
        Ok(())
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }
    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }
    fn show_cursor(&mut self) -> io::Result<()> {
        // ratatui shows the cursor whenever a frame set its position, and a
        // frame has to set it for the input method to find it — see
        // [`HIDE_THE_CURSOR`].
        match HIDE_THE_CURSOR.load(std::sync::atomic::Ordering::Relaxed) {
            true => self.inner.hide_cursor(),
            false => self.inner.show_cursor(),
        }
    }
    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }
    fn clear(&mut self) -> io::Result<()> {
        self.owed_clear = true;
        Ok(())
    }
    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        match clear_type {
            ClearType::All => self.clear(),
            _ => self.inner.clear_region(clear_type),
        }
    }
    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }
    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }
    fn flush(&mut self) -> io::Result<()> {
        if std::mem::take(&mut self.owed_clear) {
            queue!(self.inner.writer_mut(), Clear(ratatui::crossterm::terminal::ClearType::All))?;
        }
        if std::mem::take(&mut self.synchronized) {
            queue!(self.inner.writer_mut(), EndSynchronizedUpdate)?;
        }
        Backend::flush(&mut self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(backend: &mut Dotted<Vec<u8>>) -> String {
        String::from_utf8(std::mem::take(backend.inner.writer_mut())).unwrap()
    }

    /// **The clear goes out with the frame**, inside one synchronized
    /// update: nothing is written when it is asked for, and the cells
    /// follow it in the same write.
    #[test]
    fn a_clear_waits_for_the_frame_and_goes_out_with_it() {
        let mut backend = Dotted::new(Vec::new());
        backend.clear_region(ClearType::All).unwrap();
        assert_eq!(written(&mut backend), "", "nothing before the frame");
        let cell = Cell::new("字");
        backend.draw([(3u16, 1u16, &cell)].into_iter()).unwrap();
        Backend::flush(&mut backend).unwrap();
        let out = written(&mut backend);
        let begin = out.find("\x1b[?2026h").expect("opened");
        let clear = out.find("\x1b[2J").expect("cleared");
        let cell = out.find('字').expect("the cell");
        let end = out.find("\x1b[?2026l").expect("closed");
        assert!(begin < clear && clear < cell && cell < end, "{out:?}");
    }

    /// **A frame with no clear before it is written plainly**:
    /// no clear and no synchronized update.
    #[test]
    fn a_frame_without_a_clear_writes_no_clear() {
        let mut backend = Dotted::new(Vec::new());
        let cell = Cell::new("字");
        backend.draw([(0u16, 0u16, &cell)].into_iter()).unwrap();
        Backend::flush(&mut backend).unwrap();
        let out = written(&mut backend);
        assert!(out.contains('字'));
        assert!(!out.contains("\x1b[2J") && !out.contains("2026"), "{out:?}");
    }
}
