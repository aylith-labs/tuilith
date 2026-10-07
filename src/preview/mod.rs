//! Image previews in a terminal: which graphics protocol it can show, encoding off the UI thread, a
//! cache that keeps encoded pictures, and the widget that draws one.
//!
//! The parts, in the order an application meets them:
//!
//! - [`graphics`] decides the protocol — kitty, sixel, halfblocks or none — from the environment first
//!   and a terminal query second. A multiplexer answers queries about *itself*, so inside one nothing
//!   is asked and nothing asked is believed.
//! - [`probe`] asks the terminal, through [`crate::input`], so a reply that never comes costs a
//!   timeout and not the next keystroke.
//! - [`picker()`] builds ratatui-image's encoder for the decided protocol and cell size.
//! - [`Worker`] decodes, resizes and encodes on a bounded pool of threads, and drops a result a newer
//!   request for the same slot has made stale.
//! - [`PreviewCache`] keeps encoded pictures by source, revision, box, cell size and protocol, bounded
//!   by encoded bytes, so drawing the same picture again neither re-encodes it nor — for kitty —
//!   transmits it again.
//! - [`Image`] draws a cached picture, a placeholder with a spinner while one is being encoded, or a
//!   metadata card where pictures cannot be shown.
//!
//! ```no_run
//! use std::io::stdout;
//! use tuilith::input::EventStream;
//! use tuilith::preview::{self, Key, PreviewCache, Source, Worker, graphics};
//!
//! # fn main() -> std::io::Result<()> {
//! // In raw mode, before the event loop: the probe reads its answers from the same stream.
//! let events = EventStream::new()?;
//! let detected = preview::probe::detect(
//!     None,
//!     &graphics::Env::from_process(),
//!     true,
//!     &mut stdout(),
//!     &mut |wait| events.next_timeout(wait),
//!     preview::probe::TIMEOUT,
//! )?;
//! // `detected.passed` holds anything typed meanwhile; handle it like any other event.
//!
//! let worker = Worker::new(2)?;
//! let mut cache = PreviewCache::new(64 << 20);
//! let key = Key {
//!     source: "pictures/sunset.png".into(),
//!     revision: "1767225600".into(),
//!     columns: 40,
//!     rows: 12,
//!     cell: detected.cell,
//!     protocol: detected.decision.protocol,
//! };
//! if cache.get(&key).is_none() {
//!     worker.submit(1, key.clone(), Source::Path("pictures/sunset.png".into()));
//! }
//! // Each tick:
//! for done in worker.poll() {
//!     if let Ok(encoded) = done.outcome {
//!         cache.insert(done.key, encoded);
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod cache;
pub mod graphics;
pub mod picker;
pub mod probe;
pub mod widget;
pub mod worker;

pub use cache::{Encoded, Key, PreviewCache};
pub use graphics::{CellSize, Decision, Env, Probe, Protocol};
pub use picker::picker;
pub use widget::{Drawn, Image, Meta, Shows};
pub use worker::{Done, Slot, Source, Worker};

/// ratatui-image, for the encoded protocol types this module hands out.
pub use ratatui_image;
