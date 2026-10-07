//! Terminal input that keeps the terminal's *replies* — its colour scheme, its background colour, its
//! mode reports, its device attributes, its cell size, its kitty graphics answers — instead of typing
//! them into the application as keys.
//!
//! crossterm's reader is right about every byte a person produces. The bytes a terminal produces in
//! answer to a query are where it goes wrong, in two ways that each cost a running application its
//! keyboard:
//!
//! - A mode-2031 colour-scheme report (`CSI ? 997 ; 1 n`) is a `CSI ?` sequence whose final byte the
//!   parser does not know. It reads that as "incomplete" and then holds every key typed after it,
//!   waiting for a `u` or `c` that may never come.
//! - An OSC 11 background reply (`ESC ] 11 ; rgb:… BEL`) parses as Alt+`]` followed by its text as
//!   typed characters, and a kitty graphics reply (`ESC _ G … ESC \`) as Alt+`_` and the same.
//!
//! So an application reading through crossterm's `EventStream` cannot ask the terminal anything once
//! it is running — which is exactly when a light/dark switch happens. This module is crossterm's Unix
//! input path, the parser vendored from 0.29.0 with both fixed and a reader in the shape of its mio
//! source. Everything a person does arrives as crossterm's own [`crossterm::event::Event`] inside
//! [`Event::Terminal`]; raw mode, output and every key type stay crossterm's, so a page matching
//! `KeyCode::Char('j')` changes nothing. Only the reader is swapped.
//!
//! # The one rule
//!
//! **This must be the only reader of the terminal.** Two readers race for every byte and each sees
//! part of the input. Do not construct crossterm's `EventStream`, and do not call
//! `crossterm::event::read` or `poll`, while an [`EventStream`] from here is alive.
//!
//! On Windows the console delivers input as records rather than bytes, so there is nothing to
//! misparse and no reply ever arrives; there this delegates to `crossterm::event::read` on its own
//! thread.
//!
//! ```no_run
//! use std::time::Duration;
//! use tuilith::input::{Event, EventStream};
//!
//! # fn main() -> std::io::Result<()> {
//! let events = EventStream::new()?;
//! while let Some(event) = events.next_timeout(Duration::from_millis(250))? {
//!     match event {
//!         Event::Terminal(_event) => { /* keys, mouse, resize — exactly what crossterm gave */ }
//!         Event::ColorScheme(_mode) => { /* the terminal switched to this mode */ }
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossterm::event::KeyboardEnhancementFlags;

use crate::theme::Mode;

crate::provenance! {
    component: "input",
    about: "crossterm's Unix input reader, keeping terminal replies — mode-2031, OSC 11, DA1, cell size, kitty graphics — as events instead of keys",
    origin: crate::Origin::Upstream("crossterm"),
    lineage: crate::Lineage::Tracked {
        crate_name: "crossterm",
        base: "0.29.0",
        additions: "vendor/crossterm/ADDITIONS.md",
    },
    since: "0.1",
}

#[cfg(unix)]
mod unix;

/// The vendored parser. Kept in upstream's style rather than this crate's, so a later upstream fix
/// can be diffed in and taken; `vendor/crossterm/ADDITIONS.md` logs what was changed and why.
#[cfg(unix)]
#[rustfmt::skip]
#[allow(dead_code, clippy::all, clippy::pedantic)]
#[path = "../../vendor/crossterm/src/event/sys/unix/parse.rs"]
mod parse;

/// One thing the terminal sent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// Anything a person did — a key, the mouse, a paste, focus, a resize — exactly as crossterm
    /// reports it.
    Terminal(crossterm::event::Event),
    /// The terminal's colour scheme, from a mode-2031 report: sent once in reply to `CSI ? 996 n`, and
    /// again on every change while mode 2031 is set.
    ColorScheme(Mode),
    /// The terminal's background colour, in reply to an OSC 11 query.
    Background(Rgb),
    /// A DECRPM reply: whether the terminal recognises a private `mode`, and its `setting` (0 not
    /// recognised, 1 set, 2 reset, 3 permanently set, 4 permanently reset).
    ModeReport {
        /// The private mode asked about.
        mode: u16,
        /// The DECRPM setting value.
        setting: u8,
    },
    /// A primary device attributes reply (DA1), with its attribute list — the conformance level first,
    /// then what the terminal offers (`4` is sixel). Every terminal answers it, so a query followed by
    /// DA1 tells a reply that is not coming from one that is merely slow.
    DeviceAttributes(Vec<u16>),
    /// A device status report (`CSI 0 n` when `ok`), in reply to `CSI 5 n`. Replies arrive in the order
    /// their queries were sent, so this marks the end of every reply asked for before it.
    Status {
        /// The terminal reported itself in good order.
        ok: bool,
    },
    /// One character cell's size in pixels, in reply to `CSI 16 t`.
    CellSize {
        /// Pixels across.
        width: u16,
        /// Pixels down.
        height: u16,
    },
    /// A kitty graphics protocol reply: whether the terminal accepted the command for image `id`.
    KittyGraphics {
        /// The image id the command named.
        id: u32,
        /// The terminal answered `OK`.
        ok: bool,
    },
    /// A cursor position report, zero-based.
    CursorPosition {
        /// The cursor's column.
        column: u16,
        /// The cursor's row.
        row: u16,
    },
    /// The keyboard enhancement flags the terminal reports as enabled.
    KeyboardEnhancementFlags(KeyboardEnhancementFlags),
}

/// A colour as a terminal reports it: 16 bits per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb {
    /// Red, `0..=0xFFFF`.
    pub red: u16,
    /// Green, `0..=0xFFFF`.
    pub green: u16,
    /// Blue, `0..=0xFFFF`.
    pub blue: u16,
}

impl Rgb {
    /// Parse an X11 colour spec as terminals report it: `rgb:R/G/B`, each channel one to four hex
    /// digits, scaled to 16 bits.
    #[must_use]
    pub fn parse(spec: &[u8]) -> Option<Self> {
        let spec = std::str::from_utf8(spec).ok()?.strip_prefix("rgb:")?;
        let mut channels = spec.split('/').map(channel);
        let rgb = Self {
            red: channels.next()??,
            green: channels.next()??,
            blue: channels.next()??,
        };
        channels.next().is_none().then_some(rgb)
    }

    /// Whether this colour is a dark background or a light one.
    ///
    /// Decided on perceived lightness (CIE L*) against its midpoint, not on the raw channel average: a
    /// saturated blue averages to a third of white and is plainly dark.
    #[must_use]
    pub fn mode(self) -> Mode {
        fn linear(channel: u16) -> f64 {
            let value = f64::from(channel) / 65_535.0;
            if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        let luminance =
            0.2126 * linear(self.red) + 0.7152 * linear(self.green) + 0.0722 * linear(self.blue);
        let lightness = if luminance > 216.0 / 24_389.0 {
            116.0 * luminance.cbrt() - 16.0
        } else {
            luminance * 24_389.0 / 27.0
        };
        if lightness >= 50.0 {
            Mode::Light
        } else {
            Mode::Dark
        }
    }
}

/// One hex channel of an `rgb:` spec, scaled to 16 bits.
fn channel(hex: &str) -> Option<u16> {
    if hex.is_empty() || hex.len() > 4 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let max = (1_u32 << (4 * hex.len())) - 1;
    u16::try_from(value * 0xFFFF / max).ok()
}

/// What the vendored parser produces, before it is sorted into what an application sees.
#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InternalEvent {
    Event(crossterm::event::Event),
    CursorPosition(u16, u16),
    KeyboardEnhancementFlags(KeyboardEnhancementFlags),
    PrimaryDeviceAttributes(Vec<u16>),
    Status {
        ok: bool,
    },
    CellSize {
        width: u16,
        height: u16,
    },
    KittyGraphics {
        id: u32,
        ok: bool,
    },
    ColorScheme(Mode),
    Background(Rgb),
    ModeReport {
        mode: u16,
        setting: u8,
    },
    /// A complete sequence nothing here reads. Consumed so that none of it reaches the application.
    Unsupported,
}

#[cfg(unix)]
impl InternalEvent {
    fn public(self) -> Option<Event> {
        Some(match self {
            Self::Event(event) => Event::Terminal(event),
            Self::CursorPosition(column, row) => Event::CursorPosition { column, row },
            Self::KeyboardEnhancementFlags(flags) => Event::KeyboardEnhancementFlags(flags),
            Self::PrimaryDeviceAttributes(attributes) => Event::DeviceAttributes(attributes),
            Self::Status { ok } => Event::Status { ok },
            Self::CellSize { width, height } => Event::CellSize { width, height },
            Self::KittyGraphics { id, ok } => Event::KittyGraphics { id, ok },
            Self::ColorScheme(mode) => Event::ColorScheme(mode),
            Self::Background(rgb) => Event::Background(rgb),
            Self::ModeReport { mode, setting } => Event::ModeReport { mode, setting },
            Self::Unsupported => return None,
        })
    }
}

/// Events waiting for the application, and who to wake when one arrives.
#[derive(Default)]
struct Queue {
    events: VecDeque<io::Result<Event>>,
    waker: Option<Waker>,
    /// The reader has stopped, by request or because the terminal went away.
    closed: bool,
}

/// What the reader thread and the application share.
#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    arrived: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        // A panic on the other side leaves the queue itself intact, and losing input over it would be
        // worse than reading what is there.
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn push(&self, event: io::Result<Event>) {
        let waker = {
            let mut queue = self.lock();
            queue.events.push_back(event);
            queue.waker.take()
        };
        self.arrived.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn close(&self) {
        let waker = {
            let mut queue = self.lock();
            queue.closed = true;
            queue.waker.take()
        };
        self.arrived.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn is_closed(&self) -> bool {
        self.lock().closed
    }
}

/// The terminal's input, as a [`Stream`](futures_core::Stream) or by polling with a timeout.
///
/// Reading happens on a thread of its own from the moment this is created until it is dropped.
pub struct EventStream {
    shared: Arc<Shared>,
    #[cfg(unix)]
    stop: unix::Stop,
    thread: Option<JoinHandle<()>>,
}

impl EventStream {
    /// Start reading the terminal: standard input when it is one, `/dev/tty` otherwise.
    ///
    /// # Errors
    ///
    /// When there is no terminal to read, or the reader thread cannot be started.
    pub fn new() -> io::Result<Self> {
        #[cfg(unix)]
        {
            Self::from_tty(unix::tty()?)
        }
        #[cfg(not(unix))]
        {
            let shared = Arc::new(Shared::default());
            let thread = other::spawn(Arc::clone(&shared))?;
            Ok(Self {
                shared,
                thread: Some(thread),
            })
        }
    }

    /// Start reading a terminal the caller opened — a pty, or a tty that is not this process's own.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be polled, or the reader thread cannot be started.
    #[cfg(unix)]
    pub fn from_tty(tty: std::fs::File) -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let (thread, stop) = unix::spawn(tty, Arc::clone(&shared))?;
        Ok(Self {
            shared,
            stop,
            thread: Some(thread),
        })
    }

    /// The next event, waiting at most `timeout` for one. `Ok(None)` when none arrived in time.
    ///
    /// # Errors
    ///
    /// When the reader failed. It has stopped, and every later call returns `Ok(None)` at once.
    pub fn next_timeout(&self, timeout: Duration) -> io::Result<Option<Event>> {
        let deadline = Instant::now() + timeout;
        let mut queue = self.shared.lock();
        loop {
            if let Some(event) = queue.events.pop_front() {
                return event.map(Some);
            }
            if queue.closed {
                return Ok(None);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            queue = self
                .shared
                .arrived
                .wait_timeout(queue, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl futures_core::Stream for EventStream {
    type Item = io::Result<Event>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut queue = self.shared.lock();
        if let Some(event) = queue.events.pop_front() {
            return Poll::Ready(Some(event));
        }
        if queue.closed {
            return Poll::Ready(None);
        }
        queue.waker = Some(context.waker().clone());
        Poll::Pending
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        self.shared.close();
        #[cfg(unix)]
        self.stop.wake();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(not(unix))]
mod other {
    use std::io;
    use std::sync::Arc;
    use std::thread::{Builder, JoinHandle};
    use std::time::Duration;

    use super::{Event, Shared};

    /// Console input on Windows arrives as records, already decoded; there are no replies to keep.
    pub(super) fn spawn(shared: Arc<Shared>) -> io::Result<JoinHandle<()>> {
        Builder::new()
            .name(String::from("tuilith-input"))
            .spawn(move || {
                while !shared.is_closed() {
                    match crossterm::event::poll(Duration::from_millis(100)) {
                        Ok(true) => match crossterm::event::read() {
                            Ok(event) => shared.push(Ok(Event::Terminal(event))),
                            Err(error) => {
                                shared.push(Err(error));
                                shared.close();
                            }
                        },
                        Ok(false) => {}
                        Err(error) => {
                            shared.push(Err(error));
                            shared.close();
                        }
                    }
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_scales_every_channel_width_to_sixteen_bits() {
        let white = Rgb {
            red: 0xFFFF,
            green: 0xFFFF,
            blue: 0xFFFF,
        };
        assert_eq!(Rgb::parse(b"rgb:ffff/ffff/ffff"), Some(white));
        assert_eq!(Rgb::parse(b"rgb:ff/ff/ff"), Some(white));
        assert_eq!(Rgb::parse(b"rgb:f/f/f"), Some(white));
        assert_eq!(
            Rgb::parse(b"rgb:8080/0/1e1e"),
            Some(Rgb {
                red: 0x8080,
                green: 0,
                blue: 0x1E1E
            })
        );
    }

    #[test]
    fn a_malformed_spec_is_refused_rather_than_guessed() {
        assert_eq!(Rgb::parse(b"rgb:ffff/ffff"), None);
        assert_eq!(Rgb::parse(b"rgb:ffff/ffff/ffff/ffff"), None);
        assert_eq!(Rgb::parse(b"rgb:fffff/0/0"), None);
        assert_eq!(Rgb::parse(b"rgb:+f/0/0"), None);
        assert_eq!(Rgb::parse(b"#ffffff"), None);
    }

    #[test]
    fn lightness_decides_the_mode_where_an_average_would_not() {
        let rgb = |red, green, blue| Rgb { red, green, blue };
        assert_eq!(rgb(0, 0, 0).mode(), Mode::Dark);
        assert_eq!(rgb(0xFFFF, 0xFFFF, 0xFFFF).mode(), Mode::Light);
        // Catppuccin Mocha's base and Latte's base.
        assert_eq!(rgb(0x1E1E, 0x1E1E, 0x2E2E).mode(), Mode::Dark);
        assert_eq!(rgb(0xEFEF, 0xF1F1, 0xF5F5).mode(), Mode::Light);
        // Saturated blue averages to a third of white and is plainly dark.
        assert_eq!(rgb(0, 0, 0xFFFF).mode(), Mode::Dark);
        assert_eq!(rgb(0xFFFF, 0xFFFF, 0x9999).mode(), Mode::Light);
    }
}
