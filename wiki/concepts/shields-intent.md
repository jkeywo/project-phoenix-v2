---
title: Shields Runtime
type: concept
tags: [shields, ai, damage, focus, coordination, pasm]
sources: [src/ship/shields.rs, src/ship/config.rs, src/core/broadcast/audience.rs, src/core/broadcast/lifecycle.rs, src/console_ai/server.rs, src/console_ai/core.rs, src/ship/coordination_systems.rs, pasm/spec/architecture/coordination-blackboards.yaml]
updated: 2026-08-28
---

# Shields Runtime

Shield focus is an admitted fine-system command. Human controls and `ai_shield_focus` emit the same `SetShieldArcFocus` payload; `handle_shields_messages` is the shared applier for each ship's authoritative `ShipShields` state.

Sensors can send a typed `ThreatBearing` through the shared Coordination
lag. `process_coordination_lag` resolves the explicit Station at delivery time:
AI produces `DeliveredCoordination`, Human receives the ordinary popup, and
Offline consumes without delivery. `receive_shields_coordination` lives in the
Shields module, verifies that the address is the authored owner of the ship's
shield arcs and that the authored Shields focus capability still operates AI,
then latches the exact bearing in `PendingShieldsThreatBearing`. On the
following Shields AI decision, the pending value takes priority over damage
analysis and focuses the authored arc whose centre is closest to the bearing.

The generic lag router never reads or writes `PendingShieldsThreatBearing`.
That component is private Shields-domain state, written by the Shields receiver
and consumed once by the Shields AI.

`ShipShieldsPlugin` also owns Shields replication lifecycle. The periodic and
reconnect paths share one `ShieldStatus` builder and the same
`HoldingSystemKind("shields")` audience resolution against the LocalShip's
authored topology. The instance id remains hull-authored rather than becoming
a hidden protocol constant. Reconnect therefore reaches only the current
holder of the actual owning Station, and because Shields has no delta cache it
cannot disturb another recipient's next live snapshot.

Without a pending bearing, the authored policy examines timestamped recent
damage and normalized arc health: concentrated incoming damage wins, then a
disproportionately weak arc, otherwise focus is cleared. Single-arc ships have
nothing to focus. Human Shields retains exclusive focus control whenever the
hull's authored Shields focus capability is human-operated.

The policy thresholds and windows come from the hull's AI policy parameters. The pure decision kernel is `tick_shield_focus_ai` in `src/console_ai/core.rs`; the Bevy host in `src/console_ai/server.rs` supplies current per-ship state and emits the admitted command. High-fidelity gating and the shared AI cadence keep the decision deterministic.

## Related

- [Information-Parity Audit](./information-parity-audit.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Science / Sensors target](./science-plugin.md)
