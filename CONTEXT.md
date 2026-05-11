# Project Phoenix — Domain Vocabulary

Use these terms consistently across code, comments, PRs, and architecture discussions.

---

## Game Domain

**Console** — a role a player occupies on the ship. Currently shipped: `CaptainChair`, `Helm`, `Tactical`, `Engineering`, `Science`. Each console has exactly one seat; vacancy is immediate on disconnect. A player may hold more than one console simultaneously (the `Player.consoles` field is a `Vec<Console>`); the JS tab bar uses the `ActiveConsole` resource to switch which console panel is displayed. Planned (PRD #118): `Engineering` → `Repair` rename + new `Power` console. Planned (PRD #119): `Comms` console. Planned (PRD #120): per-console picking is replaced by per-station picking.

**Station** *(planned, PRD #120)* — a player's role bundle of one or more consoles, defined per player count in `player_ship.toml`. Joining/leaving auto-shuffles players between stations. Spectators wait in a FIFO queue.

**Session** — the server-side record of a connected (or recently-disconnected) player. Keyed by session token, not peer ID. Survives reconnects.

**Session Token** — a UUIDv4 stored in `localStorage`. The persistent identity of a player across page refreshes and reconnects. Distinct from PeerJS peer IDs, which are ephemeral.

**Lobby Phase** — the game state before `StartGame`. Players join, pick consoles, set names. Only the captain can advance the phase.

**In-Progress Phase** — the game state after `StartGame`. Helm sends inputs; captain toggles Red Alert; simulation runs.

**Captain** — the player holding `CaptainChair`. Authority to start the game and toggle Red Alert. Server enforces this.

**Helm Input** — `{ thrust: f32, steering: f32 }` sent at 10 Hz by the Helm console. Drives `compute_physics()`.

**Red Alert** — a ship-wide state toggled by the captain. Visualised as a red vignette on the view screen and client consoles.

**Hull Integrity** — the ship's hit-point pool, starting at 100 and clamped to [0, 100]. Reduced by asteroid collisions (5–15 HP depending on impact speed, via `damage.rs`). Restored by successful repairs.

**Shield Facing** — one of four quadrants (fore/aft/port/starboard) absorbing damage before hull. Tracked in `shield.rs` and broadcast via `ShieldStatus`.

**Phaser Bank** — `Port` or `Starboard` directional energy weapon. Has its own arc, lock state, and cooldown. Modelled in `phaser.rs`.

**Torpedo Tube** — `ForePort`, `ForeStarboard`, or `Aft`. Each tube reloads independently and launches a homing torpedo via `torpedo.rs`.

**Impulse Drive** — a charged short-burst speed boost triggered by Helm via `StartImpulseCharge`, cancellable by Science via `CancelImpulse`. Modelled in `impulse.rs`.

**Breakdown** — a console-repair assignment triggered by hull damage. Every 10 cumulative HP of damage generates one breakdown. A `BreakdownQueue` (FIFO in `breakdown.rs`) tracks pending assignments; the front entry is the `authorized_repair_console` broadcast in `SimSnapshot`. Planned (PRD #118): replaced by shape-matching with a `Shape` enum and three repair teams.

**Repair** — the action a console player performs to clear a breakdown. Only the console named in `authorized_repair_console` may repair without penalty. Sending `Repair` from the wrong console incurs a cooldown penalty instead.

**Modifier** *(planned, PRD #117)* — a multiplier registered on a named `ModifierSlot` (`MaxSpeed`, `MaxYawRate`, `RadarRange`, `PhaserDamage`, `HullDamageTaken`, `RepairRate`) by a `ModifierSource` (a console, the impulse drive, a region effect). Resolved into a per-slot cached multiplier consumed by physics, weapons, repair, and radar systems.

**Power Allocation** *(planned, PRD #118)* — 6 base + up to 2 battery points distributed across `Helm`, `Tactical`, and `Science` by the Power console. Drives modifiers on each console's relevant slots. Battery exhaustion locks all controls to level 1 until recharged to an emergency threshold.

**Save Slot** *(planned, PRD #116)* — a `localStorage`-keyed snapshot (`phoenix_save_<uuid>`) holding `SaveMeta` (version, timestamps, player names) plus full `SaveState` (ship pose, hull, breakdowns, weapons, surviving asteroids). Saved on `Engage`, every 30 s, and on best-effort tab close.

**Scenario** *(planned, PRD #119)* — a TOML file loaded on top of the map at runtime that spawns entities, registers triggers (`on_attacked`, `on_destroyed`, `on_hailed`, `on_timer`), fires actions (`load_scenario`, `add_objective`, push comms message), and scripts comms exchanges. Owns the entities, objectives, and messages it created; cleans them up when unloaded.

**View Mode** — the server-side camera perspective on the view screen. `Camera(direction)` shows one of four hull cameras (Fore/Aft/Port/Starboard); `Radar` shows a top-down tactical view; `ScienceRadar` and `SystemChart` are pushed by Science. Settable by clients via `SetView`.

**Radar** — the overhead mini-map showing asteroid (and planned: station) positions relative to the ship. Rendered on the server view screen (when in `Radar` mode), and inside the Helm and Tactical (weapons radar) panels. Science shows a longer-range overlay.

**World Data** — the snapshot of asteroids and asteroid fields visible to all players. Sent in `Welcome` and on `WorldSetup`. Asteroids stream in/out of the spawned ECS world via `asteroid_lifecycle.rs` based on range from the ship.

**Entity Config** — a TOML file under `assets/entities/` describing one entity type's tags, geometry, physics, weapons, shields, and (planned) `on_attacked`/`on_destroyed` triggers. Loaded by `entity_config.rs` and surfaced as Bevy resources via `config_cache.rs`.

**Map Config** — a TOML file under `assets/maps/` defining named spawn anchors, asteroid fields, and the default scenario reference. Loaded by `map_config.rs`.

---

## Architecture

**LobbyHandlerResult** — the return type of `process_message()` and `process_disconnect()` in `lobby_handler.rs`. Contains `new_phase: Option<GamePhase>` (None = no transition) and `outbound: Vec<(Target, ServerMessage)>`. `Target` is `All | Token(String) | AllExcept(String)`.

**radar_dots** — the shared pure iterator in `radar.rs` that projects a slice of `AsteroidInfo` onto the radar plane given ship position and yaw. Returns `impl Iterator<Item = (f32, f32, f32)>` (radar_x, radar_y, scaled_radius). Both the server renderer and client helm console use this.

**Console Plugin** — a Bevy plugin that owns all UI, marker components, setup systems, and event handlers for a single console. On the server, the renderer handles all console views in `renderer.rs`. On the client, console panels are registered in `client_app.rs` (which hosts `CaptainPanel`, `HelmPanel`, etc.). Adding a new console = one new panel setup + visibility toggle + button handlers in `client_app.rs`.

**View-Model** — a pure derived snapshot that a renderer reads instead of raw session/simulation state. On the client, `LobbyView` (in `client_lobby.rs`) is the view-model for lobby rendering; `ClientSimState` (in `client_sim.rs`) is the view-model for in-game console rendering. On the server, `GameState` serves as the view-model for `renderer.rs`.

**ClientSimState** — the client-side mirror of the server's `SimSnapshot`. Maintained by applying `ServerMessage`s in `client_sim.rs`. Holds `red_alert`, `view_mode`, ship pose, `world` (asteroid layout), and repair state fields. Bevy `Resource`.

**ActiveConsole** — a Bevy `Resource` on the client that tracks which console panel the local player is currently viewing (set by the JS tab bar via `wasm_client_set_active_console`). `None` means auto-select the sole held console.
