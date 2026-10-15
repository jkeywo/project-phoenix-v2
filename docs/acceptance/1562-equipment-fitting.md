# Increment 13: equipment fitting acceptance

Increment 13 is in progress. The compiler, equipment library and physical team
extension have source checkpoints; integrated runtime tests and the remaining
authoring, planner, recovery and continuation journeys are being completed.
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
