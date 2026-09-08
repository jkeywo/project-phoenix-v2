// Issue #1300 — the retained M1 peer-tracer exit gate.
//
// This is intentionally one whole-path browser test rather than another set of
// isolated fixtures. Every simulation page runs the shipped WASM build, every
// crew command enters through the shipped phone client, and every host-mesh
// frame crosses the real JavaScript/Rust boundary. The only stand-ins are the
// smoke tier's transport and deterministic authored world, because CI has no
// WebRTC service and production scenario tuning is not a stable test contract.

import fs from 'fs';
import path from 'path';
import {
  captureServerPageErrors,
  createTestClient,
  expect,
  expectFixtureWorld,
  readHostPeerId,
  test,
  waitForWasmReady,
} from './fixtures';

const GM_IDENTITY_KEY = 'phoenix.fleet.gm-identity.v1';
const REPORT_PATH = path.resolve(__dirname, '../../target/gm-m1-exit/report.json');

const FIELD_PATH = 'assets/entities/smoke_gm_m1_field.toml';
const ORDINARY_ROCK_PATH = 'assets/entities/smoke_gm_m1_ordinary_asteroid.toml';
const AUTHORED_ROCK_PATH = 'assets/entities/smoke_gm_m1_authored_asteroid.toml';
const REGION_PATH = 'assets/entities/smoke_gm_m1_region.toml';
const STRUCTURE_PATH = 'assets/entities/smoke_gm_m1_structure.toml';

const GM_M1_WORLD = `
[global]
seed = 1300
title = "GM M1 peer tracer"
description = "Deterministic whole-path M1 browser exit gate."
sim_tick_hz = 30
gm_activity_history_depth = 128

[ambient_light]
color = [0.6, 0.55, 0.5]
brightness = 300.0

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[[entity]]
template_path = "assets/entities/alliance_courier.toml"
id = "map-npc"
name = "gm_m1_map_npc"
display_name = "entity.alliance_courier.display_name"
transform = { position = [420.0, 0.0, 0.0] }
spawn_on = "game_start"

[[entity]]
template_path = "assets/entities/alliance_courier.toml"
id = "damage-npc"
name = "gm_m1_damage_npc"
display_name = "entity.alliance_courier.display_name"
transform = { position = [300.0, 0.0, 300.0] }
spawn_on = "game_start"

[[entity]]
template_path = "assets/entities/region_radiation_zone.toml"
id = "gm-m1-hazard"
name = "entity.region_radiation_zone.name"
transform = { position = [300.0, 0.0, 300.0] }
overrides = { shape = { radius = 140.0 }, effects = { damage_zone = { damage_per_second = 6000.0, shield_pierce = 1.0 } } }
spawn_on = "game_start"

[[entity]]
template_path = "${REGION_PATH}"
id = "gm-m1-region"
name = "entity.region_nebula.name"
transform = { position = [-320.0, 0.0, 0.0] }
spawn_on = "game_start"

[[entity]]
template_path = "${STRUCTURE_PATH}"
id = "gm-m1-structure"
name = "Starbase Alpha"
transform = { position = [0.0, 0.0, 520.0] }
spawn_on = "game_start"

[[entity]]
template_path = "${FIELD_PATH}"
id = "gm-m1-field"
name = "server.gm.entity.kind.asteroid_field"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"

[[entity]]
template_path = "${AUTHORED_ROCK_PATH}"
id = "gm-m1-authored-rock"
name = "gm_m1_authored_rock"
display_name = "entity.asteroid.name"
transform = { position = [120.0, 0.0, -80.0] }
spawn_on = "game_start"

[script]
setup = """
on_timer(0, "gm_m1_started");

fn gm_m1_started(ctx) {
    ctx.effects.add_objective(#{
        id: "obj-gm-m1",
        text: "Complete the GM M1 peer tracer.",
        mandatory: false
    });
}
"""
`;

const GM_M1_FIELD = `
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
asteroid_type_paths = ["${ORDINARY_ROCK_PATH}"]
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

const GM_M1_ORDINARY_ROCK = `
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

const GM_M1_AUTHORED_ROCK = `
name = "entity.asteroid.name"
tags = ["asteroid"]

[radar_appearance]
icon = "asteroid"
colour = [0.7, 0.62, 0.45]
size = 8.0
`;

const GM_M1_REGION = `
name = "entity.region_nebula.name"
tags = ["region"]

[radar_appearance]
region_colour = [0.2, 0.7, 0.8]

[shape]
type = "sphere"
radius = 90.0
`;

const GM_M1_STRUCTURE = `
name = "Starbase Alpha"
tags = ["structure"]

[radar_appearance]
icon = "station"
colour = [0.3, 0.8, 0.6]
size = 18.0
`;

const REQUIRED_FEED_CATEGORIES = [
  'damage',
  'destruction',
  'objective',
  'trigger',
  'red_alert',
  'connection',
  'gm_action',
];

// The crew join code's authored shape (issue #1353): its length and alphabet
// come from assets/join/join-codes.json, the same table the host mints from, so
// this wait cannot drift from the designer's table the way a pinned "five
// letters" did. Kept as a regex SOURCE because it crosses into the page.
const JOIN_CODE_SUFFIX = JSON.parse(
  fs.readFileSync(path.resolve(__dirname, '../../assets/join/join-codes.json'), 'utf8'),
).suffix;
const JOIN_CODE_RE_SOURCE = `^[${JOIN_CODE_SUFFIX.alphabet}]{${JOIN_CODE_SUFFIX.length}}$`;

async function installScenario(context) {
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_M1_WORLD }));
  await context.route(`**/${FIELD_PATH}`, (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_M1_FIELD }));
  await context.route(`**/${ORDINARY_ROCK_PATH}`, (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_M1_ORDINARY_ROCK }));
  await context.route(`**/${AUTHORED_ROCK_PATH}`, (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_M1_AUTHORED_ROCK }));
  await context.route(`**/${REGION_PATH}`, (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_M1_REGION }));
  await context.route(`**/${STRUCTURE_PATH}`, (route) =>
    route.fulfill({ contentType: 'text/plain', body: GM_M1_STRUCTURE }));
}

async function bootHost(context) {
  const page = await context.newPage();
  await page.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await page.waitForFunction(
    (source) => new RegExp(source).test(document.getElementById('join-code')?.textContent ?? ''),
    JOIN_CODE_RE_SOURCE,
    { timeout: 30_000 },
  );
  return page;
}

async function openFleetTab(page) {
  await page.bringToFront();
  if (!await page.locator('#server-settings-overlay').isVisible()) {
    await page.click('#server-settings-btn');
  }
  await page.click('.server-settings-tab[data-tab="gameplay"]');
  await page.waitForSelector('[data-control="fleet-code"]', { state: 'attached' });
}

async function closeSettings(page) {
  if (await page.locator('#server-settings-overlay').isVisible()) {
    await page.keyboard.press('Escape');
    await expect(page.locator('#server-settings-overlay')).toBeHidden();
  }
}

async function selectGmProfile(page) {
  await openFleetTab(page);
  await Promise.all([
    page.waitForURL((url) => url.searchParams.get('gm') === '1'),
    page.click('[data-control="fleet-role-gm"]'),
  ]);
  await waitForWasmReady(page);
  await openFleetTab(page);
}

async function prepareGm(context, { clearIdentity }) {
  const page = await bootHost(context);
  if (clearIdentity) {
    await page.evaluate((key) => localStorage.removeItem(key), GM_IDENTITY_KEY);
  }
  await selectGmProfile(page);
  return page;
}

async function openFleet(page) {
  await openFleetTab(page);
  await page.click('[data-control="fleet-open"]');
  await page.waitForFunction(
    (source) => new RegExp(source).test(document.getElementById('fleet-code')?.textContent ?? ''),
    JOIN_CODE_RE_SOURCE,
    { timeout: 30_000 },
  );
  const code = await page.locator('#fleet-code').textContent();
  await closeSettings(page);
  return code;
}

async function joinGm(page, code, { waitForAdmission }) {
  await page.fill('[data-control="fleet-code"]', code);
  await page.click('[data-control="fleet-join"]');
  if (waitForAdmission) {
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
    await closeSettings(page);
  }
}

async function selectAndWait(client, station) {
  await client.send('SelectStation', { station });
  await client.page.waitForFunction(
    ({ token, expected }) => window.__messages?.some(
      (message) => message.type === 'StationAssigned'
        && message.data.token === token
        && message.data.station?.toLowerCase() === expected.toLowerCase(),
    ),
    { token: client.token, expected: station },
    { timeout: 15_000 },
  );
}

async function reconnectCrew(context, hostId, token, station) {
  const page = await context.newPage();
  await page.addInitScript((sessionToken) => {
    sessionStorage.setItem('session-token', sessionToken);
  }, token);
  await page.goto(`/client/index.html#${hostId}`);
  await page.waitForFunction(
    ({ sessionToken, expectedStation }) => {
      const player = window.lobbyState?.players?.find(
        (candidate) => candidate.token === sessionToken,
      );
      return window.lobbyState?.phase === 'InProgress'
        && player?.station?.toLowerCase() === expectedStation;
    },
    { sessionToken: token, expectedStation: station.toLowerCase() },
    { timeout: 30_000 },
  );
  await expect(page.locator('#ready-btn')).toBeVisible({ timeout: 15_000 });
  await page.locator('#ready-btn').click();
  await expect(page.locator(`#${station.toLowerCase()}-ui`)).toHaveClass(/active/, {
    timeout: 30_000,
  });
  return page;
}

function clickGmControl(page, id) {
  return page.locator(`#${id}`).click();
}

function readStatusEvidence(page, id) {
  return page.evaluate((nodeId) => {
    const node = document.getElementById(nodeId);
    return {
      id: nodeId,
      text: node?.textContent?.trim() ?? '',
      visible: !!node && node.getClientRects().length > 0,
      role: node?.getAttribute('role') ?? null,
      live: node?.getAttribute('aria-live') ?? null,
      atomic: node?.getAttribute('aria-atomic') ?? null,
      paused: node?.dataset?.paused ?? null,
    };
  }, id);
}

function redactDiagnostic(value, secrets = []) {
  let text = String(value ?? '');
  for (const secret of secrets) {
    if (typeof secret === 'string' && secret.length > 0) {
      text = text.replaceAll(secret, '[redacted private capability]');
    }
  }
  return text
    .replace(/reconnect_?credential/gi, '[redacted private capability field]')
    .replace(/portable[-_ ]record[-_ ]slice/gi, '[redacted snapshot payload]');
}

function redactArtifact(value, secrets = []) {
  if (typeof value === 'string') return redactDiagnostic(value, secrets);
  if (Array.isArray(value)) return value.map((entry) => redactArtifact(entry, secrets));
  if (!value || typeof value !== 'object') return value;
  return Object.fromEntries(Object.entries(value).flatMap(([key, entry]) => {
    if (/reconnect_?credential/i.test(key) || /portable[-_ ]record[-_ ]slice/i.test(key)) {
      return [];
    }
    return [[key, redactArtifact(entry, secrets)]];
  }));
}

// Observe the real mesh seam without changing a byte passed to or returned by
// WASM. Snapshot chunks retain only framing metadata and byte counts; their RON
// text is deliberately never copied into the test trace or retained report.
async function installMeshTrace(page, label) {
  await page.evaluate((traceLabel) => {
    const trace = { label: traceLabel, frames: [], joinStatuses: [] };
    const rememberFrame = (frame, direction, authenticatedSlot = null) => {
      if (!frame || typeof frame !== 'object') return;
      const body = frame.d && typeof frame.d === 'object' ? frame.d : {};
      const row = {
        direction,
        authenticatedSlot,
        protocol: frame.m ?? null,
        type: frame.t ?? null,
        tick: frame.tick ?? body.tick ?? null,
        from: body.from ?? null,
      };
      if (frame.t === 'snapshot') {
        Object.assign(row, {
          transferId: body.transfer_id ?? null,
          sequence: body.seq ?? null,
          total: body.total ?? null,
          wholeHash: body.whole_hash ?? null,
          crc: body.crc ?? null,
          bytes: typeof body.text === 'string' ? body.text.length : 0,
        });
      } else if (frame.t === 'digest') {
        row.digest = body.digest ?? null;
      } else if (frame.t === 'gm-join') {
        Object.assign(row, {
          kind: body.kind ?? null,
          joinKind: body.join_kind ?? null,
          joinId: body.join_id ?? null,
          candidate: body.candidate ?? null,
          operatorId: body.operator_id ?? null,
          applyTick: body.apply_tick ?? null,
          digest: body.digest ?? null,
        });
      } else if (frame.t === 'gm-action') {
        Object.assign(row, {
          kind: body.kind ?? null,
          operatorId: body.operator_id ?? null,
          correlation: body.correlation ?? null,
          action: body.action ?? body.action_kind ?? null,
          outcomeReason: body.reason ?? null,
        });
      } else if (frame.t === 'tick') {
        const commands = Array.isArray(body.commands) ? body.commands : [];
        if (commands.length === 0 && body.start_grant == null) return;
        row.startGrant = body.start_grant == null ? null : {
          applyTick: body.start_grant.apply_tick ?? null,
          operatorId: body.start_grant.operator_id ?? null,
          kind: body.start_grant.kind ?? null,
        };
        row.commands = commands.map((command) => ({
          tick: command.tick ?? null,
          origin: command.origin ?? null,
          sequence: command.seq ?? null,
          ship: command.ship ?? null,
          target: command.target ?? null,
          payload: command.payload ?? null,
        }));
      } else {
        return;
      }
      trace.frames.push(row);
    };

    const receive = window.wasm_receive_mesh_frame;
    window.wasm_receive_mesh_frame = (slot, raw) => {
      try { rememberFrame(JSON.parse(raw), 'inbound', Number(slot)); } catch (_) {}
      return receive(slot, raw);
    };

    const take = window.wasm_take_mesh_frames;
    window.wasm_take_mesh_frames = (...args) => {
      const raw = take(...args);
      try {
        const frames = JSON.parse(raw);
        if (Array.isArray(frames)) {
          for (const frame of frames) rememberFrame(frame, 'outbound');
        }
      } catch (_) {}
      return raw;
    };

    const joinStatus = window.wasm_gm_join_status;
    let priorStatus = null;
    window.wasm_gm_join_status = (...args) => {
      const raw = joinStatus(...args);
      try {
        const value = JSON.parse(raw);
        const approval = value?.approval ?? null;
        const commit = value?.commit ?? null;
        const row = {
          status: value?.status ?? null,
          joinId: value?.id ?? approval?.id ?? commit?.id ?? null,
          joinKind: value?.kind ?? approval?.kind ?? commit?.kind ?? null,
          applyTick: approval?.apply_tick ?? null,
          commitTick: commit?.tick ?? null,
          digest: commit?.digest ?? null,
          reason: value?.reason ?? null,
        };
        const encoded = JSON.stringify(row);
        if (encoded !== priorStatus) {
          trace.joinStatuses.push(row);
          priorStatus = encoded;
        }
      } catch (_) {}
      return raw;
    };

    window.__gmM1Trace = trace;
  }, label);
}

async function readTrace(page) {
  if (!page || page.isClosed()) return null;
  return page.evaluate(() => structuredClone(window.__gmM1Trace ?? null));
}

function publicRoster(page) {
  return page.evaluate(() => ({
    ships: [...document.querySelectorAll('#fleet-slots li')].map((row) => ({
      slot: row.dataset.slot ?? null,
      label: row.textContent.trim(),
    })),
    gms: [...document.querySelectorAll('#fleet-gms [data-gm-id]')].map((row) => ({
      operatorId: row.dataset.gmId,
      label: row.textContent.trim(),
    })),
    policy: window.__hostGmStartState?.().policy ?? null,
  }));
}

function collectTraceEvidence(report, traces) {
  const frames = [...traces.values()].flatMap((trace) => (
    trace?.frames?.map((frame) => ({ peer: trace.label, ...frame })) ?? []
  ));
  report.snapshots = frames
    .filter((frame) => frame.type === 'snapshot')
    .map((frame) => ({
      peer: frame.peer,
      direction: frame.direction,
      from: frame.from,
      transferId: frame.transferId,
      tick: frame.tick,
      sequence: frame.sequence,
      total: frame.total,
      wholeHash: frame.wholeHash,
      crc: frame.crc,
      bytes: frame.bytes,
    }));
  report.digests = frames
    .filter((frame) => frame.digest != null)
    .map((frame) => ({
      peer: frame.peer,
      direction: frame.direction,
      type: frame.type,
      kind: frame.kind ?? null,
      joinId: frame.joinId ?? null,
      tick: frame.tick,
      digest: frame.digest,
    }));
  report.commands.push(...frames.flatMap((frame) => {
    if (frame.type === 'gm-action') {
      return [{
        source: 'mesh',
        peer: frame.peer,
        direction: frame.direction,
        tick: frame.tick,
        operatorId: frame.operatorId,
        action: frame.action,
        result: frame.kind,
        correlation: frame.correlation,
      }];
    }
    return (frame.commands ?? []).map((command) => ({
      source: 'mesh',
      peer: frame.peer,
      direction: frame.direction,
      ...command,
    }));
  }));
  report.phases.joinTransactions = [...traces.values()].flatMap((trace) => (
    trace?.joinStatuses?.map((status) => ({ peer: trace.label, ...status })) ?? []
  ));
}

test('M1 exits through a retained deterministic GM peer trace', async ({ context }, testInfo) => {
  test.setTimeout(420_000);
  const report = {
    schema: 1,
    issue: 1300,
    seed: 1300,
    scenarioTitle: 'GM M1 peer tracer',
    phases: {},
    roster: {},
    commands: [],
    snapshots: [],
    digests: [],
    feedAssertions: {},
    mapAssertions: {},
    accessibility: {},
    finalStatus: 'running',
    errors: [],
  };
  const traces = new Map();
  const tracked = [];
  const trackedErrors = [];
  const track = (page, label) => {
    const errors = captureServerPageErrors(page);
    tracked.push({ page, label });
    trackedErrors.push({ label, errors });
    return errors;
  };
  const preserveTrace = async (page, label) => {
    const trace = await readTrace(page);
    if (trace) traces.set(label, trace);
  };

  let gmOnly;
  let ship;
  let gmOne;
  let gmTwo;
  let gmTwoReturning;
  let captain;
  let helm;
  let engineering;
  let science;
  let captainUi;
  let helmReturning;
  let privateReconnectCredential = null;

  try {
    await fs.promises.rm(path.dirname(REPORT_PATH), { recursive: true, force: true });
    await installScenario(context);

    // Phase 1: the public role picker boots an explicit, rendererless GM host.
    // Its product roster has one peer GM and no invented player ship, and its
    // collective Ready is sufficient to start the deterministic session.
    gmOnly = await prepareGm(context, { clearIdentity: true });
    track(gmOnly, 'gm-only');
    await installMeshTrace(gmOnly, 'gm-only');
    await gmOnly.click('[data-control="fleet-open"]');
    await gmOnly.waitForFunction(
      () => window.__hostGmStartState?.().admitted === true
        && window.__hostGmStartState?.().policy?.validation_passed === true,
      undefined,
      { timeout: 30_000 },
    );
    await closeSettings(gmOnly);
    await expect(gmOnly.locator('#fleet-slots li')).toHaveCount(0);
    await clickGmControl(gmOnly, 'gm-ready-btn');
    await gmOnly.waitForFunction(
      () => window.__saveSlotsPhase === 'InProgress'
        && window.__hostGmStartState?.().policy?.started === true
        && window.__hostGmStartState?.().fixedResult?.status === 'applied',
      undefined,
      { timeout: 30_000 },
    );
    report.roster.gmOnly = await publicRoster(gmOnly);
    expect(report.roster.gmOnly.ships).toHaveLength(0);
    expect(report.roster.gmOnly.policy).toMatchObject({
      connected_players: 0,
      connected_gms: 1,
      ready_gms: 1,
      all_ready: true,
      started: true,
    });
    report.phases.gmOnly = { automaticStart: 'applied', playerShips: 0, gmPeers: 1 };
    await preserveTrace(gmOnly, 'gm-only');
    await gmOnly.close();

    // Phase 2 starts with four deterministic phone peers already holding the
    // fixed cruiser stations. Only Captain readies; the admitted GM's ordinary
    // Force Start control is therefore what starts this crewed run.
    ship = await bootHost(context);
    track(ship, 'ship-host');
    await installMeshTrace(ship, 'ship-host');
    const hostId = await readHostPeerId(ship);
    captain = await createTestClient(context, hostId, { name: 'Captain' });
    helm = await createTestClient(context, hostId, { name: 'Helm' });
    engineering = await createTestClient(context, hostId, { name: 'Engineering' });
    science = await createTestClient(context, hostId, { name: 'Science' });
    await selectAndWait(captain, 'Captain');
    await selectAndWait(helm, 'Helm');
    await selectAndWait(engineering, 'Engineering');
    await selectAndWait(science, 'Science');
    const fleetCode = await openFleet(ship);

    gmOne = await prepareGm(context, { clearIdentity: true });
    track(gmOne, 'gm-1');
    await installMeshTrace(gmOne, 'gm-1');
    await joinGm(gmOne, fleetCode, { waitForAdmission: true });
    await captain.send('SetReady', { ready: true });
    const started = captain.waitForMessage('GameStarted', 30_000);
    await clickGmControl(gmOne, 'gm-force-start-btn');
    await started;
    await Promise.all([ship, gmOne].map((page) => page.waitForFunction(
      () => window.__saveSlotsPhase === 'InProgress'
        && window.__hostGmStartState?.().fixedResult?.status === 'applied'
        && window.__hostMeshStatus?.().in_fleet === true,
      undefined,
      { timeout: 30_000 },
    )));
    const worldSetup = await captain.waitForMessage('WorldSetup', 15_000);
    expectFixtureWorld(worldSetup, GM_M1_WORLD);
    report.commands.push({ source: 'operator', operatorId: 'gm-1', action: 'force-start' });
    report.roster.crewedStart = await publicRoster(gmOne);
    expect(report.roster.crewedStart.policy).toMatchObject({
      connected_players: 4,
      ready_players: 1,
      connected_gms: 1,
      started: true,
    });
    expect(report.roster.crewedStart.policy.all_ready).toBe(false);

    // The map must be a product projection: authored points plus aggregate
    // field/Region geometry. Streamed rocks exist for crew but never become GM
    // point contacts. Touch picks a Region; keyboard moves to another contact.
    await captain.page.waitForFunction(
      (path) => window.__messages?.some(
        (message) => message.type === 'AsteroidSpawned' && message.data.config_path === path,
      ),
      ORDINARY_ROCK_PATH,
      { timeout: 15_000 },
    );
    await gmOne.waitForFunction(() => {
      const map = document.getElementById('gm-entity-map');
      return !map?.hidden
        && Array.isArray(map.state?.blips)
        && Array.isArray(map.state?.regions)
        && map.state.blips.length + map.state.regions.length > 0;
    }, undefined, { timeout: 30_000 });
    await gmOne.waitForFunction(() => {
      const map = document.getElementById('gm-entity-map');
      const pointKinds = new Set((map?.state?.blips ?? []).map((entry) => entry.kind));
      const regionKinds = new Set((map?.state?.regions ?? []).map((entry) => entry.kind));
      return ['player_ship', 'npc_ship', 'structure', 'authored_asteroid']
        .every((kind) => pointKinds.has(kind))
        && ['hazard', 'region', 'asteroid_field'].every((kind) => regionKinds.has(kind));
    }, undefined, { timeout: 30_000 });
    const mapped = await gmOne.evaluate(() => {
      const map = document.getElementById('gm-entity-map');
      return {
        blips: map.state.blips.map(({ uuid, kind }) => ({ uuid, kind })),
        regions: map.state.regions.map(({ uuid, kind, selectable }) => ({
          uuid, kind, selectable,
        })),
      };
    });
    const ordinaryRockIds = await captain.page.evaluate((path) => window.__messages
      .filter((message) => message.type === 'AsteroidSpawned'
        && message.data.config_path === path)
      .map((message) => message.data.uuid), ORDINARY_ROCK_PATH);
    report.mapAssertions.observed = mapped;
    report.mapAssertions.projection = await gmOne.evaluate(() => (
      window.__gmM1Trace?.projections?.at(-1) ?? null
    ));
    expect(mapped.blips.map((entry) => entry.kind)).toEqual(expect.arrayContaining([
      'player_ship', 'npc_ship', 'structure', 'authored_asteroid',
    ]));
    expect(mapped.regions.map((entry) => entry.kind)).toEqual(expect.arrayContaining([
      'hazard', 'region', 'asteroid_field',
    ]));
    expect(ordinaryRockIds.length).toBeGreaterThan(0);
    const mapIds = [...mapped.blips, ...mapped.regions].map((entry) => entry.uuid);
    expect(ordinaryRockIds.filter((uuid) => mapIds.includes(uuid))).toEqual([]);
    const touchRegion = mapped.regions.find((entry) => entry.kind === 'region');
    expect(touchRegion).toMatchObject({ kind: 'region', selectable: true });
    await gmOne.evaluate((uuid) => {
      const surface = document.getElementById('gm-entity-map');
      const region = surface.state.regions.find((entry) => entry.uuid === uuid);
      const canvas = surface.shadowRoot.querySelector('canvas');
      const rect = canvas.getBoundingClientRect();
      const radius = Math.min(rect.width, rect.height) / 2;
      const touchX = region.x;
      const touchZ = region.z + region.radius * 0.65;
      const clientX = rect.left + rect.width / 2 + (touchX / surface.state.range) * radius;
      const clientY = rect.top + rect.height / 2 - (touchZ / surface.state.range) * radius;
      const point = new Touch({ identifier: 1300, target: canvas, clientX, clientY });
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
    }, touchRegion.uuid);
    await gmOne.waitForFunction(
      (uuid) => document.getElementById('gm-entity-map').navigationSelectedUuid() === uuid,
      touchRegion.uuid,
      { timeout: 15_000 },
    );
    const inspector = gmOne.locator('#gm-entity-card');
    await expect(inspector).toBeVisible();
    await expect(inspector).toHaveAttribute('data-entity-id', touchRegion.uuid);
    await expect(inspector).toHaveAttribute('data-kind', touchRegion.kind);
    const touchInspector = await gmOne.evaluate(() => {
      const card = document.getElementById('gm-entity-card');
      return {
        visible: !card.hidden,
        entityId: card.dataset.entityId ?? null,
        kind: card.dataset.kind ?? null,
      };
    });
    const gmMap = gmOne.locator('#gm-entity-map');
    await gmMap.focus();
    await gmMap.press('ArrowRight');
    await gmOne.waitForFunction(
      (uuid) => {
        const selected = document.getElementById('gm-entity-map').navigationSelectedUuid();
        return typeof selected === 'string' && selected !== uuid;
      },
      touchRegion.uuid,
      { timeout: 15_000 },
    );
    const keyboardSelectedId = await gmOne.evaluate(
      () => document.getElementById('gm-entity-map').navigationSelectedUuid(),
    );
    const keyboardEntity = [...mapped.blips, ...mapped.regions]
      .find((entry) => entry.uuid === keyboardSelectedId);
    expect(keyboardEntity).toBeDefined();
    await expect(inspector).toBeVisible();
    await expect(inspector).toHaveAttribute('data-entity-id', keyboardSelectedId);
    await expect(inspector).toHaveAttribute('data-kind', keyboardEntity.kind);
    const keyboardInspector = await gmOne.evaluate(() => {
      const card = document.getElementById('gm-entity-card');
      return {
        visible: !card.hidden,
        entityId: card.dataset.entityId ?? null,
        kind: card.dataset.kind ?? null,
      };
    });
    report.mapAssertions = {
      ...report.mapAssertions,
      pointKinds: [...new Set(mapped.blips.map((entry) => entry.kind))].sort(),
      regionKinds: [...new Set(mapped.regions.map((entry) => entry.kind))].sort(),
      ordinaryAsteroidsObserved: ordinaryRockIds.length,
      ordinaryAsteroidsProjected: 0,
      touchSelectedRegion: true,
      keyboardMovedSelection: true,
      inspector: {
        touch: touchInspector,
        keyboard: keyboardInspector,
      },
    };

    // Move the deterministic Captain transport peer onto the shipped phone
    // shell before issuing the user-visible command. The retained session token
    // proves that this is the same station tenure, while the click below now
    // traverses the real client iframe and semantic-action path.
    const captainToken = captain.token;
    await captain.close();
    captain = null;
    captainUi = await reconnectCrew(context, hostId, captainToken, 'captain');
    track(captainUi, 'captain-client');
    await expect(captainUi.locator('#captain-ui')).toHaveClass(/active/, {
      timeout: 30_000,
    });
    const alertButton = captainUi.frameLocator('#captain-iframe')
      .locator('ph-red-alert').locator('#alert-btn');
    await expect(alertButton).toBeVisible({ timeout: 20_000 });
    await alertButton.click();
    await gmOne.waitForFunction(
      () => window.__hostGmActivityState?.().entries?.some(
        (entry) => entry.category === 'red_alert' && entry.detail?.data?.active === true,
      ),
      undefined,
      { timeout: 30_000 },
    );
    report.commands.push({
      source: 'crew', station: 'captain', action: 'set-red-alert', surface: 'shipped-client-ui',
    });

    const beforePause = await gmOne.evaluate(() => window.wasm_sim_tick());
    await clickGmControl(gmOne, 'gm-session-pause');
    await Promise.all([ship, gmOne].map((page) => page.waitForFunction(
      () => window.wasm_is_paused() === true,
      undefined,
      { timeout: 30_000 },
    )));
    await expect(gmOne.locator('#gm-session-state')).toBeVisible();
    await expect(gmOne.locator('#gm-session-state')).toHaveAttribute('data-paused', 'true');
    await expect(gmOne.locator('#gm-session-state')).toContainText(/session paused/i);
    const pausedSessionState = await readStatusEvidence(gmOne, 'gm-session-state');
    expect(pausedSessionState).toMatchObject({
      visible: true, role: 'status', live: 'polite', atomic: 'true', paused: 'true',
    });
    expect(pausedSessionState.text).toMatch(/session paused/i);
    const pausedAt = await gmOne.evaluate(() => window.wasm_sim_tick());
    await gmOne.waitForTimeout(300);
    expect(await gmOne.evaluate(() => window.wasm_sim_tick())).toBe(pausedAt);
    await clickGmControl(gmOne, 'gm-session-resume');
    await Promise.all([ship, gmOne].map((page) => page.waitForFunction(
      () => window.wasm_is_paused() === false,
      undefined,
      { timeout: 30_000 },
    )));
    await gmOne.waitForFunction(
      (tick) => window.wasm_sim_tick() > tick,
      pausedAt,
      { timeout: 30_000 },
    );
    await expect(gmOne.locator('#gm-session-state')).toHaveAttribute('data-paused', 'false');
    await expect(gmOne.locator('#gm-session-state')).toContainText(/session running/i);
    const runningSessionState = await readStatusEvidence(gmOne, 'gm-session-state');
    expect(runningSessionState).toMatchObject({
      visible: true, role: 'status', live: 'polite', atomic: 'true', paused: 'false',
    });
    expect(runningSessionState.text).toMatch(/session running/i);
    report.accessibility.sessionControl = {
      paused: pausedSessionState,
      running: runningSessionState,
    };
    report.commands.push(
      { source: 'operator', operatorId: 'gm-1', action: 'pause', requestedAfterTick: beforePause },
      { source: 'operator', operatorId: 'gm-1', action: 'resume', pausedAtTick: pausedAt },
    );

    // A first-time GM is private until an existing peer visibly accepts. The
    // panel announces both transaction and pause state to assistive technology.
    gmTwo = await prepareGm(context, { clearIdentity: true });
    track(gmTwo, 'gm-2-first-join');
    await installMeshTrace(gmTwo, 'gm-2-first-join');
    await joinGm(gmTwo, fleetCode, { waitForAdmission: false });
    await expect(gmOne.locator('#gm-join-controls')).toBeVisible({ timeout: 30_000 });
    await expect(gmOne.locator('#gm-join-accept')).toBeVisible();
    await expect(gmOne.locator('#gm-join-reject')).toBeVisible();
    await expect(gmOne.locator('#gm-join-state'))
      .toContainText(/wants to join as a Game Master/i);
    await expect(gmOne.locator('#gm-join-pause-state')).toContainText(/session running/i);
    expect(await gmTwo.evaluate(() => window.__hostGmStartState?.().admitted)).toBe(false);
    expect((await publicRoster(gmOne)).gms).toHaveLength(1);
    report.accessibility.firstJoin = await gmOne.evaluate(() => {
      const panel = document.getElementById('gm-join-controls');
      const status = document.getElementById('gm-join-state');
      const pause = document.getElementById('gm-join-pause-state');
      return {
        panelRole: panel.getAttribute('role'),
        labelledBy: panel.getAttribute('aria-labelledby'),
        statusRole: status.getAttribute('role'),
        statusLive: status.getAttribute('aria-live'),
        statusAtomic: status.getAttribute('aria-atomic'),
        statusText: status.textContent.trim(),
        statusVisible: status.getClientRects().length > 0,
        pauseRole: pause.getAttribute('role'),
        pauseLive: pause.getAttribute('aria-live'),
        pauseAtomic: pause.getAttribute('aria-atomic'),
        pauseText: pause.textContent.trim(),
        pauseVisible: pause.getClientRects().length > 0,
        acceptVisible: !document.getElementById('gm-join-accept').hidden,
        rejectVisible: !document.getElementById('gm-join-reject').hidden,
      };
    });
    expect(report.accessibility.firstJoin).toMatchObject({
      panelRole: 'region',
      labelledBy: 'gm-join-heading',
      statusRole: 'status',
      statusLive: 'polite',
      statusAtomic: 'true',
      pauseRole: 'status',
      pauseLive: 'polite',
      pauseAtomic: 'true',
      statusVisible: true,
      pauseVisible: true,
      acceptVisible: true,
      rejectVisible: true,
    });
    expect(report.accessibility.firstJoin.statusText)
      .toMatch(/wants to join as a Game Master/i);
    expect(report.accessibility.firstJoin.pauseText).toMatch(/session running/i);
    await gmOne.locator('#gm-join-accept').click();
    await gmTwo.waitForFunction(
      () => window.__hostGmStartState?.().admitted === true
        && window.__hostMeshStatus?.().in_fleet === true,
      undefined,
      { timeout: 60_000 },
    );
    await Promise.all([ship, gmOne, gmTwo].map((page) => page.waitForFunction(
      () => window.wasm_is_paused() === true,
      undefined,
      { timeout: 30_000 },
    )));
    await expect(gmOne.locator('#gm-join-state'))
      .toContainText(/joined with matching session state/i);
    await expect(gmOne.locator('#gm-join-pause-state')).toContainText(/session paused/i);
    report.accessibility.firstJoin.committed = {
      status: await readStatusEvidence(gmOne, 'gm-join-state'),
      pause: await readStatusEvidence(gmOne, 'gm-join-pause-state'),
    };
    expect(report.accessibility.firstJoin.committed.status).toMatchObject({
      visible: true, role: 'status', live: 'polite', atomic: 'true',
    });
    expect(report.accessibility.firstJoin.committed.pause).toMatchObject({
      visible: true, role: 'status', live: 'polite', atomic: 'true',
    });
    await closeSettings(gmTwo);
    const gmTwoIdentity = await gmTwo.evaluate(
      (key) => JSON.parse(localStorage.getItem(key)),
      GM_IDENTITY_KEY,
    );
    privateReconnectCredential = typeof gmTwoIdentity?.reconnectCredential === 'string'
      ? gmTwoIdentity.reconnectCredential : null;
    expect(gmTwoIdentity.operatorId).toBe('gm-2');
    expect(gmTwoIdentity.rolePreset).toBeNull();
    expect(typeof gmTwoIdentity.reconnectCredential).toBe('string');
    expect(gmTwoIdentity.reconnectCredential.length).toBeGreaterThan(10);
    report.roster.afterFirstJoin = await publicRoster(gmOne);
    expect(report.roster.afterFirstJoin.gms).toHaveLength(2);
    report.phases.firstJoin = {
      candidatePrivateBeforeAccept: true,
      explicitAccept: true,
      committed: true,
      pausedAfterCommit: true,
      operatorId: gmTwoIdentity.operatorId,
      privateCapabilityRetained: true,
    };
    await clickGmControl(gmOne, 'gm-session-resume');
    await Promise.all([ship, gmOne, gmTwo].map((page) => page.waitForFunction(
      () => window.wasm_is_paused() === false,
      undefined,
      { timeout: 30_000 },
    )));
    report.commands.push(
      { source: 'operator', operatorId: 'gm-1', action: 'accept-first-time-gm' },
      { source: 'operator', operatorId: 'gm-1', action: 'resume-after-first-join' },
    );

    // The same private capability returns to the same public operator/slot.
    // Reconnect has no Accept/Reject affordance, still transfers a whole record,
    // and still leaves the fleet paused until a human explicitly resumes it.
    await preserveTrace(gmTwo, 'gm-2-first-join');
    await gmTwo.close();
    await expect(gmOne.locator('#fleet-gms [data-gm-id="gm-2"]'))
      .toContainText(/disconnected/i, { timeout: 30_000 });
    gmTwoReturning = await prepareGm(context, { clearIdentity: false });
    track(gmTwoReturning, 'gm-2-reconnect');
    await installMeshTrace(gmTwoReturning, 'gm-2-reconnect');
    await joinGm(gmTwoReturning, fleetCode, { waitForAdmission: false });
    await gmOne.waitForFunction(
      () => window.wasm_is_paused() === true,
      undefined,
      { timeout: 60_000 },
    );
    await expect(gmOne.locator('#gm-join-controls')).toBeVisible();
    await expect(gmOne.locator('#gm-join-accept')).toBeHidden();
    await expect(gmOne.locator('#gm-join-reject')).toBeHidden();
    await gmTwoReturning.waitForFunction(
      () => window.__hostGmStartState?.().admitted === true
        && window.__hostMeshStatus?.().in_fleet === true,
      undefined,
      { timeout: 60_000 },
    );
    await closeSettings(gmTwoReturning);
    const returnedIdentity = await gmTwoReturning.evaluate(
      ({ key, expectedCredential }) => {
        const identity = JSON.parse(localStorage.getItem(key));
        return {
          operatorId: identity?.operatorId ?? null,
          rolePreset: identity?.rolePreset ?? null,
          credentialMatches: identity?.reconnectCredential === expectedCredential,
          credentialPresent: typeof identity?.reconnectCredential === 'string'
            && identity.reconnectCredential.length > 10,
        };
      },
      { key: GM_IDENTITY_KEY, expectedCredential: privateReconnectCredential },
    );
    expect(returnedIdentity.operatorId).toBe(gmTwoIdentity.operatorId);
    expect(returnedIdentity.rolePreset).toBe(gmTwoIdentity.rolePreset);
    expect(returnedIdentity.credentialPresent).toBe(true);
    expect(returnedIdentity.credentialMatches).toBe(true);
    expect(await gmTwoReturning.evaluate(() => window.wasm_is_paused())).toBe(true);
    await expect(gmOne.locator('#gm-join-state'))
      .toContainText(/joined with matching session state/i);
    await expect(gmOne.locator('#gm-join-pause-state')).toContainText(/session paused/i);
    report.accessibility.reconnect = {
      joinRegionVisible: true,
      acceptVisible: false,
      rejectVisible: false,
      status: await readStatusEvidence(gmOne, 'gm-join-state'),
      pause: await readStatusEvidence(gmOne, 'gm-join-pause-state'),
    };
    expect(report.accessibility.reconnect.status).toMatchObject({
      visible: true, role: 'status', live: 'polite', atomic: 'true',
    });
    expect(report.accessibility.reconnect.pause).toMatchObject({
      visible: true, role: 'status', live: 'polite', atomic: 'true',
    });
    report.phases.reconnect = {
      sameOperatorId: returnedIdentity.operatorId,
      automaticApproval: true,
      committed: true,
      pausedAfterCommit: true,
    };
    await clickGmControl(gmTwoReturning, 'gm-session-resume');
    await Promise.all([ship, gmOne, gmTwoReturning].map((page) => page.waitForFunction(
      () => window.wasm_is_paused() === false,
      undefined,
      { timeout: 30_000 },
    )));
    report.commands.push({
      source: 'operator', operatorId: 'gm-2', action: 'resume-after-reconnect',
    });

    // Authentic station puppet: retain the player's Helm tenure, let Backfill
    // take over on disconnect, then use the projected cruiser iframe for both an
    // Impulse click and its real ArrowUp input before releasing it back to AI.
    const helmToken = helm.token;
    await helm.close();
    helm = null;
    await ship.waitForFunction(
      // eslint-disable-next-line no-eval
      (token) => (0, eval)('hostConnections').targets(`token:${token}`, 'reliable').length === 0,
      helmToken,
      { timeout: 15_000 },
    );
    await gmOne.waitForFunction(
      () => window.__hostGmStationState?.().projection?.ships?.some((shipRow) =>
        shipRow.stations?.some((station) => station.station_id === 'helm'
          && station.rating === 'Backfill')),
      undefined,
      { timeout: 30_000 },
    );
    await gmOne.evaluate(() => {
      const select = document.getElementById('gm-station-select');
      const option = [...select.options].find((candidate) => candidate.textContent.endsWith('Helm'));
      if (!option) throw new Error('Helm Station is absent from the GM projection');
      select.value = option.value;
      select.dispatchEvent(new Event('change'));
    });
    await gmOne.waitForFunction(
      () => {
        const row = window.__hostGmStationState?.().selectedRow;
        return row?.station?.station_id === 'helm'
          && row.station.rating === 'Backfill'
          && row.station.console === 'gui/cruiser/helm.html';
      },
      undefined,
      { timeout: 30_000 },
    );
    await expect(gmOne.locator('#gm-station-frame'))
      .toHaveAttribute('src', 'gui/cruiser/helm.html');
    expect(gmOne.viewportSize()).toEqual({ width: 1280, height: 720 });
    const takeover = gmOne.locator('#gm-station-toggle');
    await takeover.scrollIntoViewIfNeeded();
    const takeoverBox = await takeover.boundingBox();
    expect(takeoverBox.y).toBeGreaterThanOrEqual(0);
    expect(takeoverBox.y + takeoverBox.height).toBeLessThanOrEqual(720);
    await takeover.click();
    const operatorId = await gmOne.evaluate(() => window.__hostLocalGm().id);
    await gmOne.waitForFunction(
      (operator) => {
        const row = window.__hostGmStationState?.().selectedRow;
        return row?.station?.operators?.includes(operator)
          && row.ship.control_sources?.['helm-thrust'] === 'Human';
      },
      operatorId,
      { timeout: 30_000 },
    );
    const stationFrame = gmOne.locator('#gm-station-frame');
    await stationFrame.scrollIntoViewIfNeeded();
    const helmFrame = gmOne.frameLocator('#gm-station-frame');
    const impulse = helmFrame.locator('#impulse-btn').locator('#btn');
    const impulseFeedback = helmFrame.locator(
      '.semantic-action-feedback__item[data-action-id="helm.impulse"]',
    );
    await expect(impulse).toBeEnabled({ timeout: 20_000 });
    await impulse.click();
    await expect(impulseFeedback).toHaveAttribute('data-state', 'Applied', {
      timeout: 30_000,
    });
    const joystick = helmFrame.locator('#helm-joystick');
    await joystick.focus();
    await gmOne.keyboard.down('ArrowUp');
    await gmOne.waitForFunction(
      (operator) => window.__hostGmStationState?.().projection?.activity?.some(
        (entry) => entry.operator_id === operator && entry.target === 'helm-thrust',
      ),
      operatorId,
      { timeout: 30_000 },
    );
    await gmOne.keyboard.up('ArrowUp');
    await takeover.scrollIntoViewIfNeeded();
    await takeover.click();
    await gmOne.waitForFunction(
      (operator) => {
        const row = window.__hostGmStationState?.().selectedRow;
        return row?.station?.rating === 'Backfill'
          && !row.station.operators?.includes(operator)
          && row.ship.control_sources?.['helm-thrust'] === 'Ai';
      },
      operatorId,
      { timeout: 30_000 },
    );
    helmReturning = await reconnectCrew(context, hostId, helmToken, 'helm');
    await expect(helmReturning.frameLocator('#helm-iframe').locator('#gm-takeover-banner'))
      .toBeHidden({ timeout: 20_000 });
    report.commands.push(
      { source: 'operator', operatorId, station: 'helm', action: 'take-over' },
      { source: 'operator', operatorId, station: 'helm', action: 'impulse' },
      { source: 'operator', operatorId, station: 'helm', action: 'keyboard-arrow-up' },
      { source: 'operator', operatorId, station: 'helm', action: 'release' },
    );
    report.phases.stationPuppet = {
      console: 'gui/cruiser/helm.html',
      viewport: { width: 1280, height: 720 },
      takeoverControlReachable: true,
      impulseApplied: true,
      keyboardActivityApplied: true,
      releasedTo: 'Ai',
      retainedPlayerStation: 'helm',
    };

    // Close the exit gate on the complete retained feed and agreeing peer mesh.
    await gmOne.waitForFunction(
      (required) => {
        const categories = new Set(
          (window.__hostGmActivityState?.().entries ?? []).map((entry) => entry.category),
        );
        return required.every((category) => categories.has(category));
      },
      REQUIRED_FEED_CATEGORIES,
      { timeout: 60_000 },
    );
    const feed = await gmOne.evaluate(() => window.__hostGmActivityState());
    const categoryCounts = Object.fromEntries(REQUIRED_FEED_CATEGORIES.map((category) => [
      category,
      feed.entries.filter((entry) => entry.category === category).length,
    ]));
    for (const category of REQUIRED_FEED_CATEGORIES) {
      expect(categoryCounts[category]).toBeGreaterThan(0);
      await expect(gmOne.locator(`.gm-activity-entry[data-category="${category}"]`).first())
        .toBeVisible();
    }
    expect(feed.entries.length).toBeLessThanOrEqual(feed.capacity);
    expect(feed.entries.every((entry, index) => (
      index === 0 || feed.entries[index - 1].tick <= entry.tick
    ))).toBe(true);
    report.feedAssertions = {
      requiredCategories: REQUIRED_FEED_CATEGORIES,
      categoryCounts,
      capacity: feed.capacity,
      retainedEntries: feed.entries.length,
      oldestFirst: true,
      hostChannelInjectionUsed: false,
    };

    const finalPeers = [
      { peer: 'ship-host', role: 'player_ship', operator: null, page: ship },
      { peer: 'gm-1', role: 'gm', operator: 'gm-1', page: gmOne },
      { peer: 'gm-2-reconnect', role: 'gm', operator: 'gm-2', page: gmTwoReturning },
    ];
    const mesh = await Promise.all(finalPeers.map(async ({ page }) => {
      const healthy = await page.waitForFunction(
        () => {
          const mesh = window.__hostMeshStatus?.();
          return mesh?.in_fleet === true && mesh.stalled === false && mesh.disagreement == null
            ? mesh : false;
        },
        undefined,
        { timeout: 60_000 },
      );
      // Keep the state that satisfied the gate. A later read may legitimately
      // be waiting for the next tick's inputs even though this peer recovered.
      const status = await healthy.jsonValue();
      await healthy.dispose();
      return status;
    }));
    expect(mesh.map((status) => status.slot).sort((a, b) => a - b)).toEqual([1, 2, 3]);
    expect(mesh.every((status) => status.disagreement == null)).toBe(true);
    report.roster.final = await publicRoster(gmOne);
    report.roster.finalPeerMesh = finalPeers.map(({ peer, role, operator }, index) => ({
      peer,
      role,
      operator,
      slot: mesh[index].slot,
      inFleet: mesh[index].in_fleet,
      stalled: mesh[index].stalled,
      disagreement: mesh[index].disagreement ?? null,
    }));
    expect(report.roster.finalPeerMesh.every((peer) => (
      peer.inFleet === true && peer.stalled === false && peer.disagreement === null
    ))).toBe(true);
    report.phases.crewed = {
      forceStart: 'applied',
      deterministicPeerSlots: mesh.map((status) => status.slot).sort((a, b) => a - b),
      disagreements: 0,
    };

    for (const { page, label } of tracked) await preserveTrace(page, label);
    collectTraceEvidence(report, traces);
    const committed = report.digests.filter((row) => (
      row.type === 'gm-join' && row.kind === 'committed'
    ));
    expect(new Set(committed.map((row) => row.joinId)).size).toBeGreaterThanOrEqual(2);
    expect(report.snapshots.length).toBeGreaterThan(0);
    report.phases.snapshotFraming = {};
    for (const peer of ['gm-2-first-join', 'gm-2-reconnect']) {
      const peerFrames = report.snapshots.filter((row) => (
        row.peer === peer && row.direction === 'inbound'
      ));
      expect(peerFrames.length, `${peer} must receive snapshot chunks`).toBeGreaterThan(0);
      expect(peerFrames.every((row) => (
        /^[0-9a-f]{16}$/i.test(row.transferId)
          && Number.isInteger(row.sequence)
          && Number.isInteger(row.total)
          && row.total > 0
          && row.sequence >= 0
          && row.sequence < row.total
          && /^[0-9a-f]{16}$/i.test(row.wholeHash)
          && Number.isInteger(row.crc)
          && row.bytes > 0
      )), `${peer} snapshot framing`).toBe(true);
      const transfers = [...new Set(peerFrames.map((row) => row.transferId))].map((transferId) => {
        const chunks = peerFrames.filter((row) => row.transferId === transferId);
        const total = chunks[0].total;
        const sequences = [...new Set(chunks.map((row) => row.sequence))].sort((a, b) => a - b);
        const complete = chunks.every((row) => (
          row.total === total && row.wholeHash === chunks[0].wholeHash
        )) && sequences.length === total && sequences.every((sequence, index) => sequence === index);
        return {
          transferId,
          wholeHash: chunks[0].wholeHash,
          total,
          observedSequences: sequences.length,
          complete,
        };
      });
      expect(transfers.some((transfer) => transfer.complete), `${peer} complete snapshot transfer`)
        .toBe(true);
      report.phases.snapshotFraming[peer] = {
        inboundChunks: peerFrames.length,
        transfers,
      };
    }
    for (const joinId of new Set(committed.map((row) => row.joinId))) {
      const values = new Set(committed
        .filter((row) => row.joinId === joinId)
        .map((row) => row.digest));
      expect(values.size).toBe(1);
    }
    for (const { label, errors } of trackedErrors) {
      expect(errors, `${label} server page errors`).toEqual([]);
    }
    report.finalStatus = 'passed';
  } catch (error) {
    report.finalStatus = 'failed';
    report.errors.push(redactDiagnostic(
      error?.stack ?? String(error),
      [privateReconnectCredential],
    ));
    throw error;
  } finally {
    for (const { page, label } of tracked) {
      try { await preserveTrace(page, label); } catch (_) {}
    }
    if (report.snapshots.length === 0 && traces.size > 0) {
      try {
        collectTraceEvidence(report, traces);
      } catch (error) {
        report.errors.push(redactDiagnostic(
          `trace collection: ${error?.stack ?? String(error)}`,
          [privateReconnectCredential],
        ));
      }
    }
    for (const { label, errors } of trackedErrors) {
      for (const error of errors) {
        report.errors.push(redactDiagnostic(
          `${label}: ${error}`,
          [privateReconnectCredential],
        ));
      }
    }
    // The artifact is a diagnostic index, never a save export or bearer-token
    // dump. Deep-redact before serialization, write first so a failing test
    // always leaves its evidence behind, then assert the stored invariant.
    const safeReport = redactArtifact(report, [privateReconnectCredential]);
    const encoded = JSON.stringify(safeReport, null, 2);
    const redactionFailed = encoded.includes('reconnectCredential')
      || encoded.includes('portable-record-slice')
      || (privateReconnectCredential && encoded.includes(privateReconnectCredential));
    const stored = redactionFailed
      ? JSON.stringify({
        schema: 1,
        issue: 1300,
        seed: 1300,
        scenarioTitle: 'GM M1 peer tracer',
        finalStatus: 'failed',
        errors: ['Report redaction invariant failed; unsafe diagnostic details were omitted.'],
      }, null, 2)
      : encoded;
    await fs.promises.mkdir(path.dirname(REPORT_PATH), { recursive: true });
    await fs.promises.writeFile(REPORT_PATH, `${stored}\n`, 'utf8');
    await testInfo.attach('gm-m1-exit-report', {
      path: REPORT_PATH,
      contentType: 'application/json',
    });
    if (redactionFailed) {
      throw new Error('GM M1 report redaction invariant failed; wrote safe fallback artifact');
    }
  }
});
