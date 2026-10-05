---
title: Reusable Layers
type: concept
tags: [architecture, packages, reuse, testing]
sources: [Cargo.toml, scripts/check-layers.mjs, src/lib.rs, src/boot/mod.rs, crates/phoenix-runtime/src/lib.rs, crates/phoenix-transport/src/lib.rs, crates/phoenix-platform/src/lib.rs, crates/phoenix-model/src/lib.rs, crates/phoenix-content/src/lib.rs, crates/phoenix-simulation/src/lib.rs, crates/phoenix-sim-contracts/src/lib.rs, crates/phoenix-sim-gameplay/src/lib.rs, crates/phoenix-sim-world/src/lib.rs, crates/phoenix-sim-session/src/lib.rs, crates/phoenix-presentation/src/lib.rs, examples/grid/src/lib.rs, pasm/spec/architecture/reusable-layers.yaml]
updated: 2026-10-05
---

# Reusable Layers

The Cargo workspace and JavaScript packages enforce the dependency directions.
The root `project-phoenix` crate composes hosts and preserves existing public
paths. Its binaries and browser entry points retain their existing names.

| Owner | Responsibilities | Work here when changing |
|---|---|---|
| `phoenix-runtime` | Command ordering, lockstep readiness, continuation, digest history, recovery election and chunk transfer | How peers advance and recover |
| `phoenix-transport` | Connection generations, delivery classes, paired transports, rendezvous messages and native relay/socket state machines | How bytes and logical clients travel |
| `phoenix-platform` | Atomic files, monitor identity, pane geometry/input, document surfaces, bounded frame pools, frame lifetimes, optional GPU upload/Ultralight/audio decoding | How a host uses its machine |
| `phoenix-model` | Shared Phoenix wire vocabulary, identities, rig and visual declarations | Data crossing game layer boundaries |
| `phoenix-content` | Archive and asset validation, manifests, includes/overrides, content ledger/overlays, strings and rig parsing | How authored bytes become prepared content |
| `phoenix-simulation` | Schedule assembly, live adapters between branches, content materialization, composed validation, snapshots and recovery | How the independent branches form a running mission |
| `phoenix-sim-gameplay` | Whole EntityConfig parser, ship/entity state, mechanics, physics, weapons, policy machines and Helm AI operators | How entities act |
| `phoenix-sim-world` | Whole WorldConfig parser, scripts, objectives, flags, deadlines, commitments and narrative/report state | What a scenario asks for and records |
| `phoenix-sim-session` | Crew, Station tenure, lobby decisions, connection lifecycle and fleet protocol state | Who participates and controls each Station |
| `phoenix-sim-contracts` | Shared identities, tick/RNG, schedule labels, authority rules, command envelopes and authored vocabulary | A contract used by more than one branch |
| `phoenix-presentation` | Bevy renderer, effects, cameras, HUD and Workshop preview | What the shared screen draws |
| Root `src/` | Browser exports, native host, headless runner, Workshop host and presentation adapters | Which capabilities a host installs |
| `packages/transport`, `packages/session` | Browser connection and continuation mechanisms with injected game identity/configuration | Shared browser infrastructure |
| `gui/`, `assets/` | Phoenix console presentation and authored game content | Screens, text, hulls and scenarios |

`phoenix-math` remains the deterministic numerical foundation. Runtime,
transport and platform do not depend on Phoenix model, content, simulation or
presentation. The grid game depends only on those reusable packages.

Simulation accepts narrow presentation readiness/input resources. Presentation
reads simulation state and publishes those inputs through host adapters; the
simulation package cannot import the presentation crate. Its independent build
contains no Bevy renderer/window stack or Ultralight. Content uses image decoding
for admission, which does not create a GPU or window. Model's optional `ecs`
feature derives the existing GamePhase/ShipStations ECS traits; its default
build needs neither ECS nor rendering.

Gameplay, World and Session never import one another, including in tests.
Their common contracts sit beneath them. The parent composes live borrowed
inputs and applies ordered script effects at the original schedule slots.
Unit tests live with their owning branch; cross-domain tests stay in the parent.
Snapshots and digests still fold the same state in the same order.

EntityConfig belongs wholly to Gameplay and WorldConfig wholly to World;
neither is a second, reduced parser. Validation that resolves both domains and
prepared content stays in the parent. Content's include, merge, archive
and asset mechanisms accept data or injected resolvers, keeping the dependency
one-way. Native PaneLoop retains game bridge ordering and Station/lobby choices;
the platform package owns the underlying surfaces and frames.

## Dependency graph

Open [the interactive HTML graph](../../docs/architecture/layers.html) for the
composition tree and shared foundations. Select a layer to see all its actual
dependencies, callers and source evidence. Solid arrows include runtime
dependencies; dashed arrows are test/build-only. External libraries and authored
asset data are outside this graph. Regenerate it with `npm run layers:graph`
after changing Cargo declarations or JavaScript imports;
`node scripts/generate-layers-graph.mjs --check` checks for drift.

## Focused checks

```sh
npm run layers:check
node scripts/check-layers.mjs --dependency-tree
cargo test -p phoenix-runtime -p phoenix-transport -p phoenix-model -p phoenix-content
cargo test -p phoenix-sim-contracts -p phoenix-sim-gameplay -p phoenix-sim-world -p phoenix-sim-session --lib
cargo test -p phoenix-simulation --lib --no-default-features
cargo test -p phoenix-grid
```

The dependency check covers target-specific, dev and build Cargo dependencies,
JavaScript static/dynamic imports, and independently selected dependency
graphs for the parent and every simulation branch. Workspace feature unification is not evidence of independence.

[Grid](../../examples/grid/README.md) exercises the shared command, digest,
recovery and network code through a native host and a WASM browser host. Its
console is pure JavaScript. `tests/layers/grid-smoke.mjs` drives both with a real
browser against the local rendezvous service.
