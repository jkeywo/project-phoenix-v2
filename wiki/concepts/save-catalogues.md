---
title: Peer-Local Save Catalogues
type: concept
tags: [save, snapshot, persistence, autosave, browser, native, catalogue]
sources: [src/save_slots.rs, src/save_slots_lifecycle.rs, src/startup_restore.rs, src/save_slots_store.rs, src/snapshot.rs, src/core/collision_history.rs, tests/collision_history.rs, tests/same_target_damage_ordering.rs, src/gm_action.rs, src/sim_digest.rs, src/headless/replay.rs, src/headless/replay/recorded_gm.rs, tests/recorded_gm_exports.rs, src/server/bridge.rs, src/server_app/world_setup.rs, src/lockstep/mod.rs, src/ship/coordination_systems.rs, src/bin/phoenix_host.rs, src/delivery/args.rs, src/entities/config.rs, src/world/config.rs, gui/save-slots.js, gui/browser-save-identity.js, gui/browser-save-identity-worker.js, server.html, tests/save_slots_persistence.rs, tests/smoke/save-slots.spec.js]
updated: 2026-09-08
---

# Peer-Local Save Catalogues

Each authoritative simulation peer owns a private catalogue of the same
`vellum-save` `StoredRun` records used by portable save files. Persistence is a
local side effect: catalogue state, pending requests and storage outcomes never
enter the simulation digest or fleet mesh.

## Capture lifecycle

`save_slots::SaveSchedule` chooses completed logical ticks, and
`save_slots_lifecycle` captures in `FixedLast` after the simulation work and
`SimTick` advance have committed. The rolling `autosave` slot is replaced on the
first in-progress tick, every authored interval, and the first observed
`GameOver` tick. `[global] autosave_interval_secs` defaults to 30 simulation
seconds and world loading accepts it only when it converts to a positive whole
number of `sim_tick_hz` ticks.

`src/core/collision_history.rs` owns collision attribution on every simulation
host. It collects in `FixedLast` before peer digest sampling and tick advance,
so capture includes the collision events from the step just completed. The
snapshot preserves all collision rows; restore replaces history and advances
each collision/report reader past bootstrap messages without clearing their
shared input stream. Optional headless `RunTelemetry` does not affect the
digest. Rules `0.5` name this correction; the collision row shape still uses
snapshot format `32`, and older rules are refused.

A named manual request is local intent for the peer's next fixed boundary. It
captures only while the phase is `InProgress`; a request that reaches its
boundary after a phase change is consumed with a local refusal. Snapshot
construction occurs on the fixed boundary, while the browser or native Store
write drains later, so a slow or failed backend cannot delay a simulation tick.

The catalogue reserves `autosave` for the rolling row. Manual rows use opaque
UUID-shaped Store keys and keep their arbitrary display names in metadata
sidecars. Missing, corrupt or unreadable metadata and damaged/incompatible runs
remain visible and deletable where possible; rename changes only the sidecar,
and deletion requires explicit confirmation in both host surfaces.

## Browser and native ownership

The browser uses `vellum_save::LocalStorage`, but not one origin-wide namespace.
`gui/browser-save-identity.js` claims a 32-hex host identity before catalogue
access or simulation startup, and the bridge caches the resulting
`phoenix:<identity>` namespace. An exclusive Web Lock is the preferred live
ownership primitive. Where Web Locks are unavailable or denied, the
same-origin module `SharedWorker` in `gui/browser-save-identity-worker.js`
serializes live claims and transfers a Start navigation through a one-shot
session ticket acknowledged before the old document releases ownership.
LocalStorage holds only an append-only directory of durable candidate
identities; it never decides which live page owns one. There is no timeout or
lease that could make a sleeping tab lose its catalogue to another host.

A duplicated or simultaneous host therefore receives a different durable
namespace, while each namespace can be recovered once after every old page has
closed. If neither Web Locks nor SharedWorker is available, isolation wins: the
page uses a fresh in-memory identity and saved-session Start fails visibly
instead of risking a shared durable catalogue. Browser storage and Store
failures are host-page status only and are never broadcast.

The native authoritative host installs `vellum_save::FileStore`. Its default
private directory is `.phoenix/saves`, resolved relative to the directory from
which `phoenix-host` was launched; `--save-dir` overrides it. Before any Store
or resume read, the process creates that directory and takes a non-blocking
exclusive lock on its persistent `.phoenix-host.lock` sentinel. A second host
pointed at the same directory is refused at startup and must choose a distinct
stable `--save-dir`; closing the process
releases the lock, while the non-`.ron` sentinel remains and is ignored by
`FileStore`'s slot listing. The CLI exposes
`--save-list`, `--save-create`, `--save-rename`, `--save-export`, confirmed
`--save-delete`, and startup-only `--resume-save`. A CLI create made before the
mission begins waits for its first capturable in-progress fixed tick.

## Compatibility and startup restore

Catalogue rows project the display name plus canonical run metadata: scenario,
seed, capture tick, versions, and boot identity. Admission first applies
`vellum_save::Versions::check` over snapshot format, simulation rules and the
loaded content digest. A pre-scenario catalogue may defer only the content
answer until the row's scenario has loaded; damaged records and format/rules
movement are hard refusals immediately.

Before either local catalogue preparation or portable-save import performs its
full pre-init compatibility check, the browser declares the loaded root
world's `#scripts` ledger record from its lifted source set, using the same
sorted-source hash as compilation. This declaration does not compile or
activate scripts, or seal the ledger. Startup still owns compilation,
validation and the final content freeze before spawning.

The snapshot envelope also requires a `BootIdentity`: the selected hull,
the frozen `FleetRoster`, and the authored-order UUID identity of every entity
that actually spawned at `GameStart`. Startup validates the scenario and hull,
checks the saved fleet against any already-staged fleet, and verifies that each
saved authored index still names a `GameStart` row before those UUIDs are used
to build the fresh world. Restore then proceeds through the ordinary snapshot
roster/layer readiness and digest checks.

Restore replaces captured `ShipRedAlert`, attacker attribution and stored
Station stances together. It rebases only the alert component's change marker
to an already-consumed tick, including when the bootstrap value differs or its
insertion has not yet been read. Restoration therefore cannot manufacture a
Captain alert command that clears the captured attacker or changes a captured
stance. Subsequent ordinary Captain commands still update neutral stances and
clear attribution on stand-down. Separately, restoring objective records marks
their presentation dirty, producing one exact `ObjectiveSummary` refresh even
when authoritative objective state is unchanged. The damage continuation
fixture retains the exact `All` / `Reliable` wire refresh and proves that it
drains once on the restored impact tick, after impact publications and before
any GameOver transition. It is absent from the pending queue and later wire
rows. Only that proven extra refresh is removed for comparison; all other
state, attribution, events and messages must compare equal.

The same format preserves the session-pause bit, complete canonical GM action
journal, and exact `applied_grants` reducer frontier. That frontier cannot be
inferred from `SimTick`: a grant at the continuation tick may have arrived but
not yet passed through `apply_due_actions`. Restore reinstalls the journal and
frontier, derives the current log from only the applied prefix, and rejects a
snapshot whose applied prefix contains a grant beyond its capture tick. A
paused save therefore restores at the exact frozen boundary with its
operator-scoped idempotency and attributed history intact; Resume remains a new
typed GM action rather than an implicit side effect of loading.

A saved multi-peer roster reconstructs its ship count, hull choices, authored
spawns and this saving peer's local ship, but not the old live host mesh. The
new session clears the old crew assignments, installs no `FleetLockstep` wait
set and uses zero command delay: the local ship follows the new App's live
Sessions, while the other saved player ships begin on AI backfill. This is what
lets either peer's private save advance independently after the old fleet has
gone away instead of waiting forever for its old peers' watermarks.

Restore is deliberately fresh-session-only. A browser catalogue start reloads
the host page and stages the record before `wasm_init`; native staging refuses
once `SimTick` is non-zero. There is no action that mutates a running World into
a saved one. While a staged record bootstraps, lifecycle capture is suppressed,
pending manual requests are refused, and bootstrap artifacts are discarded.
After success, the scheduler marks the restored phase as already observed and
rebases its periodic cadence from the restored continuation tick, preventing a
duplicate run-start/final save and placing the next periodic autosave exactly
one interval later.

`startup_restore` owns the shared Bevy driver, staged record and terminal
outcome. The native Store adapter and browser bridge only hand off validated
records and report the result. Waiting starts after `GameStartEntityUuids`
records the completed authored roster walk, so an operator may remain in the
fresh lobby indefinitely and a later `GameOver` cannot strand restore.
Unresolved layers and missing entities consume one 1,800-frame budget; at
expiry, rebuilding is allowed only with ready layers and rebuildable entities.
The driver resolves capture suspension after checking both the report and
digest. A failure resumes capture at the current continuation without rolling
back changes already applied to the World.

## Portable transfer

Export and import do not define another format. `snapshot::TransferStore` moves
the same RON `StoredRun` text through the same save/load functions and version
gate. Browser import is a pre-activation action that loads the artifact's named
scenario and then applies the same hull and `GameStart` identity checks as a
catalogue row; exports may copy an incompatible but intact row because
compatibility gates starting, not transport. The native CLI can export any
selected local slot to a newly created file.

## Recorded GM continuation

`src/headless/replay/recorded_gm.rs` provides the bounded #1316 native verifier
for two ordinary browser save exports. It uses shared boot and snapshot restore,
retains frozen crew and selected ratings, then appends the final journal's new
GM requests to the restored origin and derives their effects. Exact initial
digest, final tick/frontier, final digest and actual terminal outcomes must all
match. This path is distinct from the ordinary fresh-session load described
above: no old crew is cleared and no live network wait set is installed.

`tests/recorded_gm_exports.rs` owns the full native regressions and the ignored
file-driven entry point used by a browser evidence collector. The
[recording contract and JSON schema](../../docs/acceptance/1316-recorded-gm-replay.md)
describe its GM-only input scope and refusal boundaries. Ordinary exports omit
crew input/seat history, so a successful comparison requires separate collector
evidence of immutable crew and absence of ordinary human commands.

## Related

- [World Data](../entities/world-data.md) — the authoritative snapshot payload
- [Native Host](./native-host.md) — `FileStore` and operator CLI integration
- [Game Loop](./game-loop.md) — fixed logical-tick ordering
