# Project Phoenix — External Operations

| Field | Value |
|---|---|
| Status | Current mechanic |
| Scope | Stabilise, tow, escort, transfer and field-repair actions performed beyond the ship |
| Audience | Design, content, UI, simulation and playtest |

External operations let the crew use the ship as a working vessel, not only as a weapon. They convert position, capability, power, repair capacity and environmental safety into sustained actions on structures and other craft.

Related documents: [World and Environmental Systems](../systems/world-environmental-systems.md), [Damage, Diagnosis and Repair](./damage-repair.md), [Power and Resource Network](./power-resource-network.md), [Navigation and Relative Motion](./navigation-relative-motion.md), and [Falling Skyway](../content/scenarios/falling-skyway.md).

Future personnel and mission operations are specified separately in [Duty Teams, Officers and Operations](./duty-teams-and-operations.md). They reuse the operation principles without turning every external hold into an Away Mission.

## Experience goals

- Give scenarios strong non-combat verbs with the same authority and feedback quality as weapons.
- Make holding position and protecting an operation meaningful crew work.
- Reuse ship capabilities, power and repair teams instead of inventing isolated minigames.
- Let hazards interrupt progress in authored, predictable ways.
- Provide generic verbs that many scenarios can compose without becoming content-specific code.

## Operation vocabulary

The common runner supports stabilise, tow, escort, transfer and field repair. A hull authors which verbs it can perform and their parameters. A scenario target supplies the relevant infrastructure condition, capacities or relationship needed by the operation.

Stabilise arrests or reverses a structure's failing condition through a timed hold. Field repair applies continuous condition improvement while work proceeds. Transfer moves an authored capacity between valid endpoints. Tow attaches the target to the operator's authored rig while eligibility holds. Escort represents sustained protection or accompaniment against scenario-defined completion conditions.

These verbs are generic mechanical contracts. Their fiction comes from string ids, target entities, capacities and script consequences. A “transfer” can move evacuees, cargo or coolant without adding a new transfer engine for each noun.

## Starting an operation

The host validates proximity, the operator hull's capability, required power, grid availability, free repair teams and the capacities of both endpoints. Invalid starts produce a typed refusal and change no authoritative state.

Eligibility should be previewable. The console shows the selected verb, target, required range and resources, and any currently known blocker. The host remains final because entities, damage and power may change between display and command.

An operation uses a reserved ship blackboard channel rather than pretending to be a repairable fine system. It can consume real systems and teams, but the operation itself is an activity performed by the ship.

## Hold and progress

Once started, the operation advances a deterministic timed hold at an authored rate. Progress, current state and effective rate are authoritative. Fractional progress survives hazard changes within a tick so repeated slowdown does not accumulate host-dependent rounding drift.

The crew's job is to maintain eligibility: hold the target in range, preserve required systems, allocate power, protect repair teams and respond to external danger. A completed hold pays its result into the infrastructure condition or capacity queues through their single authoritative writers.

The last settled operation remains available for presentation so a console can report completion, failure or cancellation rather than instantly returning to a blank panel.

## Interruptions

Operation definitions author how recent incoming fire and hazard-band membership affect work. An interruption may slow, pause or fail the hold. Losing a fixable prerequisite stalls progress; losing an irrecoverable prerequisite fails it.

The distinction must be visible. “Paused: target out of range” invites Helm to recover position. “Failed: transfer source destroyed” tells the crew that resuming is impossible. A generic red failure state is insufficient.

Tow has an additional physical consequence: while active, the target is held on the operator's rig. The attachment must release predictably on completion, cancellation, failure or destruction and must not create a second hidden motion authority.

## System interactions

Movement establishes and maintains geometry. Power supplies the authored group requirement. Repair teams may become unavailable for internal diagnosis while committed externally. Damage can disable the required capability. Sensors provides target condition and hazard knowledge. Navigation manages approach, escape and traffic. Comms may secure consent or terms before the mechanical operation is attempted.

This interdependence is the point. An external operation should create a temporary crew posture with opportunity cost, not a progress bar one player starts and forgets.

## AI and backfill

AI may operate an external verb only under authored scenario or hull policy. It uses the same start command, eligibility checks and interruption rules. It must not move the ship, allocate power or commandeer repair teams through hidden shortcuts; those needs travel through the normal station and coordination mechanisms.

For zero-player or partially crewed simulations, AI policies should be sufficient to complete explicitly supported scenario paths. They need not solve every dramatic negotiation or choose among irreversible beneficiaries unless the scenario authors that authority.

## Authoring and tuning

Hull capabilities, ranges, durations, rates, power and team requirements, rig positions and interruption policies belong in entity TOML. Targets author infrastructure flags and capacities. Rhai opens and closes narrative windows, reacts to settled outcomes and resolves commitments; it should not advance a parallel progress counter.

## Playtest questions

- Can the crew tell what an operation requires before starting it?
- Does maintaining the hold create work across multiple stations?
- Are slow, paused and failed states distinct and recoverable where intended?
- Do generic operations feel specific once scenarios provide actors, stakes and resources?
- Are internal repair and external work competing for teams in a legible way?
- Can AI complete authored support paths without using hidden capabilities?

## Canonical sources

- [World files architecture](../../../pasm/spec/architecture/world-files.yaml)
- [Power, modifiers and regions architecture](../../../pasm/spec/architecture/power-modifiers-regions.yaml)
- [Falling Skyway world](../../../assets/worlds/falling_skyway.toml)
- [World and environmental systems](../systems/world-environmental-systems.md)
