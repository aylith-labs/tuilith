---
name: tuilith
tagline: Reusable terminal UI components with declared provenance
description: >-
  Reusable ratatui components for Rust terminal applications, with a generated provenance record
  and dependency checks in CI. Optional features isolate components that bring extra dependencies.
category: developer-tools
features:
  - Provenance declarations rendered into a diff-checked component record
  - Picker, tabs, scroll area, overlays and other reusable terminal UI pieces
  - Optional background detection and JSON document tree features
  - CI checks with cargo-vet and cargo-deny
targetUser: Rust developers building terminal applications who want reusable components with stated origins
---

## Why

Rust terminal applications often need the same small interaction and layout pieces. Tuilith gathers
these pieces behind a ratatui API and records the stated origin of each component.

## What it is for

Use it as a dependency in a Rust terminal application. Read `PROVENANCE.md` for each component's
declared origin and lineage. The repository's tests check the record against the declarations and
the repository's dependency metadata; they do not establish a complete external audit.
