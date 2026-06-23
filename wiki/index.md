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
- **[Navigation Console](./entities/navigation-console.md)** — System chart at long range; sets the shared navigation waypoint (free or entity-anchored).
- **[Console](./entities/console.md)** — All four consoles (CaptainChair, Helm, Tactical, Engineering) and how to add more.
- **[Ship](./entities/ship.md)** — The player-controlled vessel. Capsule collider, XZ plane, Y-up.
- **[Asteroid](./entities/asteroid.md)** — Static obstacle in the field. Sphere collider.
- **[World Data](./entities/world-data.md)** — Fixed asteroid layout for a session. Deterministic.
- **[Bridge Crew Stations (planned)](./entities/bridge-crew-stations-planned.md)** — Weapons, Engineering, Science, Comms.
- **[Editor](./entities/editor.md)** — In-browser TOML authoring tool (Scenario / Entity / Definitions modes) over the File System Access API. Vitest-tested deep modules. Not part of the game runtime.

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
- **[RepairPanel](./concepts/repair-panel.md)** — Client-side plugin for the Repair console UI: breakdown label, shape-match buttons, three team status rows, repair icon label, panel visibility. Lives in `src/repair_panel.rs`; extracted from `client/app.rs` (#255).
- **[PowerPanel](./concepts/power-panel.md)** — Client-side plugin for the Power console UI: per-console power allocation rows, battery bar, lock state, overflow controls, panel visibility. Lives in `src/power_panel.rs`; extracted from `client/app.rs` (#259).
- **[SciencePanel](./concepts/science-panel.md)** — Client-side plugin for the Science (Sensors) console UI: view-mode selector (Radar / System Chart), On Screen button, Cancel Impulse button, `ScienceView` resource, panel visibility. Lives in `src/science_panel.rs` (#262).
- **[CommsPanel](./concepts/comms-panel.md)** — Comms console inbox/chat model, including thread grouping, contact/channel labels, and multi-speaker dialogue via TOML `speaker`.
- **[Comms range](./concepts/comms-range.md)** — Per-entity `[comms].range` opt-in; `CommsRange` Component; pure `comms::in_range` helper; `update_comms_range_flags` server system stamps `in_range` / `sender_in_range`; client hides out-of-range contacts and greys response buttons; server enforces Hail/Respond gate.
- **[Server HTML Lobby UI](./concepts/server-lobby-ui.md)** — HTML lobby overlay in `server.html`; `LobbyStateChanged` → `__updateLobby` push channel. Replaced the deleted Bevy `LobbyScreenRoot` tree (#436). Auto-fit grid + portrait reflow.
- **[Client Architecture](./concepts/client-architecture.md)** — Full panel plugin inventory, `add_client_plugins` composition entry point, shared client resources, `ClientSimState` audit notes, client-split series history (#263).
- **[Broadcaster Seam](./concepts/broadcaster-seam.md)** — `SimBroadcaster` + `LobbyBroadcaster` registration API; Audience, Cadence, producer-registration recipe, full message catalogue with file:line references, `OutboundMessage` write contract, and cross-links to PRDs #117/#118/#120/#153/#154/#180/#187.
- **[Modifier Coordination](./concepts/modifier-coordination.md)** — Single owner of `ShipModifiers`; complete catalogue of three modifier sources (power, regions, impulse) with translator recipe, read-interface guide, and per-UUID source identity.
- **[Build & Deployment](./concepts/build-and-deployment.md)** — Trunk, two HTML entry points, GitHub Pages.
- **[Asset Preload](./concepts/asset-preload.md)** — Server-side discovery + pre-cache of GLBs, radar icons, model-rig sidecars, sub-world TOMLs. Sidecar inbox is a single-consumer queue (renderer takes; preload only peeks) — see the 2026-06-17 race-fix.
- **[Coarse-system migration](./concepts/coarse-system-migration.md)** — Naming convention, migration status table for all 9 coarse systems, fine-system forward reference.
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
- **[PRD #187 — Phone Console HUD — Diegetic Bezel Frame](./sources/prd-187-phone-console-hud.md)** — Shipped. `phone_border/` plugin: bezel wraps every console; full helm + captain chrome. *Being superseded by PRD #438.*
- **[PRD #191 — Grid-Based Asteroid Lifecycle](./sources/prd-191-grid-asteroid-lifecycle.md)** — Shipped. `asteroid_window.rs`, player-centred ring buffer, destroyed asteroids respawn on return.

Open (planned work):

- **[PRD #116 — Save/Load Game Sessions](./sources/prd-116-save-load-sessions.md)** — `localStorage` save slots; `save.rs` is the second sanctioned `serde_json` surface.
- **[PRD #119 — Stations, Scenarios & Comms](./sources/prd-119-stations-scenarios-comms.md)** — TOML scenario engine, station entities, `Console::Comms`. Builds on PRD #153's entity pipeline.
- **[PRD #142 — AI and Behaviour System](./sources/prd-142-ai-and-behaviour.md)** — Data-driven state-machine NPCs that emit the same input messages as players. Depends on #119.
- **[PRD #350 — Scenario Editor Rewrite](./sources/prd-350-scenario-editor-rewrite.md)** — Three-mode in-browser TOML editor (World / Entity / Definitions) over the FSA. Adds `extra_worlds` + `load_world`/`unload_world` trigger actions. Slices 1–6 = v1.
- **[PRD #438 — HTML/JS Client GUI Shell](./sources/prd-438-html-client-gui-shell.md)** — Shipped. Bevy lobby + tab bar + phone bezel replaced with HTML/CSS/JS. Slices: #439 (bezel), #440 (lobby), #441 (tab bar), #442 (Bevy cleanup).
- **[PRD #487 — Station / Console / System architecture redesign](./sources/prd-487-station-console-system-redesign.md)** — Open. Replaces monolithic console ownership with fixed Stations, cohesive Consoles, fine-grained Systems, AI backfill, per-system damage, power groups, and `StationId`/`SystemId` addressing.
- **[Issue #488 — Station/System ADR](./sources/issue-488-station-system-adr.md)** — Open. Foundational contract slice for the future ship-config schema, stable station/system ids, per-station ratings, power groups, and additive typed control wire scaffold.
- **[Issue #489 — Ship config loader + verifier](./sources/issue-489-ship-config-loader.md)** — Open. Pure `src/ship/config.rs` loader for the future `[[station]]`/`[[system]]` schema, rating tables, power groups, and load-time validation.
- **[Issue #490 — System registry + Red Alert system](./sources/issue-490-system-registry-red-alert.md)** — Open. Pure system-kind registry with mandatory AI controller registration, plus `ControlSystem` routing and AUTO/read-only HTML rendering for Red Alert.
- **[Issue #493 — Coordination-lag scope](./sources/issue-493-coordination-lag-scope.md)** — Open. Decision slice: channel-3 lag applies to all bus traffic; target control resolves at delivery time.
- **[PRD #517 — Consistency cleanup for the 9 coarse systems](./sources/prd-517-consistency-cleanup.md)** — Open. Eight slices closing inconsistencies from the coarse-system conversion PRs; Repair + Navigation conversions; hardcoded console list fix; `serde_json` cleanup; `SystemId` naming pin.
- **[Issue #523 — Console ID lookup](./sources/issue-523-console-id-lookup.md)** — Open. PRD #517 slice A4: `Console::from_console_id` helper symmetric with `station_console_id`; replaces hardcoded array in `process_coordination_lag`.
- **[Issue #524 — serde_json outside codec cleanup](./sources/issue-524-serde-json-cleanup.md)** — Shipped. PRD #517 slice A5: removed direct `serde_json` calls from `coordination.rs`, `flag_kind.rs`, `effects.rs`; moved `RegionEffectKind` round-trips into `codec.rs`.
- **[Issue #525 — SystemId naming convention](./sources/issue-525-systemid-naming.md)** — Shipped. PRD #517 slice A6: module-level doc + pinning tests in `system_registry.rs`; `REPAIR_SYSTEM_ID`/`repair_system_id()` added; `wiki/concepts/coarse-system-migration.md` created.
- **[Issue #526 — Repair coarse-system conversion](./sources/issue-526-repair-control-system.md)** — Shipped. PRD #517 slice A7: `repair` registered in `with_core_systems`; `handle_dispatch_repair_team` accepts `ControlSystem` dispatch with `policy_for` gating; 5 new tests.
- **[Issue #527 — Navigation coarse-system conversion](./sources/issue-527-navigation-control-system.md)** — Shipped. PRD #517 slice A8: `handle_navigation_waypoint` accepts `ControlSystem` waypoints with `policy_for` gating; `ui_action_to_client_message` emits `ControlSystem`; 6 new tests + 3 codec tests. 9/9 consoles converted.
- **[Issue #528 — Shields advisories through CoordinationEnqueue](./sources/issue-528-shields-coordination.md)** — Shipped. PRD #517 slice A1: removed direct `CoordinationPopup` push from `shields.rs`; shield advisories now flow via `CoordinationEnqueue` → `process_coordination_lag`.
- **[Issue #439 — HTML Phone Bezel Frame](./sources/issue-439-html-phone-bezel.md)** — Shipped. First slice of #438; `gui/phone-bezel.js` + DOM bezel in `client.html`; SimState reads `snap.red_alert`.
- **[Issue #440 — Lobby Integration + Phase Toggle](./sources/issue-440-html-lobby-phase-toggle.md)** — Shipped. Lobby merged into `client.html` as `#lobby-ui`; `gui/phase-toggle.js` pure function drives section visibility (treats `GameOver` as in-game).
- **[Issue #441 — Tab Bar + Content Switching](./sources/issue-441-html-tab-bar-content-switching.md)** — Shipped. `gui/tab-bar.js` + `gui/content-switcher.js` pure modules; `#console-tab-bar` strip in the bezel safe zone (portrait top / landscape left, initials at 5+ in portrait); `setActiveConsole()` consolidates the three call sites.
- **[Issue #442 — Bevy Cleanup (lobby + tab bar + bezel)](./sources/issue-442-bevy-cleanup.md)** — Shipped. Final slice of #438; deletes the Bevy lobby UI, embedded tab bar widget, and phone bezel frame from `src/client/{app,console_shell,phone_border/framing}.rs` (~2700 → ~900 lines). `ConsoleShell::spawn` signature preserved so the nine per-console panels compile unchanged; `PhoneAssets` + `DeviceOrientation` retained.

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
- **[Refactor 2026-05 — Entity Schema](./sources/refactor-2026-05-entity-schema.md)** — Four-slice consolidation: deleted Star/Planet/Station/ScienceConsole configs; added `TransformConfig`, `AmbientLightConfig`, `[[light]]`, `[mesh].emissive`, `EntityConfig.name`.
- **[Architecture Improvement Notes](./sources/notes-architecture-improvements.md)** — Per-console message subscriptions.

### Project documents

- **[README.md](./sources/repo-readme.md)** — User-facing overview.
- **[AGENTS.md](./sources/repo-agents.md)** — Agent operating manual.
- **[CONTEXT.md](./sources/repo-context.md)** — Domain vocabulary.
- **[player_ship.toml](./sources/player_ship_toml.md)** — Player ship config (hull, weapons, banks, tubes, stations, …).

## Roadmap

Synthesis of where the project is going.

- **[Roadmap Overview](./roadmap/overview.md)** — Shipped vs in-flight vs drafted.
- **[Polish Audit](./roadmap/polish-audit.md)** — Missing quality-of-life, presentation, audio, and juice work for the current game.
- **[Console Expansion](./roadmap/console-expansion.md)** — Path from 2 consoles to 6.
- **[Combat & Damage](./roadmap/combat-and-damage.md)** — Hull, shields, phasers, torpedoes.
- **[Data-Driven Content](./roadmap/data-driven-content.md)** — Entity files, scenarios, system maps.
- **[Open Architectural Questions](./roadmap/open-architectural-questions.md)** — Per-console messaging, scenarios.
