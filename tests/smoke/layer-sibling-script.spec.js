// Browser/WASM regression for issue #1045. A fetched layer may declare a
// sibling Rhai unit; the host must retain the already-fetched TOML while that
// second request is in flight, then activate both halves atomically.

import {
  test,
  expect,
  MINIMAL_DEFAULT_WORLD,
  createServerPage,
  createTestClient,
  readHostPeerId,
} from './fixtures';

const ROOT_WORLD = MINIMAL_DEFAULT_WORLD.replace(
  /\[script\][\s\S]*$/,
  `[script]
setup = """
on_world_loaded("load_smoke_layer");

fn load_smoke_layer(ctx) {
    ctx.effects.load_world("assets/worlds/smoke_layer.toml");
}
"""
`,
);

const LAYER_WORLD = `
script = "smoke_layer.rhai"

[global]
seed = 1045
title = "Sibling script smoke layer"
description = "Fetched layer fixture for issue 1045."
`;

const LAYER_SCRIPT = `
on_world_loaded("publish_layer_objective");

fn publish_layer_objective(ctx) {
    ctx.effects.add_objective(#{
        id: "obj-layer-sibling-fired",
        text: "world.smoke.layer_sibling.objective.text",
        mandatory: true
    });
}
`;

const rootLoading = (path) => MINIMAL_DEFAULT_WORLD.replace(
  /\[script\][\s\S]*$/,
  `[script]
setup = """
on_world_loaded("load_smoke_layer");

fn load_smoke_layer(ctx) {
    ctx.effects.load_world("${path}");
}
"""
`,
);

const ENTITY_LAYER = (script, name) => `
script = "${script}"

[global]
seed = 1045
title = "Sibling script edge layer"
description = "Fetched layer fixture for issue 1045."

[[entity]]
template_path = "assets/entities/station_axiom.toml"
name = "${name}"
transform = { position = [750.0, 0.0, 0.0] }
`;

async function startSoloCaptain(context) {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);
  const captain = await createTestClient(context, hostId, { name: 'Captain' });

  await captain.send('SelectStation', { station: 'Captain' });
  await captain.page.waitForFunction(
    (token) => window.__messages?.some(
      (message) => message.type === 'StationAssigned' && message.data.token === token,
    ),
    captain.token,
    { timeout: 8_000 },
  );
  await captain.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 10_000);
  return { serverPage, captain };
}

test('fetched layer requests its sibling Rhai once and applies its authoritative effect', async ({ context }) => {
  let layerRequests = 0;
  let scriptRequests = 0;

  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: ROOT_WORLD }),
  );
  await context.route('**/assets/worlds/smoke_layer.toml', (route) => {
    layerRequests += 1;
    return route.fulfill({ contentType: 'text/plain', body: LAYER_WORLD });
  });
  await context.route('**/assets/worlds/smoke_layer.rhai', (route) => {
    scriptRequests += 1;
    return route.fulfill({ contentType: 'text/plain', body: LAYER_SCRIPT });
  });

  const { captain } = await startSoloCaptain(context);
  await captain.page.waitForFunction(
    () => window.__messages?.some(
      (message) => message.type === 'ObjectiveSummary'
        && message.data?.objectives?.some(
          (objective) => objective.id === 'obj-layer-sibling-fired',
        ),
    ),
    { timeout: 10_000 },
  );

  expect(layerRequests).toBe(1);
  expect(scriptRequests).toBe(1);

  await captain.close();
});

test('an empty fetched sibling is Ready content and its layer activates', async ({ context }) => {
  const layerPath = 'assets/worlds/smoke_empty_sibling_layer.toml';
  const scriptPath = 'assets/worlds/smoke_empty_sibling_layer.rhai';
  const witness = 'Empty Sibling Activation Witness';
  let layerRequests = 0;
  let scriptRequests = 0;

  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: rootLoading(layerPath) }),
  );
  await context.route(`**/${layerPath}`, (route) => {
    layerRequests += 1;
    return route.fulfill({
      contentType: 'text/plain',
      body: ENTITY_LAYER('smoke_empty_sibling_layer.rhai', witness),
    });
  });
  await context.route(`**/${scriptPath}`, (route) => {
    scriptRequests += 1;
    return route.fulfill({ contentType: 'text/plain', body: '' });
  });

  const { captain } = await startSoloCaptain(context);
  await captain.page.waitForFunction(
    (entityName) => window.__messages?.some(
      (message) => message.type === 'EntitySpawned'
        && message.data?.snapshot?.name === entityName,
    ),
    witness,
    { timeout: 10_000 },
  );

  expect(layerRequests).toBe(1);
  expect(scriptRequests).toBe(1);
  await captain.close();
});

test('a failed sibling fetch refuses the whole layer once with no activated content', async ({ context }) => {
  const layerPath = 'assets/worlds/smoke_failed_sibling_layer.toml';
  const scriptPath = 'assets/worlds/smoke_failed_sibling_layer.rhai';
  const witness = 'Failed Sibling Must Not Spawn';
  let layerRequests = 0;
  let scriptRequests = 0;

  const failedLayer = `${ENTITY_LAYER('smoke_failed_sibling_layer.rhai', witness)}
[[deadline]]
id = "failed-sibling-deadline"
label = "world.smoke.failed_sibling.deadline"
due_secs = 0
visible = true
`;
  const wouldHaveBeenScript = `
on_world_loaded("failed_sibling_handler");
on_deadline("failed-sibling-deadline", "failed_sibling_deadline");
fn failed_sibling_handler(ctx) {
    ctx.effects.add_objective(#{ id: "failed-sibling-handler-ran", text: "forbidden" });
}
fn failed_sibling_deadline(ctx) {
    ctx.effects.add_objective(#{ id: "failed-sibling-deadline-ran", text: "forbidden" });
}
`;

  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: rootLoading(layerPath) }),
  );
  await context.route(`**/${layerPath}`, (route) => {
    layerRequests += 1;
    return route.fulfill({ contentType: 'text/plain', body: failedLayer });
  });
  await context.route(`**/${scriptPath}`, (route) => {
    scriptRequests += 1;
    return route.fulfill({
      status: 503,
      contentType: 'text/plain',
      body: wouldHaveBeenScript,
    });
  });

  const { serverPage, captain } = await startSoloCaptain(context);
  await expect.poll(() => scriptRequests, { timeout: 10_000 }).toBe(1);
  await serverPage.waitForTimeout(1_000);

  expect(layerRequests).toBe(1);
  expect(scriptRequests, 'a terminal failure must not retry-spin').toBe(1);
  const forbidden = await captain.page.evaluate((entityName) => ({
    entity: window.__messages?.some(
      (message) => message.type === 'EntitySpawned'
        && message.data?.snapshot?.name === entityName,
    ),
    objective: window.__messages?.some(
      (message) => message.type === 'ObjectiveSummary'
        && message.data?.objectives?.some(
          (objective) => objective.id === 'failed-sibling-handler-ran'
            || objective.id === 'failed-sibling-deadline-ran',
        ),
    ),
  }), witness);
  expect(forbidden).toEqual({ entity: false, objective: false });

  await serverPage.evaluate(() => window.wasm_toggle_scenario_state());
  await serverPage.waitForFunction(() => {
    try {
      const payload = JSON.parse(window.wasm_get_scenario_state());
      return Array.isArray(payload?.triggers) && Array.isArray(payload?.deadlines);
    } catch {
      return false;
    }
  }, { timeout: 5_000 });
  const scenarioState = await serverPage.evaluate(
    () => JSON.parse(window.wasm_get_scenario_state()),
  );
  expect(scenarioState.triggers).toHaveLength(1); // root loader only
  expect(scenarioState.deadlines).toHaveLength(0);

  await captain.close();
});

test('a fetched layer and sibling reload from the session cache without duplicate activation', async ({ context }) => {
  const layerPath = 'assets/worlds/smoke_reload_sibling_layer.toml';
  const scriptPath = 'assets/worlds/smoke_reload_sibling_layer.rhai';
  const witness = 'Sibling Reload Script Witness';
  let layerRequests = 0;
  let scriptRequests = 0;

  const cyclingRoot = MINIMAL_DEFAULT_WORLD.replace(
    /\[script\][\s\S]*$/,
    `[script]
setup = """
on_world_loaded("cycle_smoke_layer");

fn cycle_smoke_layer(ctx) {
    ctx.effects.load_world("${layerPath}");
    ctx.schedule.after(1, |ctx| {
        ctx.effects.unload_world("${layerPath}");
        ctx.schedule.after(1, |ctx| {
            ctx.effects.load_world("${layerPath}");
        });
    });
}
"""
`,
  );
  const reloadLayer = `
script = "smoke_reload_sibling_layer.rhai"

[global]
seed = 1045
title = "Sibling reload smoke layer"
description = "Durable fetched-source fixture for issue 1045."
`;
  const reloadScript = `
on_world_loaded("spawn_reload_witness");

fn spawn_reload_witness(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/station_axiom.toml",
        name: "${witness}",
        position: [800, 0, 0]
    });
}
`;

  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: cyclingRoot }),
  );
  await context.route(`**/${layerPath}`, (route) => {
    layerRequests += 1;
    return route.fulfill({ contentType: 'text/plain', body: reloadLayer });
  });
  await context.route(`**/${scriptPath}`, (route) => {
    scriptRequests += 1;
    return route.fulfill({ contentType: 'text/plain', body: reloadScript });
  });

  const { serverPage, captain } = await startSoloCaptain(context);
  await captain.page.waitForFunction(
    (entityName) => window.__messages?.filter(
      (message) => message.type === 'EntitySpawned'
        && message.data?.snapshot?.name === entityName,
    ).length === 2,
    witness,
    { timeout: 12_000 },
  );

  // A retained duplicate handler would fire again after the reload. Give the
  // fixed-tick pipeline time to expose that before pinning the exact count.
  await captain.page.waitForTimeout(500);
  const witnessSpawns = await captain.page.evaluate((entityName) => (
    window.__messages?.filter(
      (message) => message.type === 'EntitySpawned'
        && message.data?.snapshot?.name === entityName,
    ).length ?? 0
  ), witness);
  expect(witnessSpawns).toBe(2);

  await serverPage.evaluate(() => window.wasm_toggle_scenario_state());
  await serverPage.waitForFunction(() => {
    try {
      const payload = JSON.parse(window.wasm_get_scenario_state());
      return Array.isArray(payload?.triggers) && payload.triggers.length === 2;
    } catch {
      return false;
    }
  }, { timeout: 5_000 });
  const scenarioState = await serverPage.evaluate(
    () => JSON.parse(window.wasm_get_scenario_state()),
  );
  expect(scenarioState.triggers).toHaveLength(2); // root cycle + one live layer handler
  expect(layerRequests).toBe(1);
  expect(scriptRequests).toBe(1);

  await captain.close();
});
