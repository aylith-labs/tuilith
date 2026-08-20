//! Terminal-UI components for [ratatui], curated and audited, each carrying where it came from.
//!
//! Two things distinguish this from a bag of widgets:
//!
//! **Every component declares its provenance.** A [`Wrapper`] is upstream's code re-exported, so its
//! API moves when upstream's does. A [`Tracked`] fork is upstream's code vendored at a revision with
//! our additions logged, so upstream's later fixes can still be taken. [`Inspired`] is our own
//! implementation of an idea seen elsewhere, sharing no code with it. [`Original`] was first written
//! here. The record is derived from the components themselves and published as `PROVENANCE.md`.
//!
//! **The dependency set is audited, not just pinned.** `cargo vet` certifies the *delta* between the
//! versions we had and the versions we take, weekly, so an upgrade is a reviewed change rather than a
//! version bump nobody read.
//!
//! [`Wrapper`]: provenance::Lineage::Wrapper
//! [`Tracked`]: provenance::Lineage::Tracked
//! [`Inspired`]: provenance::Lineage::Inspired
//! [`Original`]: provenance::Lineage::Original

#[cfg(feature = "background")]
pub mod background;
#[cfg(feature = "document-tree")]
pub mod document_tree;
pub mod inspect;
pub mod overlay;
pub mod provenance;
pub mod scroll;
pub mod theme;

#[cfg(feature = "background")]
pub use background::{Reading, Source};
pub use overlay::Overlay;
pub use provenance::{Lineage, Origin, Provenance};
pub use scroll::Axis;
pub use theme::{DEFAULT_DARK, DEFAULT_LIGHT, Mode, Palette};
