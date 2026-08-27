---
title: Shields Runtime
type: concept
tags: [shields, ai, damage, focus, coordination]
sources: [src/ship/shields.rs, src/console_ai/core.rs, src/console_ai/server.rs, src/ship/coordination_systems.rs]
updated: 2026-08-27
---

# Shields Runtime

Shield focus is an admitted fine-system command. Human controls and `ai_shield_focus` emit the same `SetShieldArcFocus` payload; `handle_shields_messages` is the shared applier for each ship's authoritative `ShipShields` state.

Backfill first consumes a delivered Sensors `ThreatBearing`, focusing the closest authored arc. Without that override, the authored policy examines timestamped recent damage and normalized arc health: concentrated incoming damage wins, then a disproportionately weak arc, otherwise focus is cleared. Single-arc ships have nothing to focus.

The policy thresholds and windows come from the hull's AI policy parameters. The pure decision kernel is `tick_shield_focus_ai` in `src/console_ai/core.rs`; the Bevy host in `src/console_ai/server.rs` supplies current per-ship state and emits the admitted command. High-fidelity gating and the shared AI cadence keep the decision deterministic.

## Related

- [Information-Parity Audit](./information-parity-audit.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Science / Sensors target](./science-plugin.md)
