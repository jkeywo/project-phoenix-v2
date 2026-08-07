// Smoke test: data-driven server audio.
//
// Audio playback lives in the host page's JS (Bevy audio was reverted
// in-browser), but every filename and tuning value comes from TOML that only
// Rust parses. This test verifies the whole chain end-to-end: ship/world TOML
// → EntityConfig/WorldConfig → ShipAudioSection on the LocalShip →
// AudioConfigChanged → "audio_config" host channel → window.__audioConfig →
// constructed <audio> elements.
//
// It asserts on the audio module's *state*, not on sound — headless Chromium
// can't be asked whether something is audible. The `__audioDebug()` hook in
// server.html exposes what's needed.
//
// Panning correctness (the yaw sign) is covered by unit tests in
// src/audio_config.rs; it can't be verified from here.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
  stripHeavyEntities,
  tomlString,
  tomlNumber,
} from './fixtures';
import fs from 'fs';
import path from 'path';

const asset = (rel: string) => fs.readFileSync(path.join(__dirname, '../..', rel), 'utf-8');

const COMBAT_TEST_TOML = asset('assets/worlds/combat_test.toml');

// The hull this test drives, picked by template path below so a roster reshuffle
// cannot silently swap it for a single-station one.
const SHIP_TEMPLATE = 'assets/entities/alliance_battleship.toml';
const SHIP_TOML = asset(SHIP_TEMPLATE);

// ── Every expectation is read out of the TOML, never written down twice ──────
//
// Issue #941: this spec used to pin the authored values — 'assets/sounds/
// Ambient.mp3', volume 0.25, thrust coefficient 0.15 — which made it break
// whenever a designer swapped a sound or retuned a mix. Worse, it made the
// central claim weak: the bug this spec exists to catch is JS hardcoding a
// value instead of reading the pushed config, and a hardcoded JS constant
// matching a pinned test constant passes.
//
// Comparing the wire against the TOML instead is both stabler and stricter:
// it fails the moment the two disagree, whichever one moved.
const EXPECTED = {
  ambient: tomlString(SHIP_TOML, 'audio.ambient', 'file'),
  engine: tomlString(SHIP_TOML, 'audio.engine', 'file'),
  blaster: tomlString(SHIP_TOML, 'audio.blaster', 'file'),
  phaserLoop: tomlString(SHIP_TOML, 'audio.phaser_loop', 'file'),
  forcefield: tomlString(SHIP_TOML, 'audio.forcefield', 'file'),
  ambientVolume: tomlNumber(SHIP_TOML, 'audio.ambient', 'volume'),
  phaserVolume: tomlNumber(SHIP_TOML, 'audio.phaser_loop', 'volume'),
  engineIdle: tomlNumber(SHIP_TOML, 'audio.engine', 'idle_volume'),
  engineFullThrust: tomlNumber(SHIP_TOML, 'audio.engine', 'volume_at_full_thrust'),
  // Red-alert audio comes from the *world* TOML, not the ship.
  siren: tomlString(COMBAT_TEST_TOML, 'audio.red_alert', 'siren_file'),
  music: tomlString(COMBAT_TEST_TOML, 'audio.red_alert', 'music_file'),
};

test('audio config is data-driven from ship + world TOML and builds the audio graph', async ({
  context,
}) => {
  await context.route('**/assets/worlds/combat_test.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: stripHeavyEntities(COMBAT_TEST_TOML) }),
  );

  const serverPage = await context.newPage();

  // An unclamped volume would surface here as an IndexSizeError, and a
  // bubbling decodeAudioData rejection would surface as an unhandled
  // rejection. Both must stay empty.
  const pageErrors: string[] = [];
  serverPage.on('pageerror', (e) => pageErrors.push(String(e)));

  const audioRequests: string[] = [];
  serverPage.on('request', (r) => {
    const u = r.url();
    if (/assets\/sounds\//.test(u)) audioRequests.push(u.split('/').pop() as string);
  });

  await serverPage.goto('/?scenario=assets/worlds/combat_test.toml');

  // combat_test.toml declares several [[available_ships]]. Deliberately pick
  // the crewed Battleship by its template path, not by card position — some
  // hulls are single-station and have no "Captain" station, so this test's
  // multi-station client flow below needs a crewed hull regardless of where
  // new ships get added to the roster.
  await serverPage.waitForSelector('#scenario-panel ph-ship-picker .ship-card', {
    state: 'visible',
    timeout: 60_000,
  });
  await serverPage.click(
    `#scenario-panel ph-ship-picker .ship-card[data-template="${SHIP_TEMPLATE}"]`,
  );

  await waitForWasmReady(serverPage);

  // The callbacks must be registered before the config push, which lands on
  // the InProgress edge.
  expect(
    await serverPage.evaluate(() => ({
      config: typeof (window as any).__audioConfig,
      cue: typeof (window as any).__audioCue,
      level: typeof (window as any).__audioLevel,
    })),
  ).toEqual({ config: 'function', cue: 'function', level: 'function' });

  // No hardcoded <audio> tags may remain in the markup — every element is
  // constructed from the pushed config. (The constructed ones are detached
  // `new Audio()` objects and never appear in the DOM, so a non-zero count
  // here means literal markup survived.)
  expect(await serverPage.locator('audio').count()).toBe(0);

  // Nothing is configured until the game starts.
  expect(await serverPage.evaluate(() => (window as any).__audioDebug().cfg)).toBeNull();

  const hostId = await readHostPeerId(serverPage);
  const captain = await createTestClient(context, hostId, { name: 'Cap' });
  await captain.send('SelectStation', { station: 'Captain' });
  await captain.page.waitForFunction(
    (t) =>
      (window as any).__messages?.some(
        (m: any) => m.type === 'StationAssigned' && m.data.token === t,
      ),
    captain.token,
    { timeout: 5_000 },
  );
  await captain.send('SetReady', { ready: true });
  await captain.waitForMessage('GameStarted', 10_000);

  // The config push rides the InProgress edge.
  await serverPage.waitForFunction(() => !!(window as any).__audioDebug().cfg, undefined, {
    timeout: 10_000,
  });

  const dbg = await serverPage.evaluate(() => (window as any).__audioDebug());

  // Ship-level sounds come from the selected ship's entity TOML...
  expect(dbg.cfg.ambient.file).toBe(EXPECTED.ambient);
  expect(dbg.cfg.engine.file).toBe(EXPECTED.engine);
  expect(dbg.cfg.blaster.file).toBe(EXPECTED.blaster);
  expect(dbg.cfg.phaser_loop.file).toBe(EXPECTED.phaserLoop);
  expect(dbg.cfg.forcefield.file).toBe(EXPECTED.forcefield);

  // ...and red-alert audio from the *world* TOML, not the ship. These being
  // different files is what makes the two halves of the chain distinguishable.
  expect(dbg.cfg.red_alert.siren_file).toBe(EXPECTED.siren);
  expect(dbg.cfg.red_alert.music_file).toBe(EXPECTED.music);
  expect(
    [EXPECTED.siren, EXPECTED.music].some((f) => f === EXPECTED.ambient),
    'the world and ship audio must name different files or this test cannot tell them apart',
  ).toBe(false);

  // The forcefield envelope stays server-side: JS gets the file and nothing
  // else, because Rust computes the level.
  expect(Object.keys(dbg.cfg.forcefield)).toEqual(['file']);

  // Blaster rolloff params must survive as Web Audio's exact spellings.
  expect(dbg.cfg.blaster.distance_model).toBe('inverse');
  expect(dbg.cfg.blaster.panning_model).toBe('equalpower');
  expect(typeof dbg.cfg.blaster.ref_distance).toBe('number');
  expect(typeof dbg.cfg.blaster.max_distance).toBe('number');

  // Elements were constructed and playback started.
  expect(dbg.els).toEqual(
    expect.arrayContaining(['ambient', 'engine', 'phaser', 'forcefield', 'music', 'siren']),
  );
  expect(dbg.started).toBe(true);

  // The configured files are actually fetched — by the name the TOML gives,
  // which is the whole point (a hardcoded JS filename would fetch its own).
  const ambientFile = EXPECTED.ambient.split('/').pop() as string;
  await serverPage.waitForFunction(
    (name: string) =>
      performance.getEntriesByType('resource').some((e) => e.name.endsWith(name)),
    ambientFile,
    { timeout: 10_000 },
  );
  expect(audioRequests).toEqual(expect.arrayContaining([ambientFile]));

  // Volumes must be inside the range HTMLMediaElement.volume accepts;
  // out-of-range throws IndexSizeError. Read them from the module, not from
  // document.querySelectorAll — the elements are detached and a DOM sweep
  // would return nothing and pass vacuously.
  expect(Object.keys(dbg.volumes).length).toBeGreaterThan(0);
  for (const [name, v] of Object.entries(dbg.volumes as Record<string, number>)) {
    expect(v, `${name} volume out of range`).toBeGreaterThanOrEqual(0);
    expect(v, `${name} volume out of range`).toBeLessThanOrEqual(1);
  }

  // The authored ambient volume must survive the trip from TOML to the
  // element — this is the value the JS used to hardcode.
  expect(dbg.volumes.ambient).toBeCloseTo(EXPECTED.ambientVolume, 5);
  // Engine starts at its authored idle volume until thrust rides it up.
  expect(dbg.volumes.engine).toBeCloseTo(EXPECTED.engineIdle, 5);

  // Music only plays under red alert, which hasn't fired.
  expect(dbg.musicPlaying).toBe(false);

  // The forcefield level callback clamps rather than throwing.
  const clamped = await serverPage.evaluate(() => {
    const a = window as any;
    a.__audioLevel(5);
    const high = a.__audioDebug().volumes.forcefield;
    a.__audioLevel(-2);
    const low = a.__audioDebug().volumes.forcefield;
    a.__audioLevel(0.4);
    const mid = a.__audioDebug().volumes.forcefield;
    return { high, low, mid };
  });
  expect(clamped).toEqual({ high: 1, low: 0, mid: 0.4 });

  // ── HUD-driven audio state ─────────────────────────────────────────────
  // Drive __updateHud directly: reaching real red alert / phaser fire from a
  // smoke test would need a full combat setup, and the state->sound mapping
  // is what's under test here, not the sim that produces the state.
  const hud = (over: Record<string, unknown>) => ({
    heading: 0,
    hull_pct: 100,
    condition: 'NOMINAL',
    red_alert: false,
    engine_thrust: 0,
    phaser_firing: false,
    ...over,
  });

  // Red alert on: music starts and loops under the siren.
  const alertOn = await serverPage.evaluate((s) => {
    (window as any).__updateHud(JSON.stringify(s));
    return (window as any).__audioDebug();
  }, hud({ red_alert: true, condition: 'ALERT' }));
  expect(alertOn.musicPlaying).toBe(true);

  // Red alert clear: music stops.
  const alertOff = await serverPage.evaluate((s) => {
    (window as any).__updateHud(JSON.stringify(s));
    return (window as any).__audioDebug();
  }, hud({ red_alert: false }));
  expect(alertOff.musicPlaying).toBe(false);

  // Phaser firing: the loop starts at its authored volume, then stops.
  const firing = await serverPage.evaluate((s) => {
    (window as any).__updateHud(JSON.stringify(s));
    return (window as any).__audioDebug();
  }, hud({ phaser_firing: true }));
  expect(firing.phaserPlaying).toBe(true);
  expect(firing.volumes.phaser).toBeCloseTo(EXPECTED.phaserVolume, 5);

  const ceased = await serverPage.evaluate((s) => {
    (window as any).__updateHud(JSON.stringify(s));
    return (window as any).__audioDebug();
  }, hud({ phaser_firing: false }));
  expect(ceased.phaserPlaying).toBe(false);

  // Engine volume tracks thrust using the TOML coefficients
  // (idle_volume + thrust * volume_at_full_thrust), not a hardcoded constant.
  // `idle_volume + thrust * volume_at_full_thrust` — the formula named in
  // src/audio_config.rs and applied in server.html's __updateHud.
  const engineVolumeAt = (thrust: number) =>
    EXPECTED.engineIdle + thrust * EXPECTED.engineFullThrust;

  const thrusting = await serverPage.evaluate((s) => {
    (window as any).__updateHud(JSON.stringify(s));
    return (window as any).__audioDebug();
  }, hud({ engine_thrust: 1.0 }));
  expect(thrusting.volumes.engine).toBeCloseTo(engineVolumeAt(1.0), 5);

  const halfThrust = await serverPage.evaluate((s) => {
    (window as any).__updateHud(JSON.stringify(s));
    return (window as any).__audioDebug();
  }, hud({ engine_thrust: 0.5 }));
  expect(halfThrust.volumes.engine).toBeCloseTo(engineVolumeAt(0.5), 5);

  expect(pageErrors, `unexpected page errors: ${pageErrors.join('\n')}`).toEqual([]);

  await captain.close();
});
