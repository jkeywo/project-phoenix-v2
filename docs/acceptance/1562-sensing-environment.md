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
The compatibility boundary is protocol 13, snapshot format 51, rules 0.23;
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

Automatic broad acquisition and retained uncertain tracks, permitted radar and
command routing, explicit sharing/correlation and reference work, focused aims,
ordinary closed-tree controllers, forecasts and complete terrain/content migration
remain part of this increment. The initial foundations are not a substitute for
those consumers. Complete-host continuation and opposed-order multiplayer tests,
shipped scenario journeys, Workshop rehearsal, browser/native operation, station
activity, baseline performance and final integration gates remain required.

Increment 15 runtime has not started. No push is requested or claimed.
