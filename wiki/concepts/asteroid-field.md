---
title: Asteroid Field
type: concept
tags: [asteroid, world, deterministic, density, streaming]
sources: [src/asteroids/spawner.rs, src/asteroids/window.rs, src/asteroids/lifecycle.rs, src/asteroids/mod.rs, src/entities/config.rs]
updated: 2026-08-27
---

# Asteroid Field

Asteroid fields are authored density contributions evaluated over one deterministic world lattice. The runtime streams a window of cells around the player rather than generating one fixed session-wide list.

`src/asteroids/spawner.rs` is the pure generator. Each field contributes authored shape, radii, anchor offset, grid/noise parameters, weighted gameplay/cosmetic types, and collision/render tuning. Overlapping fields blend into the composed evaluator so one cell is populated once rather than once per overlapping author block.

`src/asteroids/window.rs` tracks the active cell window. `src/asteroids/lifecycle.rs` spawns/despawns the authoritative gameplay asteroids and cosmetic layers as cells enter or leave it.

Determinism is cell-local: density/noise and each spawn are seeded from the authored layer salt plus lattice coordinates. Re-entering a cell recreates its baseline population, so destroyed asteroids respawn fresh after the player leaves and returns. Identical world content and logical movement produce the same layout.

## Related

- [Asteroid](../entities/asteroid.md)
- [World Data](../entities/world-data.md)
- [WorldPlugin](./world-plugin.md)
