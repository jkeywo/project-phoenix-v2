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
