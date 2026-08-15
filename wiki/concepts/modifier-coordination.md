# Modifier Coordination

Single owner of the `ShipModifiers` lifecycle and a translator system for each
modifier source. All writes to `ShipModifiers` flow through this module.

## Per-entity migration (PRD #597 PR 6; Resource fallback deleted in #606)

After PR 6 of PRD #597 (2026-07-02), `ShipModifiers` also derived `Component`
alongside `Resource`. Every ship entity (player and NPC alike) carries a
per-entity `ShipModifiers` component. Issue #606 (2026-07-04) removed the
`Resource` derive entirely — `ShipModifiers` is now `Component`-only, and every
production fallback branch that read it as `Res<ShipModifiers>` /
`ResMut<ShipModifiers>` has been deleted. Tests that used to
`app.insert_resource(ShipModifiers::new())` now spawn/insert the component
directly on the ship entity instead.

Read patterns:

- Systems scoped to `With<LocalShip>` query the per-entity component on the
  LocalShip entity directly; there is no Resource fallback.
- `handle_slow_zone_speed_clamp` reads the SUBJECT entity's `ShipModifiers`
  component (so NPCs entering slow zones are affected by their own
  modifier cache — region membership tracks NPCs since PR 9).
- `modifier_events_broadcaster` drains `pending_events` from the LocalShip
  component only.

Write patterns (translators):

- `translate_power_modifiers` iterates every ship (`With<Ship>`) and writes
  directly to each ship's own `ShipModifiers` component — no dual-write.
- `translate_impulse_modifiers` writes to LocalShip's `ShipModifiers`
  component only.
- `on_region_entered` / `on_region_exited` observers write to the subject
  entity's `ShipModifiers` component only.

## Coordinator's role

`ModifierCoordinationPlugin` (`src/modifiers/coordination.rs:28`) no longer
calls `init_resource::<ShipModifiers>()` — that call site was deleted in #606
along with the `Resource` derive. `ShipModifiers` is inserted as a `Component`
on each ship entity at spawn time (see `entities/spawner.rs` for NPCs, the
player-ship spawn path in `server_app.rs` for the player) instead of being
initialised as a global resource.

Historically (pre-#606), three different plugins (`SimulationPlugin`,
`RegionPlugin`, and the old `modifiers.rs`) each called `init_resource`,
creating a soft contract violation: if any two ran in the wrong order the
second call would silently reset the resource, losing all modifier state. That
whole class of bug is now moot since there is no shared resource to race on.

Every other plugin reads `ShipModifiers` through `&ShipModifiers` query items
or writes through `&mut ShipModifiers` query items, scoped per-ship.

## Translator pattern

Each modifier source gets a **translator system** that reads the source's state
resource and calls a pure helper to update `ShipModifiers`. Translators are
registered in `Update` after the source's own systems, ensuring inputs have
settled before modifiers are recomputed.

> **Correction (2026-07-04 lint pass):** the registration snippet below was
> written against the pre-simulation-split `SimulationPlugin`/`simulation.rs`,
> which no longer exist (`server_app.rs` is the composition root now — see
> [Server App Composition](./server-app.md)). This is unrelated to issue #606;
> flagged here because the lint pass touched this page anyway.

The power and impulse translators are registered in `add_simulation_plugins`
at `src/server_app.rs:341-348`, both in `SimSet::Modifiers`:

```rust
.add_systems(Update, crate::modifier_coordination::translate_power_modifiers
    .in_set(crate::sim_sets::SimSet::Modifiers))
.add_systems(Update, crate::modifier_coordination::translate_impulse_modifiers
    .in_set(crate::sim_sets::SimSet::Modifiers))
```

There is no `translate_region_modifiers` system — region effects are applied
via the `on_region_entered` / `on_region_exited` observers instead (see below).
`handle_slow_zone_speed_clamp` lives in `src/regions/server.rs`, not a
`region_plugin` module.

### Power translator (`translate_power_modifiers`)

| Aspect | Reference |
|---|---|
| System | `src/modifiers/coordination.rs:48` |
| Pure helper | `apply_power_modifiers` at `src/modifiers/coordination.rs:122` |
| Reads | `ShipPowerSystem` (the `PowerSystem` struct) + `PowerMultiplierResource` |
| Writes | `ModifierSource::PowerGroup(PowerGroupId::helm())` → `MaxSpeed`, `MaxYawRate` |
|   | `ModifierSource::PowerGroup(PowerGroupId::weapons())` → `PhaserDamage` |
|   | `ModifierSource::PowerGroup(PowerGroupId::sensors())` → `RadarRange` |
| Ordering | `.after(handle_power_messages).after(tick_power_system)` |

Each power group's level (1–4) is mapped through a per-group multiplier
array indexed by `level - 1`. The default array is `[-0.5, 0.0, 0.25, 0.5]`.
Level 2 gives zero bonus; level 1 is a penalty; levels 3 and 4 are buffs. Prior
to #617/#619 this source variant was `ModifierSource::Console(Console::*)`
keyed on the Console enum; the enum is deleted, `PowerGroupId` is the
survivor.

### Region translator (`on_region_entered` / `on_region_exited` observers)

> **Correction (2026-07-04 lint pass):** this section previously described a
> `translate_region_modifiers` polling system. That system no longer exists in
> the codebase — region effects are applied via Bevy observers instead. This
> drift predates and is unrelated to issue #606; flagged here because the lint
> pass touched this page anyway. See `Open questions` below.

| Aspect | Reference |
|---|---|
| System | `on_region_entered` at `src/modifiers/coordination.rs:283`, `on_region_exited` at `src/modifiers/coordination.rs:302` (registered as observers in `ModifierCoordinationPlugin::build`, not `Update`-scheduled systems) |
| Pure helper | `apply_region_effects` at `src/modifiers/coordination.rs:180` |
| Reads | `RegionEntered` / `RegionExited` trigger payloads |
| Writes | Per `RegionEffectKind` variant (see below), on the trigger's subject entity |
| Ordering | N/A — observers fire synchronously when the event is triggered, not via `Update` ordering |

Effect-to-modifier mapping:

| `RegionEffectKind` variant | Modifier / flag | Source identity |
|---|---|---|
| `RadarDampening { multiplier }` | `ModifierSlot::RadarRange` with the given bonus | `RegionEffect { uuid }` |
| `SlowZone { thrust_modifier, yaw_rate_modifier }` | `MaxSpeed` (if thrust), `MaxYawRate` (if yaw) | `RegionEffect { uuid }` |
| `CommsJam` | `FlagKind::CommsJammed` (OR-aggregated) | `RegionEffect { uuid }` |
| `SensorBlind` | `FlagKind::SensorBlind` (OR-aggregated) | `RegionEffect { uuid }` |
| `DamageZone { .. }` | **Not a modifier** — handled directly by regions plugin | N/A |
| `BlocksImpulse` | **Not a modifier** — handled directly by regions plugin | N/A |

Every value in the first two rows is a **signed bonus, not a multiplier** —
including the one whose serde alias is literally `multiplier`. It is summed onto
the slot and resolved by `ShipModifiers::rebuild_cache`'s two-sided formula
(`1 + sum` when non-negative, `1 / (1 + |sum|)` when negative), so an effect
that makes a ship WORSE authors a NEGATIVE number, and the bonus for a wanted
multiplier `m` is `-(1/m - 1)`. Both hazard effects shipped with the sign
inverted. Two templates authored a positive `range_modifier` on
`radar_dampening`, which made the radar reach further inside the hazard than
outside it; the two storm bands authored positive `thrust_modifier` /
`yaw_rate_modifier` on `slow_zone`, which made ships fly FASTER and turn harder
inside a front than in clear space (`region_storm_band.toml` at 1.5x/1.6x and
`region_radiation_band.toml` at 1.6x/1.7x). Each is now held by a pair of
guards — one over the data, one through the engine:

| effect | data-side | runtime |
|---|---|---|
| `radar_dampening` | `regions::effects::shipped_assets::every_shipped_radar_dampening_actually_dampens` | `regions::server::tests::every_shipped_dampening_region_shortens_the_radar_it_is_entered_with` |
| `slow_zone` | `regions::effects::shipped_assets::every_shipped_slow_zone_actually_slows` | `regions::server::tests::every_shipped_slow_zone_slows_the_ship_that_enters_it` |

The two defects differ in what they reach. `RadarRange` is folded into no
digest, so correcting it moved none; `MaxSpeed` IS folded — `ship::physics_systems`
multiplies the hull's authored `max_speed` by the slot before the helm
integrates — but a speed cap only binds on a ship pushing it, and every hull
standing in a shipped band is station-keeping, so that correction moved no
digest either. Both facts are measured (`±999` A/Bs) rather than reasoned, in
the world headers of `probe_operations`, `probe_storm`, `probe_destroy` and
`falling_skyway`.

A `slow_zone` with neither field authored is not a sign error but the presence
marker an operation's `[[operations.capability.interrupt]]` names; its rate
lives on the capability as `rate_percent`, which is a true percentage and was
never affected. Both `slow_zone` guards skip that shape.

On region exit, `on_region_exited` calls `modifiers.clear_source()`
with the leaving region's `ModifierSource::RegionEffect { uuid }`, removing all
modifiers and flags that originated from that region. This is how stale modifier
accumulation is prevented.

A companion system `handle_slow_zone_speed_clamp` at `src/regions/server.rs:342`
reads the SUBJECT ship's `ShipModifiers` component (query, not `Res`) and
clamps that ship's forward speed so the effective max reflects the slow-zone
modifier.

### Impulse translator (`translate_impulse_modifiers`)

| Aspect | Reference |
|---|---|
| System | `src/modifiers/coordination.rs:259` |
| Pure helper | `apply_impulse_to` at `src/modifiers/coordination.rs:236` |
| Reads | `ShipImpulse` (the `ImpulseState` struct) |
| Writes | `ModifierSource::ImpulseDrive` → `MaxSpeed` (active only) |
| Ordering | `.after(handle_impulse_messages)` |

When `ImpulsePhase::Active`, registers a `MaxSpeed` modifier with bonus
`speed_multiplier - 1.0` under `ModifierSource::ImpulseDrive`. The
`speed_multiplier` value is read from `ImpulseConfigResource` (populated from
`[helm_console].impulse_speed_multiplier` in `assets/entities/player_ship.toml`);
the `IMPULSE_SPEED_MULTIPLIER` const is kept only as the resource `Default`.
When idle or charging, the modifier is removed so the cache returns to the
identity multiplier.

The system uses `Local<Option<ImpulsePhase>>` to change-detect on phase
transitions, avoiding redundant modifier events when the phase hasn't changed.

### Console-AI translator (planned)

Reads console-complexity state, translates AI decisions into the modifier
cache. Not yet implemented.

## Pure helpers

### `apply_power_modifiers(modifiers, power, multipliers)`

`src/modifiers/coordination.rs:122`. Non-Bevy, fully unit-tested. Computes the
per-console bonus from the current power level and writes `Modifier` entries
using `ModifierSource::Console(console)`. Re-calling with the same input is
idempotent — `add_or_update` replaces the previous entry rather than stacking.

### `apply_region_effects(modifiers, region_uuid, effects)`

`src/modifiers/coordination.rs:180`. Non-Bevy, fully unit-tested. Iterates a
slice of `RegionEffectKind` and registers the corresponding modifiers and flags
under `ModifierSource::RegionEffect { uuid: region_uuid }`.

### `apply_impulse_to(modifiers, impulse, speed_multiplier)`

`src/modifiers/coordination.rs:236`. Non-Bevy, fully unit-tested. When the
impulse drive is active (`is_active()`), writes a `MaxSpeed` modifier with
bonus `speed_multiplier - 1.0` under `ModifierSource::ImpulseDrive`. When
idle or charging, removes that modifier so the cache returns to the
identity multiplier. Note: the per-tick **acceleration** boost is applied
separately inside `process_helm_inputs` (`src/ship/helm_admission.rs`) by multiplying
`ShipPhysicsConfig.acceleration` by `ImpulseConfigResource.acceleration_multiplier`;
it does not flow through `ShipModifiers`. `ImpulseConfigResource` is a
`Component` only (its `Resource` derive was removed in issue #606, same as
`ShipModifiers`).

## Read interface for consumers

`ShipModifiers` is a per-entity `Component` (as of issue #606, no `Resource`
form exists at all). Consumers never hold a mutable reference outside the
translators/observers above — they query read-only. The canonical way to read
modifier state:

```rust
fn my_system(modifiers_q: Query<&ShipModifiers, With<LocalShip>>) {
    let Ok(modifiers) = modifiers_q.single() else { return };
    let speed_mult = modifiers.get(&ModifierSlot::MaxSpeed);
    let is_jammed = modifiers.has_flag(&FlagKind::CommsJammed);
    let all_flags = modifiers.flags();
}
```

Key `ShipModifiers` methods (defined in `src/modifiers/cache.rs`):

| Method | Returns | Description |
|---|---|---|
| `get(&ModifierSlot) -> f32` | The computed multiplier (identity=1.0) | Sum all bonuses for that slot; positive → `1+sum`, negative → `1/(1+\|sum\|)` |
| `has_flag(&FlagKind) -> bool` | `true` if any source has set the flag | OR-aggregated across all sources |
| `flags() -> Vec<FlagKind>` | All currently set flags | Snapshot for broadcast |

Current consumers of the per-entity `ShipModifiers` component (all query
`&ShipModifiers`, none hold `Res<ShipModifiers>` — that type does not exist
post-#606):

| Location | What it reads |
|---|---|
| `src/console/weapons/beam.rs` — `handle_set_target` | `ModifierSlot::RadarRange` for the human target-lock range gate (ACQUISITION) |
| `src/console/weapons/mod.rs` — `ai_target_selection` | `ModifierSlot::RadarRange` to bound the AI selector's candidate horizon — the same acquisition gate `handle_set_target` applies to a human, which is what keeps the two symmetric |
| `src/console/weapons/blackboard.rs` — `publish_tactical_radar_blackboard` | `ModifierSlot::RadarRange` to bound the tactical radar blips the local ship renders |
| `src/ship/helm_admission.rs` — `process_helm_inputs` | `ModifierSlot::MaxSpeed` for acceleration/reverse-speed caps |
| `src/server_app.rs:798` — `handle_collisions` | `ModifierSlot::HullDamageTaken` to scale collision damage |
| `src/console/weapons/beam.rs` — `tick_beams_prepare` | `ModifierSlot::PhaserDamage` to scale beam DPS (per shooter). **Not** `RadarRange`: since issue #955 nothing scales a weapon's reach, which is its authored `beam_range` |
| `src/console/repair/server.rs:176` — `tick_repair_teams` | `ModifierSlot::RepairRate` to scale repair speed |
| `src/regions/server.rs:342` — `handle_slow_zone_speed_clamp` | `ModifierSlot::MaxSpeed` to clamp ship speed on slow-zone entry |

## How `RegionEffect { uuid }` source identity prevents stale accumulation

When a region is exited, `translate_region_modifiers` calls
`modifiers.clear_source(&ModifierSource::RegionEffect { uuid })`. This removes
every modifier and flag whose source matches that specific UUID. Because each
region entity has a unique UUID (from its TOML config via `EntityUuid`), exit
of region A only cleans up region A's effects. If region B is still active, its
effects remain.

This solves the stale-modifier problem: without per-source identity, exiting
one region would require remembering which effects it contributed, and
re-entering a different region with overlapping effects would risk double-counting
or incorrect cleanup. With UUID-keyed sources, cleanup is always a single
`clear_source()` call keyed on the exact region that was left.

The OR-aggregated flag system (`FlagKind::CommsJammed`, `FlagKind::SensorBlind`)
works the same way: `add_flag(source, flag)` adds the source to the flag's
source set. `remove_flag(source, flag)` removes it. The flag is unset only when
the last source is removed. This means two overlapping jammer regions both set
`CommsJammed`, and exiting either one does not clear the flag — it stays set
until the last jammer region is left.

## How to add a new source (recipe)

1. **Add a `ModifierSource` variant** in `src/core/messages.rs:22`. If the
   source needs identity (like regions), include an identifier field (e.g.
   `Uuid`, `Entity`).

2. **Write a pure helper** in `src/modifiers/coordination.rs`:
   ```rust
   pub fn apply_<source>_to(modifiers: &mut ShipModifiers, state: &SourceState) {
       // call modifiers.add_or_update(...) and/or modifiers.add_flag(...)
   }
   ```

3. **Write a translator system** in the same file. `ShipModifiers` is
   `Component`-only since issue #606 — query it per-ship, never `Res`/`ResMut`:
   ```rust
   pub fn translate_<source>_modifiers(
       source_state: Res<SourceResource>,
       mut ships_q: Query<&mut ShipModifiers, With<LocalShip>>,
   ) {
       // gate on GamePhase::InProgress if needed
       let Ok(mut modifiers) = ships_q.single_mut() else { return };
       apply_<source>_to(&mut modifiers, &source_state.0);
   }
   ```

4. **Register the system** in `add_simulation_plugins` at
   `src/server_app.rs` (see the power/impulse translator registrations above
   for the current pattern):
   ```rust
   .add_systems(Update, crate::modifier_coordination::translate_<source>_modifiers
       .in_set(crate::sim_sets::SimSet::Modifiers))
   ```

5. **Unit-test the pure helper** in the `#[cfg(test)]` block of
   `coordination.rs`. Cover: no effect when source is inactive, correct
   bonus/slot mapping when active, removal when source goes away, and
   idempotent re-application.

6. **Update serialization** for the new `ModifierSource` variant in
   `src/core/messages.rs:30-46` if it introduces new hash/eq behaviour.

## ModifierSlot catalogue

All six modifier slots, defined in `src/core/messages.rs:11`:

| Slot | Meaning | Default multiplier |
|---|---|---|
| `MaxSpeed` | Ship forward speed cap | 1.0 |
| `MaxYawRate` | Ship turn rate cap | 1.0 |
| `RadarRange` | Tactical radar horizon: blips and the range a target lock may be taken at. **Acquisition only** — it does not scale weapon reach (issue #955) | 1.0 |
| `PhaserDamage` | Damage per phaser tick | 1.0 |
| `HullDamageTaken` | Incoming damage multiplier (penalty) | 1.0 |
| `RepairRate` | Repair team tick speed | 1.0 |

## ModifierSource catalogue

All three source variants, defined in `src/core/messages.rs`:

| Variant | Identity | Used by |
|---|---|---|
| `PowerGroup(PowerGroupId)` | Per-power-group id (`"helm"`, `"weapons"`, `"sensors"`) | Power translator |
| `ImpulseDrive` | Singleton | Impulse translator |
| `RegionEffect { uuid }` | Per-region UUID | Region translator |

The `Console(Console)` variant that appeared here pre-#619 was deleted with
the Console enum; `PowerGroup(PowerGroupId)` is the survivor for power-driven
modifiers.

## Files

| File | Role |
|---|---|
| `src/modifiers/coordination.rs` | Plugin + pure helpers + translator systems (power + impulse) + region observers |
| `src/modifiers/cache.rs` | `ShipModifiers` — per-entity `Component` (no `Resource` since #606) + `Modifier` type + `ModifierEvent` enum |
| `src/modifiers/mod.rs` | Module re-exports (`pub use cache::...`) |
| `src/core/messages.rs` | `ModifierSlot`, `ModifierSource` enum definitions |
| `src/regions/server.rs` | Region membership detection + `handle_slow_zone_speed_clamp` (read-only consumer) |

## Open questions

- The "region translator" was described here as a polling system
  (`translate_region_modifiers`) registered after `update_region_membership`.
  That system does not exist in the current codebase — region effects are
  applied via `on_region_entered`/`on_region_exited` observers instead (fixed
  in this lint pass). This drift predates issue #606 and its origin (which PR
  converted the region translator to observers) is not otherwise documented in
  the wiki; worth tracking down if a future source ingest touches regions.

## Related

- [Architecture](./architecture.md) — Where the coordinator fits in the plugin map.
- [Server App Composition](./server-app.md) — Composition root that registers the translators.
- [PRD #117](https://github.com/jkeywo/project-phoenix-v2/issues/117) — Modifier system (`modifiers.rs` cache + wire).
- [PRD #118](https://github.com/jkeywo/project-phoenix-v2/issues/118) — Power console (6+2 power allocation driving `ModifierSource::Console`).
- [PRD #153](https://github.com/jkeywo/project-phoenix-v2/issues/153) — Region entities (`RegionEffectKind`, `EntityUuid` driving `ModifierSource::RegionEffect`).
- PRD #597 — Ship Parity — Introduced the per-entity `ShipModifiers` Component (PR 6); issue #606 later deleted the `Resource` half.
- [Broadcaster Seam](./broadcaster-seam.md) — How modifier events (`ModifierAdded`/`ModifierRemoved`) are broadcast to clients.
