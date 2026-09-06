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
  waitForJoinCode,
} from './fixtures';
import { ts } from './strings';

const GM_FIELD_PATH = 'assets/entities/smoke_gm_asteroid_field.toml';
const GM_ROCK_PATH = 'assets/entities/smoke_gm_ordinary_asteroid.toml';
const GM_REGION_PATH = 'assets/entities/smoke_gm_inert_region.toml';
const GM_LAYER_PATH = 'assets/worlds/smoke_gm_region_layer.toml';

// Scenario-authored, presentation-only GM role presets (issue #1319). Two
// distinct ids with disjoint panels/quick_actions, alongside the built-in
// reserved "all", so a browser test can prove both authored options render
// with real String Table copy and that live-switching one narrows exactly
// its own authored panels/quick actions -- never any GM's authority.
const GM_ROLE_PRESET_WORLD = `
[global]
seed = 1319
title = "GM role preset smoke fixture"
description = "Scenario-authored presentation-only role presets for issue 1319."
sim_tick_hz = 30

[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "entity.alliance_courier.display_name"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[[gm_role_preset]]
id = "tactical"
label = "world.smoke_gm_role_preset.tactical.label"
panels = ["gm-map-panel", "gm-activity"]
quick_actions = ["gm-session-pause"]

[[gm_role_preset]]
id = "narrative"
label = "world.smoke_gm_role_preset.narrative.label"
`;

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
// Issue #1301: a manual-only `gm_event` with an implied Fire control, plus one
// ordinary entity so the GM-only session has a world to run. The handler's own
// effect is covered by the Rust pipeline tests; what this spec is here for is
// the browser half — the row's String Table label, its Fire control, the
// attributed Applied result, and the one-shot lifecycle closing behind it.
const GM_EVENT_WORLD = `
[global]
seed = 1301
title = "GM event smoke fixture"
description = "Manual gm_event Fire coverage for issue 1301."

[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "entity.alliance_courier.display_name"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[script]
setup = """
gm_event("breach_alarm", "world.smoke_gm.event.breach_alarm", "on_breach_alarm");
fn on_breach_alarm(ctx) { ctx.flags.increment("breach_alarms", 1); }
"""
`;

async function selectAndWait(client, station) {
  await client.send('SelectStation', { station });
  await client.page.waitForFunction(
    ({ token, expected }) => window.__messages?.some(
      message => message.type === 'StationAssigned'
        && message.data.token === token
        && message.data.station?.toLowerCase() === expected.toLowerCase(),
    ),
    { token: client.token, expected: station },
    { timeout: 15_000 },
  );
}

async function openFleetTab(page) {
  await page.bringToFront();
  await page.click('#server-settings-btn');
  await page.click('.server-settings-tab[data-tab="gameplay"]');
  await page.waitForSelector('[data-control="fleet-code"]', { state: 'attached' });
}

async function joinFleetAsGm(page, code) {
  await page.evaluate(() => localStorage.removeItem('phoenix.fleet.gm-identity.v1'));
  await openFleetTab(page);
  await Promise.all([
    page.waitForURL(url => url.searchParams.get('gm') === '1'),
    page.click('[data-control="fleet-role-gm"]'),
  ]);
  await waitForWasmReady(page);
  await openFleetTab(page);
  await page.fill('[data-control="fleet-code"]', code);
  await page.click('[data-control="fleet-join"]');
  await page.waitForFunction(
    () => {
      const state = window.__hostGmStartState?.();
      return state?.admitted === true
        && state.presentationReady === true
        && state.localValidation === true;
    },
    undefined,
    { timeout: 30_000 },
  );
  await page.click('#server-settings-btn');
  await page.waitForSelector('#server-settings-overlay', { state: 'hidden' });
}

async function reconnectRealCrew(context, hostId, token, station) {
  const page = await context.newPage();
  await page.addInitScript(sessionToken => {
    sessionStorage.setItem('session-token', sessionToken);
  }, token);
  await page.goto(`/client/index.html#${hostId}`);
  await page.waitForFunction(
    ({ sessionToken, expectedStation }) => {
      const state = window.lobbyState;
      const player = state?.players?.find(candidate => candidate.token === sessionToken);
      return state?.phase === 'InProgress'
        && player?.station?.toLowerCase() === expectedStation;
    },
    { sessionToken: token, expectedStation: station.toLowerCase() },
    { timeout: 30_000 },
  );
  // Disconnect deliberately un-readies a retained Station holder. Resume via
  // the shipped Take Station control so the real console is mounted; the
  // roster assertion above is the ownership-preservation evidence.
  await expect(page.locator('#ready-btn')).toBeVisible({ timeout: 15_000 });
  await page.locator('#ready-btn').click();
  try {
    await page.waitForSelector(`#${station.toLowerCase()}-ui.active`, { timeout: 30_000 });
  } catch (error) {
    const state = await page.evaluate(() => ({
      url: location.href,
      token: sessionStorage.getItem('session-token'),
      status: document.getElementById('status')?.textContent ?? null,
      activeStations: [...document.querySelectorAll('[id$="-ui"].active')]
        .map(element => element.id),
      stationRows: document.querySelectorAll('#station-list .station-row').length,
    }));
    throw new Error(`${station} reconnect did not restore its console: ${JSON.stringify(state)}`, {
      cause: error,
    });
  }
  return page;
}

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

test('rendererless GM maps and inspects stable local ship truth', { tag: '@core' }, async ({ context }) => {
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

// Truth / Crew Knowledge / Difference comparison panel (issue #1318). A NEW
// feature needs a NEW @core test (AGENTS.md Testing Strategy). This does NOT
// reuse the rendererless GM boot above: that profile opens its OWN fleet as
// lead with no ship host ever joining it, so `roster.len() == 0` and
// `spawn_game_start_entities` never marks any GameStart row `is_fleet_ship`
// (src/server_app/world_setup.rs) — no `Ship`/`FleetSlotOf` entity, therefore
// no row `gm_station`'s ships query (src/gm_projection.rs) can ever find, no
// matter how long the wait or whether an ordinary (station-less) crew member
// also connects to that same solo GM host. A `selectedShipId` needs a REAL
// fleet ship, which needs a SHIP host (not `?gm=1`) to open the fleet and the
// GM to join it — the same topology "a GM reaches and operates a spatial Helm
// Station" below uses, trimmed to the minimum this panel needs: no crew, no
// station takeover, since the lone host's own GameStart ship spawns and
// Backfills every station whether or not anyone ever connects to fly it.
// Fuller coverage of the panel's individual fixes (finding 1: no raw Truth
// String Table id leaks; finding 2: no double-resolved `⟨...⟩` wrapper) lives
// at the pure/DOM-controller level in tests/client/gm-knowledge-compare.test.js;
// what only a real end-to-end run proves is that the actual WASM wiring
// renders something at all (issue #1318 review, finding 6).
test('a GM compares Truth and Crew Knowledge for the one connected fleet ship', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(60_000);

  const ship = await context.newPage();
  const shipErrors = captureServerPageErrors(ship);
  await ship.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(ship);

  await ship.evaluate(() => window.__hostFleetOpen());
  await waitForJoinCode(ship, 'fleet-code', 30_000);
  const fleetCode = await ship.locator('#fleet-code').textContent();

  const gm = await context.newPage();
  const gmErrors = captureServerPageErrors(gm);
  await gm.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(gm);
  await joinFleetAsGm(gm, fleetCode);

  await gm.evaluate(() => document.getElementById('gm-ready-btn').click());
  await Promise.all([
    ship.waitForFunction(() => window.__saveSlotsPhase === 'InProgress', undefined, {
      timeout: 30_000,
    }),
    gm.waitForFunction(() => window.__saveSlotsPhase === 'InProgress', undefined, {
      timeout: 30_000,
    }),
  ]);

  await gm.waitForFunction(() => {
    const panel = document.getElementById('gm-knowledge-panel');
    return !!panel && !panel.hidden && !!window.__hostGmKnowledgeState?.().selectedShipId;
  }, undefined, { timeout: 30_000 });
  // `publish_sensors_blackboard` (src/ship/sensors.rs) computes every ship's
  // Sensors blackboard regardless of locality, so `default.toml`'s starbase
  // and patrol raider — within this world's default radar range from the
  // spawn point — join the Sensors-contacts category from the first tick.
  // Comms is NOT the same: `console::comms::server.rs`'s blackboard system
  // clones its one shared `local_bb` onto the process's OWN local ship only
  // and gives every OTHER ship an empty default — and the GM peer never has
  // a local ship (AGENTS.md), so this ship's Comms blackboard, Truth AND Crew
  // Knowledge alike, stays the "equal empty" state on a GM's own instance
  // until a per-ship Comms filter (#1063/#1065/#1070) lands. That is exactly
  // the "equal" state this comparison must cover, not a bug to route around —
  // so this test asserts on the category the current architecture actually
  // populates (Sensors) and leaves Comms to fold into the same leak check
  // without requiring it non-empty.
  await gm.waitForFunction(() => {
    const rows = document.getElementById('gm-knowledge-contacts-rows');
    return !!rows && rows.children.length > 0;
  }, undefined, { timeout: 30_000 });
  const knowledgeTruthTexts = await gm.evaluate(() => [
    ...document.getElementById('gm-knowledge-comms-contacts-rows').children,
    ...document.getElementById('gm-knowledge-contacts-rows').children,
  ].map((row) => row.children[1].textContent));
  expect(knowledgeTruthTexts.length).toBeGreaterThan(0);
  for (const text of knowledgeTruthTexts) {
    expect(text).not.toMatch(/^world\./);
    expect(text).not.toContain('⟨');
  }

  expect(shipErrors).toEqual([]);
  expect(gmErrors).toEqual([]);
});

/// Issue #1301 exit evidence in a real browser: an authored `gm_event` reaches
/// the GM mission panel with its String Table label and an implied Fire, one
/// press runs the ordinary handler through the ordinary trigger pipeline, and
/// the one-shot lifecycle closes behind it so a second Fire is unavailable.
test('a manual gm_event is listed, fired once, and then spent in the GM mission panel', async ({ context }) => {
  test.setTimeout(90_000);
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_EVENT_WORLD }),
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
  }, undefined, { timeout: 30_000 });
  await page.evaluate(() => document.getElementById('gm-ready-btn').click());
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');

  // The authored event reaches the panel from the authoritative projection —
  // nothing in this spec injects a Host Channel payload.
  const row = page.locator('#gm-mission-events .gm-mission-event[data-event-id="base-world::breach_alarm"]');
  await expect(row).toBeVisible({ timeout: 30_000 });
  await expect(row.locator('.gm-mission-event-label'))
    .toHaveText(ts('world.smoke_gm.event.breach_alarm'));
  await expect(row.locator('.gm-mission-event-state'))
    .toHaveText(ts('server.gm.mission.state_ready'));
  await expect(page.locator('#gm-mission-empty')).toBeHidden();
  expect(await page.evaluate(() => window.__hostGmMissionState())).toMatchObject({
    events: 1,
    fireable: 1,
  });

  const fire = row.locator('button[data-role="fire"]');
  await expect(fire).toBeEnabled();
  await fire.click();

  // One attributed Applied result, and the authored one-shot lifecycle spent.
  const applied = page.locator('#gm-mission-log .gm-mission-log-entry[data-outcome="applied"]');
  await expect(applied).toHaveCount(1, { timeout: 30_000 });
  await expect(applied).toContainText('base-world::breach_alarm');
  await expect(row).toHaveAttribute('data-spent', 'true', { timeout: 30_000 });
  await expect(row.locator('.gm-mission-event-state'))
    .toHaveText(ts('server.gm.mission.state_spent'));
  await expect(fire).toBeDisabled();
  expect(await page.evaluate(() => window.__hostGmMissionState())).toMatchObject({
    events: 1,
    fireable: 0,
    pending: 0,
  });

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

test('a GM reaches and operates a spatial Helm Station at 1280x720, then releases it', async ({ context }) => {
  test.setTimeout(180_000);

  const ship = await context.newPage();
  const shipErrors = captureServerPageErrors(ship);
  await ship.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(ship);
  const hostId = await readHostPeerId(ship);

  // Four connected players select the fixed 6P layout. Helm is deliberately
  // the disconnected Station: unlike Captain, its authentic interface needs
  // the complete spatial/world projection to be useful.
  const captain = await createTestClient(context, hostId, { name: 'Captain' });
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const engineering = await createTestClient(context, hostId, { name: 'Engineering' });
  const science = await createTestClient(context, hostId, { name: 'Science' });
  await selectAndWait(captain, 'Captain');
  await selectAndWait(helm, 'Helm');
  await selectAndWait(engineering, 'Engineering');
  await selectAndWait(science, 'Science');

  await ship.evaluate(() => window.__hostFleetOpen());
  await waitForJoinCode(ship, 'fleet-code', 30_000);
  const fleetCode = await ship.locator('#fleet-code').textContent();

  const gm = await context.newPage();
  const gmErrors = captureServerPageErrors(gm);
  const gmErrorDetails = [];
  gm.on('pageerror', error => gmErrorDetails.push(error.stack || error.message));
  await gm.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(gm);
  await joinFleetAsGm(gm, fleetCode);

  for (const crew of [captain, helm, engineering, science]) {
    await crew.send('SetReady', { ready: true });
  }
  await gm.evaluate(() => document.getElementById('gm-ready-btn').click());
  await helm.waitForMessage('GameStarted', 20_000);
  await Promise.all([
    ship.waitForFunction(() => window.__saveSlotsPhase === 'InProgress', undefined, {
      timeout: 30_000,
    }),
    gm.waitForFunction(() => window.__saveSlotsPhase === 'InProgress', undefined, {
      timeout: 30_000,
    }),
  ]);

  const helmToken = helm.token;
  await helm.close();
  await ship.waitForFunction(
    // eslint-disable-next-line no-eval
    token => (0, eval)('hostConnections').targets(`token:${token}`, 'reliable').length === 0,
    helmToken,
    { timeout: 15_000 },
  );

  // Select the exact projected Helm row. Its URL and Backfill rating both
  // come from the authoritative local ship projection; the test never supplies
  // a console path or clones a Helm control.
  await gm.waitForFunction(
    () => window.__hostGmStationState?.().projection?.ships?.some(shipRow =>
      shipRow.stations?.some(station => station.station_id === 'helm'
        && station.rating === 'Backfill')
      && shipRow.entities?.length > 0
      && shipRow.entity_states?.some(entity => Array.isArray(entity.position))
      && Number.isFinite(shipRow.ship_pose?.x)
      && Number.isFinite(shipRow.ship_pose?.yaw)),
    undefined,
    { timeout: 30_000 },
  );
  await gm.evaluate(() => {
    const select = document.getElementById('gm-station-select');
    const option = [...select.options].find(candidate => candidate.textContent.endsWith('Helm'));
    if (!option) throw new Error('Helm Station is absent from the GM projection');
    select.value = option.value;
    select.dispatchEvent(new Event('change'));
  });
  await gm.waitForFunction(
    () => {
      const row = window.__hostGmStationState?.().selectedRow;
      return row?.station?.station_id === 'helm'
        && row.station.rating === 'Backfill'
        && row.station.console === 'gui/cruiser/helm.html';
    },
    undefined,
    { timeout: 15_000 },
  );
  await expect(gm.locator('#gm-station-frame'))
    .toHaveAttribute('src', 'gui/cruiser/helm.html');

  // The configured Chromium viewport is 1280x720. Reach the takeover through
  // the GM surface's real scroll container, prove its bounding box is visible,
  // and use an ordinary Playwright click (no force/evaluate bypass).
  expect(gm.viewportSize()).toEqual({ width: 1280, height: 720 });
  // Scope the host's crew QR overlay: Playwright also pierces the GM map's
  // shadow root, which has its own unrelated private #overlay.
  await expect(gm.locator('#overlay:has(> #qr-panel)')).toBeHidden();
  const takeover = gm.locator('#gm-station-toggle');
  await takeover.scrollIntoViewIfNeeded();
  await expect(takeover).toBeVisible();
  const takeoverBox = await takeover.boundingBox();
  expect(takeoverBox.y).toBeGreaterThanOrEqual(0);
  expect(takeoverBox.y + takeoverBox.height).toBeLessThanOrEqual(720);
  expect(await gm.locator('#gm-console').evaluate(element => ({
    scrollTop: element.scrollTop,
    scrollHeight: element.scrollHeight,
    clientHeight: element.clientHeight,
  }))).toMatchObject({ clientHeight: 720 });
  expect(await gm.locator('#gm-console').evaluate(
    element => element.scrollHeight > element.clientHeight,
  )).toBe(true);
  await takeover.click();
  const operatorId = await gm.evaluate(() => window.__hostLocalGm().id);
  await gm.waitForFunction(
    operator => {
      const row = window.__hostGmStationState?.().selectedRow;
      return row?.station?.operators?.includes(operator)
        && row.ship.control_sources?.['helm-thrust'] === 'Human';
    },
    operatorId,
    { timeout: 30_000 },
  );

  const frameElement = gm.locator('#gm-station-frame');
  await frameElement.scrollIntoViewIfNeeded();
  const frameBox = await frameElement.boundingBox();
  expect(frameBox.y).toBeGreaterThanOrEqual(0);
  expect(frameBox.y + frameBox.height).toBeLessThanOrEqual(720);
  const helmFrame = gm.frameLocator('#gm-station-frame');
  const radar = helmFrame.locator('ph-helm-radar');
  const joystick = helmFrame.locator('#helm-joystick');
  await expect(radar).toBeVisible({ timeout: 15_000 });
  await expect(joystick).toBeVisible({ timeout: 15_000 });
  const spatial = await radar.evaluate(element => ({
    x: element.state?.x,
    z: element.state?.z,
    range: element.state?.range,
    blips: element.state?.blips?.length ?? 0,
  }));
  const authoredConfig = await gm.evaluate(() => {
    const config = window.__hostGmStationState().selectedRow.ship.ship_config;
    return {
      hullId: config.hull_id,
      helmRange: config.helm_radar_range,
      sensorsRange: config.sensors_radar_range,
      navRange: config.nav_chart_range,
      hostileArcColor: config.hostile_arc_color,
      helmTutorials: config.station_tutorials?.helm?.length ?? 0,
      helmAssistRatings: Object.keys(config.station_assist_gaps?.helm || {}),
      phaserArcs: config.phaser_banks?.map(bank => bank.fire_arc_deg) || [],
    };
  });
  expect(Number.isFinite(spatial.x)).toBe(true);
  expect(Number.isFinite(spatial.z)).toBe(true);
  expect(spatial.range).toBe(authoredConfig.helmRange);
  expect(spatial.blips).toBeGreaterThan(0);
  expect(authoredConfig).toMatchObject({
    hullId: 'AEV-1864',
    helmRange: 93.75,
    sensorsRange: 300,
    navRange: 800,
    hostileArcColor: [1, 0.3, 0.3, 0.07],
    phaserArcs: [270, 270],
  });
  expect(authoredConfig.helmTutorials).toBeGreaterThan(0);
  expect(authoredConfig.helmAssistRatings.length).toBeGreaterThan(0);

  // Capture the iframe's own correlated semantic actions at the parent seam.
  // The assertions below compare those opaque identities to canonical Rust
  // results, then read the authentic iframe's ordinary feedback surface.
  await gm.evaluate(() => {
    window.__gmStationCorrelatedActions = [];
    window.addEventListener('message', event => {
      const frame = document.getElementById('gm-station-frame');
      if (event.source !== frame?.contentWindow || event.data?.type !== 'console_action') return;
      try {
        const action = JSON.parse(event.data.payload);
        if (typeof action.correlation === 'string') {
          window.__gmStationCorrelatedActions.push(action);
        }
      } catch (_) { /* malformed actions are outside this tracer */ }
    });
  });

  const impulse = helmFrame.locator('#impulse-btn').locator('#btn');
  const impulseFeedback = helmFrame.locator(
    '.semantic-action-feedback__item[data-action-id="helm.impulse"]',
  );
  await expect(impulse).toBeVisible();
  await impulse.click();
  await expect(impulseFeedback).toHaveAttribute('data-state', 'Applied', { timeout: 30_000 });
  await gm.waitForFunction(
    operator => {
      const action = window.__gmStationCorrelatedActions
        ?.find(candidate => candidate.semantic_action === 'helm.impulse');
      return !!action && window.__hostGmStationState().projection.results.some(result =>
        result.operator_id === operator
          && result.correlation === action.correlation
          && result.action_kind === 'station-command'
          && result.outcome === 'applied');
    },
    operatorId,
    { timeout: 30_000 },
  );

  // Keep the authentic Helm success above, then prove the same iframe feedback
  // lifecycle waits for a real System consumer refusal. Tactical Backfill has
  // loaded each authored one-round tube before takeover; a double click sends
  // two ordinary correlated FireTorpedo actions before the next projection can
  // disable the button. The first command consumes the round (or is held by an
  // authored weapon gate) and at least one command is refused by the torpedo
  // consumer. Admission is still valid throughout: `system-refused`, never the
  // later station-not-puppeted refusal exercised after Helm release below.
  const stationSelect = gm.locator('#gm-station-select');
  const stationOptions = await stationSelect.locator('option').evaluateAll(options => (
    options.map(option => ({ value: option.value, label: option.textContent || '' }))
  ));
  const helmOption = stationOptions.find(option => option.label.endsWith('Helm'));
  const tacticalOption = stationOptions.find(option => option.label.endsWith('Tactical'));
  expect(helmOption).toBeTruthy();
  expect(tacticalOption).toBeTruthy();

  await stationSelect.scrollIntoViewIfNeeded();
  await stationSelect.selectOption(tacticalOption.value);
  await gm.waitForFunction(
    () => {
      const row = window.__hostGmStationState?.().selectedRow;
      return row?.station?.station_id === 'tactical'
        && row.station.rating === 'Backfill'
        && row.station.console === 'gui/cruiser/tactical.html';
    },
    undefined,
    { timeout: 30_000 },
  );
  await expect(frameElement).toHaveAttribute('src', 'gui/cruiser/tactical.html');
  await frameElement.scrollIntoViewIfNeeded();
  const tacticalFrame = gm.frameLocator('#gm-station-frame');
  const torpedoControls = tacticalFrame.locator('#torpedo-controls');
  const loadedTorpedoFire = torpedoControls.locator('.tube-row .btn.armed').first();
  await expect(torpedoControls).toBeVisible({ timeout: 20_000 });
  await expect(loadedTorpedoFire).toBeEnabled({ timeout: 30_000 });

  await takeover.scrollIntoViewIfNeeded();
  await takeover.click();
  await gm.waitForFunction(
    operator => {
      const row = window.__hostGmStationState?.().selectedRow;
      return row?.station?.station_id === 'tactical'
        && row.station.operators?.includes(operator);
    },
    operatorId,
    { timeout: 30_000 },
  );
  await loadedTorpedoFire.scrollIntoViewIfNeeded();
  const torpedoFireBox = await loadedTorpedoFire.boundingBox();
  expect(torpedoFireBox.y).toBeGreaterThanOrEqual(0);
  expect(torpedoFireBox.y + torpedoFireBox.height).toBeLessThanOrEqual(720);

  await gm.evaluate(() => { window.__gmStationCorrelatedActions = []; });
  await loadedTorpedoFire.dblclick();
  await gm.waitForFunction(
    () => window.__gmStationCorrelatedActions
      ?.filter(candidate => candidate.semantic_action === 'tactical.torpedo-fire')
      .length >= 2,
    undefined,
    { timeout: 15_000 },
  );
  const torpedoCorrelations = await gm.evaluate(() => (
    window.__gmStationCorrelatedActions
      .filter(candidate => candidate.semantic_action === 'tactical.torpedo-fire')
      .slice(0, 2)
      .map(candidate => candidate.correlation)
  ));
  await gm.waitForFunction(
    ({ operator, correlations }) => correlations.every(correlation => (
      window.__hostGmStationState().projection.results.some(result => (
        result.operator_id === operator
          && result.correlation === correlation
          && result.action_kind === 'station-command'
          && (result.outcome === 'applied' || result.outcome === 'refused')
      ))
    )),
    { operator: operatorId, correlations: torpedoCorrelations },
    { timeout: 30_000 },
  );
  const torpedoResults = await gm.evaluate(
    ({ operator, correlations }) => window.__hostGmStationState().projection.results
      .filter(result => result.operator_id === operator
        && correlations.includes(result.correlation)),
    { operator: operatorId, correlations: torpedoCorrelations },
  );
  expect(torpedoResults).toEqual(expect.arrayContaining([
    expect.objectContaining({
      action_kind: 'station-command',
      outcome: 'refused',
      reason: 'system-refused',
    }),
  ]));
  const tacticalFeedback = tacticalFrame.locator(
    '.semantic-action-feedback__item[data-action-id="tactical.torpedo-fire"]',
  );
  await expect(tacticalFeedback).toHaveAttribute('data-state', 'Refused', {
    timeout: 30_000,
  });

  // Release only Tactical, then return to the still-active Helm takeover for
  // the existing spatial input, crew visibility and ordered release proof.
  await takeover.scrollIntoViewIfNeeded();
  await takeover.click();
  await gm.waitForFunction(
    operator => {
      const row = window.__hostGmStationState?.().selectedRow;
      return row?.station?.station_id === 'tactical'
        && row.station.rating === 'Backfill'
        && !row.station.operators?.includes(operator);
    },
    operatorId,
    { timeout: 30_000 },
  );
  await stationSelect.scrollIntoViewIfNeeded();
  await stationSelect.selectOption(helmOption.value);
  await gm.waitForFunction(
    operator => {
      const row = window.__hostGmStationState?.().selectedRow;
      return row?.station?.station_id === 'helm'
        && row.station.operators?.includes(operator)
        && row.station.console === 'gui/cruiser/helm.html';
    },
    operatorId,
    { timeout: 30_000 },
  );
  await expect(frameElement).toHaveAttribute('src', 'gui/cruiser/helm.html');
  await frameElement.scrollIntoViewIfNeeded();
  await expect(joystick).toBeVisible({ timeout: 20_000 });

  // Drive the real iframe's existing keyboard path. The authoritative activity
  // projection, not a DOM side effect, proves that Helm admission applied it.
  const priorCrewActivity = await captain.page.evaluate(() => {
    for (const message of [...(window.__messages || [])].reverse()) {
      if (message.type !== 'SimState') continue;
      const row = message.data.snapshot?.station_puppets?.find(
        entry => entry.station === 'helm',
      );
      if (row?.latest_activity) return row.latest_activity;
    }
    return null;
  });
  await joystick.focus();
  await gm.keyboard.down('ArrowUp');

  await gm.waitForFunction(
    operator => window.__hostGmStationState?.().projection?.activity?.some(entry =>
      entry.operator_id === operator && entry.target === 'helm-thrust'),
    operatorId,
    { timeout: 30_000 },
  );
  await gm.keyboard.up('ArrowUp');
  const releaseTick = await gm.evaluate(() => window.wasm_sim_tick());
  await gm.waitForFunction(
    tick => window.wasm_sim_tick() > tick + 12,
    releaseTick,
    { timeout: 15_000 },
  );
  // The authentic joystick emits both longitudinal thrust and its steering
  // vector. Crew projection intentionally carries only the canonical latest
  // admitted activity, so first observe the next real crew row and then prove
  // that exact row exists in the GM's canonical activity projection. This does
  // not assume which of the two existing Helm commands publishes last.
  const crewActivityHandle = await captain.page.waitForFunction(
    ({ operator, prior }) => {
      for (const message of [...(window.__messages || [])].reverse()) {
        if (message.type !== 'SimState') continue;
        const row = message.data.snapshot?.station_puppets?.find(
          entry => entry.station === 'helm',
        );
        const activity = row?.latest_activity;
        if (!row?.operators?.includes(operator) || activity?.operator_id !== operator) continue;
        if (!prior || activity.tick !== prior.tick || activity.target !== prior.target
            || activity.action !== prior.action) return activity;
      }
      return false;
    },
    { operator: operatorId, prior: priorCrewActivity },
    { timeout: 30_000 },
  );
  const expectedCrewActivity = await crewActivityHandle.jsonValue();
  expect(['helm-thrust', 'helm-steering']).toContain(expectedCrewActivity.target);
  await gm.waitForFunction(
    ({ operator, activity }) => window.__hostGmStationState().projection.activity.some(entry =>
      entry.operator_id === operator
        && entry.station === 'helm'
        && entry.tick === activity.tick
        && entry.target === activity.target
        && entry.action === activity.action),
    { operator: operatorId, activity: expectedCrewActivity },
    { timeout: 30_000 },
  );

  // Reconnecting the same Player proves Station ownership survived takeover.
  // The real authored Helm iframe receives the crew-public takeover/activity
  // projection and renders the shared banner; no test-only surface is involved.
  const helmDuringTakeover = await reconnectRealCrew(
    context, hostId, helmToken, 'helm',
  );
  const crewBanner = helmDuringTakeover
    .frameLocator('#helm-iframe')
    .locator('#gm-takeover-banner:not([hidden])');
  await expect(crewBanner).toBeVisible({ timeout: 20_000 });
  await expect(crewBanner).toHaveAttribute('data-latest-operator', operatorId);
  await helmDuringTakeover.close();
  await ship.waitForFunction(
    // eslint-disable-next-line no-eval
    token => (0, eval)('hostConnections').targets(`token:${token}`, 'reliable').length === 0,
    helmToken,
    { timeout: 15_000 },
  );

  // Freeze fixed ticks through the real GM Pause control. Release and then
  // click the still-mounted authentic Impulse control while the projection is
  // intentionally unchanged; Resume applies those two canonical actions in
  // submission order. The command is therefore refused after release, and its
  // exact iframe-minted correlation must settle rather than timing out Pending.
  const pause = gm.locator('#gm-session-pause');
  const resume = gm.locator('#gm-session-resume');
  await pause.scrollIntoViewIfNeeded();
  await pause.click();
  await gm.waitForFunction(
    () => window.__hostGmSessionState?.().paused === true,
    undefined,
    { timeout: 30_000 },
  );
  await gm.evaluate(() => { window.__gmStationCorrelatedActions = []; });
  await takeover.scrollIntoViewIfNeeded();
  await takeover.click();
  await frameElement.scrollIntoViewIfNeeded();
  await expect(impulse).toBeEnabled({ timeout: 20_000 });
  await impulse.click();
  await gm.waitForFunction(
    () => window.__gmStationCorrelatedActions
      ?.some(candidate => candidate.semantic_action === 'helm.impulse'),
    undefined,
    { timeout: 15_000 },
  );
  await resume.scrollIntoViewIfNeeded();
  await resume.click();
  await gm.waitForFunction(
    () => window.__hostGmSessionState?.().paused === false,
    undefined,
    { timeout: 30_000 },
  );
  await expect(impulseFeedback).toHaveAttribute('data-state', 'Refused', { timeout: 30_000 });
  await gm.waitForFunction(
    operator => {
      const action = window.__gmStationCorrelatedActions
        ?.find(candidate => candidate.semantic_action === 'helm.impulse');
      return !!action && window.__hostGmStationState().projection.results.some(result =>
        result.operator_id === operator
          && result.correlation === action.correlation
          && result.action_kind === 'station-command'
          && result.outcome === 'refused'
          && result.reason === 'station-not-puppeted');
    },
    operatorId,
    { timeout: 30_000 },
  );

  // The ordinary AI source is restored after release. The crew-public
  // projection clears before a second ownership-preserving reconnect.
  await gm.waitForFunction(
    () => window.__hostGmStationState?.().selectedRow?.station?.rating === 'Backfill',
    undefined,
    { timeout: 30_000 },
  );
  await gm.waitForFunction(
    operator => {
      const row = window.__hostGmStationState?.().selectedRow;
      return row?.station?.rating === 'Backfill'
        && !row.station.operators?.includes(operator)
        && row.ship.control_sources?.['helm-thrust'] === 'Ai';
    },
    operatorId,
    { timeout: 30_000 },
  );
  await captain.page.waitForFunction(
    () => {
      const states = (window.__messages || []).filter(message => message.type === 'SimState');
      const latest = states.at(-1);
      return !!latest
        && !(latest.data.snapshot?.station_puppets || [])
          .some(entry => entry.station === 'helm');
    },
    undefined,
    { timeout: 30_000 },
  );

  const helmAfterRelease = await reconnectRealCrew(context, hostId, helmToken, 'helm');
  const clearedBanner = helmAfterRelease
    .frameLocator('#helm-iframe')
    .locator('#gm-takeover-banner');
  await expect(clearedBanner).toBeHidden({ timeout: 20_000 });
  await expect(helmAfterRelease.locator('#helm-ui')).toHaveClass(/active/);

  // Truth / Crew Knowledge / Difference comparison panel (issue #1318): a
  // presentation-only reuse of the same two Host Channels this test already
  // exercised above (`gm_entity` for Truth, `gm_station` for this ship's own
  // replica), asserted here after Helm release so it never depends on — or
  // interferes with — the takeover control checked earlier. AC #2 (selecting
  // another ship recomputes the view) is pinned with real production
  // fixtures at the pure/DOM-controller level in
  // tests/client/gm-knowledge-compare.test.js — the smoke scenario has a
  // single player ship, so there is no second ship here to switch to. What
  // ONLY this test can prove is that the real WASM wiring, running the real
  // localised/unlocalised Host Channel payloads through the panel, renders a
  // comparison that never leaks a raw Truth String Table id (finding 1) or a
  // double-resolved `⟨...⟩` wrapper (finding 2) into a rendered cell.
  await gm.waitForFunction(() => {
    const panel = document.getElementById('gm-knowledge-panel');
    return !!panel && !panel.hidden && !!window.__hostGmKnowledgeState?.().selectedShipId;
  }, undefined, { timeout: 30_000 });
  const knowledge = await gm.evaluate(() => ({
    selected: window.__hostGmKnowledgeState().selectedShipId,
    shipIds: window.__hostGmKnowledgeState().shipIds,
    pendingHidden: document.getElementById('gm-knowledge-pending').hidden,
  }));
  expect(knowledge.pendingHidden).toBe(true);
  expect(knowledge.shipIds).toContain(knowledge.selected);

  // Wait for at least one real row so the assertions below inspect actual
  // rendered cells rather than an empty table. `publish_sensors_blackboard`
  // (src/ship/sensors.rs) computes every ship's Sensors blackboard regardless
  // of locality, so `default.toml`'s starbase and patrol raider — within this
  // world's default radar range — join the Sensors-contacts category from
  // the first tick. Comms-contacts is NOT the same category to wait on here:
  // `console::comms::server.rs`'s blackboard system clones its one shared
  // `local_bb` onto the process's OWN local ship only and gives every OTHER
  // ship an empty default, and the GM peer observing this fleet ship never
  // has a local ship of its own (AGENTS.md) — real crew aboard the SHIP's own
  // process does not change what the GM's own process computes. So Comms
  // stays the "equal empty" state on a GM's own instance until a per-ship
  // Comms filter (#1063/#1065/#1070) lands; that is a state this comparison
  // must cover; not a bug to route around.
  await gm.waitForFunction(() => {
    const rows = document.getElementById('gm-knowledge-contacts-rows');
    return !!rows && rows.children.length > 0;
  }, undefined, { timeout: 30_000 });
  const knowledgeCells = await gm.evaluate(() => ({
    commsContactTruthTexts: [...document.getElementById('gm-knowledge-comms-contacts-rows').children]
      .map((row) => row.children[1].textContent),
    // Objectives may legitimately be empty at this point in the scenario (no
    // scenario trigger has fired yet) — assert whatever IS rendered never
    // leaks a raw Truth id (finding 1) or a double-resolved wrapper
    // (finding 2); real coverage of both fixes' actual bug lives in the
    // pure/DOM-level tests in tests/client/gm-knowledge-compare.test.js.
    contactTruthTexts: [...document.getElementById('gm-knowledge-contacts-rows').children]
      .map((row) => row.children[1].textContent),
    objectiveCellTexts: [...document.getElementById('gm-knowledge-objectives-rows').children]
      .flatMap((row) => [row.children[1].textContent, row.children[2].textContent]),
  }));
  expect(knowledgeCells.contactTruthTexts.length).toBeGreaterThan(0);
  for (const text of [...knowledgeCells.commsContactTruthTexts, ...knowledgeCells.contactTruthTexts]) {
    expect(text).not.toMatch(/^world\./);
    expect(text).not.toContain('⟨');
  }
  for (const text of knowledgeCells.objectiveCellTexts) {
    expect(text).not.toContain('⟨');
  }

  expect(shipErrors).toEqual([]);
  expect(gmErrors, gmErrorDetails.join('\n')).toEqual([]);
  await helmAfterRelease.close();
  await captain.close();
  await engineering.close();
  await science.close();
});

test(
  'scenario-authored role presets render with String Table copy and filter presentation only',
  { tag: '@core' },
  async ({ context }) => {
    await context.route('**/assets/worlds/default.toml', (route) =>
      route.fulfill({ contentType: 'text/plain', body: GM_ROLE_PRESET_WORLD }),
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

    // Both authored presets render beside the built-in All, with real String
    // Table copy -- not a raw id, and not an unresolved ⟨missing.id⟩
    // placeholder -- proving the TOML -> parse_world -> wasm_get_gm_role_presets
    // -> the real <select> path end to end (acceptance criterion 1).
    const options = await page.locator('#gm-role-preset-select option').evaluateAll(
      (nodes) => nodes.map((node) => ({ value: node.value, text: node.textContent })),
    );
    expect(options).toEqual([
      { value: 'all', text: '[All]' },
      { value: 'tactical', text: '[Tactical]' },
      { value: 'narrative', text: '[Narrative]' },
    ]);

    // The default is All: nothing authored is hidden yet.
    await expect(page.locator('#gm-map-panel')).toBeVisible();
    await expect(page.locator('#gm-inspector')).toBeVisible();
    await expect(page.locator('#gm-session-resume')).toBeVisible();

    // Live-switch to Tactical through the real <select>. gm-inspector and the
    // gm-session-resume quick action are hidden -- neither is in Tactical's
    // authored lists -- while gm-map-panel, which IS authored, stays visible.
    await page.selectOption('#gm-role-preset-select', 'tactical');
    await expect(page.locator('#gm-inspector')).toBeHidden();
    await expect(page.locator('#gm-session-resume')).toBeHidden();
    await expect(page.locator('#gm-map-panel')).toBeVisible();

    // Presentation only: gm-session-pause stays visible and enabled under
    // Tactical (it IS in the authored quick_actions), and clicking it still
    // applies the real GmAction and pauses the authoritative simulation --
    // the preset narrows what renders, never what this GM may do.
    const pause = page.locator('#gm-session-pause');
    await expect(pause).toBeVisible();
    await expect(pause).toBeEnabled();
    await pause.scrollIntoViewIfNeeded();
    await pause.click();
    await page.waitForFunction(
      () => window.__hostGmSessionState?.().paused === true,
      undefined,
      { timeout: 15_000 },
    );
    expect(await page.evaluate(() => window.wasm_is_paused())).toBe(true);

    expect(errors).toEqual([]);
  },
);
