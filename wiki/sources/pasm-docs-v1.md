---
title: PASM v1.0 Documentation Set
type: source
tags: [pasm, architecture, validation, tooling]
sources: [pasm/spec/README_PASM_v1.0.md, pasm/spec/PASM_CORE_CONCEPTS_v1.0.md, pasm/spec/WRITING_PASM_v1.0.md, pasm/spec/PASM_RUNTIME_v1.0.md, pasm/spec/PASM_IMPLEMENTATION_ROADMAP_v1.0.md]
source_path: pasm/spec/
updated: 2026-07-14
---

Summary

The `pasm/spec/` markdown set defines PASM as a Project Phoenix-specific executable model that combines authored intent, typed Python semantics, repository observation, validation, and later audit workflows. For the first implementation slice it explicitly recommends starting with typed core entities, restricted YAML loading, references, source locations, structured findings, and a `pasm validate` CLI.

## Status

The docs define the target design and phased roadmap. They do not ship executable code themselves.

## Problem

Project Phoenix needs a machine-checkable representation of architectural intent, migration allowances, and evidence so drift can be detected even when the code still compiles and tests pass.

## Solution

PASM is split into:

- human-authored model files using a restricted YAML subset;
- a Python runtime that parses, validates, resolves references, and later compares intent with observed repository facts;
- structured findings with severity, confidence, rule ids, evidence, and source locations;
- a phased implementation plan that starts small before adding scanners, migrations, game-design semantics, and LLM-assisted audits.

## Key decisions

- Start architecture-first; do not build a universal ontology.
- Keep intent separate from observed implementation.
- Prefer deterministic validation before any LLM reasoning.
- Reject unknown fields and unsupported YAML constructs instead of silently accepting them.
- Begin with Phase 0-2 foundations before repository scanning or semantic audits.

## Open user stories

- Encode one real Phoenix vertical slice after the core runtime settles.
- Add implementation mappings, repository observation, and migration checks in later phases.
- Keep the first observed-code comparison narrow enough to preserve source locations and deterministic tests before attempting a whole-repo inventory.
- Add game-design and cross-domain validation only after the parser and findings pipeline are stable.

## Cross-references

- [PASM Runtime](../concepts/pasm-runtime.md)
- [README.md](./repo-readme.md)
