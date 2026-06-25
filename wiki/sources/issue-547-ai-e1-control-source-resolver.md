---
title: Issue #547 — A1 ControlSourceResolver utilities
type: source
tags: [ai, control-source, resolver, prd-520, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/547
status: shipped
updated: 2026-06-25
---

# Issue #547 — A1 ControlSourceResolver utilities

PRD #520 slice A1. Establishes `ControlSource`, `ControlSourceResolver`, and `ControlTickPolicy` as the authoritative gate for per-system human/AI routing.

## Key decisions

- `ControlSource` is `Human` (default) or `Ai`.
- `ControlTickPolicy` derives `{ accept_human_input, operate_ai, coordinate }` from the source.
- `ControlSourceResolver` maps `SystemId → ControlSource`; defaults to `Human` for unmapped systems.
- `policy_for(system_id)` is the only allowed call site for control gating.

## Files

- `src/ship/control_source.rs`

## Cross-references

- [PRD #520](./prd-520-ai-ship-unification.md) — parent
- [AI Ship Unification](../concepts/ai-ship-unification.md)
