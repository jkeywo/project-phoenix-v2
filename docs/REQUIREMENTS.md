# Project Phoenix — Consolidated Requirements

A unified feature list distilled from every PRD issue on the project issue tracker, processed in chronological order. Requirements that were superseded or removed by later PRDs have been dropped silently. Where a feature appears in multiple PRDs, the most recent definition wins.

---

## Project Overview

Project Phoenix is a multiplayer bridge simulator in the spirit of Artemis. One screen — the "view screen" — drives a shared 3D view of space and acts as the authoritative simulation host. Each crew member joins from their own phone by scanning a QR code, picks a station that bundles one or more consoles, and plays through scripted missions against AI ships and environmental hazards.

The game is built in Rust + Bevy, deployed two ways:
- A WebAssembly build hosted on GitHub Pages, with one tab as the view screen and phones joining via PeerJS/WebRTC.
- A native PC binary that runs the same simulation core, exposes the same client page over an embedded HTTP/WebSocket server, and tunnels to the internet via a bundled Cloudflare quick-tunnel sidecar.

Both deployments share the same simulation core, the same wire protocol, and the same client page. Phones never install anything.

---

## Lobby & Crew Assignment

### Joining a game
- A host opens the view screen (browser or native binary) and a QR code appears in the lobby.
- A player scans the QR code with their phone and is taken to the client page in their browser. No install.
- On first load, a player is given a random space-themed name and a session token stored in `localStorage`. Both persist across reloads, reconnects, and device swaps.
- Players can edit their name before the game starts.
- A connection-status dot in the top-right of every page shows the WebRTC/WebSocket peer state (`connecting`, `ready`, `disconnected`, `error`) with auto-reconnect on transient drops and a clear "refresh to retry" message on fatal errors.
- A fullscreen toggle button sits next to the connection-status dot on both client and view-screen pages.

### Stations (per-player-count topology)
- The unit of selection is a **station**, not a console. A station bundles one or more consoles and represents one player's role.
- Each ship's `player_ship.toml` declares stations per player count, with explicit `next`/`previous` chains that describe how roles shift as crew joins or leaves.
- **Mid-game (InProgress phase):** a player leaving triggers a deterministic reassignment cascade so every station at the new player count stays filled, and a queued spectator is auto-promoted into the next vacated bottom-of-chain station. Reconnects are treated as fresh joins (no station stickiness).
- **Lobby phase:** neither the reassignment cascade nor spectator auto-promotion fires. A leaver's slot simply goes empty and remains available for any subsequent joiner — including a sitting spectator — to pick manually by clicking the row.
- When all stations at the current player count are full, additional joiners become spectators in a FIFO queue. Auto-promotion into a vacated bottom-of-chain station happens **only when the vacancy opens mid-game**; in the lobby phase the spectator must click the empty row themselves.
- The captain is whoever holds the station containing the `CaptainChair` console (not a fixed seat).
- The lobby shows one row per station at the current player count plus rows for spectators; rows display station name + description + current holder. Spectator rows show "(spectating)" without queue position.
- Clicking an empty station while in another station is an atomic swap. Clicking your own station or an occupied station is a no-op.
- A "Leave station" affordance returns the player to the spectator pool.
- The Engage button is visible only on the captain's panel and only enables when every station at the current player count is filled.
- After Engage, spectators continue to see the lobby with a "Game in progress" banner; auto-promotion mid-game swaps them straight to their assigned consoles.

### Multi-console stations
- A player on a multi-console station sees each console as a tab.
- The active tab is preserved across reassignments when the new station still contains it; otherwise the client lands on the first console of the new station.
- Each tab displays a red dot when its console has taken any damage (see Damage & Hull).

### Captain detection & Engage
- `StartGame` is accepted only when the sender holds `CaptainChair` and all stations at the current player count are filled.
- A failed `SelectStation` (unknown name, occupied, wrong phase) is silently dropped; the client re-syncs from the next assignment broadcast.

### Validation
- Malformed station configs refuse to boot the server with a structured error rendered as a fatal overlay on the view-screen page (PeerJS never starts, lobby never opens).

---

## Console Complexity

A per-console complexity tier hides UI elements and adds AI to operate the hidden controls. Game mechanics are unchanged across tiers; the cost of "Low" complexity is reaction-time and coordination latency, not reduced effectiveness.

### Principles
- **Hint vs Act.** AI hints to a full-complexity console (the player still presses the button). AI acts on behalf of a low-complexity console.
- **Three-tier delegation.** When a control is hidden on its native console, it is either delegated to a partner console with its own opt-in receiver UI, or operated by AI on a configurable delay.

### Tiers and authoring
- Defaults are **Low** and **Std**; designers can add more by editing TOML.
- Each console section in `player_ship.toml` references a complexity TOML file (e.g. `assets/complexity/tactical.toml`).
- A console without a complexity reference defaults to a single implicit Full preset, no AI, no dropdown.
- Complexity tiers ship for **Tactical, Sensors, and Power** only. Repair stays Full only (the dispatch role is the whole gameplay). Helm and Captain are already minimal. Shields, Navigation, and Comms deferred.

### Per-console behaviour
- **Tactical (Low):** torpedo settings hidden — AI auto-fires when locked target's facing shield is down AND a tube is loaded in arc AND magazine > 0 (deterministic tube priority). Phaser-frequency buttons hidden — delegated to Sensors (Tier 2) or operated by AI (Tier 3).
- **Sensors (Low):** enemy shield-frequency readout hidden. Targeting still works.
- **Tactical Full + Sensors Low:** Tactical's correct-frequency button highlights after `auto_hint_delay_secs` (default 3s) of shared target. Player still clicks.
- **Both Low (or Sensors unmanned):** AI auto-matches Tactical's phaser frequency to the locked target's shield frequency after `auto_match_delay_secs` (default 3s). Frequency persists when the trigger ends.
- **Power (Low):** player still manually allocates the 6 base points. AI manages battery overflow (points 7-8), capped at 4 per console. Helm rule: sustained thrust ≥ threshold for engage delay + battery above engage min → +1 Helm. Red-Alert rule: red alert sustained for engage delay + battery above engage min → +1 Weapons. Rules stack to total +2. Immediate disengage below threshold; re-engage only after battery recharges to recharge_pct (default 100%).

### Client UX
- Settings dropdown on each console picks the tier; hidden when only one preset is defined.
- First-use pop-up on a console with more than one preset, Low pre-selected, requires explicit choice.
- Preference persists in `localStorage` per console; on rejoin the client compares stored preference to the server's current setting and re-syncs.
- Stored preset name no longer in TOML → re-prompt rather than silent fallback.
- Hidden elements use `Display::None` (true layout removal).
- The lobby roster shows each console's current preset alongside the holder's name.

### Server enforcement
- `SetComplexity` validated on sender (must hold the console) and preset existence; broadcast on success.
- Per-control delegation allowlist gates which sender can issue which message under which complexity state (e.g. `SetPhaserFrequency` from Sensors is accepted only when Tactical is at Low).
- Complexity flips mid-AI-countdown cancel any pending AI action.
- AI runs server-side as pure decision functions per behaviour.

---

## Save / Load

- A pre-WASM selection screen on the view-screen page lists every saved slot (creation date, last-saved timestamp, player names) plus a "New Game" entry.
- WASM compiles in the background while the host views the selection screen.
- Each slot has a stable UUID (`phoenix_save_<uuid>` in `localStorage`) and is one versioned JSON blob containing meta + full state.
- Resuming a slot restores the full game state before opening the lobby; "New Game" generates a fresh slot UUID.
- Slots from an incompatible save format are shown greyed-out with an "incompatible version" label; Resume disabled, Delete still enabled.
- Delete has a confirmation prompt.
- Saves fire automatically on the `InProgress` phase transition, every 30 seconds during play, and best-effort on `beforeunload` / `visibilitychange`.
- Full-fidelity save covers: ship position/yaw/speed, per-console HP (current per-console hull system), repair-team state, last helm input, collision cooldown, weapons target lock + active beam + cooldown, surviving asteroids with current HP (despawned-at-save asteroids reset to full HP on re-spawn), modifier state, scenario state, power state, player names.

---

## Hosting & Deployment

### Browser deployment (WASM)
- One Rust crate, two Trunk entry points: `server.html` (view screen) and `client.html` (phones).
- CI deploys built assets to GitHub Pages via `peaceiris/actions-gh-pages` on push to `main`.
- Transport is PeerJS (WebRTC, star topology). The view screen is the host; phones are spokes.
- PeerJS cloud broker used for signalling. The view-screen page automatically reconnects with `peer.reconnect()` on transient drops; permanent error states require manual refresh.

### Native PC binary
- `cargo build --release --features native` produces a single executable.
- Release zip contains: the binary, a bundled `cloudflared` per platform, and the static `dist/client/` directory.
- Double-click the binary → 3D viewscreen renders in a native OS window using the full GPU; QR code overlay appears once the Cloudflare quick-tunnel is `Ready`.
- A `TunnelManager` parses cloudflared stdout for the `trycloudflare.com` URL; the view-screen UI shows spinner (Pending) → QR code (Ready) → error (Failed).
- An embedded axum server serves the static client page at `/client/*` and a WebSocket endpoint at `/ws`, sharing a single TCP port that is tunnelled to the internet.
- Map and entity TOML loaded synchronously from disk; no internet needed to start a session.
- `client.html` auto-detects transport from the URL fragment: a `wss://` / `ws://` fragment opens a WebSocket; otherwise the existing PeerJS path runs. The first message in either transport is `Identify { token, name }`.
- The WASM deployment and native binary share the simulation, lobby, session, physics, damage, scenario, and codec modules unchanged. Native and WASM bridges live behind mutually exclusive Cargo features (`server`, `client`, `native`).

---

## World & Simulation

### Component-driven entity pipeline
- Every world entity (ship, asteroid, region, star, planet, station, asteroid field) is a `[[entity]]` instance pointing to a template TOML file plus per-instance position, optional shape, optional `id`, and optional `[entity.overrides]`.
- The same loader is used by maps and scenarios.
- Generic spawner: the merged `EntityConfig` is walked once and every present `Option<T>` section produces a Bevy component insertion. No per-type dispatch.
- Generic deep-merge over `toml::Value` for template + per-instance overrides. Removal of template sections is not supported.
- Static vs dynamic is implicit from components: an entity is dynamic iff it has a movement component or a `Hull` component. Static entities ship once in `WorldData`; dynamic entities ship per-tick in `SimSnapshot.entity_states`.
- Player ship spawns via `[[entity]]` with `spawn_on = "game_start"` — ship does not exist during the lobby phase.
- Every spawned entity has a server-assigned UUID; an optional human-readable `id` from the instance is also passed through to wire snapshots.

### World TOML
- A world TOML (`assets/worlds/*.toml`) is the single content file for a session. It declares the global `seed`, named `[anchors]`, a list of `[[entity]]` instances (anonymous entries are static layout; entries carrying a `name` field are UUID-assigned and trigger/comms-eligible — PRD #339 slice 2), `[[trigger]]` reactions, `[[comms]]` dialogue templates, and `[[objective]]` entries. Legacy `[[spawn]]` blocks remain during the PRD #337 transition for the patrol-NPC entries still pending migration in slice 3.
- Internally the parser is moving toward a single-pass `parse_world` → unified `WorldConfig` (PRD #337/#338 slice 1 + #339 slice 2). The legacy `MapConfig` / `ScenarioConfig` two-pass split still runs for sections not yet folded into the unified pipeline; named `[[entity]]` entries flow through `spawn_world_entities`, while anonymous entries continue through `setup_world_from_config`.
- Named spawn anchors are declared at the top of the world file and referenced by name from `[[spawn]]` entries' `anchor = "..."` field; positions never need to be hardcoded in scripts. Anchor lookup on `[[entity]]` is pending a later slice of PRD #337 — for now, named `[[entity]]` entries inline their position.
- A single global seed drives all deterministic generation; per-field index offsets prevent reshuffling when one field's config changes.
- Scenario chaining (`load_scenario` / `unload_scenario` actions) is **not supported.** Each session loads exactly one world file at startup and runs it to completion.

### Asteroids and asteroid fields
- An asteroid field is an entity with inner radius, outer radius, density, and a list of gameplay/cosmetic asteroid type paths.
- Asteroid lifecycle is driven by a **2D ring-buffer window** centred on the player's current grid cell rather than a distance-based candidate queue. The window has separate `spawn_cells` and `despawn_cells` radii at a fixed `resolution` (world units per cell), and each window slot holds either an `AsteroidData` (uuid, config path, current/max HP, y) or `None`.
- A deterministic per-cell evaluation (`eval_cell`) decides what spawns in each cell, seeded by the global map seed and the cell coordinates so a given cell always produces the same content.
- When the player moves to a new grid cell, the window is updated incrementally: cells that fall outside `despawn_cells` are despawned, cells that enter `spawn_cells` are evaluated and spawned. A jump that exceeds the window triggers a full rebuild.
- Damaged-but-not-destroyed asteroid state is not persisted across despawn/respawn — asteroids that despawn out of range and re-spawn always come back at full HP.
- **Destroyed asteroids are not remembered across cell re-entry.** When a destroyed asteroid's cell leaves the window and is later re-entered, the cell is re-evaluated and a fresh asteroid spawns with the same content (the per-session destroyed-UUID set described in earlier PRDs is not implemented).
- Asteroids are damage-tracked entities with their own hull pool; phaser fire destroys them progressively.
- On destruction, a radial ripple visual plays on the viewscreen and an `AsteroidDestroyed { uuid }` broadcast clears it from every client.
- Asteroid spawn/destroy uses the specialised `AsteroidSpawned` / `AsteroidDestroyed` channel rather than the generic `EntitySpawned` / `EntityDespawned` flow, because the asteroid window manages lifecycle independently and the wire shape carries per-spawn HP and config-path fields.

### Stars, planets, and other set dressing
- Stars are emissive sphere meshes with no collider, no hull, indestructible.
- Planets are lit sphere meshes with no collider, no hull, indestructible.
- Anything with a `[shape]` section becomes a region (see Regions).

### Regions (environmental effects)
- A region is any entity with a `RegionShape` component plus one or more effect components. Shape primitives are 2D in the XZ plane: `Sphere { radius }`, `Box { extents, yaw }`, `Torus { major_radius, minor_radius }`. Ship Y-coordinate is ignored.
- Region templates live in `assets/entities/region_*.toml`; instances are placed in maps or scenarios.
- A single per-tick system computes containment, diffs against the previous tick's `RegionMembership`, and emits `RegionEntered` / `RegionExited` events; region despawn while a ship is inside emits an implicit exit.
- Effects (each is a separate component, opted into via a `[effects.*]` sub-table; multiple effects per region are allowed):
  - `blocks_impulse` — cancels charging AND active impulse on entry; prevents new transitions out of Idle while inside.
  - `radar_dampening { range_modifier }` — registers a `RadarRange` modifier with `ModifierSource::RegionEffect { uuid }` for the duration of containment. Multiple regions stack.
  - `damage_zone { damage_per_second }` — applies `dps * dt` per tick, bypassing shields, via the shared damage helper. Fractional damage accumulates.
  - `slow_zone { thrust_modifier?, yaw_rate_modifier? }` — registers `MaxSpeed` / `MaxYawRate` modifiers on entry AND immediately clamps current velocity. Exit removes modifiers; previously-clamped velocity is not restored.
  - `comms_jammed` — sets `FlagKind::CommsJammed` keyed by region UUID; OR-aggregates across multiple sources.
  - `sensor_blind` — sets `FlagKind::SensorBlind` keyed by region UUID; OR-aggregates across multiple sources.

### Simulation tick rate and broadcast
- Discrete events fire immediately.
- Continuous state broadcasts at 10 Hz via `SimSnapshot`.
- Per-console payloads are routed directly (e.g. `Target::One(token)`) when only one console needs them; world-wide state goes to all clients.
- The simulation only ticks during the `InProgress` phase; phase gating uses Bevy `States` + `.run_if(in_state(GamePhase::InProgress))` rather than per-system guards.

### Architecture conventions
- "Scenario" is a file format, not a runtime concept: the World plugin owns parse → spawn → trigger → broadcast end-to-end. The default scenario (named by the map) is loaded at startup; an embedded fallback applies when no scenario file is available.
- Bevy `Observer` / `Trigger` is the canonical pattern for one-shot lifecycle reactions (region enter/exit, beam start/end, modifier add/remove). Wire-level `ServerMessage` broadcasts remain on the pull-based message system.
- A project-wide `SystemSet` hierarchy (`Input → Physics → Damage → Modifiers → Broadcast`) orders systems globally; per-plugin systems attach to a set rather than declaring local `.before()` / `.after()`.

---

## World Files & Triggers

### World engine
- World files are TOMLs in `assets/worlds/`, fetched at runtime by JS and passed into Rust via a single `wasm_load_world(path, toml_str)` call (WASM) or read from disk (native).
- A world file can spawn entities, react to trigger conditions, fire actions, manage objectives, and script comms exchanges — all the things the old separate "scenario" file used to do, plus the static layout (anchors, entity instances) the old separate "map" file used to do.
- **One world per session.** Scenario chaining is removed. The `LoadScenario` / `UnloadScenario` trigger actions no longer exist. The world file loaded at startup is the only world for that session.
- World triggers fire only the first time their condition is met per session (single-shot).
- Internally `ScenarioManager`, `ScenarioOwner`, and the `scenario_path` field on triggers/comms/dialogues survive as plumbing for `CommsInbox::unload_scenario` and `ObjectiveManager::unload_scenario` cleanup paths — they are not exposed to TOML authors. PRD #337 will delete these and the multi-scenario `HashMap<path, …>` layering throughout the runtime.

### Trigger conditions
- `on_attacked` — fires when the named entity is attacked.
- `on_destroyed` — fires when the named entity is destroyed (hull reaches 0).
- `on_hailed` — fires when an entity is hailed by the Comms officer.
- `on_timer { seconds }` — fires after a duration.
- `on_entity_attacked { entity }` / `on_entity_destroyed { entity }` — scenario subscribes to AI/world events on a named entity.

### Trigger actions
- `load_scenario { path }` — additively loads a follow-on scenario alongside any currently active scenarios. No-op if the path is already active.
- `unload_scenario { path }` — explicitly unloads a named scenario: fires `on_scenario_unloaded` on all owned AI entities immediately, orphans owned comms messages, removes the scenario from the active map. Owned entities persist as self-directed.
- `add_objective` / `complete_objective` / `fail_objective` — manage the objective list.
- Inline branching dialogue inside a single scenario file for short exchanges.
- `set_ai_state { entity, state, target? }` — forces a named AI entity into a given state; resets `state_entered_at`, optionally overwrites blackboard `target`, leaves `last_attacker` and `waypoint_index` alone.
- `apply_modifier { entity, tag, slot, bonus: f32 }` / `remove_modifier { entity, tag, slot }` — scenario-applied float modifier on a slot.
- `apply_int_modifier { entity, tag, slot, bonus: i32 }` / `remove_int_modifier { entity, tag, slot }` — scenario-applied integer modifier (e.g. grant a bonus repair team).
- `apply_flag { entity, tag, kind }` / `remove_flag { entity, tag, kind }` — scenario-applied flag (e.g. CommsJammed).
- `game_over { message? }` — instantly transition the game to a terminal state with a scenario-supplied message.

Scenario-applied modifiers and flags have `ModifierSource::Scenario { id, tag }`; the `(id, tag)` pair is the identity key for replacement and removal.

### Spawn entries
- `[[spawn]] template = "…" position = anchor_name | [x,y,z] | entity_relative` plus optional `name`, `shape`, `[spawn.overrides]`.
- Region instances additionally require a `shape` block.
- Spawn `name` is resolved to UUID by the scenario engine; scripts reference entities via `$param_name`, never raw UUIDs.

### Default content
- A canonical default scenario (Starbase Alpha) spawns a raider and a station. The station can be hailed (short inline branching dialogue). When the **raider** is attacked, an `on_attacked` trigger fires a broadcast comms message (no player interaction required) and `load_scenario` chains to the patrol scenario, which spawns reinforcements. The station has a parallel `on_attacked` trigger with its own distress broadcast.
- A canonical AI demo scenario (patrol) spawns a `pirate_raider` AI ship at named anchors, exercising every state and most conditions.
- Two default factions ship: Federation (player) and Pirate (enemies = Federation), in `assets/factions/`.

---

## Viewscreen

### Camera and view modes
- The viewscreen camera is a first-person hull camera, positioned at the ship's centre offset by the collision capsule radius in the active view direction.
- View modes form a single typed enum: `Camera(Fore | Aft | Port | Starboard)`, `Radar`, `SensorsRadar`, `ScienceRadar` (legacy alias), `NavigationChart`, `SystemChart` (legacy alias), `Comms`.
- Per-mode authorization is enforced server-side:
  - Captain — any `Camera(direction)`.
  - Helm — `Radar`.
  - Sensors — `SensorsRadar` (and the legacy `ScienceRadar`).
  - Navigation — `NavigationChart` (and the legacy `SystemChart`).
  - Comms — `Comms` (push the active message).
- Default view at game start is `Camera(Fore)`.
- A view-mode label is rendered top-centre of the screen during `InProgress`, showing the active mode in caps (e.g. `FORE`, `RADAR`, `SYSTEM CHART`).

### Frame UI (Bevy UI border)
- A tiled pixel-art frame surrounds the 3D scene during `InProgress` only.
- Ten border sprites: four corners (240×140), four edges (tiled), top cap (320×56), bottom cap (520×56). Edges tile to fill the gaps.
- Top cap displays the static designation `AEV-074 · PHOENIX`.
- Bottom cap shows a three-column HUD: HEADING (000–359, integer compass bearing increasing clockwise from forward), HULL (current/max), CONDITION (NOMINAL / ALERT).
- Visible only during `InProgress`; despawns on phase transitions back to Lobby.

### Red Alert
- Captain toggles Red Alert from the Captain's Chair console.
- Border sprites swap instantly to their alert-variant textures.
- A custom `UiMaterial` shader renders a radial-gradient red vignette behind the border; intensity ramps in over ~0.25s, pulses on a 1.3s sine between 0.55 and 1.0 while active, and eases off over ~0.25s when stood down.
- Designation and status values switch from signal-cyan (`#5fd8e8`) to alert-red (`#ff3344`); labels stay neutral (`#b8c0c8`).
- All phone consoles also gain a red border around their UI when Red Alert is active.

### Asteroid destruction effect
- A radial ripple plays on the viewscreen when an asteroid is destroyed.

### Phaser beam rendering
- An active phaser beam is rendered on the viewscreen as a line/glow from the ship to the target while it persists.

### Debug overlay (developer)
- A JS-rendered `<div>` overlay reads `wasm_get_debug_state()` once per frame when active and renders three labelled sections: Flags, Float Modifiers, Int Modifiers, each entry showing value and source(s).
- F3 toggles the debug overlay; F4 toggles region wireframes. Both `keydown` listeners call `preventDefault()` to suppress browser defaults.
- Region wireframes render on the viewscreen and on every radar when enabled; never reach clients without the toggle.

---

## Captain's Chair

- The captain has no console-side mechanics beyond directing the bridge.
- Red Alert toggle button. Drives the viewscreen alert visuals and the per-console red border.
- View selector: four directional buttons arranged as a compass cross (Fore top, Aft bottom, Port left, Starboard right), with a "View" label in the centre cell. The active direction is highlighted. Pressing any direction button returns the viewscreen to a camera view even if Helm or Science had pushed Radar / SystemChart / ScienceRadar.
- Read-only mission objectives summary appears on the captain's panel, sorted with mandatory objectives first. Updated only when objectives change.
- The captain's console state persists across brief disconnects.

---

## Helm

### Physics and controls
- Thrust slider (0–100%) sets target forward speed.
- Steering joystick (snap-to-centre on release) sets angular velocity.
- Helm sends `HelmInput { thrust, steering }` at 10 Hz while controls are active.
- Ship velocity lerps toward target at `max_acceleration` (~3s to max from rest); on zero thrust, decelerates at `max_deceleration` (~1s stop).
- Ship is locked to the XZ plane with yaw-only rotation (no pitch, no roll).
- Direct velocity control each tick on a dynamic Rapier rigid body; asteroids are static rigid bodies.
- All physics constants (max forward speed, max reverse speed, acceleration, deceleration, max yaw rate) are loaded per-ship from `[helm_console]` in the ship TOML.

### Collision
- Asteroid impact zeroes the ship's forward velocity and applies damage to the per-console hull pool (scaled by impact speed via the shared damage helper).

### Helm radar
- A 2D radar panel renders ship-aligned (ship at centre, heading indicator pointing along the ship's forward axis), with asteroids as circles scaled to actual size, plus an outer range ring (50 units) and a mid ring (25 units).
- The radar takes ~90% of the narrowest screen dimension and is shown simultaneously with the joystick.
- A shared radar module produces the same visual whether rendered on the phone or pushed to the viewscreen.

### On-Screen
- An "On Screen" button sends `SetView { Radar }`, pushing the radar to the viewscreen. Any subsequent captain direction press returns the viewscreen to a camera.

### Impulse drive
- Helm requests an impulse charge (`StartImpulseCharge`).
- Charge time defaults to 6 seconds, cancelable.
- Cancellation triggers: damage during charge, Science cancel command, or entering a `blocks_impulse` region.
- Active impulse multiplies effective `MaxSpeed` (via the modifier system) and disables steering (yaw input is ignored).
- A `blocks_impulse` region on entry cancels charging or active impulse and prevents new charges while inside.

---

## Tactical (Weapons)

### Targeting and radar
- A Tactical radar projection (ship-aligned) shows ships, stations, asteroids, and torpedoes within range. The radar `shows` list is configurable per ship in TOML.
- Asteroids are colour-coded: yellow within target range, pulsing red within fire range.
- Tap-to-lock sends `SetTarget { uuid }` to the server.
- Target lock is 360° within target range (default 60 units); fire requires both 40-unit range AND a 180° forward arc check at the moment Fire is pressed (not at lock time).
- Server validates target exists and is within the configured target range, then responds with `TargetLock`. Server also tells client when the locked target enters/leaves fire range so the Fire button can light up.
- Per-target indicator on the Tactical radar: a Science target suggestion (advisory only — see Science) is highlighted on the Tactical radar.

### Phaser banks
- The ship has **two independent phaser banks**, port and starboard, each with its own cooldown timer and fire arc. Both banks share a single firing-mode setting.
- Each bank has a 270° fire arc (a 90° blind cone is on the opposite side: the port bank's blind cone is pure starboard, and vice versa). Within that arc, a narrower 240° auto-fire arc (30° margin from the fire-arc edge) governs Auto-mode shots.
- All bank parameters are configurable per ship: `cooldown_secs` (default 3s between shots per bank), `auto_fire_range` (default 40 units), `fire_arc_deg` (default 270), `auto_arc_deg` (default 240), plus beam colour and `beam_range`.
- A `SetPhaserMode { mode }` message switches between `Auto` (banks fire automatically when a target is in arc, in range, and off cooldown) and `Manual` (operator must press Fire). Mode is shared across both banks.
- Manual fire: the Fire button is gated on the locked target being in fire range AND in at least one bank's 270° arc AND that bank being off cooldown.
- Auto fire: an off-cooldown bank fires whenever a locked target is inside the 240° auto-fire arc and within `auto_fire_range`.
- Each shot is an instantaneous beam: `BeamStarted { target_uuid }` → damage applied → `BeamEnded { target_uuid }` plus a `PhaserFired { bank, target_uuid }` for the renderer. The bank's cooldown starts when it fires.
- Beam severs immediately if any of: target destroyed, target leaves fire range, target leaves the bank's arc.
- No damage refund on sever. Multiple beams from different banks can be active simultaneously, but each bank can only have one beam active at a time.

### Phaser frequency tuning
- Tactical (Full complexity) sees phaser frequency buttons (alpha / beta / gamma) to match the locked target's shield frequency.
- A matching frequency applies a damage multiplier via the modifier system.
- Frequency may be delegated to the Sensors operator (see Console Complexity) or operated by AI.

### Torpedoes
- Multiple tubes per ship (e.g. ForePort, ForeStarboard, Aft), each with its own reload cycle, magazine, and arc.
- Tubes load from a shared magazine.
- Torpedoes are fired by selecting a tube and pressing Fire (or by AI auto-fire at Low complexity).
- AI auto-fire fires only when the target's facing-quadrant shield is down (or the target has no shields) AND a tube is loaded in arc AND magazine > 0; deterministic tube priority.
- Torpedoes appear on radars as entities.

---

## Sensors, Shields, and Navigation (the former "Science" role)

What the early PRDs called the Science console has been split into **three independent consoles**, each held and played as its own role. They share the goal of giving the bridge situational awareness and defensive control but they don't share a tabbed UI — each is a separate panel.

### Sensors
- Long-range radar showing entities with configurable tags (ships, stations, asteroid fields, regions, etc.).
- Asteroid fields render as donut-shaped rings (inner/outer radius); regions render as their `RegionShape` footprint.
- Tapping an entity sends `SetSensorsTarget { uuid }`. The server broadcasts `SensorsTargetSuggestion { uuid }` so Tactical sees the suggestion on their radar (advisory only — no mechanical coupling).
- Range scales with Sensors power level via the modifier system.
- The legacy `SetScienceTarget` / `ScienceTargetSuggestion` wire pair is retained alongside `SetSensorsTarget` / `SensorsTargetSuggestion` for back-compat (both behave identically).
- Shield-frequency readout (Full complexity): the locked target's shield frequency (alpha/beta/gamma) is shown so Tactical (or Sensors via delegation) can match phaser frequency. Hidden at Low complexity; targeting still works.
- Push to viewscreen: Sensors can push its long-range radar via `SetView { SensorsRadar }` (and the legacy `ScienceRadar` mode is still accepted).

### Shields
- A 2D top-down ship diagram with shield arcs (default fore/aft/port/starboard) as pie slices.
- Each arc shows current HP as text and fills from centre to edge based on shield-charge percentage.
- Shield-arc count and per-arc HP/regen/offline are ship-configurable.
- The Shields operator can focus a single arc to redistribute strength across the shield system (see Shields System → Focus mechanic).

### Navigation
- System chart: a navigational view showing stars, planets, asteroid field rings, and the ship's position at galactic scale. Non-interactive for targeting.
- Impulse cancel: Navigation sends `CancelImpulse` to abort a charging or active impulse drive. Helm requests charges; Navigation is the cancel authority.
- Push to viewscreen: Navigation can push the system chart via `SetView { NavigationChart }` (and the legacy `SystemChart` mode is still accepted).

---

## Shields (System)

- Up to four shield facings (default fore/port/aft/starboard) as pie-slice arcs around the ship; arc count and angles are configurable per ship (e.g. fewer arcs for lightweight ships, with built-in label sets for 1/2/3/4 arcs).
- Facings are indexed counter-clockwise from forward (Fore = 0 → Port → Aft → Starboard).
- Each facing has configurable HP, regen rate, and offline duration.
- Damage is applied to the facing nearest the attacker bearing (bearing relative to ship yaw determines hit facing).
- A facing's HP regenerates passively while it has HP > 0.
- A facing that drops to 0 goes offline for its `offline_duration`; while offline, full damage passes to the per-console hull pool. When the offline timer expires the facing comes back at full max HP.
- Shield-facing state is broadcast as `ShieldStatus { facings: Vec<ShieldFacingStatus> }` (10 Hz or on-change) and rendered by the Shields console.
- Shield charge is independent of the Shields console's own HP (see Damage & Hull): a damaged Shields console means the operator cannot operate shield controls but remaining shield charge still absorbs damage normally.

### Focus mechanic
- The Shields operator can elect to **focus** one facing using `SetShieldFocus { facing: Option<ViewDirection> }`. `None` clears focus.
- Focusing redistributes shield strength rather than freely adding to the system: the focused facing gains `bonus_max_hp` extra capacity and `bonus_regen` extra regen per second, while every other facing loses `penalty_max_hp` capacity and `penalty_regen` regen.
- Non-focused facings whose current HP is now above their reduced effective max decay toward the new cap at `decay_rate` HP/s. Regen on a decaying facing is suppressed until it reaches the cap, so the transition is gradual rather than snapping.
- Clearing focus restores every facing's base `max_hp` and `regen_per_sec`; HP above the original cap (e.g. on the previously focused arc) decays back the same way.
- Focus configuration (`bonus_max_hp`, `bonus_regen`, `penalty_max_hp`, `penalty_regen`, `decay_rate`) is ship-configurable; defaults are +50 / +5 / −25 / −2.5 / 10 HP/s.
- Each broadcast `ShieldFacingStatus` carries an `is_focused: bool` flag so consoles can highlight the focused arc.

---

## Power

### Layout and controls
- Three rows: Helm, Tactical (Weapons), Sensors. Each row shows current level (1–4) plus increment and decrement buttons. (The Sensors row inherits what older PRDs called the "Science" allocation; Shields and Navigation are not separately powered.)
- Total budget: 6 base points + up to 2 from the auxiliary battery (cap 8).
- A battery charge bar and percentage display sit alongside the rows.
- Increment is disabled when total allocated = 8 or when locked. Decrement is disabled when a console is at 1 or when locked.

### Battery economics
- Battery rate table indexed by total allocated points: 3=+6.0/s, 4=+5.0/s, 5=+4.0/s, 6=+2.0/s, 7=−2.0/s, 8=−6.0/s (configurable per ship).
- Exhaustion: battery hits 0 → all consoles forced to 1, controls locked, battery recharges at the maximum rate.
- Unlocks once the battery reaches `emergency_threshold` (default 25%).

### Modifier wiring
- Helm power → `ModifierSlot::MaxSpeed` AND `ModifierSlot::MaxYawRate`.
- Tactical power → `ModifierSlot::PhaserDamage`.
- Sensors power → `ModifierSlot::RadarRange`.
- Per-level bonus table per console, configurable per ship; level 2 is baseline (0.0 bonus = 1.0× multiplier), level 1 is half performance (−0.5), level 4 is +0.5 (1.5×).

### `PowerState` broadcast
- `PowerState { helm, weapons, sensors, battery_charge, locked }` is sent to the Power holder at 10 Hz.
- `SimSnapshot.power_levels` carries Helm/Tactical/Sensors levels (as a `(u8, u8, u8)` tuple) to all clients so other consoles can present their power state.

---

## Repair

### Direct repair team dispatch
- Repair-team count is configurable per ship (default 2). Each team has its own row showing a status label, a progress bar, and four console-targeting buttons (Helm, Tactical, Power, Shields).
- The four console-targeting buttons sit **under each team's own row**, so the operator chooses both which team and which console. Wire shape: `DispatchRepairTeam { team_idx: u8, console }`.
- Targeting buttons are always visible — including when the console is undamaged — so a Repair operator can accidentally waste a team. Crew communication is required to avoid waste.
- **Redirect and recall.** A team in any non-Idle state can be reassigned:
  - Clicking a *different* console while a team is `Travelling` or `Repairing`: the team transitions immediately to `Returning` with the new console queued. On reaching engineering the team auto-dispatches to the queued console without a second click.
  - Clicking the *current* console while a team is `Travelling` or `Repairing`: cancels and recalls the team (queue = None; team returns to Idle on arrival).
- **Return time mirrors travel time.** If a team is recalled or redirected while `Travelling`, its return takes exactly as long as it has already spent travelling — the progress bar drains back from the current position at the same rate it filled. A team recalled from `Repairing` always takes the full travel time to return (it is at the far end of the ship regardless of how long it was repairing).
- Any HP restored before a recall remains on the console — partial healing is not reversed.

### Team state machine
- `Idle` → `Travelling { console, elapsed }` → `Repairing { console, elapsed }` → `Returning { remaining: f32, queued: Option<Console> }` → `Idle`.
- Travel time: 5 seconds each way.
- Repair rate: 1 HP per 2 seconds, applied while `Repairing`.
- Auto-return: when the target console reaches full HP, the team transitions to `Returning { remaining: TRAVEL_DURATION, queued: None }`.
- Immediate-arrival return: a team dispatched to a console already at full HP transitions to Returning immediately on arrival (round trip ≈ 10 seconds wasted).
- On redirect mid-`Travelling` with elapsed `t`: `Returning { remaining: t, queued: Some(new_console) }`.
- On redirect mid-`Repairing`: `Returning { remaining: TRAVEL_DURATION, queued: Some(new_console) }`.
- On completing `Returning` with `queued = Some(c)`: auto-dispatch to `Travelling { console: c, elapsed: 0 }`.
- On completing `Returning` with `queued = None`: transition to `Idle`.
- Teams are immediately `Idle` after `Returning` completes — no cooldown.

### Repair UI extras
- Total ship hull as current/max is displayed on the Repair console (and on the viewscreen HUD).
- A green glow on a console's HP bar (on the operator's panel) indicates a repair team is currently `Repairing` that console.

### Damage cues
- Each console tab shows a red dot when its console HP < max. The dot clears the moment the console returns to full HP.

---

## Damage & Hull

### Per-console hull points
- Four consoles carry their own HP pool: Helm, Tactical, Power, Shields. (Comms HP is deferred until Comms ships.)
- Default 25 HP per console, configurable per ship.
- Damage that passes through shields is applied to the per-console pool. The damage helper picks a console weighted by current HP (a console with more HP is proportionally more likely to absorb the next hit), with overflow spilling into a fresh weighted draw against the remaining non-empty consoles. Consoles at 0 HP are never targeted.
- A console at 0 HP becomes unresponsive: all input is disabled and an OFFLINE banner / dimmed-screen indicator is shown. The console panel remains visible (not hidden) so the player can still see its state.
- Damage source is not discriminated (weapons vs collisions vs regions all use the same pool).
- Console HP and the system function it controls are independent — a 0-HP Helm console disables the panel but does not directly degrade the underlying ship physics; gameplay impact comes from the operator losing control.

### Total hull display
- `ConsoleHull::total_current()` / `total_max()` aggregate across all per-console pools.
- The viewscreen HUD shows hull as current/max.

### Game over
- A `GameOver` state with optional reason carries the message and triggers an end-of-game screen.
- Triggered automatically when all damageable consoles reach 0 HP (default reason `"All consoles destroyed"`).
- Triggered by the `game_over { message? }` scenario action with the supplied message.
- Transition is instant. No animated death sequence.

### NPC and station hull
- NPC ships and asteroids use the same `ConsoleHull` component as the player ship. For NPCs the data convention is: a single `CaptainChair` console entry holds all the entity's HP (e.g. `captain_chair = 60`); no HP is assigned to Helm, Tactical, or Shields console entries. This means a NPC destroyed when `CaptainChair` HP reaches 0.
- NPC entities that have a `[weapons_console]` section in their config get a `WeaponsConsole` component, which gates `entity_phaser_ready` and related `WorldView` fields. Same pattern for `[helm_console]` → `HelmConsole` and `[shields_console]` → `ShieldsConsole`. Sensors, Navigation, Comms, Power, and Repair console components are not expected on NPC entities.
- Asteroids use `ConsoleHull` with a single pool (not per-console); phaser fire and collision both route damage through the shared damage helper.
- Phaser fire damage is applied to all hull-bearing entities in range (NPCs, stations, asteroids) — not only asteroids.

### Collision damage
- Asteroid impacts kill ship forward velocity to zero and apply damage via the shared helper, with damage scaled by impact speed.

### Hull as a fractional value
- Hull integrity is `f32` end-to-end across the simulation; clients round for display.
- Fractional damage from continuous sources (e.g. damage zones, beam DPS) accumulates correctly across ticks.

---

## Comms

### Console layout
- Two-panel UI on the Comms phone:
  - Left: incoming message list, each item showing sender and subject.
  - Right: when a message is selected, the full chat-style exchange with a stack of predefined response buttons.
- Free-text replies are not supported; responses are picked from a predefined list per dialogue node.
- An "incoming message" stream supports both scenario-pushed messages and Comms-initiated hails.

### Hailing
- Comms officer can hail any entity in the contact list. `Hail { target_uuid }` initiates the exchange.
- AI ships can be hailed but produce no automated response (deferred to a later PRD).

### Objectives
- An objectives panel beside the message list shows current mission objectives ordered with mandatory first, optional below.
- Each objective shows status: active, completed, failed.
- Completed and failed objectives remain visible until the Comms officer clears them.
- The captain receives a read-only `ObjectiveSummary` on objective changes (event-driven, not 10 Hz).

### Orphaning and cleanup
- Messages from a scenario that unloads remain visible but marked "transmission ended" with response buttons disabled.
- "Clear all" wipes read and orphaned messages in one action.
- Cleared objectives leave a debrief record until the operator clears them as well.

### Push to viewscreen
- Comms can push an active message to the viewscreen via `SetView { Comms }`. The captain can override back to a camera view at any time.

### Wire shape
- `CommsState { messages, objectives, contacts }` is pushed event-driven (only when state changes), not on a 10 Hz schedule.

---

## AI & Behaviour

### Architecture
- AI controllers are architecturally separate from the entities they drive. A controller emits the same input messages a human player would (`HelmInput`, `SetTarget`, `FirePhaser`, `FireTorpedo`, `Hail`, `RespondToMessage`); the simulation cannot tell AI inputs from player inputs.
- Each AI-controlled entity has a synthetic token `ai:<entity_uuid>` registered in an `AiTokenRegistry`. The simulation's token-to-entity lookup falls back to this registry when the player session lookup misses.
- The AI tick is a pure function: `tick(controller, world_view) -> AiTickOutput { new_state, inputs }`. No Bevy dependency.
- `WorldView` is populated from the entity's console components: `WeaponsConsole` presence populates phaser-ready and weapons-range fields; `ShieldsConsole` presence populates shield fields; `ConsoleHull` populates `self_hull_fraction`. A console component absent means the corresponding `WorldView` field is `None` / `false` — the AI cannot attempt that action.
- AI weapon and shield outputs are routed through the **same shared server-side systems** as player inputs (same phaser cooldown, range, and damage resolution). The AI emits `FirePhaser` / `FireTorpedo` outputs; the server processes them identically to player fire commands.
- `FactionRegistryResource` is **target-agnostic** (no `#[cfg(wasm32)]` gate). It is built at app startup from faction TOML data on both WASM and native targets.

### Configuration
- Any entity with a `[behaviour]` block in its config is automatically driven by a state machine controller.
- States and transitions are declared per entity-config; per-spawn `[spawn.overrides]` can change `initial_state`, individual states' parameters (e.g. waypoints), or replace the whole transitions list.
- Each controller has a fixed five-slot blackboard: `target`, `last_attacker`, `home_position`, `waypoint_index`, `state_entered_at`.

### State vocabulary
- `idle` — emits nothing.
- `patrolling { waypoints, loop_path, target_speed }` — steers toward the current waypoint, advances on arrival; loops or stops based on `loop_path`.
- `pursuing { target_speed }` — steers toward target; no-op when target is None.
- `attacking { maintain_range, target_speed }` — steers toward target; thrust=0 within `maintain_range`, else `target_speed`. Fires phasers in beam range. Fires torpedoes when the target's facing-quadrant shield is offline/depleted (or freely for shieldless targets).
- `fleeing { target_speed }` — steers 180° from `last_attacker`; falls back to hold-heading if absent.
- `warping_out { duration_secs, target_speed }` — holds current heading at `target_speed`; despawns after `duration_secs`. A projected exit point is derivable client-side from `position + velocity * remaining_secs` and rendered as a vertical circle.
- All states accept `target_speed` in `[0.0, 1.0]`.

### Transition vocabulary
- `on_attacked` — fires once per state-entry, sets `target` and `last_attacker`.
- `on_scenario_unloaded` — fires on scenario teardown.
- `on_timer { seconds }` — fires after a duration in the current state.
- `in_weapons_range` — reads target and weapons config.
- `hull_below { threshold }` — fires when hull fraction drops below threshold.
- `target_destroyed` — fires when the controller's target is removed from the world.
- `enemy_in_range { radius }` — finds the first enemy via factions and sets `target`.

Transitions are evaluated in declaration order; first match fires. `from` accepts either a single state name or a list (string-or-vec serde deserializer).

### Faction system
- Each faction has a UUID, a name, and a list of enemy faction UUIDs.
- Entity configs carry an optional `faction` field referencing a faction UUID. At spawn time this is attached as a `Faction(Uuid)` component on the entity — it is not looked up at AI tick time.
- `is_enemy(a, b)` predicate is the only consumer in v1; hostility is asymmetric by construction.
- Factionless entities are neither enemies nor targets.

### Edge-triggered emission
- AI ticks at simulation rate but `HelmInput` emissions are filtered by epsilon (~0.02) on each field so AI does not flood the message queue. `SetTarget` / `FirePhaser` / `FireTorpedo` are event-shaped.

### Phase gating
- AI tick systems run only during `InProgress`.
- AI controllers are minted fresh per session; AI state is not persisted by Save/Load.

### Lifecycle
- On spawn (entity with `[behaviour]`): AI plugin attaches an `AiController`, mints a token, registers it, seeds the blackboard (`home_position = spawn_position`, `current_state = behaviour.initial_state`).
- On despawn: token is unregistered; in-flight messages from the dead token are silently dropped.

---

## Space Stations

- Stations are persistent world entities with a `[shape]` (sphere, cylinder, torus) rendered as a `Mesh3d` on the viewscreen, hull integrity, collision, and tags.
- Stations appear on Helm radar, on Tactical radar (targetable), and on Science long-range radar/system chart per their tags.
- Stations can be hailed by Comms.
- Stations take damage and can be destroyed; on destruction the scenario engine can fire `on_destroyed` triggers.
- Station spawn/despawn flow through the generic entity-lifecycle channel (`EntitySpawned { snapshot }` / `EntityDespawned { uuid }`) — there is no station-specific wire message. Reconnecting clients receive the current world state via a live ECS query at send time.

---

## Modifier System

### Float modifiers (multipliers)
- Central `ShipModifiers` resource holds a fixed set of named `ModifierSlot`s with a pre-computed cached multiplier per slot.
- Slots: `MaxSpeed`, `MaxYawRate`, `RadarRange`, `PhaserDamage`, `HullDamageTaken`, `RepairRate` (extensible).
- `Modifier { source, slot, bonus: f32 }` is the value-type; identity key is `(source, slot)`. Adding the same source+slot replaces.
- Multiple sources on the same slot stack additively. Sum `s ≥ 0` → cache `1.0 + s` (buff). Sum `s < 0` → cache `1.0 / (1.0 + |s|)` (debuff; never reaches zero).
- O(1) lookup via cached array.
- Empty cache values are 1.0.

### Integer modifiers
- Server-internal `IntModifierSlot` enum with a parallel integer cache. First variant: `RepairTeams`.
- `IntModifier { source, slot, bonus: i32 }`. Cache value is the straight sum.
- Used for discrete quantities that cannot be modelled as a multiplier (e.g. scenario grants an extra repair team).

### Flags (boolean)
- `ShipModifiers` carries a `HashMap<FlagKind, HashSet<ModifierSource>>` for OR-aggregated boolean flags.
- `FlagKind` variants: `CommsJammed`, `SensorBlind` (extensible).
- `add_flag(source, kind)` / `remove_flag(source, kind)` / `has_flag(kind)` / `flags()`.
- A flag is set iff its source-set is non-empty. Multiple sources of the same flag stack via OR; removing the last source clears the flag.

### Modifier sources
- `Console(Console)` — power levels register here.
- `ImpulseDrive` — impulse registers here.
- `RegionEffect { uuid: Uuid }` — regions register here, keyed by region UUID.
- `Scenario { id: String, tag: String }` — scenario actions register here; `(id, tag)` pair is the identity key.

### Broadcast
- `ServerMessage::ModifierAdded` / `ModifierRemoved` broadcast on each table change.
- `SimSnapshot.flags` carries the active flag set per tick.
- `SimSnapshot.radar_state` carries effective radar ranges per tick (resolved from base × multipliers).
- Integer modifiers are server-internal in v1 (not in `SimSnapshot`).

---

## Networking & Wire Protocol

### Transports
- WebRTC via PeerJS (browser deployment, star topology).
- WebSocket via axum (native deployment).
- Both transports carry the same `ClientMessage` / `ServerMessage` JSON, with `Identify { token, name }` as the first frame.

### Identity
- Session token (UUID v4) stored in client `localStorage`. Server maps tokens to player records and re-attaches reconnecting players to their previous station/spectator slot per the station reassignment rules.
- PeerJS peer IDs and WebSocket connection IDs are ephemeral; identity is solely token-based.

### Routing
- Outbound targets: `All`, `Token(token)`, `AllExcept(token)`.
- Per-console payloads use `Token` routing.

### Message types

#### Inbound (`ClientMessage`)
- Identity / lobby: `Identify`, `SetName`, `SelectStation`, `ReleaseStation`, `StartGame`, `SetComplexity`.
- Viewscreen / captain: `SetView`, `ToggleRedAlert`.
- Helm: `HelmInput { thrust, steering }`, `StartImpulseCharge`, `CancelImpulse` (note: `CancelImpulse` is sent by the Navigation operator, not Helm).
- Tactical / weapons: `SetTarget`, `FirePhaser`, `SetPhaserMode { mode }`, `SetPhaserFrequency { frequency }`, `FireTorpedo { tube, target_uuid? }`.
- Sensors: `SetSensorsTarget { uuid }` (current) and `SetScienceTarget { uuid }` (legacy, retained).
- Shields: `SetShieldFocus { facing: Option<ViewDirection> }`.
- Power: `IncreasePower { console }`, `DecreasePower { console }`.
- Repair: `DispatchRepairTeam { team_idx: u8, console }` — the operator selects both the team and target console. A team in any non-Idle state can be redirected or recalled (see Repair section for queue and return-time mechanics).
- Comms: `Hail { target_uuid }`, `SelectCommsMessage`, `RespondToMessage`, `ClearComms`.

#### Outbound (`ServerMessage`)
- Lobby / lifecycle: `Welcome { state, ship_stations }`, `PlayerJoined`, `PlayerLeft`, `NameChanged`, `StationAssigned`, `GameStarted`, `WorldSetup`, `ComplexityChanged`, `GameOver { reason }`.
- Continuous state: `SimState { snapshot }` (10 Hz, carries the `SimSnapshot`).
- Entity lifecycle: `EntitySpawned { snapshot }` and `EntityDespawned { uuid }` are the **generic runtime delta channel** for every UUID-bearing world entity (stations, regions, stars, planets, asteroid fields, the ship). A single tracker scans the ECS each tick and emits these messages for any entity that has appeared or disappeared since the previous tick.
- Asteroid lifecycle: `AsteroidSpawned { uuid, x, y, z, config_path, max_hp, current_hp }` and `AsteroidDestroyed { uuid }` are emitted **in parallel to** the generic channel, by the asteroid ring-buffer window system rather than the generic tracker. They are retained as a specialised channel because they carry per-asteroid HP/config-path fields that `EntitySnapshot` does not, and because the window's spawn/despawn cadence is decoupled from the generic tracker.
- Tactical / weapons: `TargetLock`, `WeaponsUpdate` (per-tick to Tactical: target uuid, fire-ready, cooldown, torpedo magazine count, per-tube loaded/reload state), `BeamStarted`, `BeamEnded`, `PhaserFired { bank, target_uuid }`, `TorpedoLaunched`, `TorpedoDestroyed`, `FrequencyHint { frequency }` (sent to Tactical when a Sensors-Low / AI hint fires), `SensorsTargetSuggestion` (current) and `ScienceTargetSuggestion` (legacy, retained).
- Shields: `ShieldStatus { facings: Vec<ShieldFacingStatus> }`.
- Repair: `RepairState { teams }`.
- Power: `PowerState { helm, weapons, sensors, battery_charge, locked }`.
- Comms: `CommsState { messages, objectives, contacts }`, `ObjectiveSummary { objectives }` (captain only, event-driven).
- Damage / feedback: `DamageTaken { hull, shield }`, `ShipDestroyed` (one-shot, fired in lockstep with the `GameOver` phase transition when all per-console hull pools reach 0).
- Modifiers: `ModifierAdded`, `ModifierRemoved`.

> **Note on superseded messages.** `StationSpawned { uuid, name, position, shape, radius, hull_integrity }` and `StationDestroyed { uuid }` exist in the `ServerMessage` enum and the codec round-trip tests but are **never emitted by production code** — stations flow through the generic `EntitySpawned` / `EntityDespawned` channel. They should be considered legacy scaffolding pending removal. `ShipDestroyed` is similarly redundant with `GameOver { reason }` but kept as a one-shot signal alongside the state transition.

#### Phase enum
- `GamePhase`: `Lobby`, `InProgress`, `GameOver` (derives `States` for Bevy state-driven scheduling).

### Snapshots
- `Welcome` includes the current `GameState` (phase, players, current station assignments, complexity presets) plus `ship_stations` (parsed config for client-side rendering) plus a `world: Option<WorldData>` (None in lobby, Some in InProgress; built from a live ECS query at send time so reconnecting/late-join clients receive the current state).
- `WorldData.entities: Vec<EntitySnapshot>` — every world entity with optional aspect fields (`uuid`, `id`, `position`, `tags`, `shape?`, `radius?`, `colour?`, `yaw?`, `hull_fraction?`).
- `SimSnapshot` (10 Hz) carries: ship pose (x, z, yaw), helm view mode, hull (per-console + total), power levels, view mode, modifier flags, radar state, dynamic entity states (`entity_states: Vec<EntityStateSnapshot>`).
- Clients are idempotent on `EntitySpawned`/`EntityDespawned` (already-have-uuid → ignore on spawn; don't-have-uuid → ignore on despawn).

### Codec
- Serialisation goes through a single `MessageCodec` trait. Production uses `serde_json`. The interface is the only surface the rest of the codebase touches.
- `save.rs` is the second and only other sanctioned `serde_json` surface.
- All other TOML/serde traffic uses the `toml` crate.

---

## Configuration & Authoring

### World TOML (`assets/worlds/default.toml`)
- `seed` (global) plus a list of `[anchors]`, a list of `[[entity]]` instances (the map half: immediate or game-start spawn, optional `overrides`), a list of named `[[spawn]]` entries (the scenario half: anchor/relative_to/absolute positioning, UUID-assigned, trigger/comms-eligible), `[[trigger]]` reactions, `[[comms]]` dialogue templates, and `[[objective]]` entries. PRD #337 will collapse `[[entity]]` and `[[spawn]]` into one block type.

### Entity TOML (`assets/entities/*.toml`)
- Component-bag: each `[section]` present produces a Bevy component on the spawned entity.
- Console-as-feature-flag: `[captain_console]`, `[helm_console]`, `[weapons_console]`, `[sensors_console]`, `[shields_console]`, `[navigation_console]`, `[repair_console]`, `[power_console]`, `[comms_console]` presence determines which consoles a ship exposes. (`[science_console]` from earlier PRDs is replaced by separate `[sensors_console]` / `[shields_console]` / `[navigation_console]` sections.)
- Physical: `[hull]`, `[collider] kind="capsule|sphere" radius length`, `[appearance]`, `[shape]` (for regions: Sphere/Box/Torus).
- Helm console parameters: `[helm_console]` — max forward/reverse speed, acceleration, deceleration, turn speed, plus `[helm_console.radar] range, shows = [tags...]`.
- Tactical console parameters: `[weapons_console]` — per-bank `cooldown_secs`, `auto_fire_range`, `fire_arc_deg`, `auto_arc_deg`, beam colour, plus torpedo tube config and `[weapons_console.radar]`.
- Sensors console parameters: `[sensors_console.long_range_radar] range, shows`, plus power-multiplier sub-table.
- Shields console parameters: `[shields_console]` — focus bonus/penalty/decay (`bonus_max_hp`, `bonus_regen`, `penalty_max_hp`, `penalty_regen`, `decay_rate`).
- Navigation console parameters: `[navigation_console.system_map] range, shows`.
- Repair console parameters: repair rate, HP per cycle, repair cooldown, team count.
- Shields: `[shields] default_hp, default_regen_rate, default_offline_duration, [[shields.arcs]] start_angle, end_angle, hp`.
- Impulse: `[impulse] speed_multiplier, charge_time`.
- Power: `[power] capacity, rates, emergency_threshold` plus `[<console>.power_multipliers]` per-level bonus tables.
- AI: `[behaviour] initial_state, [[behaviour.state]] name, params, [[behaviour.transition]] from, condition, to`.
- Faction: `faction = "<uuid>"` references `assets/factions/<name>.toml`.
- Region effects: `[effects.blocks_impulse] / [effects.radar_dampening] / [effects.damage_zone] / [effects.slow_zone] / [effects.comms_jammed] / [effects.sensor_blind]` per template.
- Stations: `[stations] min_players, max_players, [[stations.config]] player_count, [[stations.config.station]] name, description, consoles, next?, previous?`.

### World TOML (`assets/worlds/*.toml`)
- `title`, `description` — lobby display.
- `preload = [...]` — entity paths to fetch before spawning begins.
- `[[entity]] template_path, id?, position, spawn_on (immediate | game_start), [entity.overrides]` — static layout instances.
- `[[spawn]] template, position (anchor name | absolute | entity-relative), id?, shape?, [spawn.overrides]` — named, trigger-eligible spawns.
- `[[trigger]] condition, entity?, actions`.
- `[[comms]] from, message, trigger, [[comms.responses]] text, actions` with inline `follow_up` branching.
- `[[objective]] id, text, optional`.

### Complexity TOML (`assets/complexity/<console>.toml`)
- Per-console file declaring presets, each with: `hidden_elements`, `delegated` (per receiver console), `ai` (per-behavior config block with tuning numbers).

### Faction TOML (`assets/factions/<name>.toml`)
- `uuid`, `name`, `enemies = [<faction_uuid>, ...]`.

### Loading flow
- Browser: JS fetches the world TOML, calls `wasm_load_world(path, toml_str)` (a single call that internally hands the same TOML to both the map and scenario parsers — see PRD #337), drains the config-request callback for entity/faction TOML files, then calls `wasm_init` when `wasm_load_config` returns `Ok(true)`.
- Native: synchronous `std::fs` reads inside `native_config_loader` populate the config cache before the Bevy app starts.

### Static assets
- `assets/worlds/`, `assets/entities/`, `assets/factions/`, `assets/complexity/`, `assets/viewscreen/` (border PNGs), `assets/fonts/` (Chakra Petch, JetBrains Mono), `assets/shaders/` (WGSL).
- Trunk `<link data-trunk rel="copy-dir" .../>` directives copy these into `dist/` for both `trunk serve` and `trunk build --release`.

---

## Smoke Testing

- Playwright-based smoke harness runs in Chromium against `dist/` served by `npx serve`.
- A BroadcastChannel PeerJS shim is injected via Playwright's `addInitScript` to replace `window.Peer` with a fake transport — zero production-code footprint.
- A `wasm-ready` window event from the shim signals reliable WASM readiness; tests then wait ~500ms before the first `Identify` to let Bevy startup systems complete.
- Smoke specs cover: server.html loads + WASM initialises without console errors; client connect + Identify + Welcome handshake; station picker (replaces old console picker) including SelectStation/ReleaseStation, atomic swap, captain validation; reassignment cascades (2→3 join, 3→2 leave, spectator promotion); StartGame all-clients-receive-GameStarted; first SimState within 2s; HelmInput → next SimState reflects change; complexity broadcast; comms flow; AI patrol → pursue transition; regions render on Science radar; damage zone reduces hull; modifier flag at the wire level; debug-overlay toggle; native WebSocket transport reaches Connected.
- Three additional smoke tests are written co-located with the features they validate: **tactical fire-flow** (phaser fires, hits hull-bearing target, damage applied — written alongside the phaser/NPC damage fix); **helm input/physics** (thrust and yaw inputs produce expected position and heading changes — written alongside the impulse data-driven fix); **view-selector** (switching view modes produces correct `SimSnapshot` view-mode fields — written alongside view-mode work).
- Smoke tests run automatically on every pull request and push to `main`; depend on the build job's `dist/` artifact (no recompile).
- The smoke test CI job is the required status check for `main`.

---

## Build & CI

### Crate layout
- Single Rust crate with three mutually exclusive Cargo features: `server` (WASM view screen, default), `client` (WASM phone console), `native` (PC binary + axum/tokio).
- `wasm-bindgen` is only present for the WASM features; the native build uses plain Rust types.
- The `webgl2` Bevy feature is included for `server`/`client` only; native uses the native wgpu backend.

### Entry points
- `server.html` (Trunk) → WASM view screen. PeerJS chrome stays as plain HTML/CSS/JS (fullscreen button, connection-status dot, QR code, save-slot selection screen).
- `client.html` (Trunk) → WASM phone console; identical JS chrome to `server.html` for fullscreen, status dot, and name input.
- Native `[[bin]]` target compiled only when `native` is active.

### CI workflows
- `deploy.yml` — Trunk build → push to `gh-pages` on `main`.
- `smoke-test.yml` — Playwright suite on PR and `main`, downloads `dist/` artifact from the build job.
- `release.yml` — produces native zips for Windows, macOS, and Linux per release tag (binary + bundled cloudflared + `dist/client/`).

### Asset handling
- Trunk `copy-dir` directives handle static assets (TOMLs, viewscreen sprites, fonts, shaders).
- The `dist/client/` path served by the native HTTP server is configurable via CLI arg or adjacent-to-exe convention so dev and release layouts both work.

---

## Out of Scope (across all PRDs)

The following appear as explicit non-goals in one or more PRDs and remain out of scope unless a future PRD re-opens them:

- Spectator / observer mode beyond the auto-station-queue spectator role.
- Multiple simultaneous game rooms.
- Authentication / access control.
- Mobile-native apps.
- A native client (phones remain browser-based).
- Self-hosted PeerJS broker.
- Binary wire format (architecture supports swapping; not implemented).
- macOS / Windows code signing.
- Lobby ship selection (the player ship is hardcoded at startup).
- Save migration between incompatible save-format versions.
- AI perception (sensor range, line-of-sight, radar dampening reactions on the AI side).
- Player possession of AI entities (architecturally permitted by the controller/entity split; no UI in v1).
- AI fleet coordination, kiting, broadside maneuvering, comms responses, power management, repair decisions.
- Per-console damage degrading underlying system performance (HP is binary: operable or not).
- Comms console damage state (Comms is not in the damageable-consoles list).
- Save/load of AI controllers and per-console HP state (carried by the generic save system without dedicated AI work).
- Region-cleanup of scenario-applied modifiers (explicit remove only).
- Free-text comms; all responses are predefined.
- Custom AI primitives / DSL (states and conditions are a fixed vocabulary).
