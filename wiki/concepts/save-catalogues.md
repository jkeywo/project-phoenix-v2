---
title: Peer-Local Save Catalogues
type: concept
tags: [save, snapshot, persistence, autosave, browser, native, catalogue]
sources: [src/save_slots.rs, src/save_slots_lifecycle.rs, src/save_slots_store.rs, src/snapshot.rs, src/server/bridge.rs, src/server_app/world_setup.rs, src/lockstep/mod.rs, src/ship/coordination_systems.rs, src/bin/phoenix_host.rs, src/delivery/args.rs, src/entities/config.rs, src/world/config.rs, gui/save-slots.js, gui/browser-save-identity.js, gui/browser-save-identity-worker.js, server.html, tests/save_slots_persistence.rs]
updated: 2026-08-30
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

Current snapshot format 15 also requires a `BootIdentity`: the selected hull,
the frozen `FleetRoster`, and the authored-order UUID identity of every entity
that actually spawned at `GameStart`. Startup validates the scenario and hull,
checks the saved fleet against any already-staged fleet, and verifies that each
saved authored index still names a `GameStart` row before those UUIDs are used
to build the fresh world. Restore then proceeds through the ordinary snapshot
roster/layer readiness and digest checks.

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

## Portable transfer

Export and import do not define another format. `snapshot::TransferStore` moves
the same RON `StoredRun` text through the same save/load functions and version
gate. Browser import is a pre-activation action that loads the artifact's named
scenario and then applies the same hull and `GameStart` identity checks as a
catalogue row; exports may copy an incompatible but intact row because
compatibility gates starting, not transport. The native CLI can export any
selected local slot to a newly created file.

## Related

- [World Data](../entities/world-data.md) — the authoritative snapshot payload
- [Native Host](./native-host.md) — `FileStore` and operator CLI integration
- [Game Loop](./game-loop.md) — fixed logical-tick ordering
