# Increment 5: shared jobs and team execution

Contract: [increment 5](../gameplay-redesign/implementation/increment-5.md).

## Implemented scope

One per-ship job/team owner now carries internal repair assignments and exact
external commitments. Independent jobs retain stable identity, issuing System,
physical activity, private workflow continuation and bounded last outcome.
Local redirects keep their committed team's identity; repeated identical external
dispatch is idempotent. External retargeting starts a new activation.

The existing repair phase executor retains travel, repair rate, sweep order,
priorities and one-phase-per-tick boundaries. Ordinary admitted human and AI
commands still choose work. Restricted repair workflows reuse the state-tree
schema/compiler/evaluator/editor, including inheritance and reference calls.
Final writes gate supported effects once per step. A premature terminal workflow
reports interruption and retains its commitment; ordinary recall/release cleans up.

Repair reports remain separate received knowledge. Work-list rows expose issued
assignments and outcomes without hidden damage, private memory or extra controls.
Console feeds do not confer command authority. Onsite visibility remains unchanged.
External repair retains immediate dispatch/release and sorted condition effects;
it does not gain a travel timer or release merely because the target is full.

Workshop edits `repair.workflow` through the existing accepted-source history,
validation, save/reload, Preview and disposable Test journey. The shipped shared
workflow fragment reproduces current effects. Supported defaults cover content
that does not override its repair workflow. Inventories, parts, routes and further
job consumers remain later increments.

Snapshot 41/rules 0.13 store canonical jobs once, including queued redirection and
private continuation. Restore checks owner/crew/identity/workflow; unsupported
earlier state receives the existing version refusal. The digest folds canonical
jobs, with no second external claim or derived TeamSlot store. Protocol 6 and the
existing networking remain.

## Evidence

All required integrated gates pass: 9,112 native tests (102 existing ignored),
4,684 renderer-independent Simulation tests and 8,730 JavaScript tests. Formatting,
Clippy, generated surfaces, strict strings, LODs, layer/dependency checks and PASM
validate/scan/traceability pass. The wiki lint and 216-entry ledger checks pass.

Fifteen Repair browser cases and two rendered Workshop journeys pass. Workshop
covers inherited workflow materialisation, editing, validation, undo/redo,
export/reload, disposable Test and actual rendered Preview. A separate paused-job
browser case passes; its desktop screenshot was inspected for readable assignment
and pause status. These are automated/browser evidence, not physical device tests.

Independent read-only reviews passed the pure executor, Simulation adapters,
Workshop and root persistence/digest/client integration. Tests cover physical
phase equivalence, named-team exclusion, failure cleanup, reference/final-write
effects, RON continuation, malformed-state refusal and permitted projections.
Explicit external-repair ordering resolves shared team access without increasing
the ambiguity allowlist. Snapshot/digest characterization and authored-field
inventory assertions remain exact after their intentional schema updates.

Compact command/results are in [the validation receipt](../gameplay-redesign/implementation/jobs-validation.json).
The [baseline comparison](../gameplay-redesign/implementation/jobs-baseline.md)
records deterministic gameplay and performance separately from human acceptance.
Both bounded scenario reports match Increment 4 byte for byte; seeded repeats,
replay and continuation agree. Cruiser results remain identical: 20/20 victories
and all 16 unchanged thresholds pass. Median tick timings measured 4–7% higher
in the same-machine diagnostic; this is not release-budget acceptance.

## Human/device acceptance

Physical phones, native multi-monitor displays and human workload acceptance
remain open, including prior increments. Automated browser rendering is separate
from this acceptance. The user authorised advancing with “Do 5”; Increment 6
has not started.

On the six-seat Cruiser, take Repair under human control, dispatch a team, redirect
it during travel and recall it on site. Confirm ordinary controls, travel delays
and diagnosis visibility remain familiar. Delegate to ordinary AI and reconnect
without restarting work. Save/resume with a queued redirect and a team abroad;
confirm neither becomes free early. Repeat supported 2/4/8-seat journeys.

In Workshop select a repair-bearing hull. Edit its Repair Job workflow, validate,
undo/redo, save/reload, Preview and run disposable Test. Pause a repair effect,
confirm the work-list indication and use ordinary recall to recover the team.
Restore the standard workflow. Judge the work list and controls on physical
phones and native Station displays; assess attention cost separately.
