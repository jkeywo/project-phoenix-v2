// Issue #917 — Smoke coverage: production-style demo build shows exactly the
// curated scenario/hull surface.
//
// `?manifest=<path>` (server.html's resolveManifestPath()) selects an
// alternate scenario manifest instead of the base `assets/scenarios.toml`.
// `assets/scenarios.demo.toml` curates the catalogue down to one scenario with
// a single playable hull via the manifest's `ships` allowlist — the referenced
// world TOML itself is never edited and still authors its full hull list.
//
// Which scenario, and which hull, is authored data and is read out of the two
// manifests below rather than pinned here (issue #941).
//
// This exercises the QR-first pre-load catalog flow (buildScenarioCatalog /
// renderScenarioLockState / the scenario-arbiter), NOT the `?scenario=<path>`
// dev/test bypass that other specs (e.g. combat-test-scenario.spec.js) use to
// skip straight into a world — that bypass never touches the manifest at all.
//
// Needs a built dist (`TRUNK_BUILD_RELEASE=true trunk build --release` +
// `node scripts/build-client.mjs`) served by the smoke suite's webServer, per
// AGENTS.md — this spec is written but intentionally not run here.

import {
  test,
  expect,
  readHostPeerId,
  waitForWasmReady,
  stripHeavyEntities,
  tableArrayValues,
} from './fixtures';
import fs from 'fs';
import path from 'path';

const asset = (rel) => fs.readFileSync(path.join(__dirname, '../..', rel), 'utf-8');

const COMBAT_TEST_TOML = asset('assets/worlds/combat_test.toml');

// This spec is about the *manifest mechanism*, so the two shipped manifests are
// the subject rather than incidental data — but their contents are still
// authored and still move. Both expectations below are therefore read out of
// the manifests instead of pinned (issue #941): a scenario added to
// `scenarios.toml` must show up in the catalogue, which is the behaviour worth
// asserting, and the assertion updates itself when the roster grows.
const BASE_SCENARIO_IDS = tableArrayValues(asset('assets/scenarios.toml'), 'scenario', 'id');
const DEMO_SCENARIO_IDS = tableArrayValues(asset('assets/scenarios.demo.toml'), 'scenario', 'id');

// The demo manifest's `ships` allowlist, and the full hull list the referenced
// world authors — both read from the committed TOML so this spec asserts
// "curation narrowed X to Y" rather than "curation produced the destroyer".
const CURATED_SHIPS = (() => {
  const m = asset('assets/scenarios.demo.toml').match(/^\s*ships\s*=\s*\[([^\]]*)\]/m);
  return m ? Array.from(m[1].matchAll(/"([^"]+)"/g)).map((x) => x[1]) : [];
})();
const WORLD_SHIPS = tableArrayValues(COMBAT_TEST_TOML, 'available_ships', 'template_path');
const CURATED_AWAY = WORLD_SHIPS.filter((s) => !CURATED_SHIPS.includes(s));

// Only the entity template TOMLs matter for the "which hull got requested"
// assertions below.
function hullTemplateRequests(context) {
  const requested = [];
  context.on('request', (req) => {
    const url = req.url();
    if (/\/assets\/entities\/[a-z0-9_]+\.toml(\?.*)?$/.test(url)) {
      requested.push(url);
    }
  });
  return requested;
}

test('demo manifest (?manifest=assets/scenarios.demo.toml): host offers only the curated scenario and resolves straight to its single curated hull', async ({ context }) => {
  // The curation this spec exercises only exists when the manifest narrows the
  // world's hull list to exactly one — that is what makes the pre-load arbiter
  // skip the ship stage. Fail loudly rather than silently testing nothing if
  // the shipped demo manifest is ever re-curated.
  expect(
    DEMO_SCENARIO_IDS.length,
    'assets/scenarios.demo.toml no longer curates to a single scenario — this spec needs rewriting',
  ).toBe(1);
  expect(
    CURATED_SHIPS.length,
    'assets/scenarios.demo.toml no longer curates to a single hull — this spec needs rewriting',
  ).toBe(1);
  expect(
    CURATED_AWAY.length,
    'the curated world authors no hull the demo manifest excludes, so there is nothing to narrow',
  ).toBeGreaterThan(0);

  // Strip heavy entities from combat_test.toml (asteroid fields / planet /
  // sun pull in ~150 MB of GLBs) so the world load stays fast, same as
  // combat-test-scenario.spec.js. assets/scenarios.demo.toml itself is the
  // real, tiny, committed manifest — no need to stub it.
  await context.route('**/assets/worlds/combat_test.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: stripHeavyEntities(COMBAT_TEST_TOML) }),
  );
  const hullRequests = hullTemplateRequests(context);

  const serverPage = await context.newPage();
  await serverPage.goto('/?manifest=assets/scenarios.demo.toml');
  await serverPage.bringToFront();

  // Scenario stage: exactly the manifest's scenarios, no more. `.world-btn`
  // alone would also match the always-present #mod-pack-btn, so scope to the
  // scenario-only data attribute.
  const scenarioButtons = serverPage.locator('#world-list .world-btn[data-scenario-id]');
  await scenarioButtons.first().waitFor({ state: 'visible', timeout: 30_000 });
  await expect(scenarioButtons).toHaveCount(DEMO_SCENARIO_IDS.length);
  await expect(scenarioButtons.first()).toHaveAttribute('data-scenario-id', DEMO_SCENARIO_IDS[0]);

  await scenarioButtons.first().click();

  // Ship stage never appears: the demo manifest curates the scenario to one
  // hull, so the pre-load arbiter auto-resolves it (server.html
  // renderScenarioLockState) instead of showing <ph-ship-picker>.
  await expect(serverPage.locator('#scenario-panel ph-ship-picker')).toHaveCount(0);
  await expect(serverPage.locator('#scenario-panel')).toBeHidden({ timeout: 30_000 });

  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);
  expect(hostId).toBeTruthy();

  // Only the allowlisted hull's template TOML was ever fetched — every other
  // hull the world authors is never requested, confirming the curation
  // actually narrowed the playable hull and didn't just narrow what the UI
  // happens to display.
  await expect
    .poll(() => hullRequests.some((u) => u.endsWith(`/${CURATED_SHIPS[0]}`)))
    .toBe(true);
  for (const other of CURATED_AWAY) {
    expect(
      hullRequests.some((u) => u.endsWith(`/${other}`)),
      `unexpected request for ${other}`,
    ).toBe(false);
  }
});

test('dev/default (no ?manifest param): host still offers the full base catalogue', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.bringToFront();

  const scenarioButtons = serverPage.locator('#world-list .world-btn[data-scenario-id]');
  await scenarioButtons.first().waitFor({ state: 'visible', timeout: 30_000 });

  // Every selectable root `assets/scenarios.toml` declares must render a
  // button, and nothing else may. Read from the manifest rather than pinned
  // (issue #941): the previous version of this listed the three ids shipping
  // that week, so adding a fourth scenario broke a spec about the *manifest
  // mechanism*. Set equality still catches the failures worth catching — a
  // declared scenario missing from the catalogue, or one appearing that the
  // manifest never declared.
  const ids = await scenarioButtons.evaluateAll((els) =>
    els.map((el) => el.dataset.scenarioId),
  );
  expect(ids.slice().sort()).toEqual(BASE_SCENARIO_IDS.slice().sort());

  // The demo manifest curates a valid SUBSET of (or equal to) the base
  // catalogue: every scenario it offers is one the base offers too, and
  // `combat_test` is present in both. The two rosters currently coincide — the
  // demo build offers the whole base catalogue — so this no longer asserts a
  // divergence, only that curation never widens past the base. If the base
  // roster grows again the subset relation still holds without a rewrite.
  for (const id of DEMO_SCENARIO_IDS) {
    expect(
      BASE_SCENARIO_IDS,
      `demo scenario ${id} must also be offered by the base catalogue`,
    ).toContain(id);
  }
  expect(BASE_SCENARIO_IDS, 'combat_test must be offered by the base catalogue').toContain(
    'combat_test',
  );
  expect(DEMO_SCENARIO_IDS, 'combat_test must be offered by the demo catalogue').toContain(
    'combat_test',
  );
  for (const id of DEMO_SCENARIO_IDS) {
    expect(ids, `${id} must still be offered when no manifest is curated`).toContain(id);
  }
});
