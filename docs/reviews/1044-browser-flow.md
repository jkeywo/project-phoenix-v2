# Falling Skyway browser major-flow evidence (#1044)

The opt-in `tests/smoke/falling-skyway-flow.spec.js` follows the unchanged shipped
world with the Alliance Destroyer and its authored seed 1034. It does not replace
timers, inject scenario flags, invoke script handlers, or accelerate the clock.

```powershell
$env:PHOENIX_SMOKE_PORT = '3182'
$env:PHOENIX_SKYWAY_FLOW = '1'
npx playwright test falling-skyway-flow.spec.js --project chromium
```

Run from `tests/smoke` against the matching built bundle. The environment gate
keeps the full authored timeline out of ordinary CI; the test has a 40-minute
timeout. The candidate uses `project-phoenix-5f8fb494bdf5d61a` and world SHA-256
`48f89f0f608431ea5c640a3fe1b0f8a5b90bfca6cd14b3edeab7e610b4f87d9f`.

Two ordinary spectators join before launch. One observes reliable protocol
messages and the other reconnects on the same participant token into the real
client page. All four destroyer stations remain on Backfill. The host's existing
`wasm_force_start` entry point launches the mission: its AI-only button is hidden
once spectators join, and late spectators miss initial WorldSetup/objective
broadcasts. This is not evidence of a visible operator launch action with
spectators already connected.

The test checks WorldSetup identity, opening corridor/triage objectives, corridor
completion, the actual negotiation flag and Act 1 resolution, each of the four
storm deadlines, Act 2 resolution, the transfer-window deadline, the actual
window close, allocation objective status consistent with actual bookings,
reliable GameOver report delivery with lift state/outcome consistent with actual
served/berthed claimants, and
the real spectator page's visible ending/report rows. A compact history retains
objective states and checkpoint ticks; transport frame histories are bounded.
The terminal artifact includes the world hash, scenario debug state, objective
history, ending payload and page errors, plus a screenshot of the crew ending.

Independent read-only review passed after correcting phase-specific flag names,
spectator acknowledgement and history bounds. Earlier setup attempts were not
mission failures: they used a lobby notification the protocol does not emit,
attempted the hidden AI-only button, or joined too late for initial broadcasts.

The first reviewed run reached all three storm bands before a power outage
interrupted it. It has no terminal receipt and is not counted as a pass. The
unchanged test was restarted on port 3179 on 9 September. That run failed at
28.7 minutes on an incorrect branch expectation: the authored browser seed 1034
booked and served Havelock, completing the window objective, while the test
expected the separate native seed-1 run's hold/no-lift outcome. The retained
`target/issue-1449/skyway-browser-hold-assumption-failed.json` contains that state,
including completed storm/lee objectives, both completed first acts and the
transfer checkpoint at tick 96,040. It has no ending and is not a passing run.
The corrected test explicitly requires the observed booking, served flag,
completed allocation, closed window and partial-lift ending; it was restarted
on port 3180. Independent read-only review passed those corrections against
the retained state and authored script, but that new run took the hold path
instead (`skyway_held_berth=2`, no booking), failing the opposite branch oracle
at 28.7 minutes. Its state is retained in
`target/issue-1449/skyway-browser-allocation-assumption-failed.json`. Neither
run reached its final assertions. The authored seed alone therefore does not
justify prescribing a Backfill choice across these live browser boots.

The final test checks the authored consequence of the actual allocation:
window closed/open flags, completed objective iff a lift was booked (failed
otherwise), and the public report state/outcome for the actual served/berthed
claimants, with explicit mission-finalized/resolved flags. It still requires
every major checkpoint,
the reliable ending and real client's visible report. It was restarted on port
3181, then stopped early when review caught that the public report omits score
and zero flags are elided. Those test assumptions were corrected before
restarting on port 3182. No world, runtime, deadline or scenario flag was changed.

**Result: PASS, 1 passed / 0 failed / 0 skipped, exit 0, 30.7 minutes.** The
port-3182 run closed the window at tick 108,042 and displayed the crew ending
at tick 110,208. All nine report rows were delivered; this run carried nobody
clear (`lifts: lost`), matching the observed allocation and failed window
objective. Both storm/lee objectives completed and page errors were empty.
The retained [JSON receipt](1044-browser-flow.json) contains every checkpoint
and final state; the [ending screenshot](../acceptance/1044-skyway-ending.png)
shows the actual visible nine-row report. Independent source review passed.

This is one automated Backfill path. It does
not establish human storm navigation, alternative allocations, per-console work,
three-person doubling, warning recognition or the two-thirds-work tuning target.
Those remain the distinct human criteria in #1044/#1352.
