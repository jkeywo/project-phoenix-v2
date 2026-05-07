# Project Phoenix — Domain Vocabulary

Use these terms consistently across code, comments, PRs, and architecture discussions.

---

## Game Domain

**Console** — a role a player occupies on the ship. Currently: `CaptainChair`, `Helm`. Each console has exactly one seat; vacancy is immediate on disconnect.

**Session** — the server-side record of a connected (or recently-disconnected) player. Keyed by session token, not peer ID. Survives reconnects.

**Session Token** — a UUIDv4 stored in `localStorage`. The persistent identity of a player across page refreshes and reconnects. Distinct from PeerJS peer IDs, which are ephemeral.

**Lobby Phase** — the game state before `StartGame`. Players join, pick consoles, set names. Only the captain can advance the phase.

**In-Progress Phase** — the game state after `StartGame`. Helm sends inputs; captain toggles Red Alert; simulation runs.

**Captain** — the player holding `CaptainChair`. Authority to start the game and toggle Red Alert. Server enforces this.

**Helm Input** — `{ thrust: f32, steering: f32 }` sent at 10 Hz by the Helm console. Drives `compute_physics()`.

**Red Alert** — a ship-wide state toggled by the captain. Visualised as a red vignette on the view screen and client consoles.

**View Mode** — the server-side camera perspective on the view screen (`Free`, `Hull`, etc.). Settable by clients via `SetView`.

**Radar** — the overhead mini-map showing asteroid positions relative to the ship. Rendered on both the server view screen and the Helm console.

**World Data** — the fixed asteroid layout for a game session. Generated once on `StartGame` using a seeded deterministic generator. Sent to clients as `WorldSetup`.

---

## Architecture

**LobbyHandlerResult** — the return type of the pure lobby handler functions in `lobby_handler.rs`. Contains `new_phase: Option<GamePhase>` (None = no transition) and `outbound: Vec<(Target, ServerMessage)>`.

**radar_dots** — the shared pure iterator in `radar.rs` that projects a slice of `AsteroidInfo` onto the radar plane given ship position and yaw. Returns `impl Iterator<Item = (f32, f32, f32)>` (radar_x, radar_y, scaled_radius). Both the server renderer and client helm console use this.

**Console Plugin** — a Bevy plugin that owns all UI, marker components, setup systems, and event handlers for a single console. Current console plugins: `CaptainConsolePlugin`, `HelmConsolePlugin`. Adding a new console = one new plugin.

**View-Model** — a pure derived snapshot that a renderer reads instead of raw session/simulation state. On the client, `LobbyView` is the view-model for lobby rendering. On the server, `GameState` (stored as a Bevy resource) serves as the view-model for `renderer.rs`.
