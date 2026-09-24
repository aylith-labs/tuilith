//! Reusable terminal-UI components for [ratatui], with declared provenance.
//!
//! The library records where each component is stated to come from and configures dependency checks
//! in CI.
//!
//! **Every component declares its provenance.** A [`Wrapper`] denotes upstream code re-exported.
//! A [`Tracked`] fork denotes code vendored at a revision with additions logged. [`Inspired`] denotes
//! an independent implementation crediting an earlier idea. [`Original`] denotes code first written
//! in one of our repositories. The record is generated from those declarations as `PROVENANCE.md`;
//! its structural checks do not independently verify the code's lineage.
//!
//! **Dependency checks have limits.** CI runs `cargo vet` against audits, imported audit sets,
//! publisher trust and exemptions, and runs `cargo deny` against repository policy. Exemptions remain
//! a review backlog; these gates do not establish that every dependency has been audited.
//!
//! [`Wrapper`]: provenance::Lineage::Wrapper
//! [`Tracked`]: provenance::Lineage::Tracked
//! [`Inspired`]: provenance::Lineage::Inspired
//! [`Original`]: provenance::Lineage::Original

#[cfg(feature = "background")]
pub mod background;
#[cfg(feature = "document-tree")]
pub mod document_tree;
pub mod float;
pub mod inspect;
pub mod overlay;
pub mod pick;
pub mod provenance;
pub mod scroll;
pub mod tabs;
pub mod theme;

#[cfg(feature = "background")]
pub use background::{Reading, Source};
pub use float::{Anchor, Placement, Window};
pub use overlay::Overlay;
pub use pick::{Filter, Hit, Match};
pub use provenance::{Lineage, Origin, Provenance};
pub use scroll::Axis;
pub use theme::{DEFAULT_DARK, DEFAULT_LIGHT, Mode, Palette};
