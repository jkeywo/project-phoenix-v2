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

Canonical transfer-settlement changes, live recovery, port services, later
continuation coverage and full/manual acceptance still require their own evidence.
