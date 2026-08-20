//! A scrollbar that cannot land on the text it belongs to.
//!
//! A vertical scrollbar occupies a column. Nothing in ratatui says which one, so the choice is made at
//! every call site — and the shortest way to write it is to hand the bar the same rect the text was
//! drawn into, where it renders on the right edge and paints over whatever filled the last column. The
//! failure is invisible for as long as the text rarely reaches the edge, which is most of the time, and
//! it does not look like an overlap when it arrives: the bar *overwrites* the character rather than
//! pushing it, so the row still looks like a row and the sentence just ends a letter early. A
//! screen-buffer assertion cannot see it either, for the same reason.
//!
//! So the column is not something a caller decides. [`Area`] splits a rect into the content and the
//! bar's own track, and the content rect is the only way to get one — there is no order of calls that
//! draws text where the bar goes. It also refuses to spend a column twice: asked to ride a frame's
//! border, it measures whether the frame's padding already separates the text from it.
//!
//! ```no_run
//! # use ratatui::{Frame, layout::Rect, widgets::{Block, Paragraph}};
//! # use tuilith::scroll;
//! # fn draw(frame: &mut Frame, outer: Rect, lines: Vec<String>) {
//! let block = Block::bordered();
//! let area = scroll::Area::on_border(outer, block.inner(outer));
//! frame.render_widget(block, outer);
//!
//! // Wrap to the content width, not to the rect you started with.
//! let wrapped = lines.join("\n");
//! let bar = area.bar(lines.len(), 0).viewport(area.content().height as usize);
//!
//! // `bar.overflows()` is the one source of truth for a "j/k scroll" hint: no overflow, no hint.
//! frame.render_widget(Paragraph::new(wrapped), area.content());
//! bar.draw(frame);
//! # }
//! ```

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

crate::provenance! {
    component: "scroll",
    about: "A scrollbar whose track is carved out of the area, so it cannot be drawn over the text",
    origin: crate::Origin::Repo("polygit"),
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// Which edge the bar runs along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// A column down the right-hand side.
    Vertical,
    /// A row along the bottom.
    Horizontal,
}

/// An area split into the content and the scrollbar's track.
///
/// The content rect is the point: it is smaller than what you started with, by exactly the columns the
/// bar and its gap take, and it is available before anything is drawn — so the wrap width, the column
/// budget and the row count are all computed against the space the text actually gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    content: Rect,
    track: Rect,
    axis: Axis,
}

impl Area {
    /// Carve the track out of `area` itself: the last column for the bar, and the one before it left
    /// blank so the text is not pressed against it.
    ///
    /// For a surface with no frame of its own. Where there *is* a border to ride, [`Area::on_border`]
    /// costs a column less.
    ///
    /// Narrow rects give up the gap before the bar and the bar before the content, because a rect too
    /// thin to hold both is better off showing text than furniture: at two columns there is a bar and
    /// no gap, at one there is only content, and at zero there is nothing to draw either way.
    #[must_use]
    pub fn inside(area: Rect) -> Self {
        let (content_width, track_width) = match area.width {
            0 | 1 => (area.width, 0),
            2 => (1, 1),
            width => (width - 2, 1),
        };
        Self {
            content: Rect {
                width: content_width,
                ..area
            },
            track: Rect {
                x: area.x + area.width.saturating_sub(track_width),
                width: track_width,
                ..area
            },
            axis: Axis::Vertical,
        }
    }

    /// Run the bar down `outer`'s right border, so the content keeps its own width.
    ///
    /// `inner` is the frame's content rect — what [`Block::inner`] returned. The border column is
    /// already furniture, so the bar costs nothing there; what it still needs is a blank column between
    /// itself and the text, and whether one exists is something to measure rather than assume. A frame
    /// with right padding has one already and `inner` is returned untouched; a frame without gives up
    /// its last column, which is the same column the bar would otherwise have painted over.
    ///
    /// [`Block::inner`]: ratatui::widgets::Block::inner
    #[must_use]
    pub fn on_border(outer: Rect, inner: Rect) -> Self {
        let border = outer.x + outer.width.saturating_sub(1);
        // The gap the frame's own padding provides, in columns between the text and the border.
        let padded = border.saturating_sub(inner.x + inner.width);
        let content_width = if padded > 0 {
            inner.width
        } else {
            inner.width.saturating_sub(1)
        };
        Self {
            content: Rect {
                width: content_width,
                ..inner
            },
            track: Rect {
                x: border,
                y: inner.y,
                width: 1,
                height: inner.height,
            },
            axis: Axis::Vertical,
        }
    }

    /// A horizontal bar along the bottom of `area`, with the row above it left to the content.
    ///
    /// The mirror of [`Area::inside`] on the other axis, and it exists for the same reason: a bottom bar
    /// handed the content's own rect paints over its last row.
    #[must_use]
    pub fn under(area: Rect) -> Self {
        let (content_height, track_height) = match area.height {
            0 | 1 => (area.height, 0),
            height => (height - 1, 1),
        };
        Self {
            content: Rect {
                height: content_height,
                ..area
            },
            track: Rect {
                y: area.y + area.height.saturating_sub(track_height),
                height: track_height,
                ..area
            },
            axis: Axis::Horizontal,
        }
    }

    /// Where the content goes. Wrap, truncate and count rows against this, never against the rect it
    /// was carved from.
    #[must_use]
    pub const fn content(&self) -> Rect {
        self.content
    }

    /// Where the bar goes.
    ///
    /// Worth having even when nothing overflows: it is the rect a drag or a wheel event hit-tests
    /// against, and a host that registers it only while the bar is visible has a scrollbar that stops
    /// being grabbable exactly when the content shrinks under the pointer.
    #[must_use]
    pub const fn track(&self) -> Rect {
        self.track
    }

    /// Which edge this area's bar runs along.
    #[must_use]
    pub const fn axis(&self) -> Axis {
        self.axis
    }

    /// The bar for `total` units of content sitting at `offset`.
    ///
    /// The viewport defaults to the content's own extent along the axis, which is what a list of rows
    /// wants. Override it with [`Bar::viewport`] where the unit is not a row.
    #[must_use]
    pub const fn bar(&self, total: usize, offset: usize) -> Bar {
        let viewport = match self.axis {
            Axis::Vertical => self.content.height as usize,
            Axis::Horizontal => self.content.width as usize,
        };
        Bar {
            track: self.track,
            axis: self.axis,
            total,
            viewport,
            offset,
            thumb: None,
        }
    }
}

/// A scrollbar bound to a track, aware of whether there is anything to scroll.
#[derive(Debug, Clone, Copy)]
pub struct Bar {
    track: Rect,
    axis: Axis,
    total: usize,
    viewport: usize,
    offset: usize,
    thumb: Option<Style>,
}

impl Bar {
    /// How much of the content is on screen at once, when it is not one unit per row.
    #[must_use]
    pub const fn viewport(mut self, viewport: usize) -> Self {
        self.viewport = viewport;
        self
    }

    /// Style the thumb — the handle — for instance to brighten it while it is being dragged.
    #[must_use]
    pub const fn thumb(mut self, style: Style) -> Self {
        self.thumb = Some(style);
        self
    }

    /// Whether there is anything to scroll.
    ///
    /// The single source of truth for both halves of the answer: the bar draws only when this is true,
    /// and so should any hint that advertises the keys for it. A hint that is always on teaches the
    /// reader to ignore it, and one derived from a second count drifts from the bar beside it.
    #[must_use]
    pub const fn overflows(&self) -> bool {
        self.total > self.viewport
    }

    /// The furthest offset that still fills the viewport.
    #[must_use]
    pub const fn max_offset(&self) -> usize {
        self.total.saturating_sub(self.viewport)
    }

    /// Draw the bar, if there is anything to scroll and anywhere to draw it.
    ///
    /// A no-op otherwise, so a caller never has to ask twice — the branch it would write is the branch
    /// that gets forgotten, leaving a bar on a list that fits.
    pub fn draw(self, frame: &mut Frame) {
        if !self.overflows() || self.track.width == 0 || self.track.height == 0 {
            return;
        }
        // ratatui maps the position over `content_length - 1`: its model is the index of the top line,
        // at its maximum when the LAST line is at the top. An offset that maxes out with the last line
        // at the BOTTOM — which is what a viewport-filling scroll does — therefore has to declare a
        // shorter content length, or the thumb stops short of the end.
        let content = self.max_offset() + 1;
        let mut state = ScrollbarState::new(content)
            .position(self.offset)
            .viewport_content_length(self.viewport);
        let orientation = match self.axis {
            Axis::Vertical => ScrollbarOrientation::VerticalRight,
            Axis::Horizontal => ScrollbarOrientation::HorizontalBottom,
        };
        let mut scrollbar = Scrollbar::new(orientation)
            .begin_symbol(None)
            .end_symbol(None);
        if let Some(thumb) = self.thumb {
            scrollbar = scrollbar.thumb_style(thumb);
        }
        frame.render_stateful_widget(scrollbar, self.track, &mut state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::widgets::{Block, Padding};

    #[test]
    fn the_content_never_includes_the_column_the_bar_draws_in() {
        for width in 0..40u16 {
            let area = Rect::new(3, 1, width, 8);
            for split in [Area::inside(area), Area::on_border(area, area)] {
                let content = split.content();
                let track = split.track();
                if track.width == 0 {
                    continue;
                }
                assert!(
                    content.right() <= track.x,
                    "width {width}: content {content:?} reaches into the track {track:?}"
                );
            }
        }
    }

    #[test]
    fn a_rect_too_thin_for_furniture_spends_its_columns_on_text() {
        assert_eq!(Area::inside(Rect::new(0, 0, 1, 4)).content().width, 1);
        assert_eq!(Area::inside(Rect::new(0, 0, 1, 4)).track().width, 0);
        // Two columns: a bar and no gap. Text pressed against it beats no bar at all, because the bar
        // is the only thing that says the rest of the content exists.
        assert_eq!(Area::inside(Rect::new(0, 0, 2, 4)).content().width, 1);
        assert_eq!(Area::inside(Rect::new(0, 0, 2, 4)).track().width, 1);
        assert_eq!(Area::inside(Rect::new(0, 0, 3, 4)).content().width, 1);
    }

    #[test]
    fn a_frame_that_already_pads_its_right_edge_keeps_its_full_content_width() {
        let outer = Rect::new(0, 0, 20, 6);
        let padded = Block::bordered().padding(Padding::uniform(1));
        let bare = Block::bordered();

        let with_padding = Area::on_border(outer, padded.inner(outer));
        assert_eq!(
            with_padding.content(),
            padded.inner(outer),
            "the padding is the gap; taking another column would leave two"
        );
        // …and the bar still lands on the border, one clear column away from the text.
        assert_eq!(with_padding.track().x, outer.right() - 1);
        assert!(with_padding.content().right() < with_padding.track().x);

        let without = Area::on_border(outer, bare.inner(outer));
        assert_eq!(
            without.content().width,
            bare.inner(outer).width - 1,
            "no padding means the content owes the gap"
        );
        assert_eq!(without.track().x, outer.right() - 1);
    }

    #[test]
    fn overflow_is_the_one_answer_the_bar_and_a_hint_both_read() {
        let area = Area::inside(Rect::new(0, 0, 10, 5));
        assert!(
            !area.bar(5, 0).overflows(),
            "exactly a viewport does not scroll"
        );
        assert!(!area.bar(4, 0).overflows());
        assert!(area.bar(6, 0).overflows());
        // A unit that is not a row: the viewport has to be told, or a wide-line count reads as overflow.
        assert!(!area.bar(9, 0).viewport(9).overflows());
        assert_eq!(area.bar(9, 0).viewport(4).max_offset(), 5);
    }

    #[test]
    fn a_horizontal_bar_takes_the_bottom_row_and_not_the_content_row() {
        let area = Area::under(Rect::new(0, 0, 30, 4));
        assert_eq!(area.axis(), Axis::Horizontal);
        assert_eq!(area.content().height, 3);
        assert_eq!(area.track().height, 1);
        assert_eq!(area.track().y, area.content().bottom());
        // One row tall: the content keeps it, because a rect with a bar and no content shows nothing.
        let thin = Area::under(Rect::new(0, 0, 30, 1));
        assert_eq!(thin.content().height, 1);
        assert_eq!(thin.track().height, 0);
    }

    #[test]
    fn the_thumb_reaches_the_end_of_a_track() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut terminal = Terminal::new(TestBackend::new(12, 6)).unwrap();
        let area = Area::inside(Rect::new(0, 0, 12, 6));
        let bar = area.bar(40, area.bar(40, 0).max_offset());
        terminal.draw(|frame| bar.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let column = area.track().x;
        let bottom = buffer[(column, 5)].symbol().to_string();
        assert_eq!(
            bottom, "█",
            "scrolled to the end, the thumb has to reach the last row of the track"
        );
    }
}
