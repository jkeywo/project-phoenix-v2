---
title: PASM Runtime
type: concept
tags: [pasm, python, validation, architecture, implementation]
sources: [pasm/spec/PASM_RUNTIME_v1.0.md, pasm/spec/PASM_IMPLEMENTATION_ROADMAP_v1.0.md, README.md, pyproject.toml, pasm/core/model.py, pasm/core/parser.py, pasm/core/validation.py, pasm/cli/main.py, pasm/implementation/observation.py, pasm/scanners/javascript.py, pasm/scanners/rust.py, pasm/scanners/html.py]
updated: 2026-07-14
---

Summary

PASM is now seeded as a small Python runtime in `pasm/` that covers Phase 0 through Phase 6: package layout, typed core entities, restricted YAML parsing, typed architecture and implementation sections, cross-file reference validation, deterministic findings, repository-wide observation, typed migration semantics, unit fixtures, `pasm validate`, `pasm query entity`, `pasm query implementation`, `pasm query migration`, and `pasm scan`.

## Current scope

Implemented in this slice:

- package skeleton matching the PASM runtime layout (`pasm/`, `pasm/core/`, `pasm/cli/`, plus placeholder later-phase packages);
- typed core model: `EntityId`, `SourceLocation`, lifecycle `Status`, `Confidence`, `Reference`, `ExceptionSpec`, `EvidenceItem`, `SpecEntity`, `Finding` ([pasm/core/model.py](/C:/Coding/project-phoenix-v2/pasm/core/model.py), [pasm/core/findings.py](/C:/Coding/project-phoenix-v2/pasm/core/findings.py));
- typed architecture model for ownership, dependencies, authority, runtimes, and messages ([pasm/architecture/model.py](/C:/Coding/project-phoenix-v2/pasm/architecture/model.py));
- typed implementation model for declared code paths, symbols, message names, tests, and migration path buckets ([pasm/implementation/model.py](/C:/Coding/project-phoenix-v2/pasm/implementation/model.py));
- typed migration model for legacy entities, target entities, approved legacy callers, temporary adapters, legacy/target symbols, and fixed removal conditions. Migration entities use `migration_plan` as their nested YAML section, avoiding a duplicate `migration` key ([pasm/migration/model.py](/C:/Coding/project-phoenix-v2/pasm/migration/model.py));
- observed implementation model for Git-revision-linked Cargo packages plus Rust/JS/TS/HTML files, source-located imports, resolved local file edges, and declared-file compatibility checks ([pasm/implementation/observation.py](/C:/Coding/project-phoenix-v2/pasm/implementation/observation.py));
- restricted YAML parser using composed PyYAML nodes so source line/column data survives and architecture fields are schema-checked ([pasm/core/parser.py](/C:/Coding/project-phoenix-v2/pasm/core/parser.py));
- validation pipeline for file discovery, duplicate ids, unknown fields, malformed YAML, unresolved references, temporary exceptions missing removal conditions, authoritative-state ownership, forbidden dependencies, trust-boundary message validation, declared-path existence, empty implementation mappings, missing implementation coverage for shipped entities, stale declared symbols/messages, direct declared-versus-observed dependency drift, and Phase 6 migration checks for undeclared legacy callers, overlapping writers, target-side legacy residue, and fixed removal conditions ([pasm/core/validation.py](/C:/Coding/project-phoenix-v2/pasm/core/validation.py), [pasm/core/references.py](/C:/Coding/project-phoenix-v2/pasm/core/references.py), [pasm/architecture/validation.py](/C:/Coding/project-phoenix-v2/pasm/architecture/validation.py), [pasm/implementation/validation.py](/C:/Coding/project-phoenix-v2/pasm/implementation/validation.py), [pasm/migration/validation.py](/C:/Coding/project-phoenix-v2/pasm/migration/validation.py));
- `pasm validate`, `pasm query entity <id>`, `pasm query implementation <id>`, `pasm query migration <id>`, and `pasm scan [spec_root] --entity <id>` CLI commands with text and JSON output ([pasm/cli/main.py](/C:/Coding/project-phoenix-v2/pasm/cli/main.py));
- lightweight Rust, JavaScript/TypeScript, and HTML scanners that recognise top-level/exported declarations, imports, Rust module declarations, and HTML script sources ([pasm/scanners/rust.py](/C:/Coding/project-phoenix-v2/pasm/scanners/rust.py), [pasm/scanners/javascript.py](/C:/Coding/project-phoenix-v2/pasm/scanners/javascript.py), [pasm/scanners/html.py](/C:/Coding/project-phoenix-v2/pasm/scanners/html.py));
- unit fixtures under `tests/pasm/fixtures/`.
- a real authored Phoenix architecture slice at [pasm/spec/architecture/engineering-damage.yaml](/C:/Coding/project-phoenix-v2/pasm/spec/architecture/engineering-damage.yaml).

## Deliberate boundaries

Not implemented yet:

- game-design semantics beyond preserving raw `game_design` mappings;
- simulations, queries beyond validation, and any LLM integration.

## Loader decisions

- The runtime package and its original design docs both live in `pasm/`; authored YAML and the v1.0 markdown documents share `pasm/spec/`.
- The parser rejects unknown fields, anchors, custom tags, and non-string scalar types except explicit booleans. That keeps Phase 0-2 deterministic and easy to test.
- The `architecture` section is now typed and validated, while `game_design` and `implementation` are still accepted as raw mappings so later phases can add semantics without blocking authored work now.
- Phase 5 observes the repository independently of declared mappings, but only compares simple local syntactic edges where both files have unambiguous PASM ownership. It does not infer runtime semantics, package-import ownership, shared-file attribution, or transitive dependency meaning.
- Migration entities use a typed `migration_plan` section. The fixed-scope runtime supports only a small predicate set and declared-file legacy-caller checks, which keeps Phase 6 deterministic and easy to test.
- PASM uses the repository-managed `uv` environment: `uv sync --group dev` provisions PyYAML and pytest, then `uv run pytest -q` runs the PASM suite. `uv.lock` records the resolved test environment.
- `validate_spec_root()` and every CLI command accept an optional workspace root. Production continues to infer it from `pasm/spec`; nested fixture or external spec roots should pass `--workspace-root` explicitly.
- For observation, `symbols` means top-level Rust items, top-level/exported JavaScript/TypeScript declarations, and HTML `id` attributes. Imports and local dependency edges retain their own source locations. JSON payload keys such as `core_systems` are represented through their builder functions rather than treated as implementation symbols.
- The default validation target is `pasm/spec/`, which now contains a tiny seed model so the CLI has a live root from day one.
- The first real Phoenix vertical slice is the existing repair-and-damage path, because the repo does not currently expose a literal "diagnose fault" mechanic. PASM records the truthful shipped surface rather than an aspirational one.
- The first representative migration slice is the helm move from direct-write AI helpers toward a shared motion planner. PASM records that migration as intended architecture, while its removal conditions remain intentionally pending in the current code.

## Open questions

- Whether the project should standardise on Python 3.11 or 3.12 for local PASM work.
- When the first real Phoenix vertical slice should move from fixtures into authored PASM spec files.
