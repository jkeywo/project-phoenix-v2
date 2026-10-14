# Increment 12: automatic logistics and production

Adopted implementation contract prepared against main `c7909634`. Runtime starts
only after increment 11's initial integrated pin. No increment 12 runtime, native
validation or playable exit is claimed. The authorised
pipeline permits preparation and later implementation after the preceding
initial integrated commit while pinned validation continues. Earlier validation
and fixes remain required; an initial commit does not establish an exit.

## Outcome and selected scope

Make routine replenishment automatic and physical. Operators choose useful stock
targets, protected reserves, production priorities and recovery opportunities;
ordinary individual System AI executes those choices using actual supplies,
handling capacity, routes, machinery and time. A novice need not approve each
batch. Supply shortfalls create alternatives and communication, not a required
mining loop or an inventory grid.

The selected sources are [G046/G054/G065–67/G108 and S014/S089–91](../system-review.md),
[AD23/29/58–63/65–67](../architecture/index.html), the
[ship catalogue](../ship-systems.md) and the
[approved roadmap](../../../pasm/spec/roadmap/gameplay-redesign.yaml).
The [focused PASM slice](../../../pasm/spec/design/automatic-logistics-and-production.yaml)
distinguishes user selections from the adopted integration judgments below. Preserve the
existing transport, Console knowledge, System-addressed admission, per-System AI,
finite stores, thermal mechanisms and optional GM operation.

Included: hold-to-local targets, timed internal batches, compatible located
storage, destination-space claims, finite production, authorised docked target
replenishment, actual cargo/material recovery, compatible scrap processing and
small optional harvest sites. Inter-location deliveries use actual vessels;
internal batches are aggregate job state, not rendered cargo actors. This does
not implement a fleet traffic/economy planner, supply drones or new carried craft
before increments 19/21. G066's richer consignment hazards remain later content.

**Explicit dependency reconciliation:** G046's portable intact component/equipment
recovery is retained for increment 13, where fitting supplies its first real item
consumer. Increment 12 recovers actual resource cargo and supported materials;
it does not invent an equipment consignment inventory. S034 records a selected
lossy-recycling direction, but the approved implementation roadmap retains coolant
recycling as provisional. Do not add used-coolant collection, a recycler or an
assumed yield in this increment. Normal coolant consumption/replenishment stays
fully functional independently.

## Current owners and gaps

| Current owner | Reuse and necessary extension |
|---|---|
| Gameplay `resources/stock.rs` | One bounded integer `Stock` per actual capacity/magazine; source reservations are included in level. Add receiving-space claims as a distinct capacity concept: existing `reserved` means held source quantity and must not be reinterpreted. |
| Gameplay `resources/service.rs`; Simulation `resources/server.rs` | Resolve named services/explicit providers once, then transact actual endpoints. Existing temporary transfer views are not authoritative stores. Extend all competing transfer paths to honour receiving claims and preceding staged mutations. |
| Gameplay `infrastructure/condition.rs`; weapon magazine owner | Keep actual capacity/magazine quantities here. Stock locations and goals reference these owners; a new Logistics state must not mirror their levels. Keep tube-held/pending-burst occupancy and existing magazine claims. |
| Gameplay `jobs/mod.rs`, physical repair jobs | Reuse `JobId`, supported job lifecycle/workflow and interruption patterns. `TeamJobs` is an actual crew pool, not a generic delivery queue: general handling gets bounded non-personnel slots, without fake repair teams. |
| Gameplay `ship/interior.rs`, access machinery | Reuse compartment/connection IDs, stable route tie-breaking, committed crossings and physical boundary checks. Extract a small shared route-progress primitive only when both repair and delivery consume it; preserve repair behaviour. |
| Canonical local energy and physical damage | Pay actual machinery work from the owning System reserve; Operation reduces output for the same paid work, zero pauses it. Intake/cooling faults retain existing independent meanings. A destination stock target grants neither power nor immunity to damage. |
| Console registry/observations and closed state trees | Add fitted inventory/logistics, fabrication and supported acquisition Systems with explicit Console ownership, observations and admitted channels. Each has one ordinary AI; there is no central omniscient planner. |
| Content Preparation + Workshop Resources/Interiors/Composition/tree panels | Add exact-source editing/validation for stock locations, handling, goals and recipes alongside implementation. History, inheritance, preview and disposable Test remain the same workflow. |
| Simulation snapshots/digests | Capture goals, real transit quantities/locations, claims, partial work and private execution. Validate the complete candidate entity roster and endpoint relationships before any restore writes. |

At this pin there are no stock targets, incoming-space claims, internal cargo
batches, production jobs or supported salvage/mining System kinds. Current
`CapacityConfig` also represents non-cargo numbers such as berths and personnel;
never turn every named capacity into physical cargo merely because it is an
integer. Current repair stages consume source parts directly and coolant uses a
real central reservoir; neither is evidence of a timed internal supply network.

## Authored contracts

Add a small closed logistics definition through EntityConfig and the existing
System registry. Exact Rust/TOML names are fixed in the runtime implementation
spec before code; the relationships below are the review contract.

- A **managed store binding** names a real resource endpoint, resource type,
  owning/consuming System where local, compartment and main/local purpose. No
  endpoint may have two physical locations or conflicting resource identities.
  Existing non-cargo capacities remain outside managed storage.
- A **storage compartment profile** lists compatible resource types, finite
  shared space and explicit space cost per resource unit. Individual endpoint
  ceilings still apply. Stock already held by source claims occupies space;
  departed cargo does not. Receiving claims and actual stored occupancy share
  the same space limit. No arbitrary crate geometry is introduced.
- A fitted **inventory/logistics System** has authored general handling throughput,
  concurrent batch limit and supported batch size, plus its ordinary energy,
  damage, placement and Console definitions. It schedules transfers; it cannot
  change another System's goal or protected commitment. Existing admin-only
  Console summaries do not substitute for a fitted capability.
- A **local stock goal** belongs to its local System and names desired amount and
  an allowed main-store source binding. The hold's ordinary inventory controller
  knows delivered demand reports, supported own-store readings and route reports through explicit typed Console bindings.
  The local controller owns its actual local readings; no cross-Console pool or
  foreign-stock access is implied. A goal is a request, never a claim on unavailable stock or another operator.
- **Main-store goals/reserve floors/priorities** belong to Inventory. Ordinary
  replenishment/production must not draw below a protected floor. Explicit human
  changes follow its existing ownership/admission; no automatic raid on repair
  reservations or another System's loaded stock. Existing valid claims persist.
- A **recipe** has stable ID, closed input/output resource requirements, finite
  positive work time and an actual fitted execution capability. Production and
  processing share vocabulary, not one universal solver. Initial output types
  use real capacities and the existing canonical torpedo magazine; new missile
  payload families remain increment 15. Costs, yields, capacities and initial
  AI thresholds are authored tuning, not research-derived constants.

Do not introduce global market pricing, a currency conversion shortcut, fluid
simulation, per-item factory actors or unrestricted Workshop scripting.

## Timed internal delivery and ownership

A delivery Job holds its owner, source/destination endpoints, actual quantity,
current compartment or committed crossing, route/progress, receiving claim and
normal lifecycle/report state. Dispatch atomically removes quantity from the
source and creates that exact in-transit quantity while reserving compatible
receiving space. Refusal changes neither endpoint, transit nor claims. Queued
future requests reserve nothing.

Arrival deposits the batch and releases its receiving claim together. Every
ordinary loading/transfer/production path respects the same receiving claims,
including external umbilicals. A destination target counts its own incoming
committed quantities, avoiding duplicate deficit orders. Protected reserve floors
apply at dispatch; later edits change uncommitted work, not cargo already departed.

Use the existing deterministic compartment route costs and connection IDs.
Controllers plan from permitted route reports; execution checks actual access
before entry and reports encountered blockage. A committed crossing completes to
its endpoint under the established interlock, then waits/replans. No repair team
is consumed and no route job creates another AI controller.

Cancellation/stop does not delete cargo, restore it instantly to the source or
release the only accounting record. **Initial supported recovery:** retain
the batch at its physical location and request a routed return to the original
source. Secure compatible return space before releasing the original receiving
claim; if return is blocked/full, keep the quantity and report the obstruction.
Additional diversion/intentional disposal is not required for the first vertical
slice. Destroyed handling machinery pauses new work and progress under its
supported operating rules; it does not erase in-transit stock. Compartment damage
does not imply unselected per-hit cargo loss. Any eventual cargo-loss rule needs
an explicit physical cause and conservation evidence.

General handling slots count active deliveries, including blocked cargo; changing
priority affects waiting work and the next uncommitted stage. It never preempts
an active batch automatically. Authored bounds keep queues, active records and
reports finite. Manual movements use these exact same jobs rather than a fast
administrative transfer.

## Fabrication, processing and bounded acquisition

Suitable ships have a fitted fabrication System. Its controller maintains
approved output targets/priorities; humans can choose a supported recipe, stop
and reorder future work. At the next indivisible production step, secure actual
inputs, required output capacity and affordable energy together; failure leaves
no new partial claim, payment or progress. Explicit delivery/preparation jobs may
already have valid independent commitments. Time-sliced paid work and fractional
progress persist; tune step sizes and energy costs in authored data. Zero funded
work never advances production. Passive battery/heat rules continue while blocked.

For the first supported finite recipe, treat batch intake as an explicit
preparation commitment: when all inputs, output space and a positive affordable
first work step are available, consume the batch inputs into identified
work-in-progress exactly once and retain its output-space claim. Later funded
steps advance that same batch without claiming or consuming inputs again;
underfunding preserves it. The first attempt with no affordable work creates no
new preparation commitment. Exact step/partial-payment arithmetic must be proved
against the existing continuous local-energy API during implementation.
Completed output enters its real endpoint only through the reserved
capacity transaction. All required outputs must fit: do not silently discard one
product or advance only another. Stopping temporarily preserves committed work;
explicit recipe change discards consumed unfinished work, releases unused claims,
and keeps unused inputs/completed products where compatible space permits.
Capacity-changing configuration edits happen through supported refit/authoring,
not a free mid-mission storage swap. If a supported recipe change truly cannot
retain compatible stock, report explicit disposal of the excess; no silent loss.

The first usable production recipe makes existing torpedo ammunition from a small
explicit material set, with at least one genuine competition with repair supply.
It does not convert every finished part to any missile. Fabrication must remain
optional: carried ammunition and ordinary starbase replenishment still work.

Supported salvage targets expose actual remaining resource endpoints and optional
finite recoverable material stocks. Acquisition uses fitted tools, an observed
eligible target, physical range/access and real receiving capacity. Move source
cargo exactly once; processing consumes actual compatible scrap through a finite
recipe. Source depletion changes future recoverability. No kill-triggered random
loot, HP-to-infinite-scrap conversion or respawned deposit is allowed.

Add at most a small authored deposit/tool example to prove optional harvesting:
a finite source quantity, compatible output, paid work and safe stop. Existing
shipped missions must remain viable without mining. No prospecting minigame,
recursive production tree, new asteroid damage owner or mandatory harvesting
Station is needed. Portable component removal/fitting remains the explicit
increment 13 dependency above.

## Docked replenishment, controls and ordinary AI

Reuse the existing dock partner, transfer machinery and finite provider stocks.
At an eligible allied port, ordinary Inventory AI publishes approved replenishment
demand through explicit permitted feeds to the fitted Umbilical System. Its own
ordinary controller alone issues the admitted transfer start/resource/direction
controls, retaining that System's current human/AI ownership. Inventory does not
operate another System or install a parallel transfer planner; a human-operated
transfer System retains its normal decision/authorised maintain-target intent.
No remote supplier
search, free stock creation, instant inter-location delivery or control of the
provider's Systems is granted. Hulls without the required transfer equipment keep
their authored supported journeys; do not add a silent global refill command.

Port-owned permission is closed: `FreeAlliedReplenishment` or `ManualOffer`;
absence grants no automatic supply. Free-allied permission covers only approved
resource/quota requests, an eligible relationship and actual dock/transfer coupling.
The provider retains its finite stocks/rates. Tie permission and quota claims to
the docked partner/visit; withdraw permission on undock, release unperformed
commitments normally and retain actual completed deliveries. Never infer a
permission from a service binding or a visible stock count.

`ManualOffer` presents an automatically populated editable bounded resource order.
It creates no claims or spending until the current authorised human, or an
explicit mission order with the appropriate authority, starts it. Support authored
exchange-cost requirements only through actual typed stocks and physical transfer;
no currency or general price engine is required. Unsupported trade fails clearly.
Do not treat allied faction as unlimited spending permission, visitor control of
port machinery or foreign-stock omniscience. Provider capability/stocks, access,
dock duration and physical transfer time remain real constraints.

Keep the six-seat Cruiser as reference. Inventory and main-store policy live on
Engineering's existing Console, data-driven like other role assignments. Detailed
Engineering exposes main-store targets, reserve floors and resupply priorities;
simplest/middle delegate them without required preset setup. Local operators keep
their own local goals. Suitable fabrication/acquisition Consoles use authored
hosting/ratings and fit the existing 2/4/6/8-seat layouts without new claimable seats.

Human and AI read the same permitted stocks, demand/job outcomes and known route
blockages. A refused transfer does not reveal hidden provider stock or unseen
component faults. Use ordinary report paths for shortages, arrivals and work
blocked by access, resources or machinery. Co-hosted tabs do not pool knowledge.
Show local amount, desired amount, incoming quantity and shortage separately;
main-store available/protected/committed amounts must not be conflated.

AI is an authored reusable closed state tree for each newly migrated capability,
with bounded supported goal/priority/start/stop outputs. It reads its Console's
permitted views, not live foreign inventories. Backfill uses the same controller
and preserves goals/active jobs/private memory; no temporary-fill behaviour.

## Processing, continuation and failure boundaries

Keep existing fixed-step authority and deterministic RNG. Process contending
requests in stable ship UUID/System ID/job ID/endpoint order with defined ties.
Routing remains deterministic and never consumes new random numbers for a tie.

**Initial phase contract:** schedule logistics/production after existing powered
work and resource-transfer drains. Settle prior active work and arrivals, then
admit/start new bounded jobs; a newly started job cannot progress until the next
fixed step. Outputs become spendable by existing machinery on its next normal
step. New stages may not chain unbounded production or delivery through this
phase. Write explicit dependency edges for every conflicting real owner rather
than rely on Bevy registration/query order. Verify the exact schedule edges against
repair, coolant, magazine and physical-damage producers during implementation.

All endpoint debits/credits, input/output claims and local energy payment use
validated staged transactions and commit together. A transient transaction view
may clone owners; it does not survive the phase. Existing queued capacity effects
must be included exactly once, never overwritten by a later stale clone.

Snapshot/digest additions cover actual batches/locations/routes/crossings,
receiving claims, goals/floors/priorities, job IDs/outcomes, recipe and consumed
work-in-progress, fractional work, observation/report continuity and private tree
execution. Restore preflight validates the whole candidate roster, endpoint
identity/resource compatibility, exact incoming/source claim ownership, capacity,
route stages and recipe-progress invariants before any writes. Missing actors or
endpoints require supported retained-cargo recovery state; no vanished destination
may silently lose its batch. Advance affected versions and reject unsupported old
content/state with actionable conversion/version guidance.

## Small playable implementation order

Each row includes authored data, normal controls/AI, Workshop and continuation;
commit a reviewable working slice before broadening it.

| Slice | Concrete consumer and playable proof |
|---|---|
| 12.1 — One internal delivery | Explicit Cruiser hold ammunition → existing magazine, one general handling System, storage/location and receiving claims, real route/crossing, stop/return, normal manual order and ordinary AI. Existing tube loading and external replenishment still respect capacity. |
| 12.2 — Local goals and supply policy | Migrate actual repair supply and coolant service reservoirs to hold/local bindings, retaining current consumer service identities. Local targets trigger timed batches automatically; detailed Engineering chooses main reserves/priorities. Demonstrate blocked access, reserve rationing, simple-seat delegation and all supported hull layouts. |
| 12.3 — Finite ammunition production | One real fitted fabricator, torpedo recipe, shared repair-material constraint, paid local work, all-output admission, interruption/recipe switching and target-driven ordinary AI. Carried ammo and port resupply remain alternatives. |
| 12.4 — Recovery and processing | Recover an actual finite cargo/material source; one compatible scrap-processing recipe and one optional bounded deposit example. No reward stock independent of source state; fit tools only on appropriate ships. |
| 12.5 — Authorised port goals and integration | Ordinary allied docked replenishment, manually initiated editable authored trade offer, continued partial transfers and simultaneous stores/jobs/production. Audit all consumers, static actors and 2/4/6/8-seat workloads. |

For shipped migration, preserve stable resource/service/System IDs where practical.
Explicitly place managed endpoints in every affected resolved hull and provide
finite real provider stocks. Split existing resource totals between hold and local
stores rather than accidentally duplicate them. New feedstock/capacity/tool costs
are explicit intentional tuning, with a written initial-total/ceiling comparison.
Generated equipment, includes, World overrides and fixtures migrate together.
No automatic local supply exists on an unconfigured external pack: require a
supported new-version definition or preserve its stated supported capability.

## Acceptance and evidence

- **Conservation and contention:** source stock + actual transit + destinations
  changes only by documented consumption/production/loss; two competing batches,
  an umbilical and a loading tube cannot overbook the same supply/space. Protected
  floors and existing claims survive repeated updates. No hidden integer overflow.
- **Routes and stop:** a positive-duration batch cannot arrive instantly. Seal
  before entry, mid-crossing interlock, blocked return/full source, machinery
  loss and explicit cancel retain exact quantity/location/claims. Restore each
  frontier into a fresh host and continue deterministically.
- **Production:** energy/inputs/output-space unavailable causes no partial new
  payment/claims; successful work consumes once. Full secondary output blocks
  the whole recipe. Pause/resume retains work; deliberate change loses only
  consumed unfinished material and explicit incompatible excess.
- **Real consumers:** deplete a local magazine/repair store/coolant reservoir,
  observe shortage without losing intent, then arrive a real batch and resume
  actual launch/restoration/cooling. Damage Operation lowers output for the same
  paid work; zero Operation pauses it. No free fallback or per-tick test refill.
- **Recovery:** recover cargo once from an actual target, run finite scrap/feedstock
  processing, prove source depletion and receiving capacity. No-own-tool and empty
  sources refuse cleanly. An ordinary non-harvesting mission remains completable.
- **Authority/knowledge:** local-goal ownership, Inventory policy, stale host,
  explicit feed-only views, AI/Human equivalence, GM disablement and Backfill
  continuity. A trade cannot spend without initiation/limits. No unseen inventory
  or route failure becomes private AI truth.
- **Authoring and playability:** exact source/inherited materialisation,
  create/edit/remove ownership and endpoint bindings, undo/redo, save/export/reload,
  strict validation, preview and disposable Test. Human controls use correlated
  feedback, common status, readable phone/native layouts and normal delegation.
- **Whole-host evidence:** fresh JSON continuation during delivery and production,
  missing-target retained cargo, complete preflight no-write refusal and different
  local-ship peers with ordinary AI/full digests. Record bounded CPU/state size
  and combat/non-combat activity beside Cruiser balance, without weakening prior
  thresholds merely to accommodate new supply behaviour.

Use focused owner tests while developing, independently review source, and run the
required integrated gates once before a push. Keep automated results separate
from browser/native/manual acceptance and earlier pinned-increment evidence.
