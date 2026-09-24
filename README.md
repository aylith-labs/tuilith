# tuilith

Reusable terminal-UI components for [ratatui](https://ratatui.rs), with declared provenance and
dependency checks configured in CI.

```toml
[dependencies]
tuilith = { git = "https://github.com/aylith-labs/tuilith", default-features = false }
```

Two things make this more than a bag of widgets.

## Every component says what it owes upstream

The [provenance record](PROVENANCE.md) is generated from component declarations and checked for drift
in CI. It records the stated lineage; the checks do not establish an independent code audit. Four
lineage kinds are supported:

| Kind | What it is | Can it take upstream's fixes? |
|---|---|---|
| **wrapper** | Upstream's code, re-exported. Its API and semver ride on the upstream's. | yes, by bumping |
| **tracked fork** | Upstream's code vendored at a revision, with our additions logged. | yes, by re-syncing from the recorded base |
| **inspired** | Our own implementation of an idea seen elsewhere, sharing no code with it. | **no** — a rewrite is not a fork |
| **original** | First written here. | n/a |

A component declares this beside itself:

```rust
tuilith::provenance! {
    component: "document_tree",
    about: "A JSON document as a tree you can fold",
    origin: Origin::Private,
    lineage: Lineage::Inspired { by: "polygit's settings preview" },
    since: "0.1",
}
```

Five checks cover duplicate declarations, a wrapper's stated dependency requirement, a tracked
fork's vendored tree and licence files, an inspired component's named source, and version claims.
These are structural checks on the declarations and repository files.

## You compile the components you take

`float`, `inspect`, `overlay`, `pick`, `provenance`, `scroll`, `tabs` and `theme` are always available.
Background detection and the JSON document tree are feature-gated because they add dependencies:

```toml
tuilith = { git = "https://github.com/aylith-labs/tuilith", default-features = false }
tuilith = { git = "https://github.com/aylith-labs/tuilith", default-features = false, features = ["document-tree"] }
```

| Feature | Component | What it pulls |
|---|---|---|
| `background` | terminal background detection | `terminal-colorsaurus` |
| `document-tree` | a JSON document as a foldable tree | `serde_json` with `preserve_order` |
| `os-appearance` | the desktop's light/dark setting (implies `background`) | — |

`preserve_order` is the reason this is a feature rather than a size optimisation: cargo unifies features
across a dependency graph, so a consumer who wanted only `inspect` would otherwise find their own
`serde_json` reordering maps.

## Dependency checks and their limits

CI runs [`cargo vet`](https://mozilla.github.io/cargo-vet/) against the repository's audits,
imported audit sets, publisher trust entries and exemptions. It also runs `cargo deny` against the
configured licence, advisory, ban and source policies. Passing those checks is not a claim that
every dependency or component has been independently audited.

The current `supply-chain/config.toml` has **130 exemption entries**. Exemptions are an explicit
review backlog, not evidence of completed reviews. `cargo vet suggest` can help prioritize that work.

Publisher trust entries are recorded in `supply-chain/audits.toml`; they are another basis on which
`cargo vet` can accept a dependency without a local review of each version.

`deny.toml` has a permissive licence allowlist and omits copyleft licences. The CI check evaluates
the resolved dependency graph against that policy.

A scheduled workflow attempts weekly dependency updates and opens a PR with the version diffs and
`cargo vet suggest` output. The presence of that workflow does not establish that updates were
reviewed or merged.

## Licence

MIT ([LICENSE-MIT](LICENSE-MIT)) or Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)), at your option.
Vendored code keeps its own notice alongside, under `vendor/`.
