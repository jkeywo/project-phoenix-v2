---
title: Server App Composition
type: concept
tags: [architecture, plugins, server, composition, fixed-tick]
sources: [src/server_app/mod.rs, src/server_app/registration.rs, src/server_app/components.rs, src/server_app/broadcast.rs, src/server_app/collision.rs, src/server_app/broadcast_publish.rs, src/server_app/world_setup.rs, src/server_app_render.rs, src/core/broadcast/lifecycle.rs, src/console/repair/visibility.rs, src/console/weapons/blackboard.rs, src/console/weapons/server.rs, src/lobby/server.rs, src/ship/power.rs, src/ship/shields.rs, src/ship/sensors.rs, src/server/bridge.rs, src/headless/app.rs]
updated: 2026-08-28
---

# Server App Composition

`crate::server_app` is the server-side Bevy composition facade. The browser host and headless runner both call `add_simulation_plugins` (or `add_simulation_plugins_with`) rather than owning parallel simulation registrations.

## Module seams

| File | Current responsibility |
|---|---|
| `src/server_app/mod.rs` | Thin facade and re-exports. It keeps existing `crate::server_app::…` imports stable while implementation lives in focused sibling modules. |
| `src/server_app/registration.rs` | The composition root: `SimPluginOptions`, fixed-tick setup, Rapier ordering, plugin registration, resources, systems, and broadcaster registration. |
| `src/server_app/components.rs` | Cross-cutting ECS components, resources, and `SystemParam` bundles owned by the simulation assembly, including the explicitly classified `SimOutbox`. |
| `src/server_app/broadcast.rs` | Authoritative simulation snapshot builders, including `sim_state_broadcaster`, its entity position/health delta caches, lifecycle reset, and explicit UUID pruning. |
| `src/server_app/broadcast_publish.rs` | Publish/HUD systems, Blackboard live/lifecycle projection, `modifier_events_broadcaster`, the class-preserving `sim_outbox_broadcaster`, and world-setup publication. |
| `src/server_app/collision.rs` | Rapier contact handling and collision damage. |
| `src/server_app/world_setup.rs` | Static world setup and game-start entity spawning. |
| `src/server_app_render.rs` | Render-only entity materialisation and mesh LOD updates, registered only when `SimPluginOptions::render` is true. |

## Registration contract

`add_simulation_plugins` uses the default `SimPluginOptions`. `add_simulation_plugins_with` is the test/headless seam: it can omit render-coupled systems, move Rapier registration to prove explicit schedule edges, and deterministically shuffle top-level plugin registration to expose accidental order dependencies.

The simulation runs in `FixedUpdate` on the authored logical tick. `SimSet` remains the authoritative chain:

```text
Input → Physics → Damage → Modifiers → Publish → PublishAggregate → Broadcast
```

Rapier's backend sync runs after `SimSet::Physics`; its writeback completes before `SimSet::Damage`. Collision outcomes therefore depend on logical ticks, not rendered frames or plugin insertion order.

The registration root installs the console, ship, AI, world-support, infrastructure, campaign, civilian, tractor, dock, umbilical, delivery, and broadcast plugins. Each domain plugin still owns its own systems and authoritative state; `server_app` owns only their composition and genuinely cross-domain seams.

## Broadcast path

The registration root installs:

- `weapons_update_broadcaster` from `src/console/weapons/blackboard.rs`;
- `sim_state_broadcaster` from `src/server_app/broadcast.rs`;
- `modifier_events_broadcaster` and `sim_outbox_broadcaster` from `src/server_app/broadcast_publish.rs`;
- `shields_state_broadcaster` from `src/ship/shields.rs`, registered by `ShipShieldsPlugin` and routed to the holder of the Station that owns the authored `shields` System kind, regardless of its instance id.

`SimOutbox` is the arbitrary-target simulation queue. Its raw entries are
private, so producers must choose `Snapshot` or `Reliable` through an explicit
insertion method. `sim_outbox_broadcaster` drains and forwards that stored class
without matching on the message variant. See [Broadcaster Seam](./broadcaster-seam.md).

The registration root calls owner registrars for Blackboard and `SimState`
entity replication. `RepairPlugin`, `ShipShieldsPlugin`, and `WeaponsPlugin`
register their own adapters beside their publishers. The resulting stable keys
are `blackboards`, `entity-state`, `hull`, `shields`, and `weapons`; the generic
runners invoke them lexically without knowing cache resources or message
shapes. Position/health are reset-only and retain explicit UUID pruning;
Shields is cache-free and reconnects only the authored System holder;
Blackboard, Hull, and Weapons reconnect projections do not mutate their live
delta caches.

## Tests

Module-level composition tests live in `src/server_app/mod_tests.rs`. Cross-binary and schedule-order properties live in integration tests, including the headless runner and registration-order determinism checks. Render code stays outside those headless paths behind `SimPluginOptions::render`.

## Related

- [Game Loop](./game-loop.md)
- [Message Flow](./message-flow.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [WorldPlugin](./world-plugin.md)
