# Project Phoenix — Duty Teams, Officers and Operations

| Field | Value |
|---|---|
| Status | Accepted, planned but not scheduled |
| Scope | Duty Teams, Duty Officers, system assignments, Away Missions, boarding, Medical and Operations systems |
| Audience | Design, content, UI, simulation and campaign |

Duty Teams turn the repair-team pattern into a shared workforce model without simulating every person aboard. Duty Officers provide named leadership and traits; Operations systems assemble and launch bounded off-screen missions that the bridge can support through its ordinary stations.

Related documents: [External Operations](./external-operations.md), [Damage, Diagnosis and Repair](./damage-repair.md), [Comms and Commitments](./comms-commitments.md), [Campaign Continuity](../foundation/campaign-continuity.md), and [Planned but Not Scheduled](../future/planned-not-scheduled.md).

## Duty Teams

A ship authors a fixed number of anonymous Duty Teams by type, such as Engineering, Security, Science or Medical. Each team represents a group of ensigns and has one named Duty Officer leader. A leaderless team remains fully functional but receives no Duty Officer trait bonus.

An internal task consumes one whole team. Examples include repairing a station, guarding the brig, repelling boarders, staffing a medical response or performing scenario-specific technical work. A team committed to one task is unavailable elsewhere until released.

Duty Teams retain only type, leader, assignment and availability. Individual ensigns are never named or continuously simulated.

## Duty Officers

Duty Officers are named campaign entities with department, role tags, rank, traits and status. Eligible ship systems author a small number of compatible officer slots, normally one. An assigned officer contributes explicit trait effects such as a performance modifier, improved information, an AI-policy option or reduced task risk.

Trait effects do not stack unless the system authors distinct slots. Only officers assigned to a system or mission are exposed to fatigue, injury, disappearance or death; unassigned available officers are abstractly off duty and safe. Watches, sleep and continuous location are not simulated.

Shipboard casualty checks occur on discrete significant events such as a system crossing a damage tier, destruction, a boarding breach or an authored hazard. Repeated damage ticks do not repeatedly roll casualties. The event, assigned officer, Medical capacity and protection traits determine the visible outcome probabilities.

## Operations systems

A ship may carry multiple Operations systems. Each authors mission tags, transport or deployment method, range and eligibility, personnel authority, support requirements and concurrent-mission capacity. A shuttle bay, transporter room, carrier facility or specialist mission office can therefore share one mission framework without becoming the same piece of equipment.

Mission definitions declare tags and may launch only from a compatible Operations system. Capacity defaults to one active mission but is authored per system; concurrency consumes separate transport, personnel and support slots.

An Operations system may integrate assignment authority locally or leave it remote. Local integration explicitly delegates the relevant Duty Officer and Duty Team pools to the Operations operator, supporting rapid actions such as Tactical assembling a boarding party. Under remote assignment, Operations requests authored required and optional slots from the stations that own those personnel and cannot override a human-held station.

## Away Missions

Away Missions are off-screen operational sequences, not character-scale play. Operations selects a mission and deployment method, fills required and optional slots, reviews exact check percentages and launches when every required slot is valid. Optional empty slots forgo their benefits. AI-controlled or assisted stations may fill remote requests through authored policy.

Missions progress through timed phases, checks and decision events. The deployed team may request direction, information, support or extraction from Operations. Prior bridge actions—such as scanning a destination, negotiating access or suppressing a threat—modify exact success probabilities. Results are deterministic from the authoritative seed and state.

Content may be scenario-specific or selected from a reusable generic mission library. Branches may depend on check success, bridge responses, deadlines, assigned officer traits, team type, equipment and transport. Outcomes return through normal state: objectives, evidence, casualties, reputation, cargo, commitments, infrastructure and campaign facts.

## Deployment methods

Shuttles and carried craft are interceptable physical actors and may support reconnaissance, rescue, combat, escort or specialist missions. Fighters are combat-focused carried-craft missions within this model, not a separate RTS subsystem.

Transporters launch and recover missions within limited range more quickly and without interception. Shields, interference, transporter damage, destination suitability and scenario rules may still block them.

## Boarding and defence

Boarding is a combat Away Mission against a shieldless target within shuttle or transporter constraints. The attacker chooses an authored objective such as disabling a system, seizing cargo, rescuing personnel, gathering intelligence or capturing the vessel. Defending Security Teams oppose the checks. Ship combat and bridge support continue, and success applies only the selected objective's effects.

Security Teams assigned internally to repel boarders act as a persistent defensive task. Boarding breaches can trigger discrete Duty Officer casualty checks where relevant.

## Medical

Medical owns limited treatment capacity, patient triage and recovery priority for injured Duty Officers and teams. It supports Away Missions with personnel, supplies and remote advice; contributes capacity to evacuation or disaster operations; and assesses casualties, treatment outcomes, quarantine and biological hazards.

Medical asks Science to perform specialist biological or environmental scans, then can inspect medical results itself. It does not continuously simulate every crew member's health.

## Station databases

Every station may present a focused database projection over one shared evidence and reference model. Intelligence remains the primary cross-domain search and correlation surface. Station databases organise relevant procedures, known systems, scenario facts and historical readings without copying authoritative truth or exposing unacquired information.

Engineering's symptom-to-order manual, Medical treatment records and Operations mission history use this framework. Science scans can publish findings to affected station views. Comms-specific Away Missions may add durable Intelligence entries.

## Personnel consequences

Officers may return available, fatigued, injured, missing or dead. Injury and disappearance persist through campaign continuity. Lethal outcomes exist only when a mission or damage event explicitly presents that risk. Medical treatment and later scenario outcomes may recover non-terminal statuses.

## Acceptance criteria

- Internal tasks consume fixed typed teams; Duty Officers add bonuses without being required for base team function.
- Systems and missions expose explicit compatible officer/team slots and assignment authority.
- Required mission slots block launch; optional slots change visible odds or capability.
- Every check shows its exact percentage and contributing factors.
- Mission progress, communications, support and extraction use ordinary authoritative scenario systems.
- Multiple Operations systems can run compatible missions up to their authored capacity.
- Assigned personnel alone face shipboard casualty events, resolved on discrete significant events.
- Campaign persistence carries named officer status and mission consequences without simulating individual ensigns.

## Canonical sources

- [Campaign Continuity](../foundation/campaign-continuity.md)
- [External Operations](./external-operations.md)
- [World files architecture](../../../pasm/spec/architecture/world-files.yaml)
- [Engineering and damage architecture](../../../pasm/spec/architecture/engineering-damage.yaml)

