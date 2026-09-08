---
title: Testing Strategy
type: concept
tags: [tests, rust, javascript, playwright, pasm, ci]
sources: [AGENTS.md, .github/workflows/ci.yml, tests/client/, tests/smoke/, tests/headless_runner.rs, src/core/codec_tests.rs, scripts/prepare-gm-live-event.mjs, docs/acceptance/1320-gm-live-event.md, src/perf/phase.rs, src/perf/phase_trace.rs, src/headless/app.rs, src/bin/phoenix_headless.rs, tests/phase_profiling.rs, docs/phase-timing-reduction.md, tests/common/default_pool.rs, docs/default-pool-perturbations.md, src/server_app/components.rs, tests/admitted_producer_ordering.rs]
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
authoritative digest. The focused post-split run passed these default-pool
and retained pinned guards. Scope, continuation limits and the targeted command
are in [Default-pool perturbations](../../docs/default-pool-perturbations.md).

`tests/admitted_producer_ordering.rs` exercises four actual NPC command producers
(Power, Shields, Navigation and Repair) and their six pairwise conflicts. Nine
fresh processes cover ordinary and both forced orders under default and pinned
pools. All nine 380-tick gameplay traces matched. The fixture also checks actual
foreign-prefix responses, consumer effects, repair travel/work and preserved
production ordering/deferred visibility from the live graph. Its bounded
coverage and reproduced command are in
[Four admitted-command producers](../../docs/admitted-producer-six-proof.md).

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
work visible. `tests/phase_profiling.rs` passed real coverage, binary-path and
measured/unmeasured state/census checks on the preserved declaration-stage source;
the phase implementation is unchanged in the post-split candidate.
See [the scope and metric meanings](../../docs/phase-timing-reduction.md).

## Interior-write access proof

The component module's interior_write_access_tests initialize the actual damage
systems and GM reducer, checking Bevy's resource-write metadata. Torpedo lifecycle
exposes RNG and mint writes; blaster hits and collisions expose only RNG writes.
The GM reducer exposes its Comms mint, while direct effects retain event-local
RNG reads. These checks do not execute a mission or certify an unordered pair as
commutative. The post-split census and bounded continuation guards passed.

The instance-level FixedUpdate diagnostic in
`src/headless/determinism_audit/graph.rs` exports authored and flattened edges,
nested membership and actual deferred barriers without running the inspection
App. See `docs/fixed-update-ambiguity-audit.md` for the capture command and its
capture-local identity limits. Both structural tests and the actual capture passed.

#1400's named live RNG handles keep the existing serialized aggregate while
making scheduler writes specific to actual generator cells. `sim_rng::install`
is the synchronous seed/restore boundary; `LiveStream` refuses missing or stale
seeded handles. `sim_rng::live_stream_tests` covers actual first-draw restoration,
checkpoint rollback and aggregate coherence; the census test keeps same-stream
conflicts and checks different-stream independence. The focused post-split
run below validates this source; prior graph receipts retain its base.

The state enumeration has twelve explicit full-type-path ownership aliases:
seven stream handles to `SimRng`, four namespace handles to `WorldIdMint`, and
reconnect request scratch to the existing `ReplicationLifecycleRegistry` Cache
owner. They add no canonical
`StateCensus.entries()` rows. Alias lookup inherits the existing owner class and
PASM identity and rejects missing owners, chains, shadowing and conflicting
bindings. The enumeration guard recognizes only exact physical aliases, so an
undeclared instantiation still fails classification. Typed mutable scheduler
access and the shared snapshot/digest cells are unchanged.

The post-split focused native run passed 69 tests, including actual access,
first-draw/mint restore and rollback, Fleet re-adoption, native runtime load,
Projectile identity/recoil/continuation, pool equivalence and perturbation/resume
guards. That post-split graph had 1,929 complete conflict rows: zero added and
39 removed against the original 1,968 allowance. The diagnostic owner-alias
correction passed six registry tests and all four enumeration tests; the integrator owns
final combined gates.
Cross-peer default-pool counterparts now reuse the original two-crew mesh,
stationless GM versus two ship peers, and chunked snapshot continuation guards.
Their three fresh-child parents and three pinned companions passed in the SDK-enabled native configuration, with seven completed-App reports using 16 workers and MultiThreaded FixedUpdate. The ordinary headless gate is separate. See `docs/default-pool-perturbations.md`; ordinary pinned guards and
all production schedules remain unchanged.

The narrow lobby-outbox access follow-up is covered by
`src/lobby/outbox_access_tests.rs` and `tests/lobby_outbox_ordering.rs`.
The latter records ordered mission-start/reconnect messages and every advancing
authoritative boundary for source-bound before/after comparison. Both source-bound SDK-enabled stages passed: all 90 boundaries and lifecycle messages match across default/default/pinned runs. That pre-rebase capture had 1,711 rows, with the 29 new typed vectors replacing 247 exclusive markers; all 21 deferred instances and all dependency edges matched. The original allowance is unchanged, and final combined/ordinary-headless gates remain separate.

The finalized 2026-09-08 worktree capture reports 1,540 remaining ambiguities
after typed reconnect projection, five Tactical pair annotations, the
reserved-torpedo loading edge, Comms/ObjectiveSummary ordering before the
simulation outbox drain, Viewscreen ordering before aggregate publication, and
three Power/Shields/Repair publisher pair annotations. The live
subset/history gate passes against the original 1,968-row ledger. This remains
partial #1400 work: passing debt accounting is not proof that the remaining
conflicts commute. The focused SDK-enabled publisher proof passed twelve fresh
children before and after those three annotations. Complete setup, gameplay,
wire/cache traces and child graph reports match across both stages. The full
graph loses exactly the three named conflict vectors and retains every node and
edge; five capture-local type-set IDs are matched by their unique complete
metadata. The foreign-consumer proof also passed nine children covering five
pairs and retains its unannotated scope. Full traces and graph receipts remain
available with the bounded proof.
See [Publisher order proof](../../docs/publisher-order-proof.md) and
[Foreign admitted-command consumer proof](../../docs/admitted-foreign-consumer-proof.md).
The reconnect change also preserves complete Shields/Weapons traces against
the earlier 1,685-row baseline and passes real delayed Repair/Identify coverage.
See [Typed reconnect projection proof](../../docs/reconnect-projection-proof.md)
for the actual plugin-finalized graph and focused diagnostic-test status.
