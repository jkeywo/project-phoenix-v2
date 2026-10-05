---
title: Ship
type: entity
tags: [ship, physics, collision, viewscreen]
sources: [crates/phoenix-simulation/src/entities/ship_spawn.rs, crates/phoenix-simulation/src/ship/state.rs, crates/phoenix-simulation/src/ship/physics.rs, crates/phoenix-simulation/src/ship/components.rs, crates/phoenix-simulation/src/ship/physics_systems.rs, crates/phoenix-simulation/src/entities/spawner.rs, crates/phoenix-presentation/src/server/renderer.rs, assets/entities/dynasty_player_cruiser.toml]
updated: 2026-10-04
---

# Ship

A ship is an ECS entity assembled from authored entity configuration. It carries authoritative components for physics, systems, damage, power, shields, sensors, Red Alert, and viewscreen state as its configuration requires.

`ShipPhysics` stores the simulation position, yaw, velocity, and lateral velocity. `ship_plugin` applies admitted or AI helm input through the pure `ship::physics` calculation and synchronises the resulting position to the render transform. Clients receive published snapshots and never simulate a ship locally.

The host's local hull has a presentation-only `RenderInterp` pose pair. `FixedLast` captures consecutive committed poses, cinematic rendering blends between them at frame rate, and `FixedFirst` restores the exact committed transform before any simulation system runs. The interpolated transform is therefore visual state only and never becomes authoritative input.

Ship capabilities are authored rather than inferred from a special ship category. A craft without a system simply lacks that capability; station access is derived from its configured stations and systems.

Shields require authored shield base or arcs, repair teams require a positive
count, and weapons require their own configuration. These rules also apply to
the local hull. Policy-only declarations grant no equipment; schema defaults
inside declared blocks and mandatory physics/reactor state remain shared.

The crewed Dynasty cruiser is a separate composed hull from the Harrow NPC
cruiser. It reuses the ordinary player-cruiser system implementations while
authoring a six-Station Dynasty roster and presentation; existing NPC
encounters continue to resolve `ship_harrow_cruiser.toml` unchanged.
