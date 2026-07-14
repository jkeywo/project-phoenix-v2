---
title: WeaponsPlugin
---

# WeaponsPlugin

Extracted from `simulation.rs` as part of the simulation split series (issue [#245](https://github.com/jkeywo/project-phoenix-v2/issues/245)).

## Ownership

`WeaponsPlugin` owns all phaser, torpedo, and beam-render state and message handling for the Tactical console.

### Systems

| System | Responsibility |
|---|---|
| `handle_set_target` | Processes `SetTarget` from the Tactical holder; locks target if in radar range |
| `handle_fire_phaser` | Processes `FirePhaser`; starts a beam if target is in arc and phaser is ready |
| `handle_set_phaser_mode` | Processes `SetPhaserMode { Auto \| Manual }` from Tactical holder |
| `handle_set_phaser_frequency` | Processes `SetPhaserFrequency` from Tactical or Sensors (complexity-gated) |
| `handle_fire_torpedo` | Processes `FireTorpedo { tube, target_uuid }`; launches if tube is loaded |
| `handle_load_tube` | Processes `LoadTube { tube }`; manually starts loading a tube |
| `handle_unload_tube` | Processes `UnloadTube { tube }`; manually unloads or cancels loading |
| `tick_beams` | Advances every active phaser beam (player + NPC): damage accumulation, sever-on-range, natural end, cooldown start |
| `tick_torpedo_system` | Advances all in-flight torpedoes; fires `TorpedoDestroyed` for expired ones |
| `tick_weapons_arc_request` | Emits `ArcBearingRequest` channel-3 coordination to Helm when weapons target is in range but outside all firing arcs |

### Resources

| Resource | Purpose |
|---|---|
| `WeaponsTarget` | Currently locked target UUID (`None` if no lock) |
| `ActiveBeam` | Active phaser beam: target UUID, remaining seconds, damage accumulator, bank |
| `PhaserCooldown` | Post-beam cooldown (duration sourced from `PhaserCombatConfigResource`) |
| `CurrentPhaserMode` | Auto or Manual phaser mode |
| `PhaserRenderConfig` | Beam colour and max render range, populated from ship TOML during world setup |
| `PhaserCombatConfigResource` | Player phaser tuning (beam duration, cooldown, damage/sec, range); sourced from `[weapons_console]` |
| `TorpedoSystemResource` | Wraps the pure-Rust `TorpedoSystem` state machine |
| `WeaponsArcRequestState` | Tracks last arc-missed target to debounce `ArcBearingRequest` emission |

### Message type

| Type | Registered by |
|---|---|
| `AsteroidDestroyedVfx` | `WeaponsPlugin` via `add_message::<AsteroidDestroyedVfx>()` |

### Coordination payloads

| Variant | Destination | Trigger | Human AI routing |
|---|---|---|---|
| `ArcBearingRequest { uuid, label }` | Helm | Target locked + in weapons range + outside all phaser-bank firing arcs | `route_coordination` — human Helm gets a popup; AI Helm consumes silently and biases steering via `PendingArcBearingRequest` |

### Public constants

| Constant | Value | Used by |
|---|---|---|
| `BEAM_DAMAGE_PER_SEC` | `5.0` | `weapons_plugin.rs` (tick), `simulation.rs` (tests) |

## Registration

```rust
.add_plugins(crate::weapons_plugin::WeaponsPlugin)
```

Registered by `add_simulation_plugins()` in `src/server_app.rs`. The module is declared in `src/lib.rs`.

The `weapons_update_broadcaster()` function (a `SimBroadcaster` producing `WeaponsUpdate` at 10 Hz to the Tactical holder) is defined in `src/console/weapons/server.rs:3838` and registered by `add_simulation_plugins()` in `src/server_app.rs:329`.

## Broadcaster

`weapons_update_broadcaster()` reads:
- `ShipState` — for ship position and yaw (fire-ready arc check)
- `WorldResource` — for target entity position
- `WeaponsTarget`, `PhaserCooldown`, `ActiveBeam`
- `TorpedoSystemResource` — for per-tube reload state and magazine count
- `ShipModifiers` — for `RadarRange` multiplier (effective weapons range)

Produces `ServerMessage::WeaponsUpdate` sent to the Tactical console holder at 10 Hz.

## NPC shields (#471)

Single-facing shield component for NPCs and stations. Distinct from the player ship's four-quadrant `ShipShields` resource: NPCs carry an `EntityShield` ECS component (`src/entities/spawner.rs`) populated from a top-level `[shields]` block on the entity TOML:

```toml
[shields]
max_hp        = 60.0
regen_per_sec = 1.0
```

Damage routing — three paths in `src/console/weapons/server.rs` route NPC-bound damage through the shield:

| Path | System | Pierce source |
|---|---|---|
| Player phaser → NPC | `tick_beams` | Active bank's `shield_pierce` (Option<f32>) |
| Player torpedo → NPC | `tick_torpedo_system` | `TorpedoDetonation.shield_pierce` (snapshot at launch) |
| NPC phaser → NPC/station | `tick_beams` | Active bank's `shield_pierce` (per-entity `PhaserCombatConfigResource`) |

Each path applies `split_damage_for_pierce(damage, pierce)`: the `pierced` portion lands on hull directly, `absorbed` hits the shield, and any overflow leaks back to hull. Damage with no shield component falls through to the legacy hull-direct path unchanged (zero regression for asteroids and shieldless stations).

**Permanent break semantics** — once `current_hp` reaches `0.0`, the shield latches `broken = true` and never recovers. All subsequent damage skips the shield routing entirely and goes straight to hull regardless of the attacker's `shield_pierce`. There is no offline timer / recovery model (unlike the player ship). `tick_npc_shield_regen` (Physics set) advances `current_hp` by `regen_per_sec * dt` only while `!broken && current_hp < max_hp`.

**Wire format** — `EntitySnapshot.shield_fraction: Option<f32>` and `EntityStateSnapshot.shield_fraction: Option<f32>` carry the live shield ratio (`Some(current/max)` for shielded entities, broken shields read as `Some(0.0)`, shieldless entities omit the field). Used by the Sensors panel target-info row (#473).

## Tests

Tests live in `src/weapons_plugin.rs` under `#[cfg(test)] mod tests`.

| Test | Behaviour verified |
|---|---|
| `fire_phaser_on_valid_target_broadcasts_beam_started` | Beam starts when target is in range and arc |
| `fire_phaser_rejected_when_target_behind_ship` | Beam rejected if target not in forward arc |
| `fire_phaser_rejected_during_cooldown` | Beam rejected while cooldown is active |
| `weapons_update_fire_ready_true_when_target_in_range_and_arc` | `WeaponsUpdate.fire_ready` true for valid target |
| `weapons_update_fire_ready_false_when_target_out_of_phaser_range` | `WeaponsUpdate.fire_ready` false for out-of-range target |
| `phaser_damage_modifier_doubles_kill_rate` | `PhaserDamage` modifier at +1 doubles effective DPS |

Integration tests (test-app exercises `WeaponsPlugin` as a complete plugin) are in `src/server_app.rs::tests`.

## Torpedo configuration (TOML-driven)

The `TorpedoSystemResource` is initialised by `WeaponsPlugin::build` with
`TorpedoConfig::default()` so test apps that never load a ship TOML still
get a working torpedo system. The live game path overrides this resource
during `spawn_game_start_entities` (`src/server_app.rs`) when the spawned
ship's `EntityConfig` carries a `[torpedoes]` block:

```toml
[torpedoes]
count = 10
damage_hull = 50
damage_shields = 5
speed = 30.0
turn_rate_deg_per_sec = 45.0   # converted to radians at the boundary
lifespan = 20.0
load_time = 10.0
```

All fields use `serde(default)` and fall back to the same values as
`TorpedoConfig::default()` (`src/weapons/torpedo.rs:49`). Designer-facing
`turn_rate_deg_per_sec` is converted to radians in
`TorpedoesConfig::to_runtime()` (`src/entities/config.rs`).

NPC ships that want a different loadout simply add their own `[torpedoes]`
block to their entity TOML; NPC ships that omit the block keep the defaults.

A drift-guard test
(`player_ship_toml_torpedoes_block_matches_runtime_default_values` in
`src/entities/config.rs`) fails if the player ship's `[torpedoes]` values
diverge from `TorpedoConfig::default()`.

## Phaser combat configuration (TOML-driven)

Player phaser tuning (beam duration, cooldown, damage-per-second, range)
is sourced from the existing `[weapons_console]` block in
`assets/entities/player_ship.toml`:

```toml
[weapons_console]
beam_range = 40.0
beam_damage_per_sec = 5.0
beam_duration_secs = 6.0
cooldown_secs = 6.0
```

`PhaserCombatConfig::from_weapons_console` (`src/entities/config.rs`)
maps these into a `PhaserCombatConfig` (using "zero means absent"
fallback to defaults, mirroring the NPC weapons path). The value is
inserted as `PhaserCombatConfigResource` during
`spawn_game_start_entities` (`src/server_app.rs`). `WeaponsPlugin::build`
seeds the resource with `PhaserCombatConfig::default()` so test apps
that never load a ship TOML still get a working phaser system.

`handle_fire_phaser`, `tick_beams`, and the
`weapons_update_broadcaster` all read this resource instead of the
legacy module-private `_LEGACY_BEAM_DURATION_SECS` /
`_LEGACY_BEAM_COOLDOWN_SECS` constants. The public
`BEAM_DAMAGE_PER_SEC` constant is retained because integration tests in
`src/server_app.rs` use it as a baseline assertion value.

Deliberately **not** TOML-driven (engineering invariants): the forward
firing arc (180° hemisphere) and `radar::PHASER_RANGE` (only referenced
by `radar.rs`'s own unit tests now). The `PhaserConfig` /
`PhaserSystem` types in `src/weapons/phaser.rs` are dead code in the
live game and were intentionally left untouched this slice.

Drift guards in `src/entities/config.rs`:
- `player_ship_toml_weapons_console_phaser_combat_matches_runtime_defaults`
- `player_ship_toml_shields_base_block_matches_runtime_default_values`

End-to-end "TOML flows to live state" tests:
- `phaser_combat_config_resource_reflects_player_ship_toml_weapons_console`
  (`src/console/weapons/server.rs`)
- `shield_system_reflects_player_ship_toml_shields_console_base_block`
  (`src/weapons/shield.rs`)

## Shield base configuration (TOML-driven)

Shield base tuning (facing count, max HP, regen rate, offline duration)
lives in a nested sub-block under `[shields_console]`:

```toml
[shields_console.base]
num_facings = 4         # UI assumes 4; changing this will break the Shields panel
max_hp = 100
regen_per_sec = 5.0
offline_duration = 10.0
```

The nested block mirrors the `[sensors_console.long_range_radar]`
precedent. `ShieldsBaseConfig::to_runtime()` produces a
`ShieldConfig`; `spawn_game_start_entities` constructs the live
`ShieldSystem` via
`ShieldSystem::new(&sc.base.map(to_runtime).unwrap_or_default())` and
then overlays the focus configuration. `Option<ShieldsBaseConfig>` was
used so the 22 existing `EntityConfig {...}` test literals scattered
across the codebase did not need to be touched.

## HTML Shields console runtime

The live phone Shields panel is `gui/shield-console.html`, embedded by
`client.html` as the `Shields` iframe. It receives `ShieldsConsoleState`
through `gui/console-core.js`, renders the four shield facings over
`assets/shield_console/shield-hex-bg.png`, and sends
`set_shield_focus` actions through `gui/action-map.js`. Focus is controlled
by clicking or keyboard-activating the shield segment itself: clicking the
currently focused segment clears focus by sending `facing: null`.

As of 2026-06-14, the HTML panel uses the shield hex bitmap as an SVG
pattern fill for the segment paths themselves, not just as the broad diagram
backdrop. The surrounding panel/cards also reuse `assets/gui/panel-bg.png`,
and the center marker uses the existing `assets/phone_border/compass-ring.png`
and `needle.png` bitmaps so the console more closely matches the mockup's
image-heavy treatment.

## Virtual entities are excluded from torpedo detonation

`tick_torpedo_system` (`src/console/weapons/server.rs:3010`) builds the
proximity-detonation target list from both live ECS entities and the
`WorldResource` snapshot. Every entry carries an `(uuid, x, z, radius)`
that `find_detonation_hits` (`src/weapons/torpedo.rs:771`) tests against
each in-flight torpedo: a hit fires when
`distance(torpedo, entity) ≤ detonation_radius + entity.radius`.

Two entity kinds are **virtual** — organisational/effect-only anchors
with no physical body that the player should pass through:

- **Asteroid-field anchors** carry an `AsteroidFieldSection`. Their
  `EntitySnapshot.radius` is populated from the field's `outer_radius`
  (`src/server_app.rs:1598`), so a `default.toml`-style field at the
  world origin with `outer_radius = 350` registers as a 350 m torpedo
  target. With the player ship at `(280, 0, 0)`, every torpedo fired
  from the ship detonated on the field anchor on its first physics
  tick — invisible from the viewscreen because the sphere lifetime was
  a single frame.
- **Region trigger volumes** carry a `RegionShapeSection`. Their
  snapshot radius comes from the region shape (`Sphere.radius`,
  `Box.max_he`, or `Torus.outer_radius`) at `src/server_app.rs:1691`.

`tick_torpedo_system` excludes both via a `virtual_entity_q` query
(`Or<(With<AsteroidFieldSection>, With<RegionShapeSection>)>`) plus a
shape-based filter for snapshot-only entries (anything with
`EntitySnapshot.shape.is_some()` is treated as virtual). The exclusion
applies to the **proximity-detonation** target list only; the homing
`target_positions` map is left intact (locked-target homing pre-filters
to real targets via `SetTarget` authorisation).

Regression test:
`torpedo_does_not_detonate_on_asteroid_field_anchor_entity`
(`src/console/weapons/server.rs`).

## Sources

- `src/weapons_plugin.rs`
- `src/server_app.rs` (integration tests)
- Issue [#245](https://github.com/jkeywo/project-phoenix-v2/issues/245)
- [Console UI Authoring Library](./console-ui-library.md)
- [Broadcaster Seam](./broadcaster-seam.md)

## Per-bank phasers and per-tube torpedoes (2026-05)

The single-phaser / three-hardcoded-tube model was replaced with a
data-driven loadout. The ship's TOML now declares every bank and tube
explicitly, and the entire wire / server / client / UI stack reads from
that loadout. There is no fallback to a hardcoded `"port"` bank or to
`fore_port|fore_starboard|aft` tubes once the new schema is in effect.

### TOML schema

```toml
[weapons_console]
beam_range = 40.0
beam_damage_per_sec = 5.0
beam_duration_secs = 6.0
cooldown_secs = 6.0
beam_color = [1.0, 0.4, 0.1, 1.0]
torpedo_arc_color = [1.0, 0.55, 0.2, 1.0]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 270.0
auto_arc_deg = 180.0    # auto_arc_deg ≤ fire_arc_deg (validator)

[[weapons_console.phaser_banks]]
id = "aft"
facing_deg = 180.0
fire_arc_deg = 270.0
auto_arc_deg = 180.0

[torpedoes]
count = 10              # shared ammo pool across all tubes
load_time = 10.0

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = -30.0
fire_arc_deg = 90.0

[[torpedoes.tubes]]
id = "fore_starboard"
facing_deg = 30.0
fire_arc_deg = 90.0

[[torpedoes.tubes]]
id = "aft"
facing_deg = 180.0
fire_arc_deg = 90.0
```

- `PhaserBankConfig` lives at `src/entities/config.rs:268` with validator
  `validate_phaser_banks` enforcing `auto_arc_deg ∈ (0, fire_arc_deg]`
  and `fire_arc_deg ∈ (0, 360]`.
- `TorpedoTubeConfig` lives at `src/entities/config.rs:289`.
- Torpedo tubes start unloaded in both legacy and TOML-driven constructors
  (`src/weapons/torpedo.rs`). Firing a loaded tube returns it to `unloaded`;
  it does not auto-reload. Operators must send `LoadTube` to start loading.

### Wire shape

`WeaponsUpdate` is now per-bank and per-tube:

```rust
WeaponsUpdate {
    target_uuid: Option<String>,
    banks: Vec<PhaserBankState>,    // id, fire_ready, on_cooldown, cooldown_remaining
    tubes: Vec<TorpedoTubeState>,   // id, loaded, reload_secs
    torpedo_count: u32,             // shared ammo pool
    phaser_mode: PhaserMode,
}

ClientMessage::FirePhaser { bank: String }
ClientMessage::FireTorpedo { tube: String, target_uuid: String }
ClientMessage::LoadTube { tube: String }
ClientMessage::UnloadTube { tube: String }
```

`ShipClientConfig` in `Welcome` ships the bank and tube layouts plus the
two render colours so the client can render fire-arc overlays without
knowing the server-side `auto_arc_deg`:

```rust
ShipClientConfig {
    phaser_banks: Vec<PhaserBankClientConfig>,     // id, facing_deg, fire_arc_deg, cooldown_secs
    torpedo_tubes: Vec<TorpedoTubeClientConfig>,   // id, facing_deg, fire_arc_deg
    phaser_beam_color: [f32; 4],
    torpedo_arc_color: [f32; 4],
    ...
}
```

Populated by `lobby/server.rs::update_session_with_config` from the
ship's `[weapons_console]` and `[torpedoes]` blocks. `cooldown_secs`
mirrors the server's "zero means absent" fallback to
`PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS` so the client always
sees the real per-bank cooldown duration; this lets the Tactical UI
render an accurate per-bank cooldown bar (denominator = `cooldown_secs`,
numerator = `PhaserBankState.cooldown_remaining`).

### Client UI

The Tactical console UI is **`gui/weapons-console.html`** — a static
HTML/JS panel (no Rust client; the Bevy client was removed in #463).
`gui/console-state.js` projects `WeaponsUpdate` (per-bank `banks` and
per-tube `tubes`) plus the `Welcome` ship config (`phaser_arcs` /
`torpedo_arcs`, both carrying their own `cooldown_secs` / arc data) into
the panel's render input.

The panel keeps stable DOM nodes per id, both for torpedo tube rows
(`_tubeRowEls`) and per-bank cooldown rows (`_cooldownRowEls`), so 10 Hz
state pushes don't churn click targets and bar transitions can animate.

**Per-bank cooldown bars** — the `#phaser-cooldowns` block renders one
`.cooldown-row` per bank, in ship-config order. Each row has three
states driven by `PhaserBankState`:

| Bank state | Row class | Bar | Value |
|---|---|---|---|
| `!on_cooldown` | `is-ready` (green) | full | `READY` |
| `on_cooldown && cooldown_remaining ≈ 0` (beam firing) | `is-firing` (orange) | full | `FIRING` |
| `on_cooldown && cooldown_remaining > 0` (post-beam cool) | `is-cooling` (amber) | refilling `1 - remaining/cooldown_secs` | `x.xs` countdown |

The bar's denominator is `PhaserBankClientConfig.cooldown_secs` from
`Welcome`. If a bank ships without `cooldown_secs` (defensive — should
not happen in practice because the lobby producer always falls back to
`PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS`), the renderer tracks
the per-bank peak `cooldown_remaining` it has observed and uses that as
the denominator instead.

Radar fire arcs are drawn by `gui/radar-widget.js` from the
`phaser_arcs` (per-bank) and `torpedo_arcs` (per-tube) lists projected
into render state by `gui/console-state.js`.

### Phaser PFX target points

Viewsceen phaser beams resolve their visual endpoint from model-rig
sidecar `[[target_points]]` entries when the target model defines them.
`ModelRig` parses the points, `ModelMarkers` carries them on rendered
entities, and `src/server/pfx.rs` chooses one target point per live beam
so the beam remains stable for its duration instead of jittering between
points. Damage and target-lock logic still use the entity centre/range
rules; this is a renderer-only endpoint choice.

Ship sidecars currently carry three provisional points:
`[0.5, -0.1, 0]`, `[-0.25, -0.1, 0.25]`, and
`[-0.25, -0.1, -0.25]` in Bevy's Y-up model-rig space.

Marker and target-point positions are stored in post-base-rig space: the
sidecar `[base]` transform is applied to the rendered GLB child, while
`ModelMarkers` keeps authored points that should already match that corrected
space. Runtime PFX resolves those points with
`Transform::transform_point`, so the entity transform's current translation,
rotation, and scale are applied. This includes `[mesh].scale` /
`[mesh].rotation` because the renderer writes those onto the parent entity
transform, and it includes game yaw/roll updates from ship physics. It does
not separately apply the sidecar `[base]` transform during lookup; raw GLB-local
marker coordinates would be wrong unless converted into post-base-rig space
first. World-level `[[entity]].transform.rotation` / `scale` currently parse
but are not applied by the immediate entity spawning path.

### Drift guards and tests

- `validate_phaser_banks` / `validate_torpedo_tubes` reject empty lists,
  duplicate ids, and out-of-range arcs.
- `phaser.rs` and `torpedo.rs` stay Bevy-free and test their per-id state
  machines directly.
- `tests/smoke/tactical-fire-flow.spec.ts` exercises `FirePhaser { bank }`
  on the live wire and aggregates `WeaponsUpdate.banks` for fire-ready
  detection.
- `tests/smoke/weapons-console.spec.ts` injects bank state through
  `__updateConsole('Tactical', …)` and asserts on per-bank
  `.cooldown-row[data-id]` classes (`is-ready` / `is-cooling`) and the
  countdown text (`READY` / `1.5s`).
