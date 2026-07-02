---
title: Navigation Console
type: entity
tags: [console, navigation, waypoint, map, radar, ship]
sources: [gui/navigation-console.html, gui/console-state.js, gui/sim-state.js, gui/action-map.js, src/console/navigation/mod.rs, src/core/messages.rs, src/server_app.rs, assets/entities/player_ship.toml]
updated: 2026-07-02
---

# Navigation Console

The system-map console. Renders an overhead, north-up chart of the wider
neighbourhood (stations, planets, stars, regions, objective beacons) at a
much larger range than the Helm radar, and is the only seat that can set
the shared **navigation waypoint**.

Loaded as an iframe (`#navigation-iframe`) inside `client.html`; the entire
console is a single self-contained HTML file with an inline `<script
type="module">` block. There is no Rust client panel.

## Chart contents

Configured per-ship via `[navigation_console.system_chart]` in
`assets/entities/player_ship.toml` (parsed into `NavigationConsoleConfig`,
`src/entities/config.rs:937-949`):

- `range = 800` — world units of the chart radius.
- `shows` — entity tags rendered as blips (default: `region`,
  `asteroid_field`, `star`, `planet`, `station`, `player`,
  `objective_marker`).
- `selects` — entity tags that can be tapped to select (default: `station`,
  `planet`, `star`, `region`).

The blip pipeline runs through `buildNavigationConsoleState` in
`gui/console-state.js:784`. Asteroids and ships are intentionally
excluded from the system chart so it reads as a strategic overview.

The full-screen canvas renderer in `gui/navigation-console.html` uses the
same radar icon contract as the shared `RadarWidget`: each blip's authored
`icon` value is preferred over `kind`, assets resolve from
`../assets/radar_icons/Icon-*.png`, and authored RGB `color` values are
used for tint/fallback dots. Objective targets stay as gold rings layered
around the blip; they do not replace the authored radar icon.

## Selection

Tap any non-`own` blip to select it. `_selectedId` in
`gui/navigation-console.html:311` is the local UUID of the current
selection (matches the wire `EntitySnapshot.uuid`). The bottom-overlay
panel (`#entity-overlay`, `gui/navigation-console.html:229-267`) shows the
selected entity's name, stance/kind/faction badges, bearing and range
from the ship.

Tap empty space to clear the selection. Selection that disappears from
the next state push (entity left the chart range) auto-clears via
`gui/navigation-console.html:846-850`.

## Waypoint

The crew shares one navigation waypoint at a time, owned authoritatively
by the server. Storage variant (added 2026-06-19):

```rust
// src/console/navigation/mod.rs
pub enum WaypointMode {
    Free { x: f32, z: f32 },
    Anchored { source_uuid: String, last_x: f32, last_z: f32 },
}
pub struct NavigationWaypoint(pub Option<WaypointMode>);
```

Broadcast as `SimSnapshot.navigation_waypoint:
Option<WaypointSnapshot>` where `WaypointSnapshot { x, z, source_uuid:
Option<String> }`. `source_uuid` is omitted on the wire (and on the
client) for free waypoints; present for anchored ones.

Only the player holding `Console::Navigation` may set or clear the
waypoint (`navigation_authorized` in `src/console/navigation/mod.rs`).
NaN/Inf coordinates are rejected.

### Three ways to set a waypoint

1. **Tap-to-place (free)** — `#btn-set-waypoint` arms placement mode,
   the next canvas tap calls `set_navigation_waypoint { x, z }` (no
   `source_uuid`). Waypoint never moves on its own.
2. **Add Waypoint from selection (anchored)** — `#btn-add-waypoint`
   appears when any selectable entity is selected. Sends
   `set_navigation_waypoint { x, z, source_uuid: <selected uuid> }`.
   The server's `refresh_anchored_waypoint` system
   (`src/console/navigation/mod.rs`, runs in `SimSet::Modifiers`)
   queries `EntityUuid + Transform` every tick and overwrites
   `last_x`/`last_z` from the entity's live transform. The waypoint
   tracks the entity until despawn.
3. **Snap-to-objective (legacy)** — `#btn-set-waypoint` when the
   selected entity has `objective_target = true` skips placement mode
   and sends the entity's current coords as a *free* waypoint.

### Auto-clear on despawn

When an anchored waypoint's parent entity is no longer present
(despawned, or never spawned), `refresh_anchored_waypoint` sets
`NavigationWaypoint(None)`. The next broadcast omits
`navigation_waypoint`. Crew sees `WP NOT SET`.

### Bidirectional selection link

The waypoint blip emitted into the chart's `blips` array carries the
parent UUID as `source_uuid`. In the navigation iframe:

- `buildWaypointBlip` (`gui/console-state.js:218-258`) marks the blip
  `selectable: true` only when `source_uuid` is non-null.
- `handleTap` (`gui/navigation-console.html:657-694`) hit-tests the
  waypoint's world position separately; on a hit with non-null
  `source_uuid`, sets `_selectedId` to the parent UUID (looked up in
  `_entities`). This is how tapping the waypoint forwards selection to
  the parent.
- Free waypoints (no `source_uuid`) remain non-selectable; tapping them
  falls through to normal hit-testing.

The waypoint is also drawn on the Helm radar (`buildHelmConsoleState` →
`buildWaypointBlip` with `edgeClamp: true`), clamped to the radar edge
with `edge: true` when out of range so the helmsman can still see the
bearing.

## Wire surface

- `ClientMessage::SetNavigationWaypoint { x: f32, z: f32, source_uuid:
  Option<String> }` — sender must hold `Console::Navigation`. Adding a
  non-empty `source_uuid` switches into anchored mode.
- `ClientMessage::ClearNavigationWaypoint` — also navigation-gated.
- `SimSnapshot.navigation_waypoint: Option<WaypointSnapshot>` — present
  whenever a waypoint is set; absent (`skip_serializing_if =
  "Option::is_none"`) otherwise.

Round-trip coverage:
`src/console/navigation/mod.rs` `mod tests` (7 tests), `src/core/codec.rs`
(`client_set_navigation_waypoint{,_with_source_uuid,_legacy_payload_deserialises}`,
`sim_snapshot_with_{,anchored_}navigation_waypoint_round_trips`),
`tests/client/action-map.test.js`, `tests/client/console-state.test.js`,
`tests/smoke/navigation-console.spec.ts`.

## Cancel-impulse button

The navigation chart shows a `CANCEL IMPULSE` button while the impulse
drive is in `Charging` or `Active`. Emits `ClientMessage::CancelImpulse`.
Behaviour mirrored on Helm and Sensors — see
[Helm Console](./helm-console.md).
