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

`colour`, `float`, `flow`, `inspect`, `overlay`, `pick`, `provenance`, `scroll`, `tabs` and `theme` are always available.
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
| `find` | a finder over folders, git repositories or files, drawn at once from the goto-repo cache and revalidated on a thread without moving the row under the pointer, plus a folder-by-folder picker | — (it reads the filesystem and runs `git ls-files`, which a consumer can decline) |
| `preview` | image previews: graphics-protocol detection, an off-thread encoding pool, a cache bounded by encoded bytes, and the preview widget (implies `input`) | `ratatui-image` without its chafa backend, `image` with PNG only |

`preview` decodes only PNG by itself. A consumer that previews other formats enables those codecs on
its own `image` dependency (`features = ["jpeg", "gif", "webp"]`), and feature unification gives them
to tuilith's decoder too.

`preserve_order` is the reason this is a feature rather than a size optimisation: cargo unifies features
across a dependency graph, so a consumer who wanted only `inspect` would otherwise find their own
`serde_json` reordering maps.

## The finder

`find` came from [polygit](https://github.com/steven-pribilinskiy/polygit), which uses it for its
repository finder and its folder picker. A host describes it with `find::Config` — which sources
(git repositories, every folder under some roots, home, root, bookmarks, the whole goto-repo cache, in
any combination), how a row reads (name, path, name with the path right-aligned, or two rows), the
order, the snapshot's path and TTL — and maps its own settings onto that struct; tuilith reads no
config file of its own. It draws in the host's `Palette`.

It reads goto-repo's files as they are (`~/.cache/goto-repo/repos.tsv`, its sort file and
`~/.config/goto-repo/history`), so a pick made here ranks in every tool sharing them, and it never
writes the cache.

Three more of our applications are intended to take it, beside polygit.

## The colour picker

`colour` is a picker made of views a host composes over one shared colour, so any combination stays in
step. A host lists them as data (`colour::Kind` and `colour::Place`, each with a name its config can
write) or starts from a preset: `inline` (the typed value over a filtered list of the host's named
colours), `full` (a square field, a value strip, a side panel of hex, old and new, rgb and hsl, and
editable channel boxes, common and recent swatch rows, and a footer of keys), `sliders` and
`swatches`. The fields are a square, a wheel, an HSL diamond and a hue-by-lightness grid, drawn with
the upper half block so each cell holds two colour points; strips run through value, hue, saturation
or lightness either way; sliders edit RGB, HSL or HSV. A press or drag picks the colour drawn under
the pointer, a hover holds its hex for the views that print one, and every view walks by keyboard.

Typed text parses `#rgb`, `#rrggbb`, `rgb()`, `hsl()`, the sixteen terminal names or one of the host's
names, and is refused in place, with what is accepted, when it is none of those. A terminal that
shows 256 or 16 colours is drawn in what it can show, and the hex printed is the one drawn.
`colour::Config` holds every other choice (swatch glyph, hex case and short form, text form, colour
space, common colours, side and strip sizes, tabs between fill views, keys); tuilith keeps no file,
and the recent colours are the host's to keep (`colour::remember`). A view of the host's own is any
type implementing `colour::View`; `examples/colour.rs` adds one from outside the crate.

## Dependency checks and their limits

CI runs [`cargo vet`](https://mozilla.github.io/cargo-vet/) against the repository's audits,
imported audit sets, publisher trust entries and exemptions. It also runs `cargo deny` against the
configured licence, advisory, ban and source policies. Passing those checks is not a claim that
every dependency or component has been independently audited.

The current `supply-chain/config.toml` has **187 exemption entries**. Exemptions are an explicit
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
