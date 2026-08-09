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

/// Push everything in `area` into the background, so what is drawn next reads as being in front.
///
/// A terminal has no alpha, so a scrim cannot be a translucent layer over the page — it has to be the page
/// itself, restyled. Every glyph's colour moves most of the way toward the surface behind it, which is what
/// "receded" looks like when the only channels available are hue and contrast: the text is still there, and
/// it is plainly not what you are being asked to read.
///
/// Backgrounds are left alone deliberately. Flattening them would erase the selected row, the pane
/// boundaries and every band the page uses to say where things are — so the page would not recede, it would
/// dissolve, and dismissing the modal would look like arriving somewhere new.
///
/// Call it before drawing the thing in front. Nothing enforces that order, because a scrim over the modal
/// as well would simply dim the modal too, which is visible immediately rather than subtly wrong.
pub fn scrim(buffer: &mut Buffer, area: Rect, palette: Palette) {
    for row in area.top()..area.bottom().min(buffer.area.bottom()) {
        for column in area.left()..area.right().min(buffer.area.right()) {
            let cell = &mut buffer[(column, row)];
            let behind = if cell.bg == Color::Reset {
                palette.background
            } else {
                cell.bg
            };
            cell.fg = toward(cell.fg, behind, palette);
        }
    }
}

/// How far a scrimmed glyph travels toward the surface behind it, in hundredths.
///
/// Most of the way, not all: at 100 the text vanishes and the page reads as blank rather than as behind
/// something, which loses the sense that dismissing the modal returns you to where you were. In hundredths
/// so the blend is integer arithmetic — a weighted average of two bytes needs no float, and a float here
/// would only be a cast waiting to lose a sign.
const RECEDE_IN_HUNDREDTHS: u8 = 72;

/// A colour moved most of the way toward another.
///
/// A colour with no fixed value cannot be blended — a named ANSI colour is whatever the terminal says it is
/// — so those are left as they are. Dimming them would need to invent their value first, and a scrim is not
/// worth guessing for.
fn toward(colour: Color, behind: Color, palette: Palette) -> Color {
    let resolve = |value: Color| match value {
        Color::Reset => match palette.foreground {
            Color::Rgb(red, green, blue) => Some((red, green, blue)),
            _ => None,
        },
        Color::Rgb(red, green, blue) => Some((red, green, blue)),
        _ => None,
    };
    let (Some(from), Some(to)) = (resolve(colour), resolve(behind)) else {
        return colour;
    };
    // Integer arithmetic, so there is no float to cast back and no sign to lose: the blend is a weighted
    // average of two bytes, which `u32` holds exactly.
    let mix = |start: u8, end: u8| {
        let start = u32::from(start);
        let end = u32::from(end);
        let toward_end = u32::from(RECEDE_IN_HUNDREDTHS);
        let blended = (start * (100 - toward_end) + end * toward_end + 50) / 100;
        u8::try_from(blended.min(255)).unwrap_or(u8::MAX)
    };
    Color::Rgb(mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2))
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
    fn a_scrim_recedes_the_text_and_leaves_the_bands_that_say_where_things_are() {
        let palette = Mode::Dark.palette();
        let mut buffer = buffer(6, 2);
        let area = Rect::new(0, 0, 6, 2);
        buffer.set_style(
            area,
            Style::new().bg(palette.background).fg(palette.foreground),
        );
        buffer[(0, 0)].set_symbol("t");
        // A selected row: its band is how the page says where the cursor is.
        buffer[(0, 1)].set_symbol("s").set_bg(palette.selection);

        scrim(&mut buffer, area, palette);

        let receded = buffer[(0, 0)].fg;
        assert_ne!(
            receded, palette.foreground,
            "the text did not recede at all"
        );
        assert_ne!(
            receded, palette.background,
            "the text vanished instead of receding"
        );
        // Still readable as text, just not as the thing being read: closer to the surface than to where it
        // started, which is what makes the layer in front look in front.
        let distance = |from: Color, to: Color| match (from, to) {
            (Color::Rgb(one, two, three), Color::Rgb(four, five, six)) => {
                i32::from(one).abs_diff(i32::from(four))
                    + i32::from(two).abs_diff(i32::from(five))
                    + i32::from(three).abs_diff(i32::from(six))
            }
            _ => unreachable!("both palettes are true colour"),
        };
        assert!(
            distance(receded, palette.background) < distance(receded, palette.foreground),
            "the text is still nearer its own colour than the surface"
        );

        // The bands survive, or the page dissolves rather than recedes and dismissing reads as arriving
        // somewhere new.
        assert_eq!(
            buffer[(0, 1)].bg,
            palette.selection,
            "the selected row lost its band"
        );
        assert_eq!(buffer[(0, 0)].bg, palette.background);
    }

    #[test]
    fn a_scrim_leaves_a_colour_it_cannot_measure_alone() {
        // A named ANSI colour is whatever the terminal says it is, so blending it means inventing its value
        // first — and a scrim is not worth guessing for.
        let palette = Mode::Dark.palette();
        let mut buffer = buffer(2, 1);
        let area = Rect::new(0, 0, 2, 1);
        buffer.set_style(area, Style::new().bg(palette.background).fg(Color::Yellow));
        scrim(&mut buffer, area, palette);
        assert_eq!(buffer[(0, 0)].fg, Color::Yellow);
    }

    #[test]
    fn a_scrim_only_touches_what_it_was_given() {
        let palette = Mode::Dark.palette();
        let mut buffer = buffer(6, 3);
        let whole = buffer.area;
        buffer.set_style(
            whole,
            Style::new().bg(palette.background).fg(palette.foreground),
        );
        scrim(&mut buffer, Rect::new(0, 0, 6, 2), palette);
        assert_eq!(
            buffer[(0, 2)].fg,
            palette.foreground,
            "it dimmed a row outside the area it was handed"
        );
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
