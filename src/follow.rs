//! Following the terminal's light/dark switch while the application runs.
//!
//! [`background::read`] answers once, before the first frame, and a terminal that follows the desktop
//! theme switches long after that: at sunset, when the reader flips the system setting, when a
//! multiplexer re-themes. An application that only asked at startup draws dark text on a now-dark
//! background until someone restarts it. This keeps the answer current from three signals, in order
//! of how much they can be trusted:
//!
//! 1. **A mode-2031 report** (`CSI ? 997 ; 1|2 n`). The terminal pushes it on every change, so once
//!    one has arrived nothing else is asked. It is taken as reported and never second-guessed with an
//!    OSC 11 query: a multiplexer can send the report before its own OSC 11 answer has caught up, and
//!    the query would then undo the switch it announced.
//! 2. **An OSC 11 query**, on focus and every [`QUERY_EVERY`] — only until a report arrives, and only
//!    while the terminal answers. A DA1 query rides along with every OSC 11 query; every terminal
//!    answers DA1, so DA1 arriving with no colour before it means the terminal does not answer OSC 11
//!    and asking stops. This is the only live signal from a terminal that pushes nothing, such as
//!    Windows Terminal, and it costs one write.
//! 3. **The desktop setting** ([`background::read_while_running`]) — only when the terminal answered
//!    nothing at startup either, because a desktop setting is a proxy for the terminal and a wrong one
//!    for anybody running a dark terminal on a light desktop. Probed on a worker, never two at once,
//!    and backing off while it fails: on WSL each probe is a Windows process, and a wedged interop
//!    must not be fed a new one every few seconds.
//!
//! ```no_run
//! use tuilith::background;
//! use tuilith::follow::Follower;
//! use tuilith::input::{Event, EventStream};
//!
//! # fn main() -> std::io::Result<()> {
//! // Before the alternate screen: the startup probe reads its answer from the terminal.
//! let mut reading = background::read();
//! // ... enter raw mode and the alternate screen ...
//! let events = EventStream::new()?;
//! let mut follower = Follower::start(reading)?;
//! loop {
//!     if let Some(event) = events.next_timeout(std::time::Duration::from_millis(250))? {
//!         if let Some(now) = follower.observe(&event) {
//!             reading = now;
//!         }
//!         if let Event::Terminal(_event) = event { /* the application's own handling */ }
//!     }
//!     if let Some(now) = follower.tick() {
//!         reading = now;
//!     }
//!     // draw with reading.mode.palette()
//! #   break;
//! }
//! # Ok(())
//! # }
//! ```

use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::background::{self, Reading, Source};
use crate::input::Event;
use crate::theme::Mode;

crate::provenance! {
    component: "follow",
    about: "Keeps a running application's light/dark mode current from mode-2031 reports, OSC 11 and the desktop",
    origin: crate::Origin::Here,
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// Turns on mode-2031 reports and focus reporting, and asks for the current scheme.
///
/// [`Follower::start`] writes it; an application that writes its own setup can include it there.
pub const ENABLE: &str = "\x1b[?2031h\x1b[?1004h\x1b[?996n";

/// Undoes [`ENABLE`]. A [`Follower`] writes it when dropped; write it from a panic hook as well, or
/// a terminal left reporting goes on sending colour-scheme reports to the shell after the
/// application has gone.
pub const DISABLE: &str = "\x1b[?2031l\x1b[?1004l";

/// OSC 11 asks for the background; DA1 behind it says when no answer is coming.
const QUERY: &str = "\x1b]11;?\x1b\\\x1b[c";

/// How often a terminal that pushes nothing is asked for its background.
pub const QUERY_EVERY: Duration = Duration::from_secs(5);

/// How often the desktop setting is read while it is the signal being followed.
const DESKTOP_EVERY: Duration = Duration::from_secs(5);

/// The longest the desktop probe backs off to while it keeps failing.
const DESKTOP_BACKOFF_CAP: Duration = Duration::from_mins(5);

/// Whether the terminal answers OSC 11.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Osc {
    /// Not yet established.
    Unknown,
    /// It has answered at least once.
    Answers,
    /// DA1 arrived with no colour before it: it does not answer, so it is not asked again.
    Silent,
}

/// The desktop probe's schedule.
struct Desktop {
    /// The probe in flight, if one is.
    probe: Option<Receiver<Reading>>,
    next: Instant,
    every: Duration,
}

/// Keeps a light/dark [`Reading`] current while the application runs. See the module docs.
pub struct Follower {
    reading: Reading,
    /// The startup answer came from the desktop or from nobody, so the desktop is the best signal
    /// there is. False once the terminal itself answered at startup.
    trust_desktop: bool,
    /// A mode-2031 report has arrived: the terminal pushes, and nothing else needs asking.
    pushed: bool,
    osc: Osc,
    /// An OSC 11 query is outstanding.
    awaiting_reply: bool,
    next_query: Instant,
    desktop: Desktop,
    out: Box<dyn Write + Send>,
}

impl Follower {
    /// Start following from the startup `reading`, writing to standard output.
    ///
    /// Call it once the application owns the terminal (raw mode, and reading through
    /// [`crate::input::EventStream`]), because the replies it asks for arrive as input.
    ///
    /// # Errors
    ///
    /// When [`ENABLE`] cannot be written.
    pub fn start(reading: Reading) -> io::Result<Self> {
        Self::start_on(reading, Box::new(io::stdout()), Instant::now())
    }

    /// [`Follower::start`], writing to `out` from `now` — for an application drawing somewhere other
    /// than standard output, and for tests.
    ///
    /// # Errors
    ///
    /// When [`ENABLE`] cannot be written.
    pub fn start_on(
        reading: Reading,
        out: Box<dyn Write + Send>,
        now: Instant,
    ) -> io::Result<Self> {
        let mut follower = Self {
            reading,
            trust_desktop: matches!(
                reading.source,
                Source::WindowsRegistry | Source::MacOsDefaults | Source::Nobody
            ),
            pushed: false,
            osc: if reading.source == Source::Osc {
                Osc::Answers
            } else {
                Osc::Unknown
            },
            awaiting_reply: false,
            // The startup probe has just asked; the first scheduled query can wait its turn, unless
            // nobody answered it, in which case finding out is worth doing now.
            next_query: if reading.source == Source::Osc {
                now + QUERY_EVERY
            } else {
                now
            },
            desktop: Desktop {
                probe: None,
                next: now + DESKTOP_EVERY,
                every: DESKTOP_EVERY,
            },
            out,
        };
        follower.out.write_all(ENABLE.as_bytes())?;
        follower.out.flush()?;
        Ok(follower)
    }

    /// The current reading.
    #[must_use]
    pub fn reading(&self) -> Reading {
        self.reading
    }

    /// Feed every event from the input stream through here. Returns the new reading when it changed.
    pub fn observe(&mut self, event: &Event) -> Option<Reading> {
        self.observe_at(event, Instant::now())
    }

    /// [`Follower::observe`] at a given instant.
    pub fn observe_at(&mut self, event: &Event, now: Instant) -> Option<Reading> {
        match event {
            Event::ColorScheme(mode) => {
                self.pushed = true;
                self.awaiting_reply = false;
                self.set(*mode, Source::Report2031)
            }
            Event::Background(rgb) => {
                self.osc = Osc::Answers;
                self.awaiting_reply = false;
                // A report is authoritative; a colour answer behind it may be the multiplexer's stale
                // cache rather than the switch the report announced.
                if self.pushed {
                    None
                } else {
                    self.set(rgb.mode(), Source::Osc)
                }
            }
            Event::DeviceAttributes => {
                if self.awaiting_reply {
                    self.awaiting_reply = false;
                    if self.osc == Osc::Unknown {
                        self.osc = Osc::Silent;
                    }
                }
                None
            }
            Event::Terminal(crossterm::event::Event::FocusGained) => {
                // Coming back to the window is when a switch made elsewhere is most likely to show.
                if self.asks_terminal() {
                    self.query(now);
                }
                if self.follows_desktop() {
                    self.desktop.next = now;
                }
                None
            }
            _ => None,
        }
    }

    /// Call on the application's tick. Asks the terminal and the desktop when they are due, and
    /// returns the new reading when a desktop probe changed it.
    pub fn tick(&mut self) -> Option<Reading> {
        self.tick_at(Instant::now())
    }

    /// [`Follower::tick`] at a given instant.
    pub fn tick_at(&mut self, now: Instant) -> Option<Reading> {
        let changed = self.collect_desktop(now);
        if self.asks_terminal() && now >= self.next_query {
            self.query(now);
        }
        if self.follows_desktop() && self.desktop.probe.is_none() && now >= self.desktop.next {
            self.probe_desktop(now);
        }
        changed
    }

    fn asks_terminal(&self) -> bool {
        !self.pushed && self.osc != Osc::Silent
    }

    fn follows_desktop(&self) -> bool {
        self.trust_desktop && !self.pushed && self.osc != Osc::Answers
    }

    fn query(&mut self, now: Instant) {
        self.next_query = now + QUERY_EVERY;
        // A failed write is a missed poll, not a reason to stop the application.
        if self.out.write_all(QUERY.as_bytes()).is_ok() && self.out.flush().is_ok() {
            self.awaiting_reply = true;
        }
    }

    fn probe_desktop(&mut self, now: Instant) {
        self.desktop.next = now + self.desktop.every;
        let (send, receive) = mpsc::channel();
        // `Builder` rather than `thread::spawn`, which panics when the OS refuses a thread.
        let spawned = std::thread::Builder::new()
            .name(String::from("tuilith-desktop-probe"))
            .spawn(move || {
                let _ = send.send(background::read_while_running());
            });
        if spawned.is_ok() {
            self.desktop.probe = Some(receive);
        }
    }

    fn collect_desktop(&mut self, now: Instant) -> Option<Reading> {
        let outcome = match self.desktop.probe.as_ref()?.try_recv() {
            Ok(reading) => Some(reading),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => None,
        };
        self.desktop.probe = None;
        // `COLORFGBG` can answer here too, but it is fixed for the process's lifetime: it is not the
        // desktop answering, and a desktop probe that fell through to it failed.
        let desktop = outcome.filter(|reading| {
            matches!(
                reading.source,
                Source::WindowsRegistry | Source::MacOsDefaults
            )
        });
        if let Some(reading) = desktop {
            self.desktop.every = DESKTOP_EVERY;
            self.desktop.next = now + DESKTOP_EVERY;
            if self.follows_desktop() {
                self.set(reading.mode, reading.source)
            } else {
                None
            }
        } else {
            self.desktop.every = (self.desktop.every * 2).min(DESKTOP_BACKOFF_CAP);
            self.desktop.next = now + self.desktop.every;
            None
        }
    }

    fn set(&mut self, mode: Mode, source: Source) -> Option<Reading> {
        let reading = Reading { mode, source };
        if reading == self.reading {
            None
        } else {
            self.reading = reading;
            Some(reading)
        }
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        let _ = self.out.write_all(DISABLE.as_bytes());
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use super::*;
    use crate::input::Rgb;

    /// What the follower wrote, shared with the test that reads it.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<u8>>>);

    impl Write for Recorder {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Recorder {
        fn queries(&self) -> usize {
            let written = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            String::from_utf8_lossy(&written).matches(QUERY).count()
        }

        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap_or_else(PoisonError::into_inner))
                .into_owned()
        }
    }

    const WHITE: Rgb = Rgb {
        red: 0xFFFF,
        green: 0xFFFF,
        blue: 0xFFFF,
    };
    const BLACK: Rgb = Rgb {
        red: 0,
        green: 0,
        blue: 0,
    };

    fn started(source: Source) -> (Follower, Recorder, Instant) {
        let recorder = Recorder::default();
        let now = Instant::now();
        let follower = Follower::start_on(
            Reading {
                mode: Mode::Dark,
                source,
            },
            Box::new(recorder.clone()),
            now,
        )
        .expect("a recorder accepts every write");
        (follower, recorder, now)
    }

    #[test]
    fn starting_enables_reports_and_dropping_disables_them() {
        let (follower, recorder, _) = started(Source::Osc);
        assert!(recorder.text().starts_with(ENABLE));
        drop(follower);
        assert!(recorder.text().ends_with(DISABLE));
    }

    #[test]
    fn a_report_is_taken_as_reported_and_ends_the_asking() {
        let (mut follower, recorder, now) = started(Source::Osc);
        assert_eq!(
            follower.observe_at(&Event::ColorScheme(Mode::Light), now),
            Some(Reading {
                mode: Mode::Light,
                source: Source::Report2031
            }),
        );
        // A colour answer behind the report does not undo it.
        assert_eq!(follower.observe_at(&Event::Background(BLACK), now), None);
        assert_eq!(follower.reading().mode, Mode::Light);
        // And nothing is asked any more.
        follower.tick_at(now + QUERY_EVERY * 3);
        follower.observe_at(
            &Event::Terminal(crossterm::event::Event::FocusGained),
            now + QUERY_EVERY * 3,
        );
        assert_eq!(recorder.queries(), 0);
    }

    #[test]
    fn a_terminal_that_pushes_nothing_is_asked_on_a_cadence() {
        let (mut follower, recorder, now) = started(Source::Osc);
        assert_eq!(follower.tick_at(now + QUERY_EVERY / 2), None);
        assert_eq!(
            recorder.queries(),
            0,
            "the startup probe has only just asked"
        );
        follower.tick_at(now + QUERY_EVERY);
        assert_eq!(recorder.queries(), 1);
        assert_eq!(
            follower.observe_at(&Event::Background(WHITE), now + QUERY_EVERY),
            Some(Reading {
                mode: Mode::Light,
                source: Source::Osc
            }),
        );
    }

    #[test]
    fn focus_asks_at_once() {
        let (mut follower, recorder, now) = started(Source::Osc);
        follower.observe_at(&Event::Terminal(crossterm::event::Event::FocusGained), now);
        assert_eq!(recorder.queries(), 1);
    }

    #[test]
    fn a_terminal_that_answers_device_attributes_first_is_not_asked_again() {
        let (mut follower, recorder, now) = started(Source::ColorFgBg);
        follower.tick_at(now);
        assert_eq!(
            recorder.queries(),
            1,
            "an unanswered startup probe is followed up at once"
        );
        follower.observe_at(&Event::DeviceAttributes, now);
        follower.tick_at(now + QUERY_EVERY * 4);
        follower.observe_at(
            &Event::Terminal(crossterm::event::Event::FocusGained),
            now + QUERY_EVERY * 4,
        );
        assert_eq!(recorder.queries(), 1);
    }

    #[test]
    fn the_desktop_is_never_consulted_once_the_terminal_answered() {
        let (mut follower, _, now) = started(Source::Osc);
        follower.tick_at(now + DESKTOP_EVERY * 3);
        assert!(follower.desktop.probe.is_none());
        assert!(!follower.follows_desktop());
    }

    #[test]
    fn a_desktop_reading_applies_only_while_the_desktop_is_the_signal() {
        let (mut follower, _, now) = started(Source::WindowsRegistry);
        let (send, receive) = mpsc::channel();
        follower.desktop.probe = Some(receive);
        send.send(Reading {
            mode: Mode::Light,
            source: Source::WindowsRegistry,
        })
        .expect("the receiver is held");
        assert_eq!(
            follower.tick_at(now),
            Some(Reading {
                mode: Mode::Light,
                source: Source::WindowsRegistry
            }),
        );

        // Once the terminal answers, a later desktop reading is ignored.
        follower.observe_at(&Event::Background(BLACK), now);
        let (send, receive) = mpsc::channel();
        follower.desktop.probe = Some(receive);
        send.send(Reading {
            mode: Mode::Light,
            source: Source::WindowsRegistry,
        })
        .expect("the receiver is held");
        assert_eq!(follower.tick_at(now), None);
        assert_eq!(follower.reading().source, Source::Osc);
    }

    #[test]
    fn a_failing_desktop_probe_backs_off() {
        let (mut follower, _, now) = started(Source::Nobody);
        let (send, receive) = mpsc::channel();
        follower.desktop.probe = Some(receive);
        send.send(Reading {
            mode: Mode::Dark,
            source: Source::Nobody,
        })
        .expect("the receiver is held");
        assert_eq!(follower.tick_at(now), None);
        assert_eq!(follower.desktop.every, DESKTOP_EVERY * 2);
    }
}
