//! A colour picker with a view of the host's own, added from outside the crate.
//!
//! `Complement` is not a view tuilith ships: it draws the colour opposite the chosen one and picks it
//! when pressed. It reads and writes the same shared state as the views beside it, so pressing it
//! moves the field's marker and the panel's hex in the same frame.
//!
//! Run with `cargo run --example colour`; it draws the picker once, presses the complement, and
//! prints both frames with their colours.

use std::fmt::Write as _;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use tuilith::colour::{
    Config, Context, Hsv, Outcome, Picker, Place, Preset, Rgb, Size, State, View,
};

/// The colour across the wheel from the chosen one.
struct Complement;

impl Complement {
    fn of(state: &State) -> Rgb {
        let hsv = state.hsv();
        Hsv {
            h: hsv.h + 180.0,
            ..hsv
        }
        .rgb()
    }
}

impl View for Complement {
    fn title(&self) -> &'static str {
        "complement"
    }

    fn size(&self, _state: &State, _config: &Config) -> Size {
        Size {
            width: 20,
            height: 2,
        }
    }

    fn draw(&self, area: Rect, buffer: &mut Buffer, state: &State, context: &Context) {
        let dim = Style::default().fg(context.palette.dim);
        buffer.set_stringn(area.x, area.y, "complement", area.width as usize, dim);
        if area.height > 1 {
            let swatch = Rect::new(area.x, area.y + 1, area.width.min(8), 1);
            let colour = context.config.depth.color(Self::of(state));
            buffer.set_style(swatch, Style::default().bg(colour));
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
        let swatch = Rect::new(area.x, area.y + 1, area.width.min(8), 1);
        (row == swatch.y && column >= swatch.x && column < swatch.right()).then(|| Self::of(state))
    }

    fn key(&mut self, key: KeyEvent, state: &mut State, _config: &Config) -> Outcome {
        if key.code == KeyCode::Char('c') {
            state.set_rgb(Self::of(state));
            return Outcome::Changed;
        }
        Outcome::Ignored
    }
}

fn print(buffer: &Buffer) {
    for y in buffer.area.y..buffer.area.bottom() {
        let mut line = String::new();
        for x in buffer.area.x..buffer.area.right() {
            let cell = &buffer[(x, y)];
            if let (Color::Rgb(fr, fg, fb), Color::Rgb(br, bg, bb)) = (cell.fg, cell.bg) {
                let _ = write!(line, "\x1b[38;2;{fr};{fg};{fb};48;2;{br};{bg};{bb}m");
            } else if let Color::Rgb(br, bg, bb) = cell.bg {
                let _ = write!(line, "\x1b[0;48;2;{br};{bg};{bb}m");
            } else {
                line.push_str("\x1b[0m");
            }
            line.push_str(cell.symbol());
        }
        println!("{line}\x1b[0m");
    }
}

fn main() {
    let config = Config::default();
    let mut picker = Picker::preset(Preset::Full, &config);
    picker.push(Box::new(Complement), Place::Below);
    let mut state = State::new(Rgb::new(0x5c, 0xbd, 0xfa));
    state.recent = vec![Rgb::new(0xff, 0x8a, 0x33), Rgb::new(0x8c, 0xd9, 0x64)];

    let area = Rect::new(0, 0, 72, 26);
    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, &state, &config, &tuilith::DEFAULT_DARK);
    print(&buffer);

    // Press the complement's swatch wherever the layout put it.
    let (_, rect) = picker
        .layout(area, &state, &config)
        .into_iter()
        .find(|(at, _)| picker.slots()[*at].view.title() == "complement")
        .expect("the complement is placed");
    let outcome = picker.press(area, rect.x, rect.y + 1, &mut state, &config);
    println!(
        "pressed the complement: {outcome:?}, now {}",
        state.rgb().hex()
    );

    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, &state, &config, &tuilith::DEFAULT_DARK);
    print(&buffer);
}
