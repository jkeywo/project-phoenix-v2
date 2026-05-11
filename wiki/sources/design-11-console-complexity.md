---
title: Draft 11 — Console Complexity
type: source
tags: [draft, console, complexity, accessibility, tactical, science, engineering]
source_path: docs/11. Draft Design - Console Complexity.md
status: draft
updated: 2026-05-11
---

# Draft 11 — Console Complexity

Players can select a complexity rating per console. Low-complexity modes automate or abstract advanced functions to reduce cognitive load.

## Console-level, not player-level

Complexity is tied to **consoles**, not players. When a player joins a console they inherit its current complexity. Stored preferences in `localStorage` are pushed via `SetComplexity` on join.

## Coverage

- **No complexity (out of scope):** Captain's Chair, Communication, Helm.
- **Complexity available:** Tactical, Science, Engineering.

## Shield Frequency System

Three frequencies (alpha / beta / gamma). Enemy shields tuned to one. Science detects when targeting; Tactical tunes phasers to match for a full damage bonus. Torpedoes unaffected.

## Per-console simplifications

### Tactical (Low)
- Flat 1/3 of the full frequency-match damage bonus (consistency over peak).
- Auto-fires a torpedo when target shields hit 0.
- Auto-fire defaults ON.

### Science (Low)
- Ignores enemy shield frequency if **either** Science or Tactical is Low (cascading simplification).

### Engineering (Low)
- No battery — max 6 power points instead of 6+2.
- Base 6 points grant slightly higher bonuses to compensate.

## Switching behaviour

- **Low → Full:** unlocks the advanced mechanic immediately.
- **Full → Low:** removes it immediately. Engineering Low: excess battery points removed from consoles with 2+ points (consoles at 1 are protected).

## Persistence

Per-console preference in browser `localStorage`. First-use prompt. `SetComplexity { console, level, token }` synchronises to the server. Server stores complexity on the console; broadcasts changes to all clients. Last message wins on concurrent changes.

## Design philosophy

- **Simplicity has a cost.** Low trades peak effectiveness for reduced load.
- **Immediate changes**, no cooldown.
- **No notifications** — the power difference is the signal.
- **Console-level**, not player-level — consistent regardless of operator.

## Cross-references

- Builds on top of [PRD #118 — Repair + Power](./prd-118-repair-and-power-consoles.md) (Engineering / Power model).
- Builds on top of [Draft 4 — Combat Update](./design-04-combat-update.md) (shield frequency).
- [Roadmap: Console Expansion](../roadmap/console-expansion.md)
