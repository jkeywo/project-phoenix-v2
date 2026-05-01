---
type: AFK
priority: infrastructure
---

# 001 — Cargo Project Setup

Set up the Rust crate so the rest of the modules have somewhere to live.

## What to build

- `Cargo.toml` — single lib crate (`cdylib + rlib`), with Bevy, wasm-bindgen, serde/serde_json, uuid, web-sys as dependencies
- `src/lib.rs` — minimal module declarations; must pass `cargo check`
- `server.html` — stub host/view-screen page that loads the WASM bundle
- `client.html` — stub phone/client page (pure HTML for now)
- `package.json` — scripts: `"test": "cargo test"`, `"typecheck": "cargo check"`

## Acceptance Criteria

- [ ] `cargo check` passes (native target)
- [ ] `cargo test` exits 0 (no tests yet is fine)
- [ ] `server.html` and `client.html` exist as valid HTML stubs
