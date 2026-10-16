# Increment 15 — bounded physical marking payload

Status: proposed contract; no runtime adoption or implementation claim. Based on
`efcddb08`. Root approved preparing this contract, including review of contact,
attachment projection and report timing by Root, Craft and Comms before runtime.

## Selected scope and open choices

S012 option A in `system-review.md` and the accepted physical-payload decision in
`pasm/spec/design/layered-combat.yaml` select physical observation-producing
beacons. Increment15 requires finite vulnerable equipment, actual subject-relative
placement, paid operation and a working report link. It does not select a generic
accuracy bonus, hidden interior access or permanent perfect tracking.

Exact measurements, endurance, link, removal and countermeasures were left open.
The bounded choices below are proposed implementation contracts, not quotations
of settled tuning. Equipment-specific penetration is separate. General mines,
terrain manipulation, Bay handling, recovery and cleaning are outside this slice.
No new authored entity enters the global catalogue before an actual accepted
`resolve_from_disk` → `EntityConfig::from_toml` regression accompanies it.

## Existing owners and missing joins

| Responsibility | Existing owner | Proposed extension |
|---|---|---|
| Finite round and loading | `weapons/torpedo.rs`, actual magazine Stock and tube/burst | Optional mark-only delivery frozen with the round; no beacon inventory |
| Interception and impact | Existing PDC/torpedo lifecycle, stable collision order | Surviving actual contact can create one body; interception cannot also deliver |
| Equipment installation | `entities/spawner.rs` and `ship_spawn.rs` | Use the same accepted configured entity installer |
| Runtime reconstruction | `EntitySpawnOrigin`, template/content ledger, snapshot preflight | Share accepted runtime equipment provenance, rather than pretending a script spawned it |
| Pose | `ship/physics_systems.rs` physical synchronization and existing corrections | One explicit rigid-constraint consumer for attached bodies; no independent motion integrator |
| Paid measurement/history | `science/broad.rs`, actual BroadTracks | Restrict this instrument to its attached subject; retain existing error, cadence and provenance |
| Report and receipt | Actual Radio payment/route; existing radar sample export/receipt | Optional paid radio-backed export of original samples into existing recipient BroadTracks |
| Damage and endurance | Actual ShipPowerSystem and ShipDamage | Ordinary fitted local batteries, Operation and compartment damage |

`physical_probe.toml` demonstrates ordinary finite equipment. It is not a marking
consumer: its accepted profile is fixed, it has no attached location state, and
`battery_probe.toml` has neither a Radio nor a sharing configuration. Carried
custody deliberately disables active work and must not be used for attachment.

Existing `science/sharing.rs` preserves original sample identity and history and
checks range/jamming/equipment availability. It does not prove actual paid radio
work. Only the new opted-in report path gains the stricter physical Radio gate;
legacy sharing is not migrated by this change.

## Delivery, accepted recipe and real contact

An optional mark-only payload on the generic Guided delivery references an
accepted battery-equipment recipe. It is per actual tube, not a new global
TorpedoSystem mode. Ordinary controls, target selection, ammunition claims,
loading, barrel pattern, launch work, guidance and PDC remain their existing owners.
The initial optional reference fit changes one tube; shipped hulls remain intact.

Freeze the accepted payload recipe identity and actual launch faction with each
committed burst/round. Source removal cannot erase the payload or silently change
its equipment/faction. A frozen profile containing a recipe reference may require
Clone instead of Copy; do not replace it with a source-only lookup that fails
after launcher loss. Follow the actual accepted dependency through native/browser
preload, pack validation and frozen content closure. A runtime file lookup or a
private JSON merge is not accepted equipment construction.

First eligible subjects are ordinary planar physical ship hulls with supported
collider geometry and ordinary shield arcs. Asteroids, invisible markers, missiles,
Carried/Disabled bodies and another attached beacon are not attachment subjects.
Actual collision eligibility, not the launch target UUID or allegiance, chooses
the struck subject. Friendly/civilian impact has the same physical consequence.

The marker has zero damaging/EMP payload and zero proximity detonation radius.
The actual collision owner must verify contact against the current supported
hull geometry; a loose bounding radius or ordinary explosive proximity event
alone is insufficient. Root/Craft review must settle the existing collision
query/shape seam before runtime. Do not quietly inherit the usual five-unit
explosive radius or invent a hull hit from acquired target intent.

A currently raised shield arc stops the marker; it consumes the actual round
without spawning equipment or applying damage. A successful unshielded contact
consumes the surviving round once and places one real body at that actual contact
point with a finite subject-local offset. Detonation, interception, expiry and
restore cannot deliver it again. Unsupported/missing recipe or subject refuses
attachment without a substitute body or resource refund. Marker identity is a
deterministic child of the consumed round, not a second mutable ID ledger.

Install the real body with ordinary Systems, Consoles, accepted energy, damage,
interior, faction and runtime spawn provenance. No station is added. The accepted
recipe is engineless and crewless, has no Dock/Tractor/Bay/fitting actuator, and
contains only its supported SensorRadar and Radio equipment. No second stock,
controller or player/AI simulation variant is introduced.

## One physical constraint owner

An explicit accepted attachment profile permits one canonical location state:
Attached(subject UUID, subject-local offset, attached tick), or Detached. Only
physical contact creates Attached. It is mechanical state, never a public target
handle, observation or machinery command. First offsets follow planar subject
yaw with fixed relative height; this does not add capital-ship 3D control.

The existing physical pose-synchronization owner gains a reusable rigid-constraint
projection. For an attached body that projection is its sole pose writer:
there is no Helm integrator, low-LOD motion, Dock/Tractor writer, independent
velocity integration or thrust. ShipPhysics/Transform are the physical projection
used by current collision, sensing and presentation consumers, not another
independent position intent. Child motion cannot move the subject or spend free
propulsion on its behalf.

The same projection must run at the physical synchronization boundary before
Rapier SyncBackend, and after existing Modifiers pose corrections (including
TickDock) before measurements/reports use the body. This is one shared physical
constraint algorithm at the two required boundaries, not two body integrators.
The exact integration seam and sanctioned writer/census change need Root/Craft
review. Reordering the entire existing physics/damage/Dock graph is not implied.
If this cannot be joined coherently, record that dependency rather than adding a
second independently scheduled pose writer or measuring from a different pose.

Actual subject removal detaches the real body at its last physical pose and
withdraws new subject-only measurements. Missing observations or a broken radio
link never mean the subject was removed. Detached equipment stays an ordinary
vulnerable finite body; it does not acquire a new target, free drift or recovery
mode. Beacon destruction follows the normal actual removal path. No lifetime
counter duplicates energy: exhausted equipment remains inert physical hardware.

## Limited observations and actual Radio reports

The real SensorRadar pays its existing continuous work and respects Operation,
actual range/coverage, signal attenuation, uncertainty and sample cadence. Its
first supported question is attached-subject position and observed velocity.
Restrict its physical measurement input to that subject; no surrounding contact
sweep, identification, faction truth, assessment, component coordinates, interior,
shield frequency or live private UUID is granted to a controller or report.
Normal contact handles, maturation, age and confidence apply. Two real samples
are needed for observed motion. The attachment does not fabricate observations.

An ordinary same-faction compatible recipient SensorRadar can deliberately receive
the original measured samples through its existing sharing intent, BroadTracks
and explicit Console feeds. This is information only. It cannot select a target,
retarget a missile, illuminate, issue an order or grant control. Routine emission
needs no per-beacon polling; the ordinary sensor/radio controllers remain the
only controllers of those Systems.

Proposed exact report frontier:

1. TickCommsRadio pays ordinary listening once on its real current endpoints.
   Expose only derived current-tick payment/availability facts for this optional
   consumer, cleared on restore. Do not pay again or infer payment from an
   unrelated work receipt or a range-only legacy endpoint.
2. After final constrained pose and actual Broad measurement in Modifiers, stage
   one newest original sample for the beacon's configured channel. Each new
   observed tick can emit once. Failed payment retains eligibility; no unbounded
   backlog or same-tick chaining is created.
3. Recheck actual current equipment/condition, current physical positions and
   compatible Radio route, keys, receiver listening selection, jamming/protection
   and channel policy. Cached pre-motion coordinates are not current geometry.
   The source additionally pays the normal discrete transmission cost once per
   successful emission; ordinary continuous receiver work is not charged twice.
4. Emission success advances one retained report frontier, regardless of whether
   anyone hears. Only actually reachable, paid and monitoring compatible receivers
   receive through existing sample validation. Existing range-only sharing is
   not a fallback for a failed beacon Radio.
5. Copy original sample identity/time/error/origin into recipient BroadTracks.
   Receipt or retransmission cannot refresh measurement age or turn a private
   subject UUID into identity/correlation evidence. Losing link/power stops new
   arrivals; retained history ages and expires normally.

The emission frontier belongs to this equipment's existing observation/report
owner. No transport changes, player message-capacity accounting, AI voice,
separate radio controller or second sensor history is introduced. Exact first
recipe cadence, battery/Radio costs and ranges are authored provisional tuning;
finite positive work and bounds are mandatory. Root/Comms review must settle the
shared paid-endpoint and report-order API before runtime, including S066 receive
coverage if present in the integrated Radio.

## Continuation, knowledge and authoring

Capture/fold the attachment mode/offset/contact tick, immutable accepted runtime
spawn origin and last successful report frontier. Existing owners capture actual
energy, damage and Broad history. Rebuild derived physical projection, permitted
views and current paid Radio receipts; restoration never creates a fresh report,
new consent or a fresh observation. Attached rows must agree with the complete
candidate subject pose/accepted equipment at both restore doors. Reject cycles,
self/missing/Carried/unsupported subjects, foreign payload origins, nonfinite or
out-of-contact offsets, future ticks and duplicate round-child identity before
writes. Detached history can legitimately outlive a removed subject.

Workshop edits the supported payload reference through the existing generic
weapon document/history workflow and validates the actual dependent equipment.
Reference closure, finite reserves, matching System/Console/energy/layout,
restricted sensing and compatible paid radio are preview/Test prerequisites.
Status and existing radar history display real own/received equipment evidence;
no global list of secretly attached beacons or owner-free controls is added.

## Required proof before a delivered claim

- Actual accepted parent/child loader regression, including complete inherited
  reactor/Console/interior equipment and frozen dependency closure; malformed
  recipes refuse before spawning anything into a global catalogue.
- Real finite Human and ordinary delegated round preparation/launch, genuine
  hull contact and one equipped body. PDC interception, shield-up impact,
  expiry and unsupported contact create no beacon; all stock/paid work conserved.
- Single physical constraint owner through steering, transit-compatible local
  motion and Dock/tractor corrections; no subject force or Carried-disable trick.
  Subject loss detaches at last pose and ends subject-only measurements.
- Actual paid limited samples and compatible radio reception beyond the original
  observing ship's sensor reach; no identity/interior/aim bonus. Blocked/keyed/
  unmonitored/unpowered links withhold new reports while history ages unchanged.
- Complete fresh JSON360 capture/digest continuation through attached, blocked
  report, exhausted and detached states; actual launcher loss; forged relation,
  payload provenance/report-clock and candidate-pose no-write cases.
- Workshop accepted edit/undo/save/reload/validate/Preview/disposable Test and
  human activity evidence. Worker source review and owner checks remain separate
  from root Cargo/native/integration gates and physical device acceptance.

This draft changes no runtime, asset, balance value, compatibility epoch or
existing selected decision. The slice becomes adopted only after Root, Craft
and Comms review of its exact missing physical/report joins.
