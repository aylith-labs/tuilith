//! Assertions about a rendered frame: is it painted, is it readable, is it the right variant.
//!
//! Three failures that a type system cannot see and a screenshot only catches if somebody looks at the
//! right screenshot. Each is a pure function over a [`Buffer`], so a test can make the claim without a
//! terminal.
//!
//! What each one refuses to guess is as much the point as what it checks. [`Color::Reset`] has no colour
//! of its own — it is whatever the terminal's default happens to be — and a named ANSI colour has no
//! fixed value either, because the user's palette decides. Scoring those against a reference palette
//! would be measuring an assumption and reporting it as a fact, so [`readable`] resolves what is
//! resolvable and *counts* the rest.
//!
//! ```no_run
//! # use ratatui::{buffer::Buffer, layout::Rect};
//! # use tuilith::{Mode, inspect};
//! # fn check(buffer: &Buffer) {
//! let mode = Mode::Dark;
//! inspect::fully_painted(buffer, buffer.area).expect("an overlay left the terminal showing through");
//! inspect::no_leak(buffer, buffer.area, mode.inverse().palette()).expect("drawn in the wrong variant");
//! let verdict = inspect::readable(buffer, buffer.area, mode.palette(), 3.0);
//! assert!(verdict.failed.is_empty(), "{verdict}");
//! # }
//! ```

use std::fmt;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::theme::Palette;

crate::provenance! {
    component: "inspect",
    about: "Painted, readable and right-variant assertions over a rendered buffer",
    origin: crate::Origin::Here,
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// A cell that kept the terminal's own colour instead of the theme's.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Unpainted {
    /// Where it is, in buffer coordinates.
    pub at: (u16, u16),
    /// What the cell holds, so a report can say whether it is a glyph or a gap.
    pub symbol: String,
    /// Which half was left unset.
    pub half: Half,
}

/// Which of a cell's two colours was left at the terminal's default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Half {
    /// The surface. Always a defect on an overlay: it is the terminal showing through.
    Background,
    /// The glyph's colour. A defect wherever the cell actually draws something.
    Foreground,
}

impl fmt::Display for Unpainted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (column, row) = self.at;
        let half = match self.half {
            Half::Background => "background",
            Half::Foreground => "foreground",
        };
        write!(
            formatter,
            "the cell at ({column}, {row}) holding {:?} kept the terminal's {half}",
            self.symbol
        )
    }
}

/// Every cell in `area` carries a colour the theme chose.
///
/// The background is required everywhere: an unpainted cell on a floating surface *is* the page or the
/// terminal showing through. The foreground is required only where the cell draws something, because a
/// blank cell's glyph colour is not visible and demanding one would fail every legitimate gap.
///
/// # Errors
///
/// The first cell that kept a terminal default, so the message names one place to look rather than a
/// count.
pub fn fully_painted(buffer: &Buffer, area: Rect) -> Result<(), Unpainted> {
    for row in area.top()..area.bottom().min(buffer.area.bottom()) {
        for column in area.left()..area.right().min(buffer.area.right()) {
            let cell = &buffer[(column, row)];
            let unpainted = |half| {
                Err(Unpainted {
                    at: (column, row),
                    symbol: cell.symbol().to_owned(),
                    half,
                })
            };
            if cell.bg == Color::Reset {
                return unpainted(Half::Background);
            }
            if cell.fg == Color::Reset && !cell.symbol().trim().is_empty() {
                return unpainted(Half::Foreground);
            }
        }
    }
    Ok(())
}

/// A colour of the other variant, found in a frame that should not contain one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Leak {
    /// Where it is.
    pub at: (u16, u16),
    /// Which role of the other variant it is.
    pub role: &'static str,
}

impl fmt::Display for Leak {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (column, row) = self.at;
        write!(
            formatter,
            "the cell at ({column}, {row}) is drawn in the other variant's `{}`",
            self.role
        )
    }
}

/// No cell in `area` is drawn in a colour belonging to `other`.
///
/// The check for "did this frame draw in the wrong variant" — usually against
/// [`Mode::inverse`](crate::Mode::inverse)'s palette. It catches the common shape of that bug, which is
/// code reaching for a palette constant instead of the one it was handed.
///
/// Its limit, because it is worth knowing: it finds a colour that *is* one of `other`'s roles. A colour
/// belonging to neither palette is not a leak by this definition, and is [`readable`]'s business only if
/// it is also hard to read. This is sound only while the two palettes share no colour — a value in both
/// could not be attributed to either.
///
/// # Errors
///
/// The first cell drawn in the other variant.
pub fn no_leak(buffer: &Buffer, area: Rect, other: Palette) -> Result<(), Leak> {
    for row in area.top()..area.bottom().min(buffer.area.bottom()) {
        for column in area.left()..area.right().min(buffer.area.right()) {
            let cell = &buffer[(column, row)];
            for colour in [cell.fg, cell.bg] {
                if let Some(role) = other.role_of(colour) {
                    return Err(Leak {
                        at: (column, row),
                        role,
                    });
                }
            }
        }
    }
    Ok(())
}

/// A pair of colours that do not have enough contrast between them.
#[derive(Clone, PartialEq, Debug)]
pub struct Failure {
    /// Where it is.
    pub at: (u16, u16),
    /// What the cell draws.
    pub symbol: String,
    /// The palette role the glyph is in, when it is one.
    pub foreground: Option<&'static str>,
    /// The palette role the surface is in, when it is one.
    pub background: Option<&'static str>,
    /// Their WCAG contrast ratio, between 1.0 and 21.0.
    pub ratio: f64,
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (column, row) = self.at;
        let name = |role: Option<&str>| role.unwrap_or("a colour outside the palette").to_owned();
        write!(
            formatter,
            "{:?} at ({column}, {row}): {} on {} is {:.2}",
            self.symbol,
            name(self.foreground),
            name(self.background),
            self.ratio,
        )
    }
}

/// What [`readable`] found.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Verdict {
    /// Pairs below the floor, worst first.
    pub failed: Vec<Failure>,
    /// Cells skipped because a colour has no fixed value to measure — a named ANSI colour or one of the
    /// low sixteen, both of which the user's terminal defines.
    ///
    /// A palette built from true colour makes this zero, so a non-zero count is a live signal that a
    /// named colour has appeared somewhere.
    pub unresolved: usize,
    /// Cells that drew nothing, so their glyph colour was not judged.
    pub blank: usize,
}

impl fmt::Display for Verdict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "{} pair(s) below the floor, {} unmeasurable, {} blank",
            self.failed.len(),
            self.unresolved,
            self.blank
        )?;
        for failure in &self.failed {
            writeln!(formatter, "  {failure}")?;
        }
        Ok(())
    }
}

/// Every glyph in `area` clears `floor` against the surface behind it.
///
/// `Color::Reset` is substituted with the palette's own background or foreground, which is only correct
/// because an application paints its canvas explicitly — [`fully_painted`] is what tests that, so this
/// check is meaningful in the places that one passes.
///
/// Ratios are WCAG relative-luminance contrast, so 1.0 is invisible and 21.0 is black on white. A useful
/// floor is lower than the 4.5 the web guidelines ask for body text: a terminal palette's border and
/// selection roles are deliberately close to their background, because a rule that shouts is worse than
/// one that is merely visible.
#[must_use]
pub fn readable(buffer: &Buffer, area: Rect, palette: Palette, floor: f64) -> Verdict {
    let mut verdict = Verdict::default();
    for row in area.top()..area.bottom().min(buffer.area.bottom()) {
        for column in area.left()..area.right().min(buffer.area.right()) {
            let cell = &buffer[(column, row)];
            if cell.symbol().trim().is_empty() {
                verdict.blank += 1;
                continue;
            }
            let foreground = resolve(cell.fg, palette.foreground);
            let background = resolve(cell.bg, palette.background);
            let (Some(foreground), Some(background)) = (foreground, background) else {
                verdict.unresolved += 1;
                continue;
            };
            let ratio = contrast(foreground, background);
            if ratio < floor {
                verdict.failed.push(Failure {
                    at: (column, row),
                    symbol: cell.symbol().to_owned(),
                    foreground: palette.role_of(cell.fg),
                    background: palette.role_of(cell.bg),
                    ratio,
                });
            }
        }
    }
    verdict
        .failed
        .sort_by(|left, right| left.ratio.total_cmp(&right.ratio));
    verdict
}

/// A colour's relative luminance, or `None` where it has no fixed value.
///
/// `Reset` takes the substitute the caller supplies. A named colour and the low sixteen indices take
/// nothing: the terminal's own configuration decides what they are, so `Color::Black` can legitimately
/// be white and any number this returned for it would be invented.
fn resolve(colour: Color, substitute: Color) -> Option<f64> {
    match colour {
        Color::Reset => resolve(substitute, Color::Reset),
        Color::Rgb(red, green, blue) => Some(luminance(red, green, blue)),
        // The 6×6×6 cube and the grey ramp are fixed by the xterm specification, unlike the sixteen
        // below them, so these can be measured.
        Color::Indexed(index) if index >= 16 => {
            let (red, green, blue) = from_xterm(index);
            Some(luminance(red, green, blue))
        }
        _ => None,
    }
}

/// An xterm palette index above 15, as the colour the specification fixes it to.
fn from_xterm(index: u8) -> (u8, u8, u8) {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    if index >= 232 {
        let grey = 8 + (u16::from(index) - 232) * 10;
        let grey = u8::try_from(grey).unwrap_or(u8::MAX);
        return (grey, grey, grey);
    }
    let cube = usize::from(index) - 16;
    (LEVELS[cube / 36], LEVELS[(cube % 36) / 6], LEVELS[cube % 6])
}

/// WCAG relative luminance.
fn luminance(red: u8, green: u8, blue: u8) -> f64 {
    fn channel(value: u8) -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue)
}

/// The WCAG contrast ratio between two luminances.
fn contrast(one: f64, other: f64) -> f64 {
    let (lighter, darker) = if one > other {
        (one, other)
    } else {
        (other, one)
    };
    (lighter + 0.05) / (darker + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::Overlay;
    use crate::theme::{Mode, DEFAULT_DARK, DEFAULT_LIGHT};

    fn buffer(width: u16, height: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, width, height))
    }

    #[test]
    fn a_cleared_surface_that_nobody_repainted_is_caught_and_located() {
        // The defect this module exists for, in its smallest form.
        let mut buffer = buffer(6, 3);
        let found = fully_painted(&buffer, buffer.area).expect_err("an empty buffer is unpainted");
        assert_eq!(found.at, (0, 0));
        assert_eq!(found.half, Half::Background);

        let area = Rect::new(0, 0, 6, 3);
        let _ = Overlay::new(Mode::Dark.palette()).draw_on(&mut buffer, area);
        fully_painted(&buffer, area).expect("a drawn overlay is painted");
    }

    #[test]
    fn one_unpainted_column_is_still_caught() {
        // Proves it is not a spot check: the overlay covers everything but the last column.
        let mut buffer = buffer(8, 3);
        let _ = Overlay::new(Mode::Dark.palette()).draw_on(&mut buffer, Rect::new(0, 0, 7, 3));
        let found = fully_painted(&buffer, buffer.area).expect_err("the last column is unpainted");
        assert_eq!(found.at, (7, 0), "it found the wrong cell");
    }

    #[test]
    fn a_blank_cell_may_keep_the_terminals_foreground_but_a_glyph_may_not() {
        let mut buffer = buffer(3, 1);
        buffer.set_style(
            buffer.area,
            ratatui::style::Style::new().bg(Color::Rgb(0, 0, 0)),
        );
        fully_painted(&buffer, buffer.area).expect("blank cells need no glyph colour");

        buffer[(1, 0)].set_symbol("x");
        let found = fully_painted(&buffer, buffer.area).expect_err("a glyph needs a colour");
        assert_eq!(found.at, (1, 0));
        assert_eq!(found.half, Half::Foreground);
    }

    #[test]
    fn a_frame_holding_the_other_variants_colour_is_caught_and_named() {
        let mut buffer = buffer(4, 2);
        let area = buffer.area;
        let _ = Overlay::new(DEFAULT_DARK).draw_on(&mut buffer, area);
        no_leak(&buffer, area, DEFAULT_LIGHT).expect("a dark frame holds no light colour");

        // The real bug shape: one draw call reaching for a constant instead of the palette it was given.
        buffer[(2, 1)].set_fg(DEFAULT_LIGHT.accent);
        let leak =
            no_leak(&buffer, buffer.area, DEFAULT_LIGHT).expect_err("the light accent is a leak");
        assert_eq!(leak.at, (2, 1));
        assert_eq!(leak.role, "accent");
    }

    #[test]
    fn an_unreadable_pair_is_reported_with_its_roles_and_its_ratio() {
        let palette = DEFAULT_DARK;
        let mut buffer = buffer(2, 1);
        buffer.set_style(
            buffer.area,
            ratatui::style::Style::new()
                .bg(palette.selection)
                .fg(palette.border),
        );
        buffer[(0, 0)].set_symbol("x");

        let verdict = readable(&buffer, buffer.area, palette, 3.0);
        assert_eq!(verdict.failed.len(), 1, "{verdict}");
        assert_eq!(verdict.failed[0].foreground, Some("border"));
        assert_eq!(verdict.failed[0].background, Some("selection"));
        assert!(verdict.failed[0].ratio < 2.0, "{verdict}");
        // The blank cell beside it was not judged, because its glyph colour is not visible.
        assert_eq!(verdict.blank, 1);
    }

    #[test]
    fn a_named_colour_is_refused_rather_than_assumed() {
        // `Color::Yellow` is whatever the user's terminal says it is, so any ratio computed for it would
        // be invented. It must land in `unresolved`, never in `failed` and never silently in neither.
        let mut buffer = buffer(1, 1);
        buffer.set_style(
            buffer.area,
            ratatui::style::Style::new()
                .bg(Color::Yellow)
                .fg(Color::White),
        );
        buffer[(0, 0)].set_symbol("x");

        let verdict = readable(&buffer, buffer.area, DEFAULT_DARK, 21.0);
        assert_eq!(verdict.unresolved, 1);
        assert!(verdict.failed.is_empty(), "a guess was reported as a fact");
    }

    #[test]
    fn an_indexed_colour_the_specification_fixes_is_measured() {
        // Index 16 is pure black and 231 pure white, so the two of them are the highest contrast the
        // cube can produce and must resolve rather than land in `unresolved`.
        assert_eq!(from_xterm(16), (0, 0, 0));
        assert_eq!(from_xterm(231), (255, 255, 255));
        assert_eq!(from_xterm(232), (8, 8, 8));
        assert_eq!(from_xterm(255), (238, 238, 238));

        let mut buffer = buffer(1, 1);
        buffer.set_style(
            buffer.area,
            ratatui::style::Style::new()
                .bg(Color::Indexed(231))
                .fg(Color::Indexed(16)),
        );
        buffer[(0, 0)].set_symbol("x");
        let verdict = readable(&buffer, buffer.area, DEFAULT_DARK, 3.0);
        assert_eq!(verdict.unresolved, 0, "{verdict}");
        assert!(verdict.failed.is_empty(), "{verdict}");
    }

    #[test]
    fn reset_is_measured_as_the_palette_the_application_painted() {
        // Sound only because an application paints its canvas; `fully_painted` is what tests that claim.
        let mut buffer = buffer(1, 1);
        buffer[(0, 0)].set_symbol("x");
        let verdict = readable(&buffer, buffer.area, DEFAULT_DARK, 3.0);
        assert_eq!(verdict.unresolved, 0);
        // Reset on Reset becomes foreground on background, which is the palette's most readable pair.
        assert!(verdict.failed.is_empty(), "{verdict}");
    }

    #[test]
    fn the_contrast_maths_agrees_with_the_published_extremes() {
        let black = luminance(0, 0, 0);
        let white = luminance(255, 255, 255);
        assert!((contrast(black, white) - 21.0).abs() < 0.01);
        assert!((contrast(white, white) - 1.0).abs() < 0.001);
    }

    #[test]
    fn both_palettes_keep_their_text_readable_against_their_own_surfaces() {
        // The measured floor, recorded so a palette edit that drops below it fails here rather than in
        // somebody's terminal. `foreground` clears 15, and the tightest role is `accent` on dark at 4.09.
        for palette in [DEFAULT_DARK, DEFAULT_LIGHT] {
            let text = contrast(
                resolve(palette.foreground, palette.background).unwrap(),
                resolve(palette.background, palette.background).unwrap(),
            );
            assert!(text > 7.0, "ordinary text is only {text:.2}");
            for role in [
                palette.dim,
                palette.accent,
                palette.ok,
                palette.warn,
                palette.bad,
            ] {
                let ratio = contrast(
                    resolve(role, palette.background).unwrap(),
                    resolve(palette.background, palette.background).unwrap(),
                );
                assert!(
                    ratio > 4.0,
                    "a role reads at {ratio:.2} against its own background"
                );
            }
        }
    }
}
