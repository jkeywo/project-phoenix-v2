---
title: PASM Midpoint Audit
type: concept
tags: [pasm, audit, repair, helm, roadmap]
sources: [pasm/spec/PASM_IMPLEMENTATION_ROADMAP_v1.0.md, pasm/spec/PASM_RUNTIME_v1.0.md, pasm/README.md, pasm/spec/architecture/engineering-damage.yaml, pasm/spec/architecture/helm-controls.yaml, pasm/core/validation.py, pasm/implementation/observation.py, pasm/migration/validation.py, src/console/repair/server.rs, src/modifiers/repair_teams.rs, src/ship_plugin.rs, src/ai/core.rs, gui/console-state.js]
updated: 2026-07-14
---

Summary

At the Phase 6 midpoint, PASM has a useful typed foundation but does not yet satisfy every Phase 0-6 roadmap exit criterion. Phases 1-5 are substantially present, including a repository-wide observed model with direct-edge conformance, while Phase 6 cannot reliably discover undeclared callers outside already-modelled entities. The authored repair and helm slices also retain several design decisions that are only prose or are absent.

## Phase status

- Phase 0 is partial: subsystem fixtures and package scaffolding exist, but the roadmap's ten to fifteen audit questions were never recorded.
- Phase 1 is substantially implemented: typed IDs, locations, lifecycle, confidence, references, exceptions, evidence, and findings exist. Duplicate IDs are incorrectly checked by `(kind, id)`, despite the documented global uniqueness rule.
- Phase 2 is substantially implemented: restricted YAML, source locations, unknown-field rejection, cross-file loading, validation CLI, JSON, and exit codes exist. The live authored helm file does not parse because it uses unsupported confidence values.
- Phase 3 is substantially implemented for authored intent: architecture types and ownership, dependency, authority, trust-boundary, and message checks exist. There is no dedicated architecture query, transitive dependency checking, or observed-code enforcement.
- Phase 4 is substantially implemented: typed mappings, path existence, basic coverage, and implementation query exist. Test declarations are strings only and are not verified or executed by PASM.
- Resolved on 2026-07-14: Phase 5 records repository revision, Cargo package metadata, Rust/JS/TS/HTML files, source-located imports, and resolved local file edges. It compares direct observed entity edges against declared and forbidden architecture dependencies only when file ownership is unambiguous; unsupported semantic or transitive relationships remain explicit scope boundaries.
- Phase 6 is partial: the typed model, representative helm migration, removal predicates, overlap heuristic, and fixture checks exist. Caller discovery only searches implementation files of PASM entities, `test-passes` is hard-coded unsatisfied, and the target-symbol/removal model is much narrower than the migration semantics in the design docs.

## Runtime and packaging findings

- `pasm/spec/architecture/helm-controls.yaml` uses `confidence: intended` and `confidence: mixed`, but `pasm/core/model.py` only accepts confirmed, inferred, provisional, disputed, and unknown. These entities fail parsing and cause follow-on unresolved-reference findings.
- `vertical-movement-mode-state` names `ship-capability-model` as its owner, but no such entity is declared.
- `pyproject.toml` omits `pasm.migration` from the explicit package list, so a built installation can omit code imported by the core model and CLI.
- Package metadata still describes Phase 0-2 and `pasm/README.md` opens with Phase 0-4, while the wiki claims Phase 0-6.
- `validate_spec_root()` derives the workspace as exactly two parents above the chosen spec root. This works for `pasm/spec` but makes custom spec roots and fixtures depend on directory depth rather than explicit configuration.
- PASM tests could not be run in the current desktop runtime because Python, PyYAML, and pytest are unavailable. Syntax compilation had passed previously; this audit verified the game with `cargo check`, 56 repair-focused Rust tests, 81 helm-focused Rust tests, and 379 focused client tests.

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
- The agreed harsh impulse steering multiplier (default 0.1) is absent from PASM and production config. Production still forces steering and lateral input to zero during impulse.
- The agreed rule that boost is disabled during impulse is absent from PASM and production. Boost can remain active, modifies movement, and drains during impulse.
- The size rule that larger ships ignore smaller ships, per-actuator hazard sensitivities, and Y-aware weapon range/collision rules are not encoded beyond broad prose.
- Capability ownership is incomplete: PASM references an undeclared ship capability model, while production still represents helm through a mixture of optional TOML sections, coarse/fine systems, and fallback physics defaults.
- Production collision avoidance is duplicated between `operate_helm` and `operate_lateral_thrust`, remains planar, and uses shared constants at the call sites. The TOML behaviour fields for avoidance buffer and look-ahead exist but the main helm operators currently pass constants instead of those authored values. The Phase 6 migration check intentionally relies on pending symbol-removal conditions rather than file-level caller attribution: multiple PASM entities share `src/ship_plugin.rs`, so the latter produced false callers without identifying a real dependency.

## Recommended order

1. Make the current PASM model self-validating: fix confidence values, declare the ship capability entity, include the migration package, correct metadata, and add a runnable Python environment/CI check.
2. Add an integration test and fix station-target repair routing before building further PASM phases.
3. Bring repair mappings/statuses up to date with ship-specific console Web Components and explicitly mark the visibility, request, and priority mechanics as unimplemented intent.
4. Complete Phase 5 as designed, or rename it honestly as declared-file conformance and defer the real observed repository model. Resolved on 2026-07-14 with the repository inventory and direct-edge conformance pass.
5. Encode the missing helm decisions explicitly before Phase 7, especially impulse steering, boost/impulse exclusion, exact desired-motion fields, size precedence, and per-actuator hazard tuning.
