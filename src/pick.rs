//! A typeahead filter that says where each match landed.
//!
//! Filtering a list is easy; explaining the result is not. A `contains` answers whether a row belongs
//! and nothing else, so a filtered list shows rows whose reason for being there is invisible — the
//! reader re-scans each one for the letters they typed. What closes that gap is not a better predicate
//! but a different return: [`score`] answers *where* the query landed, and [`row`] draws those cells in
//! the accent colour, so the list explains itself.
//!
//! ```no_run
//! # use ratatui::{Frame, layout::Rect};
//! # use tuilith::{pick, scroll, theme::Palette};
//! # fn draw(frame: &mut Frame, area: Rect, palette: &Palette, items: &[String]) {
//! let mut filter = pick::Filter::new();
//! filter.push('c');
//! filter.refilter(items.iter().map(String::as_str));
//!
//! let area = scroll::Area::inside(area);
//! filter.follow(area.content().height as usize);
//! for (offset, hit) in filter.visible(area.content().height as usize) {
//!     let selected = filter.is_selected(offset);
//!     let _line = pick::row(&items[hit.index], hit, selected, palette, area.content().width);
//! }
//! area.bar(filter.hits().len(), filter.scroll()).draw(frame);
//! # }
//! ```
//!
//! **The string you match is the string you draw.** Offsets are measured against one candidate and
//! drawn against another only by mistake, and that mistake is invisible — the highlight simply lands on
//! the wrong letters. [`Filter::measured`] records how many candidates the last [`Filter::refilter`]
//! saw so a caller can assert it, and [`row`] falls back to plain text rather than slicing a string the
//! offsets do not fit. Neither can make a wrong list right; both stop it being drawn as if it were.
//!
//! **What it deliberately does not do**: own a rect, or draw a scrollbar. Both belong to
//! [`scroll::Area`](crate::scroll::Area), which exists precisely so a bar cannot be drawn over the text
//! it belongs to — a second implementation here would recreate that defect.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Palette;

crate::provenance! {
    component: "pick",
    about: "A typeahead filter that scores by subsequence and says where each match landed",
    origin: crate::Origin::Here,
    lineage: crate::Lineage::Inspired { by: "fzf's positional bonus scoring" },
    since: "0.1",
}

/// Every matched character earns this much, so a longer query outranks a shorter one.
const MATCH: i32 = 16;
/// The candidate's very first character. A query someone typed usually starts where the name does.
const FIRST: i32 = 12;
/// After a separator, or at a lower-to-upper transition — the places a human reads as word starts.
const BOUNDARY: i32 = 8;
/// Carried by each character of a run after the first, so `tab` beats `t…a…b`.
const CONSECUTIVE: i32 = 8;
/// A tie-breaker only: with everything else equal, the candidate that matched the case wins.
const SAME_CASE: i32 = 1;
/// Opening a gap costs more than widening one, so two gaps lose to one longer gap.
const GAP_START: i32 = -3;
const GAP_EXTEND: i32 = -1;

/// The characters a boundary bonus follows.
const SEPARATORS: [char; 6] = [' ', '-', '_', '/', '.', ':'];

/// Where one candidate matched, and how well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    score: i32,
    at: Vec<usize>,
}

impl Match {
    /// How well it matched. Comparable only against other scores for the **same** query.
    #[must_use]
    pub fn score(&self) -> i32 {
        self.score
    }

    /// The byte offset of each matched character, ascending.
    ///
    /// Byte offsets rather than character indices because the caller slices the string it scored, and
    /// every one of these is a character boundary in it.
    #[must_use]
    pub fn at(&self) -> &[usize] {
        &self.at
    }
}

/// One candidate that matched, by its position in the list handed to [`Filter::refilter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Its index in the candidate list.
    pub index: usize,
    /// Where it matched.
    pub matched: Match,
}

/// Score `candidate` against `query`, or `None` when the query is not a subsequence of it.
///
/// Case-insensitive on ASCII. An empty query matches everything at zero with no offsets, which is the
/// state a picker opens in — it must not reorder a list nobody has filtered yet.
///
/// The window is found forwards to the **earliest** end that completes the query, tightened backwards
/// to the latest start within it, and scored once over what survives. That is one pass rather than the
/// `query × candidate` table a full alignment needs, and it is enough to produce the offsets a
/// highlight wants.
///
/// **Its known cost, stated rather than discovered:** only the earliest window is ever considered, so a
/// later one that would have scored higher is not compared. `ab` against `"a-b axb"` returns the
/// leading `a-b`, and would do so even if the trailing run scored better. A test pins that. Where the
/// ranking matters more than the pass — a list of thousands, or candidates that repeat the query —
/// a full alignment is the thing to reach for, and this is not it.
#[must_use]
pub fn score(query: &str, candidate: &str) -> Option<Match> {
    if query.is_empty() {
        return Some(Match {
            score: 0,
            at: Vec::new(),
        });
    }

    let needles: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let hay: Vec<(usize, char)> = candidate.char_indices().collect();

    // Forwards to the earliest end that can hold the whole query.
    let mut end = None;
    let mut wanted = 0;
    for (position, (_, character)) in hay.iter().enumerate() {
        if fold(*character) == needles[wanted] {
            wanted += 1;
            if wanted == needles.len() {
                end = Some(position);
                break;
            }
        }
    }
    let end = end?;

    // Backwards from there to the latest start, which is the tightest window holding it.
    let mut start = end;
    let mut wanted = needles.len();
    for position in (0..=end).rev() {
        if fold(hay[position].1) == needles[wanted - 1] {
            wanted -= 1;
            if wanted == 0 {
                start = position;
                break;
            }
        }
    }

    let mut at = Vec::with_capacity(needles.len());
    let mut total = 0;
    let mut wanted = 0;
    let mut previous_matched = None;
    let mut in_gap = false;

    for position in start..=end {
        let (offset, character) = hay[position];
        if wanted < needles.len() && fold(character) == needles[wanted] {
            total += MATCH;
            if position == 0 {
                total += FIRST;
            } else if is_boundary(hay[position - 1].1, character) {
                total += BOUNDARY;
            }
            if previous_matched == Some(position.wrapping_sub(1)) {
                total += CONSECUTIVE;
            }
            if character == query.chars().nth(wanted).unwrap_or(character) {
                total += SAME_CASE;
            }
            at.push(offset);
            previous_matched = Some(position);
            wanted += 1;
            in_gap = false;
        } else {
            total += if in_gap { GAP_EXTEND } else { GAP_START };
            in_gap = true;
        }
    }

    (wanted == needles.len()).then_some(Match { score: total, at })
}

fn fold(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
}

fn is_boundary(previous: char, current: char) -> bool {
    SEPARATORS.contains(&previous) || (previous.is_lowercase() && current.is_uppercase())
}

/// A query, what it matched, and where the selection and the viewport sit.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    query: String,
    focused: bool,
    hits: Vec<Hit>,
    selected: usize,
    scroll: usize,
    measured: usize,
}

impl Filter {
    /// An empty filter, unfocused, matching nothing until [`Self::refilter`] is called.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What has been typed so far.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether keystrokes are going to the query rather than to the list.
    #[must_use]
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Send keystrokes to the query, or back to the list.
    pub fn focus(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Append a character to the query. Call [`Self::refilter`] afterwards.
    pub fn push(&mut self, character: char) {
        self.query.push(character);
    }

    /// Drop the last character. Call [`Self::refilter`] afterwards.
    pub fn pop(&mut self) {
        self.query.pop();
    }

    /// Empty the query. Call [`Self::refilter`] afterwards.
    pub fn clear(&mut self) {
        self.query.clear();
    }

    /// Re-run the query over `candidates`.
    ///
    /// The selection stays on the same candidate wherever it still matches, and clamps rather than
    /// dangling where it does not — a selection that jumped on every keystroke would make the list
    /// unusable exactly when it is longest.
    pub fn refilter<'a>(&mut self, candidates: impl Iterator<Item = &'a str>) {
        let held = self.selected().map(|hit| hit.index);

        self.hits.clear();
        self.measured = 0;
        for (index, candidate) in candidates.enumerate() {
            self.measured += 1;
            if let Some(matched) = score(&self.query, candidate) {
                self.hits.push(Hit { index, matched });
            }
        }

        // Score, then the shorter candidate, then the order they arrived in. That last term is what
        // stops the list reshuffling under a keystroke that changed nothing: two candidates scoring
        // alike would otherwise swap on every pass, and the jitter is invisible to a test that only
        // checks the top hit.
        if !self.query.is_empty() {
            self.hits.sort_by(|left, right| {
                right
                    .matched
                    .score
                    .cmp(&left.matched.score)
                    .then_with(|| left.index.cmp(&right.index))
            });
        }

        self.selected = held
            .and_then(|index| self.hits.iter().position(|hit| hit.index == index))
            .unwrap_or(0)
            .min(self.hits.len().saturating_sub(1));
    }

    /// What matched, best first.
    #[must_use]
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// How many candidates the last [`Self::refilter`] saw.
    ///
    /// A caller drawing against a list of a different length is drawing against offsets nobody measured
    /// on it. This is what lets that be asserted rather than assumed.
    #[must_use]
    pub fn measured(&self) -> usize {
        self.measured
    }

    /// The hit under the cursor, if anything matched.
    #[must_use]
    pub fn selected(&self) -> Option<&Hit> {
        self.hits.get(self.selected)
    }

    /// Whether the hit at `offset` in [`Self::hits`] is the selected one.
    #[must_use]
    pub fn is_selected(&self, offset: usize) -> bool {
        offset == self.selected
    }

    /// Move the cursor, clamped at both ends rather than wrapping: a list that wraps takes the reader
    /// somewhere they did not ask to go, and they only find out by reading the row.
    pub fn move_by(&mut self, delta: isize) {
        if self.hits.is_empty() {
            self.selected = 0;
            return;
        }
        let last = self.hits.len() - 1;
        self.selected = if delta >= 0 {
            self.selected.saturating_add(delta.unsigned_abs()).min(last)
        } else {
            self.selected.saturating_sub(delta.unsigned_abs())
        };
    }

    /// Scroll so the selection is inside a viewport `height` rows tall.
    pub fn follow(&mut self, height: usize) {
        if height == 0 {
            self.scroll = 0;
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }
        self.scroll = self.scroll.min(self.hits.len().saturating_sub(height));
    }

    /// The first row of the viewport, as an offset into [`Self::hits`].
    #[must_use]
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The hits inside the viewport, each with its offset into [`Self::hits`].
    pub fn visible(&self, height: usize) -> impl Iterator<Item = (usize, &Hit)> {
        self.hits.iter().enumerate().skip(self.scroll).take(height)
    }
}

/// One row, with the matched characters picked out.
///
/// `text` must be the string `hit` was scored against. Where it is not, the offsets will not land on
/// its character boundaries and the row is drawn as plain text — wrong, but visibly ordinary rather
/// than sliced apart at the wrong letters.
#[must_use]
pub fn row<'a>(
    text: &'a str,
    hit: &Hit,
    selected: bool,
    palette: &Palette,
    width: u16,
) -> Line<'a> {
    let base = Style::default().fg(palette.foreground);
    let base = if selected {
        base.bg(palette.selection).add_modifier(Modifier::BOLD)
    } else {
        base
    };
    let matched = base.fg(palette.accent).add_modifier(Modifier::BOLD);

    let fits = hit
        .matched
        .at
        .iter()
        .all(|offset| text.is_char_boundary(*offset));
    if !fits || hit.matched.at.is_empty() {
        return Line::from(Span::styled(clip(text, width), base));
    }

    let mut spans = Vec::new();
    let mut cursor = 0;
    for offset in &hit.matched.at {
        let offset = *offset;
        if offset > cursor {
            spans.push(Span::styled(&text[cursor..offset], base));
        }
        let end = text[offset..]
            .char_indices()
            .nth(1)
            .map_or(text.len(), |(step, _)| offset + step);
        spans.push(Span::styled(&text[offset..end], matched));
        cursor = end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(&text[cursor..], base));
    }
    Line::from(spans)
}

/// The search box: what has been typed, or what to type, and how much it left.
#[must_use]
pub fn query_line(
    filter: &Filter,
    placeholder: &str,
    palette: &Palette,
    width: u16,
) -> Line<'static> {
    let sigil = Style::default().fg(if filter.focused() {
        palette.accent
    } else {
        palette.dim
    });
    let mut spans = vec![Span::styled(" / ", sigil)];

    if filter.query().is_empty() {
        spans.push(Span::styled(
            placeholder.to_string(),
            Style::default().fg(palette.dim),
        ));
    } else {
        spans.push(Span::styled(
            filter.query().to_string(),
            Style::default().fg(palette.foreground),
        ));
    }
    if filter.focused() {
        spans.push(Span::styled("▏".to_string(), sigil));
    }

    // Only once a query has actually dropped something: "7 of 7" is noise.
    if !filter.query().is_empty() && filter.hits().len() != filter.measured() {
        let count = format!("{} of {}", filter.hits().len(), filter.measured());
        let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
        let pad = (width as usize).saturating_sub(used + count.chars().count() + 1);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(count, Style::default().fg(palette.dim)));
    }
    Line::from(spans)
}

fn clip(text: &str, width: u16) -> &str {
    let width = width as usize;
    if text.chars().count() <= width {
        return text;
    }
    let end = text
        .char_indices()
        .nth(width)
        .map_or(text.len(), |(offset, _)| offset);
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::widgets::Paragraph;

    fn ranked<'a>(query: &str, candidates: &[&'a str]) -> Vec<&'a str> {
        let mut filter = Filter::new();
        for character in query.chars() {
            filter.push(character);
        }
        filter.refilter(candidates.iter().copied());
        filter
            .hits()
            .iter()
            .map(|hit| candidates[hit.index])
            .collect()
    }

    /// The state a picker opens in. Reordering a list nobody has filtered would
    /// move rows the reader is still looking at.
    #[test]
    fn an_empty_query_keeps_the_order_it_was_given() {
        let candidates = ["zebra", "apple", "mango"];
        assert_eq!(ranked("", &candidates), candidates);
        let matched = score("", "anything").expect("an empty query matches");
        assert_eq!(matched.score(), 0);
        assert!(matched.at().is_empty(), "and highlights nothing");
    }

    /// The invariant that makes a highlight correct rather than decorative: every
    /// offset points at a character the query actually asked for.
    #[test]
    fn every_returned_offset_points_at_a_character_the_query_asked_for() {
        for (query, candidate) in [
            ("cpu", "cpu usage"),
            ("mem", "memory_percent"),
            ("tbr", "tab_bar_rows"),
            ("é", "café au lait"),
            ("ab", "a-b axb"),
        ] {
            let matched = score(query, candidate).expect("a match");
            let picked: String = matched
                .at()
                .iter()
                .map(|offset| candidate[*offset..].chars().next().expect("a boundary"))
                .collect();
            assert_eq!(
                picked.to_lowercase(),
                query.to_lowercase(),
                "{query} against {candidate}"
            );
        }
    }

    #[test]
    fn a_candidate_missing_a_character_never_matches() {
        assert!(score("cpx", "cpu usage").is_none());
        assert!(score("upc", "cpu").is_none(), "order is part of the query");
        assert!(score("cpuu", "cpu").is_none());
    }

    #[test]
    fn a_candidate_holding_them_in_order_always_does() {
        assert!(score("cu", "cpu").is_some());
        assert!(score("CPU", "cpu usage").is_some(), "case is folded");
        assert!(score("cpu", "CPU").is_some());
    }

    #[test]
    fn a_match_at_a_word_boundary_beats_one_inside_a_word() {
        assert_eq!(ranked("cpu", &["occupancy", "cpu usage"])[0], "cpu usage");
        assert_eq!(
            ranked("bar", &["subarray", "tab_bar_rows"])[0],
            "tab_bar_rows",
            "after an underscore is a boundary"
        );
    }

    /// With no boundary to separate them — neither candidate starts on a match
    /// and neither has a separator — the run wins on adjacency alone.
    #[test]
    fn a_run_of_adjacent_matches_beats_the_same_characters_scattered() {
        assert_eq!(
            ranked("abc", &["xaxbxc", "xxabc"])[0],
            "xxabc",
            "three in a row beats three apart"
        );
    }

    /// And a word-boundary match outranks a run buried mid-word, which is the
    /// trade the bonuses make: `a_b_c` reads as an acronym and is meant to win.
    #[test]
    fn an_acronym_across_word_starts_outranks_a_buried_run() {
        assert_eq!(ranked("abc", &["xxabc", "a_b_c_x"])[0], "a_b_c_x");
    }

    /// Only the earliest completing window is considered. Pinned as behaviour
    /// rather than left to be discovered: the trailing `axb` is never compared,
    /// however it would have scored.
    #[test]
    fn only_the_earliest_window_is_considered() {
        let matched = score("ab", "a-b axb").expect("a match");
        assert_eq!(
            matched.at(),
            [0, 2],
            "the leading a-b, and the later run is never looked at"
        );

        // The start is still tightened within that window: the second `a` is the
        // one that matches, not the first.
        assert_eq!(score("ab", "a a b").expect("a match").at(), [2, 4]);
    }

    /// Without the arrival-order tie-break the list reshuffles under a keystroke
    /// that changed nothing, and a test checking only the top hit cannot see it.
    #[test]
    fn two_candidates_that_score_alike_keep_the_order_they_arrived_in() {
        let candidates = ["ab", "ab"];
        let mut filter = Filter::new();
        filter.push('a');
        filter.refilter(candidates.iter().copied());
        let first: Vec<usize> = filter.hits().iter().map(|hit| hit.index).collect();
        filter.refilter(candidates.iter().copied());
        let again: Vec<usize> = filter.hits().iter().map(|hit| hit.index).collect();
        assert_eq!(first, again);
        assert_eq!(first, vec![0, 1]);
    }

    #[test]
    fn the_selection_survives_a_keystroke_that_drops_the_row_it_was_on() {
        let candidates = ["cpu", "memory", "load"];
        let mut filter = Filter::new();
        filter.refilter(candidates.iter().copied());
        filter.move_by(1);
        assert_eq!(filter.selected().expect("a hit").index, 1, "memory");

        // A query that still holds it keeps the selection on it, wherever it ranks.
        filter.push('m');
        filter.refilter(candidates.iter().copied());
        assert_eq!(filter.selected().expect("a hit").index, 1);

        // One that drops it clamps rather than dangling past the end.
        filter.clear();
        filter.push('l');
        filter.refilter(candidates.iter().copied());
        assert_eq!(filter.selected().expect("a hit").index, 2, "load");
        assert!(filter.selected.lt(&filter.hits().len()));
    }

    /// Offsets measured on one list cannot highlight another. It cannot make the
    /// wrong list right; it stops it being drawn as though it were.
    #[test]
    fn offsets_measured_on_one_string_do_not_highlight_another() {
        let hit = Hit {
            index: 0,
            matched: score("cpu", "cpu usage").expect("a match"),
        };
        let palette = crate::theme::DEFAULT_DARK;

        let honest = row("cpu usage", &hit, false, &palette, 40);
        assert!(honest.spans.len() > 1, "matched runs are picked out");

        // A multi-byte string the offsets do not land on boundaries of.
        let wrong = row("é", &hit, false, &palette, 40);
        assert_eq!(wrong.spans.len(), 1, "plain text rather than a wrong slice");
    }

    /// The one that catches a highlight a column left: read it off the frame.
    #[test]
    fn the_matched_columns_are_the_ones_painted_in_the_accent() {
        let palette = crate::theme::DEFAULT_DARK;
        let hit = Hit {
            index: 0,
            matched: score("cu", "cpu").expect("a match"),
        };
        let area = Rect::new(0, 0, 8, 1);
        let mut terminal =
            Terminal::new(TestBackend::new(area.width, area.height)).expect("backend");
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new(row("cpu", &hit, false, &palette, area.width)),
                    area,
                );
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        let accented: Vec<char> = (0..3)
            .filter(|column| buffer[(*column, 0)].fg == palette.accent)
            .map(|column| buffer[(column, 0)].symbol().chars().next().expect("a cell"))
            .collect();
        assert_eq!(accented, vec!['c', 'u'], "the p between them is ordinary");
        assert_eq!(
            buffer[(1u16, 0u16)].fg,
            palette.foreground,
            "and it is drawn in the ordinary foreground rather than merely not accented"
        );
    }

    #[test]
    fn a_count_appears_only_once_the_query_has_dropped_something() {
        let candidates = ["cpu", "memory"];
        let mut filter = Filter::new();
        filter.refilter(candidates.iter().copied());
        let palette = crate::theme::DEFAULT_DARK;

        let unfiltered = query_line(&filter, "type to filter", &palette, 40);
        assert!(
            !unfiltered
                .spans
                .iter()
                .any(|span| span.content.contains(" of ")),
            "nothing has been dropped, so there is nothing to count"
        );

        filter.push('c');
        filter.refilter(candidates.iter().copied());
        let filtered = query_line(&filter, "type to filter", &palette, 40);
        assert!(
            filtered
                .spans
                .iter()
                .any(|span| span.content.contains("1 of 2"))
        );
    }

    #[test]
    fn the_viewport_follows_the_selection_and_stops_at_the_ends() {
        let candidates: Vec<String> = (0..10).map(|index| format!("row {index}")).collect();
        let mut filter = Filter::new();
        filter.refilter(candidates.iter().map(String::as_str));
        assert_eq!(filter.measured(), 10);

        filter.follow(4);
        assert_eq!(filter.scroll(), 0);

        filter.move_by(6);
        filter.follow(4);
        assert_eq!(filter.scroll(), 3, "the selection is the last visible row");

        filter.move_by(100);
        filter.follow(4);
        assert_eq!(filter.scroll(), 6, "and it cannot scroll past the end");
        assert_eq!(filter.visible(4).count(), 4);
    }
}
