# Project Phoenix — Crew-Owned External Systems

| Field | Value |
|---|---|
| Status | Current mechanic |
| Scope | Tractor tow/stabilise/escort, docking, umbilical transfer, and external repair dispatch |
| Audience | Design, content, UI, simulation and playtest |

External work makes the ship a practical rescue and logistics platform. The
retired `[operations]` runner no longer owns these verbs: each job is a chain of
ordinary ship systems, controlled by the station that owns it and driven by the
same admitted command whether that station is human or Backfill.

Related documents: [World and Environmental Systems](../systems/world-environmental-systems.md), [Damage, Diagnosis and Repair](./damage-repair.md), [Power and Resource Network](./power-resource-network.md), [Navigation and Relative Motion](./navigation-relative-motion.md), and [Falling Skyway](../content/scenarios/falling-skyway.md).

## Experience goals

- Give scenarios strong non-combat verbs with the authority and feedback quality of weapons.
- Make geometry, target selection, power, capacity, and repair-team allocation shared bridge work.
- Let human and Backfill seats operate the same controls without hidden completion shortcuts.
- Put completion on the target's authoritative state rather than on a parallel progress counter.

## Current system chains

| Fictional work | Crew-owned chain | Authoritative result |
|---|---|---|
| Tow | Tactical locks; Engineering engages the tractor | Target is coupled to the operator's rig while eligibility holds |
| Stabilise | Tactical locks; Engineering engages the tractor | Target's `arrest-decline` response queues condition recovery |
| Escort | Tactical locks; Engineering engages the tractor | Target's `formation-keep` response holds its authored slot |
| Transfer | Helm docks; Engineering starts the umbilical | Capacity moves between both docked entities' ledgers |
| Field repair | Repair dispatches one free team | Target condition rises while range and team assignment hold |

The tractor, dock and umbilical are first-class `[[system]]` entries with their
own power group, damage row, admission consumer, blackboard, snapshot state and
Backfill policy. External repair uses the existing repair system and commits a
real team that cannot simultaneously repair the operator's own hull.

## Target-side meaning

The operator authors how it can couple; the target authors what coupling means.
`[held_response]` selects `follow`, `arrest-decline`, `station-keep`, or
`formation-keep`. Arrest-decline cancels the target's ordinary decay and adds
its authored recovery rate through the infrastructure adjustment queue, so the
target crosses its own condition thresholds. Formation-keep supplies its own
offset and distance; other held responses use the operator's coupling rig.

Transfer is similarly target-backed. The umbilical names one capacity that must
exist on both docked ends and moves it at an authored rate, clamped by source
level and destination headroom. A scenario should define success with a
capacity-backed threshold on the receiving entity. That produces the same
authoritative `FlagSet`/`FlagCleared` edge as a condition threshold and proves
that the cargo arrived; source depletion or elapsed time does not.

## Objectives and Backfill

Scenario objectives expose `Tow`, `Stabilise`, `Transfer`, and `FieldRepair`
directives. The relevant station AIs read the shared scored objective pool:
for a `Transfer`, Helm docks with the named target and Engineering starts the
umbilical. The directive coordinates work but does not decide completion.

If a target is available before the objective posts, the scenario may latch its
completion edge and use remembered-objective posting later. Genuine early work
then appears complete without inventing an early objective; an unworked deadline
may fail the optional task but must never complete it for free.

Protected dramatic choices remain separate. Backfill may prepare machinery and
keep the mandatory scenario spine moving, but it must not choose an irreversible
beneficiary unless the scenario explicitly grants that authority.

## Interruptions and recovery

Each physical system owns its interruptions. Tractor coupling drops on lost
lock, range, power, system availability, or target loss. Docking owns approach,
mating, undocking and partner loss. Umbilical flow stops when the dock parts,
power or system availability is lost, or either ledger cannot participate; what
already moved stays moved. External repair stops when its real team or range is
lost. There is no generic operation stall/failure state and no retired
`[[operations.capability.interrupt]]` authoring surface.

Workforce state is also not a hidden system modifier. A scenario reads the
published workforce flags and authors the specific permission, capacity,
condition, objective or dialogue consequence it needs.

## Authoring and tuning

Operator ranges, rates, rig offsets, power requirements and repair-team rules
belong in entity TOML. Dock markers belong in model rig sidecars. Targets author
`[held_response]` and `[infrastructure]` state. Rhai posts objectives, remembers
early state, reacts to threshold edges, sets deadlines and resolves commitments;
it does not run a second progress simulation.

## Playtest questions

- Can each station tell which part of the chain it owns and what currently blocks it?
- Does the geometry create coordination without making recovery tedious?
- Is target-side completion visible on both the objective list and the relevant system/dossier surface?
- Can Backfill complete supported preparation paths using only admitted controls?
- Are protected allocation and escalation decisions still visibly the players' responsibility?

## Canonical sources

- [World files architecture](../../../pasm/spec/architecture/world-files.yaml)
- [Falling Skyway world](../../../assets/worlds/falling_skyway.toml)
- `src/tractor/`, `src/dock/`, `src/umbilical/`
- `src/console/repair/external_server.rs`
