---
title: Codec Seam
type: concept
tags: [codec, serialization, serde, abstraction, wire]
sources: [src/core/codec.rs, src/core/codec_tests.rs, src/server/bridge.rs, AGENTS.md]
updated: 2026-09-08
---

# Codec Seam

`src/core/codec.rs` is the only production module allowed to import `serde_json`. Every other Rust module trades in typed `ClientMessage` and `ServerMessage` values.

`JsonCodec` exposes the four concrete client/server encode/decode methods. There is no codec trait or alternate production implementation. `src/server/bridge.rs` calls it at the JavaScript/WASM boundary for inbound and outbound frames. Centralising the format keeps protocol changes and exact wire-shape pins in one place. Codec tests feed both compact and pretty JSON into the same production decoder.

`decode_bridge_client_messages` partitions a batch into valid messages and decode errors, so one malformed client frame does not discard unrelated valid input. The bridge logs each rejection and continues.

The table-driven tests in `src/core/codec_tests.rs` cover every message discriminant and preserve explicit JSON pins for compatibility-sensitive shapes. A new wire variant must add its sample/round-trip coverage in the same change.

## Related

- [Message Flow](./message-flow.md)
- [Networking](./networking.md)
