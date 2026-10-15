# Native fleet media bridge evidence

The native retained fleet page forwards selected-call media over the existing
relay socket and host-lobby bridge. Its ordinary sends retain their existing
path. The page exposes unknown network backlog and requires a genuine native
socket capability, generation and checked sequence floor for optional sends.

An optional send admits one bounded local packet until an attempt receipt or a
250 ms local monotonic expiry. Neither admission nor receipt promises remote
delivery. Expiry permits a new sequence and never retries the expired packet.
The complete wire frame is capped at 32 KiB, the serialized bridge record at
64 KiB, and each best-effort bridge tail at four records and 128 KiB. Tail
overflow drops the incoming record without replacing existing work. Ordinary
bridge records, projections and socket sends run before optional work.

The same actual host-media decoder classifies incoming relay payloads before
native socket staging and before lobby-bridge staging, independently of the
caller-controlled drop-only flag. Invalid, oversized or Reliable-class media is
refused. Valid media reaches the existing authenticated host-slot callback and
MediaMesh instead of the lockstep inbox or continuation journal.

## Executed checks

Base: `283a78a4`; the contract was recorded first in `4db42bde`.

- `npx vitest run tests/client/native-fleet-peer.test.js tests/client/native-fleet-snapshot.test.js`: 23 tests passed, 2 files, 350 ms on the final functional slice.
- `node scripts/check-layers.mjs`: passed.
- Owned Rust sources formatted with `rustfmt`; `git diff --check`: passed.

The JavaScript checks cover absent/old capability, invalid or stale generation,
sequence floor after page recreation, expiry after a downstream drop, late
receipts, full/failing staging, complete UTF-8 envelope bounds, byte/count caps,
and ordinary-record precedence. Existing native fleet tests remain included.

## Pending integration evidence

Native source tests cover actual optional socket refusal, ordinary-send
precedence, replay, wrong generation, malformed/oversized/wrong-class media,
physical-reader classifier installation, sequence exhaustion and non-evicting
bridge staging. These tests were authored without running Cargo; the designated
integrator owns native compilation and execution.

Required interfaces come from the generic transport donor `e889e9ad` and the
corresponding public MediaHostFrame/MediaMesh runtime donor. In particular,
ordinary Snapshot sends retain their compatibility behavior when the bridge's
backlog is unknown; strict optional sends use the explicit bounded hook.

No physical-device, live native relay, multi-host or combined gameplay workload
acceptance is claimed here. The integration gates and full increment-16 exit
remain separate.
