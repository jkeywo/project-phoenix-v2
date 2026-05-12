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

## [2026-05-11] ingest | PRDs #115-120 + design drafts 9-11; refresh AGENTS/README/CONTEXT | touched: AGENTS.md, README.md, CONTEXT.md, sources/prd-115..120, sources/design-09..11, roadmap/overview, index.md

Bulk ingest pass for the new wave of planned work and to bring the root docs in
sync with the current codebase.

New source pages (six PRDs + three drafts):

- `sources/prd-115-native-pc-server.md` — native binary, cloudflared tunnel, WS transport
- `sources/prd-116-save-load-sessions.md` — `localStorage` save slots, `save.rs` as the
  *second* sanctioned `serde_json` surface
- `sources/prd-117-modifier-system.md` — pure `modifiers.rs` infrastructure
- `sources/prd-118-repair-and-power-consoles.md` — Engineering split (Repair + Power),
  shape-matching, 6+2 power
- `sources/prd-119-stations-scenarios-comms.md` — TOML scenario engine, station entities,
  `Console::Comms`
- `sources/prd-120-station-based-lobby.md` — per-station picking, cascade reassignment,
  spectator FIFO
- `sources/design-09-ai-and-behaviour.md` — NPC state machine
- `sources/design-10-region-entities.md` — invisible trigger volumes
- `sources/design-11-console-complexity.md` — Low/Full per console, shield frequency

Pages updated:

- `AGENTS.md` — added "Wiki — Read It and Maintain It" section pointing at SCHEMA.md
  with workflow at a glance; refreshed PRD list (shipped + open); added `phaser.rs`,
  `torpedo.rs`, `shield.rs`, `impulse.rs`, `entity_config.rs`, `map_config.rs`,
  `config_cache.rs`, `radar_config.rs`, `entity_tags.rs`, `asteroid_lifecycle.rs`,
  `beam_render.rs` to the file layout; flagged `src/server/`, `src/client/`,
  `src/shared/` as unwired draft refactor; added `assets/` tree; added Science
  console; added impulse + torpedo + phaser-mode messages to In-Progress flow.
- `README.md` — added Science to the consoles table; added Tactical/Helm details;
  pointed at `wiki/` and the open PRDs; refreshed `Project Structure` tree with the
  new modules.
- `CONTEXT.md` — added Science to Console; added planned-vocabulary entries (Station,
  Modifier, Power Allocation, Save Slot, Scenario, Shield Facing, Phaser Bank,
  Torpedo Tube, Impulse Drive); added Entity Config + Map Config terms.
- `wiki/index.md` — split PRD list into shipped + open; added the three new design
  drafts; reflagged drafts 1–5 as shipped/superseded as appropriate.
- `wiki/roadmap/overview.md` — full rewrite. Documents the wave of work that landed
  without labelled PRDs (phasers, torpedoes, shields, impulse, science, data-driven
  entities); summarises the six open PRDs with a suggested ordering;
  re-categorises the design drafts (1–5 mostly shipped, 6–8 absorbed by #119, 9–11
  candidates); calls out cross-PRD tensions (wire-break cadence, viewscreen
  authority, save fidelity vs scenario state, spectator persistence).

Pages not yet updated in this pass (may need follow-up if their facts shift further):
`concepts/architecture`, `concepts/testing-strategy`, `concepts/console-plugin-pattern`,
`concepts/view-model-pattern`, `entities/helm-console`, `entities/captain-console`,
`entities/console`, `entities/bridge-crew-stations-planned`, `roadmap/combat-and-damage`,
`roadmap/console-expansion`, `roadmap/data-driven-content`,
`roadmap/open-architectural-questions`.

Open questions surfaced:

- #116 save format vs #118 `Engineering` rename — a save written today would be
  invalid after #118 lands, since `Console::Engineering` no longer exists.
- #116 vs #119 — save fidelity does not yet cover scenario state, objectives, or
  active comms threads.
- #120 vs #116 — save format also doesn't restore the spectator FIFO.
- Captain `SetView` authority vs Science / Comms viewscreen pushes — needs an
  explicit override path. PRD #119 user story 13 calls this out for Comms.

## [2026-05-12] ingest | Issue #182 — viewscreen static border frame

Second slice of PRD #180. `ViewscreenBorderPlugin` (`src/viewscreen_border.rs`)
now loads all ten normal-state border PNGs and, on `GameStarted`, spawns a
viewport-filling root `Node` with ten `ImageNode` children: four 240×140 corners,
top cap (320×56), bottom cap (520×56), and four edges using
`NodeImageMode::Tiled`. The root carries a `ViewscreenBorderRoot` marker and is
despawned on transition back to `Lobby` (defensive — current code never returns).
No alert state, vignette, or HUD readouts yet — those land in #183 and #184.
No new tests: per the PRD, the plugin is Bevy plumbing with no testable surface
in this slice; pure helpers (`yaw_to_compass_bearing`, `pulse_intensity`) land
with #184.

## [2026-05-12] ingest | Issue #183 � red-alert visual: border swap + UiMaterial vignette + remove CSS overlay

Landed the final slice of PRD #180 inside iewscreen_border. Added ten alert-variant
image handles, a BorderSlot marker on every border ImageNode, and a per-frame
swap_border_textures system that flips each handle on ShipState.red_alert change.
Introduced RedAlertVignetteMaterial (single intensity uniform) registered via
UiMaterialPlugin, with a full-bleed MaterialNode spawned as the FIRST child of the
border root so border sprites occlude the outer ring (the demo's leak-from-behind
look). Replaced the placeholder WGSL with the real two-stop inset radial-gradient
shader. Added pure helper pulse_intensity(time, alert, prev, dt) -> f32 that combines
a quarter-second on/off ease with a 1.3s sine pulse between 0.55 and 1.0; driven each
frame by drive_vignette_intensity. 7 unit tests cover idle, ease in/out monotonicity,
steady-state band, and sine phase points; an 8th test confirms BorderSlot::handle`nswitches variants. Removed the #red-alert-overlay div, its CSS (ox-shadow,
adial-gradient, edalert-pulse keyframes), and the SimState red-alert handler
in server.html's outeOutbound � Bevy now owns the alert visual end-to-end.
Added pub fn ShipState::red_alert(&self) -> bool accessor so the plugin can read
the flag without exposing the field. All 659 lib tests pass; cargo check passes
for both native and wasm32 with --features server.

## [2026-05-12] ingest | Issue #184 — designation + HEADING/HULL/CONDITION HUD; PRD #180 wiki | touched: src/viewscreen_border.rs, wiki/sources/prd-180-viewscreen-frame.md, wiki/concepts/ui-materials.md, wiki/concepts/view-modes.md, wiki/index.md

Final slice of PRD #180. Added the static designation `"AEV-074 · PHOENIX"` centred on
the top cap (Chakra Petch) and a three-column status strip on the bottom cap with
`HEADING` / `HULL` / `CONDITION` labels (Chakra Petch, neutral `#b8c0c8`) above their
value cells (JetBrains Mono, signal-cyan / alert-red). Introduced pure helper
`yaw_to_compass_bearing(yaw_radians: f32) -> u32` (8 unit tests covering 0/90/180/270/360
cardinals, negative yaw, multi-turn yaw, and the 359.5° rounding boundary that wraps to 0).
Added `update_hud` per-frame system that formats heading as `{:03}`, hull as integer
percent (read from `ShipHullIntegrity`), and condition as `NOMINAL`/`ALERT`, then toggles
`TextColor` on the designation and value cells based on `ShipState.red_alert`. Labels never
swap colour. Loaded the JetBrains Mono font handle alongside Chakra Petch in
`ViewscreenAssets`. All 667 lib tests pass; `cargo check --features server` passes for
native and wasm32.

Wiki: created `wiki/sources/prd-180-viewscreen-frame.md` summarising the parent PRD across
its four shipped child slices (#181/#182/#183/#184); created `wiki/concepts/ui-materials.md`
documenting the `UiMaterial` + WGSL shader pattern with `RedAlertVignetteMaterial` as the
worked example and PRD #119 station chrome / comms / future damage indicators flagged as
follow-on candidates; updated `wiki/concepts/view-modes.md` to mention the new viewscreen
chrome around the camera output; added both new pages to `wiki/index.md`.
