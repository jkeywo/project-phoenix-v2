# Increment 10: interiors, boundaries and team routes

Implementation and validation are in progress. This record does not claim a
verified playable exit. The adopted contract is [increment 10](../gameplay-redesign/implementation/increment-10.md).

## Implemented slices

Workshop and authored-data slices were independently reviewed and committed as
`a740fc8b` and `de5e9d80`. Twenty-one definitions carry 305 installed System
placements, including five ordinary access gates, plus 18 explicit legacy
repair-target placements. The original equipment, damage pools and repair rates
are retained. Five player hulls have multiple decks. The visual editor uses the
existing accepted-source, inheritance, history and export path; System/shield
creation and team-count edits preserve layout references atomically.

The initial runtime integration is committed at `22b31f57`. Model defines the authored layout
and permitted positions. Gameplay compiles the graph and stores actual positions
inside the existing RepairJobState. RepairPhases remains the work executor;
physical arrival gates its transitions, spending and on-site knowledge. Routing
uses stable minimum-cost paths and preserves committed crossings on redirect.
Shared access state distinguishes a protected seal request, actuator intent and
actual transition. Ordinary AI cannot clear the protected request. Offline
equipment retains progress; a paused team clears its current crossing before
waiting at an endpoint.

Snapshot 45/rules 0.17 integrate physical positions, route continuation and actual
boundary state. Layout and route caches are rebuilt from accepted definitions.
Restore rejects mismatched commitments, unknown physical targets and malformed
positions before writes. The existing transport and System-addressed admission
are retained. Access has its own presentation family to preserve Repair command
identity under arbitrary authored ordering.

## Evidence

- Authoring slice: 110 focused JavaScript tests in 12 suites passed, plus layer
  checks. The legacy-target follow-up passed 50 affected tests in six suites and
  the final source/map/content checks (28 tests). Independent read reviews passed.
- Initial map/repair/dispatch checks passed 78 tests. Expanded map and command
  checks passed 201 tests in four suites.
- Final mixed Console, feed isolation, map and renderer checks passed 50 tests
  in five suites. Earlier renderer shape assertions required updating for the
  newly forwarded layout fields; no runtime behavior assertion was removed.
- Integrated strict strings pass (5,124 entries), layer dependency checks pass,
  and wiki lint passes (70 pages, 1,916 references). A scoped string-rule
  regression preserves passage endpoint IDs while retaining message localisation.
- Independent runtime review identified and corrected job/location restore
  mismatches, missing AI knowledge bindings, paused-crossing interlocks,
  unavailable actuator transitions and legacy repair-target homes.
- Eight actual Simulation access-owner regression cases were independently
  reviewed and committed as `a61e1548`; all pass against the integrated runtime.
- Renderer-independent Simulation compilation passed. Nine pure Gameplay graph,
  access and physical-repair tests passed. The latest five-suite client run
  passed 157 tests, with a further eight map/projection checks passing after
  physical passage progress replaced the obsolete phase-timer readout.
- The paused-team committed-crossing regression passes, bringing pure physical
  Gameplay coverage to ten passing cases.
- Final independent source review passed after correcting zero-team geometry
  publication and rejection of an omitted canonical repair-state block. The
  actual Simulation batch passes 12 cases: nine access/publication cases, one
  transactional snapshot case and two strict shipped-hull loader cases. Its
  first run found missing explicit Cruiser gate rating coverage and four generated
  NPC Red Alert placements; authored corrections pass the same compiled runtime.
  The command was `cargo test -p phoenix-simulation --lib --no-default-features --
  ship::access::tests:: strict_mode_accepts_every_shipped_hull
  strict_mode_is_on_by_default_and_every_shipped_hull_still_loads
  interior_snapshot_rejects`, then the unchanged compiled executable was rerun
  after those data-only fixes. This is working-tree evidence; unrelated increment
  9 fixture fixes were integrated during compilation, so it does not certify all
  increment 9 fixtures at one pinned source revision.
- At the same runtime source, the real headless host passed three selected
  cases: startup without a renderer, automatic mission launch and exact
  deterministic repeat. Command: `cargo test -p project-phoenix
  --no-default-features --features headless --test headless_runner --
  builds_and_ticks_without_a_renderer auto_start_reaches_in_progress_with_no_players
  deterministic_runs_reproduce_exactly`. The source changed only by documentation
  commits during that build. Existing unused-import/dead-code warnings in this
  narrow feature configuration remain separate from the ordinary final gates.

Broader validation is pinned to `22b31f57` in the isolated increment-10-validation
worktree. Later implementation proceeds under the user's authorised pipeline;
fixes and evidence return in dependency order. No verified exit is claimed.

## Pinned native follow-up

The [targeted validation receipt](../gameplay-redesign/implementation/interiors-native-targeted-validation.json)
records exact commands, revisions, log hashes and corrections. These checks use
historical increment 10 runtime `22b31f57` (protocol 7, snapshot 45, rules 0.17),
with test-only follow-ups through `fd464568`. They do not certify later increment
11 changes or a complete playable exit.

- Authoritative Workshop composition passes 32 owner tests. Explicit zero-team
  geometry loads without invented homes. Adding a System without physical
  placement is refused; the atomic System/placement edit succeeds and preserves
  dependency bytes. The first run found a missing-key error in the new test setup;
  correcting that setup retains the actual refusal and success assertions.
- The production resolver accepts all 248 entity instances across 70 shipped
  world files, including 115 instance overrides, 172 physical layouts and 84
  zero-team layouts. References and team homes remain consistent after includes
  and overrides.
- Ordinary AI dispatches repair work to a real remote compartment. Capture occurs
  during an actual crossing, before any remote repair. JSON restore into a fresh,
  different bootstrap retains identical full-world digests, jobs, routes and
  access for 360 steps, followed by positive funded on-site repair progress.
- Two peers with different LocalShip owners exchange actual Mesh frames for 720
  steps. Both ships physically cross and repair on both peers while full-world
  digests match. The first fleet run correctly failed its two-hull precondition:
  the fixture had authored only one ship. Adding the second real fleet slot fixes
  that fixture; the selected case passes without changing runtime or assertions.
- The admitted Repair/reconnect proof passes its fresh default/default/pinned
  child processes. It now targets a Tactical phaser in a remote compartment and
  derives its travel allowance from authored connections. Existing command-delay,
  correlation, visibility, complete-wire and shared-state assertions remain.
  Its first invocation reported the intentionally ignored parent; the explicit
  `--ignored` invocation executed and passed.

These are selected native checks. No full repository gate, push, browser build,
performance comparison or human acceptance is claimed by this follow-up.

## Remaining evidence

Exercise broader human dispatch, blocked-route, handoff and workload journeys.
Build matching browser/Workshop artifacts and run actual
edit/undo/export/reload/Preview/Test
journeys. Compare combat and non-combat activity, performance and supported
2/4/6/8-seat hull journeys. Record phone/native-monitor and human workload
acceptance separately. Complete the pinned PASM checks.

Increment 11 replaces damage only after physical layout and restoration paths
are ready. No compartment HP, penetration or new repair-resource requirements
are introduced by this increment.
