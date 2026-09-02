// Issue #1291 — the explicit production browser GM profile boots the real
// authoritative WASM simulation without a renderer or a local player ship.

import {
  captureServerPageErrors,
  createTestClient,
  expect,
  expectFixtureWorld,
  readHostPeerId,
  test,
  waitForWasmReady,
} from './fixtures';

const GM_FIELD_PATH = 'assets/entities/smoke_gm_asteroid_field.toml';
const GM_ROCK_PATH = 'assets/entities/smoke_gm_ordinary_asteroid.toml';
const GM_REGION_PATH = 'assets/entities/smoke_gm_inert_region.toml';
const GM_LAYER_PATH = 'assets/worlds/smoke_gm_region_layer.toml';

// A real region-damage producer drives the activity stream. The NPC survives
// long enough to select from the first row, then leaves the map while its
// retained feed identity remains readable. Nothing in this fixture injects a
// Host Channel payload.
const GM_ACTIVITY_WORLD = `
[global]
seed = 1297
title = "GM activity smoke fixture"
description = "Bounded deterministic region damage for issue 1297."
sim_tick_hz = 30
gm_activity_history_depth = 4

[[entity]]
template_path = "assets/entities/region_radiation_zone.toml"
name = "entity.region_radiation_zone.name"
transform = { position = [0.0, 0.0, 0.0] }
overrides = { shape = { radius = 500.0 }, effects = { damage_zone = { damage_per_second = 50.0, shield_pierce = 1.0 } } }

[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "entity.alliance_courier.display_name"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
`;

const GM_MAP_WORLD = `
[global]
seed = 1296
title = "GM map coverage smoke fixture"
description = "Aggregate field and layered Region fixture for issue 1296."

[ambient_light]
color = [0.6, 0.55, 0.5]
brightness = 300.0

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[[entity]]
template_path = "${GM_FIELD_PATH}"
name = "server.gm.entity.kind.asteroid_field"
transform = { position = [0.0, 0.0, 0.0] }

# Dynamic layers consume the root session's already-resolved template cache on
# WASM. This declaration preloads the Region template without spawning a root
# copy, leaving the one below wholly owned by the load/unload layer lifecycle.
[[entity]]
template_path = "${GM_REGION_PATH}"
id = "gm-region-template-preload"
name = "gm_region_template_preload"
when = "flag(smoke_region_template_disabled)"

[script]
setup = """
on_world_loaded("load_gm_map_layer");

fn load_gm_map_layer(ctx) {
    ctx.effects.load_world("${GM_LAYER_PATH}");
    ctx.schedule.after(15, |ctx| {
        ctx.effects.unload_world("${GM_LAYER_PATH}");
        ctx.schedule.after(2, |ctx| {
            ctx.effects.load_world("${GM_LAYER_PATH}");
        });
    });
}
"""
`;

const GM_FIELD = `
name = "server.gm.entity.kind.asteroid_field"
tags = ["asteroid_field"]

[radar_appearance]
region_colour = [0.52, 0.32, 0.18]

[asteroid_field]
inner_radius = 0.0
outer_radius = 100.0
density = 0.02
spawn_distance = 75.0
despawn_distance = 100.0
asteroid_type_paths = ["${GM_ROCK_PATH}"]
cosmetic_type_paths = []
tags = ["asteroid_field"]

[asteroid_field.grid]
resolution = 25.0
fill_gameplay = 0.0
fill_cosmetic = 0.0
uniformity = 1.0
noise_freq = 0.02
noise_octaves = 1
density_noise_freq = 0.01
density_noise_octaves = 1
jitter = 0.0
cosmetic_y_offset = 0.0
`;

const GM_ORDINARY_ROCK = `
name = "entity.asteroid.name"
tags = ["asteroid"]

[collider]
shape = "Ball"
radius = 2.0
length = 0.0

[hull]
hull_integrity = 30.0

[radar_appearance]
icon = "asteroid"
colour = [0.55, 0.5, 0.42]
size = 2.0
`;

const GM_INERT_REGION = `
name = "entity.region_nebula.name"
tags = ["region"]

[radar_appearance]
region_colour = [0.2, 0.7, 0.8]

[shape]
type = "sphere"
radius = 50.0
`;

const GM_REGION_LAYER = `
[global]
seed = 1296
title = "GM map layered Region smoke fixture"
description = "Layer-owned inert Region for identity/removal coverage."

[[entity]]
template_path = "${GM_REGION_PATH}"
name = "entity.region_nebula.name"
transform = { position = [160.0, 0.0, 0.0] }
`;

test('the public Fleet role control selects the explicit GM boot profile', async ({ context }) => {
  const page = await context.newPage();
  const errors = captureServerPageErrors(page);
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);

  await page.click('#server-settings-btn');
  await page.click('.server-settings-tab[data-tab="gameplay"]');
  await Promise.all([
    page.waitForURL((url) => url.searchParams.get('gm') === '1'),
    page.click('[data-control="fleet-role-gm"]'),
  ]);
  await waitForWasmReady(page);

  expect(await page.evaluate(() => ({
    requested: document.documentElement.dataset.phoenixBootRequest,
    actual: window.wasm_boot_profile(),
    role: window.__hostFleetRole(),
  }))).toEqual({
    requested: 'browser-game-master',
    actual: 'browser-game-master',
    role: 'gm',
  });
  await expect(page.locator('#gm-console')).toBeVisible();
  await expect(page.locator('#canvas')).toBeHidden();
  expect(errors).toEqual([]);
});

test('rendererless GM maps and inspects stable local ship truth', async ({ context }) => {
  const page = await context.newPage();
  const errors = captureServerPageErrors(page);
  await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);

  const boot = await page.evaluate(() => ({
    webdriver: navigator.webdriver,
    requested: document.documentElement.dataset.phoenixBootRequest,
    actual: window.wasm_boot_profile(),
    role: window.__hostFleetRole(),
  }));
  expect(boot.webdriver).toBe(true);
  expect(boot.requested).toBe('browser-game-master');
  expect(boot.actual).toBe('browser-game-master');
  expect(boot.role).toBe('gm');
  await expect(page.locator('#gm-console')).toBeVisible();
  await expect(page.locator('#canvas')).toBeHidden();

  const tick0 = await page.evaluate(() => window.wasm_sim_tick());
  await page.waitForFunction((tick) => window.wasm_sim_tick() > tick + 5, tick0);

  await page.evaluate(() => window.__hostFleetOpen());
  await page.waitForFunction(() => {
    const state = window.__hostGmStartState?.();
    return state?.admitted === true
      && state.presentationReady === true
      && state.localValidation === true;
  });
  expect(await page.evaluate(() => window.__hostFleetState().role)).toBe('gm');

  await page.evaluate(() => document.getElementById('gm-ready-btn').click());
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');
  await page.waitForFunction(() => {
    const map = document.getElementById('gm-entity-map');
    return !map?.hidden && Array.isArray(map.state?.blips) && map.state.blips.length > 0;
  });

  const mapped = await page.evaluate(() => {
    const map = document.getElementById('gm-entity-map');
    return map.state.blips.map((blip) => ({
      uuid: blip.uuid,
      kind: blip.kind,
      destroyed: blip.destroyed,
    }));
  });
  expect(mapped.length).toBeGreaterThan(0);
  for (const blip of mapped) {
    expect(blip.uuid).toMatch(/^[0-9a-f-]{36}$/i);
    expect(['player_ship', 'npc_ship', 'structure', 'authored_asteroid']).toContain(blip.kind);
    expect(typeof blip.destroyed).toBe('boolean');
  }

  // The real custom element is the browser interaction seam: keyboard picks
  // the first stable UUID, while wheel zoom remains a local map operation.
  const map = page.locator('#gm-entity-map');
  await map.focus();
  await map.press('ArrowRight');
  await page.waitForSelector('#gm-entity-card:not([hidden])');
  const canvas = map.locator('canvas');
  await canvas.hover();
  await page.mouse.wheel(0, -120);

  const first = await page.evaluate(() => {
    const card = document.getElementById('gm-entity-card');
    return {
      id: card.dataset.entityId,
      selected: document.getElementById('gm-entity-map').dataset.selectedEntityId,
      kind: card.dataset.kind,
      destroyed: card.dataset.destroyed,
      hull: document.getElementById('gm-entity-hull').value,
      status: document.getElementById('gm-entity-status').textContent,
      position: document.getElementById('gm-entity-position').textContent,
      legend: document.getElementById('gm-map-legend').textContent,
      tick: window.wasm_sim_tick(),
    };
  });
  expect(first.id).toMatch(/^[0-9a-f-]{36}$/i);
  expect(first.selected).toBe(first.id);
  expect(['player_ship', 'npc_ship', 'structure', 'authored_asteroid']).toContain(first.kind);
  expect(Number(first.hull)).toBeGreaterThanOrEqual(0);
  expect(Number(first.hull)).toBeLessThanOrEqual(100);
  expect(first.status.length).toBeGreaterThan(0);
  expect(first.position.length).toBeGreaterThan(0);
  expect(first.legend).toContain('X: destroyed');

  await page.waitForFunction((tick) => window.wasm_sim_tick() > tick + 10, first.tick);
  const second = await page.evaluate(() => {
    const card = document.getElementById('gm-entity-card');
    return {
      id: card.dataset.entityId,
      selected: document.getElementById('gm-entity-map').dataset.selectedEntityId,
      mapHasId: document.getElementById('gm-entity-map').state.blips
        .some((blip) => blip.uuid === card.dataset.entityId),
    };
  });
  expect(second).toEqual({ id: first.id, selected: first.id, mapHasId: true });
  expect(errors).toEqual([]);
});

test('authored field and layer fixture supports aggregate and Region inspection end to end', async ({ context }) => {
  test.setTimeout(90_000);
  let layerRequests = 0;
  let regionTemplateRequests = 0;
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_MAP_WORLD }),
  );
  await context.route(`**/${GM_FIELD_PATH}`, (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_FIELD }),
  );
  await context.route(`**/${GM_ROCK_PATH}`, (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_ORDINARY_ROCK }),
  );
  await context.route(`**/${GM_REGION_PATH}`, (route) => {
    regionTemplateRequests += 1;
    return route.fulfill({ contentType: 'text/plain', body: GM_INERT_REGION });
  });
  await context.route(`**/${GM_LAYER_PATH}`, (route) => {
    layerRequests += 1;
    return route.fulfill({ contentType: 'text/plain', body: GM_REGION_LAYER });
  });

  const page = await context.newPage();
  const errors = captureServerPageErrors(page);
  await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await page.evaluate(() => {
    const dispatch = window.__hostChannel;
    window.__gmEntityPayloads = [];
    window.__hostChannel = (name, payload) => {
      if (name === 'gm_entity') {
        window.__gmEntityPayloads.push({
          tick: window.wasm_sim_tick(),
          payload: typeof payload === 'string' ? JSON.parse(payload) : payload,
        });
      }
      return dispatch(name, payload);
    };
  });
  await page.evaluate(() => window.__hostFleetOpen());
  await page.waitForFunction(() => {
    const state = window.__hostGmStartState?.();
    return state?.admitted === true
      && state.presentationReady === true
      && state.localValidation === true;
  });

  // Connect before GameStarted so the fixture proves its ordinary asteroid
  // population really exists independently of the GM Host Channel.
  const observer = await createTestClient(
    context,
    await readHostPeerId(page),
    { name: 'Observer' },
  );
  await page.evaluate(() => document.getElementById('gm-ready-btn').click());
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');
  const worldSetup = await observer.waitForMessage('WorldSetup', 10_000);
  expectFixtureWorld(worldSetup, GM_MAP_WORLD);
  await observer.page.waitForFunction(() => window.__messages?.some(
    (message) => message.type === 'AsteroidSpawned',
  ), null, { timeout: 10_000 });
  await page.bringToFront();
  await expect.poll(() => layerRequests, { timeout: 5_000 }).toBe(1);
  await expect.poll(() => regionTemplateRequests, { timeout: 5_000 }).toBe(1);

  await page.waitForFunction(() => window.__gmEntityPayloads.some((entry) => (
    entry.payload.entities.some((entity) => entity.kind === 'asteroid_field')
      && entry.payload.entities.some((entity) => entity.kind === 'region')
  )), null, { timeout: 10_000 });
  const projectionKinds = await page.evaluate(() => window.__gmEntityPayloads.map((entry) => ({
    tick: entry.tick,
    kinds: entry.payload.entities.map((entity) => entity.kind),
  })));
  expect(projectionKinds.some((entry) => entry.kinds.includes('asteroid_field'))).toBe(true);
  expect(projectionKinds.some((entry) => entry.kinds.includes('region'))).toBe(true);

  await page.waitForFunction(() => {
    const map = document.getElementById('gm-entity-map');
    return !map?.hidden
      && map.state?.regions?.filter((region) => region.kind === 'asteroid_field').length === 1
      && map.state?.regions?.some((region) => region.kind === 'region');
  });
  const mapped = await page.evaluate(() => {
    const map = document.getElementById('gm-entity-map');
    return {
      fields: map.state.regions.filter((region) => region.kind === 'asteroid_field'),
      inertRegions: map.state.regions.filter((region) => region.kind === 'region'),
      layered: map.state.regions.find((region) => region.kind === 'region'),
      blips: map.state.blips.map((blip) => ({ uuid: blip.uuid, kind: blip.kind })),
    };
  });
  expect(mapped.fields).toHaveLength(1);
  expect(mapped.inertRegions).toHaveLength(1);
  expect(mapped.fields[0]).toMatchObject({
    kind: 'asteroid_field',
    shape: 'torus',
    inner_radius: 0,
    outer_radius: 100,
    selectable: true,
  });
  expect(mapped.layered).toMatchObject({ kind: 'region', shape: 'sphere', selectable: true });
  expect(mapped.layered.uuid).toMatch(/^[0-9a-f-]{36}$/i);
  expect(mapped.blips.some((blip) => blip.kind === 'authored_asteroid')).toBe(false);

  const ordinaryRockIds = await observer.page.evaluate(() => window.__messages
    .filter((message) => message.type === 'AsteroidSpawned')
    .map((message) => message.data.uuid));
  expect(ordinaryRockIds.length).toBeGreaterThan(0);
  expect(await page.evaluate((ids) => {
    const map = document.getElementById('gm-entity-map');
    const mapIds = [...map.state.blips, ...map.state.regions].map((entry) => entry.uuid);
    return ids.filter((id) => mapIds.includes(id));
  }, ordinaryRockIds)).toEqual([]);

  // Touch the real layered Region at a deterministic off-centre point. This
  // dispatches the same TouchEvents a phone sends instead of calling
  // navigationSelect.
  const map = page.locator('#gm-entity-map');
  await page.evaluate((uuid) => {
    const surface = document.getElementById('gm-entity-map');
    const region = surface.state.regions.find((entry) => entry.uuid === uuid);
    const canvas = surface.shadowRoot.querySelector('canvas');
    const rect = canvas.getBoundingClientRect();
    const radius = Math.min(rect.width, rect.height) / 2;
    // An NPC contact shares the Region centre in this composed fixture. Touch
    // well inside the Region but away from that point so this assertion covers
    // Region hit testing; the strict point-wins overlap is pinned in Vitest.
    const touchX = region.x;
    const touchZ = region.z + region.radius * 0.75;
    const clientX = rect.left + rect.width / 2 + (touchX / surface.state.range) * radius;
    const clientY = rect.top + rect.height / 2 - (touchZ / surface.state.range) * radius;
    const point = new Touch({ identifier: 1296, target: canvas, clientX, clientY });
    canvas.dispatchEvent(new TouchEvent('touchstart', {
      bubbles: true,
      cancelable: true,
      touches: [point],
      targetTouches: [point],
      changedTouches: [point],
    }));
    canvas.dispatchEvent(new TouchEvent('touchend', {
      bubbles: true,
      cancelable: true,
      touches: [],
      targetTouches: [],
      changedTouches: [point],
    }));
  }, mapped.layered.uuid);
  await page.waitForSelector('#gm-entity-card:not([hidden])');
  const touched = await page.evaluate(() => {
    const card = document.getElementById('gm-entity-card');
    return {
      id: card.dataset.entityId,
      selected: document.getElementById('gm-entity-map').dataset.selectedEntityId,
      kind: card.dataset.kind,
      hullHidden: document.getElementById('gm-entity-hull').hidden,
      status: document.getElementById('gm-entity-status').textContent,
      tick: window.wasm_sim_tick(),
    };
  });
  expect(touched).toMatchObject({
    id: mapped.layered.uuid,
    selected: mapped.layered.uuid,
    kind: 'region',
    hullHidden: true,
  });
  expect(touched.status.length).toBeGreaterThan(0);

  // An absolute refresh updates the same UUID in place without dropping the
  // Region selection.
  await page.waitForFunction((tick) => window.wasm_sim_tick() > tick + 10, touched.tick);
  expect(await page.evaluate(() => ({
    id: document.getElementById('gm-entity-card').dataset.entityId,
    selected: document.getElementById('gm-entity-map').navigationSelectedUuid(),
  }))).toEqual({ id: mapped.layered.uuid, selected: mapped.layered.uuid });

  // Keyboard traversal shares the UUID-sorted point + Region set. Step from
  // the touched Region to the aggregate field and prove the shared inspector.
  await map.focus();
  const keySteps = await page.evaluate(({ from, to }) => {
    const state = document.getElementById('gm-entity-map').state;
    const ids = [...state.blips, ...state.regions]
      .filter((entry) => entry.uuid && (entry.world_x !== undefined || entry.selectable === true))
      .map((entry) => entry.uuid)
      .sort((left, right) => left.localeCompare(right));
    return (ids.indexOf(to) - ids.indexOf(from) + ids.length) % ids.length;
  }, { from: mapped.layered.uuid, to: mapped.fields[0].uuid });
  for (let index = 0; index < keySteps; index += 1) await map.press('ArrowRight');
  await page.waitForFunction((uuid) => (
    document.getElementById('gm-entity-card').dataset.entityId === uuid
  ), mapped.fields[0].uuid);
  expect(await page.evaluate(() => ({
    kind: document.getElementById('gm-entity-card').dataset.kind,
    hullHidden: document.getElementById('gm-entity-hull').hidden,
  }))).toEqual({ kind: 'asteroid_field', hullHidden: true });

  // Re-select the layer Region before its authored unload. The absolute
  // replacement clears map + inspector state; reload restores the exact UUID
  // without resurrecting stale selection.
  expect(await page.evaluate((uuid) => (
    document.getElementById('gm-entity-map').navigationSelect({ uuid })
  ), mapped.layered.uuid)).toBe(true);
  await page.waitForFunction((uuid) => {
    const surface = document.getElementById('gm-entity-map');
    return !surface.state.regions.some((region) => region.uuid === uuid)
      && surface.navigationSelectedUuid() === null
      && document.getElementById('gm-entity-card').hidden;
  }, mapped.layered.uuid, { timeout: 30_000 });
  await page.waitForFunction((uuid) => {
    const surface = document.getElementById('gm-entity-map');
    return surface.state.regions.some((region) => region.uuid === uuid)
      && surface.navigationSelectedUuid() === null;
  }, mapped.layered.uuid, { timeout: 10_000 });
  expect(await page.evaluate(() => {
    const regions = document.getElementById('gm-entity-map').state.regions;
    return {
      fields: regions.filter((region) => region.kind === 'asteroid_field').length,
      inert: regions.filter((region) => region.kind === 'region').length,
    };
  })).toEqual({ fields: 1, inert: 1 });

  // Pin the authoritative Host Channel boundary itself: its absolute
  // replacement stream contains the same UUID, then removes it, then restores
  // it. The DOM assertions above therefore cannot pass through stale UI state.
  const identityLifecycle = await page.evaluate((uuid) => window.__gmEntityPayloads.map((entry) => ({
    tick: entry.tick,
    present: entry.payload.entities.some((entity) => entity.entity_id === uuid),
  })), mapped.layered.uuid);
  const firstPresent = identityLifecycle.findIndex((entry) => entry.present);
  const removed = identityLifecycle.findIndex((entry, index) => index > firstPresent && !entry.present);
  const restored = identityLifecycle.findIndex((entry, index) => index > removed && entry.present);
  expect(firstPresent).toBeGreaterThanOrEqual(0);
  expect(removed).toBeGreaterThan(firstPresent);
  expect(restored).toBeGreaterThan(removed);
  expect(identityLifecycle[removed].tick).toBeGreaterThan(identityLifecycle[firstPresent].tick);
  expect(identityLifecycle[restored].tick).toBeGreaterThan(identityLifecycle[removed].tick);

  await observer.close();
  expect(errors).toEqual([]);
});

test('real damage and destruction stay ordered bounded and selectable after removal', async ({ context }) => {
  test.setTimeout(90_000);
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_ACTIVITY_WORLD }),
  );

  const page = await context.newPage();
  const errors = captureServerPageErrors(page);
  await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await page.evaluate(() => window.__hostFleetOpen());
  await page.waitForFunction(() => {
    const state = window.__hostGmStartState?.();
    return state?.admitted === true
      && state.presentationReady === true
      && state.localValidation === true;
  });
  await page.evaluate(() => document.getElementById('gm-ready-btn').click());
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');

  await page.waitForFunction(() => {
    const feed = window.__hostGmActivityState?.();
    return feed?.entries?.some((entry) => entry.category === 'damage')
      && document.querySelector('.gm-activity-entry[data-category="damage"]'
        + ' .gm-activity-link[data-involvement="victim"]:not(:disabled)');
  });
  const liveVictim = await page.evaluate(() => {
    const feed = window.__hostGmActivityState();
    const entry = feed.entries.find((candidate) => candidate.category === 'damage');
    const victim = entry.links.find((link) => link.role === 'victim').entity;
    const button = [...document.querySelectorAll('.gm-activity-entry[data-category="damage"]'
      + ' .gm-activity-link[data-involvement="victim"]')]
      .find((candidate) => candidate.dataset.entityId === victim.entity_id);
    button.click();
    return { id: victim.entity_id, name: button.textContent };
  });
  expect(liveVictim.id).toMatch(/^[0-9a-f-]{36}$/i);
  expect(liveVictim.name.length).toBeGreaterThan(0);
  await page.waitForFunction((uuid) => (
    document.getElementById('gm-entity-map').navigationSelectedUuid() === uuid
      && document.getElementById('gm-entity-card').dataset.entityId === uuid
  ), liveVictim.id);

  // Category and semantic ship filters compose as a strict AND; clearing is
  // one explicit operation that restores both defaults.
  await page.selectOption('#gm-activity-category-filter', 'damage');
  await page.selectOption('#gm-activity-ship-filter', liveVictim.id);
  expect(await page.locator('.gm-activity-entry').evaluateAll((rows, uuid) => (
    rows.length > 0 && rows.every((row) => {
      const state = window.__hostGmActivityState();
      const entry = state.entries.find((candidate) => String(candidate.tick) === row.dataset.tick
        && candidate.category === row.dataset.category
        && candidate.ships.some((ship) => ship.entity_id === uuid));
      return row.dataset.category === 'damage' && Boolean(entry);
    })
  ), liveVictim.id)).toBe(true);
  await page.click('#gm-activity-clear-filters');
  expect(await page.evaluate(() => ({
    category: document.getElementById('gm-activity-category-filter').value,
    ship: document.getElementById('gm-activity-ship-filter').value,
  }))).toEqual({ category: 'all', ship: 'all' });

  await page.waitForFunction((uuid) => {
    const feed = window.__hostGmActivityState?.();
    const map = document.getElementById('gm-entity-map');
    return feed?.capacity === 4
      && feed.entries.length === 4
      && feed.entries.at(-1)?.category === 'destruction'
      && feed.entries.at(-1)?.links.some((link) => (
        link.role === 'victim' && link.entity.entity_id === uuid
      ))
      && !map.state.blips.some((blip) => blip.uuid === uuid);
  }, liveVictim.id, { timeout: 30_000 });

  const retained = await page.evaluate((uuid) => {
    const feed = window.__hostGmActivityState();
    const buttons = [...document.querySelectorAll(
      `.gm-activity-link[data-involvement="victim"][data-entity-id="${uuid}"]`,
    )];
    return {
      capacity: feed.capacity,
      entries: feed.entries,
      buttons: buttons.map((button) => ({
        text: button.textContent,
        disabled: button.disabled,
      })),
      selected: document.getElementById('gm-entity-map').navigationSelectedUuid(),
      cardHidden: document.getElementById('gm-entity-card').hidden,
    };
  }, liveVictim.id);
  expect(retained.capacity).toBe(4);
  expect(retained.entries).toHaveLength(4);
  expect(retained.entries.every((entry) => entry.ships.some((ship) => (
    ship.entity_id === liveVictim.id
  )))).toBe(true);
  expect(retained.entries.at(-1).category).toBe('destruction');
  const finalTick = retained.entries.at(-1).tick;
  const finalBatch = retained.entries.filter((entry) => entry.tick === finalTick);
  expect(finalBatch.map((entry) => entry.category)).toEqual(['damage', 'destruction']);
  const damageSignatures = retained.entries
    .filter((entry) => entry.category === 'damage')
    .map((entry) => JSON.stringify(entry.detail.data));
  expect(new Set(damageSignatures).size).toBeLessThan(damageSignatures.length);
  expect(retained.buttons.length).toBeGreaterThan(0);
  expect(retained.buttons.every((button) => button.text === liveVictim.name)).toBe(true);
  expect(retained.buttons.every((button) => button.disabled)).toBe(true);
  expect(retained.selected).toBeNull();
  expect(retained.cardHidden).toBe(true);
  expect(errors).toEqual([]);
});
