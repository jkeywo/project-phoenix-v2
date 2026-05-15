# Modifier Coordination

Single owner of the `ShipModifiers` lifecycle and a translator system for each
modifier source.

## Purpose

Before this seam, `ShipModifiers` was `init_resource`'d by three different
plugins (`SimulationPlugin`, `RegionPlugin`, and the old `modifiers.rs`).
Each call was a soft contract violation -- if any two plugins ran in the wrong
order the second `init_resource` would silently reset the resource, losing
modifier state.

`ModifierCoordinationPlugin` (`src/modifiers/coordination.rs:15`) is the sole
call site for `init_resource::<ShipModifiers>()`.  Every other plugin reads or
writes `ShipModifiers` through `Res`/`ResMut` after this plugin has
initialised it.

## Translator pattern

Each modifier source (power, regions, impulse, console-AI) gets a *translator system*
that reads the source's state resource and calls a pure helper to update
`ShipModifiers`.  Translators are registered in `Update` after the source's
own systems, ensuring inputs have settled before modifiers are recomputed.

### Power translator (first source)

Located at `src/modifiers/coordination.rs:38`.  System
`translate_power_modifiers` reads `ShipPowerSystem` + `PowerMultiplierResource`
and calls `apply_power_modifiers()` into `ResMut<ShipModifiers>`.

The system is ordered `.after(handle_power_messages).after(tick_power_system)`
so that power-level changes (increase, decrease, battery-exhaustion resets)
are applied to `ShipModifiers` in the same frame.

### Region translator (second source)

Located at `src/modifiers/coordination.rs:142`.  System
`translate_region_modifiers` reads `RegionEntered` / `RegionExited` messages
each frame.  On region entry it calls `apply_region_effects()` to write
modifiers and flags; on region exit it calls `clear_source()` to remove all
modifier entries and flags for the exiting region's `ModifierSource::RegionEffect`
UUID.

The system is ordered `.chain().after(crate::region_plugin::update_region_membership)`
so that boundary-crossing detection has settled before effects are applied.
A companion system `handle_slow_zone_speed_clamp` in `region_plugin.rs`
reads the updated modifiers and clamps the ship's forward speed on slow-zone
entry; it is chained after `translate_region_modifiers` in `SimulationPlugin`.

Effects translated by this system:
- `RadarDampening { multiplier }` → `ModifierSlot::RadarRange` modifier
- `SlowZone { thrust_modifier, yaw_rate_modifier }` → `MaxSpeed` / `MaxYawRate` modifiers
- `CommsJam` → `FlagKind::CommsJammed` (OR-aggregated across sources)
- `SensorBlind` → `FlagKind::SensorBlind` (OR-aggregated across sources)
- `DamageZone` and `BlocksImpulse` are **not** modifier effects and are
  handled directly by `region_plugin.rs`.

### Impulse translator (third source)

Located at `src/modifiers/coordination.rs:146`.  System
`translate_impulse_modifiers` reads `ShipImpulse` and calls
`apply_impulse_to()` into `ResMut<ShipModifiers>`.  Change-detects
`ImpulsePhase` via a `Local<Option<ImpulsePhase>>` so it only writes on
transitions, avoiding redundant modifier events.

When impulse is active (`ImpulsePhase::Active`) it registers a
`ModifierSlot::MaxSpeed` bonus of `IMPULSE_SPEED_MULTIPLIER - 1.0` under
`ModifierSource::ImpulseDrive`.  When impulse is idle or charging the
modifier is removed.

The system is ordered `.after(handle_impulse_messages)` so that
start/cancel decisions settle before the modifier cache is updated.

### Console-AI translator (future)

Reads console-complexity state, translates AI decisions into the modifier
cache.  Planned.

## Pure helpers

### `apply_power_modifiers(modifiers, power, multipliers)`

`src/modifiers/coordination.rs:57`.  Non-Bevy, fully unit-tested.  Computes
the per-console bonus from the current power level and writes
`Modifier` entries using `ModifierSource::Console(console)`.  Re-calling with
the same input is idempotent (add_or_update replaces, not stacks).

### `apply_region_effects(modifiers, region_uuid, effects)`

`src/modifiers/coordination.rs:104`.  Non-Bevy, fully unit-tested.  Iterates
a slice of `RegionEffectKind` and registers the corresponding modifiers and
flags under `ModifierSource::RegionEffect { uuid: region_uuid }`.

### `apply_impulse_to(modifiers, impulse)`

`src/modifiers/coordination.rs:155`.  Non-Bevy, fully unit-tested.  When the
impulse drive is active (`is_active()`), writes a `MaxSpeed` modifier with
bonus `IMPULSE_SPEED_MULTIPLIER - 1.0` under `ModifierSource::ImpulseDrive`.
When idle or charging, removes that modifier so the cache returns to the
identity multiplier.

## Files

| File | Role |
|---|---|
| `src/modifiers/coordination.rs` | Plugin + pure helpers + translator systems (power + region + impulse) |
| `src/modifiers/cache.rs` | `ShipModifiers` resource definition |
| `src/modifiers/mod.rs` | Module re-exports |
| `src/regions/server.rs` | Region membership detection + speed clamp (no modifier writes) |
