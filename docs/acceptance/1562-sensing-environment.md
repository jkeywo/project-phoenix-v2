# Increment 14 — sensing and environmental interaction

Source implementation is in progress under the authorised worktree pipeline.
The selected contract is `docs/gameplay-redesign/implementation/increment-14.md`;
this record distinguishes delivered foundations from the complete playable exit.

## Authored machinery and controls

The integrated source includes independent focused, broad and passive equipment
profiles, real Cruiser resource bindings, bounded spatial fields and substantial
terrain signal materials. Workshop edits these accepted definitions through the
ordinary document/history, validation, Preview and disposable Test workflow.

`8d53ada5` adds focused enquiry progress, cancel and retained-finding controls,
plus passive reception controls and bearing/signature readouts on the existing
Science/Sensors screens. Exact System ownership is required for controls; a feed
alone remains read-only. The independently reviewed worker batch passed 592
affected JavaScript cases across seven suites; the final text cleanup passed 27
cases. These overlap and are not added into a unique-case count.

On the integrated source, the combined field/instrument Workshop batch passed
**86 cases across five suites**, with zero failures or skipped cases:

```text
npx vitest run editor/tests/workshop-fields.test.js editor/tests/workshop-sensing.test.js tests/client/workshop-fields-panel.test.js tests/client/workshop-sensing-panel.test.js tests/client/workshop-authoring.test.js
```

Log: `.phoenix/increment14-workshop-integrated-js-retry.log`. The preceding
sandbox attempt failed to rename Vite's temporary cache before running tests;
it is not passing evidence. The same command completed with normal cache access.

The integrated sensing controls and protocol 13 stamp passed **14 cases across
two suites**:

```text
npx vitest run tests/client/client-stamp.test.js tests/client/sensing-instruments.test.js
```

Log: `.phoenix/increment14-sensing-integrated-js.log`. This is automated DOM/action
coverage, separate from observed browser, phone or native acceptance.

## Runtime foundations and verification in progress

The focused owner now retains timed paid enquiry work and historical readings.
It checks the shared Sensors path before spending work; opaque intervening
terrain prevents completion even with clear endpoints. Passive reception reads
actual paid machinery expenditure and publishes anonymous measured bearings.
Spatial field state uses a causal tick anchor, deterministic sampling and shared
effect-specific paths; technical networking is unchanged.

Runtime test compilation first found an inaccessible angle helper, then three
fixtures requiring the new explicit ScanBlackboard and EntityConfig fields.
These are recorded failures, corrected in source. The first runtime run then
failed all four newly selected cases at strict Cruiser loading: passive trees
were compiled by runtime but absent from authored admission. Admission now uses
the same supported bindings. A later integration build caught a duplicate field
guard and incorrect use of RegionEffectsConfig as a vector; both were corrected.

At `77d4503a` plus the retained integration patch and new focused/passive files,
the targeted command passed **11 Gameplay and five Simulation cases**, with zero
failures or ignored cases:

```text
cargo test -p phoenix-sim-gameplay -p phoenix-simulation --lib --no-default-features -- science::focused science::passive regions::field regions::signal_path regions::server::tests::restored_field --test-threads=1
```

The build completed in 4m02s; test execution was 0.00s and 0.30s respectively.
The log `.phoenix/increment14-sensing-field-targeted-retry.log` has SHA256
`8881c5e4989f8a89971909ee9dee0998fd5e4f1358838ddea342e576fa48f5cf`.
The retained tracked-source patch has SHA256
`e589caaac9f439cebe2cab2412f46df5e54e3adb505b16042cc082add5157d50`.
New source files and matching owner dependency records are copied in
`.phoenix/increment14-sensing-field-passed-dependencies/`. Later startup/spawn-origin
field admission guards are source changes requiring their own consumer evidence.

Continuation adds field and observation ownership to existing snapshots/digests.
The compatibility boundary is protocol 13, snapshot format 52, rules 0.24;
pre-increment state must fail explicitly rather than silently omit new work.
Independent review identified and corrected focused path bypass, battery-powered
emissions, hidden-source ordinal leakage and zero-strength reception. Field
origin/product bounds and first-Input restore projections are also being checked.

## Design verification

The adopted increment 14 PASM contracts passed validate, scan and traceability
using an isolated workspace at base `86f37307` with the then-current integration
files copied in. This avoids recursively scanning the main workspace's nested
worktree/build forest. It verifies those contracts and that source checkpoint,
not subsequent uncommitted runtime changes.

The retained logs and SHA256 values are:

| Log under `.phoenix/` | SHA256 |
| --- | --- |
| `increment13-14-pasm-validate-isolated.log` | `76fa91ae7d8c807ad9e2992ce83a2c8ad1fd79a8f05bf710f99c0e52217c46a2` |
| `increment13-14-pasm-scan-isolated.log` | `4eda7596ebc7ac02e312dca5d774cca7a8dd49066d973332dbdf956a5f4db451` |
| `increment13-14-pasm-traceability-isolated.log` | `899552610dfc76b4e4da865d1f917b58fe126ee1b9d110c30db03cc5b6d423eb` |

Validate reported `Status: OK`; informational findings are not failures. Earlier
main-workspace attempts were cancelled or failed sandbox access and do not count.

## Remaining playable exit

### Per-instrument radar integration checkpoint

`6732439f` integrates independent broad profiles/states for each fitted radar,
including Workshop array editing and explicit finite coverage for six NPC hulls.
The source changes their former zero/unbounded AI radar horizon to the existing
authored AI sensor ranges. This is intentional tuning requiring scenario and
balance comparison; it is not an equivalence claim. The isolated authoring batch
passed 113 JavaScript cases across eight suites. Rust cases were not run there.

The subsequent integration adds broad state to capture, whole-candidate preflight,
restore and deterministic folding; measured Helm/Tactical/Science publications;
and observer-local contact routing through the consuming System's permitted
Console observations. Target permission is measured from each radar's actual
`selects` filter, independently of identification. An unselectable or predicted
return cannot acquire a target. The fitted Science/Tactical decision paths consume
their permitted recorded tracks instead of world-query candidates.

The renderer-independent library check passed after nesting the extended Helm
query within Bevy's tuple limit:

```text
cargo check -p phoenix-simulation --lib --no-default-features
```

The successful check took 20.86 seconds. Its log is
`.phoenix/increment14-broad-routing-check-retry.log`, SHA256
`0e6859608fdcea43daf6f1e94aca60e56997f3abf645f5e0e5abca9e41f1df2a`.
The tracked-source patch is `.phoenix/increment14-broad-routing-checked.patch`,
SHA256 `c7c8c274f969f85c59b947eeb96b6d0d4955d05c05206affbf74a344e4f72abc`;
the new contact-routing modules and matching owner dependency record are retained
in `.phoenix/increment14-broad-routing-check-proof/`. This is a compile check,
not passing Rust test or gameplay evidence. Later aggregate-routing/restore
joins and the outgoing disclosure projection still require integrated checks.

Measured radar client integration passed 425 cases across two suites:

```text
npx vitest run tests/client/sensor-tracks.test.js tests/client/console-state.test.js
```

These include empty-picture handling, targeting permission, recorded target facts,
uncertainty and combination of already-permitted Helm/Tactical feeds. An empty
track array never falls back to live entity snapshots. They do not establish
browser/native acceptance. `acf4c87c` adds reviewed browser/native outbound
filtering and explicit chart tags; its eight new native/Simulation tests remain
unexecuted at this checkpoint.

Automatic broad acquisition, retained uncertain tracks, permitted radar and
command routing, and ordinary closed-tree sensing controllers are now integrated
in source. Their complete-host validation remains open. Explicit sharing/fusion,
unusual-reference work, focused aims, forecasts and complete terrain/content
migration remain required. Complete-host continuation and opposed-order multiplayer tests,
shipped scenario journeys, Workshop rehearsal, browser/native operation, station
activity, baseline performance and final integration gates remain required.

Increment 15 runtime has not started. No push is requested or claimed.

### Merged instrument controls

The merged measured-radar and canonical `scans[]` instrument-picker controls pass
468 tests across five suites. The command was:

```text
npx vitest run tests/client/sensor-tracks.test.js tests/client/console-state.test.js tests/client/sensing-instruments.test.js tests/client/ph-scan-readout.test.js tests/client/sensors-actions.test.js
```

The retained log is `.phoenix/increment14-radar-focused-client-tests.log`.
This covers pure client projection and controls, independently of native Rust
execution and human browser acceptance. A previous sandboxed attempt could not
write its temporary transform cache and ran zero tests; its authorised retry
passed and is recorded separately.

### Integrated measured-routing owner evidence

The integrated renderer-independent Simulation command passed **59 cases**:

```text
cargo test -p phoenix-simulation --lib --no-default-features -- science::contact_routing::tests science::broad::tests science::focused::tests science::focused_ai::tests console_delivery::tests server_app::entity_removal::tests resources::recovery_server::tests resources::equipment_recovery_server::tests
```

The log is `.phoenix/increment14-measured-integrated-owner-tests.log`, SHA256
`b8e69f4f0d22d9a2c73aa229ee6f9a78418fb0fca78e32540f1166a3386b9950`.
Compilation took 4m49s; the 59 cases ran in 0.58s. They exercise actual paid
broad/focused instruments, report substitution, Console permission, focus-only
guess refusal, lifecycle projection, and ordinary Human/AI Recovery execution.
Private physical associations remain machinery inputs; public target labels come
from held observations. Lost contact does not erase admitted Recovery work.

The earlier 29/30 run exposed a fixture expecting a second Tactical instrument
without a Science feed. The corrected test first proves refusal, then explicitly
grants that feed and proves receipt. That corrected case passes in the 59-case
integrated command above. No production permission rule was relaxed.

The separate Gameplay track/composition command passed **27 cases**:

```text
cargo test -p phoenix-sim-gameplay --lib -- science::tracks::tests science::sensing::tests
```

Its log is `.phoenix/increment14-reported-track-composition-tests.log`. Matching
Simulation dependency evidence, the tracked-source patch and new routing modules
are retained under `.phoenix/increment14-measured-owner-proof/`.

After combining all already-permitted SensorRadar pictures, five client suites
pass **469 cases** (`.phoenix/increment14-multiple-radar-client-tests-retry.log`).
The new browser `@core` focused-instrument journey passes on a freshly rebuilt
pure-JS client, using the repository's pinned Playwright runner and its own port:

```text
cd tests/smoke
PHOENIX_SMOKE_PORT=3096 npx --no-install playwright test sensors-console.spec.js --project chromium --grep 'independent focused'
```

The browser log is `.phoenix/increment14-focused-browser-smoke.log`, SHA256
`f0ea40be3f940f237f3b3142d12d7dd2e34ed39db364acaa0e467b5288ec38b9`.
It proves instrument choice, exact-owner correlated actions, delegation and stable
working view in the actual shipped screen. It does not exercise WASM host state,
multiplayer, native monitor displays or the full increment 14 playable exit.

### Measured viewscreen and issued forecast slices

The isolated measured-viewscreen candidate `1b3fd30e` passed World **2**,
Presentation **15**, and renderer-independent Science server **19** cases. The
initial Presentation compile stopped on an unsupported read-view iterator API;
the committed fix uses sorted permitted-key lookup and the retry passed. Exact
commands, logs, hashes and source/dependency evidence are retained in
`.phoenix/worktrees/increment-14-measured-viewscreen/.phoenix/increment14-measured-native-handoff.json`.
The later generated-rock visual follow-up `4fe7d32e` has independent source
review; its added native visual case is not included in those earlier counts.

The forecast/track pure owner command passed **26 cases** (54.06s build):

```text
cargo test -p phoenix-sim-gameplay --lib -- science::forecast::tests science::tracks::tests
```

Its retained log is `.phoenix/increment14-forecast-owner-tests-retry.log`, SHA256
`97791bba9ce38f5b3534f259b5800129fa8fbf1590e5fbc32a99b95ae95252a6`.
The initial compile caught a moved string in a new synthetic-report fixture;
that fixture is corrected in the passing retry. Independent arithmetic review
also caught loss of adjacent integer quantities above binary64's exact range;
such capacity projections are now withheld and covered by a regression.

Forecast GUI and instrument tests passed **30 cases**, with log
`.phoenix/increment14-forecast-client-tests.log`. The extended actual `@core`
focused-instrument browser journey passed **1 case in 3.3s** on a freshly rebuilt
pure-JS client. The browser log is `.phoenix/increment14-forecast-browser-smoke.log`,
SHA256 `26a127d52bc380c0dc2f2fb74658f93f8f69202cd4efa577ff981a72fa655283`.
It checks original samples/horizon/range, exact-owner actions, delegation and
removal of another instrument's forecast when switching. These results do not
establish full WASM-host, native monitor, multiplayer or increment exit acceptance.
