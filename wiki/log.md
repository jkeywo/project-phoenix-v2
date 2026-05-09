# Wiki Log

Append-only chronological record. Most recent entries at the bottom. Format:

```
## [YYYY-MM-DD] <ingest|query|lint|seed> | <one-line title> | <details>
```

`grep "^## \[" log.md | tail` to see recent activity.

---

## [2026-05-08] seed | Bootstrap wiki from existing project state

Initial wiki creation. Inspired by Karpathy's LLM-Wiki gist
(https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f).

Sources ingested in this pass:

- `README.md`, `AGENTS.md`, `CONTEXT.md` (repo root)
- All Rust modules under `src/` (server, client, shared)
- All draft design docs under `docs/` (1–8 + Architecture Improvement Notes)
- All issues labeled `PRD` on GitHub: #1, #17, #22, #36, #51, #66

Pages created:

- `SCHEMA.md`, `index.md`, `log.md`
- `entities/`: player, session, console, captain-console, helm-console, ship,
  asteroid, world-data, bridge-crew-stations-planned
- `concepts/`: project-overview, architecture, networking, message-flow,
  codec-seam, game-phases, game-loop, ship-physics, asteroid-field,
  radar-projection, view-modes, view-model-pattern, console-plugin-pattern,
  build-and-deployment, testing-strategy
- `sources/`: prd-001, prd-017, prd-022, prd-036, prd-051, prd-066,
  design-01..design-08, notes-architecture-improvements,
  repo-readme, repo-agents, repo-context
- `roadmap/`: overview, console-expansion, combat-and-damage,
  data-driven-content, open-architectural-questions

Open questions captured:

- Drafts 6, 7, 8 are stubs in the source — wiki entries flag this.
- "Architecture Improvement Notes" describes per-console message subscription
  but no PRD has been written yet — captured under
  `roadmap/open-architectural-questions.md`.

## [2026-05-08] ingest | Roadmap pages written | touched: roadmap/overview, roadmap/console-expansion, roadmap/combat-and-damage, roadmap/data-driven-content, roadmap/open-architectural-questions

Synthesised the five `roadmap/` pages from existing source pages (PRDs #1, #17,
#22, #36, #51, #66 and design drafts 1–8 + Architecture Improvement Notes).
No new sources ingested; this is a synthesis pass.

Cross-cutting tensions surfaced:

- PRD #66's single hull pool vs Draft 4's four-quadrant shields (rewrite path).
- Captain-only `SetView` vs Draft 3's Science-driven viewscreen content.
- One-shot `WorldData` vs Draft 2's streamed nearby asteroids.
- Hardcoded tunables vs Draft 5's power-modulated multipliers.

Six open architectural questions catalogued; each blocks at least one drafted
feature.

## [2026-05-09] ingest | PRD #66 shipped; client is now Bevy/WASM; 4 consoles | touched: AGENTS.md, README.md, CONTEXT.md, sources/prd-066, sources/repo-agents, sources/repo-context, entities/console, roadmap/overview, index.md

Major update pass reflecting the current state of the codebase, which has diverged
significantly from the 2026-05-08 bootstrap.

Key changes discovered and documented:

1. **Client is now a full Bevy/WASM app.** `client.html` is no longer plain HTML/JS.
   New client-side Rust modules: `client_lobby.rs`, `client_sim.rs`, `client_helm.rs`,
   `client_app.rs`, `client_bridge.rs`. All are compiled via the `client` Cargo feature.

2. **4 consoles shipped.** `Tactical` (phasers, target lock) and `Engineering` (repair loop)
   are now live. `Console` enum has 4 variants. `client_lobby.rs` defines `ALL_CONSOLES [4]`.

3. **PRD #66 fully implemented.** Moved from "In flight" to "Shipped" in roadmap.
   - `damage.rs`: `collision_damage()` formula + `HullIntegrity` struct
   - `breakdown.rs`: `BreakdownQueue` FIFO + `breakdowns_from_damage()` bucket formula
   - New messages: `SetTarget`, `FirePhaser`, `Repair`, `TargetLock`, `WeaponsUpdate`,
     `BeamStarted`, `BeamEnded`, `AsteroidDestroyed`, `RepairState`
   - `SimSnapshot` gained `hull_integrity` and `authorized_repair_console` fields

4. **Cargo features** (`server` / `client`) split the two builds. Previously only a
   single WASM target existed; now each HTML page has its own feature gate.

5. **`lobby_handler.rs` extracted.** Pure handler functions (`process_message`,
   `process_disconnect`) separated from the Bevy plugin in `lobby.rs` for testability.

6. **`radar.rs` expanded.** Added `is_fire_ready()` (range + arc gate for phasers)
   and `WEAPONS_RADAR_RANGE` constant.

7. **Multi-console tab support.** `ActiveConsole` Bevy resource + `wasm_client_set_active_console`
   bridge call allow the JS tab bar to switch which panel is shown when a player holds
   multiple consoles.

Pages updated: AGENTS.md, README.md, CONTEXT.md, `sources/prd-066`,
`sources/repo-agents`, `sources/repo-context`, `entities/console`, `roadmap/overview`,
`index.md`.

Pages not yet updated (may have stale file paths or line numbers):
`concepts/architecture`, `concepts/testing-strategy`, `concepts/console-plugin-pattern`,
`concepts/view-model-pattern`, `entities/helm-console`, `entities/captain-console`,
`roadmap/combat-and-damage`, `roadmap/console-expansion`.
