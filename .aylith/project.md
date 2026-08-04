---
name: tuilith
tagline: Audited terminal-UI components for Rust
description: >-
  A curated component library for ratatui where every component records where it came from — a
  re-exported wrapper, a tracked fork that can still take upstream's fixes, a rewrite that owes an idea
  to a project it shares no code with, or something first written here. The dependency set is audited
  per version-delta with cargo-vet rather than merely pinned, reviewed weekly, so an upgrade is a
  reviewed change rather than a version bump nobody read.
category: developer-tools
status: building
features:
  - Provenance declared beside each component and published as a generated, diff-checked record
  - Five tests holding a lineage claim to its promises, including that vendored code keeps its licence
  - Weekly dependency upgrade gated on cargo-vet certification of every delta
  - Licence allowlist built from the real graph, with copyleft absent by design
targetUser: Rust developers building terminal applications who want components they can audit
---

## Why

Two of the lab's Rust TUIs wanted the same components, and a third would have wanted them again. The
generic widgets already exist on crates.io and are better maintained there than they would be here —
what does not exist is a curated set you can *audit*, with each piece honest about whose code it is.

## What it is for

Depend on it from any terminal app. Read `PROVENANCE.md` to see, per component, whether you are looking
at someone else's code, someone else's code with ours on top, or an independent implementation — and
therefore whether an upstream fix can reach you.
