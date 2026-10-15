# Increment 23 — Continuing modes and persistent mission consequences

Approved future plan, 14 October 2026. Full completion follows [increment 22](increment-22.md)'s
verified playable exit. The 15 October [parallel-work contract](parallel-work.md) permits implementation after its initial integrated prerequisite; each mode retains its own playable exit.
See [architecture decisions](../architecture/README.md#mission-planning-increments-22-24)
and [PASM design](../../../pasm/spec/design/gameplay-redesign.yaml).

## Playable exit

Authored Patrol, then War, then Sandbox use mission planning across encounters,
retaining real assets and obligations through transit, servicing and save/resume.
These authored examples establish acceptance cases for later opportunity search.

## 23A — Persistent commitments and entity relationships

Bind plans/situations to stable entities and durable outcomes. Ownership/control,
custody, objective participation and factual history retain their authoritative
owners. Plans refer to that state rather than copying inventories or conditions.

Distinguish tentative proposals, revealed offers, accepted commitments and completed
outcomes. Track ongoing escort, passenger and repair demands in future scheduling.
A captured vessel retains actual damage, equipment, crew requirements and capacity
after its originating situation finishes. Revalidate shared assets: incompatible
transport assignments cannot both claim the same vessel's capacity.

Persist bindings and commitment transitions through existing save infrastructure.
Cancellation follows physical work rules; it does not teleport passengers, refund
consumed stock or remove a ship. Avoid duplicate completion effects after restore.

**Exit:** a situation ends while its vessel and obligations remain available to a
later situation; shared-resource conflict and restoration cases are demonstrated.

## 23B — Authored Patrol demonstration

Author a security encounter with capture, negotiated and refusal outcomes. Provide
alternative opportunities to return the vessel to port or use it for civilian
evacuation. Both require actual preparation, transport and reception. Account for
unavailability, destruction and abandonment, including passengers already aboard.

These branches are authored, not generated. Reveal opportunities through ordinary
observations, Comms and optional objectives, with plausible sources of information.
Do not add an omniscient opportunity board. Acceptance is a commitment, not an
automatic command to operate human-owned Systems. Refusal leaves a coherent mission.

**Exit:** capture, service and reuse a ship across encounters; complete either
return or evacuation, and resume during transport without changing asset identity.

## 23C — Authored War

After Patrol passes, introduce competing commitments, faction objectives and causal
service dependencies using the same contracts. Outside actors resolve or change
needs through actual work. Reconcile changed needs with existing independent
objective-instance membership and completion-credit rules; a world change alone
does not grant every crew objective credit.

**Exit:** an outside actor resolves a need before the player arrives; the mission
updates coherently, physical state is conserved and completion credit is correct.

## 23D — Authored Sandbox

After War passes, support persistent actors and assets with optional activities and
no compulsory overarching mission sequence. Retain servicing, transit, factual
history and debrief. Report completed, declined and partial outcomes only to the
extent known to the crew. A lack of accepted work is valid, not an execution error.

**Exit:** players can refuse offers, keep or transfer assets and continue play with
consistent obligations and history through multiple sessions.

## Authoring, acceptance and handoff

Workshop supports authoring continuing-mode goals and bindings, inspecting durable
commitments and testing the Patrol/War/Sandbox sequences through ordinary document
history, validation, export/reload and disposable Test. Inspect entity references
and ongoing work separately from private situation progression.

- Resume before/after capture, offer, acceptance, embarkation and delivery.
- Verify passenger/resource conservation, ownership and custody transitions.
- Test conflicting commitments, unavailable ports, destroyed vessels and abandonment.
- Verify outside-actor work changes needs without duplicate completion or credit.
- Preserve permitted knowledge, command admission and ordinary individual System AI.
- Compare combat and non-combat Station participation across supported crew sizes.

Each mode is a separate playable exit, not three unverified features delivered at
once. Include runtime, authoring, player feedback and deterministic continuation
with each. Use targeted tests and human playtests, then the established integration
gates. Hand the working examples and their outcome expectations to
[increment 24](increment-24.md), which discovers combinations rather than requiring
a bespoke captured-ship evacuation template.


## First23A consumer: bind acceptance to the existing promise ledger

Adopted 15 October 2026. An optional `planning.required_commitment` names one
existing World CommitmentLedger promise. Before the first root choice, the exact
unique record must be Open with a valid made-at clock. A normal Comms dialogue
response records that promise through its existing admitted callback/effect path.
An offered or declined dialogue does not record acceptance and does not start work.
There is no new promise store, resource claim, physical executor or forced Human
command. Existing plans without this field keep their current behaviour.

The accepted PlanChoice retains the immutable required promise ID. Full-candidate
preflight checks that source binding against the same saved ledger record, its
original made/resolved clocks and the choice tick. Later Kept or Broken resolution
does not erase the root, alter its deadline or abandon ordinary physical work.
Only explicit authored Situation outcomes may request cancellation through the
actual owning mechanism. Actual Scenario evidence can explicitly keep or break
the same promise via the existing buffered callback; script success is not itself
proof of any physical completion.

Preserve missing-root protection for a launch-resolved actor. A dormant root may
lack its required promise, or await the collection pass after a promise made this
same tick. A promise made and resolved within one tick can close without a root.
An earlier Open interval spanning collection requires the retained choice; later
resolution cannot explain away an erased accepted plan. Unresolved actors retain
the existing delayed-binding behaviour. Validate unique referenced records, closed
Open/resolved clock shape and nonfuture clocks before writes, including malformed
whole-candidate restore.

Expose the bound promise ID through existing privileged Scenario inspection and
edit the optional source field through Workshop's existing validated root/history
workflow. Prove ordinary Comms accept/refuse, no pre-acceptance instruction, real
owner work, explicit outcome resolution and fresh JSON continuation. This bounded
consumer establishes acceptance/history plumbing. It does not deliver proposals
for multiple roots, physical capacity reservations or the Patrol capture/transport/
reception demonstration, which still needs the actual20/19/21 custody consumers.


The first source consumer `committed-assistance-world.toml` presents an ordinary
scripted Comms offer with explicit zero AI choice weights. Only a normal Human
response records the existing promise; decline leaves the optional root dormant.
The real ReportStatus and internal Repair operators use the same imported domain
contracts as22. A positive actual Repair achievement explicitly keeps the promise.
The native probe delegates Comms through normal Station release, resumes real
partial reading from fresh JSON, introduces a later incident through Damage only
after its Repair instruction exists, and compares1200 complete updates with
forged-ledger no-write checks. These tests await integrator execution. This is
accepted assistance rather than Patrol capture/evacuation: no custody, capacity,
passenger transfer or operational ownership transition is claimed.
