---
type: AFK
priority: feature-tracer
---

# 008 — WASM/JS Bridge

wasm-bindgen API surface between Bevy and the JavaScript layer.

## What to build

`src/bridge.rs`:

- `wasm_init()` — entry point called by JS on page load; starts Bevy App
- `wasm_receive_message(json: &str)` — JS calls this to push an inbound `ClientMessage` into Bevy event queue
- `set_message_callback(cb: js_sys::Function)` — JS registers a callback; Bevy calls it with outbound JSON strings

All functions `#[wasm_bindgen]` exported.

## Acceptance Criteria

- [ ] `cargo check --target wasm32-unknown-unknown` passes (requires WASM target installed)
- [ ] Native `cargo check` passes
