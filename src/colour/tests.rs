//! Tests for the colour picker.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::views::{Channel, Field, Panel, Sliders, Strip, Swatches};
use super::*;
use crate::theme::DEFAULT_DARK;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn typed(picker: &mut Picker, state: &mut State, config: &Config, text: &str) {
    for character in text.chars() {
        picker.key(key(KeyCode::Char(character)), state, config);
    }
}

fn entries() -> Vec<Entry> {
    vec![
        Entry::new("accent", Some(Rgb::new(0x33, 0x66, 0xff))).detail("#3366ff"),
        Entry::new("warn", Some(Rgb::new(0xfb, 0xc0, 0x2d))).detail("#fbc02d"),
        Entry::new("red", Some(Rgb::new(0xf3, 0x56, 0x5d))),
        Entry::new("default", None),
    ]
}

fn row_text(buffer: &Buffer, y: u16) -> String {
    (buffer.area.x..buffer.area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

fn frame_text(buffer: &Buffer) -> String {
    (buffer.area.y..buffer.area.bottom())
        .map(|y| row_text(buffer, y))
        .collect::<Vec<_>>()
        .join("\n")
}

// --- typed text -------------------------------------------------------------------------------------

#[test]
fn every_written_form_parses_to_the_colour_it_names() {
    let cases = [
        ("#5cbdfa", Rgb::new(0x5c, 0xbd, 0xfa)),
        ("5CBDFA", Rgb::new(0x5c, 0xbd, 0xfa)),
        ("#f80", Rgb::new(0xff, 0x88, 0x00)),
        ("#208", Rgb::new(0x22, 0x00, 0x88)),
        ("rgb(92, 189, 250)", Rgb::new(92, 189, 250)),
        ("hsl(0, 100%, 50%)", Rgb::new(255, 0, 0)),
        ("hsl(120, 100%, 25%)", Rgb::new(0, 128, 0)),
        ("light blue", Rgb::new(0x5c, 0x5c, 0xff)),
        ("dark_grey", Rgb::new(0x7f, 0x7f, 0x7f)),
    ];
    for (text, expected) in cases {
        assert_eq!(parse(text, &[]), Ok(Choice::Colour(expected)), "{text}");
    }
}

#[test]
fn text_that_is_no_colour_is_refused_with_what_is_accepted() {
    // `208` is a number to many hosts, so three bare digits are not a colour without their `#`.
    for text in [
        "208",
        "f80",
        "#12",
        "#12345",
        "#gggggg",
        "rgb(300, 0, 0)",
        "hsl(1, 200%, 5%)",
        "teal-ish",
        "",
    ] {
        let refused = parse(text, &[]).expect_err(text);
        assert!(refused.0.contains("#rrggbb"), "{text}: {}", refused.0);
    }
}

#[test]
fn a_hosts_name_wins_over_a_built_in_one_and_ignores_case() {
    let entries = entries();
    assert_eq!(parse("RED", &entries), Ok(Choice::Named(2)));
    assert_eq!(parse("Accent", &entries), Ok(Choice::Named(0)));
    // Without the host's entry, `red` is the terminal's.
    assert_eq!(parse("red", &[]), Ok(Choice::Colour(xterm(1))));
}

#[test]
fn format_writes_each_form_and_case() {
    let rgb = Rgb::new(0xff, 0x88, 0x00);
    let hex = Format::default();
    assert_eq!(format(rgb, &hex), "#ff8800");
    assert_eq!(format(rgb, &Format { upper: true, ..hex }), "#FF8800");
    assert_eq!(format(rgb, &Format { short: true, ..hex }), "#f80");
    // A colour with no short form keeps its long one.
    assert_eq!(
        format(Rgb::new(0x12, 0x34, 0x56), &Format { short: true, ..hex }),
        "#123456"
    );
    assert_eq!(
        format(
            rgb,
            &Format {
                form: Form::Rgb,
                ..hex
            }
        ),
        "rgb(255, 136, 0)"
    );
    assert_eq!(
        format(
            Rgb::new(255, 0, 0),
            &Format {
                form: Form::Hsl,
                ..hex
            }
        ),
        "hsl(0, 100%, 50%)"
    );
}

#[test]
fn the_three_spaces_round_trip_through_rgb() {
    for rgb in [
        Rgb::new(0, 0, 0),
        Rgb::new(255, 255, 255),
        Rgb::new(0x5c, 0xbd, 0xfa),
        Rgb::new(0x89, 0xb4, 0xfa),
        Rgb::new(200, 10, 90),
        Rgb::new(12, 200, 64),
    ] {
        assert_eq!(rgb.hsv().rgb(), rgb, "hsv {rgb:?}");
        assert_eq!(rgb.hsl().rgb(), rgb, "hsl {rgb:?}");
        assert_eq!(rgb.hsl().hsv().rgb(), rgb, "hsl to hsv {rgb:?}");
    }
    // Worked example from the reference picture: #5cbdfa is hsl 203 94% 67%.
    let hsl = Rgb::new(0x5c, 0xbd, 0xfa).hsl();
    assert_eq!(
        (
            hsl.h.round(),
            (hsl.s * 100.0).round(),
            (hsl.l * 100.0).round()
        ),
        (203.0, 94.0, 67.0)
    );
}

#[test]
fn going_grey_keeps_the_hue_a_drag_back_out_will_need() {
    let mut state = State::new(Rgb::new(0x33, 0x66, 0xff));
    let hue = state.hsv().h;
    state.set_rgb(Rgb::new(128, 128, 128));
    assert!((state.hsv().h - hue).abs() < 0.01);
    state.set_rgb(Rgb::new(255, 0, 0));
    assert!(
        state.hsv().h.abs() < 0.01,
        "a coloured colour brings its own hue"
    );
}

// --- depth ------------------------------------------------------------------------------------------

#[test]
fn a_shallow_terminal_is_drawn_and_told_the_colour_it_can_show() {
    let rgb = Rgb::new(0x5c, 0xbd, 0xfa);
    assert_eq!(Depth::TrueColor.shown(rgb), rgb);
    assert_eq!(Depth::TrueColor.color(rgb), Color::Rgb(0x5c, 0xbd, 0xfa));

    let Color::Indexed(index) = Depth::Ansi256.color(rgb) else {
        panic!("256 colours draw an index");
    };
    assert_eq!(Depth::Ansi256.shown(rgb), xterm(index));
    // The cube's nearest step to #5cbdfa is #5fafff (index 75).
    assert_eq!(index, 75);
    assert_eq!(xterm(75), Rgb::new(0x5f, 0xaf, 0xff));

    let shown = Depth::Ansi16.shown(Rgb::new(250, 10, 10));
    assert_eq!(shown, Rgb::new(255, 0, 0));
    assert_eq!(Depth::Ansi16.color(Rgb::new(250, 10, 10)), Color::LightRed);
}

#[test]
fn the_accepted_colour_is_the_one_a_shallow_terminal_drew() {
    let config = Config {
        depth: Depth::Ansi256,
        ..Config::default()
    };
    let mut picker = Picker::preset(Preset::Full, &config);
    let mut state = State::new(Rgb::new(0x5c, 0xbd, 0xfa));
    let outcome = picker.key(key(KeyCode::Enter), &mut state, &config);
    assert_eq!(
        outcome,
        Outcome::Accepted(Choice::Colour(Rgb::new(0x5f, 0xaf, 0xff)))
    );

    let truecolor = Config::default();
    let mut state = State::new(Rgb::new(0x5c, 0xbd, 0xfa));
    let outcome = picker.key(key(KeyCode::Enter), &mut state, &truecolor);
    assert_eq!(
        outcome,
        Outcome::Accepted(Choice::Colour(Rgb::new(0x5c, 0xbd, 0xfa)))
    );
}

#[test]
fn remember_puts_the_newest_first_once_and_keeps_the_cap() {
    let (a, b, c) = (Rgb::new(1, 0, 0), Rgb::new(2, 0, 0), Rgb::new(3, 0, 0));
    let mut recent = Vec::new();
    remember(&mut recent, a, 2);
    remember(&mut recent, b, 2);
    assert_eq!(recent, [b, a]);
    remember(&mut recent, a, 2);
    assert_eq!(
        recent,
        [a, b],
        "an old colour picked again moves to the front"
    );
    let mut long = vec![b, a];
    remember(&mut long, a, 5);
    assert_eq!(long, [a, b], "once");
    remember(&mut recent, c, 2);
    assert_eq!(recent, [c, a], "the oldest falls off past the cap");
}

// --- fields -----------------------------------------------------------------------------------------

/// Draw `field` alone, then press every cell: the colour picked must be the one painted as the cell's
/// top point, read back off the buffer — never recomputed the way the field computes it.
fn press_every_cell_picks_what_was_drawn(field: &Field) {
    let config = Config {
        titles: false,
        ..Config::default()
    };
    let area = Rect::new(2, 1, 20, 8);
    let start = Rgb::new(0x5c, 0xbd, 0xfa);
    let state = State::new(start);
    let mut buffer = Buffer::empty(Rect::new(0, 0, 24, 10));
    let context = Context {
        palette: &DEFAULT_DARK,
        config: &config,
        focused: false,
    };
    field.draw(area, &mut buffer, &state, &context);
    let mut pressed = 0;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let cell = &buffer[(x, y)];
            if cell.symbol() != "▀" {
                continue;
            }
            let Color::Rgb(r, g, b) = cell.fg else {
                panic!("{x},{y} drawn in {:?}", cell.fg);
            };
            let mut probe = State::new(start);
            let mut view = field.clone();
            assert!(view.press(area, x, y, &mut probe, &config), "{x},{y}");
            assert_eq!(
                probe.rgb(),
                Rgb::new(r, g, b),
                "{:?} at {x},{y}",
                field.shape
            );
            assert_eq!(
                view.at(area, x, y, &state, &config),
                Some(Rgb::new(r, g, b)),
                "hover at {x},{y}"
            );
            pressed += 1;
        }
    }
    assert!(pressed > 40, "{:?} drew only {pressed} points", field.shape);
}

#[test]
fn a_press_on_the_square_picks_the_colour_drawn_there() {
    press_every_cell_picks_what_was_drawn(&Field::square());
}

#[test]
fn a_press_on_the_wheel_picks_the_colour_drawn_there() {
    press_every_cell_picks_what_was_drawn(&Field::wheel());
}

#[test]
fn a_press_on_the_diamond_picks_the_colour_drawn_there() {
    press_every_cell_picks_what_was_drawn(&Field::diamond());
}

#[test]
fn a_press_on_the_grid_picks_the_colour_drawn_there() {
    press_every_cell_picks_what_was_drawn(&Field::grid());
}

#[test]
fn the_wheel_is_round_and_a_press_outside_it_picks_nothing() {
    let config = Config {
        titles: false,
        ..Config::default()
    };
    let area = Rect::new(0, 0, 24, 12);
    let mut state = State::new(Rgb::new(10, 20, 30));
    let mut buffer = Buffer::empty(area);
    let context = Context {
        palette: &DEFAULT_DARK,
        config: &config,
        focused: false,
    };
    Field::wheel().draw(area, &mut buffer, &state, &context);
    // The corners are outside the circle and left alone; the centre row's middle is inside.
    assert_eq!(buffer[(0, 0)].symbol(), " ");
    assert_eq!(buffer[(23, 11)].symbol(), " ");
    assert_ne!(buffer[(6, 6)].symbol(), " ");
    let before = state.rgb();
    assert!(!Field::wheel().press(area, 0, 0, &mut state, &config));
    assert_eq!(state.rgb(), before);

    // The diamond is pointed at the top and bottom: the corners of its rect hold nothing.
    let mut buffer = Buffer::empty(area);
    Field::diamond().draw(area, &mut buffer, &state, &context);
    assert_eq!(buffer[(0, 0)].symbol(), " ");
    assert_eq!(buffer[(23, 11)].symbol(), " ");
    assert_ne!(buffer[(11, 6)].symbol(), " ");
    assert!(!Field::diamond().press(area, 0, 0, &mut state, &config));
}

#[test]
fn the_marker_sits_where_the_colour_is() {
    let config = Config {
        titles: false,
        ..Config::default()
    };
    let area = Rect::new(0, 0, 37, 10);
    // Pure red at full saturation: hue 0 is the square's left edge, saturation 1 its top row.
    let state = State::new(Rgb::new(255, 0, 0));
    let mut buffer = Buffer::empty(area);
    let context = Context {
        palette: &DEFAULT_DARK,
        config: &config,
        focused: false,
    };
    Field::square().draw(area, &mut buffer, &state, &context);
    assert_eq!(buffer[(0, 0)].symbol(), "◉");
    assert_eq!(row_text(&buffer, 0).matches('◉').count(), 1);
    // Halfway round, half saturated: the middle of the square.
    let state = State::new(
        Hsv {
            h: 180.0,
            s: 0.5,
            v: 1.0,
        }
        .rgb(),
    );
    let mut buffer = Buffer::empty(area);
    Field::square().draw(area, &mut buffer, &state, &context);
    let marked: Vec<(u16, u16)> = (0..10)
        .flat_map(|y| (0..37).map(move |x| (x, y)))
        .filter(|at| buffer[*at].symbol() == "◉")
        .collect();
    assert_eq!(marked, [(18, 5)]);
}

#[test]
fn the_arrows_walk_a_field_and_page_keys_walk_its_value() {
    let config = Config::default();
    let mut field = Field::square();
    let mut state = State::new(
        Hsv {
            h: 100.0,
            s: 0.5,
            v: 0.5,
        }
        .rgb(),
    );
    let before = state.hsv();
    assert_eq!(
        field.key(key(KeyCode::Right), &mut state, &config),
        Outcome::Changed
    );
    assert!((state.hsv().h - before.h - config.hue_step).abs() < 0.5);
    field.key(key(KeyCode::Up), &mut state, &config);
    assert!((state.hsv().s - before.s - config.step).abs() < 0.02);
    field.key(key(KeyCode::PageDown), &mut state, &config);
    assert!((state.hsv().v - before.v + config.step).abs() < 0.02);
    assert_eq!(
        field.key(key(KeyCode::Char('x')), &mut state, &config),
        Outcome::Ignored
    );
}

// --- strips, sliders, swatches ----------------------------------------------------------------------

#[test]
fn a_press_on_the_value_strip_sets_the_value_and_keeps_the_hue() {
    let config = Config {
        titles: false,
        ..Config::default()
    };
    let area = Rect::new(0, 0, 3, 10);
    let mut state = State::new(
        Hsv {
            h: 200.0,
            s: 0.8,
            v: 1.0,
        }
        .rgb(),
    );
    let mut strip = Strip::vertical(Channel::Value);
    assert!(strip.press(area, 1, 9, &mut state, &config));
    assert!(state.hsv().v < 0.1, "the bottom is dark: {:?}", state.hsv());
    assert!(strip.press(area, 1, 0, &mut state, &config));
    assert!((state.hsv().v - 1.0).abs() < 0.01);
    assert!((state.hsv().h - 200.0).abs() < 1.0);
}

#[test]
fn a_slider_press_sets_that_channel_alone() {
    let config = Config {
        titles: false,
        ..Config::default()
    };
    let area = Rect::new(0, 0, 40, 5);
    let mut state = State::new(Rgb::new(10, 20, 30));
    let mut sliders = Sliders::default();
    // The green bar is on row 2, two columns in; its right end is 255.
    let right = 2 + (40 - 2 - 5) - 1;
    assert!(sliders.press(area, right, 2, &mut state, &config));
    assert_eq!(state.rgb(), Rgb::new(10, 255, 30));
    assert!(
        !sliders.press(area, right, 1, &mut state, &config),
        "between bars"
    );
    // The space switch moves the same bars to hsl.
    state.space = Space::Hsl;
    assert!(sliders.press(area, 2, 0, &mut state, &config));
    assert_eq!(state.channels(Space::Hsl)[0], 0);
}

#[test]
fn a_swatch_press_picks_that_swatch_and_marks_it() {
    let config = Config::default();
    let area = Rect::new(0, 0, 40, 3);
    let mut state = State::new(Rgb::new(0, 0, 0));
    let mut row = Swatches::common(&config);
    // Title on row 0, light tints on row 1, strong colours on row 2; each swatch is 2 cells and a gap.
    assert!(row.press(area, 3, 2, &mut state, &config));
    assert_eq!(state.rgb(), COMMON[1][1]);
    assert_eq!(row.at(area, 4, 1, &state, &config), Some(COMMON[0][1]));
    assert_eq!(row.at(area, 5, 1, &state, &config), None, "the gap");
    let mut buffer = Buffer::empty(area);
    let context = Context {
        palette: &DEFAULT_DARK,
        config: &config,
        focused: false,
    };
    row.draw(area, &mut buffer, &state, &context);
    assert_eq!(buffer[(3, 2)].symbol(), "•", "the current colour is marked");
    assert_eq!(buffer[(0, 2)].symbol(), "█");
}

#[test]
fn every_cell_of_a_swatch_carries_its_colour() {
    // A glyph that measures one cell and a font that draws it wider: both cells are the colour, so the
    // overflow lands on its own colour rather than on whatever was there.
    let config = Config {
        swatch: "●".into(),
        titles: false,
        ..Config::default()
    };
    let area = Rect::new(0, 0, 30, 1);
    let state = State::new(Rgb::new(0, 0, 0));
    let row = Swatches::rows("mine", vec![vec![Rgb::new(200, 0, 0), Rgb::new(0, 200, 0)]]);
    let mut buffer = Buffer::empty(area);
    let context = Context {
        palette: &DEFAULT_DARK,
        config: &config,
        focused: false,
    };
    row.draw(area, &mut buffer, &state, &context);
    assert_eq!(buffer[(0, 0)].symbol(), "●");
    assert_eq!(buffer[(0, 0)].bg, Color::Rgb(200, 0, 0));
    assert_eq!(
        buffer[(2, 0)].bg,
        Color::Rgb(0, 200, 0),
        "a one-cell glyph steps two"
    );
    assert_eq!(buffer[(1, 0)].bg, Color::Reset, "the gap is no swatch");
    assert_eq!(
        buffer[(1, 0)].fg,
        Color::Rgb(200, 0, 0),
        "but an overflow into it is drawn in the swatch's colour"
    );

    let wide = Config {
        swatch: "██".into(),
        titles: false,
        ..Config::default()
    };
    let mut buffer = Buffer::empty(area);
    row.draw(
        area,
        &mut buffer,
        &state,
        &Context {
            config: &wide,
            ..context
        },
    );
    for x in 0..2 {
        assert_eq!(buffer[(x, 0)].fg, Color::Rgb(200, 0, 0), "cell {x}");
        assert_eq!(buffer[(x, 0)].bg, Color::Rgb(200, 0, 0), "cell {x}");
    }
}

#[test]
fn the_recent_row_waits_for_two_colours_and_picks_one() {
    let config = Config::default();
    let mut picker = Picker::new().with(Swatches::recent(), Place::Fill);
    let area = Rect::new(0, 0, 30, 4);
    let mut state = State::new(Rgb::new(0, 0, 0));
    state.recent = vec![Rgb::new(9, 9, 9)];
    assert!(
        picker.layout(area, &state, &config).is_empty(),
        "one kept colour draws nothing"
    );
    state.recent.push(Rgb::new(200, 100, 0));
    assert_eq!(picker.layout(area, &state, &config).len(), 1);
    assert_eq!(
        picker.press(area, 3, 1, &mut state, &config),
        Outcome::Accepted(Choice::Colour(Rgb::new(200, 100, 0))),
        "a swatch press is a choice"
    );
}

#[test]
fn a_named_swatch_chooses_the_name() {
    let config = Config {
        titles: false,
        ..Config::default()
    };
    let area = Rect::new(0, 0, 30, 1);
    let mut state = State::new(Rgb::new(0, 0, 0)).with_entries(entries());
    let mut row = Swatches::named("theme");
    assert!(row.press(area, 3, 0, &mut state, &config));
    assert_eq!(state.choice(), Choice::Named(1));
    assert_eq!(state.rgb(), Rgb::new(0xfb, 0xc0, 0x2d));
    // An entry with no colour draws no swatch; `default` is the fourth entry and has no place.
    assert_eq!(row.at(area, 9, 0, &state, &config), None);
}

// --- the typed field and the list -------------------------------------------------------------------

#[test]
fn typing_a_colour_previews_it_and_enter_accepts_it() {
    let config = Config::default();
    let mut picker = Picker::preset(Preset::Inline, &config);
    let mut state = State::new(Rgb::new(1, 2, 3)).with_entries(entries());
    typed(&mut picker, &mut state, &config, "#f80");
    assert_eq!(
        state.rgb(),
        Rgb::new(0xff, 0x88, 0x00),
        "a whole colour previews as typed"
    );
    assert_eq!(
        picker.key(key(KeyCode::Enter), &mut state, &config),
        Outcome::Accepted(Choice::Colour(Rgb::new(0xff, 0x88, 0x00)))
    );
}

#[test]
fn typed_text_that_parses_as_nothing_is_refused_in_place() {
    let config = Config::default();
    let mut picker = Picker::preset(Preset::Inline, &config);
    let original = Rgb::new(1, 2, 3);
    let mut state = State::new(original).with_entries(entries());
    typed(&mut picker, &mut state, &config, "#zz");
    assert_eq!(
        picker.key(key(KeyCode::Enter), &mut state, &config),
        Outcome::Changed
    );
    assert_eq!(state.rgb(), original, "nothing taken");
    let refusal = state.refusal().expect("a refusal stands").0.clone();
    assert!(refusal.starts_with("not a colour"), "{refusal}");

    let area = Rect::new(0, 0, 90, 6);
    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, &state, &config, &DEFAULT_DARK);
    assert!(
        row_text(&buffer, 1).contains("not a colour"),
        "{}",
        frame_text(&buffer)
    );
    assert_eq!(buffer[(0, 1)].fg, DEFAULT_DARK.bad);

    // Typing again clears it.
    picker.key(key(KeyCode::Backspace), &mut state, &config);
    assert!(state.refusal().is_none());
}

#[test]
fn typing_filters_the_list_and_enter_takes_the_match_by_name() {
    let config = Config::default();
    let mut picker = Picker::preset(Preset::Inline, &config);
    let mut state = State::new(Rgb::new(1, 2, 3)).with_entries(entries());
    typed(&mut picker, &mut state, &config, "acc");
    assert_eq!(state.filter.hits().len(), 1);
    let area = Rect::new(0, 0, 40, 6);
    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, &state, &config, &DEFAULT_DARK);
    assert!(
        row_text(&buffer, 2).contains("accent #3366ff"),
        "{}",
        frame_text(&buffer)
    );
    assert!(!frame_text(&buffer).contains("warn"));
    assert_eq!(
        picker.key(key(KeyCode::Enter), &mut state, &config),
        Outcome::Accepted(Choice::Named(0))
    );
}

#[test]
fn a_list_row_draws_its_swatch_and_a_press_chooses_it() {
    let config = Config::default();
    let mut picker = Picker::preset(Preset::Inline, &config);
    let mut state = State::new(Rgb::new(1, 2, 3)).with_entries(entries());
    let area = Rect::new(0, 0, 40, 8);
    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, &state, &config, &DEFAULT_DARK);
    // Typed takes rows 0 and 1; the list starts on row 2.
    assert_eq!(buffer[(0, 3)].fg, Color::Rgb(0xfb, 0xc0, 0x2d));
    assert!(row_text(&buffer, 5).contains("default"));
    assert_eq!(buffer[(0, 5)].symbol(), " ", "no colour, no swatch");
    assert_eq!(
        picker.press(area, 10, 3, &mut state, &config),
        Outcome::Accepted(Choice::Named(1)),
        "a list press is a choice"
    );
}

// --- the panel, the footer, the picker --------------------------------------------------------------

#[test]
fn the_panel_prints_the_drawn_hex_and_the_hover_hex() {
    let config = Config::default();
    let picker = Picker::preset(Preset::Full, &config);
    let mut state = State::new(Rgb::new(0x5c, 0xbd, 0xfa));
    let area = Rect::new(0, 0, 80, 24);
    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, &state, &config, &DEFAULT_DARK);
    let panel_x = 80 - config.side_width;
    assert!(row_text(&buffer, 0)[usize::from(panel_x)..].starts_with("#5cbdfa"));
    assert!(frame_text(&buffer).contains("rgb 92 189 250"));
    assert!(frame_text(&buffer).contains("hsl 203 94% 67%"));

    let hovered = picker
        .hover(area, 0, 1, &mut state, &config)
        .expect("over the field");
    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, &state, &config, &DEFAULT_DARK);
    assert!(
        row_text(&buffer, 0).contains(&format!("{}  hover", hovered.hex())),
        "{}",
        row_text(&buffer, 0)
    );
    // Off every view, the hover goes.
    assert!(picker.hover(area, 79, 22, &mut state, &config).is_none());
    assert!(state.hover().is_none());
}

#[test]
fn the_panels_hex_box_takes_a_typed_hex_and_refuses_a_bad_one() {
    let config = Config::default();
    let mut panel = Panel::default();
    let mut state = State::new(Rgb::new(1, 2, 3));
    assert_eq!(
        panel.key(key(KeyCode::Enter), &mut state, &config),
        Outcome::Handled
    );
    for character in "#12zz".chars() {
        panel.key(key(KeyCode::Char(character)), &mut state, &config);
    }
    assert_eq!(
        panel.key(key(KeyCode::Enter), &mut state, &config),
        Outcome::Handled
    );
    assert_eq!(state.rgb(), Rgb::new(1, 2, 3));
    assert!(state.refusal().is_some());
    for _ in 0..2 {
        panel.key(key(KeyCode::Backspace), &mut state, &config);
    }
    for character in "3456".chars() {
        panel.key(key(KeyCode::Char(character)), &mut state, &config);
    }
    assert_eq!(
        panel.key(key(KeyCode::Enter), &mut state, &config),
        Outcome::Changed
    );
    assert_eq!(state.rgb(), Rgb::new(0x12, 0x34, 0x56));
}

#[test]
fn the_old_swatch_puts_the_original_back() {
    let config = Config::default();
    let mut panel = Panel::default();
    let mut state = State::new(Rgb::new(1, 2, 3));
    state.set_rgb(Rgb::new(200, 200, 0));
    let area = Rect::new(0, 0, 26, 9);
    assert!(panel.press(area, 1, 1, &mut state, &config));
    assert_eq!(state.rgb(), Rgb::new(1, 2, 3));
}

#[test]
fn the_keys_move_focus_switch_space_copy_and_cancel() {
    let config = Config::default();
    let mut picker = Picker::preset(Preset::Full, &config);
    let mut state = State::new(Rgb::new(0x5c, 0xbd, 0xfa));
    assert_eq!(state.focus(), 0);
    picker.key(key(KeyCode::Tab), &mut state, &config);
    assert_eq!(state.focus(), 1, "field to strip");
    for _ in 0..3 {
        picker.key(key(KeyCode::Tab), &mut state, &config);
    }
    assert_eq!(
        state.focus(),
        0,
        "the footer and the hidden recent row are skipped"
    );
    picker.key(key(KeyCode::BackTab), &mut state, &config);
    assert_eq!(state.focus(), 3, "back to the common row");

    picker.key(key(KeyCode::Char('m')), &mut state, &config);
    assert_eq!(state.space, Space::Hsl);
    assert_eq!(
        picker.key(key(KeyCode::Char('#')), &mut state, &config),
        Outcome::Copied("#5cbdfa".into())
    );

    picker.key(key(KeyCode::Tab), &mut state, &config);
    picker.key(key(KeyCode::Right), &mut state, &config);
    assert_ne!(state.rgb(), state.original());
    assert_eq!(
        picker.key(key(KeyCode::Esc), &mut state, &config),
        Outcome::Cancelled
    );
    assert_eq!(state.rgb(), Rgb::new(0x5c, 0xbd, 0xfa));
}

#[test]
fn rebound_keys_are_the_ones_answered_and_the_ones_hinted() {
    let config = Config {
        keys: Keys {
            copy: vec![KeyCode::Char('y')],
            switch_space: vec![KeyCode::F(2)],
            ..Keys::default()
        },
        ..Config::default()
    };
    let mut picker = Picker::preset(Preset::Sliders, &config);
    let mut state = State::new(Rgb::new(1, 2, 3));
    assert_eq!(
        picker.key(key(KeyCode::Char('y')), &mut state, &config),
        Outcome::Copied("#010203".into())
    );
    assert_eq!(
        picker.key(key(KeyCode::Char('#')), &mut state, &config),
        Outcome::Ignored
    );
    picker.key(key(KeyCode::F(2)), &mut state, &config);
    assert_eq!(state.space, Space::Hsl);
    let area = Rect::new(0, 0, 80, 12);
    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, &state, &config, &DEFAULT_DARK);
    let footer = row_text(&buffer, 11);
    assert!(
        footer.contains("Y copy") && footer.contains("F2 switch"),
        "{footer}"
    );
    assert!(!footer.contains("# copy"));
}

#[test]
fn under_tabs_one_fill_view_shows_and_the_strip_switches_it() {
    let config = Config {
        tabs: true,
        ..Config::default()
    };
    let mut picker = Picker::new()
        .with(Field::square(), Place::Fill)
        .with(Field::wheel(), Place::Fill)
        .with(Sliders::default(), Place::Fill);
    let mut state = State::new(Rgb::new(0x5c, 0xbd, 0xfa));
    let area = Rect::new(0, 0, 50, 14);
    let layout = picker.layout(area, &state, &config);
    assert_eq!(layout.len(), 1);
    assert_eq!(layout[0].0, 0);
    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, &state, &config, &DEFAULT_DARK);
    let strip = row_text(&buffer, 0);
    assert!(
        strip.contains("field") && strip.contains("wheel") && strip.contains("channels"),
        "{strip}"
    );
    let at = strip.find("wheel").expect("drawn") as u16;
    assert_eq!(
        picker.press(area, at, 0, &mut state, &config),
        Outcome::Handled
    );
    assert_eq!(state.tab(), 1);
    assert_eq!(picker.layout(area, &state, &config)[0].0, 1);
}

// --- universality -----------------------------------------------------------------------------------

/// One of every view tuilith ships.
fn every_view(config: &Config) -> Vec<Box<dyn View>> {
    Kind::all()
        .into_iter()
        .filter(|kind| {
            // Two strips stand for the eight: a vertical through one channel, a bar through another.
            !matches!(kind, Kind::Strip(channel, _) if !matches!(channel, Channel::Value | Channel::Hue))
        })
        .map(|kind| kind.view(config))
        .collect()
}

#[test]
fn every_kind_and_place_is_named_and_read_back() {
    let all = Kind::all();
    assert_eq!(all.len(), 20);
    for kind in all {
        assert_eq!(Kind::named(&kind.name()), Some(kind), "{}", kind.name());
    }
    assert_eq!(
        Kind::named("value-strip"),
        Some(Kind::Strip(
            Channel::Value,
            super::views::Orientation::Vertical
        ))
    );
    assert_eq!(
        Kind::named("hue-bar"),
        Some(Kind::Strip(
            Channel::Hue,
            super::views::Orientation::Horizontal
        ))
    );
    assert_eq!(Kind::named("nonsense"), None);
    for place in PLACES {
        assert_eq!(Place::named(place.name()), Some(place));
    }
    for name in ["inline", "full", "sliders", "swatches"] {
        assert!(Preset::named(name).is_some(), "{name}");
    }
    assert_eq!(Preset::named("everything"), None);
}

const PLACES: [Place; 5] = [
    Place::Fill,
    Place::Beside,
    Place::Below,
    Place::Side,
    Place::Footer,
];

fn sentinel(area: Rect) -> Buffer {
    let mut buffer = Buffer::empty(area);
    for cell in &mut buffer.content {
        cell.set_symbol("§");
    }
    buffer
}

#[test]
fn every_view_composes_with_every_other_at_any_size_without_overlap() {
    let sizes = [(8, 3), (30, 10), (120, 40)];
    let mut state = State::new(Rgb::new(0x5c, 0xbd, 0xfa)).with_entries(entries());
    state.recent = vec![Rgb::new(1, 1, 1), Rgb::new(2, 2, 2), Rgb::new(3, 3, 3)];
    let mut composed = 0;
    for tabs in [false, true] {
        let config = Config {
            tabs,
            ..Config::default()
        };
        let count = every_view(&config).len();
        for first in 0..count {
            for second in 0..count {
                for (first_place, second_place) in PLACES
                    .iter()
                    .flat_map(|a| PLACES.iter().map(move |b| (*a, *b)))
                {
                    let mut views = every_view(&config);
                    let b = views.remove(second);
                    let a = every_view(&config).remove(first);
                    let mut picker = Picker::new();
                    picker.push(a, first_place);
                    picker.push(b, second_place);
                    for (width, height) in sizes {
                        let area = Rect::new(1, 1, width, height);
                        check_composition(&mut picker, area, &mut state, &config);
                        composed += 1;
                    }
                }
            }
        }
        // And all of them at once, in every place.
        for place in PLACES {
            let mut picker = Picker::new();
            for view in every_view(&config) {
                picker.push(view, place);
            }
            for (width, height) in sizes {
                check_composition(
                    &mut picker,
                    Rect::new(0, 0, width, height),
                    &mut state,
                    &config,
                );
            }
        }
    }
    assert!(composed > 7_000, "{composed}");
}

fn check_composition(picker: &mut Picker, area: Rect, state: &mut State, config: &Config) {
    let layout = picker.layout(area, state, config);
    for (at, (index, rect)) in layout.iter().enumerate() {
        assert!(
            area.contains(rect.as_position())
                && rect.right() <= area.right()
                && rect.bottom() <= area.bottom(),
            "{rect:?} outside {area:?}"
        );
        for (_, other) in &layout[at + 1..] {
            assert!(
                !rect.intersects(*other),
                "{index}: {rect:?} overlaps {other:?}"
            );
        }
        // Each view draws inside its own rect and nowhere else.
        let mut buffer = sentinel(Rect::new(0, 0, area.right() + 2, area.bottom() + 2));
        let context = Context {
            palette: &DEFAULT_DARK,
            config,
            focused: true,
        };
        picker.slots()[*index]
            .view
            .draw(*rect, &mut buffer, state, &context);
        for y in buffer.area.y..buffer.area.bottom() {
            for x in buffer.area.x..buffer.area.right() {
                if !contains(*rect, x, y) {
                    assert_eq!(
                        buffer[(x, y)].symbol(),
                        "§",
                        "{index} drew at {x},{y} outside {rect:?}"
                    );
                }
            }
        }
    }
    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, state, config, &DEFAULT_DARK);
    // Input anywhere never panics, and keys never do.
    for (x, y) in [
        (area.x, area.y),
        (area.right() - 1, area.bottom() - 1),
        (area.x + area.width / 2, area.y + area.height / 2),
    ] {
        picker.hover(area, x, y, state, config);
        picker.press(area, x, y, state, config);
    }
    for code in [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Tab,
        KeyCode::Char('a'),
    ] {
        picker.key(key(code), state, config);
    }
}

#[test]
fn every_preset_draws_at_a_small_and_a_large_size() {
    let config = Config::default();
    for preset in [
        Preset::Inline,
        Preset::Full,
        Preset::Sliders,
        Preset::Swatches,
    ] {
        let picker = Picker::preset(preset, &config);
        let mut state = State::new(Rgb::new(0x5c, 0xbd, 0xfa)).with_entries(entries());
        state.recent = vec![Rgb::new(1, 1, 1), Rgb::new(2, 2, 2)];
        for (width, height) in [(12, 4), (90, 30)] {
            let area = Rect::new(0, 0, width, height);
            let layout = picker.layout(area, &state, &config);
            assert!(
                !layout.is_empty(),
                "{preset:?} at {width}x{height} draws nothing"
            );
            let mut buffer = Buffer::empty(area);
            picker.draw(area, &mut buffer, &state, &config, &DEFAULT_DARK);
        }
    }
}

#[test]
fn every_view_in_the_full_preset_sees_the_same_colour() {
    let config = Config::default();
    let mut picker = Picker::preset(Preset::Full, &config);
    let mut state = State::new(Rgb::new(10, 10, 10));
    let area = Rect::new(0, 0, 80, 24);
    // A press on a common swatch moves the panel's hex and the field's marker together.
    let layout = picker.layout(area, &state, &config);
    let (_, common) = layout
        .iter()
        .find(|(at, _)| *at == 3)
        .copied()
        .expect("common row placed");
    picker.press(area, common.x, common.y + 2, &mut state, &config);
    assert_eq!(state.rgb(), COMMON[1][0]);
    let mut buffer = Buffer::empty(area);
    picker.draw(area, &mut buffer, &state, &config, &DEFAULT_DARK);
    assert!(row_text(&buffer, 0).contains(&COMMON[1][0].hex()));
    assert!(frame_text(&buffer).contains('◉'), "the field marks it");
}

#[test]
fn a_field_press_adjusts_where_a_list_or_swatch_press_chooses() {
    let config = Config::default();
    let mut picker = Picker::preset(Preset::Full, &config);
    let mut state = State::new(Rgb::new(0x5c, 0xbd, 0xfa));
    let area = Rect::new(0, 0, 80, 24);
    assert_eq!(
        picker.press(area, 4, 3, &mut state, &config),
        Outcome::Changed
    );
}

#[test]
fn a_hosts_parser_decides_what_typing_takes_and_says_why_not() {
    let config = Config::default();
    let mut picker = Picker::preset(Preset::Inline, &config);
    // A host whose key takes palette roles only.
    let roles_only = Parser::new(|text, entries| {
        entries
            .iter()
            .position(|entry| entry.label.eq_ignore_ascii_case(text.trim()))
            .map(Choice::Named)
            .ok_or_else(|| Refusal("takes a palette role: accent, warn or red".into()))
    });
    let original = Rgb::new(1, 2, 3);
    let mut state = State::new(original)
        .with_entries(entries())
        .with_parser(roles_only);
    typed(&mut picker, &mut state, &config, "#ff8800");
    assert_eq!(
        state.rgb(),
        original,
        "a colour the host refuses does not preview"
    );
    assert_eq!(
        picker.key(key(KeyCode::Enter), &mut state, &config),
        Outcome::Changed
    );
    assert_eq!(
        state.refusal().map(|r| r.0.as_str()),
        Some("takes a palette role: accent, warn or red")
    );
    // Without the hook the same text is a colour.
    let mut plain = State::new(original).with_entries(entries());
    typed(&mut picker, &mut plain, &config, "#ff8800");
    assert_eq!(plain.rgb(), Rgb::new(0xff, 0x88, 0x00));

    let mut refused = State::new(original);
    refused.refuse(Refusal("the host says no".into()));
    assert_eq!(
        refused.refusal().map(|r| r.0.as_str()),
        Some("the host says no")
    );
}

#[test]
fn a_bare_hex_loses_only_to_a_name_it_starts() {
    let config = Config::default();
    let mut picker = Picker::preset(Preset::Inline, &config);
    let names = vec![
        Entry::new("facade", Some(Rgb::new(1, 1, 1))),
        Entry::new("surface_dim", Some(Rgb::new(2, 2, 2))),
    ];
    // `facade` is a hex, and the name it spells wins.
    let mut state = State::new(Rgb::new(0, 0, 0)).with_entries(names);
    typed(&mut picker, &mut state, &config, "facade");
    assert_eq!(
        picker.key(key(KeyCode::Enter), &mut state, &config),
        Outcome::Accepted(Choice::Named(0))
    );
    // `fedcba` is a subsequence of nothing it starts: the hex wins over a fuzzy match.
    let mut state =
        State::new(Rgb::new(0, 0, 0)).with_entries(vec![Entry::new("surface_dim_fedcba", None)]);
    assert_eq!(state.name_starting("ace"), None);
    typed(&mut picker, &mut state, &config, "fedcba");
    assert_eq!(
        picker.key(key(KeyCode::Enter), &mut state, &config),
        Outcome::Accepted(Choice::Colour(Rgb::new(0xfe, 0xdc, 0xba)))
    );
}
