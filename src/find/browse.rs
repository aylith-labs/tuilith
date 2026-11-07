//! A folder picker that moves through the tree one folder at a time: breadcrumbs, home, bookmarks,
//! up and back, a filter over the current folder, and git checkouts marked.
//!
//! State and painting are separate, and neither persists anything: the host seeds home and the
//! bookmarks, saves [`Browser::bookmarks`] when it closes, and decides what a pick does.

use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::source::{Kind, kind_of};
use crate::pick::{Filter, query_line};
use crate::scroll;
use crate::theme::Palette;

/// One folder in the current listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its name.
    pub name: String,
    /// Where it is.
    pub path: PathBuf,
    /// Whether it is a git checkout, and of which shape.
    pub kind: Kind,
}

/// The folders directly in `folder`, by name, ignoring case.
///
/// # Errors
///
/// Whatever reading the folder returned.
pub fn read_entries(folder: &Path) -> std::io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let path = entry.path();
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            kind: kind_of(&path),
            path,
        });
    }
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    Ok(entries)
}

/// What feeding a key or a click did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Still open.
    Open,
    /// Closed without a pick.
    Cancelled,
    /// This folder was picked.
    Picked(PathBuf),
}

/// The picker's state.
#[derive(Debug, Clone)]
pub struct Browser {
    /// The folder being listed.
    pub current: PathBuf,
    /// Where `ctrl+h` goes.
    pub home: PathBuf,
    /// The bookmarked folders, which `ctrl+b` adds the current folder to or removes it from.
    pub bookmarks: Vec<PathBuf>,
    /// List only git checkouts. `ctrl+g` flips it.
    pub checkouts_only: bool,
    entries: Vec<Entry>,
    names: Vec<String>,
    shown: Vec<usize>,
    filter: Filter,
    back: Vec<PathBuf>,
}

impl Browser {
    /// Open at `start`.
    #[must_use]
    pub fn new(start: PathBuf, home: PathBuf, bookmarks: Vec<PathBuf>) -> Self {
        let mut browser = Self {
            current: start,
            home,
            bookmarks,
            checkouts_only: false,
            entries: Vec::new(),
            names: Vec::new(),
            shown: Vec::new(),
            filter: Filter::new(),
            back: Vec::new(),
        };
        browser.filter.focus(true);
        browser.reload();
        browser
    }

    /// Read the current folder again.
    pub fn reload(&mut self) {
        self.entries = read_entries(&self.current).unwrap_or_default();
        self.relist();
    }

    fn relist(&mut self) {
        self.shown = (0..self.entries.len())
            .filter(|index| !self.checkouts_only || self.entries[*index].kind.is_git())
            .collect();
        self.names = self
            .shown
            .iter()
            .map(|index| self.entries[*index].name.clone())
            .collect();
        self.filter.refilter(self.names.iter().map(String::as_str));
    }

    /// List another folder. The one left is where [`Self::back`] returns to.
    pub fn go(&mut self, folder: PathBuf) {
        if folder == self.current {
            return;
        }
        self.back.push(std::mem::replace(&mut self.current, folder));
        self.filter.clear();
        self.filter.select(0);
        self.reload();
    }

    /// Up to the parent folder.
    pub fn up(&mut self) {
        if let Some(parent) = self.current.parent() {
            self.go(parent.to_path_buf());
        }
    }

    /// To the home folder.
    pub fn go_home(&mut self) {
        self.go(self.home.clone());
    }

    /// Back to the folder listed before this one.
    pub fn back(&mut self) {
        if let Some(previous) = self.back.pop() {
            self.current = previous;
            self.filter.clear();
            self.filter.select(0);
            self.reload();
        }
    }

    /// Bookmark the current folder, or drop its bookmark. True when it is now bookmarked.
    pub fn toggle_bookmark(&mut self) -> bool {
        if let Some(at) = self.bookmarks.iter().position(|path| *path == self.current) {
            self.bookmarks.remove(at);
            false
        } else {
            self.bookmarks.push(self.current.clone());
            true
        }
    }

    /// List only git checkouts, or everything.
    pub fn toggle_checkouts_only(&mut self) {
        self.checkouts_only = !self.checkouts_only;
        self.relist();
    }

    /// What has been typed.
    #[must_use]
    pub fn query(&self) -> &str {
        self.filter.query()
    }

    /// The entries the filter keeps, in order, each with its offset.
    pub fn visible(&self) -> impl Iterator<Item = (usize, &Entry)> {
        self.filter
            .hits()
            .iter()
            .enumerate()
            .map(|(offset, hit)| (offset, &self.entries[self.shown[hit.index]]))
    }

    /// The entry under the cursor.
    #[must_use]
    pub fn selected(&self) -> Option<&Entry> {
        self.filter
            .selected()
            .map(|hit| &self.entries[self.shown[hit.index]])
    }

    /// Move the cursor, clamped at both ends.
    pub fn move_by(&mut self, delta: isize) {
        self.filter.move_by(delta);
    }

    /// Put the cursor on the entry at `offset` — a click.
    pub fn select(&mut self, offset: usize) {
        self.filter.select(offset);
    }

    /// Open the entry under the cursor: a git checkout is picked, any other folder is entered.
    pub fn activate(&mut self) -> Outcome {
        let Some(entry) = self.selected().cloned() else {
            return Outcome::Open;
        };
        if entry.kind.is_git() {
            return Outcome::Picked(entry.path);
        }
        self.go(entry.path);
        Outcome::Open
    }

    /// Feed a key. `enter` opens, `ctrl+s` picks the current folder itself, `left` or `backspace` on
    /// an empty query goes up, `ctrl+h` home, `ctrl+b` bookmarks, `ctrl+g` lists checkouts only.
    pub fn on_key(&mut self, key: crossterm::event::KeyEvent) -> Outcome {
        use crossterm::event::{KeyCode, KeyModifiers};
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Outcome::Cancelled,
            KeyCode::Enter => return self.activate(),
            KeyCode::Down => self.move_by(1),
            KeyCode::Up => self.move_by(-1),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::Left => self.up(),
            KeyCode::Backspace => {
                if self.filter.query().is_empty() {
                    self.up();
                } else {
                    self.filter.pop();
                    self.filter.refilter(self.names.iter().map(String::as_str));
                }
            }
            KeyCode::Char('s') if ctrl => return Outcome::Picked(self.current.clone()),
            KeyCode::Char('b') if ctrl => {
                self.toggle_bookmark();
            }
            KeyCode::Char('h') if ctrl => self.go_home(),
            KeyCode::Char('g') if ctrl => self.toggle_checkouts_only(),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.filter.push(character);
                self.filter.refilter(self.names.iter().map(String::as_str));
            }
            _ => {}
        }
        Outcome::Open
    }
}

/// What [`draw`] painted where.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Geometry {
    /// Each breadcrumb's rect and the folder it goes to.
    pub crumbs: Vec<(Rect, PathBuf)>,
    /// Each visible entry's row and its offset.
    pub rows: Vec<(Rect, usize)>,
}

/// Paint the picker into `area`: breadcrumbs, the filter, the folders, and the current path on the
/// last row. No border or title — that chrome is the host's.
pub fn draw(frame: &mut Frame, area: Rect, browser: &mut Browser, palette: &Palette) -> Geometry {
    let mut geometry = Geometry::default();
    if area.height < 4 || area.width == 0 {
        return geometry;
    }
    let base = Style::default()
        .bg(palette.background)
        .fg(palette.foreground);
    let dim = base.fg(palette.dim);
    frame.buffer_mut().set_style(area, base);

    geometry.crumbs = crumbs(
        frame,
        Rect { height: 1, ..area },
        &browser.current,
        base,
        palette,
    );

    let query = query_line(&browser.filter, "filter this folder", palette, area.width);
    frame
        .buffer_mut()
        .set_line(area.x, area.y + 1, &query, area.width);

    let list = Rect::new(area.x, area.y + 2, area.width, area.height - 3);
    let region = scroll::Area::inside(list);
    let content = region.content();
    let viewport = usize::from(content.height);
    browser.filter.follow(viewport);
    let scroll = browser.filter.scroll();
    let rows: Vec<(usize, Entry)> = browser
        .visible()
        .skip(scroll)
        .take(viewport)
        .map(|(offset, entry)| (offset, entry.clone()))
        .collect();
    for (row, (offset, entry)) in rows.iter().enumerate() {
        let y = content.y + row as u16;
        let selected = browser.filter.is_selected(*offset);
        let mut style = if entry.kind.is_git() {
            base.fg(palette.ok)
        } else {
            base
        };
        if selected {
            style = style.bg(palette.selection).add_modifier(Modifier::BOLD);
            frame
                .buffer_mut()
                .set_style(Rect::new(content.x, y, content.width, 1), style);
        }
        let mut spans = vec![Span::styled(entry.name.clone(), style)];
        if entry.kind.is_git() {
            spans.push(Span::styled("  ", style));
            spans.push(Span::styled(
                if entry.kind == Kind::Repo {
                    "git repo"
                } else {
                    "worktree"
                },
                style.fg(palette.dim).remove_modifier(Modifier::BOLD),
            ));
        }
        frame
            .buffer_mut()
            .set_line(content.x, y, &Line::from(spans), content.width);
        geometry
            .rows
            .push((Rect::new(content.x, y, content.width, 1), *offset));
    }
    region
        .bar(browser.filter.hits().len(), scroll)
        .viewport(viewport)
        .draw(frame);

    let marked = if browser.bookmarks.contains(&browser.current) {
        "★ "
    } else {
        ""
    };
    let footer = Line::from(vec![
        Span::styled(marked, base.fg(palette.accent)),
        Span::styled(browser.current.display().to_string(), dim),
    ]);
    frame
        .buffer_mut()
        .set_line(area.x, area.y + area.height - 1, &footer, area.width);
    geometry
}

/// The breadcrumbs: `/` then each component of `folder`, every one a place to go.
fn crumbs(
    frame: &mut Frame,
    row: Rect,
    folder: &Path,
    base: Style,
    palette: &Palette,
) -> Vec<(Rect, PathBuf)> {
    let mut crumbs = Vec::new();
    let mut x = row.x;
    let right = row.x + row.width;
    let mut path = PathBuf::new();
    for (index, component) in folder.components().enumerate() {
        path.push(component);
        let label = match component {
            std::path::Component::RootDir => "/".to_string(),
            other => other.as_os_str().to_string_lossy().into_owned(),
        };
        if index > 1 {
            x = frame
                .buffer_mut()
                .set_stringn(
                    x,
                    row.y,
                    " / ",
                    usize::from(right - x),
                    base.fg(palette.dim),
                )
                .0;
        }
        if x >= right {
            break;
        }
        let width = (Span::raw(&label).width() as u16).min(right - x);
        frame.buffer_mut().set_stringn(
            x,
            row.y,
            &label,
            usize::from(width),
            base.fg(palette.accent),
        );
        crumbs.push((Rect::new(x, row.y, width, 1), path.clone()));
        x += width;
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL)
    }

    fn names(browser: &Browser) -> Vec<String> {
        browser
            .visible()
            .map(|(_, entry)| entry.name.clone())
            .collect()
    }

    #[test]
    fn it_enters_folders_picks_checkouts_and_goes_back() {
        let dir = super::super::source::tests::scratch("browse");
        for folder in ["code/alpha/.git", "code/notes/deep", "Music"] {
            std::fs::create_dir_all(dir.join(folder)).expect("built");
        }
        let mut browser = Browser::new(dir.clone(), dir.join("code"), Vec::new());
        assert_eq!(names(&browser), ["code", "Music"], "by name, ignoring case");

        assert_eq!(browser.activate(), Outcome::Open, "a folder is entered");
        assert_eq!(browser.current, dir.join("code"));
        assert_eq!(names(&browser), ["alpha", "notes"]);
        assert_eq!(
            browser.activate(),
            Outcome::Picked(dir.join("code/alpha")),
            "a checkout is picked"
        );

        browser.on_key(ctrl('g'));
        assert_eq!(names(&browser), ["alpha"], "checkouts only");
        browser.on_key(ctrl('g'));

        browser.on_key(key(KeyCode::Char('n')));
        assert_eq!(names(&browser), ["notes"]);
        browser.on_key(key(KeyCode::Backspace));
        assert_eq!(
            browser.current,
            dir.join("code"),
            "backspace edits a query first"
        );
        browser.on_key(key(KeyCode::Backspace));
        assert_eq!(browser.current, dir, "and goes up once it is empty");
        browser.back();
        assert_eq!(browser.current, dir.join("code"));

        assert!(browser.toggle_bookmark());
        assert_eq!(browser.bookmarks, [dir.join("code")]);
        assert!(!browser.toggle_bookmark());
        assert!(browser.bookmarks.is_empty());
        assert_eq!(browser.on_key(ctrl('s')), Outcome::Picked(dir.join("code")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
