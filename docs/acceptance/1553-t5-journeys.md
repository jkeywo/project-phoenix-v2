# T5 named journey kit — version 1

Issue #1553 joins existing features at named boundaries. It does not certify
the whole product, replace originating-feature checks, or rerun the transport
matrix and balance batch. Evidence format: **`phoenix-t5-journeys-v1`**.

## Automated journeys

| ID | Boundary and observable result | Instrument |
| --- | --- | --- |
| J1 | Two competing slot picks, loser selects another berth, confirmed hulls freeze, exactly those two ships launch, shared Objective reaches a convoy report | `tests/t5_journeys.rs`, both arrival orders |
| J2 | Ship-filtered GM instance action, German switch with retained focus, explicit confirmation, correlated result, score-free report presentation | `tests/client/t5-journeys.test.js` |
| GM | Explicit slot > faction > all precedence; ambiguous activation and mixed terminal bulk action refuse atomically; live faction changes, completed joining, fixed credit and frozen history | Six existing `tests/gm_objective.rs` tests, separate from scenario stories |
| J3 | Existing GM disconnect/reconnect retains identity; public status moves through disconnected/restoring/live; separately, a diverged simulation is restored and fleet digests reconverge | Client journey plus exact `lockstep_recovery` case |
| J4 | Destroyed crew identity and closed control authority; camera selection waits for the authoritative projection, lost target retargets with focus retained, report remains narrative | Client journey, exact `cruiser_elimination` case and `crew_spectator_dead_hull_refuses_controls_while_live_crew_still_controls_own_ship` admission regression |
| J5 | Imported keyboard remap and chosen gamepad activate the ordinary strike-boost action; feedback requires matching acknowledgement; Backfill blocks a further command | Client journey |

The J1 lane shortens convoy travel; it does not measure normal mission pacing.
The JS lane uses jsdom and the real registry/protocol over in-process socket/RTC
stand-ins. J3's restoring/live projections are authored test inputs; only its
separate Rust case performs snapshot recovery. Neither claims real transport
owner handover (#1534), remote browser reconnection or hardware latency. German
fixtures contain representative long text; they do not certify German coverage
or pixel layout. Profile/action tests do not simulate physical controllers.

Run only the required lane, from a clean or explicitly recorded worktree:

```powershell
node scripts/t5-journeys.mjs --lane js --out <fresh-js-directory>
# Acquire the shared Cargo slot before running the next command.
$env:CARGO_TARGET_DIR = 'C:\Coding\project-phoenix-v2\target'
node scripts/t5-journeys.mjs --lane rust --out <fresh-rust-directory>
```

The runner refuses existing output directories, nonzero exits and missing or
unexpected test counts. It retains command arguments, UTC times, source SHA,
dirty patch, instrument/content hashes and full logs. Rust runs sequentially;
the first run refreshes the library timestamp for the shared target. Confirm
the retained compile log names the intended worktree and inspect the library
dependency record before handing the target to another task. A reused binary
or new test executable alone is not proof of the linked library's source.
No timing thresholds are imposed. Do not convert skipped or refused journeys
into passing results. Per-issue build and final integration gates remain owned
by their designated tasks.

## Recorded automated run — 27 September 2026

Tested prerequisite revision: `84b51c74e4b143156cee942c6aaba1f27139eb64`,
plus the instrument hashes retained in each manifest. The new instruments were
uncommitted during execution and are retained with this kit. Windows native
headless execution and Node/jsdom cover the boundaries stated above.

- [Native manifest](1553-evidence-v1/native/evidence.json): **10/10** selected
  tests passed: J1 1, dedicated GM 6, recovery 1, destruction 1, admission 1.
- [JavaScript manifest](1553-evidence-v1/js/evidence.json): **4/4** named client
  journeys passed. The separate runner integrity tests passed **2/2**.
- [Library provenance](1553-evidence-v1/native/native-link-verification.json)
  records the refreshed source, dependency record and linked executable/PDB
  verification before releasing the shared target. Full selected-test logs
  are retained as `.txt` files beside their manifests.
- PASM validate, scan and traceability passed; the changed wiki page's 38
  source paths and 63 index links resolved. Independent read-only review passed
  after adding actual hull-template and closed-admission assertions.

The [initial failed attempt](1553-evidence-v1/initial-failure/evidence.json)
is retained: J1 expected the base Objective ID instead of the published named
instance ID `escort_convoy::shared_escort`. Correcting that assertion produced
the passing fresh native run above; no product behavior was changed to pass it.

These results do not settle the human checks below, real owner handover,
rendered German layout, physical input devices or the unratified #1543 limits.

## Bounded human journeys — not yet run

Use the same built revision/content on every endpoint. Record browser/native
versions, hardware, resolution/zoom, locale, keyboard/controller profile, seed,
ship/GM identities and actual link routes. Use two ship hosts, one console per
crew and two GM operators; the four-crew convoy variant is a separate pass.
Allow roughly 40 minutes for these boundary checks, excluding ordinary scenario
play. A shortened disposable Workshop copy must be labelled as such.

### J1: competing picks and launch (5 minutes)

1. Select Alliance Convoy Escort. Two hosts attempt the same named berth before
   either confirms a hull. Exactly one holds it; the other sees its occupied
   state and can select a different berth. A loser must not confirm the winner's
   hull. Use keyboard-only navigation on one surface.
2. Confirm different allowed hulls. Launch without a GM start override. Check
   each crew's hull, berth and shared escort Objective; unclaimed convoy slots
   remain absent. On a second attempt, reverse the order of picks.
3. Disconnect one host after launch. Its ship remains on Backfill, threat scale
   does not change and optional assignments do not silently move to every crew.

### J2 and GM: scoped intervention and report (10 minutes)

1. In a three/four-ship convoy, inspect the separate recovery/pursuit assignments.
   Select an assigned ship in the GM panel; only its applicable instances show.
   Switch the GM surface to German while a named instance button is focused.
   Check distinct instance/all-instance names, visible focus and readable text.
2. As the second GM, preview and complete one instance. Cancel once with Escape,
   then confirm. Check attribution, correlation, scope and fixed completion
   membership; the other crew must not acquire the instance's private progress.
3. For precedence/atomicity/completed-joining cases, use the dedicated palette
   and six bounded steps in `1542-gm-objective-instances.md`; record these as
   **GM**, not as a convoy story. Automated GM cases remain the authority proof.
4. Let transport fates resolve or use the labelled shortened Workshop copy.
   Read the convoy and side-outcome rows. Check actual zero/one/two/three saved
   wording, no displayed diagnostic score, and no misleading global verdict.

### J3: status, disconnect and recovery (5 minutes)

1. Keep the owner running. Disconnect a non-owner ship/GM and inspect the same
   persistent status region with keyboard focus and a screen reader. Record
   whether the route is direct or relay and whether Backfill covers the ship.
2. Reconnect the existing GM using its ordinary retained identity. Verify a
   restored session does not mint a second operator or grant another ship's
   controls. Status must describe actual restoration and clear only when live.
3. Actual owner loss, replacement-host credential handover, forced divergence
   and real-network recovery remain the #1534/runtime acceptance matrix. If
   unavailable, record **unrun/blocked**, not a simulated pass from this kit.

### J4–J5: destruction, retargeting and private controls (10 minutes)

1. In Cruiser Elimination, operate one Dynasty Station using an imported
   keyboard remap and selected gamepad; check ordinary correlated feedback for
   strike boost, depletion and refused control while Backfill holds the system.
2. When that ship is destroyed, gameplay controls become unavailable while the
   crew keeps its identity and private Objective history. Choose a surviving
   ship through the camera picker. Selection changes the shared camera only.
3. Lose the selected target. Check deterministic retargeting, a useful focused
   control, and an honest empty state when no ship remains. Read the team report.
4. Repeat the visible picker/report/confirmation boundaries in German at the
   actual phone and native-console sizes, with 200% text scale, high contrast
   and reduced motion. Check labels, wrapping, focus and non-colour status cues.
   Record physical gamepad and screen-reader observations separately from JS.

## Evidence record and defect ownership

For each row, retain: kit version; tested source/content/bundle hashes; command
or exact human steps; environment; expected/observed result; timestamp; status
(`passed`, `failed`, `unrun`, `blocked`); artifact path; and originating issue.
Machine results cover only the explicit instrument boundaries above. Human
checks are unrun until an operator actually observes them. #1543 limits remain
unratified and are not used as this kit's pass criteria.

New defects belong to the originating feature: slot authority #1522; scoped
Objectives #1528/#1542; fleet health/recovery #1535/#1534; destroyed crew #1527;
private profiles/feedback #1536/#1537; Dynasty guidance #1540; locale #1545/#1554;
scenario outcomes #1551/#1549. Record a minimal reproduction, expected/actual
behavior and evidence, then create a focused follow-up issue through the normal
triage workflow. Do not broaden this kit into a UI redesign or conceal a defect
by weakening assertions. The feature owner fixes essential accessibility,
authority and localisation before claiming that feature accepted.
