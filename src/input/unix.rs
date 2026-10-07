//! The reader thread: `poll(2)` over the terminal and a wake socket, one read per wakeup, every byte
//! through the vendored parser.
//!
//! Shaped after crossterm 0.29's `source/unix/mio.rs`, with two differences that are the reason it is
//! not a copy. It polls level-triggered with `poll(2)` rather than through mio's edge-triggered
//! epoll, so a read that leaves bytes behind is woken again instead of waiting for the next keystroke.
//! And a lone ESC at the end of a read is held for [`ESCAPE_GRACE`] rather than read as the Esc key at
//! once: a reply split across two reads starts with one, and on WSL replies do split.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::thread::{Builder, JoinHandle};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};

use super::{InternalEvent, Shared, parse};

/// How long a lone ESC waits for the rest of a sequence before it is the Esc key.
///
/// Long enough for the second half of a reply that crossed a read boundary, short enough that a
/// person pressing Esc does not notice.
const ESCAPE_GRACE: Duration = Duration::from_millis(20);

/// One read's worth. Upstream measured reads of at most 1 022 bytes; a longer burst simply takes
/// another wakeup, which level-triggered polling guarantees.
const BUFFER: usize = 1_024;

/// The terminal to read: the first standard stream that is one, and `/dev/tty` only when none is.
///
/// crossterm goes straight from stdin to `/dev/tty`. Here stderr and stdout come first, because a
/// terminal emulator opens its pty read-write and hands the same device to all three streams, and
/// because macOS `poll(2)` does not work on `/dev/tty` — so piping stdin into an application there
/// would leave it with a reader that never wakes.
pub(super) fn tty() -> io::Result<File> {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Ok(File::from(stdin.as_fd().try_clone_to_owned()?));
    }
    let stderr = io::stderr();
    if stderr.is_terminal() {
        return Ok(File::from(stderr.as_fd().try_clone_to_owned()?));
    }
    let stdout = io::stdout();
    if stdout.is_terminal() {
        return Ok(File::from(stdout.as_fd().try_clone_to_owned()?));
    }
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
}

/// What stops the reader: a byte on the socket it polls beside the terminal.
pub(super) struct Stop {
    socket: UnixStream,
    window_change: signal_hook::SigId,
}

impl Stop {
    /// Wake the reader so it sees it has been closed.
    pub(super) fn wake(&self) {
        let _ = (&self.socket).write_all(&[0]);
    }
}

impl Drop for Stop {
    fn drop(&mut self) {
        signal_hook::low_level::unregister(self.window_change);
    }
}

/// Start the reader on `tty`, delivering into `shared`.
pub(super) fn spawn(tty: File, shared: Arc<Shared>) -> io::Result<(JoinHandle<()>, Stop)> {
    // One socket carries both wakeups: SIGWINCH writes to it from the signal handler, and dropping
    // the stream writes to it to stop the thread. The thread tells them apart by asking whether it
    // has been closed.
    let (wake, wake_write) = UnixStream::pair()?;
    wake.set_nonblocking(true)?;
    let window_change = signal_hook::low_level::pipe::register(
        signal_hook::consts::SIGWINCH,
        wake_write.try_clone()?,
    )?;
    let stop = Stop {
        socket: wake_write,
        window_change,
    };
    let thread = Builder::new()
        .name(String::from("tuilith-input"))
        .spawn(move || read(&tty, &wake, &shared))?;
    Ok((thread, stop))
}

/// The reader loop. Returns when the stream is dropped or the terminal goes away.
fn read(mut tty: &File, mut wake: &UnixStream, shared: &Shared) {
    let mut parser = Parser::default();
    let mut buffer = [0_u8; BUFFER];
    let grace = Timespec::try_from(ESCAPE_GRACE).ok();

    loop {
        let timeout = if parser.awaiting_escape() {
            grace.as_ref()
        } else {
            None
        };
        let mut fds = [
            PollFd::new(&tty, PollFlags::IN),
            PollFd::new(&wake, PollFlags::IN),
        ];
        let ready = match poll(&mut fds, timeout) {
            Ok(ready) => ready,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => {
                fail(shared, error.into());
                return;
            }
        };
        let (terminal, woken) = (fds[0].revents(), fds[1].revents());

        if ready == 0 {
            // Nothing followed the ESC within the grace: it was the key.
            parser.flush_escape();
        }

        if terminal.contains(PollFlags::IN) {
            match tty.read(&mut buffer) {
                Ok(0) => {
                    shared.close();
                    return;
                }
                Ok(count) => parser.advance(&buffer[..count]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => {
                    fail(shared, error);
                    return;
                }
            }
        } else if terminal.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            shared.close();
            return;
        }

        if woken.contains(PollFlags::IN) {
            let mut drain = [0_u8; 64];
            while matches!(wake.read(&mut drain), Ok(count) if count > 0) {}
            if shared.is_closed() {
                return;
            }
            match crossterm::terminal::size() {
                Ok((columns, rows)) => {
                    let resize = crossterm::event::Event::Resize(columns, rows);
                    parser.events.push_back(InternalEvent::Event(resize));
                }
                Err(error) => {
                    fail(shared, error);
                    return;
                }
            }
        }

        while let Some(event) = parser.events.pop_front() {
            if let Some(event) = event.public() {
                shared.push(Ok(event));
            }
        }
    }
}

fn fail(shared: &Shared, error: io::Error) {
    shared.push(Err(error));
    shared.close();
}

/// Bytes in, events out — upstream's `Parser`, plus the held ESC.
#[derive(Default)]
struct Parser {
    buffer: Vec<u8>,
    events: VecDeque<InternalEvent>,
}

impl Parser {
    fn advance(&mut self, bytes: &[u8]) {
        for (index, byte) in bytes.iter().enumerate() {
            let more = index + 1 < bytes.len();
            self.buffer.push(*byte);
            if !more && self.awaiting_escape() {
                // Held, not parsed: see `ESCAPE_GRACE`.
                return;
            }
            self.parse(more);
        }
    }

    fn parse(&mut self, more: bool) {
        match parse::parse_event(&self.buffer, more) {
            Ok(Some(event)) => {
                self.events.push_back(event);
                self.buffer.clear();
            }
            // Not enough bytes for the sequence yet: keep them.
            Ok(None) => {}
            // Not a sequence anything reads: drop it and carry on with the next.
            Err(_) => self.buffer.clear(),
        }
    }

    fn awaiting_escape(&self) -> bool {
        self.buffer == [0x1B]
    }

    fn flush_escape(&mut self) {
        if self.awaiting_escape() {
            self.parse(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::theme::Mode;

    fn key(code: KeyCode) -> InternalEvent {
        InternalEvent::Event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    #[test]
    fn keys_typed_after_a_scheme_report_still_arrive() {
        let mut parser = Parser::default();
        parser.advance(b"\x1B[?997;2nj");
        assert_eq!(
            parser.events.drain(..).collect::<Vec<_>>(),
            vec![
                InternalEvent::ColorScheme(Mode::Light),
                key(KeyCode::Char('j'))
            ],
        );
    }

    #[test]
    fn a_reply_split_after_its_escape_is_still_one_reply() {
        let mut parser = Parser::default();
        parser.advance(b"\x1B");
        assert!(
            parser.events.is_empty(),
            "a lone ESC is held, not read as the key"
        );
        assert!(parser.awaiting_escape());
        parser.advance(b"]11;rgb:0000/0000/0000\x07");
        assert_eq!(
            parser.events.drain(..).collect::<Vec<_>>(),
            vec![InternalEvent::Background(super::super::Rgb {
                red: 0,
                green: 0,
                blue: 0
            })],
        );
    }

    #[test]
    fn a_lone_escape_becomes_the_key_once_the_grace_runs_out() {
        let mut parser = Parser::default();
        parser.advance(b"\x1B");
        parser.flush_escape();
        assert_eq!(
            parser.events.drain(..).collect::<Vec<_>>(),
            vec![key(KeyCode::Esc)]
        );
    }
}
