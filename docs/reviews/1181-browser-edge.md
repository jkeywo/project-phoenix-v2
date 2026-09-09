# Browser edge ownership (#1181)

Implemented over `644da210` for #1449 B3, after the #1248 pre-init resolver and
#1244 restored-seed proof. `src/server/bridge.rs` contains no `thread_local!`
declarations or direct cell borrows. The private `browser_edge.rs` adapter owns
60 browser storage items behind typed enqueue, take, read and publish functions.
No mutable closure, raw cell or stored Bevy World crosses this boundary.

The inventory reconciles all 61 original items:

- Transport ingress, disconnects, mesh frames and host loss/claim observations
  wait at the edge for scheduled drains. The existing queue and overflow
  policies are retained; this change does not impose new transport limits.
- Fleet join/leave generations, GM admission staging and save/import/export
  intents exist before an App and retain their existing incarnation and FIFO
  rules. Pending fleet projections are requeued until adoption resolves.
- Selected content, local save namespace and startup options are pre-App inputs.
  Content resolution and compatibility checks retain their existing owners.
- Readback strings, render values, status snapshots and debug flag mirrors are
  published projections. Simulation systems do not read them as authority.
- Two JavaScript callback handles are cloned before invocation, releasing the
  cell borrow before synchronous JavaScript can replace the callback.
- The remaining item, `SLOT_CLAIM_SEQ`, is now the `SlotClaimSequence` Bevy
  Resource. Scheduled claim draining mints in arrival order; successful fleet
  leave resets the counter. It is declared in the fleet lockstep timer state.

Independent read-only review: PASS. It reconciled every original name and
checked queue order, generation filtering, stale completions, save handoff,
leave/reset ordering and callback reentry.

Validation on the working tree over `644da210`:

- `cargo check --target wasm32-unknown-unknown --lib`: exit 0.
- `trunk build` (with `NO_COLOR=true`), then `node scripts/build-client.mjs`:
  exit 0; served WASM bundle `project-phoenix-5f8fb494bdf5d61a`.
- From `tests/smoke`, `PHOENIX_SMOKE_PORT=3157 npx playwright test
  browser-edge.spec.js preinit-content.spec.js`: **13 passed**, exit 0,
  1.7 minutes. Two real scheduled callback replacement cases and eleven
  pre-App local/import/content refusal cases.
- On fresh port 3158, `npx playwright test afk.spec.js --grep reconnect`:
  **1 passed**, exit 0. Same-token reconnect retains the held Station and AFK
  state through the ordinary transport.

- `cargo test --lib --features headless -- server::bridge::tests
  lockstep::tests::slot_claim_sequence`: **31 passed, 0 failed, 0 ignored**,
  exit 0. Queue bounds/FIFO, duplicate toggles, namespace privacy, generation
  projections, import/restore gates and Resource reset are included.
- `cargo test --features headless --test snapshot_resume --
  the_bounded_duel_resumes_across_several_seeds
  the_bounded_duel_resumes_on_the_seed_that_still_diverges
  default_pool_preserves_snapshot_resume_guards`: **3 passed, 0 failed,
  0 ignored**, exit 0, 8.78 seconds. Compilation began before the separate
  Windows preference edits; the tested bridge/lockstep sources match this change.

Final integration gates and push are separate.
