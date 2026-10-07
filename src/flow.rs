//! Layout that answers to the width a panel was given, not to the terminal's.
//!
//! A terminal UI is laid out with constants — a list thirty columns wide, a label column of sixteen,
//! a row of buttons on one line — and those constants are right at exactly one terminal size. At a
//! narrower one a row runs off the edge and ratatui clips it without a word, so a control simply is
//! not there; at a wider one a fixed column truncates text beside an empty half of the screen.
//!
//! The web answers this with container queries: a component decides its layout from the width of the
//! box it was put in. These functions are that decision, as pure geometry on display widths, so the
//! caller keeps its own spans and styles and asks only *where the breaks go*:
//!
//! - [`rows`] breaks a run of items — buttons, chips, switches — into lines, never splitting an item.
//! - [`pairs`] decides whether a key/value list fits side by side or stacks the value under its key.
//! - [`fits`] says whether a right-aligned group still fits beside the left, or must drop a line.
//! - [`columns`] says how many equal columns of a given item width a panel can hold.
//! - [`wrap`] word-wraps text, breaking a token too long for the line after a path separator first.
//!
//! **Every width is display width.** A wide glyph takes two cells, and a layout that counted
//! characters would put a break one column late for each one.

use std::ops::Range;

use ratatui::text::Span;

crate::provenance! {
    component: "flow",
    about: "Width-driven layout decisions — flowing rows, key/value stacking, columns, path-aware wrap — so a panel lays out from its own rect",
    origin: crate::Origin::Private,
    lineage: crate::Lineage::Original,
    since: "0.1",
}

/// Display width of `text` in terminal cells.
#[must_use]
pub fn width(text: &str) -> usize {
    Span::raw(text).width()
}

/// Break items of the given display widths into lines no wider than `width`, keeping each item whole.
///
/// Lines after the first start at `indent`, so a run that follows a lead label wraps under the first
/// item rather than under the label. An item wider than a line still gets a line of its own: a run
/// cannot lose an item, which is the failure this exists to prevent.
#[must_use]
pub fn rows(widths: &[usize], width: usize, indent: usize) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = indent;
    for (index, item) in widths.iter().enumerate() {
        if index > start && used + item > width {
            lines.push(start..index);
            start = index;
            used = indent;
        }
        used += item;
    }
    if start < widths.len() {
        lines.push(start..widths.len());
    }
    lines
}

/// How a key/value list lays out in a given width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pairs {
    /// Keys in a column of this many cells, values beside them.
    Side {
        /// Width of the key column, including the gap after the longest key.
        key: usize,
    },
    /// Each value on the line under its key.
    Stacked,
}

/// Side by side while the value column keeps at least `value_min` cells, stacked below that.
///
/// `key_width` is the longest key; the key column adds a two-cell gap after it.
#[must_use]
pub fn pairs(key_width: usize, value_min: usize, width: usize) -> Pairs {
    let key = key_width + 2;
    if width >= key + value_min {
        Pairs::Side { key }
    } else {
        Pairs::Stacked
    }
}

/// Whether `left` and a right-aligned `right` fit on one line with at least one cell between them.
#[must_use]
pub fn fits(left: usize, right: usize, width: usize) -> bool {
    left + 1 + right <= width
}

/// How many columns of `item` cells, separated by `gap`, fit in `width` — at least one, at most `max`.
#[must_use]
pub fn columns(item: usize, gap: usize, width: usize, max: usize) -> usize {
    let fitted = (width + gap) / (item + gap).max(1);
    fitted.clamp(1, max.max(1))
}

/// Word-wrap `text` to `width` cells, keeping explicit newlines.
///
/// A token wider than the line breaks after its last `/`, `-`, `_` or `.` that fits, so a path splits
/// between segments rather than mid-name, and only falls back to a hard split when no such place
/// exists.
#[must_use]
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for word in paragraph.split(' ').filter(|word| !word.is_empty()) {
            let mut word = word.to_string();
            let mut cells = self::width(&word);
            if used > 0 && used + 1 + cells <= width {
                line.push(' ');
                line.push_str(&word);
                used += 1 + cells;
                continue;
            }
            if used > 0 {
                lines.push(std::mem::take(&mut line));
            }
            while cells > width {
                let (head, tail) = split_token(&word, width);
                lines.push(head);
                word = tail;
                cells = self::width(&word);
            }
            line = word;
            used = cells;
        }
        lines.push(line);
    }
    lines
}

/// The longest head of `token` that fits in `width` cells, preferring to end on a separator.
fn split_token(token: &str, width: usize) -> (String, String) {
    let mut fit = 0;
    let mut used = 0;
    let mut separator = None;
    for (index, character) in token.char_indices() {
        let step = self::width(character.encode_utf8(&mut [0; 4]));
        if used + step > width {
            break;
        }
        used += step;
        fit = index + character.len_utf8();
        if matches!(character, '/' | '-' | '_' | '.') {
            separator = Some(fit);
        }
    }
    // A separator in the first quarter would leave a stub of a line; the hard split reads better. A
    // glyph wider than the whole line still has to go somewhere, so the head is never empty.
    let cut = match separator {
        Some(at) if at * 4 >= fit => at,
        _ => fit.max(token.chars().next().map_or(0, char::len_utf8)),
    };
    (token[..cut].to_string(), token[cut..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_keep_items_whole_and_indent_continuations() {
        // A 6-cell lead, then three 8-cell buttons in 24 cells: two fit after the lead, the third wraps.
        assert_eq!(rows(&[6, 8, 8, 8], 24, 6), vec![0..2, 2..4]);
        assert_eq!(
            rows(&[8, 8, 8], 30, 0),
            vec![0..3],
            "everything fits on one line"
        );
        assert_eq!(
            rows(&[40, 3], 10, 0),
            vec![0..1, 1..2],
            "an oversized item still gets its line"
        );
        assert!(rows(&[], 10, 0).is_empty());
    }

    #[test]
    fn rows_never_exceed_the_width_when_every_item_fits_a_line() {
        let widths = [7, 12, 3, 9, 4, 11, 6];
        for width in 14..60 {
            for (number, line) in rows(&widths, width, 2).into_iter().enumerate() {
                let lead = if number == 0 { 0 } else { 2 };
                let used: usize = lead + widths[line].iter().sum::<usize>();
                assert!(used <= width, "a line of {used} cells in {width}");
            }
        }
    }

    #[test]
    fn pairs_stack_when_the_value_column_would_be_too_narrow() {
        assert_eq!(pairs(14, 24, 60), Pairs::Side { key: 16 });
        assert_eq!(
            pairs(14, 24, 40),
            Pairs::Side { key: 16 },
            "exactly enough room stays beside"
        );
        assert_eq!(pairs(14, 24, 39), Pairs::Stacked);
    }

    #[test]
    fn fits_leaves_a_gap() {
        assert!(fits(10, 5, 16));
        assert!(!fits(10, 5, 15));
    }

    #[test]
    fn columns_stay_between_one_and_max() {
        assert_eq!(columns(20, 2, 70, 4), 3);
        assert_eq!(columns(20, 2, 10, 4), 1, "never zero, however narrow");
        assert_eq!(columns(5, 1, 200, 4), 4, "never more than asked for");
    }

    #[test]
    fn wrap_breaks_paths_between_segments() {
        let path = "/home/user/.local/state/example-app/sessions/decisions.md";
        // 22 cells reach into "state": a hard split would cut that segment in half.
        let lines = wrap(path, 22);
        assert!(lines.iter().all(|line| width(line) <= 22), "{lines:?}");
        assert_eq!(lines.concat(), path, "nothing is lost or added");
        assert_eq!(
            lines[0], "/home/user/.local/",
            "the first break lands after a separator"
        );
    }

    #[test]
    fn wrap_keeps_words_and_newlines() {
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap("a\nb", 10), vec!["a", "b"]);
        assert_eq!(wrap("", 10), vec![""]);
    }

    #[test]
    fn wrap_counts_cells_not_characters() {
        // Each CJK glyph is two cells: four fit in eight, the fifth wraps.
        assert_eq!(wrap("文字文字文", 8), vec!["文字文字", "文"]);
    }
}
