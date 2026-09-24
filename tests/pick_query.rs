//! Public picker layout behavior at terminal column widths.

use tuilith::{pick, theme::DEFAULT_DARK};

#[test]
fn wide_query_keeps_result_count_inside_the_requested_columns() {
    let mut filter = pick::Filter::new();
    for character in "界界".chars() {
        filter.push(character);
    }
    filter.refilter(["界界", "other"].into_iter());

    let line = pick::query_line(&filter, "search", &DEFAULT_DARK, 14);
    assert_eq!(line.width(), 14);
    assert_eq!(line.spans.last().expect("count").content, "1 of 2");
}
