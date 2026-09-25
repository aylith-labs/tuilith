# tuilith — Claude Code guidance

<!-- aylith-handbook:start -->
> **📖 Aylith handbook (authoritative).** This repo is part of the `aylith-labs` lab. Before any
> cross-repo, catalog, design-system, CI/runner, or data-flow work you **must** consult the org
> handbook — the single source of truth for these conventions:
> https://github.com/aylith-labs/aylith-handbook (locally `../aylith-handbook/`, skill `aylith-labs`).
<!-- aylith-handbook:end -->


## Project Overview

A reusable component library for [ratatui](https://ratatui.rs) terminal UIs, consumed by the
lab's Rust CLIs. A crates.io publication has not been verified. Rust 2024, `ratatui` 0.30,
`crossterm` 0.29. Every component declares its stated provenance and the registry renders a
diff-checked record from those declarations. CI runs dependency checks with `cargo vet` and
`cargo deny`; exemptions and publisher trust are among the bases those checks can accept.

## Commands

```bash
cargo test --workspace                              # unit + the provenance checks
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo vet                                           # check audits, imports, trust and exemptions
cargo vet suggest                                   # the review backlog, smallest diff first
cargo deny --all-features check                     # licences, advisories, bans, sources
cargo test --workspace render_the_provenance_record # rewrites PROVENANCE.md from the registry
```

CI runs the formatting, lint, test, provenance-record, vet and deny checks; `cargo vet suggest` is
a local way to inspect the review backlog. `PROVENANCE.md` is diff-checked against declarations,
so regenerate it in the same commit as any provenance change. That check does not verify external
lineage independently.

## Architecture

- `src/provenance.rs` — the `Lineage`/`Origin` taxonomy, the `provenance!` macro, the `inventory`
  registry, and the Markdown renderer. A component declares itself; nothing keeps a second list.
- `src/theme.rs` — nine semantic colour roles as a light/dark pair, plus terminal background detection.
- `tests/provenance.rs` — structural checks on declared lineage and repository files. They read the
  manifest, vendored trees and licence files; they do not compare implementations with upstream code.
- `vendor/<crate>/` — a tracked fork's vendored source, its `ADDITIONS.md`, and upstream's licence.
- `supply-chain/` — `cargo vet`'s audits, imported peer audit sets, and the exemption backlog.

## Conventions

- **A new component declares its provenance in the same commit.** The completeness test fails otherwise,
  and a component whose lineage nobody wrote down is one nobody can audit later.
- **A component that needs a dependency of its own is a feature.** `theme`, `overlay`, `inspect` and
  `provenance` cost only ratatui and the registry and are always compiled in; `background` and
  `document-tree` gate their own dependency behind `dep:`. The reason is unification, not binary size:
  `serde_json`'s `preserve_order` reaches every consumer that shares the graph, so enabling it for a
  consumer who wanted a different component changes JSON ordering in code that never asked. CI compiles
  each feature alone from the manifest's own list, so a component that silently needs a sibling fails.
- **This repository is PUBLIC. Nothing that names a private repository, a private product, or an
  employer's internals may land in it** — not in source, not in a test fixture, not in `PROVENANCE.md`,
  which is generated *and* shipped inside the crate via the `include` list. Use `Origin::Private` for
  a component moved from a private repository: the lineage is what the record owes a reader, and the
  name is something they cannot look up anyway. Fixtures are synthetic — a captured response carries
  the field names and identifiers of wherever it was captured.
- **`Inspired` is not a fork.** Use it for a rewrite, and name what it was learned from — the credit is
  the whole obligation, and an empty one is the way to get it wrong while looking right.
- **Never widen the `deny.toml` licence allowlist to make a dependency fit.** Copyleft is absent on
  purpose; a crate offering only copyleft is a conversation, not a config edit.
- **A dependency requirement names the floor a consumer can hold, not the version it was written
  against.** Widen it to the lowest that actually compiles — CI pins every direct dependency to its
  declared floor and runs the gates there, so an over-declared requirement fails rather than
  quietly refusing a consumer. Same reasoning as `rust-version`, which is checked the same way.
- **Never certify a delta you have not read**, and prefer `cargo vet trust` only where the imported
  peers already trust that publisher. Trust entries carry a one-year expiry and are never recorded for a
  crate with multiple publishers.
- Verify a widget by looking at it in both theme variants — a colour bug in one is invisible in the
  other, which is why the palette guarantees no role shares a value between them.
- **Registers in none of the handbook's cross-repo registries.** A component library — and the planned
  CLI with its TUI storybook — runs locally, so there is no health endpoint for `aylith-infra` and no
  linkable entity for `entity-graph`, and the hub has nothing to group. The one that changes: adding a
  crates.io publish workflow makes it a `deploy-alert-targets.json` target.
