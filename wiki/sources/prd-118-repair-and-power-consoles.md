---
title: PRD #118 — Engineering Split: Repair + Power Consoles
type: source
tags: [prd, console, engineering, repair, power, modifier, shipped]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/118
status: shipped
updated: 2026-05-13
---

# PRD #118 — Engineering Split: Repair + Power Consoles

Splits the existing `Engineering` console into two: a `Repair` console with a shape-matching social-deduction repair loop and a new `Power` console allocating 6+2 power points across Helm, Tactical, and Science. Depends on PRD #117 (Modifier System).

## Status

Shipped (2026-05-12). `Console::Engineering` removed; `Console::Repair` and `Console::Power` are live. Pure modules `src/repair_teams.rs` and `src/power_system.rs` carry the logic. Consumes PRD #117's `ShipModifiers`.

## Problem

The current single Engineering console has shallow gameplay (one button per breakdown) and the breakdown queue has no social dimension — the server tells you which console to repair. There is also no way for the crew to make ship-wide power trade-offs.

## Solution

- **Rename** `Console::Engineering` → `Console::Repair` (breaking wire change). **Add** `Console::Power`.
- **Repair** is a shape-matching mini-game: each breakdown carries a random `Shape` (`Square`, `Triangle`, `Circle`). Damaged consoles see their shape via `ShowRepairIcon`; one *decoy* undamaged console also sees a fake icon. The Repair officer talks to the crew, picks the right shape button, dispatches one of three repair teams. Wrong button = team to 10 s cooldown. Correct = 30 s repair (10 HP).
- **Power** is a 6 base + up to 2 battery distribution across Helm/Tactical/Science. Battery drains/recharges by total allocated points (rate table indexed 3..8). At 0 battery: lock all to level 1 until charge recovers to `emergency_threshold`. Power level → modifier bonus on the relevant slot via PRD #117.

## Key decisions

- **`Console::Engineering` → `Console::Repair`.** Hard rename, no coexistence.
- **`Console::Power`** added. Both selectable in the lobby; one player may hold both on a small crew.
- **`Shape` enum** (`Square`, `Triangle`, `Circle`) — used in `BreakdownEntry`, `ClientMessage::Repair`, `ServerMessage::ShowRepairIcon`, `TeamSlot`.
- **`BreakdownQueue<BreakdownEntry>`** replaces `<Console>`. Shape fixed for entry's lifetime.
- **`RepairTeams` resource** (pure) — three slots, each `Idle | Repairing { progress } | Cooldown { progress }`. Repair = 30 s, cooldown = 10 s. In-progress repairs are never interrupted. Two teams can repair different entries on the same console simultaneously.
- **Decoy management.** On every queue change, server clears old decoy and assigns a new one from undamaged consoles. Decoy and real icons use the same wire messages — clients never know their role.
- **`PowerSystem` resource** (pure) — levels (1..4 each), battery (0..100), locked flag. Modifier registration on level change via `ModifierSource::Console(Console::Power)` (or per affected console — TBD by implementation).
- **`authorized_repair_console` removed** from `SimState`. Shape-matching is the sole gate.
- **Per-console power multiplier tables** in `player_ship.toml`. Default `[-0.5, 0.0, 0.25, 0.5]` (level 1 = half, level 2 = baseline, level 4 = +50%). Battery section configurable.
- **Repair UI:** breakdown row + 3 shape buttons + 3 team rows (idle/repairing/cooldown bars).
- **Power UI:** 3 console rows with +/- buttons, battery bar; greyed-out during exhaustion.

## Schema additions (planned)

- `Shape` enum (pure).
- `BreakdownEntry { console, shape }`.
- New pure modules: `repair_teams.rs`, `power_system.rs`.
- New `ClientMessage`: `Repair { shape }` (replaces old), `IncreasePower { console }`, `DecreasePower { console }`.
- New `ServerMessage`: `ShowRepairIcon { shape }`, `ClearRepairIcon`, `PowerState { helm, weapons, science, battery_charge, locked }`.
- Extended `ServerMessage::RepairState` with `teams: [TeamSlot; 3]`, `current_breakdown: Option<(Console, Shape)>`. `authorized_repair_console` field removed.
- Extended `ServerMessage::SimState` with `power_levels: (u8, u8, u8)`.
- `EntityConfig`: per-console `power_multipliers` table; new `[power]` section.

## Out of scope

- Modifier System itself (PRD #117 — dependency).
- Science console implementation.
- Shield/torpedo regen as power effects.
- Acceleration as a modifier slot.
- Client-side modifier buff/debuff UI.
- Any renderer / 3D visual changes.
- Captain console changes.

## Cross-references

- Depends on [PRD #117 — Modifier System](./prd-117-modifier-system.md).
- [Console](../entities/console.md) · [Engineering Console (current)](../entities/console.md) · [Roadmap: Console Expansion](../roadmap/console-expansion.md)
- [Draft 5 — Ship's Power](./design-05-ships-power.md) (informs the power model).
