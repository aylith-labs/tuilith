//! Encoded pictures, kept so drawing one again costs nothing.
//!
//! Encoding is the expensive part of a preview — decode, resize, and for kitty a base64 payload the
//! size of the raw pixels — and the terminal's side is expensive too: a kitty picture is transmitted
//! once and then placed by its id. So the cache keeps the *encoded* protocol, and hands back the same
//! one for the same key. A preview scrolled away and back is drawn from the entry it left: no
//! re-encode, and for kitty no second transmission, because ratatui-image marks the shared payload
//! sent on its first draw. A sixel picture is the exception the protocol imposes: sixel has no
//! placement by id, so moving one on screen sends its data again — but it is not encoded again.
//!
//! The key holds everything an encoding depends on, so a stale entry is never served: a new revision
//! of the source, a resized box, a changed cell size or protocol each miss and encode afresh.
//! [`PreviewCache::invalidate`] drops every entry of a source at once, for a file known to have
//! changed or been deleted. Entries are evicted least recently used first, by encoded bytes rather
//! than by count, because one full-screen kitty picture outweighs a hundred thumbnails.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use ratatui_image::protocol::Protocol as Encoding;
use ratatui_image::protocol::halfblocks::HalfBlock;

use super::graphics::{CellSize, Protocol};

crate::provenance! {
    component: "preview::cache",
    about: "Encoded pictures keyed by source, revision, box, cell size and protocol, evicted least recently used by encoded bytes",
    origin: crate::Origin::Here,
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// Everything an encoding depends on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    /// What the picture is of — a path, a URL, an id the caller understands.
    pub source: String,
    /// Which version of it: a modification time, a hash, an etag. A new one is a new picture.
    pub revision: String,
    /// The box it was encoded to fit, in cells across.
    pub columns: u16,
    /// The box it was encoded to fit, in cells down.
    pub rows: u16,
    /// The cell size it was encoded for.
    pub cell: CellSize,
    /// The protocol it was encoded in.
    pub protocol: Protocol,
}

/// A picture encoded for one protocol and box, ready to draw.
pub struct Encoded {
    encoding: Encoding,
    bytes: usize,
    pixels: (u32, u32),
    format: Option<&'static str>,
}

/// What a kitty placeholder cell costs beyond the payload: the placeholder character and its
/// row and column diacritics.
const PLACEHOLDER_BYTES: usize = 12;

impl Encoded {
    /// Wrap an encoding. `cell` is the cell size it was encoded for, `pixels` the source picture's
    /// size, and `format` its file format's usual extension.
    #[must_use]
    pub fn new(
        encoding: Encoding,
        cell: CellSize,
        pixels: (u32, u32),
        format: Option<&'static str>,
    ) -> Self {
        let bytes = payload_bytes(&encoding, cell);
        Self {
            encoding,
            bytes,
            pixels,
            format,
        }
    }

    /// The encoded protocol, for ratatui-image's `Image` widget.
    #[must_use]
    pub fn encoding(&self) -> &Encoding {
        &self.encoding
    }

    /// What this costs to hold, which is what the cache's budget counts.
    ///
    /// Exact for sixel, whose payload is a string. For kitty it is the base64 of the RGBA pixels the
    /// encoded box covers, an upper bound on what was transmitted; for halfblocks, the cells held.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The cells it covers when drawn, across and down.
    #[must_use]
    pub fn cells(&self) -> (u16, u16) {
        let size = self.encoding.size();
        (size.width, size.height)
    }

    /// The source picture's size in pixels.
    #[must_use]
    pub fn pixels(&self) -> (u32, u32) {
        self.pixels
    }

    /// The source's file format, as its usual extension (`png`), when it was recognised.
    #[must_use]
    pub fn format(&self) -> Option<&'static str> {
        self.format
    }
}

impl fmt::Debug for Encoded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Encoded")
            .field("cells", &self.cells())
            .field("bytes", &self.bytes)
            .field("pixels", &self.pixels)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

fn payload_bytes(encoding: &Encoding, cell: CellSize) -> usize {
    let size = encoding.size();
    let cells = usize::from(size.width) * usize::from(size.height);
    match encoding {
        Encoding::Sixel(sixel) => sixel.data.len(),
        Encoding::Kitty(_) | Encoding::ITerm2(_) => {
            let pixels = cells * usize::from(cell.width) * usize::from(cell.height);
            pixels * 4 * 4 / 3 + cells * PLACEHOLDER_BYTES
        }
        Encoding::Halfblocks(_) => cells * std::mem::size_of::<HalfBlock>(),
    }
}

struct Entry {
    encoded: Arc<Encoded>,
    used_at: u64,
}

/// Encoded pictures by [`Key`], bounded by their encoded bytes.
pub struct PreviewCache {
    budget: usize,
    used: usize,
    clock: u64,
    entries: HashMap<Key, Entry>,
}

impl PreviewCache {
    /// A cache holding at most `budget` encoded bytes.
    #[must_use]
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            used: 0,
            clock: 0,
            entries: HashMap::new(),
        }
    }

    /// The entry for `key`, marked as just used. The same allocation every time, so drawing it again
    /// re-encodes nothing.
    pub fn get(&mut self, key: &Key) -> Option<Arc<Encoded>> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.entries.get_mut(key)?;
        entry.used_at = clock;
        Some(Arc::clone(&entry.encoded))
    }

    /// Whether `key` is held, without marking it used.
    #[must_use]
    pub fn contains(&self, key: &Key) -> bool {
        self.entries.contains_key(key)
    }

    /// Keep `encoded` under `key`, replacing what was there, then evict least recently used entries
    /// until the budget holds. Returns how many were evicted.
    ///
    /// The entry just inserted is never evicted by its own insertion, even when it alone exceeds the
    /// budget: the picture being shown is the one picture that must stay drawable.
    pub fn insert(&mut self, key: Key, encoded: Arc<Encoded>) -> usize {
        if let Some(replaced) = self.entries.remove(&key) {
            self.used -= replaced.encoded.bytes();
        }
        self.used += encoded.bytes();
        let evicted = self.evict_to_budget();
        self.clock += 1;
        let entry = Entry {
            encoded,
            used_at: self.clock,
        };
        self.entries.insert(key, entry);
        evicted
    }

    /// Drop every entry of `source`, whatever its revision, box or protocol. Returns how many.
    pub fn invalidate(&mut self, source: &str) -> usize {
        let before = self.entries.len();
        let mut freed = 0;
        self.entries.retain(|key, entry| {
            let keep = key.source != source;
            if !keep {
                freed += entry.encoded.bytes();
            }
            keep
        });
        self.used -= freed;
        before - self.entries.len()
    }

    /// Drop everything — after the terminal's cell size or protocol changed, say.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.used = 0;
    }

    /// The encoded bytes held.
    #[must_use]
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// The most encoded bytes it holds before evicting.
    #[must_use]
    pub fn budget_bytes(&self) -> usize {
        self.budget
    }

    /// How many entries are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Evict least recently used entries until the budget holds, with the bytes of an entry about to
    /// be inserted already counted — so it is never among those evicted.
    fn evict_to_budget(&mut self) -> usize {
        let mut evicted = 0;
        while self.used > self.budget {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used_at)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.used -= entry.encoded.bytes();
                evicted += 1;
            }
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, Rgba, RgbaImage};
    use ratatui::layout::Size;
    use ratatui_image::Resize;

    use super::*;
    use crate::preview::picker::picker;

    const CELL: CellSize = CellSize {
        width: 10,
        height: 20,
    };

    /// A synthetic picture, encoded in halfblocks for a box of `columns` × `rows`.
    fn encoded(columns: u16, rows: u16) -> Arc<Encoded> {
        let pixels = (
            u32::from(columns) * u32::from(CELL.width),
            u32::from(rows) * u32::from(CELL.height),
        );
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            pixels.0,
            pixels.1,
            Rgba([30, 120, 200, 255]),
        ));
        let encoding = picker(Protocol::Halfblocks, CELL)
            .expect("halfblocks has an encoder")
            .new_protocol(image, Size::new(columns, rows), Resize::Fit(None))
            .expect("a synthetic picture encodes");
        Arc::new(Encoded::new(encoding, CELL, pixels, Some("png")))
    }

    fn key(source: &str, revision: &str) -> Key {
        Key {
            source: source.to_owned(),
            revision: revision.to_owned(),
            columns: 4,
            rows: 2,
            cell: CELL,
            protocol: Protocol::Halfblocks,
        }
    }

    #[test]
    fn the_same_key_hands_back_the_same_encoding() {
        let mut cache = PreviewCache::new(1 << 20);
        let picture = encoded(4, 2);
        cache.insert(key("a", "1"), Arc::clone(&picture));
        let first = cache.get(&key("a", "1")).expect("just inserted");
        let again = cache.get(&key("a", "1")).expect("still there");
        assert!(Arc::ptr_eq(&first, &again), "a hit was re-encoded");
        assert!(Arc::ptr_eq(&first, &picture));
        assert!(cache.get(&key("a", "2")).is_none(), "a new revision hit");
    }

    #[test]
    fn eviction_is_by_encoded_bytes_and_least_recently_used_goes_first() {
        let size = encoded(4, 2).bytes();
        assert!(
            size > 0,
            "the fixture costs nothing, so the budget is untestable"
        );
        // Room for two entries and a half: the third insert must evict exactly one.
        let mut cache = PreviewCache::new(size * 5 / 2);
        assert_eq!(cache.insert(key("a", "1"), encoded(4, 2)), 0);
        assert_eq!(cache.insert(key("b", "1"), encoded(4, 2)), 0);
        // Touch `a`, so `b` is now the least recently used.
        assert!(cache.get(&key("a", "1")).is_some());
        assert_eq!(cache.insert(key("c", "1"), encoded(4, 2)), 1);
        assert!(cache.contains(&key("a", "1")));
        assert!(
            !cache.contains(&key("b", "1")),
            "the recently used entry went first"
        );
        assert!(cache.contains(&key("c", "1")));
        assert_eq!(cache.used_bytes(), size * 2);
        assert!(cache.used_bytes() <= cache.budget_bytes());
    }

    #[test]
    fn a_bigger_entry_evicts_several_smaller_ones() {
        let small = encoded(2, 1).bytes();
        let big = encoded(8, 4);
        assert!(big.bytes() > small * 3);
        let mut cache = PreviewCache::new(big.bytes() + small);
        for source in ["a", "b", "c", "d"] {
            cache.insert(key(source, "1"), encoded(2, 1));
        }
        let evicted = cache.insert(key("big", "1"), big);
        assert!(
            evicted >= 3,
            "only {evicted} evicted for a picture of many times the size"
        );
        assert!(cache.used_bytes() <= cache.budget_bytes());
        assert!(cache.contains(&key("big", "1")));
    }

    #[test]
    fn an_entry_over_the_whole_budget_is_kept_and_everything_else_goes() {
        let mut cache = PreviewCache::new(1);
        cache.insert(key("a", "1"), encoded(2, 1));
        assert!(
            cache.contains(&key("a", "1")),
            "the picture being shown was evicted"
        );
        cache.insert(key("b", "1"), encoded(2, 1));
        assert!(!cache.contains(&key("a", "1")));
        assert!(cache.contains(&key("b", "1")));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn replacing_an_entry_does_not_count_it_twice() {
        let mut cache = PreviewCache::new(1 << 20);
        cache.insert(key("a", "1"), encoded(4, 2));
        let once = cache.used_bytes();
        cache.insert(key("a", "1"), encoded(4, 2));
        assert_eq!(cache.used_bytes(), once);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn invalidating_a_source_drops_every_revision_of_it_and_nothing_else() {
        let mut cache = PreviewCache::new(1 << 20);
        cache.insert(key("a", "1"), encoded(4, 2));
        cache.insert(key("a", "2"), encoded(4, 2));
        cache.insert(key("b", "1"), encoded(2, 1));
        let kept = cache.get(&key("b", "1")).expect("inserted").bytes();
        assert_eq!(cache.invalidate("a"), 2);
        assert!(!cache.contains(&key("a", "1")));
        assert!(!cache.contains(&key("a", "2")));
        assert!(cache.contains(&key("b", "1")));
        assert_eq!(cache.used_bytes(), kept);
        assert_eq!(cache.invalidate("nothing"), 0);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.used_bytes(), 0);
    }
}
