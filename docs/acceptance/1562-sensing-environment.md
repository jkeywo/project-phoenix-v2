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

The integrated forecast/Broad/continuation/outgoing batch then passed **31 cases**
(5m56s build, 0.22s execution):

```text
cargo test -p phoenix-simulation --lib --no-default-features -- science::server::tests::forecasts_use science::broad::tests science::focused::tests console_delivery::tests
```

The log is `.phoenix/increment14-forecast-rock-integrated-tests.log`, SHA256
`7ff783f69b7545bc6d3c0a2be6e1d723a3ce1ac6eda0fe7f0661ba3dd11749ad`.
The tested source is `0a63c743` plus the retained pre-target-metadata Broad patch
under `.phoenix/increment14-forecast-rock-proof/`, including matching library
dependency records. This proves issued forecasts remain unchanged after live
condition changes, projection loss and earlier-sample pruning; invalid forecast
intervals refuse validation.

Independent review subsequently found generated rocks were using general display
tags for target selection. `e68ab8a8` now preserves actual template `target.tags`
through streaming and the existing whole-roster restore path. The Broad producer
uses those separate tags, with a source-reviewed regression for differing display
and target tags and absent-target denial. That correction and its real streaming
restore regression await the next integrated native batch; the earlier 31 cases
must not be read as post-correction execution evidence.

### Integrated deliberate radar sharing

The source checkpoint `93baf3c9` adds actual Sensors/Tactical faction feeds,
original sample provenance, receipt history, geometry-only refinement and
Workshop controls. Integration retains the synthetic-report safeguards and
selectability rules, adds ordinary admitted sharing controls, and publishes
the same permitted fused picture for client routing and machinery handles.

The no-renderer compile check passed after correcting a source validation block
that had been placed on mount geometry instead of `SensingConfig`. A subsequent
check of canonical physical endpoints and radio/jam availability passed in 12.11s:
`.phoenix/increment14-sharing-physical-endpoint-check.log`, SHA256
`396a32a2df9b767038ab6bb971e38b383225b1458745a26f904e26e1c9c56a74`.
The actual sharing native cases remain unexecuted at this checkpoint.

Six integrated client/Workshop suites passed **459 cases**:

```text
npx vitest run tests/client/radar-sharing.test.js tests/client/radar-sharing-projection.test.js tests/client/sensor-tracks.test.js tests/client/console-state.test.js tests/client/workshop-sensing-panel.test.js editor/tests/workshop-sensing.test.js
```

The log is `.phoenix/increment14-sharing-client-integrated-tests-retry.log`,
SHA256 `557895c12fa042dde3715e7d7375f2394cf1e93c3c7c88098eb725c11befc289`.
The initial run exposed a wrong import in the new authority-projection test;
the corrected retry passed. Tests cover exact ownership, typed-feed read-only
views, co-host isolation, ordinary AI delegation, report parity and editing.

The rebuilt actual Sensors `@core` journey passed **1 case in 3.5s**, exercising
the sharing checkbox, exact System/correlation envelope, delegation, and existing
focused/forecast controls. Log: `.phoenix/increment14-sharing-browser-smoke.log`,
SHA256 `ded9f08c6f1a6d40b43d73e723e92e733e8c5e04d59d23ed41dc297e03587198`.
Native sharing, whole-host continuation, multiplayer and human acceptance remain
separate outstanding evidence, alongside the remaining selected increment scope.

### Reference and reading integration checkpoint

The reference source checkpoint `5175d434` is connected to Model command/DTO
contracts, exact-kind admission, the existing Science schedule and actual
permitted Console projection. Shared-contact refresh preserves deliberate
attachments. Candidate restore preflight checks marked evidence identity and
original acquisition time before accepting attachments, and validates private
AI reading against the captured logical tick independently of tree cadence.
Evidence reference metadata is included in the deterministic digest.

The integrated Console/reference client command below passed **419 cases** on
2026-10-15; an initial sandbox run failed before test execution because temporary
file renames were denied. The unrestricted targeted retry passed.

```text
npx vitest run tests/client/console-state.test.js tests/client/sensor-references.test.js
```

After rebuilding the pure-JS client, the actual Sensors reference `@core`
journey passed **1 case in 5.8s**, including explicit opaque-contact attachment,
correlated exact-owner command, preserved search and disabled controls after
delegation. The sandbox browser attempt was interrupted when its server could
not become usable; the unrestricted run on a fresh port passed.

```text
PHOENIX_SMOKE_PORT=3098 npx --no-install playwright test sensors-console.spec.js --project chromium --grep 'held unusual reference'
```

This is client acceptance evidence. The new snapshot and reading joins still
need native execution and whole-host continuation; this checkpoint does not
close increment 14.

### Conditional observed-ordnance approaches

The existing in-flight torpedo owner now supplies physical position samples to
ordinary paid Broad instruments. Radar vocabulary, range, signal paths and
concealment constrain acquisition; no launcher, guidance target, true velocity
or firing-tube identity is included in public records. The existing Missile tag
also covers authored `torpedo` filters. Recognition and measured velocity follow
the ordinary observation lifecycle.

Helm's approach projection reads only its permitted measured Console feeds plus
own motion. Optional `helm_console.projection` reuses the existing positive
forecast-horizon policy; the Cruiser's 60-second horizon is provisional display
tuning. The result states its constant-observed-motion assumption, shows a
growing corridor and separation interval, and does not issue steering commands
or claim an impending collision. Unknown motion/error metadata and conflicting
copies are withheld. Numeric bounds include coordinate narrowing and are rounded
outwards before publication.

Targeted Helm/Console client suites passed **438 cases**. After rebuilding the
pure-JS client, the actual Cruiser Helm `@core` journey passed **1 case in 5.2s**:

```text
npx vitest run tests/client/ph-helm-radar.test.js tests/client/console-state.test.js
PHOENIX_SMOKE_PORT=3099 npx --no-install playwright test helm-observed-approaches.spec.js --project chromium
```

Strict strings passed with 5756 entries and no errors/warnings. The new pure
calculation, actual projectile observation and permitted-feed publication tests
are authored but have not run natively at this checkpoint. Visual Workshop
enable/disable authoring, broader integrated continuation and increment exit
acceptance remain pending.

### Integrated reference, enquiry and Navigation checkpoint

At `afe7cbf2`, the targeted no-renderer Gameplay command below passed **31
cases**, covering conditional approaches, reference correlation, deterministic
AI reading and forecast/sensing calculations:

```text
cargo test -p phoenix-sim-gameplay --lib --no-default-features -- science::approaches::tests science::references::tests ai::text_reading::tests science::forecast::tests science::sensing::tests --test-threads=1
```

Log: `.phoenix/increment14-integrated-reference-approach-gameplay-tests-retry.log`,
SHA256 `7c73356983848db7ff1099a01389245bf622611dc0f6bfafd14926c79280bf6d`.
The initial attempt found a missing `reading_ticks` field in a full fixture
literal; the corrected retry passed. This is not Simulation or whole-host
continuation evidence.

Navigation sharing and Helm projection authoring are integrated through
`680c6e57`. The no-renderer Simulation compile check at `8b0da994` passed,
and eight integrated Navigation/Helm/Sensing client suites passed **502 cases**
at `680c6e57`. Strict strings passed with **5799 entries**, no errors or warnings.
These totals overlap earlier client runs and must not be added to them.

```text
npx vitest run tests/client/navigation-sharing.test.js tests/client/navigation-sharing-projection.test.js tests/client/workshop-sensing-panel.test.js editor/tests/workshop-sensing.test.js tests/client/workshop-helm-projection-panel.test.js editor/tests/workshop-helm-projection.test.js tests/client/console-state.test.js tests/client/ph-helm-radar.test.js
```

Logs: `.phoenix/increment14-navigation-integrated-check.log`, SHA256
`7a8777828f0ae883fc8949c8c30e3e2a7bebf71e8e1b1f78b4987ffeee80efae`;
`.phoenix/increment14-integrated-navigation-helm-client-tests.log`, SHA256
`ec2931c28a9f19f17a05fe89cbdfd1050be6577b6dfca6c9bdc96550e1bfe250`;
`.phoenix/increment14-integrated-navigation-helm-strings.log`, SHA256
`b1adcd74212ea19df84414133cbcc4645a6c6d2784d58699c7ee30c58d8282e6`.

The first combined Simulation batch against `680c6e57` plus the two explicit
schema/DTO fixture additions **failed: 24 passed, 26 failed**. The passing cases
include reading, reference preflight, enquiry sampling, observed approaches,
Navigation sharing and strict shipped-hull AI loading. Failures comprise twenty
scan fixtures retaining sharing references to removed instruments, four enquiry
fixtures missing their fitted passive-receiver profile, a Navigation restore
fixture missing its repair-team owner, and the explicit Live schema ratchet.
Corrections and affected retries are in progress; the batch is not green.
Log: `.phoenix/increment14-integrated-enquiry-reference-navigation-owner-tests-retry.log`,
SHA256 `823ff6b3af2bf83719f994b8fbde60f0fabb88e5b28fce312a0af2ac3772e40d`.

The affected server/schema retry passed **21 cases**, against `680c6e57` plus
the explicit schema/DTO literal additions, corrected fitted sharing references
and reviewed exact schema ratchet. The initial batch's other five failures
remain pending and are not covered by this retry. Cargo reported a filesystem
hard-link cache warning and copied the files; no code warning was emitted.

```text
cargo test -p phoenix-simulation --lib --no-default-features -- science::server::tests complete_active_schema_inventory_is_ratcheted_across_all_five_domains --test-threads=1
```

Log: `.phoenix/increment14-server-schema-fixture-retry.log`, SHA256
`d7c7868dea9728245a65d076b5c5ca732a23eaa510391854f4a7e713034e82f0`.

Separately, the frozen sharing worktree completed Gameplay **32**, Model Cargo
**11**, Simulation **36**, Presentation **14**, and the whole-host generated
asteroid **1** targeted cases at their recorded pins through `6a2a60fe`.
Its receipt is `.phoenix/worktrees/increment-14-sharing-validation/.phoenix/validation/receipt.json`,
SHA256 `32e8bafd744a9c4228bcb0f4eaf713bfe61621447bacfdf1e4d53a8315f62d07`.
Those pins precede the newer integrated reference, enquiry, Helm and Navigation
changes; they do not prove the combined main result.

The remaining information-access audit covers Helm AI, Navigation anchors and
traffic, and Dock/Tractor/Transporter public-contact routing. Whole-host
continuation, affected browser journeys, performance/activity, multiplayer and
human/native acceptance remain open. Implementation evidence publication for
increments 9–18 is explicitly authorised; the current published checkpoint is
[PRD #1562 comment](https://github.com/jkeywo/project-phoenix-v2/issues/1562#issuecomment-6186902157).

### Reviewed contact boundaries and unified native handoff

`2189e205` closes machinery ingress/egress through permitted public handles;
`0b77fc20` builds fitted Helm AI inputs from measured findings; `b5d9710e`
does the same for Navigation and retains successful own traffic requests per
public contact. Independent source reviews passed after correcting duplicate
observations, concealment, legacy egress, simultaneous traffic requests and
historical scans incorrectly refreshing live personnel counts. The latter
counts are withheld until a supported personnel observation exists; own active
transport/progress remains operational state. Transporter has no current phone
widget and gains no invented UI acceptance claim.

The no-renderer compile check after the machinery/Helm integration passed.
Four measured-Helm Gameplay cases then passed, including observed velocity,
unknown foreign reach/arcs and ordinary own weapon arc assistance:

```text
cargo test -p phoenix-sim-gameplay --lib --no-default-features -- measured_tests --test-threads=1
```

Log: `.phoenix/increment14-helm-measured-gameplay-tests.log`, SHA256
`c88268547cb2ccd5e68df8b066f120f9158bd38125124c037bf789091870c215`.
Three targeted Model enquiry cases also passed at `9193b6ec`; log
`.phoenix/increment14-typed-enquiry-model-tests.log`, SHA256
`b9318f8a1f73c37095fe821f487f677651d3f8783ecd107344c55d0cce3df6ad`.

Actual Dock/Tractor `@core` Chromium phone journeys passed **2 cases in 6.4s**
on the isolated machinery source and rebuilt pure-JS client; their reviewed
source is integrated as `f394aafc`. They exercise actual widgets, keyboard
release and correlated action-map envelopes with opaque contacts; they do not
establish Rust/WASM acquisition or admission behavior.

At the initial handoff, unified native validation ran in
`.phoenix/worktrees/increment-14-native-validation`, initially pinned to
`b5d9710e`. Its first compilation found three Navigation test-harness errors
before executing tests. The reviewed fixture corrections are main `75ccfb94`
(donor `4f0b2fe4`). At that handoff, whole-host enquiries,
Navigation continuation and the combined playable exit were pending. Root
continues increment 15 source in isolated worktrees; its contract `ee3bd7b0`
and in-progress implementation are not completion evidence.

### Unified native validation and continuation

The isolated native stream starts at integrated `b5d9710e`. Its logs, exact
source pins, status files, byte-preserving library refresh records and matching
Cargo dependency records are retained under `.phoenix/increment14-native-validation/`.
The shared Cargo target was used sequentially; no other worker compiled during
this stream.

The selected Simulation owner batch first ran **120 passed / 13 failed**.
Corrections restore actual finite machinery in sparse fixtures, register the
actual optional capture owners, use the actual permitted publication kind and
retain exact equality assertions. The existing supported publication registry
now also recognises Navigation, SensorRadar and TacticalRadar instance IDs;
wrong-kind and unregistered feeds still fail validation.

At `7c3d6717`, **132 passed / 1 failed**. The remaining focused continuation
fixture was missing normal publication and field-cache rebuilding. Its affected
retry at `a7e3a492` passed the exact 360-step JSON and digest comparison. This
combination covers the selected 133 cases; it is not a fresh all-133 passing run.
The associated Gameplay selection at `670d577f` passed **6 cases**, including
real custom-ID publication round trips and wrong-kind rejection.

Actual headless continuation exposed a separate runtime publication-frontier
bug: publishers run before the logical counter advances, but restore rebuilt
held radar uncertainty at the next counter. `92a9b793` preserves only saved
tracks that agree exactly with the canonical instrument state at the saved or
preceding fixed step, after rebuilding missing views. Its affected **10 cases
passed**, including exact focused continuation, positive paid radar acquisition,
forged/missing/wrong-kind/lost-current publication refusals, spatial-field
continuation, Navigation sharing and historical forecasts.

At `3cce032e`, the actual `sensing_enquiry_continuation` journey passed **1 case**:
ordinary Identify/Ready/rating admission, a paid intrusive Resources enquiry,
AI handoff preserving that same observed request and mode, a later ordinary
General enquiry, detectable passive scan exposure, exactly one contextual World
reaction, 360 exact complete captures/digests and forged candidate no-write
checks. The two actual `snapshot_resume navigation_` cases also passed.

The multiplayer batch at that pin was **16 passed / 2 failed**. The failures
were retained: the old guard read retired hull percentage state, then correctly
measured physical damage and exposed Alliance ships lacking an actual Target
capability. The four base hulls now explicitly declare ship selection tags.
Their paid four-hull regression passes, including removal of actual Target
withholding both public selection and Console routing; display tags are never a
selection fallback.

The resulting real combat then exposed an entity-digest restore mismatch:
shield-only beams retained a target key without a physical sample, although the
snapshot stores only complete retained samples. `ff85bb09` installs the key only
when a real physical hit returns a sample. Its actual writer test passes through
shield-only contact without physical RNG, then hull damage, sample reuse and
retargeting. Snapshot format and digest folding remain unchanged.

The full `lockstep_snapshot_transfer` binary then passed **18/18 cases** at
`ff85bb09` (2m03 build, 39.77s execution). This includes actual compartment damage,
exact transferred digest at tick 399 and 120 subsequent identical ticks, paused
restore, GM join/reconnect, corruption refusal and persisted-dimension coverage.
The same production checkpoint passed the shield-only beam case, the four-hull
Target case and two physical beam restore guards. Two additional older repair
fixtures failed in that broader six-case selection; their affected retry is
recorded separately rather than calling the six-case batch green.
At `e0341119`, those two affected repair fixtures pass (2m57 build, 1.07s
execution). The duplicate-task negative checks the exact earlier physical-task
preflight refusal and keeps all no-write checks. The positive runs the three
ordinary fitted publishers before capture in its bare host; full capture equality
and exact one-time parts consumption remain unchanged. This is a two-case
fixture retry, not a repeat of the six-case batch.

```text
cargo test -p project-phoenix --features headless --test sensing_enquiry_continuation -- --test-threads=1
cargo test -p project-phoenix --features headless --test snapshot_resume navigation_ -- --test-threads=1
cargo test -p project-phoenix --features headless --test lockstep_snapshot_transfer -- --test-threads=1
```

| Log in `.phoenix/increment14-native-validation/` | Source | Result | SHA256 |
| --- | --- | --- | --- |
| `owner-final.log` | `7c3d6717` | 132 pass / 1 fail | `13eb67a9b4574bde84f2e523f29d10c009364457e960fb493746da8e73ffe128` |
| `focused-fields.log` | `a7e3a492` | affected 1 pass | `6ff6482a5b2f7cd43c1945b5a0b38e52ea471a2568699f1ef79742771e8f7318` |
| `publication-owner.log` | `92a9b793` | affected 10 pass | `2a02a6579ae5627e2c047eaf3db0ad90786766f3dfe17f04b002f234f41119c0` |
| `wholehost-verified-enquiry.log` | `3cce032e` | 1 pass | `0af61331460d7b7cc78767a83bde769405c65ec5024194a1a16329bcfb4954ac` |
| `wholehost-verified-navigation.log` | `3cce032e` | 2 pass | `90310fd5de7e01b77485ab8c659bb92146e728f120187ed236657f36d558182e` |
| `beam-pair-owner.log` | `ff85bb09` | 4 pass / 2 fail | `38918b034b27b49846419cb5f8460130643417a7fc2085752a7807594aecaebb` |
| `beam-pair-mesh.log` | `ff85bb09` | 18 pass | `0f9b30aa643003ef525282e8dd8f74ad0f4b2debb8f88d427a960b326f6c26a5` |
| `repair-fixture-owner.log` | `e0341119` | affected 2 pass | `4e7437f0525a8ac0e88e856ed1390db9cf2c59d3070411291d23d52cb5c92278` |

These selections overlap; their counts must not be added into a suite total.

Known headless-build warnings remain for two native audio methods unused by this
feature combination, repeated process-global Bevy logger setup, and the duel's
dynamically computed template outside the frozen content ledger. These are
retained in the logs, not reported as warning-free acceptance.

These are automated, scoped results. They do not constitute the repository's
full integrated gates, balance/performance/activity evidence, native monitor or
human acceptance. The ignored legacy reconnect baselines were not run or counted.
The unselected older survey save journey still requires migration from raw UUID
scan requests and instantaneous scan timing; its compile-time field access was
updated without claiming that journey passed.
