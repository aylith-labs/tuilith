//! A finder over folders, git repositories or files: type to filter, pick one.
//!
//! The list draws on the first frame from what is already known — the goto-repo cache, bookmarks, the
//! finder's own snapshot of its last scan — and a scan runs off the drawing thread when that snapshot is
//! older than its TTL. When the scan lands, a row under the pointer does not move: the new list is held,
//! and [`Finder::pending`] says how many rows it would add, until the list is at the top with nothing
//! under the pointer or the person asks for it ([`Finder::show_pending`]).
//!
//! ```no_run
//! # use ratatui::{Frame, layout::Rect};
//! # use tuilith::{find, theme::Palette};
//! # fn frame(frame: &mut Frame, area: Rect, palette: &Palette, finder: &mut find::Finder) {
//! # let (column, row) = (0, 0);
//! finder.poll(); // takes a finished scan, if one landed
//! let painted = find::draw(frame, area, finder, palette, "search folders");
//! // What a click hit-tests, and what the pointer is reported against. Report `None` only when the
//! // pointer is over no row: that is what lets a held scan land.
//! finder.hover(painted.item_at(column, row));
//! # }
//! # fn open() -> find::Finder {
//! let mut config = find::Config::default(); // git repositories, from the goto-repo cache
//! config.sources.bookmarks = true;
//! config.display = find::Display::TwoRows;
//! find::Finder::open(config)
//! # }
//! ```
//!
//! **Everything is the host's to set**, through [`Config`]: which sources, how a row reads, the order,
//! the TTL, where the snapshot lives, where the walk starts and how deep it goes. tuilith reads no config
//! file of its own; a host maps its settings onto the struct. **It draws in the host's [`Palette`]**,
//! so it themes with everything around it.
//!
//! The same component lists files: [`Finder::files`] takes a root — picked with a folder finder — and
//! matches over `git ls-files` there, or over a walk of the folder outside a repository.
//!
//! [`browse`] is the other way to reach a folder: a picker that moves through the tree one folder at a
//! time, with breadcrumbs, home, bookmarks and back.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, SystemTime};

use crate::pick::{Filter, Hit};

pub mod browse;
pub mod cache;
mod draw;
pub mod ranking;
pub mod source;

pub use draw::{Geometry, draw};
pub use ranking::{History, SortMode};
pub use source::{Item, Kind, Sources};

#[cfg(doc)]
use crate::theme::Palette;

crate::provenance! {
    component: "find",
    about: "A finder over folders, repositories or files, drawn at once from a cache and revalidated without moving the row under the pointer",
    origin: crate::Origin::Repo("polygit"),
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// How a row reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    /// The folder's name alone. The query matches the name.
    Name,
    /// The whole path. The query matches the path.
    Path,
    /// The name, with the path right-aligned in the dim colour. The query matches the path.
    #[default]
    NameAndPath,
    /// The name, with the path under it on a second row. The query matches the path.
    TwoRows,
}

/// Everything a host decides about a finder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // Independent switches a host maps one-to-one onto settings.
pub struct Config {
    /// Which sources to list.
    pub sources: Sources,
    /// How a row reads.
    pub display: Display,
    /// The order before anything is typed.
    pub sort: SortMode,
    /// The home folder: the `home` source, and what [`Self::tilde`] shortens.
    pub home: PathBuf,
    /// Where a revalidation walks for repositories and folders.
    pub roots: Vec<PathBuf>,
    /// How many levels below a root the walk goes.
    pub depth: usize,
    /// Folder names the walk never enters.
    pub skip: Vec<String>,
    /// The most items a finder lists.
    pub limit: usize,
    /// The bookmarks source.
    pub bookmarks: Vec<PathBuf>,
    /// The goto-repo cache, `repos.tsv`. Read, never written.
    pub launcher_cache: Option<PathBuf>,
    /// The usage history the recent and most-used orders read, and a pick is appended to.
    pub history: Option<PathBuf>,
    /// Where the finder keeps its own snapshot of its last scan. Without one, every open rescans.
    pub snapshot: Option<PathBuf>,
    /// How old a snapshot may be before an open rescans.
    pub ttl: Duration,
    /// Whether an open may rescan at all. Off, the list is what is already known.
    pub revalidate: bool,
    /// Show the home folder as `~` in a path.
    pub tilde: bool,
    /// Show each item's kind in a column of its own.
    pub show_kind: bool,
    /// Show how many times each item was picked.
    pub show_count: bool,
}

impl Config {
    /// The defaults, around `home`: git repositories from the goto-repo cache under it, its usage history,
    /// a walk of `home` five levels deep that skips the folders build tools fill, a five-minute TTL.
    #[must_use]
    pub fn new(home: PathBuf) -> Self {
        Self {
            sources: Sources::default(),
            display: Display::default(),
            sort: SortMode::default(),
            roots: vec![home.clone()],
            depth: 5,
            skip: [
                "node_modules",
                "target",
                "vendor",
                "dist",
                "build",
                "__pycache__",
            ]
            .map(String::from)
            .to_vec(),
            limit: 20_000,
            bookmarks: Vec::new(),
            launcher_cache: Some(ranking::goto_repo_cache_dir(&home).join("repos.tsv")),
            history: Some(ranking::goto_repo_history_path(&home)),
            snapshot: None,
            ttl: Duration::from_mins(5),
            revalidate: true,
            tilde: true,
            show_kind: false,
            show_count: false,
            home,
        }
    }
}

impl Default for Config {
    /// [`Config::new`] around the process's home folder, or `/` where it has none.
    fn default() -> Self {
        Self::new(std::env::home_dir().unwrap_or_else(|| PathBuf::from("/")))
    }
}

/// What a scan that landed while the list was in use would change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    /// Items it lists that are not on screen.
    pub added: usize,
    /// Items on screen it no longer lists.
    pub removed: usize,
}

/// What feeding a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Still open.
    Open,
    /// Closed without a pick.
    Cancelled,
    /// This was picked. It has been counted in the history already.
    Picked(Item),
}

/// A finder's state: the items, the query, the cursor, and a scan that may be running.
#[derive(Debug)]
pub struct Finder {
    config: Config,
    files_root: Option<PathBuf>,
    items: Vec<Item>,
    /// Item indices in list order, and the text each one matches against, in the same order.
    order: Vec<usize>,
    texts: Vec<String>,
    filter: Filter,
    history: History,
    hovered: Option<usize>,
    /// A scan that landed while the list was in use, and what it would change — counted once when it
    /// lands, since [`draw`] asks on every frame.
    held: Option<(Vec<Item>, Pending)>,
    scan: Option<Receiver<Vec<Item>>>,
}

impl Finder {
    /// Open over the configured sources: draws from the snapshot, or from what is known without a walk,
    /// and starts a rescan on another thread when the snapshot is missing or past its TTL.
    #[must_use]
    pub fn open(config: Config) -> Self {
        let snapshot = config.snapshot.as_deref().and_then(cache::load);
        let fresh = snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.is_stale(config.ttl, SystemTime::now()));
        let items = snapshot.map_or_else(|| source::instant(&config), |snapshot| snapshot.items);
        let rescan = config.revalidate && !fresh;
        let mut finder = Self::with_items(items, config);
        if rescan {
            finder.rescan();
        }
        finder
    }

    /// A finder over items the host already has. Nothing is read and nothing is scanned, though the
    /// configured history still ranks them.
    #[must_use]
    pub fn with_items(items: Vec<Item>, config: Config) -> Self {
        let history = config
            .history
            .clone()
            .map_or_else(History::in_memory, History::load);
        let mut finder = Self {
            config,
            files_root: None,
            items,
            order: Vec::new(),
            texts: Vec::new(),
            filter: Filter::new(),
            history,
            hovered: None,
            held: None,
            scan: None,
        };
        finder.filter.focus(true);
        finder.rebuild(None);
        finder
    }

    /// A finder over the files under `root`. See [`source::files`].
    #[must_use]
    pub fn files(root: PathBuf, config: Config) -> Self {
        let items = source::files(&root, config.limit);
        let mut finder = Self::with_items(items, config);
        finder.files_root = Some(root);
        finder.rebuild(None);
        finder
    }

    /// Start a scan of the configured sources on another thread. [`Self::poll`] takes its answer.
    pub fn rescan(&mut self) {
        let (send, receive) = std::sync::mpsc::channel();
        let config = self.config.clone();
        let spawned = std::thread::Builder::new()
            .name("tuilith-find-scan".into())
            .spawn(move || {
                let items = source::scan(&config);
                if let Some(path) = &config.snapshot {
                    let _ = cache::save(path, &items);
                }
                let _ = send.send(items);
            });
        self.scan = spawned.is_ok().then_some(receive);
    }

    /// Take a scan's answer if one has landed. True when anything changed that a frame should show.
    pub fn poll(&mut self) -> bool {
        let Some(receive) = &self.scan else {
            return false;
        };
        match receive.try_recv() {
            Ok(items) => {
                self.scan = None;
                self.revalidated(items);
                true
            }
            Err(TryRecvError::Disconnected) => {
                self.scan = None;
                true
            }
            Err(TryRecvError::Empty) => false,
        }
    }

    /// Hand the finder a fresh list, as a finished scan does.
    ///
    /// Applied at once only when the list is scrolled to the top and nothing is under the pointer, so
    /// nothing the person is looking at can move; the cursor stays on the item it was on. Otherwise it
    /// is held, and [`Self::pending`] says what it would change.
    pub fn revalidated(&mut self, items: Vec<Item>) {
        if items == self.items {
            self.held = None;
            return;
        }
        if self.hovered.is_none() && self.filter.scroll() == 0 {
            self.held = None;
            self.replace(items);
        } else {
            let shown: HashSet<&Path> = self.items.iter().map(|item| item.path.as_path()).collect();
            let landed: HashSet<&Path> = items.iter().map(|item| item.path.as_path()).collect();
            let pending = Pending {
                added: landed.difference(&shown).count(),
                removed: shown.difference(&landed).count(),
            };
            self.held = Some((items, pending));
        }
    }

    /// What a held scan would change, if one is held.
    #[must_use]
    pub fn pending(&self) -> Option<Pending> {
        self.held.as_ref().map(|(_, pending)| *pending)
    }

    /// Apply a held scan now.
    pub fn show_pending(&mut self) {
        if let Some((items, _)) = self.held.take() {
            self.replace(items);
        }
    }

    /// Whether a scan is running.
    #[must_use]
    pub fn loading(&self) -> bool {
        self.scan.is_some()
    }

    /// Which row the pointer is over, as an offset into [`Self::hits`], or `None` when it is over none.
    ///
    /// This is what keeps a landing scan from moving that row. When the pointer leaves a list that is
    /// at the top, a held scan is applied.
    pub fn hover(&mut self, offset: Option<usize>) {
        self.hovered = offset;
        if offset.is_none() && self.filter.scroll() == 0 {
            self.show_pending();
        }
    }

    /// The row under the pointer, as last reported.
    #[must_use]
    pub fn hovered(&self) -> Option<usize> {
        self.hovered
    }

    /// The configuration it was opened with.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The root a file finder lists under, or `None` for a folder finder.
    #[must_use]
    pub fn files_root(&self) -> Option<&Path> {
        self.files_root.as_deref()
    }

    /// What has been typed.
    #[must_use]
    pub fn query(&self) -> &str {
        self.filter.query()
    }

    /// Type a character.
    pub fn push(&mut self, character: char) {
        self.filter.push(character);
        self.refilter();
    }

    /// Delete the last character typed.
    pub fn pop(&mut self) {
        self.filter.pop();
        self.refilter();
    }

    /// Empty the query.
    pub fn clear(&mut self) {
        self.filter.clear();
        self.refilter();
    }

    /// The order before anything is typed.
    #[must_use]
    pub fn sort(&self) -> SortMode {
        self.config.sort
    }

    /// Change the order. The cursor stays on its item.
    pub fn set_sort(&mut self, sort: SortMode) {
        self.config.sort = sort;
        self.rebuild(self.selected().map(Item::key));
    }

    /// Change how rows read. What the query matches follows it, so the list is filtered again.
    pub fn set_display(&mut self, display: Display) {
        self.config.display = display;
        self.rebuild(self.selected().map(Item::key));
    }

    /// The next order: relevance, name, recent, most used.
    pub fn cycle_sort(&mut self) {
        self.set_sort(self.config.sort.cycle());
    }

    /// What matched, best first, in list order before anything is typed.
    #[must_use]
    pub fn hits(&self) -> &[Hit] {
        self.filter.hits()
    }

    /// How many items there are, matched or not.
    #[must_use]
    pub fn total(&self) -> usize {
        self.items.len()
    }

    /// The item a hit names.
    #[must_use]
    pub fn item(&self, hit: &Hit) -> &Item {
        &self.items[self.order[hit.index]]
    }

    /// The text a hit was matched against — the string a row draws its highlights on.
    #[must_use]
    pub fn text(&self, hit: &Hit) -> &str {
        &self.texts[hit.index]
    }

    /// The item at `offset` in [`Self::hits`].
    #[must_use]
    pub fn item_at(&self, offset: usize) -> Option<&Item> {
        self.filter.hits().get(offset).map(|hit| self.item(hit))
    }

    /// How many times an item was picked.
    #[must_use]
    pub fn uses(&self, item: &Item) -> u32 {
        self.history.count(&item.key())
    }

    /// The item under the cursor.
    #[must_use]
    pub fn selected(&self) -> Option<&Item> {
        self.filter.selected().map(|hit| self.item(hit))
    }

    /// Whether the hit at `offset` is under the cursor.
    #[must_use]
    pub fn is_selected(&self, offset: usize) -> bool {
        self.filter.is_selected(offset)
    }

    /// Move the cursor, clamped at both ends.
    pub fn move_by(&mut self, delta: isize) {
        self.filter.move_by(delta);
    }

    /// Put the cursor on the hit at `offset` — a click.
    pub fn select(&mut self, offset: usize) {
        self.filter.select(offset);
    }

    /// Scroll so the cursor is inside a viewport `rows` items tall. [`draw`] calls it.
    pub fn follow(&mut self, rows: usize) {
        self.filter.follow(rows);
    }

    /// The first item in the viewport, as an offset into [`Self::hits`].
    #[must_use]
    pub fn scroll(&self) -> usize {
        self.filter.scroll()
    }

    /// Pick the item under the cursor: count it in the history and return it.
    pub fn pick(&mut self) -> Option<Item> {
        let item = self.selected()?.clone();
        let key = match &self.files_root {
            Some(root) => root.join(&item.path).display().to_string(),
            None => item.key(),
        };
        self.history.record_use(&key);
        Some(item)
    }

    /// Feed a key: type, move, pick or close. `ctrl+s` cycles the order and `ctrl+r` applies a held
    /// scan; any other key with `ctrl` or `alt` is left to the host.
    pub fn on_key(&mut self, key: crossterm::event::KeyEvent) -> Outcome {
        use crossterm::event::{KeyCode, KeyModifiers};
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Outcome::Cancelled,
            KeyCode::Enter => {
                if let Some(item) = self.pick() {
                    return Outcome::Picked(item);
                }
            }
            KeyCode::Down => self.move_by(1),
            KeyCode::Up => self.move_by(-1),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::Backspace => self.pop(),
            KeyCode::Char('s') if ctrl => self.cycle_sort(),
            KeyCode::Char('r') if ctrl => self.show_pending(),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.push(character);
            }
            _ => {}
        }
        Outcome::Open
    }

    fn replace(&mut self, items: Vec<Item>) {
        let held = self.selected().map(Item::key);
        self.items = items;
        self.rebuild(held);
    }

    fn refilter(&mut self) {
        self.filter.refilter(self.texts.iter().map(String::as_str));
    }

    /// Order the items, compute what each matches against, filter, and put the cursor back on `held`.
    fn rebuild(&mut self, held: Option<String>) {
        let history = &self.history;
        let items = &self.items;
        let mut order: Vec<usize> = (0..items.len()).collect();
        match self.config.sort {
            SortMode::Relevance => {}
            SortMode::Name => order.sort_by_cached_key(|index| items[*index].name().to_lowercase()),
            SortMode::Recent => order.sort_by_cached_key(|index| {
                std::cmp::Reverse(history.last_used(&items[*index].key()))
            }),
            SortMode::MostUsed => order
                .sort_by_cached_key(|index| std::cmp::Reverse(history.count(&items[*index].key()))),
        }
        self.texts = order
            .iter()
            .map(|index| self.matched_text(&self.items[*index]))
            .collect();
        self.order = order;
        self.refilter();
        if let Some(key) = held
            && let Some(offset) = self
                .filter
                .hits()
                .iter()
                .position(|hit| self.items[self.order[hit.index]].key() == key)
        {
            self.filter.select(offset);
        }
    }

    /// What the query is matched against, which is also what a row draws: the name in [`Display::Name`],
    /// the path otherwise.
    fn matched_text(&self, item: &Item) -> String {
        match self.config.display {
            Display::Name => item.name(),
            Display::Path | Display::NameAndPath | Display::TwoRows => self.path_text(item),
        }
    }

    /// A path as a row shows it: relative under a file finder's root, with home as `~` when asked.
    #[must_use]
    pub fn path_text(&self, item: &Item) -> String {
        if self.files_root.is_none()
            && self.config.tilde
            && let Ok(rest) = item.path.strip_prefix(&self.config.home)
        {
            return if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.display())
            };
        }
        item.key()
    }
}

#[cfg(test)]
mod tests;
