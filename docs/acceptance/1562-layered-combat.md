# Increment 15: layered combat evidence

Implementation is in progress. This record does not establish a playable exit,
full integration gates, browser/native device acceptance or combat balance.
The [implementation contract](../gameplay-redesign/implementation/increment-15.md)
remains the scope and completion boundary. Local main's increment 14 evidence is
recorded separately in [sensing and fields](1562-sensing-environment.md).

## Integrated source

The increment 15 integration branch includes reviewed source for prepared weapon
groups, Console-filtered assignment views, finite shared turrets, causal EMP
resistance receipts, optional acquired shield-frequency findings, observed shield
cover, strict observed target filters, finite-round equipment and frozen guided
profiles. Sustained beam execution freezes its operation in the existing active
slot and includes capture, restore and digest support.

The integrator's current executor work joins ordinary human and AI commands to
the same per-System assignments and permitted measured aims. Explicit Hold and
lost observations retain intent without falling back to another target. A fitted
Console missing its immutable scope cannot borrow legacy scene knowledge.
Selection filters do not confer physical collateral immunity.

Finite barrel work resolves a managed Local store to its actual Capacity
endpoint. It stages the existing stock and Power owners together, preserves
other reservations, and consumes one actual barrel's work. Recoil projects the
emitted projectile bearing onto both ship axes. Missing listed physical barrels
must be skipped at their original indices; implicit legacy centre origins remain
supported. Measured beam delivery requires a real physical intersection.

Compatibility is advanced explicitly to snapshot 53, rules 0.25 and protocol 14;
the content epoch remains unchanged. These are unreleased increment boundaries,
not migration support for older saves or external content.

## Targeted execution so far

Runs use one sequential Cargo stream with the shared repository target directory.
The Simulation source timestamp is refreshed when switching source trees. Logs
live under the integration worktree's ignored `.phoenix/` directory.

| Evidence | Result | Limits |
|---|---|---|
| Model EMP and shield-enquiry owner run | 11 passed | Shared rules and serialization; no complete combat journey |
| Gameplay assignments, weapon operations and turrets | 13 passed | Targeted owner tests; no PDC or complete-host continuation claim |
| Guided-round Model owner run | 2 passed | Required frozen profile and active numeric bounds |
| Combined matching client files | 17 passed in 4 files | Assignment controls, beam Stop, observed shield cover and group editor; no browser-host acceptance |
| Combined Workshop and client run | 58 passed in 8 files | Existing weapon, group and shield editors plus matching controls; no disposable Test or device acceptance |
| Earlier composed Simulation selection | 15 passed, 4 failed | Bare capture registrations and a measured-position fixture expectation were corrected; affected cases await a new integrated run |
| Composed renderer-free check after frozen-flight integration | Failed at 8 old snapshot/digest field references | Guided profile snapshot join is being integrated; this is not a passing compile result |
| Composed native weapon/restore selection | 32 passed, 9 failed | Actual measured beam miss and friendly collateral, assignments, EMP, mount and guided continuation passed; new content/fixture premises required corrections |
| Affected composed native retry | 11 passed | Corrected Broadside feed, measured bearing and actual ECS baseline; includes sustained operation, finite round transactions, readiness and forged guided candidates |
| Affected Gameplay charge retry | 1 passed | Unfunded due steps are skipped while an existing emitted projectile continues moving |

The retained logs are `increment15-composed-model-tests.log`,
`increment15-composed-gameplay-tests.log`, `increment15-guided-model-tests.log`,
`increment15-composed-client-tests.log` and `increment15-composed-check2.log`.
Earlier results apply only to their tested source, before subsequent integration
changes. A later retry must be recorded as a retry, without rewriting a failed
batch into an all-green result.

The later receipts are `increment15-composed-simulation-tests-retry.log`,
`increment15-composed-simulation-fixtures-retry.log`,
`increment15-charge-case-retry.log`,
`increment15-composed-authoring-client-tests.log` and
`increment15-executor-source-receipt.json`. The first composed test build also
found mechanical fixture imports, obsolete torpedo fields and exhaustive schema
patterns; those were corrected without replacing exhaustive patterns with `..`.
The affected retry does not claim a fresh passing run of the whole earlier batch.

PASM validate, scan and traceability passed after recording explicit state-owner
mirrors: 1,083 entities, 3,605 scanned files, 7,693 observed dependencies, 18 Cargo
packages and 246 traceability rows. An initial missing-owner-mirror failure is
retained separately from the corrected run. The layer dependency check passed
with path-canonicalization warnings retained in its log. The affected wiki lint
resolved 57 source references and all 71 index links. The restored Workshop build
script passed `node --check`; a full browser bundle is not claimed.

## Measured defensive targeting checkpoint, 15 October 2026

Source `d767ee99` retains automatic fallback in the canonical per-System assignment,
settles admitted human/AI target commands before Physics, and uses permitted dated
elevation for missile eligibility and full spatial range. Explicit individual/group
orders, including Hold, remain decisive. The ordinary bank AI selects from current
permitted approaching observations using its own known relative motion; it reads
neither launcher allegiance nor private projectile orders. Actual finite turrets
consume the same filtered designation. This is an integration checkpoint, not the
completed interception/launch journey.

| Targeted evidence | Result | Limits |
|---|---|---|
| Pure interception geometry | 2 passed | Actual moving lead and spatial sweep rules |
| Pure elevation, assignment precedence and 3D flight | 12 passed | Includes paid sample/error bounds, sample-derived vertical motion, inherited observations and frozen emitted motion |
| Automatic-target client projections | 11 passed | Public pending/unavailable/override controls; no device acceptance |
| Native assignments, physical mounts and spatial bolt snapshots | 15 passed | Includes missing height, full spatial range, own relative velocity, explicit Hold, current Conceal, real refit freeze and exact settled JSON continuation |

The renderer-free Simulation check passed after correcting the composed Bevy query
tuple limit. The first native build found three older measurement test literals
missing the new optional elevation field; those fixtures now explicitly retain
unknown height. The affected native selection then passed. No test or threshold was
removed or relaxed. `increment15-aim-proof.json` in the ignored integration directory
pins the source hashes, formatting-only dirty files and log hashes. Subsequent
Rust 2021 formatting changes do not alter these tested behaviours.

Logs: `increment15-parallel-joined-pure-tests.log`,
`increment15-parallel-elevation-assignment-tests.log`,
`increment15-auto-target-client-tests.log`,
`increment15-aim-elevation-check-retry.log` and
`increment15-aim-elevation-native-tests-retry.log`. The later native run is 15 tests;
it does not replace earlier failed batches or claim the remaining physical PDC,
launcher-loss, guidance, complete-host, balance, performance or human exits.

## Remaining exit work

The first composed physical-carrier selection ran 17 native tests: 12 passed,
5 failed (`increment15-physical-carrier-native-tests.log`). All five physical PDC
cases passed, including real collider shapes, moving crossing, unknown blockers
and finite paid salvo saturation. Carrier despawn/replacement and mixed component
registration-order cases also passed. Sequential replacement exposed a real copied
flight duplication; four restoration comparisons exposed incomplete bare-host
initialisation and remain under investigation. Preserve the exact continuation
assertions and record affected retries separately. This is not a passing carrier
exit or complete combat journey.

The later guided-phase selection against `af38ad5d` ran 45 native tests: 41
passed and four valid-restore digest comparisons failed
(`increment15-guided-phase-native-tests-retry.log`). The copied-flight duplication
and carrier settlement cases now pass; malformed carrier candidates also preserve
the captured state and digest. The four remaining failures occur on valid restored
hosts, not on those malformed-candidate refusals. The initial build's three old
guidance-schema fixtures were migrated with their assertions retained.

Scope diagnostics at `fb70f64c` reproduce the valid-restore failures and identify
the empty asteroid namespace as the first difference; run, entity and resource
stages agree. Restore's normal derived radar query registers spatial components
that the bare fixture had omitted. `cb303175` registers that normal host roster;
production digest semantics remain unchanged. The affected retry, expanded with
actual seeker acquisition and same-step interception-before-payload coverage,
is pending. The diagnostic log is `increment15-carrier-digest-diagnostics.log`;
an intervening new-fixture constructor build failure is retained separately.

The combined adopted lane/state-owner PASM checks pass: validate reports 1,111
entities and `Status: OK`, scan reports 3,626 files and 7,735 dependencies, and
traceability reports 246 rows. This pins the contract preparation through
`1c297adf`, before the later 22A evidence contract. Current-runtime wiki lint
resolves 63 source references and 71 index links. Human measured-elevation details
retain the original sample tick; their three targeted client suites pass 68
tests, with no browser/device acceptance claimed.

Complete and verify guided-flight/pending-burst continuation, independent flight
after launcher loss, observed interception and saturation, guidance/link loss,
independent Science illumination, payload observations and acquired approach
advice. Exercise actual admission, delegation, stale-host rejection, optional
equipment, stock replenishment and active-state save/reconnect. Review corrected
physical-hit and barrel behavior independently and run the affected owner tests.
Record representative combat and non-combat activity across supported hull sizes.

Repository integration gates run once on the integrated result before a push.
No push has been requested or performed for this work.
