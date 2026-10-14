# Gameplay redesign implementation

The user approved the incremental implementation plan on 13 October 2026.
Execution is tracked in [PRD #1562](https://github.com/jkeywo/project-phoenix-v2/issues/1562). The
[implementation report](./index.html) presents the serial sequence and the
complete [migration ledger](./migration-ledger.json). The
[mechanism reconciliation](../architecture/feature-reconciliation.html) and
[architecture decisions](../architecture/README.md) constrain implementation.

## Working contract

Implement one increment at a time. Each changed capability includes authored
configuration, runtime, human controls, ordinary individual System AI, Workshop
support and integration with deterministic state, snapshots and restore.
Structural migration first preserves observable behaviour; intentional gameplay
changes then receive new balance/activity evidence.

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
acceptance remains open; Increment 6 has not started.
