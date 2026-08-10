/**
 * script-wasm.test.js — the browser WASM adapter's marshalling + graceful
 * degradation (#983). The real wasm-bindgen module is faked via the injected
 * loader; the actual WASM path is browser-verified separately.
 */
import { describe, it, expect, vi } from 'vitest';
import { fileURLToPath } from 'node:url';
import { createScriptWasm, DEFAULT_MODULE_URL } from '../script-wasm.js';

describe('createScriptWasm', () => {
  it('marshals the host-fn registry from the module', async () => {
    const fakeModule = {
      wasm_get_script_host_fns: () => [
        { name: 'on_destroyed', receiver: '', category: 'trigger' },
      ],
      wasm_script_diagnostics: () => [],
    };
    const load = vi.fn(async () => fakeModule);
    const adapter = createScriptWasm({ load });
    const fns = await adapter.getHostFns();
    expect(fns).toHaveLength(1);
    expect(fns[0].name).toBe('on_destroyed');
    expect(load).toHaveBeenCalledOnce();
  });

  it('passes source + line offset to the diagnostics export', async () => {
    const seen = [];
    const fakeModule = {
      wasm_get_script_host_fns: () => [],
      wasm_script_diagnostics: (src, offset) => {
        seen.push({ src, offset });
        return [{ message: 'bad', line: 1 + offset, column: 0, severity: 'error' }];
      },
    };
    const adapter = createScriptWasm({ load: async () => fakeModule });
    const diags = await adapter.getDiagnostics('fn a() {', 7);
    expect(seen[0]).toEqual({ src: 'fn a() {', offset: 7 });
    expect(diags[0].line).toBe(8);
  });

  it('caches the module across calls', async () => {
    const load = vi.fn(async () => ({
      wasm_get_script_host_fns: () => [],
      wasm_script_diagnostics: () => [],
    }));
    const adapter = createScriptWasm({ load });
    await adapter.getHostFns();
    await adapter.getDiagnostics('x', 0);
    expect(load).toHaveBeenCalledOnce();
  });

  it('degrades to empty results (no throw) when the module fails to load', async () => {
    const adapter = createScriptWasm({ load: async () => { throw new Error('no wasm here'); } });
    expect(await adapter.getHostFns()).toEqual([]);
    expect(await adapter.getDiagnostics('x', 0)).toEqual([]);
  });

  it('degrades when the module lacks the export', async () => {
    const adapter = createScriptWasm({ load: async () => ({}) });
    expect(await adapter.getHostFns()).toEqual([]);
    expect(await adapter.getDiagnostics('x', 0)).toEqual([]);
  });

  it('exposes the stable dist alias as the default module URL (#995)', () => {
    // The build's post_build hook (scripts/wasm-editor-alias.mjs) writes exactly
    // this file; if the two drift, the editor loads nothing. Pin them together.
    expect(DEFAULT_MODULE_URL).toBe('../dist/phoenix.js');
  });

  it('isAvailable() reports the load outcome (honest deferral, #995)', async () => {
    // Before any load attempt: not yet available.
    const ok = createScriptWasm({
      load: async () => ({
        wasm_get_script_host_fns: () => [],
        wasm_script_diagnostics: () => [],
      }),
    });
    expect(ok.isAvailable()).toBe(false);
    await ok.getDiagnostics('x', 0);
    expect(ok.isAvailable()).toBe(true);

    // A failed load stays unavailable — the real editor path today, where the
    // #995 alias is not served and the dynamic import throws.
    const failed = createScriptWasm({ load: async () => { throw new Error('no wasm'); } });
    await failed.getDiagnostics('x', 0);
    expect(failed.isAvailable()).toBe(false);

    // Loaded but missing the export is unavailable too (can't actually diagnose).
    const noExport = createScriptWasm({ load: async () => ({}) });
    await noExport.getDiagnostics('x', 0);
    expect(noExport.isAvailable()).toBe(false);
  });
});

// Smoke/integration test (#995): exercise the DEFAULT load path — the real
// `defaultLoad` dynamic-import + `await mod.default()` — against a
// realistically-shaped ES module (default `init` gating the two named exports),
// NOT the injected fake used above. This catches a URL/init regression: the
// fixture's exports throw until init has run, so dropping the init call would
// make these assertions fail rather than silently pass.
describe('createScriptWasm — real default-load + init flow (#995 smoke)', () => {
  // Resolve the fixture to an absolute file URL string so the adapter's own
  // `import(url)` (with no injected loader) loads it exactly as it would the
  // built `dist/phoenix.js`.
  const fixtureUrl = new URL('./fixtures/fake-phoenix-glue.mjs', import.meta.url).href;
  // Sanity: the fixture is a real on-disk module, not a virtual mock.
  fileURLToPath(fixtureUrl);

  it('imports the module, runs init once, and returns real host fns', async () => {
    const adapter = createScriptWasm({ moduleUrl: fixtureUrl }); // no injected load
    const fns = await adapter.getHostFns();
    expect(adapter.isAvailable()).toBe(true);
    expect(fns).toHaveLength(1);
    expect(fns[0].name).toBe('on_destroyed');
    expect(fns[0].signature).toBe('on_destroyed(tag)');
  });

  it('surfaces diagnostics on the offset-adjusted line after init', async () => {
    const adapter = createScriptWasm({ moduleUrl: fixtureUrl });
    const diags = await adapter.getDiagnostics('ok\nBAD line', 5);
    expect(adapter.isAvailable()).toBe(true);
    expect(diags).toHaveLength(1);
    // 'BAD' is on source line 2; with lineOffset 5 it reports document line 7.
    expect(diags[0].line).toBe(7);
    expect(diags[0].severity).toBe('error');
  });
});
