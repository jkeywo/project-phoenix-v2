---
title: GM Operator
type: entity
tags: [gm, operator, identity, reconnect, roster, readiness, force-start, action, pause, host-mesh, map, activity, damage, destruction, regions, asteroids]
sources: [src/gm_roster.rs, src/gm_action.rs, src/gm_join.rs, src/gm_projection.rs, src/gm_activity.rs, src/entities/config.rs, src/entities/tags.rs, src/boot/mod.rs, src/lobby/start_policy.rs, src/core/balance.rs, src/core/messages.rs, src/core/codec.rs, src/command_admission/log.rs, src/lobby/server.rs, src/lockstep/frame.rs, src/lockstep/host_loss.rs, src/lockstep/mod.rs, src/lockstep/snapshot_relay.rs, src/server/bridge.rs, src/snapshot.rs, src/sim_digest.rs, src/headless/replay.rs, gui/host-channel.js, gui/gm-local-projection.js, gui/gm-activity-feed.js, gui/entity-inspector.js, gui/components/ph-navigation-map.js, gui/gm-session-actions.js, gui/gm-session-controls.js, gui/host-mesh.js, gui/fleet-session.js, gui/lobby-state.js, server.html, client.html]
updated: 2026-09-01
---

# GM Operator

A GM Operator is a privileged host-class participant admitted only through the
privileged fleet-host path. It is deliberately not a [Player](./player.md),
Spectator, Station holder, or player ship. Several GMs may be present and all
have the same role; the host mesh's technical star centre and private mesh slot
do not create a public leader or permission tier.

## Identity and roster

The fleet owner mints two separate identities for a GM:

- a stable public operator id such as `gm-1`, projected with only `name`,
  `connected`, and `ready`; and
- a private opaque reconnect capability retained by that operator's server
  page and presented on a later privileged reconnect.

Rendezvous peer ids and technical mesh slots are private transport details.
Neither they nor the reconnect capability enter the public roster. While the
Lobby topology is still mutable, a known disconnected GM can bind a new
connection to the same operator row without consuming a ship slot or displacing
a connected operator. Once frozen, that technical slot is part of Rust's
deterministic wait-set. A departed GM is then refused `recovery-only` even with
the correct capability until #1294 can restore the authoritative snapshot,
watermark, and any one-shot start boundary it missed.

## First-time mid-session admission

An unknown GM joining a running fleet first receives only a private candidate
connection and reserved identity. Every existing host-class page displays the
request, and any admitted host or GM may accept or reject it; the star centre
delivers that decision but has no product-level veto. Rejection before
acceptance changes neither the public roster nor authoritative pause state.

Acceptance asks Rust's technical owner to assign exactly one synchronized pause
boundary. Its provisional topology is a private `GmJoinBootstrap`, which gives
world setup the topology it needs without installing `FleetRoster` or
`FleetLockstep`. The candidate receives the canonical #1117 run privately: its
world snapshot, complete admitted-command history, and snapshotted
`GmActionJournal`. It reports the restored digest through the typed
`GmJoinFrame` lane. Only a matching digest commits the transaction, installs
the public GM roster and lockstep wait-set, and makes the connection a live
fleet peer. An authenticated owner-carried peer loss received after snapshot
capture remains in private, non-authoritative `GmJoinPendingHostLoss`; it cannot
alter digest proof or admit a wait-set. Commit drains it once into
`PendingHostLoss` and marks that peer departed in the newly installed wait-set
before Resume. Successful restore engages the candidate's technical join hold
even when its private world began behind the pause boundary. An accepted
disconnect or authenticated transfer/restore failure is terminally refused to
every peer; if the disconnect callback races an existing Commit, Rust preserves
and re-emits the same committed proof instead of losing the transaction.
A stalled not-ready restore exhausts an authenticated, owner-sequenced
`RestoreBoundary` protocol clock; duplicate, stale, and render-only updates are
inert, while a candidate lost after commit follows the ordinary host-loss path.

Once acceptance has scheduled it, `GmJoinPauseHold` keeps the session paused
after either terminal Rust outcome. It is a protocol hold, not a fabricated GM
action, and is released only when a newly applied, attributed
`SetSessionPaused { active: false }` follows that terminal result. Consequently
admission can never resume as a side effect of roster or transport completion.

## Authoritative projection

`GmRoster` is the Rust-side bounded, canonical full-replacement projection. The
browser fleet layer sends only public rows through `wasm_set_gm_roster`; Rust
validates unique ids, bounds names and row count, and broadcasts changes through
`Welcome`, `GmRosterChanged`, and the host lobby channel. Host and crew lobby
views render GMs in their own labelled presence group, outside Station,
Spectator, and crew-capacity calculations. Disconnect and reconnect retain the
operator id but clear readiness.

## Lobby readiness and force-start

Connected GM Operators participate in the same collective start policy as
connected non-Spectator crew. Every connected participant must be ready for an
automatic start, including a GM-only roster. Any connected GM may request a
force-start that bypasses readiness but never content, compatibility, preload,
or peer validation. The mesh authenticates the requesting connection and emits
one attributed, idempotent proposal. The proposal carries no apply tick. Rust on
the technical owner assigns the safe exact tick and puts the grant in its
authenticated `TickFrame`; every member adopts that canonical value. The
star-centre host does not gain a leader permission.

The explicit `?gm=1` page selects the production
`BootProfile::BrowserGameMaster` before `wasm_init`, independently of
WebDriver. It keeps the browser window, world ingest, fixed-tick simulation,
and fleet participant while installing no renderer and no `SelectedShipResource`
or local ship. The local `gm_entity` Host Channel carries an absolute,
UUID-sorted projection of facilitation-relevant local truth. Ships remain
player or NPC contacts, while `StaticPointDefence` is a structure. The typed
canonical `structure` tag also projects fixed authored structures that carry
neither Station nor infrastructure components. Structures, hazards, inert
Regions, asteroid fields, and selectable authored
asteroids use semantic kinds rather than exposing ECS components. Region,
hazard, and field footprints reuse the authored `RegionShape`; an asteroid
field is one aggregate Region, while lifecycle-streamed `Asteroid` rocks never
become individual contacts. A named/selectable asteroid authored in world
content is the only asteroid point contact. Each row contains stable
`EntityUuid`, position, String Table display ids, broad hull/infrastructure
condition, current Tactical target where applicable, and narrowed authored
radar appearance. It carries no Bevy entity id, component inventory, layer
identity, effect tuning, System detail, or M2 action capability. The Host
Channel dispatcher exempts this strict DTO from recursive localisation, keeping
those identities and display ids raw until the map or inspector presents them.

`gui/gm-local-projection.js` validates and narrows that DTO, partitions point
contacts from geometry Regions, then adapts both to the shared
`ph-navigation-map` in local inspect mode. Selection is keyed only by stable
UUID. An absolute refresh updates the linked inspector without dropping
selection; removal clears both map selection and stale inspector detail, and a
later same-UUID reappearance does not resurrect selection. At overlap a point
wins over a Region, then stable UUID breaks ties. Touch and keyboard traverse
selectable Regions as well as contacts only in inspect mode; Navigation's
interaction contract is unchanged. Semantic marker/outline patterns, a
selected outline, a destroyed cross, text legend, and textual inspector status
make kind and condition independent of colour. The inspector hides its hull
meter for kinds without hull applicability. The projection is not a
`ServerMessage`, `MeshFrame`, or `SimOutbox`; peer state transfer remains the
snapshot-recovery path.

The sibling local `gm_activity` Host Channel is an absolute oldest-first view
of a bounded presentation ring. `GmActivityPlugin` reads only the existing
unconditional `BalanceEvent::DamageApplied` and `EntityDestroyed` facts after
their fixed-tick producers and before `SimTick` advances. Within one tick it
sorts damage before destruction, then victim UUID, optional source UUID, and
damage detail; equal rows remain separate occurrences. The authored
`global.gm_activity_history_depth` sets the capacity and defaults to 128.
Capacity changes, new rows, and Lobby reset publish a replacement; an unchanged
tick does not.

A presentation-only UUID-to-name directory is sampled before combat, then
pruned to identities that are live or referenced by the bounded history. Thus
final damage and destruction rows retain a despawned victim's display identity
without accumulating every UUID minted by asteroid streaming. Unknown and
ordinary `AsteroidUuid` identities fall back to their UUID and remain
unselectable. The browser validates the raw DTO, filters by category or exact
involved identity, and resolves only known String Table ids at the render site;
literal mod-authored names remain literal. Identity buttons call the same
stable-UUID map selection seam; each `gm_entity` replacement reconciles their
availability, so a removed or
racing target becomes a readable disabled button and cannot clear another
selection. This feed is likewise absent from `ServerMessage`, mesh frames,
snapshots, digests, and peer transport.

## Typed session actions

`SetSessionPaused { active }` is the first complete GM action. The page submits
an absolute value plus the public operator id and a bounded durable
`GmActionId`; it never calls the local/debug toggle. Rust binds that identity to
the authenticated GM slot in the frozen private `FleetRoster`. Any GM may send
a proposal, but only the frozen technical owner assigns its exact logical
boundary and canonical `(apply_tick, GmActionOrder { sequence, origin })` key.
The proposal's `from` remains the requester and a decision's `sequenced_by`
names the technical owner; neither field grants operator precedence. Exact
retransmission is inert and a new id that requests the current value records an
explicit No-op. JavaScript relays the opaque Proposal, Granted, and Refused
frames without interpreting the Rust-owned action protocol.

The complete bounded `GmActionJournal` is snapshotted as authoritative input
and the whole journal remains the operator-scoped idempotency record. Current
state and digest fold only `initial_paused`, the exact `applied_prefix()`, and
`SimulationPaused`; future grants received early are not current state.
`apply_due_actions` advances the durable `applied_grants` reducer frontier in
`PreUpdate`, so Resume remains consumable while Pause has starved `FixedUpdate`.
Its derived log supplies attributed Applied/No-op results; canonical or local
admission failures add Refused results without becoming successful history.
The existing lockstep gate still owns the combined virtual-time decision, so a
GM resume cannot release a peer, recovery, or model-readiness hold.

Replay installs a one-shot `ReplayGmSeed` after the ordinary run-start reset.
It loads only the source run's applied prefix into an empty live journal with a
zero frontier, then re-applies those recorded boundaries through the production
reducer; an unapplied suffix never extends replay beyond the artifact's final
tick. The GM page receives only an absolute local `gm_session` Host Channel
projection and does not optimistically change the displayed pause state.

## Related

- [Session](./session.md)
- [Player](./player.md)
- [Networking](../concepts/networking.md)
- [Server Lobby UI](../concepts/server-lobby-ui.md)
