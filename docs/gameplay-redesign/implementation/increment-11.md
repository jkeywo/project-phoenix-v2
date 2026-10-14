# Increment 11: component damage and restoration

Implementation contract adopted after increment 10 initial commit `22b31f57`.
Implementation is in progress. Supporting runtime commits do not establish
verification or a playable exit. Follow the later authorised pipeline: after
increment 10's initial integrated commit, its pinned validation and fixes can
continue while the next increment proceeds, with relevant fixes integrated in
dependency order. An initial commit does not establish a verified exit.
Increment 10's remaining evidence stays visible and is not discharged by this
document. Runtime work began after increment 10's initial commit. The concrete integration
choices below are now adopted implementation judgments; coefficients remain
authored tuning subject to evidence.

## Outcome and authority

Replace primary System percentage damage with compartment structure and component
conditions. Deliver meaningful capability loss, diagnosis, structural/component
repair, typed parts use and recoverable disablement together. A ship must never
become damageable under a model its crews cannot restore.

The selected requirements are [G057–60/G064 and S075–77](../system-review.md),
[AD37–55](../architecture/index.html#compartment-hp), the
[catalogues](../README.md) and the approved ordered roadmap. The corresponding
[PASM decisions](../../../pasm/spec/design/component-damage-and-restoration.yaml)
separate those selections from the implementation recommendations below.
Numerical coefficients remain explicit authored tuning, with balance evidence
required before exit. Keep existing networking, saves, System-addressed command
admission, Console scopes, individual System AI and optional GM operation.

## State and authored contracts

### Adopted initial integration vocabulary

Use `phoenix_model::damage` for closed serializable definitions and readouts,
and Gameplay `ship/compartment_damage.rs` for pure processing. `DamageType`
initially supports `physical` and `laser`, each with actual producers. A common
`DamageProfile { damage_type, penetration }` accompanies existing delivered hull
and shield payloads; it does not duplicate their amount or shield-bypass rules.
Author the profile beside each common weapon mount/delivery definition and carry
the selected tube's profile with its launched projectile and continuation.

`SystemInstanceConfig.damage_component` holds explicit mechanism-bound physical
targets. `DamageComponentConfig` contains `id`, a closed `operation`, `intake`
or `cooling` binding, susceptibility per type, positive `fault_dose_threshold`,
`degraded_factor`, full/optional patch remedy IDs and `field_replaceable`.
An operation binding affects its actual owning System; intake/cooling bindings
require a real energy-network endpoint. Generated shield arcs carry the same
definitions into their generated System exactly once. Component identity is
`ComponentRef { system, component }`; no ID-string inference or second controller.

Compartment definitions add `max_hp`, `recovery_fraction` and
`structural_remedy`. The entity's `damage.materials` table defines supported
type resistance and `residual_fraction`; `damage.remedies` defines bounded stage
work, structural HP per stage when applicable, typed actual stock/link
requirements and full/patch outcomes. Same-deck positive-area overlap is refused.
The initial material and conserved-deposition formula recommended below is
adopted. Deposited fault dose is divided among eligible occupants by authored
susceptibility, retained fractionally, and applied at amount thresholds rather
than one fault roll per simulation tick. Supported conditions start healthy,
damaged, destroyed or indefinitely patched; compartment disablement is a separate
source. Bindings combine conservatively, and structural restoration clears only
the compartment source.

`RepairWorkTarget` distinguishes compartment structure from an exact component.
Existing team/job state retains stage progress and reservations. Reserve a whole
bounded stage atomically from actual Infrastructure stock; consume on completion
and release unconsumed claims on cancellation. Full restoration is selected when
possible, otherwise an eligible stable patch. No stock copy authorizes work.
Initially one team may work each structural compartment task, while other teams
repair distinct components there. Intake faults alter actual incoming flow;
charged local reserves continue ordinary operation.

These choices adopt the draft's five bounded integration decisions. They do not
authorize unsupported condition scripting, arbitrary Workshop code, automatic
deliveries or new networking.

Physical-state integration uses a Gameplay ECS wrapper around the one pure
DamageState, re-exported by Simulation, so Gameplay mechanics consume the actual
owner without a reverse layer dependency or another mutable damage projection.
Repair assignment, recall and prioritisation are crew orders: their existing
ownership and individual AI remain meaningful when powered tools are damaged.
Admit those orders against the actual baseline controller and crew scope while
retaining GM disablement and stale-host refusal. Tool operation keeps ordinary
availability gates. Each remedy explicitly authors `manual_work_fraction` in
`(0,1]`; real team work can proceed at that fraction without powered tools, with
paid tool work accelerating the remainder. Access, finite parts and the same
bounded stage are required in either case. This prevents a zero-HP Repair room
from permanently making its own restoration impossible. It adds no alternative
Backfill policy, controller or portable energy store.

Use the increment 10 interior, System placements, stable IDs and routed teams.
Add no wall geometry, per-cell holes, precise component positions or second job
owner. A walkable connection is not a damage path.

| Record | Required meaning |
|---|---|
| Compartment definition | Existing rectangle/deck/material plus positive maximum structural HP and recovery fraction strictly between zero and one. |
| Material definition | Stable material ID, finite nonnegative resistance by supported damage type, and an authored residual-resistance factor in `[0,1]` at zero HP. Assigned only at compartment scope. |
| Damage component definition | Ship-local `(SystemId, component_id)`; supported mechanism binding, susceptibility and applicable remedies. Its compartment is the owning System's placement. |
| Structural state | Current compartment HP and the zero-HP disabling latch. Latch clears at recovery fraction; it reapplies only at zero. |
| Component condition | Stable target, supported kind/severity, source and necessary duration/progress/reapplication state. No component HP. |
| Repair target | Typed `Compartment(id)` or `Component { system, component }`; ordinary System/group orders may select these tasks through known evidence. |
| Repair commitment | Existing job/team identity, exact target/remedy, progress, actual parts claims/consumed resources and location. |

Most current fine Systems are not composed from physical component records:
`SystemComponentConfig` currently supports Dock, PowerInput and AccessBoundary,
while weapons, engine axes, scanners and shields retain dedicated owners.
Do not invent component references by parsing System IDs or treating arbitrary
Workshop keys as executable capabilities.

**Recommended narrow migration:** author explicit damage-component records on
existing Systems. Reuse composition component IDs for composed machinery. For an
otherwise indivisible existing capability, author one `operation` component bound
to that actual capability; it is a real condition/repair target, not a second
controller or HP pool. Add separate energy/coolant-link components only where a
real consumer resolves that link. Closed bindings identify which existing
operation, intake or cooling behavior is affected. Reject unsupported bindings
and missing coverage at resolved load. Parent adoption must settle the exact
serde placement of these definitions before schema work starts.

Every shipped equipped entity needs explicit structure/material profiles and
component coverage, including generated weapons/arcs, gates and passive
equipment. Template/include editing remains the existing Workshop document
workflow. `[interior.repair_placements]` and legacy Core/Science/Captain damage
buckets are removed, not renamed into fictitious equipment. Their former
structural durability must be allocated explicitly to authored compartments.

## Hit resolution and conservation

All actual ship damage producers call one Gameplay processor through their
Simulation adapters. Its input includes planar incoming direction, actual damage
type, penetration budget, payload budget and permitted aim information. Shields
first perform their existing funded interception/bypass split; material
penetration neither grants shield bypass nor repeats interception per room.

1. Select a deck using positive authored weights and the existing deterministic
   RNG. Ignore shot height. Stable deck order fixes weighted selection.
2. Produce a hit line on that deck from actual incoming planar direction. An
   ordinary hit samples the union of the deck's projected facing intervals,
   rather than always using the centreline or rolling hit chance again. Existing
   accuracy/spread must be incorporated once. Informed aiming uses only an
   acquired equipment compartment; no component position or hidden layout leaks.
3. Intersect rectangles, order segments by entry distance then stable ID, and
   resolve material cost by actual planar length. Treat geometry as physical in
   this increment: reject positive-area overlaps within a deck, with an
   actionable conversion error. Increment 10 allowed display overlaps; all
   shipped layouts must be checked before tightening this rule.
4. Deposit a conserved portion of the payload and spend penetration. Apply
   structural loss and eligible component effects from the deposited event.
   Persist exact residual/fractional state where future results depend on it.
5. At zero structural HP, attach the compartment-sourced disabling condition
   to its contents. Later damage can still affect them. Zero HP is not deletion,
   complete protection or permission to skip component damage.

**Recommended initial material rule, pending adoption:** freeze each
compartment's resistance for that hit at entry as
`resistance[type] * (floor + (1-floor) * HP/max_HP)`. Spend this per metre.
For positive initial penetration, deposit incident payload in proportion to the
fraction of initial penetration spent there; residual payload leaves with
residual penetration and is not copied into every room. A zero-penetration
payload deposits at its first encountered room. Non-structural payload effects
carry their own conserved supported dose. This supplies deep/light and
shallow/heavy variants without a second weapon implementation. It is a proposed
simple contract, not a selected balance formula.

Use the incoming projectile's final planar travel direction, beam direction,
collision relative direction or authored hazard direction as appropriate. Specify
a deterministic fallback for near-vertical, radial or directionless damage;
never quietly substitute the firing ship's later bearing. Retain the existing
meaning of per-target AoE damage: each affected entity receives its supported
incident payload budget, conserved across that entity's rooms. Do not replicate
the full per-target payload in every room or silently replace authored per-target
AoE damage with a shared budget across all entities.

Component effects are generated by deposited damage events and susceptibility,
across the room's eligible occupants without proximity weighting. Low HP does
not continuously reapply impact faults. Do not implement one independent fault
roll per beam tick or apply a complete fault to every component. Before coding,
adopt an amount-based dose/selection contract whose persisted accounting makes
equivalent continuous work insensitive to tick subdivision. RNG order, weighted
selection and fault dose belong in deterministic tests and continuation.

## Conditions and real capability consumers

Start with a closed supported palette sufficient for the migrated machinery:
degraded operation, unavailable/destroyed equipment and the separately sourced
structural-disable condition. Retain the selected EMP refresh and stacking
duration-resistance rules when an actual EMP producer/consumer is introduced;
do not add an unused EMP adapter here or turn temporary EMP into a parts-consuming
repair job. Include a stable
patch condition that reduces the supported capability and suppresses its old
fault's escalation without granting immunity to new damage.

Each effect must have a real consumer. Whole-operation unavailability gates
normal work without resetting saved intent or private AI. Partial link failure
affects actual incoming flow/cooling, while charged reserves retain ordinary
use. Efficiency changes affect the relevant cost/output once, not every layer.
Compose independent causes deterministically; removing a compartment condition
must not clear destroyed equipment, EMP, thermal trips or a GM latch.

Some selected faults may escalate or spread, others remain stable. Implement
only closed causal rules tied to an actual affected mechanism, with readable
evidence and an intervention path; do not add a generic arbitrary propagation
language, universal contagion or routine healthy-operation wear timer. Wider
specialized fault libraries may follow their owning capability, but each shipped
condition in this milestone needs complete operation, evidence and restoration.

## Restoration, materials and jobs

Extend `RepairTeams`/`RepairJobState` and the existing closed repair workflow,
not a separate component-task scheduler. One component has at most one active
team; teams may work on distinct components of one System and on structure in
parallel. **Recommended structural bound:** one team per compartment structural
task initially. Structural disablement alone is not an unsafe-access condition.
Use actual passage/access state and existing persistent routes.

Ordinary System/group priority commands remain useful; detailed controls may
choose a known component or compartment. Team selection/reservations prevent
duplicate task claims deterministically. Preserve work on recall, interruption,
handoff and save/resume where the physical remedy permits it; a recipe change
must not reuse incompatible progress or duplicate consumed parts.

Teams choose a complete repair/replacement when eligible and affordable. Only
when full restoration cannot be performed do they choose an authored eligible
patch. No eligible remedy means an explicit waiting/refused reason, not free
healing. Patches last indefinitely at reduced capability with no spread/expiry;
later full repair removes the patch and the repaired cause only. Most destroyed
components have field replacement recipes; authored irreparable catastrophic
loss is not a normal repair target.

Use existing `Stock`, `ResourceRequirement`, `reserve_requirements`, resource services and actual infrastructure
capacity endpoints. Author a small useful set of part types with finite stocks
and normal Umbilical restocking. **Recommended transaction:** reserve the next
bounded work stage's complete requirements atomically, debit at its irreversible
completion, and release unconsumed claims on cancellation. Already consumed
material/progress is retained. Missing parts pause work without losing team
identity. Structural and component recipes compete for the same real stocks.
No mirror repair inventory, free refill, distant provider search or automatic
internal delivery system is introduced; those logistics follow in increment 12.
Selected cannibalisation and broader salvage/processing remain explicit later
acquisition work; no compatible part is conjured from another System here.

Local operators see measurements, symptoms and known faults. Supported diagnosis
is observation-only, using time/access and existing reports; obvious failures
need no artificial delay. On-site teams reveal permitted component detail.
Repair AI schedules from the same reports and known work/stock constraints,
without querying all hidden conditions to select the optimal next target.
Completion reports distinguish restored components, restored structure and any
remaining known limitation. Existing provenance/age and explicit feeds remain.

## Disablement, defeat and compatibility

Remove migrated ship consumers of `SystemHull::is_destroyed()` as the trigger
for deletion, spectator state, command denial and destroyed events. A recoverable
ship can have lost useful propulsion and generation while local batteries still
support limited work. Define an explicit operational-status interpretation from
actual fitted capabilities; intentionally engineless facilities are not disabled
merely for lacking engines. Do not turn hidden truth into attacker knowledge.

Physical destruction requires a supported catastrophic cause. Preserve scenario
destroyed-event semantics by migrating their producers, selectors, counters,
debrief and defeat checks explicitly. GM destruction and supported scripted
catastrophe remain deliberate causes. Damage which formerly deleted a ship at
aggregate zero must instead produce the authored disabled/recoverable state.
Broader evacuation/capture and return-to-service personnel execution retain
AD53–55 but stay in their scheduled increments.

The integrated compatibility boundary is protocol 8, content epoch 3, snapshot
46 and simulation rules 0.18. Reject unsupported old states before any writes. Where old hull fields remain for asteroids or
non-equipped destructible props, retain that separate owner explicitly. A legacy
wire hull summary may be a derived compartment view only; it cannot drive new
damage, repair, availability or ship death. Remove the increment 10
`repair_placements` adapter before closing this milestone.

## Concrete owner entrypoints

| Owner | Integration |
|---|---|
| Gameplay `ship/interior.rs`, `ship/damage.rs`, `ship/composition.rs`, `entities/config.rs` | Validate definitions/coverage, compile planar geometry and supported bindings, own pure structure/condition rules and recovery. |
| Gameplay `jobs/repair.rs`, `jobs/repair_navigation.rs`, `modifiers/repair_teams.rs` | Typed exact work targets, retained progress, exclusive component claims and ordinary routed execution. |
| Gameplay `resources/stock.rs`, `resources/service.rs`; Simulation `resources/server.rs` | Atomic actual parts claims/debits and service resolution; extend real consumers rather than create new inventory. |
| Simulation `console/weapons/{beam,blaster,torpedo}.rs`, ship collisions, region hazards, GM effects | Replace every actual ship hull-damage entrypoint with the same directional processor; audit external repair and script damage too. |
| Simulation `ship/damage_sync.rs`, `server_app/system_status.rs`, machinery owners | Replace percentage-tier effects and availability with supported condition projections and consumers; preserve independent causes. |
| Simulation `console/repair/{server,dispatch,reports,visibility,external_server}.rs`, `console_ai` | Apply actual work/parts, report scope and ordinary individual AI; external restoration uses the same remedies. |
| Simulation `entities/ship_spawn.rs`, `snapshot`, `sim_digest.rs`, `crew_spectator.rs`, scenario/event adapters | Spawn one owner, preflight/capture all continuation, classify recoverable disablement and explicit destruction. |
| Model messages/interior/job types; GUI common status and Repair map | Typed target/status/read-view contracts, component details and structural jobs without unrestricted knowledge. |
| Content Preparation, existing Workshop Interiors/Composition/Resources/tree panels | Exact-source/history/include support, coverage/material/recipe validation, preview and disposable Test. |

These paths name current integration points, not an instruction to centralize
their domain responsibilities in Simulation or import sibling domains.

## Adoption decisions still required before runtime edits

1. Confirm explicit damage-component definitions/bindings for currently
   indivisible fine Systems, and their canonical serde location.
2. Adopt the material/deposition rule, hit-line accuracy integration and
   directionless/AoE fallback, including the new overlap validation boundary.
3. Fix event/dose accounting, the initial condition/effect stacking library,
   EMP adapter scope and any actual causal escalation case.
4. Name the minimal part types, bounded remedy stages, patch eligibility and
   structural recipe; values remain authored tuning, not implicit constants.
5. Specify fitted-capability disablement and supported catastrophe predicates,
   and enumerate affected scenario triggers/selectors. No aggregate-HP death
   compatibility rule may silently survive the migration.

These are implementation-contract choices for the integrator using the approved
design, not a request to reopen the gameplay interview. Record their adoption in
PASM and this specification before implementing their consumers.

## Verification and exit

### Integration notes in progress

The canonical physical owner is in Gameplay, with a Simulation re-export.
Typed component/compartment orders specialise the existing team Job rather than
creating another crew pool. A visit diagnoses only after physical arrival;
travel permission cannot authorise same-step repair. Each bounded stage claims
actual capacity stock, consumes it on completion and releases unconsumed claims
on recall or replacement. Continuation validates target/recipe compatibility,
unique task commitments and actual inventory claims before writes. A patched
component can later receive full restoration; it cannot receive repeated patches.

Permitted equipment details expose actual compartment structure and component
conditions, without synthetic System HP, private dose or causal source. Repair
map buttons issue typed work through ordinary System-addressed admission. Ship
HP zero remains recoverable. Current combat scenarios need explicit neutralisation
conditions for defeat; literal destruction remains a distinct event.

Existing collision budget becomes a blunt Physical incident with penetration
zero, using the direction captured before separation. Existing directionless
DamageZone budget becomes four equal local-cardinal Laser exposures with zero
penetration. Their offered shares sum to the old field budget and retain stable
region/RNG order. These hazard profiles are provisional adapter tuning. GM direct
damage distributes exact clamped loss over unique actual scoped compartments
through the canonical internal-incident processor; shared-room equipment shares
structural consequences.

Targeted evidence so far: Gameplay `cargo check -p phoenix-sim-gameplay
--no-default-features` passed after two route-reference type corrections. Client
map/status/work-list tests passed 16 tests. New native physical repair tests,
Simulation integration and the complete playable exit remain pending. Supporting
schema/content commits are preparation, not milestone completion.

Develop in vertical owner slices with targeted tests. Structural setup can land
first with existing behavior, but do not switch accepted ships onto new damage
until operation, parts-backed restoration and recovery are integrated.

- Every shipped entity/scenario loads with exact component/structure coverage;
  old `repair_placements` and System percentage-damage authoring are converted.
- Seeded penetration demonstrates direction, weighted deck selection, profile
  distribution, length/angle/material effects, weakening, budget conservation,
  repeated breakthrough and no precision component targeting.
- Beam tick subdivision, projectile arrivals, shield bypass, AoE, collisions,
  hazards and GM/script sources use the intended single accounting contract.
- Human and ordinary AI diagnose/triage, route, reserve real parts, restore
  structure/components in parallel, pause on shortages/blockers and recover
  capability. Patch, permanent catastrophe and transient EMP remain distinct.
- Prove Console isolation, reports without extra control, private AI continuity,
  stale-host rejection, GM latches and available local reserves during faults.
- Capture/JSON/restore/next-tick tests cover conditions, structural latches,
  doses/timers, jobs/navigation, stock claims, partial work and AI memory. Invalid
  targets, absent required state and impossible claims refuse before any writes.
- Workshop creates/edits material/HP/component bindings/remedies, undo/redo,
  inherited materialisation, export/reload, authoritative refusal, Preview/Test.
- Exercise Cruiser combat/non-combat activity and 2/4/6/8-seat journeys; record
  balance/performance and browser/native/manual acceptance separately. Keep
  existing transport and command boundary. Run repository integration gates once
  on the integrated result before pushing; this draft supplies no exit evidence.
