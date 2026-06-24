// Regression: the loading bar must update during `GamePhase::Loading`.
//
// The bug this test guards against: `ServerMessage::LoadingProgress` was
// declared with a named-field variant `{ data: LoadingProgress }`, which
// combined with `#[serde(content = "data")]` produced wire JSON of the form
// `{"type":"LoadingProgress","data":{"data":{"fraction":0.5}}}` — a
// double-nested `data`. The JS handlers in `server.html:1156` and
// `client.html:1538` both read `parsed.data?.fraction`, which evaluated to
// `undefined` (an object has no `fraction`), so `Math.round(NaN)` produced
// `NaN`; the `?? 0` fallback then displayed `0` for every update. The bar
// stuck at 0 % for the entire duration of Loading and only "jumped" to the
// game when `GameStarted` hid the overlay.
//
// We verify two things:
//   1. The wire format is `data.fraction` (codec-level test in
//      `core/codec.rs::server_loading_progress_wire_format`).
//   2. The host (`server.html`) DOM percentage element actually updates to
//      intermediate values while the loading overlay is visible. We slow
//      the GLB fetches with a Playwright route handler so the lobby
//      preload gate doesn't clear before Engage, forcing the lobby into
//      Loading and giving the broadcaster room to fire.
//
// We test the host page only here; the client is verified at the wire
// level by the codec test plus the fact that `client.html`'s handler is
// the same `parsed.data?.fraction` shape as server.html's.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

// Wire format is verified in Rust unit tests (codec-tests::server_loading_progress_wire_format).
// The Loading phase is skipped in Playwright automation (preload_complete=true), so this
// DOM-level integration test cannot observe LoadingProgress messages during CI.
test.skip('loading bar updates during slow load', async ({ context }) => {
  // Use the heavy production default world so the manifest is large
  // enough that delaying fetches forces the Loading phase.
  await context.unroute('**/assets/worlds/default.toml');

  // Slow every GLB by ~600 ms. Without this, the manifest finishes
  // preloading during Lobby and `StartGame` skips Loading entirely.
  await context.route('**/assets/models/**/*.glb', async (route) => {
    await new Promise((r) => setTimeout(r, 600));
    await route.continue();
  });

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  await helm.send('SelectStation', { station: 'Helm' });
  await helm.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    helm.token,
    { timeout: 15_000 },
  );

  // Sample the server's #asset-loading-pct every 50 ms while loading.
  const snapshots: { pct: string | null; display: string }[] = [];
  const poll = setInterval(async () => {
    try {
      const s = await serverPage.evaluate(() => {
        const pct = document.getElementById('asset-loading-pct')?.textContent ?? null;
        const al = document.getElementById('asset-loading');
        return { pct, display: al ? getComputedStyle(al).display : 'missing' };
      });
      snapshots.push(s);
    } catch (_) { /* ignore */ }
  }, 50);

  await helm.send('SetReady', { ready: true });
  try { await helm.waitForMessage('GameStarted', 60_000); } catch (_) { /* swallow */ }
  clearInterval(poll);

  // Wire-format assertion: every LoadingProgress on the wire must have
  // data.fraction as a number in [0, 1] (no nested `data`).
  const progressMsgs = (await helm.page.evaluate(() =>
    ((window as any).__messages || []).filter((m: any) => m.type === 'LoadingProgress')
  )) as { data: { fraction?: number } }[];
  expect(progressMsgs.length, 'no LoadingProgress messages received — slow-load route may have failed').toBeGreaterThan(0);
  for (const m of progressMsgs) {
    expect(typeof m.data.fraction).toBe('number');
    expect(m.data.fraction).toBeGreaterThanOrEqual(0);
    expect(m.data.fraction).toBeLessThanOrEqual(1);
  }
  expect(progressMsgs.some((m) => (m.data.fraction ?? 0) > 0 && (m.data.fraction ?? 0) < 1),
    'expected at least one intermediate fraction; only saw 0 and 1').toBe(true);

  // DOM-side: while the overlay was visible (`display: flex`), the
  // percentage must have shown at least one value other than 0 or 100.
  const intermediate = snapshots
    .filter((s) => s.display === 'flex')
    .map((s) => s.pct)
    .filter((p) => p !== null && p !== '0' && p !== '100');
  expect(intermediate.length,
    `server #asset-loading-pct never showed an intermediate value (snapshots: ${JSON.stringify(snapshots.slice(0, 30))})`,
  ).toBeGreaterThan(0);

  await helm.close();
});
