// A realistically-shaped stand-in for the phoenix wasm-bindgen glue re-exported
// through `dist/phoenix.js` (issue #995). Used by script-wasm smoke tests to
// exercise the REAL `defaultLoad` path (dynamic import + `await mod.default()`),
// not just an injected fake. The named exports THROW until `init` has run — the
// same ordering wasm-bindgen enforces (calling an export before the module is
// initialised traps) — so a regression that drops the init call is caught: the
// exports would throw, the adapter would degrade to [] and isAvailable()===false,
// and the smoke test would fail.

let ready = false;

// Mirrors wasm-bindgen's `__wbg_init as default`: async, returns once the wasm
// is "instantiated". Accepts the `{ module_or_path }` object the stable alias
// passes (ignored here — there is no real wasm to fetch).
export default async function init(_opts) {
  ready = true;
  return {};
}

export function initSync(_opts) {
  ready = true;
  return {};
}

export function wasm_get_script_host_fns() {
  if (!ready) throw new Error('wasm_get_script_host_fns before init');
  return [
    {
      name: 'on_destroyed',
      receiver: '',
      category: 'trigger',
      summary: 'Fires when an entity is destroyed.',
      signature: 'on_destroyed(tag)',
      params: ['tag'],
    },
  ];
}

export function wasm_script_diagnostics(source, lineOffset) {
  if (!ready) throw new Error('wasm_script_diagnostics before init');
  // A trivially "broken" source: any line containing BAD reports an error on
  // that line, shifted by lineOffset (mirrors the real bridge's line math).
  const diags = [];
  String(source)
    .split('\n')
    .forEach((text, i) => {
      if (text.includes('BAD')) {
        diags.push({
          message: 'unexpected token',
          line: i + 1 + lineOffset,
          column: 0,
          severity: 'error',
        });
      }
    });
  return diags;
}
