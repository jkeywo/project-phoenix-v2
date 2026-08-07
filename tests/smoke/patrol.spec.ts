// Issue #210 — Smoke test: an NPC ship authored in a world actually spawns.
//
// After game start the raider entity (tags include "npc") must appear in the
// WorldSetup entity list, confirming that the
// entity template -> world -> entity-spawn pipeline is wired end-to-end.
//
// Issue #941: this used to serve the production `assets/worlds/patrol.toml` and
// assert on the tags that world's raider happened to carry — it broke when the
// hull behind that raider was replaced and the new one described itself
// differently, which says nothing about the spawn pipeline. The world is now a
// self-contained fixture (below), the same convention `tactical-fire-flow.spec.ts`
// uses; see the header of `fixtures.ts` for where every fixture world lives.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  expectFixtureWorld,
} from './fixtures';

// Self-contained smoke-test world: the player ship plus one NPC. No GLB-heavy
// templates, so the lobby preload gate clears immediately in CI.
//
// `ship_harrow_patrol.toml` is a production hull and is deliberately NOT
// asserted on — it is the subject being spawned, not the expectation. So this
// world authors the `npc` tag the assertion below selects on, rather than
// inheriting whatever the hull happens to describe itself as: the tags go
// through `overrides`, which the instance layer *replaces* wholesale
// (src/entities/entity_override.rs), so the production hull's third tag
// (`comms_contact`) is dropped and this spec owns the whole array. Three
// shipped worlds narrow this same hull the same way.
//
// A bare `tags = [...]` on the `[[entity]]` block would NOT work: `WorldEntity`
// (src/world/config.rs) has no such field and serde drops unknown keys
// silently, which is precisely how the production coupling survived unnoticed.
const PATROL_TEST_WORLD = `
[global]
seed = 42
title = "Patrol Spawn Fixture"
description = "Player ship + one NPC raider; see tests/smoke/patrol.spec.ts."

[ambient_light]
color      = [0.6, 0.55, 0.5]
brightness = 300.0

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id            = "player-ship"
transform     = { position = [0.0, 0.0, 0.0] }
spawn_on      = "game_start"
overrides     = { tags = ["ship"] }

[[entity]]
template_path = "assets/entities/ship_harrow_patrol.toml"
name          = "raider_alpha"
transform     = { position = [400.0, 0.0, -400.0] }
spawn_on      = "game_start"
overrides     = { tags = ["ship", "npc"] }
`;

test('a world-authored NPC appears in WorldSetup after game start', async ({ context }) => {
  // Intercept the default world fetch and serve the fixture instead.
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: PATROL_TEST_WORLD }),
  );

  const serverPage = await context.newPage();
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  // Two-player lobby: Helm station carries CaptainChair, so the helm player starts.
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const tactical = await createTestClient(context, hostId, { name: 'Tac' });

  await helm.send('SelectStation', { station: 'Helm' });
  await helm.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    helm.token,
    { timeout: 5_000 },
  );

  await tactical.send('SelectStation', { station: 'Tactical' });
  await tactical.page.waitForFunction(
    (t) => (window as any).__messages?.some(
      (m: any) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    tactical.token,
    { timeout: 5_000 },
  );

  await helm.send('SetReady', { ready: true });
  await tactical.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 10_000);

  // WorldSetup is broadcast once after GameStarted.
  const worldSetupMsg = await helm.waitForMessage('WorldSetup', 5_000) as any;

  // The world that was parsed must be the fixture above, not production
  // `assets/worlds/default.toml` — which also carries an `npc`-tagged raider
  // and would pass the assertion below on the wrong content.
  expectFixtureWorld(worldSetupMsg, PATROL_TEST_WORLD);

  const entities: any[] = worldSetupMsg?.data?.world?.entities ?? [];

  // Exactly one NPC is authored, so `npc` is unambiguous here. (Selecting on
  // `enemy` too would re-couple this to a hull's self-description — that tag is
  // descriptive only; hostility comes from factions.)
  const raider = entities.find(
    (e: any) => Array.isArray(e.tags) && e.tags.includes('npc'),
  );

  expect(
    raider,
    `Expected the fixture's NPC (tag: npc) in WorldSetup.world.entities. Got: ${JSON.stringify(entities.map((e: any) => ({ id: e.id, tags: e.tags })))}`,
  ).toBeDefined();

  await helm.close();
  await tactical.close();
});
