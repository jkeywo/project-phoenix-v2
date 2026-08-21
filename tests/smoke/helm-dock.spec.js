// Issue #1159 — Smoke test: the contextual helm dock control.
//
// This is the browser-level smoke the AC asks for. It runs under the same
// Playwright harness as the other `tests/smoke/*.spec.js` (WASM host + real
// PeerJS-shimmed clients), so it is GATE/CI work, not part of the cheap
// `npx vitest run` unit pass — building the WASM host and launching browsers is
// expensive and is not run inline during development.
//
// WHAT IT COVERS NOW
//
//   The dock control is CONTEXTUAL: it appears on the helm console only while a
//   valid dock target is in range, and is ABSENT otherwise. The shipped hulls
//   carry no `dock` system yet (destroyer docking is deferred to #1164 S11a), so
//   the observable browser invariant today is the ABSENT half: a Helm player on a
//   shipped scenario is never sent a `dock` blackboard, so the console renders no
//   dock control and nothing about the helm console breaks for a hull without a
//   dock.
//
// WHAT COVERS THE "APPEARS / DISAPPEARS WITH RANGE" HALF UNTIL #1164
//
//   * The headless probe `two_hulls_reach_a_mated_dock_...` in
//     `tests/headless_runner.rs` proves the SERVER flips `available_target` as the
//     berth enters and leaves the authored range.
//   * The `helm dock control (issue #1159)` vitest block in
//     `tests/client/console-state.test.js` proves the CLIENT's
//     `buildHelmConsoleState(...).dock` view appears when `available` and becomes
//     the undock control when `docked`.
//
//   When a dock-capable client hull ships (#1164), extend this spec to drive a
//   dockable world and assert the `#dock-panel` element toggles as the ship
//   closes on and backs off a berth.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
} from './fixtures';

async function waitForStation(client, timeout = 5_000) {
  await client.page.waitForFunction(
    (t) =>
      window.__messages?.some(
        (m) => m.type === 'StationAssigned' && m.data.token === t,
      ),
    client.token,
    { timeout },
  );
}

test('helm receives no dock blackboard on a hull with no dock system', async ({
  context,
}) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  await helm.send('SelectStation', { station: 'Helm' });
  await waitForStation(helm);
  await helm.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 10_000);

  // Let a few ticks of blackboards flow, then assert none carried a `dock`
  // entry — a hull with no dock system publishes no dock blackboard, so the
  // contextual control is absent.
  await helm.page.waitForTimeout(1_000);
  const sawDock = await helm.page.evaluate(() =>
    (window.__messages || []).some(
      (m) =>
        m.type === 'BlackboardUpdate' &&
        (m.data.updates || []).some(([systemId]) => systemId === 'dock'),
    ),
  );
  expect(sawDock).toBe(false);

  await helm.close();
});
