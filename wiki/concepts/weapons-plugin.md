---
title: WeaponsPlugin
---

# WeaponsPlugin

Extracted from `simulation.rs` as part of the simulation split series (issue [#245](https://github.com/jkeywo/project-phoenix-v2/issues/245)). The single `src/console/weapons/server.rs` file (which grew to ~12,000 lines including tests) was subsequently decomposed into per-domain files by the weapons decomposition series (issue [#685](https://github.com/jkeywo/project-phoenix-v2/issues/685); issues #721–#731). `server.rs` itself was folded into `mod.rs` at the end of that series so the final layout matches issue #731's expected structure exactly — see [File layout](#file-layout) below.

## Ownership

`WeaponsPlugin` owns all phaser, torpedo, and beam-render state and message handling for the Tactical console.

## File layout

`src/console/weapons/` is a module tree, not a single file:

| File | Owns |
|---|---|
| `mod.rs` | Declares the sibling modules; hosts `WeaponsPlugin` (all system + resource registration), the systems that are still genuinely shared between phaser and torpedo (`integrate_weapons_state`, `ai_target_selection`, `tick_weapons_arc_request`, `tick_npc_auto_match_frequency`), the re-export blocks that keep every sibling module's items reachable at `crate::console::weapons::…` / `crate::weapons_plugin::…`, and the `#[cfg(test)] mod tests` declaration — matching issue #731's expected layout of "plugin assembly, re-exports" with no separate `server.rs` |
| `server_tests.rs` | All 119 `#[cfg(test)]` tests + test helpers for the weapons module, loaded into `mod.rs` via `#[path = "server_tests.rs"] mod tests;` (kept as a child module rather than an external `tests/` integration file — see [Test placement](#test-placement)) |
| `shared.rs` | Small pure helpers used by every weapons file (`live_entity_xz`, `tactical_authorized`, `system_is_registered`, bank/AI-operates predicates) plus the one-tick handoff resources `BeamContext`, `ShooterState`, `TorpedoTargetSnapshot` |
| `beam.rs` | Phaser/beam domain: `ActiveBeam`, `PhaserCooldown`, `TacticalRadarSelection`, `LastShipAttacker`, `CurrentPhaserMode`, `PhaserCombatConfigResource`, beam events, `handle_fire_phaser`, `ai_phaser_auto_fire`, `handle_set_phaser_mode`/`_frequency`, `handle_set_target`, `drain_power_for_active_beam`, and the three-phase beam tick (see below) |
| `torpedo.rs` | Torpedo domain: `TorpedoSystemResource`, `handle_fire_torpedo`, `handle_load_tube`, `handle_unload_tube`, `handle_set_torpedo_volley_target`, `handle_torpedo_magazine_inter_system`, and the two-phase torpedo tick (see below) |
| `blaster.rs` | Blaster (NPC hitscan weapon) domain: `BlasterSystemResource`, `handle_fire_blaster`, `tick_blaster_auto_fire`, `tick_blaster_system`, `handle_blaster_hits` |
| `blackboard.rs` | Publish-phase output: the four `publish_*_blackboard` systems, `compute_current_weapons_update`, `weapons_update_broadcaster`, `LastWeaponsUpdate`, and the radar-blip projection helpers. The radar blips and region overlays are published into `TacticalRadarBlackboard` (not `WeaponsBlackboard`) by `publish_tactical_radar_blackboard` (#829); the ship's combat target is `TacticalRadarSelection`, lifted into `ViewscreenBlackboard::combat_lock` — which is what `publish_weapons_core_blackboard`, `publish_phaser_bank_blackboards` and `compute_current_weapons_update` read for `target_uuid`, never the live component (spec §3) |

Every system stays registered from `WeaponsPlugin::build` in `mod.rs`; the sibling files are pure implementation modules, not separate plugins. Cross-file items are `pub(crate)` (or `pub` where an item is also consumed outside the weapons module, e.g. `TorpedoSystemResource`) and re-exported from `mod.rs` so both the plugin build function and the test module resolve them without needing to know which file they actually live in.

### The three-phase beam tick

`tick_beams` used to be a single ~660-line system. It is now three systems chained in `SimSet::Damage`, connected by the one-tick `BeamContext(Vec<ShooterState>)` resource (`shared.rs`):

1. **`tick_beams_prepare`** — snapshots shooter state: live position lookup, arc/range check, damage accumulator, LOS raycast (Rapier), friendly-fire classification. Clears and repopulates `BeamContext`.
2. **`tick_beams_apply_damage`** — shield routing, hull damage, asteroid/NPC despawn, instagib/god-mode, VFX, broadcasts (`DamageTaken`, `ShipDestroyed`, `AsteroidDestroyed`, `EntityDespawned`), attacker tracking (`LastShipAttacker` via `set_if_neq`).
3. **`tick_beams_tick_lifetimes`** — ends beams on destroyed targets, ticks `remaining_secs`, fires `BeamEndedEvent`, clears `TacticalRadarSelection`.

Registered as an instance-based `.chain()` rather than type-set `.after(...)` edges, because the test harness registers a second instance of each phase — `SystemTypeSet` ordering would be ambiguous (and panic at schedule build) across two instances of the same system type.

### The two-phase torpedo tick

`tick_torpedo_system` was similarly split into two systems in `SimSet::Physics`, connected by the one-tick `TorpedoTargetSnapshot` resource (`shared.rs`):

1. **`build_torpedo_target_snapshot`** — builds the UUID→(x,z) position map (live ECS + `WorldResource` fallback) and the detonation target list `(uuid, x, z, radius)`, excluding virtual entities (asteroid-field anchors, region triggers — see [Virtual entities](#virtual-entities-are-excluded-from-torpedo-detonation)).
2. **`tick_torpedo_lifecycle`** — per-ship torpedo tick: proximity detection, shield routing, hull damage, despawn, broadcasts, VFX. Same `.chain()` rationale as the beam split.

A torpedo that kills the **player's** ship latches game over the same way the beam and blaster kill sites do (`ShipDestroyed` → `GamePhase::GameOver` → first-write `GameOverReason` + `Outcome::Defeat`), and the LocalShip entity is never despawned. `NextState` + `GameOverReason` ride in the `server_app::PlayerDeathLatch` `SystemParam` because the lifecycle system sits on Bevy's 16-parameter ceiling.

### The AI torpedo doctrine gate is per-arc

`console_ai::auto_fire_torpedo` holds fire while the target's shields are up — phasers strip, torpedoes finish. The gate asks about the **one arc the torpedo would strike**, not the sum over all arcs: `console_ai::server::ai_torpedo_auto_fire` computes the attack bearing with `shield::attacker_bearing_relative` (in the *target's* frame, so its own yaw matters) and resolves the arc with the target's own `ShieldSystem::facing_index_for_bearing` — the same resolver `apply_damage` uses, so there is one bearing→arc resolver, not two. An offline arc and a target with no arcs at all both report 0, i.e. nothing is blocking the shot.

The magazine is **not** part of that gate. `TorpedoAiInput.magazine` is reported but never read as a veto: `TorpedoSystem::start_load` (and the auto-load block in `TorpedoSystem::tick`) decrements `torpedoes_remaining` when a load *starts*, so the magazine counts what is left to reload with, not what can be fired. An `input.magazine == 0` conjunct used to sit here and made any hull whose magazine divides evenly into its battery permanently unable to fire its last, fully loaded salvo — `ship_harrow_cruiser` (8 rounds, a 4-round battery) reaches that state on its second reload every run, and `tubes_full` then read permanently true, so the helm's salvo-spent resume could never fire either. Running dry is enforced where the rounds are actually taken: `start_load` and `claim_magazine_round` both refuse at zero.

The hit side has to hold up its end of that: `TorpedoDetonation` carries `impact_x`/`impact_z` (the torpedo's own position when it went off, alongside `source_uuid`/`tube_id`, because the torpedo leaves `in_flight` at detonation), and `tick_torpedo_lifecycle` turns it into a bearing in the victim's frame before calling `apply_damage`. Passing a constant `0.0` there — as the path did until the per-arc gate landed — put every torpedo from every direction on whichever arc contains bearing 0, so a shot green-lit against a collapsed aft arc was absorbed by a healthy fore arc. The gate predicts from the launcher and the hit resolves from the impact point, so a heavily-homing shot can still land on a neighbouring arc; the resolver is shared, the moment sampled is not. **The blaster path still passes `0.0`** (`console/weapons/blaster.rs`) — latent while every blaster-armed hull declares a single omni arc. The beam path already passes a real bearing for the player's ship.

Above that host gate sits the per-tube launch **policy** (`torpedo_launch` channel). Its fact snapshot is seeded by `seed_torpedo_tube_launch_facts` (`console/weapons/torpedo.rs`) and, since issue #791, carries a ship-wide `tubes_full` reading — every tube at `loaded_count == volley_max` — alongside the per-tube `loaded` (which is only `loaded_count > 0`). The two answer different questions, and the two shipped doctrines take one each. `ship_harrow_cruiser` fires all its tubes into one shield gap or none of them, so it gates on `tubes_full`. `ship_harrow_warhawk` (issue #793) is the opposite: its fore and aft launchers are opportunistic close defence, each taking whatever bearing the artillery hold happens to give it, so each gates on its own `loaded` — under `tubes_full` a loaded launcher bearing on a collapsed arc would hold its round because the *other* launcher was mid-reload, and the pair would degrade into one. `target_facing_shields` beside them is an HP value, not a boolean.

Summing every arc meant three healthy rear arcs vetoed a shot into a collapsed front arc while the attacker sat dead ahead. With per-arc regen and short offline windows a four-arc Alliance hull practically never has all arcs down at once, so AI crews on those hulls never launched a torpedo and same-class duels reported 0% torpedo contribution. Single-omni-arc NPCs are unaffected — their one arc faces every bearing.

### Per-blackboard publish

`publish_weapons_blackboard` was split into four systems in `SimSet::Publish`, one per blackboard entry the ship writes (`publish_weapons_core_blackboard`, `publish_phaser_bank_blackboards`, `publish_torpedo_tube_blackboards`, `publish_torpedo_magazine_blackboard`). `ShipSystemBlackboards` is a per-ship `HashMap<SystemId, SystemBlackboard>` component, not a struct with named fields — each system writes a disjoint set of map keys, so all four register as a bare (unordered) tuple. Because none of them may depend on another having run first, the bank/tube list each entry needs is *recomputed* per system via shared pure helpers (`build_bank_states`, `build_tube_states` in `blackboard.rs`) rather than read back out of another system's freshly-written map entry.

### Systems (by SimSet)

| System | SimSet | Responsibility |
|---|---|---|
| `handle_set_target` | Input | Processes `SetTarget` from the Tactical holder; locks target if in radar range |
| `handle_fire_phaser` | Input | Processes `FirePhaser`; starts a beam if target is in arc and phaser is ready |
| `ai_phaser_auto_fire` | Input | AI equivalent of `handle_fire_phaser`; writes `PhaserIntents` |
| `handle_set_phaser_mode` | Input | Processes `SetPhaserMode { Auto \| Manual }` from Tactical holder |
| `handle_set_phaser_frequency` | Input | Consumes admitted `SetPhaserFrequency` envelopes on `phaser-control`, writes `ShipPhaserFrequency` (legacy top-level message deleted, #804) |
| `handle_fire_torpedo` | Physics | Consumes admitted `FireTorpedo { tube, target_uuid }` from each ship's own `AdmittedCommands` (#846); launches if the tube is loaded and the tube + magazine fine systems are online. `ConsoleAiPlugin` registers `ai_torpedo_auto_fire` `.before` it. **That edge is load-bearing**: both sit in `SimSet::Physics`, and without it the resolved order put this consumer first, so the AI's admitted `FireTorpedo` was wiped by `clear_before_input` on the next tick and an AI-crewed ship never launched a torpedo. The weapons test harness supplied the edge locally, so every unit test passed; `tests/headless_runner.rs::ai_crewed_ships_actually_launch_torpedoes_in_a_real_run` is the guard that boots the real plugin set |
| `handle_load_tube` | Physics | Consumes admitted `LoadTube { tube }`; manually starts loading a tube |
| `handle_unload_tube` | Physics | Consumes admitted `UnloadTube { tube }`; manually unloads or cancels loading |
| `handle_set_torpedo_volley_target` | Input | Applies admitted `SetTorpedoVolleyTarget` for a specific tube, reading **every ship's own `AdmittedCommands`** (`With<Ship>`) so AI-crewed ships receive it too — the AI loader `console_ai::server::ai_torpedo_load` issues the same command a human console sends. The target `SystemId` is matched by forward-mapping each tube id through `system_registry::torpedo_tube_system_id`, never by inverting the string — the mapping folds `_` to `-`, so inverting silently dropped every order for a hull that authors hyphenated tube ids (`alliance_battleship`) |
| `handle_fire_blaster` | Input | Processes `FireBlaster` for NPC hitscan weapons |
| `tick_blaster_auto_fire` | Input | AI auto-fire decision for blaster-equipped NPCs |
| — | — | `integrate_weapons_state` (the old shared `PhaserIntents`/`TorpedoIntents` adapter) is **gone**: since #846 both AI deciders emit admitted `ControlSystem` payloads and the human-path consumers (`handle_fire_phaser`, `handle_fire_torpedo`) are the only appliers |
| `drain_power_for_active_beam` | Physics | Drains ship power while a beam is active |
| `build_torpedo_target_snapshot` → `tick_torpedo_lifecycle` | Physics (chained) | See [two-phase torpedo tick](#the-two-phase-torpedo-tick) |
| — | — | `ship_plugin`'s `ai_policy_state_tick` declares `.after(handle_fire_torpedo)` and `.after(tick_torpedo_lifecycle)` (issue #791): it seeds a `torpedoes_in_flight` helm fact from the live `TorpedoSystemResource`, so both writers of `in_flight` must have run first or the reading would be run-order-dependent. That fact counts `in_flight.len()` **plus** every live `TubeBurstState`'s `pending`: a burst launch puts only its first round in the air and leaves the rest on `burst_interval_secs`, so the airborne count alone dips to zero mid-salvo — measured on the cruiser with the first pair resolving 0.23 s after launch against a 0.35 s interval, which released the hull and threw the back half of its own salvo outside the tubes' 24-degree arc. It seeds two armament facts off the same component. `tubes_full` is the launcher's own question asked helm-side — every tube at `loaded_count == volley_max`, the identical expression `ai_torpedo_auto_fire` evaluates — and it is what the cruiser's `orbit → torpedo_run` guard takes, because the transition that gives up a firing geometry has to ask exactly what the launcher will ask. `tubes_fillable` sits beside it as the slower reachability reading — "could every tube still reach `volley_max`", i.e. tubes + magazine online and `torpedoes_remaining >= TorpedoSystem::salvo_shortfall()` — and catches what `tubes_full` cannot, a loaded-but-destroyed tube or a magazine that can no longer top the battery up. Reachability alone is *not* an entry guard: it stays true through the whole 18 s reload after a salvo, which is how the cruiser ended up spending 506 bow-on ticks against 431 orbiting with only 5.7% of them tubes-full. `tubes_full` is also what *bounds* the phase — the salvo-spent resume guard depends on the hull's own armament rather than on the target ever raising a shield, which a resolvable target with no `[shields]` block never does. It cannot bound it alone, though: `tubes_full` reads the ROUNDS, and a tube shot out mid-phase keeps the rounds already loaded into it, so the cruiser carries a third resume on `tubes_fillable` for exactly that case — reachability sits on an exit as well as on the entry guard |
| `tick_blaster_system` | Physics | Advances in-flight blaster shots |
| `handle_torpedo_magazine_inter_system` | Physics | Cross-system torpedo magazine claim routing |
| `tick_beams_prepare` → `tick_beams_apply_damage` → `tick_beams_tick_lifetimes` | Damage (chained) | See [three-phase beam tick](#the-three-phase-beam-tick) |
| `handle_blaster_hits` | Damage | Applies blaster hit damage |
| `publish_weapons_core_blackboard`, `publish_phaser_bank_blackboards`, `publish_torpedo_tube_blackboards`, `publish_torpedo_magazine_blackboard` | Publish (unordered) | See [Per-blackboard publish](#per-blackboard-publish) |
| `tick_weapons_arc_request` | — | Emits `ArcBearingRequest` channel-3 coordination to Helm when weapons target is in range but outside all firing arcs |
| `ai_target_selection` | Input | Chooses/clears the AI-selected target and its `locked_target` carry-forward |
| `tick_npc_auto_match_frequency` | — | NPC phaser-frequency auto-match AI |

### Resources

| Resource | Purpose |
|---|---|
| `TacticalRadarSelection` | Currently locked target UUID (`None` if no lock); the ship's Combat Lock, lifted into `ViewscreenBlackboard::combat_lock` (#829) |
| `ActiveBeam` | Active phaser beams, tracked **per bank** (issue #790): an ordered `bank -> {target UUID, remaining seconds, damage accumulator}` map. A hull whose arcs overlap burns both banks at once on a target abeam, and the two shipped fore/aft pairs overlap by different amounts: `ship_harrow_cruiser` authors 270° on **both** `fire_arc_deg` and `auto_arc_deg`, so it double-broadsides on the manual (`handle_fire_phaser`) and AI (`ai_phaser_auto_fire`) paths alike; `alliance_cruiser` authors `fire_arc_deg = 270` but `auto_arc_deg = 180`, so it double-broadsides on the manual path only — its two auto arcs abut on the beam line rather than overlapping. Same shape as `PhaserCooldown`, ordered rather than hashed so the per-tick shooter snapshots (and the seeded damage draws they feed) stay deterministic |
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

The `weapons_update_broadcaster()` function (a `SimBroadcaster` producing `WeaponsUpdate` at 10 Hz to the Tactical holder) is defined in `src/console/weapons/blackboard.rs` and registered by `add_simulation_plugins()` in `src/server_app.rs`.

## Broadcaster

`weapons_update_broadcaster()` reads:
- `ShipState` — for ship position and yaw (fire-ready arc check)
- `WorldResource` — for target entity position
- `TacticalRadarSelection`, `PhaserCooldown`, `ActiveBeam`
- `TorpedoSystemResource` — for per-tube reload state and magazine count

It does **not** read `ShipModifiers`. Since issue #955 a bank reaches its authored `beam_range`, unscaled, so the broadcaster has no use for the `RadarRange` multiplier. That multiplier still bounds the tactical **acquisition** horizon, but it is read by `publish_tactical_radar_blackboard()` (`src/console/weapons/blackboard.rs`) — a different system.

Produces `ServerMessage::WeaponsUpdate` sent to the Tactical console holder at 10 Hz.

## NPC shields (#471)

Single-facing shield component for NPCs and stations. Distinct from the player ship's four-quadrant `ShipShields` resource: NPCs carry an `EntityShield` ECS component (`src/entities/spawner.rs`) populated from a top-level `[shields]` block on the entity TOML:

```toml
[shields]
max_hp        = 60.0
regen_per_sec = 1.0
```

Damage routing — three paths route NPC-bound damage through the shield:

| Path | System | Pierce source |
|---|---|---|
| Player phaser → NPC | `tick_beams_apply_damage` (`beam.rs`) | Active bank's `shield_pierce` (Option<f32>) |
| Player torpedo → NPC | `tick_torpedo_lifecycle` (`torpedo.rs`) | `TorpedoDetonation.shield_pierce` (snapshot at launch) |
| NPC phaser → NPC/station | `tick_beams_apply_damage` (`beam.rs`) | Active bank's `shield_pierce` (per-entity `PhaserCombatConfigResource`) |

Each path applies `split_damage_for_pierce(damage, pierce)`: the `pierced` portion lands on hull directly, `absorbed` hits the shield, and any overflow leaks back to hull. Damage with no shield component falls through to the legacy hull-direct path unchanged (zero regression for asteroids and shieldless stations).

**Permanent break semantics** — once `current_hp` reaches `0.0`, the shield latches `broken = true` and never recovers. All subsequent damage skips the shield routing entirely and goes straight to hull regardless of the attacker's `shield_pierce`. There is no offline timer / recovery model (unlike the player ship). `tick_npc_shield_regen` (Physics set) advances `current_hp` by `regen_per_sec * dt` only while `!broken && current_hp < max_hp`.

**Wire format** — `EntitySnapshot.shield_fraction: Option<f32>` and `EntityStateSnapshot.shield_fraction: Option<f32>` carry the live shield ratio (`Some(current/max)` for shielded entities, broken shields read as `Some(0.0)`, shieldless entities omit the field). Used by the Sensors panel target-info row (#473).

## Test placement

All 119 weapons tests (plus their ~65 test-only helper functions, e.g. `test_app`, `los_test_app`, `lock_and_fire`, `weapons_blackboard_of`) live in `src/console/weapons/server_tests.rs`, loaded into `mod.rs` as a child module:

```rust
#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
```

This is a deliberate deviation from a plain external `tests/weapons_test.rs` integration file. An external integration-test crate can only see `pub` items of the library, but the weapons tests exercise ~36 systems and types that are intentionally kept `pub(crate)`/private after the per-file split (issues #726–#729) — e.g. `run_system_once`-ing `integrate_weapons_state` directly, registering `ai_target_selection` as a bare system, calling `shared::system_is_registered` and `shared::any_bank_accepts_human_input` directly. Moving the tests externally would have forced promoting all of those to `pub`, reversing the encapsulation the split was for and violating this repo's "test through the public interface" convention (see `AGENTS.md`). The `#[path]` child-module form keeps the tests in the `console::weapons::tests` module path — `use super::*;` resolves exactly as it did when the tests were inline — while still shrinking `server.rs`'s original ~8,300 lines down to a ~1,100-line block that now lives directly in `mod.rs`.

Representative tests:

| Test | Behaviour verified |
|---|---|
| `fire_phaser_on_valid_target_broadcasts_beam_started` | Beam starts when target is in range and arc |
| `beam_severs_when_target_leaves_phaser_range` | Beam ends early when the target moves out of range mid-tick |
| `torpedo_does_not_detonate_on_asteroid_field_anchor_entity` | Virtual-entity exclusion holds for torpedo proximity detonation |
| `publish_writes_phaser_fore_blackboard_when_bank_configured` | Per-bank blackboard publish system writes the expected entry |
| `los_enemy_blocker_redirects_damage_away_from_target` | Rapier line-of-sight raycast redirects beam damage to a blocker |
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
ai_volley_target = 2            # rounds an AI crew keeps loaded per tube
```

`[[torpedoes.tubes]]` may override the AI standing load per tube with
`ai_target_count`. Resolution order is per-tube `ai_target_count` →
`[torpedoes] ai_volley_target` → the tube's own `volley_max`, clamped to
`volley_max`; the result lands on `TorpedoTube::ai_target_count` and is read
only by `ai_torpedo_load`. Tubes always *start* at `target_count = 0` for
human and AI alike — a non-zero default would pre-load a human player's tubes
and drain the magazine with no order given.

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

`handle_fire_phaser`, `tick_beams_prepare`/`tick_beams_apply_damage`, and the
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
  (`src/console/weapons/server_tests.rs`)
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

## Shield collapse and recovery (#788)

`ShieldFacing` (`src/weapons/shield.rs`) is the one arc model the player ship
and every arc-shielded NPC share — nothing in it branches on who owns the hull.

When a facing's HP reaches 0 it **collapses**: `offline_remaining` is set to the
authored `offline_duration` and every subsequent hit passes straight to hull.
`offline_duration` is a *no-damage delay*, not a recharge time. When it expires
the facing comes back online **at 0 HP** and climbs at its authored
`regen_per_sec` from there (`ShieldFacing::tick`). Before #788 it snapped
straight back to `max_hp`, so there was no instant at which a shield was
*partially* recovered — which made any fractional threshold ("wait until shields
are back to 75%") either already met or unreachable.

Two consequences worth knowing:

- A hit during the ramp knocks the facing back to 0 and restarts the full
  `offline_duration`, so sustained fire keeps a shield down rather than letting
  it flicker back.
- A facing may now sit at 0 HP while **online** (the first instant of its ramp).
  `apply_damage` therefore only re-arms the offline timer when the effective
  damage was non-zero.

`ShieldSystem::fraction()` is the whole-ship reading over all arcs, `[0, 1]`
(0.0 for a hull with no shield capacity). It is what
`src/ship/helm_ai.rs` seeds as `fact(shield_fraction)` for a ship's own
policies — see the Harrow destroyer's shield-recovery doctrine in
`assets/entities/ship_harrow_destroyer.toml`.

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

`build_torpedo_target_snapshot` (`src/console/weapons/torpedo.rs`) builds the
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

`build_torpedo_target_snapshot` excludes both via a `virtual_entity_q` query
(`Or<(With<AsteroidFieldSection>, With<RegionShapeSection>)>`) plus a
shape-based filter for snapshot-only entries (anything with
`EntitySnapshot.shape.is_some()` is treated as virtual). The exclusion
applies to the **proximity-detonation** target list only; the homing
`target_positions` map is left intact (locked-target homing pre-filters
to real targets via `SetTarget` authorisation).

Regression test:
`torpedo_does_not_detonate_on_asteroid_field_anchor_entity`
(`src/console/weapons/server_tests.rs`).

## How to add a new weapon type

Follow the blaster as the template (`blaster.rs` is the newest, smallest weapon domain and doesn't carry the historical baggage of beam/torpedo). Broadly:

1. **Pick a home file.** A new weapon type gets its own `src/console/weapons/<name>.rs`, declared in `mod.rs`. Don't add it to `server.rs` — that file is reserved for `WeaponsPlugin` registration and the systems genuinely shared across weapon types (`integrate_weapons_state`, `ai_target_selection`, arc-request coordination).
2. **Define the per-ship state** as a `#[derive(Resource, Component, Clone, Default)]` struct (mirrors `BlasterSystemResource`/`TorpedoSystemResource`) so it can live as both a per-entity component (NPC ships) and, where convenient, a `Res`/`ResMut` fallback for the local ship.
3. **Split Input/Physics/Damage from the start** if the weapon has an in-flight or channel-open phase (a torpedo/beam-shaped weapon), following the phase-resource pattern in `shared.rs`: a one-tick `Resource` wrapping a `Vec<YourShotState>`, cleared and repopulated by the first ("prepare"/"snapshot") system in `SimSet::Physics` or `SimSet::Damage`, read by later phases, and chained (`.chain()`) rather than ordered by `SystemTypeSet` — the test harness commonly registers a second instance of the plugin, and type-set ordering panics across duplicate instances. If the weapon is pure hitscan (like the blaster), a single system per phase (fire-intent handler → tick → hit-apply) is enough — no snapshot resource needed.
4. **Add a publish system**, not a shared blackboard builder — the publish phase intentionally has one system per blackboard entry (`SimSet::Publish`, unordered, disjoint `ShipSystemBlackboards` map keys) so weapon types never depend on each other's publish order. Add your new `SystemBlackboard` variant to `crate::messages` and a `publish_<weapon>_blackboard` system in `blackboard.rs` (or your weapon's own file, re-exported the same way blaster/beam/torpedo systems are).
5. **Register everything from `WeaponsPlugin::build`** in `server.rs`, in the correct `SimSet`, with `.in_set(...)` and (only where genuinely required) explicit `.chain()`/`.before()`/`.after()` edges — do not add ordering "just in case."
6. **Source all tunable numbers from TOML** on the ship's `EntityConfig` (a new `[<weapon>_console]` or similar block), following the `[weapons_console]` / `[torpedoes]` precedent above: `serde(default)` fields, a `to_runtime()` conversion, a drift-guard test in `src/entities/config.rs` comparing the player-ship TOML against the runtime defaults, and `WeaponsPlugin::build` seeding a default resource so test apps that never load a TOML still work.
7. **Add tests to `server_tests.rs`**, not a new file — it's the single home for all weapons-module tests (see [Test placement](#test-placement)) so `test_app()`/`combined_test_app()`/helpers stay shared.

## Sources

- `src/console/weapons/` (`server.rs`, `server_tests.rs`, `shared.rs`, `beam.rs`, `torpedo.rs`, `blaster.rs`, `blackboard.rs`)
- `src/server_app.rs` (integration tests)
- Issue [#245](https://github.com/jkeywo/project-phoenix-v2/issues/245)
- Issue [#685](https://github.com/jkeywo/project-phoenix-v2/issues/685) (weapons decomposition series, #721–#731)
- Issue [#788](https://github.com/jkeywo/project-phoenix-v2/issues/788) (shield collapse ramp + destroyer recovery orbit)
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
