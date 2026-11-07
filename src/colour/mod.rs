//! A colour picker as a set of views over one shared colour.
//!
//! Nothing here is a fixed screen. A [`Picker`] is a list of [`View`]s, each placed by the host — the
//! fill, a narrow column beside it, a side column, rows below, the footer — and every one of them reads
//! and writes the same [`State`], so a press on a field moves the sliders, the panel's numbers and the
//! typed text in the same frame. A host or another crate adds a view of its own by implementing
//! [`View`]; the ones shipped are in [`views`].
//!
//! Colour is typed ([`parse`]), chosen from a list of host-named entries ([`views::List`]), or picked
//! by pressing a point on a field drawn with the upper half block, two colour points a cell
//! ([`views::Field`]). A terminal that shows 256 or 16 colours is told so through [`Depth`]: every
//! view draws what the terminal can show, and the hex a view prints is the one it drew.
//!
//! ```
//! use ratatui::buffer::Buffer;
//! use ratatui::layout::Rect;
//! use tuilith::colour::{Config, Picker, Preset, State, Rgb};
//!
//! let config = Config::default();
//! let mut picker = Picker::preset(Preset::Full, &config);
//! let mut state = State::new(Rgb::new(0x5c, 0xbd, 0xfa));
//! let area = Rect::new(0, 0, 70, 24);
//! let mut buffer = Buffer::empty(area);
//! picker.draw(area, &mut buffer, &state, &config, &tuilith::DEFAULT_DARK);
//! // A press on the field picks the colour under it.
//! let outcome = picker.press(area, 5, 3, &mut state, &config);
//! assert!(outcome.changed());
//! ```

// Colour maths is in short single letters by convention (h, s, v, l, r, g, b), and every float cast
// back to a channel or a cell is clamped to a non-negative range first.
#![allow(clippy::many_single_char_names, clippy::cast_sign_loss)]

use std::fmt::Write as _;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::pick::Filter;
use crate::theme::Palette;

pub mod views;

#[cfg(test)]
mod tests;

crate::provenance! {
    component: "colour",
    about: "A colour picker as composable views over one shared colour: typed value, named list, fields, strips, sliders, swatches and a panel",
    origin: crate::Origin::Here,
    lineage: crate::Lineage::Inspired {
        by: "ratatui-color-picker's field, value strip and side panel; tui_color_picker's wheel; iro.js and react-colorful",
    },
    since: "0.1",
}

// ---------------------------------------------------------------------------------------------------
// Colour spaces
// ---------------------------------------------------------------------------------------------------

/// A colour as the terminal is sent it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Rgb {
    /// Red, 0 to 255.
    pub r: u8,
    /// Green, 0 to 255.
    pub g: u8,
    /// Blue, 0 to 255.
    pub b: u8,
}

impl Rgb {
    /// A colour from its three channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// The colour as ratatui draws it in truecolor.
    #[must_use]
    pub const fn color(self) -> Color {
        Color::Rgb(self.r, self.g, self.b)
    }

    /// The colour of a ratatui [`Color`], where it names one: `Rgb` exactly, `Indexed` and the
    /// sixteen named colours at xterm's values. `Reset` names none.
    #[must_use]
    pub fn of(color: Color) -> Option<Self> {
        match color {
            Color::Rgb(r, g, b) => Some(Self::new(r, g, b)),
            Color::Indexed(index) => Some(xterm(index)),
            Color::Reset => None,
            named => ANSI16
                .iter()
                .position(|(_, candidate)| *candidate == named)
                .map(|at| xterm(at as u8)),
        }
    }

    /// `#rrggbb`, lower case.
    #[must_use]
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// This colour in hue, saturation and value.
    #[must_use]
    pub fn hsv(self) -> Hsv {
        let (r, g, b) = self.unit();
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        Hsv {
            h: hue(r, g, b, max, delta),
            s: if max <= 0.0 { 0.0 } else { delta / max },
            v: max,
        }
    }

    /// This colour in hue, saturation and lightness.
    #[must_use]
    pub fn hsl(self) -> Hsl {
        let (r, g, b) = self.unit();
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let l = f32::midpoint(max, min);
        let s = if delta <= 0.0 {
            0.0
        } else {
            delta / (1.0 - (2.0 * l - 1.0).abs())
        };
        Hsl {
            h: hue(r, g, b, max, delta),
            s: s.clamp(0.0, 1.0),
            l,
        }
    }

    /// Relative luminance, for choosing readable text over this colour.
    #[must_use]
    pub fn luminance(self) -> f32 {
        let (r, g, b) = self.unit();
        let linear = |c: f32| {
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    fn unit(self) -> (f32, f32, f32) {
        (
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
        )
    }

    fn from_unit(r: f32, g: f32, b: f32) -> Self {
        let channel = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self::new(channel(r), channel(g), channel(b))
    }
}

fn hue(r: f32, g: f32, b: f32, max: f32, delta: f32) -> f32 {
    if delta <= 0.0 {
        return 0.0;
    }
    let raw = if (max - r).abs() < f32::EPSILON {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if (max - g).abs() < f32::EPSILON {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    raw.rem_euclid(360.0)
}

/// Hue in degrees, saturation and value from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Hsv {
    /// Hue, 0 to 360.
    pub h: f32,
    /// Saturation, 0 to 1.
    pub s: f32,
    /// Value, 0 to 1.
    pub v: f32,
}

impl Hsv {
    /// The colour these coordinates name.
    #[must_use]
    pub fn rgb(self) -> Rgb {
        let h = self.h.rem_euclid(360.0) / 60.0;
        let s = self.s.clamp(0.0, 1.0);
        let v = self.v.clamp(0.0, 1.0);
        let c = v * s;
        let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = sector(h, c, x);
        let m = v - c;
        Rgb::from_unit(r + m, g + m, b + m)
    }
}

/// Hue in degrees, saturation and lightness from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Hsl {
    /// Hue, 0 to 360.
    pub h: f32,
    /// Saturation, 0 to 1.
    pub s: f32,
    /// Lightness, 0 to 1.
    pub l: f32,
}

impl Hsl {
    /// The colour these coordinates name.
    #[must_use]
    pub fn rgb(self) -> Rgb {
        let h = self.h.rem_euclid(360.0) / 60.0;
        let s = self.s.clamp(0.0, 1.0);
        let l = self.l.clamp(0.0, 1.0);
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = sector(h, c, x);
        let m = l - c / 2.0;
        Rgb::from_unit(r + m, g + m, b + m)
    }

    /// The same colour in HSV, keeping this hue.
    #[must_use]
    pub fn hsv(self) -> Hsv {
        let v = self.l + self.s * self.l.min(1.0 - self.l);
        let s = if v <= 0.0 {
            0.0
        } else {
            2.0 * (1.0 - self.l / v)
        };
        Hsv {
            h: self.h,
            s: s.clamp(0.0, 1.0),
            v: v.clamp(0.0, 1.0),
        }
    }
}

fn sector(h: f32, c: f32, x: f32) -> (f32, f32, f32) {
    match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    }
}

/// Which three numbers the sliders and the panel's boxes edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Space {
    /// Red, green, blue.
    #[default]
    Rgb,
    /// Hue, saturation, lightness.
    Hsl,
    /// Hue, saturation, value.
    Hsv,
}

impl Space {
    /// The next space, for the switch key.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Rgb => Self::Hsl,
            Self::Hsl => Self::Hsv,
            Self::Hsv => Self::Rgb,
        }
    }

    /// The space's name as the picker prints it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Rgb => "rgb",
            Self::Hsl => "hsl",
            Self::Hsv => "hsv",
        }
    }

    /// The three channels' one-letter names.
    #[must_use]
    pub fn channels(self) -> [&'static str; 3] {
        match self {
            Self::Rgb => ["R", "G", "B"],
            Self::Hsl => ["H", "S", "L"],
            Self::Hsv => ["H", "S", "V"],
        }
    }

    /// The largest value of channel `index`: 255 for RGB, 360 for a hue, 100 for a percentage.
    #[must_use]
    pub fn max(self, index: usize) -> u16 {
        match (self, index) {
            (Self::Rgb, _) => 255,
            (_, 0) => 360,
            _ => 100,
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// Colour depth
// ---------------------------------------------------------------------------------------------------

/// How many colours the terminal shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Depth {
    /// Any 24-bit colour.
    #[default]
    TrueColor,
    /// The xterm 256-colour palette.
    Ansi256,
    /// The sixteen named colours, at xterm's values.
    Ansi16,
}

impl Depth {
    /// The colour the terminal will actually show for `rgb`. A view that prints a hex prints this.
    #[must_use]
    pub fn shown(self, rgb: Rgb) -> Rgb {
        match self {
            Self::TrueColor => rgb,
            Self::Ansi256 => xterm(nearest(rgb, 16..=255)),
            Self::Ansi16 => xterm(nearest(rgb, 0..=15)),
        }
    }

    /// What to hand ratatui to draw `rgb`.
    #[must_use]
    pub fn color(self, rgb: Rgb) -> Color {
        match self {
            Self::TrueColor => rgb.color(),
            Self::Ansi256 => Color::Indexed(nearest(rgb, 16..=255)),
            Self::Ansi16 => ANSI16[nearest(rgb, 0..=15) as usize].1,
        }
    }
}

const ANSI16: [(&str, Color); 16] = [
    ("black", Color::Black),
    ("red", Color::Red),
    ("green", Color::Green),
    ("yellow", Color::Yellow),
    ("blue", Color::Blue),
    ("magenta", Color::Magenta),
    ("cyan", Color::Cyan),
    ("gray", Color::Gray),
    ("darkgray", Color::DarkGray),
    ("lightred", Color::LightRed),
    ("lightgreen", Color::LightGreen),
    ("lightyellow", Color::LightYellow),
    ("lightblue", Color::LightBlue),
    ("lightmagenta", Color::LightMagenta),
    ("lightcyan", Color::LightCyan),
    ("white", Color::White),
];

/// xterm's value for palette index `index`.
#[must_use]
pub fn xterm(index: u8) -> Rgb {
    const BASE: [u32; 16] = [
        0x00_0000, 0xcd_0000, 0x00_cd00, 0xcd_cd00, 0x00_00ee, 0xcd_00cd, 0x00_cdcd, 0xe5_e5e5,
        0x7f_7f7f, 0xff_0000, 0x00_ff00, 0xff_ff00, 0x5c_5cff, 0xff_00ff, 0x00_ffff, 0xff_ffff,
    ];
    match index {
        0..=15 => {
            let value = BASE[index as usize];
            Rgb::new((value >> 16) as u8, (value >> 8) as u8, value as u8)
        }
        16..=231 => {
            let at = index - 16;
            let level = |step: u8| if step == 0 { 0 } else { 55 + step * 40 };
            Rgb::new(level(at / 36), level((at / 6) % 6), level(at % 6))
        }
        _ => {
            let grey = 8 + (index - 232) * 10;
            Rgb::new(grey, grey, grey)
        }
    }
}

fn nearest(rgb: Rgb, range: std::ops::RangeInclusive<u8>) -> u8 {
    let distance = |candidate: Rgb| {
        let dr = i32::from(rgb.r) - i32::from(candidate.r);
        let dg = i32::from(rgb.g) - i32::from(candidate.g);
        let db = i32::from(rgb.b) - i32::from(candidate.b);
        // Weighted toward green, as the eye is.
        2 * dr * dr + 4 * dg * dg + 3 * db * db
    };
    range
        .min_by_key(|index| distance(xterm(*index)))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------------
// Typed text
// ---------------------------------------------------------------------------------------------------

/// A colour the host names: a palette role, a named colour, anything it can write by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// What is written and what typing matches: `accent`, `red`.
    pub label: String,
    /// The colour it stands for now, if it has one. `None` draws no swatch (an unset, a `default`).
    pub colour: Option<Rgb>,
    /// Drawn dim after the label, such as the role's current hex. Empty draws nothing.
    pub detail: String,
}

impl Entry {
    /// An entry with a colour and no detail.
    #[must_use]
    pub fn new(label: impl Into<String>, colour: Option<Rgb>) -> Self {
        Self {
            label: label.into(),
            colour,
            detail: String::new(),
        }
    }

    /// The same entry with a dim detail after the label.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
}

/// What a person chose: one of the host's entries by name, or a colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// The entry at this index of [`State::entries`].
    Named(usize),
    /// A colour, typed or picked.
    Colour(Rgb),
}

/// Why typed text was refused, in words a person can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal(pub String);

/// Read typed text as a colour.
///
/// Accepts `#rgb`, `#rrggbb` (the `#` optional), `rgb(r, g, b)`, `hsl(h, s%, l%)`, one of the sixteen
/// terminal colour names, or the label of one of `entries` (case ignored). An entry wins over a
/// built-in name of the same spelling, because the host's meaning of `red` is the one being edited.
///
/// # Errors
///
/// A [`Refusal`] saying what is accepted, when the text is none of those.
pub fn parse(text: &str, entries: &[Entry]) -> Result<Choice, Refusal> {
    let text = text.trim();
    if let Some(at) = entries
        .iter()
        .position(|entry| entry.label.eq_ignore_ascii_case(text))
    {
        return Ok(Choice::Named(at));
    }
    let lower = text.to_ascii_lowercase();
    if let Some(rgb) = hex(&lower) {
        return Ok(Choice::Colour(rgb));
    }
    if let Some(rgb) = function(&lower) {
        return Ok(Choice::Colour(rgb));
    }
    let bare: String = lower.chars().filter(|c| *c != ' ' && *c != '_').collect();
    let bare = bare.replace("grey", "gray");
    if let Some(at) = ANSI16.iter().position(|(name, _)| *name == bare) {
        return Ok(Choice::Colour(xterm(at as u8)));
    }
    let names = if entries.is_empty() {
        "a colour name"
    } else {
        "a name from the list"
    };
    Err(Refusal(format!(
        "not a colour: type #rgb, #rrggbb, rgb(r, g, b), hsl(h, s%, l%) or {names}"
    )))
}

fn hex(text: &str) -> Option<Rgb> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = |at: std::ops::Range<usize>| u8::from_str_radix(&digits[at], 16).ok();
    match digits.len() {
        3 => {
            let expand = |at: usize| value(at..at + 1).map(|v| v * 17);
            Some(Rgb::new(expand(0)?, expand(1)?, expand(2)?))
        }
        6 => Some(Rgb::new(value(0..2)?, value(2..4)?, value(4..6)?)),
        _ => None,
    }
}

fn function(text: &str) -> Option<Rgb> {
    let (name, rest) = text.split_once('(')?;
    let args = rest.strip_suffix(')')?;
    let numbers: Vec<f32> = args
        .split(',')
        .map(|part| part.trim().trim_end_matches(['%', '°']).parse::<f32>().ok())
        .collect::<Option<Vec<_>>>()?;
    let [a, b, c] = numbers[..] else {
        return None;
    };
    match name.trim() {
        "rgb" if [a, b, c].iter().all(|v| (0.0..=255.0).contains(v)) => {
            Some(Rgb::new(a as u8, b as u8, c as u8))
        }
        "hsl" if (0.0..=100.0).contains(&b) && (0.0..=100.0).contains(&c) => Some(
            Hsl {
                h: a,
                s: b / 100.0,
                l: c / 100.0,
            }
            .rgb(),
        ),
        "hsv" if (0.0..=100.0).contains(&b) && (0.0..=100.0).contains(&c) => Some(
            Hsv {
                h: a,
                s: b / 100.0,
                v: c / 100.0,
            }
            .rgb(),
        ),
        _ => None,
    }
}

/// How a colour is written as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Form {
    /// `#5cbdfa`.
    #[default]
    Hex,
    /// `rgb(92, 189, 250)`.
    Rgb,
    /// `hsl(203, 94%, 67%)`.
    Hsl,
}

/// Every choice about how a colour is printed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Format {
    /// Which text form.
    pub form: Form,
    /// `#5CBDFA` rather than `#5cbdfa`.
    pub upper: bool,
    /// `#fa0` where the colour allows it.
    pub short: bool,
}

/// `rgb` written the way `format` says.
#[must_use]
pub fn format(rgb: Rgb, format: &Format) -> String {
    match format.form {
        Form::Hex => {
            let mut text = rgb.hex();
            let bytes = text.as_bytes();
            if format.short && bytes[1] == bytes[2] && bytes[3] == bytes[4] && bytes[5] == bytes[6]
            {
                text = format!("#{}{}{}", &text[1..2], &text[3..4], &text[5..6]);
            }
            if format.upper {
                text.make_ascii_uppercase();
            }
            text
        }
        Form::Rgb => format!("rgb({}, {}, {})", rgb.r, rgb.g, rgb.b),
        Form::Hsl => {
            let hsl = rgb.hsl();
            format!(
                "hsl({:.0}, {:.0}%, {:.0}%)",
                hsl.h,
                hsl.s * 100.0,
                hsl.l * 100.0
            )
        }
    }
}

/// Put `rgb` at the front of a recent-colours list, dropping its older copy and anything past `cap`.
///
/// The list is the host's to keep — tuilith writes no file — and this is the one rule for it.
pub fn remember(recent: &mut Vec<Rgb>, rgb: Rgb, cap: usize) {
    recent.retain(|held| *held != rgb);
    recent.insert(0, rgb);
    recent.truncate(cap);
}

// ---------------------------------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------------------------------

/// The keys the picker answers. Each is a list, so a host can bind several.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    /// Move focus to the next view.
    pub focus_next: Vec<KeyCode>,
    /// Move focus to the previous view.
    pub focus_previous: Vec<KeyCode>,
    /// Switch the sliders and the panel between RGB, HSL and HSV.
    pub switch_space: Vec<KeyCode>,
    /// Start editing the focused box, or accept the colour.
    pub accept: Vec<KeyCode>,
    /// Copy the colour's text.
    pub copy: Vec<KeyCode>,
    /// Leave, putting the original colour back.
    pub cancel: Vec<KeyCode>,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            focus_next: vec![KeyCode::Tab],
            focus_previous: vec![KeyCode::BackTab],
            switch_space: vec![KeyCode::Char('m'), KeyCode::Char('M')],
            accept: vec![KeyCode::Enter],
            copy: vec![KeyCode::Char('#')],
            cancel: vec![KeyCode::Esc],
        }
    }
}

impl Keys {
    fn hit(list: &[KeyCode], key: KeyCode) -> bool {
        list.contains(&key)
    }

    /// The first binding of a list, as a hint prints it.
    #[must_use]
    pub fn label(list: &[KeyCode]) -> String {
        match list.first() {
            Some(KeyCode::Tab) => "Tab".into(),
            Some(KeyCode::BackTab) => "S-Tab".into(),
            Some(KeyCode::Enter) => "Enter".into(),
            Some(KeyCode::Esc) => "Esc".into(),
            Some(KeyCode::Char(c)) => c.to_ascii_uppercase().to_string(),
            Some(KeyCode::F(n)) => format!("F{n}"),
            Some(other) => format!("{other:?}"),
            None => String::new(),
        }
    }
}

/// The sixteen strong and light colours of the common-colours rows: grey, red, green, blue, yellow,
/// violet, orange, teal, light blue and indigo, light tints first.
pub const COMMON: [[Rgb; 10]; 2] = [
    [
        Rgb::new(0xf0, 0xf0, 0xf0),
        Rgb::new(0xff, 0xcc, 0xcc),
        Rgb::new(0xcc, 0xf5, 0xcc),
        Rgb::new(0xb8, 0xdd, 0xff),
        Rgb::new(0xfa, 0xf0, 0xbf),
        Rgb::new(0xe2, 0xd4, 0xff),
        Rgb::new(0xff, 0xd8, 0xb8),
        Rgb::new(0xbf, 0xee, 0xea),
        Rgb::new(0xee, 0xf2, 0xf7),
        Rgb::new(0xd8, 0xe6, 0xff),
    ],
    [
        Rgb::new(0xc4, 0xc4, 0xc4),
        Rgb::new(0xff, 0x6b, 0x6b),
        Rgb::new(0x8c, 0xd9, 0x64),
        Rgb::new(0x7a, 0xc0, 0xff),
        Rgb::new(0xff, 0xd3, 0x4d),
        Rgb::new(0xc8, 0xa8, 0xff),
        Rgb::new(0xff, 0x8a, 0x33),
        Rgb::new(0x33, 0xc4, 0xc8),
        Rgb::new(0xb4, 0xcd, 0xf2),
        Rgb::new(0x2c, 0x2c, 0xb0),
    ],
];

/// Everything a host decides about the picker. tuilith reads no file of its own: a host maps its
/// settings onto this.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// What each swatch is drawn with. Its display width is the swatch's width; a glyph a font draws
    /// wider than it measures is styled across every cell it covers, so the overflow is the colour.
    pub swatch: String,
    /// How hexes and typed values are printed.
    pub format: Format,
    /// The space the sliders and the panel start in.
    pub space: Space,
    /// How many colours the terminal shows.
    pub depth: Depth,
    /// The common-colours rows, each a row of swatches.
    pub common: Vec<Vec<Rgb>>,
    /// The fewest recent colours that make the recent row worth drawing.
    pub recent_minimum: usize,
    /// The width of the side column.
    pub side_width: u16,
    /// The width of a strip placed beside the fill, or the height of one placed below.
    pub strip_size: u16,
    /// Show the views placed in the fill one at a time, with a tab strip between them.
    pub tabs: bool,
    /// The point a field marks the colour with.
    pub marker: char,
    /// Show a view's title on its top row.
    pub titles: bool,
    /// The keys.
    pub keys: Keys,
    /// How far an arrow moves a hue, in degrees.
    pub hue_step: f32,
    /// How far an arrow moves saturation, value or lightness, from 0 to 1.
    pub step: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            swatch: "██".into(),
            format: Format::default(),
            space: Space::Rgb,
            depth: Depth::TrueColor,
            common: COMMON.iter().map(|row| row.to_vec()).collect(),
            recent_minimum: 2,
            side_width: 26,
            strip_size: 3,
            tabs: false,
            marker: '◉',
            titles: true,
            keys: Keys::default(),
            hue_step: 5.0,
            step: 0.05,
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------------

/// The one colour every view reads and writes, and what has been typed.
#[derive(Clone, Debug)]
pub struct State {
    hsv: Hsv,
    original: Rgb,
    named: Option<usize>,
    /// The host's named entries: palette roles, named colours. Shown by the list and the named
    /// swatch row, and matched by typing.
    pub entries: Vec<Entry>,
    /// Recently picked colours, newest first. The host keeps them; see [`remember`].
    pub recent: Vec<Rgb>,
    /// The space the sliders and panel show.
    pub space: Space,
    /// What has been typed into the field, and the list's filter over [`Self::entries`].
    pub filter: Filter,
    refusal: Option<Refusal>,
    hover: Option<Rgb>,
    focus: usize,
    tab: usize,
    edit: Option<(usize, String)>,
}

impl State {
    /// A state holding `rgb`, which is also what cancelling puts back.
    #[must_use]
    pub fn new(rgb: Rgb) -> Self {
        let mut filter = Filter::new();
        filter.refilter(std::iter::empty());
        Self {
            hsv: rgb.hsv(),
            original: rgb,
            named: None,
            entries: Vec::new(),
            recent: Vec::new(),
            space: Space::Rgb,
            filter,
            refusal: None,
            hover: None,
            focus: 0,
            tab: 0,
            edit: None,
        }
    }

    /// A state with host-named entries, the list filtered by nothing yet.
    #[must_use]
    pub fn with_entries(mut self, entries: Vec<Entry>) -> Self {
        self.entries = entries;
        self.refilter();
        self
    }

    /// The colour now.
    #[must_use]
    pub fn rgb(&self) -> Rgb {
        self.hsv.rgb()
    }

    /// The colour in HSV, with its hue kept through grey.
    #[must_use]
    pub fn hsv(&self) -> Hsv {
        self.hsv
    }

    /// The colour the state was opened with.
    #[must_use]
    pub fn original(&self) -> Rgb {
        self.original
    }

    /// What was chosen: a named entry, when the last change was one, or the colour.
    #[must_use]
    pub fn choice(&self) -> Choice {
        self.named.map_or(Choice::Colour(self.rgb()), Choice::Named)
    }

    /// Set the colour, keeping the hue when the new colour is grey.
    pub fn set_rgb(&mut self, rgb: Rgb) {
        let mut hsv = rgb.hsv();
        if hsv.s <= f32::EPSILON || hsv.v <= f32::EPSILON {
            hsv.h = self.hsv.h;
        }
        self.hsv = hsv;
        self.named = None;
        self.refusal = None;
    }

    /// Set the colour in HSV, exactly.
    pub fn set_hsv(&mut self, hsv: Hsv) {
        self.hsv = Hsv {
            h: hsv.h.rem_euclid(360.0),
            s: hsv.s.clamp(0.0, 1.0),
            v: hsv.v.clamp(0.0, 1.0),
        };
        self.named = None;
        self.refusal = None;
    }

    /// Choose the host's entry at `index`, taking its colour where it has one.
    pub fn set_named(&mut self, index: usize) {
        if let Some(rgb) = self.entries.get(index).and_then(|entry| entry.colour) {
            self.set_rgb(rgb);
        }
        if index < self.entries.len() {
            self.named = Some(index);
        }
        self.refusal = None;
    }

    /// Why the typed text was refused, while it stands.
    #[must_use]
    pub fn refusal(&self) -> Option<&Refusal> {
        self.refusal.as_ref()
    }

    /// The colour under the pointer, when it rests on something a press would pick.
    #[must_use]
    pub fn hover(&self) -> Option<Rgb> {
        self.hover
    }

    /// Which view has the keyboard, as an index into the picker's views.
    #[must_use]
    pub fn focus(&self) -> usize {
        self.focus
    }

    /// The fill view shown while [`Config::tabs`] is on.
    #[must_use]
    pub fn tab(&self) -> usize {
        self.tab
    }

    /// The box being edited in the panel, and its text.
    #[must_use]
    pub fn editing(&self) -> Option<(usize, &str)> {
        self.edit.as_ref().map(|(at, text)| (*at, text.as_str()))
    }

    /// Re-run the typed filter over the entries.
    pub fn refilter(&mut self) {
        let labels: Vec<&str> = self
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect();
        self.filter.refilter(labels.into_iter());
    }

    /// Take the typed text: a colour or an entry it parses as, or the list's selection when the text
    /// matches one, else a refusal held in the state and nothing changed.
    ///
    /// Returns whether the colour changed.
    pub fn commit_typed(&mut self) -> bool {
        let typed = self.filter.query().trim().to_string();
        if typed.is_empty() {
            if let Some(hit) = self.filter.selected() {
                let index = hit.index;
                self.set_named(index);
                return true;
            }
            return false;
        }
        // `acc` is a short hex and the start of `accent`. Without a `#`, a name the list still shows
        // wins: someone typing into a list of names is typing a name.
        let bare_hex = !typed.starts_with('#') && hex(&typed.to_ascii_lowercase()).is_some();
        if bare_hex && let Some(hit) = self.filter.selected() {
            let index = hit.index;
            self.set_named(index);
            return true;
        }
        match parse(&typed, &self.entries) {
            Ok(Choice::Named(index)) => {
                self.set_named(index);
                true
            }
            Ok(Choice::Colour(rgb)) => {
                self.set_rgb(rgb);
                true
            }
            Err(refusal) => {
                if let Some(hit) = self.filter.selected() {
                    let index = hit.index;
                    self.set_named(index);
                    return true;
                }
                self.refusal = Some(refusal);
                false
            }
        }
    }

    /// The channel values of [`Self::space`], as the panel and sliders print them.
    #[must_use]
    pub fn channels(&self, space: Space) -> [u16; 3] {
        let rgb = self.rgb();
        match space {
            Space::Rgb => [u16::from(rgb.r), u16::from(rgb.g), u16::from(rgb.b)],
            Space::Hsl => {
                let hsl = rgb.hsl();
                [
                    self.hsv.h.round() as u16 % 360,
                    (hsl.s * 100.0).round() as u16,
                    (hsl.l * 100.0).round() as u16,
                ]
            }
            Space::Hsv => [
                self.hsv.h.round() as u16 % 360,
                (self.hsv.s * 100.0).round() as u16,
                (self.hsv.v * 100.0).round() as u16,
            ],
        }
    }

    /// Set one channel of `space` to `value`, clamped to its range.
    pub fn set_channel(&mut self, space: Space, index: usize, value: u16) {
        let value = value.min(space.max(index));
        let mut channels = self.channels(space);
        channels[index] = value;
        let [a, b, c] = channels.map(f32::from);
        match space {
            Space::Rgb => self.set_rgb(Rgb::new(a as u8, b as u8, c as u8)),
            Space::Hsl => self.set_hsv(
                Hsl {
                    h: a,
                    s: b / 100.0,
                    l: c / 100.0,
                }
                .hsv(),
            ),
            Space::Hsv => self.set_hsv(Hsv {
                h: a,
                s: b / 100.0,
                v: c / 100.0,
            }),
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// Views and the picker
// ---------------------------------------------------------------------------------------------------

/// What a view draws with: the host's palette and the picker's config.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    /// The host's colours for text, borders and selection.
    pub palette: &'a Palette,
    /// The picker's config.
    pub config: &'a Config,
    /// Whether this view has the keyboard.
    pub focused: bool,
}

/// Where a view sits in the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The main area. Several fill views share it top to bottom, or one at a time under
    /// [`Config::tabs`].
    Fill,
    /// A narrow column immediately right of the fill, such as a value strip.
    Beside,
    /// Under the fill, at the view's own height, such as a swatch row or a horizontal strip.
    Below,
    /// The right-hand column, such as the panel.
    Side,
    /// The bottom row.
    Footer,
}

/// What a view wants of its rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Size {
    /// The width the view draws well in; a beside or side view gets this.
    pub width: u16,
    /// The height the view draws well in; a below view gets this.
    pub height: u16,
}

/// A piece of the picker. A host or another crate implements this to add a view of its own.
///
/// A view never holds the colour; it reads and writes [`State`]. That is what keeps any combination
/// of views in step.
pub trait View {
    /// The view's name, for a tab strip and a title.
    fn title(&self) -> &str;

    /// What it wants of its rect, given what it will be drawn with.
    fn size(&self, state: &State, config: &Config) -> Size;

    /// Whether drawing it now is worth anything. The recent row says no until two colours are kept.
    fn shown(&self, _state: &State, _config: &Config) -> bool {
        true
    }

    /// Whether Tab stops on it.
    fn focusable(&self) -> bool {
        true
    }

    /// Draw into `area`, and nowhere else.
    fn draw(&self, area: Rect, buffer: &mut Buffer, state: &State, context: &Context);

    /// The colour a press at `(column, row)` would pick, if any. Used for hover.
    fn at(&self, area: Rect, column: u16, row: u16, state: &State, config: &Config) -> Option<Rgb>;

    /// A press or a drag at `(column, row)`. Returns whether the state changed.
    fn press(
        &mut self,
        area: Rect,
        column: u16,
        row: u16,
        state: &mut State,
        config: &Config,
    ) -> bool {
        match self.at(area, column, row, state, config) {
            Some(rgb) => {
                state.set_rgb(rgb);
                true
            }
            None => false,
        }
    }

    /// A key while focused. Returns what it did.
    fn key(&mut self, key: KeyEvent, state: &mut State, config: &Config) -> Outcome;
}

/// What an input did.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Outcome {
    /// Nothing.
    #[default]
    Ignored,
    /// Taken, nothing visible moved (focus, a hover).
    Handled,
    /// The colour or the typed text changed. A host that previews live repaints.
    Changed,
    /// The person chose. The host writes [`State::choice`] and, for a colour, [`remember`]s it.
    Accepted(Choice),
    /// The person left. The state is back at its original colour.
    Cancelled,
    /// The colour's text, for the host to put on the clipboard.
    Copied(String),
}

impl Outcome {
    /// Whether the colour or the text moved.
    #[must_use]
    pub fn changed(&self) -> bool {
        matches!(self, Self::Changed)
    }
}

/// A view and where it sits.
pub struct Slot {
    /// The view.
    pub view: Box<dyn View>,
    /// Where.
    pub place: Place,
}

/// The ready-made compositions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// The compact dropdown a settings row opens first: the typed field over the named list.
    Inline,
    /// A square field, the value strip beside it, the side panel, common and recent rows, the footer.
    Full,
    /// Channel sliders with the side panel and the footer.
    Sliders,
    /// Common and recent rows only.
    Swatches,
}

impl Preset {
    /// The views the preset composes, so a host can start from one and change it.
    #[must_use]
    pub fn views(self) -> Vec<(Kind, Place)> {
        use views::{Channel, Orientation};
        match self {
            Self::Inline => vec![(Kind::Typed, Place::Fill), (Kind::List, Place::Fill)],
            Self::Full => vec![
                (Kind::Square, Place::Fill),
                (
                    Kind::Strip(Channel::Value, Orientation::Vertical),
                    Place::Beside,
                ),
                (Kind::Panel, Place::Side),
                (Kind::Common, Place::Below),
                (Kind::Recent, Place::Below),
                (Kind::Footer, Place::Footer),
            ],
            Self::Sliders => vec![
                (Kind::Sliders, Place::Fill),
                (Kind::Panel, Place::Side),
                (Kind::Footer, Place::Footer),
            ],
            Self::Swatches => vec![(Kind::Common, Place::Fill), (Kind::Recent, Place::Fill)],
        }
    }

    /// The preset a host's text names: `inline`, `full`, `sliders`, `swatches`.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "inline" => Some(Self::Inline),
            "full" => Some(Self::Full),
            "sliders" => Some(Self::Sliders),
            "swatches" => Some(Self::Swatches),
            _ => None,
        }
    }
}

/// Every view tuilith ships, by name, so a host's config can list them as data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// [`views::Field::square`].
    Square,
    /// [`views::Field::wheel`].
    Wheel,
    /// [`views::Field::diamond`].
    Diamond,
    /// [`views::Field::grid`].
    Grid,
    /// A [`views::Strip`].
    Strip(views::Channel, views::Orientation),
    /// [`views::Sliders`].
    Sliders,
    /// The common-colours rows.
    Common,
    /// The recent-colours row.
    Recent,
    /// The host's named entries as swatches.
    Named,
    /// [`views::Typed`].
    Typed,
    /// [`views::List`].
    List,
    /// [`views::Panel`].
    Panel,
    /// [`views::Footer`].
    Footer,
}

impl Kind {
    /// Every kind, strips in both orientations through every channel.
    #[must_use]
    pub fn all() -> Vec<Self> {
        use views::{Channel, Orientation};
        let mut all = vec![Self::Square, Self::Wheel, Self::Diamond, Self::Grid];
        for channel in [
            Channel::Value,
            Channel::Hue,
            Channel::Saturation,
            Channel::Lightness,
        ] {
            for orientation in [Orientation::Vertical, Orientation::Horizontal] {
                all.push(Self::Strip(channel, orientation));
            }
        }
        all.extend([
            Self::Sliders,
            Self::Common,
            Self::Recent,
            Self::Named,
            Self::Typed,
            Self::List,
            Self::Panel,
            Self::Footer,
        ]);
        all
    }

    /// The name a host's config writes: `square`, `value-strip`, `hue-bar`, `common`.
    #[must_use]
    pub fn name(self) -> String {
        use views::{Channel, Orientation};
        match self {
            Self::Square => "square".into(),
            Self::Wheel => "wheel".into(),
            Self::Diamond => "diamond".into(),
            Self::Grid => "grid".into(),
            Self::Strip(channel, orientation) => {
                let channel = match channel {
                    Channel::Value => "value",
                    Channel::Hue => "hue",
                    Channel::Saturation => "saturation",
                    Channel::Lightness => "lightness",
                };
                let shape = match orientation {
                    Orientation::Vertical => "strip",
                    Orientation::Horizontal => "bar",
                };
                format!("{channel}-{shape}")
            }
            Self::Sliders => "sliders".into(),
            Self::Common => "common".into(),
            Self::Recent => "recent".into(),
            Self::Named => "named".into(),
            Self::Typed => "typed".into(),
            Self::List => "list".into(),
            Self::Panel => "panel".into(),
            Self::Footer => "footer".into(),
        }
    }

    /// The kind a name from [`Self::name`] stands for.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::all().into_iter().find(|kind| kind.name() == name)
    }

    /// A fresh view of this kind.
    #[must_use]
    pub fn view(self, config: &Config) -> Box<dyn View> {
        use views::{Field, Footer, List, Panel, Sliders, Strip, Swatches, Typed};
        match self {
            Self::Square => Box::new(Field::square()),
            Self::Wheel => Box::new(Field::wheel()),
            Self::Diamond => Box::new(Field::diamond()),
            Self::Grid => Box::new(Field::grid()),
            Self::Strip(channel, orientation) => Box::new(Strip {
                channel,
                orientation,
            }),
            Self::Sliders => Box::new(Sliders::default()),
            Self::Common => Box::new(Swatches::common(config)),
            Self::Recent => Box::new(Swatches::recent()),
            Self::Named => Box::new(Swatches::named("named")),
            Self::Typed => Box::new(Typed),
            Self::List => Box::new(List::default()),
            Self::Panel => Box::new(Panel::default()),
            Self::Footer => Box::new(Footer),
        }
    }
}

impl Place {
    /// The name a host's config writes: `fill`, `beside`, `below`, `side`, `footer`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Beside => "beside",
            Self::Below => "below",
            Self::Side => "side",
            Self::Footer => "footer",
        }
    }

    /// The place a name from [`Self::name`] stands for.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        [
            Self::Fill,
            Self::Beside,
            Self::Below,
            Self::Side,
            Self::Footer,
        ]
        .into_iter()
        .find(|place| place.name() == name)
    }
}

/// A composition of views over one [`State`].
#[derive(Default)]
pub struct Picker {
    slots: Vec<Slot>,
}

impl Picker {
    /// A picker with no views.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a view at `place`. Views in one place are laid out in the order they were added.
    #[must_use]
    pub fn with(mut self, view: impl View + 'static, place: Place) -> Self {
        self.slots.push(Slot {
            view: Box::new(view),
            place,
        });
        self
    }

    /// Add an already boxed view.
    pub fn push(&mut self, view: Box<dyn View>, place: Place) {
        self.slots.push(Slot { view, place });
    }

    /// The views, in the order they were added.
    #[must_use]
    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// One of the ready-made compositions.
    #[must_use]
    pub fn preset(preset: Preset, config: &Config) -> Self {
        Self::compose(&preset.views(), config)
    }

    /// A picker of the views a host's config names, in that order.
    #[must_use]
    pub fn compose(views: &[(Kind, Place)], config: &Config) -> Self {
        let mut picker = Self::new();
        for (kind, place) in views {
            picker.push(kind.view(config), *place);
        }
        picker
    }

    /// Where each view is drawn in `area`, as `(slot index, rect)`. A view not shown, not on the
    /// current tab, or with no room left gets no rect. No two rects overlap and every one is inside
    /// `area`.
    #[must_use]
    pub fn layout(&self, area: Rect, state: &State, config: &Config) -> Vec<(usize, Rect)> {
        let mut placed = Vec::new();
        let shown = |at: usize| self.slots[at].view.shown(state, config);
        let of = |place: Place| -> Vec<usize> {
            (0..self.slots.len())
                .filter(|at| self.slots[*at].place == place && shown(*at))
                .collect()
        };
        let mut rest = area;

        if let Some(&footer) = of(Place::Footer).first()
            && rest.height >= 2
        {
            {
                placed.push((footer, Rect::new(rest.x, rest.bottom() - 1, rest.width, 1)));
                rest.height -= 1;
            }
        }

        let side = of(Place::Side);
        if !side.is_empty() {
            let width = config.side_width.min(rest.width / 2);
            if width > 0 {
                let column = Rect::new(rest.right() - width, rest.y, width, rest.height);
                rest.width -= width;
                stack(&self.slots, &side, column, state, config, &mut placed);
            }
        }

        let below = of(Place::Below);
        if !below.is_empty() {
            let wanted: u16 = below
                .iter()
                .map(|at| self.slots[*at].view.size(state, config).height)
                .sum();
            let height = wanted.min(rest.height / 2);
            if height > 0 {
                let band = Rect::new(rest.x, rest.bottom() - height, rest.width, height);
                rest.height -= height;
                stack(&self.slots, &below, band, state, config, &mut placed);
            }
        }

        let beside = of(Place::Beside);
        for at in beside.iter().rev() {
            let width = self.slots[*at]
                .view
                .size(state, config)
                .width
                .max(1)
                .min(rest.width / 2);
            if width == 0 {
                continue;
            }
            placed.push((
                *at,
                Rect::new(rest.right() - width, rest.y, width, rest.height),
            ));
            rest.width -= width;
        }

        let fill = of(Place::Fill);
        if config.tabs && fill.len() > 1 {
            if rest.height >= 2 {
                let tab = state.tab.min(fill.len() - 1);
                placed.push((
                    fill[tab],
                    Rect::new(rest.x, rest.y + 1, rest.width, rest.height - 1),
                ));
            }
        } else {
            stack(&self.slots, &fill, rest, state, config, &mut placed);
        }
        placed
    }

    /// The rect of the tab strip, while [`Config::tabs`] has more than one fill view to switch.
    fn tab_strip(&self, area: Rect, state: &State, config: &Config) -> Option<(Rect, Vec<usize>)> {
        let fill: Vec<usize> = (0..self.slots.len())
            .filter(|at| {
                self.slots[*at].place == Place::Fill && self.slots[*at].view.shown(state, config)
            })
            .collect();
        if !config.tabs || fill.len() < 2 {
            return None;
        }
        let layout = self.layout(area, state, config);
        let (_, rect) = layout.iter().find(|(at, _)| fill.contains(at))?;
        Some((Rect::new(rect.x, rect.y - 1, rect.width, 1), fill))
    }

    /// Draw every view.
    pub fn draw(
        &self,
        area: Rect,
        buffer: &mut Buffer,
        state: &State,
        config: &Config,
        palette: &Palette,
    ) {
        let area = area.intersection(buffer.area);
        for (at, rect) in self.layout(area, state, config) {
            let context = Context {
                palette,
                config,
                focused: state.focus == at,
            };
            self.slots[at].view.draw(rect, buffer, state, &context);
        }
        if let Some((strip, fill)) = self.tab_strip(area, state, config) {
            let titles: Vec<crate::tabs::Tab<'_>> = fill
                .iter()
                .map(|at| crate::tabs::Tab::new(self.slots[*at].view.title()))
                .collect();
            let styles = crate::tabs::Styles::from_palette(palette);
            let line = crate::tabs::strip(&titles, state.tab.min(fill.len() - 1), &styles).line();
            buffer.set_line(strip.x, strip.y, &line, strip.width);
        }
    }

    /// A press or a drag at `(column, row)`: focus moves to the view under it, and that view picks.
    pub fn press(
        &mut self,
        area: Rect,
        column: u16,
        row: u16,
        state: &mut State,
        config: &Config,
    ) -> Outcome {
        if let Some((strip, fill)) = self.tab_strip(area, state, config)
            && row == strip.y
            && column >= strip.x
            && column < strip.right()
        {
            let titles: Vec<crate::tabs::Tab<'_>> = fill
                .iter()
                .map(|at| crate::tabs::Tab::new(self.slots[*at].view.title()))
                .collect();
            // The ranges a strip reports depend on the labels' widths alone, never on the styles.
            let styles = crate::tabs::Styles::from_palette(&crate::theme::DEFAULT_DARK);
            let drawn = crate::tabs::strip(&titles, state.tab, &styles);
            if let Some(tab) = drawn.at(column - strip.x) {
                state.tab = tab;
                state.focus = fill[tab];
                return Outcome::Handled;
            }
            return Outcome::Ignored;
        }
        for (at, rect) in self.layout(area, state, config) {
            if contains(rect, column, row) {
                state.focus = at;
                state.hover = None;
                return if self.slots[at].view.press(rect, column, row, state, config) {
                    Outcome::Changed
                } else {
                    Outcome::Handled
                };
            }
        }
        Outcome::Ignored
    }

    /// The pointer resting at `(column, row)`: what a press would pick there is held as the hover,
    /// which the views showing a hex print.
    pub fn hover(
        &self,
        area: Rect,
        column: u16,
        row: u16,
        state: &mut State,
        config: &Config,
    ) -> Option<Rgb> {
        let found = self
            .layout(area, state, config)
            .into_iter()
            .find(|(_, rect)| contains(*rect, column, row))
            .and_then(|(at, rect)| self.slots[at].view.at(rect, column, row, state, config))
            .map(|rgb| config.depth.shown(rgb));
        state.hover = found;
        found
    }

    /// A key. The focused view takes it first; what it leaves, the picker answers: focus, the space
    /// switch, copy, accept and cancel.
    pub fn key(&mut self, key: KeyEvent, state: &mut State, config: &Config) -> Outcome {
        let focusable: Vec<usize> = (0..self.slots.len())
            .filter(|at| {
                self.slots[*at].view.focusable() && self.slots[*at].view.shown(state, config)
            })
            .collect();
        if !focusable.contains(&state.focus) {
            state.focus = focusable.first().copied().unwrap_or(0);
        }
        if let Some(slot) = self.slots.get_mut(state.focus) {
            let outcome = slot.view.key(key, state, config);
            if outcome != Outcome::Ignored {
                return outcome;
            }
        }
        let keys = &config.keys;
        let code = key.code;
        if Keys::hit(&keys.focus_next, code) || Keys::hit(&keys.focus_previous, code) {
            if focusable.is_empty() {
                return Outcome::Ignored;
            }
            let at = focusable
                .iter()
                .position(|candidate| *candidate == state.focus)
                .unwrap_or(0);
            let step = if Keys::hit(&keys.focus_next, code) {
                1
            } else {
                focusable.len() - 1
            };
            state.focus = focusable[(at + step) % focusable.len()];
            if let Some(tab) = self.fill_index(state.focus, state, config) {
                state.tab = tab;
            }
            return Outcome::Handled;
        }
        if Keys::hit(&keys.switch_space, code)
            && key.modifiers - KeyModifiers::SHIFT == KeyModifiers::NONE
        {
            state.space = state.space.next();
            return Outcome::Handled;
        }
        if Keys::hit(&keys.copy, code) {
            return Outcome::Copied(format(config.depth.shown(state.rgb()), &config.format));
        }
        if Keys::hit(&keys.accept, code) {
            if let Choice::Colour(rgb) = state.choice() {
                state.set_rgb(config.depth.shown(rgb));
            }
            return Outcome::Accepted(state.choice());
        }
        if Keys::hit(&keys.cancel, code) {
            let original = state.original;
            state.set_rgb(original);
            return Outcome::Cancelled;
        }
        Outcome::Ignored
    }

    fn fill_index(&self, at: usize, state: &State, config: &Config) -> Option<usize> {
        (0..self.slots.len())
            .filter(|candidate| {
                self.slots[*candidate].place == Place::Fill
                    && self.slots[*candidate].view.shown(state, config)
            })
            .position(|candidate| candidate == at)
    }
}

fn stack(
    slots: &[Slot],
    order: &[usize],
    area: Rect,
    state: &State,
    config: &Config,
    placed: &mut Vec<(usize, Rect)>,
) {
    if order.is_empty() || area.is_empty() {
        return;
    }
    // Every view but the last gets its own height; the last takes what is left, so a fill view
    // under a fixed-height one grows with the room.
    let mut y = area.y;
    for (position, at) in order.iter().enumerate() {
        let remaining = area.bottom().saturating_sub(y);
        if remaining == 0 {
            break;
        }
        let last = position + 1 == order.len();
        let wanted = slots[*at].view.size(state, config).height.max(1);
        let height = if last {
            remaining
        } else {
            wanted.min(remaining)
        };
        placed.push((*at, Rect::new(area.x, y, area.width, height)));
        y += height;
    }
}

pub(crate) fn contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
}

/// The text a hint row prints for a list of bindings and what they do.
pub(crate) fn hint(text: &mut String, keys: &[KeyCode], what: &str) {
    if keys.is_empty() {
        return;
    }
    if !text.is_empty() {
        text.push_str("  ");
    }
    let _ = write!(text, "{} {what}", Keys::label(keys));
}
