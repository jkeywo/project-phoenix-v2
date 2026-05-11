---
title: PRD #117 — Modifier System for Cross-Console Multipliers
type: source
tags: [prd, modifier, infrastructure, simulation, planned]
source_url: https://github.com/jkeywo/project-phoenix-v2/issues/117
status: open
updated: 2026-05-11
---

# PRD #117 — Modifier System for Cross-Console Multipliers

A central, pure-Rust system for applying multiplicative bonuses across consoles, regions, and ship systems. Infrastructure for PRD #118 (Power) and beyond.

## Status

Open. Pure module; no Bevy. Should land before any feature that depends on it.

## Problem

Every game constant is hardcoded — `BEAM_DAMAGE_PER_SEC = 5.0`, `ShipPhysicsConfig::new()`, `REPAIR_HP_PER_SEC = 1.0/3.0`. As Impulse, Shields, Region effects, Phaser banks, Power, etc. land, each cross-system interaction needs its own bespoke multiplier path. Without a central system every interaction is ad-hoc, untestable, and impossible to reason about.

## Solution

A `modifiers.rs` pure module owns a `ShipModifiers` resource. Any system can `add_or_update(source, slot, bonus)` or `remove(source, slot)`. The cache is rebuilt eagerly. Consumers (physics, weapons, repair, radar) call `get(slot) -> f32` for an O(1) multiplier lookup and multiply their base value.

## Key decisions

- **Pure Rust, fully testable.** No Bevy import in `modifiers.rs`.
- **Identity = (source, slot).** Re-applying replaces. Different sources on the same slot stack additively.
- **Asymmetric formula.** Sum bonuses for the slot. If sum ≥ 0, multiplier is `1.0 + sum`. If sum < 0, multiplier is `1.0 / (1.0 + |sum|)` — debuffs can never zero a value.
- **Initial slots:** `MaxSpeed`, `MaxYawRate`, `RadarRange`, `PhaserDamage`, `HullDamageTaken`, `RepairRate`. Add as needed.
- **Initial sources:** `Console(Console)`, `ImpulseDrive`, `RegionEffect { region_id }`.
- **Wire transport.** `ServerMessage::ModifierAdded { source, slot, bonus }` and `ModifierRemoved { source, slot }`. Client renders if it wants.
- **Empty table = no behaviour change.** Cache is all `1.0`; existing systems are unaffected until a source registers a modifier.

## Schema additions (planned)

- New module: `modifiers.rs` (pure).
- New types: `ModifierSlot`, `ModifierSource`, `Modifier`, `ShipModifiers`.
- `messages.rs`: `ModifierAdded` / `ModifierRemoved` server variants; the slot/source enums also serialised.
- `simulation.rs`: insert `ShipModifiers` resource, wire `get()` calls into helm/physics, beam tick, collision damage, repair tick, and radar/weapons range checks.

## Out of scope

- The actual Impulse Drive feature (issue #99) — it will register into the slots when it lands.
- The Shield System (#95).
- Region Effects (#105).
- Phaser Banks (#107).
- A Power System — see PRD #118.
- Client-side modifier UI.
- Any modifier slot beyond the initial six.

## Cross-references

- Consumed by [PRD #118 — Engineering Split](./prd-118-repair-and-power-consoles.md).
- [Roadmap Overview](../roadmap/overview.md)
- [Open Architectural Questions](../roadmap/open-architectural-questions.md)
