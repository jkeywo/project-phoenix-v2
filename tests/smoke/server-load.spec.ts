// Issue #54 — Smoke test: view screen loads and WASM initialises.

import { test, expect } from './fixtures';

test('server page: WASM initialises without JS errors', async ({ context }) => {
  const errors: string[] = [];
  const serverPage = await context.newPage();

  serverPage.on('pageerror', (err) => errors.push(err.message));

  await serverPage.goto('/');

  // window.__wasmReady is set by the shim after TrunkApplicationStarted fires
  // and startPhoenix() has run.  15 s is generous for cold-load from localhost.
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  expect(errors).toHaveLength(0);
});
