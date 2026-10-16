---
title: Shields Runtime
type: concept
tags: [shields, ai, damage, focus, coordination, pasm]
sources: [crates/phoenix-sim-gameplay/src/entities/config/shields.rs, crates/phoenix-sim-gameplay/src/ship/shields.rs, crates/phoenix-sim-gameplay/src/ship/shield_ai.rs, crates/phoenix-sim-gameplay/src/weapons/shield.rs, crates/phoenix-simulation/src/ship/shields.rs, crates/phoenix-simulation/src/ship/shield_evidence.rs, crates/phoenix-simulation/src/console_ai/shields_tree.rs, crates/phoenix-simulation/src/snapshot/preflight.rs, gui/components/ph-shield-operations.js, pasm/spec/design/shield-energy-and-protection.yaml]
updated: 2026-10-16
---

# Shields Runtime

Each authored shield arc is an ordinary System with its own local energy reserve,
command authority and state-tree continuation. `ShipShields` stores protection
strength, collapse time, raised intent and recovery destination. It does not own
a second battery. The canonical Power network owns energy and thermal state.

`tick_shields` runs after reactor intake and admitted shield commands, before
weapon impacts. All arcs pay upkeep before discretionary work. Funded upkeep
determines effective interception; unfunded damage uses the ordinary hull
payload. Lowered shields retain strength. Recharge spends local battery energy
for actual strength restored and generates ordinary depletion heat.

Recovery removes donor strength only for energy actually accepted by an authored
eligible battery. Recovery and reactor intake share the destination's intake
budget. The recipient still performs its own timed recharge. Thermal trips clear
effective protection while preserving intent. The same physical processing and
admitted command applier serve static equipped entities.

Humans and individual arc trees emit the same `SetShieldRaised`,
`SetShieldRecovery` and `SetShieldArcFocus` commands through System admission.
The tree host reads only its Console's permitted publications. Fine `ShieldArc`
observations contain operation readiness, battery, thermal state and recent
damage; broad `ShieldStatus` contains no private energy data. Explicit feeds
provide information without command authority.

`[[shield_arc]].focus_available` is authored capability, default true. It is
carried into the permitted fine operation view, gates AI focus output and human
controls, and is enforced by the shared command applier. Workshop edits it on
the arc. Restore rejects focused state for equipment without that capability.
The shipped single-arc Harrows opt out in data; arc count has no runtime role.
AI `preferred_focus` ranks only capable arcs, while `preferred_protection` and
recipient `target-preferred-*` rank all observed arcs for donor/recovery decisions.

The physical damage-history producer runs regardless of delegation. Recharge,
focus decay and voluntary recovery adjust its comparison baseline and do not
become incoming-damage reports. Arc trees can use permitted threat/lock bearings,
recent damage and normalized strength for defensive preference. Their authored
recovery thresholds provide hysteresis.

Snapshots and digests include raised intent, recovery destination, partial
interception carry, charge and damage history. Restore preflights arc identities
and physical bounds. Existing Console layouts mount shared operation cards from
permitted fine observations, keeping strength, battery, heat and protection
separate. Delegated arcs use the ordinary status summaries.

## Related

- [Information-Parity Audit](./information-parity-audit.md)
- [Modifier Coordination](./modifier-coordination.md)
- [Science / Sensors target](./science-plugin.md)
