# ADR-0001: HTML Bridge Transport Contract & Action Schema

**Status:** Accepted  
**Date:** 2026-06-07  
**Issue:** [#421](https://github.com/jkeywo/project-phoenix-v2/issues/421)  
**Parent PRD:** [#419 — HTML-Based Console UI](https://github.com/jkeywo/project-phoenix-v2/issues/419)

---

## Context

Console UIs are being migrated from Bevy widget trees to HTML/CSS panels served across three targets: browser WASM server, browser WASM client, and native wry server. All three targets must share a single HTML file per console. This ADR locks the bridge contract that every HTML console file and downstream Rust implementation must implement. No contract detail may be changed without a superseding ADR.

---

## Decisions

### 1. `window.__sendAction` — Outbound action schema

Every action sent from an HTML console to Rust uses this envelope:

```json
{ "action": "<snake_case_verb>", "console": "<ConsoleName>", ...payload }
```

- `action` — snake_case discriminant identifying the operation.
- `console` — PascalCase console name matching the `Console` enum variant (e.g. `"Helm"`, `"Tactical"`). Used by Rust to route the action without inspecting the payload.
- Remaining fields are action-specific payload, also snake_case.

#### Helm actions

| `action` | Extra fields | Notes |
|---|---|---|
| `"helm_input"` | `thrust: f32, steering: f32` | Continuous joystick input; −1.0–1.0 |
| `"start_impulse_charge"` | — | Begin charging impulse drive |
| `"cancel_impulse"` | — | Cancel active or charging impulse |
| `"set_target"` | `uuid: string` | Lock a radar blip as the helm target |

#### Tactical (Weapons) actions

| `action` | Extra fields | Notes |
|---|---|---|
| `"fire_phaser"` | `bank: string` | Bank id matches `player_ship.toml` e.g. `"port"` |
| `"set_phaser_mode"` | `mode: "Auto" \| "Manual"` | Phaser firing mode |
| `"fire_torpedo"` | `tube: string, target_uuid: string \| null` | Tube id e.g. `"fore_port"`; null = unguided |
| `"set_target"` | `uuid: string` | Lock a radar blip as the weapons target |

#### All other consoles

TBD — action schema defined when each console's implementation issue is opened.

---

### 2. `window.__updateConsole(name, stateJson)` — Inbound state push

Rust calls this global to push state into a loaded HTML console.

- `name: string` — PascalCase console name (e.g. `"Helm"`, `"Tactical"`).
- `stateJson: string` — JSON-serialised typed Rust struct (serde_json, snake_case fields).
- **No versioning.** HTML files ship with the game build and are always in sync with the Rust structs that produce them. A `console.warn` is sufficient if `__updateConsole` is not defined on the window.
- **Fire-and-forget / latest-wins.** Rust emits on `Changed<T>` only. If JS is busy the call still lands; no queue, no dropped-update tracking. Single-threaded WASM makes this safe.

---

### 3. Transport shim — context detection

Each HTML file includes this inline `<script>` block at the top of `<body>`, before any UI code:

```html
<script>
  /* ── Bridge transport shim (ADR-0001 §3) ─────────────────────── */
  var _bc = ('BroadcastChannel' in window) ? new BroadcastChannel('phoenix-console-state') : null;
  window.__sendAction = function(json) {
    if (window !== window.parent) {
      window.parent.postMessage({ type: 'console_action', payload: json }, '*');
    } else if (window.ipc) {
      window.ipc.postMessage(json);
    } else if (window.wasmBindings && typeof window.wasmBindings.wasm_ui_action === 'function') {
      window.wasmBindings.wasm_ui_action(json);
    } else if (_bc) {
      _bc.postMessage({ type: 'console_action', payload: json });
    }
  };
  if (_bc) {
    _bc.onmessage = function(e) {
      if (e.data && e.data.type === 'console_state' && e.data.name === '<ConsoleName>') {
        if (typeof window.__updateConsole === 'function') window.__updateConsole(e.data.name, e.data.json);
      }
    };
  }
</script>
```

Replace `<ConsoleName>` with the PascalCase Console enum variant for the console (e.g. `'Helm'`, `'Tactical'`, `'Repair'`).

The four detection targets, in priority order:

| Priority | Target | How it works |
|---|---|---|
| 1 | `window !== window.parent` | Console is running inside a `client.html` iframe; forwards via `postMessage` |
| 2 | `window.ipc` | wry native host; injected automatically by the wry webview |
| 3 | `window.wasmBindings.wasm_ui_action` | Browser WASM (server or client); checked with `typeof` guard |
| 4 | BroadcastChannel (`_bc`) | Same-origin separate-tab mode; broadcast to the server.html peer |

The shim also receives inbound state via BroadcastChannel (target 4 receive path), filtering by the console's own name. The `window.__updateConsole` callback is assigned in the bottom `<script>` block and is always ready before any async BroadcastChannel message arrives.

**Preferred implementation:** use `gui/console-core.js` (see below) which encapsulates all four transports, rather than copy-pasting this shim verbatim.

---

### 4. wasm-bindgen export signatures

Both `src/server/bridge.rs` and `src/client/bridge.rs` gain these exports:

```rust
/// Called by the HTML transport shim when the player triggers a console action.
/// Decodes the action JSON and emits the appropriate Bevy event.
#[wasm_bindgen]
pub fn wasm_ui_action(json: &str) { ... }

/// Called by JS once to register the state-push callback.
/// Bevy calls callback(name: string, stateJson: string) when console state changes.
#[wasm_bindgen]
pub fn set_console_state_callback(callback: js_sys::Function) { ... }
```

Callback invocation from Rust uses `call2(&JsValue::NULL, &name, &state_json)`, matching the existing pattern in `flush_outbound` / `set_message_callback`.

---

### 5. Back-pressure policy

**Fire-and-forget.** No queue. No dropped-update counter. Rationale: WASM is single-threaded; Rust emits only on `Changed<T>` which naturally throttles; JS receives the latest snapshot synchronously. For the wry native target, `webview.evaluate_script(...)` is also fire-and-forget. If a stricter policy is needed in future it can be added in a follow-up ADR without changing the wire format.

---

## Consequences

- Every HTML console file **must** include the transport shim verbatim (copy-paste from this ADR).
- Rust console state structs **must** derive `serde::Serialize` and use the default serde snake_case field names.
- Downstream implementation issues must not introduce new `action` discriminants for Helm or Tactical without updating this ADR.
- The `wry` dependency remains gated behind `#[cfg(not(target_arch = "wasm32"))]` (or the planned `native` feature flag from issue #115) so the WASM build is unaffected.
