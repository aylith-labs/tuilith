# tuilith

Terminal-UI components for [ratatui](https://ratatui.rs), curated and audited, each carrying where it
came from.

```toml
[dependencies]
tuilith = "0.1"
```

Two things make this more than a bag of widgets.

## Every component says what it owes upstream

The [provenance record](PROVENANCE.md) is generated from the components themselves, so it cannot claim a
lineage the code no longer has. Four kinds, and the difference matters to anyone depending on them:

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
    origin: Origin::Repo("a private repo"),
    lineage: Lineage::Inspired { by: "polygit's settings preview" },
    since: "0.1",
}
```

and five tests hold it to it: no component declared twice; a wrapper's crate is really a dependency at
the version it claims; a tracked fork's vendored tree, additions log **and upstream licence** all exist,
because attribution travels with vendored code; an inspired component names what it learned from; and
nothing claims a version this crate has not reached.

## You compile the components you take

`theme`, `overlay`, `inspect` and `provenance` need only ratatui, so they are always there. The two that
carry a dependency of their own are features, on by default and free to turn off:

```toml
tuilith = { version = "0.1", default-features = false }   # the four above, and nothing else
tuilith = { version = "0.1", features = ["document-tree"] }
```

| Feature | Component | What it pulls |
|---|---|---|
| `background` | terminal background detection | `terminal-colorsaurus` |
| `document-tree` | a JSON document as a foldable tree | `serde_json` with `preserve_order` |
| `os-appearance` | the desktop's light/dark setting (implies `background`) | — |

`preserve_order` is the reason this is a feature rather than a size optimisation: cargo unifies features
across a dependency graph, so a consumer who wanted only `inspect` would otherwise find their own
`serde_json` reordering maps.

## The dependency set is audited, not just pinned

Every dependency delta — the diff from the version we had to the version we take — has to have been
reviewed by someone we trust before it can land. That is [`cargo vet`](https://mozilla.github.io/cargo-vet/),
and it is a required check.

Where the graph stands: **63 crates fully audited, 130 grandfathered as exemptions** at the start,
against imported audit sets from Mozilla, Google, the Bytecode Alliance, Embark, ISRG and Zcash. The
exemptions are the honest backlog — `cargo vet suggest` ranks them by how little there is to read, and
the weekly job shrinks them.

Publisher trust is recorded where the peers already trust someone (`dtolnay`, `cuviper`), with a
one-year expiry, and never for a crate several people can publish — trusting one person does not cover
who else holds the keys.

`cargo deny` enforces a licence allowlist built from what is actually in the graph, with copyleft
deliberately absent: several crates offer it as one arm of an `OR`, so omitting it means we take the
permissive arm, and a crate offering *only* copyleft fails the check rather than resolving silently
during someone's bump.

Weekly, a scheduled job takes the latest of everything and opens a PR carrying the diffs and the
uncertified deltas. It cannot merge until they are certified.

## Licence

MIT ([LICENSE-MIT](LICENSE-MIT)) or Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)), at your option.
Vendored code keeps its own notice alongside, under `vendor/`.
