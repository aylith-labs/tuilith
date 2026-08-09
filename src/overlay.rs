//! A floating surface whose background cannot be forgotten.
//!
//! Anything drawn over a page has to clear the cells underneath it first, and clearing resets them to
//! the terminal's own colours rather than the theme's. So an overlay that clears and then draws only a
//! border and some text shows *the terminal's* background under theme-coloured glyphs — which is
//! invisible for as long as the theme's polarity happens to match the terminal's, and illegible the
//! moment it does not.
//!
//! The fix is not a convention. [`Overlay::draw`] is the only way to put one of these on screen, and it
//! clears, paints the background and foreground, draws the frame and hands back the inside — so there is
//! no order of calls that produces an unpainted overlay.
//!
//! ```no_run
//! # use ratatui::{Frame, layout::Rect, widgets::Padding};
//! # use tuilith::{Mode, overlay::Overlay};
//! # fn draw(frame: &mut Frame, area: Rect) {
//! let palette = Mode::Dark.palette();
//! let inside = Overlay::bordered(palette)
//!     .edge(palette.bad)
//!     .padding(Padding::horizontal(1))
//!     .draw(frame, area);
//! // `inside` is where the content goes.
//! # }
//! ```
//!
//! [`inspect::fully_painted`](crate::inspect::fully_painted) is the assertion that a surface actually
//! came out painted, and it is what proves this type is doing its job.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Padding, Widget};

use crate::theme::Palette;

crate::provenance! {
    component: "overlay",
    about: "A floating surface that clears and repaints from the theme, so it cannot come out unpainted",
    origin: crate::Origin::Here,
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// A surface to draw over a page: a modal, a notice, a popover, a pane.
///
/// Built like a [`Block`], because it is one underneath — but it owns the palette, so drawing it always
/// paints. The builders cover what an overlay legitimately varies; the background is not among them.
pub struct Overlay<'a> {
    palette: Palette,
    block: Block<'a>,
}

impl<'a> Overlay<'a> {
    /// An overlay with no border.
    #[must_use]
    pub fn new(palette: Palette) -> Self {
        Self {
            palette,
            block: Block::new(),
        }
    }

    /// An overlay bordered on all four sides.
    #[must_use]
    pub fn bordered(palette: Palette) -> Self {
        Self {
            palette,
            block: Block::bordered(),
        }
    }

    /// Which sides carry a border.
    #[must_use]
    pub fn borders(mut self, borders: Borders) -> Self {
        self.block = self.block.borders(borders);
        self
    }

    /// The border's colour — the one thing an overlay usually varies, since it is how a notice says
    /// whether it is a warning or a failure.
    #[must_use]
    pub fn edge(mut self, colour: Color) -> Self {
        self.block = self.block.border_style(Style::new().fg(colour));
        self
    }

    /// A title on the border.
    #[must_use]
    pub fn title(mut self, title: Line<'a>) -> Self {
        self.block = self.block.title(title);
        self
    }

    /// Padding between the border and the content.
    #[must_use]
    pub fn padding(mut self, padding: Padding) -> Self {
        self.block = self.block.padding(padding);
        self
    }

    /// Where the content will go, without drawing anything.
    ///
    /// For a caller that has to decide whether the overlay is worth drawing at all — a box too short for
    /// its own text is worse than no box — and so needs the inside before it commits. It agrees with
    /// what [`draw`](Self::draw) returns.
    #[must_use]
    pub fn inner(&self, area: Rect) -> Rect {
        self.block.inner(area)
    }

    /// Clear, paint, draw the frame, and return the inside.
    #[must_use]
    pub fn draw(self, frame: &mut Frame, area: Rect) -> Rect {
        self.draw_on(frame.buffer_mut(), area)
    }

    /// The same against a bare buffer, which is how a test looks at one.
    #[must_use]
    pub fn draw_on(self, buffer: &mut Buffer, area: Rect) -> Rect {
        let inside = self.block.inner(area);
        Clear.render(area, buffer);
        // Both halves, not just the background. A cell left with the terminal's foreground draws its
        // glyph in a colour the theme never chose, which on a filled surface is the same defect one
        // layer in.
        self.block
            .style(
                Style::new()
                    .bg(self.palette.background)
                    .fg(self.palette.foreground),
            )
            .render(area, buffer);
        inside
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Mode;

    fn buffer(width: u16, height: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, width, height))
    }

    #[test]
    fn what_it_promised_to_leave_is_what_it_left() {
        // The two must agree or a caller that measured before drawing writes its content in the wrong
        // place — and `inner` exists precisely so a caller can measure before committing.
        for padding in [Padding::ZERO, Padding::horizontal(1), Padding::uniform(2)] {
            let area = Rect::new(1, 2, 30, 10);
            let overlay = Overlay::bordered(Mode::Dark.palette()).padding(padding);
            let promised = overlay.inner(area);
            let left = overlay.draw_on(&mut buffer(40, 20), area);
            assert_eq!(promised, left);
        }
    }

    #[test]
    fn every_cell_it_covers_carries_the_palette_rather_than_the_terminals_colours() {
        let palette = Mode::Dark.palette();
        let mut buffer = buffer(20, 8);
        let area = Rect::new(2, 1, 15, 6);
        let _ = Overlay::bordered(palette)
            .edge(palette.bad)
            .draw_on(&mut buffer, area);

        for row in area.top()..area.bottom() {
            for column in area.left()..area.right() {
                let cell = &buffer[(column, row)];
                assert_eq!(
                    cell.bg, palette.background,
                    "cell ({column}, {row}) kept the terminal's background"
                );
            }
        }
    }

    #[test]
    fn it_paints_only_its_own_area() {
        // An overlay that bled would repaint the page underneath it, which is the opposite failure and
        // just as invisible when the two happen to share a background.
        let mut buffer = buffer(20, 8);
        let area = Rect::new(2, 1, 5, 3);
        let _ = Overlay::new(Mode::Dark.palette()).draw_on(&mut buffer, area);
        assert_eq!(buffer[(1, 1)].bg, Color::Reset, "it painted to the left");
        assert_eq!(buffer[(7, 1)].bg, Color::Reset, "it painted to the right");
        assert_eq!(buffer[(2, 0)].bg, Color::Reset, "it painted above");
        assert_eq!(buffer[(2, 4)].bg, Color::Reset, "it painted below");
    }

    #[test]
    fn the_border_keeps_the_colour_it_was_given() {
        let palette = Mode::Dark.palette();
        let mut buffer = buffer(12, 5);
        let area = Rect::new(0, 0, 12, 5);
        let _ = Overlay::bordered(palette)
            .edge(palette.warn)
            .draw_on(&mut buffer, area);
        assert_eq!(buffer[(0, 0)].fg, palette.warn);
        // …and the fill is still the theme's, so the edge colour did not become the surface's.
        assert_eq!(buffer[(6, 2)].bg, palette.background);
        assert_eq!(buffer[(6, 2)].fg, palette.foreground);
    }

    #[test]
    fn a_surface_with_no_room_left_inside_still_paints_what_it_covers() {
        // A caller bails on this case, but it must not be able to bail *after* a half-drawn box: the
        // paint happens whatever the geometry.
        let palette = Mode::Light.palette();
        let mut buffer = buffer(4, 2);
        let area = Rect::new(0, 0, 2, 2);
        let inside = Overlay::bordered(palette).draw_on(&mut buffer, area);
        assert_eq!(inside.width, 0);
        assert_eq!(inside.height, 0);
        assert_eq!(buffer[(0, 0)].bg, palette.background);
        assert_eq!(buffer[(1, 1)].bg, palette.background);
    }
}
