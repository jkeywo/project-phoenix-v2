# Staff one actual facility service visit

**Adopted implementation contract, 16 October 2026. Runtime is not implemented.**
Root adopted the independently reviewed bounded increment 21/20 foundation,
including its explicit AI-only outside-provider boundary. It does not close
workforce, outside actors or increment 21.
See [increment 21](increment-21.md), the
[PASM decision](../../../pasm/spec/design/facility-service-workforce.yaml) and
[existing facility decisions](../../../pasm/spec/design/facilities-actors.yaml).

## Audited baseline and authority boundary

Source audit: `0c9f5be8e6c867fb82b675b800438e5fe1652c4f`.
Existing equipped ports already use actual provider Dock and visitor Umbilical
energy, finite stock/quota/capacity exchange, physical visits and the visitor's
single PortOrder/port_flow progress and receipt. `ports_server` supports several
coupled visitors and settles in stable visitor order. A general worker-present
flag would therefore let one team staff several visits at once.

`PersonnelRoster` owns stable PersonRef identity, actual officer condition and
custody. `DutyTeamConfig.team_idx` names an existing finite Repair team; it does
not create people. `RepairActivity`, TeamJobs and the interior cursor own that
team's exclusive work, travel and return. `WorldContentRuntime.workforce` owns
existing strike/disposition records, with no population, health or location.

Fixed equipment currently receives Systems, Consoles, energy, ordinary policies
and admitted-command storage through `entities/ship_spawn.rs::install`, but the
non-Ship path does not install the actual Repair teams or CompiledPersonnel.
Personnel admission, Repair ticking, AI and several dispatch consumers still
filter on Ship. Axiom has no Repair/personnel profile and has `team_homes = []`.
These are real installation/execution gaps, not reasons to invent a facility
worker pool or mark every fixed entity as a moving ship.

There is no current Human Repair authority path to the proposed fixed provider.
Network Human ControlSystem messages route to LocalShip in
`command_admission/mod.rs::admit_system_commands`. GM Station capability checks
are Ship-only; non-FleetSlot NPC interfaces currently admit only audited Helm
screens/kinds in `gm/operations/puppet/capability.rs::station_capability`.
An authored Repair screen or a Comms acceptance cannot bypass those boundaries.

**First consumer: an ordinary AI-operated outside provider and a Human or
ordinary AI visitor.** Its Repair Console is screenless/AI-only. This slice adds
no remote Human provider control, GM Repair interface, claimable Station, local
console token exception or new communication request. Later Human provider
operation requires its own real interface/producer/admission audit. Existing
Human Repair operation on playable ships remains unchanged.

## The actual worker requirement

Opt in one optional equipment-backed staffed port whose actual entity has no
Ship marker. Preserve shipped unmanned ports and omitted staffing behaviour. Its requirement is one finite compatible
team plus one actual named service lead posted to that team. Reuse the existing
`MechanicalRepair` competency on both the team and officer: this consumer is
physical Dock servicing. Do not introduce a synonymous service competency.

The officer is required by this optional service profile because the lead is an
actual finite, located person whose Fit/Minor/Severe condition already exists.
This is a selected specialist requirement, not a claim that every anonymous team
member now has individual health, or that every port universally needs an
Officer. It is not the officer's repair-work multiplier: that bonus remains
limited to existing internal physical repair. No new worker injury simulation,
condition roll, damage pool, or fabricated service-rate bonus is selected.
Existing authored conditions and actual treatment are sufficient first proofs.

The same ordinary Repair AI must produce the existing admitted posting operation
at the actual team home from its own permitted view; this is not an assumed
pre-posted fixture state. The same officer then remains `WithRepairTeam { actor, team_idx }`, located solely through the
existing team cursor. It cannot simultaneously staff a second team, receive
care, escort passengers, remain on a craft or give an offship bonus. First workers
are local-origin facility residents; foreign disembarkation and prize crews are
not introduced by this slice.

## One shared assignment per physical visit

Append a service-assignment activity to the existing RepairActivity enum,
preserving all existing serialized variant ordinals. It uses the same TeamJobs
mint, workflow, exclusivity, cursor and terminal-retention rules. Its immutable
scope binds the actual provider Repair owner, provider Dock System, visitor UUID,
existing PortVisit identity, selected team and stable service-lead PersonRef.

The ordinary Repair AI sees only its own explicitly bound staffing/visit and
local officer/team views. It chooses an eligible current visit using a
deterministic authored policy, first emits normal SetDutyOfficerPosting(Some)
for a compatible idle-home lead/team, then emits service dispatch on a later
frontier after actual posting is visible. Both use the existing AI emission/
admission path for that Repair System. No pre-posted worker or Human provider
action may substitute for this initial producer. The normal
consumer revalidates the complete observed visit and officer/assignment intent
before any claim or Job mint. UUIDs are internal references, not evidence of
permission; a remote request or foreign roster lookup cannot create readiness.
One active assignment per team, officer and actual visitor/visit is allowed.
The first profile has one service team, making concurrent-visit exclusion real.

Lifecycle: assign from the actual idle home, walk the accepted compartment route,
remain at the provider Dock compartment while assigned, then physically return
on recall or visit end. Entry/access, blocked routes and component conditions
use existing owners. Return remains possible through ordinary supported cleanup;
no terminal/release occurs before home. No separate service-work timer, provider
fraction, order queue or copied worker location is added. A service assignment
confers no completion credit for delivered goods. The existing PortOrder/flow
alone measures actual paid work and delivered stock.

Do not silently cancel another committed Repair/care/escort/refit job to staff a
visitor. The same exclusive free-team checks decide availability. The ordinary
Repair controller selects staffing through supported authored policy inputs;
no second facility controller or hidden staffing scheduler runs alongside it.
The same ordinary AI must recall its service assignment when that visit ends,
the lead becomes unfit, or the actual willingness policy refuses service. It
uses normal Recall, waits for physical home, then emits posting(None) to release
the actual lead. Ordinary local care can then select that same patient through
its existing admitted treatment path. This selected service cleanup does not
preempt unrelated committed jobs or create a general preemption mechanism.
A willing recovered lead can be posted and dispatched again only through the
same real home/assignment checks.

## Willingness and the atomic service boundary

The optional staffing definition binds the actual Repair System and uses the
facility's existing `infrastructure.workforce` association. An explicit authored
minimum disposition and the actual strike state govern this service only.
The selected World must contain that named register record. Unknown/unarmed or
missing required records fail closed for opted-in service. Existing unstated
associations and legacy ports retain their current semantics; do not globally
change the register's legacy unknown-group behaviour.

Simulation joins World willingness and Gameplay's physical worker state. Neither
branch imports its sibling or maintains a second register. Read actual register
records, not their scenario mirror flags. Strike and disposition changes use the
existing World mutation owner. First scenario setup/negotiation values are not
selected by this contract; run the required PASM design digest before reading or
editing a World for that design work.

At `resources/ports_equipment.rs::settle`, before staging power, progress or stock,
require the exact current visitor/visit and its one active staffing assignment;
the compatible actual lead must be Fit and WithRepairTeam on this provider; the
unpaused team cursor must be at the actual Dock compartment; the supported
workflow and normal equipment/fitting/Operation checks must permit work; and the
actual workforce record must satisfy the authored willingness policy. Derive
readiness from these owners, never persist a second readiness flag. Recheck at
the exclusive commit boundary or collect a same-tick derived view in complete
owner order with explicit stale/absent refusal.

The one visit-scoped assignment cannot fund a different visitor. Several offer
lines of its same visit continue through the existing serial PortOrder/flow;
there is no extra parallel staff work allowance or accumulation while paused.
Existing positive machinery costs, atomic pay_pair and finite exchange rules stay
intact. Unready workforce moves no goods, charges no service work, and advances
no service progress. Existing fractional progress, real goods and quota remain
in their current owners. Independent passive energy loss still occurs normally.

Strike, loss of fitness or a blocked journey does not damage machines, despawn
people, teleport the team home or roll back already received goods. Physical
readiness can resume only through a still-valid on-site assignment or the
ordinary return/release/reassignment lifecycle. An injured lead needs the same
ordinary AI recall, return/release and real care before re-posting;
there is no remote cure. Actor destruction uses existing actual-holder loss and
ordinary destruction events. Offship survivors retain their sole real custody.

## Minimal ownership and authoring work

| Owner | Required join |
|---|---|
| Simulation installation | Factor existing team/personnel installation out of the Ship-only branch and invoke it once for an entity actually declaring accepted Repair teams/personnel. Keep stable origin/profile acceptance and the same team count. |
| Simulation Repair | Extend owning admission/tick/AI/publication/dispatch seams to actual capable fixed entities, requiring their real System, sources, modifiers, layout, condition and energy inputs. Do not broaden all targets or add Ship markers/propulsion. |
| Shared jobs and personnel | Add the single assignment variant and supported intent/recall to the same finite owner; keep actual officer posting and cursor-derived location. |
| Ports | Add an optional authored staffing requirement and same-current-visit gate to existing service settlement; retain PortOrder, stock, quota, receipt and payment ownership. |
| World/Simulation | Validate required consent association for the opted-in consumer; join the existing record through a pure supported readiness input. No new World workforce population. |
| Workshop | Accepted typed composition, same existing document/history, actual team/home/route/Repair/Dock/officer/competency/consent references, feeds and ordinary AI policy. Preview/Test refuse broken references before running. |
| Persistence | New activity/intent joins existing snapshots, restore and digests once. Whole-candidate checks bind actual team, officer custody, route/phase, Dock placement, visit and consent source; absent required state is not silently defaulted. Root owns epochs and central schedule/census joins. |

The first optional asset must supply the complete accepted equipment/energy and
interior profiles, finite teams, connected homes/rooms, actual service stock and
ordinary Repair policy. If demonstrating condition recovery, it also includes
the existing accepted local care capability, compatible finite team, medication
stock, and real replenishment/service path. A condition change alone cannot
stand in for treatment. Use the existing canonical Repair owner required by
CompiledPersonnel; multiple arbitrary personnel controllers are not added.
Assert that actual accepted installation does not add Ship, propulsion or
ProbeCustody. A base with behaviour/static point defence would exercise the
existing Ship path instead of proving this fixed-capability migration. Preserve the base model's atomic-root merge semantics;
a placements-only replacement cannot erase inherited decks, compartments or
machinery. Its provider publication gives the visitor only an attributable
service wait/refusal, never the provider's private roster, health, reserves or
unearned current location. No raw UUID/Job/visit identifiers belong in player UI.

## Required evidence and boundaries

The independently reviewed AI-only provider scope is adopted. Split the
structural fixed-capability migration from the staffing behaviour where possible.

The real consumer proof must include a Human visitor using the normal Dock and
Umbilical path and an ordinary AI provider selecting/dispatching staff through
its actual Repair owner. Verify same finite people/team travel, one visit at a
time, positive existing paid work, conserved finite goods, no free work while
walking or blocked, and ordinary return/reassignment. Prove a team already busy
with repair/care cannot also staff; use actual condition/care and existing World
willingness changes, not injected readiness or invented damage.

Save/JSON restore must compare complete captures and digests for at least 360
steps at walking, actual service and returning frontiers. Malformed candidates
must refuse before writes: wrong/duplicate team or officer, conflicting job,
wrong room/route/owner/visit, stale intent, forged condition/required consent
binding, missing state and cross-visitor service credit. Include provider/visitor
loss, ended visit and retained offship personnel. Source fixtures must preserve
ordinary publication/resources and supported local authority on both hosts.

Targeted native tests, actual accepted-loader/Workshop Test, browser/native,
multiplayer, performance and broader Human/AI workforce acceptance remain separate
recorded evidence. No tests or runtime are delivered by this proposal. General
population simulation, foreign worker reception, remote Human staffing, labour
negotiation content, construction/economy, and full increment 21 remain open.
