// Issue #169 — Smoke test: debug_regions URL parameter enables wireframe overlay
//   - Without ?debug_regions=1: resource is false, no behavioural change
//   - With ?debug_regions=1: resource is true

import { test, expect, captureServerPageErrors, waitForWasmReady } from './fixtures';

test('without URL param: debug regions disabled', async ({ context }) => {
  const serverPage = await context.newPage();
  const errors = captureServerPageErrors(serverPage);

  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  await serverPage.waitForFunction(() => {
    try {
      return typeof JSON.parse(window.wasm_get_debug_flags()).Regions === 'boolean';
    } catch (_) {
      return false;
    }
  });
  const enabled = await serverPage.evaluate(
    () => JSON.parse(window.wasm_get_debug_flags()).Regions,
  );
  expect(enabled).toBe(false);
  expect(errors).toHaveLength(0);
});

test('with ?debug_regions=1: debug regions enabled', async ({ context }) => {
  const serverPage = await context.newPage();
  const errors = captureServerPageErrors(serverPage);

  await serverPage.goto('/?debug_regions=1&scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  await serverPage.waitForFunction(() => {
    try {
      return JSON.parse(window.wasm_get_debug_flags()).Regions === true;
    } catch (_) {
      return false;
    }
  });
  const enabled = await serverPage.evaluate(
    () => JSON.parse(window.wasm_get_debug_flags()).Regions,
  );
  expect(enabled).toBe(true);
  expect(errors).toHaveLength(0);
});
