---
title: PASM Feature Capture Backlog
type: concept
tags: [pasm, audit, architecture, game-design, backlog]
sources: [pasm/spec, src/, gui/, assets/, wiki/concepts/pasm-runtime.md]
updated: 2026-07-16
---

# PASM Feature Capture Backlog

This is the ordered inventory of Phoenix features that PASM should eventually
describe. It distinguishes existing PASM slices from the missing contracts they
still need, so an already-named feature is not mistaken for fully captured
behaviour.

Status legend: **Captured** has a current PASM slice; **Expand** has one but
needs the listed semantics; **New** has no focused PASM slice yet.

## 1. Station, system, and authority model — Captured

PASM now captures the foundational contract: ship-authored
stations and fine systems, stable ids, system availability/damage, station
ratings, `Human`/`Ai`/`Offline` control sources, command admission, Backfill,
and the human/AI symmetric-control rule. This is the common authority layer
for every console and NPC ship.

Sources: `src/ship/config.rs`, `src/ship/system_registry.rs`,
`src/ship/control_source.rs`, `src/ship/rating.rs`, `src/server_app.rs`.

## 2. Sessions, multiplayer, and replication — Captured

PASM now captures session-token identity, PeerJS connection lifecycle, reconnect and
station reclaim, lobby/in-game command routing, late-join reconstruction,
audience selection, broadcaster cadence, snapshot versus reliable messages,
and the server-authority boundary.

Sources: `src/lobby/session.rs`, `src/lobby/handler.rs`,
`src/server/bridge.rs`, `src/core/broadcast/`, `gui/connection-manager.js`.

## 3. Shared ship state, blackboards, and coordination — Captured

PASM now captures the simulation-set ordering, system blackboards, frozen previous-tick
reads, viewscreen aggregation, channel-2 inter-system messages, channel-3
coordination lag, delivery-time target resolution, player coordination
messages, and AI chatter. This is distinct from player-facing Comms.

Sources: `src/sim_sets.rs`, `src/ship/coordination.rs`,
`src/core/messages.rs`, `src/ship_plugin.rs`.

## 4. Game flow and ship lifecycle — Captured

PASM now captures the requested pre-scenario selection → lobby → loading →
in-progress → game-over → selection return flow, including the immediate QR,
first-writer-wins scenario/ship selection from host or client, ready/countdown,
and the legacy host-preloaded-world confirmation signal. PASM now also records
game-start spawning, entity destruction/despawn, and the route from player or
authored terminal outcomes into GameOver. Explicit selection commands remain
proposed.

Current coverage: Worlds records authored content lifecycle; Repair and
Weapons cover parts of damage and destruction. Neither captures the complete
game-state transition and entity-lifecycle contract.

Sources: `src/lobby/server.rs`, `src/server_app.rs`,
`src/console/weapons/mod.rs`, `src/regions/server.rs`,
`gui/lobby-state.js`.

## 5. Ship configuration, selection, and entity files — Captured

PASM now captures entity-file loading and validation, player ship selection,
station and
system layouts, system-generated configuration such as shield arcs, hull and
repair tuning, power groups, mesh/model-rig markers, faction/behaviour links,
and entity overrides. PASM now also records schema validation, capability-based
template composition, and the complete template-section to runtime-component
contract. The Editor slice below remains the authoring surface.

Sources: `src/entities/config.rs`, `src/entities/loader.rs`,
`src/entities/spawner.rs`, `src/ship/config.rs`, `assets/entities/`.

## 6. Objectives — Captured

**Captured.** Current PASM covers declaration, scoring, AI directives, Captain
boost, player projection, authoring validation, and world lifecycle.

PASM now additionally captures concrete `Destroy`, `Patrol`, `Reach`, and
`Hail` directives, doctrine score gating, target-name resolution, and outcome
paths. PASM now also records the observed player/AI projection divergence and
complete authored Comms consequence route. Later semantic review can test those
policies, but their current runtime contracts are captured.

Sources: `src/objectives.rs`, `src/ai/core.rs`, `src/ship/viewscreen.rs`.

## 7. NPC AI, doctrine, factions, and ships — Captured

**Captured.** PASM now records faction relationships and configurations, NPC
controller provisioning, doctrine evaluation, combat memory, per-system AI
operators, and the intended same-command path for human, NPC, and Backfill
control. It preserves the current direct-write Helm exception and the stub
operator gaps as implementation limitations rather than claiming symmetry that
the runtime does not yet provide.

Current content includes Federation, Harrow, Pirate, and Requiem factions, and
Alliance, Harrow, Pirate, Requiem, Dynasty, station, and outpost templates.

Sources: `src/ai/core.rs`, `src/ai/server.rs`, `src/ai/faction.rs`,
`assets/factions/`, `assets/entities/`.

## 8. Helm and spatial movement — Captured

**Captured.** PASM covers player inputs, impulse, boost, hazards, desired
motion, planned 3D movement, vertical modes, and the Helm migration.

The slice now also records the 10 Hz human input cadence and current planar
physics/collision damage. The difference between shipped planar physics and
the proposed 3D fine-actuator design remains explicit.

Sources: `src/console/helm/server.rs`, `src/ship_plugin.rs`,
`src/ship/physics.rs`, `src/asteroids/`.

## 9. Repair and damage — Captured

**Captured.** PASM records the intended information and request/dispatch model
for repair teams.

The slice now also records the shipped per-system hull and damage-availability
model, damage-source routing, repair-team travel/repair/return cycle, and
player-versus-NPC destruction outcomes. Owner repair requests and on-site
priorities remain explicitly proposed.

Sources: `src/ship/damage.rs`, `src/modifiers/repair_teams.rs`,
`src/console/repair/server.rs`.

## 10. Power, modifiers, flags, and regions — Captured

**Captured.** PASM records Power Reactor and Battery authority, allocation,
battery lock/recharge, source-aware modifier and flag aggregation, and region
containment/effect translation. The region effects include damage, slow,
impulse blocking, radar dampening, Comms jamming, sensor blindness, and fog.

This is one feature family because power, impulse, and regions all alter ship
behaviour through the same modifier/flag contract.

Sources: `src/ship/power.rs`, `src/modifiers/`, `src/regions/`,
`src/core/messages.rs` (`FlagKind`).

## 11. Weapons — Captured

**Captured.** Phaser, blaster, torpedo, readiness/range/arc feedback, and the
accepted future multi-barrel/3D work are in PASM.

The slice now also records fine-system ownership, the torpedo magazine
round-claim protocol, and Tactical AI's target/doctrine/arc-bearing behavior.
The accepted family-aware arc-bearing design remains planned work.

Sources: `src/console/weapons/mod.rs`, `src/weapons/`,
`src/ship/system_registry.rs`.

## 12. Shields — Captured

**Captured.** PASM records authored shield arcs, their fine-system health and
regeneration, focus commands, damage routing, blackboard publication, and the
current Shields AI seam. Shields remain a distinct player authority and
damage-absorption system rather than a Weapons subfeature.

Sources: `src/ship/shields.rs`, `src/weapons/shield.rs`,
`gui/components/ph-shield-panel.js`.

## 13. Radar, Sensors, targeting, and identification — Captured

**Captured.** PASM records the shared radar projection, system-specific
presentation filters, selected Sensors target, Sensors-to-Tactical target and
frequency advice, damage-scaled sensor range, and the current Sensors AI stub.
Red Alert's selected-target display remains an extension of the Red Alert
slice.

Sources: `src/radar.rs`, `src/radar_config.rs`, `src/ship/sensors.rs`,
`src/server/radar.rs`, `gui/radar-widget.js`.

## 14. Navigation — Captured

**Captured.** PASM records free and entity-anchored waypoints, auto-clear on
target despawn, Navigation chart projection, Navigation AI objective selection,
and Navigation-to-Helm coordination. Explicit Navigation cancellation of
impulse remains future design work.

Sources: `src/console/navigation/mod.rs`, `gui/components/ph-navigation-map.js`.

## 15. Comms — Captured

**Captured.** PASM records the shared ship Comms inbox, contacts and dialogues,
hailing/responding/clearing, range and jamming enforcement, sender endpoint
identity versus display speaker, objective projection, and the current Comms
AI stub. All comms still arrives through the ship Comms system rather than
role-specific visibility rules.

Sources: `src/console/comms/`, `src/comms/`, `gui/comms-state.js`.

## 16. Viewscreen, cameras, and debug information — Captured

**Captured.** PASM records Captain and cinematic cameras, system-driven radar,
navigation, and Comms views, host rendering/HUD effects, and host-local debug
overlay toggles. Red Alert itself remains captured separately.

Sources: `src/ship/viewscreen.rs`, `src/server/renderer.rs`,
`src/server/viewscreen_border.rs`, `src/debug_overlay.rs`.

## 17. Worlds, triggers, authored scenarios, and composition — Captured

**Captured.** PASM covers world parsing, entities, names/display names, flags,
triggers, objectives, Comms, validation, lifecycle, and desired composition.

The slice now also records the concrete trigger action catalogue, immediate and
game-start spawning, runtime anchor resolution and aliases, generated entity
cleanup, additive world layers, and load/unload cleanup. Atomic authoring
failure and cross-world name collision detection remain required proposed
behavior.

Sources: `src/world/config.rs`, `src/world/server.rs`, `src/world/dispatch.rs`, `assets/worlds/`.

## 18. Terrain, space entities, and streaming — Captured

**Captured.** PASM records the common authored entity pipeline for stars,
planets, stations/outposts, beacons, docks and regions, plus deterministic
asteroid-window streaming and fresh respawn after a cell is revisited. Terrain
effects link to the modifier-and-region slice rather than duplicate its rules.

Sources: `src/asteroids/`, `src/entities/`, `assets/entities/`.

## 19. Specific worlds and content packs — Captured

**Captured.** PASM records `default`, `combat_test`, and `before_the_fire` as
selectable root content, and `patrol`, `reinforcements`, `btf_path_a`,
`btf_path_b`, `btf_path_c`, and `btf_aphelion_protocol` as additive loadable
content. Composition validation remains owned by the generic World contract.

Sources: `assets/worlds/`.

## 20. Editor — Captured

**Captured.** PASM records local World, Entity, Definitions, and model/marker
authoring surfaces; File System Access boundaries; non-blocking validation;
draft state; and the lack of live runtime reload.

Sources: `gui/editor/` and `wiki/entities/editor.md`.

## 21. Client shell, settings, help, and accessibility — Captured

**Captured and extended.** PASM records the pure-JS console registry, state
projections, station-derived access, active tabs, responsive shell, connection
state, action mapping, local settings, and the current static help surface. It
also records the desired replacement: ship-authored station overviews plus
system-generated, vessel-specific manual sections aggregated into tabs for all
stations. The client remains non-authoritative.

Sources: `gui/console-registry.js`, `gui/action-map.js`,
`gui/settings-panel.js`, `gui/help-panel.js`, `gui/`.

## 22. Presentation, effects, audio, and loading — Captured

**Captured.** PASM records asset manifest discovery/preloading and loading
progress, model rigs and markers, host effects/rendering, and client-local
audio. Existing hardcoded presentation constants are preserved as observed
technical debt rather than misrepresented as configuration-owned values.

Sources: `src/server/pfx.rs`, `src/server/asset_preload.rs`,
`src/server/renderer.rs`, `src/entities/model_rig.rs`, `assets/`.

## 23. Red Alert — Captured

**Captured.** PASM records the desired explicit set-state command,
selected-Sensors-target status, and AI-ship capability provisioner.

The slice now also records the current toggle command, per-ship Captain AI and
combat-activity trigger, Captain blackboard/visual/audio presentation, and the
toggle-to-set migration plan.

Sources: `src/console/captain/server.rs`, `src/server/viewscreen_border.rs`,
`src/ship/combat_activity.rs`.

## Suggested delivery order

1. Station/system authority, sessions/replication, and blackboards/coordination.
2. Game flow, ship/entity configuration, and NPC AI/doctrine.
3. Expand Repair, Helm, Objectives, Weapons, Red Alert, and Worlds against the
   current runtime.
4. Power/modifiers/regions, Shields, Radar/Sensors, Navigation, and Comms.
5. Terrain/streaming, specific content packs, viewscreen/debug, and editor.
6. Client shell plus presentation/audio/loading.

This order makes later player-facing slices describe a single shared authority,
information, and lifecycle model rather than each redefining it.
