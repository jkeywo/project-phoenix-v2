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
