# Increment 12: logistics and production acceptance

The pinned initial timed-delivery slice passes three targeted native integration
cases. This records automated evidence for Increment 12.1; the complete Increment
12 playable exit remains pending.

## Pinned automated evidence

Runtime `06470cf1`, reviewed tests `271a80bc`, protocol 9, base content epoch 3,
snapshot 47 and rules 0.19. The clean isolated worktree used renderer-independent
headless features and refreshed eight owner libraries plus the root library
without changing bytes before reusing the shared Cargo target. Compiler paths and
retained Gameplay/Simulation dependency records identify that worktree.

```text
cargo test -p project-phoenix --no-default-features --features headless --test logistics_continuation -- --test-threads=1 --nocapture
```

Result: **3 passed, 0 failed, 0 ignored**, with no filtered cases; build 3m43s,
execution 11.89s.

- Ordinary authored Cruiser AI carries a real positive batch across an authored
  connection. Complete JSON save/resume onto a different bootstrap preserves
  claims, partial work, actual stores, Power and private AI state. Whole-world
  digest and exact owner frontiers match for at least 720 subsequent steps.
- Four individually valid Stock shapes with corrupt incoming ownership or a
  missing saved Logistics owner are refused before any live writes. A positive
  restore control rules out unrelated changed-clock/alert refusals. Full before/
  after captures, digest and entity counts remain exact.
- Two real fleet ships on two peers with different LocalShip ownership exchange
  ordinary Mesh frames. At least 720 complete-world comparisons, four positive
  crossing witnesses and four completed-delivery witnesses prevent an idle pass.
  Both actual hulls finish both bounded batches.

Every populated gameplay step accounts for all six initial rounds in their actual
hold, magazine, loaded tube, pending burst or transit owner. Receiving promises
never count as ammunition. Completion bounds use existing authored work and route
costs; no timing, capacity or balance values changed. The two existing quiet-world
fixtures and shipped hull data are reused unchanged.

The run emits the known multiple-App Bevy logger diagnostic and the same two
no-default root-library warnings. These assertions passed; this is not clippy or
full-gate evidence. Independent source review passed separately from execution.
Exact commands, hashes, source provenance and limits are in the
[targeted native receipt](../gameplay-redesign/implementation/logistics-native-targeted-validation.json).

## Remaining acceptance

The batch-2/six-round result belongs to this exact 12.1 pin. Local parts/coolant
storage, later batch tuning, fabrication, authorised port replenishment and
supported acquisition require their own integrated evidence. Final repository
gates, matching browser/native/Workshop controls and authoring journeys remain
pending. Human acceptance, crew workload, balance and performance have not been
established by this automated batch.

## Local supply and fabrication checkpoint

At clean isolated revision `f454148f` (the `1899d876` local-supply/fabrication
integration, reviewed magazine/recall fix `01a9d9a7`, and the otherwise unused
pure recovery module), the renderer-independent Simulation owner command passed
**38 tests, 0 failed, 0 ignored**. Build took 5m03s; execution took 3.08s. It
includes all seven fabrication cases, local stock and delivery cases, admission
routing, every shipped hull, and the exact five-domain schema inventory pin.
Six owner libraries were refreshed without changing bytes; compiler paths and
the Simulation dependency record identify the isolated source tree.

```text
cargo test -p phoenix-simulation --lib --no-default-features -- resources::logistics_server::tests resources::production_server::tests resources::server::tests command_admission::router::tests complete_active_schema_inventory_is_ratcheted_across_all_five_domains strict_mode_accepts_every_shipped_hull --test-threads=1
```

The retained log is
`.phoenix/worktrees/increment-12-recovery-core/.phoenix/increment12-native-38-retry.log`,
SHA256 `2f7cbfedf4bcdc1c4a7764d82189b069777c4894c0fb5194fe17ee7622ac777a`.
The earlier `1899d876` run passed 36 and failed two cases: the magazine's sole
stock-control consumer was not registered as its machinery owner, and a recall
fixture retained demand for another independent batch. The production cases
already passed. The reviewed fix registers the real magazine interface and
lowers demand through the local operator before recalling the batch; conservation
and travel-time assertions remain intact. Both cases pass in the 38-test retry.

Separate owner evidence covers 47 pure resource/production/route cases at
`5132c036`, and eight pure recovery cases at `5c5a326d`. These are distinct source
pins. Integrated client checks passed 99 of 100 cases on their first run; the
remaining Workshop history case hit its unchanged five-second timeout during
concurrent compilation. Its unchanged isolated suite then passed all eight cases.
This is not a claim that the first 100-case run passed.

This checkpoint predates canonical transfer settlement and live recovery. Port
services, later continuation coverage and full/manual acceptance remain separate
evidence boundaries.

## Canonical transfer settlement

At isolated checkpoint `c9c1195d` plus the two-file fixture correction committed
as `074009cd`, the combined renderer-free owner command passed **29 tests,
0 failed, 0 ignored**: nine resource-settlement, twelve Umbilical and eight
Infrastructure cases. Build took 2m15s; execution took 0.26s. The final source
bytes match the committed correction; eight owner libraries were refreshed
without changing bytes and actual Gameplay/Simulation dependency records retained.

```text
cargo test -p phoenix-simulation --lib --no-default-features -- resources::server::tests:: umbilical::server::tests:: infrastructure::server::tests:: --test-threads=1
```

Resource cases include simultaneous finite supply, protected floors, receiving
claims/shared space, zero receipts and a mixed magazine/capacity bay. Prior
capacity effects settle through Infrastructure; atomic transfers then preserve
actual quantities and issue truthful receipts. The mixed-bay regression prevents
an earlier accepted capacity credit being rejected after a later magazine update,
which previously could credit another endpoint without a matching debit.

The retained root artifact is `.phoenix/increment-12-settlement-owner-tests.log`,
SHA256 `71deb9de5d7721d5df9f176af942b88e9f01340cb7d1b9210ee185d6e4bfa483`.
Immutable dependency records and provenance are under
`.phoenix/increment-12-settlement-dependencies/` and the two matching provenance
JSON files. Earlier CLI refusal, test integer-type compile refusal, and the
28-pass/one-failed run remain retained separately. The latter found an isolated
Umbilical fixture lacking the new required receipt queue; the correction installs
that queue and the real Infrastructure-before-transfer order. No production
assertion was weakened. This is targeted owner evidence, not full integration or
manual/browser acceptance.

## Finite recovery checkpoint

At main revision `478a5c48`, the renderer-independent owner command passed
**12 tests, 0 failed, 0 ignored** (2m07s build, 1.88s execution). Coverage includes
seven live Recovery cases, three snapshot/preflight cases, all shipped hulls and
the exact five-domain state inventory. Human and ordinary AI use actual admitted
controls, explicit contact feeds, paid energy and finite source settlement.

```text
cargo test -p phoenix-simulation --lib --no-default-features -- resources::recovery_server::tests snapshot::recovery_tests complete_active_schema_inventory_is_ratcheted_across_all_five_domains strict_mode_accepts_every_shipped_hull --test-threads=1
```

The log `.phoenix/increment12-recovery-integrated-owner-retry.log` has SHA256
`c6827101caf1d4699c8f703b3f38d3609311617c08971642dcdd9ab53c907b7b`.
The preceding run passed eleven cases and failed the unchanged state-inventory
pin: the new owners increased the measured Live count from 950 to 957. The retry
uses the measured inventory and fingerprint with exhaustive classification
checks retained. The copied dependency record under
`.phoenix/recovery-owner-dependencies/` identifies this main source tree.

At `87b44f42`, Model and Gameplay checks separately passed **14 cases**: three
port contract cases and eleven pure Recovery/profile cases (1m23s build,
0.12s combined execution). Log
`.phoenix/increment12-recovery-pure-and-port-contract.log`, SHA256
`48b4076de761d40ac3430a7514faaab473af0d7bb0203db910d595d765db4107`.
An earlier run passed eight pure cases but could not construct three profiles:
the test's JSON-to-TOML conversion rejected null values before assertions. The
corrected fixture uses the actual shipped ownership, equipment and AI definitions;
negative profile checks remain intact.

Revision `0084c280` additionally prevents a recovery manifest from relabelling
a managed stock's resource. All **three profile tests** passed (45.18s build,
0.11s execution), including the invalid declaration and repaired positive control.
Log `.phoenix/increment12-recovery-resource-identity-profile.log`, SHA256
`b12b5d0436e57d73625c00447d5bc2be9fb24d8e6130f26ba0947c49e67bb512`.

The first whole-host continuation run at `2649e5d6` failed all three cases:
test participants claimed stations without the ordinary Ready step, so real
command admission refused their controls. The corrected test journey is being
run separately. These owner results do not establish whole-host continuation,
browser/native acceptance, workload, performance or completion of increment 12.

## Port implementation checkpoint

The initial integrated port implementation is committed in `bd4fedd6`. Its
targeted native evidence belongs to isolated revision `5a3ca807`: **58 passed,
0 failed, 0 ignored** (Model 3, Gameplay 8, Simulation 47). Build took 2m38s;
execution took 3.40s plus the two pure suites' reported 0.00s each.

```text
cargo test -p phoenix-model -p phoenix-sim-gameplay -p phoenix-simulation --lib --no-default-features -- resources::ports_server::tests snapshot::ports_tests umbilical::server::tests resources::ports::tests ports::tests core::codec::tests::port_controls system_blackboard_umbilical_round_trips_with_explicit_port_intent complete_active_schema_inventory_is_ratcheted_across_all_five_domains strict_mode_accepts_every_shipped_hull --test-threads=1
```

Coverage includes actual goods/cost exchange, protected floors, shared receiving
space and full-endpoint net exchange; free goal maintenance and feasible priority;
ordinary AI without purchase authority; explicit trusted mission approval;
permission loss, donation, quota, undocking and partial/disabled machinery;
candidate-roster no-write refusal, actual barter save/restore and missing-provider
materialisation; both port wire payloads, shipped hulls and the measured Live
inventory. The broad `ports::tests` substring also selected existing reports
cases; 47 is the actual Simulation count, rather than a claimed 47 new port cases.

Log `.phoenix/increment-12-ports-owner-retry3.log`, SHA256
`bfc2d97089d6a03ebaff08282cd95d3afc301bd445b8de76880dbb0677bcf9e9`.
Eight owner libraries were refreshed on switching to the isolated tree; the
modified Gameplay and Simulation libraries were refreshed on subsequent retries.
Copied dependency records and hashes are retained under
`.phoenix/increment-12-port-tested-dependencies/`; Gameplay and Simulation records
identify the tested port worktree.

Earlier runs remain distinct: the initial compile refused an omitted explicit
EntityConfig field; `d939ec68` passed 16 and failed 17; `caa7d824` passed 54 and
failed three across the same three crates. The real startup defect was default
tick zero being mistaken for an already processed physical visit. An explicit
initialisation fact now preserves idempotence and quota at that frontier; boot
approval is valid while paid work begins at tick one. Other corrections retain
finite authored ceilings, publish changed fixture stocks normally, initialise
ordinary host owners/namespaces in sparse continuation fixtures, and adopt the
measured 972-row Live inventory and exact fingerprint. No stock, authority,
shared-space, full-capture or digest assertion was relaxed.

Separate evidence: the trusted World script contract passed its focused case at
`b2b32387`; port Workshop/client checks and later 17-case validation followup were
recorded independently by their workers. The merged comment and Workshop wording
followups do not change the native resource processor. Snapshot 49, rules 0.21,
protocol 11 and base content epoch 4 explicitly delimit supported state/content.

This is the authorised initial-commit checkpoint. Recovery's exact whole-host
continuation/clock correction remains in isolated validation; final integrated
gates, browser/native human journeys, balance, workload and performance remain
open. Advancing to increment 13 does not mark increment 12's verified exit passed.
