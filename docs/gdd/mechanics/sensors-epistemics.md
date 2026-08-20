# Project Phoenix — Sensors and Epistemics

| Field | Value |
|---|---|
| Status | Current targeting, scan and evidence mechanics with accepted extensions |
| Scope | Radar projection, science targeting, scans, evidence, dossiers, confidence and environmental observation |
| Audience | Design, content, UI, simulation and playtest |

Sensors is the crew's discipline for turning an uncertain world into actionable knowledge. It separates what exists, what the ship can currently observe, what the crew chose to investigate and what they can honestly claim to know.

Related documents: [Targeting and Weapons](./targeting-weapons.md), [Navigation and Relative Motion](./navigation-relative-motion.md), [Comms and Commitments](./comms-commitments.md), [World and Environmental Systems](../systems/world-environmental-systems.md), and [Campaign Continuity](../foundation/campaign-continuity.md).

## Experience goals

- Make information gathering an active station role rather than a passive radar feed.
- Preserve the difference between simulation truth, current observation and historical evidence.
- Give Science useful influence over Tactical and command decisions without merging their authority.
- Let scenarios present incomplete, conflicting or misleading accounts without secretly changing mechanical truth.
- Support future multi-ship play where knowledge belongs to the observing crew and must be communicated.

## Radar projection

Radar is a derived view of authoritative entities, not a second entity database. Each surface projects contacts into its own range, orientation and display needs while retaining stable entity identity. Tactical, Helm, Navigation and Sensors can therefore show different useful views of the same world without owning divergent positions.

Selection authority remains surface-specific. Tactical owns the Combat Lock. Sensors owns a Science Target. Navigation owns chart selection locally until it commits a waypoint. A visible blip does not imply that every station has selected or fully identified it.

## Science target and designation

Human and AI Sensors select through the same admitted command. The Sensors blackboard holds the current target and the information needed for its panel. Selecting a Science Target can send a Channel-3 designation to Tactical; clearing it does not issue a combat command.

Tactical may independently validate and adopt the advisory. Sensors never writes the Combat Lock. This keeps discovery and engagement separate while allowing a concise crew handoff.

## Current scan mechanic

A hull may author a survey suite with range bands, fidelity, a minimum power requirement and interference behavior. `ScanTarget` targets the ship's real Sensors system. The host derives a reading from the subject's current published infrastructure condition, operational flags and capacities.

Nearer or better bands report more precise values; condition is quantised from the same authoritative fraction rather than copied into a second coarse truth. Interference can degrade the achieved band. An unreadable target, excessive range, insufficient power or blinding interference produces a typed refusal.

The last successful reading is a snapshot of what the crew saw at that tick. It is not continually updated as the target changes. The last refusal and last reading are mutually exclusive so a fresh failure cannot sit beside stale data that appears current.

Taking a scan latches `scan.<entity-id>.taken` for scenario logic, but completion does not automatically add prose evidence. A scenario that wants the result preserved as a finding appends an authored string entry with scan provenance.

## Knowledge ladder

The accepted scan ladder moves from detection, through identification and classification, to tactical assessment and intrusive analysis. Detection supplies location and broad category. Identification adds transponder, faction or declared identity. Classification adds hull class, role and known capabilities. Tactical assessment adds observable shields, damage, emissions and active systems. Individual entities may omit or falsify authored fields, while evidence records how each claim was obtained.

Band C2 covers detection through tactical assessment using confidence and focused observation. Band C5 adds intrusive analysis of protected internals, cargo, personnel or vulnerabilities. Intrusive scanning accelerates or unlocks deep information, emits a detectable signature and may trigger authored diplomatic or tactical reactions.

Band C5 also adds the EWar MVP: Jam degrades Sensors and Comms while raising the attacker's signature; Spoof produces uncertain contacts that observation can disprove; Harden protects the ship's Sensors and Comms at an authored power, heat or capability cost. Broader remote disruption remains unscheduled.

## Evidence and dossiers

Evidence is an append-only log. Each entry names the subject entity, a string-catalogue finding, one of the typed provenances—scan, dialogue, records or briefing—and the tick when it was learned. Repeating the same subject, provenance and text is a no-op that preserves the first discovery time. Contradictions are appended rather than rewriting history.

A dossier is a current projection, not an independent truth store. It combines identity, crew-facing description, faction, current hail standing, published infrastructure facts, commitments and gathered evidence. Hidden or unpublished state has no route into it. An empty dossier means the crew has a recognised subject with nothing yet on file; a missing dossier means there is no eligible subject surface.

## Accepted confidence model

The accepted extension gives each observing ship its own observation and evidence stores. Knowledge crosses ships only through Comms and carries provenance. This is required before multi-ship or opposed-crew play so one crew's scan does not silently become everyone's knowledge.

Observation confidence uses a small ordered ladder: detected, rough, fair and precise. Scan effort advances confidence per subject, with displayed ranges derived from the tier and a seeded offset so an estimate is not always centred on truth. Interference can slow progress or cap the reachable tier.

The scan suite gains one clear mode axis: wide scanning accumulates slowly across subjects in range; focused scanning advances one named subject quickly while other progress pauses. Environmental fields become scan subjects rather than requiring a separate scanner.

## Fields and hazard knowledge

Future environmental fields upgrade the existing region model with intensity, falloff and deterministic motion or growth. Radiation, interference and debris have real effects. Sensors observes those fields through the same evidence model, while Helm and Navigation receive actionable hazard advisories rather than raw omniscience.

Forecasting evaluates authored field parameters at a future logical tick. It must not simulate a secret mutable copy of the field or give AI a cleaner answer than the player-facing observation supports.

## AI and backfill

AI Sensors chooses among valid targets using authored policy and its own damage-scaled sensor horizon. Common priorities include mirroring a valid Combat Lock, following a named objective or selecting the nearest hostile. It emits ordinary selection and scan commands and uses the same fidelity and interference rules.

AI must not read hidden infrastructure state when deciding what the crew knows. In future multi-ship play, it uses the observing ship's evidence store, not a world-global dossier.

## Playtest questions

- Can players distinguish a contact, a Science Target, a Combat Lock, a scan reading and a dossier finding?
- Do scan refusal reasons lead to a clear next action?
- Is historical evidence useful when current structure state has changed?
- Can scenarios present contradictions without players assuming the UI is inconsistent?
- Do Sensors designations help Tactical while preserving Tactical's authority?
- In the future confidence model, do wide and focused scans create a comprehensible trade-off?

## Canonical sources

- [Radar and sensors architecture](../../../pasm/spec/architecture/radar-sensors.yaml)
- [Fields and epistemics design](../../../pasm/spec/design/fields-epistemics.yaml)
- [World files architecture](../../../pasm/spec/architecture/world-files.yaml)
- [Radar projection wiki](../../../wiki/concepts/radar-projection.md)
