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

Each modifier source (power, regions, console-AI) gets a *translator system*
that reads the source's state resource and calls a pure helper to update
`ShipModifiers`.  Translators are registered in `Update` after the source's
own systems, ensuring inputs have settled before modifiers are recomputed.

### Power translator (first source)

Located at `src/modifiers/coordination.rs:43`.  System
`translate_power_modifiers` reads `ShipPowerSystem` + `PowerMultiplierResource`
and calls `apply_power_modifiers()` into `ResMut<ShipModifiers>`.

The system is ordered `.after(handle_power_messages).after(tick_power_system)`
so that power-level changes (increase, decrease, battery-exhaustion resets)
are applied to `ShipModifiers` in the same frame.

### Future translators

- **Region translator** -- reads `RegionMembership` + region config, writes
  region-effect modifiers into `ShipModifiers`.  Planned.
- **Console-AI translator** -- reads console-complexity state, translates AI
  decisions into the modifier cache.  Planned.

## Pure helpers

### `apply_power_modifiers(modifiers, power, multipliers)`

`src/modifiers/coordination.rs:29`.  Non-Bevy, fully unit-tested.  Computes
the per-console bonus from the current power level and writes
`Modifier` entries using `ModifierSource::Console(console)`.  Re-calling with
the same input is idempotent (add_or_update replaces, not stacks).

## Files

| File | Role |
|---|---|
| `src/modifiers/coordination.rs` | Plugin + pure helper + translator system |
| `src/modifiers/cache.rs` | `ShipModifiers` resource definition |
| `src/modifiers/mod.rs` | Module re-exports |
