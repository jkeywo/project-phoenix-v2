---
title: Navigation Console
type: entity
tags: [console, navigation, waypoint, map, radar, ship]
sources: [gui/battleship/navigation.html, gui/cruiser/comms.html, gui/destroyer/tactical.html, gui/components/ph-navigation-map.js, gui/console-state.js, gui/sim-state.js, gui/action-map.js, src/console/navigation/mod.rs, src/core/messages.rs, src/entities/config.rs, assets/entities/alliance_battleship.toml]
updated: 2026-07-15
---

# Navigation Console

Navigation provides the shared strategic chart and desired-destination surface. The Battleship has a dedicated Navigation console, while the Cruiser embeds Navigation in Comms and the Destroyer embeds it in Tactical. All three consume the same Navigation state and shared `ph-navigation-map` component; there is no Rust client panel.

## Chart contents

The ship entity's `[navigation_console.system_chart]` block supplies the chart range plus the `shows` and `selects` tag filters. `NavigationConsoleConfig` parses that data, the Navigation blackboard publishes it, and `buildNavigationConsoleState` turns the authoritative snapshot into the browser state used by every hull layout.

The chart is overhead, north-up, and world-anchored. It uses authored radar icons and colours, plots the ship at its world position, and renders the current shared waypoint. Pan and zoom are local presentation state.

## Selection and waypoint placement

Tapping a chart blip selects it locally, opens its information overlay, and sends `set_navigation_waypoint` with the blip's coordinates and `source_uuid`. This creates an anchored waypoint. Tapping empty chart space clears the local selection and sends the same action without `source_uuid`, creating a free waypoint at that world position. The Battleship page also exposes an explicit Clear Waypoint button.

The server owns one waypoint per ship:

```rust
pub enum WaypointMode {
    Free { x: f32, z: f32 },
    Anchored { source_uuid: String, last_x: f32, last_z: f32 },
}

pub struct NavigationWaypoint(pub Option<WaypointMode>);
```

An anchored waypoint follows the source entity's live transform. If the source despawns, `refresh_anchored_waypoint` clears it. A free waypoint remains fixed. Non-finite coordinates are rejected.

## Authority and sharing

Navigation commands enter through the same `ControlSystem` command path as other fine-system controls and are admitted for the Navigation system. The Navigation blackboard publishes the waypoint for all consumers. Helm and other consoles can display it, and AI Helm should treat it as the normal shared desired destination whether a human or Navigation AI wrote it.

`NavigateTo` is the level-3 clearance that releases the AI Helm to follow the waypoint: it carries the waypoint's `generation` (not a position), serves the ship's `coordination_lag_secs` in the coordination queue, and latches into `HelmWaypointClearance` on delivery. It is issued from one origin-agnostic place — `issue_navigate_to_clearance` in `src/console/navigation/mod.rs` — once per new waypoint generation, and re-issued when the helm axes flip Human→AI while the current generation is unlatched (so a waypoint set under a human helm is still flown after a disconnect/Backfill flip). Neither waypoint writer sends its own clearance.

## Wire surface

- `SystemControlPayload::SetNavigationWaypoint { x, z, source_uuid }` sets a free or anchored waypoint.
- `SystemControlPayload::ClearNavigationWaypoint` clears it.
- `NavigationBlackboard.navigation_waypoint: Option<WaypointSnapshot>` publishes the current shared value.

`gui/action-map.js` maps the browser actions onto these authoritative commands. `buildWaypointBlip` also projects the shared waypoint onto Helm and other radar views, edge-clamping it where the consuming view requests that behaviour.
