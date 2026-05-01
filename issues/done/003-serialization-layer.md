---
type: AFK
priority: infrastructure
---

# 003 — Serialization Abstraction Layer

Wrap serde_json behind a trait so the wire format can be swapped later.

## What to build

`src/codec.rs`:

- `MessageCodec` trait with `encode` and `decode` methods for `ClientMessage` and `ServerMessage`
- `JsonCodec` struct implementing `MessageCodec` via `serde_json`
- The rest of the codebase calls `JsonCodec` methods — never `serde_json` directly

## Tests

Round-trip tests in `#[cfg(test)]` — one per `ClientMessage` variant, one per `ServerMessage` variant: serialize → deserialize → assert equality.

## Acceptance Criteria

- [ ] `cargo test codec` passes
- [ ] All message variants covered by round-trip tests
