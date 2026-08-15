//! A document, as a tree you can fold.
//!
//! What a 3.3 MB response needs is not a highlighter — it is folds. A collapsed node says what it is
//! and how much is in it (`▸ result {1}`, `▾ favorites [5]`), and only what is open costs rows. That
//! makes a document browsable at any size, where pretty-printed text is a wall however well coloured.
//!
//! Fold state is held **by path**, never by row index: folding a node above the selection moves every
//! row below it, and an index would then point at a different value than the one being read.
//!
//! It knows nothing about what is showing it: a result pane, a request body, a record drawer and a
//! settings preview are the same problem, which is why this takes a `Value` and a palette and no more.

use std::collections::BTreeSet;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::theme::Palette;

crate::provenance! {
    component: "document_tree",
    about: "A JSON document as a tree you can fold, with per-node counts and folds held by path",
    origin: crate::Origin::Private,
    lineage: crate::Lineage::Inspired { by: "polygit's settings preview" },
    since: "0.1",
}

/// What a row holds, which decides how it is drawn and whether it can fold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// An object, and how many fields it holds.
    Object(usize),
    /// An array, and how many items it holds.
    Array(usize),
    /// A string, drawn with its quotes so `"4"` and `4` stay distinguishable.
    Text,
    /// A number.
    Number,
    /// A boolean.
    Bool,
    /// Present and empty, which is different from absent.
    Null,
}

impl Kind {
    fn of(value: &Value) -> Self {
        match value {
            Value::Object(fields) => Self::Object(fields.len()),
            Value::Array(items) => Self::Array(items.len()),
            Value::String(_) => Self::Text,
            Value::Number(_) => Self::Number,
            Value::Bool(_) => Self::Bool,
            Value::Null => Self::Null,
        }
    }

    /// Whether it holds anything worth opening. An empty object folds to nothing, so it is a leaf.
    #[must_use]
    pub fn is_container(self) -> bool {
        matches!(self, Self::Object(1..) | Self::Array(1..))
    }

    /// `{12}` or `[5]` — brackets that say which kind it is without a word for it.
    ///
    /// Only for a container: an empty object has nothing to count and nothing to open, so it reads as
    /// the leaf value `{}` rather than as a foldable node holding zero things.
    fn count(self) -> Option<String> {
        match self {
            Self::Object(len) if len > 0 => Some(format!("{{{len}}}")),
            Self::Array(len) if len > 0 => Some(format!("[{len}]")),
            _ => None,
        }
    }
}

/// One drawn line of the tree.
pub struct Row {
    /// Dotted path from the root, which is the row's identity across folds.
    pub path: String,
    /// How deep in the document it sits, which is its indentation.
    pub depth: u16,
    /// The key, or `[3]` for an array's item.
    pub label: String,
    /// What it holds, which decides how it draws and whether it can fold.
    pub kind: Kind,
    /// The scalar's own text, for a leaf.
    pub value: Option<String>,
    /// Whether it is currently unfolded.
    pub open: bool,
}

/// A document, with what is folded and where the cursor is.
pub struct JsonTree {
    value: Value,
    /// Paths of the containers currently expanded. The root is always open.
    open: BTreeSet<String>,
    /// Rebuilt when the folds or the value change, not per frame: a fully unfolded document is tens
    /// of thousands of rows, and rebuilding that at the frame rate is what makes a TUI feel slow.
    rows: Vec<Row>,
    selected: Option<String>,
    scroll: usize,
}

impl JsonTree {
    /// A document, everything folded — where a large one is legible.
    #[must_use]
    pub fn new(value: Value) -> Self {
        let mut tree = Self {
            value,
            open: BTreeSet::new(),
            rows: Vec::new(),
            selected: None,
            scroll: 0,
        };
        tree.rebuild();
        tree
    }

    /// Replaces the document, keeping which paths were open — a re-run of the same page should not
    /// re-fold everything the reader had opened.
    pub fn replace(&mut self, value: Value) {
        self.value = value;
        self.rebuild();
    }

    /// The rows as currently folded, in document order.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The first row drawn.
    #[must_use]
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The selected row's path, if there is one.
    #[must_use]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Whether this row is the selected one.
    #[must_use]
    pub fn is_selected(&self, row: &Row) -> bool {
        self.selected.as_deref() == Some(row.path.as_str())
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        collect(&self.value, "", 0, &self.open, &mut rows);
        self.rows = rows;
        // A selection whose path no longer exists — the document changed under it — falls back to the
        // first row rather than to nothing, so the pane is never left with no cursor.
        if !self.rows.iter().any(|row| self.is_selected(row)) {
            self.selected = self.rows.first().map(|row| row.path.clone());
        }
    }

    /// Folds or unfolds the selected node. Returns whether anything moved.
    pub fn toggle(&mut self) -> bool {
        let Some(path) = self.selected.clone() else {
            return false;
        };
        self.toggle_path(&path)
    }

    /// Folds or unfolds a node by path. Returns whether anything moved.
    pub fn toggle_path(&mut self, path: &str) -> bool {
        let Some(row) = self.rows.iter().find(|row| row.path == path) else {
            return false;
        };
        if !row.kind.is_container() {
            return false;
        }
        if !self.open.remove(path) {
            self.open.insert(path.to_owned());
        }
        self.rebuild();
        true
    }

    /// Everything closed, which is where a large document is legible.
    pub fn fold_all(&mut self) {
        self.open.clear();
        self.scroll = 0;
        self.rebuild();
    }

    /// Everything open. Bounded by nothing but the document: an explicit ask is allowed to be slow,
    /// and stopping part-way would read as the document ending there.
    pub fn unfold_all(&mut self) {
        let mut paths = BTreeSet::new();
        expand(&self.value, "", &mut paths);
        self.open = paths;
        self.rebuild();
    }

    /// Moves the cursor by whole rows, stopping at the ends.
    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let current = self
            .rows
            .iter()
            .position(|row| self.is_selected(row))
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(self.rows.len() - 1);
        self.selected = Some(self.rows[next].path.clone());
    }

    /// Selects a path, if the tree currently shows it.
    pub fn select_path(&mut self, path: &str) {
        if self.rows.iter().any(|row| row.path == path) {
            self.selected = Some(path.to_owned());
        }
    }

    /// Keeps the selected row on screen, scrolling only when it has left.
    pub fn follow(&mut self, height: usize) {
        let height = height.max(1);
        let Some(index) = self.rows.iter().position(|row| self.is_selected(row)) else {
            return;
        };
        if index < self.scroll {
            self.scroll = index;
        } else if index >= self.scroll + height {
            self.scroll = index + 1 - height;
        }
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(height));
    }

    /// Scrolls without moving the cursor, stopping at the last screenful.
    pub fn scroll_by(&mut self, delta: isize, height: usize) {
        let last = self.rows.len().saturating_sub(height.max(1));
        self.scroll = self.scroll.saturating_add_signed(delta).min(last);
    }
}

fn collect(value: &Value, path: &str, depth: u16, open: &BTreeSet<String>, rows: &mut Vec<Row>) {
    match value {
        Value::Object(fields) => {
            for (key, field) in fields {
                let child = join(path, key);
                push(rows, &child, depth, key.clone(), field, open);
                if open.contains(&child) {
                    collect(field, &child, depth + 1, open, rows);
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let child = join(path, &index.to_string());
                push(rows, &child, depth, format!("[{index}]"), item, open);
                if open.contains(&child) {
                    collect(item, &child, depth + 1, open, rows);
                }
            }
        }
        // A scalar at the root is its own single row.
        other => push(rows, path, depth, String::new(), other, open),
    }
}

fn push(
    rows: &mut Vec<Row>,
    path: &str,
    depth: u16,
    label: String,
    value: &Value,
    open: &BTreeSet<String>,
) {
    let kind = Kind::of(value);
    rows.push(Row {
        path: path.to_owned(),
        depth,
        label,
        kind,
        value: (!kind.is_container()).then(|| leaf_text(value)),
        open: open.contains(path),
    });
}

/// Every container's path, for unfolding the lot.
fn expand(value: &Value, path: &str, into: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            for (key, field) in fields {
                let child = join(path, key);
                if Kind::of(field).is_container() {
                    into.insert(child.clone());
                }
                expand(field, &child, into);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let child = join(path, &index.to_string());
                if Kind::of(item).is_container() {
                    into.insert(child.clone());
                }
                expand(item, &child, into);
            }
        }
        _ => {}
    }
}

fn join(path: &str, segment: &str) -> String {
    if path.is_empty() {
        segment.to_owned()
    } else {
        format!("{path}.{segment}")
    }
}

/// A leaf's text. A string keeps its quotes, because `"4"` and `4` are different values and a table
/// that hides the difference is where a type confusion starts.
fn leaf_text(value: &Value) -> String {
    match value {
        Value::String(text) => format!("\"{text}\""),
        Value::Object(_) => "{}".to_owned(),
        Value::Array(_) => "[]".to_owned(),
        other => other.to_string(),
    }
}

/// One row as a line. `width` is the pane's, so a long leaf is clipped rather than wrapped — a
/// document's shape is the point, and a wrapped value breaks the indentation that carries it.
#[must_use]
pub fn line<'a>(row: &'a Row, selected: bool, palette: &Palette, width: u16) -> Line<'a> {
    let indent = " ".repeat(1 + row.depth as usize * 2);
    let marker = match (row.kind.is_container(), row.open) {
        (true, true) => "▾ ",
        (true, false) => "▸ ",
        // Aligned with its foldable siblings rather than shifted left.
        (false, _) => "  ",
    };
    let key_style = Style::new().fg(palette.accent).add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(indent, Style::new()),
        Span::styled(marker, Style::new().fg(palette.dim)),
        Span::styled(row.label.clone(), key_style),
    ];
    match (row.kind.count(), &row.value) {
        (Some(count), _) => {
            spans.push(Span::styled(
                format!("  {count}"),
                Style::new().fg(palette.dim),
            ));
        }
        (None, Some(value)) => {
            let separator = if row.label.is_empty() { "" } else { ": " };
            spans.push(Span::styled(separator, Style::new().fg(palette.dim)));
            // Clipped against what the prefix already spent, not against the pane: budgeting the
            // value alone overruns the row by however wide the key was.
            let spent: usize = spans.iter().map(|span| span.content.chars().count()).sum();
            spans.push(Span::styled(
                clip(value, usize::from(width).saturating_sub(spent)),
                value_style(row.kind, palette),
            ));
        }
        (None, None) => {}
    }
    let line = Line::from(spans);
    if selected {
        // The whole row, so the band reads as a selection rather than as coloured text.
        line.style(Style::new().bg(palette.selection))
    } else {
        line
    }
}

/// Colour by type, within the theme's nine roles: a string is `ok`, a boolean `warn`, a number the
/// plain foreground, an absent value `dim`. No tenth role is invented for this — a custom theme
/// supplies nine and would have no value for a tenth.
fn value_style(kind: Kind, palette: &Palette) -> Style {
    match kind {
        Kind::Text => Style::new().fg(palette.ok),
        Kind::Bool => Style::new().fg(palette.warn),
        Kind::Null => Style::new().fg(palette.dim),
        // A number, and a container with no value of its own to style.
        _ => Style::new().fg(palette.foreground),
    }
}

fn clip(text: &str, budget: usize) -> String {
    if text.chars().count() <= budget {
        return text.to_owned();
    }
    // One cell of the budget is the marker that says it was clipped.
    let kept: String = text.chars().take(budget.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Synthetic, and deliberately so: the shape is what the tests need — two levels of nesting under
    /// a numeric key, an array of strings, a null leaf and a boolean — and a captured response from
    /// somewhere real would put that somewhere's field names and identifiers in a public repository.
    fn document() -> Value {
        json!({
            "success": true,
            "result": {
                "10000000000001": {
                    "20000000000002": {
                        "0": { "node_id": "0", "name": "alpha(1)", "parent_id": null },
                        "1": { "node_id": "1", "name": "alpha(2)", "parent_id": null }
                    }
                }
            },
            "favorites": ["first-entry", "second-entry"]
        })
    }

    fn labels(tree: &JsonTree) -> Vec<String> {
        tree.rows()
            .iter()
            .map(|row| {
                let marker = match (row.kind.is_container(), row.open) {
                    (true, true) => "▾",
                    (true, false) => "▸",
                    _ => " ",
                };
                let tail = match (row.kind.count(), &row.value) {
                    (Some(count), _) => format!(" {count}"),
                    (None, Some(value)) => format!(": {value}"),
                    _ => String::new(),
                };
                format!(
                    "{:indent$}{marker} {}{tail}",
                    "",
                    row.label,
                    indent = row.depth as usize * 2
                )
            })
            .collect()
    }

    #[test]
    fn a_document_opens_folded_with_a_count_per_container() {
        let tree = JsonTree::new(document());
        assert_eq!(
            labels(&tree),
            ["  success: true", "▸ result {1}", "▸ favorites [2]"]
        );
    }

    #[test]
    fn an_array_labels_its_items_by_index_and_keeps_string_quotes() {
        let mut tree = JsonTree::new(document());
        assert!(tree.toggle_path("favorites"));
        assert_eq!(
            labels(&tree),
            [
                "  success: true",
                "▸ result {1}",
                "▾ favorites [2]",
                "    [0]: \"first-entry\"",
                "    [1]: \"second-entry\"",
            ]
        );
    }

    #[test]
    fn opening_descends_one_level_at_a_time() {
        let mut tree = JsonTree::new(document());
        tree.toggle_path("result");
        assert!(
            labels(&tree).contains(&"  ▸ 10000000000001 {1}".to_owned()),
            "{:?}",
            labels(&tree)
        );
        tree.toggle_path("result.10000000000001");
        tree.toggle_path("result.10000000000001.20000000000002");
        let drawn = labels(&tree);
        assert!(drawn.contains(&"      ▸ 0 {3}".to_owned()), "{drawn:?}");
        // The leaf itself is still folded: nothing opens that was not asked for.
        assert!(
            !drawn.iter().any(|row| row.contains("alpha(1)")),
            "{drawn:?}"
        );
    }

    #[test]
    fn unfolding_everything_reaches_the_leaves_and_folding_returns_to_the_top() {
        let mut tree = JsonTree::new(document());
        tree.unfold_all();
        let drawn = labels(&tree);
        assert!(
            drawn.iter().any(|row| row.contains("name: \"alpha(1)\"")),
            "{drawn:?}"
        );
        assert!(
            drawn.iter().any(|row| row.contains("parent_id: null")),
            "{drawn:?}"
        );

        tree.fold_all();
        assert_eq!(tree.rows().len(), 3, "{:?}", labels(&tree));
    }

    #[test]
    fn an_empty_container_is_a_leaf_because_there_is_nothing_to_open() {
        let tree = JsonTree::new(json!({ "images": [], "meta": {} }));
        assert_eq!(labels(&tree), ["  images: []", "  meta: {}"]);
        assert!(!tree.rows()[0].kind.is_container());
    }

    #[test]
    fn the_selection_is_a_path_so_folding_above_it_does_not_move_it() {
        let mut tree = JsonTree::new(document());
        tree.toggle_path("favorites");
        tree.select_path("favorites.1");
        assert_eq!(tree.selected(), Some("favorites.1"));
        // Opening `result` inserts rows above the selection; an index would now point elsewhere.
        tree.toggle_path("result");
        assert_eq!(tree.selected(), Some("favorites.1"));
        let selected = tree
            .rows()
            .iter()
            .find(|row| tree.is_selected(row))
            .unwrap();
        assert_eq!(selected.value.as_deref(), Some("\"second-entry\""));
    }

    #[test]
    fn a_selection_the_new_document_does_not_have_falls_back_rather_than_vanishing() {
        let mut tree = JsonTree::new(document());
        tree.toggle_path("favorites");
        tree.select_path("favorites.1");
        tree.replace(json!({ "success": false }));
        assert_eq!(tree.selected(), Some("success"));
    }

    #[test]
    fn replacing_the_document_keeps_what_was_open() {
        let mut tree = JsonTree::new(document());
        tree.toggle_path("result");
        tree.replace(document());
        assert!(
            tree.rows()
                .iter()
                .any(|row| row.path == "result.10000000000001"),
            "a re-run must not re-fold what the reader opened"
        );
    }

    #[test]
    fn a_long_leaf_is_clipped_to_the_pane_rather_than_wrapped() {
        let long = "x".repeat(400);
        let tree = JsonTree::new(json!({ "note": long }));
        let drawn = line(&tree.rows()[0], false, &crate::theme::DEFAULT_DARK, 40);
        assert!(drawn.width() <= 40, "{}", drawn.width());
        assert!(
            drawn.spans.iter().any(|span| span.content.ends_with('…')),
            "a clip has to say it clipped"
        );
    }

    #[test]
    fn a_scalar_document_is_one_row() {
        let tree = JsonTree::new(json!("just a string"));
        assert_eq!(tree.rows().len(), 1);
        assert_eq!(tree.rows()[0].value.as_deref(), Some("\"just a string\""));
    }
}
