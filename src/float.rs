//! A floating window whose remembered position survives the terminal changing size.
//!
//! The obvious way to remember where a user dragged a panel is to store the rect, and clamp it back
//! into the viewport on every frame. That works until the terminal gets smaller: the clamp writes
//! the constrained position back, and growing the terminal again leaves the window wherever the
//! small one forced it. The position is not restored because it no longer exists — it was
//! overwritten by a temporary condition, and nothing recorded that.
//!
//! The second problem is arithmetic. Clamp-and-store means the code that resizes a window subtracts
//! the window's own position from the viewport's far edge, and on `u16` that underflows the moment
//! the position is outside the viewport — which the clamp is supposed to prevent, so the subtraction
//! is safe only while the clamp is guaranteed to have run first. That guarantee is an ordering
//! convention between two functions in different files, and conventions are what this crate exists
//! to replace.
//!
//! So a position here is a [`Placement`]: a corner and how far in from it, which is meaningful at
//! every terminal size and cannot represent an off-screen window. [`Placement::resolve`] is a pure
//! function from a placement and a viewport to a rect. It clamps for the frame it draws and never
//! writes anything back, so a shrink is forgotten as soon as it ends. There is no order of calls
//! that loses the position, and none that can underflow, because the only way to obtain a rect is
//! to resolve one against the viewport it will be drawn in.

use ratatui::layout::Rect;

crate::provenance! {
    component: "float",
    about: "A floating window placed by corner and offset, so a resize cannot lose or corrupt where it was",
    origin: crate::Origin::Repo("polygit"),
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// Which corner of the viewport a floating window is measured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Anchor {
    /// Offsets run right and down from the top-left.
    TopLeft,
    /// Offsets run left and down from the top-right. The usual home for an overlay that must not
    /// cover the left-hand content.
    #[default]
    TopRight,
    /// Offsets run right and up from the bottom-left.
    BottomLeft,
    /// Offsets run left and up from the bottom-right.
    BottomRight,
}

impl Anchor {
    /// Every anchor, in reading order — for a picker that offers all four.
    pub const ALL: [Anchor; 4] = [
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomLeft,
        Anchor::BottomRight,
    ];

    /// A short label for a menu row.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Anchor::TopLeft => "top left",
            Anchor::TopRight => "top right",
            Anchor::BottomLeft => "bottom left",
            Anchor::BottomRight => "bottom right",
        }
    }

    /// Whether offsets are measured from the right-hand edge rather than the left.
    #[must_use]
    pub const fn from_right(self) -> bool {
        matches!(self, Anchor::TopRight | Anchor::BottomRight)
    }

    /// Whether offsets are measured from the bottom edge rather than the top.
    #[must_use]
    pub const fn from_bottom(self) -> bool {
        matches!(self, Anchor::BottomLeft | Anchor::BottomRight)
    }

    /// The anchor whose quadrant contains `point`, within `bounds`.
    ///
    /// Taken from the window's CENTRE rather than its top-left, which is the difference between a
    /// window dragged to the middle-left of a wide viewport anchoring left — where the user put it —
    /// and anchoring to whichever corner its origin happened to be nearest.
    #[must_use]
    pub fn containing(bounds: Rect, point: (u16, u16)) -> Self {
        let right = point.0.saturating_sub(bounds.x) >= bounds.width / 2;
        let bottom = point.1.saturating_sub(bounds.y) >= bounds.height / 2;
        match (right, bottom) {
            (false, false) => Anchor::TopLeft,
            (true, false) => Anchor::TopRight,
            (false, true) => Anchor::BottomLeft,
            (true, true) => Anchor::BottomRight,
        }
    }
}

/// Where a floating window sits: a corner, and how far in from it.
///
/// Offsets are unsigned and always point INWARD, so every value that can be constructed is a
/// position that makes sense at every viewport size. A signed offset would add representable states
/// that only mean "outside the viewport", which [`resolve`](Placement::resolve) would have to clamp
/// away anyway — and a value whose only correct handling is to be discarded is one worth not having.
///
/// This is the shape to persist. It is meaningful without knowing the terminal size it was written
/// at, which a stored rect is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Placement {
    anchor: Anchor,
    dx: u16,
    dy: u16,
}

impl Placement {
    /// A placement `dx` columns and `dy` rows in from `anchor`.
    #[must_use]
    pub const fn new(anchor: Anchor, dx: u16, dy: u16) -> Self {
        Self { anchor, dx, dy }
    }

    /// The corner this placement is measured from.
    #[must_use]
    pub const fn anchor(self) -> Anchor {
        self.anchor
    }

    /// How far in from the corner, as `(columns, rows)`.
    #[must_use]
    pub const fn offset(self) -> (u16, u16) {
        (self.dx, self.dy)
    }

    /// The same offsets measured from a different corner.
    ///
    /// Note this MOVES the window — it reinterprets the offsets rather than preserving the position.
    /// To keep a window where it is while changing its anchor, resolve it first and hand the rect to
    /// [`Placement::from_rect`].
    #[must_use]
    pub const fn with_anchor(self, anchor: Anchor) -> Self {
        Self { anchor, ..self }
    }

    /// Where a window of `size` sits in `bounds`.
    ///
    /// Pure: it clamps the result into `bounds` for the frame being drawn and changes nothing. That
    /// is what lets a placement survive a terminal that shrinks and grows again — the constrained
    /// position exists only in the rect handed back, never in the placement that produced it.
    ///
    /// A window larger than the viewport is given the whole viewport rather than being refused; a
    /// caller that would rather draw nothing at that point should test the size itself, which it can
    /// do before calling.
    #[must_use]
    pub fn resolve(self, bounds: Rect, size: (u16, u16)) -> Rect {
        let width = size.0.min(bounds.width);
        let height = size.1.min(bounds.height);
        // The furthest the window's origin can sit and still fit. Saturating because `bounds` may be
        // smaller than the window even after the clamp above, when either is zero.
        let max_x = bounds.x + bounds.width.saturating_sub(width);
        let max_y = bounds.y + bounds.height.saturating_sub(height);

        let x = if self.anchor.from_right() {
            max_x.saturating_sub(self.dx)
        } else {
            bounds.x.saturating_add(self.dx)
        };
        let y = if self.anchor.from_bottom() {
            max_y.saturating_sub(self.dy)
        } else {
            bounds.y.saturating_add(self.dy)
        };

        Rect {
            x: x.clamp(bounds.x, max_x),
            y: y.clamp(bounds.y, max_y),
            width,
            height,
        }
    }

    /// The placement that puts a window of `size` at `rect`'s position within `bounds`.
    ///
    /// The anchor comes from the quadrant holding the window's centre, so dropping a drag re-bases
    /// it onto the corner the user dragged it toward. Resolving the result reproduces `rect` — a
    /// window must not jump a cell when the mouse is released.
    #[must_use]
    pub fn from_rect(bounds: Rect, rect: Rect) -> Self {
        let centre = (
            rect.x.saturating_add(rect.width / 2),
            rect.y.saturating_add(rect.height / 2),
        );
        let anchor = Anchor::containing(bounds, centre);
        let max_x = bounds.x + bounds.width.saturating_sub(rect.width.min(bounds.width));
        let max_y = bounds.y + bounds.height.saturating_sub(rect.height.min(bounds.height));
        let dx = if anchor.from_right() {
            max_x.saturating_sub(rect.x)
        } else {
            rect.x.saturating_sub(bounds.x)
        };
        let dy = if anchor.from_bottom() {
            max_y.saturating_sub(rect.y)
        } else {
            rect.y.saturating_sub(bounds.y)
        };
        Self { anchor, dx, dy }
    }
}

/// A floating window: where it sits, how big it is, and how small it may get.
///
/// Pairs a [`Placement`] with a size for surfaces the user can resize as well as move. Every
/// mutator goes through the placement, so none of them can produce a position that
/// [`Placement::resolve`] would have to rescue — which is what removes the underflow described in
/// the module docs, rather than guarding against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    placement: Placement,
    size: (u16, u16),
    min: (u16, u16),
}

impl Window {
    /// A window at `placement`, `size` cells, never smaller than `min`.
    #[must_use]
    pub fn new(placement: Placement, size: (u16, u16), min: (u16, u16)) -> Self {
        Self {
            placement,
            size: (size.0.max(min.0), size.1.max(min.1)),
            min,
        }
    }

    /// Where it currently sits.
    #[must_use]
    pub const fn placement(&self) -> Placement {
        self.placement
    }

    /// Its size before any viewport clamping.
    #[must_use]
    pub const fn size(&self) -> (u16, u16) {
        self.size
    }

    /// The rect to draw it in, for this frame's `bounds`.
    #[must_use]
    pub fn rect(&self, bounds: Rect) -> Rect {
        self.placement.resolve(bounds, self.size)
    }

    /// Put its top-left at `(x, y)`, re-basing onto whichever corner it ends up nearest.
    ///
    /// Signed, because a drag reports where the mouse is and the grab offset can carry the origin
    /// past the viewport's edge; the placement it produces cannot.
    pub fn move_to(&mut self, x: i32, y: i32, bounds: Rect) {
        let width = self.size.0.min(bounds.width);
        let height = self.size.1.min(bounds.height);
        let max_x = i32::from(bounds.x) + i32::from(bounds.width.saturating_sub(width));
        let max_y = i32::from(bounds.y) + i32::from(bounds.height.saturating_sub(height));
        let x = x.clamp(i32::from(bounds.x), max_x.max(i32::from(bounds.x)));
        let y = y.clamp(i32::from(bounds.y), max_y.max(i32::from(bounds.y)));
        // Both are inside `bounds` after the clamp, so the casts cannot wrap.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let rect = Rect {
            x: x as u16,
            y: y as u16,
            width,
            height,
        };
        self.placement = Placement::from_rect(bounds, rect);
    }

    /// Shift it by `(dx, dy)` cells — the keyboard equivalent of a drag.
    pub fn nudge(&mut self, dx: i32, dy: i32, bounds: Rect) {
        let rect = self.rect(bounds);
        self.move_to(i32::from(rect.x) + dx, i32::from(rect.y) + dy, bounds);
    }

    /// Resize so the bottom-right corner tracks `(col, row)`, keeping the top-left put.
    ///
    /// This is the operation that underflows when a window's position is stored rather than
    /// derived: it needs the distance from the window's origin to the viewport's far edge, and that
    /// distance is negative for an origin outside the viewport. Here the origin comes from
    /// [`Window::rect`], which resolved it against these same `bounds`, so it is inside them by
    /// construction and the subtraction has nothing to underflow.
    pub fn resize_to(&mut self, col: u16, row: u16, bounds: Rect) {
        let rect = self.rect(bounds);
        let width = col.saturating_sub(rect.x).saturating_add(1);
        let height = row.saturating_sub(rect.y).saturating_add(1);
        self.set_size(width, height, rect, bounds);
    }

    /// Grow or shrink by `(dw, dh)` cells, keeping the top-left put.
    pub fn resize_by(&mut self, dw: i32, dh: i32, bounds: Rect) {
        let rect = self.rect(bounds);
        let width = (i32::from(rect.width) + dw).max(0);
        let height = (i32::from(rect.height) + dh).max(0);
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        self.set_size(
            width.min(i32::from(u16::MAX)) as u16,
            height.min(i32::from(u16::MAX)) as u16,
            rect,
            bounds,
        );
    }

    /// Apply a new size, floored at `min` and capped at what fits below and right of `rect`.
    fn set_size(&mut self, width: u16, height: u16, rect: Rect, bounds: Rect) {
        // Room from the window's origin to the viewport's far edge. `rect` came from `resolve`, so
        // its origin is within `bounds` and these cannot go negative.
        let room_x = (bounds.x + bounds.width).saturating_sub(rect.x);
        let room_y = (bounds.y + bounds.height).saturating_sub(rect.y);
        self.size = (
            width.max(self.min.0).min(room_x.max(1)),
            height.max(self.min.1).min(room_y.max(1)),
        );
        // The origin may have been clamped to draw this frame; re-base from where it actually is so
        // a resize against a too-small viewport does not silently move the window.
        self.placement = Placement::from_rect(
            bounds,
            Rect {
                width: self.size.0,
                height: self.size.1,
                ..rect
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Rect = Rect {
        x: 0,
        y: 0,
        width: 150,
        height: 44,
    };

    /// The property the whole design exists for. A window remembered at a comfortable size, drawn
    /// once into a viewport too small for it, must come back exactly where it was when the viewport
    /// grows again. The clamp-and-store model fails this: its clamp writes the constrained position
    /// back, and the original is gone.
    #[test]
    fn a_temporary_shrink_does_not_destroy_the_remembered_position() {
        let placement = Placement::new(Anchor::TopRight, 1, 1);
        let size = (25, 20);

        let before = placement.resolve(VIEWPORT, size);
        let squeezed = placement.resolve(
            Rect {
                width: 20,
                height: 10,
                ..VIEWPORT
            },
            size,
        );
        let after = placement.resolve(VIEWPORT, size);

        assert!(
            squeezed.width <= 20 && squeezed.height <= 10,
            "it fits the small viewport"
        );
        assert_eq!(
            after, before,
            "and the big viewport gets the original position back"
        );
    }

    /// The same scenario against the model this replaces, so the claim in the module docs is a
    /// demonstration rather than an assertion. Storing the rect and clamping it into the viewport
    /// every frame is the obvious implementation; it loses the position permanently, and this shows
    /// exactly where. If `Placement` ever regresses to that shape, the test above starts failing and
    /// this one keeps passing — the pair is what distinguishes the two designs.
    #[test]
    fn the_clamp_and_store_model_loses_the_position_and_this_one_does_not() {
        /// The explorer's `clamp_floating`, reduced to its essentials: constrain, and write back.
        fn clamp_in_place(rect: &mut Rect, bounds: Rect) {
            rect.width = rect.width.min(bounds.width);
            rect.height = rect.height.min(bounds.height);
            let max_x = bounds.x + bounds.width.saturating_sub(rect.width);
            let max_y = bounds.y + bounds.height.saturating_sub(rect.height);
            rect.x = rect.x.clamp(bounds.x, max_x);
            rect.y = rect.y.clamp(bounds.y, max_y);
        }

        let small = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 10,
        };
        let size = (25, 20);

        let mut stored = Placement::new(Anchor::TopRight, 1, 1).resolve(VIEWPORT, size);
        let originally = stored;
        clamp_in_place(&mut stored, small);
        clamp_in_place(&mut stored, VIEWPORT);
        assert_ne!(
            stored, originally,
            "the stored-rect model is supposed to lose the position here — if it stopped, this \
             test is no longer evidence for anything"
        );

        let placement = Placement::new(Anchor::TopRight, 1, 1);
        let _ = placement.resolve(small, size);
        assert_eq!(
            placement.resolve(VIEWPORT, size),
            originally,
            "a placement does not"
        );
    }

    /// Resolving must be total. These are the shapes that underflow a stored-rect implementation:
    /// an offset larger than the viewport, a window larger than the viewport, and a viewport with no
    /// area at all.
    #[test]
    fn resolve_is_total_and_never_leaves_the_viewport() {
        let sizes = [(1, 1), (25, 20), (400, 400)];
        let viewports = [
            VIEWPORT,
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
            Rect {
                x: 10,
                y: 5,
                width: 30,
                height: 12,
            },
        ];
        for anchor in Anchor::ALL {
            for (dx, dy) in [(0, 0), (1, 1), (60_000, 60_000)] {
                for size in sizes {
                    for bounds in viewports {
                        let rect = Placement::new(anchor, dx, dy).resolve(bounds, size);
                        assert!(
                            rect.x >= bounds.x && rect.y >= bounds.y,
                            "{anchor:?} {rect:?}"
                        );
                        assert!(
                            rect.x + rect.width <= bounds.x + bounds.width,
                            "{anchor:?} {dx} {rect:?} in {bounds:?}"
                        );
                        assert!(
                            rect.y + rect.height <= bounds.y + bounds.height,
                            "{anchor:?} {dy} {rect:?} in {bounds:?}"
                        );
                    }
                }
            }
        }
    }

    /// Dropping a drag must not move the window. Re-basing reads the position back out as a corner
    /// and an offset; resolving that has to land on the same cell, or the window jumps on mouse-up.
    #[test]
    fn re_basing_a_dragged_window_does_not_move_it() {
        let size = (25, 16);
        for x in [0_u16, 1, 40, 74, 120, 125] {
            for y in [0_u16, 1, 12, 22, 28] {
                let rect = Rect {
                    x,
                    y,
                    width: size.0,
                    height: size.1,
                };
                let placement = Placement::from_rect(VIEWPORT, rect);
                assert_eq!(
                    placement.resolve(VIEWPORT, size),
                    rect,
                    "re-basing moved a window at ({x}, {y})"
                );
            }
        }
    }

    /// The anchor is chosen by the window's CENTRE, not its origin. A window on the left half of a
    /// wide viewport anchors left even when its top-left is past the midpoint of nothing in
    /// particular — anchoring by origin puts a centred window on whichever side its corner fell.
    #[test]
    fn the_anchor_follows_the_window_centre() {
        let size = (60, 20);
        // Origin left of centre, centre right of it: by origin this would anchor left.
        let rect = Rect {
            x: 60,
            y: 4,
            width: size.0,
            height: size.1,
        };
        assert!(
            rect.x < VIEWPORT.width / 2,
            "the origin is on the left half"
        );
        assert!(
            rect.x + rect.width / 2 >= VIEWPORT.width / 2,
            "the centre is not"
        );
        assert_eq!(
            Placement::from_rect(VIEWPORT, rect).anchor(),
            Anchor::TopRight
        );
    }

    /// The historical position this replaced: one column in from the top-right. Pinning it keeps the
    /// default from drifting a cell when the arithmetic is touched.
    #[test]
    fn the_default_placement_sits_one_cell_in_from_the_top_right() {
        let rect = Placement::new(Anchor::TopRight, 1, 1).resolve(VIEWPORT, (25, 18));
        assert_eq!(
            rect,
            Rect {
                x: 124,
                y: 1,
                width: 25,
                height: 18
            }
        );
    }

    /// Resizing is the operation that panics in a stored-rect implementation. Drive it at every
    /// corner, including against a viewport far smaller than the window, and require no panic and a
    /// rect that still fits.
    #[test]
    fn resizing_never_underflows_however_cramped_the_viewport() {
        for anchor in Anchor::ALL {
            for bounds in [
                VIEWPORT,
                Rect {
                    x: 0,
                    y: 0,
                    width: 8,
                    height: 4,
                },
            ] {
                let mut window = Window::new(Placement::new(anchor, 2, 2), (40, 12), (10, 4));
                window.resize_to(0, 0, bounds);
                window.resize_to(u16::MAX, u16::MAX, bounds);
                window.resize_by(-1000, -1000, bounds);
                window.resize_by(1000, 1000, bounds);
                window.move_to(-500, -500, bounds);
                window.move_to(100_000, 100_000, bounds);
                let rect = window.rect(bounds);
                assert!(
                    rect.x + rect.width <= bounds.x + bounds.width,
                    "{anchor:?} {rect:?}"
                );
                assert!(
                    rect.y + rect.height <= bounds.y + bounds.height,
                    "{anchor:?} {rect:?}"
                );
            }
        }
    }

    /// A window never shrinks below its stated minimum, however small the mouse asks for.
    #[test]
    fn a_window_keeps_its_minimum_size() {
        let mut window = Window::new(Placement::new(Anchor::TopLeft, 0, 0), (40, 12), (20, 6));
        window.resize_to(0, 0, VIEWPORT);
        assert_eq!(window.size(), (20, 6));
    }

    /// Nudging is a drag by another name: the same cell count, and it stops at the edge rather than
    /// wrapping past it.
    #[test]
    fn nudging_moves_by_cells_and_stops_at_the_edge() {
        let mut window = Window::new(Placement::new(Anchor::TopLeft, 10, 5), (25, 10), (10, 4));
        window.nudge(-3, -2, VIEWPORT);
        assert_eq!(
            window.rect(VIEWPORT),
            Rect {
                x: 7,
                y: 3,
                width: 25,
                height: 10
            }
        );
        window.nudge(-100, -100, VIEWPORT);
        assert_eq!(
            window.rect(VIEWPORT),
            Rect {
                x: 0,
                y: 0,
                width: 25,
                height: 10
            }
        );
    }

    /// An anchor's quadrant is decided against the viewport's own origin, not the screen's — a
    /// window inside an offset region must not anchor by absolute coordinates.
    #[test]
    fn quadrants_are_measured_within_the_viewport_not_the_screen() {
        let bounds = Rect {
            x: 100,
            y: 50,
            width: 40,
            height: 20,
        };
        assert_eq!(Anchor::containing(bounds, (105, 55)), Anchor::TopLeft);
        assert_eq!(Anchor::containing(bounds, (135, 65)), Anchor::BottomRight);
    }
}
