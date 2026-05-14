# Project Phoenix — Domain Vocabulary

Use these terms consistently across code, comments, PRs, and architecture discussions.

---

## Game Domain

**Console** — a role a player occupies on the ship. Currently shipped: `CaptainChair`, `Helm`, `Tactical`, `Repair`, `Science`, `Power` (six). Each console has exactly one seat; vacancy is immediate on disconnect. A player may hold more than one console simultaneously (the `Player.consoles` field is a `Vec<Console>`); the JS tab bar uses the `ActiveConsole` resource to switch which console panel is displayed. Planned (PRD #119): `Comms` console.

**Station** — a player's role bundle of one or more consoles, defined per player count in `player_ship.toml` and parsed by `stations.rs`. Joining/leaving auto-shuffles players between stations via `reassign_on_join` / `reassign_on_leave`. Spectators wait in a FIFO queue. Lobby selection is per-station (`SelectStation` / `ReleaseStation` / `StationAssigned`), not per-console. Shipped with PRD #120.

**Session** — the server-side record of a connected (or recently-disconnected) player. Keyed by session token, not peer ID. Survives reconnects.

**Session Token** — a UUIDv4 stored in `localStorage`. The persistent identity of a player across page refreshes and reconnects. Distinct from PeerJS peer IDs, which are ephemeral.

**Lobby Phase** — the game state before `StartGame`. Players join, pick consoles, set names. Only the captain can advance the phase.

**In-Progress Phase** — the game state after `StartGame`. Helm sends inputs; captain toggles Red Alert; simulation runs.

**Captain** — the player holding `CaptainChair`. Authority to start the game and toggle Red Alert. Server enforces this.

**Helm Input** — `{ thrust: f32, steering: f32 }` sent at 10 Hz by the Helm console. Drives `compute_physics()`.

**Red Alert** — a ship-wide state toggled by the captain. Visualised as a red vignette on the view screen and client consoles.

**Hull Integrity** — the ship's hit-point pool, tracked as `f32` end-to-end (PRD #153 migration), starting at 100 and clamped to [0, 100]. Reduced by asteroid collisions (5–15 HP depending on impact speed) and by region damage zones (`damage_per_second * dt`, fractional accumulation across ticks). Both paths funnel through the shared `apply_hull_damage` helper in `damage.rs`, which feeds the breakdown system. Restored by successful repairs. Client rounds for display.

**Shield Facing** — one of four quadrants (fore/aft/port/starboard) absorbing damage before hull. Tracked in `shield.rs` and broadcast via `ShieldStatus`.

**Phaser Bank** — `Port` or `Starboard` directional energy weapon. Has its own arc, lock state, and cooldown. Modelled in `phaser.rs`.

**Torpedo Tube** — `ForePort`, `ForeStarboard`, or `Aft`. Each tube reloads independently and launches a homing torpedo via `torpedo.rs`.

**Impulse Drive** — a charged short-burst speed boost triggered by Helm via `StartImpulseCharge`, cancellable by Science via `CancelImpulse`. Modelled in `impulse.rs`.

**Breakdown** — a console-repair assignment triggered by hull damage. Every 10 cumulative HP of damage generates one breakdown. A `BreakdownQueue` (FIFO in `breakdown.rs`) tracks pending assignments; each entry carries a randomly-assigned `Shape` (`Square`, `Triangle`, `Circle`). Repair is shape-matching (PRD #118): the Repair console must dispatch the head entry's shape to one of three repair teams (`repair_teams.rs`).

**Repair** — the action a Repair-console player performs by sending `Repair { shape: Shape }`. The shape must match the head of the `BreakdownQueue` and a repair-team slot must be free. Wrong shape, wrong console, or no free team incurs a cooldown penalty. Decoy shapes (`ShowRepairIcon` / `ClearRepairIcon`) are broadcast to make the puzzle non-trivial.

**Modifier** — a multiplier registered on a named `ModifierSlot` (`MaxSpeed`, `MaxYawRate`, `RadarRange`, `PhaserDamage`, `HullDamageTaken`, `RepairRate`) by a `ModifierSource` (a console-power level, the impulse drive, a region effect identified by `uuid: Uuid`). Resolved into a per-slot cached multiplier consumed by physics, weapons, repair, and radar systems. Sum of bonuses `s ≥ 0` → cache `1.0 + s` (buff); `s < 0` → `1.0 / (1.0 + |s|)` (debuff). Implemented in `modifiers.rs` (PRD #117). Broadcast over the wire via `ModifierAdded` / `ModifierRemoved`.

**Flag** — a typed boolean state (`FlagKind::CommsJammed`, `FlagKind::SensorBlind`) set on `ShipModifiers` by one or more sources with OR aggregation; the flag clears only when the last source removes it. Emitted by `comms_jammed` and `sensor_blind` region effects; available to any system. Lives in `flag_kind.rs` (PRD #153). Carried per-tick in `SimSnapshot.flags`.

**Power Allocation** — 6 base + up to 2 battery points distributed across `Helm`, `Tactical`, and `Science` by the Power console (`power_system.rs`, PRD #118). Each level registers modifiers on the relevant slots. Battery exhaustion locks all consoles to level 1 until recharged to an emergency threshold. Wire: `IncreasePower { console }` / `DecreasePower { console }`; broadcast every 100 ms as `PowerState` to the Power holder and as `power_levels` on `SimSnapshot` to all.

**Save Slot** *(planned, PRD #116)* — a `localStorage`-keyed snapshot (`phoenix_save_<uuid>`) holding `SaveMeta` (version, timestamps, player names) plus full `SaveState` (ship pose, hull, breakdowns, weapons, surviving asteroids). Saved on `Engage`, every 30 s, and on best-effort tab close.

**Scenario** *(planned, PRD #119)* — a TOML file loaded on top of the map at runtime that spawns entities, registers triggers (`on_attacked`, `on_destroyed`, `on_hailed`, `on_timer`), fires actions (`load_scenario`, `add_objective`, push comms message), and scripts comms exchanges. Owns the entities, objectives, and messages it created; cleans them up when unloaded.

**Region** — a non-visual entity carrying a `RegionShape` (Sphere / Box / Torus, all 2D in XZ) and one or more effect components (`blocks_impulse`, `radar_dampening`, `damage_zone`, `slow_zone`, `comms_jammed`, `sensor_blind`). Containment is checked per tick; ships entering or exiting fire `RegionEntered` / `RegionExited` events that drive modifier registration, flag toggling, and impulse cancellation. Shipped with PRD #153 alongside the component-driven entity pipeline.

**Entity Snapshot** — the unified wire shape (`EntitySnapshot` in `messages.rs`) that replaced the bespoke per-type wire formats. Every world entity (asteroid, station, region, future AI ship) ships in `WorldData.entities` as one `EntitySnapshot` with optional aspect fields (`shape`, `radius`, `colour`, `yaw`, `hull_fraction`). Dynamic entities additionally appear in `SimSnapshot.entity_states` per tick. Runtime spawn/despawn uses `EntitySpawned` / `EntityDespawned` deltas; clients are idempotent on both. Shipped with PRD #153.

**Asteroid Window** — the player-centred ring-buffer grid (`asteroid_window.rs`, PRD #191) that replaced the pre-generated deterministic asteroid layout. The world is divided into `resolution × resolution` cells; a `WindowedGrid` of size `(2 × despawn_cells + 1)²` follows the player. As the player moves, delta-tracked cells outside `despawn_cells` are forgotten (`None`) and cells inside `spawn_cells` are evaluated fresh from a `(field_idx, gx, gz) + Perlin` seed. Destroyed asteroids respawn when the player leaves and returns. The old "fixed seeded layout per session" model is gone.

**Console Complexity** — a per-console preset (`Low` / `Full`, defined in `assets/complexity/<console>.toml`) selected by the console holder via `SetComplexity { console, preset_name }` and broadcast as `ComplexityChanged`. Low complexity hides UI elements (`Display::None`) and runs server-side AI in `console_ai` to operate the hidden controls (auto-fire torpedoes, auto-match phaser frequency, auto-manage power battery overflow). Game mechanics are unchanged; the cost of Low is reaction-time and coordination latency. Three-tier delegation: native → delegated to partner console → AI fallback. Shipped with PRD #154.

**View Mode** — the server-side camera perspective on the view screen. `Camera(direction)` shows one of four hull cameras (Fore/Aft/Port/Starboard); `Radar` shows a top-down tactical view; `ScienceRadar` and `SystemChart` are pushed by Science. Settable by clients via `SetView`.

**Radar** — the overhead mini-map showing asteroid (and planned: station) positions relative to the ship. Rendered on the server view screen (when in `Radar` mode), and inside the Helm and Tactical (weapons radar) panels. Science shows a longer-range overlay.

**World Data** — the snapshot of all world entities visible to all players, sent in `Welcome` and on `WorldSetup` as `WorldData.entities: Vec<EntitySnapshot>`. Asteroids stream in/out of the spawned ECS world via `asteroid_lifecycle.rs` driven by the `AsteroidWindow` ring buffer; spawn/despawn is broadcast as `EntitySpawned` / `EntityDespawned` deltas. Reconnect/late-join builds `WorldData` from a live ECS query so destroyed asteroids and runtime-spawned regions are reflected without delta replay.

**Entity Config** — a TOML file under `assets/entities/` describing one entity type's tags, geometry, physics, weapons, shields, and (planned) `on_attacked`/`on_destroyed` triggers. Loaded by `entity_config.rs` and surfaced as Bevy resources via `config_cache.rs`.

**Map Config** — a TOML file under `assets/maps/` defining named spawn anchors, asteroid fields, and the default scenario reference. Loaded by `map_config.rs`.

---

## Architecture

**LobbyHandlerResult** — the return type of `process_message()` and `process_disconnect()` in `lobby_handler.rs`. Contains `new_phase: Option<GamePhase>` (None = no transition) and `outbound: Vec<(Target, ServerMessage)>`. `Target` is `All | Token(String) | AllExcept(String)`. Lobby selection is per-station: `SelectStation { station }` and `ReleaseStation` from clients; `StationAssigned { token, station, consoles }` broadcast in response.

**radar_dots** — the shared pure iterator in `radar.rs` that projects a slice of `AsteroidInfo` onto the radar plane given ship position and yaw. Returns `impl Iterator<Item = (f32, f32, f32)>` (radar_x, radar_y, scaled_radius). Both the server renderer and client helm console use this.

**Console Plugin** — a Bevy plugin that owns all UI, marker components, setup systems, and event handlers for a single console. On the server, the renderer handles all console views in `renderer.rs`. On the client, console panels are registered in `client_app.rs` (which hosts `CaptainPanel`, `HelmPanel`, etc.). Adding a new console = one new panel setup + visibility toggle + button handlers in `client_app.rs`.

**View-Model** — a pure derived snapshot that a renderer reads instead of raw session/simulation state. On the client, `LobbyView` (in `client_lobby.rs`) is the view-model for lobby rendering; `ClientSimState` (in `client_sim.rs`) is the view-model for in-game console rendering. On the server, `GameState` serves as the view-model for `renderer.rs`.

**ClientSimState** — the client-side mirror of the server's `SimSnapshot`. Maintained by applying `ServerMessage`s in `client_sim.rs`. Holds `red_alert`, `view_mode`, ship pose, `world` (asteroid layout), and repair state fields. Bevy `Resource`.

**ActiveConsole** — a Bevy `Resource` on the client that tracks which console panel the local player is currently viewing (set by the JS tab bar via `wasm_client_set_active_console`). `None` means auto-select the sole held console.

**Broadcaster** — the seam through which all `OutboundMessage`s are emitted. A per-domain plugin registers a payload-builder system together with a `Cadence` and an `Audience`; the broadcaster resolves the audience against the live `SessionManager` each tick/event and invokes the system only when the audience is non-empty. Replaces the hand-coded `if phase != InProgress { return; } if !timer.just_finished() { return; } for console ... write(OutboundMessage { ... })` preamble that previously appeared at every broadcast site.

**LobbyBroadcaster / SimBroadcaster** — two `Broadcaster` instances, each phase-gated. `LobbyBroadcaster` runs only in `GamePhase::Lobby`; `SimBroadcaster` runs only in `GamePhase::InProgress`. The pure `lobby_handler.rs` keeps returning `Vec<(Target, ServerMessage)>`; the `lobby/server.rs` plugin funnels those outputs into `LobbyBroadcaster` as `Cadence::Once` registrations so all writes go through one path.

**Audience** — a predicate over the live session set, resolved by the `Broadcaster` to a set of session tokens. Built-ins: `Audience::all()`, `Audience::holding(Console)`, `Audience::all_except(Token)`, `Audience::token(Token)`. Because each console has exactly one seat, `holding(_)` resolves to 0 or 1 tokens.

**Cadence** — when a registered broadcast fires. `Cadence::hz(f32)` / `Cadence::period(Duration)` for periodic; `Cadence::on_event::<E>()` for event-driven; `Cadence::once()` for single-shot. The broadcaster owns timers internally; callers do not manage `Timer` resources.
