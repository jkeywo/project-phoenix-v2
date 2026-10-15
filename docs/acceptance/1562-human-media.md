# Increment 16 selected human media evidence

This is bounded leaf evidence, not the full increment-16 playable exit. Root
owns integrated Rust validation, central wire/Session/mesh joins and final gates.
No media device or two-host acceptance has been claimed by this lane.

## Adopted contract and reviewed leaves

The contract is recorded in `pasm/spec/design/comms-media.yaml` and
`docs/gameplay-redesign/implementation/increment-16-media-contract.md`:
ordinary Contact request, Human acceptance and actual returned receipt; canonical
selected call; ordinary fixed paid carrier; ephemeral current-operator device
lease; existing Phoenix Snapshot delivery only with no Reliable fallback.

- `5e51fb79`, `4d784454`, `d0b510d6`: adopted contract, including predecode
  320×180 JPEG allocation bound. Independent source review passed.
- `2c77ea68`: generic Snapshot-only helper on actual direct/fallback adapters;
  actual socket readiness, full relay envelope byte measurement and backlog
  refusal. Independent source review passed after correcting stale-socket
  success reporting and envelope accounting.
- `62f152d9`: closed Model packets/modes, optional actual equipment capability,
  ordinary MediaCall/Accept/AI-decline path, canonical selected intent, derived
  current paid routes and ephemeral lease owner. Independent source review
  passed; Rust compilation and native tests remain root-owned and pending.

The actual owner test `actual_paid_media_requires_returned_accept_both_selected_intents_and_current_carriers`
uses a real paid introduction and Contact request. Recipient acceptance does not
give the source knowledge until the reverse Outcome actually arrives. Source-only
selection leaves the carrier closed; both selected intents permit the current
paid route, and lost actual receiver energy removes it. Its physical fixture
move is a narrow path-boundary probe, not evidence of a player-flown journey.

## Browser and Workshop checks

The browser leaf keeps capture at page scope and mounts controls in the actual
Comms thread. Call selection uses ordinary correlated System control; request
buttons use actual published media capability and one selected Contact. Media
frames/status bypass game-state reducers. Link loss, Welcome/WorldSetup, call or
operator-derived eligibility change and delayed grants cannot resume capture.

Independent device/client source review passed after adding canvas clearing:
stop, new call, binding and unbinding remove old remote pixels. Targeted tests
exercise the production microphone block encoder/worklet, permission denial and
late permission cleanup, local visibility stop, bounded 200 ms audio scheduling,
one pending video encode/decode, late video promise cleanup and lease/call checks.
Image decoder tests use controlled headers/bitmap promises; they do not establish
real browser JPEG, camera or microphone acceptance.

The existing Workshop radio transaction now edits the optional media capability.
Its closed validator, preview/Test entry points and undo/redo retain the existing
document/history owner.

Author-run checks on this lane:

- Targeted fourteen JavaScript test files: **314 passed**, including direct and
  actual fallback adapter helper tests, media leaves, Comms actions/rendering,
  ordinary action mapping and Workshop radio transaction/history tests.
- `node scripts/check-layers.mjs`: passed.
- `node scripts/check-strings.mjs --strict`: 6330 strings, zero errors/warnings.
- `node scripts/build-client.mjs`: passed; ordinary pure-JS client bundle includes
  the media modules and sampling worklet. This is not browser/device acceptance.
- Owned Rust files formatted and `git diff --check` passed. No Cargo, final gate
  batch or push was run by this worker.

## Integration and acceptance still required

Central integration must register the closed messages and actual System handler,
publish own equipment capability, capture/digest selected intent, clear derived
carriers and leases on restore while preserving their monotonic high-water, and
reconcile silent route/authority loss even with no incoming frames. Wire ingress
must enforce the complete encoded byte/rate bounds before general staging.

Actual cross-host routing must derive authenticated frozen source/destination
slots and resolve the destination's own current Session. Never route a claimed
foreign token or send media through retained fleet broadcast/continuation.

Required remaining evidence includes actual stale/foreign operator refusal,
ordinary AI decline, no media/lease save or digest state, save/reconnect without
device restart, direct and native/fallback two-human calls, and simultaneous
normal commands and gameplay snapshots under shared Snapshot pressure. The
generic helper alone does not establish gameplay snapshot protection. Native
embedded panes have no browser media backend; insecure-origin capture remains
explicitly unavailable. Actual supported browser/device acceptance is separate.
