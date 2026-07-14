---
title: PASM Observed Repository Model
type: roadmap
tags: [pasm, architecture, conformance, tooling]
sources: [pasm/spec/PASM_IMPLEMENTATION_ROADMAP_v1.0.md, wiki/concepts/pasm-midpoint-audit.md, pasm/implementation/observation.py]
updated: 2026-07-14
---

# PASM Observed Repository Model

## Status

Implemented on 2026-07-14. This replaces the earlier declared-file-only observation pass with the Phase 5 observed repository model defined by the PASM roadmap.

## Scope

- Record the current repository revision in every generated inventory.
- Discover Cargo workspace/package structure and Rust module/import relationships.
- Scan JavaScript, TypeScript, and HTML files into a repository inventory.
- Produce stable JSON for the observed files, symbols, modules, imports, and dependency edges.
- Compare declared implementation mappings and architecture dependencies with the observed inventory.
- Add focused fixtures that demonstrate at least one real or fixture conformance violation.

## Boundaries

- Keep scanners deterministic and lightweight; they should not attempt full language semantics.
- Preserve the existing declared-file checks as a compatibility layer where they remain useful.
- Do not begin Phase 7 game-design semantics, simulation, LLM integration, or custom grammar work in this task.

## Acceptance Criteria

- `pasm scan` generates a revision-linked repository inventory as JSON, even when validation reports pending findings.
- Rust, JS/TS, and HTML inventory records retain paths and source locations where available.
- PASM reports direct declared-versus-observed dependency drift deterministically when file ownership is unambiguous.
- Tests include known conformance violations and the existing authored model remains runnable.

## Related Work

- [PASM Midpoint Audit](../concepts/pasm-midpoint-audit.md)
- [PASM Runtime](../concepts/pasm-runtime.md)
