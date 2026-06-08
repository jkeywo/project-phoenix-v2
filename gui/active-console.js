// Single-source helper for the "active console" tab selection.
//
// The inline `setActiveConsole(name)` function in `client.html` forwards
// its argument to `wasm_client_set_active_console(...)`. The Rust side
// (`src/client/bridge.rs:160`) treats `""` as the "no override, follow
// the player's primary console" sentinel — i.e. `null`/`undefined` on
// the JS side must become `""` over the bridge, not the string
// `"null"`.
//
// This module exports a pure helper so the contract is locked by a
// Vitest test instead of by inspection of the inline `<script>`.

// Pure: given the current active console and a new name (string, null,
// undefined, or empty string), returns what the inline `setActiveConsole`
// should do next.
//
// Returns:
//   { changed: bool, next: string|null, wasmArg: string }
//
// Where:
//   - `next` is the value to store in `activeConsole` (null when no
//     console is active).
//   - `changed` is true when `next !== current` — callers use this to
//     skip redundant WASM round-trips on no-change rerenders.
//   - `wasmArg` is what to pass to `wasm_client_set_active_console`. It
//     is `""` whenever `next` is `null`, matching the auto sentinel at
//     `src/client/bridge.rs:160`.
export function nextActiveConsole(current, name) {
  const next = name || null;
  const cur = current || null;
  return {
    changed: next !== cur,
    next,
    wasmArg: next || '',
  };
}

// Expose for non-module scripts in `client.html`.
if (typeof window !== 'undefined') {
  window.nextActiveConsole = nextActiveConsole;
}
