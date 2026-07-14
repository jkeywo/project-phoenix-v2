---
title: Ship
type: entity
tags: [ship, physics, collision, viewscreen]
sources: [src/ship/state.rs, src/ship/physics.rs, src/ship_plugin.rs, src/entities/spawner.rs]
updated: 2026-07-14
---

# Ship

A ship is an ECS entity assembled from authored entity configuration. It carries authoritative components for physics, systems, damage, power, shields, sensors, Red Alert, and viewscreen state as its configuration requires.

`ShipPhysics` stores the simulation position, yaw, velocity, and lateral velocity. `ship_plugin` applies admitted or AI helm input through the pure `ship::physics` calculation and synchronises the resulting position to the render transform. Clients receive published snapshots and never simulate a ship locally.

Ship capabilities are authored rather than inferred from a special ship category. A craft without a system simply lacks that capability; station access is derived from its configured stations and systems.
