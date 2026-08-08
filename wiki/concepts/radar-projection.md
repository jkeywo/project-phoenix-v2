---
title: Radar Projection
type: concept
tags: [radar, helm, navigation, viewscreen, pure-iterator, shared]
sources: [gui/console-state.js, gui/radar-widget.js, gui/battleship/navigation.html, gui/sim-state.js, gui/components/ph-tactical-radar.js, client.html, src/radar.rs, src/console/weapons/blackboard.rs, CONTEXT.md]
updated: 2026-08-08
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

Lives in `src/radar.rs` so both server and client can use it.

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

`gui/battleship/navigation.html` draws only the live chart background, grid, server-derived blips, waypoint, and own-ship marker. It does not carry hardcoded sector polygons; scenario/region overlays should be data-driven rather than baked into the Navigation background.

`gui/console-state.js::buildRadarRegions()` is the HTML path for shaped map overlays. It emits `region`, `asteroid_field`, and objective-marker entities as sphere/box/torus payloads for Navigation and Sensors. Region entities normally carry `shape` from the server snapshot; asteroid-field entities may be normalised from `radius` + `inner_radius` into a torus/sphere overlay so fields remain visible on the Navigation map even when they are not point blips.

The Sensors page carries a demo animation for standalone mockup viewing, but it must be cancelled on the first live console state push. If that interval keeps running, it can call `RadarWidget.update()` with an empty `regions` array and make only asteroid-field/region overlays flicker while blips remain stable.

Entity TOML `[radar_appearance].icon` flows into `EntitySnapshot.radar_icon` when the server builds `WorldResource` entries for `WorldSetup` and reconnects. `EntitySnapshot.radar_icon` is authoritative for HTML blip kind as well as bitmap icon selection. If `assets/entities/star_sun.toml` says `icon = "star"`, `buildBlips()` and the Navigation chart both treat it as a star blip; tag classification is only the fallback.

## HTML objective markers

`ObjectiveSummary` is shared mission state, so it is broadcast to all clients and mirrored into `gui/sim-state.js::ClientSimState.objectives`. The HTML radar builders mark active objective targets by matching each objective `targets` entry against entity `name`, `id`, or `uuid`.

`gui/console-state.js::buildBlips()` lets active objective targets through even when their tag is normally hidden, which is how invisible `objective_marker` beacon entities appear only when referenced by an active objective. Point objectives render as normal blips with `objective_target: true`; shaped objective targets (`shape = "sphere" | "box" | "torus"`) also emit region overlays via `buildRadarRegions()`. Sensors draws those overlays in the shared pre-projected `RadarWidget`, while Navigation draws the same region payload on its custom full-screen map.

## Navigation's two-stage filter

`buildNavigationConsoleState` (`gui/console-state.js:1414`) is the odd one out among the per-console builders: it runs **two** filters in series.

1. **Outer filter** (`console-state.js:1425-1429`) walks every entity and keeps it only if it is either an active objective target *or* one of its tags appears in `navChartShows`.
2. **Inner filter** is the standard `buildBlips()` range/`shows` filter, applied to whatever the outer filter let through.

This means an empty `navChartShows` is **not** a no-op for Navigation — it makes the outer filter drop every non-objective entity, leaving the chart blank. Tactical and Sensors only have the inner filter, where an empty `shows` falls through and shows everything.

`navChartShows` / `navChartSelects` / `navChartRange` live on `ShipClientConfig` (`src/core/messages.rs:934-962`), sourced from `[navigation_console.system_chart]` in the per-hull ship TOML (e.g. `assets/entities/alliance_battleship.toml`). Since the client-shell rework (#819–#823) there is a **single** client store: `gui/sim-state.js` (`window.simState`), applied from `Welcome` and each `SimSnapshot`. The per-console builders read straight off that store — there is no longer a separate `client.html` `state` mirror to keep in sync (and no `blips arrive when state is built via the client.html Welcome mirror path` regression test guarding it; that path was removed). `gui/dirty-consoles.js` tracks which consoles changed each tick and `gui/client-router.js` drives the fan-out, calling `window.buildConsoleState(consoleName, simState)` per dirty console.

Tactical radar blips follow the same single-store path: the server publishes them into the ship's `tactical-radar` blackboard (`TacticalRadarBlackboard.blips`, #829), `simState` carries the blackboards, and `buildWeaponsConsoleState` (`gui/console-state.js:597`) reads `state.blackboards['tactical-radar'].blips` as the authoritative source (falling back to a local `buildBlips()` projection only when the blackboard carries none). It **copies** that array rather than aliasing it: the science marker and waypoint are appended to the result, and the store's array is replaced only when a `BlackboardUpdate` arrives — which the server's `LastBroadcastBlackboards` delta cache withholds while nothing changes — so aliasing would grow the store's array on every build.

## Torpedo-armed contacts (#957)

`RadarBlip.torpedo_armed` is the host's answer to "can that bird put a torpedo on
us", sent so the Tactical console can badge a torpedo boat **before** the first
torpedo is in flight. `publish_tactical_radar_blackboard` sets it from the
contact's own live `TorpedoSystemResource` (tubes non-empty) AND
`crate::faction::is_enemy` against the observing ship — the same faction
predicate the helm hostile-arc overlay (#874) uses, so a friendly torpedo boat
and the player's own ship never badge themselves. The client never re-derives it
from a hull name, icon or model.

It is **capability, not readiness**: `true` with every tube unloaded and the
magazine empty, and it does not promise a launch — the world-scoped torpedo
conservation gate (#943) can still refuse one from a fully-armed hull. Nothing
scan-gates it: the only gates are the ones the blip already passed (the radar's
`shows` tag filter and its effective-range cull). The precedent is on the blip
itself — `RadarBlip.threat_level`, `.description` and `.target_tags` are authored
hostile intel shipped through those same two gates and no third.

It is also **not red-alert gated**, unlike its nearest wire sibling
`HelmBlackboard.hostile_weapon_arcs` (#874), which is populated for the local
ship only and only at red alert. That gate is right for arcs and wrong here: arcs
are live per-contact firing geometry carried as an extra channel on the helm
blackboard, so withholding them until the shooting starts costs the crew nothing,
whereas `torpedo_armed` is one static bit about a contact the tactical radar has
already drawn, sitting next to three ungated intel fields on the same blip — and
knowing a torpedo boat is out there is precisely what a crew needs *before* red
alert. Do not assume the arc overlay's gate came along with the precedent.

The wire carries the boolean only. `foldTorpedoBadges` in `gui/console-state.js`
resolves `console.radar.torpedo_armed` into `RadarBlip.torpedo_badge`, and
`gui/components/ph-tactical-radar.js` draws it as an SVG label in the
`#torpedo-badges` overlay group. The copy is the client's, so it never crosses
the `localiseTree` ingress boundary that resolves *server-sent* ids — and
spelling the id out in a `t()` call is what keeps `check-strings.mjs` able to
see it.

## Future filters

PRD #66 mentions extending `radar.rs` for **range-based** and **type-based** filtering — Weapons console wants 60-unit targeting range; Science (Draft 3) wants only stars/planets, no asteroids. The current iterator can be wrapped in standard `.filter()` calls; explicit filter combinators may follow.

## Related

- [Helm Console](../entities/helm-console.md) · [View Modes](./view-modes.md)
- PRD #66
