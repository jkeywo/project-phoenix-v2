---
title: GM Operator
type: entity
tags: [gm, operator, identity, reconnect, roster, readiness, force-start, action, pause, host-mesh]
sources: [src/gm_roster.rs, src/gm_action.rs, src/gm_projection.rs, src/boot/mod.rs, src/lobby/start_policy.rs, src/core/messages.rs, src/core/codec.rs, src/lobby/server.rs, src/lockstep/frame.rs, src/lockstep/mod.rs, src/server/bridge.rs, src/snapshot.rs, src/sim_digest.rs, src/headless/replay.rs, gui/gm-local-projection.js, gui/gm-session-actions.js, gui/gm-session-controls.js, gui/host-mesh.js, gui/fleet-session.js, gui/lobby-state.js, server.html, client.html]
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
or local ship. Its first tracer reads the lexicographically lowest stable
`EntityUuid` with authoritative hull status and renders that absolute value on
the page's local `gm_entity` Host Channel. Despawn or world reset sends an
explicit null projection so stale identity cannot remain. The projection is
not a `ServerMessage`, `MeshFrame`, or `SimOutbox`; peer state transfer remains
the snapshot-recovery path.

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
