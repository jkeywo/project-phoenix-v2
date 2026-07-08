# Project Phoenix — Domain Vocabulary

Use these terms consistently across code, comments, PRs, and architecture discussions.

---

## Game Domain

**Console** - a ship operator surface. Currently shipped: `CaptainChair`, `Helm`, `Tactical`, `Repair`, `Sensors`, `Shields`, `Navigation`, `Power`, `Comms` (nine), plus `Core` for ownerless repair targets. Players do not own a per-player console vector; the server derives console access from `Player.station: Option<StationId>` and the loaded `ShipConfig` station roster. The JS tab bar displays the consoles derived from the player's station. The old `Science` console was split into `Sensors`, `Shields`, and `Navigation` (see individual entries below).

**Station** - the authoritative player role/seat on the ship, identified by stable `StationId` and defined in the ship entity TOML (e.g. `assets/entities/alliance_battleship.toml`) as `[[station]]`. Lobby selection is per-station (`SelectStation` / `ReleaseStation` / `StationAssigned`), and each `Player` stores `station: Option<StationId>`. Spectators wait in a FIFO queue. Disconnect does not reshuffle stations; it applies the station `Backfill` rating so AI operates that station's systems until reconnect or a new claim.

**System** — the fine-grained operable unit beneath a console, identified by a stable `SystemId` string (lowercase kebab). Three id patterns (pinned by issue #525, documented in `src/ship/system_registry.rs`): coarse systems match their kind (`"helm"`, `"tactical"`, `"power"`), fine systems add an instance suffix (`"phaser-fore"`, `"torpedo-tube-fore-port"`), and ownerless capabilities use a bare id (`"red-alert"`, `"viewscreen"`). Every registered system kind declares an AI controller, so any system can be operated by AI when its control source demands it.

**Control Source** — who operates a system this tick: `Human`, `Ai`, or `Offline` (`src/ship/control_source.rs`). Resolved per system per ship via a `ControlSourceResolver`; `control_tick_policy(source)` yields `{ accept_human_input, operate_ai, coordinate }`. This is one of only two differences between player and NPC ships (the other is `LocalShip`).

**Station Rating** — a named per-station table in the ship config declaring which of the station's systems run on AI. The implicit `Backfill` rating (always available) automates *every* system the station owns; it is applied on disconnect and cleared on reconnect. See `src/ship/rating.rs`.

**Session** - the server-side record of a connected or recently-disconnected player. Keyed by session token, not peer ID. Survives reconnects and stores `connected`, `ready`, `station`, and `last_rating`.

**Session Token** — a UUIDv4 stored in `localStorage`. The persistent identity of a player across page refreshes and reconnects. Distinct from PeerJS peer IDs, which are ephemeral.

**Lobby Phase** - the game state before play. Players join, pick stations, set names, and toggle `SetReady`. When every connected player is ready, the server auto-starts by entering `Loading` or `InProgress`; the legacy start message is gone.

**In-Progress Phase** - the game state after `GameStarted`. Console handlers process station-authorized inputs; helm sends inputs; captain toggles Red Alert; simulation runs. Disconnect applies Backfill AI and reconnect restores the old station/rating only if no connected player claimed it.

**Captain** - the player whose station owns `CaptainChair`. Authority to toggle Red Alert. Start-of-game authority is collective `SetReady` auto-start rather than a captain-only command.

**Helm Input** — `{ thrust: f32, steering: f32 }` sent at 10 Hz by the Helm console (as a `ControlSystem` message targeting the `helm` system). Drives `compute_physics()`.

**Red Alert** — a ship-wide state toggled by the captain via a `ControlSystem` message targeting the ownerless `red-alert` system. Visualised as a red vignette on the view screen and client consoles.

**Hull Integrity** — tracked **per console**, not as a single pool. `ConsoleHull` (`src/ship/damage.rs`) stores `(console, current_hp, max_hp)` entries with per-console tier thresholds. Incoming damage routes through shields first (`apply_damage_with_shields`; `split_damage_for_pierce` lets a `shield_pierce` fraction bypass them), then `apply_hull_damage` distributes the leaked amount randomly across consoles that still have HP, spilling onward when one reaches 0. The ship is destroyed when every console is at 0. Damaged consoles degrade by tier and are restored by repair teams. Hull is `f32` end-to-end; clients round for display.

**Repair** — the Repair console dispatches one of three repair teams to a damaged console: `DispatchRepairTeam { team_idx, target }` where `target` is a `RepairTarget` (a console, or `Core` for the ownerless hull). Each team runs a state machine (`src/modifiers/repair_teams.rs`): travel to the console, repair at `repair_rate_hp_per_sec`, travel back. Timings come from the `[repair]` block in the ship entity TOML (e.g. `assets/entities/alliance_battleship.toml`) (`RepairTimings`). The old shape-matching breakdown-queue puzzle (PRD #118) has been deleted.

**Modifier** — a multiplier registered on a named `ModifierSlot` (`MaxSpeed`, `MaxYawRate`, `RadarRange`, `PhaserDamage`, `HullDamageTaken`, `RepairRate`) by a `ModifierSource` (a power level, the impulse drive, a region effect identified by `uuid: Uuid`). Resolved into a per-slot cached multiplier consumed by physics, weapons, repair, and radar systems. Sum of bonuses `s ≥ 0` → cache `1.0 + s` (buff); `s < 0` → `1.0 / (1.0 + |s|)` (debuff). Implemented in `src/modifiers/cache.rs` (PRD #117). Broadcast over the wire via `ModifierAdded` / `ModifierRemoved`.

**Flag** — a typed boolean state (`FlagKind::CommsJammed`, `FlagKind::SensorBlind`) set on `ShipModifiers` by one or more sources with OR aggregation; the flag clears only when the last source removes it. Emitted by `comms_jammed` and `sensor_blind` region effects; available to any system. Lives in `flag_kind.rs` (PRD #153). Carried per-tick in `SimSnapshot.flags`.

**Power Group** — a named allocation bucket (`PowerGroupId`) defined by the ship config; consoles map to groups via `power_group_for_console` (e.g. helm, sensors, weapons groups). The Power console sets a group's level with `ControlSystem` / `SetPowerGroupAllocation { group, level }`.

**Power Allocation** — data-driven from the `[power]` block in the ship entity TOML (e.g. `assets/entities/alliance_battleship.toml`) (`capacity`, per-level `rates`, `emergency_threshold`, plus `[power.ai]` tuning floors). Allocation levels register modifiers on the relevant slots; battery exhaustion locks the power system until it recharges past the emergency threshold. Raw power truth is published each tick as `PowerBlackboard` (consoles, total/total_max, battery_charge/battery_max, locked). The old fixed "6+2" pool and the `IncreasePower` / `DecreasePower` wire messages are gone.

**Save Slot** *(planned, PRD #116)* - a `localStorage`-keyed snapshot (`phoenix_save_<uuid>`) holding `SaveMeta` (version, timestamps, player names) plus full `SaveState` (ship pose, hull, breakdowns, weapons, surviving asteroids). Planned save triggers should follow the current ready/auto-start flow rather than the removed captain-engage path.

**Scenario (legacy term)** — historically a separate TOML under `assets/scenarios/` that paired with a map TOML under `assets/maps/`. Both have been replaced by a single unified TOML under `assets/worlds/` (see *World File* below). The old `Scenario*` Rust types and multi-world layering runtime were deleted in PRD #342.

**World File** — a single TOML under `assets/worlds/` that declares everything a session contains: anchors, `[[entity]]` instances, named `[[spawn]]` entries (trigger/comms-eligible), `[[trigger]]` reactions, `[[comms]]` dialogue templates, and objectives. One world file per session; chaining trigger actions are not supported. Loaded by JS via `wasm_load_world(path, toml_str)`. The unified `WorldPlugin` (`src/world/server.rs`) consumes it.

**World (the plugin / the place)** — the unified server-side substrate that owns everything spatial: entity spawning, asteroid streaming (the `AsteroidWindow` ring buffer + lifecycle), region containment, world-file loading + trigger evaluation, objective tracking. There is no separate Scenario plugin; world files are content the World consumes. `WorldPlugin` lives at `src/world/server.rs`. The `World Data` wire snapshot below is the broadcast view of this state.

**Region** — a non-visual entity carrying a `RegionShape` (Sphere / Box / Torus, all 2D in XZ) and one or more effect components (`blocks_impulse`, `radar_dampening`, `damage_zone`, `slow_zone`, `comms_jammed`, `sensor_blind`). Containment is checked per tick; ships entering or exiting fire `RegionEntered` / `RegionExited` events that drive modifier registration, flag toggling, and impulse cancellation. Shipped with PRD #153 alongside the component-driven entity pipeline.

**Entity Snapshot** — the unified wire shape (`EntitySnapshot` in `messages.rs`) that replaced the bespoke per-type wire formats. Every world entity (asteroid, station, region, AI ship) ships in `WorldData.entities` as one `EntitySnapshot` with optional aspect fields (`shape`, `radius`, `colour`, `yaw`, `hull_fraction`). Dynamic entities additionally appear in `SimSnapshot.entity_states` per tick. Runtime spawn/despawn uses `EntitySpawned` / `EntityDespawned` deltas; clients are idempotent on both. Shipped with PRD #153.

**Asteroid Window** — the player-centred ring-buffer grid (`AsteroidWindow` in `src/asteroids/lifecycle.rs`, PRD #191) that replaced the pre-generated deterministic asteroid layout. The world is divided into `resolution × resolution` cells; a `WindowedGrid` of size `(2 × despawn_cells + 1)²` follows the player. As the player moves, delta-tracked cells outside `despawn_cells` are forgotten (`None`) and cells inside `spawn_cells` are evaluated fresh from a `(field_idx, gx, gz) + Perlin` seed. Destroyed asteroids respawn when the player leaves and returns. The old "fixed seeded layout per session" model is gone.

**Console Complexity (legacy term)** — the per-console `Low` / `Full` preset system from PRD #154 (`SetComplexity` wire message, `assets/complexity/*.toml`). Superseded by the per-system Control Source model: instead of a console-wide preset hiding UI and delegating to `console_ai`, each *system* now runs under `Human` / `Ai` / `Offline` control resolved from station ratings. A vestigial `complexity: HashMap<Console, String>` field remains on the wire; do not build new features on it.

**Sensors Console** - split from the old `Science` console. Handles long-range radar overlay, advisory target suggestion (`SetScienceTarget`), and pushes `SensorsRadar` to the viewscreen. Holder checks are station-derived via `Console::Sensors`.

**Shields Console** - split from the old `Science` console. Handles four-quadrant shield status and the focus mechanic (directing shield strength to one facing). Holder checks are station-derived via `Console::Shields`.

**Navigation Console** - split from the old `Science` console. Handles system chart on the viewscreen (`NavigationChart`), the shared navigation waypoint, and cancelling an active impulse charge (`CancelImpulse`). Holder checks are station-derived via `Console::Navigation`.

**Shield Facing** — one of four quadrants (fore/aft/port/starboard) absorbing damage before hull. Tracked in `shields.rs` and broadcast via `ShieldStatus`.

**Phaser Bank** — `Port` or `Starboard` directional energy weapon. Has its own arc, lock state, and cooldown. Modelled in `phaser.rs`.

**Torpedo Tube** — `ForePort`, `ForeStarboard`, or `Aft`. Each tube reloads independently and launches a homing torpedo via `torpedo.rs`.

**Impulse Drive** — a charged short-burst speed boost triggered by Helm via `StartImpulseCharge`, cancellable by Navigation via `CancelImpulse`. Modelled in `impulse.rs`.

**Comms Console** — manages contacts, the message inbox, and active mission objectives. Receives `CommsState { messages, contacts }` per broadcast. Sends `SelectCommsMessage { message_id }`. Client state lives in `gui/comms-state.js`; the panel is the `gui/comms-console.html` iframe; server inbox in `src/console/comms/inbox.rs`. Comms is re-marked dirty via `mark_comms_dirty_on_game_start` (an `OnEnter(InProgress)` system) so the initial contact list is delivered on the first InProgress tick.

**View Mode** — the server-side camera perspective on the view screen. `Camera(direction)` shows one of four hull cameras (Fore/Aft/Port/Starboard); `Radar` shows a top-down tactical view; `SensorsRadar` is pushed by the Sensors console; `NavigationChart` is pushed by the Navigation console; `CommsMessage` is pushed by Comms. Settable by clients via `SetView` (a `ControlSystem` message targeting the ownerless `viewscreen` system).

**Radar** — the overhead mini-map showing entity positions relative to the ship. Rendered on the server view screen (when in `Radar` mode), and inside the Helm and Tactical (weapons radar) panels. Sensors shows a longer-range overlay.

**World Data** — the snapshot of all world entities visible to all players, sent in `Welcome` and on `WorldSetup` as `WorldData.entities: Vec<EntitySnapshot>`. Asteroids stream in/out of the spawned ECS world via `src/asteroids/lifecycle.rs` driven by the `AsteroidWindow` ring buffer; spawn/despawn is broadcast as `EntitySpawned` / `EntityDespawned` deltas. Reconnect/late-join builds `WorldData` from a live ECS query so destroyed asteroids and runtime-spawned regions are reflected without delta replay.

**Entity Config** — a TOML file under `assets/entities/` describing one entity type's tags, geometry, physics, weapons, shields, and (planned) `on_attacked`/`on_destroyed` triggers. Loaded by `src/entities/config.rs` and surfaced as Bevy resources via `src/entities/config_cache.rs`.

---

## Architecture

**ControlSystem (wire)** — the unified in-game command: `ClientMessage::ControlSystem { target: SystemId, payload: SystemControlPayload }`. All console actions (red alert, view, helm input, power allocation, hail, respond, …) are `SystemControlPayload` variants addressed to a system; `ui_action_to_client_message` maps JS UI actions onto it. Humans and AI issue the same commands, so the server cannot tell them apart past admission.

**AdmittedCommand** — an authority-checked intra-system command produced by `admit_system_commands`. Admission strips the source identity (`response_token` survives purely for reply routing, never for behavioural branching). Everything downstream of admission is source-agnostic.

**Blackboard** — the per-system published state channel (Channel 1). Each system writes a typed `SystemBlackboard` variant (`HelmBlackboard`, `PowerBlackboard`, …, wire-serialised as a tagged enum) into the per-ship `ShipSystemBlackboards` component during `SimSet::Publish`; ship-wide aggregators (e.g. Viewscreen) read them in `SimSet::PublishAggregate`. Cross-system reads during Physics/Damage/Modifiers use `FrozenBlackboards` — last tick's snapshot — for determinism. Blackboard sync to clients is dirty-tracked (issue #557).

**Coordination Lag** — the lagged inter-console advisory bus. Producers enqueue via `CoordinationEnqueue`; entries wait in `CoordinationLagQueue` (`src/ship/coordination.rs`) and `process_coordination_lag` resolves each `DeliverAction` at delivery time, so target control resolves when the message lands, not when it was sent (issue #493).

**LocalShip** — a marker component (`src/server_app.rs`) on the player crew's ship entity. Used only for viewscreen rendering and local-scoped queries. Player and NPC ships otherwise share identical per-entity components and code paths (PRD #597); the only other difference is `ShipSystemControlSources` (human vs AI control sources).

**LobbyHandlerResult** — the return type of `process_message()` and `process_disconnect()` in `lobby/handler.rs`. Contains `new_phase: Option<GamePhase>` (None = no transition) and `outbound: Vec<(Target, ServerMessage)>`. `Target` is `All | Token(String) | AllExcept(String)`. Lobby selection is per-station: `SelectStation { station }` and `ReleaseStation` from clients; `StationAssigned { token, station, consoles }` broadcast in response.

**radar_dots** — the shared pure iterator in `radar.rs` that projects entity positions onto the radar plane given ship position and yaw. Returns `impl Iterator<Item = (f32, f32, f32)>` (radar_x, radar_y, scaled_radius).

**Console Plugin (server)** — a Bevy plugin that owns the server-side logic for one console, at `src/console/<name>/server.rs`, registered in `server_app.rs`. There is no client-side Rust: each console's UI is a standalone HTML iframe (`gui/<name>-console.html`) listed in `gui/console-registry.js`, receiving state via `iframe-bridge.js` and sending actions through `gui/action-map.js`.

**View-Model** — a pure derived snapshot that a renderer reads instead of raw session/simulation state. On the server, `GameState` (cached in `GameStateCache`) is the view-model for `server/renderer.rs`. On the client, the view-models are pure JS: `gui/lobby-state.js` for the lobby, `gui/sim-state.js` for shared sim state, and the per-console `build*(state)` functions in `gui/console-state.js` that produce each iframe's JSON.

**Client Sim State (JS)** — `gui/sim-state.js`, the pure-JS mirror of the server's `SimSnapshot` (a direct port of the deleted Rust `ClientSimState`). `apply(msg)` folds each parsed `ServerMessage` into the state object maintained by `client.html`; per-console radar configs live here, radar projection lives in `gui/radar-math.js`.

**Active Console** — the tab the local player is viewing. Pure selection logic in `gui/active-console.js`; `setActiveConsole(name)` in `client.html` shows/hides the per-console iframe sections via `gui/content-switcher.js`. There is no Bevy `ActiveConsole` resource any more.

**Broadcaster** — the seam through which all `OutboundMessage`s are emitted. A per-domain plugin registers a payload-builder system together with a `Cadence` and an `Audience`; the broadcaster resolves the audience against the live `SessionManager` each tick/event and invokes the system only when the audience is non-empty. Replaces the hand-coded `if phase != InProgress { return; } if !timer.just_finished() { return; } for console ... write(OutboundMessage { ... })` preamble that previously appeared at every broadcast site. Lives in `src/core/broadcast/`.

**LobbyBroadcaster / SimBroadcaster** — two `Broadcaster` instances, each phase-gated. `LobbyBroadcaster` runs only in `GamePhase::Lobby`; `SimBroadcaster` runs only in `GamePhase::InProgress`. The pure `lobby/handler.rs` keeps returning `Vec<(Target, ServerMessage)>`; the `lobby/server.rs` plugin funnels those outputs into `LobbyBroadcaster` as `Cadence::Once` registrations so all writes go through one path.

**Audience** - a predicate over the live session set, resolved by the `Broadcaster` to a set of session tokens. Built-ins: `Audience::all()`, `Audience::holding(Console)`, `Audience::all_except(Token)`, `Audience::token(Token)`. `holding(_)` resolves by mapping the console id through `ShipConfig` to the station held by a connected player.

**Cadence** — when a registered broadcast fires. `Cadence::hz(f32)` / `Cadence::period(Duration)` for periodic; `Cadence::on_event::<E>()` for event-driven; `Cadence::once()` for single-shot. The broadcaster owns timers internally; callers do not manage `Timer` resources.

**SimSet** — a `SystemSet` enum (`Input`, `Physics`, `Damage`, `Modifiers`, `Publish`, `PublishAggregate`, `Broadcast`) defined in `sim_sets.rs` and chained in that order in `server_app.rs`. `Publish` (phase 1a) is where every system writes its own blackboard; `PublishAggregate` (phase 1b) is where ship-wide aggregators read them. All in-game systems declare membership via `.in_set(SimSet::X)`. The entire chain is gated by `.run_if(in_state(GamePhase::InProgress))`.

**States\<GamePhase\>** — Bevy's native state framework (`bevy::state`) replacing the old `CurrentPhase` resource. `GamePhase` derives `States`, `Hash`, `Default` (default = `Lobby`). Transitions use `NextState<GamePhase>`. `LobbyPlugin` calls `app.init_state::<GamePhase>()` and adds `StatesPlugin` explicitly (required because unit tests don't add `DefaultPlugins`). Start-of-game systems use `OnEnter(GamePhase::InProgress)` schedules.

**WorldPlugin** — the unified server-side substrate in `src/world/server.rs` that owns world-file loading (`wasm_load_world`), entity lifecycle, trigger evaluation, objective tracking, and `WorldSetup` broadcast. World content is loaded via `WorldContentResource` / `WorldContentRuntime`. The JS bridge calls `wasm_load_world(path, toml_str)` with a single TOML, parsed once by `world::config::parse_world` into the `WORLD_CONFIG` thread-local. PRD #337 retired the legacy `MapConfig` / `ScenarioConfig` two-pass split; the unified `WorldConfig` is the sole world type, with `[[entity]]` as the only spawn surface.
