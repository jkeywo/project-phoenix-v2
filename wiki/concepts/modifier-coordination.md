---
title: Modifier Coordination
type: concept
tags: [modifiers, power, regions, impulse, collision, repair]
sources: [crates/phoenix-sim-gameplay/src/modifiers/cache.rs, crates/phoenix-sim-gameplay/src/modifiers/coordination.rs, crates/phoenix-simulation/src/server_app/registration.rs, crates/phoenix-simulation/src/server_app/collision.rs, crates/phoenix-simulation/src/regions/server.rs, crates/phoenix-model/src/messages.rs, crates/phoenix-simulation/src/snapshot/mod.rs]
updated: 2026-10-14
---

# Modifier Coordination

`ShipModifiers` is the per-ship cache of keyed, composable effects. Domains write entries identified by `ModifierSource` and `ModifierSlot`; gameplay consumers read the resulting aggregate. The cache prevents unrelated systems from overwriting one another's contribution.

## Ownership

`crates/phoenix-sim-gameplay/src/modifiers/cache.rs` owns storage, recomputation, and snapshot representation. `ModifierCoordinationPlugin` in `crates/phoenix-sim-gameplay/src/modifiers/coordination.rs` owns adapters for sources that otherwise would write the cache directly:

- damage produces equipment penalties;
- retired allocation and unconditional impulse contributions are cleared;
- region enter/exit observers add or remove radar, speed, and yaw effects.

Actual impulse and boost effects are resolved with their local energy grants by
physics. The compatibility translator clears the former unconditional speed
contribution, including after snapshot restore. Allocation never amplifies a
modifier slot; machinery consumes actual energy at its owning work boundary.
The compatibility cleanup retains its component-first/resource-only fallback
boundary and removes the same retired keys directly, without building an unused
allocation snapshot or multiplier defaults.

Each adapter updates only its own source keys. Removing one effect recomputes the slot from the remaining contributions. Float bonuses sum first: a nonnegative sum produces `1 + sum`, while a negative sum produces `1 / (1 + abs(sum))`. Integer bonuses sum directly. Replacing or removing one source therefore preserves the other sources' contributions.

## Consumers

The main consumers are:

- ship physics (`MaxSpeed`, `MaxYawRate`);
- Tactical target acquisition (`RadarRange`), Helm radar (`HelmRadarRange`), and Sensors (`SensorRadarRange`);
- beam damage (`PhaserDamage`);
- shield recovery (`ShieldRegen`);
- collision damage (`HullDamageTaken`) in `crates/phoenix-simulation/src/server_app/collision.rs`;
- internal repair progress (`RepairRate`);
- region entry clamping (`MaxSpeed`) in `crates/phoenix-simulation/src/regions/server.rs`.

Weapon firing range is authored on the weapon. `RadarRange` limits acquisition, not a beam's reach.

## Ordering

The registration root places modifier translation after its authoritative source state has been applied and before consumers that require the new aggregate. Observers use the same keyed cache and do not introduce parallel authoritative state.

## Adding a modifier

1. Add or reuse a typed `ModifierSlot`/`ModifierSource` in the wire vocabulary.
2. Put the authoritative fact in its owning domain.
3. Add one coordinator adapter that writes the source's keyed entry.
4. Read the aggregate only at the gameplay consumer.
5. Test composition and removal with at least one other source present.

## Related

- [Power Runtime](./power-plugin.md)
- [Ship Physics](./ship-physics.md)
- [Damage and Repair Intent](./damage-and-repair-intent.md)
