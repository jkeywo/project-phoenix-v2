---
title: Navigation Console
type: entity
tags: [console, navigation, waypoint, map, radar, ship]
sources: [gui/battleship/navigation.html, gui/cruiser/comms.html, gui/courier/captain.html, gui/components/ph-navigation-map.js, gui/components/ph-civilian-traffic.js, gui/stations/navigation-actions.js, gui/stations/navigation-action-control.js, gui/semantic-action-registry.js, gui/gamepad-input.js, gui/console-state.js, gui/sim-state.js, gui/action-map.js, gui/hero-bar.js, src/command_admission/mod.rs, src/console/navigation/mod.rs, src/console/navigation/server.rs, src/console/helm/server.rs, src/civilian/traffic.rs, src/civilian/server.rs, src/core/messages.rs, src/entities/config.rs, src/ship/coordination_systems.rs, assets/entities/alliance_battleship.toml, assets/entities/alliance_cruiser.toml, assets/entities/alliance_courier.toml, assets/entities/alliance_destroyer.toml]
updated: 2026-08-31
---

# Navigation Console

Navigation provides the shared strategic chart and desired-destination surface.
The selected hull authors both the Navigation station and its console URL. The
Battleship exposes a directly claimable Navigation seat; the Cruiser and
Destroyer author Navigation as auxiliary, human-seeking stations presented on
an eligible holder's Hero Bar, while the Courier embeds Navigation under its
Captain console. Their layouts consume the same authoritative Navigation state
and shared `ph-navigation-map` component; there is no Rust client panel.

## Chart contents

The ship entity's `[navigation_console.system_chart]` block supplies the chart range plus the `shows` and `selects` tag filters. `NavigationConsoleConfig` parses that data, the Navigation blackboard publishes it, and `buildNavigationConsoleState` turns the authoritative snapshot into the browser state used by every hull layout.

The chart is overhead, north-up, and world-anchored. It draws region hulls — sphere, torus, and box, matching the viewscreen radar's shapes — beneath the blips, then uses authored radar icons and colours to plot the ship, waypoint, and contacts at their world positions. An `objective_target` region or blip gets a gold ring or outline instead of its own colour, so mission geometry reads apart from ordinary landmarks. Moon-tagged contacts ride the same blip path as any other point contact, with no chart glyph of their own. Pan and zoom are local presentation state.

## Selection and waypoint placement

The product registry names twelve Navigation actions: chart, free waypoint
place, selected-contact anchor, clear waypoint, contact selection, civilian
order, four map-pan directions, and zoom in/out.
Each has two context-scoped binding slots. The parent gamepad runtime routes by
the actual claimed Station iframe, so composite hosts retain their real
`captain` or `comms` context. Horizontal pan uses the left stick, vertical pan
uses the paired triggers, and zoom uses the right stick's vertical axis. The
chart/civilian controls shipped only by the direct Navigation station use the
left stick's vertical pair in that non-overlapping context. Those defaults
remain distinct from the concurrent Captain, Comms, Power, Repair and
Sensors/Science catalogue. Select, Start and the two stick buttons carry the
remaining waypoint/contact operations. A composite adapter will act only on a
focused, open or visible map, so a hidden Courier overlay cannot consume those
controls. The direct Navigation console may use its only map without that
overlay test.

Tapping a chart blip selects it locally and opens its information overlay;
tapping empty chart space clears that selection. Selection alone never changes
the shared waypoint. Every host of `ph-navigation-map` renders the component's
explicit waypoint controls: **Set Waypoint** arms a pick mode whose next map tap
creates a free waypoint, **Set as Waypoint** creates an anchored waypoint for
the selected blip, and **Clear Waypoint** removes the shared waypoint. Pick mode
also exposes a visible keyboard cursor: arrows move it, while Shift+arrows expose
and move the same cursor outside pick mode,
and Enter or Space submits the same placement operation as a pointer tap. The
component emits `navselect` whenever its local selection changes so a host may
mirror that state in its own readout. Pointer, native control, keyboard and
gamepad inputs all enter the same semantic adapter. Pan, zoom and contact
selection settle immediately through the shared local-feedback lifecycle;
waypoint and civilian operations wait for authoritative terminal feedback.

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

Navigation commands enter through the same `ControlSystem` command path as other fine-system controls and are admitted for the Navigation system. Semantic waypoint submissions use its correlated form without changing the payload or command-log identity. The owning Navigation consumer returns `Applied` only after it has set or cleared the waypoint and `Refused` for a non-finite placement. The browser may display Pending but never paints a submitted waypoint optimistically; the next Navigation blackboard remains the only state update. The Navigation blackboard publishes the waypoint for all consumers. Helm and other consoles display it, and AI Helm consumes it as the shared desired destination whether a human or Navigation AI wrote it.

`NavigateTo` is the level-3 clearance that releases the AI Helm to follow the waypoint: it carries the waypoint's `generation` (not a position), serves the ship's `coordination_lag_secs` in the generic queue/router, and Helm's `receive_helm_coordination` latches that exact generation into `HelmWaypointClearance` only for a live AI `helm-steering` recipient. It is issued from one origin-agnostic place — `issue_navigate_to_clearance` in `src/console/navigation/server.rs` — once per new waypoint generation, and re-issued when the helm axes flip Human→AI while the current generation is unlatched (so a waypoint set under a human helm is still flown after a disconnect/Backfill flip). Neither waypoint writer sends its own clearance.

## Civilian traffic

The console also carries a traffic picture: every civilian craft in the world,
with the authored `[[route]]` it is flying, which leg of how many, its standing
order, and whether it is doing as it was asked. `<ph-civilian-traffic>` renders
that authoritative projection where the hull's Navigation layout includes it.

Compliance is published, never inferred client-side, because "has not started turning yet" and "has decided not to" look identical on a chart. `refused` and `non_compliant` are different words and different row styles for the same reason: a refusal is a decision (the craft declined and carried on down its own lane), non-compliance is a failure (it agreed, set off, and the world moved). Only the second needs a crew.

Orders are issued as `SystemControlPayload::OrderCivilian`, admitted for the same Navigation system the waypoint payloads are, and answered on the craft's own authored clock rather than immediately — an order is a negotiation with an actor, not a remote-control input. The owning civilian adapter returns correlated `Applied` once a valid order has entered that compliance machine, or `Refused` when the target or order cannot be delivered. An order the host cannot deliver at all (unknown craft, malformed divert) also retains the domain-specific `ServerMessage::CivilianOrderRejected`; a craft that later says no does not turn the accepted input into a command refusal, it changes compliance state. The mechanism lives in `src/civilian/`.

The human control surface is finite and authored. A civilian's optional `[civilian].order_options` list gives each control a stable id, a `strings.csv` label and one existing `hold`, `divert` or `dock` order. The host validates those options with the entity config, copies them into `CivilianTrafficSnapshot`, and `<ph-civilian-traffic>` renders them as native buttons. A button submits `order_civilian` with the published target and order; the browser does not choose destinations or carry scenario policy. Civilians without options have a read-only row.

## Wire surface

- `SystemControlPayload::SetNavigationWaypoint { x, z, source_uuid }` sets a free or anchored waypoint.
- `SystemControlPayload::ClearNavigationWaypoint` clears it.
- `SystemControlPayload::OrderCivilian { target, order }` orders a civilian to hold, divert or dock.
- `ServerMessage::CivilianOrderRejected { target, reason }` is the rejection-only reply for an order that could not be delivered.
- `ServerMessage::ActionFeedback { correlation, result, response_token }` settles a correlated waypoint or civilian-order input at its actual owning consumer.
- `NavigationBlackboard.navigation_waypoint: Option<WaypointSnapshot>` publishes the current shared value.
- `NavigationBlackboard.civilians: Vec<CivilianTrafficSnapshot>` publishes the traffic picture and each craft's finite `order_options`, on the local ship only.

`gui/action-map.js` maps the browser actions onto these authoritative commands. `buildWaypointBlip` also projects the shared waypoint onto Helm and other radar views, edge-clamping it where the consuming view requests that behaviour.
