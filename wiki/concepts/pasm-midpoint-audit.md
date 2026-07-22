---
title: PASM Midpoint Audit
type: concept
tags: [pasm, audit, repair, helm, roadmap]
sources: [pasm/spec/PASM_IMPLEMENTATION_ROADMAP_v1.0.md, pasm/spec/PASM_RUNTIME_v1.0.md, pasm/README.md, pasm/spec/architecture/engineering-damage.yaml, pasm/spec/architecture/helm-controls.yaml, pasm/core/validation.py, pasm/implementation/observation.py, pasm/migration/validation.py, src/console/repair/server.rs, src/modifiers/repair_teams.rs, src/ship_plugin.rs, src/ai/core.rs, gui/console-state.js]
updated: 2026-07-14
---

Summary

This supersedes the Phase 6 midpoint framing. PASM Phases 1-9 are implemented and exercised by 45 Python tests; `pasm validate` is error-free and `cargo check` passes. The model is useful for declared architecture, direct repository-edge observation, migrations, design traceability, and lightweight scenarios. It is not a proof that proposed feature behaviour is already in the game.

## Phase status

- Phases 1-4: implemented. The typed core, restricted source-located YAML, architecture ownership/authority checks, implementation mappings, `validate`, and implementation queries work.
- Phase 5: implemented for repository inventory and unambiguous direct local file dependencies. It intentionally does not infer transitive or runtime/dataflow relationships.
- Phase 6: implemented as a bounded migration audit. Removal conditions and writer overlap work; arbitrary caller/dataflow discovery remains out of scope.
- Phases 7-8: implemented. Typed game design, design-to-architecture enforcement links, and truthful traceability distinguish proposed design from implemented mappings.
- Phase 9: implemented as deliberately lightweight ordered scenario validation, not a game simulation or general reachability engine.

## Runtime and packaging findings

- `uv run pytest -q tests/pasm` passes 45 tests and `uv run pasm validate` reports no errors.
- Validation retains ten warnings: two missing mappings for world state entities, four declared Repair/Helm dependencies without observed direct file edges, and four intentional pending Helm migration conditions/overlapping writers.
- Direct-edge observation remains a conservative signal. A clean report cannot prove runtime control flow, actor identity, or information visibility.
- Some parser diagnostic text still says "Phase 0-2" although the implementation has reached Phase 9. This is documentation noise, not a semantic failure.

## Repair findings

- The intended information gate is not implemented. `publish_repair_blackboard` publishes every fine-system hull entry to the Repair holder, and `buildRepairConsoleState` derives all station damage percentages before a team arrives.
- On-site detail grouped by team and on-site subsystem priority commands are proposed PASM entities only; no production message, state, UI, or handler exists.
- AI-owned station level-3 repair requests are modelled as intent but have no producer in production code. Repair Backfill AI instead dispatches all idle teams directly to the single most-damaged exact system.
- Station-owner damage is implemented more fully than PASM says: `SystemHullUpdate` is broadcast to all players and each ship-specific console derives its own station footer through `aggregateStationHull` and `ph-station-damage`. The PASM entities still say partial/suspected and map legacy generic HTML rather than the current ship-specific Web Components.
- Resolved on 2026-07-14: station dispatches now resolve through `ShipConfig::systems_for_station` to the most damaged repairable owned fine hull system, while retaining a coarse-system fallback. A server integration test covers command admission, travel, arrival, and HP restoration for `helm-engine-port`.
- The older wiki claim that a station dispatch repairs all damaged systems owned by that station is not true of the current implementation.

## Helm findings

- The current 2D human and AI paths are broadly captured and their focused tests pass.
- The intended shared 3D desired-motion and hazard planner, vertical movement modes, vertical thrust system, capability facts, and 6DOF torpedoes are proposed PASM only, not game implementation.
- The agreed `desired_velocity_local` and `desired_facing_local` fields are described in prose but not declared as typed PASM fields or entities.
- The agreed harsh impulse steering multiplier (default 0.1) is implemented in production config (`ImpulseConfigResource.steering_multiplier`, `[helm_capability.impulse] steering_multiplier` in TOML, issue #740).
- The agreed rule that boost is disabled during impulse is implemented in production (`tick_boost` uses `normalized_boost_drain_factor(1.0, 0.0)` when impulse active, issue #740).
- The size rule that larger ships ignore smaller ships, per-actuator hazard sensitivities, and Y-aware weapon range/collision rules are not encoded beyond broad prose.
- Capability ownership is incomplete: PASM references an undeclared ship capability model, while production still represents helm through a mixture of optional TOML sections, coarse/fine systems, and fallback physics defaults.
- Production collision avoidance is duplicated between `operate_helm` and `operate_lateral_thrust`, remains planar, and uses shared constants at the call sites. The TOML behaviour fields for avoidance buffer and look-ahead exist but the main helm operators currently pass constants instead of those authored values. The Phase 6 migration check intentionally relies on pending symbol-removal conditions rather than file-level caller attribution: multiple PASM entities share `src/ship_plugin.rs`, so the latter produced false callers without identifying a real dependency.

## Recommended order

1. Make the current PASM model self-validating: fix confidence values, declare the ship capability entity, include the migration package, correct metadata, and add a runnable Python environment/CI check.
2. Add an integration test and fix station-target repair routing before building further PASM phases.
3. Bring repair mappings/statuses up to date with ship-specific console Web Components and explicitly mark the visibility, request, and priority mechanics as unimplemented intent.
4. Complete Phase 5 as designed, or rename it honestly as declared-file conformance and defer the real observed repository model. Resolved on 2026-07-14 with the repository inventory and direct-edge conformance pass.
5. Encode the missing helm decisions explicitly before Phase 7, especially impulse steering, boost/impulse exclusion, exact desired-motion fields, size precedence, and per-actuator hazard tuning.
