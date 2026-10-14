# Increment 8 — local energy and thermal operation

Runtime implementation is committed. Native integration, browser operation,
gameplay balance and physical/device acceptance are still being verified.
An implementation commit does not establish this increment's playable exit.

The implementation contract is [increment-8.md](../gameplay-redesign/implementation/increment-8.md).
Its explicitly provisional authored values are recorded in
[the tuning note](../gameplay-redesign/implementation/increment-8-energy-tuning.md).

## Implemented behavior

Incoming allocation fills each physical System's local reserve. Machinery pays
for actual work, with partial continuous grants and atomic discrete expenditure.
Zero intake preserves usable stored energy. Gross charging and depletion produce
heat; thermal protection preserves the owning System's intent. The reactor uses
the same capacity/thermal machinery and its own canonical capacitor.

Cooling withdraws actual finite coolant through existing service bindings.
Manual shortages cut incoming flows and retain requested settings; Minimal mode
uses authored priorities. Human controls and the ordinary individual reactor AI
share admitted commands and Console-scoped observations. Tactical weapons
restraint is explicit and independent of incoming allocation.

Shipped entities carry authored hardware and supported numeric state trees.
Workshop edits those profiles through its existing history, validation, Preview
and disposable Test workflow. Snapshot 43/rules 0.15 capture the new state;
transport and protocol 6 remain unchanged. Shield upkeep and energy recovery
remain increment 9.

## Verification at the initial integration

At runtime revision `2ce8e316`, the actual content loader successfully resolved
all shipped entity templates and scenario instance overrides. That check exposed
and led to removal of inherited obsolete Power policies and an obsolete
ranked-allocation strike budget check. Configuring both Power controllers is now
an explicit error.

The production compile check passed. Targeted pure energy/network, numeric tree,
weapon and client authoring/control tests passed during development. Those
bounded checks do not stand in for the pending integrated native or browser exit.

All three PASM checks pass against the clean worktree pinned to `2ce8e316`:
validation reports `Status: OK` (1,033 entities; 593 warnings/concerns, no errors),
scan reports `Validation: OK` (3,200 files; 18 Cargo packages), and traceability
reports `Validation: OK` (246 rows). Root-local validation must exclude unrelated
worktrees; the evidence above comes from an isolated clean checkout.

Strict string validation passes with 5,011 rows and no findings. Current runtime
wiki lint passes with 70 pages and 1,910 references. Browser acceptance sources
are migrated to the actual local-energy controls and explicit Tactical restraint;
execution requires a matching built bundle and remains pending here.

## Playable and continuation checks to record

1. Operate and delegate a Cruiser reactor. Change flow/cooling, preserve reserves
   during a cut, exhaust a local reserve, recover from heat and replenish coolant.
2. Exercise Min/mid/detail controls, priority ordering and Console handoff on the
   Courier and Destroyer. Retain all four shipped seat-count journeys.
3. Hold and release Tactical restraint while charged weapons have zero intake.
   Verify that machinery expenditure precedes ammunition, effects and cooldowns.
4. Edit physical hardware and a supported Power state tree in Workshop, including
   undo/redo, save/export/reload, validation, real Preview and disposable Test.
5. Save/resume during a cut, thermal stop, fractional coolant use and partial work.
   Check reconnect authority, stale-host rejection and deterministic checkpoints.
6. Record combat and non-combat activity, unchanged published balance thresholds
   and performance against increment 7. Intentional gameplay changes require
   updated evidence rather than a claim of exact structural parity.

Native tests, browser automation and headless reports must name their exact
revision and configuration. Actual phone/native-monitor interaction and human
station workload remain separate acceptance records.
