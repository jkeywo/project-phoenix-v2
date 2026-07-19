// `?log=` / `?log_entity=` plumbing: URL param → JS → wasm_bindgen export →
// thread-local → `wasm_init` → `LogFilterConfig` resource.
//
// The same `parse_log_spec` backs the headless runner's `--log`, so these cover
// the browser half of one shared parser.

import { test, expect, captureServerPageErrors, waitForWasmReady } from './fixtures';

test('the log exports are wired to window', async ({ context }) => {
  const serverPage = await context.newPage();
  const errors = captureServerPageErrors(serverPage);

  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const wired = await serverPage.evaluate(() => ({
    spec: typeof (window as any).wasm_set_log_spec,
    entity: typeof (window as any).wasm_set_log_entity,
  }));
  expect(wired).toEqual({ spec: 'function', entity: 'function' });
  expect(errors).toHaveLength(0);
});

test('?log= raises the level for a category', async ({ context }) => {
  const serverPage = await context.newPage();
  const errors = captureServerPageErrors(serverPage);
  const logs: string[] = [];
  serverPage.on('console', (m) => logs.push(m.text()));

  await serverPage.goto('/?log=config=debug&scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  // `config` events are emitted at debug during world/template setup. The
  // default level is warn, so their presence proves the spec was applied.
  expect(logs.join('\n')).toContain('config');
  expect(errors).toHaveLength(0);
});

// A typo in a debug URL param must not stop the game booting — `wasm_init`
// warns and falls back to the default config.
test('a malformed ?log= does not break startup', async ({ context }) => {
  const serverPage = await context.newPage();
  const errors = captureServerPageErrors(serverPage);

  await serverPage.goto('/?log=warpcore=debug&scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const alive = await serverPage.evaluate(
    () => typeof (window as any).wasm_set_log_spec === 'function'
  );
  expect(alive).toBe(true);
  expect(errors).toHaveLength(0);
});
