# M2 Combat Test browser and replay gate

Issue #1316's repeatable automation is `tests/smoke/gm-m2.spec.js`. It loads the
actual `assets/worlds/combat_test.toml`, selects the Alliance Cruiser, seats
four crew participants before the recording, and joins two equal GM pages.
The crew remain witnesses during the recording. All directing actions use the
ordinary GM controls and authoritative command ingress.

Combat Test supplies an optional relief-ship palette, two optional Objectives,
two NPC doctrine choices, and presentation-only role presets. Its existing
eight wave timers expose Fire and Pause. Only the second-wave *report* exposes
Skip, so skipping that report cannot make the `waves_spawned >= 8` victory
condition unreachable. Normal timers, wave handlers and ending conditions
remain in force. The automation pauses future wave timers, operates the GM
families, then releases and clears every remaining wave through those controls.

## Run against one integrated revision

Use the combined implementation of every dependency listed in #1316. Build
the native `recorded_gm_exports` integration-test executable and run its focused
tests first. Preserve that executable, its exact revision and SHA256. Build
the browser with the repository's ordinary Trunk and client build commands.
Cargo and Trunk must follow the shared compiler queue; this test invokes the
already-built native executable and never starts another Cargo process.

From `tests/smoke`, in PowerShell:

```powershell
$env:PHOENIX_M2_EXIT = '1'
$env:PHOENIX_GM_REPLAY_EXE = '<absolute path to the verified frozen executable>'
$env:PHOENIX_SMOKE_PORT = '3477'
npx playwright test gm-m2.spec.js --project chromium --reporter line
```

Without `PHOENIX_M2_EXIT=1`, ordinary smoke discovery explicitly skips this
milestone test. A skip is not exit-gate evidence. The explicit run requires the
native executable and fails if replay refuses the artifacts or cannot match
the recorded final tick, applied outcomes and digest. Native execution uses
the repository root regardless of the shell's current directory.

## Evidence and limits

The test output retains the ordinary initial manual export and final GameOver
autosave as RON, the final save catalogue, GM submit requests, per-action
confirmation profiles and decisions, operator identities, projected states and
activity, final crew views, outgoing crew-message witnesses, native replay
output and original/replay digests. Partial browser observations are retained
on failure and do not claim a pass.

The collector observes the production submit function while forwarding its
original arguments and return value. It records button, Escape and backdrop
cancellation through the real shared dialog. It cannot create a command.
Snapshots are the durable authoritative record; page ticks sampled later are
not substituted for their capture boundaries.

Ordinary manual exports omit ordinary crew command history. This proof is
therefore deliberately limited to GM directing between two captures with
unchanged crew seats and ratings. The browser witness records every outgoing
crew message after initial seating and refuses ordinary inputs or transient
seat/rating changes. The native verifier separately checks the frozen crew,
boot identity, exact restored origin, and recomputed continuation. This is not
a general attended-session replay format or the human acceptance run for
#1320/#1323.

## Local milestone evidence

The latest local run, r5c on 8 September 2026, passed exactly one browser case
in 2.2 minutes, with zero retries, and its native replay child exited 0. Two
operators (`gm-1`, `gm-2`) directed Combat Test, seed 475, with four unchanged
crew witnesses. The ordinary exports span tick 25 to tick 3066 and
113 applied actions. Native replay matched every action result and the final
digest `14338432081949084171`; the initial digest was
`496071060238843454`.

The native, Trunk and client r5c bindings share source-metadata SHA256
`D6F4596143F6C6A4EA2850751AF501ED8F902CF65BB0C9D3522D82D3A7D2AFF5`
and runtime-manifest SHA256
`8543618364AE2211D1B4C3214327110C1E21B0BA5BCCC739EE2F312D49C7026A`.
They bind the unchanged Vellum revision
`606b0c06a6e6419a5b2a8c4f5bfb255146f87814`, rules 0.5 and snapshot format 32.
The SDK-enabled targeted native selection passed 17 tests; its separate manual
browser-export replay driver was retained as one ignored test and then executed
explicitly and successfully by this browser case.

The Playwright report and trace, ordinary exports, crew-input witnesses,
browser observations, native child receipt and replay report are retained under
`.t2-batch/t2-final-browser-r5c/`; `.t2-batch/m2-r5c-summary.json` records their
hashes. This is local M2 evidence within the GM-only, unchanged-crew scope above.
It does not claim remote CI, general attended replay or the human acceptance
sessions for #1320 and #1323.

### Earlier r4 result (historical)

The integrated run on 8 September 2026 passed the complete browser case and its
native replay, with zero retries. Two operators (`gm-1`, `gm-2`) directed Combat
Test, seed 475, with four unchanged crew witnesses. The ordinary exports span
tick 27 to tick 3081 and 113 applied actions. Native replay matched every action
result and the final digest `10996081562223247825`; its child process exited 0.
The initial digest was `2891290022737559438`. Both exports use rules 0.5 and
snapshot format 32.

Local evidence is retained under `.t2-batch/t2-final-browser-r4/`: the Playwright
report and trace, origin/final exports, crew-input witnesses, browser observations,
native child receipt and replay report. The native, Trunk and client r4 bindings
identify the tested artifacts and runtime manifest
`9B18BD786A9EC4C9F710BAD11B38C16B2A9E277EC9CFBC4FD598CFB8F301DDC7`.
The associated targeted native run passed 66 tests, with its separate manual
browser-export driver intentionally ignored; the browser case executed that
driver explicitly and passed. This records the local M2 milestone, not CI or
the human acceptance sessions for #1320 and #1323.
