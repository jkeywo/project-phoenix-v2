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

const GM_REMOVAL_WORLD = `
[global]
seed = 1306
title = "GM removal smoke fixture"
description = "Safe entity removal through the real map."
[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "Removable courier"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
# The map reads the GameStart hull's config name; author its displayed identity
# as well as the scenario reference name so these two couriers are distinct.
overrides = { name = "Removable courier", display_name = "Removable courier", tags = ["ship", "gm_removable"] }
[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "Protected courier"
transform = { position = [200.0, 0.0, 0.0] }
spawn_on = "game_start"
overrides = { name = "Protected courier", display_name = "Protected courier" }
[[entity]]
template_path = "assets/entities/station_axiom.toml"
name = "Removable structure"
transform = { position = [-200.0, 0.0, 0.0] }
overrides = { tags = ["station", "gm_removable"] }
[[entity]]
template_path = "assets/entities/region_radiation_zone.toml"
name = "Foundational hazard"
transform = { position = [2000.0, 0.0, 0.0] }
overrides = { tags = ["region", "gm_removable"], shape = { radius = 10.0 } }
[[gm_palette]]
id = "hazard"
label = "server.gm.entity.kind.hazard"
template_path = "assets/entities/region_radiation_zone.toml"
name_prefix = "removable_hazard"
[[gm_palette.variant]]
id = "removable"
label = "server.gm.despawn.heading"
overrides = { tags = ["region", "gm_removable"], shape = { radius = 10.0 } }
`;

test('a GM confirms safe removal from the map and protected targets remain', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(120_000);
  await context.route('**/assets/worlds/default.toml', route => route.fulfill({ contentType: 'text/plain', body: GM_REMOVAL_WORLD }));
  const page = await context.newPage();
  const errors = captureServerPageErrors(page);
  await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await page.evaluate(() => window.__hostFleetOpen());
  await page.waitForFunction(() => {
    const state = window.__hostGmStartState?.();
    return state?.admitted && state.presentationReady && state.localValidation;
  });
  await page.evaluate(() => document.getElementById('gm-ready-btn').click());
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');
  const map = page.locator('#gm-entity-map');
  const select = async (name) => {
    await map.scrollIntoViewIfNeeded(); await map.focus();
    for (let index = 0; index < 20; index++) {
      await map.press('ArrowRight');
      if (await page.evaluate(expected => window.__hostGmDespawnState().selected?.name === expected, name)) return;
    }
    const state = await page.evaluate(() => ({
      tick: window.wasm_sim_tick(),
      blips: document.getElementById('gm-entity-map').state.blips,
      regions: document.getElementById('gm-entity-map').state.regions,
      selected: window.__hostGmDespawnState().selected,
    }));
    throw new Error(`Map keyboard selection did not reach ${name}: ${JSON.stringify(state)}`);
  };
  for (const name of ['Protected courier', 'Foundational hazard']) {
    await select(name);
    await expect(page.locator('#gm-despawn-preview')).toBeDisabled();
    await expect(page.locator('#gm-despawn-confirmation')).toBeHidden();
  }
  await select('Removable courier');
  await page.locator('#gm-despawn-preview').click();
  await expect(page.locator('#gm-despawn-consequence')).toContainText('Removable courier');
  await page.locator('#gm-despawn-cancel').click();
  await expect(page.locator('#gm-despawn-confirmation')).toBeHidden();
  expect(await page.evaluate(() => window.__hostGmDespawnState().results.length)).toBe(0);
  let count = 0;
  for (const name of ['Removable courier', 'Removable structure']) {
    await select(name);
    const id = await page.evaluate(() => window.__hostGmDespawnState().selected.entity_id);
    await page.locator('#gm-despawn-preview').click();
    await page.locator('#gm-despawn-confirm').click();
    await expect(page.locator('#gm-despawn-results li[data-outcome="applied"]')).toHaveCount(++count);
    await page.waitForFunction(uuid => !document.getElementById('gm-entity-map').state.blips.some(b => b.uuid === uuid), id);
    await expect(page.locator('#gm-entity-card')).toBeHidden();
  }
  // A runtime hazard receives normal spawn provenance through the real palette.
  const row = page.locator('#gm-spawn-palette [data-palette-id="hazard"].gm-spawn-entry');
  await row.locator('select').selectOption('removable');
  await row.locator('button[data-role="place"]').click();
  await page.keyboard.press('ArrowRight'); await page.keyboard.press('Enter');
  await expect(page.locator('#gm-spawn-log [data-outcome="applied"]')).toHaveCount(1);
  await page.waitForFunction(() => document.getElementById('gm-entity-map').state.regions.some(r => r.name.startsWith('removable_hazard')));
  const hazardName = await page.evaluate(() => document.getElementById('gm-entity-map').state.regions.find(r => r.name.startsWith('removable_hazard')).name);
  await select(hazardName);
  await expect(page.locator('#gm-despawn-preview')).toBeEnabled();
  await page.locator('#gm-despawn-preview').click(); await page.locator('#gm-despawn-confirm').click();
  await expect(page.locator('#gm-despawn-results li[data-outcome="applied"]')).toHaveCount(3);
  await select('Protected courier');
  await expect(page.locator('#gm-despawn-preview')).toBeDisabled();
  expect(errors).toEqual([]);
});

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

// Issue #1302: an ORDINARY condition-bearing trigger that declares gm_controls,
// beside one that declares nothing. The declared event's condition (the courier
// being destroyed) never occurs in this spec, which is the whole point — the GM
// fires it anyway, and the undeclared trigger never reaches the panel at all.
const GM_AUTOMATIC_EVENT_WORLD = `
[global]
seed = 1302
title = "GM automatic event smoke fixture"
description = "gm_controls on an automatic trigger, coverage for issue 1302."

[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "entity.alliance_courier.display_name"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[script]
setup = """
on_destroyed("entity.alliance_courier.display_name", "on_courier_lost")
    .gm_controls("courier_lost", "world.smoke_gm.event.courier_lost");
on_world_loaded("on_quiet");
fn on_courier_lost(ctx) { ctx.flags.increment("courier_losses", 1); }
fn on_quiet(ctx) { }
"""
`;

// Issue #1304: an ORDINARY condition-bearing trigger declaring BOTH levers, so
// a browser test can press one and watch the other stay available. The courier
// is never destroyed in this spec — the point is the arm itself, which waits on
// an occurrence that never comes, exactly as it does in a real mission.
const GM_SKIPPABLE_EVENT_WORLD = `
[global]
seed = 1304
title = "GM skip-next smoke fixture"
description = "gm_controls().skip() on an automatic trigger, coverage for issue 1304."

[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "entity.alliance_courier.display_name"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[script]
setup = """
on_destroyed("entity.alliance_courier.display_name", "on_raider_lost")
    .gm_controls("raider_lost", "world.smoke_gm.event.raider_lost")
    .repeat()
    .skip();
on_world_loaded("on_quiet");
fn on_raider_lost(ctx) { ctx.flags.increment("raider_losses", 1); }
fn on_quiet(ctx) { }
"""
`;

// One selectable damageable hull and nothing else, so a browser test can name
// the target it aims at without depending on which blip the map happens to
// order first (issue #1310).
const GM_DIRECT_EFFECT_WORLD = `
[global]
seed = 1310
title = "GM direct effect smoke fixture"
description = "Direct Entity damage and healing coverage for issue 1310."

[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "entity.alliance_courier.display_name"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
`;

// The Station display names `assets/entities/alliance_courier.toml` authors
// beside the ids the picker keys on (issue #1311). The GM projection publishes
// them exactly as the station-interface surface does, so the scope picker reads
// as the hull was authored instead of showing the raw authoring key.
const COURIER_STATION_NAMES = { captain: 'Captain', tactical: 'Tactical' };

// Issue #1305: one `[[gm_palette]]` row with one authored variant, plus one
// ordinary entity so the GM-only session has a world to place into. The
// authoritative half is covered by the Rust pipeline tests; what this fixture
// is here for is the browser half — the row's String Table label, the map
// gesture, the keyboard path, and the attributed Applied results.
const GM_PALETTE_WORLD = `
[global]
seed = 1305
title = "GM palette smoke fixture"
description = "Map-gesture palette placement coverage for issue 1305."

[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "entity.alliance_courier.display_name"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[[gm_palette]]
id = "tender"
label = "world.smoke_gm_palette.tender.label"
template_path = "assets/entities/alliance_tender.toml"
name_prefix = "gm_tender"

[[gm_palette.variant]]
id = "escort"
label = "world.smoke_gm_palette.tender.escort.label"
overrides = { radar_appearance = { size = 7.0 } }
`;

// Issue #1303: two ORDINARY condition-bearing triggers, only ONE of which
// declares `.pauseable()`. The pair is the point — a Pause toggle appearing on
// the declared row and nowhere else is what "only an event declaring Pause
// exposes the toggle" looks like in a real browser, and the second row is the
// control that would catch a panel rendering the toggle for everything.
const GM_PAUSABLE_EVENT_WORLD = `
[global]
seed = 1303
title = "GM pausable event smoke fixture"
description = "pauseable() on an automatic trigger, coverage for issue 1303."

[[entity]]
template_path = "assets/entities/alliance_courier.toml"
name = "entity.alliance_courier.display_name"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[script]
setup = """
on_destroyed("entity.alliance_courier.display_name", "on_courier_lost")
    .gm_controls("courier_lost", "world.smoke_gm.event.courier_lost").pauseable();
on_hull_below("entity.alliance_courier.display_name", flt("0.5"), "on_witness")
    .gm_controls("witness", "world.smoke_gm.event.witness");
fn on_courier_lost(ctx) { ctx.flags.increment("courier_losses", 1); }
fn on_witness(ctx) { ctx.flags.increment("witnesses", 1); }
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

/// Issue #1302 exit evidence in a real browser: an ORDINARY authored trigger
/// that declares gm_controls reaches the GM mission panel under its String
/// Table label, a Fire runs it even though its automatic condition has not
/// occurred, and the trigger beside it that declares no controls never appears.
test('an automatic event declaring gm_controls is listed and fireable, and an undeclared one is not', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(90_000);
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_AUTOMATIC_EVENT_WORLD }),
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

  const row = page.locator('#gm-mission-events .gm-mission-event[data-event-id="base-world::courier_lost"]');
  await expect(row).toBeVisible({ timeout: 30_000 });
  await expect(row.locator('.gm-mission-event-label'))
    .toHaveText(ts('world.smoke_gm.event.courier_lost'));
  await expect(row.locator('.gm-mission-event-state'))
    .toHaveText(ts('server.gm.mission.state_ready'));

  // The world authors TWO triggers; only the one declaring gm_controls is
  // addressable, which is what "invisible and unavailable" has to mean.
  expect(await page.evaluate(() => window.__hostGmMissionState())).toMatchObject({
    events: 1,
    fireable: 1,
  });

  // The courier is alive and stays alive: the automatic condition never occurs.
  const fire = row.locator('button[data-role="fire"]');
  await expect(fire).toBeEnabled();
  await fire.click();

  const applied = page.locator('#gm-mission-log .gm-mission-log-entry[data-outcome="applied"]');
  await expect(applied).toHaveCount(1, { timeout: 30_000 });
  await expect(applied).toContainText('base-world::courier_lost');
  await expect(row).toHaveAttribute('data-spent', 'true', { timeout: 30_000 });
  await expect(fire).toBeDisabled();
  expect(await page.evaluate(() => window.__hostGmMissionState())).toMatchObject({
    events: 1,
    fireable: 0,
    pending: 0,
  });

  expect(errors).toEqual([]);
});

/// Issue #1304 exit evidence in a real browser: an authored trigger that
/// declares `.skip()` offers a Skip control beside its Fire, arming it reports
/// Applied against the authoritative projection, a second arm reports the
/// deterministic No-op, and the Fire beside it never stops being available —
/// which is the visible half of "Fire does not consume an armed Skip".
///
/// Nothing here injects a Host Channel payload: every row, every state sentence
/// and every result comes from the running simulation's own projection.
test('a GM arms a Skip of an authored event and a second arm reports the No-op', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(90_000);
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_SKIPPABLE_EVENT_WORLD }),
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

  const row = page.locator('#gm-mission-events .gm-mission-event[data-event-id="base-world::raider_lost"]');
  await expect(row).toBeVisible({ timeout: 30_000 });
  await expect(row.locator('.gm-mission-event-label'))
    .toHaveText(ts('world.smoke_gm.event.raider_lost'));
  await expect(row.locator('.gm-mission-event-skip-state'))
    .toHaveText(ts('server.gm.mission.state_skip_ready'));
  expect(await page.evaluate(() => window.__hostGmMissionState())).toMatchObject({
    events: 1,
    fireable: 1,
    skippable: 1,
    armedSkips: 0,
  });

  const skip = row.locator('button[data-role="skip"]');
  const fire = row.locator('button[data-role="fire"]');
  await expect(skip).toBeEnabled();
  await skip.click();

  // The authoritative projection reports the arm, and the panel says so.
  const applied = page.locator('#gm-mission-log .gm-mission-log-entry[data-outcome="applied"]');
  await expect(applied).toHaveCount(1, { timeout: 30_000 });
  await expect(applied).toContainText('base-world::raider_lost');
  await expect(applied).toHaveAttribute('data-lever', 'skip');
  await expect(row).toHaveAttribute('data-skip-armed', 'true', { timeout: 30_000 });
  await expect(row.locator('.gm-mission-event-skip-state'))
    .toHaveText(ts('server.gm.mission.state_skip_armed'));
  expect(await page.evaluate(() => window.__hostGmMissionState())).toMatchObject({
    armedSkips: 1,
    pending: 0,
  });

  // Fire is untouched by the armed Skip: the two levers are independent.
  await expect(fire).toBeEnabled();

  // A second arm is a deterministic No-op, and the operator can SEE it.
  await expect(skip).toBeEnabled();
  await skip.click();
  const noOp = page.locator('#gm-mission-log .gm-mission-log-entry[data-outcome="no-op"]');
  await expect(noOp).toHaveCount(1, { timeout: 30_000 });
  await expect(noOp).toContainText('base-world::raider_lost');
  await expect(row).toHaveAttribute('data-skip-armed', 'true');

  expect(errors).toEqual([]);
});

// Direct Entity damage and healing (issue #1310). A NEW feature needs a NEW
// @core test (AGENTS.md Testing Strategy). Nothing here injects a Host Channel
// payload: the target, its absolute hull, the overflow preview and every
// result row come from the authoritative projection the running simulation
// publishes.
test('a GM damages and repairs one Entity through the typed action path', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(90_000);
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_DIRECT_EFFECT_WORLD }),
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
  await page.waitForFunction(() => {
    const map = document.getElementById('gm-entity-map');
    return !map?.hidden && (map.state?.blips?.length ?? 0) > 0;
  }, undefined, { timeout: 30_000 });

  // Nothing is aimable until an entity is selected, and the panel says so.
  const panel = page.locator('#gm-effect-panel');
  await expect(panel).toHaveAttribute('data-damageable', 'false');
  await expect(page.locator('#gm-effect-empty')).toHaveText(ts('server.gm.effect.empty'));
  await expect(page.locator('#gm-effect-controls')).toBeHidden();

  // The map selection IS the target picker: no second identity is composed.
  const map = page.locator('#gm-entity-map');
  await map.focus();
  await map.press('ArrowRight');
  await expect(panel).toHaveAttribute('data-damageable', 'true', { timeout: 30_000 });
  await expect(page.locator('#gm-effect-controls')).toBeVisible();
  const targetId = await page.locator('#gm-entity-card').getAttribute('data-entity-id');
  expect(targetId).toMatch(/^[0-9a-f-]{36}$/i);

  const hullBefore = await page.evaluate(() => {
    const surface = document.getElementById('gm-entity-map');
    const id = document.getElementById('gm-entity-card').dataset.entityId;
    return surface.state.blips.find((blip) => blip.uuid === id).hull_percent;
  });
  expect(hullBefore).toBe(100);

  // A modest hit: applied whole, nothing discarded, nothing destroyed.
  await page.locator('#gm-effect-amount').fill('5');
  await expect(page.locator('#gm-effect-damage')).toBeEnabled();
  await page.locator('#gm-effect-damage').click();

  const applied = page.locator('#gm-effect-log .gm-effect-log-entry[data-outcome="applied"]');
  await expect(applied).toHaveCount(1, { timeout: 30_000 });
  await expect(applied).toHaveAttribute('data-entity', targetId);
  await expect(applied).toHaveAttribute('data-effect', 'damage');
  await expect(applied).toHaveAttribute('data-applied', '5000');
  await expect(applied).toHaveAttribute('data-discarded', '0');
  await expect(applied).toHaveAttribute('data-destroyed', 'false');

  // The crew-facing consequence is the ORDINARY damage row, from the same
  // unconditional balance event a beam hit produces.
  await expect(page.locator('#gm-activity-list [data-category="damage"]').first())
    .toBeVisible({ timeout: 30_000 });
  // And the GM's own attributed row names what it hit.
  await expect(
    page.locator('#gm-activity-list [data-category="gm_action"]').filter({ hasText: targetId }),
  ).toHaveCount(1, { timeout: 30_000 });

  await page.waitForFunction((id) => {
    const surface = document.getElementById('gm-entity-map');
    return (surface.state.blips.find((blip) => blip.uuid === id)?.hull_percent ?? 100) < 100;
  }, targetId, { timeout: 30_000 });

  // A repair far beyond the maxima clamps and REPORTS what it threw away.
  await page.locator('#gm-effect-amount').fill('9999');
  await expect(page.locator('#gm-effect-warning')).toHaveAttribute('data-discarded', /\d+/);
  await expect(page.locator('#gm-effect-heal')).toBeEnabled();
  await page.locator('#gm-effect-heal').click();
  const healed = page.locator('#gm-effect-log .gm-effect-log-entry[data-effect="heal"]');
  await expect(healed).toHaveCount(1, { timeout: 30_000 });
  await expect(healed).toHaveAttribute('data-outcome', 'applied');
  expect(Number(await healed.getAttribute('data-discarded'))).toBeGreaterThan(0);

  expect(await page.evaluate(() => window.__hostGmEffectState())).toMatchObject({
    selected: targetId,
    damageable: true,
    pending: 0,
  });

  expect(errors).toEqual([]);
});

// Station- and System-scoped damage and healing (issue #1311). A NEW feature
// needs a NEW @core test (AGENTS.md Testing Strategy), and this is the only
// whole-path evidence that the narrowing survives every layer between the
// picker and the hull: nothing here injects a Host Channel payload, so the
// Station list, the scoped hull readings, the refusals and every result row
// come from the authoritative projection the running simulation publishes.
//
// The Alliance courier is the fixture because its authored ownership already
// has every shape the criteria name: `tactical` owns four damageable Systems
// (112 hull), `captain` owns two (56), and `core` (32) is authored under no
// Station at all — so a Station scope, a System scope and an ownerless System
// are all reachable on one hull.
test('a GM damages and repairs one Station and one System without touching their siblings', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(120_000);
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_DIRECT_EFFECT_WORLD }),
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
  await page.waitForFunction(() => {
    const map = document.getElementById('gm-entity-map');
    return !map?.hidden && (map.state?.blips?.length ?? 0) > 0;
  }, undefined, { timeout: 30_000 });

  const map = page.locator('#gm-entity-map');
  await map.focus();
  await map.press('ArrowRight');
  await expect(page.locator('#gm-effect-panel'))
    .toHaveAttribute('data-damageable', 'true', { timeout: 30_000 });
  const targetId = await page.locator('#gm-entity-card').getAttribute('data-entity-id');

  // The picker is built from the projection's own per-System breakdown: every
  // option here is an authoring key the simulation published, in hull order.
  const scope = page.locator('#gm-effect-scope');
  await expect(scope).toBeEnabled();
  const options = await scope.evaluate((select) => [...select.options].map((o) => o.value));
  expect(options.slice(0, 3)).toEqual(['entity', 'station:tactical', 'station:captain']);
  expect(options).toContain('system:core');
  expect(options).toContain('system:power-reactor');

  const hullLine = page.locator('#gm-effect-hull');
  const hullPercent = () => page.evaluate((id) => {
    const surface = document.getElementById('gm-entity-map');
    return surface.state.blips.find((blip) => blip.uuid === id)?.hull_percent ?? null;
  }, targetId);
  expect(await hullPercent()).toBe(100);

  // A Station scope reads that Station's own totals, not the hull's 200.
  await scope.selectOption('station:captain');
  await expect(hullLine).toHaveText(ts('server.gm.effect.scope_hull', {
    scope: ts('server.gm.effect.scope_station', { name: COURIER_STATION_NAMES.captain }),
    current: '56',
    max: '56',
  }));

  // Far more than that Station can absorb: it empties, it reports the overflow,
  // and it is explicitly NOT lethal, because 144 hull points elsewhere on this
  // ship are still alive.
  await page.locator('#gm-effect-amount').fill('9999');
  const warning = page.locator('#gm-effect-warning');
  await expect(warning).toHaveAttribute('data-emptied', 'true');
  await expect(warning).not.toHaveAttribute('data-lethal', 'true');
  await page.locator('#gm-effect-damage').click();

  const applied = page.locator('#gm-effect-log .gm-effect-log-entry[data-outcome="applied"]');
  await expect(applied).toHaveCount(1, { timeout: 30_000 });
  await expect(applied).toHaveAttribute('data-scope', 'station:captain');
  await expect(applied).toHaveAttribute('data-applied', '56000');
  await expect(applied).toHaveAttribute('data-destroyed', 'false');
  expect(Number(await applied.getAttribute('data-discarded'))).toBeGreaterThan(0);

  // The ship took it and survived it. The exact figures are on the result row
  // above, which the reducer settled at the apply tick; the hull reading here is
  // deliberately directional, because this ship's own repair teams may restore a
  // damaged System at any moment and an exact percent is not a stable fact.
  await page.waitForFunction(
    (id) => {
      const surface = document.getElementById('gm-entity-map');
      const hull = surface.state.blips.find((blip) => blip.uuid === id)?.hull_percent ?? 100;
      return hull < 100 && hull > 0;
    },
    targetId,
    { timeout: 30_000 },
  );

  // THE ACCEPTANCE CRITERION, read straight off the authoritative projection:
  // the sibling Station is untouched, and so is the ownerless System.
  await scope.selectOption('station:tactical');
  await expect(hullLine).toHaveText(ts('server.gm.effect.scope_hull', {
    scope: ts('server.gm.effect.scope_station', { name: COURIER_STATION_NAMES.tactical }),
    current: '112',
    max: '112',
  }));
  await scope.selectOption('system:core');
  await expect(hullLine).toHaveText(ts('server.gm.effect.scope_hull', {
    scope: ts('server.gm.effect.scope_system', { name: ts('system_hull.core.display_name') }),
    current: '32',
    max: '32',
  }));

  // A System scope stops at its own System — no spill to a sibling on the same
  // Station, and none to the rest of the hull.
  await page.locator('#gm-effect-amount').fill('9999');
  await page.locator('#gm-effect-damage').click();
  const coreHit = page.locator(
    '#gm-effect-log .gm-effect-log-entry[data-scope="system:core"]',
  );
  await expect(coreHit).toHaveCount(1, { timeout: 30_000 });
  await expect(coreHit).toHaveAttribute('data-applied', '32000');
  await expect(coreHit).toHaveAttribute('data-destroyed', 'false');
  await scope.selectOption('system:core');
  await expect(hullLine).toHaveText(ts('server.gm.effect.scope_hull', {
    scope: ts('server.gm.effect.scope_system', { name: ts('system_hull.core.display_name') }),
    current: '0',
    max: '32',
  }), { timeout: 30_000 });
  await scope.selectOption('station:tactical');
  await expect(hullLine).toHaveText(ts('server.gm.effect.scope_hull', {
    scope: ts('server.gm.effect.scope_station', { name: COURIER_STATION_NAMES.tactical }),
    current: '112',
    max: '112',
  }));

  // Healing mirrors scope and clamps: the emptied Station refills to exactly
  // its own maximum and reports everything it threw away.
  await scope.selectOption('station:captain');
  await page.locator('#gm-effect-amount').fill('9999');
  await page.locator('#gm-effect-heal').click();
  const healed = page.locator(
    '#gm-effect-log .gm-effect-log-entry[data-effect="heal"][data-scope="station:captain"]',
  );
  await expect(healed).toHaveCount(1, { timeout: 30_000 });
  await expect(healed).toHaveAttribute('data-outcome', 'applied');
  expect(Number(await healed.getAttribute('data-applied'))).toBeGreaterThan(0);
  expect(Number(await healed.getAttribute('data-discarded'))).toBeGreaterThan(0);
  await expect(hullLine).toHaveText(ts('server.gm.effect.scope_hull', {
    scope: ts('server.gm.effect.scope_station', { name: COURIER_STATION_NAMES.captain }),
    current: '56',
    max: '56',
  }), { timeout: 30_000 });

  // A scope this hull does not author is REFUSED at the apply tick, with the
  // reason spelled from the shared String Table rather than a wire token. The
  // picker cannot offer one, so the request is composed at the page seam — the
  // shape a stale page or a replayed request takes.
  const refusals = await page.evaluate((entity) => {
    const send = (scopeKind, scopeId) => window.__hostApplyDirectEffect({
      entity,
      effect: 'damage',
      amount_milli_hp: 1_000,
      correlation: `scope-refusal-${scopeKind}`,
      scope: scopeKind,
      scope_id: scopeId,
    });
    return {
      station: send('station', 'engineering'),
      system: send('system', 'warp-core'),
      // A narrowed scope with no id is not a request this seam will compose at
      // all: it fails closed in the browser rather than reaching the journal.
      malformed: window.__hostApplyDirectEffect({
        entity,
        effect: 'damage',
        amount_milli_hp: 1_000,
        correlation: 'scope-refusal-malformed',
        scope: 'station',
        scope_id: null,
      }),
    };
  }, targetId);
  expect(refusals).toEqual({ station: true, system: true, malformed: false });

  const unknownStation = page.locator(
    '#gm-effect-log .gm-effect-log-entry[data-reason="unknown-station"]',
  );
  await expect(unknownStation).toHaveCount(1, { timeout: 30_000 });
  await expect(unknownStation).toContainText(ts('server.gm.session.reason.unknown_station'));
  const unknownSystem = page.locator(
    '#gm-effect-log .gm-effect-log-entry[data-reason="unknown-system"]',
  );
  await expect(unknownSystem).toHaveCount(1, { timeout: 30_000 });
  await expect(unknownSystem).toContainText(ts('server.gm.session.reason.unknown_system'));

  // Neither refusal touched anything: no result row claims to have applied one,
  // which is what "never affects a System outside its authored ownership" means
  // for a scope that owns nothing at all.
  await expect(page.locator(
    '#gm-effect-log .gm-effect-log-entry[data-scope="station:engineering"][data-outcome="applied"]',
  )).toHaveCount(0);
  await expect(page.locator(
    '#gm-effect-log .gm-effect-log-entry[data-scope="system:warp-core"][data-outcome="applied"]',
  )).toHaveCount(0);

  // And the GM activity feed names the Station the applied hit was aimed at.
  await expect(
    page.locator('#gm-activity-list [data-category="gm_action"]')
      .filter({ hasText: ts('server.gm.activity.action.direct_effect_station', { scope: 'captain' }) })
      .first(),
  ).toBeVisible({ timeout: 30_000 });

  expect(errors).toEqual([]);
});

/// Issue #1305 exit evidence in a real browser: an authored `[[gm_palette]]`
/// reaches the GM placement panel with its String Table label, a real map
/// press-and-drag places one hull at resolved world coordinates, and the
/// keyboard path — no pointer at all — places a second. Both come back as
/// attributed Applied results, and the omniscient map grows by exactly the two
/// hulls that were placed.
test('a GM places palette entries by map drag and by keyboard alone', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(120_000);
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_PALETTE_WORLD }),
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

  // The authored palette reaches the panel from the authoritative projection —
  // nothing in this spec injects a Host Channel payload.
  const row = page.locator('#gm-spawn-palette .gm-spawn-entry[data-palette-id="tender"]');
  await expect(row).toBeVisible({ timeout: 30_000 });
  await expect(row.locator('.gm-spawn-entry-label'))
    .toHaveText(ts('world.smoke_gm_palette.tender.label'));
  await expect(page.locator('#gm-spawn-empty')).toBeHidden();
  // Only what the scenario authored is offered: one entry, one variant beside
  // the bare template. There is no control anywhere that names an asset path.
  expect(await page.evaluate(() => window.__hostGmSpawnState())).toMatchObject({ palette: 1 });
  await expect(row.locator('.gm-spawn-entry-variant option'))
    .toHaveText([ts('server.gm.spawn.variant_none'), ts('world.smoke_gm_palette.tender.escort.label')]);

  const before = await page.evaluate(
    () => document.getElementById('gm-entity-map').state.blips.length,
  );

  // 1. The map gesture: press picks the position, drag picks the heading.
  await row.locator('button[data-role="place"]').click();
  await expect(page.locator('#gm-entity-map')).toHaveAttribute('data-placement-armed', '');
  // Arming brings the chart to the operator. This console is one long
  // scrolling page whose placement panel sits above the workspace, so on this
  // viewport the chart is entirely below the fold when PLACE is pressed —
  // and a gesture aimed at a chart that is not on screen lands on whatever
  // is, and reports nothing at all. The chart also takes the focus, which is
  // what makes the keyboard path below reachable without a pointer.
  const chart = await page.locator('#gm-entity-map canvas').boundingBox();
  const viewport = page.viewportSize();
  expect(chart.y).toBeGreaterThanOrEqual(0);
  expect(chart.y + chart.height).toBeLessThanOrEqual(viewport.height);
  await expect(page.locator('#gm-entity-map')).toBeFocused();
  await page.mouse.move(chart.x + chart.width * 0.65, chart.y + chart.height * 0.4);
  await page.mouse.down();
  await page.mouse.move(chart.x + chart.width * 0.65, chart.y + chart.height * 0.2, { steps: 4 });
  await page.mouse.up();

  const applied = page.locator('#gm-spawn-log .gm-spawn-log-entry[data-outcome="applied"]');
  await expect(applied).toHaveCount(1, { timeout: 30_000 });
  await expect(applied.first()).toHaveAttribute('data-palette', 'tender');
  await expect(applied.first()).toContainText(ts('world.smoke_gm_palette.tender.label'));

  // 2. The accessible non-drag path: the chart's own keyboard cursor. Nothing
  // here focuses the chart — arming did, so pressing PLACE is the whole
  // pointer involvement, and a keyboard operator reaches the same placement.
  await row.locator('button[data-role="place"]').click();
  await expect(page.locator('#gm-entity-map')).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press(']');
  await page.keyboard.press('Enter');
  await expect(applied).toHaveCount(2, { timeout: 30_000 });

  // Both placements are real hulls in the omniscient world, not just rows.
  await page.waitForFunction(
    (expected) => document.getElementById('gm-entity-map').state.blips.length >= expected,
    before + 2,
    { timeout: 30_000 },
  );
  expect(await page.evaluate(() => window.__hostGmSpawnState()))
    .toMatchObject({ arming: null, pending: 0 });

  expect(errors).toEqual([]);
});

/// Issue #1303 exit evidence in a real browser: the Pause toggle appears only
/// on the event that declares it, a press crosses the typed GM action path and
/// comes back Applied under the PAUSE verb rather than as a fire, the row flips
/// to Resume, Fire keeps working while the event is paused, and Resume puts it
/// back.
test('a pausable authored event toggles end to end and an undeclared one has no toggle', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(90_000);
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_PAUSABLE_EVENT_WORLD }),
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

  const row = page.locator('#gm-mission-events .gm-mission-event[data-event-id="base-world::courier_lost"]');
  const witness = page.locator('#gm-mission-events .gm-mission-event[data-event-id="base-world::witness"]');
  await expect(row).toBeVisible({ timeout: 30_000 });
  await expect(witness).toBeVisible();

  // The absent-control refusal as an operator meets it: the event beside this
  // one is equally GM-operable and equally automatic, and simply has no toggle.
  const toggle = row.locator('button[data-role="pause"]');
  await expect(toggle).toBeEnabled();
  await expect(toggle).toHaveText(ts('server.gm.mission.pause'));
  await expect(witness.locator('button[data-role="pause"]')).toHaveCount(0);
  expect(await page.evaluate(() => window.__hostGmMissionState())).toMatchObject({
    events: 2,
    pausable: 1,
    paused: 0,
  });

  await toggle.click();
  const applied = page.locator('#gm-mission-log .gm-mission-log-entry[data-outcome="applied"]');
  await expect(applied).toHaveCount(1, { timeout: 30_000 });
  await expect(applied).toContainText('base-world::courier_lost');
  // The verb, not just the event: a result folded on the action KIND alone
  // would say "fired" here, because Pause and Fire share one kind.
  await expect(applied).toHaveAttribute('data-verb', 'pause');
  await expect(applied).toContainText(ts('server.gm.mission.verb_pause'));
  await expect(row).toHaveAttribute('data-paused', 'true', { timeout: 30_000 });
  await expect(row.locator('.gm-mission-event-state'))
    .toHaveText(ts('server.gm.mission.state_paused'));
  await expect(toggle).toHaveText(ts('server.gm.mission.resume'));
  expect(await page.evaluate(() => window.__hostGmMissionState())).toMatchObject({
    paused: 1,
    pending: 0,
  });

  // Fire is independent of Pause, which is why both levers exist: a GM stops
  // the world choosing the moment so they can choose it themselves.
  const fire = row.locator('button[data-role="fire"]');
  await expect(fire).toBeEnabled();
  await fire.click();
  await expect(applied).toHaveCount(2, { timeout: 30_000 });
  await expect(row).toHaveAttribute('data-spent', 'true', { timeout: 30_000 });
  await expect(row).toHaveAttribute('data-paused', 'true');

  // And the toggle goes back, on the same absolute terms.
  await toggle.click();
  await expect(applied).toHaveCount(3, { timeout: 30_000 });
  await expect(row).toHaveAttribute('data-paused', 'false', { timeout: 30_000 });
  await expect(toggle).toHaveText(ts('server.gm.mission.pause'));

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
