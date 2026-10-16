# First whole-vessel acquisition contract

Adopted, 16 October 2026, with the acquisition-time and doctrine clarifications
below. Source audit: `7f99ab79`; implementation starts from `b243bcc6`. It narrows the
next physical consumer needed by [increment 20](increment-20.md) and
[increment 23](increment-23.md); it does not close either increment.

## Deliverable

An explicitly eligible, disabled non-player NPC can be acquired by completing a
real paid external Security **Board** Job. The acquired object keeps the same
`EntityUuid`, physical body, damage, equipment, stocks, crew records, access
boundaries, Console records and individual System controllers. Acquisition changes
the whole vessel's operational owner. Useful boarding work and the team's later
return are distinct outcomes.

The first authored target must have a genuinely absent or empty accepted personnel
profile and an absent or empty actual roster origin. A populated profile does not
become empty because its officers are unavailable, Lost or aboard another craft.
This avoids inventing a prize crew or cooperation. General crew-dependent capture
remains open and must leave an acquired vessel non-operational until its real
staffing and consent contracts are satisfied.

Use an optional, accepted externally capable Security profile for this first
demonstration. Current local Security profiles replace the external target pool
and use local routes. Combining local and external work in the same team owner
remains separate work; the Cruiser must not silently gain a second boarding
controller or lose its existing local jobs.

## Owned types and boundaries

The following are proposed types, not existing runtime APIs.

| Owner | Contract |
|---|---|
| Model | `VesselAcquisitionCause::SecurityBoard { scope: ActivityJob, team: u32, target: String, work_completed_tick: u64 }`. The target equals the acquired entity; the scope names the real issuing actor, Security System and Job. No synthetic Job or Scenario flag is evidence. |
| Session | `VesselOperationalOwner { controller: String, faction: Uuid, acquired_tick: u64, cause: VesselAcquisitionCause }`, retained on the acquired entity. First acquisition only: an existing owner refuses a replacement. The faction is frozen from the actual acquiring actor at acquisition commit; `acquired_tick` equals the cause's `work_completed_tick`. |
| Gameplay | The existing Security Job retains its optional achieved acquisition cause while `returning_outcome` and eventual `Job.outcome` retain the actual return frontier. Its original target and Job identity survive withdrawal. This adds no second team clock. |
| Content | An optional closed `AcquireVessel` success effect on an authored Board action. Other Security actions cannot select it. Existing Board actions and `outcome_flag` consequences retain their present meaning. First-version authoring requires disabled-target and empty-personnel eligibility; unsupported modes fail validation. |
| Simulation | An exclusive adapter prepares the physical and Session candidate, then commits ownership with the actual Security success transition. It resolves live owners and invokes their existing contracts; Gameplay, World and Session acquire no sibling imports. |

`FactionComponent` becomes an effective projection for acquired vessels: use the
frozen faction in `VesselOperationalOwner`, otherwise the accepted original recipe.
Do not persist an independently editable second faction selection. The canonical
ownership record must be restored before faction-dependent consumers run.

`FleetSlotOf`, `AuthoredShipSlotId`, `FrozenShipSlots`, `LocalShip`, Session tokens,
Station tenure, ratings, Console hosts and `ShipCrewPresence` do not change.
Acquisition does not grant a human token machinery rights. The ordinary network
route and System admission gate remain intact.

## Preconditions and acquisition transaction

At dispatch, the existing admitted Security command must identify an actual
installed, available team and an offered Board action. The capture-specific offer
requires the issuing Console's permitted contact information; an authored target
UUID or a private World lookup does not supply detection. Existing reach,
deployment, paid work, Operation damage and interruption rules still apply.

Before the useful-work success transition, prepare all of these without writes:

1. The same real Job, actor, Security owner, team, target and authored acquisition
   effect still agree. Its positive work was granted by the ordinary power owner.
2. The target remains a supported physical vessel, actually disabled under the
   existing damage/power defeat rule. A historical disablement notification alone
   cannot establish eligibility. Do not cut power, alter HP or repair it to make
   the check pass.
3. It is outside player authority: no `LocalShip`, `FleetSlotOf` or
   `AuthoredShipSlotId`; no occupied crew presence, Human System source or GM
   Station puppet. A vacant player slot remains a player vessel.
4. The actual acquiring actor has a supported registered faction, and the target
   still meets its accepted empty-personnel restriction. Its existing owner is
   absent. Any compatibility or required ordinary controller transition has been
   checked before mutation. These are acquisition-time checks. Later legitimate
   GM puppeting, crew presence, System control-source or actor faction changes
   cannot retroactively invalidate the recorded acquisition.

Commit the prepared Session owner, its effective faction, the Security achieved
cause and the same Job's ordinary withdrawal together. The existing Security
write phase must not first publish Completed or a success flag and later discover
that ownership cannot be committed. Refusal records a closed unsuccessful result
and returns the same team through ordinary physical rules; consumed work is not
refunded. The terminal Job outcome is written only when that team actually arrives
home. Recall before success cannot acquire a ship.

Concurrent successful candidates use the existing deterministic actor UUID, team
and Job ordering. After the first commits, later contenders revalidate and refuse;
they do not replace its ownership or duplicate completion credit. The first slice
refuses recapture rather than maintaining unbounded ownership history.

## What the acquired owner can do

Ownership is the Session authority for later ordinary remote-order admission; it
does not bypass a target System's AI, knowledge, equipment, resources or readiness.
The first authority slice must visibly identify the actual controller and reject
foreign attempts at unsupported direct machinery control. Remote-order execution
is a remaining consumer, not an implied ability of this record.

Effective allegiance must reach existing hostility, sensing, Comms and service
consumers. Revalidate stale AI selections before further weapon admission. Preserve
already emitted projectiles and other physical consequences; changing ownership
does not erase hazards, clear all work, open doors or refresh observations. Capture
does not make private compartments or enemy Console records universally visible.

The chosen first autonomous target must explicitly bind a compatible
post-acquisition doctrine, such as an actually supported hold directive. Prepare
and apply it through the existing `NpcDoctrineControl` transaction together with
acquisition, so power recovery cannot revive its original hostile objectives.
Actual hardware/configuration still determines operation. Do not introduce a
non-operational latch, blanket AI-off state or second controller. Originally
populated personnel profiles remain unsupported by this first effect.

Loss of the acquiring actor leaves the recorded acquisition and frozen faction
intact. It does not revert the asset, destroy its crew or transfer it to a new
player. Future owner succession and recapture require their own supported actions.

## Continuation, receipts and proofs

Add the optional canonical Session owner to the existing entity snapshot,
application, deterministic digest and authoritative-state census. Rebuild its
effective faction before ordinary readers on fresh restore. Declare the owner as
Folded and its effective faction as a Derived projection; migrate the former
immutable-recipe assumption instead of declaring two authoritative factions.
Keep the receipt in
the actual Security Job and ownership cause; World may bind to those exact facts
later, without copying custody or treating team return as the acquisition event.

Whole-candidate preflight checks the accepted target capability and empty-personnel
recipe, registered frozen faction, exact actor/target/owner/Job/team tuple and
causal clocks. When the original Job remains retained, it must agree with the
cause; one physical Job cannot name conflicting acquisitions. Actor removal after
the recorded event does not invalidate history. Current crew, GM puppeting,
control sources, actual disablement and current faction equality are not replayed
as historical eligibility checks. These are consistency and
provenance checks, not a claim that arbitrary coordinated rewriting of every
historical fact can be authenticated without an action ledger.

Required actual-consumer evidence:

- Paid partial boarding followed by JSON restore and at least 360 complete
  capture/digest comparisons, including success while the same team is returning
  and later home settlement. Repeated success/restore cannot reacquire the asset.
- Same entity identity and conserved physical damage, equipment, finite stock,
  crew, access and observation clocks before/after acquisition. No seat or
  machinery admission changes; hidden records stay hidden.
- Target repair/recovery before success, range/loss/destruction, issuer disablement,
  recall, occupied/GM/player targets and simultaneous contenders. Owner loss
  after success preserves the recorded asset.
- Malformed owner, target recipe, faction, Job/team, cause and clock candidates
  refuse before any write. Fresh restored faction and normal AI admission agree
  on peers with different `LocalShip` projections.
- Workshop edits and validates the opt-in action/profile, preserves ordinary
  document history, previews the real effect, and exercises it in disposable Test.

## Remaining joins

Negotiated surrender needs an actual normal Comms request and receiving decision,
with a separate typed consent cause calling the same Session ownership transaction.
It must not fabricate a Security Job or assume crew cooperation. Actual prize-crew
custody, foreign room access, staffing, remote orders and service/reuse remain open.
Existing pod reception keeps people `AboardCraft`; it is not one of these grants.

No new planner primitive, captured-vessel Patrol substitute, continuing-mode exit
or Director pacing is included. Those wait for the physical and permission
consumers they need.

## Existing source seams

- `crates/phoenix-simulation/src/security/server.rs`: external target pool,
  paid Board work, useful-work transition and withdrawal.
- `crates/phoenix-sim-gameplay/src/jobs/security.rs`: real `TeamJobs`, JobId mint,
  `returning_outcome` and home settlement.
- `crates/phoenix-simulation/src/command_admission/mod.rs`: existing local human
  route and per-System admission; no foreign machinery route is implied.
- `crates/phoenix-simulation/src/gm/operations/npc.rs`: compatible ordinary
  doctrine replacement and retained state.
- `crates/phoenix-simulation/src/ship_slots.rs` and
  `crates/phoenix-simulation/src/lobby/crew_replication.rs`: launch identity and
  current crew authority that acquisition must preserve.
- `crates/phoenix-simulation/src/ai/server.rs`,
  `crates/phoenix-simulation/src/world/server/effects.rs` and
  `crates/phoenix-simulation/src/resources/ports_server.rs`: effective faction and
  hostility/service consumers.
- `crates/phoenix-simulation/src/snapshot/schema.rs`, `snapshot/application.rs`
  and `sim_digest.rs`: continuation, projection ordering and deterministic state.

The first optional autonomous target selects a base-World palette entry whose
nonempty doctrine contains only the existing `AiDirective::None` hold objectives.
Hardware compatibility alone is insufficient: a hostile Destroy objective is not
an admissible post-acquisition replacement in this bounded slice. This does not
introduce an AI-off latch, change System sources or disable machinery.
