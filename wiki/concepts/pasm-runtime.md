---
title: PASM Runtime
type: concept
tags: [pasm, validation, architecture, game-design]
sources: [pasm/README.md, pyproject.toml, .github/workflows/ci.yml, pasm/spec/design/engineering-diagnosis.yaml, pasm/spec/architecture/engineering-damage.yaml]
updated: 2026-08-27
---

# PASM Runtime

PASM is the repository's executable architecture and game-design model. Phoenix
owns the YAML under `pasm/spec/`; the `pasm` command itself is supplied by the
Vellum dependency pinned in `pyproject.toml`. The tool's Python package and unit
tests do not live in this repository.

## Sources of truth

- Code and authored assets describe shipped runtime behavior.
- `pasm/spec/architecture/` maps ownership, authority, messages, dependencies,
  implementation paths, and migrations.
- `pasm/spec/design/` declares gameplay intent, player verbs, information,
  protected decisions, failures, tuning bounds, and scenario claims.
- GitHub issues hold plans and backlog. PASM entities must distinguish shipped,
  partial, and proposed mappings rather than presenting plans as current code.

Cross-domain references let PASM trace a design declaration through its
architecture boundary to implementation evidence. They use semantic entity ids;
implementation mappings use repository paths and symbols that observation can
check against the checkout.

## Local commands

`uv run pasm validate` checks schema, references, authority and dependency
rules, declared implementation paths/symbols, migrations, and linked design
constraints. `uv run pasm scan` compares declared implementation structure with
repository observation. `uv run pasm traceability` produces the
design-to-architecture-to-implementation report.

The CI `pasm` job runs validation and a gating scan through Vellum's composite
action, then uploads scan and traceability JSON. A successful validation may
still print informational warnings; errors determine the exit status.

## Scenario design workflow

Scenario design work begins with `uv run pasm design digest`, which combines
the declared design slice with live authored world values. Tunable changes
should use `pasm design writeback` where supported; it is bounded by the model
and hash-guards the source world. Structural script changes remain direct world
edits and require the three PASM checks above.

PASM scenario reachability is deliberately bounded: it reasons over declared
facts and authored transitions, not arbitrary Rust, Rhai, timers, or general
dataflow. A green model therefore proves model integrity and declared
traceability, not full runtime correctness.

## Related

- [Architecture](./architecture.md)
- [Testing Strategy](./testing-strategy.md)
- [World Plugin](./world-plugin.md)
