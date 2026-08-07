// Issue #70 — Smoke test: Per-system hull integrity in SimState.
//
// SimSnapshot no longer carries a flat hull_integrity; it carries
// system_hull: Vec<SystemHullStatus { system_id, display_name, current, max_hp, ... }>
// broadcast via `SystemHullUpdate` (issue #618, renamed from
// `ConsoleHullUpdate` when the publisher stopped emitting legacy
// Console-keyed wire fields).
//
// Since issue #737 the message is a per-recipient projection rather than a
// Target::All broadcast, so what a client receives depends on which station it
// holds — see src/console/repair/visibility.rs.

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';
import type { BrowserContext } from '@playwright/test';

async function waitForStation(client: { page: import('@playwright/test').Page; token: string }, timeout = 5_000) {
  await client.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t
    ),
    client.token,
    { timeout },
  );
}

async function startGameWithEngineering(context: BrowserContext) {
  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  const captain = await createTestClient(context, hostId, { name: 'Cap' });
  const engineer = await createTestClient(context, hostId, { name: 'Eng' });

  // Fixed roster (#495): one console per station, so the Repair/power console
  // lives on `engineering` and nowhere else. This used to claim Tactical and
  // call it "the engineer" — true only while every station received identical
  // whole-ship hull detail. Since #737 the payload is scoped to the viewer's
  // role, so the client under test must actually hold Engineering.
  await captain.send('SelectStation', { station: 'Helm' });
  await waitForStation(captain);

  await engineer.send('SelectStation', { station: 'Engineering' });
  await waitForStation(engineer);

  await captain.send('SetReady', { ready: true });
  await engineer.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 10_000);
  await engineer.waitForMessage('GameStarted', 10_000);

  return { captain, engineer };
}

test('Engineering player receives SystemHullUpdate after game start', async ({ context }) => {
  const { captain, engineer } = await startGameWithEngineering(context);

  const msg = await engineer.waitForMessage('SystemHullUpdate', 2_000) as any;
  const hull = msg.data.entries;

  expect(Array.isArray(hull)).toBe(true);
  expect(hull.length).toBeGreaterThanOrEqual(1);
  for (const entry of hull) {
    expect(typeof entry.system_id).toBe('string');
    expect(typeof entry.display_name).toBe('string');
    expect(typeof entry.current).toBe('number');
    expect(typeof entry.max_hp).toBe('number');
    expect(entry.current).toBeGreaterThanOrEqual(0);
    expect(entry.current).toBeLessThanOrEqual(entry.max_hp);
  }

  await captain.close();
  await engineer.close();
});

// Issue #941 replaced this test's predecessor, which asserted
// `expect(total).toBe(189)` — the sum of the hull HP alliance_cruiser.toml
// happened to author for Engineering's rows on the day it was last edited.
// That number had already been rewritten four times (#511, #512, #514,
// #639-641, then the c1af00c0 / f94c356e balance passes) and every rewrite was
// a false failure: the projection code was fine, a designer had retuned a
// hull. The exact-row-set arithmetic is covered where it belongs — against a
// self-contained ship config, in `src/console/repair/visibility.rs::tests`
// (`live_broadcast_gives_engineering_core_only_with_no_team_on_site`,
// `live_broadcast_gives_a_station_owner_only_its_own_systems`,
// `every_recipient_receives_the_same_ship_wide_aggregate`).
//
// What only a smoke test can show is that the projection survives the real
// wire: two clients on one host, holding different stations, are sent
// *different* row sets over the actual peer connection. Before #737 this was a
// single `Target::All` broadcast, so a regression to that shape — the actual
// information leak the issue exists to prevent — fails here, and does so
// without naming a single authored HP figure.
test('SystemHullUpdate is projected per recipient, not broadcast whole (#737)', async ({ context }) => {
  const { captain, engineer } = await startGameWithEngineering(context);

  // `captain` holds Helm (see startGameWithEngineering), `engineer` holds
  // Engineering. Both own damageable systems on any hull that declares any:
  // Engineering is entitled to the ownerless Core bucket plus its own, a
  // station holder to its own only.
  const engMsg = await engineer.waitForMessage('SystemHullUpdate', 5_000) as any;
  const helmMsg = await captain.waitForMessage('SystemHullUpdate', 5_000) as any;

  const rowIds = (m: any): string[] =>
    (m.data.entries as any[]).map((e) => e.system_id).sort();
  const engIds = rowIds(engMsg);
  const helmIds = rowIds(helmMsg);

  expect(engIds.length, 'Engineering must receive its Core + owned rows').toBeGreaterThan(0);
  expect(helmIds.length, 'a station holder must receive its own rows').toBeGreaterThan(0);

  // The projection claim: no row reaches both recipients. A regression to the
  // pre-#737 `Target::All` broadcast makes these two lists identical.
  const shared = engIds.filter((id) => helmIds.includes(id));
  expect(
    shared,
    `Engineering and Helm were both sent ${JSON.stringify(shared)} — SystemHullUpdate is ` +
      'no longer a per-recipient projection',
  ).toEqual([]);

  // The aggregate is the one whole-ship figure every recipient may have, so it
  // must be identical across two clients that see different rows...
  expect(helmMsg.data.aggregate_fraction).toBe(engMsg.data.aggregate_fraction);
  // ...and an undamaged ship is at full hull whatever the authored ladder says.
  expect(engMsg.data.aggregate_fraction).toBe(1);

  await captain.close();
  await engineer.close();
});