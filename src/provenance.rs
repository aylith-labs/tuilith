//! Where each component came from, declared beside the component itself.
//!
//! A curated library is only trustworthy if you can tell, per component, whether you are looking at
//! someone else's code, someone else's code with our changes on top, or a rewrite that owes an idea to
//! a project it shares no lines with. Those are different promises: a wrapper's API moves when its
//! upstream moves, a tracked fork can take upstream's fixes, and an inspired component cannot.
//!
//! The declaration lives next to the component and registers itself, so the catalogue is *derived*
//! from what exists rather than kept alongside it. A hand-kept list is the thing that goes stale, and
//! it goes stale silently — which for provenance means claiming an origin that is no longer true.

use std::fmt;
use std::fmt::Write as _;

/// How a component relates to code someone else wrote.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lineage {
    /// Upstream, re-exported. No divergence, nothing vendored — and its API and semver ride on the
    /// upstream's, which is exactly what a consumer needs to know before depending on it.
    Wrapper {
        /// The crate as named in `Cargo.toml`.
        crate_name: &'static str,
        /// The version requirement it is depended on at.
        req: &'static str,
    },
    /// Upstream's code, vendored at a revision, with our additions on top and a log of them.
    ///
    /// The point of recording the base is that upstream's later fixes can still be taken: a fork whose
    /// starting point nobody wrote down cannot be re-synced, only re-guessed.
    Tracked {
        /// The crate the vendored copy came from.
        crate_name: &'static str,
        /// The upstream revision the vendored copy started from.
        base: &'static str,
        /// Where the local additions are logged, relative to the repository root.
        additions: &'static str,
    },
    /// Our own implementation of an idea seen elsewhere, sharing no code with it.
    ///
    /// Not a fork. Nothing can be pulled from what inspired it and nothing is owed to it but credit —
    /// which is why the credit is mandatory rather than optional.
    Inspired {
        /// What it was learned from, named so the debt is visible.
        by: &'static str,
    },
    /// First written here.
    Original,
}

impl Lineage {
    /// The word a catalogue row uses for it.
    #[must_use]
    pub fn kind(self) -> &'static str {
        match self {
            Self::Wrapper { .. } => "wrapper",
            Self::Tracked { .. } => "tracked fork",
            Self::Inspired { .. } => "inspired",
            Self::Original => "original",
        }
    }

    /// The crate this component is a wrapper for, if it is one.
    #[must_use]
    pub fn wrapped_crate(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Wrapper { crate_name, req } => Some((crate_name, req)),
            _ => None,
        }
    }

    /// Whether upstream's later work can still be taken into this component.
    ///
    /// True only for the two lineages that share code with an upstream. An `Inspired` component cannot
    /// take a fix from what inspired it, however similar they look.
    #[must_use]
    pub fn can_take_upstream(self) -> bool {
        matches!(self, Self::Wrapper { .. } | Self::Tracked { .. })
    }
}

/// Where a component was first written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// Written for this library.
    Here,
    /// Written in another of our repositories and moved here.
    Repo(&'static str),
    /// Someone else's crate.
    Upstream(&'static str),
}

impl fmt::Display for Origin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Here => formatter.write_str("tuilith"),
            Self::Repo(name) | Self::Upstream(name) => formatter.write_str(name),
        }
    }
}

/// One component's record.
pub struct Provenance {
    /// The module path a consumer reaches it by (`document_tree`).
    pub component: &'static str,
    /// One line: what it is for.
    pub about: &'static str,
    /// Where it was first written.
    pub origin: Origin,
    /// What it owes to code someone else wrote.
    pub lineage: Lineage,
    /// The `tuilith` version it first shipped in.
    pub since: &'static str,
}

inventory::collect!(Provenance);

/// Every component's record, in catalogue order.
#[must_use]
pub fn components() -> Vec<&'static Provenance> {
    let mut all: Vec<&'static Provenance> = inventory::iter::<Provenance>().collect();
    all.sort_by_key(|entry| entry.component);
    all
}

/// Declare a component's provenance next to the component.
///
/// A macro rather than a bare `inventory::submit!` so the fields cannot be given in the wrong order,
/// and so every declaration reads the same in every module.
#[macro_export]
macro_rules! provenance {
    (
        component: $component:literal,
        about: $about:literal,
        origin: $origin:expr,
        lineage: $lineage:expr,
        since: $since:literal,
    ) => {
        inventory::submit! {
            $crate::provenance::Provenance {
                component: $component,
                about: $about,
                origin: $origin,
                lineage: $lineage,
                since: $since,
            }
        }
    };
}

/// The catalogue as Markdown, which `PROVENANCE.md` is checked against.
///
/// Rendered from the registry rather than written by hand, and diff-checked in CI, so the published
/// record cannot claim a lineage the code no longer has.
#[must_use]
pub fn render() -> String {
    let mut out = String::from(
        "# Provenance\n\n\
         GENERATED by `cargo test render_the_provenance_record` — do not edit.\n\n\
         Every component, where it came from, and what that means for depending on it. A **wrapper**\n\
         is upstream's code re-exported, so its API moves when upstream's does. A **tracked fork** is\n\
         upstream's code vendored at a revision with our additions logged, so upstream's later fixes\n\
         can still be taken. **Inspired** is our own implementation of an idea seen elsewhere, sharing\n\
         no code with it — nothing can be pulled from what inspired it. **Original** was first written\n\
         here.\n\n\
         | Component | Kind | Origin | Upstream | Since | What it is |\n\
         |---|---|---|---|---|---|\n",
    );
    for entry in components() {
        let upstream = match entry.lineage {
            Lineage::Wrapper { crate_name, req } => format!("`{crate_name}` {req}"),
            Lineage::Tracked {
                crate_name, base, ..
            } => format!("`{crate_name}` @ `{base}`"),
            Lineage::Inspired { by } => format!("after {by}"),
            Lineage::Original => "—".to_owned(),
        };
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} |",
            entry.component,
            entry.lineage.kind(),
            entry.origin,
            upstream,
            entry.since,
            entry.about,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapper_names_the_crate_it_wraps_and_can_take_its_fixes() {
        let lineage = Lineage::Wrapper {
            crate_name: "tui-overlay",
            req: "0.1.2",
        };
        assert_eq!(lineage.wrapped_crate(), Some(("tui-overlay", "0.1.2")));
        assert!(lineage.can_take_upstream());
        assert_eq!(lineage.kind(), "wrapper");
    }

    #[test]
    fn an_inspired_component_can_take_nothing_from_what_inspired_it() {
        // The distinction the taxonomy exists for: a rewrite is not a fork, so no upstream fix applies
        // to it however alike the two look.
        let lineage = Lineage::Inspired {
            by: "polygit's settings preview",
        };
        assert!(!lineage.can_take_upstream());
        assert_eq!(lineage.wrapped_crate(), None);
        assert_eq!(lineage.kind(), "inspired");
    }

    #[test]
    fn a_tracked_fork_records_the_base_it_can_be_resynced_from() {
        let lineage = Lineage::Tracked {
            crate_name: "tui-popup",
            base: "a1b2c3d",
            additions: "vendor/tui-popup/ADDITIONS.md",
        };
        assert!(lineage.can_take_upstream());
        // It is not a wrapper: its API is ours, so a consumer must not be told upstream's applies.
        assert_eq!(lineage.wrapped_crate(), None);
    }

    #[test]
    fn the_rendered_record_is_a_table_of_whatever_is_registered() {
        let rendered = render();
        assert!(rendered.starts_with("# Provenance"), "{rendered}");
        assert!(rendered.contains("| Component | Kind |"), "{rendered}");
        // One row per registered component, and the header lines are not rows.
        let rows = rendered
            .lines()
            .filter(|line| line.starts_with("| `"))
            .count();
        assert_eq!(rows, components().len());
    }
}
