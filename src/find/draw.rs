//! Painting a finder: the query line, the list, and the scrollbar.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{Display, Finder, Item};
use crate::pick::{Hit, query_line};
use crate::scroll;
use crate::theme::Palette;

/// What [`draw`] painted where, for a host to hit-test against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Geometry {
    /// The query line.
    pub query: Rect,
    /// The `N new` control, while a held scan is waiting. Pressing it is [`Finder::show_pending`].
    pub pending: Option<Rect>,
    /// Each visible item's rect — two rows tall in [`Display::TwoRows`] — and its offset into
    /// [`Finder::hits`]. Report the one under the pointer to [`Finder::hover`].
    pub rows: Vec<(Rect, usize)>,
}

impl Geometry {
    /// The offset of the item painted at `(column, row)`, if any.
    #[must_use]
    pub fn item_at(&self, column: u16, row: u16) -> Option<usize> {
        self.rows
            .iter()
            .find(|(rect, _)| {
                column >= rect.x
                    && column < rect.x + rect.width
                    && row >= rect.y
                    && row < rect.y + rect.height
            })
            .map(|(_, offset)| *offset)
    }
}

/// Paint `finder` into `area`: the query on the first row, with a scan's progress or a held scan's
/// count at its right end, and the list under it with its scrollbar.
///
/// Nothing around it — no border, no title, no shadow. That chrome is the host's, so the finder sits in
/// a modal, a dropdown or a pane alike.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    finder: &mut Finder,
    palette: &Palette,
    placeholder: &str,
) -> Geometry {
    let mut geometry = Geometry::default();
    if area.height == 0 || area.width == 0 {
        return geometry;
    }
    let base = Style::default()
        .bg(palette.background)
        .fg(palette.foreground);
    frame.buffer_mut().set_style(area, base);

    let header = Rect { height: 1, ..area };
    geometry.query = header;
    let (status, pressable) = status(finder);
    let status_width = Span::raw(&status).width() as u16;
    let query_width = header
        .width
        .saturating_sub(status_width + u16::from(status_width > 0));
    let query = query_line(&finder.filter, placeholder, palette, query_width);
    frame
        .buffer_mut()
        .set_line(header.x, header.y, &query, query_width);
    if status_width > 0 && status_width < header.width {
        let at = header.x + header.width - status_width;
        let style = if pressable {
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.dim)
        };
        frame
            .buffer_mut()
            .set_string(at, header.y, &status, style.bg(palette.background));
        if pressable {
            geometry.pending = Some(Rect::new(at, header.y, status_width, 1));
        }
    }

    let list = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    let region = scroll::Area::inside(list);
    let content = region.content();
    let per_item: u16 = if finder.config.display == Display::TwoRows {
        2
    } else {
        1
    };
    let viewport = usize::from(content.height / per_item);
    finder.follow(viewport);

    let mut y = content.y;
    let visible: Vec<(usize, Hit)> = finder
        .filter
        .visible(viewport)
        .map(|(offset, hit)| (offset, hit.clone()))
        .collect();
    for (offset, hit) in visible {
        let rect = Rect::new(content.x, y, content.width, per_item);
        let lines = row_lines(
            finder,
            &hit,
            finder.is_selected(offset),
            palette,
            content.width,
        );
        for (line_offset, line) in lines.iter().enumerate() {
            let line_y = y + line_offset as u16;
            if finder.is_selected(offset) {
                frame.buffer_mut().set_style(
                    Rect::new(content.x, line_y, content.width, 1),
                    Style::default().bg(palette.selection),
                );
            }
            frame
                .buffer_mut()
                .set_line(content.x, line_y, line, content.width);
        }
        geometry.rows.push((rect, offset));
        y += per_item;
    }
    region
        .bar(finder.hits().len(), finder.scroll())
        .viewport(viewport)
        .draw(frame);
    geometry
}

/// The right end of the query row: a held scan's count, which is pressable, or a running scan.
fn status(finder: &Finder) -> (String, bool) {
    if let Some(pending) = finder.pending() {
        let text = if pending.added > 0 {
            format!("{} new", pending.added)
        } else {
            "list changed".to_string()
        };
        return (text, true);
    }
    if finder.loading() {
        return (format!("scanning · {}", finder.total()), false);
    }
    (String::new(), false)
}

/// One item's row, or two in [`Display::TwoRows`].
fn row_lines(
    finder: &Finder,
    hit: &Hit,
    selected: bool,
    palette: &Palette,
    width: u16,
) -> Vec<Line<'static>> {
    let item = finder.item(hit);
    let text = finder.text(hit);
    let at = hit.matched.at();
    let mut base = Style::default().fg(palette.foreground);
    if selected {
        base = base.bg(palette.selection).add_modifier(Modifier::BOLD);
    }
    let dim = base.fg(palette.dim).remove_modifier(Modifier::BOLD);
    let matched = base.fg(palette.accent).add_modifier(Modifier::BOLD);

    let mut lead = Vec::new();
    if finder.config.show_kind {
        lead.push(Span::styled(format!("{:<5}", item.kind.label()), dim));
    }
    if finder.config.show_count {
        let uses = finder.uses(item);
        let count = if uses > 0 {
            uses.to_string()
        } else {
            String::new()
        };
        lead.push(Span::styled(format!("{count:>3} "), dim));
    }
    let lead_width: usize = lead.iter().map(Span::width).sum();
    let room = usize::from(width).saturating_sub(lead_width);

    let mut first = lead;
    match finder.config.display {
        Display::Name | Display::Path => {
            first.extend(highlighted(text, at, room, Cut::Tail, base, matched));
            vec![Line::from(first)]
        }
        Display::NameAndPath => {
            let (name, name_at) = name_of(item, text, at);
            let name_spans = highlighted(&name, &name_at, room, Cut::Tail, base, matched);
            let name_width: usize = name_spans.iter().map(Span::width).sum();
            first.extend(name_spans);
            // The path takes what is left after a two-column gap, cut from the left: the end of a path
            // is the part that tells two of them apart.
            let space = room.saturating_sub(name_width + 2);
            if space > 0 {
                let path = highlighted(text, at, space, Cut::Head, dim, matched);
                let path_width: usize = path.iter().map(Span::width).sum();
                first.push(Span::styled(
                    " ".repeat(room - name_width - path_width),
                    base,
                ));
                first.extend(path);
            }
            vec![Line::from(first)]
        }
        Display::TwoRows => {
            let (name, name_at) = name_of(item, text, at);
            first.extend(highlighted(&name, &name_at, room, Cut::Tail, base, matched));
            let indent = lead_width.max(2);
            let mut second = vec![Span::styled(" ".repeat(indent), base)];
            second.extend(highlighted(
                text,
                at,
                usize::from(width).saturating_sub(indent),
                Cut::Head,
                dim,
                matched,
            ));
            vec![Line::from(first), Line::from(second)]
        }
    }
}

/// The item's name, and the offsets of `at` that fall inside it when `text` ends with it. A path's
/// last component is its name, so a highlight on a path lands on the name drawn beside it.
fn name_of(item: &Item, text: &str, at: &[usize]) -> (String, Vec<usize>) {
    let name = item.name();
    if text.ends_with(&name) {
        let start = text.len() - name.len();
        let inside = at
            .iter()
            .filter(|offset| **offset >= start)
            .map(|offset| offset - start)
            .collect();
        (name, inside)
    } else {
        (name, Vec::new())
    }
}

/// Which end of a text gives way when it does not fit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cut {
    /// Keep the start, end with `…`.
    Tail,
    /// Keep the end, start with `…`.
    Head,
}

/// `text` in at most `width` columns, with the characters at byte offsets `at` in `matched`.
fn highlighted(
    text: &str,
    at: &[usize],
    width: usize,
    cut: Cut,
    base: Style,
    matched: Style,
) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let characters: Vec<(usize, char)> = text.char_indices().collect();
    let (kept, ellipsis) = if characters.len() <= width {
        (&characters[..], false)
    } else if cut == Cut::Tail {
        (&characters[..width - 1], true)
    } else {
        (&characters[characters.len() + 1 - width..], true)
    };
    let mut spans = Vec::new();
    if ellipsis && cut == Cut::Head {
        spans.push(Span::styled("…", base));
    }
    let mut run = String::new();
    let mut run_matched = false;
    for (offset, character) in kept {
        let is_matched = at.contains(offset);
        if is_matched != run_matched && !run.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut run),
                if run_matched { matched } else { base },
            ));
        }
        run_matched = is_matched;
        run.push(*character);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_matched { matched } else { base }));
    }
    if ellipsis && cut == Cut::Tail {
        spans.push(Span::styled("…", base));
    }
    spans
}
