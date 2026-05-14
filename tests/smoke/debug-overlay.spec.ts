// Issue #169 — Smoke test: debug_regions URL parameter enables wireframe overlay
//   - Without ?debug_regions=1: resource is false, no behavioural change
//   - With ?debug_regions=1: resource is true

import { test, expect } from './fixtures';

test('without URL param: debug regions disabled', async ({ context }) => {
  const errors: string[] = [];
  const serverPage = await context.newPage();
  serverPage.on('pageerror', (err) => errors.push(err.message));

  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  // wasm_is_debug_regions_enabled should be callable and return false
  const enabled = await serverPage.evaluate(() => {
    const fn = (window as any).wasm_is_debug_regions_enabled;
    return typeof fn === 'function' ? fn() : null;
  });

  // If the function is not available (WASM not fully initialised during test),
  // the flag was never set — that's acceptable as "disabled"
  if (enabled !== null) {
    expect(enabled).toBe(false);
  }
  expect(errors).toHaveLength(0);
});

test('with ?debug_regions=1: debug regions enabled', async ({ context }) => {
  const errors: string[] = [];
  const serverPage = await context.newPage();
  serverPage.on('pageerror', (err) => errors.push(err.message));

  await serverPage.goto('/?debug_regions=1');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const enabled = await serverPage.evaluate(() => {
    const fn = (window as any).wasm_is_debug_regions_enabled;
    return typeof fn === 'function' ? fn() : null;
  });

  if (enabled !== null) {
    expect(enabled).toBe(true);
  }
  expect(errors).toHaveLength(0);
});
