//! A tab strip that reports where it drew each tab.
//!
//! Drawing tabs is trivial; making a click land on the one under the pointer is where it goes wrong.
//! The strip is a run of spans, so the columns a tab occupies exist only as a consequence of laying
//! the run out — and a caller that needs those columns computes them a second time, from the same
//! labels, with the same separator arithmetic, in a different function. The two agree until a label
//! gains a badge or a wide character, and then a click quietly selects the neighbour.
//!
//! [`strip`] lays out once and returns both: the spans to draw, and the half-open column range each
//! tab landed on. [`Strip::at`] answers the hit-test from those ranges, so the click and the paint
//! cannot disagree.
//!
//! ```no_run
//! # use ratatui::{Frame, layout::Rect, widgets::Paragraph};
//! # use tuilith::{tabs, theme::Palette};
//! # fn draw(frame: &mut Frame, area: Rect, palette: &Palette, active: usize, clicked: u16) {
//! let strip = tabs::strip(
//!     &[tabs::Tab::new("features").badge(17), tabs::Tab::new("plugins").badge(4)],
//!     active,
//!     palette,
//! );
//! frame.render_widget(Paragraph::new(strip.line()), area);
//! let _picked = strip.at(clicked.saturating_sub(area.x));
//! # }
//! ```
//!
//! **Ranges are display width, never character counts.** A label holding a wide glyph advances two
//! columns per glyph and `chars().count()` reports one, so a count-derived range drifts left by one
//! column per wide character — invisible until someone labels a tab with CJK or an emoji, and then
//! every tab after it is wrong.
//!
//! **What it deliberately does not do**: own a rect, scroll, or elide. A strip wider than its area is
//! the caller's problem, because the answer depends on what the caller would rather lose — the tabs
//! past the fold, or the labels' tails. [`Strip::width`] is what that decision reads.

use std::ops::Range;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Palette;

crate::provenance! {
    component: "tabs",
    about: "A tab strip that returns the column range each tab landed on, so a click cannot drift from the paint",
    origin: crate::Origin::Private,
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// One tab: what it says, and optionally how many things are behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tab<'a> {
    label: &'a str,
    badge: Option<usize>,
}

impl<'a> Tab<'a> {
    /// A tab with no badge.
    #[must_use]
    pub fn new(label: &'a str) -> Self {
        Self { label, badge: None }
    }

    /// A count drawn after the label.
    ///
    /// What the count *counts* is the caller's business, and the choice matters: a badge showing what
    /// a tab holds while a filter is active sends the reader to a tab that will look empty when they
    /// get there. Count what the tab would show.
    #[must_use]
    pub fn badge(mut self, count: usize) -> Self {
        self.badge = Some(count);
        self
    }
}

/// A laid-out strip: the spans that draw it, and where each tab landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strip {
    spans: Vec<Span<'static>>,
    ranges: Vec<Range<u16>>,
}

impl Strip {
    /// The spans, in order, for a caller composing its own line.
    #[must_use]
    pub fn spans(&self) -> &[Span<'static>] {
        &self.spans
    }

    /// The strip as a line, for a caller drawing nothing else on the row.
    #[must_use]
    pub fn line(&self) -> Line<'static> {
        Line::from(self.spans.clone())
    }

    /// Columns the whole strip occupies.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.ranges.last().map_or(0, |range| range.end)
    }

    /// The half-open column range a tab was drawn into, relative to the strip's own start.
    #[must_use]
    pub fn range(&self, index: usize) -> Option<Range<u16>> {
        self.ranges.get(index).cloned()
    }

    /// Which tab covers a column, or `None` for the gaps between them and anything past the end.
    ///
    /// A separator belongs to neither neighbour on purpose: a click that lands between two tabs has
    /// not chosen one, and picking the nearer would make the strip act on a press the reader did not
    /// aim.
    #[must_use]
    pub fn at(&self, column: u16) -> Option<usize> {
        self.ranges.iter().position(|range| range.contains(&column))
    }
}

/// Lay out a strip and style it from the palette.
///
/// `active` past the end simply styles nothing as active, which is what a caller with an empty list
/// or a stale index should get — a panic here would turn a cosmetic bug into a crash.
#[must_use]
pub fn strip(tabs: &[Tab<'_>], active: usize, palette: &Palette) -> Strip {
    let mut spans = Vec::with_capacity(tabs.len() * 3);
    let mut ranges = Vec::with_capacity(tabs.len());
    let mut cursor = 0u16;

    for (index, tab) in tabs.iter().enumerate() {
        if index > 0 {
            let separator = Span::raw(" ");
            cursor += separator.width() as u16;
            spans.push(separator);
        }

        let is_active = index == active;
        let label_style = if is_active {
            Style::default()
                .fg(palette.background)
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.dim)
        };

        let start = cursor;
        // The label carries a space each side so an active tab's band reads as a button rather than
        // as a highlighted word.
        let label = Span::styled(format!(" {} ", tab.label), label_style);
        cursor += label.width() as u16;
        spans.push(label);

        if let Some(count) = tab.badge {
            let badge = Span::styled(format!("{count} "), label_style);
            cursor += badge.width() as u16;
            spans.push(badge);
        }

        ranges.push(start..cursor);
    }

    Strip { spans, ranges }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::widgets::Paragraph;

    fn palette() -> Palette {
        crate::theme::DEFAULT_DARK
    }

    /// Where the strip says a tab is, is where the terminal drew it.
    ///
    /// The claim every other method rests on, and the one a second derivation gets wrong. Read off a
    /// painted buffer rather than recomputed from the labels: recomputing here would agree with the
    /// layout by construction and could never catch it being wrong.
    #[test]
    fn every_reported_range_is_where_the_tab_was_painted() {
        let tabs = [Tab::new("features").badge(17), Tab::new("plugins").badge(4)];
        for active in 0..tabs.len() {
            let strip = strip(&tabs, active, &palette());
            let area = Rect::new(0, 0, strip.width().max(1), 1);
            let mut terminal = Terminal::new(TestBackend::new(area.width, 1)).expect("backend");
            terminal
                .draw(|frame| frame.render_widget(Paragraph::new(strip.line()), area))
                .expect("backend");
            let buffer = terminal.backend().buffer();

            for (index, tab) in tabs.iter().enumerate() {
                let range = strip.range(index).expect("a range per tab");
                let painted: String = (range.start..range.end)
                    .map(|column| buffer[(column, 0)].symbol().to_string())
                    .collect();
                assert!(
                    painted.contains(tab.label),
                    "tab {index} reports {range:?}, which paints {painted:?}"
                );
                let background = buffer[(range.start, 0)].bg;
                if index == active {
                    assert_eq!(
                        background,
                        palette().accent,
                        "the active tab wears the accent"
                    );
                } else {
                    assert_ne!(background, palette().accent, "only the active one does");
                }
            }
        }
    }

    /// A click on a tab selects that tab, and a click between two selects neither.
    #[test]
    fn a_column_hit_tests_to_the_tab_drawn_on_it() {
        let tabs = [Tab::new("one"), Tab::new("two"), Tab::new("three")];
        let strip = strip(&tabs, 0, &palette());

        for index in 0..tabs.len() {
            let range = strip.range(index).expect("a range per tab");
            assert_eq!(strip.at(range.start), Some(index));
            assert_eq!(strip.at(range.end - 1), Some(index));
        }

        let between = strip.range(0).expect("the first").end;
        assert_eq!(strip.at(between), None, "a separator belongs to neither");
        assert_eq!(strip.at(strip.width()), None, "and nothing is past the end");
    }

    /// Ranges are display width, not character counts.
    ///
    /// The defect this component exists to remove: a count-derived range drifts one column left per
    /// wide character, so every tab after a CJK label is wrong. The assertion is against the painted
    /// buffer, so it fails on a layout that counted chars even though the arithmetic looks right.
    #[test]
    fn a_wide_label_does_not_shift_the_tabs_after_it() {
        let tabs = [Tab::new("設定"), Tab::new("plain")];
        let strip = strip(&tabs, 0, &palette());
        let area = Rect::new(0, 0, strip.width(), 1);
        let mut terminal = Terminal::new(TestBackend::new(area.width, 1)).expect("backend");
        terminal
            .draw(|frame| frame.render_widget(Paragraph::new(strip.line()), area))
            .expect("backend");
        let buffer = terminal.backend().buffer();

        let second = strip.range(1).expect("the second tab");
        let painted: String = (second.start..second.end)
            .map(|column| buffer[(column, 0)].symbol().to_string())
            .collect();
        assert!(
            painted.contains("plain"),
            "the tab after a wide label reports {second:?}, which paints {painted:?}"
        );
        assert_eq!(
            strip.range(0).expect("the first").end,
            6,
            "two double-width glyphs and a space each side"
        );
    }

    /// A badge widens the tab it belongs to, and a tab without one is narrower by exactly its width.
    #[test]
    fn a_badge_belongs_to_its_own_tab() {
        let bare = strip(&[Tab::new("all")], 0, &palette());
        let badged = strip(&[Tab::new("all").badge(69)], 0, &palette());
        assert_eq!(
            badged.width() - bare.width(),
            3,
            "`69` and its trailing space"
        );
        assert_eq!(
            badged.at(badged.width() - 1),
            Some(0),
            "the badge hit-tests to the tab it counts, not to nothing"
        );
    }

    /// An index nothing can satisfy styles nothing, rather than panicking.
    #[test]
    fn an_active_index_past_the_end_is_drawn_as_no_tab_active() {
        let tabs = [Tab::new("one"), Tab::new("two")];
        let strip = strip(&tabs, 9, &palette());
        assert_eq!(strip.range(0).expect("still laid out").start, 0);
        assert!(
            strip
                .spans()
                .iter()
                .all(|span| span.style.bg != Some(palette().accent))
        );
    }

    #[test]
    fn an_empty_strip_draws_nothing_and_hit_tests_to_nothing() {
        let strip = strip(&[], 0, &palette());
        assert_eq!(strip.width(), 0);
        assert_eq!(strip.at(0), None);
        assert!(strip.spans().is_empty());
    }
}
