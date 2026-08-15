//! The checks that make a provenance claim mean something.
//!
//! A record nothing verifies is a comment. These read the registry against the repository — the
//! manifest, the vendored trees, the licence files — so a claim that has stopped being true fails
//! here rather than misleading someone who reads it.

use std::path::{Path, PathBuf};

use tuilith::provenance::{self, Lineage};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn manifest() -> String {
    std::fs::read_to_string(root().join("Cargo.toml")).expect("the manifest is readable")
}

/// Renders the catalogue to `PROVENANCE.md`. CI runs this and then fails on any diff, so the published
/// record cannot drift from what the components declare.
#[test]
// Rendering under a feature subset writes a record missing whatever was compiled out, and the diff
// check downstream would then be judging a truncated file against the full one. The complete set is
// the only valid input, so the test declines rather than producing a plausible wrong answer.
#[cfg_attr(
    not(all(feature = "background", feature = "document-tree")),
    ignore = "the record is only complete when every component is compiled in"
)]
fn render_the_provenance_record() {
    let path = root().join("PROVENANCE.md");
    let rendered = provenance::render();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing != rendered {
        std::fs::write(&path, &rendered).expect("PROVENANCE.md is writable");
    }
}

#[test]
fn every_component_is_declared_exactly_once() {
    let mut names: Vec<&str> = provenance::components()
        .iter()
        .map(|entry| entry.component)
        .collect();
    let before = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        before,
        names.len(),
        "a component declares its provenance twice, so the catalogue has two answers for it"
    );
}

/// A wrapper's promise is that its API is upstream's. That is only true if upstream is actually a
/// dependency at the version the record claims — otherwise the row tells a consumer to read the docs
/// of a crate this library does not use.
#[test]
fn every_wrapper_names_a_real_dependency_at_the_stated_requirement() {
    let manifest = manifest();
    for entry in provenance::components() {
        let Some((crate_name, req)) = entry.lineage.wrapped_crate() else {
            continue;
        };
        let declared = manifest.lines().any(|line| {
            let line = line.trim();
            line.starts_with(&format!("{crate_name} ")) && line.contains(req)
        });
        assert!(
            declared,
            "`{}` says it wraps `{crate_name}` {req}, which is not a dependency at that version",
            entry.component
        );
    }
}

/// A tracked fork's promise is that upstream's later fixes can still be taken. That needs three things
/// to exist: the vendored tree, a log of what was added to it, and upstream's licence — vendoring
/// someone's MIT code carries an attribution obligation, and a machine should hold that rather than a
/// person remembering it.
#[test]
fn every_tracked_fork_can_be_resynced_and_keeps_its_upstream_licence() {
    for entry in provenance::components() {
        let Lineage::Tracked {
            crate_name,
            base,
            additions,
        } = entry.lineage
        else {
            continue;
        };
        let vendored = root().join("vendor").join(crate_name);
        assert!(
            vendored.is_dir(),
            "`{}` is a tracked fork of `{crate_name}`, but {} does not exist",
            entry.component,
            vendored.display()
        );
        assert!(
            !base.trim().is_empty(),
            "`{}` names no upstream revision, so it cannot be resynced — only re-guessed",
            entry.component
        );

        let log = root().join(additions);
        let logged = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            !logged.trim().is_empty(),
            "`{}` logs its additions at {}, which is missing or empty",
            entry.component,
            log.display()
        );

        assert!(
            carries_a_licence(&vendored),
            "`{}` vendors `{crate_name}` without its licence — the attribution travels with the code",
            entry.component
        );
    }
}

fn carries_a_licence(vendored: &Path) -> bool {
    std::fs::read_dir(vendored).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("LICEN")
        })
    })
}

/// An inspired component owes credit and nothing else. An empty credit is the one way to get that
/// wrong while looking right.
#[test]
fn every_inspired_component_credits_what_it_learned_from() {
    for entry in provenance::components() {
        if let Lineage::Inspired { by } = entry.lineage {
            assert!(
                !by.trim().is_empty(),
                "`{}` is inspired by nothing named, which is not a credit",
                entry.component
            );
        }
    }
}

/// The version a component says it shipped in has to be one this crate has reached.
#[test]
fn no_component_claims_to_have_shipped_in_a_version_that_does_not_exist_yet() {
    let current = env!("CARGO_PKG_VERSION");
    let (current_major, current_minor) = version_parts(current);
    for entry in provenance::components() {
        let (major, minor) = version_parts(entry.since);
        assert!(
            (major, minor) <= (current_major, current_minor),
            "`{}` says it shipped in {}, which is ahead of this crate's {current}",
            entry.component,
            entry.since
        );
    }
}

fn version_parts(version: &str) -> (u32, u32) {
    let mut parts = version.split('.');
    let major = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let minor = parts.next().unwrap_or("0").parse().unwrap_or(0);
    (major, minor)
}
