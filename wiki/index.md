# Wiki Index

Catalog of every page in the Project Phoenix wiki. Each entry: link · one-line summary.

See [SCHEMA.md](./SCHEMA.md) for conventions and [log.md](./log.md) for the change history.

## Start here

- **[Project Overview](./concepts/project-overview.md)** — What Project Phoenix is in one page.
- **[Architecture](./concepts/architecture.md)** — Star topology, two HTML pages, one WASM binary.
- **[Game Loop](./concepts/game-loop.md)** — Lobby → In-Progress, message flow, tick rates.

## Entities

People, things, and game objects.

- **[Player](./entities/player.md)** — A connected human, identified by session token.
- **[Session](./entities/session.md)** — Server-side player record, survives reconnects.
- **[Console](./entities/console.md)** — A role on the bridge (one seat each).
- **[Captain Console](./entities/captain-console.md)** — Red Alert + View selector. Game-start authority.
- **[Helm Console](./entities/helm-console.md)** — Thrust + steering. The only console that moves the ship. Radar overlay.
- **[Console](./entities/console.md)** — All four consoles (CaptainChair, Helm, Tactical, Engineering) and how to add more.
- **[Ship](./entities/ship.md)** — The player-controlled vessel. Capsule collider, XZ plane, Y-up.
- **[Asteroid](./entities/asteroid.md)** — Static obstacle in the field. Sphere collider.
- **[World Data](./entities/world-data.md)** — Fixed asteroid layout for a session. Deterministic.
- **[Bridge Crew Stations (planned)](./entities/bridge-crew-stations-planned.md)** — Weapons, Engineering, Science, Comms.

## Concepts

Architecture, patterns, processes.

- **[Project Overview](./concepts/project-overview.md)** — One-page elevator pitch.
- **[Architecture](./concepts/architecture.md)** — Layers, processes, where state lives.
- **[Networking](./concepts/networking.md)** — PeerJS, WebRTC, star topology, session tokens.
- **[Message Flow](./concepts/message-flow.md)** — Phone → JS → WASM → Bevy → JS → broadcast.
- **[Codec Seam](./concepts/codec-seam.md)** — Why `serde_json` lives in exactly one module.
- **[Game Phases](./concepts/game-phases.md)** — Lobby vs In-Progress; transition rules.
- **[Game Loop](./concepts/game-loop.md)** — Tick rates, helm input @10 Hz, sim broadcast @10 Hz.
- **[Ship Physics](./concepts/ship-physics.md)** — Pure controller, accel/decel curves, yaw model.
- **[Asteroid Field](./concepts/asteroid-field.md)** — Seeded generator, clear zone, deterministic layout.
- **[Radar Projection](./concepts/radar-projection.md)** — Shared pure iterator, server + helm reuse.
- **[View Modes](./concepts/view-modes.md)** — Camera (Fore/Aft/Port/Starboard) vs Radar.
- **[UiMaterial Shader Pattern](./concepts/ui-materials.md)** — Custom WGSL fragment shaders behind UI nodes (Red Alert vignette as worked example).
- **[View-Model Pattern](./concepts/view-model-pattern.md)** — Pure derived snapshots for renderers.
- **[Console Plugin Pattern](./concepts/console-plugin-pattern.md)** — One Bevy plugin per console.
- **[WorldPlugin](./concepts/world-plugin.md)** — Owns world bootstrap (starfield + player ship). Landing zone for the World/Scenario merger (#218).
- **[CaptainPlugin](./concepts/captain-plugin.md)** — First extracted console plugin: red alert toggle + view selector. Validates the simulation-split pattern (#227).
- **[ShipPlugin](./concepts/ship-plugin.md)** — Second simulation split: helm physics, impulse drive. Extracted from `simulation.rs` (#239).
- **[WeaponsPlugin](./concepts/weapons-plugin.md)** — Third simulation split: phasers, torpedoes, beam handling. Extracted from `simulation.rs` (#245).
- **[RepairPlugin](./concepts/repair-plugin.md)** — Fourth simulation split: breakdown queue, three-team dispatch, repair-icon broadcast. Extracted from `simulation.rs` (#250).
- **[PowerPlugin](./concepts/power-plugin.md)** — Fifth simulation split: 6+2 power allocation, battery exhaustion lock, recharge threshold, `PowerState` broadcaster. Extracted from `simulation.rs` (#254).
- **[SciencePlugin](./concepts/science-plugin.md)** — Sixth simulation split: `SetScienceTarget` advisory hand-off from Sensors to Tactical. Extracted from `simulation.rs` (#258).
- **[ShipView](./concepts/ship-view.md)** — Client-side `Resource` holding shared ship state (pose, red alert, view mode, power levels, hull fraction, impulse charge). Updated by `ShipViewPlugin`; read by all console panels instead of `ClientSimState` (#234).
- **[CaptainPanel](./concepts/captain-panel.md)** — Client-side plugin for the captain console UI: compass dial, direction pad, red alert toggle, panel visibility logic. Lives in `phone_border/captain.rs`; extracted from `client/app.rs` (#240).
- **[HelmPanel](./concepts/helm-panel.md)** — Client-side plugin for the helm console UI: compass-ring radar, polished thumbstick, 10 Hz resend, On Screen button, gizmo radar overlay. Lives in `src/helm_panel.rs`; extracted from `client/app.rs` and `phone_border/helm.rs` (#246).
- **[WeaponsPanel](./concepts/weapons-panel.md)** — Client-side plugin for the Tactical console UI: phaser fire/mode, torpedo tube selection + fire, gizmo radar overlay, panel visibility. Lives in `src/weapons_panel.rs`; extracted from `client/app.rs` (#251).
- **[Broadcaster Seam](./concepts/broadcaster-seam.md)** — `SimBroadcaster` + `LobbyBroadcaster` registration API; Audience, Cadence, producer-registration recipe, full message catalogue with file:line references, `OutboundMessage` write contract, and cross-links to PRDs #117/#118/#120/#153/#154/#180/#187.
- **[Modifier Coordination](./concepts/modifier-coordination.md)** — Single owner of `ShipModifiers`; complete catalogue of three modifier sources (power, regions, impulse) with translator recipe, read-interface guide, and per-UUID source identity.
- **[Build & Deployment](./concepts/build-and-deployment.md)** — Trunk, two HTML entry points, GitHub Pages.
- **[Testing Strategy](./concepts/testing-strategy.md)** — `cargo test` + Playwright smoke tests.

## Sources

Faithful summaries of external artifacts. One page per source.

### Product Requirements (GitHub issues)

Shipped:

- **[PRD #1 — Browser-Based Bridge Simulator](./sources/prd-001-bridge-simulator.md)** — Closed. The PoC: lobby, captain, red alert, rotating cube.
- **[PRD #17 — Mobile UX, Canvas Resize, Connection Status](./sources/prd-017-mobile-ux-and-status.md)** — Closed. Fullscreen, top-right status bar, Bevy node UI.
- **[PRD #22 — Helm and Game World](./sources/prd-022-helm-and-game-world.md)** — Closed. Helm console, ship physics, asteroid field, collisions.
- **[PRD #36 — Captain View Selector](./sources/prd-036-captain-view-selector.md)** — Closed. Fore/aft/port/starboard hull cameras.
- **[PRD #51 — Smoke Test Harness](./sources/prd-051-smoke-test-harness.md)** — Closed. Playwright + BroadcastChannel PeerJS shim.
- **[PRD #66 — Weapons & Engineering Consoles](./sources/prd-066-weapons-and-engineering.md)** — Shipped. Tactical (phasers, lock), Engineering (repair loop), hull integrity, breakdown queue.
- **[PRD #115 — Native PC Server](./sources/prd-115-native-pc-server.md)** — PRD itself closed; deployment slices #135–#141 are still on hold and not built.
- **[PRD #117 — Modifier System](./sources/prd-117-modifier-system.md)** — Shipped. Pure `modifiers.rs` cache + `ModifierAdded`/`ModifierRemoved` wire.
- **[PRD #118 — Repair + Power Consoles](./sources/prd-118-repair-and-power-consoles.md)** — Shipped. `Engineering` → `Repair`; new `Power` console; shape-matching repair with three teams; 6+2 power allocation.
- **[PRD #120 — Station-Based Lobby](./sources/prd-120-station-based-lobby.md)** — Shipped. Per-station picking, auto-shuffle, spectator FIFO. `SelectStation` / `ReleaseStation` / `StationAssigned` wire.
- **[PRD #153 — Region Entities, Component-Driven Spawning & Modifier Flags](./sources/prd-153-region-entities-and-entity-pipeline.md)** — Shipped. Single `[[entity]]` pipeline; six region effects; `f32` hull; `FlagKind`; unified `EntitySnapshot`.
- **[PRD #154 — Console Complexity: UI Hiding + AI Automation](./sources/prd-154-console-complexity.md)** — Shipped. Per-console `Low`/`Full` presets; hide UI + server-side `console_ai` to operate hidden controls.
- **[PRD #180 — Viewscreen Frame](./sources/prd-180-viewscreen-frame.md)** — Shipped. Bevy UI border, `RedAlertVignetteMaterial`, designation + HEADING / HULL / CONDITION HUD.
- **[PRD #187 — Phone Console HUD — Diegetic Bezel Frame](./sources/prd-187-phone-bezel.md)** — Shipped. `phone_border/` plugin: bezel wraps every console; full helm + captain chrome.
- **[PRD #191 — Grid-Based Asteroid Lifecycle](./sources/prd-191-grid-asteroid-lifecycle.md)** — Shipped. `asteroid_window.rs`, player-centred ring buffer, destroyed asteroids respawn on return.

Open (planned work):

- **[PRD #116 — Save/Load Game Sessions](./sources/prd-116-save-load-sessions.md)** — `localStorage` save slots; `save.rs` is the second sanctioned `serde_json` surface.
- **[PRD #119 — Stations, Scenarios & Comms](./sources/prd-119-stations-scenarios-comms.md)** — TOML scenario engine, station entities, `Console::Comms`. Builds on PRD #153's entity pipeline.
- **[PRD #142 — AI and Behaviour System](./sources/prd-142-ai-and-behaviour.md)** — Data-driven state-machine NPCs that emit the same input messages as players. Depends on #119.

### Design drafts (`docs/`)

- **[Draft 1 — Entity Config Files](./sources/design-01-entity-config-files.md)** — Asteroid + Ship as data-driven entities.
- **[Draft 2 — Game Map](./sources/design-02-game-map.md)** — Solar systems, planets, asteroid fields by file.
- **[Draft 3 — Science Console](./sources/design-03-science-console.md)** — Long-range radar, impulse, system chart.
- **[Draft 4 — Combat Update](./sources/design-04-combat-update.md)** — Phaser banks, torpedoes, four-quadrant shields.
- **[Draft 5 — Ship's Power](./sources/design-05-ships-power.md)** — Engineering 6-point distribution, aux battery.
- **[Draft 6 — Space Stations](./sources/design-06-space-stations.md)** — *Stub. Consolidated into PRD #119.*
- **[Draft 7 — Scenario File](./sources/design-07-scenario-file.md)** — *Stub. Consolidated into PRD #119.*
- **[Draft 8 — Comms Console](./sources/design-08-comms-console.md)** — *Stub. Consolidated into PRD #119.*
- **[Draft 9 — AI and Behaviour](./sources/design-09-ai-and-behaviour.md)** — State-machine NPCs.
- **[Draft 10 — Region Entities](./sources/design-10-region-entities.md)** — Invisible trigger volumes (radar dampening, damage zones, impulse blockers).
- **[Draft 11 — Console Complexity](./sources/design-11-console-complexity.md)** — Per-console Low / Full complexity toggle.
- **[Architecture Improvement Notes](./sources/notes-architecture-improvements.md)** — Per-console message subscriptions.

### Project documents

- **[README.md](./sources/repo-readme.md)** — User-facing overview.
- **[AGENTS.md](./sources/repo-agents.md)** — Agent operating manual.
- **[CONTEXT.md](./sources/repo-context.md)** — Domain vocabulary.

## Roadmap

Synthesis of where the project is going.

- **[Roadmap Overview](./roadmap/overview.md)** — Shipped vs in-flight vs drafted.
- **[Console Expansion](./roadmap/console-expansion.md)** — Path from 2 consoles to 6.
- **[Combat & Damage](./roadmap/combat-and-damage.md)** — Hull, shields, phasers, torpedoes.
- **[Data-Driven Content](./roadmap/data-driven-content.md)** — Entity files, scenarios, system maps.
- **[Open Architectural Questions](./roadmap/open-architectural-questions.md)** — Per-console messaging, scenarios.
