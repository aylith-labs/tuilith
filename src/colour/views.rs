//! The views tuilith ships. Each is a [`View`]; a host composes them with [`Picker::with`].
//!
//! [`Picker::with`]: super::Picker::with

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::{
    Choice, Config, Context, Depth, Hsl, Hsv, Keys, Outcome, Rgb, Size, Space, State, View,
    contains, format, hex, hint,
};
use crate::pick;

// ---------------------------------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------------------------------

/// The upper half block: its foreground is the cell's top point, its background the bottom one.
const HALF: &str = "▀";

/// Black or white, whichever reads over `rgb`.
fn contrast(rgb: Rgb) -> Color {
    if rgb.luminance() > 0.35 {
        Color::Black
    } else {
        Color::White
    }
}

fn cell(buffer: &mut Buffer, x: u16, y: u16) -> Option<&mut ratatui::buffer::Cell> {
    buffer.cell_mut((x, y))
}

/// Paint one cell as two points.
fn points(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    top: Option<Rgb>,
    bottom: Option<Rgb>,
    depth: Depth,
) {
    let Some(target) = cell(buffer, x, y) else {
        return;
    };
    match (top, bottom) {
        (None, None) => {}
        (Some(top), bottom) => {
            target.set_symbol(HALF);
            target.set_fg(depth.color(top));
            target.set_bg(bottom.map_or(Color::Reset, |b| depth.color(b)));
        }
        (None, Some(bottom)) => {
            target.set_symbol("▄");
            target.set_fg(depth.color(bottom));
            target.set_bg(Color::Reset);
        }
    }
}

/// Put the marker in a cell drawn as `under`, readable over it.
fn mark(buffer: &mut Buffer, x: u16, y: u16, marker: char, under: Rgb, depth: Depth) {
    if let Some(target) = cell(buffer, x, y) {
        let mut text = [0u8; 4];
        target.set_symbol(marker.encode_utf8(&mut text));
        target.set_fg(contrast(depth.shown(under)));
        target.set_bg(depth.color(under));
    }
}

fn text(buffer: &mut Buffer, x: u16, y: u16, width: u16, content: &str, style: Style) -> u16 {
    if width == 0 {
        return x;
    }
    buffer.set_stringn(x, y, content, width as usize, style).0
}

/// A swatch: every cell it covers carries the colour, then the glyph is written over them. The cell
/// after it, while still inside `bound`, takes the colour as its ink, so a glyph a font draws wider
/// than it measures overflows in its own colour rather than in whatever was written there.
fn swatch(buffer: &mut Buffer, rect: Rect, bound: Rect, glyph: &str, colour: Color) {
    let style = Style::default().fg(colour).bg(colour);
    buffer.set_style(rect, style);
    if rect.width > 0 {
        buffer.set_stringn(rect.x, rect.y, glyph, rect.width as usize, style);
    }
    if contains(bound, rect.right(), rect.y)
        && let Some(tail) = cell(buffer, rect.right(), rect.y)
    {
        tail.set_fg(colour);
    }
}

fn glyph_width(config: &Config) -> u16 {
    (Span::raw(config.swatch.as_str()).width() as u16).max(1)
}

/// A title on a view's top row, when titles are on and the view has room for one.
fn titled(area: Rect, buffer: &mut Buffer, title: &str, context: &Context) -> Rect {
    if !context.config.titles || area.height < 3 {
        return area;
    }
    let style = if context.focused {
        Style::default()
            .fg(context.palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(context.palette.dim)
    };
    text(buffer, area.x, area.y, area.width, title, style);
    Rect::new(area.x, area.y + 1, area.width, area.height - 1)
}

fn body(area: Rect, config: &Config) -> Rect {
    if !config.titles || area.height < 3 {
        return area;
    }
    Rect::new(area.x, area.y + 1, area.width, area.height - 1)
}

fn fraction(position: u16, span: u16) -> f32 {
    if span <= 1 {
        0.0
    } else {
        f32::from(position) / f32::from(span - 1)
    }
}

fn shift(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::SHIFT)
}

// ---------------------------------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------------------------------

/// The shape of a two-dimensional field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Hue left to right, saturation top to bottom, at the current value.
    Square,
    /// Hue around, saturation outward from the centre, at the current value.
    Wheel,
    /// The HSL plane through the current hue: lightness top to bottom, saturation outward from the
    /// grey axis; right of the axis is the hue, left of it the opposite hue.
    Diamond,
    /// Hue across, lightness down, at full saturation: a grid of distinct colours.
    Grid,
}

/// A field of colour points, two to a cell, that a press or a drag picks from.
#[derive(Clone, Debug)]
pub struct Field {
    /// Which field.
    pub shape: Shape,
}

impl Field {
    /// The square field.
    #[must_use]
    pub fn square() -> Self {
        Self {
            shape: Shape::Square,
        }
    }

    /// The round field.
    #[must_use]
    pub fn wheel() -> Self {
        Self {
            shape: Shape::Wheel,
        }
    }

    /// The HSL diamond.
    #[must_use]
    pub fn diamond() -> Self {
        Self {
            shape: Shape::Diamond,
        }
    }

    /// The hue-by-lightness grid.
    #[must_use]
    pub fn grid() -> Self {
        Self { shape: Shape::Grid }
    }

    /// The colour at point `(px, py)` of a field `w` points wide and `h` tall, if the point is in it.
    #[must_use]
    pub fn point(&self, px: u16, py: u16, w: u16, h: u16, hsv: Hsv) -> Option<Rgb> {
        match self.shape {
            Shape::Square => Some(
                Hsv {
                    h: fraction(px, w) * 359.9,
                    s: 1.0 - fraction(py, h),
                    v: hsv.v,
                }
                .rgb(),
            ),
            Shape::Wheel => {
                let (dx, dy, radius) = wheel(px, py, w, h);
                let distance = dx.hypot(dy);
                (distance <= radius && radius > 0.0).then(|| {
                    Hsv {
                        h: (-dy).atan2(dx).to_degrees().rem_euclid(360.0),
                        s: (distance / radius).min(1.0),
                        v: hsv.v,
                    }
                    .rgb()
                })
            }
            Shape::Diamond => {
                let l = 1.0 - fraction(py, h);
                let half = f32::from(w.saturating_sub(1)) / 2.0;
                let dx = f32::from(px) - half;
                let room = 1.0 - (2.0 * l - 1.0).abs();
                let reach = room * half;
                if dx.abs() > reach + 0.5 || half <= 0.0 {
                    return None;
                }
                let s = if reach <= 0.0 {
                    0.0
                } else {
                    (dx.abs() / reach).min(1.0)
                };
                let h = if dx < 0.0 { hsv.h + 180.0 } else { hsv.h };
                Some(Hsl { h, s, l }.rgb())
            }
            Shape::Grid => {
                let columns = w.max(1);
                let h_deg = f32::from(px.min(columns - 1)) / f32::from(columns) * 360.0;
                let l = 1.0 - (f32::from(py) + 1.0) / (f32::from(h) + 1.0);
                Some(
                    Hsl {
                        h: h_deg,
                        s: 1.0,
                        l,
                    }
                    .rgb(),
                )
            }
        }
    }

    /// The point the colour sits at, if the field can show it.
    fn marker(&self, w: u16, h: u16, state: &State) -> (u16, u16) {
        let hsv = state.hsv();
        let scale = |t: f32, span: u16| {
            (t.clamp(0.0, 1.0) * f32::from(span.saturating_sub(1))).round() as u16
        };
        match self.shape {
            Shape::Square => (scale(hsv.h / 359.9, w), scale(1.0 - hsv.s, h)),
            Shape::Wheel => {
                let (_, _, radius) = wheel(0, 0, w, h);
                let cx = f32::from(w.saturating_sub(1)) / 2.0;
                let cy = f32::from(h.saturating_sub(1)) / 2.0;
                let angle = hsv.h.to_radians();
                let x = cx + hsv.s * radius * angle.cos();
                let y = cy - hsv.s * radius * angle.sin();
                (x.round().max(0.0) as u16, y.round().max(0.0) as u16)
            }
            Shape::Diamond => {
                let hsl = state.rgb().hsl();
                let half = f32::from(w.saturating_sub(1)) / 2.0;
                let room = 1.0 - (2.0 * hsl.l - 1.0).abs();
                let x = half + hsl.s * room * half;
                (x.round().max(0.0) as u16, scale(1.0 - hsl.l, h))
            }
            Shape::Grid => {
                let hsl = state.rgb().hsl();
                let column = ((hsv.h / 360.0) * f32::from(w)).floor() as u16;
                let row = ((1.0 - hsl.l) * (f32::from(h) + 1.0) - 1.0)
                    .round()
                    .max(0.0) as u16;
                (
                    column.min(w.saturating_sub(1)),
                    row.min(h.saturating_sub(1)),
                )
            }
        }
    }
}

fn wheel(px: u16, py: u16, w: u16, h: u16) -> (f32, f32, f32) {
    let cx = f32::from(w.saturating_sub(1)) / 2.0;
    let cy = f32::from(h.saturating_sub(1)) / 2.0;
    let radius = f32::from(w.min(h)) / 2.0 - 0.5;
    (f32::from(px) - cx, f32::from(py) - cy, radius)
}

impl View for Field {
    fn title(&self) -> &str {
        match self.shape {
            Shape::Square => "field",
            Shape::Wheel => "wheel",
            Shape::Diamond => "diamond",
            Shape::Grid => "grid",
        }
    }

    fn size(&self, _state: &State, _config: &Config) -> Size {
        Size {
            width: 32,
            height: 12,
        }
    }

    fn draw(&self, area: Rect, buffer: &mut Buffer, state: &State, context: &Context) {
        let area = titled(area, buffer, self.title(), context);
        let depth = context.config.depth;
        let (w, h) = (area.width, area.height * 2);
        let hsv = state.hsv();
        for row in 0..area.height {
            for column in 0..area.width {
                let top = self.point(column, row * 2, w, h, hsv);
                let bottom = self.point(column, row * 2 + 1, w, h, hsv);
                points(buffer, area.x + column, area.y + row, top, bottom, depth);
            }
        }
        let (px, py) = self.marker(w, h, state);
        if px < w && py < h {
            mark(
                buffer,
                area.x + px,
                area.y + py / 2,
                context.config.marker,
                state.rgb(),
                depth,
            );
        }
    }

    fn at(&self, area: Rect, column: u16, row: u16, state: &State, config: &Config) -> Option<Rgb> {
        let area = body(area, config);
        if !contains(area, column, row) {
            return None;
        }
        // A press lands on a cell, which holds two points; the top one is taken, so the colour picked
        // is one the cell shows.
        self.point(
            column - area.x,
            (row - area.y) * 2,
            area.width,
            area.height * 2,
            state.hsv(),
        )
    }

    fn press(
        &mut self,
        area: Rect,
        column: u16,
        row: u16,
        state: &mut State,
        config: &Config,
    ) -> bool {
        let inner = body(area, config);
        if !contains(inner, column, row) {
            return false;
        }
        let (w, h) = (inner.width, inner.height * 2);
        let (px, py) = (column - inner.x, (row - inner.y) * 2);
        let hsv = state.hsv();
        // Set the coordinates rather than the colour where the field has them, so a press on the
        // grey edge keeps the hue a drag back out will need.
        match self.shape {
            Shape::Square => state.set_hsv(Hsv {
                h: fraction(px, w) * 359.9,
                s: 1.0 - fraction(py, h),
                v: hsv.v,
            }),
            Shape::Wheel => {
                let (dx, dy, radius) = wheel(px, py, w, h);
                let distance = dx.hypot(dy);
                if distance > radius || radius <= 0.0 {
                    return false;
                }
                state.set_hsv(Hsv {
                    h: (-dy).atan2(dx).to_degrees().rem_euclid(360.0),
                    s: (distance / radius).min(1.0),
                    v: hsv.v,
                });
            }
            Shape::Diamond | Shape::Grid => match self.point(px, py, w, h, hsv) {
                Some(rgb) => {
                    let keep = rgb.hsv();
                    state.set_hsv(Hsv {
                        h: if keep.s <= f32::EPSILON {
                            hsv.h
                        } else {
                            keep.h
                        },
                        ..keep
                    });
                }
                None => return false,
            },
        }
        true
    }

    fn key(&mut self, key: KeyEvent, state: &mut State, config: &Config) -> Outcome {
        let mut hsv = state.hsv();
        let step = if shift(key) {
            config.step * 4.0
        } else {
            config.step
        };
        let hue = if shift(key) {
            config.hue_step * 4.0
        } else {
            config.hue_step
        };
        match (self.shape, key.code) {
            (Shape::Diamond, KeyCode::Up | KeyCode::Down) => {
                let mut hsl = state.rgb().hsl();
                hsl.h = hsv.h;
                hsl.l += if key.code == KeyCode::Up { step } else { -step };
                hsl.l = hsl.l.clamp(0.0, 1.0);
                state.set_hsv(hsl.hsv());
                return Outcome::Changed;
            }
            (Shape::Diamond, KeyCode::Left | KeyCode::Right) => {
                let mut hsl = state.rgb().hsl();
                hsl.h = hsv.h;
                hsl.s += if key.code == KeyCode::Right {
                    step
                } else {
                    -step
                };
                hsl.s = hsl.s.clamp(0.0, 1.0);
                state.set_hsv(hsl.hsv());
                return Outcome::Changed;
            }
            (Shape::Grid, KeyCode::Up | KeyCode::Down) => {
                let mut hsl = state.rgb().hsl();
                hsl.h = hsv.h;
                hsl.s = 1.0;
                hsl.l += if key.code == KeyCode::Up { step } else { -step };
                hsl.l = hsl.l.clamp(0.0, 1.0);
                state.set_hsv(hsl.hsv());
                return Outcome::Changed;
            }
            (_, KeyCode::Left) => hsv.h -= hue,
            (_, KeyCode::Right) => hsv.h += hue,
            (Shape::Wheel | Shape::Square, KeyCode::Up) => hsv.s += step,
            (Shape::Wheel | Shape::Square, KeyCode::Down) => hsv.s -= step,
            (_, KeyCode::PageUp) => hsv.v += step,
            (_, KeyCode::PageDown) => hsv.v -= step,
            _ => return Outcome::Ignored,
        }
        state.set_hsv(hsv);
        Outcome::Changed
    }
}

// ---------------------------------------------------------------------------------------------------
// Strips
// ---------------------------------------------------------------------------------------------------

/// Which channel a strip runs through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// HSV value: black to the colour.
    Value,
    /// Hue, all the way round, at the colour's saturation and value.
    Hue,
    /// HSV saturation: grey to the colour.
    Saturation,
    /// HSL lightness: black, the colour, white.
    Lightness,
}

/// Which way a strip runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// Top is the most; beside a field.
    Vertical,
    /// Right is the most; under a field.
    Horizontal,
}

/// One channel, drawn as a bar of the colours it runs through.
#[derive(Clone, Debug)]
pub struct Strip {
    /// Which channel.
    pub channel: Channel,
    /// Which way.
    pub orientation: Orientation,
}

impl Strip {
    /// A strip that runs top to bottom.
    #[must_use]
    pub fn vertical(channel: Channel) -> Self {
        Self {
            channel,
            orientation: Orientation::Vertical,
        }
    }

    /// A strip that runs left to right.
    #[must_use]
    pub fn horizontal(channel: Channel) -> Self {
        Self {
            channel,
            orientation: Orientation::Horizontal,
        }
    }

    fn colour(&self, t: f32, state: &State) -> Rgb {
        let hsv = state.hsv();
        match self.channel {
            Channel::Value => Hsv { v: t, ..hsv }.rgb(),
            Channel::Hue => Hsv {
                h: t * 359.9,
                ..hsv
            }
            .rgb(),
            Channel::Saturation => Hsv { s: t, ..hsv }.rgb(),
            Channel::Lightness => {
                let hsl = state.rgb().hsl();
                Hsl {
                    h: hsv.h,
                    s: hsl.s,
                    l: t,
                }
                .rgb()
            }
        }
    }

    fn value(&self, state: &State) -> f32 {
        let hsv = state.hsv();
        match self.channel {
            Channel::Value => hsv.v,
            Channel::Hue => hsv.h / 359.9,
            Channel::Saturation => hsv.s,
            Channel::Lightness => state.rgb().hsl().l,
        }
    }

    fn set(&self, t: f32, state: &mut State) {
        let t = t.clamp(0.0, 1.0);
        let hsv = state.hsv();
        match self.channel {
            Channel::Value => state.set_hsv(Hsv { v: t, ..hsv }),
            Channel::Hue => state.set_hsv(Hsv {
                h: t * 359.9,
                ..hsv
            }),
            Channel::Saturation => state.set_hsv(Hsv { s: t, ..hsv }),
            Channel::Lightness => {
                let mut hsl = state.rgb().hsl();
                hsl.h = hsv.h;
                hsl.l = t;
                state.set_hsv(hsl.hsv());
            }
        }
    }

    /// The fraction along the strip of `(column, row)`, top or right being 1.
    fn along(&self, area: Rect, column: u16, row: u16) -> Option<f32> {
        if !contains(area, column, row) {
            return None;
        }
        Some(match self.orientation {
            Orientation::Vertical => 1.0 - fraction((row - area.y) * 2, area.height * 2),
            Orientation::Horizontal => fraction(column - area.x, area.width),
        })
    }
}

impl View for Strip {
    fn title(&self) -> &str {
        match self.channel {
            Channel::Value => "V",
            Channel::Hue => "H",
            Channel::Saturation => "S",
            Channel::Lightness => "L",
        }
    }

    fn size(&self, _state: &State, config: &Config) -> Size {
        match self.orientation {
            Orientation::Vertical => Size {
                width: config.strip_size,
                height: 12,
            },
            Orientation::Horizontal => Size {
                width: 32,
                height: config.strip_size.clamp(1, 2),
            },
        }
    }

    fn draw(&self, area: Rect, buffer: &mut Buffer, state: &State, context: &Context) {
        let area = match self.orientation {
            Orientation::Vertical => titled(area, buffer, self.title(), context),
            Orientation::Horizontal => area,
        };
        let depth = context.config.depth;
        let value = self.value(state);
        match self.orientation {
            Orientation::Vertical => {
                let span = area.height * 2;
                for row in 0..area.height {
                    let top = self.colour(1.0 - fraction(row * 2, span), state);
                    let bottom = self.colour(1.0 - fraction(row * 2 + 1, span), state);
                    for column in 0..area.width {
                        points(
                            buffer,
                            area.x + column,
                            area.y + row,
                            Some(top),
                            Some(bottom),
                            depth,
                        );
                    }
                }
                let at = ((1.0 - value) * f32::from(span.saturating_sub(1))).round() as u16 / 2;
                if at < area.height {
                    let under = self.colour(value, state);
                    for column in 0..area.width {
                        mark(buffer, area.x + column, area.y + at, '━', under, depth);
                    }
                }
            }
            Orientation::Horizontal => {
                for column in 0..area.width {
                    let colour = self.colour(fraction(column, area.width), state);
                    for row in 0..area.height {
                        points(
                            buffer,
                            area.x + column,
                            area.y + row,
                            Some(colour),
                            Some(colour),
                            depth,
                        );
                    }
                }
                let at = (value * f32::from(area.width.saturating_sub(1))).round() as u16;
                if at < area.width {
                    for row in 0..area.height {
                        mark(
                            buffer,
                            area.x + at,
                            area.y + row,
                            '┃',
                            self.colour(value, state),
                            depth,
                        );
                    }
                }
            }
        }
    }

    fn at(&self, area: Rect, column: u16, row: u16, state: &State, config: &Config) -> Option<Rgb> {
        let area = match self.orientation {
            Orientation::Vertical => body(area, config),
            Orientation::Horizontal => area,
        };
        self.along(area, column, row).map(|t| self.colour(t, state))
    }

    fn press(
        &mut self,
        area: Rect,
        column: u16,
        row: u16,
        state: &mut State,
        config: &Config,
    ) -> bool {
        let area = match self.orientation {
            Orientation::Vertical => body(area, config),
            Orientation::Horizontal => area,
        };
        match self.along(area, column, row) {
            Some(t) => {
                self.set(t, state);
                true
            }
            None => false,
        }
    }

    fn key(&mut self, key: KeyEvent, state: &mut State, config: &Config) -> Outcome {
        let step = if shift(key) {
            config.step * 4.0
        } else {
            config.step
        };
        let (more, less) = match self.orientation {
            Orientation::Vertical => (KeyCode::Up, KeyCode::Down),
            Orientation::Horizontal => (KeyCode::Right, KeyCode::Left),
        };
        let value = self.value(state);
        if key.code == more {
            self.set(value + step, state);
        } else if key.code == less {
            self.set(value - step, state);
        } else {
            return Outcome::Ignored;
        }
        Outcome::Changed
    }
}

// ---------------------------------------------------------------------------------------------------
// Sliders
// ---------------------------------------------------------------------------------------------------

/// One bar per channel, in the state's space, usable alone as the whole picker.
#[derive(Clone, Debug, Default)]
pub struct Sliders {
    row: usize,
}

impl Sliders {
    /// The rows the three bars sit on, spaced apart when there is room.
    fn rows(area: Rect) -> [u16; 3] {
        let gap = if area.height >= 5 { 2 } else { 1 };
        [area.y, area.y + gap, area.y + 2 * gap]
    }

    /// The bar of channel `index`: its rect within `area`.
    fn bar(area: Rect, index: usize) -> Rect {
        let y = Self::rows(area)[index];
        let x = area.x + 2;
        let width = area.width.saturating_sub(2 + 5);
        Rect::new(x, y, width, 1)
    }

    fn colour(state: &State, space: Space, index: usize, t: f32) -> Rgb {
        let mut probe = state.clone();
        probe.set_channel(
            space,
            index,
            (t * f32::from(space.max(index))).round() as u16,
        );
        probe.rgb()
    }

    fn hit(area: Rect, column: u16, row: u16) -> Option<(usize, f32)> {
        (0..3).find_map(|index| {
            let bar = Self::bar(area, index);
            contains(bar, column, row).then(|| (index, fraction(column - bar.x, bar.width)))
        })
    }
}

impl View for Sliders {
    fn title(&self) -> &'static str {
        "channels"
    }

    fn size(&self, _state: &State, config: &Config) -> Size {
        Size {
            width: 32,
            height: 5 + u16::from(config.titles),
        }
    }

    fn draw(&self, area: Rect, buffer: &mut Buffer, state: &State, context: &Context) {
        let title = format!("channels · {}", state.space.label());
        let area = titled(area, buffer, &title, context);
        let space = state.space;
        let values = state.channels(space);
        let depth = context.config.depth;
        for (index, value) in values.into_iter().enumerate() {
            let y = Self::rows(area)[index];
            if y >= area.bottom() {
                break;
            }
            let selected = context.focused && index == self.row;
            let label = Style::default().fg(if selected {
                context.palette.accent
            } else {
                context.palette.dim
            });
            text(buffer, area.x, y, 2, space.channels()[index], label);
            let bar = Self::bar(area, index);
            let max = space.max(index);
            let at = if bar.width == 0 {
                0
            } else {
                ((f32::from(value) / f32::from(max)) * f32::from(bar.width - 1)).round() as u16
            };
            for column in 0..bar.width {
                let colour = Self::colour(state, space, index, fraction(column, bar.width));
                points(buffer, bar.x + column, y, Some(colour), Some(colour), depth);
            }
            if bar.width > 0 {
                let under = Self::colour(state, space, index, fraction(at, bar.width));
                mark(buffer, bar.x + at, y, '┃', under, depth);
            }
            let number = format!("{value:>4}");
            text(
                buffer,
                bar.right() + 1,
                y,
                area.right().saturating_sub(bar.right() + 1),
                &number,
                Style::default().fg(context.palette.foreground),
            );
        }
    }

    fn at(&self, area: Rect, column: u16, row: u16, state: &State, config: &Config) -> Option<Rgb> {
        let (index, t) = Self::hit(body(area, config), column, row)?;
        Some(Self::colour(state, state.space, index, t))
    }

    fn press(
        &mut self,
        area: Rect,
        column: u16,
        row: u16,
        state: &mut State,
        config: &Config,
    ) -> bool {
        let Some((index, t)) = Self::hit(body(area, config), column, row) else {
            return false;
        };
        self.row = index;
        let space = state.space;
        state.set_channel(
            space,
            index,
            (t * f32::from(space.max(index))).round() as u16,
        );
        true
    }

    fn key(&mut self, key: KeyEvent, state: &mut State, _config: &Config) -> Outcome {
        let space = state.space;
        let step = if shift(key) { 10 } else { 1 };
        let value = state.channels(space)[self.row];
        match key.code {
            KeyCode::Up => self.row = self.row.saturating_sub(1),
            KeyCode::Down => self.row = (self.row + 1).min(2),
            KeyCode::Left => {
                state.set_channel(space, self.row, value.saturating_sub(step));
                return Outcome::Changed;
            }
            KeyCode::Right => {
                state.set_channel(space, self.row, value + step);
                return Outcome::Changed;
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Handled
    }
}

// ---------------------------------------------------------------------------------------------------
// Swatch rows
// ---------------------------------------------------------------------------------------------------

/// Which colours a swatch view shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Rows of fixed colours: [`Config::common`] by default.
    Rows(Vec<Vec<Rgb>>),
    /// [`State::recent`], newest first, shown once [`Config::recent_minimum`] are kept.
    Recent,
    /// [`State::entries`] that have a colour; a press chooses the entry by name.
    Named,
}

/// Rows of swatches. A press picks one; arrows walk them.
#[derive(Clone, Debug)]
pub struct Swatches {
    /// What it shows.
    pub source: Source,
    title: String,
    cursor: usize,
}

/// One swatch's place and what it picks.
struct Placed {
    rect: Rect,
    rgb: Rgb,
    named: Option<usize>,
}

impl Swatches {
    /// The common-colours rows, from the config.
    #[must_use]
    pub fn common(config: &Config) -> Self {
        Self {
            source: Source::Rows(config.common.clone()),
            title: "common".into(),
            cursor: 0,
        }
    }

    /// The recent-colours row.
    #[must_use]
    pub fn recent() -> Self {
        Self {
            source: Source::Recent,
            title: "recent".into(),
            cursor: 0,
        }
    }

    /// The host's named colours.
    #[must_use]
    pub fn named(title: impl Into<String>) -> Self {
        Self {
            source: Source::Named,
            title: title.into(),
            cursor: 0,
        }
    }

    /// Fixed rows under a title of the host's.
    #[must_use]
    pub fn rows(title: impl Into<String>, rows: Vec<Vec<Rgb>>) -> Self {
        Self {
            source: Source::Rows(rows),
            title: title.into(),
            cursor: 0,
        }
    }

    fn rows_of(&self, state: &State) -> Vec<Vec<(Rgb, Option<usize>)>> {
        match &self.source {
            Source::Rows(rows) => rows
                .iter()
                .map(|row| row.iter().map(|rgb| (*rgb, None)).collect())
                .collect(),
            Source::Recent => vec![state.recent.iter().map(|rgb| (*rgb, None)).collect()],
            Source::Named => vec![
                state
                    .entries
                    .iter()
                    .enumerate()
                    .filter_map(|(at, entry)| entry.colour.map(|rgb| (rgb, Some(at))))
                    .collect(),
            ],
        }
    }

    /// Where every swatch lands in `area`, wrapping a row that does not fit.
    fn placed(&self, area: Rect, state: &State, config: &Config) -> Vec<Placed> {
        let width = glyph_width(config);
        let pitch = width + 1;
        let mut out = Vec::new();
        if width > area.width {
            return out;
        }
        let mut y = area.y;
        for row in self.rows_of(state) {
            let mut x = area.x;
            for (rgb, named) in row {
                if x + width > area.right() {
                    x = area.x;
                    y += 1;
                }
                if y >= area.bottom() {
                    return out;
                }
                out.push(Placed {
                    rect: Rect::new(x, y, width, 1),
                    rgb,
                    named,
                });
                x += pitch;
            }
            y += 1;
        }
        out
    }
}

impl View for Swatches {
    fn title(&self) -> &str {
        &self.title
    }

    fn size(&self, state: &State, config: &Config) -> Size {
        let rows = self.rows_of(state);
        let pitch = glyph_width(config) + 1;
        let widest = rows.iter().map(Vec::len).max().unwrap_or(0) as u16;
        Size {
            width: widest * pitch,
            height: rows.len() as u16 + u16::from(config.titles),
        }
    }

    fn shown(&self, state: &State, config: &Config) -> bool {
        match self.source {
            Source::Recent => state.recent.len() >= config.recent_minimum.max(1),
            Source::Named => state.entries.iter().any(|entry| entry.colour.is_some()),
            Source::Rows(ref rows) => rows.iter().any(|row| !row.is_empty()),
        }
    }

    fn draw(&self, area: Rect, buffer: &mut Buffer, state: &State, context: &Context) {
        let config = context.config;
        let area = if config.titles && area.height >= 2 {
            let style = Style::default().fg(if context.focused {
                context.palette.accent
            } else {
                context.palette.dim
            });
            text(buffer, area.x, area.y, area.width, &self.title, style);
            Rect::new(area.x, area.y + 1, area.width, area.height - 1)
        } else {
            area
        };
        let current = config.depth.shown(state.rgb());
        for (at, placed) in self.placed(area, state, config).iter().enumerate() {
            swatch(
                buffer,
                placed.rect,
                area,
                &config.swatch,
                config.depth.color(placed.rgb),
            );
            let cursor = context.focused && at == self.cursor;
            if cursor || config.depth.shown(placed.rgb) == current {
                mark(
                    buffer,
                    placed.rect.x,
                    placed.rect.y,
                    if cursor { config.marker } else { '•' },
                    placed.rgb,
                    config.depth,
                );
            }
        }
    }

    fn at(&self, area: Rect, column: u16, row: u16, state: &State, config: &Config) -> Option<Rgb> {
        let area = if config.titles && area.height >= 2 {
            Rect::new(area.x, area.y + 1, area.width, area.height - 1)
        } else {
            area
        };
        self.placed(area, state, config)
            .into_iter()
            .find(|placed| contains(placed.rect, column, row))
            .map(|placed| placed.rgb)
    }

    fn press(
        &mut self,
        area: Rect,
        column: u16,
        row: u16,
        state: &mut State,
        config: &Config,
    ) -> bool {
        let area = if config.titles && area.height >= 2 {
            Rect::new(area.x, area.y + 1, area.width, area.height - 1)
        } else {
            area
        };
        let placed = self.placed(area, state, config);
        let Some((at, hit)) = placed
            .iter()
            .enumerate()
            .find(|(_, placed)| contains(placed.rect, column, row))
        else {
            return false;
        };
        self.cursor = at;
        match hit.named {
            Some(index) => state.set_named(index),
            None => state.set_rgb(config.depth.shown(hit.rgb)),
        }
        true
    }

    fn key(&mut self, key: KeyEvent, state: &mut State, config: &Config) -> Outcome {
        let rows = self.rows_of(state);
        let flat: Vec<(Rgb, Option<usize>)> = rows.iter().flatten().copied().collect();
        if flat.is_empty() {
            return Outcome::Ignored;
        }
        let across = rows.first().map_or(1, Vec::len).max(1);
        let last = flat.len() - 1;
        match key.code {
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(last),
            KeyCode::Up => self.cursor = self.cursor.saturating_sub(across),
            KeyCode::Down => self.cursor = (self.cursor + across).min(last),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = last,
            _ => return Outcome::Ignored,
        }
        let (rgb, named) = flat[self.cursor.min(last)];
        match named {
            Some(index) => state.set_named(index),
            None => state.set_rgb(config.depth.shown(rgb)),
        }
        Outcome::Changed
    }
}

// ---------------------------------------------------------------------------------------------------
// The typed field and the named list
// ---------------------------------------------------------------------------------------------------

fn typing(key: KeyEvent, state: &mut State) -> Option<Outcome> {
    match key.code {
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            state.filter.push(character);
        }
        KeyCode::Backspace => state.filter.pop(),
        _ => return None,
    }
    state.refusal = None;
    state.refilter();
    let query = state.filter.query();
    let bare = !query.starts_with('#') && !state.filter.hits().is_empty();
    if let Ok(Choice::Colour(rgb)) = super::parse(query, &state.entries)
        && !bare
    {
        // A typed colour previews as it becomes one; nothing is chosen until Enter.
        state.set_rgb(rgb);
    }
    Some(Outcome::Changed)
}

fn accept_typed(state: &mut State, config: &Config) -> Outcome {
    if state.commit_typed() {
        if let Choice::Colour(rgb) = state.choice() {
            state.set_rgb(config.depth.shown(rgb));
        }
        Outcome::Accepted(state.choice())
    } else if state.refusal.is_some() {
        Outcome::Changed
    } else {
        Outcome::Ignored
    }
}

/// The field a colour is typed into: a swatch of the colour, the text, and under it the line that
/// says why the text was refused, or the hex under the pointer.
#[derive(Clone, Copy, Debug, Default)]
pub struct Typed;

impl View for Typed {
    fn title(&self) -> &'static str {
        "value"
    }

    fn size(&self, _state: &State, _config: &Config) -> Size {
        Size {
            width: 24,
            height: 2,
        }
    }

    fn draw(&self, area: Rect, buffer: &mut Buffer, state: &State, context: &Context) {
        let config = context.config;
        let palette = context.palette;
        let width = glyph_width(config);
        let shown = config.depth.shown(state.rgb());
        swatch(
            buffer,
            Rect::new(area.x, area.y, width.min(area.width), 1),
            area,
            &config.swatch,
            config.depth.color(shown),
        );
        let x = area.x + width + 1;
        let room = area.right().saturating_sub(x);
        let query = state.filter.query();
        let (words, style) = if query.is_empty() {
            let current = match state.choice() {
                Choice::Named(index) => state.entries[index].label.clone(),
                Choice::Colour(_) => format(shown, &config.format),
            };
            (current, Style::default().fg(palette.dim))
        } else {
            (query.to_string(), Style::default().fg(palette.foreground))
        };
        let end = text(buffer, x, area.y, room, &words, style);
        if context.focused && end < area.right() {
            text(
                buffer,
                end,
                area.y,
                1,
                "▏",
                Style::default().fg(palette.accent),
            );
        }
        if area.height < 2 {
            return;
        }
        let y = area.y + 1;
        if let Some(refusal) = state.refusal() {
            text(
                buffer,
                area.x,
                y,
                area.width,
                &refusal.0,
                Style::default().fg(palette.bad),
            );
        } else if let Some(hover) = state.hover() {
            text(
                buffer,
                area.x,
                y,
                area.width,
                &format(hover, &config.format),
                Style::default().fg(palette.dim),
            );
        }
    }

    fn at(
        &self,
        _area: Rect,
        _column: u16,
        _row: u16,
        _state: &State,
        _config: &Config,
    ) -> Option<Rgb> {
        None
    }

    fn press(
        &mut self,
        _area: Rect,
        _column: u16,
        _row: u16,
        state: &mut State,
        _config: &Config,
    ) -> bool {
        state.filter.focus(true);
        false
    }

    fn key(&mut self, key: KeyEvent, state: &mut State, config: &Config) -> Outcome {
        if let Some(outcome) = typing(key, state) {
            return outcome;
        }
        match key.code {
            KeyCode::Enter => accept_typed(state, config),
            KeyCode::Up => {
                state.filter.move_by(-1);
                Outcome::Handled
            }
            KeyCode::Down => {
                state.filter.move_by(1);
                Outcome::Handled
            }
            _ => Outcome::Ignored,
        }
    }
}

/// The host's named entries, each with a swatch, filtered by what is typed.
#[derive(Clone, Debug, Default)]
pub struct List {
    height: Cell<u16>,
}

impl List {
    fn row_rect(area: Rect, line: usize) -> Rect {
        Rect::new(area.x, area.y + line as u16, area.width, 1)
    }

    fn hit(area: Rect, column: u16, row: u16, state: &State) -> Option<usize> {
        if !contains(area, column, row) {
            return None;
        }
        let line = usize::from(row - area.y);
        state
            .filter
            .visible(usize::from(area.height))
            .nth(line)
            .map(|(_, hit)| hit.index)
    }
}

impl View for List {
    fn title(&self) -> &'static str {
        "named"
    }

    fn size(&self, state: &State, _config: &Config) -> Size {
        Size {
            width: 28,
            height: state.filter.hits().len().clamp(1, 12) as u16,
        }
    }

    fn draw(&self, area: Rect, buffer: &mut Buffer, state: &State, context: &Context) {
        self.height.set(area.height);
        let config = context.config;
        let palette = context.palette;
        let width = glyph_width(config);
        let labels: Vec<&str> = state
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect();
        if state.filter.measured() != labels.len() {
            // The host changed the entries without refiltering; draw nothing rather than offsets
            // nobody measured.
            return;
        }
        for (line, (offset, hit)) in state.filter.visible(usize::from(area.height)).enumerate() {
            let rect = Self::row_rect(area, line);
            let entry = &state.entries[hit.index];
            let selected = state.filter.is_selected(offset);
            if selected {
                buffer.set_style(rect, Style::default().bg(palette.selection));
            }
            if let Some(rgb) = entry.colour {
                swatch(
                    buffer,
                    Rect::new(rect.x, rect.y, width.min(rect.width), 1),
                    rect,
                    &config.swatch,
                    config.depth.color(rgb),
                );
            }
            let x = rect.x + width + 1;
            let room = rect.right().saturating_sub(x);
            let line = pick::row(&entry.label, hit, selected, palette, room);
            let end = buffer.set_line(x, rect.y, &line, room).0;
            if !entry.detail.is_empty() && end + 1 < rect.right() {
                let style = Style::default().fg(palette.dim);
                let style = if selected {
                    style.bg(palette.selection)
                } else {
                    style
                };
                text(
                    buffer,
                    end + 1,
                    rect.y,
                    rect.right() - end - 1,
                    &entry.detail,
                    style,
                );
            }
        }
        if state.filter.hits().is_empty() && area.height > 0 {
            let words = if state.filter.query().is_empty() {
                "no named colours"
            } else {
                "no name matches; Enter takes a colour typed as #rgb or #rrggbb"
            };
            text(
                buffer,
                area.x,
                area.y,
                area.width,
                words,
                Style::default().fg(palette.dim),
            );
        }
    }

    fn at(
        &self,
        area: Rect,
        column: u16,
        row: u16,
        state: &State,
        _config: &Config,
    ) -> Option<Rgb> {
        Self::hit(area, column, row, state).and_then(|index| state.entries[index].colour)
    }

    fn press(
        &mut self,
        area: Rect,
        column: u16,
        row: u16,
        state: &mut State,
        _config: &Config,
    ) -> bool {
        let line = usize::from(row.saturating_sub(area.y));
        let Some(index) = Self::hit(area, column, row, state) else {
            return false;
        };
        state.filter.select(state.filter.scroll() + line);
        state.set_named(index);
        true
    }

    fn key(&mut self, key: KeyEvent, state: &mut State, config: &Config) -> Outcome {
        let height = usize::from(self.height.get().max(1));
        let moved = match key.code {
            KeyCode::Up => Some(-1),
            KeyCode::Down => Some(1),
            KeyCode::PageUp => Some(-height.cast_signed()),
            KeyCode::PageDown => Some(height.cast_signed()),
            _ => None,
        };
        if let Some(delta) = moved {
            state.filter.move_by(delta);
            state.filter.follow(height);
            return Outcome::Handled;
        }
        if key.code == KeyCode::Enter {
            return accept_typed(state, config);
        }
        typing(key, state).unwrap_or(Outcome::Ignored)
    }
}

// ---------------------------------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------------------------------

/// The hex, the old colour beside the new, the rgb and hsl lines, and editable boxes: the hex, then
/// the RGB channels, then the channels of the state's other space.
#[derive(Clone, Debug, Default)]
pub struct Panel {
    selected: usize,
}

/// One editable box of the panel.
struct Field7 {
    rect: Rect,
    label: &'static str,
    value: String,
}

impl Panel {
    fn second(state: &State) -> Space {
        match state.space {
            Space::Rgb => Space::Hsl,
            other => other,
        }
    }

    fn boxes(area: Rect, state: &State, config: &Config) -> Vec<Field7> {
        let mut out = Vec::new();
        let hex_row = area.y + 5;
        if hex_row < area.bottom() {
            out.push(Field7 {
                rect: Rect::new(area.x, hex_row, area.width, 1),
                label: "HEX",
                value: format(
                    config.depth.shown(state.rgb()),
                    &super::Format {
                        form: super::Form::Hex,
                        ..config.format
                    },
                ),
            });
        }
        let third = area.width / 3;
        for (line, space) in [(6, Space::Rgb), (7, Self::second(state))] {
            let y = area.y + line;
            if y >= area.bottom() || third == 0 {
                continue;
            }
            let values = state.channels(space);
            for (index, value) in values.iter().enumerate() {
                out.push(Field7 {
                    rect: Rect::new(area.x + third * index as u16, y, third, 1),
                    label: space.channels()[index],
                    value: value.to_string(),
                });
            }
        }
        out
    }

    fn commit(state: &mut State) -> bool {
        let Some((at, typed)) = state.edit.take() else {
            return false;
        };
        if at == 0 {
            if let Some(rgb) = hex(&typed.to_ascii_lowercase()) {
                state.set_rgb(rgb);
                return true;
            }
        } else if let Ok(value) = typed.trim().parse::<u16>() {
            let space = if at <= 3 {
                Space::Rgb
            } else {
                Self::second(state)
            };
            let index = (at - 1) % 3;
            if value <= space.max(index) {
                state.set_channel(space, index, value);
                return true;
            }
        }
        state.refusal = Some(super::Refusal(if at == 0 {
            "not a hex: type #rgb or #rrggbb".into()
        } else {
            "out of range".into()
        }));
        state.edit = Some((at, typed));
        false
    }
}

impl Panel {
    fn draw_boxes(
        &self,
        area: Rect,
        buffer: &mut Buffer,
        state: &State,
        context: &Context,
        strong: Style,
    ) {
        let config = context.config;
        let palette = context.palette;
        for (at, field) in Self::boxes(area, state, config).iter().enumerate() {
            let selected = context.focused && at == self.selected;
            let label = Style::default().fg(if selected {
                palette.accent
            } else {
                palette.dim
            });
            let end = text(
                buffer,
                field.rect.x,
                field.rect.y,
                field.rect.width,
                field.label,
                label,
            );
            let editing = state.editing().filter(|(which, _)| *which == at);
            let value = editing.map_or(field.value.as_str(), |(_, typed)| typed);
            let style = if selected {
                strong.bg(palette.selection)
            } else {
                Style::default().fg(palette.foreground)
            };
            let end = text(
                buffer,
                end + 1,
                field.rect.y,
                field.rect.right().saturating_sub(end + 1),
                value,
                style,
            );
            if editing.is_some() && end < field.rect.right() {
                text(
                    buffer,
                    end,
                    field.rect.y,
                    1,
                    "▏",
                    Style::default().fg(palette.accent),
                );
            }
        }
    }
}

impl View for Panel {
    fn title(&self) -> &'static str {
        "panel"
    }

    fn size(&self, _state: &State, config: &Config) -> Size {
        Size {
            width: config.side_width,
            height: 9,
        }
    }

    fn draw(&self, area: Rect, buffer: &mut Buffer, state: &State, context: &Context) {
        let config = context.config;
        let palette = context.palette;
        let shown = config.depth.shown(state.rgb());
        let strong = Style::default()
            .fg(palette.foreground)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(palette.dim);
        let mut y = area.y;
        let line = |buffer: &mut Buffer, y: u16, content: &str, style: Style| {
            if y < area.bottom() {
                text(buffer, area.x, y, area.width, content, style);
            }
        };
        match state.hover() {
            Some(hover) => line(
                buffer,
                y,
                &format!("{}  hover", format(hover, &config.format)),
                dim,
            ),
            None => line(buffer, y, &format(shown, &config.format), strong),
        }
        y += 1;
        if y < area.bottom() {
            let half = area.width.saturating_sub(3) / 2;
            let old = config.depth.shown(state.original());
            buffer.set_style(
                Rect::new(area.x, y, half, 1),
                Style::default().bg(config.depth.color(old)),
            );
            text(buffer, area.x + half + 1, y, 1, "→", dim);
            buffer.set_style(
                Rect::new(area.x + half + 3, y, half, 1),
                Style::default().bg(config.depth.color(shown)),
            );
        }
        y += 1;
        line(
            buffer,
            y,
            &format!("rgb {} {} {}", shown.r, shown.g, shown.b),
            dim,
        );
        y += 1;
        let hsl = shown.hsl();
        line(
            buffer,
            y,
            &format!(
                "hsl {:.0} {:.0}% {:.0}%",
                hsl.h,
                hsl.s * 100.0,
                hsl.l * 100.0
            ),
            dim,
        );
        self.draw_boxes(area, buffer, state, context, strong);
        if let Some(refusal) = state.refusal()
            && state.editing().is_some()
        {
            line(
                buffer,
                area.y + 8,
                &refusal.0,
                Style::default().fg(palette.bad),
            );
        }
    }

    fn at(&self, area: Rect, column: u16, row: u16, state: &State, config: &Config) -> Option<Rgb> {
        // The old swatch picks the colour the picker opened with.
        let half = area.width.saturating_sub(3) / 2;
        (contains(Rect::new(area.x, area.y + 1, half, 1), column, row))
            .then(|| config.depth.shown(state.original()))
    }

    fn press(
        &mut self,
        area: Rect,
        column: u16,
        row: u16,
        state: &mut State,
        config: &Config,
    ) -> bool {
        if let Some(rgb) = self.at(area, column, row, state, config) {
            state.set_rgb(rgb);
            return true;
        }
        if let Some(at) = Self::boxes(area, state, config)
            .iter()
            .position(|field| contains(field.rect, column, row))
        {
            self.selected = at;
        }
        false
    }

    fn key(&mut self, key: KeyEvent, state: &mut State, config: &Config) -> Outcome {
        let count = 7;
        if let Some((at, mut typed)) = state.edit.take() {
            match key.code {
                KeyCode::Enter => {
                    state.edit = Some((at, typed));
                    return if Self::commit(state) {
                        Outcome::Changed
                    } else {
                        Outcome::Handled
                    };
                }
                KeyCode::Esc => {
                    state.refusal = None;
                    return Outcome::Handled;
                }
                KeyCode::Backspace => {
                    typed.pop();
                }
                KeyCode::Char(character) => typed.push(character),
                _ => {}
            }
            state.edit = Some((at, typed));
            return Outcome::Handled;
        }
        let space = if self.selected <= 3 {
            Space::Rgb
        } else {
            Self::second(state)
        };
        let step = if shift(key) { 10 } else { 1 };
        match key.code {
            KeyCode::Left => self.selected = self.selected.saturating_sub(1),
            KeyCode::Right => self.selected = (self.selected + 1).min(count - 1),
            KeyCode::Up | KeyCode::Down if self.selected > 0 => {
                let index = (self.selected - 1) % 3;
                let value = state.channels(space)[index];
                let value = if key.code == KeyCode::Up {
                    value + step
                } else {
                    value.saturating_sub(step)
                };
                state.set_channel(space, index, value);
                return Outcome::Changed;
            }
            code if Keys::hit(&config.keys.accept, code) => {
                state.edit = Some((self.selected, String::new()));
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Handled
    }
}

// ---------------------------------------------------------------------------------------------------
// The footer
// ---------------------------------------------------------------------------------------------------

/// The keys, one line.
#[derive(Clone, Copy, Debug, Default)]
pub struct Footer;

impl View for Footer {
    fn title(&self) -> &'static str {
        "keys"
    }

    fn size(&self, _state: &State, _config: &Config) -> Size {
        Size {
            width: 40,
            height: 1,
        }
    }

    fn focusable(&self) -> bool {
        false
    }

    fn draw(&self, area: Rect, buffer: &mut Buffer, _state: &State, context: &Context) {
        let keys = &context.config.keys;
        let palette = context.palette;
        let mut spans = Vec::new();
        for (list, what) in [
            (&keys.focus_next, "focus"),
            (&keys.switch_space, "switch"),
            (&keys.accept, "edit"),
            (&keys.copy, "copy"),
            (&keys.cancel, "cancel"),
        ] {
            let mut label = String::new();
            hint(&mut label, list, "");
            if label.is_empty() {
                continue;
            }
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                Keys::label(list),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!(" {what}"),
                Style::default().fg(palette.dim),
            ));
        }
        buffer.set_line(area.x, area.y, &Line::from(spans), area.width);
    }

    fn at(
        &self,
        _area: Rect,
        _column: u16,
        _row: u16,
        _state: &State,
        _config: &Config,
    ) -> Option<Rgb> {
        None
    }

    fn key(&mut self, _key: KeyEvent, _state: &mut State, _config: &Config) -> Outcome {
        Outcome::Ignored
    }
}
