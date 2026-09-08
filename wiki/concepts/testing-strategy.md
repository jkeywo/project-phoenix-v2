---
title: Testing Strategy
type: concept
tags: [tests, rust, javascript, playwright, pasm, ci]
sources: [AGENTS.md, .github/workflows/ci.yml, tests/client/, tests/smoke/, tests/headless_runner.rs, src/core/codec_tests.rs, scripts/prepare-gm-live-event.mjs, docs/acceptance/1320-gm-live-event.md, src/perf/phase.rs, src/perf/phase_trace.rs, src/headless/app.rs, src/bin/phoenix_headless.rs, tests/phase_profiling.rs, docs/phase-timing-reduction.md, tests/common/default_pool.rs, docs/default-pool-perturbations.md, src/server_app/components.rs, src/sim_sets/order.rs, src/sim_sets/order/pass.rs, tests/fixed_update_ambiguities.rs, tests/pool_equivalence.rs, tests/admitted_producer_ordering.rs, tests/captain_sensors_ordering.rs, tests/tactical_target_ordering.rs, tests/snapshot_resume.rs, tests/fixtures/worlds/scripted_order_resume.toml, docs/script-callback-order-proof.md, docs/declared-fixed-order.md, pasm/spec/architecture/deterministic-simulation.yaml]
updated: 2026-09-08
---

# Testing Strategy

Tests are placed at the narrowest public seam that can prove the behavior, with
the full CI workflow providing integration coverage across Rust, client JavaScript,
PASM, WebAssembly, rendering, performance, and balance.

## Rust

Pure modules use native unit tests: arrange public state, perform an action, and
assert on observable output. Bevy adapters use small `App` fixtures where
schedule ordering or ECS integration is part of the contract. Large test modules
live in sibling `*_tests.rs` files while remaining children of the production
module.

The headless runner is an integration test because it loads native entity
templates and boots the whole authoritative simulation. Tests that populate
the process-global native config cache belong there rather than in the library
test binary.

The manual Falling Skyway timeline tests keep scenario damage and terminal
outcomes live. Fixtures observing late dialogue or campaign records shelter
the player through storm exposure, then approach the relevant contact. An
idle-crew fixture takes Tactical designation before its operate objective
opens, so Backfill cannot complete a deliberately deferred rescue. Civilian
loss fixtures leave the traffic on its authored lanes while protecting only
the observing crew. These setup boundaries live in `tests/headless_runner.rs`.

The archetype-order, schedule-order, registration-order and snapshot-resume
binaries retain their pinned guards and add `default_pool_` parents for #1400.
`tests/common/default_pool.rs` runs exact original guards in fresh child
processes, checking actual compute-worker count, executor, seed, tick and
authoritative digest. `tests/pool_equivalence.rs` compares the mission's full
frame/tick digest sequence across two default processes and one pinned process.
Scope, continuation limits and the targeted command
are in [Default-pool perturbations](../../docs/default-pool-perturbations.md).

The Tactical, publisher, admitted-producer, foreign-consumer and Captain/Sensors proof
binaries vary actual plugin registration order under default and pinned pools.
They check the declared production order alongside complete gameplay, wire,
refusal, control-tenure and RNG/mint observations. Their graph checks retain
duplicate system registrations and compare full access and membership metadata.
Earlier opposed-order commutation receipts describe the pre-declaration source;
the current fixtures preserve the explicit order while perturbing registration.
See [Tactical ordering](../../docs/fixed-update-ambiguity-audit.md),
[Publishers](../../docs/publisher-order-proof.md),
[four admitted producers](../../docs/admitted-producer-six-proof.md),
[foreign consumers](../../docs/admitted-foreign-consumer-proof.md) and
[Captain/Sensors](../../docs/captain-sensors-order-proof.md) for bounded coverage.

The pending-script callback and same-tick ordered-callback resume guards use two
actual Apps under both pinned and fresh default-pool configurations. The small
`scripted_order_resume.toml` world makes two same-due effects noncommutative:
only their saved order produces victory. The fixture observes the complete
restored queue and every continuation tick's digest, counters and terminal
outcome, continuing after execution to catch a replay. See
[Script callback order](../../docs/script-callback-order-proof.md) for its scope.

## Client JavaScript

Vitest under `tests/client/` covers pure state builders, routing, localization,
components, authoring scripts, and other browser-independent modules. Player
console behavior stays in pure HTML/CSS/JS; tests do not introduce a client
WASM layer.

## Browser smoke and rendering

Playwright boots the real server WASM and client pages in Chromium, replacing
only the transport — a `BroadcastChannel` stand-in for the rendezvous socket
and for WebRTC, terminating the REAL worker-rendezvous registry inside the host
page (`tests/smoke/rendezvous-shim.js`, behind the `tests/smoke/transport-fixture.js`
seam). The normal project
checks message flow and DOM behavior without a GPU. The render project uses
SwiftShader and includes a pixel-level viewscreen check so a clean-console
render-graph failure cannot silently produce a blank scene.

## PASM

`uv run pasm validate`, `uv run pasm scan`, and `uv run pasm traceability`
check the repository-owned model under `pasm/spec/`. The PASM tool and its unit
suite live in Vellum; Phoenix does not have a local PASM pytest suite.

## CI gates

The ordinary Rust job runs `cargo test --workspace --features headless`.
The separate viewer job runs `cargo test --lib --features viewer viewer::`
and refuses zero matched tests. The library test binary still compiles in full,
but general tests are executed only by the ordinary suite and integration-test
binaries are not built for the viewer job.

Demo-build tests (`PHOENIX_DEMO_BUILD=true`, with the build flag, debug admission,
and absent-wire-route filters) and debug host/capture binary builds run in
independent `demo-test` and `tooling-build` jobs. Both remain deployment gates,
alongside `test`, `viewer-test`, `boundary`, `editor-test`, `build`, and `smoke`.
Each Rust job has its own cache; new jobs initially pay a cold-cache cost.

The WASM build runs independently; smoke depends on its artifact. PRs and main
pushes run the core smoke tier; nightly/manual runs and PRs labelled
`smoke-full` run the full suite. Native release builds, performance, and balance
keep their nightly/manual schedules. The Cruiser balance matrix gates its job;
regular performance comparisons report warnings rather than blocking deployment.
PASM retains its independent validation, scan and traceability job.

The #1320 human GM event has an opt-in preparation tool,
`scripts/prepare-gm-live-event.mjs`. It derives an ordinary two-slot world and
curated manifest from integrated Combat Test authoring without changing the
single-player source. The acceptance kit requires generated-asset hashes,
two live Fleet hulls proven on the integrated build, and the human event's
separate decision; generation alone establishes none of those runtime results.

During implementation, use targeted tests. Run the documented final gates once
before pushing, including the additional native configurations when verifying
the full CI matrix. See AGENTS.md for commands and PowerShell demo environment
handling. Check viewer discovery as well as its exit status.

Prefer observable behaviour over source-text pins. The enabled logging filter
is exercised by the existing world-spawned duel's damage/death assertions;
there is no separate logging duel or claim that it captures emitted log text.

## Related

- [PASM Runtime](./pasm-runtime.md)
- [Build and Deployment](./build-and-deployment.md)
- [Codec Seam](./codec-seam.md)

## Observed phase timing

The headless perf-capture path uses `src/perf/phase_trace.rs` to observe actual
FixedUpdate/system spans outside App and `src/perf/phase.rs` for pure reduction.
Per-phase execution intervals and an adjacent coverage report keep unattributed
work visible. `tests/phase_profiling.rs` covers real span attribution, the binary
capture path and measured/unmeasured state and census equivalence.
See [the scope and metric meanings](../../docs/phase-timing-reduction.md).

## Interior-write access proof

The component module's `interior_write_access_tests` initialize actual damage
systems and the GM reducer, checking Bevy's resource-write metadata. Torpedo
lifecycle exposes RNG and mint writes; blaster hits and collisions expose RNG
writes. The GM reducer exposes its Comms mint, while event-local effects retain
seed reads. These access checks do not certify unordered pairs as commutative.

Named live RNG and mint handles expose the actual generator or namespace cells
to the scheduler while preserving the serialized aggregates. `sim_rng::install`
and the mint's installation path provide synchronous seed/restore boundaries;
live handles reject missing or stale state. Their unit tests cover first-use
restoration, checkpoint rollback and aggregate coherence. The ambiguity guard
retains same-cell conflicts and checks independence between distinct cells.

The State Census has twelve explicit full-type-path ownership aliases: seven
stream handles to `SimRng`, four namespace handles to `WorldIdMint`, and reconnect
request scratch to the existing `ReplicationLifecycleRegistry` Cache owner.
Aliases add no canonical `StateCensus.entries()` rows. Alias lookup inherits the
owner's class and PASM identity and rejects missing owners, chains, shadowing
and conflicting bindings; undeclared physical instantiations remain errors.

Typed lobby/reconnect projections retain the existing cache and output owners.
`tests/lobby_outbox_ordering.rs` records ordered lifecycle messages and every
advancing authoritative boundary. The Shields, Weapons and delayed Repair
reconnect guards check consumer-visible continuation; their seams and limits
are described in [Typed reconnect projection](../../docs/reconnect-projection-proof.md).

## Declared FixedUpdate order

`src/sim_sets/order.rs` records typed owner sequences within the existing fixed
phases and at named early and late boundaries. Each `FixedStep` marker stays on
the original system registration beside its existing conditions and phase.
`order::pass::DeclaredOrder` adds execution edges after Bevy's automatic deferred
insertion, preserving command publication as a separate contract. Its unit
fixtures observe resource visibility at an existing flush and the absence of
publication at a new execution-only edge, and reject contradictory order.

The fresh-App diagnostic in `src/headless/determinism_audit/graph.rs` completes
ordinary plugin finish/cleanup before capturing authored and flattened edges,
nested membership, full conflicts and actual deferred barriers. Capture-local
identities preserve repeated instances. The ignored execution-order diagnostic
in `tests/fixed_update_ambiguities.rs` also exposes actual executable order for
comparing ordered deferred command buffers. Inspection initializes the schedule
without running Startup or a simulation frame.

The isolated ambiguity test preserves full names, access vectors and instance
multiplicity. It compares against a verified pre-change allowance, which may
shrink and may not grow. The current allowance is empty, so the standing gate
enables Bevy's ambiguity errors.
A zero census alone does not prove gameplay or deferred visibility. Final
validation combines structural comparison with the mission, perturbation,
restore and cross-peer guards. See
[Declared FixedUpdate order](../../docs/declared-fixed-order.md) and
[Ambiguity audit](../../docs/fixed-update-ambiguity-audit.md) for the contracts.
