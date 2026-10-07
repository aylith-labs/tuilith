//! The input reader against a real pty in raw mode.
//!
//! The parser's unit tests prove each sequence parses; only a terminal proves the reader delivers it —
//! that a reply is read off the tty, that the keys typed after it are not held, and that a reply split
//! across two reads still arrives as one. The controller end of the pty plays the terminal.

#![cfg(all(unix, feature = "input"))]

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::time::Duration;

use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyModifiers};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{OptionalActions, tcgetattr, tcsetattr};
use tuilith::Mode;
use tuilith::input::{Event, EventStream, Rgb};

/// The terminal's end, which the test writes to, and the application's end in raw mode.
fn pty() -> (File, File) {
    let controller = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).expect("a pty is available");
    grantpt(&controller).expect("grantpt");
    unlockpt(&controller).expect("unlockpt");
    let name = ptsname(&controller, Vec::new()).expect("the pty has a name");
    let application = OpenOptions::new()
        .read(true)
        .write(true)
        .open(name.to_str().expect("a pty name is ASCII"))
        .expect("the pty opens");
    // Canonical mode would hold every byte until a newline; an application reads in raw mode.
    let mut termios = tcgetattr(&application).expect("tcgetattr");
    termios.make_raw();
    tcsetattr(&application, OptionalActions::Now, &termios).expect("tcsetattr");
    (File::from(controller), application)
}

fn next(events: &EventStream) -> Event {
    events
        .next_timeout(Duration::from_secs(2))
        .expect("the reader is healthy")
        .expect("an event arrives")
}

fn key(code: KeyCode) -> Event {
    Event::Terminal(TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

#[test]
fn a_scheme_report_arrives_as_an_event_and_the_keys_after_it_still_arrive() {
    let (mut terminal, application) = pty();
    let events = EventStream::from_tty(application).expect("the reader starts");

    terminal
        .write_all(b"\x1b[?997;2n")
        .expect("the pty accepts writes");
    assert_eq!(next(&events), Event::ColorScheme(Mode::Light));
    // The failure this exists for: stock crossterm holds this key in the report's buffer.
    terminal.write_all(b"j").expect("the pty accepts writes");
    assert_eq!(next(&events), key(KeyCode::Char('j')));

    // Report and key in the same read.
    terminal
        .write_all(b"\x1b[?997;1nk")
        .expect("the pty accepts writes");
    assert_eq!(next(&events), Event::ColorScheme(Mode::Dark));
    assert_eq!(next(&events), key(KeyCode::Char('k')));
}

#[test]
fn a_background_reply_split_after_its_escape_arrives_as_one_reply() {
    let (mut terminal, application) = pty();
    let events = EventStream::from_tty(application).expect("the reader starts");

    terminal.write_all(b"\x1b").expect("the pty accepts writes");
    std::thread::sleep(Duration::from_millis(2));
    terminal
        .write_all(b"]11;rgb:ffff/ffff/ffff\x1b\\")
        .expect("the pty accepts writes");
    assert_eq!(
        next(&events),
        Event::Background(Rgb {
            red: 0xFFFF,
            green: 0xFFFF,
            blue: 0xFFFF,
        })
    );
    terminal.write_all(b"q").expect("the pty accepts writes");
    assert_eq!(next(&events), key(KeyCode::Char('q')));
}

#[test]
fn a_lone_escape_is_still_the_key() {
    let (mut terminal, application) = pty();
    let events = EventStream::from_tty(application).expect("the reader starts");

    terminal.write_all(b"\x1b").expect("the pty accepts writes");
    assert_eq!(next(&events), key(KeyCode::Esc));
}
