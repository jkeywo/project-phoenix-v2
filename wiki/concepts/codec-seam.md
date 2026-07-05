---
title: Codec Seam
type: concept
tags: [codec, serialization, serde, abstraction]
sources: [src/shared/codec.rs, AGENTS.md]
updated: 2026-05-08
---

# Codec Seam

> `serde_json` may **never** be called directly outside `src/shared/codec.rs`.

This is one of the project's load-bearing rules.

## What's in `codec.rs`

- The `MessageCodec` trait — `encode<T>` / `decode<T>` over byte buffers.
- The single production implementation: `JsonCodec` wrapping `serde_json`.
- Round-trip tests for **every** `ClientMessage` and `ServerMessage` variant.

## Why

The project may need a binary wire format later (MessagePack via `rmp-serde` is the obvious choice — smaller payloads, faster decode). Keeping the only `serde_json::*` call sites inside one module means that swap is a one-file change. Every other module touches `MessageCodec`, not JSON.

## Where the seam is enforced

- `src/server/bridge.rs` — calls `JsonCodec::decode` on inbound JSON, `JsonCodec::encode` on outbound `ServerMessage`.
- `src/client/client_bridge.rs` — same pattern, opposite direction.

That's it. Every other module trades in typed Rust values.

## Decode resilience (#602)

`decode_bridge_client_messages` (added in `codec.rs`) partitions a batch of decode results into successes and `DecodeError` entries instead of failing the whole batch on one bad message. Each failure is logged at `warn!` with the raw JSON snippet. `drain_inbound` in `src/server/bridge.rs` calls this helper rather than decoding individually.

## Table-driven round-trip harness (#610)

The former ~164 hand-written per-variant serde round-trip tests were replaced with a table-driven harness using `strum::EnumDiscriminants + EnumIter`. A test iterates every discriminant of `ClientMessage` and `ServerMessage`, finds its sample data row in a table, and runs the encode-decode-assert cycle. Exhaustiveness is enforced: a new variant without a table entry fails the test run, naming the specific missing variant. Wire-format string pins (exact JSON assertions) are preserved as standalone tests. The change reduced `codec.rs` from 3289 to 1826 lines.

Four version-skew tests pin current decode behaviour: unknown field on a known variant (silently ignored) and unknown type tag (decode error) — all with doc comments documenting the pinned behaviour.

## Round-trip test pattern

Every variant gets a test like:

```rust
#[test]
fn helm_input_round_trips() {
    let original = ClientMessage::HelmInput { thrust: 0.75, steering: -0.5 };
    let bytes = JsonCodec::encode(&original).unwrap();
    let decoded: ClientMessage = JsonCodec::decode(&bytes).unwrap();
    assert_eq!(original, decoded);
}
```

When you add a new message variant, the codec test is the **first** thing that should exist. See the workflow in [AGENTS.md](../sources/repo-agents.md).

## Related

- [Message Flow](./message-flow.md) — where the seam is crossed
- [Architecture](./architecture.md)
