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

## [2026-05-13] ingest | Doc-sync pass: PRDs #115/#117/#118/#120/#153/#154/#187/#191 shipped; six-console roster | touched: AGENTS.md, README.md, CONTEXT.md, wiki/index.md, wiki/entities/console.md, wiki/sources/prd-115/117/118/120/142/153/154/187/191, wiki/log.md

Audit + sync pass against the live codebase and gh issue tracker. The wiki had drifted: it still listed four consoles including `Engineering`, treated PRDs #115/#117/#118/#120 as open, and had no source pages for the wave of PRDs that closed since 2026-05-12 (#153, #154, #187, #191).

Confirmed via `gh issue view`:

- #115 closed 2026-05-11 (PRD itself; deployment slices #135–#141 remain on hold — no implementation has shipped)
- #117 closed 2026-05-11 (modifier system, `src/modifiers.rs`)
- #118 closed 2026-05-12 (`Engineering` → `Repair`; `Power` added; `repair_teams.rs` + `power_system.rs`)
- #120 closed 2026-05-11 (station-based lobby, `stations.rs`)
- #153 closed (region entities, `f32` hull, `FlagKind`, unified `EntitySnapshot` wire)
- #154 closed (per-console Low/Full complexity + `console_ai`)
- #187 closed (phone bezel, `phone_border/`)
- #191 closed (grid-based asteroid lifecycle, `asteroid_window.rs`)

Console enum (`src/messages.rs:142`) is now six variants: `CaptainChair, Helm, Tactical, Repair, Science, Power`. `Engineering` is gone from code, wire, and tests.

Pages updated:

- Root docs (`README.md`, `AGENTS.md`, `CONTEXT.md`) brought in line with the six-console / station-picking / `f32`-hull / shape-repair / 6+2-power / region-effect / streaming-asteroids / phone-bezel reality. PRD lists split into Shipped vs Open. Module Map test bullets and file layout refreshed for the new modules (`flag_kind`, `modifiers`, `power_system`, `repair_teams`, `stations`, `asteroid_window`, `viewscreen_border`, `phone_border/`).
- `wiki/index.md` PRD lists rewritten: shipped includes #66/#115/#117/#118/#120/#153/#154/#180/#187/#191; open list reduced to #116/#119/#142.
- `wiki/entities/console.md` rewritten for six consoles + station model + complexity invariant.
- `wiki/sources/prd-115/#117/#118/#120` status front-matter flipped to `shipped` (or `shipped-prd-on-hold-slices` for #115); status sections rewritten to reflect actual landed code.

Pages created:

- `wiki/sources/prd-142-ai-and-behaviour.md` — open. Depends on #119; shares action vocabulary with #154's `console_ai`.
- `wiki/sources/prd-153-region-entities.md` — shipped. Unified `[[entity]]` pipeline, six region effects, `f32` hull, `FlagKind`, `EntitySnapshot` wire.
- `wiki/sources/prd-154-console-complexity.md` — shipped. `Low`/`Full` per-console presets, `console_ai` operates hidden controls.
- `wiki/sources/prd-187-phone-console-hud.md` — shipped. `phone_border/` plugin; full helm + captain chrome.
- `wiki/sources/prd-191-grid-based-asteroid-lifecycle.md` — shipped. Player-centred ring buffer; deterministic per-cell respawn-on-return.

Open PRDs now: #116 (Save/Load), #119 (Stations + Scenarios + Comms), #142 (AI). #116 and #119 are blockers/shaping-influences for #142.

Pages still not yet updated in this pass (deferred — facts mostly correct but file paths / line numbers may drift): `concepts/architecture`, `concepts/testing-strategy`, `concepts/console-plugin-pattern`, `concepts/view-model-pattern`, `entities/helm-console`, `entities/captain-console`, `entities/bridge-crew-stations-planned` (now superseded by `stations.rs` — candidate for retirement), `roadmap/combat-and-damage`, `roadmap/console-expansion`, `roadmap/data-driven-content`, `roadmap/open-architectural-questions`, `roadmap/overview` (the open-PRD list inside it still mentions #115/#117/#118/#120 as planned).

## [2026-05-14] ingest | Issue #176 — delegation allowlist + Science phaser-frequency control | delegation.rs + SetPhaserFrequency

Implemented issue #176 via TDD (12 new tests added, all passing). 1087 total tests.

New module:
- `src/delegation.rs` — pure (no Bevy) allowlist. `DelegatedControl` enum (currently `SetPhaserFrequency` only), `ComplexityContext` struct (carrying `tactical_is_low: bool`), and `is_sender_authorized(control, sender, ctx)` function. Allowlist table: Tactical may always set phaser frequency; Science may set it only when Tactical is Low.

Changes:
- `src/messages.rs` — new `ClientMessage::SetPhaserFrequency { frequency: f32 }` variant.
- `src/ship_state.rs` — new `phaser_frequency: f32` field (default 0.5) on `ShipState`.
- `src/simulation.rs` — new `handle_set_phaser_frequency` system. Consults `ConsoleComplexityState` + `delegation::is_sender_authorized`; clamps value to `[0.0, 1.0]`. Registered in both `SimulationPlugin` and the test app.
- `src/lobby_handler.rs` — `SetPhaserFrequency` added to the pass-through arm of the inbound match (lobby ignores it).
- `src/client_sim.rs` — `phaser_frequency: f32` field on `ClientSimState`; `is_science_phaser_panel_visible(complexity)` pure helper; `set_phaser_frequency_message(frequency)` message builder.
- `src/client_elements.rs` — no new TOML asset needed; Science panel visibility is driven by Tactical's complexity state, not a hidden-elements list.
- `src/codec.rs` — three round-trip tests for `SetPhaserFrequency`.
- `assets/complexity/science.toml` — created (two empty presets) for future use.
- `src/lib.rs` — registered `pub mod delegation`.

## [2026-05-14] ingest | Issue #177 — Science Low hides frequency readout + auto-hint AI | console_ai, console_ai_plugin, client_sim, client_elements, messages, codec, complexity

Implemented issue #177 via TDD (12 new unit tests + 9 plugin integration tests, 1111 total).

Changes:
- `assets/complexity/science.toml` — Science Low now hides `shield_frequency_readout`; added `[preset.ai] auto_hint = { auto_hint_delay_secs = 3.0 }`.
- `src/messages.rs` — new `ServerMessage::FrequencyHint { frequency: f32 }` variant. Sent to the Tactical holder by the Science Low AI after a configurable delay.
- `src/console_ai.rs` — new `tick_frequency_hint(state, input) -> FrequencyHintOutput` pure decision function + `FrequencyHintState`, `FrequencyHintInput`, `FrequencyHintOutput` types. Timer resets on target change or when target is cleared.
- `src/console_ai_plugin.rs` — new `run_science_hint_ai` Bevy system; `FrequencyHintTimer` and `AutoHintDelaySecs` resources; `load_auto_hint_delay_secs()` reads from embedded TOML. Hint fires only when Tactical=Full + Science=Low + Tactical occupied + target locked.
- `src/client_elements.rs` — `hideable_element_names` now handles `Console::Science` (reads `science.toml`). New tests for Science Low/Full.
- `src/client_complexity.rs` — `ComplexityStore::for_console` now creates ["Low","Full"] presets for both Tactical and Science.
- `src/client_sim.rs` — `frequency_hint: Option<f32>` field added to `ClientSimState`; `apply` handles `FrequencyHint`; reset on `Welcome`.
- `src/codec.rs` — `server_frequency_hint_round_trips` + boundary test.
- `src/client_lobby.rs` — `FrequencyHint` added to the pass-through arm.

## [2026-05-14] ingest | Issue #175 — auto-fire torpedo AI | console_ai + console_ai_plugin

Implemented issue #175 via TDD (21 new tests, all passing).

New modules:
- `src/console_ai.rs` — pure (no Bevy) `auto_fire_torpedo` decision function. `TorpedoAiInput` / `TubeSummary` types. Conditions: target locked + shields ≤ 0 + tube loaded + tube in arc + magazine > 0. Returns tubes in deterministic priority order [ForePort, ForeStarboard, Aft].
- `src/console_ai_plugin.rs` — Bevy orchestrator. `ConsoleComplexityState` resource tracks current preset per console (updated from outbound `ComplexityChanged` messages). `run_tactical_ai` synthesises `FireTorpedo` `InboundMessage` each tick when Tactical is occupied and at Low complexity. Continuous re-fire on reload: the system re-evaluates every frame so any newly-loaded tube fires immediately. Switching to Full stops AI immediately (checked every tick).
- `ConsoleAiPlugin` wired into `SimulationPlugin`.
- Pre-existing compile error in `src/client_sim.rs` (missing `RadarStateSnapshot` import in tests) fixed as a prerequisite.

## [2026-05-14] ingest | Issue #179 — auto-match frequency AI (both Tactical + Science Low or Science unmanned) | console_ai, console_ai_plugin, tactical.toml

Implemented issue #179 via TDD (10 pure unit tests + 9 plugin integration tests, 1156 total).

Changes:
- `assets/complexity/tactical.toml` — added `auto_match_delay_secs = 3.0` to `[preset.ai] frequency_match`.
- `src/console_ai.rs` — new `tick_auto_match_frequency(state, input) -> FrequencyMatchOutput` pure decision function + `FrequencyMatchState`, `FrequencyMatchInput`, `FrequencyMatchOutput` types. Trigger activates when both Tactical and Science are Low, or Science is unmanned. Timer resets on target change or when trigger deactivates. Frequency persists on trigger end — no auto-revert.
- `src/console_ai_plugin.rs` — new `run_auto_match_ai` Bevy system; `FrequencyMatchTimer` and `AutoMatchDelaySecs` resources; `load_auto_match_delay_secs()` reads from embedded `tactical.toml`. Synthesises `SetPhaserFrequency` as an `InboundMessage` from the Tactical holder token after delay elapses. Either console flipping to Full cancels the pending countdown. Registered in `ConsoleAiPlugin` alongside existing AI systems.

## [2026-05-14] lint | Plan-shift: AI is no longer a stub; husk list mostly obsolete; folder reorg planned | touched: wiki/concepts/architecture.md, wiki/concepts/console-plugin-pattern.md, wiki/sources/prd-142-ai-and-behaviour.md

During an architectural-improvement grilling session, the running plan called for deleting `ai.rs` + `ai_plugin.rs` as `husks awaiting PRD #142`. A check against the codebase confirmed the user's correction: those files now contain ~1650 lines of working code (issues #175/#176/#177/#179 landed during this session). The husk list collapses to just `comms_plugin.rs` (24 lines), which will fold naturally into a future `CommsConsolePlugin` once the per-console split lands. `delegation.rs` (142 LoC, well-tested pure allowlist) and `region_effects.rs` (217 LoC, pure TOML-config schema) were also flagged for inlining, but inspection shows both are clean focused modules whose deletion would *worsen* locality by folding them into 40+KB neighbours.

Wiki updates:

- `wiki/concepts/architecture.md` � replaced the entire `src/server/` / `src/client/` / `src/shared/` module map (none of those folders exist) with the actual flat ~56-module layout, grouped by naming convention. Noted the planned domain-grouped folder reorg.
- `wiki/concepts/console-plugin-pattern.md` � flipped from `current plugins: CaptainConsolePlugin, HelmConsolePlugin` (neither file exists) to `partially realised`: documents the current god-module reality (`client_app.rs` ~2329 LoC, `client_sim.rs` ~2136 LoC) and the planned per-console split. Added locality-of-behaviour rationale.
- `wiki/sources/prd-142-ai-and-behaviour.md` � status flipped from `open` to `in-flight`. Documented the landed pieces (`ai.rs`, `ai_plugin.rs`, `faction.rs`, console-AI siblings) and the still-open work (TOML state-machine schema, squad behaviours, scenario integration).

Filed issue #218 � `Architecture: Merge Scenario into World` � capturing the rationale for the planned merger of `scenario_plugin.rs` into a unified `WorldPlugin` at `src/world/server.rs`. To be executed as part of the upcoming reorg.

Not yet touched (deferred to the actual reorg PRs): `wiki/roadmap/open-architectural-questions.md` (will be amended once the reorg + 6 deepenings start landing); per-PRD source pages for the AI sub-issues (#175/#176/#177/#179).

## [2026-05-15] ingest | World merger #1: WorldPlugin skeleton | Created src/world/mod.rs + src/world/server.rs; moved setup_world_hardcoded from simulation.rs into WorldPlugin; registered WorldPlugin in bridge.rs. Added wiki/concepts/world-plugin.md. Closes #219.

## [2026-05-15] ingest | World merger #2: scenario types moved to world/content.rs | Moved all pure scenario types/functions/tests from src/scenario.rs into src/world/content.rs. Deleted src/scenario.rs, removed pub mod scenario from lib.rs, updated imports in config_cache.rs and scenario_plugin.rs. Fixed include_str! paths. Closes #220.

## [2026-05-15] ingest | World merger #3: scenario_plugin.rs folded into WorldPlugin | Moved all resources (ScenarioRuntime, CommsInboxRes, ObjectiveManagerRes) and systems from src/scenario_plugin.rs into src/world/server.rs under WorldPlugin. Deleted scenario_plugin.rs, removed pub mod from lib.rs, removed ScenarioPlugin registration from bridge.rs. Closes #221.

## [2026-05-15] ingest | World merger #4: wire default_scenario from map config | Added choose_bootstrap() to WorldPlugin with three-tier precedence (scenario → map-config → hardcoded). Added unit tests for fallback and parse paths. Added smoke spec tests/smoke/world-bootstrap.spec.ts. Documented bootstrap precedence in wiki. Closes #222.

## [2026-05-15] ingest | Architecture: document target domain-grouped tree | Rewrote wiki/concepts/architecture.md: removed stale flat-layout description, added target src/ tree (core/, lobby/, ship/, weapons/, ..., world/), documented design rules and current transitional state. Closes #223.

## [2026-05-15] ingest | Folder reorg #1: Create src/ship/ and move ship pure modules | Moved ship_state.rs → ship/state.rs, ship_physics.rs → ship/physics.rs, impulse.rs → ship/impulse.rs, damage.rs → ship/damage.rs. Added src/ship/mod.rs. Added pub use re-exports in lib.rs so existing crate::ship_state:: paths compile unchanged. Closes #229.

## [2026-05-15] ingest | Folder reorg #2: Create src/weapons/ and move phaser/torpedo/shield/beam_render | Moved phaser.rs → weapons/phaser.rs, torpedo.rs → weapons/torpedo.rs, shield.rs → weapons/shield.rs, beam_render.rs → weapons/beam_render.rs. Added src/weapons/mod.rs. Added pub use re-exports in lib.rs. Closes #235.

## [2026-05-15] ingest | Folder reorg #3: Create src/asteroids/ and move spawner/window/lifecycle | Moved asteroid_spawner.rs → asteroids/spawner.rs, asteroid_window.rs → asteroids/window.rs, asteroid_lifecycle.rs → asteroids/lifecycle.rs. Added src/asteroids/mod.rs. Added pub use re-exports in lib.rs. Closes #241.

## [2026-05-15] ingest | Folder reorg #4: Create src/regions/ and src/entities/ | Moved region_effects.rs → regions/effects.rs, region_shape.rs → regions/shape.rs, region_plugin.rs → regions/server.rs. Moved entity_config.rs → entities/config.rs, entity_tags.rs → entities/tags.rs, entity_loader.rs → entities/loader.rs, entity_override.rs → entities/entity_override.rs, entity_spawner.rs → entities/spawner.rs, map_config.rs → entities/map_config.rs, config_cache.rs → entities/config_cache.rs. Fixed include_str! paths. Added pub use re-exports in lib.rs. Closes #247.

## [2026-05-15] ingest | Folder reorg #5: Create src/modifiers/ and src/core/ | Moved modifiers.rs → modifiers/cache.rs, power_system.rs → modifiers/power_system.rs, repair_teams.rs → modifiers/repair_teams.rs, breakdown.rs → modifiers/breakdown.rs, modifier_coordination.rs → modifiers/coordination.rs. Moved messages.rs → core/messages.rs, codec.rs → core/codec.rs, flag_kind.rs → core/flag_kind.rs. Added re-exports in mod.rs files and lib.rs. Closes #252.

## [2026-05-15] ingest | Stations split #1: StationsConfig in src/lobby/stations_config.rs | stations_config (parse + lookup) and stations_policy (assignment) live in src/lobby/; src/stations.rs is a thin re-export shim; wiki/concepts/stations.md created. Closes #230.

## [2026-05-15] ingest | Stations split #3: Delete src/stations.rs (final cleanup) | Deleted src/stations.rs shim and removed pub mod stations from lib.rs. Updated all 9 call sites (bridge.rs, server/bridge.rs, viewscreen_border.rs, server/viewscreen_border.rs, lobby/client_panel.rs, lobby/server.rs, client_sim.rs, core/codec.rs, core/messages.rs) to import directly from crate::stations_config or crate::stations_policy. wiki/concepts/stations.md updated to reflect the two-module split with no mention of the old combined file. Closes #242.

## [2026-05-15] ingest | Broadcaster #1: SimBroadcaster + LobbyBroadcaster + PowerState tracer | Expanded src/core/broadcast.rs into src/core/broadcast/{mod,audience,cadence,sim,lobby}.rs. Introduced SimBroadcaster and LobbyBroadcaster plugins with register(audience, cadence, producer) API. Migrated broadcast_power_state from simulation.rs into SimBroadcaster via power_state_broadcaster(). Registered LobbyBroadcaster in src/bridge.rs + src/server/bridge.rs. Created wiki/concepts/broadcaster-seam.md. Closes #231.

## [2026-05-15] ingest | Broadcaster #4: Migrate lobby outputs to LobbyBroadcaster | Updated LobbyProducer to match Producer signature (Fn(&mut World) -> Vec<ServerMessage>), added LobbyOutbox resource, wired process_lobby/handle_disconnect to push to outbox instead of direct OutboundMessage writes, added lobby_outbox_broadcaster() helper, registered in bridge, removed apply_result direct-write path. LobbyBroadcaster now supports both Cadence::Once and Cadence::OnEvent. wiki/concepts/broadcaster-seam.md updated. Closes #248.

## [2026-05-15] ingest | Broadcaster #6: Wiki — document the broadcaster seam | Expanded wiki/concepts/broadcaster-seam.md: Audience/Cadence semantic notes, producer-registration recipe (power_state_broadcaster as worked example), complete catalogue with file:line for all 6 SimBroadcaster + 1 LobbyBroadcaster producers, SimOutbox/LobbyOutbox forwarding patterns, OutboundMessage write-contract verification, non-migrated system list, cross-links to PRDs #117/#118/#120/#153/#154/#180/#187. Updated wiki/index.md and wiki/concepts/architecture.md. Closes #257.

## [2026-05-15] ingest | Modifier coordination #1: coordination plugin + power source tracer | Created `ModifierCoordinationPlugin` as sole `init_resource` of `ShipModifiers`. Added `translate_power_modifiers` system in coordination.rs; registered in `SimulationPlugin` after `handle_power_messages`/`tick_power_system`. Removed `sync_power_modifiers` and direct `ShipModifiers`/`PowerMultiplierResource` access from simulation's `handle_power_messages` and `tick_power_system`. Created `wiki/concepts/modifier-coordination.md`. Closes #232.

## [2026-05-15] ingest | Modifier coordination #2: regions modifier source | Added `apply_region_effects` pure helper and `translate_region_modifiers` system to `src/modifiers/coordination.rs`. Removed 7 modifier-writing handlers from `src/regions/server.rs` (`handle_comms_jam_enter`, `handle_sensor_blind_enter`, `handle_radar_dampening_enter`, `handle_slow_zone_enter`, `handle_radar_dampening_exit`, `handle_flag_region_exit`, `handle_slow_zone_exit`). Added `handle_slow_zone_speed_clamp` (reads `Res<ShipModifiers>`, no mutation). Made `update_region_membership` pub(crate). Registered `translate_region_modifiers` + speed clamp chained in `SimulationPlugin`. 12 new unit tests for `apply_region_effects`. Updated test helpers to include coordinator + translator. All 1566 tests pass. Updated `wiki/concepts/modifier-coordination.md`. Closes #238.

## [2026-05-15] ingest | Modifier coordination #3: impulse modifier source | Added `apply_impulse_to` pure helper and `translate_impulse_modifiers` system to `src/modifiers/coordination.rs`. Translator change-detects `ImpulsePhase` via `Local<Option<ImpulsePhase>>` to avoid redundant events. Registers `ModifierSlot::MaxSpeed` with bonus `IMPULSE_SPEED_MULTIPLIER - 1.0` under `ModifierSource::ImpulseDrive` when impulse is active; removes it when idle or charging. Registered in `SimulationPlugin` and `test_app` after `handle_impulse_messages`. 6 new unit tests for `apply_impulse_to`. All 1572 tests pass. `cargo check --features server` passes. `ResMut<ShipModifiers>` appears only in coordination module. Updated `wiki/concepts/modifier-coordination.md`. Closes #244.

## [2026-05-15] ingest | Modifier coordination #4: wiki documentation | Expanded `wiki/concepts/modifier-coordination.md` to full page: coordinator role (sole `init_resource`), complete catalogue of three sources with file:line references, read-interface guide for consumers, recipe for adding a new source, `RegionEffect { uuid }` source-identity design preventing stale accumulation, cross-links to PRDs #117/#118/#153. Updated `wiki/index.md` description, referenced coordinator in `wiki/concepts/architecture.md`. Closes #249.

## [2026-05-15] ingest | Simulation split #1: CaptainPlugin | Extracted `CaptainPlugin` from `simulation.rs`. Moved `handle_toggle_red_alert`, `handle_set_view`, and 7 unit tests to `src/captain_plugin.rs`. Removed 3 captain-specific view tests from `simulation.rs`. Created `wiki/concepts/captain-plugin.md`. Closes #233.

## [2026-05-15] ingest | Simulation split #2: ShipPlugin | Extracted `ShipPlugin` from `simulation.rs`. Moved `process_helm_inputs`, `sync_ship_position`, `handle_impulse_messages`, `is_inside_blocks_impulse`, resources `LastHelmInput`/`HelmInputTimer`, and 7 impulse unit tests to `src/ship_plugin.rs`. Removed moved code from `simulation.rs`; `handle_collisions` stayed in simulation due to Rapier coupling. Registered `ShipPlugin` in `SimulationPlugin` + `lib.rs`. Updated `console_ai` import path. Created `wiki/concepts/ship-plugin.md`. Closes #239.

## [2026-05-16] ingest | Simulation split #3: WeaponsPlugin | Completed extraction of `WeaponsPlugin` from `simulation.rs`. `src/weapons_plugin.rs` already held the moved systems; fixed remaining compilation blockers: removed duplicate `AsteroidDestroyedVfx` struct and `impl Default for PhaserRenderConfig` from `simulation.rs`, added `add_message::<AsteroidDestroyedVfx>()` to `WeaponsPlugin::build`, made `BEAM_DAMAGE_PER_SEC` `pub`, and updated the simulation `test_app()` to use `WeaponsPlugin` instead of manually registering weapons functions. All 1602 tests pass; `cargo check --features server` clean. Created `wiki/concepts/weapons-plugin.md`. Closes #245.

## [2026-05-16] ingest | Simulation split #4: RepairPlugin | Extracted `RepairPlugin` from `simulation.rs`. Moved `handle_repair`, `tick_repair_teams`, `broadcast_repair_icons`, `repair_state_broadcaster()`, and resources `ShipRepairTeams`/`BreakdownQueueResource`/`RepairIconState`/`REPAIR_TEAM_HP` to `src/repair_plugin.rs`. Moved 14 repair and repair-icon tests into `repair_plugin.rs`. Re-exported types from `simulation.rs` for backwards compat. Registered `RepairPlugin` in `SimulationPlugin` and simulation `test_app()`. Added `pub mod repair_plugin;` to `src/lib.rs`. All 1616 tests pass; `cargo check --features server` clean. Created `wiki/concepts/repair-plugin.md`. Closes #250.

## [2026-05-16] ingest | Simulation split #5: PowerPlugin | Extracted `PowerPlugin` from `simulation.rs`. Moved `handle_power_messages`, `tick_power_system`, `power_state_broadcaster()`, and resources `ShipPowerSystem`/`PowerConfigResource`/`PowerMultiplierResource` to `src/power_plugin.rs`. Moved 12 power tests into `power_plugin.rs`. Power side does not mutate `ShipModifiers` directly — modifier writes stay in `translate_power_modifiers` (coordination.rs), which now imports from `power_plugin` instead of `simulation`. Re-exported types from `simulation.rs` for backwards compat. Registered `PowerPlugin` in `SimulationPlugin` and simulation `test_app()`. Added `pub mod power_plugin;` to `src/lib.rs`. All 1628 tests pass; `cargo check --features server` clean. Created `wiki/concepts/power-plugin.md`. Closes #254.

## [2026-05-16] ingest | Simulation split #6: SciencePlugin | Extracted SciencePlugin from simulation.rs. Moved handle_set_science_target and 3 SetScienceTarget tests to src/science_plugin.rs. Removed science handling from SimulationPlugin direct systems; SciencePlugin registered as sub-plugin. Added pub mod science_plugin to lib.rs. Created wiki/concepts/science-plugin.md. All 1629 tests pass; cargo check --features server clean. Closes #258.

## [2026-05-16] ingest | Simulation split #7 (final): Delete simulation.rs; write server_app.rs | Copied remaining simulation.rs content to src/server_app.rs. Replaced `SimulationPlugin` with `pub fn add_simulation_plugins(app: &mut App)` as the composition entry point. Updated src/server/bridge.rs and src/bridge.rs to call `add_simulation_plugins(&mut app)` instead of `.add_plugins(SimulationPlugin)`. Replaced `pub mod simulation` in lib.rs with `pub mod server_app` + `pub use server_app as simulation` backward-compat alias. Deleted src/simulation.rs. All 1629 tests pass; cargo check --features server clean. SimulationPlugin no longer appears in any code. The simulation-split series (#227: issues #233, #239, #245, #250, #254, #258, #261) is complete. Created wiki/concepts/server-app.md; updated wiki/concepts/architecture.md. Closes #261.

## [2026-05-16] ingest | Client split #1: ShipView resource extracted from ClientSimState | Extracted ship-level fields (pose, red_alert, view_mode, power_levels, hull_fraction, impulse_charge_progress) from ClientSimState into ShipView resource. Added ShipViewPlugin (client feature gate) that init_resource::<ShipView>() and runs apply_ship_view_messages system reading InboundServerMessage events. Moved is_active_camera_direction() from ClientSimState to ShipView. Updated compute_helm/weapons/system_chart/science_long_range_radar_view() signatures to take both &ClientSimState and &ShipView. Updated 10 systems in client/app.rs and 5 systems in client/phone_border/ to read Res<ShipView>. Registered ShipViewPlugin in client/bridge.rs wasm_client_init. Removed duplicate ship-view fields from ClientSimState + Default + apply(). All 1626 tests pass; cargo check --features client clean. Created wiki/concepts/ship-view.md. Closes #234.

## [2026-05-16] ingest | Client split #2: CaptainPanelPlugin extraction | Moved toggle_captain_panel_visibility from client/app.rs into src/client/phone_border/captain.rs. Extracted pure captain_panel_visible(lobby, token, active) -> bool helper and added 6 unit tests covering lobby phase, non-captain, single/multi-console visibility rules. Removed dead captain UI code from client/app.rs: setup_captain_ui (no-op stub), refresh_view_dir_highlights, refresh_red_alert_button, handle_view_dir_button_press, handle_red_alert_button_press, ViewDirButton/RedAlertButton/RedAlertLabel components, VIEW_BTN_BG_*/RED_ALERT_BG_* constants. 1682 tests pass with --features client; cargo check --features client clean. Created wiki/concepts/captain-panel.md. Closes #240.

## [2026-05-16] ingest | Client split #3: HelmPanelPlugin extraction | Moved all helm console UI from client/app.rs and client/phone_border/helm.rs into src/helm_panel.rs. Extracted pure helm_panel_visible(lobby, token, active) -> bool helper + helpers bearing_ticks, range_ring_radii, range_ring_labels, yaw_to_heading with 22 unit tests total. Removed from client/app.rs: toggle_helm_panel_visibility, helm_resend_tick, refresh_helm_knob_position, refresh_helm_readout, handle_on_screen_button_press, refresh_on_screen_button_style, draw_helm_radar, HelmTickTimer struct, HelmJoystickState/HelmTickTimer resource insertions, helm colour constants. Replaced client/phone_border/helm.rs with a thin re-export shim. Updated client/bridge.rs to register crate::helm_panel::HelmPanelPlugin directly. Added pub mod helm_panel to lib.rs (already present). All 1626 tests pass; cargo check --features client clean. Created wiki/concepts/helm-panel.md. Closes #246.

## [2026-05-16] ingest | Client split #4: WeaponsPanelPlugin extraction | Moved all Tactical console UI from client/app.rs into src/weapons_panel.rs. Extracted pure weapons_panel_visible(lobby, token, active) -> bool helper with 6 unit tests. Added 9 message-builder and SelectedTube-toggle tests (15 total). Removed from client/app.rs: toggle_weapons_panel_visibility, handle_fire_phaser_button_press, handle_phaser_mode_toggle_press, refresh_weapons_panel, handle_torpedo_tube_button_press, handle_fire_torpedo_button_press, refresh_torpedo_ui, draw_weapons_radar, setup_weapons_ui, SelectedTube resource, 10 private marker component definitions, weapons-radar colour constants, PhaserMode/fire_*_message imports. Made HideableElement, ComplexityPopupRoot, ComplexityPresetButton, ComplexityPopupConfirm, ComplexityDropdownRoot pub in client/app.rs for cross-module use. Registered WeaponsPanelPlugin in client/bridge.rs. Added pub mod weapons_panel to lib.rs (client feature gate). All 1703 tests pass with --features client; cargo check --features client clean. Created wiki/concepts/weapons-panel.md; updated wiki/index.md. Closes #251.

## [2026-05-16] ingest | Client split #5: RepairPanelPlugin extraction | Moved all Repair console UI from client/app.rs into src/repair_panel.rs. Extracted pure repair_panel_visible(lobby, token, active) -> bool helper with 6 unit tests. Removed from client/app.rs: setup_repair_ui, toggle_repair_panel_visibility, refresh_repair_panel, handle_repair_shape_button_press, refresh_repair_icon, 7 private marker component definitions (RepairPanel, RepairBreakdownLabel, RepairShapeButton, RepairShapeButtonRoot, RepairTeamRow, RepairTeamFill, RepairTeamStatusText), Shape import from messages. RepairPanel marker component now defined in repair_panel.rs. RepairButton/RepairButtonLabel/RepairIconLabel remain in client/app.rs (used by handle_repair_button_press and refresh_repair_button which handle the Helm-panel repair button). Registered RepairPanelPlugin in client/bridge.rs after WeaponsPanelPlugin. Added pub mod repair_panel to lib.rs (client feature gate). Apply paths for RepairState, ShowRepairIcon, ClearRepairIcon remain in client_sim.rs. All 1709 tests pass with --features client; cargo check --features client clean. Created wiki/concepts/repair-panel.md; updated wiki/index.md. Closes #255.

## [2026-05-16] ingest | Client split #6: PowerPanelPlugin extraction | Moved all Power console UI from client/app.rs into src/power_panel.rs. Extracted pure power_panel_visible(lobby, token, active) -> bool helper with 6 unit tests covering lobby phase, non-power player, single/multi-console visibility rules. Removed from client/app.rs: setup_power_ui, toggle_power_panel_visibility, refresh_power_panel, handle_increase_power, handle_decrease_power, 7 private marker component definitions (PowerPanel, PowerRow, PowerRowLevel, PowerIncButton, PowerDecButton, BatteryBar, BatteryLabel), 8 colour constants. Registered PowerPanelPlugin in client/bridge.rs after RepairPanelPlugin. Added pub mod power_panel to lib.rs (client feature gate). Apply paths for PowerState remain in client_sim.rs. All 1626 tests pass; cargo check --features client clean. Created wiki/concepts/power-panel.md; updated wiki/index.md. Closes #259.

## [2026-05-16] ingest | Client split #7: SciencePanelPlugin + CommsPanelPlugin | Created src/science_panel.rs with SciencePanelPlugin, ScienceView resource, view-mode selector (ScienceRadar/SystemChart), On Screen button, Cancel Impulse button, pure science_panel_visible helper, and 4 unit tests. Created src/comms_panel.rs with CommsPanelPlugin, CommsView placeholder resource, ClientCommsState init_resource (folded from comms_plugin.rs), toggle_comms_panel_visibility system, pure comms_panel_visible helper, and 3 unit tests. Deleted src/comms_plugin.rs. Removed pub mod comms_plugin from lib.rs; added pub mod science_panel and pub mod comms_panel (both client feature-gated). Registered SciencePanelPlugin and CommsPanelPlugin in client/bridge.rs after CaptainPanelPlugin. All 1626 tests pass; cargo check --features client clean. Created wiki/concepts/science-panel.md and wiki/concepts/comms-panel.md; updated wiki/index.md. Closes #262.

## [2026-05-16] ingest | Client split #8 (final): Thin composition add_client_plugins; client-split series #228 complete | Added pub fn add_client_plugins(app: &mut App) to src/client/app.rs (~35 lines) as the single canonical point registering all 10 client-side plugins. Updated src/client/bridge.rs::wasm_client_init to call add_client_plugins(&mut app) instead of listing plugins individually; removed per-plugin imports from the use block. Audit findings: src/client_sim.rs retained — ClientSimState still holds active console-specific state (repair, weapons, shields, world entities, modifiers, power, torpedo) not yet migrated to per-panel resources; the file is Bevy-free with an extensive test suite and is correct to keep. All 1626 tests pass; cargo check --features client clean. Created wiki/concepts/client-architecture.md cataloguing all panel plugins. Client-split series (#228: issues #234, #240, #246, #251, #255, #259, #262, #263) complete. Closes #263.

## [2026-05-19] ingest | World/scenario/map merger: single `assets/worlds/*.toml` per session | Unified map and scenario into one TOML file per world under `assets/worlds/` (`default.toml`, `patrol.toml`). New `wasm_load_world(path, toml)` in `server/bridge.rs` calls both `wasm_load_map` and `wasm_load_world_content` on the same TOML; internal `MapConfig` and `ScenarioConfig` kept as implementation detail and each parser silently ignores the other half's sections. Added `parse_world` + `WorldConfig` in `world/content.rs`. Renamed `ModifierSource::Scenario { id, tag }` to `ModifierSource::World { id, tag }` (codec, cache, world/server). Removed `TriggerAction::LoadScenario` and `TriggerAction::UnloadScenario` along with their TOML parse arms, dispatch arms, and 5 tests; scenario chaining is no longer supported (one world per session). `ScenarioManager::load_scenario`/`unload_scenario` remain as internal plumbing for `CommsInbox` / `ObjectiveManager` cleanup. Updated `server.html` to fetch `assets/worlds/default.toml` and call `wasm_load_world` (single fetch, no map->scenario chain). Deleted `assets/maps/` (default, axiom_system) and `assets/scenarios/` (default, patrol, before_the_fire, btf_*). Updated smoke tests `tactical-fire-flow.spec.ts` and `patrol.spec.ts` to intercept the worlds path and load the unified TOML. 1664 tests pass; both WASM builds (server + client) clean. Wiki: rewrote `concepts/world-plugin.md`.

## [2026-05-19] correction | Map/scenario merger is partial, not complete | Strict review of the previous log entry found the merger was only step 1: asset directory unified, JS-visible loader unified, server.html single-fetch, ModifierSource rename, LoadScenario/UnloadScenario removal. But underneath: `MapConfig` and `ScenarioConfig` remain distinct types, `WorldConfig` is `struct { map, scenario }` and `parse_world` runs both parsers over the same TOML, `wasm_load_world` is a shim calling `wasm_load_map` + `wasm_load_world_content` sequentially, TOML still has two block types (`[[entity]]` + `[[spawn]]`), three spawn pipelines remain, `ScenarioManager` + `ScenarioOwner` + `scenario_path` plumbing all survive, legacy `[[star]]`/`[[planet]]`/`[[asteroid_field]]` shorthand parsing still in `MapConfig`. Also fixed two bugs the rejected attempt left in: duplicate `use bevy::prelude::*` (world/server.rs:1,4) and duplicate unreachable `TriggerAction::SetAiState` match arm (world/server.rs:731). Rewrote `wiki/concepts/world-plugin.md` to honestly describe the partial state. Remaining work tracked by PRD #337.

## [2026-05-19] correction | Restored deleted scenario content | The world-merge commit (5c132e7) deleted 8 authored scenario/map TOMLs (`before_the_fire.toml`, 5 `btf_*.toml`, `axiom_system.toml`, scenario `default.toml`) on the rationale that they used schema the current engine doesn't support (`load_scenario`, `set_flag`/`flag_is_set`, `force_ai_state`, `on_attacked_by`, `comms.response.actions`) and depended on the scenario-chaining `LoadScenario` trigger action that the same commit removed. They were authored content (the "Before the Fire" scenario in Axiom System with 3 mutually-exclusive narrative paths + 2 side-quests), not drafts. Restored all 8 files into `assets/worlds/`. They are NOT loadable by the current engine — they remain as preserved authored content until either (a) the engine grows the missing features, or (b) someone migrates them to the supported schema (which would lose the mutual-path-exclusivity design, since that depends on scenario chaining).

## [2026-05-19] ingest | PRD #338 slice 1: unified WorldConfig + spawn_world_entities | Issue #338 slice 1 shipped. New pure module `src/world/config.rs` introduces `RawWorld`, `WorldConfig` (typed `HashMap<String, [f32;3]>` anchors + `Vec<EntityInstance>` entities), `parse_world` (single `toml::from_str`), `entity_template_paths` (dedup queue), and `partition_immediate_entities` (asteroid-field vs. other routing). `wasm_load_world` in `config_cache.rs` is no longer a shim ?? it parses the unified `WorldConfig` into a new `WORLD_CONFIG` thread-local, then transitionally still drives `parse_map_config` and `parse_scenario` to populate `MAP_CONFIG` / `WORLD_CONTENT_CONFIG` for callers that have not migrated yet; `server/bridge.rs` shrinks to a thin `#[wasm_bindgen]` delegate. New `WorldPlugin` Startup chain `insert_world_config_resource` �~F~R `spawn_world_entities` �~F~R `spawn_scenario_entities` �~F~R `init_scenario_runtime`: the new `spawn_world_entities` system reads `Res<WorldConfig>` and spawns ONLY entries whose resolved `EntityConfig.asteroid_field.is_some()`. Mirror skip guard added to `server_app::setup_world_from_config` so the same instance can never spawn twice. `ai::server::tick_ai_controllers` now prefers `Res<WorldConfig>`, falls back to `MapConfig::anchors` only when absent (native tests); extracted pure helper `anchors_from_world_config`. 1683 lib tests pass (was 1664, +19: 17 in `world::config`, 2 in `ai::server`). Both `cargo check --features server` and `cargo check --features client --no-default-features` clean at the same 23-warning baseline as before the slice. Updated `AGENTS.md` (entities/config_cache + world/server + world/config rows) and `docs/toml-authoring-guide.md` ##1. Wiki: rewrote `concepts/world-plugin.md` to describe the new load path and spawn coordination.

## [2026-05-19] PRD #341 cleanup | Moved non-loadable authored world TOMLs out of assets/worlds/ | The "Before the Fire" scenario content (axiom_system.toml, before_the_fire.toml, btf_*.toml — 8 files total) was preserved in assets/worlds/ by an earlier correction commit despite being non-loadable (uses LoadScenario/UnloadScenario, set_flag/flag_is_set, force_ai_state, on_attacked_by, comms.response.actions — none of which the current engine supports). Per PRD #341 these were moved to docs/preserved-content/ with a README explaining provenance and migration prerequisites. assets/worlds/ now contains only default.toml + patrol.toml, both loadable. The acceptance grep g "^\[\[(star|planet|asteroid_field)\]\]" assets/ returns zero hits. Also as part of PRD #341: deleted src/entities/map_config.rs (EntityInstance → world::config::WorldEntity, EntityInstanceSpawnOn → world::config::WorldEntitySpawnOn; leaf shape types Global/Star/Planet/Grid/AsteroidFieldConfig moved to entities/config.rs). Renamed the AI's WorldEntity → AiWorldEntity to free the name. Added two-branch tests for setup_fallback_world's run_if gate. 1608 tests pass; 0 warnings on both features; both wasm builds clean.

## [2026-05-19] ingest | PRD #342 slice 5: multi-world layering deleted | Removed the legacy ScenarioManager/ScenarioRuntime/owner-component plumbing and the per-element world-path tags from TriggerState/CommsTemplateState/ActiveDialogue/FiredTrigger/FiredCommsTemplate. Simplified CommsInbox::inject and ObjectiveManager::add (dropped the world-id parameter, removed the matching unload paths and record fields). Rewrote src/world/content.rs without the layered runtime; updated src/world/server.rs trigger/comms call sites to use a constant "world" modifier id. Deleted the two scenario-owner component tests in src/world/server.rs and the owner usage in src/ai/server.rs tests. 1592 tests pass; both feature builds 0 warnings. Updated AGENTS.md, CONTEXT.md, docs/REQUIREMENTS.md, wiki/concepts/world-plugin.md.


## [2026-05-19] ingest | PRD #343 slice 6: world-merger final validation | Full build/test matrix green on PRD #337 completion. cargo test 1592 passed; cargo check --features server clean; cargo check --features client --no-default-features clean; cargo build --target wasm32-unknown-unknown both features succeeds; trunk build --release server + client succeeds. Smoke tests: world-bootstrap, patrol, comms, tactical-fire-flow all green (3 tactical tests required updating tests/smoke/tactical-fire-flow.spec.ts to use [[entity]] with name= instead of legacy [[spawn]] block — overlooked test asset under PRD #341 cleanup). Pre-existing failures (2 station/reassignment specs) remain unrelated to the merger. PRD #337 closed; slices #338-#343 merged into the trunk.


## [2026-05-19] ingest | PRD #337 post-close gap sweep | Post-close review identified one critical and four doc gaps. Fixed: (C1) src/console/repair/client.rs test module was missing use ClientMessage import — client feature test build was broken since slice 4; surfaced 135 additional client-only tests. (I2) Implemented [[entity]] relative_to + offset schema per PRD spec: added fields to WorldEntity, added esolve_entity_position_with taking a 
ame -> resolved_position map, added uild_named_entity_positions helper, wired into both spawn paths (spawn_immediate_entities_internal and setup_world). Precedence is relative_to > anchor > position > origin. relative_to-to-relative_to chains are not supported (single-pass, no cycle detection). 5 new pure tests. (I4) Rewrote docs/REQUIREMENTS.md World TOML, Spawn entries (renamed to Entity entries), Configuration & Authoring sections to remove references to deleted MapConfig/ScenarioConfig/[[spawn]]/wasm_load_map/wasm_load_world_content. (I5) Rewrote docs/toml-authoring-guide.md §1 World TOML section: deleted the entire [[spawn]] reference subsection, updated parser-overview paragraph, added relative_to/offset row + example to [[entity]] table. (I6) Updated CONTEXT.md WorldPlugin entry to drop ScenarioConfig/parse_scenario qualifier. (M7) AGENTS.md line 252: '[[spawn]]s' -> 'named + anonymous [[entity]] instances'. (M8) Updated assets/worlds/default.toml: 'Starbase Alpha' entity now uses nchor = "starbase_alpha" instead of inlining its position; stale comments updated. Final test counts: server 1597, client 1732. Both cargo checks 0 warnings; both wasm builds clean; trunk release builds succeed. Smoke tests world-bootstrap, comms, patrol, tactical-fire-flow (3) all green.

## [2026-05-20] ingest | Slice 1: [torpedoes] TOML block for player_ship.toml | touched: concepts/weapons-plugin.md; player_ship.toml [torpedoes] block; entities/config.rs TorpedoesConfig + drift test; weapons_plugin TorpedoSystemResource overridden in server_app.rs

## [2026-05-20] ingest | Slice 2: [repair] TOML block + broadcast to client | touched: concepts/repair-plugin.md; player_ship.toml [repair] block; entities/config.rs RepairConfig + drift test; repair_teams.rs RepairTimings runtime struct; ShipClientConfig.repair_{travel_secs,rate_hp_per_sec} ride Welcome; console/repair/client.rs reads broadcast timings, hardcoded constants reduced to fallback

## [2026-05-20] ingest | Slice 3: PhaserCombat + ShieldsBase TOML wiring | touched: concepts/weapons-plugin.md; player_ship.toml [shields_console.base] block (reused existing [weapons_console] beam_*/cooldown_secs/beam_range for phasers); entities/config.rs PhaserCombatConfig + ShieldsBaseConfig + 11 new tests (round-trip + drift guards); console/weapons/server.rs PhaserCombatConfigResource bridges legacy BEAM_DURATION/COOLDOWN constants + radar::PHASER_RANGE for handle_fire_phaser/tick_active_beam/broadcaster; PhaserCooldown::start(&config) reads cooldown from resource; server_app.rs spawn_game_start_entities constructs ShieldSystem::new(&base) then overlays focus, inserts PhaserCombatConfigResource from [weapons_console]; console_ai/server.rs test_app init_resource patch. E2E tests in console/weapons/server.rs + weapons/shield.rs verify TOML flows to live state. fire_arc_deg/auto_arc_deg + radar::PHASER_RANGE intentionally NOT TOML-driven (engineering invariants). PhaserConfig/PhaserSystem in weapons/phaser.rs left as untouched dead code. 1649 server + 1792 client tests pass.

## [2026-05-22] ingest | Slice 1 of entity-schema refactor: delete ScienceConsoleConfig | touched: sources/design-03-science-console.md (Status note); sources/refactor-2026-05-entity-schema.md (new). EntityConfig.science_console + ScienceConsoleConfig deleted (entities/config.rs). Consoles were never data-driven; field was unused by gameplay code. No TOML migration needed.

## [2026-05-22] ingest | Slice 2 of entity-schema refactor: stations are plain entities | touched: sources/design-06-space-stations.md (Status note); sources/refactor-2026-05-entity-schema.md. StationConfig/StationShape/EntityConfig.station deleted. Stations now compose [mesh] + [hull] + [collider]; hull damage via [hull].hull_integrity flows through apply_hull_damage (ship/damage.rs) - closes latent bug where station hull damage was silently dropped. Migrated assets/entities/station_*.toml.

## [2026-05-22] ingest | Slice 3 of entity-schema refactor: stars/planets as entities, lights as array | touched: AGENTS.md (entities/config Module Map entries); entities/world-data.md (rewrite); sources/design-01-entity-config-files.md (Status note); sources/refactor-2026-05-entity-schema.md. Added EntityConfig.name (entities/config.rs:640), MeshConfig.emissive (renderer default 0.4; star=2.0), [[light]] array-of-tables as Vec<LightConfig> {kind, colour, intensity, range?}, EntityName + Lights components (entities/spawner.rs:22,28). render_spawned_entities rewritten at server_app.rs:1147. StarConfig/PlanetConfig deleted. Migrated assets/entities/{star_sun,planet_earth}.toml.

## [2026-05-22] ingest | Slice 4 of entity-schema refactor: TransformConfig + AmbientLightConfig | touched: entities/world-data.md; concepts/world-plugin.md (Startup chain); sources/design-02-game-map.md (Status note); sources/refactor-2026-05-entity-schema.md; index.md (new refactor link). New TransformConfig (world/config.rs:48) with position/anchor/relative_to/offset/rotation (XYZ Euler radians)/scale (default [1,1,1]); WorldEntity flat placement fields replaced by transform: Option<TransformConfig> (:142). resolve_entity_position_with (:752) wraps TransformConfig::resolve; precedence relative_to+offset > anchor > position > origin. Scale lives ONLY on TransformConfig (no EntityConfig.scale). New AmbientLightConfig {color, brightness} (:115) on WorldConfig (:243,:499); spawn_world_ambient_light system (server/renderer.rs:209) registered in PostStartup at :91, after insert_world_config_resource (world/server.rs:152); fallback Color::srgb(0.6,0.55,0.5) at 300.0. Migrated assets/worlds/{default,patrol}.toml. STALE OUT-OF-SCOPE: docs/toml-authoring-guide.md:973 still references deleted src/entities/map_config.rs.

## [2026-05-22] ingest | PRD #350 Scenario Editor Rewrite | Created sources/prd-350-scenario-editor-rewrite.md; rewrote entities/editor.md from 9-line stub into full schema page covering three modes (World/Entity/Definitions), FSA project-root + IndexedDB persistence, SaveFlow + InvalidationBus, entity cache, composite validation, smol-toml parsers, undo/redo, schema additions (extra_worlds + load_world/unload_world), and the Vitest test inventory under editor/tests/. Updated index.md (Entities + Sources).

## [2026-05-22] correction | PRD #350 wiki fact-check | touched: entities/editor.md, sources/prd-350-scenario-editor-rewrite.md | reason: extra_worlds + LoadWorld/UnloadWorld landed via issue #352; fixed broken prd-153 link; corrected line citations.

## [2026-05-22] update | issue #381 V1 editor deletion | touched: entities/editor.md, sources/prd-350-scenario-editor-rewrite.md | reason: editor/app.js deleted; editor/app-v2.js is now the sole boot entry mounting Scenario / Entity / Definitions uniformly. editor/scenario-mode.js (mountScenarioMode) owns the former app.js World-mode responsibilities (canvas, layers, properties, world content, triggerable worlds, new-world dialog). Removed dead v2-text scaffolding (setupOpenFile/setupSaveFile + .v2-text-toolbar + #v2-text-panel) from app-v2.js / editor.html / style.css; stripped #addLayerBtn sibling fallback from new-world-dialog.js (template owns #newWorldBtn); dropped stale null crossRefIndex args from saveFlow.saveAll/saveActive in scenario-mode.js.

## [2026-05-22] cleanup | dead load_scenario removed from Rust runtime | touched: src/world/config.rs, src/world/server.rs | reason: PRD #341 deletion was incomplete; LoadWorld/UnloadWorld replaced it but the dead variant lingered.

## [2026-05-22] feature | comms range | touched: concepts/comms-range.md (new), index.md | reason: Per-entity `[comms].range` opt-in → `CommsRange` Component → `update_comms_range_flags` system stamps `CommsContact.in_range` + `CommsMessage.sender_in_range` from ship/entity transforms via pure `comms::in_range(d, min(a,b))`. Server enforces Hail/Respond gate; client hides out-of-range contacts and greys response buttons with alert-red `[OUT OF RANGE]` tag. Entities without `[comms]` block silently dropped from contacts. `range_active` stays true after ship despawn (no back-door); orphaned senders default to false; injection sites stamp via `current_sender_in_range` helper. +20 tests, 1710 lib tests pass.

## [2026-05-22] ingest | Impulse autopilot + helm UI rework | While impulse is Active, server forces thrust=1.0/steering=0.0 and multiplies ShipPhysicsConfig.acceleration by ImpulseConfigResource.acceleration_multiplier (new TOML [helm_console].impulse_acceleration_multiplier, default 5.0, <=0.0 falls back to IMPULSE_ACCELERATION_MULTIPLIER const). Speed multiplier now actually flows from TOML through ImpulseConfigResource.speed_multiplier into translate_impulse_modifiers (const kept as resource Default). Helm UI: joystick hidden during Charging+Active, replaced by progress bar + CANCEL IMPULSE button + status text via pure format_impulse_status; ShipClientConfig.impulse_charge_duration added on the wire. Rising-edge Idle->Charging detectors on both sides clear stale state (reset_joystick_drag client, LastHelmInput server) so post-cancel ticks can't replay phantom steering. JoystickResendTimer.paused gates the 10 Hz resend. 1725 tests pass. Updated wiki/entities/helm-console.md, wiki/concepts/modifier-coordination.md, AGENTS.md.

## [2026-06-02] feature | Comms threading — chat-room UI | touched: concepts/comms-panel.md (full rewrite). Added `thread_id: String` (#[serde(default)]) to `CommsMessage`; `thread_id: String` to `ActiveDialogue` (world/content.rs). Server (world/server.rs) generates a UUID thread_id on first hail/auto-inject and propagates it to all follow-up nodes. Client (client_comms.rs) replaced `selected_message_id` with `selected_thread_id`; added `ThreadSummary`, `sorted_threads()`, `thread_messages()`, `active_message_for_thread()`. UI (console/comms/client.rs): inbox shows one row per thread with subject from latest message; chat panel renders all messages chronologically with player-reply "You: …" bubbles inline after each responded message. Response buttons appear only for the active (latest unresponded) message in the thread. 2010 tests pass.

## [2026-05-22] feature | per-bank phasers + per-tube torpedoes | touched: assets/entities/player_ship.toml, src/core/messages.rs, src/entities/config.rs, src/weapons/phaser.rs, src/weapons/torpedo.rs, src/weapons/beam_render.rs, src/console/weapons/{server,client}.rs, src/console_ai/{core,server}.rs, src/lobby/server.rs, src/gui/radar.rs, src/client_sim.rs, client.html, tests/smoke/tactical-fire-flow.spec.ts, wiki/concepts/weapons-plugin.md, wiki/sources/player_ship_toml.md (new) | reason: Replaced single hardcoded `port` phaser bank + three hardcoded torpedo tubes with data-driven `[[weapons_console.phaser_banks]]` and `[[torpedoes.tubes]]` TOML arrays. New schema: PhaserBankConfig {id, facing_deg, fire_arc_deg, auto_arc_deg, beam_range} with validator enforcing auto_arc_deg <= fire_arc_deg; TorpedoTubeConfig {id, facing_deg, fire_arc_deg}; shared torpedo ammo pool. Wire: WeaponsUpdate now per-bank+per-tube; FirePhaser{bank}; FireTorpedo{tube,target_uuid}; ShipClientConfig ships phaser_banks/torpedo_tubes/phaser_beam_color/torpedo_arc_color so the client can render fire-arc overlays without knowing auto_arc_deg. Client: Tactical panel spawns dynamically from lobby.ship_config; WeaponsPanelLayoutKey resource + respawn_weapons_on_layout_change rebuild on schema change; radar arcs via RadarArcs/RadarArcMaterial + RadarTargetHighlight + RadarEntityUuid. Legacy client.html UI aggregates banks for fire-ready and sends FirePhaser{bank:<first ready>}. 1745 server + 2023 client lib tests pass.

## [2026-06-04] lint | Reconcile PRD #115 (Native PC Server) + slices #135–141 with code | touched: sources/prd-115-native-pc-server.md, GitHub issues #115/#135/#136/#141 | reason: Reviewed PRD #115 against current code — design sound, but stale refs corrected. (1) native_config_loader scope was understated: must populate all three `ConfigCachePlugin` resource families (entity configs via world-driven traversal + complexity presets `assets/complexity/` + faction registry `assets/factions/`), not just world+entity, else console AI + NPC patrols missing — #136 rewritten. (2) `MapConfig`/`map_config.rs` don't exist → `WorldConfig`/`parse_world` in `src/world/config.rs`; map+scenario are one world TOML. (3) Console count 4→9 (CaptainChair/Helm/Tactical/Repair/Sensors/Shields/Navigation/Power/Comms); native gets all nine free via shared sim core. (4) Removed obsolete "out of scope" bullet excluding the `src/server`/`src/client`/`src/shared` draft refactor — it has landed; `src/server/`+`src/client/` are now live feature-gated bridge modules, no `src/shared/`. (5) `webgl2` is an unconditional bevy feature in Cargo.toml → #135 gains criterion to make it conditional. (6) #141 cloudflared pin `2024.11.1` flagged stale. NOTE: roadmap/overview.md remains broadly stale (says "5 consoles"; lists #118/#119/#120 as open though Repair/Power/Comms/stations have shipped) — out of scope for this pass, flagged for a future roadmap reconciliation.

## [2026-06-04] lint | Full roadmap/overview.md reconciliation | touched: roadmap/overview.md | reason: The overview was a full wave behind (last updated 2026-05-11). Verified against `Console` enum (`src/core/messages.rs`, 9 variants) and live GitHub PRD-issue states. Rewrote: (1) "5 consoles" → nine (CaptainChair/Helm/Tactical/Repair/Sensors/Shields/Navigation/Power/Comms). (2) Moved #117/#118/#119/#120 from "in flight" to Shipped, and added all the PRDs that landed since (#41, #77, #86, #142 AI, #153 regions, #154 complexity, #180 viewscreen frame, #218 scenario-into-world, #266 States/Observers/SystemSets, #271 integer modifiers, #272 per-console hull, #302 reconciliation, #317 GUI lib, #337 map/scenario merger, #344 Helm/Captain GUI, #350 editor v2, #397 branching scenarios). (3) Open set now #116 Save/Load, #398 'Before the Fire' scenario, #419 HTML console UI, plus #115 native (PRD closed, slices #135–141 on hold). (4) Drafts table updated — drafts 1–11 all shipped, mapped to their PRDs. (5) Rewrote Themes + Tensions to current reality (save fidelity vs shipped scenario/region/AI state; #419 HTML-UI migration vs Bevy/WASM client; native slices on hold staying protocol-compatible). Source pages linked where they exist; GitHub issue URLs used for PRDs without a wiki source page (#77, #86, #218, #266, #271, #272, #302, #317, #337, #344, #397, #398, #419).

## [2026-06-12] fix | Tactical short-landscape controls stay on screen | touched: gui/weapons-console.html, tests/smoke/weapons-console.spec.ts, wiki/concepts/weapons-panel.md | reason: The HTML Tactical console now applies its compact landscape rule through 480px-high viewports and scales chip/status/FIRE button heights with dynamic viewport units. The smoke regression checks an 844x390 viewport and asserts the phaser/torpedo/mode buttons remain within the visible iframe.

## [2026-06-08] feature | Responsive lobby reflow + dead Bevy UI removal | touched: concepts/server-lobby-ui.md (new), index.md, server.html (responsive CSS+JS), src/server/viewscreen_border.rs (-868 lines of dead Bevy lobby UI), tests/smoke/lobby-responsive.spec.ts (new) | reason: lobby station cards now resize and reflow with viewport via `repeat(auto-fit, minmax(220px, 360px))` + clamp() font sizing; `@media (orientation: portrait), (max-width: 720px)` triggers compact mode (right rail collapses below station grid; per-slot RESERVED placeholders hide; single aggregate chip `#reserved-aggregate.active` replaces them; CONNECTED list becomes flex-wrap of `.spectator-pill` chips). Closed the dead-code dependency in viewscreen_border.rs by deleting LobbyScreenRoot/LobbyGridRoot/LobbyStationCard/LobbyCrewDisplay/LobbyReadyVal markers + spawn_lobby_screen/rebuild_lobby_station_grid/update_lobby_header_values/toggle_lobby_screen_visibility/spawn_station_card/spawn_station_placeholder/ready_status/complexity_label functions + 5 unit tests for the deleted helpers + `Update` schedule registrations + the StationDef import. Closes #436. Cargo check + 2040 lib tests still pass; 49/49 smoke tests still pass.

## [2026-06-08] ingest | PRD #438 HTML/JS Client GUI Shell + issue #439 HTML phone bezel shipped | touched: sources/prd-438-html-client-gui-shell.md (new), sources/issue-439-html-phone-bezel.md (new), sources/prd-187-phone-console-hud.md (superseded note), concepts/client-architecture.md (PhoneBorderPlugin marked transitional), index.md (added #438 + #439 source pages, fixed broken prd-187 link from `prd-187-phone-bezel.md` → `prd-187-phone-console-hud.md`) | reason: First slice of PRD #438 shipped. New `gui/phone-bezel.js` ES module exports `bezelSrc(slot, alert)` pure function (consumed by both client.html via `window.bezelSrc` and Vitest tests). `client.html` gains DOM `#phone-bezel` (4 corner `<img>` + 4 edge `<div>`, z-index 15, hidden by default, shown only in `InProgress` phase), `applyBezelAlert()` handler, and a `state.redAlert = !!snap.red_alert` read in the SimState case. `#status` z-index raised 5 → 25 so connection status survives the bottom-left corner. New Vitest test directory `tests/client/` registered in `vitest.config.js`; `tests/client/phone-bezel.test.js` covers all 8 slots × 2 alert states + truthy/falsy edge cases (19 tests). Rust `PhoneBorderPlugin` (PRD #187) still loaded — removal deferred to issue #442 once #440 (lobby) and #441 (tab bar) ship. 1001/1001 vitest tests pass; `cargo check --features client --no-default-features` clean.

## [2026-06-08] feature | Issue #440 lobby integration + phase toggle shipped | touched: sources/issue-440-html-lobby-phase-toggle.md (new), sources/prd-438-html-client-gui-shell.md (#440 marked shipped), index.md (#440 entry), client.html (render() wiring), gui/phase-toggle.js (new ES module), tests/client/phase-toggle.test.js (new) | reason: Second slice of PRD #438. `gui/phase-toggle.js` exports the pure `sectionVisibility(phase) -> {lobby, game, bezel}` function plus `isInGame(phase)` and frozen `IN_GAME_PHASES = ['InProgress', 'GameOver']`. Wired into `render()` at `client.html:648` replacing the hand-rolled `s.phase === 'InProgress'` check, which previously hid the game shell during `GameOver` (would have black-screened the post-mission summary). Falls back to inline duplicated truth table when `window.sectionVisibility` hasn't loaded yet so first paint stays correct on slow networks. Unknown phases fail-safe to lobby visibility. Lobby HTML/CSS/JS merger itself landed earlier in commit `7d5f0d0` -- this slice is the final wiring + module. 1011/1011 vitest pass (10 new from phase-toggle); `cargo check --features client --no-default-features` clean.

## [2026-06-08] feature | Issue #441 HTML tab bar + content switcher shipped (incl. #439 bezel display fix) | touched: sources/issue-441-html-tab-bar-content-switching.md (new), sources/prd-438-html-client-gui-shell.md (#441 marked shipped), index.md (#441 entry), client.html (CSS + DOM + render() + setActiveConsole + initOrientationWatch + bezel aria-hidden), gui/tab-bar.js (new ES module), gui/content-switcher.js (new ES module), tests/client/tab-bar.test.js (new, 36 tests), tests/client/content-switcher.test.js (new, 20 tests) | reason: Third slice of PRD #438. New `gui/tab-bar.js` exports `tabBarLayout(consoles, active, orientation, inGame)`, `renderTabBar(root, layout, {onPress})`, `useInitials()`, `currentOrientation()`, and frozen `CONSOLE_LABEL` / `CONSOLE_INITIAL` / `INITIALS_THRESHOLD = 5`. Bevy parity: hidden when not in-game or single-console; portrait collapses to initials at >= 5 consoles, landscape always full names. New `gui/content-switcher.js` exports `consoleSections(activeConsole, inGame) -> {game-ui, weapons-ui, repair-ui}`, `sectionForConsole()`, `isBevyConsole()`, frozen `CONSOLE_SECTION` (CaptainChair/Tactical/Repair only -- the other six consoles are rendered by Bevy). In `client.html`: new `setActiveConsole(name)` helper consolidates the three previously-duplicated call sites (render reconciler, swipe handler, tab-bar onPress); idempotent no-op when value unchanged. New `#console-tab-bar` DOM root + CSS driven by `[data-orientation]` attribute (portrait top-strip between 120px-wide corners with `flex-wrap` for 9-initial overflow safety, landscape left-strip between 72px-tall corners, z-index 16 above bezel at 15). Visibility driven by `aria-hidden` attribute + CSS `[aria-hidden="true"] { display: none }` rule, not inline `style.display` -- the latter has the trap that `style.display = ''` removes the inline override and the stylesheet's `display: none` rule wins. The same latent bug existed on `#phone-bezel` from issue #439 (only didn't man

## [2026-06-08] feature | Issue #442 Bevy cleanup shipped — PRD #438 closes | touched: sources/issue-442-bevy-cleanup.md (new), sources/prd-438-html-client-gui-shell.md (status in-flight -> shipped; #442 marked shipped), index.md (#442 entry + PRD #438 line flipped to shipped), src/client/console_shell.rs (712 -> 195 lines), src/client/phone_border/framing.rs (597 -> 258 lines), src/client/app.rs (1525 -> 504 lines) | reason: Fourth and final slice of PRD #438. Deletes the Bevy lobby UI tree, embedded tab bar widget, and phone-bezel frame that #439/#440/#441 already superseded. Surgical-neuter approach: kept the surfaces nine per-console panel plugins depend on (`ConsoleShell::spawn(...)` exact pre-#442 signature with `_phone_assets` arg prefixed `_`; `ConsoleShellEntities { root, primary, secondary }` minus `tab_bar`; `PhoneAssets` 32+ texture handles; `DeviceOrientation` + `is_landscape`; `PhoneBorderPlugin` name kept for `add_client_plugins` compat) so no panel plugin needed touching. Removed: `EmbeddedTabBar`, `EmbeddedTabButton`, `TabBarSnapshot`, `rebuild_embedded_tab_bars`, `handle_embedded_tab_press`, `tab_button_visuals`, `ConsoleShellPlugin`, the 44px Rust bezel inset (HTML CSS now provides), `spawn_bezel_on_startup`, `refresh_alert_banner`, `swap_phone_border_textures`, `reparent_panels_into_bezel`, `update_red_alert_intensity`, `AlertBannerText`, `pulse_intensity`/`sine_pulse`/`approach` helpers + tests, plus 21 lobby-UI marker components (`LobbyRoot`, `GameOverScreen`, `LandscapeMode`, `EngageButton`, `StationButton`, `ComplexitySegControl`, ...) + `setup_lobby_ui` (~300 lines) + every station-row/detail/footer/engage-button helper + the four lobby press-handler systems + `detect_initial_orientation`/`detect_orientation_change`/`toggle_lobby_visibility_on_phase`/`toggle_game_over_visibility`/`rebuild_lobby_ui_on_change`. Dropped `ConsoleShellPlugin` from `add_client_plugins`. Added `setup_ui_camera` Startup system spawning `(Camera2d, IsDefaultUiCamera)` (moved out of the deleted `setup_lobby_ui`). `BorderAssets` left in place in src/gui/border.rs (now orphaned on client, harmless; server's `ViewscreenBorderPlugin` is independent). 2040/2040 cargo lib tests pass, 1067/1067 vitest pass, both feature builds clean.ifest in smoke tests because none assert on in-game phase visibility) and was fixed in the same commit. New `initOrientationWatch()` IIFE re-runs `scheduleRender()` on `resize` / `orientationchange`. `render()` consumes `consoleSections()` (with inline fallback) and `renderTabBar()` (with feature guard). Tab buttons use `role="tab"` + `aria-selected` (not `aria-pressed`) to match the `role="tablist"` container's ARIA contract. Swipe handler refactored to call `setActiveConsole()` + `scheduleRender()` (consistency with the rest of the message-handler path). 1067/1067 vitest pass (56 new); `cargo check --features client --no-default-features` clean. Bevy `src/client/console_shell.rs` (712 lines) still loaded -- removal deferred to issue #442. Until #442 lands, both implementations co-exist; the HTML tab bar sits on top of the Bevy one in the bezel-content area, so user-visible double-rendering is possible on a release cut between #441 and #442.

## [2026-06-09] feature | Issue #425 Power+Repair HTML consoles complete — Playwright smoke tests added | touched: tests/smoke/power-console.spec.ts (new, 3 tests), tests/smoke/repair-console.spec.ts (new, 3 tests) | reason: Final step of #425. Added 3 Playwright tests for the power console (render entries+battery, locked state, +/- button __sendAction envelopes) and 3 for the repair console (Idle/Travelling/Repairing slots, Returning+queued slot, dispatch-button __sendAction envelopes). All 58/58 smoke tests pass.

## [2026-06-08] cleanup | PRD #438 follow-up - orphan border deleted, CONSOLE_LABEL deduped, active-console extracted | touched: sources/issue-442-bevy-cleanup.md (Follow-up cleanup pass section + cross-ref note), sources/prd-438-html-client-gui-shell.md (Module 8 explicit deferral note), src/gui/border.rs (DELETED), src/gui/mod.rs (drop `pub mod border;` + re-export block + `GuiBorderPlugin` registration), src/gui/vignette.rs (doc comments 1-25 updated for post-#442 reality), src/ship_view.rs (doc comments 121-161 drop `BorderContentArea` refs), client.html (5 edits + new script tag), gui/active-console.js (new pure module), tests/client/active-console.test.js (new, 9 tests) | reason: Post-merge cleanup addressing all three `review-plan` recommendations against PRD #438 + Module 8 deferral. (1) `src/gui/border.rs` was fully orphaned after the #442 `framing.rs` rewrite - every export (`BorderAssets`, `BorderConfig`, `CornerSlot`, `EdgeSlot`, `GuiBorder`, `BorderContentArea`, `GuiBorderWidget`, `update_border_textures`, `GuiBorderPlugin`) and its 5 inline tests had zero external references. Deleted entirely; `src/gui/mod.rs` lost three lines (module decl, re-export block, plugin registration). Server's `ViewscreenBorderPlugin` is unaffected - it uses its own `ViewscreenBorderAssets` and is independent. (2) Inline `CONSOLE_LABEL`/`CONSOLE_INITIAL` tables at `client.html:699-720` duplicated `gui/tab-bar.js:18-40`. Dropped both inline tables; added a `consoleLabel(name)` helper that reads `window.CONSOLE_LABEL` from `gui/tab-bar.js` (the single frozen source) with raw-name fallback for first-paint race safety. Three call sites (`client.html:749, 837, 883`) routed through the helper. `CONSOLE_INITIAL` was declared inline but never used inline - only `gui/tab-bar.js` consumes it via its own module-local copy - so dropping the inline duplicate had no behaviour change. (3) Extracted `nextActiveConsole(current, name) -> {changed, next, wasmArg}` to `gui/active-console.js` with 9 Vitest tests covering null/undefined/empty-string + idempotency. Inline `setActiveConsole(name)` at `client.html:659` now delegates the `null`/`undefined`/`""` -> `""` sentinel mapping (the "follow primary console" contract at `src/client/bridge.rs:160`) to the module, locking it by test instead of inspection. Inline fallback branch retained for slow-network first-paint. (4) PRD #438 Module 8 (`gui/captain-{landscape,portrait}.html` reference files) marked explicitly Deferred - was already "Optional design QA" in the PRD body, no GitHub issue planned. Post-cleanup test counts: `cargo test` 2035/2035 pass (was 2040 pre-cleanup; 5 inline tests in deleted `src/gui/border.rs` are gone), `npm.cmd run test:editor` 1076/1076 pass (1067 pre-cleanup + 9 new), `cargo check --features client --no-default-features` + default `cargo check` both clean (13 pre-existing dead-code warnings in `src/server/viewscreen_border.rs` are not introduced by PRD #438 and remain).

## [2026-06-10] planning | HTML console uniformity workstream planned — issues #450–#459 filed under PRD #419 | touched: GitHub issues only (no code) | reason: Audit of the 5 converted HTML consoles found copy-pasted transport shims with drift (5x), duplicated __updateConsole boilerplate (5x), 5 near-identical iframe bridges in client.html (~200 lines), blip projection trig existing 3x (client.html x2 inline + radar-math.js), an untested 60-line action-dispatch switch, and a naming split (state pushes use 'Captain', actions use 'CaptainChair'). Plan: #450 name standardization, #451 gui/console-core.js shared runtime (tracer via Repair) + ADR-0001 supersede, #452 convert remaining 4 consoles, #453 console-registry + generic iframe-bridge, #454 console-state.js pure builders + shared buildBlips via radar-math, #455 action-map.js, #456 delete the 5 duplicated Bevy client panels (per #419 deletion plan), #457/#458 Sensors + Navigation HTML conversions on the new rails, #459 tracking issue for full client-WASM removal. Direction decision: the client migrates off Rust completely — JS is the source of truth for client-side console state; Rust structs in core/messages.rs stay as the wire contract for the server-pushed/native-wry path.
## [2026-06-10] code | #460 client state modules ported to pure JS | touched: gui/lobby-state.js, gui/sim-state.js, gui/comms-state.js, gui/complexity-store.js (new), client.html, src/client/app.rs, tests/client/*.test.js, wiki/concepts/ship-view.md | reason: Slice A of client-WASM removal (#459 direction): the four Rust client-state models now live as JS singletons with apply(serverMessage); client.html handleMessage() feeds every inbound message through them and mirrors them into the inline state object; the Bevy apply_inbound_messages drain was deleted from src/client/app.rs (ShipViewPlugin is now the sole InboundServerMessage reader; remaining Bevy readers of LobbyState/ClientSimState stop receiving updates and are removed in #461/#462).
## [2026-06-11] code | #461 complexity UI & hideable-elements ported to pure JS | touched: gui/hideable-elements.js, gui/complexity-ui.js (new), gui/console-core.js, gui/console-state.js, gui/console.css, gui/weapons-console.html, gui/power-console.html, client.html, src/client/app.rs, src/client/elements.rs, tests/client/hideable-elements.test.js, tests/smoke/{weapons,power}-console.spec.ts | reason: Slice B of client-WASM removal (#459). Per-console preset→hidden_elements maps from assets/complexity/*.toml are embedded in hideable-elements.js; console-core.js toggles .cpx-hidden on [data-hideable] nodes after every render; the lobby segmented selector (renderComplexitySelectors + complexity-ui.js) replaces the Bevy dropdown/popup and persists via window.complexityStore. Removed the five Bevy systems (refresh_complexity_ui, handle_complexity_preset_press, handle_complexity_popup_confirm, register_hideable_elements, sync_complexity_hiding), their five marker components, and HideableElementRegistry from app.rs/elements.rs; the help system in elements.rs is kept for #462. First-use popup helpers (popupPlan/confirmPopup) are ported + unit-tested but intentionally unwired — the lobby selector covers pre-launch preset choice.
## [2026-06-11] code | #462 help / device-orientation / repair button ported to pure JS | touched: gui/help-panel.js, gui/device-orientation.js, gui/repair-button.js (new), gui/console-core.js, gui/sim-state.js, gui/console.css, client.html, src/client/{app.rs, elements.rs, console_shell.rs, mod.rs}, src/client/phone_border/framing.rs, src/lib.rs, tests/client/{help-panel,device-orientation,repair-button}.test.js | reason: Slices C–E of client-WASM removal (#459). Help: 9 HelpPanel sections ported verbatim to help-panel.js; console-core.js initConsole mounts a shared "?" button + modal (click/tap to dismiss) keyed by the console name, so all 9 iframes get help with no per-file HTML. Orientation: device-orientation.js exposes window.currentOrientation() (aspect>=1.0 → landscape) updated on resize; client.html render() reads it for data-orientation. Repair: repair-button.js ports the all-busy guard + label/colour/disabled refresh and repairMessage() (DispatchRepairTeam team_idx 0 → Helm); the orphaned client.html shell stubs (renderRepair/renderRepairButtons targeting never-rendered Bevy ids) were deleted — the repair-console.html iframe owns the live hull/dispatch UI. Removed Bevy systems handle_help_button_press, handle_help_overlay_dismiss, spawn_help_*, handle_repair_button_press, refresh_repair_button, detect_orientation; components HelpButton/HelpOverlay/RepairButton/RepairButtonLabel; DeviceOrientation resource. elements.rs/console_shell.rs kept as thin stubs (file/re-export deletion is #463); transitional #[allow(unused_imports)] on the now-empty elements glob re-exports in mod.rs/lib.rs, net-zero new warnings.
## [2026-06-11] code | #463 delete Bevy client infrastructure + remove client WASM build | touched: deleted src/client/** (app, bridge, console_shell, elements, mod, phone_border/*), client-trunk.toml, tests/smoke/client-wasm-load.spec.ts; modified src/lib.rs, src/ship_view.rs, src/gui/radar.rs, Cargo.toml, Trunk.toml, client.html, package.json, .github/workflows/ci.yml, AGENTS.md; new scripts/build-client.mjs | reason: Slices F–G, completing client-WASM removal (#459). lib.rs drops the client module + client_app/client_bridge/client_elements/phone_border re-exports and gates gui to server-only. All `#[cfg(feature="client")]` sites removed (ship_view.rs ShipViewPlugin + InboundServerMessage reader; one gui/radar.rs test un-gated) so removing the `client` Cargo feature leaves no unexpected_cfgs warnings; the ungated ShipView struct/apply() stays (server_app + client_sim depend on it; client_sim/comms/complexity/client_panel kept — server_app uses client_sim radar constants). console_error_panic_hook dep removed (only deleted bridge.rs used it); wasm-bindgen/js-sys/web-sys kept (server WASM needs them). client.html stripped of the trunk rust link, WASM bridge JS and all wasm_client_* calls — pure HTML/JS, instantiates no WebAssembly. Trunk implicitly compiles the 162MB server lib into the client output even with no rust link, so client.html ships via scripts/build-client.mjs (copies index.html + gui/ + 8 asset dirs + logo.png → dist/client/, matching the old client-trunk layout the smoke suite serves) instead of trunk; CI calls the node script. Verified: cargo build [--features server] pass; cargo check --features client errors cleanly (feature removed); cargo test 2093 pass; vitest 1503 pass; smoke client-connect/server/console green.
## [2026-06-12] fix | HTML Tactical/Sensors radar targetability restored | touched: client.html, gui/console-state.js, gui/sim-state.js, tests/client/console-state.test.js | reason: After the pure-JS client migration, Tactical discarded server-projected WeaponsUpdate.blips and rebuilt radar contacts locally without the server selectable metadata; Sensors also produced blips with no selectable field, so taps were ignored. The JS bridge now preserves authoritative Tactical blips, mirrors TOML-driven radar shows/selects filters from ship_config, and the shared builder carries visibility filters plus target metadata for fallback/Sensors state. Vitest coverage added for server blip pass-through and Sensors selectable-vs-visible behavior.

## [2026-06-12] fix | Shields HTML panel hex background + segment focus toggle | touched: gui/shield-console.html, assets/shield_console/shield-hex-bg.png, scripts/build-client.mjs, wiki/concepts/weapons-plugin.md | reason: The Shields console now uses the mockup's hexagonal shield background asset and removes the separate focus button row. Shield focus is toggled directly from the diagram segments; activating the already-focused segment sends `set_shield_focus` with `facing: null` to clear focus.
## [2026-06-12] code | Multi-speaker Comms threads | touched: src/world/config.rs, src/world/server.rs, gui/comms-state.js, assets/worlds/btf_path_b.toml, docs/toml-authoring-guide.md, wiki/concepts/comms-panel.md, wiki/index.md | reason: Comms TOML now supports `speaker` on root/follow-up dialogue nodes while top-level `from` remains the channel/range/contact identity; legacy follow-up `from` remains a display-speaker alias. Before the Fire's Research Outpost/Dr. Myst thread now uses the reusable model.

## [2026-06-13] feature | Navigation custom waypoint | touched: src/core/messages.rs, src/console/navigation/mod.rs, src/server_app.rs, client.html, gui/{action-map,sim-state,console-state,radar-widget}.js, gui/navigation-console.html, wiki/entities/helm-console.md, wiki/concepts/{radar-projection,client-architecture}.md | reason: Navigation now owns one shared server-authoritative waypoint via SetNavigationWaypoint/ClearNavigationWaypoint and SimSnapshot.navigation_waypoint. The HTML Navigation chart sets it by tap and clears it by button; Helm radar renders a cyan waypoint blip and clamps it to the radar edge when outside Helm range.
