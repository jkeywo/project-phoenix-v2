---
title: PASM Runtime
type: concept
tags: [pasm, python, validation, architecture, implementation, game-design]
sources: [pasm/spec/PASM_RUNTIME_v1.0.md, pasm/spec/PASM_IMPLEMENTATION_ROADMAP_v1.0.md, README.md, pyproject.toml, pasm/core/model.py, pasm/core/parser.py, pasm/core/validation.py, pasm/cli/main.py, pasm/domains/game_design/model.py, pasm/domains/game_design/validation.py, pasm/spec/design/engineering-diagnosis.yaml, pasm/implementation/observation.py, pasm/scanners/javascript.py, pasm/scanners/rust.py, pasm/scanners/html.py]
updated: 2026-07-14
---

Summary

PASM is now seeded as a small Python runtime in `pasm/` that covers Phase 0 through Phase 12: package layout, typed core entities, restricted YAML parsing, typed architecture, implementation, migration, game-design sections, bounded fact reachability, provider-neutral semantic audit records, task contexts, CI validation, cross-file reference validation, deterministic findings, repository-wide observation, cross-domain traceability, unit fixtures, and CLI tools.

## Current scope

Implemented in this slice:

- package skeleton matching the PASM runtime layout (`pasm/`, `pasm/core/`, `pasm/cli/`, plus placeholder later-phase packages);
- typed core model: `EntityId`, `SourceLocation`, lifecycle `Status`, `Confidence`, `Reference`, `ExceptionSpec`, `EvidenceItem`, `SpecEntity`, `Finding` ([pasm/core/model.py](/C:/Coding/project-phoenix-v2/pasm/core/model.py), [pasm/core/findings.py](/C:/Coding/project-phoenix-v2/pasm/core/findings.py));
- typed architecture model for ownership, dependencies, authority, runtimes, and messages ([pasm/architecture/model.py](/C:/Coding/project-phoenix-v2/pasm/architecture/model.py));
- typed implementation model for declared code paths, symbols, message names, tests, and migration path buckets ([pasm/implementation/model.py](/C:/Coding/project-phoenix-v2/pasm/implementation/model.py));
- typed migration model for legacy entities, target entities, approved legacy callers, temporary adapters, legacy/target symbols, and fixed removal conditions. Migration entities use `migration_plan` as their nested YAML section, avoiding a duplicate `migration` key ([pasm/migration/model.py](/C:/Coding/project-phoenix-v2/pasm/migration/model.py));
- typed game-design model for roles, verbs, protected decisions, information visibility, mechanics, coordination, resources, failures, tuning, and playtest claims. It validates design-internal references, verb/decision owners, protected-decision bypass policies, reveal conditions, resource source/sink declarations, and non-terminal recovery paths ([pasm/domains/game_design/model.py](/C:/Coding/project-phoenix-v2/pasm/domains/game_design/model.py), [pasm/domains/game_design/validation.py](/C:/Coding/project-phoenix-v2/pasm/domains/game_design/validation.py));
- Phase 8 cross-domain validation and traceability: design declarations link to architecture and enforcement entities; verbs share a declared interface path with their owner role; restricted information and protected decisions name enforcement boundaries; linked shipped architecture requires an implementation mapping. `pasm traceability` reports the resulting design-to-architecture-to-implementation rows ([pasm/integration/validation.py](/C:/Coding/project-phoenix-v2/pasm/integration/validation.py), [pasm/integration/traceability.py](/C:/Coding/project-phoenix-v2/pasm/integration/traceability.py));
- observed implementation model for Git-revision-linked Cargo packages plus Rust/JS/TS/HTML files, source-located imports, resolved local file edges, and declared-file compatibility checks ([pasm/implementation/observation.py](/C:/Coding/project-phoenix-v2/pasm/implementation/observation.py));
- restricted YAML parser using composed PyYAML nodes so source line/column data survives and architecture fields are schema-checked ([pasm/core/parser.py](/C:/Coding/project-phoenix-v2/pasm/core/parser.py));
- validation pipeline for file discovery, duplicate ids, unknown fields, malformed YAML, unresolved references, temporary exceptions missing removal conditions, authoritative-state ownership, forbidden dependencies, trust-boundary message validation, declared-path existence, empty implementation mappings, missing implementation coverage for shipped entities, stale declared symbols/messages, direct declared-versus-observed dependency drift, and Phase 6 migration checks for undeclared legacy callers, overlapping writers, target-side legacy residue, and fixed removal conditions ([pasm/core/validation.py](/C:/Coding/project-phoenix-v2/pasm/core/validation.py), [pasm/core/references.py](/C:/Coding/project-phoenix-v2/pasm/core/references.py), [pasm/architecture/validation.py](/C:/Coding/project-phoenix-v2/pasm/architecture/validation.py), [pasm/implementation/validation.py](/C:/Coding/project-phoenix-v2/pasm/implementation/validation.py), [pasm/migration/validation.py](/C:/Coding/project-phoenix-v2/pasm/migration/validation.py));
- `pasm validate`, `pasm query entity <id>`, `pasm query implementation <id>`, `pasm query migration <id>`, and `pasm scan [spec_root] --entity <id>` CLI commands with text and JSON output ([pasm/cli/main.py](/C:/Coding/project-phoenix-v2/pasm/cli/main.py));
- `pasm traceability [spec_root]` emits the design-to-architecture-to-implementation report as text or JSON ([pasm/cli/main.py](/C:/Coding/project-phoenix-v2/pasm/cli/main.py));
- `pasm scenario <file>` validates the authored ordered walkthrough and, where supplied, computes finite monotonic reachability from `initial_facts`, action `requires_facts`, and declared produced facts. It deliberately does not claim arbitrary runtime reachability ([pasm/domains/game_design/scenarios.py](/C:/Coding/project-phoenix-v2/pasm/domains/game_design/scenarios.py));
- `pasm audit bundle <entity>` creates a provider-neutral, source-sliced semantic-audit bundle with a deterministic fingerprint. `pasm audit report <json> --bundle <bundle.json> --persist-dir <dir>` validates, deduplicates, and records source-linked architecture, migration, or design-alignment findings against that exact revision and entity set, without replacing deterministic validation ([pasm/audit.py](/C:/Coding/project-phoenix-v2/pasm/audit.py));
- `pasm context --entity <id> [--depth N]` traverses explicit architecture links into a bounded task-context bundle, includes mapped paths plus migration/evidence declarations, and reports linked entities omitted by the depth bound ([pasm/context.py](/C:/Coding/project-phoenix-v2/pasm/context.py));
- CI runs the PASM test suite and deterministic validation, then uploads revision-linked scan and traceability JSON as a `pasm-reports` artifact. Semantic audit reports remain advisory ([.github/workflows/ci.yml](/C:/Coding/project-phoenix-v2/.github/workflows/ci.yml)).
- lightweight Rust, JavaScript/TypeScript, and HTML scanners that recognise top-level/exported declarations, imports, Rust module declarations, and HTML script sources ([pasm/scanners/rust.py](/C:/Coding/project-phoenix-v2/pasm/scanners/rust.py), [pasm/scanners/javascript.py](/C:/Coding/project-phoenix-v2/pasm/scanners/javascript.py), [pasm/scanners/html.py](/C:/Coding/project-phoenix-v2/pasm/scanners/html.py));
- unit fixtures under `tests/pasm/fixtures/`.
- a real authored Phoenix architecture slice at [pasm/spec/architecture/engineering-damage.yaml](/C:/Coding/project-phoenix-v2/pasm/spec/architecture/engineering-damage.yaml).
- an authored Engineering diagnosis and repair design slice at [pasm/spec/design/engineering-diagnosis.yaml](/C:/Coding/project-phoenix-v2/pasm/spec/design/engineering-diagnosis.yaml).
- authored Helm and Red Alert design slices at [pasm/spec/design/helm-controls.yaml](/C:/Coding/project-phoenix-v2/pasm/spec/design/helm-controls.yaml) and [pasm/spec/design/red-alert.yaml](/C:/Coding/project-phoenix-v2/pasm/spec/design/red-alert.yaml).

## Deliberate boundaries

Not implemented yet:

- simulations, automated LLM execution, and general semantic/dataflow analysis.

## Loader decisions

- The runtime package and its original design docs both live in `pasm/`; authored YAML and the v1.0 markdown documents share `pasm/spec/`.
- The parser rejects unknown fields, anchors, custom tags, and non-string scalar types except explicit booleans. That keeps Phase 0-2 deterministic and easy to test.
- The `architecture`, `implementation`, `migration_plan`, and `game_design` sections are typed. Game-design references use semantic entity IDs where a relationship is meaningful; descriptive conditions and effects remain strings to avoid inventing a scenario language before Phase 9.
- The Phase 7 loader accepts the v1.0 writing-guide aliases `player_role`, `action`, `information_set`, `failure_state`, `game_design.player_role`, and singular `reveal_condition`, normalising them into the typed role, decision, information, failure, owner-role, and reveal-condition model.
- Cross-domain links are authored semantic entity IDs rather than file paths. A linked `implemented` architecture entity must carry its existing implementation mapping; a `proposed` target remains in the report as design-only until its implementation phase starts.
- Cross-domain links must target architecture entities. Traceability preserves the declared mapping status instead of treating any non-empty path list as proof of implementation; linked `partially-implemented` entities also require a mapping. PASM currently verifies an authoritative boundary for a protected decision, but does not claim to infer runtime automation or bypass behaviour from prose-only `must_not_be` declarations.
- Phase 5 observes the repository independently of declared mappings, but only compares simple local syntactic edges where both files have unambiguous PASM ownership. It does not infer runtime semantics, package-import ownership, shared-file attribution, or transitive dependency meaning.
- Migration entities use a typed `migration_plan` section. The fixed-scope runtime supports only a small predicate set and declared-file legacy-caller checks, which keeps Phase 6 deterministic and easy to test.
- PASM uses the repository-managed `uv` environment: `uv sync --group dev` provisions PyYAML and pytest, then `uv run pytest -q` runs the PASM suite. `uv.lock` records the resolved test environment.
- `validate_spec_root()` and every CLI command accept an optional workspace root. Production continues to infer it from `pasm/spec`; nested fixture or external spec roots should pass `--workspace-root` explicitly.
- For observation, `symbols` means top-level Rust items, top-level/exported JavaScript/TypeScript declarations, and HTML `id` attributes. Imports and local dependency edges retain their own source locations. JSON payload keys such as `core_systems` are represented through their builder functions rather than treated as implementation symbols.
- The default validation target is `pasm/spec/`, which now contains a tiny seed model so the CLI has a live root from day one.
- The first real Phoenix vertical slice is the existing repair-and-damage path, because the repo does not currently expose a literal "diagnose fault" mechanic. PASM records the truthful shipped surface rather than an aspirational one.
- The first representative migration slice is the helm move from direct-write AI helpers toward a shared motion planner. PASM records that migration as intended architecture, while its removal conditions remain intentionally pending in the current code.
- Scenario reachability is a fixed-point computation over additive, authored facts only. It is intentionally unable to model negation, timers, branching values, arbitrary calls, or unmodelled runtime state; those remain outside PASM's deterministic scope.
- Semantic-audit records are evidence, not a formal proof. A persisted report must name one of the explicit audit kinds (`architecture`, `migration`, or `design-alignment`), the audited entity set, its repository revision, and the SHA-256 fingerprint of the supplied PASM bundle. PASM retains that exact bundle beside the record, rejects mismatched bundle/revision/entity metadata, and refuses to overwrite conflicting evidence.

## Open questions

- Whether the project should standardise on Python 3.11 or 3.12 for local PASM work.
- When the first real Phoenix vertical slice should move from fixtures into authored PASM spec files.
