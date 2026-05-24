// Issue #54 — Smoke test: view screen loads and WASM initialises.

import { test, expect, waitForWasmReady } from './fixtures';

test('server page: WASM initialises without JS errors', async ({ context }) => {
  const errors: string[] = [];
  const serverPage = await context.newPage();

  serverPage.on('pageerror', (err) => errors.push(err.message));

  await serverPage.goto('/?scenario=assets/worlds/default.toml');

  await waitForWasmReady(serverPage);

  expect(errors).toHaveLength(0);
});
