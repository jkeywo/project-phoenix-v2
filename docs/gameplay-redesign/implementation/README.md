# Gameplay redesign implementation

The user approved the incremental implementation plan on 13 October 2026.
Execution is tracked in [PRD #1562](https://github.com/jkeywo/project-phoenix-v2/issues/1562). The
[implementation report](./index.html) presents the dependency-ordered sequence and the
complete [migration ledger](./migration-ledger.json). The
[mechanism reconciliation](../architecture/feature-reconciliation.html) and
[architecture decisions](../architecture/README.md) constrain implementation.

## Per-increment progress — 16 October 2026

Only increment 0 is formally closed. Increments 1–7 have recorded automated
exits; physical-device and human-workload acceptance remain open. Later initial
commits are implementation checkpoints, not verified increment exits.

The latest eight-target host batch at `fd3d06de` passed 11 tests and failed 12.
Both fixed-turret journeys, four personnel journeys, one emission-policy journey
and all four authoritative-state checks pass. Decoy acceptance, capture/decoding,
emission continuation, formation, maintenance opportunities and receiving work
still have failing cases. Reviewed fixes for the phantom shield Console, fixture
identity and receiver addressability are integrated; the affected native retry
is running at `35f62881`. The three actual accepted-loader regressions pass.

The catalogue-completeness and fixed-step ordering blockers are corrected. Those
checks establish neither a green host batch nor the full integrated gate. Earlier
owner, codec, transport and host evidence, with its source pins and limitations,
remains in the integration checkpoint. Human/device, balance and workload
acceptance remain separate.

| Increment | Implementation state | Material remaining work |
|---|---|---|
| 0 — Baseline | Complete | None within the baseline scope. |
| 1 — Consoles | Implemented; automated exit passed | Physical-device and human acceptance. |
| 2 — Components/status | Implemented; automated exit passed | Physical-device and human acceptance. |
| 3 — State trees | Implemented; automated exit passed | Human acceptance; other controllers migrate with their capabilities. |
| 4 — Observations/reports | Implemented; automated exit passed | Physical-device and human acceptance. |
| 5 — Jobs/teams | Implemented; automated exit passed | Physical-device and human acceptance. |
| 6 — Stores/services | Implemented; automated exit passed | Physical-device and human acceptance. |
| 7 — Configurable weapons | Implemented; initial automated exit passed | Retained-beam regression fix integrated; native retry and human/device acceptance. |
| 8 — Energy/thermal | Core implemented | Selected energy/continuation and final authored Power policy checks pass; full integrated, balance, performance and playable acceptance. |
| 9 — Shield energy | Core implemented | Full integrated and playable acceptance. |
| 10 — Interiors/routes | Core implemented | Full integrated and playable acceptance. |
| 11 — Damage/restoration | Core implemented | Retained Repair adapter corrections pass targeted native checks; full integrated and playable acceptance. |
| 12 — Logistics/production | Local stock targets, timed delivery, fabrication, salvage/processing and port replenishment/trade implemented | Combined production/logistics failure journeys, current authoring and full acceptance; coolant recycling remains provisional. |
| 13 — Equipment fitting | Beam/reactor fitting, physical spares/refit/salvage and personal favourites implemented | Personal-plan disposable rehearsal and whole-fit sustained/burst/thermal/cooling forecasts remain unimplemented; full acceptance. |
| 14 — Sensing/environment | Substantial implementation | Knowledge, presentation, balance and full acceptance. |
| 15 — Layered combat | Assignments, traversal, sustained/kinetic/broadside fire, 3D missiles/PDCs, guidance, illumination, EMP and shield enquiry implemented | Physical marking payloads and optional shared loader groups/priorities remain unimplemented; full-host illumination/link-loss/payload journeys, multiple viable engagement styles, balance/workload and full acceptance. |
| 16 — Comms/orders | Channels, requests and relay operational | Media/decoder owner checks pass on their tested revision; typed claims source integrated; listening priorities/chatter, contextual operations and full native/playable acceptance remain. |
| 17 — Electronic operations | Jamming/protection/identity operational; ordinary AI emission-policy host case passes | Emission continuation fixture correction awaiting retry; bounded disruption isolated source reviewed but native unverified; broader information/access and acceptance remain. |
| 18 — Movement/transit | Local jumps verified; sector code integrated; formation owner/restore checks pass | Full-host formation fixture identity correction awaiting retry; remaining assistance and acceptance. |
| 19 — Craft/deployables | Physical probe and both fixed beam-turret host journeys verified | Decoy actual accepted-loader tests pass after parser correction; host retry, moving craft, sorties, remote operation and acceptance remain. |
| 20 — Personnel | Officers/care/roster and four physical personnel host journeys verified | Consent-bound reception addressability correction awaiting retry; security acquisition isolated source reviewed but native unverified; broader boarding/away/rescue/capture and acceptance remain. |
| 21 — Facilities/actors | Finite ports and physical supply verified | Workforce, broader objectives/dependencies and acceptance. |
| 22 — Authored HTN execution | Working scheduling/activity core | Broader operator/scenario coverage and full integration. |
| 23 — Continuing modes | Accepted-commitment foundation | Patrol, War and Sandbox are not delivered. |
| 24 — Discovery/Director | Bounded maintenance opportunity code/proofs integrated | Future repair intent/prerequisite monitoring source/proofs integrated but unexecuted; broader generation/pacing and final audits. |

Current source pins, failures and retries are recorded in
[the integration checkpoint](parallel-checkpoint.json). Parallel worktree code is
not reported as integrated or native-tested before its donor and evidence land.

## Revised future increments 22–24

Approved 14 October 2026. These plans replace the earlier summaries for 22–24;
increments 0–21, current implementation progress and outstanding acceptance remain
unchanged. Plan documents in the ledger are design references, not passed evidence.

- [22 — Authored mission planning, scheduling and execution](increment-22.md): activity contracts, HTN libraries, crew-aware scheduling and monitored execution.
- [23 — Continuing modes and persistent mission consequences](increment-23.md): authored Patrol, War and Sandbox with enduring assets and commitments.
- [24 — Opportunity discovery, adaptive direction and final integration](increment-24.md): bounded GOAP-like opportunity search feeding the HTN, live pacing and original final audits.

The architecture direction is recorded in [mission planning decisions](../architecture/README.md#mission-planning-increments-22-24) and the [PASM design](../../../pasm/spec/design/gameplay-redesign.yaml).

## Parallel execution approved 15 October 2026

[Parallel work contract](parallel-work.md) supersedes the serial-only execution rule
for increments 16–24. Three isolated lanes implement Comms/electronics, movement/craft
and personnel/facilities; planner consumers can overlap once their activity contracts
are stable. Full completion dependencies and permitted early slices are separate in
the ledger. The integrator owns shared schemas, schedules, versions and final gates.
Per-increment commits and all outstanding acceptance remain required.

Prepared lane contracts describe the next consumers; they are not completion
evidence:

| Lane | Contracts |
|---|---|
| Communications | [16 — channels and requests](increment-16.md), then [17 — electronic operations](increment-17.md) |
| Movement and craft | [18 — movement and transit](increment-18.md), then [19 — deployables and craft](increment-19.md) |
| Personnel and facilities | [20 — personnel and custody](increment-20.md), [21 — facilities and outside actors](increment-21.md), then [22 — mission planning](increment-22.md) |

## Working contract

On 15 October 2026 the user extended active implementation through increment 24,
including the approved mission-planning changes above. Continue committing each
increment, with later validation and fixes on isolated agent worktrees as already
authorised. This extends the delivery scope; it does not mark unfinished increments
or outstanding playable, performance and human acceptance as passed. Preserve
networking and knowledge-limited individual System AI throughout.

Implement capabilities in dependency order. Each changed capability includes authored
configuration, runtime, human controls, ordinary individual System AI, Workshop
support and integration with deterministic state, snapshots and restore.
Structural migration first preserves observable behaviour; intentional gameplay
changes then receive new balance/activity evidence.

On 14 October 2026 the user authorised an initial implementation commit followed
by validation and fixes on a pinned agent worktree while the main thread starts
the next increment. Merge those fixes in dependency order. Initial commits and
started increments do not claim a verified exit; evidence records retain the
tested revision and outstanding acceptance. Keep local Cargo and performance
captures sequential, and run final gates on the integrated result before a push.

Preserve shipped hulls, scenarios and tools. Preserve the existing networking
architecture and knowledge boundaries. Old saves and external packs may require
explicit conversion or their original compatible build. Do not silently load
incompatible state.

Temporary adapters translate definitions or payloads without maintaining another
authoritative simulation. Record their removal increment before introducing
them. Per-increment PASM contracts are adopted before runtime work. Detailed
System tuning is specified when its owning increment is prepared.

## Ledger checks

```powershell
node scripts/redesign-ledger.mjs --check
# After changing the canonical ledger:
node scripts/redesign-ledger.mjs --write
```

The checker compares all 216 IDs and titles with the catalogue decision record,
checks completion dependencies and the acyclic early-work graph and dispositions, requires evidence for completed
increments and checks that the generated report is current.

## Increment 0 — baseline

Recorded results: [baseline interpretation](./baseline.md),
[native/balance receipt](./baseline.json), and [validation receipt](./validation.json).

Current status is recorded in the ledger. Future increments are pending until
this increment has a verified exit. Baseline evidence uses the current runtime,
without changing scenario values or gameplay.

Run a new headless capture with:

```powershell
node scripts/redesign-baseline.mjs --out .phoenix/redesign-baseline/<new-run> --profile measure
```

The runner builds the selected executable, pins runtime-source and executable
hashes, compares repeated seeded reports, verifies replay checkpoints and records
three performance captures per representative world using the existing runner.
It refuses runtime/content changes during baseline capture. Documentation drafts
are allowed and recorded; this is diagnostic evidence, not clean-release profiling.
The default `measure` profile provides optimised migration comparisons without
release's hour-long full-LTO relink. `--profile release` is available for a separate
release capture. Compare like profiles and features; do not adopt `measure`
numbers into release performance-budget files. Tick summaries exclude the first
300 harness updates, while the raw captures retain all samples.

The existing ratified Cruiser matrix can then run against that captured binary:

```powershell
node scripts/redesign-balance-baseline.mjs --capture .phoenix/redesign-baseline/<new-run>/summary.json --out .phoenix/redesign-baseline/<new-cruiser-run>
```

This reuses the existing matrix, arguments, merge and threshold logic. It does
not replace the release balance gate or modify any thresholds.

Capture commands and compact results belong here; large raw reports, captures
and build output belong under ignored `.phoenix/`. A capture must identify its
source revision, runtime-source hash, executable hash, features, world, hull,
seed, arguments and limitations. Headless measurements establish neither real
browser/native rendering performance nor human station-workload acceptance.

Retain existing combat balance thresholds. Baseline capture does not authorise
retuning them. Existing performance budgets also remain unchanged.

Representative evidence covers the six-seat Alliance Cruiser, Combat Test and
Falling Skyway. Native deterministic replay, continuation tests and shipped-hull
validation supplement those runs. Actual hardware/network/human acceptance is
recorded separately; automated source or headless tests cannot claim it.

## Scheduled PASM reconciliation

The redesign design/roadmap slices record the dependency-ordered migration and selected
future direction. Earlier planned hard battery exhaustion, mutually exclusive
wide/focused scans, orbital body motion and starbase-only refitting are replaced
on their designated increments. Current code-oriented power, sensing and save
descriptions remain until implementation changes them.

The existing GDD remains the pre-redesign comparison baseline. Wiki pages track
current runtime understanding; they are updated only when runtime changes.

## Increment 1 — explicit Consoles

Implementation and automated verification are complete; physical/device and
human-workload acceptance remain pending. See the [validation receipt](console-validation.json)
and [baseline comparison](console-baseline.md). The adopted contracts are in
`pasm/spec/design/console-ownership.yaml`; the current ownership model is indexed
by [Console](../../../wiki/entities/console.md). Record playable browser/native
results in [the acceptance record](../../acceptance/1562-explicit-consoles.md).
Passing owner tests alone does not close the increment. The ledger remains
`in_progress` until continuation, authoring, gameplay comparison and the
playable exit are verified.

## Increment 2 — composable Systems and common status

The user explicitly authorised continuing with increment 2 while increment 1's
physical/device acceptance remains pending. The ledger records that instruction
as an advance authorisation; it does not mark the outstanding checks complete.

See the [implementation contract](increment-2.md), adopted
`pasm/spec/design/system-composition.yaml`, and
[acceptance record](../../acceptance/1562-composable-systems.md). Dock is the
first consumer of typed component composition and shared command channels;
common status is a derived Console-scoped observation. This increment leaves
state trees, resource redesign and component damage in their scheduled steps.

Increment 2 runtime is committed as `efac36b7`. [Automated receipts](composition-validation.json)
and [gameplay comparison](composition-baseline.md) pass; physical/device acceptance
and the recorded landscape-phone overlap remain open. The user subsequently authorised Increment 3.

## Increment 3 — reusable AI state trees

The user explicitly authorised starting this increment while prior physical/device
acceptance remains open. See [the implementation contract](increment-3.md),
`pasm/spec/design/ai-state-trees.yaml`, and [acceptance record](../../acceptance/1562-ai-state-trees.md).
Dock is the first migrated individual controller. Other controllers move with
their owning capabilities; shared jobs and Scenario tree contexts remain later work.

Runtime implementation is committed through `e78fb194`. [Validation receipts](tree-validation.json)
and [gameplay comparison](tree-baseline.md) pass; human/device acceptance remains
open. Dock proves the shared mechanism; other System controllers retain their
existing policies until their owning increments. The user subsequently authorised Increment 4.

## Increment 4 — common observations and reports

The user explicitly authorised this increment while prior human/device acceptance
remains open. See the [implementation contract](increment-4.md),
`pasm/spec/design/common-observations.yaml` and
[acceptance record](../../acceptance/1562-common-observations.md).

Runtime is committed through `faf88fb6`. Scan readings, dossier evidence, held
Sensors reports and repair contributors use shared metadata through existing
Console feeds. Sample time stays with the record; receipt, handoff and reconnect
do not refresh it. Repair reports and recovery watermarks use the existing queue,
snapshot and digest paths. Workshop continues to edit typed feeds.

[Validation receipts](observations-validation.json) and
[gameplay comparison](observations-baseline.md) pass. Both bounded scenario
reports match Increment 3 byte for byte; the unchanged Cruiser matrix passes
20/20 with identical merged results. Physical/device and human-workload
acceptance remain separate. The user subsequently authorised Increment 5.

## Increment 5 — shared jobs and team execution

The [implementation contract](increment-5.md), adopted PASM shared-job decisions
and [acceptance record](../../acceptance/1562-shared-jobs.md) cover the current
migration. One canonical job/team owner now holds local repair work and exact
external commitments. Restricted workflows reuse state-tree machinery and its
Workshop editor; ordinary admitted human/AI controls and physical repair phases
retain existing behavior. Reports remain separate knowledge.

Snapshot 41/rules 0.13 retain IDs, queued redirects and private continuation once.
The existing Repair Console shows permitted assignment/outcome rows without
additional controls or concealed diagnosis. Physical/device and human-workload
acceptance remains open; the user subsequently authorised Increment 6.


[Validation receipts](jobs-validation.json) and [gameplay comparison](jobs-baseline.md)
record the automated exit. Required native/client/PASM gates and real Workshop
browser journeys pass. The ledger remains `in_progress` for physical/device and
human-workload acceptance; the user authorised this increment with “Do 5”.


## Increment 6 — stores, transfers and service links

The user explicitly authorised advancing with “do 6 then” while prior human/device
checks remain open. See the [implementation contract](increment-6.md), adopted
`pasm/spec/design/resource-stores.yaml`, and [acceptance record](../../acceptance/1562-resource-stores.md).
Shared stocks stay in magazine/infrastructure owners. Named services and explicit
links support real docked replenishment through ordinary controls and AI.
Snapshot 42/rules 0.14 carry stock claims and fractional flow progress. No new
consumable drains, internal stock targets, production or personnel inventory land.


[Validation receipts](resources-validation.json) and [gameplay comparison](resources-baseline.md)
record stock conservation, continuation, real replenishment and the complete
Workshop authoring journey. Preview also resolves inherited captured templates
and waits for resident mesh geometry. Automated evidence passes; physical/device
and human-workload acceptance remains separate, with this increment kept
`in_progress`. The user authorised advancing with “do 6 then”.

## Increment 7 — shared mounts and configurable weapons

All 36 weapons in ten authored entities now use the supported common definition,
compiling once into existing beam, bolt and guided strategies. Common geometry,
delivery and barrel planning feed actual runtime consumers; stable System and
strategy identities, ordinary AI and admitted controls remain intact. Workshop
supports editing, creation, explicit inherited materialisation and its existing
history/validation/export/Preview/Test journey.

[Validation receipts](weapons-validation.json), [gameplay comparison](weapons-baseline.md)
and the [acceptance kit](../../acceptance/1562-configurable-weapons.md) record the
automated exit. Required gates, ten weapon/Operations browser cases and rendered
Workshop acceptance pass. Both bounded scenario reports and the unchanged 20-run
Cruiser matrix match Increment 6. Physical/device and human-workload acceptance
remain pending, with the ledger kept `in_progress`.

Under the user's subsequent explicit workflow, validation and fixes continue on
a pinned worktree after initial implementation commits while main starts the
next increment. Increment 7 matrix evidence pins `e848e2f3`; later implementation
on main does not alter its inputs. Snapshot 42/rules 0.14/protocol 6 are unchanged.

## Increment 8 — power, batteries, coolant and heat

The [implementation contract](increment-8.md), adopted local-energy PASM design
and [acceptance record](../../acceptance/1562-local-energy.md) cover the current
runtime. Physical work consumes canonical local reserves; incoming allocation,
finite coolant, heat, reactor ramping and deterministic flow cuts share the same
network. Ordinary Power AI uses supported numeric state trees and permitted
Console observations. Workshop and human controls edit the same authored model.

Initial runtime integration is committed through `2ce8e316`; all shipped entity
and scenario overrides pass the actual loader. Snapshot 43/rules 0.15 carry the
new state; protocol 6 and the existing transport are retained. Native ownership,
AI and continuation tests and matching-bundle browser checks continue in pinned
validation worktrees. Gameplay balance and physical/device acceptance are pending;
the increment has not yet passed its verified playable exit.

## Increment 9 — shield energy and protection

The [implementation contract](increment-9.md) and adopted shield-energy PASM
decisions now have a working runtime. Each arc pays upkeep and actual recharge
from its existing reserve; lossy recovery credits an eligible battery before
ordinary timed recharge. Human controls and individual arc trees share admitted
commands and permitted Console publications. Workshop edits the same coefficients,
destinations and closed trees. Static equipment uses the same physical owner.

The [acceptance record](../../acceptance/1562-shield-energy.md) distinguishes
passing mechanical, scoped AI, content and real headless schedule/determinism
checks from pending validation. Snapshot 44/rules 0.16 retain operational intent
and fractional work. Protocol 7/base-content epoch 2 reject mismatched clients
and unconverted packs through existing compatibility checks. Existing transport
and Station displays are retained. Full continuation, browser/native, balance,
performance and human-workload acceptance remain open; this is an initial
implementation, not a verified exit.

## Increment 10 — decks, compartments and internal routes

The [implementation contract](increment-10.md), accepted interior PASM decisions
and [acceptance record](../../acceptance/1562-interiors.md) cover authored layouts,
physical repair-team travel and ordinary access-gate Systems. Layout placement is
independent of Console ownership. Minimum-cost routes retain committed crossings;
protected seals block new entry and wait for those crossings to clear. Actual
arrival gates repair work and local diagnosis. Existing percentage damage remains
until increment 11 replaces damage and restoration together.

Snapshot 45/rules 0.17 retain canonical navigation and access continuation.
Immutable geometry is rebuilt independently of team count, so equipment without
repair teams still publishes its permitted map. Workshop supports the same
layout, connection and placement definitions. Focused native/client checks and
independent source review are recorded separately from pending matching-browser,
balance, performance, multiplayer and physical acceptance. This is an initial
implementation while pinned validation continues, not a verified exit.
