# Increment 13: equipment fitting acceptance

Increment 13 is in progress. The compiler, equipment library and physical team
extension, recovery integration, Workshop and personal planner have source
checkpoints; physical delivery and integrated continuation evidence are being
completed.
This document is a receipt and acceptance procedure, not a playable-exit claim.

## Source and focused evidence

`9daa9dbb` integrates the fixed-slot compiler and finite portable-equipment owner
from reviewed worker `ff8506fd`. The Cruiser exposes an existing forward beam and
reactor with exact default hardware, two optional authored sidegrades and four
actual assemblies in finite local equipment lockers. Dynasty explicitly selects
an empty fitting profile. Other shipped hulls retain their existing complete
equipment without requiring setup. JavaScript include/provenance tests passed
29 cases separately; native compiler cases are part of the integrated batch.

`fb19bb04`, `ac532914` and `a852c418` integrate the existing Repair team's refit
activity, actual compartment navigation, workflow, recall and continuation.
The focused Gameplay batch passed **30 cases**, including six new refit cases,
at isolated `f960c4cb` (9.41s build, 0.29s execution):

```text
cargo test -p phoenix-sim-gameplay --lib -- jobs::repair --test-threads=1
```

Log `.phoenix/worktrees/increment-13-refit-jobs/.phoenix/increment13-refit-jobs-retry.log`,
SHA256 `e8e6b23e3480ef321f4e95ca2afa78fc2e98709af4e25d5fc32d0ed43aed042f`.
The matching Gameplay dependency record is retained in that tree's
`.phoenix/refit-jobs-passed-dependencies/`. The preceding run passed 20 and
failed ten fixture decodes because compartment damage fields were missing in
old route fixtures and the new cases. Updating those real fixture definitions
preserved the route, exclusion, recall and restore assertions.

`ea93db66` extends the sole existing Repair AI controller to emit ordinary
DispatchRefit orders for permitted requests when its damage queue is empty.
That parent adapter requires integrated runtime validation; the 30-case pure
result does not cover it.

`dba25e07` integrates reviewed typed facility authoring from `e67bf0ba`.
Axiom's formerly passive mating capability becomes a real AI-only Dock System
with its own Console scope, local reserve, conditions and facility allocation.
Its existing weapon settings are retained. The added facility hardware's
0.25 flow/generation allowance is intentional provisional tuning and needs
gameplay evidence; no combat balance result is inferred from source inspection.

`7cea1073` integrates equipment recovery with the ordinary enable/AI path,
permitted reports, physical task exclusion, complete-roster continuation and
protocol/snapshot/rules 12/50/0.22. A source integration compile passed with
`cargo check -p phoenix-simulation --lib --no-default-features` (25.07s;
`.phoenix/increment13-equipment-integration-check-retry.log`). This is a compile
result, not runtime or save/resume evidence. The previous failed attempt remains
recorded separately; it found the Bevy query tuple limit and an ambiguous preflight
Result type, both corrected before the retry.

The isolated pure owner at `361bb309` passed nine fitting/energy cases and 39
Content entity-override cases. Logs and source provenance are under
`C:/Users/jkeyw/.codex/worktrees/increment-13-pure-validation/project-phoenix-v2/.phoenix/`.
The separately attempted `include_resolve` filter selected zero cases and is not
counted. A preceding integrated batch had eight passes and one real facility
validation failure: port validation accepted only a passive Dock declaration.
The correction accepts either supported passive or composed Dock hardware, and
the later nine-case batch verifies Axiom's actual facility path.

`608d6508` adds physical team visits and portable fetch transitions. The combined
Gameplay repair/fitting/energy batch passed **43 cases**, zero failures or ignored,
with 0.73s test execution:

```text
cargo test -p phoenix-sim-gameplay --lib -- jobs::repair resources::fitting::tests fitting_reconfiguration --test-threads=1
```

Log `.phoenix/increment13-fetch-pure-tests.log`, SHA256
`32800139427e8490f520e8447b7f15e2fa526d1e5e1f276c58bbbe720f673603`.
Matching Gameplay dependency records are retained in
`.phoenix/increment13-fetch-pure-dependencies/`. This tests the pure `608d6508`
changes with the integrated Model definitions; parent runtime delivery is a
separate consumer requiring its own evidence.

`79108ad5` and `13371b49` integrate reviewed Workshop composition, refit/recovery
controls, data-only favourites and protocol compatibility. Focused worker
JavaScript checks passed 190 cases across 15 suites, nine plan regressions and
64 compatibility/archive cases. `85cca8b5` adds prelaunch lobby planning and
validated draft handoff to explicit ordinary in-play requests; its final bounded
batch passed 254 cases across eight suites. These batches overlap and are not
added into a unique-case total. Strict strings (5,609 labels, zero missing or
orphaned), layers and independent read reviews passed. None is browser/native
human acceptance or a complete JavaScript integration suite.

`4bbc4559` extracts one accepted-hull planner definition for lobby and live status.
Prelaunch preferences do not alter actual installed hardware. A plan queues real
requests only after the current operator's permitted live fitting rows arrive.
The source checkpoint does not establish shared inventory, physical availability
or command authority from a favourite.

## Integrated physical-work and whole-host evidence

At `5d10104d` on the pinned increment 13 validation worktree, the combined
Gameplay/Simulation owner command passed **43 Gameplay and 15 Simulation cases**:

```text
cargo test -p phoenix-sim-gameplay -p phoenix-simulation --lib --no-default-features -- jobs::repair resources::fitting::tests resources::fitting_server::tests resources::equipment_recovery_server::tests snapshot::fitting --test-threads=1
```

The owner log SHA256 is
`511c9e3125089c66d48ca8c2aab2f4e69e9b58f07eb3e8a7f1728bc9328855a9`.
The worktree `.phoenix/worktrees/increment-13-fitting-continuation/.phoenix/`
retains `increment13-owner-evidence.json` and matching dependency records.

At `98dfee5c` (integrated as `0bba857a`), the actual host command passed **five
cases**, with 39.17 seconds of test execution:

```text
cargo test -p project-phoenix --features headless --test fitting_continuation -- --test-threads=1
```

This covers human and ordinary AI field fitting, exact parts consumption and
physical item identity, active Fetching/Removing/Installing continuation over at
least 360 ticks with complete JSON and digest comparisons, malformed restore
no-write checks, stale-holder/report-only refusals, and fleet peers using different
LocalShips. Log SHA256:
`18e2c7ae40dee5b7c3246c6bd83d4dc5eeee796c41987142116c7ab970860223`.
`increment13-wholehost-evidence.json` retains configuration and source records.

Earlier runs failed compilation on two Presentation facade paths, then failed
schedule initialisation: fitting ran before the damage state it required, and a
declared Infrastructure/Umbilical edge contradicted actual consumers. Commits
`ac709766`, `16d51971` and `0bba857a` fix these causes; passing assertions were
not weakened. The initial schedule-failure log SHA256 is
`a1c9006d102562efffa9b2caa45071bfcc18f682ead7aea44f5410da398a3ca0`.

The following final continuation batch extends this earlier five-case evidence
with actual facility refitting and intact-equipment recovery. The authorised
pipeline advanced increment 14 source while isolated increment 13 validation ran.
## Final isolated continuation evidence

The final increment 13 source checkpoint is `5d5df212`. Its supported frontier is
protocol 12, snapshot 51 and simulation rules 0.23; later increment 14 migrations
have separate compatibility versions. Evidence and matching dependency records
are retained under
`.phoenix/worktrees/increment-13-fitting-continuation/.phoenix/`, with the complete
source pins and hashes in `increment13-final-handoff.json`.

The actual host journeys passed **seven cases** at that checkpoint:

```text
cargo test -p project-phoenix --features headless --test fitting_continuation --test fitting_facility_continuation -- --test-threads=1
```

The five existing cases cover human and ordinary AI field fitting, physical
fetching, active removal/installation, no-write forged claims, stale admission
and fleet peers with different LocalShips. Two additional cases exercise actual
paid Axiom facility reactor fitting and an intact, condition-bearing assembly
recovered by the real Recovery tool, stored in finite Support storage, fetched
by the ordinary Repair team and installed at Weapons. They use admitted human/GM
commands and authored fixtures rather than direct controller or condition writes.
Every resumed journey compares complete JSON captures and deterministic digests
for at least 360 ticks, with exact item identity, condition and parts conservation.
The final run built in 34.92s; the five-case and two-case executables completed
in 36.15s and 100.94s. Log `increment13-final-seven-canonical-history.log`, SHA256
`05cd4b2aa5350126f372fe64090c9095632351f5e72a1a09bfabba90890629c7`.
Matching rebuilt Simulation dependency records are in
`final-seven-5d5df2-dependencies/`; the record includes this worktree and the
canonical historical-clock owner.

Whole-host comparisons exposed real continuation defects in historical combat
and LOD clocks. Writers and consumers now share the logical fixed-tick clock;
snapshots copy canonical timestamps rather than subtracting and reconstructing
floating-point ages. Preflight rejects malformed, future and obsolete age state
before any writes. High-tick exact-bit and complete-capture controls cover both
one billion ticks and the accepted `u64` frontier. Separate boundary tests passed
**five cases** at `5d5df212`:

```text
cargo test -p project-phoenix --features headless --test snapshot_resume -- fidelity_capture ai_fidelity_timer continuation_memories_replace_bootstrap --test-threads=1
```

Log `increment13-final-history-snapshot-boundaries-retry.log`, SHA256
`f28c2398ac9227a251a68416efbc087ed57201a53852839f7ab8e513029fa348`.
Execution took 11.66s after a 15.75s build. The preceding attempt failed to compile
an older fixture missing the required impact-sample fields; the correction adds
explicit absent values and preserves the assertions.

The broader targeted clock/AI owner batch at `93dea473` passed five Gameplay
cases and **251 Simulation cases**, with **two failures** in legacy assertions
that compared unbounded yaw directly against a wrapped heading. It is not a green
full-suite result. The correction compares shortest angular error with the same
0.01 tolerance and retains all route, motion and cursor assertions. A targeted
retry of those exact two cases passed at `ac298c76`; production source is unchanged
between these pins and `5d5df212`. Logs and SHA256 values are:

- `increment13-final-history-owner-corrected.log`:
  `f3072788eb15cdaed11133042ca07cc22989d56fc3a46942b9359aa5a7eb6754`.
- `increment13-final-two-route-retries.log`:
  `16a613a4155e0a6a831ebdf7ce0ea5fe8d36321c4da2d4526c2809302f99b29b`.

An earlier broader filter also selected
`console::navigation::server::tests::operate_navigation_ai_sets_waypoint_for_distant_target`,
which failed to produce its expected waypoint. That Navigation owner and harness
are unchanged from the isolated baseline and do not invoke the changed historical
clock paths. This remains an unresolved integration finding; no baseline pass or
broader native-suite pass is claimed. Browser/native human acceptance, complete
integration gates, balance and workload evidence remain outstanding separately.

## Human and ordinary AI journeys

1. Load each shipped seat-count hull and an affected scenario through the normal
   browser/native picker. Confirm existing Console screens, claims, visiting
   tabs, native Station displays and ordinary Backfill still work.
2. On a Cruiser, inspect forward-beam equipment from its owning Console. Compare
   current hardware and the real optional spare. Request a fit. Confirm this
   changes intent only and Repair receives the request through its explicit feed.
3. With Repair human-controlled, assign a free existing team. Watch it traverse
   actual connections. Block a connection and confirm work waits. Reopen it,
   observe safe shutdown, timed removal, an unavailable empty slot and timed
   installation. Verify one parts unit was consumed and the removed actual
   assembly remains in finite local storage. Charge, heat and unrelated faults
   must not reset.
4. Repeat with Repair vacant/delegated. Its ordinary AI should assign a free team
   when higher-priority damage work permits. Reclaiming the Console stops new AI
   assignments without destroying existing physical work.
5. Cancel before and after removal, including while a team is crossing a door.
   Check actual item locations, retained consumed stages, blocked return space
   and the team's real journey home. Reinstalling the same faulty assembly must
   preserve its condition; a healthy physical replacement must use its own.
6. Request a reactor refit away from a facility, at an ineligible facility, with
   disabled facility machinery, and with a valid coupled Axiom service. Only
   the final supported case may progress, using actual facility and crew work.
   Losing the coupling pauses work and does not refund or teleport equipment.
7. Exercise intact equipment recovery, optional personal favourites, import/
   composition, comparison and disposable rehearsal after those surfaces land.
   A plan must not create a spare or grant another Console command authority.
8. Save during removal and installation and resume onto a differently aged
   bootstrap. Reconnect and transfer Console hosting. Repeat on two fleet peers
   with different LocalShips; compare complete state and deterministic digests.
   Invalid ownership, duplicate equipment, missing claims and incompatible
   hardware must refuse before live writes, including absent candidate entities.
9. In Workshop, edit the same slot/library/storage definitions, undo/redo,
   save/reload, validate, Preview and launch disposable Test. Invalid references
   and unsupported topology changes need actionable refusals.

Browser/phone and native human acceptance, station workload, combat/non-combat
activity, performance, balance and final integrated gates remain separate from
targeted automated evidence. No push has been requested.
