# CI recovery for #1449 / PR #1451

The completed implementation was pushed to `codex/1449-completion` at
`705f8524`. Automatic approval review rejected a direct push to main and
recommended the branch/PR alternative; main remained at `349dea20`.

Ordinary PR run [34327287888](https://github.com/jkeywo/project-phoenix-v2/actions/runs/34327287888)
passed, including all three core smoke shards. Full matrix
[34327286814](https://github.com/jkeywo/project-phoenix-v2/actions/runs/34327286814)
passed native, viewer, demo, tooling, release build, WASM, JavaScript, PASM,
boundary and two full smoke shards. Shard 1 failed five tests (126 passed,
2 skipped); performance was consequently skipped. This is not a passing full
matrix or a Linux performance receipt.

## Findings and corrections

- Cruiser Helm at 716x375, available docking and active tow: Linux measured
  362px scroll height against 359px client height. The existing test passed all
  eight cases locally before the change, so the Linux failure was not locally
  reproduced. Reducing only compact-landscape movement-rail padding and gaps
  from 4px to 2px preserves control hit sizes and gives the content more room.
  All eight existing cases passed after the change (3.8s, port 3185). Linux
  confirmation remains pending.
- Three fleet-start refusal cases left Settings open, then programmatically
  invoked a GM action behind its focus trap. The confirmation was correctly
  inert, and a real pointer click could not accept it. The helper now closes
  Settings with Escape and asserts it hidden before invoking the action;
  ordinary confirmation clicks remain unchanged. All six fleet-start cases
  passed locally. The defective setup and modal lifecycle predate this PR.
- GM M1's final mesh gate found actual differing digests in CI: peers agreed
  at tick 300 and join commits 326/510, but disagreed by tick 600 after receiving
  the same Helm commands. No page errors were recorded. The current local
  bundle passed the unchanged disagreement gate (27.4s), which does not
  explain or discharge the CI failure. The test now retains exit mesh status
  through its existing deep-redaction pipeline, including on failure. The CI
  trace is retained at `target/issue-1449/ci-gm-trace/report.json`; the local
  passing trace at `target/issue-1449/gm-m1-local-pass.json`.

## Fleet root cause and correction

The exact CI release bundle (`project-phoenix-d0f9e8e2da06ea7e`) reproduced
the GM failure locally twice. Its JavaScript SHA-256 was
`8de6bdd46e4a8fd371eb7d8e0c368340667231f8f4790cafe9943573f827ff95`.
A temporary diagnostic paused all three peers at tick 346 and exported their
tick-347 records. The ship host folded `be1c3aa9540190b1`; both GM peers folded
`cd82e0d275703e44`. The player ship's physics differed, and the same pending
shadow manoeuvre had 37 ticks remaining on the ship host versus 43 on each GM.
The six-tick difference matched the command delay. Raw records remain local
under `target/issue-1449/gm-snapshots/`; the temporary export code was removed.

`LobbyResultApplier` and the mid-game rating handler previously changed the
local rating and control source immediately. `replicate_local_crew_ratings`
then sent the change through delayed admission. The host's extra AI ticks
permanently changed motion and Coordination; waiting for the ratings to settle
could not restore agreement.

Local crew changes now stage an ingress request. Only the existing admitted
rating consumer mutates the authoritative rating/control source, on the same
tick on every peer. Reconnect and AFK bookkeeping read the live rating plus
the canonical pending commands and any unsent request, preserving rapid
changes without a second queue of accepted commands. The existing Input
rating handler submits on the following tick; solo behavior is unchanged.

The complete `cargo test --features headless --test lockstep_mesh` suite
passed: 16 tests, zero failed/ignored, 6.03 seconds. Three new regressions
exercise disconnect, AFK/return before application, and an Input rating change
followed by disconnect/reconnect. They compare both peers' ratings, control
sources, physics and full digests on every step, then compare command logs.
The disconnect case also proves the AI actually moves the ship.

Independent read-only review passed the implementation, all three regressions,
and PASM/wiki changes. Full Vitest passed 310 files / 7,237 tests. Debug-surface,
strict-string, LOD and capture drift checks passed. Wiki lint checked 64 pages,
62 indexed pages, 1,696 references and 20 numbered references without errors.
The first corrected release bundle passed all 15 affected browser cases in
56.3 seconds: eight Helm layouts, six fleet-start cases and the GM M1 trace
(21.9 seconds). Rust release compilation took 13m28s. Assembly initially failed
at the wasm-bindgen cache permission boundary; rerunning with cache access
reused the compiled result and completed the normal Binaryen optimization.
Bundle `project-phoenix-fee17f8232740d78` contains the crew-rating fix; its JS
SHA-256 is `862982e0e8eef56d377752d713d98f04ff3bfdca5c87f49b0049b85588e453c3`
and WASM SHA-256 is `b32a814fbd42adbecc70323d99276d1036b28d137a27af3cfb6b9f63f0963223`.
The retained trace is `target/issue-1449/crew-rating-release-gm-report.json`.

## Nonvacuous digest verification

The retained exit status exposed two further diagnostic defects. Dynamic GM
Commit installed its lockstep session but left periodic sampling disabled,
so the returning GM could report agreement with zero local samples. Also,
an incoming checkpoint was compared only if its local counterpart already
existed; reversed arrival order could leave a mismatch unreported.

`b3be4407` enables disabled sampling when a GM is committed into a nonalone
fleet, preserving existing cadence, checkpoints and disagreement history.
Both arrival paths share comparison and duplicate suppression. The browser
gate now identifies the final Helm restoration's actual command tick, then
requires identical later checkpoint ticks and digests from all three peers.
Its existing disagreement gate and timeouts remain intact.

The focused native GM-join/lockstep library suites passed **40 tests, zero
failed/ignored**, including the local-sample-last path and recovery cleanup.
The affected fleet-session JavaScript suite passed **70 tests**. Independent
review passed the changes and the stronger browser assertion. The earlier
native workspace gate was deliberately interrupted when these additional
defects were identified; its partial execution is not a suite result. Final
browser/native gates and the new full Linux matrix remain pending.


## Remaining trajectory and barrier defects

The stronger browser gate exposed real disagreement at tick 600 on the
`b3be4407` debug bundle. A repeated diagnostic run agreed at 600, while the
next run failed; this was timing-sensitive, not discharged by one pass.
Matched snapshots at tick 471 showed the owner at x=13.714449, z=11.116503,
yaw=0.5919999 and both GMs at x=12.671613, z=12.492677, yaw=0.7839742.
Other authoritative values agreed apart from insignificant elapsed-time
encoding differences. Temporary snapshot export has been removed from the test.

`StartImpulseCharge` cleared authoritative thrust/steering latches only when
the ship carried `LocalShip`. A remote GM therefore retained old AI steering
during charge. The clear now applies to every ship; only the display cache
remains local, and later admitted axis commands still override in order.

The mesh barrier also paused `Time<Virtual>` in PreUpdate without consuming
the delta already calculated by First. The fixed runner could commit one
withheld tick before the pause took effect. Every barrier hold now clears that
current delta and whole fixed-step debt, preserving the fractional remainder.

Both focused regressions failed on the prior behavior: remote thrust stayed
0.8 instead of zero, and a missing-peer barrier advanced SimTick to 8 instead
of holding 7. Independent read-only review passed both fixes. The focused lockstep/Helm suites passed 40 tests, zero failed/ignored.
Three stricter browser repeats still failed, so this is not final browser proof. The preceding full native gate on
`b3be4407` passed (fmt, clippy, workspace headless); its receipt is retained as
`target/issue-1449/mesh-diagnostics-native-gates.json`, not claimed for these
new changes.

A later diagnostic narrowed the remaining failure to collision geometry:
three peers agreed at an early post-crew snapshot (tick 343), while another
run's tick-549 snapshots recorded the owner's second asteroid collision at
439 (16 shield damage) and both GMs' at 462 (7 shield damage). Hull and drive
state agreed; hull-damage auto-cancel was therefore not the cause of this
observed mismatch.

Rapier 0.33's fixed propagation invokes Bevy's simple and parent-transform
passes without Bevy 0.18's required dirty-tree marking. Static-tree skipping
can retain old global poses for roots with visual children. A focused test
now compares moving collision roots with/without a visual child across
multiple fixed steps, checking poses and actual contacts. The first attempt
stopped on a missing fixture asset resource; after correcting that setup,
the old code failed with contact=true versus contact=false solely because of
the visual child. Adding dirty-tree marking after character controls and
before Rapier propagation made the test pass (1 passed, zero failed/ignored).
Independent review passed the applied ordering. Fresh browser proof remains
pending.


## First runtime difference after power-loss recovery

The recovered diagnostic bundle `project-phoenix-811f35f30a3d7e0c`
completed one strict browser run, which failed as expected. Per-tick traces
matched through tick 422 after reconnect. At tick 423, the owner reached
forward speed 14.399994 while both GM projections remained capped at 14.
Both drives were Active, yaw/roll matched, and no contact existed. Thus the
later collision mismatch was a consequence of earlier speed divergence.

`translate_impulse_modifiers` still queried only `LocalShip`. The shared
integrator accelerated the remote ship's active drive but never received its
impulse speed bonus. The correction applies the authored modifier per ship;
snapshot restoration must also rebuild it before the first resumed physics
step. The per-ship regression failed on the old code (remote multiplier 1 versus
expected 6), then the corrected translator passed both focused tests, zero
failed/ignored. Independent read-only review passed the translator and
restore changes. Snapshot continuation and fresh browser validation remain
pending.

The first snapshot-test invocation omitted its required `headless` feature
and selected zero tests; that run is not acceptance evidence. The corrected
feature invocation is pending. PASM validate, scan and traceability passed;
wiki lint checked 64 pages, 62 indexed pages, 1,698 references and 20 numbered
references with no errors. Temporary per-tick runtime instrumentation has
been removed from tracked source and the browser test.


The normal fixed bundle `project-phoenix-c58829413bd3656d` passed two of
three strict browser repeats. The failed repeat had the owner and GM1 agree
at checkpoint 600, while only the reconnected GM differed. A subsequent
instrumented bundle `project-phoenix-d4aa924db22873e3` passed one run and then
three of five repeats. In a retained failed run, all compared physical poses,
contacts and drive values agreed, narrowing this residue to another folded
field. Exact-checkpoint snapshot/stage diagnostics are in progress; these
mixed runs do not constitute a passing browser gate.

The correctly configured headless snapshot test first exposed an inconsistent
fixture (manual Active state with stale Idle command). Correcting that intent
still failed. Direct HP instrumentation then established the real continuation
gap: live previous/current HP were 580.4/580.4, while the restored app compared
bootstrap 590 against restored 580.4 and incorrectly cancelled its Active
drive. The fix must retain per-ship previous HP across restore, including a
pending hit after the last Input sample; resetting it to current HP would lose
that legitimate next-tick cancellation. The temporary HP prints were removed.

The hull-history implementation now uses a required per-entity component,
visits all ships, and carries the previous sample in `DriveState`. It also
bumps snapshot format 32 to 33 and simulation rules 0.5 to 0.6 under the
existing compatibility contract: older saves lack the pending-damage history
needed for faithful continuation. Added coverage retains an unprocessed hit,
checks independent remote/local cancellation and no repeat, and reruns the
first-tick snapshot regression. Validation and independent review are pending.


Exact checkpoint capture on diagnostic bundle `project-phoenix-f676f4cfca20fa0e`
passed three of five runs. Both failures first differed in the
`station-stances` digest stage: the player cruiser had no explicit stance on
the owner and GM1, but the returning GM retained `tactical-weapons-free`.
Current station ratings and resolved control sources were absent from mesh
continuation; the frozen roster described bootstrap crewing. A mesh-only
helper now restores captured token-free authority after a verified transfer
and during rollback, while ordinary standalone saves keep their fresh crew
policy. Independent review passed this separation and its state reconstruction.

The hull-history change fixed the local first-tick mismatch and passed the
pending-hit restoration test. The same test then exposed an NPC insertion-tick
problem: the detector issued Idle, but the drive consumer discarded it because
the command component was newly added. Real producers now set transient
`DriveCommandWrites`; the consumer honors and clears them, while untouched LOD
defaults remain inert. Snapshot restoration clears this transient state.
Independent review passed the applied marker lifecycle. Focused combined
validation is running. All temporary tracked diagnostics have been removed.

The corrected combined focused run passed **18 tests, zero failed/ignored**
(`--features headless --lib -- impulse_modifiers hull_damage_
added_drive_defaults mesh_crew_restore`). The minimal crew fixture now registers
the snapshot query types and asserts that the captured player row contains crew
state. Both `--features headless --test snapshot_resume impulse_` regressions
then passed: pending hull damage and first-physics-tick continuation. The latest
PASM validate/scan/traceability sequence exited 0. A fresh normal browser bundle
and strict reconnect repeats are the next acceptance gate.

Fresh normal bundle `project-phoenix-16ccc1bed1d44747` passed all **five strict
GM M1 reconnect repeats**, zero failed/skipped, in 2.7 minutes. Each retained
the shared post-Helm-rejoin checkpoint assertion. The 18 focused native tests,
two snapshot continuation tests and independent reviews also passed. The
initial Trunk run compiled Rust successfully but failed creating its external
wasm-bindgen cache; retry with normal cache access built successfully without
source changes. Final integration gates and Linux confirmation follow.

The final JavaScript gates passed on `fdd9abd2`: 310 files / 7,237 tests,
debug-surface drift, strict strings, 44 LOD outputs and 22 capture outputs.
Independent integration review of `b3be4407..fdd9abd2` returned PASS. The first
final Clippy attempt found `items_after_test_module` in the new physics
regression. Commit `22afe996` moves that unchanged test module after production
registration; the native gate sequence is rerunning. Wiki lint passed with
64 pages, 62 indexed pages, 1,701 references and 20 numbered references.

The first full native attempt after Clippy passed reached **7,435 passed,
1 failed** in the library suite. The failure was the generic modifier-removal
broadcast fixture: it injected an ImpulseDrive contribution while the drive was
Idle, so per-tick derivation had already removed it before the fixture's explicit
remove. The add/remove fixtures now use a world-owned contribution, preserving
the exact source/slot/bonus and outbound Added/Removed assertions. No runtime
behavior or browser assertion changed. Targeted verification is running before
the full native sequence resumes.

Final native gates on `cd030d165aa647f61d1ea53b8eceac84f4a1cf74` all passed:
format check (3.39s), full workspace/all-targets Clippy with the repository's
explicit feature list and `-D warnings` (23.91s), and workspace headless tests
(376.38s). The complete test log contains **8,077 passed, zero failed,
99 ignored across 85 result summaries**. `PHOENIX_AMBIGUITY_BASE_REF` was
`349dea207b6a27cec81b696fc84dd1b029aea730`. This is the full successful rerun,
not a passing isolated retry of the earlier failure. PASM and JavaScript results
remain applicable; subsequent source changes were confined to Rust test layout
and fixture ownership. The final wiki lint still reports zero errors.


The newly available four-monitor rig passed the real accessibility check and,
after correcting its dynamically authored profile's startup ordering, the real
borderless-window geometry check. Each ran one test with zero failed/ignored;
the fixture fix is `bb41f528`, with unchanged timeout and geometry assertions.
Full Clippy and formatting passed again after that test-only change. The
production source remains the fully native-tested `cd030d16`. Device details,
initial failure and human limitations are in
[1128-four-display-verification.md](1128-four-display-verification.md).
