/**
 * script-wasm.js
 *
 * Browser adapter over the phoenix WASM script exports (issue #983, Rhai M5):
 *
 *   - `wasm_get_script_host_fns()`   → the host-fn signature registry
 *   - `wasm_script_diagnostics(src, offset)` → compile diagnostics
 *
 * The editor is otherwise pure JS and loads no WASM; this is the one seam that
 * lazily instantiates the phoenix wasm-bindgen module (the same artifact
 * `trunk build` emits for `server.html`) and calls those two exports. It is a
 * LOCAL module import — not a CDN dependency — so it respects the editor's
 * no-external-resource constraint.
 *
 * Both calls degrade gracefully to an empty result (and a console warning) when
 * the module cannot be loaded, so the editor still renders. The degradation is
 * NOT silent, though: `isAvailable()` reports whether the last load actually
 * produced a usable module, so the view can say "live checks unavailable"
 * instead of misreporting an empty diagnostics result as "No problems". The
 * module loader is injected so the marshalling can be unit-tested without a real
 * WASM build; the default performs a dynamic import.
 */

/**
 * The wasm-bindgen module URL the loader tries by default.
 *
 * `trunk build` emits a content-hashed glue (`project-phoenix-<hash>.js` +
 * `_bg.wasm`) whose name changes every build. The `scripts/wasm-editor-alias.mjs`
 * post_build hook (see `Trunk.toml`, issue #995) writes a stable `dist/phoenix.js`
 * next to it that re-exports the hashed glue's named exports AND a default
 * `init` bound to the hashed `_bg.wasm` path — so this URL resolves in the
 * running editor (served from repo root, `../dist/phoenix.js`) and survives
 * content-hash churn. `defaultLoad` below imports it and runs that `init` once
 * before the first export call.
 *
 * When `dist/` has NOT been built yet, the dynamic import fails and the adapter
 * degrades closed: host-fn autocomplete and live diagnostics are unavailable,
 * `isAvailable()` returns `false`, and the view says so rather than pretending
 * the script is clean. Overridable via `createScriptWasm({ moduleUrl })` (tests
 * inject a fake loader instead).
 */
export const DEFAULT_MODULE_URL = '../dist/phoenix.js';

/**
 * Create a script-WASM adapter.
 *
 * @param {object} [opts]
 * @param {string} [opts.moduleUrl]  Override the wasm-bindgen module URL.
 * @param {(url: string) => Promise<object>} [opts.load]
 *        Injected module loader (tests supply a fake); defaults to a dynamic
 *        import that also runs the wasm-bindgen `default()` initializer.
 * @returns {{ getHostFns: () => Promise<Array>, getDiagnostics: (source: string, lineOffset?: number) => Promise<Array>, isAvailable: () => boolean }}
 */
export function createScriptWasm({ moduleUrl = DEFAULT_MODULE_URL, load } = {}) {
  const loader = load || defaultLoad;
  let modulePromise = null;
  // Records the outcome of the most recent module load so `isAvailable()` can be
  // read synchronously after a call resolves: 'pending' before any attempt,
  // 'ready' when a module exposing the diagnostics export loaded, 'unavailable'
  // when the load failed or the module lacked the export (the #995 fail-closed
  // path). Never a silent lie — the view surfaces 'unavailable' to the user.
  let moduleStatus = 'pending';

  function getModule() {
    if (!modulePromise) {
      modulePromise = Promise.resolve()
        .then(() => loader(moduleUrl))
        .then((mod) => {
          // A module counts as available only if it can actually diagnose — a
          // loaded-but-export-less module is as unavailable as a failed load.
          moduleStatus =
            mod && typeof mod.wasm_script_diagnostics === 'function'
              ? 'ready'
              : 'unavailable';
          return mod;
        })
        .catch((err) => {
          console.warn('[script-wasm] module load failed:', err?.message || err);
          moduleStatus = 'unavailable';
          modulePromise = null; // allow a later retry
          return null;
        });
    }
    return modulePromise;
  }

  return {
    /**
     * Whether the last module load produced a usable (diagnostics-capable)
     * module. Reflects the most recent `getModule()` attempt, so read it after a
     * `getHostFns()`/`getDiagnostics()` call has resolved; `false` before any
     * attempt and whenever the load failed closed (see #995 on `DEFAULT_MODULE_URL`).
     */
    isAvailable() {
      return moduleStatus === 'ready';
    },

    async getHostFns() {
      const mod = await getModule();
      if (!mod || typeof mod.wasm_get_script_host_fns !== 'function') return [];
      try {
        return Array.from(mod.wasm_get_script_host_fns());
      } catch (err) {
        console.warn('[script-wasm] getHostFns failed:', err?.message || err);
        return [];
      }
    },

    async getDiagnostics(source, lineOffset = 0) {
      const mod = await getModule();
      if (!mod || typeof mod.wasm_script_diagnostics !== 'function') return [];
      try {
        return Array.from(mod.wasm_script_diagnostics(String(source ?? ''), lineOffset >>> 0));
      } catch (err) {
        console.warn('[script-wasm] getDiagnostics failed:', err?.message || err);
        return [];
      }
    },
  };
}

async function defaultLoad(url) {
  // Dynamic import of the local wasm-bindgen glue (the stable `dist/phoenix.js`
  // alias), then run its default `init` — which fetches `_bg.wasm` and must
  // complete before the two exports are callable. `getModule()` above memoises
  // this promise, so `init` runs exactly once per adapter regardless of how many
  // getHostFns()/getDiagnostics() calls follow.
  const mod = await import(/* @vite-ignore */ url);
  if (typeof mod.default === 'function') {
    await mod.default();
  }
  return mod;
}
