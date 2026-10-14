# Gameplay redesign implementation

The user approved the incremental implementation plan on 13 October 2026.
Execution is tracked in [PRD #1562](https://github.com/jkeywo/project-phoenix-v2/issues/1562). The
[implementation report](./index.html) presents the serial sequence and the
complete [migration ledger](./migration-ledger.json). The
[mechanism reconciliation](../architecture/feature-reconciliation.html) and
[architecture decisions](../architecture/README.md) constrain implementation.

## Revised future increments 22–24

Approved 14 October 2026. These plans replace the earlier summaries for 22–24;
increments 0–21, current implementation progress and outstanding acceptance remain
unchanged. Plan documents in the ledger are design references, not passed evidence.

- [22 — Authored mission planning, scheduling and execution](increment-22.md): activity contracts, HTN libraries, crew-aware scheduling and monitored execution.
- [23 — Continuing modes and persistent mission consequences](increment-23.md): authored Patrol, War and Sandbox with enduring assets and commitments.
- [24 — Opportunity discovery, adaptive direction and final integration](increment-24.md): bounded GOAP-like opportunity search feeding the HTN, live pacing and original final audits.

The architecture direction is recorded in [mission planning decisions](../architecture/README.md#mission-planning-increments-22-24) and the [PASM design](../../../pasm/spec/design/gameplay-redesign.yaml).

## Working contract

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
checks serial dependencies and dispositions, requires evidence for completed
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

The new redesign design/roadmap slices record the serial migration and selected
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
