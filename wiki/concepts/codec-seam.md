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
