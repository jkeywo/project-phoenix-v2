# Increment 1: explicit Consoles

Status: implementation and automated verification complete; physical/device
and human-workload acceptance pending. PRD: [#1562](https://github.com/jkeywo/project-phoenix-v2/issues/1562).
Automated browser evidence below is distinct from human/device acceptance.
Compatibility versions are protocol 5, snapshot format 37 and simulation rules
0.9; unsupported old content/state is rejected explicitly.

## Tested build

Runtime implementation: `9b671984`, integration corrections `b67e25bc`,
`de19a798`, `4033e7f6`, and Console UI corrections `dbd4d162`.
Chromium tests use a fresh development WASM build from `4033e7f6`, with the
pure-JavaScript client rebuilt at `dbd4d162`. Rust sources are unchanged between
those revisions. Tests use the ordinary host, transport and admission path.
Workshop tests run the software WebGL rendering project. No browser test
establishes physical phone, remote-network or native multi-monitor acceptance.

## Playable exit

| Journey | Evidence and result |
|---|---|
| Cruiser crew join and ordinary screens | Automated Chromium roster and Engineering operation/reconnect pass. Physical phone acceptance pending. |
| Operate and delegate a System | Existing host-admission/AI-handoff browser checks pass; targeted ownership and AI tests pass. |
| Disconnect, AFK and reconnect | Browser AFK release and mid-game Engineering reconnect pass; Console working-view handoff/reconnect exercised separately. |
| Destroyer visiting Consoles | Real host test exercises ordered reassignment, AFK, stale-host view writes and machinery-command refusal. |
| Unhosted then return to the same host | Rust publisher regression verifies unchanged machinery values are resent after the gap. |
| Same screen used by two Consoles | Browser mounts two independent Navigation instances and changes one through a real wheel gesture. |
| Working-view continuation | Browser handoff/reconnect and overlay reload; Rust snapshot/restore/digest tests. Only supported fields are claimed, not arbitrary widget state. |
| Save/resume with AI work | Native snapshot continuation and Console working-view round trips; all 91 integrated snapshot/resume tests pass. |
| GM Station operation | Chromium: two equal GMs operate a human-held Station without blocking the player. Existing authority/GM tests cover current hosting and read-only feeds. |
| Shipped crew counts | Chromium preserves Courier 2, Destroyer 4, Cruiser 6, Battleship **9**. The current authored Battleship has nine seats; this structural migration does not change it to the roadmap's shorthand eight. |
| Native Station display | Native ownership/layout tests cover Station-based assignment. Actual multi-monitor launch, move, unplug and reconnect remain human acceptance. |
| Workshop composition | Render-project browser test creates/orders/assigns Consoles, changes Dock ownership/feeds, undoes/redoes, exports/reimports and launches disposable Test. |
| Inherited Workshop composition | Render-project browser test materialises inherited composition and exercises Command Console ownership in disposable Test. |
| Invalid authoring | Rust/JavaScript validators reject ownership, feed and hosting errors before valid preview/Test. |

The [validation receipt](../gameplay-redesign/implementation/console-validation.json)
records commands, revisions and raw-log hashes.

## Automated evidence

Raw local logs are in `.phoenix/console-redesign-resume/`; these ignored files
are diagnostic evidence, not shipped assets.

- `cargo test -p phoenix-simulation --lib --no-default-features`: 4,657 passed
  (`simulation-owner-8.log`), proving the simulation remains renderer-independent.
- `npx vitest run --maxWorkers=4`: **8,660 passed in 429 suites** on the final UI
  (`final-vitest-5.log`). Worker limiting leaves all tests and timeouts intact.
- Browser Workshop: **2 passed** (`browser-workshop.log`).
- Browser operational journeys: Engineering allocation, Engineering reconnect
  and AFK pass (`browser-operations.log`). Its GM failure was investigated and
  fixed; the same GM journey passes in `browser-final-ui.log`.
- Browser GM correction: Navigation sizing now waits until subclass private
  fields exist. Regression checks visible construction and retained-view resize.
- Overlay corrections: parent tab closure and internal Back changes publish
  immediately; stale open acknowledgements cannot reverse the user action.
- Console browser acceptance: **7 passed** (`browser-console-pass.log`):
  all shipped rosters, independent repeated screens, handoff/reconnect, stale-host
  refusal and overlay working-view continuity through iframe reload.
- Full workspace gate: **9,046 passed, zero failed**, 102 ignored across 141
  test binaries/doctest groups (`final-workspace-3.log`). Ignored cases include
  parent-driven children and explicitly opt-in scenarios. The sandboxed second
  attempt hit Windows temporary-profile permissions; the unrestricted third
  run is the passing integrated result.
- Rust formatting and Clippy pass on the integrated result.
- Layer boundaries and strict strings pass after the final UI corrections.
- PASM validate, scan and traceability pass on the adopted Console contracts.

The first full native run exposed old fixtures and a missing fixed-schedule
ordering constraint. Those were corrected; all nine failing targets passed
focused reruns. That is not represented as a full-suite pass: the integrated
rerun above is the passing integrated result.

## Gameplay comparison and remaining acceptance

The [increment 0 comparison](../gameplay-redesign/implementation/console-baseline.md)
passes repeat/replay/continuation checks and all 16 unchanged Cruiser balance
thresholds (20 wins). Gameplay metrics match, with the documented detached-Console
activity accounting difference. Mean headless update time rises by roughly
0.20-0.21 ms. No threshold was changed. Final continuation digests also match
in these all-Backfill samples with empty working views; this does not promise
cross-version save compatibility. Same-version continuation checks pass.

Physical phones, real remote-network conditions, native multi-monitor behavior
and human station workload remain unverified. Use the six-seat Cruiser first,
then the Destroyer visiting tabs, and record devices, tested revision and build
mode. Do not infer these results from headless or automated browser checks.
Increment 2 remains pending until the playable exit is accepted.

## Remaining human run

1. Launch the current native build with `.\run-native.bat` (the script builds
   the release host and client). Choose Combat Test and the Alliance Cruiser,
   then join from a physical phone using the on-screen QR.
2. Operate Engineering and Tactical, delegate a System, disconnect/rejoin and
   confirm its work continues. Repeat with the Destroyer while a visiting
   Console moves between occupied Stations; check its tabs and retained view.
3. On a multi-monitor bridge, assign a Station to a display, move it, add a
   second Station on the same display, and unplug/reconnect. Follow the
   [native lobby acceptance kit](1335-native-lobby.md) for the full hardware
   journey; the window remains a Station assignment throughout.
4. Run a combat and non-combat session with the six-seat Cruiser and record
   crew feedback on workload/continuity, devices, build revision and any
   visible regressions. This increment preserves gameplay rather than adding
   the later redesign mechanics.

Record results here before changing the ledger to complete. These steps have
not been performed by a human as part of this implementation record.
