---
title: PASM Midpoint Audit
type: concept
tags: [pasm, audit, repair, helm, roadmap]
sources: [pasm/spec/PASM_IMPLEMENTATION_ROADMAP_v1.0.md, pasm/spec/PASM_RUNTIME_v1.0.md, pasm/README.md, pasm/spec/architecture/engineering-damage.yaml, pasm/spec/architecture/helm-controls.yaml, src/console/repair/server.rs, src/console/repair/visibility.rs, src/modifiers/repair_teams.rs, src/ship_plugin.rs, src/ship/helm_ai.rs, src/ship/helm_planner.rs, src/ai/core.rs, gui/console-state.js]
updated: 2026-08-09
---

> **Historical record — not current-code navigation.**
> This is a point-in-time PASM midpoint audit. Its findings were true **as of
> 2026-07-14** and are preserved as written. It is deliberately *not* maintained
> as a live page: per `wiki/SCHEMA.md` the wiki is a current-state orientation
> layer, and a dated audit re-verified on every refactor is pure churn.
>
> The exception is a finding that later became **false** rather than merely
> superseded. A reader cannot tell a stale gap from an open one, and this page
> is largely a list of gaps — so every statement that a later change falsified
> is marked inline with what superseded it. Several of the Repair and Helm gaps
> below have since been closed; read the marks, not just the bullets.
>
> Full lint against `main` on 2026-08-09 (issue #972).

Summary

This supersedes the Phase 6 midpoint framing. PASM Phases 1-9 are implemented and exercised by the tool's own test suite, which now lives in vellum rather than here; `pasm validate` is error-free and `cargo check` passes. The model is useful for declared architecture, direct repository-edge observation, migrations, design traceability, and lightweight scenarios. It is not a proof that proposed feature behaviour is already in the game.

## Phase status

- Phases 1-4: implemented. The typed core, restricted source-located YAML, architecture ownership/authority checks, implementation mappings, `validate`, and implementation queries work.
- Phase 5: implemented for repository inventory and unambiguous direct local file dependencies. It intentionally does not infer transitive or runtime/dataflow relationships.
- Phase 6: implemented as a bounded migration audit. Removal conditions and writer overlap work; arbitrary caller/dataflow discovery remains out of scope.
- Phases 7-8: implemented. Typed game design, design-to-architecture enforcement links, and truthful traceability distinguish proposed design from implemented mappings.
- Phase 9: implemented as deliberately lightweight ordered scenario validation, not a game simulation or general reachability engine.

## Runtime and packaging findings

- `uv run pasm validate` reports no errors. This line used to also cite a 45-test `uv run pytest -q tests/pasm` run; that suite belonged to the vendored tool and left with it in `ada7a172`, so there is no PASM pytest suite in this repository.
- Validation retains PASM's informational warning baseline: `uv run pasm validate` exits 0 with `Status: OK` (see `pasm/README.md`) over a drifting warning count — 39 at the time of this audit, 40 on 2026-08-09. The category mix (missing observed symbols/dependencies, undeclared observed dependencies, missing implementation mappings) drifts as the model and code evolve; a change that neither adds nor resolves one should leave the count alone rather than chase it.
- Direct-edge observation remains a conservative signal. A clean report cannot prove runtime control flow, actor identity, or information visibility.
- Some parser diagnostic text still says "Phase 0-2" although the implementation has reached Phase 9. This is documentation noise, not a semantic failure. **No longer verifiable in this repository — checked 2026-08-09:** the parser emitting that text belonged to the vendored tool, which left in `ada7a172`. The surviving mentions are prose, not diagnostics: the `foundation` entity summary in `pasm/spec/core/foundation.yaml` (where it correctly scopes that entity) and a review question in `pasm/spec/PASM_PROJECT_HANDOVER_v1.0.md`.

## Repair findings

- ~~The intended information gate is not implemented. `publish_repair_blackboard` publishes every fine-system hull entry to the Repair holder, and `buildRepairConsoleState` derives all station damage percentages before a team arrives.~~
  **Resolved by issue #737 — verified 2026-08-09.** `src/console/repair/visibility.rs` is now a host-authoritative visibility projection: each recipient is sent only what it is entitled to see, across four view states (aggregate hull for everyone, Core detail for the Engineering holder, station-owner detail for that station's holder, on-site detail for the Engineering holder). Issue #830 further changed the publisher's shape — `publish_repair_blackboard` is per-ship and writes a deliberately *unprojected* host-internal copy into that ship's `ShipSystemBlackboards` for the repair AI to read; the wire filtering is `visibility::project_repair_blackboard`. So it no longer publishes "to the Repair holder" at all.
- ~~On-site detail grouped by team and on-site subsystem priority commands are proposed PASM entities only; no production message, state, UI, or handler exists.~~
  **Superseded by issue #737 — verified 2026-08-09.** On-site detail is one of the four view states that module decides, so the claim that no production state or handler exists is false. It gates non-Core hull entries with a team *on site* to the Engineering holder.
- ~~AI-owned station level-3 repair requests are modelled as intent but have no producer in production code. Repair Backfill AI instead dispatches all idle teams directly to the single most-damaged exact system.~~
  **Superseded — verified 2026-08-09.** `CoordinationPayload::RepairRequest` has a real production producer: `ship::damage_sync` emits one on a damage-tier crossing for the owning station, and `ship::coordination_systems` delivers it into the ship's `RepairRequestQueue`. `operate_repair_ai` is queue-driven and station-granular, and since #830 it emits each assignment as an admitted `DispatchRepairTeam { target: Station(..) }` through the shared admission seam rather than dispatching directly — the fine system is then resolved by the router's `resolve_repair_target`, the same code a human dispatch runs.
- Station-owner damage is implemented more fully than PASM says: each ship-specific console derives its own station footer through `aggregateStationHull` and `ph-station-damage`. The PASM entities still say partial/suspected and map legacy generic HTML rather than the current ship-specific Web Components.
  **Partially corrected 2026-08-09:** this bullet originally opened "`SystemHullUpdate` is broadcast to all players". That is false since issue #737 — `SystemHullUpdate` is now a *per-recipient projection*, not a `Target::All` broadcast, and the reconnect path in `core::broadcast::cache_registry` rebuilds the reconnecting token's own projection rather than the whole ship's hull. The `aggregateStationHull` / `ph-station-damage` half of the claim still holds.
- Resolved on 2026-07-14: station dispatches now resolve through `ShipConfig::systems_for_station` to the most damaged repairable owned fine hull system, while retaining a coarse-system fallback. A server integration test covers command admission, travel, arrival, and HP restoration for `helm-engine-port`.
- The older wiki claim that a station dispatch repairs all damaged systems owned by that station is not true of the current implementation.

## Helm findings

- The current 2D human and AI paths are broadly captured and their focused tests pass.
- ~~The intended shared 3D desired-motion and hazard planner, vertical movement modes, vertical thrust system, capability facts, and 6DOF torpedoes are proposed PASM only, not game implementation.~~
  **Largely superseded — verified 2026-08-09.** The shared planner is production code: `helm_motion_planner` in `src/ship/helm_planner.rs`, declared in `pasm/spec/architecture/helm-controls.yaml` as `helm-motion-planner` with `status: implemented` / `confidence: confirmed`, owning `desired-motion-state` and `hazard-assessment-state`. Vertical movement modes are a real authored enum (`VerticalMovementMode` in `src/entities/config.rs`, from `[helm_capability] vertical_movement_mode`), and the vertical thrust system ships as `helm-vertical-thrust-ai-operator` / `ai_helm_vertical_thrust` (issue #744). Capability facts are covered by the capability-ownership bullet below. **6DOF torpedoes remain uncaptured** — the term appears nowhere in `src/` or `pasm/spec/`.
- ~~The agreed `desired_velocity_local` and `desired_facing_local` fields are described in prose but not declared as typed PASM fields or entities.~~
  **Superseded — verified 2026-08-09.** Both are typed Rust fields on `DesiredMotion`, read by `src/ship/helm_ai.rs` via `decode_thrust_from_velocity` / `decode_steering_from_facing`, and both are named in the declared `desired-motion-state` entity in `pasm/spec/architecture/helm-controls.yaml`.
- The agreed harsh impulse steering multiplier (default 0.1) is implemented in production config (`ImpulseConfigResource.steering_multiplier`, `[helm_capability.impulse] steering_multiplier` in TOML, issue #740).
- The agreed rule that boost is disabled during impulse is implemented in production (`tick_boost` uses `normalized_boost_drain_factor(1.0, 0.0)` when impulse active, issue #740).
- The size rule that larger ships ignore smaller ships, per-actuator hazard sensitivities, and Y-aware weapon range/collision rules are not encoded beyond broad prose.
- ~~Capability ownership is incomplete: PASM references an undeclared ship capability model, while production still represents helm through a mixture of optional TOML sections, coarse/fine systems, and fallback physics defaults.~~
  **Superseded — verified 2026-08-09.** `ship-capability-model` is now a declared PASM component (`pasm/spec/architecture/helm-controls.yaml`, `status: implemented`, `implementation: observed`) owning `vertical-movement-mode-state` and `impulse-manoeuvre-policy-state`. Production carries the matching `HelmCapabilityConfig` from the entity TOML's `[helm_capability]` block, surfaced to clients on `ShipClientConfig`.
- ~~Production collision avoidance is duplicated between `operate_helm` and `operate_lateral_thrust`, remains planar, and uses shared constants at the call sites. The TOML behaviour fields for avoidance buffer and look-ahead exist but the main helm operators currently pass constants instead of those authored values.~~
  **Superseded — verified 2026-08-09.** Neither `operate_helm` nor `operate_lateral_thrust` exists in production any more; avoidance was consolidated behind the shared planner. `src/ship/helm_ai.rs` threads the authored `[behaviour] avoidance_buffer` and `avoidance_look_ahead_secs` through to the pure decision functions in `src/ai/core.rs`, falling back to the `crate::ai::AVOIDANCE_*` constants only when a template omits the section — the sanctioned parse-default shape, not a hardcoded value. Regression tests such as `lateral_thrust_ai_honours_toml_authored_avoidance_buffer` pin the authored path.
- The Phase 6 migration check intentionally relies on pending symbol-removal conditions rather than file-level caller attribution: multiple PASM entities share `src/ship_plugin.rs`, so the latter produced false callers without identifying a real dependency.

## Recommended order

This was the order recommended *at the audit*. Most of it has since been done —
see the per-item notes, verified 2026-08-09.

1. Make the current PASM model self-validating: fix confidence values, declare the ship capability entity, include the migration package, correct metadata, and add a runnable Python environment/CI check. *(Largely done: `ship-capability-model` is declared, and CI runs `pasm validate` plus `pasm scan` / `pasm traceability`.)*
2. Add an integration test and fix station-target repair routing before building further PASM phases. *(Done on 2026-07-14 — see the Repair findings above; dispatch resolves through `ShipConfig::systems_for_station`.)*
3. Bring repair mappings/statuses up to date with ship-specific console Web Components and explicitly mark the visibility, request, and priority mechanics as unimplemented intent. *(Overtaken by events: visibility and repair requests are implemented, not intent — issues #737, #682/#785/#830. Only the mappings half still applies.)*
4. Complete Phase 5 as designed, or rename it honestly as declared-file conformance and defer the real observed repository model. Resolved on 2026-07-14 with the repository inventory and direct-edge conformance pass.
5. Encode the missing helm decisions explicitly before Phase 7, especially impulse steering, boost/impulse exclusion, exact desired-motion fields, size precedence, and per-actuator hazard tuning. *(Partly done: the desired-motion fields and the capability model are declared. Size precedence and per-actuator hazard tuning remain uncaptured.)*
