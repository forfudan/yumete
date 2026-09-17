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
use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::Modifier;

/// The bit a cell sets to be underlined with dots. ratatui's own modifiers
/// take bits 0–8.
pub const DOTTED: Modifier = Modifier::from_bits_retain(1 << 9);

/// The crossterm backend, with dotted underlines.
pub struct Dotted<W: Write> {
    inner: CrosstermBackend<W>,
}

impl<W: Write> Dotted<W> {
    pub fn new(writer: W) -> Dotted<W> {
        Dotted { inner: CrosstermBackend::new(writer) }
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
        // ⚠️ **Both, in this order, and the first is not redundant.** A
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
        self.inner.show_cursor()
    }
    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }
    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }
    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }
    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }
    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }
    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}
