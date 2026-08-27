---
title: Modifier Coordination
type: concept
tags: [modifiers, power, regions, impulse, collision, repair]
sources: [src/modifiers/cache.rs, src/modifiers/coordination.rs, src/server_app/registration.rs, src/server_app/collision.rs, src/regions/server.rs, src/core/messages.rs]
updated: 2026-08-27
---

# Modifier Coordination

`ShipModifiers` is the per-ship cache of keyed, composable effects. Domains write entries identified by `ModifierSource` and `ModifierSlot`; gameplay consumers read the resulting aggregate. The cache prevents unrelated systems from overwriting one another's contribution.

## Ownership

`src/modifiers/cache.rs` owns storage, recomputation, and snapshot representation. `ModifierCoordinationPlugin` in `src/modifiers/coordination.rs` owns adapters for sources that otherwise would write the cache directly:

- reactor allocation produces speed, yaw, phaser-damage, and shield-regeneration modifiers;
- active impulse produces its authored speed multiplier;
- region enter/exit observers add or remove radar, speed, and yaw effects.

Each adapter updates only its own source keys. Removing one effect returns that slot to the product of the remaining sources, not to a hardcoded baseline.

## Consumers

The main consumers are:

- ship physics (`MaxSpeed`, `MaxYawRate`);
- Tactical target acquisition (`RadarRange`), Helm radar (`HelmRadarRange`), and Sensors (`SensorRadarRange`);
- beam damage (`PhaserDamage`);
- shield recovery (`ShieldRegen`);
- collision damage (`HullDamageTaken`) in `src/server_app/collision.rs`;
- internal repair progress (`RepairRate`);
- region entry clamping (`MaxSpeed`) in `src/regions/server.rs`.

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
