---
title: Radar Projection
type: concept
tags: [radar, helm, navigation, viewscreen, pure-iterator, shared]
sources: [gui/console-state.js, gui/radar-widget.js, gui/navigation-console.html, gui/sim-state.js, client.html, src/radar.rs, CONTEXT.md]
updated: 2026-06-17
---

# Radar Projection

A single pure iterator that turns 3D asteroid positions into 2D radar dots, ship-relative.

## API

```rust
pub fn radar_dots(
    asteroids: &[AsteroidInfo],
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
) -> impl Iterator<Item = (f32, f32, f32)>  // (radar_x, radar_y, scaled_radius)
```

Lives in `src/shared/radar.rs` so both server and client can use it.

## Two consumers

1. **Helm console (client):** the helm UI renders an overhead mini-radar of nearby asteroids.
2. **Server viewscreen Radar mode:** when `ViewMode == Radar`, the viewscreen renders the same projection full-screen.

Same input → same output → same visual semantics. This was an explicit deepening (commit `f3ef92c`) — before that, server and helm had two separate projection implementations that drifted.

## Why an iterator

- Caller decides how to consume (filter by range, take top-N, collect).
- No allocation in the hot path.
- Easy to test: collect into a `Vec` and assert.

## HTML console waypoint blips

The HTML console path uses `gui/console-state.js::buildBlips()` and `buildWaypointBlip()` before passing pre-projected blips into `gui/radar-widget.js`. Navigation owns one shared custom waypoint via server `SimSnapshot.navigation_waypoint`; Helm projects it ship-relative and clamps it to radius `0.96` with `edge: true` when it is outside Helm radar range. The radar widget renders `kind: "waypoint"` as a cyan diamond/ring without a bitmap asset.

`gui/navigation-console.html` draws only the live chart background, grid, server-derived blips, waypoint, and own-ship marker. It does not carry hardcoded sector polygons; scenario/region overlays should be data-driven rather than baked into the Navigation background.

`gui/console-state.js::buildRadarRegions()` is the HTML path for shaped map overlays. It emits `region`, `asteroid_field`, and objective-marker entities as sphere/box/torus payloads for Navigation and Sensors. Region entities normally carry `shape` from the server snapshot; asteroid-field entities may be normalised from `radius` + `inner_radius` into a torus/sphere overlay so fields remain visible on the Navigation map even when they are not point blips.

The Sensors page carries a demo animation for standalone mockup viewing, but it must be cancelled on the first live console state push. If that interval keeps running, it can call `RadarWidget.update()` with an empty `regions` array and make only asteroid-field/region overlays flicker while blips remain stable.

Entity TOML `[radar_appearance].icon` flows into `EntitySnapshot.radar_icon` when the server builds `WorldResource` entries for `WorldSetup` and reconnects. `EntitySnapshot.radar_icon` is authoritative for HTML blip kind as well as bitmap icon selection. If `assets/entities/star_sun.toml` says `icon = "star"`, `buildBlips()` and the Navigation chart both treat it as a star blip; tag classification is only the fallback.

## HTML objective markers

`ObjectiveSummary` is shared mission state, so it is broadcast to all clients and mirrored into `gui/sim-state.js::ClientSimState.objectives`. The HTML radar builders mark active objective targets by matching each objective `targets` entry against entity `name`, `id`, or `uuid`.

`gui/console-state.js::buildBlips()` lets active objective targets through even when their tag is normally hidden, which is how invisible `objective_marker` beacon entities appear only when referenced by an active objective. Point objectives render as normal blips with `objective_target: true`; shaped objective targets (`shape = "sphere" | "box" | "torus"`) also emit region overlays via `buildRadarRegions()`. Sensors draws those overlays in the shared pre-projected `RadarWidget`, while Navigation draws the same region payload on its custom full-screen map.

## Navigation's two-stage filter (and the `client.html` mirror gotcha)

`buildNavigationConsoleState` (`gui/console-state.js:521`) is the odd one out among the per-console builders: it runs **two** filters in series.

1. **Outer filter** at `console-state.js:528-532` walks every entity and keeps it only if it is either an active objective target *or* one of its tags appears in `navChartShows`.
2. **Inner filter** is the standard `buildBlips()` range/`shows` filter, applied to whatever the outer filter let through.

This means an empty `navChartShows` is **not** a no-op for Navigation — it makes the outer filter drop every non-objective entity, leaving the chart blank. Tactical and Sensors only have the inner filter, where an empty `shows` falls through and shows everything.

`navChartShows` / `navChartSelects` / `navChartRange` live on `ShipClientConfig` (`src/core/messages.rs:428-456`), sourced from `[navigation_console.system_chart]` in `assets/entities/player_ship.toml`. `gui/sim-state.js::apply('Welcome')` (lines 163-165) stores them on `window.simState`. **The inline `Welcome` handler in `client.html` must then mirror them onto the plain `state` object** passed into `buildNavigationConsoleState` — if any one of those three lines is missing, Navigation silently breaks. The bug-bait regression test in `tests/client/console-state.test.js` (`blips arrive when state is built via the client.html Welcome mirror path`) walks this exact path so the next field omission fails CI.

## Future filters

PRD #66 mentions extending `radar.rs` for **range-based** and **type-based** filtering — Weapons console wants 60-unit targeting range; Science (Draft 3) wants only stars/planets, no asteroids. The current iterator can be wrapped in standard `.filter()` calls; explicit filter combinators may follow.

## Related

- [Helm Console](../entities/helm-console.md) · [View Modes](./view-modes.md)
- [PRD #66](../sources/prd-066-weapons-and-engineering.md)
