# Gameplay redesign implementation

The user approved the incremental implementation plan on 13 October 2026.
Execution is tracked in [PRD #1562](https://github.com/jkeywo/project-phoenix-v2/issues/1562). The
[implementation report](./index.html) presents the dependency-ordered sequence and the
complete [migration ledger](./migration-ledger.json). The
[mechanism reconciliation](../architecture/feature-reconciliation.html) and
[architecture decisions](../architecture/README.md) constrain implementation.

## Per-increment progress — 16 October 2026

The user requested a pause after finishing increments 8–12 on 16 October.
Complete their verification and necessary fixes, commit the evidence, then stop.
Later implementation lanes are parked with their worktrees and unfinished work
preserved; do not continue through 24 until the user resumes that work.

Only increment 0 is formally closed. Increments 1–7 have recorded automated
exits; physical-device and human-workload acceptance remain open. Later initial
commits are implementation checkpoints, not verified increment exits.

Current integrated compatibility is **protocol 45, snapshot 88 and simulation
rules 0.73**, at the `0e47b03b` documentation source. Compatibility numbers
identify supported state; they do not establish a verified gameplay exit.

The current priority is verification of increments 8–12. The selected owner run
at `34aa3e04` finished **1250 passed, 55 failed**. The seven-target whole-host run
compiled at `9493bc5a` finished **10 passed, 8 failed**: Activity Foundation 3/0,
Facility Supply 2/0 and Physical Damage Routes 3/0 passed; Fitting Facility 0/2,
Logistics 1/2, Production 1/1 and Recovery 0/3 remain non-green. The production
positive matched every full snapshot/digest comparison through 360 steps but
failed final completion; a diagnostic at `8bc01405` identified exhausted local
energy and zero allocation, not a continuation divergence.

The separate shield whole-host proof at runtime `895e234d` passed its 360-step
paid/current-credit and malformed-state no-write checks. Original client runs
remain recorded as failures; affected production-panel and Workshop retries
passed 3/3 and 53/53 respectively. A current 21-file client selection at `5cb35b1a`
passed all 121 selected tests; this is its own selection, not a relabelled pass
for the earlier 225 tests. Clean pinned PASM at `450836ac` passed
validation, scan and traceability. None of these narrower results replaces the
failed combined native runs or supplies browser/device, balance, performance or
station-workload acceptance.

A selected owner retry at `9a3b2f89` finished **141 passed, 6 failed** in
42.34s (`.phoenix/verify-8-12-corrected-owner-retry.log`). Energy/thrust/shield
notification and three restore failures remain, with corrections under review;
this does not replace the original 1250/55 result.

The source-reviewed production power-priority correction `2d34c831` is integrated
as `0e47b03b`, with native verification pending. The bounded verification document
`65bcceb4` accompanies this checkpoint packet. The correction uses the ordinary
Engineering command and preserves the original six-second continuation and two
outputs; it grants no energy. Detailed source pins and logs are retained in
[the integration checkpoint](parallel-checkpoint.json). No all-green integration,
increment closure or push is claimed. Broader increments 13–24 and authored
Patrol, War, Sandbox and Director work remain subject to their recorded gaps.

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
| 8 — Energy/thermal | Core implemented | Current selected owner run is non-green; retry integrated corrections and prove actual resource operation, balance/performance and playable acceptance. |
| 9 — Shield energy | Core implemented; separate paid whole-host JSON360/no-write proof passed | Corrected owner retry, full integration and playable acceptance remain. |
| 10 — Interiors/routes | Core implemented; actual physical-route host target passed3/3 | Browser layout authoring, supported hull/team journeys, full integration and playable acceptance remain. |
| 11 — Damage/restoration | Core implemented; actual damage/route host target passed3/3 | Selected owner failures and corrected fixture retries remain; full integration and Human/AI repair acceptance remain. |
| 12 — Logistics/production | Local stock targets, timed delivery, fabrication, salvage/processing and port replenishment/trade implemented | Observed Logistics/recovery/fabrication-completion failures and affected native retries, browser authoring and full acceptance; coolant recycling remains provisional. |
| 13 — Equipment fitting | Beam/reactor fitting, physical spares/refit/salvage and personal favourites implemented | Personal-plan disposable rehearsal and measured whole-fit trial code is integrated with separate owner evidence; full-host/native-device and broader fitting acceptance remain. |
| 14 — Sensing/environment | Substantial implementation | Knowledge, presentation, balance and full acceptance. |
| 15 — Layered combat | Combat core implemented; shared loaders/priorities integrated and accepted-recipe/pure handling checks pass | Physical marking code and accepted recipe/launcher owner evidence are integrated; loader/marking full-host continuation, broader illumination/link-loss/payload journeys, combat styles, balance/workload and full acceptance remain. |
| 16 — Comms/orders | Channels, requests and relay operational; both actual capture/decoding host journeys pass; listening/chatter integrated with accepted-recipe, pure-owner and client passes | Listening/chatter Simulation and host verification, broader contextual operations, native media edge/device acceptance and playable acceptance remain. |
| 17 — Electronic operations | Jamming/protection/identity operational; both emission host cases pass; bounded disruption integrated and its corrected hull passes actual accepted-recipe validation | Disruption host continuation is blocked by the Security singleton-stage membership error. Broader information/access and acceptance remain. |
| 18 — Movement/transit | Local jumps verified; sector code integrated; actual formation movement/JSON360 host case passes | Remaining assistance and broader acceptance. |
| 19 — Craft/deployables | Physical probe, both fixed beam-turret and both decoy host journeys verified; ordinary Full3D utility hull passes actual accepted-recipe validation | Charted XYZ flight code and proof source are integrated; its actual host evidence, moving custody/sorties/remote operation and full acceptance remain. |
| 20 — Personnel | Officers/care/roster, four physical personnel and both reception host journeys verified; bounded acquisition source integrated | Model/Session and Content acquisition checks pass; Simulation/continuation verification, broader boarding/away/rescue/capture and acceptance remain. |
| 21 — Facilities/actors | Finite ports and physical supply verified | Workforce, broader objectives/dependencies and acceptance. |
| 22 — Authored HTN execution | Working scheduling/activity core; scoped planned-delivery consumer integrated | Delivery native verification, broader operator/scenario coverage and full integration. |
| 23 — Continuing modes | Accepted-commitment foundation | Patrol, War and Sandbox are not delivered. |
| 24 — Discovery/Director | Bounded maintenance discovery, future repair monitoring and scoped planned delivery integrated | Native autonomous execution proof after the restocking correction; broader generation/pacing and final audits remain. |

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
