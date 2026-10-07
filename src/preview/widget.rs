//! Drawing a preview: the picture when it is ready, a placeholder while it is not, a card when there
//! can be no picture.
//!
//! Three states, one widget, so a pane never has to decide what to draw in the frame between asking
//! for a picture and getting it:
//!
//! - **Picture** — the cached encoding, centred in the area. Drawing it is free: it was encoded once,
//!   and a kitty picture was transmitted once.
//! - **Pending** — a box with a spinner and the title, while the [`Worker`](super::Worker) encodes.
//!   Also what an encoding too big for the area draws, since a fresh one for the new size is what
//!   should be on its way.
//! - **Card** — the title, the picture's size, the file's size and format, and a hint line from the
//!   caller (`o open externally`). Drawn when the protocol is [`Protocol::None`], and in halfblocks
//!   mode when the caller asks for it, since halfblocks cannot show text in a picture legibly.
//!
//! Every cell of the area is painted from the palette, so nothing from the frame before shows through.
//! The widget is not interactive itself; [`Drawn`] reports where the picture and the hint landed, so a
//! caller that makes them clickable registers the rects that were actually drawn.

use std::io::Cursor;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Widget};

use super::cache::Encoded;
use super::graphics::Protocol;
use crate::theme::Palette;

crate::provenance! {
    component: "preview::widget",
    about: "Draws a cached picture, a pending box with a spinner, or a metadata card where pictures cannot be shown",
    origin: crate::Origin::Here,
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// The spinner's frames, one cell wide each.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// What a card says about a picture. Every fact is optional: a card says what is known.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// What it is called — usually the file name.
    pub title: String,
    /// Its size in pixels.
    pub pixels: Option<(u32, u32)>,
    /// The file's size in bytes.
    pub bytes: Option<u64>,
    /// Its format, as a short upper-case name (`PNG`).
    pub format: Option<String>,
}

impl Meta {
    /// What a file's bytes say about it, reading only the header — no decode.
    ///
    /// The format and size come from whichever codecs `image` was built with; a format none of them
    /// knows leaves both unknown.
    #[must_use]
    pub fn sniff(title: impl Into<String>, bytes: &[u8]) -> Self {
        let reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok();
        let format = reader
            .as_ref()
            .and_then(image::ImageReader::format)
            .and_then(|format| format.extensions_str().first())
            .map(|extension| extension.to_ascii_uppercase());
        let pixels = reader.and_then(|reader| reader.into_dimensions().ok());
        Self {
            title: title.into(),
            pixels,
            bytes: u64::try_from(bytes.len()).ok(),
            format,
        }
    }

    /// The facts under the title — `640×480 px · 1.5 KB · PNG` — leaving out what is unknown.
    #[must_use]
    pub fn facts(&self) -> String {
        let mut facts = Vec::new();
        if let Some((width, height)) = self.pixels {
            facts.push(format!("{width}×{height} px"));
        }
        if let Some(bytes) = self.bytes {
            facts.push(human_bytes(bytes));
        }
        if let Some(format) = &self.format {
            facts.push(format.clone());
        }
        facts.join(" · ")
    }
}

/// A byte count as a person reads it: `512 B`, `1.5 KB`, `12.0 MB`.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    // Tenths of a unit, in integers: no float to cast back and no precision to lose.
    let mut tenths = u128::from(bytes) * 10 / 1024;
    let mut unit = 0;
    while tenths >= 10 * 1024 && unit + 1 < UNITS.len() {
        tenths /= 1024;
        unit += 1;
    }
    format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[unit])
}

/// Which of the three states a widget draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shows {
    /// The encoded picture.
    Picture,
    /// The placeholder, while the picture is encoded.
    Pending,
    /// The metadata card.
    Card,
}

/// Where things landed, as drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drawn {
    /// The cells the picture covers, when a picture was drawn.
    pub picture: Option<Rect>,
    /// The cells the card's hint line covers, when a card with a hint was drawn.
    pub hint: Option<Rect>,
}

/// One preview. See the module docs.
pub struct Image<'a> {
    meta: &'a Meta,
    protocol: Protocol,
    palette: Palette,
    encoded: Option<&'a Encoded>,
    tick: usize,
    card_in_halfblocks: bool,
    hint: Option<&'a str>,
}

impl<'a> Image<'a> {
    /// A preview of `meta` in `protocol`, drawn from `palette`. Pending until it is given an encoding.
    #[must_use]
    pub fn new(meta: &'a Meta, protocol: Protocol, palette: Palette) -> Self {
        Self {
            meta,
            protocol,
            palette,
            encoded: None,
            tick: 0,
            card_in_halfblocks: false,
            hint: None,
        }
    }

    /// The encoded picture, when the cache has it.
    #[must_use]
    pub fn encoded(mut self, encoded: Option<&'a Encoded>) -> Self {
        self.encoded = encoded;
        self
    }

    /// Which spinner frame to draw — the application's tick count, so the spinner moves.
    #[must_use]
    pub fn tick(mut self, tick: usize) -> Self {
        self.tick = tick;
        self
    }

    /// Draw the card instead of a halfblocks picture.
    #[must_use]
    pub fn card_in_halfblocks(mut self, card: bool) -> Self {
        self.card_in_halfblocks = card;
        self
    }

    /// The card's last line, telling the reader what they can do (`o open externally`).
    #[must_use]
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Which state this draws, before knowing the area.
    #[must_use]
    pub fn shows(&self) -> Shows {
        let card_mode = !self.protocol.draws_pictures()
            || (self.card_in_halfblocks && self.protocol == Protocol::Halfblocks);
        if card_mode {
            Shows::Card
        } else if self.encoded.is_some() {
            Shows::Picture
        } else {
            Shows::Pending
        }
    }

    /// Draw into `area` of `buffer`, and say where things landed.
    pub fn draw(self, buffer: &mut Buffer, area: Rect) -> Drawn {
        let area = area.intersection(buffer.area);
        Clear.render(area, buffer);
        buffer.set_style(
            area,
            Style::new()
                .bg(self.palette.background)
                .fg(self.palette.foreground),
        );
        match (self.shows(), self.encoded) {
            (Shows::Picture, Some(encoded)) => {
                if let Some(at) = centred(area, encoded.cells()) {
                    ratatui_image::Image::new(encoded.encoding()).render(at, buffer);
                    return Drawn {
                        picture: Some(at),
                        hint: None,
                    };
                }
                self.draw_pending(buffer, area);
                Drawn::default()
            }
            (Shows::Card, _) => self.draw_card(buffer, area),
            _ => {
                self.draw_pending(buffer, area);
                Drawn::default()
            }
        }
    }

    fn frame(&self, buffer: &mut Buffer, area: Rect) -> Rect {
        let block = Block::bordered().border_style(Style::new().fg(self.palette.border));
        let inner = block.inner(area);
        block.render(area, buffer);
        inner
    }

    fn draw_pending(&self, buffer: &mut Buffer, area: Rect) {
        let inner = self.frame(buffer, area);
        let spinner = SPINNER[self.tick % SPINNER.len()];
        let line = Line::from(vec![
            Span::styled(spinner, Style::new().fg(self.palette.accent)),
            Span::styled(" loading ", Style::new().fg(self.palette.dim)),
            Span::styled(
                self.meta.title.as_str(),
                Style::new().fg(self.palette.foreground),
            ),
        ]);
        draw_lines(buffer, inner, &[line]);
    }

    fn draw_card(&self, buffer: &mut Buffer, area: Rect) -> Drawn {
        let inner = self.frame(buffer, area);
        let mut lines = vec![Line::from(Span::styled(
            self.meta.title.as_str(),
            Style::new()
                .fg(self.palette.foreground)
                .add_modifier(Modifier::BOLD),
        ))];
        let facts = self.meta.facts();
        if !facts.is_empty() {
            lines.push(Line::from(Span::styled(
                facts,
                Style::new().fg(self.palette.dim),
            )));
        }
        if let Some(hint) = self.hint {
            lines.push(Line::from(Span::styled(
                hint,
                Style::new().fg(self.palette.dim),
            )));
        }
        let placed = draw_lines(buffer, inner, &lines);
        Drawn {
            picture: None,
            hint: placed
                .last()
                .copied()
                .flatten()
                .filter(|_| self.hint.is_some()),
        }
    }
}

impl Widget for Image<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let _ = self.draw(buffer, area);
    }
}

/// A box of `cells` centred in `area`, when it fits.
fn centred(area: Rect, (columns, rows): (u16, u16)) -> Option<Rect> {
    if columns == 0 || rows == 0 || columns > area.width || rows > area.height {
        return None;
    }
    Some(Rect::new(
        area.x + (area.width - columns) / 2,
        area.y + (area.height - rows) / 2,
        columns,
        rows,
    ))
}

/// Draw `lines` centred in `area`, each on its own row, and return the rect each landed in — `None`
/// for a line there was no row left for.
fn draw_lines(buffer: &mut Buffer, area: Rect, lines: &[Line<'_>]) -> Vec<Option<Rect>> {
    let count = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let top = area.y + area.height.saturating_sub(count) / 2;
    lines
        .iter()
        .zip(0_u16..)
        .map(|(line, offset)| {
            let row = top + offset;
            if area.width == 0 || row >= area.bottom() {
                return None;
            }
            let width = u16::try_from(line.width())
                .unwrap_or(u16::MAX)
                .min(area.width);
            let column = area.x + (area.width - width) / 2;
            buffer.set_line(column, row, line, width);
            Some(Rect::new(column, row, width, 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, Rgba, RgbaImage};
    use ratatui::layout::Size;
    use ratatui_image::Resize;

    use super::*;
    use crate::inspect;
    use crate::preview::graphics::CellSize;
    use crate::preview::picker::picker;
    use crate::theme::Mode;

    fn meta() -> Meta {
        Meta {
            title: "harbour.png".to_owned(),
            pixels: Some((640, 480)),
            bytes: Some(1536),
            format: Some("PNG".to_owned()),
        }
    }

    fn row_text(buffer: &Buffer, row: u16) -> String {
        (buffer.area.left()..buffer.area.right())
            .map(|column| buffer[(column, row)].symbol())
            .collect()
    }

    fn text_of(buffer: &Buffer, rect: Rect) -> String {
        (rect.left()..rect.right())
            .map(|column| buffer[(column, rect.y)].symbol())
            .collect()
    }

    /// The checks every state owes in both variants: painted, no colour from the other variant, and
    /// readable text inside the frame.
    fn assert_themed(buffer: &Buffer, area: Rect, mode: Mode) {
        inspect::fully_painted(buffer, area).unwrap_or_else(|unpainted| panic!("{unpainted}"));
        inspect::no_leak(buffer, area, mode.inverse().palette())
            .unwrap_or_else(|leak| panic!("{leak}"));
        // Inside the border: a border is deliberately quiet, the text is not.
        let inside = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);
        let verdict = inspect::readable(buffer, inside, mode.palette(), 3.0);
        assert!(verdict.failed.is_empty(), "{verdict}");
    }

    #[test]
    fn the_pending_box_is_painted_and_readable_in_both_variants() {
        for mode in [Mode::Dark, Mode::Light] {
            let palette = mode.palette();
            let area = Rect::new(0, 0, 40, 7);
            let mut buffer = Buffer::empty(area);
            let meta = meta();
            let image = Image::new(&meta, Protocol::Kitty, palette).tick(3);
            assert_eq!(image.shows(), Shows::Pending);
            let drawn = image.draw(&mut buffer, area);
            assert_eq!(drawn, Drawn::default());
            assert_themed(&buffer, area, mode);

            let middle = row_text(&buffer, 3);
            assert!(middle.contains("loading harbour.png"), "{middle:?}");
            let spinner_column = (0_u16..40)
                .find(|column| buffer[(*column, 3)].symbol() == SPINNER[3])
                .expect("the spinner frame for tick 3 is drawn");
            assert_eq!(buffer[(spinner_column, 3)].fg, palette.accent);
            assert_eq!(buffer[(0, 0)].fg, palette.border, "{mode:?} frame");
        }
    }

    #[test]
    fn the_card_is_painted_readable_and_reports_where_its_hint_landed() {
        for mode in [Mode::Dark, Mode::Light] {
            let palette = mode.palette();
            let area = Rect::new(2, 1, 44, 9);
            let mut buffer = Buffer::empty(Rect::new(0, 0, 50, 12));
            let meta = meta();
            let image = Image::new(&meta, Protocol::None, palette).hint("o open externally");
            assert_eq!(image.shows(), Shows::Card);
            let drawn = image.draw(&mut buffer, area);
            assert_themed(&buffer, area, mode);

            let hint = drawn
                .hint
                .expect("a card with a hint reports where it went");
            assert_eq!(text_of(&buffer, hint), "o open externally");
            assert!(area.contains(hint.as_position()));
            assert_eq!(buffer[(hint.x, hint.y)].fg, palette.dim);

            let rows: Vec<String> = (area.top()..area.bottom())
                .map(|row| row_text(&buffer, row))
                .collect();
            assert!(rows.iter().any(|row| row.contains("harbour.png")));
            assert!(
                rows.iter()
                    .any(|row| row.contains("640×480 px · 1.5 KB · PNG")),
                "{rows:#?}"
            );
            // Outside the area nothing was touched.
            assert_eq!(buffer[(0, 0)].bg, ratatui::style::Color::Reset);
        }
    }

    #[test]
    fn halfblocks_draws_the_card_only_when_asked() {
        let meta = meta();
        let palette = Mode::Dark.palette();
        assert_eq!(
            Image::new(&meta, Protocol::Halfblocks, palette).shows(),
            Shows::Pending
        );
        assert_eq!(
            Image::new(&meta, Protocol::Halfblocks, palette)
                .card_in_halfblocks(true)
                .shows(),
            Shows::Card
        );
        assert_eq!(
            Image::new(&meta, Protocol::Sixel, palette)
                .card_in_halfblocks(true)
                .shows(),
            Shows::Pending,
            "the request is about halfblocks only"
        );
    }

    fn halfblocks(columns: u16, rows: u16) -> Encoded {
        let cell = CellSize {
            width: 10,
            height: 20,
        };
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            u32::from(columns) * 10,
            u32::from(rows) * 20,
            Rgba([250, 200, 20, 255]),
        ));
        let encoding = picker(Protocol::Halfblocks, cell)
            .expect("halfblocks has an encoder")
            .new_protocol(image, Size::new(columns, rows), Resize::Fit(None))
            .expect("a synthetic picture encodes");
        Encoded::new(
            encoding,
            cell,
            (u32::from(columns) * 10, u32::from(rows) * 20),
            None,
        )
    }

    #[test]
    fn a_ready_picture_is_drawn_centred_and_reported() {
        let meta = meta();
        let encoded = halfblocks(6, 2);
        let area = Rect::new(0, 0, 20, 8);
        let mut buffer = Buffer::empty(area);
        let image =
            Image::new(&meta, Protocol::Halfblocks, Mode::Dark.palette()).encoded(Some(&encoded));
        assert_eq!(image.shows(), Shows::Picture);
        let drawn = image.draw(&mut buffer, area);
        assert_eq!(drawn.picture, Some(Rect::new(7, 3, 6, 2)));
        let painted = buffer[(7, 3)].fg;
        assert_ne!(
            painted,
            Mode::Dark.palette().foreground,
            "the picture's cells kept the theme's colour"
        );
    }

    #[test]
    fn a_picture_too_big_for_the_area_waits_as_pending() {
        let meta = meta();
        let encoded = halfblocks(30, 4);
        let area = Rect::new(0, 0, 20, 8);
        let mut buffer = Buffer::empty(area);
        let drawn = Image::new(&meta, Protocol::Halfblocks, Mode::Light.palette())
            .encoded(Some(&encoded))
            .draw(&mut buffer, area);
        assert_eq!(drawn.picture, None);
        assert!(row_text(&buffer, 3).contains("loading"));
        assert_themed(&buffer, area, Mode::Light);
    }

    #[test]
    fn bytes_read_the_way_a_person_says_them() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(human_bytes(u64::MAX), "16777215.9 TB");
    }

    #[test]
    fn a_header_is_enough_to_know_the_size_and_format() {
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(12, 7, Rgba([0, 0, 0, 255])))
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("a synthetic picture encodes as PNG");
        let meta = Meta::sniff("dot.png", &bytes);
        assert_eq!(meta.pixels, Some((12, 7)));
        assert_eq!(meta.format.as_deref(), Some("PNG"));
        assert_eq!(meta.bytes, u64::try_from(bytes.len()).ok());

        let unknown = Meta::sniff("notes.txt", b"just words");
        assert_eq!(unknown.pixels, None);
        assert_eq!(unknown.format, None);
        assert_eq!(unknown.facts(), "10 B");
    }
}
