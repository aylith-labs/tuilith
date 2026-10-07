//! Decoding, resizing and encoding off the UI thread, on a bounded pool, keeping only the newest answer.
//!
//! A preview follows a selection, and a selection moves faster than a picture encodes: holding `j`
//! through a directory of photographs asks for a dozen pictures a second, for one pane. Each request
//! names the *slot* it is for — a pane, a tile, whatever place shows one picture at a time — and gets
//! a generation number. A newer request for the same slot makes every older one stale: a stale request
//! still waiting in the queue is dropped before it starts, one already running finishes but its result
//! is never delivered, and [`Worker::poll`] only ever hands back the newest answer for each slot.
//!
//! The pool is a fixed number of threads, so a burst of requests is a queue, never a thread each; and
//! the queue holds at most one waiting request per slot, because each new one replaces the last.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Cursor};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{Builder, JoinHandle};
use std::time::{Duration, Instant};

use ratatui::layout::Size;
use ratatui_image::{FilterType, Resize};

use super::cache::{Encoded, Key};
use super::picker::picker;

crate::provenance! {
    component: "preview::worker",
    about: "A bounded thread pool that decodes, resizes and encodes pictures, delivering only the newest answer for each slot",
    origin: crate::Origin::Here,
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// Where a picture will be shown: one pane, one tile. A newer request for a slot supersedes the older.
pub type Slot = u64;

/// Where the picture's bytes come from.
#[derive(Clone, Debug)]
pub enum Source {
    /// A file, read on the worker thread.
    Path(PathBuf),
    /// Bytes already in memory.
    Bytes(Arc<[u8]>),
}

/// A finished request.
#[derive(Debug)]
pub struct Done {
    /// The slot it was for.
    pub slot: Slot,
    /// Its generation, as [`Worker::submit`] returned it.
    pub generation: u64,
    /// What was asked for — the key to cache it under.
    pub key: Key,
    /// The encoded picture, or why there is none.
    pub outcome: Result<Arc<Encoded>, String>,
}

struct Task {
    slot: Slot,
    generation: u64,
    key: Key,
    source: Source,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Task>,
    /// The newest generation asked for each slot whose answer has not been delivered.
    latest: HashMap<Slot, u64>,
    next_generation: u64,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic is caught around the work itself, never while this is held, so the state is whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_current(&self, slot: Slot, generation: u64) -> bool {
        self.lock().latest.get(&slot) == Some(&generation)
    }
}

/// A fixed pool of threads encoding pictures. See the module docs.
pub struct Worker {
    shared: Arc<Shared>,
    results: Receiver<Done>,
    threads: Vec<JoinHandle<()>>,
}

impl Worker {
    /// The most threads a pool starts, whatever it is asked for.
    pub const MAX_THREADS: usize = 8;

    /// Start `threads` workers, at least one and at most [`MAX_THREADS`](Self::MAX_THREADS).
    ///
    /// # Errors
    ///
    /// When a thread cannot be started. Any already started are stopped again.
    pub fn new(threads: usize) -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let (sender, results) = mpsc::channel();
        let mut worker = Self {
            shared,
            results,
            threads: Vec::new(),
        };
        for index in 0..threads.clamp(1, Self::MAX_THREADS) {
            let shared = Arc::clone(&worker.shared);
            let sender = sender.clone();
            let thread = Builder::new()
                .name(format!("tuilith-preview-{index}"))
                .spawn(move || run(&shared, &sender))?;
            worker.threads.push(thread);
        }
        Ok(worker)
    }

    /// Ask for `key`'s picture from `source`, for `slot`. Returns the request's generation.
    ///
    /// Every earlier request for `slot` is stale from this moment: dropped if it has not started, and
    /// never delivered if it has.
    // The generation is for a caller that tracks its requests; most only watch `is_pending`.
    #[allow(clippy::must_use_candidate)]
    pub fn submit(&self, slot: Slot, key: Key, source: Source) -> u64 {
        let generation = {
            let mut state = self.shared.lock();
            state.next_generation += 1;
            let generation = state.next_generation;
            state.latest.insert(slot, generation);
            state.queue.retain(|task| task.slot != slot);
            state.queue.push_back(Task {
                slot,
                generation,
                key,
                source,
            });
            generation
        };
        self.shared.ready.notify_one();
        generation
    }

    /// Withdraw whatever was asked for `slot`: nothing for it is delivered until it is asked again.
    pub fn cancel(&self, slot: Slot) {
        let mut state = self.shared.lock();
        state.latest.remove(&slot);
        state.queue.retain(|task| task.slot != slot);
    }

    /// Whether a request for `slot` is queued or running and its answer not yet delivered — the time
    /// to draw a placeholder.
    #[must_use]
    pub fn is_pending(&self, slot: Slot) -> bool {
        self.shared.lock().latest.contains_key(&slot)
    }

    /// Every answer that has arrived and is still the newest for its slot. Never blocks.
    #[must_use]
    pub fn poll(&self) -> Vec<Done> {
        let mut delivered = Vec::new();
        while let Ok(done) = self.results.try_recv() {
            self.deliver(done, &mut delivered);
        }
        delivered
    }

    /// Like [`poll`](Self::poll), but waits up to `timeout` for the first current answer.
    #[must_use]
    pub fn wait(&self, timeout: Duration) -> Vec<Done> {
        let deadline = Instant::now() + timeout;
        let mut delivered = Vec::new();
        while delivered.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.results.recv_timeout(left) {
                Ok(done) => self.deliver(done, &mut delivered),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        delivered.extend(self.poll());
        delivered
    }

    fn deliver(&self, done: Done, delivered: &mut Vec<Done>) {
        let mut state = self.shared.lock();
        if state.latest.get(&done.slot) == Some(&done.generation) {
            state.latest.remove(&done.slot);
            delivered.push(done);
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.ready.notify_all();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// One pool thread: take the oldest request, encode it, send the answer if it is still wanted.
fn run(shared: &Shared, results: &Sender<Done>) {
    loop {
        let task = {
            let mut state = shared.lock();
            loop {
                if state.closed {
                    return;
                }
                if let Some(task) = state.queue.pop_front() {
                    break task;
                }
                state = shared
                    .ready
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        // A decoder that panics on a malformed file must not take a pool thread with it.
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| encode(&task.key, &task.source)))
            .unwrap_or_else(|_| Err("the decoder panicked".to_owned()));
        if !shared.is_current(task.slot, task.generation) {
            continue;
        }
        let done = Done {
            slot: task.slot,
            generation: task.generation,
            key: task.key,
            outcome: outcome.map(Arc::new),
        };
        if results.send(done).is_err() {
            return;
        }
    }
}

/// Read, decode, fit to the key's box and encode in the key's protocol.
fn encode(key: &Key, source: &Source) -> Result<Encoded, String> {
    let Some(encoder) = picker(key.protocol, key.cell) else {
        return Err(format!("{} draws no pictures", key.protocol));
    };
    if key.columns == 0 || key.rows == 0 {
        return Err("no room to draw in".to_owned());
    }
    let read;
    let bytes: &[u8] = match source {
        Source::Bytes(bytes) => &bytes[..],
        Source::Path(path) => {
            read = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
            &read[..]
        }
    };
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let format = reader
        .format()
        .and_then(|format| format.extensions_str().first().copied());
    let decoded = reader.decode().map_err(|error| error.to_string())?;
    let pixels = (decoded.width(), decoded.height());
    let encoding = encoder
        .new_protocol(
            decoded,
            Size::new(key.columns, key.rows),
            Resize::Fit(Some(FilterType::Triangle)),
        )
        .map_err(|error| error.to_string())?;
    Ok(Encoded::new(encoding, key.cell, pixels, format))
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

    use super::*;
    use crate::preview::graphics::{CellSize, Protocol};

    /// Generous: a liveness bound for the slowest CI machine, not a performance claim.
    const LIVENESS: Duration = Duration::from_secs(30);

    fn png(width: u32, height: u32) -> Source {
        let image = RgbaImage::from_pixel(width, height, Rgba([200, 40, 40, 255]));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .expect("a synthetic picture encodes as PNG");
        Source::Bytes(Arc::from(bytes))
    }

    fn key(source: &str, protocol: Protocol) -> Key {
        Key {
            source: source.to_owned(),
            revision: "1".to_owned(),
            columns: 8,
            rows: 4,
            cell: CellSize {
                width: 10,
                height: 20,
            },
            protocol,
        }
    }

    /// Everything delivered until none of `slots` is pending, or the liveness bound passes.
    fn settle(worker: &Worker, slots: &[Slot]) -> Vec<Done> {
        let deadline = Instant::now() + LIVENESS;
        let mut delivered = Vec::new();
        while slots.iter().any(|slot| worker.is_pending(*slot)) {
            assert!(Instant::now() < deadline, "the pool never answered");
            delivered.extend(worker.wait(Duration::from_millis(50)));
        }
        delivered
    }

    #[test]
    fn a_picture_is_decoded_fitted_and_encoded() {
        let worker = Worker::new(2).expect("threads start");
        let generation = worker.submit(1, key("red", Protocol::Halfblocks), png(160, 80));
        assert!(worker.is_pending(1));
        let delivered = settle(&worker, &[1]);
        assert_eq!(delivered.len(), 1);
        let done = &delivered[0];
        assert_eq!(done.generation, generation);
        let encoded = done.outcome.as_ref().expect("a PNG encodes");
        assert_eq!(encoded.pixels(), (160, 80));
        assert_eq!(encoded.format(), Some("png"));
        let (columns, rows) = encoded.cells();
        assert!(columns <= 8 && rows <= 4, "it does not fit its box");
        assert!(columns > 0 && rows > 0, "it encoded to nothing");
        assert!(encoded.bytes() > 0);
    }

    #[test]
    fn a_newer_request_for_a_slot_makes_the_older_one_stale() {
        // One thread, so the second request for slot 1 is queued behind the first, or replaces it.
        let worker = Worker::new(1).expect("a thread starts");
        let older = worker.submit(1, key("older", Protocol::Halfblocks), png(64, 64));
        let newer = worker.submit(1, key("newer", Protocol::Halfblocks), png(64, 64));
        let other = worker.submit(2, key("other", Protocol::Halfblocks), png(32, 32));
        assert!(older < newer && newer < other);

        let delivered = settle(&worker, &[1, 2]);
        let for_slot = |slot: Slot| -> Vec<&str> {
            delivered
                .iter()
                .filter(|done| done.slot == slot)
                .map(|done| done.key.source.as_str())
                .collect()
        };
        assert_eq!(for_slot(1), vec!["newer"], "a stale answer was delivered");
        assert_eq!(for_slot(2), vec!["other"], "another slot's request was lost");
        assert!(worker.poll().is_empty());
    }

    #[test]
    fn a_cancelled_slot_delivers_nothing() {
        let worker = Worker::new(1).expect("a thread starts");
        worker.submit(3, key("gone", Protocol::Halfblocks), png(64, 64));
        worker.cancel(3);
        assert!(!worker.is_pending(3));
        // Whether it was dropped from the queue or finished anyway, its answer is never delivered.
        let witness = worker.submit(4, key("witness", Protocol::Halfblocks), png(8, 8));
        let delivered = settle(&worker, &[4]);
        assert!(delivered.iter().all(|done| done.slot != 3));
        assert!(delivered.iter().any(|done| done.generation == witness));
    }

    #[test]
    fn bytes_that_are_not_a_picture_come_back_as_an_error() {
        let worker = Worker::new(1).expect("a thread starts");
        worker.submit(
            1,
            key("text", Protocol::Halfblocks),
            Source::Bytes(Arc::from(&b"plain text, not a picture"[..])),
        );
        let delivered = settle(&worker, &[1]);
        assert_eq!(delivered.len(), 1);
        assert!(delivered[0].outcome.is_err());
    }

    #[test]
    fn a_missing_file_comes_back_as_an_error() {
        let directory = std::env::temp_dir().join(format!("tuilith-preview-{}", std::process::id()));
        let worker = Worker::new(1).expect("a thread starts");
        worker.submit(
            1,
            key("missing", Protocol::Halfblocks),
            Source::Path(directory.join("no-such-picture.png")),
        );
        let delivered = settle(&worker, &[1]);
        assert!(delivered[0].outcome.is_err());
    }

    #[test]
    fn no_pictures_means_nothing_is_encoded() {
        let worker = Worker::new(1).expect("a thread starts");
        worker.submit(1, key("card", Protocol::None), png(8, 8));
        let delivered = settle(&worker, &[1]);
        assert!(delivered[0].outcome.is_err());
    }
}
