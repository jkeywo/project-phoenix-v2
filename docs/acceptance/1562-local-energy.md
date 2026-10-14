# Increment 8 — local energy and thermal operation

Runtime implementation and the bounded browser checks below are verified at an
isolated increment 8 pin. Integrated native gates, gameplay balance, performance
and physical/device acceptance remain pending. This record does not establish
the final integrated build's playable exit.

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
and disposable Test workflow. Snapshot 43/rules 0.15 capture the new state.
The historical browser pin below uses protocol 6/content epoch 1. The integrated
redesign advances compatibility to protocol 7/content epoch 2 because old power
controls have different semantics; matching final-bundle acceptance is separate.
The transport architecture is retained. Shield upkeep and energy recovery remain
increment 9.

## Verification at the initial integration

At runtime revision `2ce8e316`, the actual content loader successfully resolved
all shipped entity templates and scenario instance overrides. That check exposed
and led to removal of inherited obsolete Power policies and an obsolete
ranked-allocation strike budget check. Configuring both Power controllers is now
an explicit error.

The production compile check passed. Targeted pure energy/network, numeric tree,
weapon and client authoring/control tests passed during development. Those
bounded checks do not stand in for the pending integrated native exit.

All three PASM checks pass against the clean worktree pinned to `2ce8e316`:
validation reports `Status: OK` (1,033 entities; 593 warnings/concerns, no errors),
scan reports `Validation: OK` (3,200 files; 18 Cargo packages), and traceability
reports `Validation: OK` (246 rows). Root-local validation must exclude unrelated
worktrees; the evidence above comes from an isolated clean checkout.

Strict string validation passes with 5,011 rows and no findings. Current runtime
wiki lint passes with 70 pages and 1,910 references.

At `bb605478`, the no-default-feature Simulation owner runs pass 14 Power,
8 reactor AI, 4 whole-host Power and 4 equipment-status tests. These cover
current authority, stale hosts, paired-command rollback, ordinary AI knowledge,
actual coolant transactions and save continuation. The weapons batch passes
282 of 288 cases; six fixture failures were corrected in `7b89ed85`, with their
rerun and the integrated native batch still pending in this record.

## Pinned browser and Workshop evidence

The clean runtime/content source is
`8bb63e1ae8f291c542784149e0e78170051699d4`. The host and phone bundle use
protocol 6 / `phoenix-base` epoch 1. Host release compilation, the client build
and the separate Workshop Preview release build all completed. Fresh library
timestamps and dependency records are retained in the
[build receipt](../gameplay-redesign/implementation/energy-browser-build.json),
which records SHA-256 hashes for all 1,791 bundle files, including both game WASM
artifacts. The [validation receipt](../gameplay-redesign/implementation/energy-browser-validation.json)
records commands, source revisions, raw-log hashes and outstanding work.

| Run | Result | Evidence |
| --- | --- | --- |
| Chromium at the runtime pin | 16 passed; 2 crew-fixture failures | Actual sensor restraint, Tactical hold/release with zero intake and charged weapon reserves, Energy controls, delegation, keyboard, text scaling, forced colours and iframe presentation |
| Corrected crew harness `54cbdc3b` against the same bundle | 2 passed | Destroyer Engineering and Courier Captain flow commands, human-seeking Comms/Navigation tabs and Console working-view persistence through tab changes and reload |
| Render project with that harness and bundle | 1 passed | Inherited stores/weapon/energy edits, explicit materialisation, authoritative refusal, undo/redo, export/reimport, actual mesh/pixel Preview and disposable simulation Test with source preservation |

The two crew fixtures now subscribe before launch and hold/ready the other
Station. The transport helper reads the stamp from `PHOENIX_SMOKE_DIST`, matching
the server's selected bundle. These are test-harness corrections; runtime and
shipped content are unchanged. Nineteen distinct cases have passing evidence
across the 16 + 2 + 1 runs, rather than one unchanged all-green suite invocation.

This is automated Chromium/SwiftShader evidence. Physical phone/native-monitor
use, human workload, balance and performance comparisons remain pending. It also
does not prove acceptance of the final protocol 7/content epoch 2 bundle.

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
