// Issue #315 — Smoke test: tactical fire-flow (phaser hits NPC, hull decreases, NPC destroyed).
//
// Uses a custom scenario that places a pirate raider directly adjacent to the
// player ship spawn (within the 50-unit beam range), so the test never needs
// to move the ship.  The tactical player:
//   1. Locks the raider via SetTarget.
//   2. Fires phasers until BeamStarted is received.
//   3. Asserts the NPC's hull_fraction in entity_states decreases.
//   4. Keeps firing until EntityDespawned arrives (NPC hull reaches 0).

import { test, expect } from './fixtures';
import { readHostPeerId, createTestClient, createServerPage } from './fixtures';
import type { BrowserContext } from '@playwright/test';
import * as fs from 'fs';
import * as path from 'path';

// Load the real default world and append a close-range raider entity so that
// the test's SetTarget call passes the in-range gate. After PRD #337 the
// unified `[[entity]]` block (with optional `name`) is the only spawn surface;
// legacy `[[spawn]]` blocks are no longer parsed. We also strip the default
// `raider_alpha` entity (positioned at the far patrol anchor) so there is
// exactly one raider for the test to target.
const REAL_WORLD = fs.readFileSync(
  path.join(__dirname, '../../assets/worlds/default.toml'),
  'utf-8',
);
const WORLD_WITHOUT_FAR_RAIDER = REAL_WORLD.replace(
  /\[\[entity\]\]\s*\nname\s*=\s*"raider_alpha"[\s\S]*?(?=\n\[\[|$)/,
  '',
);
const CLOSE_RAIDER_WORLD = WORLD_WITHOUT_FAR_RAIDER + `

# Smoke-test override: a raider 20 units in front of the player ship spawn.
# Ship spawns at (150, 0, 0) per assets/worlds/default.toml; forward is -Z.
[[entity]]
template_path = "assets/entities/pirate_raider.toml"
name          = "raider_alpha"
position      = [150.0, 0.0, -20.0]
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
  // Intercept the unified world TOML with our close-raider variant (real
  // world content + appended close-range raider spawn).
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: CLOSE_RAIDER_WORLD }),
  );

  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.waitForFunction(() => !!(window as any).__wasmReady, { timeout: 15_000 });

  const hostId = await readHostPeerId(serverPage);

  // 2P layout: Helm station (CaptainChair+Helm) + Tactical station.
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const tactical = await createTestClient(context, hostId, { name: 'Tac' });

  await helm.send('SelectStation', { station: 'Helm' });
  await waitForStation(helm);

  await tactical.send('SelectStation', { station: 'Tactical' });
  await waitForStation(tactical);

  await helm.send('StartGame');
  await helm.waitForMessage('GameStarted', 5_000);
  await tactical.waitForMessage('GameStarted', 5_000);

  return { helm, tactical };
}

test('tactical fire-flow: BeamStarted received after locking NPC and firing', async ({ context }) => {
  const { helm, tactical } = await startGameWithTactical(context);

  // Get the raider UUID from WorldSetup.
  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetup?.data?.world?.entities ?? [];
  const raider = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('npc') && e.tags.includes('enemy'),
  );
  expect(raider, 'raider entity must appear in WorldSetup').toBeDefined();
  const raiderUuid: string = raider.uuid;

  // Lock the raider as the tactical target.
  await tactical.send('SetTarget', { uuid: raiderUuid });

  // Wait for WeaponsUpdate confirming fire_ready (target locked + in range).
  await tactical.page.waitForFunction(
    () => (window as any).__messages?.some(
      (m: any) => m.type === 'WeaponsUpdate' && m.data.fire_ready === true,
    ),
    { timeout: 5_000 },
  );

  // Fire phasers.
  await tactical.send('FirePhaser');

  // BeamStarted must be broadcast to all clients.
  const beamStarted = await tactical.waitForMessage('BeamStarted', 5_000) as any;
  expect(beamStarted.data.target_uuid).toBe(raiderUuid);

  await helm.close();
  await tactical.close();
});

test('tactical fire-flow: NPC hull_fraction decreases after phaser hit', async ({ context }) => {
  const { helm, tactical } = await startGameWithTactical(context);

  // Get raider UUID from WorldSetup.
  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetup?.data?.world?.entities ?? [];
  const raider = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('npc') && e.tags.includes('enemy'),
  );
  expect(raider, 'raider entity must appear in WorldSetup').toBeDefined();
  const raiderUuid: string = raider.uuid;

  // Lock target and wait for fire_ready.
  await tactical.send('SetTarget', { uuid: raiderUuid });
  await tactical.page.waitForFunction(
    () => (window as any).__messages?.some(
      (m: any) => m.type === 'WeaponsUpdate' && m.data.fire_ready === true,
    ),
    { timeout: 5_000 },
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

  // Fire phasers.
  await tactical.send('FirePhaser');
  await tactical.waitForMessage('BeamStarted', 5_000);

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
    { timeout: 10_000 },
  );

  await helm.close();
  await tactical.close();
});

test('tactical fire-flow: EntityDespawned received when NPC hull reaches 0', async ({ context }) => {
  test.setTimeout(90_000);
  const { helm, tactical } = await startGameWithTactical(context);

  // Get raider UUID.
  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000) as any;
  const entities: any[] = worldSetup?.data?.world?.entities ?? [];
  const raider = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('npc') && e.tags.includes('enemy'),
  );
  expect(raider, 'raider entity must appear in WorldSetup').toBeDefined();
  const raiderUuid: string = raider.uuid;

  // Lock target.
  await tactical.send('SetTarget', { uuid: raiderUuid });
  await tactical.page.waitForFunction(
    () => (window as any).__messages?.some(
      (m: any) => m.type === 'WeaponsUpdate' && m.data.fire_ready === true,
    ),
    { timeout: 5_000 },
  );

  // Keep firing phasers on cooldown cycles until EntityDespawned arrives.
  // The raider has 60 HP; beam_damage_per_sec=5, beam_duration=6s → ~30 HP per shot.
  // Two shots should destroy it.  We fire on each WeaponsUpdate where fire_ready=true,
  // waiting up to 60 s total to account for cooldowns and CI latency.
  await tactical.page.evaluate(() => {
    (window as any).__fireInterval = setInterval(() => {
      const msgs: any[] = (window as any).__messages || [];
      const despawned = msgs.some((m: any) => m.type === 'EntityDespawned');
      if (despawned) { clearInterval((window as any).__fireInterval); return; }
      const ready = msgs.some((m: any) => m.type === 'WeaponsUpdate' && m.data.fire_ready === true);
      if (ready) {
        (window as any).__conn.send(JSON.stringify({ type: 'FirePhaser' }));
      }
    }, 200);
  });

  // Wait for EntityDespawned for the raider.
  await tactical.page.waitForFunction(
    (uuid: string) => (window as any).__messages?.some(
      (m: any) => m.type === 'EntityDespawned' && m.data.uuid === uuid,
    ),
    raiderUuid,
    { timeout: 60_000 },
  );

  await tactical.page.evaluate(() => clearInterval((window as any).__fireInterval));

  // Verify the despawn message is present.
  const despawnMsg = await tactical.page.evaluate((uuid: string) => {
    return (window as any).__messages.find(
      (m: any) => m.type === 'EntityDespawned' && m.data.uuid === uuid,
    );
  }, raiderUuid);
  expect(despawnMsg).toBeDefined();

  await helm.close();
  await tactical.close();
});
