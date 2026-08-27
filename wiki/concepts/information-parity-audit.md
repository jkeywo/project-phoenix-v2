---
title: Information-Parity Audit
type: concept
tags: [ai, backfill, parity, consoles, blackboards, coordination]
sources: [pasm/spec/DATA_DRIVEN_FINE_SYSTEM_AI.md, src/entities/ai_flag_hosts.rs, src/ai/host.rs, src/ship/helm_ai/, src/ai/server.rs, src/console_ai/core.rs, src/console_ai/server.rs, src/console/captain/server.rs, src/console/comms/server.rs, src/console/repair/server.rs, src/console/navigation/server.rs, src/console/weapons/server.rs, src/console/weapons/blackboard.rs, src/ship/power.rs, src/ship/sensors.rs, src/ship/shields.rs, src/ship/coordination.rs, src/ship/coordination_systems.rs, src/core/messages.rs, gui/console-state.js, gui/console-payload.js, gui/mount-plan.js]
updated: 2026-08-27
---

# Information-Parity Audit

Backfill may derive private policy memory from facts available to the station it replaces, but it must not receive a privileged world view. The server projects authoritative facts into per-ship blackboards and typed coordination messages; both the AI host and the human console consume those same surfaces.

## Station checklist

| Domain | Shared facts | Human surface | Backfill consumer |
|---|---|---|---|
| Captain | objectives, selected priority, combat activity, red alert, weapons hold, current view | Captain blackboard and controls | `operate_captain_ai` |
| Helm | own motion, authored limits, combat lock, waypoint/clearance, scored objectives, visible contacts, weapon/shield geometry | Helm blackboard/radar and controls | hosts under `src/ship/helm_ai/` |
| Tactical | combat lock, visible/acquirable contacts, weapon readiness/arcs/range, scored operate/destroy directives | Tactical radar and weapons controls | `ai_target_selection` plus weapon-family hosts |
| Shields | own arc health/focus, damage history, threat bearing | Shields blackboard and arc controls | `ai_shield_focus` |
| Power | group allocations, battery charge, authored limits, brownout state | Power state/blackboard | `ai_power_allocation` |
| Sensors | sensor contacts, selected target, scan progress/results | Sensors radar and scan panel | `operate_sensors_ai` |
| Repair | visible system damage, team state, queue severity, external targets | Repair blackboard and team controls | `operate_repair_ai` and external-repair host |
| Comms | inbox, contacts, range flags, scripted replies, urgency | Comms blackboard/panels | Comms hosts in `src/console/comms/server.rs` |
| Navigation | chart contacts, waypoint, route cursors, civilian traffic/order state | Navigation map and traffic controls | `operate_navigation_ai` and civilian-order host |

Static selection inputs such as a hull's authored power rating may be shown at ship choice rather than repeated on every console. Derived timers, deltas, and bounded-window verdicts do not need a separate display when they are computed only from already-visible facts.

## Coordination facts

Cross-station requests use the typed coordination queue and serve the hull's authored lag before delivery. Current payloads cover target designation, threat bearing, shield-frequency hints, weapon arc bearing, navigation clearance, repair requests, shield-facing changes, and power brownout. A human recipient receives a popup/console projection; an AI recipient reads the corresponding delivered state. Producers do not choose a privileged AI-only route.

## Audit guardrails

- AI hosts read `AiHostEnv`, per-ship blackboards, live authoritative components explicitly granted to that station, and typed coordination delivery.
- A target selected through Tactical, Sensors, or Navigation can remain actionable without granting the Helm a general long-range scan.
- Fine-system control, damage, power, and station rating gate whether the host may act; there is no coarse-console fallback.
- Policy memory is deterministic and snapshot-safe. It may fold visible facts over time but cannot introduce hidden world knowledge.
- Human and AI actions converge at admitted `ControlSystem` commands. Downstream appliers never branch on actor type.
- Human console routing and payload selection follow host-projected Console
  Family metadata and typed blackboard discriminants rather than System id
  spelling; flat versus keyed wire shape is not a second hull-specific rule.

## Related

- [AI Ship Unification](./ai-ship-unification.md)
- [AI Helm Decomposition](./ai-helm-decomposition.md)
- [Message Flow](./message-flow.md)
- [Station](../entities/station.md)
