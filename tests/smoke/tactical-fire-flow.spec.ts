// Issue #315 — Smoke test: tactical fire-flow (phaser hits NPC, hull decreases, NPC destroyed).
//
// Uses a self-contained world (MINIMAL_TEST_WORLD) that places a pirate raider
// 20 units directly ahead of the player ship spawn (well within the 100-unit
// phaser range), so the test never needs to move the ship.  The tactical player:
//   1. Locks the raider via SetTarget.
//   2. Fires phasers until BeamStarted is received.
//   3. Asserts the NPC's hull_fraction in entity_states decreases.
//   4. Keeps firing until EntityDespawned arrives (NPC hull reaches 0).

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';
import type { BrowserContext } from '@playwright/test';

// Self-contained smoke-test world — intentionally does NOT read or depend on
// assets/worlds/default.toml so changes to the default world never break this
// test.  The player ship spawns at the origin facing -Z; a single raider is
// placed 15.8 units to the port bow (within the fore bank's 270° arc and
// well within the 100-unit phaser range).  The raider template is intercepted
// inside startGameWithTactical to zero every target_speed and set
// initial_state = "idle" so it stays put.
const MINIMAL_TEST_WORLD = `
[global]
seed = 42

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[anchors]
patrol_alpha = [600.0, 0.0, -600.0]
patrol_beta  = [500.0, 0.0, -300.0]
patrol_gamma = [200.0, 0.0, -600.0]

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
tags = ["ship"]

[[entity]]
template_path = "assets/entities/ship_harrow_patrol.toml"
name          = "raider_alpha"
transform     = { position = [-15.0, 0.0, -5.0] }
spawn_on      = "game_start"
`;

async function waitForStation(
  client: { page: import('@playwright/test').Page; token: string },
  timeout = 5_000,
) {
  await client.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    client.token,
    { timeout },
  );
}

async function startGameWithTactical(context: BrowserContext) {
  // Serve the self-contained minimal world instead of the real default.toml.
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: MINIMAL_TEST_WORLD }),
  );

  // Make the raider stationary: set initial_state = "idle" and zero every
  // target_speed so it never moves regardless of AI transitions.  Also
  // disable the enemy_in_range transition (200-unit radius) so the raider
  // never transitions out of idle and never auto-fires at the player ship.
  // Without this patch the raider would produce a BeamStarted (attacking
  // the player) before the player fires, causing the test's
  // waitForMessage('BeamStarted') to pick up the wrong target_uuid.
  await context.route('**/assets/entities/ship_harrow_patrol.toml', async (route) => {
    const response = await route.fetch();
    const text = await response.text();
    const patched = text
      .replace(/initial_state\s*=\s*"patrol"/, 'initial_state = "idle"')
      .replace(/target_speed\s*=\s*[\d.]+/g, 'target_speed = 0.0')
      .replace(/condition = "enemy_in_range"/, 'condition = "never_matches"');
    await route.fulfill({ contentType: 'text/plain', body: patched });
  });

  // Patch alliance_cruiser.toml: boost phaser DPS so one beam kills the 60 HP
  // raider. Headless Chromium throttles requestAnimationFrame on the
  // backgrounded server page (clients become foreground), so the Bevy sim
  // runs at ~1/15 of wall-clock. The default 5 DPS / 6 s beam / 6 s
  // cooldown would need two beam cycles (~3 min wall-clock), exceeding
  // the 60 s waitForFunction timeout. With boosted DPS one 6 s beam deals
  // 600 HP and destroys the raider before cooldown matters.
  await context.route('**/assets/entities/alliance_cruiser.toml', async (route) => {
    const response = await route.fetch();
    const text = await response.text();
    const patched = text.replace(
      /beam_damage_per_sec\s*=\s*[\d.]+/,
      'beam_damage_per_sec = 100.0',
    );
    await route.fulfill({ contentType: 'text/plain', body: patched });
  });

  const serverPage = await context.newPage();
  // Track only actual server crashes / panics, not console.errors from
  // external network requests (CORS, 404s to ICE servers — known noise on CI).
  const serverCrashes: string[] = [];
  serverPage.on('crash', () => { serverCrashes.push('server page crashed'); });
  serverPage.on('pageerror', (err) => { serverCrashes.push(err.message); });

  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  // 2P layout: Helm station (CaptainChair+Helm) + Tactical station.
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const tactical = await createTestClient(context, hostId, { name: 'Tac' });

  await helm.send('SelectStation', { station: 'Helm' });
  await waitForStation(helm);

  await tactical.send('SelectStation', { station: 'Tactical' });
  await waitForStation(tactical);

  await helm.send('SetReady', { ready: true });
  await tactical.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 10_000);
  await tactical.waitForMessage('GameStarted', 10_000);

  return { helm, tactical, serverPage, serverCrashes };
}

test('tactical fire-flow: BeamStarted received after locking NPC and firing', async ({ context }) => {
  test.setTimeout(45_000);
  const { helm, tactical, serverCrashes } = await startGameWithTactical(context);

  // Get the raider UUID from WorldSetup.
  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetup?.data?.world?.entities ?? [];
  // Select on `npc` alone. MINIMAL_TEST_WORLD spawns exactly two entities —
  // the player's cruiser and one hostile — so `npc` is unambiguous here.
  // This deliberately does NOT also require an `enemy` tag: that tag is purely
  // descriptive (nothing in `src/` or `gui/` reads it; hostility comes from
  // factions), it was carried by the retired pirate_raider hull and not by the
  // Ironveil that replaced it, and requiring it made this test depend on a
  // hull's self-description rather than on what the world actually spawns.
  const raider = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('npc'),
  );
  expect(raider, 'raider entity must appear in WorldSetup').toBeDefined();
  const raiderUuid: string = raider.uuid;

  // Lock the raider as the tactical target. Mirrors the real client
  // (gui/action-map.js `set_target`): SetTarget is a ControlSystem payload
  // addressed to the tactical-radar system (issue #801), not a legacy top-level message.
  await tactical.send('ControlSystem', {
    target: 'tactical-radar',
    payload: { type: 'SetTarget', data: { uuid: raiderUuid } },
  });

  // First verify the server is alive by waiting for SimState.
  await tactical.page.waitForFunction(
    () => (window as any).__messages?.some((m: any) => m.type === 'SimState'),
    undefined,
    { timeout: 15_000 },
  );

  // Wait for WeaponsUpdate confirming a bank is fire_ready (target locked + in range).
  await tactical.page.bringToFront();
  await tactical.page.waitForFunction(
    () => (window as any).__messages?.some(
      (m: any) => m.type === 'WeaponsUpdate'
        && Array.isArray(m.data.banks)
        && m.data.banks.some((b: any) => b.fire_ready === true),
    ),
    undefined,
    { timeout: 15_000 },
  );

  // Fire phasers — fore bank is defined first in alliance_cruiser.toml.
  // FirePhaser routes through the ControlSystem envelope addressed to the
  // `phaser-{bank}` system (issue #846); the legacy top-level message is gone.
  await tactical.send('ControlSystem', {
    target: 'phaser-fore',
    payload: { type: 'FirePhaser' },
  });

  // BeamStarted must be broadcast to all clients.
  const beamStarted = await tactical.waitForMessage('BeamStarted', 15_000) as any;
  expect(beamStarted.data.target_uuid).toBe(raiderUuid);

  if (serverCrashes.length > 0) {
    console.log('Server crashes during BeamStarted test:', serverCrashes);
  }
  expect(serverCrashes).toEqual([]);

  await helm.close();
  await tactical.close();
});

test('tactical fire-flow: NPC hull_fraction decreases after phaser hit', async ({ context }) => {
  test.setTimeout(60_000);
  const { helm, tactical, serverCrashes } = await startGameWithTactical(context);

  // Get raider UUID from WorldSetup.
  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetup?.data?.world?.entities ?? [];
  // Select on `npc` alone. MINIMAL_TEST_WORLD spawns exactly two entities —
  // the player's cruiser and one hostile — so `npc` is unambiguous here.
  // This deliberately does NOT also require an `enemy` tag: that tag is purely
  // descriptive (nothing in `src/` or `gui/` reads it; hostility comes from
  // factions), it was carried by the retired pirate_raider hull and not by the
  // Ironveil that replaced it, and requiring it made this test depend on a
  // hull's self-description rather than on what the world actually spawns.
  const raider = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('npc'),
  );
  expect(raider, 'raider entity must appear in WorldSetup').toBeDefined();
  const raiderUuid: string = raider.uuid;

  // Lock target and wait for fire_ready on any bank. SetTarget routes
  // through the ControlSystem envelope (see gui/action-map.js `set_target`).
  await tactical.send('ControlSystem', {
    target: 'tactical-radar',
    payload: { type: 'SetTarget', data: { uuid: raiderUuid } },
  });
  await tactical.page.bringToFront();
  await tactical.page.waitForFunction(
    () => (window as any).__messages?.some(
      (m: any) => m.type === 'WeaponsUpdate'
        && Array.isArray(m.data.banks)
        && m.data.banks.some((b: any) => b.fire_ready === true),
    ),
    undefined,
    { timeout: 15_000 },
  );

  // Record the initial hull_fraction (1.0 if not yet present) from entity_states.
  const initialHull: number = await tactical.page.evaluate((uuid: string) => {
    const msgs: any[] = (window as any).__messages || [];
    for (let i = msgs.length - 1; i >= 0; i--) {
      const m = msgs[i];
      if (m.type === 'SimState') {
        const es: any[] = m.data.snapshot.entity_states || [];
        const entry = es.find((e: any) => e.uuid === uuid);
        if (entry?.hull_fraction !== undefined && entry.hull_fraction !== null) {
          return entry.hull_fraction as number;
        }
      }
    }
    return 1.0;
  }, raiderUuid);

  // Fire phasers via the ControlSystem envelope (issue #846).
  await tactical.send('ControlSystem', {
    target: 'phaser-fore',
    payload: { type: 'FirePhaser' },
  });
  await tactical.waitForMessage('BeamStarted', 15_000);

  // Wait for a SimState where the raider's hull_fraction is lower than initial.
  await tactical.page.waitForFunction(
    ({ uuid, initial }: { uuid: string; initial: number }) => {
      const msgs: any[] = (window as any).__messages || [];
      return msgs.some((m: any) => {
        if (m.type !== 'SimState') return false;
        const es: any[] = m.data.snapshot.entity_states || [];
        const entry = es.find((e: any) => e.uuid === uuid);
        return entry?.hull_fraction !== undefined &&
               entry.hull_fraction !== null &&
               entry.hull_fraction < initial;
      });
    },
    { uuid: raiderUuid, initial: initialHull },
    { timeout: 30_000 },
  );

  if (serverCrashes.length > 0) {
    console.log('Server crashes during hull_fraction test:', serverCrashes);
  }
  expect(serverCrashes).toEqual([]);

  await helm.close();
  await tactical.close();
});

test('tactical fire-flow: EntityDespawned received when NPC hull reaches 0', async ({ context }) => {
  test.setTimeout(120_000);
  const { helm, tactical, serverCrashes } = await startGameWithTactical(context);

  // Get raider UUID.
  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetup?.data?.world?.entities ?? [];
  // Select on `npc` alone. MINIMAL_TEST_WORLD spawns exactly two entities —
  // the player's cruiser and one hostile — so `npc` is unambiguous here.
  // This deliberately does NOT also require an `enemy` tag: that tag is purely
  // descriptive (nothing in `src/` or `gui/` reads it; hostility comes from
  // factions), it was carried by the retired pirate_raider hull and not by the
  // Ironveil that replaced it, and requiring it made this test depend on a
  // hull's self-description rather than on what the world actually spawns.
  const raider = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('npc'),
  );
  expect(raider, 'raider entity must appear in WorldSetup').toBeDefined();
  const raiderUuid: string = raider.uuid;

  // Lock target. SetTarget routes through the ControlSystem envelope
  // (see gui/action-map.js `set_target`).
  await tactical.send('ControlSystem', {
    target: 'tactical-radar',
    payload: { type: 'SetTarget', data: { uuid: raiderUuid } },
  });
  await tactical.page.bringToFront();
  await tactical.page.waitForFunction(
    () => (window as any).__messages?.some(
      (m: any) => m.type === 'WeaponsUpdate'
        && Array.isArray(m.data.banks)
        && m.data.banks.some((b: any) => b.fire_ready === true),
    ),
    undefined,
    { timeout: 15_000 },
  );

  // Keep firing phasers on cooldown cycles until EntityDespawned arrives.
  // The raider has 60 HP; beam_damage_per_sec=5, beam_duration=6s → ~30 HP per shot.
  // Two shots should destroy it.  We fire on each WeaponsUpdate where any bank is
  // fire_ready, waiting up to 60 s total to account for cooldowns and CI latency.
  await tactical.page.evaluate(() => {
    (window as any).__fireInterval = setInterval(() => {
      const msgs: any[] = (window as any).__messages || [];
      const despawned = msgs.some((m: any) => m.type === 'EntityDespawned');
      if (despawned) { clearInterval((window as any).__fireInterval); return; }
      // Find the most recent WeaponsUpdate and pick the first fire-ready bank.
      let readyBank: string | null = null;
      for (let i = msgs.length - 1; i >= 0; i--) {
        const m = msgs[i];
        if (m.type !== 'WeaponsUpdate') continue;
        const banks: any[] = Array.isArray(m.data.banks) ? m.data.banks : [];
        const found = banks.find((b: any) => b.fire_ready === true);
        if (found) { readyBank = found.id; }
        break;
      }
      if (readyBank) {
        (window as any).__conn.send(JSON.stringify({
          type: 'ControlSystem',
          data: { target: `phaser-${readyBank}`, payload: { type: 'FirePhaser' } },
        }));
      }
    }, 200);
  });

  // Wait for EntityDespawned for the raider.
  await tactical.page.waitForFunction(
    (uuid: string) => (window as any).__messages?.some(
      (m: any) => m.type === 'EntityDespawned' && m.data.uuid === uuid,
    ),
    raiderUuid,
    { timeout: 90_000 },
  );

  await tactical.page.evaluate(() => clearInterval((window as any).__fireInterval));

  // Verify the despawn message is present.
  const despawnMsg = await tactical.page.evaluate((uuid: string) => {
    return (window as any).__messages.find(
      (m: any) => m.type === 'EntityDespawned' && m.data.uuid === uuid,
    );
  }, raiderUuid);
  expect(despawnMsg).toBeDefined();

  if (serverCrashes.length > 0) {
    console.log('Server crashes during EntityDespawned test:', serverCrashes);
  }
  expect(serverCrashes).toEqual([]);

  await helm.close();
  await tactical.close();
});
