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

test('Engineering hull detail starts at 189 in first SystemHullUpdate', async ({ context }) => {
  const { captain, engineer } = await startGameWithEngineering(context);

  const msg = await engineer.waitForMessage('SystemHullUpdate', 2_000) as any;
  const hull = msg.data.entries;
  const total = hull.reduce((sum: number, e: any) => sum + e.current, 0);

  // Ship-wide arithmetic history (alliance_cruiser, the `default.toml` player
  // ship). This used to be the asserted figure, back when SystemHullUpdate went
  // to Target::All and every station received the whole ship:
  // 150 (post-#511) + 86 (fine Tactical banks/tubes/magazine added in #512,
  // alongside the retained coarse "Tactical" entry) - 25 (Shields hull moved
  // out of console_hull into per-arc ShipArcHull in #514)
  // + 20 (alliance_battleship/destroyer hull added in #639/#640/#641) = 231.
  // - 25 (coarse Helm/Tactical/Torpedo-Magazine entries replaced by the
  // smaller helm-radar/tactical-radar/sensor-radar entries) = 206.
  // + 10 (Lateral Thrusters hull system added to cruiser/destroyer) = 216.
  //
  // #737 made SystemHullUpdate a per-recipient projection: the ship-wide 216 is
  // now host-internal, and each token receives only the rows its role entitles
  // it to (src/console/repair/visibility.rs). Engineering's entitlement is the
  // ownerless "Core" bucket plus the systems its own station owns, plus any
  // system a repair team is on site at — none at game start. At the time #737
  // shipped this was:
  //   Core (no [[system]] declares them): science 20 + core 20 = 40
  //   engineering-owned: power-reactor 15 + power-battery 10 = 25
  // for a total of 65.
  //
  // The balance pass in c1af00c0 ("one durability ladder for the fleet") and
  // f94c356e ("radar systems carry no hull, and the Lancer goes back up")
  // re-pegged alliance_cruiser.toml's whole durability ladder (cruiser pool
  // 216 -> 500, see the `[[hull.system_hull]]` header comment there), moving
  // Engineering's authored entitlement to:
  //   Core: science 58 + core 58 = 116
  //   engineering-owned: power-reactor 44 + power-battery 29 = 73
  // 189 for this viewer. The ship-wide figure survives as `aggregate_fraction`,
  // which is computed over the whole (now larger) pool and so is still 1.0
  // here.
  expect(total).toBe(189);
  expect(msg.data.aggregate_fraction).toBe(1);

  await captain.close();
  await engineer.close();
});