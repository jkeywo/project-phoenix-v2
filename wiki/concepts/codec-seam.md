---
title: Codec Seam
type: concept
tags: [codec, serialization, serde, abstraction, wire]
sources: [src/core/codec.rs, src/core/codec_tests.rs, src/core/codec_wire_tests.rs, src/core/codec_wire_cases.json, src/server/bridge.rs, AGENTS.md]
updated: 2026-10-02
---

# Codec Seam

`src/core/codec.rs` is the only production module allowed to import `serde_json`. Every other Rust module trades in typed `ClientMessage` and `ServerMessage` values.

`JsonCodec` exposes the four concrete client/server encode/decode methods. There is no codec trait or alternate production implementation. `src/server/bridge.rs` calls it at the JavaScript/WASM boundary for inbound and outbound frames. Centralising the format keeps protocol changes and exact wire-shape pins in one place. Codec tests feed both compact and pretty JSON into the same production decoder.

`decode_bridge_client_messages` partitions a batch into valid messages and decode errors, so one malformed client frame does not discard unrelated valid input. The bridge logs each rejection and continues.

The table-driven tests in `src/core/codec_tests.rs` cover every message discriminant and preserve explicit JSON pins for compatibility-sensitive shapes. A new wire variant must add its sample/round-trip coverage in the same change.

Mesh frames and flat GM requests use the private `wire` module inside `codec.rs`. It projects only the existing wire fields and converts through the already typed frame/action representations. The mesh adapter preserves hexadecimal digests and transfer identities, flat command orders, GM attribution, legacy refusal decoding, and the existing envelope bytes. The GM schema declares exact required keys, including nullable keys, before applying the existing bounds and family-specific validation.

Both paths parse text into a JSON Value first, retaining last-key-wins duplicate handling. Extra mesh fields remain ignored; extra flat GM request fields are refused. The envelope tick remains advisory. Compatibility fixtures cover accepted values, rejected shapes, and exact mesh encodings.

## Related

- [Message Flow](./message-flow.md)
- [Networking](./networking.md)
