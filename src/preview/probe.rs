//! Asking the terminal what it can draw, through [`crate::input`].
//!
//! Four queries go out in one write: a kitty graphics query, a cell-size query, DA1, and a status
//! report behind them all. Replies come back in the order they were asked, and every terminal answers
//! the status report, so its arrival means every reply that is coming has come — a terminal that does
//! not speak kitty costs one round trip, not a timeout.
//!
//! The replies are read as [`Event`]s from the application's own input stream. A query answered
//! through a second reader — ratatui-image's `Picker::from_query_stdio` spawns one — leaves that
//! reader blocked on the terminal when the answer never comes, and it then swallows the next
//! keystroke meant for the application. Here a reply that never comes costs the timeout and nothing
//! else, and anything typed while the probe waits is handed back to the caller in order.
//!
//! On Windows the console delivers input as records and replies never arrive, so nothing is asked
//! there; Windows Terminal is recognised from its environment instead.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use super::graphics::{self, CellSize, Decision, Env, Probe, Protocol};
use crate::input::Event;

crate::provenance! {
    component: "preview::probe",
    about: "Asks the terminal for kitty graphics, sixel and its cell size through `input`, so an unanswered query costs no keystroke",
    origin: crate::Origin::Here,
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// The image id the kitty query names, so its reply can be told from any other image's.
pub const KITTY_QUERY_ID: u32 = 31;

/// The kitty graphics query (`a=q`, a one-pixel RGB image that is never stored), the cell-size query
/// (`CSI 16 t`), DA1, and the status report that ends them.
pub const QUERY: &str = "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[16t\x1b[c\x1b[5n";

/// How long [`ask`] waits for the status report before deciding with what it has.
///
/// The report normally arrives within one round trip; this bounds a terminal that answers nothing at
/// all, and a slow remote link.
pub const TIMEOUT: Duration = Duration::from_secs(1);

/// The replies to [`QUERY`], gathered from events as they arrive.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Replies {
    kitty: bool,
    sixel: bool,
    cell: Option<CellSize>,
    finished: bool,
}

impl Replies {
    /// Take one event. True when it was a reply to [`QUERY`], which the application should not see;
    /// false when it belongs to the application.
    pub fn observe(&mut self, event: &Event) -> bool {
        match event {
            Event::KittyGraphics { id, ok } if *id == KITTY_QUERY_ID => {
                self.kitty = *ok;
                true
            }
            Event::DeviceAttributes(attributes) => {
                // The first value is the conformance level; what the terminal offers follows it.
                self.sixel = attributes.iter().skip(1).any(|attribute| *attribute == 4);
                true
            }
            Event::CellSize { width, height } => {
                self.cell = CellSize::new(*width, *height);
                true
            }
            Event::Status { .. } => {
                self.finished = true;
                true
            }
            _ => false,
        }
    }

    /// The status report has arrived, so every reply that is coming has come.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// What the terminal claimed.
    #[must_use]
    pub fn probe(&self) -> Probe {
        Probe {
            kitty: self.kitty,
            sixel: self.sixel,
        }
    }

    /// The cell size the terminal reported, if it did.
    #[must_use]
    pub fn cell(&self) -> Option<CellSize> {
        self.cell
    }
}

/// What [`ask`] heard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asked {
    /// The terminal's replies.
    pub replies: Replies,
    /// Every event that was not a reply, in the order it arrived — keys typed while the probe waited.
    pub passed: Vec<Event>,
}

/// Write [`QUERY`] to `out` and read replies through `next` until the status report ends them or
/// `timeout` passes.
///
/// `next` waits at most the duration it is given for one event — `EventStream::next_timeout`, for
/// the application's stream. Run this before a `follow::Follower` starts, or it may take the DA1 reply
/// the follower is waiting for.
///
/// # Errors
///
/// When writing the query fails, or `next` does.
pub fn ask(
    out: &mut dyn Write,
    next: &mut dyn FnMut(Duration) -> io::Result<Option<Event>>,
    timeout: Duration,
) -> io::Result<Asked> {
    let mut asked = Asked::default();
    if cfg!(not(unix)) {
        return Ok(asked);
    }
    out.write_all(QUERY.as_bytes())?;
    out.flush()?;
    let deadline = Instant::now() + timeout;
    while !asked.replies.finished() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        if let Some(event) = next(left)?
            && !asked.replies.observe(&event)
        {
            asked.passed.push(event);
        }
    }
    Ok(asked)
}

/// What [`detect`] settled on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    /// The protocol, and why.
    pub decision: Decision,
    /// The cell size to encode for: the terminal's report, else the kernel's, else
    /// [`CellSize::FALLBACK`].
    pub cell: CellSize,
    /// Whether the terminal was asked at all.
    pub probed: bool,
    /// Events read while waiting that were not replies, in order, for the caller to handle.
    pub passed: Vec<Event>,
}

/// Decide the protocol, asking the terminal only when the environment leaves it open.
///
/// Inside a multiplexer nothing is written to `out` and nothing is read: [`graphics::decide`] settles
/// those from the environment, so no answer can be believed by mistake.
///
/// # Errors
///
/// When the probe's write or read fails.
pub fn detect(
    choice: Option<Protocol>,
    env: &Env,
    is_terminal: bool,
    out: &mut dyn Write,
    next: &mut dyn FnMut(Duration) -> io::Result<Option<Event>>,
    timeout: Duration,
) -> io::Result<Detected> {
    let decision = graphics::decide(choice, env, is_terminal, None);
    if !decision.needs_probe {
        return Ok(Detected {
            decision,
            cell: CellSize::from_window().unwrap_or(CellSize::FALLBACK),
            probed: false,
            passed: Vec::new(),
        });
    }
    let asked = ask(out, next, timeout)?;
    Ok(Detected {
        decision: graphics::decide(choice, env, is_terminal, Some(asked.replies.probe())),
        cell: asked
            .replies
            .cell()
            .or_else(CellSize::from_window)
            .unwrap_or(CellSize::FALLBACK),
        probed: true,
        passed: asked.passed,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use crossterm::event::{Event as TerminalEvent, KeyCode};

    use super::*;

    /// Every reply a terminal claiming everything would send.
    fn lying_replies() -> VecDeque<Event> {
        VecDeque::from([
            Event::KittyGraphics {
                id: KITTY_QUERY_ID,
                ok: true,
            },
            Event::CellSize {
                width: 9,
                height: 18,
            },
            Event::DeviceAttributes(vec![65, 4, 22]),
            Event::Status { ok: true },
        ])
    }

    fn key(character: char) -> Event {
        Event::Terminal(TerminalEvent::Key(KeyCode::Char(character).into()))
    }

    #[test]
    fn inside_herdr_nothing_is_asked_and_a_lying_terminal_is_not_heard() {
        let env = Env {
            term_program: "herdr".into(),
            ..Env::default()
        };
        let mut written = Vec::new();
        let mut replies = lying_replies();
        let mut reads = 0;
        let detected = detect(
            None,
            &env,
            true,
            &mut written,
            &mut |_| {
                reads += 1;
                Ok(replies.pop_front())
            },
            TIMEOUT,
        )
        .expect("no I/O happens");
        assert_eq!(detected.decision.protocol, Protocol::Halfblocks);
        assert!(!detected.probed);
        assert!(written.is_empty(), "a query was sent into herdr");
        assert_eq!(reads, 0, "a reply was read inside herdr");
    }

    #[cfg(unix)]
    #[test]
    fn an_unknown_terminal_is_asked_and_its_answers_decide() {
        let env = Env {
            term: "xterm-256color".into(),
            ..Env::default()
        };
        let mut written = Vec::new();
        let mut replies = lying_replies();
        replies.insert(1, key('j'));
        let detected = detect(
            None,
            &env,
            true,
            &mut written,
            &mut |_| Ok(replies.pop_front()),
            TIMEOUT,
        )
        .expect("no I/O fails");
        assert_eq!(written, QUERY.as_bytes());
        assert!(detected.probed);
        assert_eq!(detected.decision.protocol, Protocol::Kitty);
        assert_eq!(
            detected.cell,
            CellSize {
                width: 9,
                height: 18
            }
        );
        assert_eq!(
            detected.passed,
            vec![key('j')],
            "a key typed during the probe was lost"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_terminal_that_answers_only_the_status_report_gets_halfblocks_at_once() {
        let env = Env::default();
        let mut replies = VecDeque::from([Event::Status { ok: true }, key('q')]);
        let started = Instant::now();
        let detected = detect(
            None,
            &env,
            true,
            &mut io::sink(),
            &mut |_| Ok(replies.pop_front()),
            Duration::from_secs(30),
        )
        .expect("no I/O fails");
        assert_eq!(detected.decision.protocol, Protocol::Halfblocks);
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the status report did not end the wait"
        );
        // Read after the status report ended the probe, so it is still the application's to read.
        assert_eq!(replies.pop_front(), Some(key('q')));
    }

    #[cfg(unix)]
    #[test]
    fn a_terminal_that_answers_nothing_is_given_up_on_at_the_timeout() {
        let mut calls = 0;
        let asked = ask(
            &mut io::sink(),
            &mut |wait| {
                calls += 1;
                std::thread::sleep(wait.min(Duration::from_millis(5)));
                Ok(None)
            },
            Duration::from_millis(40),
        )
        .expect("no I/O fails");
        assert!(!asked.replies.finished());
        assert_eq!(asked.replies.probe(), Probe::default());
        assert!(calls > 0);
    }

    #[test]
    fn the_query_names_the_id_its_reply_is_matched_by() {
        assert!(QUERY.starts_with(&format!("\x1b_Gi={KITTY_QUERY_ID},")));
        assert!(
            QUERY.ends_with("\x1b[5n"),
            "the status report must be asked last"
        );
    }

    #[test]
    fn device_attributes_name_sixel_only_after_the_conformance_level() {
        let mut replies = Replies::default();
        assert!(replies.observe(&Event::DeviceAttributes(vec![62, 1, 22])));
        assert!(!replies.probe().sixel);
        assert!(replies.observe(&Event::DeviceAttributes(vec![64, 4])));
        assert!(replies.probe().sixel);
        assert!(!replies.observe(&key('x')), "a key is not a reply");
        assert!(
            !replies.observe(&Event::KittyGraphics { id: 7, ok: true }),
            "another image's reply is not this query's"
        );
        assert!(!replies.probe().kitty);
    }
}
