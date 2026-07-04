---
title: PRD #597 — Ship Parity: Eliminate All Player/NPC Divergences
type: source
tags: [ship, npc, parity, refactor, unification]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/597
updated: 2026-07-04
---

## Status

**Complete — all 10 PRs shipped + post-review gap-closure pass** (2026-07-02).

Post-review pass unified the last four divergent code paths (`tick_active_beam` + `tick_npc_beams` → `tick_beams`; `handle_fire_phaser` + `handle_npc_beam_fire` → single `handle_fire_phaser`; `tick_phaser_auto_fire` iterates all ships; `sync_phaser_beams` renders all ships in one loop) and closed the remaining data-flow gaps (`translate_power_modifiers` and `tick_repair_teams` iterate all ships; NPCs get `TorpedoSystemResource` from `[torpedoes]` TOML; `handle_fire_torpedo` unified). `EntityShield` legacy struct + tests deleted. Final state: **the only differences between player and NPC ships are `ShipSystemControlSources` (AI vs human control per station, data-driven from TOML) and the `LocalShip` marker (viewscreen render/broadcast gate).**

## Problem

After PRD #581 unified the `Ship` marker and per-entity blackboards, a large number of divergences remain:

- **Critical bugs**: `With<Ship>` `.single()` calls that silently break when multiple ships exist; NPC-vs-NPC beams route damage to the player ship; torpedoes bypass player shields.
- **Data model mismatches**: `ShipShields` (player, n-facing) vs `EntityShield` (NPC, 1-facing); `ShipConfigComponent::default()` loads `player_ship.toml` for every NPC.
- **Feature gaps**: NPCs immune to collisions, region effects, modifiers, repair; all config resources are player-only singletons.

## Solution

Eliminate every divergence in 10 sequential PRs. After all 10: a ship is a ship. The only differences are `ShipSystemControlSources` (AI vs human control per station) and `LocalShip` (render/broadcast gate).

## PR Status

| PR | Title | Status |
|---|---|---|
| PR 1 | Fix critical `With<Ship>` regressions | **Done** (commit ebc0022) |
| PR 2 | Unified `ShipShields` (configurable `num_facings`) | **Done** |
| PR 3 | Per-entity `ShipConfigComponent` from each TOML | **Done** (commit 13c64c7, redo of f7bea89 revert) |
| PR 4 | Physics/impulse/boost/bank config → per-entity | **Done** (2026-07-01) |
| PR 5 | Weapons/torpedo/phaser config → per-entity | **Done** (2026-07-02) |
| PR 6 | Power/modifier/repair state → per-entity only | **Done** (2026-07-02) |
| PR 7 | Weapons/sensors/navigation state → per-entity; unified beam system | **Done** (2026-07-02) |
| PR 8 | Collision handling for all ships | **Done** (commit 1ce6a78) |
| PR 9 | Region effects for all ships | **Done** (2026-07-02) |
| PR 10 | Combat activity per-entity; delete `ShipHullIntegrity`; cleanup | **Done** (2026-07-02) |

## PR 1 Detailed Scope

Fix all bugs introduced when NPCs gained the `Ship` marker (PRD #581) but code was not updated:

- `tick_npc_beams` `ship_physics_q.single()` on `With<Ship>` → source-ship physics from own entity
- `tick_npc_beams` target identification: `player_ship_q.iter().any()` on `With<Ship>` → `Has<LocalShip>` check on resolved target
- `tick_npc_beams` `hull_query` filter excluded NPCs → allow NPC-vs-NPC beam damage
- `on_beam_started`/`on_beam_ended` `player_ship_q.single()` on `With<Ship>` → `With<LocalShip>`
- `handle_npc_beam_fire` `player_ship_q` `With<Ship>` misidentification → `Has<LocalShip>`
- `handle_slow_zone_speed_clamp` `ship_query.single_mut()` → `ship_query.get_mut(trigger.subject)`
- `src/server/pfx.rs` NPC engine trail filter `Without<Ship>` now always empty → use `With<AiControllerComponent>`
- `handle_coordination_enqueue` / `process_coordination_lag` → `With<LocalShip>`
- `handle_station_rating_change` → `With<LocalShip>`
- `process_lobby` / `handle_disconnect` ship query → `With<LocalShip>` + `single_mut()`

## PR 9 Detailed Scope

Region effects (damage zones, slow zones, blocks-impulse, comms-jam, sensor-blind, radar dampening) apply to every ship, not just the LocalShip. Region membership tracked per-entity in `RegionMembership.inside` (HashMap keyed by ship Entity).

- `update_region_membership` (`src/regions/server.rs:73`) iterates `With<Ship>` and computes membership per ship; stale-ship cleanup emits implicit `RegionExited` when a ship despawns while inside a region.
- `apply_damage_zone_damage` (`src/regions/server.rs:162`) iterates all ships in the damage zone, applying damage to each ship's own `EntitySystemHull` (renamed from `EntityConsoleHull` in #617) + optional `ShipShields`. Player-only side effects (`DamageTaken`, `ShipDestroyed`, `GameOver`, debug log) are gated on `Has<LocalShip>`. NPC destruction mirrors the beam-kill path: `AiEntityDestroyed` + `EntityDespawned` + `WorldResource` cleanup + entity despawn.
- `handle_slow_zone_speed_clamp` observer (already fixed in PR 1) uses `trigger.subject` — now correctly clamps any ship (player or NPC) with its own `ShipModifiers` component driving the effective max.
- Modifier-side effects (`RadarDampening`, `SlowZone`, `CommsJam`, `SensorBlind`) go through the `on_region_entered` / `on_region_exited` observers in `src/modifiers/coordination.rs`, which already use `trigger.subject` and write to the subject entity's `ShipModifiers` component.
- `handle_region_entered_event` / `handle_region_exited_event` (`src/world/server.rs:280`, `:310`) filter on `LocalShip` so world-scenario triggers remain player-driven (NPC crossings do not fire `OnEnteredRegion` triggers).
- `nebula_fog_system` (`src/server/renderer.rs:697`) filter switched from `With<Ship>` to `With<LocalShip>` — nebula fog is a rendering effect on the viewscreen camera.
- **Legitimately player-only**: `handle_blocks_impulse_region_enter` writes to `ShipImpulse` (player-only until NPC impulse is wired; at the time of PR 9 this was a global Resource, and since issue #606 it is a per-entity Component on the player ship only). Sensor-blind / comms-jam UI effects consumed by player-only Comms / Sensors panels — but the underlying `FlagKind` is set per-entity so NPC AI could opt in later.
- New tests: `npc_ship_in_damage_zone_takes_hull_damage` and `slow_zone_slows_npc_ship` in `src/regions/server.rs` prove NPC ships take damage and get speed-clamped while the player is unaffected in a different location.
- Test count: 1826 → 1828.

## PR 10 Detailed Scope

Final divergence-elimination pass: combat activity per-entity, delete `ShipHullIntegrity`, cleanup dead code.

**Goal A — Combat activity per-entity**

Four types converted from `Resource` to `Component` and inserted on every ship at spawn (player ship in `spawn_game_start_entities`, NPCs in `entities::spawner::spawn_entity`):

- `RecentCombatActivity` (`src/ship/combat_activity.rs:5`) — `last_damage_taken`, `last_hostile_fire_taken`, `last_weapon_fired`, `prev_hull`.
- `WeaponFiredThisTick` (`src/server_app.rs:82`) — set true when a weapon fires this tick.
- `ShipAttackedThisTick` (`src/server_app.rs:89`) — set true when hostile fire targets this ship this tick.
- `LastShipAttacker` (`src/console/weapons/server.rs:71`) — UUID of the most recent attacker.

Readers/writers updated:

- `update_combat_activity` (`src/ship/combat_activity.rs:20`) iterates `Query<..., With<Ship>>` — every ship tracks its own combat activity.
- `operate_captain_ai` (`src/console/captain/server.rs:127`) reads the ship's own `RecentCombatActivity` component (no more blackboard-fallback or resource-fallback branching) and loops over all ship entities where the Captain system is AI-controlled.
- `BeamStartedEvent` / `BeamEndedEvent` gained a `source_entity: Entity` field; the `on_beam_started` observer sets `WeaponFiredThisTick` on the correct firing ship and emits the correct `source_uuid`. Every trigger site (`handle_fire_phaser`, `tick_phaser_auto_fire`, `handle_npc_beam_fire`, `tick_active_beam`, `tick_npc_beams`) now passes the firing ship's entity.
- `handle_npc_beam_fire` and `tick_npc_beams` write `ShipAttackedThisTick` + `LastShipAttacker` on the *target* ship's per-entity components (looked up via `hull_query`).
- `handle_fire_torpedo` writes `WeaponFiredThisTick` on the LocalShip's per-entity component.
- `aggregate_doctrine_blackboards` (`src/ai/server.rs:293`) populates NPC viewscreen blackboards with their own `red_alert`, `last_damage_taken_secs`, `last_weapon_fired_secs`, and `last_attacker_uuid` — no more hardcoded `None` / `false`.
- `publish_viewscreen_blackboard` reads from the LocalShip's per-entity components (no `Option<Res<...>>` fallbacks).

**Goal B — Delete `ShipHullIntegrity`**

- `ShipHullIntegrity` struct definition deleted from `src/server_app.rs`.
- `sync_player_hull_to_resource` and `sync_resource_hull_to_entity` bridge systems deleted.
- All `init_resource::<ShipHullIntegrity>()` / `insert_resource(ShipHullIntegrity(...))` calls removed from production (`add_simulation_plugins`, `spawn_game_start_entities`) and from every test builder (`server_app`, `ship_plugin`, `ship/power`, `ship/sensors`, `ship/shields`, `regions/server`, `console/captain`, `console/weapons`, `console/repair`, `console/navigation`, `ship/combat_activity`).
- `debug_overlay::update_entity_inspector` migrated from `Res<ShipHullIntegrity>` to `Query<&EntitySystemHull, With<LocalShip>>`.
- Comment in `entities/spawner.rs` and `server/viewscreen_border.rs` updated to reflect that `EntitySystemHull` is the sole hull store.
- `spawn_game_start_entities` no longer synthesises a hull resource when the config has an empty `[hull]` block — `EntitySystemHull` is always inserted by `entity_spawner::spawn_entity` from the ship TOML.

**Goal C — Delete dead code**

- `EntityPhaserState` struct + `impl` in `src/ai/server.rs` deleted (0 production writers — only test scaffolding).
- `NpcHullFraction` struct + `impl` deleted (0 production writers — only test scaffolding).
- `detect_npc_hull_zero` system deleted (its only producer was the dead `NpcHullFraction`).
- `AiPlugin` registration for `detect_npc_hull_zero` removed.
- Associated tests deleted (`ai_entity_destroyed_event_emitted_when_hull_reaches_zero`, `entity_despawned_when_hull_reaches_zero`) — they were validating a code path that no production system ever reached.
- `entity_phaser_ready_true/false` tests rewritten as `npc_beam_ready_true_when_active_beam_inactive_and_no_cooldown` and `npc_beam_ready_false_when_cooldown_active` — they now exercise the real `ActiveBeam` + `PhaserCooldown` per-entity components.
- `server/pfx.rs` `sync_phaser_beams` `npc_beam_q` filter switched from `&EntityPhaserState` to `With<AiControllerComponent> + &ActiveBeam` — NPC beam rendering is now wired to the same `ActiveBeam` component that `tick_npc_beams` writes to (previously NPC beam rendering was dead code because nothing inserted `EntityPhaserState`).

**Goal D — beam-tick unification (completed 2026-07-02)**

The former `tick_active_beam` (player) and `tick_npc_beams` (NPC) have been
merged into a single `tick_beams` system (`src/console/weapons/server.rs:870`)
that iterates every ship via `Query<..., With<Ship>>`. Both former systems
have been deleted; `WeaponsPlugin::build` registers only `tick_beams`
(in `SimSet::Damage`).

Design highlights:

- Three-phase pattern to satisfy Bevy borrow-checker rules on nested queries:
  (1) snapshot per-shooter state (config, target position, damage integer,
  cooldown) and tick per-bank cooldowns; (2) apply damage to targets via
  `hull_q`, computing per-target destruction flags; (3) re-borrow `ship_q`
  to end beams (either due to target destruction or time expiry) and clear
  `WeaponsTarget` on the LocalShip.
- Every shooter reads its own `PhaserCombatConfigResource` component and its
  own `ShipModifiers` component (the global-Resource fallback for legacy test
  paths was removed by issue #606 — `ShipModifiers` is Component-only now).
- LocalShip target damage emits `DamageTaken` / `ShipDestroyed` / `GameOver`;
  non-LocalShip target damage despawns the target and emits `EntityDespawned`
  + `AiEntityDestroyed` (or `AsteroidDestroyed` + VFX for asteroids).
- Attacker tracking is uniform: every non-asteroid target has
  `ShipAttackedThisTick.0 = true`, `LastShipAttacker.0 = Some(shooter_uuid)`,
  and a fresh `AttackerThisTick` component inserted every tick the beam is
  live — so the target's AI `on_attacked` transition fires reliably.
- Also fixed as part of this PR: `handle_fire_torpedo` and
  `operate_tactical_ai` `player_ship_q` filters changed from `With<Ship>` to
  `With<LocalShip>` (Gap 2 — silently misidentified source UUIDs when
  multiple ships existed).

Verification:

- `grep 'fn tick_active_beam' src/` → 0.
- `grep 'fn tick_npc_beams' src/` → 0.
- `grep 'fn tick_beams' src/` → 1.
- No `is_npc` branches, no `fn npc_*()` helpers introduced.
- No new `Without<AiControllerComponent>` filters.
- All 1826 tests pass.

Verification:

- `grep 'ShipHullIntegrity' src/` → 2 comment-only historical mentions.
- `grep 'EntityPhaserState' src/` → 0.
- `grep 'NpcHullFraction' src/` → 0.
- Test count: 1828 → 1826 (net -2 for deleted `NpcHullFraction` tests; combat_activity tests updated in-place; captain AI tests updated in-place).
- No `is_npc` branches, no `fn npc_*()` helpers, no `Without<AiControllerComponent>` filters introduced.

## Key Decisions

- Per-entity Components are the source of truth for ship state; production readers query the LocalShip entity's Component (with Resource fallback used only for legacy test paths). Where a type is a pure per-entity Component with no Resource: `WeaponsTarget`, `ActiveBeam`, `PhaserCooldown`, `LastShipAttacker`, `SensorsTarget`, `NavigationWaypoint`, `RecentCombatActivity`, `WeaponFiredThisTick`, `ShipAttackedThisTick`, `CollisionCooldown`. Types still carrying `#[derive(Resource, Component)]` for backward compatibility with test scaffolding: `ShipModifiers`, `ShipRepairTeams`, `ShipPowerSystem`, `PowerConfigResource`, `PowerAiConfigResource`, `PowerMultiplierResource`, `PhaserCombatConfigResource`, `PhaserRenderConfig`, `TorpedoSystemResource`, `ShipPhysicsConfigResource`, `ImpulseConfigResource`, `BoostConfigResource`, `BankConfigResource`. Collapsing these to pure Component is a future cleanup.

  **Update (issue #606, 2026-07-04):** that future cleanup landed — partially. `ShipModifiers`, `ImpulseConfigResource`, and `BoostConfigResource` dropped the `Resource` derive and are now `Component`-only; every production fallback branch that read them as `Res`/`ResMut` was deleted, and tests that used to `insert_resource` them now spawn/insert the component on the ship entity instead. `ShipImpulse` and `ShipBoost` (the state types, as opposed to their `*ConfigResource` config counterparts) were also cut over to `Component`-only in the same commit. `ShipRepairTeams`, `ShipPowerSystem`, `PowerConfigResource`, `PowerAiConfigResource`, `PowerMultiplierResource`, `PhaserCombatConfigResource`, `PhaserRenderConfig`, `TorpedoSystemResource`, `ShipPhysicsConfigResource`, and `BankConfigResource` were explicitly left untouched (still dual-derive `Resource, Component`) — out of scope for #606.
- Each ship reads its own config from its TOML — including `[[station]]` and `[[system]]` blocks. No NPC-specific config helpers.
- `ShipShields` wraps the existing `ShieldSystem` (already supports configurable `num_facings`). `EntityShield` deleted in PR 2.
- Fog-of-war (NPC AI sensor range filtering) is out of scope for this PRD.
- `ShipImpulse` remains a player-only mechanic; NPCs do not have an impulse drive mechanic today. (It was a global Resource at the time this PRD shipped; issue #606 later made it a per-entity `Component` only, seeded on the player ship at spawn — still not present on NPC ships.)
- Beam-tick unification (`tick_active_beam` + `tick_npc_beams` → single `tick_beams`) completed 2026-07-02 as a follow-up to PR 10; the two former systems are deleted and every ship shares `tick_beams` in `SimSet::Damage`.

## Cross-references

- [PRD #581 — Unified Ship Entity Model](./prd-581-unified-ship-entity-model.md) — Phase 1 (completed)
- [AI Ship Unification concept](../concepts/ai-ship-unification.md)
