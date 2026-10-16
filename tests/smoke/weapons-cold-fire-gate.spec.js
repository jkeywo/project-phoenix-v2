// Tactical restraint is independent of incoming energy flow. A held fire
// command is refused; releasing the same latch lets the identical bank fire.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  expectFixtureWorld,
} from './fixtures';
import { parse, stringify } from 'smol-toml';
import { writeFileSync } from 'node:fs';

// Self-contained smoke-test world, the same shape `tactical-fire-flow.spec.js`
// uses: the player's cruiser at the origin facing -Z, one hostile 15.8 units off
// the port bow, inside the fore bank's arc, so Helm need not steer.
const MINIMAL_TEST_WORLD = `
[global]
seed = 42
title = "Weapons Cold Fixture"

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
overrides = { tags = ["ship"] }

[[entity]]
template_path = "assets/entities/ship_harrow_patrol.toml"
name          = "raider_alpha"
overrides     = { name = "raider_alpha", display_name = "raider_alpha" }
transform     = { position = [-15.0, 0.0, -5.0] }
spawn_on      = "game_start"
`;



/** Send SelectStation and wait for a StationAssigned for *this* client's token. */
async function selectAndWait(client, station, timeout = 5_000) {
  await client.send('SelectStation', { station });
  await client.page.waitForFunction(
    (t) => window.__messages?.some(
      (m) => m.type === 'StationAssigned' && m.data.token === t,
    ),
    client.token,
    { timeout },
  );
}

async function setRestraint(tactical, held) {
  const correlation = held ? 'hold-weapons' : 'release-weapons';
  await tactical.send('ControlSystemCorrelated', { target: 'tactical-radar', correlation,
    payload: { type: 'SetWeaponsRestraint', data: { held } } });
  await tactical.page.waitForFunction(({ correlation, held }) => {
    const messages = window.__messages || [];
    const accepted = messages.some(message => message.type === 'ActionFeedback'
      && message.data.correlation === correlation && message.data.outcome === 'Applied');
    const latest = messages.filter(message => message.type === 'BlackboardUpdate').reverse()
      .flatMap(message => message.data.updates || []).find(([, board]) => typeof board?.data?.weapons_restrained === 'boolean');
    return accepted && latest?.[1]?.data.weapons_restrained === held;
  }, { correlation, held }, { timeout: 15000 });
}

/** How many `BeamStarted` messages this client has seen. */
function beamCount(client) {
  return client.page.evaluate(
    () => (window.__messages || []).filter((m) => m.type === 'BeamStarted').length,
  );
}

// Capture only the actual phone's permitted publications, never host ECS data.
// This distinguishes admission/consumer refusal from missing beam delivery.
async function consoleEvidence(client) {
  return client.page.evaluate(() => {
    const messages = window.__messages || [];
    const latest = type => [...messages].reverse().find(message => message.type === type)?.data ?? null;
    const blackboards = {};
    for (const message of messages) {
      if (message.type !== 'BlackboardUpdate') continue;
      for (const [owner, board] of message.data.updates || []) {
        if (['tactical-radar', 'tactical', 'phaser-control', 'power-reactor', 'viewscreen'].includes(owner)
          || owner.startsWith('phaser-')) blackboards[owner] = board;
      }
    }
    return {
      actions: messages.filter(message => message.type === 'ActionFeedback').slice(-12),
      targetLock: latest('TargetLock'), weapons: latest('WeaponsUpdate'),
      simulation: latest('SimState'), blackboards,
    };
  });
}

async function startGame(context) {
  // Serve the self-contained world instead of the real default.toml.
  await context.route('**/assets/worlds/default.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: MINIMAL_TEST_WORLD }),
  );

  // Preserve the intended inert target using the current authored AI schema.
  // None directives hold position; each unchanged equipped bank explicitly
  // declines automatic fire. No retired FSM text or private state is patched.
  await context.route('**/assets/entities/ship_harrow_patrol.toml', async (route) => {
    const response = await route.fetch();
    const config = parse(await response.text());
    expect(config.behaviour.doctrine.map(entry => entry.id).sort()).toEqual([
      'destroy-hostiles', 'patrol-ironveil',
    ]);
    for (const entry of config.behaviour.doctrine) {
      entry.directive_kind = 'None';
      entry.directive_anchors = [];
      entry.directive_loop = false;
      entry.target_speed = 0;
    }
    expect(config.weapon.map(weapon => weapon.id).sort()).toEqual([
      'phaser-port', 'phaser-starboard',
    ]);
    for (const weapon of config.weapon) weapon.ai = { idle: true };
    await route.fulfill({ contentType: 'text/plain', body: stringify(config) });
  });

  const serverPage = await context.newPage();
  const serverCrashes = [];
  serverPage.on('crash', () => { serverCrashes.push('server page crashed'); });
  serverPage.on('pageerror', (err) => { serverCrashes.push(err.message); });

  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);

  // Three human stations keep their ordinary AI controllers delegated while
  // Tactical demonstrates restraint and Engineering deliberately cuts intake.
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  const tactical = await createTestClient(context, hostId, { name: 'Tac' });
  const engineering = await createTestClient(context, hostId, { name: 'Eng' });

  await selectAndWait(helm, 'Helm');
  await selectAndWait(tactical, 'Tactical');
  await selectAndWait(engineering, 'Engineering');

  await helm.send('SetReady', { ready: true });
  await tactical.send('SetReady', { ready: true });
  await engineering.send('SetReady', { ready: true });
  await tactical.waitForMessage('GameStarted', 10_000);
  await engineering.waitForMessage('GameStarted', 10_000);

  return { helm, tactical, engineering, serverCrashes };
}

test('Tactical holds and releases firing independently of Engineering flow', { tag: '@core' }, async ({ context }) => {
  test.setTimeout(180_000);
  const { helm, tactical, engineering, serverCrashes } = await startGame(context);

  const worldSetup = await tactical.waitForMessage('WorldSetup', 5_000);
  expectFixtureWorld(worldSetup, MINIMAL_TEST_WORLD);
  expect((worldSetup.data.world.entities || []).some(entity => entity.tags?.includes('npc'))).toBe(false);
  // Acquire the observer-local handle from the latest actual paid Tactical
  // publication. WorldSetup never supplies uncharted foreign live identities.
  await tactical.page.bringToFront();
  const contact = await tactical.page.waitForFunction(() => {
    for (const message of [...(window.__messages || [])].reverse()) {
      if (message.type !== 'BlackboardUpdate') continue;
      const entry = (message.data.updates || []).find(([owner]) => owner === 'tactical-radar');
      if (!entry) continue;
      const track = entry[1]?.data?.tracks?.find(candidate => !candidate.predicted
        && candidate.selectable && candidate.identity?.name === 'raider_alpha');
      return track?.contact_id || null;
    }
    return null;
  }, undefined, { timeout: 20000 });
  const raiderContact = await contact.jsonValue();
  expect(raiderContact, 'the target must be earned by Tactical').toBeTruthy();

  // Lock the raider and wait until a bank reports itself fire-ready, so every
  // other firing prerequisites are satisfied before Tactical holds fire.
  await tactical.send('ControlSystem', {
    target: 'tactical-radar',
    payload: { type: 'SetTarget', data: { uuid: raiderContact } },
  });
  await tactical.page.waitForFunction(handle => window.__messages?.some(message =>
    message.type === 'TargetLock' && message.data.uuid === handle && message.data.locked === true),
  raiderContact, { timeout: 15000 });
  await tactical.page.bringToFront();
  await tactical.page.waitForFunction(
    () => window.__messages?.some(
      (m) => m.type === 'WeaponsUpdate'
        && Array.isArray(m.data.banks)
        && m.data.banks.some((b) => b.fire_ready === true),
    ),
    undefined,
    { timeout: 20_000 },
  );

  // Engineering may stop incoming flow without suppressing an already
  // charged local weapon reserve. Tactical's explicit restraint does suppress it.
  await engineering.send('ControlSystemCorrelated', { target: 'power-reactor', correlation: 'flow-cut',
    payload: { type: 'SetEnergyGroup', data: { group: 'weapons', flow_per_sec: 0, cooling_per_sec: 0 } } });
  await engineering.page.waitForFunction(() => window.__messages?.some(message => message.type === 'ActionFeedback'
    && message.data.correlation === 'flow-cut' && message.data.outcome === 'Applied'));
  await setRestraint(tactical, true);
  expect(await beamCount(tactical)).toBe(0);
  await tactical.send('ControlSystemCorrelated', { target: 'phaser-fore', correlation: 'held-fire', payload: { type: 'FirePhaser' } });
  await tactical.page.waitForFunction(() => window.__messages?.some(message => message.type === 'ActionFeedback'
    && message.data.correlation === 'held-fire' && message.data.outcome === 'Refused'), undefined, { timeout: 15000 });
  expect(await beamCount(tactical), 'an acknowledged held fire order produces no beam').toBe(0);
  await setRestraint(tactical, false);
  const beforeReleasedFire = {
    tactical: await consoleEvidence(tactical), engineering: await consoleEvidence(engineering),
  };
  await tactical.send('ControlSystemCorrelated', { target: 'phaser-fore', correlation: 'released-fire', payload: { type: 'FirePhaser' } });
  try {
    const receipt = await tactical.page.waitForFunction(() => window.__messages?.find(message =>
      message.type === 'ActionFeedback' && message.data.correlation === 'released-fire')?.data,
    undefined, { timeout: 15000 });
    const feedback = await receipt.jsonValue();
    expect(feedback.outcome, 'the actual released-fire consumer must apply the command').toBe('Applied');
    const beamStarted = await tactical.waitForMessage('BeamStarted', 20000);
    expect(beamStarted.data.target_uuid).toBe(raiderContact);
  } catch (error) {
    const evidencePath = test.info().outputPath('released-fire-permitted-state.json');
    writeFileSync(evidencePath, JSON.stringify({ target: raiderContact, before: beforeReleasedFire,
      after: { tactical: await consoleEvidence(tactical), engineering: await consoleEvidence(engineering) } }, null, 2), 'utf8');
    await test.info().attach('released-fire-permitted-state', {
      path: evidencePath, contentType: 'application/json',
    });
    throw error;
  }

  expect(serverCrashes).toEqual([]);

  await helm.close();
  await tactical.close();
  await engineering.close();
});

test('Tactical visible restraint targets its radar independently of weapon delegation', async ({ page }) => {
  await page.goto('/client/gui/battleship/tactical.html');
  await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  await page.evaluate(() => { window.__restraintActions = []; window.__sendAction = json => window.__restraintActions.push(JSON.parse(json)); });
  const payload = { tactical_auto: true, weapons_restraint_auto: false, weapons_restrained: false, tactical_radar_system_id: 'radar-instance' };
  await page.evaluate(value => window.__updateConsole('tactical', JSON.stringify(value)), payload);
  const control = page.locator('[data-weapons-restraint]'); await expect(control).toBeEnabled(); await expect(control).toHaveAttribute('aria-pressed', 'false');
  await control.click();
  expect(await page.evaluate(() => window.__restraintActions)).toContainEqual(expect.objectContaining({ action: 'set_weapons_restraint', held: true,
    control_system_id: 'radar-instance', semantic_action: 'tactical.weapons-restraint', correlation: expect.any(String) }));
  await page.evaluate(value => window.__updateConsole('tactical', JSON.stringify(value)), { ...payload, tactical_auto: false, weapons_restraint_auto: true, weapons_restrained: true });
  await expect(control).toBeDisabled(); await expect(control).toHaveAttribute('aria-pressed', 'true');
});
