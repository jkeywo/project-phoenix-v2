// Issue #54 — Smoke test: view screen loads and WASM initialises.

import { test, expect, captureServerPageErrors, waitForWasmReady } from './fixtures';

test('server page: WASM initialises without JS errors', async ({ context }) => {
  const serverPage = await context.newPage();
  const errors = captureServerPageErrors(serverPage);

  await serverPage.goto('/?scenario=assets/worlds/default.toml');

  await waitForWasmReady(serverPage);

  expect(errors).toHaveLength(0);
});
