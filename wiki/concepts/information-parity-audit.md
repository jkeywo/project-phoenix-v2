---
title: Information-Parity Audit
type: concept
tags: [ai, backfill, parity, consoles, blackboards, coordination]
sources: [pasm/spec/DATA_DRIVEN_FINE_SYSTEM_AI.md, crates/phoenix-simulation/src/entities/ai_flag_hosts.rs, crates/phoenix-simulation/src/ai/host.rs, crates/phoenix-simulation/src/ship/helm_ai/, crates/phoenix-simulation/src/ai/server.rs, crates/phoenix-simulation/src/console_ai/core.rs, crates/phoenix-simulation/src/console_ai/server.rs, crates/phoenix-simulation/src/console/captain/server.rs, crates/phoenix-simulation/src/console/comms/server.rs, crates/phoenix-simulation/src/console/repair/server.rs, crates/phoenix-simulation/src/console/navigation/server.rs, crates/phoenix-simulation/src/console/weapons/server.rs, crates/phoenix-simulation/src/console/weapons/blackboard.rs, crates/phoenix-simulation/src/ship/power.rs, crates/phoenix-simulation/src/ship/sensors.rs, crates/phoenix-simulation/src/ship/shields.rs, crates/phoenix-simulation/src/ship/coordination.rs, crates/phoenix-simulation/src/ship/coordination_systems.rs, crates/phoenix-model/src/messages.rs, gui/console-state.js, gui/console-payload.js, gui/mount-plan.js]
updated: 2026-09-07
---

# Information-Parity Audit

Backfill may derive private policy memory from facts available to the station it replaces, but it must not receive a privileged world view. The server projects authoritative facts into per-ship blackboards and typed coordination messages; both the AI host and the human console consume those same surfaces.

GM contact overrides (#1309) live in `WorldContentRuntime` by observing player ship and target. Each Sensors blackboard exposes only that ship's overrides. Reveal preserves ordinary visible contacts and earned scans, adding a position-only contact only when ordinary sensors would omit it; the GM Crew Knowledge comparison must not enrich that basic contact with hull information from the raw replica. Conceal removes ordinary Sensors contacts, selected detail, scans and warnings. Comms and Objectives retain their independent authored knowledge. Normal removes the pair, while snapshot/restore preserves active pairs and lifecycle pruning removes references to vanished identities.

## Station checklist

| Domain | Shared facts | Human surface | Backfill consumer |
|---|---|---|---|
| Captain | objectives, selected priority, combat activity, red alert, current view | Captain blackboard and controls | `operate_captain_ai` |
| Helm | own motion, authored limits, combat lock, waypoint/clearance, scored objectives, visible contacts, weapon/shield geometry | Helm blackboard/radar and controls | hosts under `crates/phoenix-simulation/src/ship/helm_ai/` |
| Tactical | combat lock, visible/acquirable contacts, weapon readiness/arcs/range, scored operate/destroy directives | Tactical radar and weapons controls | `ai_target_selection` plus weapon-family hosts |
| Shields | own arc health/focus, damage history, threat bearing | Shields blackboard and arc controls | `ai_shield_focus` |
| Power | group allocations, battery charge, authored limits, brownout state | Power state/blackboard | `ai_power_allocation` |
| Sensors | sensor contacts, selected target, scan progress/results | Sensors radar and scan panel | `operate_sensors_ai` |
| Repair | visible system damage, team state, queue severity, external targets | Repair blackboard and team controls | `operate_repair_ai` and external-repair host |
| Comms | inbox, contacts, range flags, scripted replies, urgency | Comms blackboard/panels | Comms hosts in `crates/phoenix-simulation/src/console/comms/server.rs` |
| Navigation | chart contacts, waypoint, route cursors, civilian traffic/order state | Navigation map and traffic controls | `operate_navigation_ai` and civilian-order host |

Static selection inputs such as a hull's authored power rating may be shown at ship choice rather than repeated on every console. Derived timers, deltas, and bounded-window verdicts do not need a separate display when they are computed only from already-visible facts.

## Coordination facts

Cross-station requests use the typed coordination queue, carry an explicit
`CoordinationAddress::Station` or `CoordinationAddress::Ship`, and serve the
hull's authored lag before delivery. Current payloads cover target designation,
threat bearing, shield-frequency hints, weapon arc bearing, navigation
clearance, repair requests, shield-facing changes, and power brownout. A human
Station recipient receives a popup/console projection; an AI Station recipient
reads the corresponding delivered state. Ship-addressed traffic fans out in
authored Station order under the same live control policy. The payload never
chooses a privileged AI-only route.

For `RepairRequest`, the lag router applies the same `HullVisibility` policy as
the Repair console before emitting a human-popup delivery. Repair's receiver
therefore sees a recipient-projected human payload, but retains the exact
host-internal deficit when merging an AI delivery into its severity queue.

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
