# Project Phoenix — Domain Vocabulary

Use these terms consistently across code, comments, PRs, and architecture discussions.

---

## Game Domain

**Console** — a role a player occupies on the ship. Currently: `CaptainChair`, `Helm`, `Tactical`, `Engineering`. Each console has exactly one seat; vacancy is immediate on disconnect. A player may hold more than one console simultaneously (the `Player.consoles` field is a `Vec<Console>`); the JS tab bar uses the `ActiveConsole` resource to switch which console panel is displayed.

**Session** — the server-side record of a connected (or recently-disconnected) player. Keyed by session token, not peer ID. Survives reconnects.

**Session Token** — a UUIDv4 stored in `localStorage`. The persistent identity of a player across page refreshes and reconnects. Distinct from PeerJS peer IDs, which are ephemeral.

**Lobby Phase** — the game state before `StartGame`. Players join, pick consoles, set names. Only the captain can advance the phase.

**In-Progress Phase** — the game state after `StartGame`. Helm sends inputs; captain toggles Red Alert; simulation runs.

**Captain** — the player holding `CaptainChair`. Authority to start the game and toggle Red Alert. Server enforces this.

**Helm Input** — `{ thrust: f32, steering: f32 }` sent at 10 Hz by the Helm console. Drives `compute_physics()`.

**Red Alert** — a ship-wide state toggled by the captain. Visualised as a red vignette on the view screen and client consoles.

**Hull Integrity** — the ship's hit-point pool, starting at 100 and clamped to [0, 100]. Reduced by asteroid collisions (5–15 HP depending on impact speed, via `damage.rs`). Restored by successful repairs.

**Breakdown** — a console-repair assignment triggered by hull damage. Every 10 cumulative HP of damage generates one breakdown. A `BreakdownQueue` (FIFO in `breakdown.rs`) tracks pending assignments; the front entry is the `authorized_repair_console` broadcast in `SimSnapshot`.

**Repair** — the action a console player performs to clear a breakdown. Only the console named in `authorized_repair_console` may repair without penalty. Sending `Repair` from the wrong console incurs a cooldown penalty instead.

**View Mode** — the server-side camera perspective on the view screen. `Camera(direction)` shows one of four hull cameras (Fore/Aft/Port/Starboard); `Radar` shows a top-down tactical view. Settable by clients via `SetView`.

**Radar** — the overhead mini-map showing asteroid positions relative to the ship. Rendered on both the server view screen and the Helm console.

**World Data** — the fixed asteroid layout for a game session. Generated once on `StartGame` using a seeded deterministic generator. Sent to clients as `WorldSetup`.

---

## Architecture

**LobbyHandlerResult** — the return type of `process_message()` and `process_disconnect()` in `lobby_handler.rs`. Contains `new_phase: Option<GamePhase>` (None = no transition) and `outbound: Vec<(Target, ServerMessage)>`. `Target` is `All | Token(String) | AllExcept(String)`.

**radar_dots** — the shared pure iterator in `radar.rs` that projects a slice of `AsteroidInfo` onto the radar plane given ship position and yaw. Returns `impl Iterator<Item = (f32, f32, f32)>` (radar_x, radar_y, scaled_radius). Both the server renderer and client helm console use this.

**Console Plugin** — a Bevy plugin that owns all UI, marker components, setup systems, and event handlers for a single console. On the server, the renderer handles all console views in `renderer.rs`. On the client, console panels are registered in `client_app.rs` (which hosts `CaptainPanel`, `HelmPanel`, etc.). Adding a new console = one new panel setup + visibility toggle + button handlers in `client_app.rs`.

**View-Model** — a pure derived snapshot that a renderer reads instead of raw session/simulation state. On the client, `LobbyView` (in `client_lobby.rs`) is the view-model for lobby rendering; `ClientSimState` (in `client_sim.rs`) is the view-model for in-game console rendering. On the server, `GameState` serves as the view-model for `renderer.rs`.

**ClientSimState** — the client-side mirror of the server's `SimSnapshot`. Maintained by applying `ServerMessage`s in `client_sim.rs`. Holds `red_alert`, `view_mode`, ship pose, `world` (asteroid layout), and repair state fields. Bevy `Resource`.

**ActiveConsole** — a Bevy `Resource` on the client that tracks which console panel the local player is currently viewing (set by the JS tab bar via `wasm_client_set_active_console`). `None` means auto-select the sole held console.
