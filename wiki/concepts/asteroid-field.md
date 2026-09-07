---
title: Asteroid Field
type: concept
tags: [asteroid, world, deterministic, density, streaming]
sources: [src/asteroids/spawner.rs, src/asteroids/window.rs, src/asteroids/lifecycle.rs, src/asteroids/mod.rs, src/entities/config.rs, src/entities/config_cache.rs, tests/asteroid_streaming.rs]
updated: 2026-09-07
---

# Asteroid Field

Asteroid fields are authored density contributions evaluated over one deterministic world lattice. The runtime streams a window of cells around the fleet's mean ship position rather than generating one fixed session-wide list. For one ship, that is its position.

`src/asteroids/spawner.rs` is the pure generator. Each field contributes authored shape, radii, anchor offset, grid/noise parameters, weighted gameplay/cosmetic types, and collision/render tuning. Overlapping fields blend into the composed evaluator so one cell is populated once rather than once per overlapping author block.

`src/asteroids/window.rs` tracks the active cell window. `src/asteroids/lifecycle.rs` spawns/despawns the authoritative gameplay asteroids and cosmetic layers as cells enter or leave it.

Both spawn paths use `config_cache::get_cached_entity_config` for the selected template, avoiding a clone of the whole template cache per rock. Each spawn still reads the current entry; missing or replaced templates do not become stale cached rock configurations. `tests/asteroid_streaming.rs` exercises delivery and replacement through the real fixed-step lifecycle, including both cosmetic layers and stable cell identities/order after re-entry.

Determinism is cell-local: density/noise and each spawn are seeded from the authored layer salt plus lattice coordinates. Re-entering a cell recreates its baseline population, so destroyed asteroids respawn fresh after the player leaves and returns. Identical world content and logical movement produce the same layout.

## Related

- [Asteroid](../entities/asteroid.md)
- [World Data](../entities/world-data.md)
- [WorldPlugin](./world-plugin.md)
