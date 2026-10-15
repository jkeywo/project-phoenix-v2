# Increment 13: equipment fitting and refitting

Adopted implementation contract prepared against main `62e0c4cd`. This document
prepares increment 13; it contains no runtime implementation or playable-exit
claim. Increment 12's initial integrated commit and explicit advancement by the
integrator precede runtime work. Pinned validation of earlier increments remains
required under the authorised implementation pipeline. The migration ledger must
not describe increment 13 as started merely because these contracts are adopted.

## Outcome and selected scope

Let crews choose compatible equipment, install actual available replacements
through physical work, recover useful surviving equipment, and save optional
personal fitting plans. Keep complete ready-to-play defaults and the existing
Stations, Consoles, command boundary, networking and individual System AI.

Selected requirements are [G046, G072–75 and G104](../system-review.md), with
G098's existing Workshop workflow retained. The
[focused PASM slice](../../../pasm/spec/design/equipment-fitting-and-refitting.yaml)
distinguishes those selections from adopted implementation judgments. G046's
intact equipment recovery was explicitly retained for this increment by
[increment 12](./increment-12.md); finite resource salvage remains its existing
mechanism. G100 concerns external peripherals and is outside this increment.

The first playable consumer is an existing six-seat Alliance Cruiser beam
System with two authored same-family equipment choices. Both use its existing
System identity, owning Console, compartment, energy group and weapon strategy
address. Default equipment reproduces existing behaviour. Follow with a real
stored spare and field installation, supported facility reactor profiles, intact
equipment recovery, and the optional personal planner. These are implementation
steps within increment 13, not permission to close it after the first consumer.

## Reconcile historical alternatives before runtime work

The proposed [ship customisation slice](../../../pasm/spec/design/ship-customisation.yaml)
predates the selected redesign. Its starbase-only and frozen-at-spawn restrictions
are superseded by G072's compatible field changes plus facility-only major work.
Determinism requires admitted, ordered transactions and continuation, not a ban
on changing equipment. Its global battery exhaustion and overpower budget
description is superseded by increment 8's local reserves, heat, coolant and
flow-limited operation. Preserve the soft sustainability principle: equipment
that exceeds sustainable output can be legal to fit; there is no abstract power,
CPU or fitting-point currency.

Typed compatibility is now required here rather than deferred to historical
Band F. Use the approved Cruiser reference rather than the earlier unconfirmed
Battleship recommendation. Retain authored sidegrades, claimant/default
composition, the existing content resolver and measured balance evidence.
Mission-authored rewards do not imply unrestricted upgrades or account unlocks.

## Existing owners and new state

| Owner | Responsibility in this increment |
|---|---|
| Model | Closed equipment/slot identities, selected fit and work requests, portable equipment location and permitted reports. No Simulation dependency. |
| Gameplay `entities/config`, `ship/config` and capability compilers | Validate compatible definitions and compile the effective fitted capability. Existing `weapons/mount.rs::MountConfig` remains firing geometry; typed fitting slots are a distinct contract. |
| Gameplay physical damage, energy and capability owners | Remain the authoritative conditions, reserve/heat and machinery lifecycles. A fitting plan does not simulate another weapon or battery. |
| Gameplay jobs and `RepairTeams` | Extend existing typed work stages, team commitments, access routes and part claims for supported refits. No new crew pool or replacement repair controller. |
| Gameplay equipment storage | One actual identity and location per portable assembly, consumed by real installation/recovery. Fungible parts remain in their canonical `Stock`; equipment is not mirrored as a Stock count. |
| Simulation installation, admission and observations | Resolve actual owners, compile supported fit changes and commit affected owners together at a deterministic phase. Publish permitted capability/work reports. |
| Simulation snapshots/digests | Compile fitted definitions for the complete candidate roster before validating or applying continuation. Fold fit identity, equipment ownership, active stages and claims. |
| Content Preparation, Workshop and crew planner | Use the same closed compiler, includes/provenance, accepted source transaction, history, Preview and disposable Test. Personal plans do not bypass runtime acquisition or fitting. |

There is currently no portable equipment inventory or runtime loadout owner.
The generic authored weapon compiler already produces the real family strategy,
and `entities/ship_spawn.rs` installs that capability. Extend these owners through
bounded adapters, rather than introducing a second live entity configuration or
replacing an entity's arbitrary ECS component set.

## Authored slots, equipment and compatibility

A fitting slot has a stable hull-local identity, fixed target System, physical
compartment, supported family/interface and size bounds, allowed equipment
definitions and authored default. Optional empty slots are supported only when
their actual capability adapter explicitly implements the empty state. Existing
shipped equipment must remain complete without requiring player setup.

Equipment definitions describe supported capability parameters, fitting size and
interfaces, storage-space requirements, field/facility work classification and
typed work/parts requirements. Library templates use supported authored data;
they cannot inject code or arbitrary runtime components. Positive sizes, costs,
finite durations, references and unique identities are validated before preview.
Equipment families remain mostly narrow. A broader compatible slot must enumerate
the actual supported interfaces and adapter, rather than infer compatibility from
a similar name or total power consumption.

The first beam adapter retains System kind and graph identity. The first reactor
profiles retain reserve capacity and use the current shared energy/thermal owner.
Changing System kind, rooms, Consoles, routes, stores or ownership is outside a
normal fit. Later supported adapters may broaden equipment choices within this
increment; unsupported topology changes receive an actionable refusal. Ordinary
players do not rearrange hull geometry. Workshop can still author complete hulls.

Each supported alternative includes its real energy, thermal, coolant, damage
binding and work requirements. Physical compatibility and scenario availability
are hard legality checks. Sustainable power and expected throughput are forecasts,
not an extra hard fitting cap. Specific alternative coefficients and any measured
mass/handling effect are authored tuning, not requirements inferred from research.

## Authority, requests and ordinary AI

Before launch, each Station claimant owns the favourite choices for Consoles
whose authored home is that Station; unclaimed sections use complete authored
defaults. Current human-seeking placement does not transfer pre-launch ownership.
A whole-ship favourite seeds defaults. A later whole-ship import preserves every
explicit claimant choice, reports unresolved incompatible combinations, and never
silently overrides another claimant. Shared equipment belongs to its one owning
Console's home Station, not everyone who can see a summary.

In play, desired equipment is an admitted command to the actual target System.
Its current authorised Console operator may choose it, including when the Console
is visiting another Station. The old host loses that authority on reassignment.
An explicit feed or visible planner does not grant control. A fitted System's
ordinary AI may choose among approved equipment only through supported closed
facts/channels and scenario policy; human operation has no simultaneous AI
planner. Authored defaults remain usable when no refitting policy is configured.

Selecting a desired fit creates a request, not installation or permission to
command Repair. Repair's sole ordinary controller receives it through an explicit
typed permitted feed and assigns/recalls/prioritises its existing teams. Human and
AI use the same work admission and executor. Existing crew-order policy permits
manual recovery planning when powered tools are damaged, while preserving GM,
ownership, catastrophe and tenure gates. Knowing a request grants neither hidden
equipment condition nor foreign inventory access.

## Actual equipment and physical work

An assembly has one stable identity, definition, supported condition/continuation
and one physical owner/location: installed in a slot, stored in compatible finite
space, or held by a specific in-progress job at its actual location. While
installed, its physical condition belongs only to the canonical ShipDamage owner.
Removal transfers the supported condition payload into the stored/in-work
assembly; installation transfers that assembly's condition back into ShipDamage.
There is no competing live condition copy on an installed item. Claims are
exclusive. Reserving an assembly does not duplicate it; a blueprint does not
create one. Equipment storage uses explicit finite space and locations without
requiring per-component inventory actors or an inventory grid.

Refitting reuses existing route/access semantics, one team per committed task,
finite source parts, stage reservations and supported manual work plus paid tool
acceleration. A team must reach the actual compartment before work. Access
blockage, missing parts, unavailable equipment, damaged tools or an unsuitable
facility is reported through normal work reports. No failed preparation pays
parts, moves equipment or advances work partially across owners.

Use explicit preparation, safe removal and installation stages. The old assembly
remains installed until removal commits; its replacement and receiving space
are secured before that transition. After removal, its real location and the
vacant/unavailable capability are explicit. Cancellation or source disappearance
releases unconsumed claims and retains removed equipment and consumed stages;
it cannot teleport the old fit back, refund consumed parts, erase an assembly or
install the new one instantly. Return work waits for compatible receiving space.
Completed installation changes the actual capability once, then publishes the
result. Competing repair/refit/salvage work cannot own the same physical task.

The first beam sidegrade is field-installable when its authored requirements are
met. Reactor replacement is the initial major refit and requires a suitable
authored facility capability and actual service coupling/access permission. The
facility performs real work with actual equipment/parts; no global starbase
command grants instant replacement. Additional major/field classifications must
be explicit authored supported recipes.

## Preserve hardware state and work

An adapter defines exactly how its old and new hardware continuation relates.
Fixed-slot reserve charge, heat, coolant credit, stock, loaded ammunition,
in-flight payloads, operator intent and unrelated AI memory must not reset from
authored initial values. The first beam and reactor adapters explicitly distinguish
replaceable Operation hardware from fixed slot intake/cooling links and buffers.
Those links and buffers keep their existing owners; a portable assembly does not
gain a second EnergyState. Unsupported state migration is refused.

Choosing a blueprint or reinstalling the same assembly cannot cleanse its damage.
Installing a genuinely healthy replacement uses that new assembly's actual
condition, rather than retaining the removed assembly's faults. Only the explicitly
replaced hardware changes condition; unaffected component damage and compartment
structure remain with the physical ship. Stored/in-work equipment carries its
supported condition until transferred into the installed ShipDamage owner.

The first adapters preserve capacity. A future capacity-changing adapter must
define a physical safe transition before admission; silently clamping stored
energy or inventing storage is unsupported. Current heat remains real under new
thermal thresholds and can trip operation. Active projectiles retain their
launch snapshots. A beam/cycle is safely quiesced through its own lifecycle before
removal, rather than deleted to evade an active commitment. Preserve unchanged
program execution; a changed program requires an explicit validated migration or
documented reset of only that System's private frame.

## Intact salvage and personal plans

Recover a supported actual surviving assembly from a known eligible source using
the existing recovery/repair access, range, work and finite storage mechanisms.
The source loses that same equipment/capability when removal commits. A whole
equipment manifest reveals only permitted supported availability; source-private
dose, reserves and components do not become a planner oracle. Execution rechecks
actual eligibility. Material recovery remains the finite resource mechanism from
increment 12. No anonymous kill loot, automatic equipment roll or compulsory
individual item for every damage component is introduced.

The optional crew planner can compare, save, share and rehearse per-hull Station
or whole-ship favourites. A plan references compatible equipment definitions and
is separate from actual ownership, availability and installed state. Preserve
explicit claimant choices on composition, label missing items and scenario
restrictions, and validate imported versions/references before application.
Unsupported plans fail clearly rather than silently substituting equipment.
Personal persistence is a preference, not an account unlock or multiplayer truth.

Forecasts label assumed active Systems, duty cycle, initial stores/charge,
reactor mode and cooling conditions. Compare sustained demand, burst reserve,
heat/coolant limits and capability output with those assumptions; do not promise
exact survival time or reject a physically legal but unsustainable fit. Rehearsal
uses disposable Workshop Test and the ordinary simulation, not another evaluator.

## Deterministic phases and continuation

Resolve fit choices into the effective immutable definition and its identity.
Admit requests before starting work; progress existing jobs in stable job order;
commit each completed supported stage atomically before subsequent machinery
steps observe it. Freeze exact schedule edges against real consumers during
implementation, including active fire, part settlement and team claims. No
unbounded same-tick chain or second controller advances an installation.

Persist installed selections, portable identities/locations/conditions, requests,
active stage/progress, actual claims, safe-transition continuation and required AI
execution. On restore, first resolve and validate every candidate entity's fitted
definition, including entities absent from bootstrap. Then validate complete
cross-entity item ownership, part/space claims, jobs, damage, local energy, stores
and compiled AI against those definitions before any live entity/global writes.
Base SpawnOrigin hardware followed by a post-restore fit is insufficient.

Include authoritative fit/item/job state in existing snapshots, digests and
replication classes. Advance affected compatibility definitions together and
explicitly reject unsupported old saves/packs. Keep the existing transport and
delivery classes. Personal planner preferences need not enter a simulation digest;
the resolved accepted fit does.

## Ordered implementation and exit evidence

1. Reconcile these contracts; implement typed fixed slots and equipment compiler
   through the Cruiser beam pair. Migrate all shipped defaults, includes,
   generated equipment, overrides and fixtures without changing existing journeys.
2. Add actual finite equipment storage and a spare; execute field removal/install
   with existing RepairTeams, routes, parts, knowledge and continuation. Provide
   human controls, ordinary AI and Workshop editing in this playable slice.
3. Add supported facility reactor profiles and intact assembly salvage as real
   consumers of the same equipment/work owners. Preserve shared local energy and
   the existing physical recovery/resource paths.
4. Add optional personal Station/whole favourites, composition, labelled forecasts
   and compare/share/rehearse using the same supported templates and compiler.

Use targeted owner tests during development, independent read review, then the
required integrated gates once before a push. Record source pin, commands,
nonzero test counts and remaining evidence separately. Required meaningful cases:

- Default shipped hull/scenario compatibility, all supported seat counts and
  native Station displays; unchanged beam behaviour before intentional sidegrades.
- A human and ordinary AI complete an actual compatible field fit; stale host,
  foreign feed, incompatible slot, missing item and absent facility cannot do so.
- One equipment identity and exact finite parts/space accounting through removal,
  install, recall, interruption, source loss and blocked return. No duplicate team
  claims or free condition/energy/ammunition restoration.
- Save/resume and two different LocalShip peers agree at active removal/install,
  intact recovery and completed fits. Forged duplicate items, stale definitions,
  invalid fit graph and mismatched energy/damage/AI claims refuse without writes.
- Workshop create/edit/materialise, undo/redo, save/reload, reference validation,
  Preview and disposable Test; optional favourite import/composition preserves
  explicit claimant choices and rejects incompatible versions.
- Cruiser combat and non-combat activity, sustain/burst/cooling consequences and
  station workload compared with the current baseline. Keep existing balance
  thresholds; intentional sidegrades require evidence rather than assumed parity.
- Browser/phone and native human acceptance recorded separately from automated
  execution. Initial source integration alone does not establish this exit.
