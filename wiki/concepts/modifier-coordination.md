# Modifier Coordination

Single owner of the `ShipModifiers` lifecycle and a translator system for each
modifier source. All writes to `ShipModifiers` flow through this module.

## Per-entity migration (PRD #597 PR 6)

After PR 6 of PRD #597 (2026-07-02), `ShipModifiers` also derives `Component`
alongside `Resource`. Every ship entity (player and NPC alike) carries a
per-entity `ShipModifiers` component; the global Resource is kept as a
backward-compat fallback and is dual-written by the translators/observers when
both the Component and the Resource are present.

Read patterns:

- Systems scoped to `With<LocalShip>` prefer the per-entity component on the
  LocalShip entity and fall back to `Res<ShipModifiers>`.
- `handle_slow_zone_speed_clamp` reads the SUBJECT entity's `ShipModifiers`
  component (so NPCs entering slow zones will be affected by their own
  modifier cache once region membership tracks NPCs in PR 9).
- `modifier_events_broadcaster` drains `pending_events` from the LocalShip
  component with a Resource fallback.

Write patterns (translators):

- `translate_power_modifiers` reads per-entity power/multipliers on LocalShip
  first, writes to LocalShip's `ShipModifiers` component, then dual-writes the
  Resource.
- `translate_impulse_modifiers` writes to LocalShip's `ShipModifiers`
  component, dual-writes the Resource.
- `on_region_entered` / `on_region_exited` observers write to the subject
  entity's `ShipModifiers` component; when the subject is LocalShip they also
  dual-write the global Resource.

## Coordinator's role

`ModifierCoordinationPlugin` (`src/modifiers/coordination.rs:24`) is the sole
call site for `init_resource::<ShipModifiers>()` (`src/modifiers/coordination.rs:28`).
Before this seam, three different plugins (`SimulationPlugin`, `RegionPlugin`,
and the old `modifiers.rs`) each called `init_resource`, creating a soft
contract violation: if any two ran in the wrong order the second call would
silently reset the resource, losing all modifier state.

Every other plugin reads `ShipModifiers` through `Res<ShipModifiers>` or writes
through `ResMut<ShipModifiers>` after this plugin has initialised it. The
`init_resource` call happens in `ModifierCoordinationPlugin::build()`, which is
registered in both `src/bridge.rs:12` and `src/server/bridge.rs:12` (server
build) and in `src/regions/server.rs:725` / `:852` / `:1062` (test app builders).

A grep for `init_resource.*ShipModifiers` confirms exactly one call site remains
in the entire codebase.

## Translator pattern

Each modifier source gets a **translator system** that reads the source's state
resource and calls a pure helper to update `ShipModifiers`. Translators are
registered in `Update` after the source's own systems, ensuring inputs have
settled before modifiers are recomputed.

All three translators are registered in `SimulationPlugin` at
`src/simulation.rs:370-379`:

```rust
.add_systems(Update, crate::modifier_coordination::translate_power_modifiers
    .after(handle_power_messages).after(tick_power_system))
.add_systems(Update, crate::modifier_coordination::translate_impulse_modifiers
    .after(handle_impulse_messages))
.add_systems(Update, (
    crate::modifier_coordination::translate_region_modifiers,
    crate::region_plugin::handle_slow_zone_speed_clamp,
).chain().after(crate::region_plugin::update_region_membership))
```

### Power translator (`translate_power_modifiers`)

| Aspect | Reference |
|---|---|
| System | `src/modifiers/coordination.rs:43` |
| Pure helper | `apply_power_modifiers` at `src/modifiers/coordination.rs:62` |
| Reads | `ShipPowerSystem` (the `PowerSystem` struct) + `PowerMultiplierResource` |
| Writes | `ModifierSource::Console(Console::Helm)` → `MaxSpeed`, `MaxYawRate` |
|   | `ModifierSource::Console(Console::Tactical)` → `PhaserDamage` |
|   | `ModifierSource::Console(Console::Sensors)` → `RadarRange` |
| Ordering | `.after(handle_power_messages).after(tick_power_system)` |

Each console's power level (1–4) is mapped through a per-console multiplier
array indexed by `level - 1`. The default array is `[-0.5, 0.0, 0.25, 0.5]`.
Level 2 gives zero bonus; level 1 is a penalty; levels 3 and 4 are buffs.

### Region translator (`translate_region_modifiers`)

| Aspect | Reference |
|---|---|
| System | `src/modifiers/coordination.rs:194` |
| Pure helper | `apply_region_effects` at `src/modifiers/coordination.rs:104` |
| Reads | `RegionEntered` / `RegionExited` events, `RegionMembership` resource |
| Writes | Per `RegionEffectKind` variant (see below) |
| Ordering | `.chain().after(update_region_membership)` |

Effect-to-modifier mapping:

| `RegionEffectKind` variant | Modifier / flag | Source identity |
|---|---|---|
| `RadarDampening { multiplier }` | `ModifierSlot::RadarRange` with the given bonus | `RegionEffect { uuid }` |
| `SlowZone { thrust_modifier, yaw_rate_modifier }` | `MaxSpeed` (if thrust), `MaxYawRate` (if yaw) | `RegionEffect { uuid }` |
| `CommsJam` | `FlagKind::CommsJammed` (OR-aggregated) | `RegionEffect { uuid }` |
| `SensorBlind` | `FlagKind::SensorBlind` (OR-aggregated) | `RegionEffect { uuid }` |
| `DamageZone { .. }` | **Not a modifier** — handled directly by regions plugin | N/A |
| `BlocksImpulse` | **Not a modifier** — handled directly by regions plugin | N/A |

On region exit, `translate_region_modifiers` calls `modifiers.clear_source()`
with the leaving region's `ModifierSource::RegionEffect { uuid }`, removing all
modifiers and flags that originated from that region. This is how stale modifier
accumulation is prevented.

A companion system `handle_slow_zone_speed_clamp` at `src/regions/server.rs:191`
reads the updated `Res<ShipModifiers>` and clamps the ship's forward speed so
the effective max reflects the slow-zone modifier.

### Impulse translator (`translate_impulse_modifiers`)

| Aspect | Reference |
|---|---|
| System | `src/modifiers/coordination.rs:174` |
| Pure helper | `apply_impulse_to` at `src/modifiers/coordination.rs:155` |
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

`src/modifiers/coordination.rs:62`. Non-Bevy, fully unit-tested. Computes the
per-console bonus from the current power level and writes `Modifier` entries
using `ModifierSource::Console(console)`. Re-calling with the same input is
idempotent — `add_or_update` replaces the previous entry rather than stacking.

### `apply_region_effects(modifiers, region_uuid, effects)`

`src/modifiers/coordination.rs:104`. Non-Bevy, fully unit-tested. Iterates a
slice of `RegionEffectKind` and registers the corresponding modifiers and flags
under `ModifierSource::RegionEffect { uuid: region_uuid }`.

### `apply_impulse_to(modifiers, impulse, speed_multiplier)`

`src/modifiers/coordination.rs:155`. Non-Bevy, fully unit-tested. When the
impulse drive is active (`is_active()`), writes a `MaxSpeed` modifier with
bonus `speed_multiplier - 1.0` under `ModifierSource::ImpulseDrive`. When
idle or charging, removes that modifier so the cache returns to the
identity multiplier. Note: the per-tick **acceleration** boost is applied
separately inside `process_helm_inputs` (`src/ship_plugin.rs`) by multiplying
`ShipPhysicsConfig.acceleration` by `ImpulseConfigResource.acceleration_multiplier`;
it does not flow through `ShipModifiers`.

## Read interface for consumers

Consumers never hold `ResMut<ShipModifiers>` — they are read-only. The
canonical way to read modifier state:

```rust
fn my_system(modifiers: Res<ShipModifiers>) {
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

Current consumers of `Res<ShipModifiers>`:

| Location | What it reads |
|---|---|
| `src/simulation.rs:654` — `handle_set_target` | `ModifierSlot::RadarRange` for target-lock range gate |
| `src/simulation.rs:752` — `handle_helm_input` | `ModifierSlot::MaxSpeed` and `ModifierSlot::MaxYawRate` for acceleration/steering caps |
| `src/simulation.rs:825` — `tick_collisions` | `ModifierSlot::HullDamageTaken` to scale collision damage |
| `src/simulation.rs:936` — `handle_fire_phaser` | `ModifierSlot::RadarRange` to scale effective phaser range |
| `src/simulation.rs:1298` — `tick_repair` | `ModifierSlot::RepairRate` to scale repair speed |
| `src/simulation.rs:1328` — `tick_active_beam` | `ModifierSlot::PhaserDamage` to scale beam DPS |
| `src/regions/server.rs:194` — `handle_slow_zone_speed_clamp` | `ModifierSlot::MaxSpeed` to clamp ship speed on slow-zone entry |

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

3. **Write a translator system** in the same file:
   ```rust
   pub fn translate_<source>_modifiers(
       source_state: Res<SourceResource>,
       mut modifiers: ResMut<ShipModifiers>,
   ) {
       // gate on GamePhase::InProgress if needed
       apply_<source>_to(&mut modifiers, &source_state.0);
   }
   ```

4. **Register the system** in `SimulationPlugin` at `src/simulation.rs:370`:
   ```rust
   .add_systems(Update, crate::modifier_coordination::translate_<source>_modifiers
       .after(<source's mutating systems>))
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
| `RadarRange` | Radar / phaser lock range | 1.0 |
| `PhaserDamage` | Damage per phaser tick | 1.0 |
| `HullDamageTaken` | Incoming damage multiplier (penalty) | 1.0 |
| `RepairRate` | Repair team tick speed | 1.0 |

## ModifierSource catalogue

All three source variants, defined in `src/core/messages.rs:22`:

| Variant | Identity | Used by |
|---|---|---|
| `Console(Console)` | Per-console variant | Power translator |
| `ImpulseDrive` | Singleton | Impulse translator |
| `RegionEffect { uuid }` | Per-region UUID | Region translator |

## Files

| File | Role |
|---|---|
| `src/modifiers/coordination.rs` | Plugin + pure helpers + translator systems (power + region + impulse) |
| `src/modifiers/cache.rs` | `ShipModifiers` resource + `Modifier` type + `ModifierEvent` enum |
| `src/modifiers/mod.rs` | Module re-exports (`pub use cache::...`) |
| `src/core/messages.rs` | `ModifierSlot`, `ModifierSource` enum definitions |
| `src/regions/server.rs` | Region membership detection + `handle_slow_zone_speed_clamp` (read-only consumer) |

## Related

- [Architecture](./architecture.md) — Where the coordinator fits in the plugin map.
- [PRD #117](https://github.com/jkeywo/project-phoenix-v2/issues/117) — Modifier system (`modifiers.rs` cache + wire).
- [PRD #118](https://github.com/jkeywo/project-phoenix-v2/issues/118) — Power console (6+2 power allocation driving `ModifierSource::Console`).
- [PRD #153](https://github.com/jkeywo/project-phoenix-v2/issues/153) — Region entities (`RegionEffectKind`, `EntityUuid` driving `ModifierSource::RegionEffect`).
- [Broadcaster Seam](./broadcaster-seam.md) — How modifier events (`ModifierAdded`/`ModifierRemoved`) are broadcast to clients.
