// Issue #917 — Smoke coverage: production-style demo build shows exactly the
// curated scenario/hull surface.
//
// `?manifest=<path>` (server.html's resolveManifestPath()) selects an
// alternate scenario manifest instead of the base `assets/scenarios.toml`.
// `assets/scenarios.demo.toml` curates the catalogue down to `combat_test`
// with a single playable hull (the Alliance Destroyer) via the manifest's
// `ships` allowlist — combat_test.toml itself is never edited and still
// authors all four Alliance hulls (destroyer/cruiser/battleship/courier).
//
// This exercises the QR-first pre-load catalog flow (buildScenarioCatalog /
// renderScenarioLockState / the scenario-arbiter), NOT the `?scenario=<path>`
// dev/test bypass that other specs (e.g. combat-test-scenario.spec.ts) use to
// skip straight into a world — that bypass never touches the manifest at all.
//
// Needs a built dist (`TRUNK_BUILD_RELEASE=true trunk build --release` +
// `node scripts/build-client.mjs`) served by the smoke suite's webServer, per
// AGENTS.md — this spec is written but intentionally not run here.

import { test, expect, readHostPeerId, waitForWasmReady, stripHeavyEntities } from './fixtures';
import fs from 'fs';
import path from 'path';

const COMBAT_TEST_TOML = fs.readFileSync(
  path.join(__dirname, '../../assets/worlds/combat_test.toml'),
  'utf-8',
);

// Only the Alliance entity template TOMLs matter for the "which hull got
// requested" assertions below.
function allianceHullRequests(context: import('@playwright/test').BrowserContext) {
  const requested: string[] = [];
  context.on('request', (req) => {
    const url = req.url();
    if (/\/assets\/entities\/alliance_[a-z_]+\.toml(\?.*)?$/.test(url)) {
      requested.push(url);
    }
  });
  return requested;
}

test('demo manifest (?manifest=assets/scenarios.demo.toml): host offers only combat_test and resolves straight to the Alliance Destroyer', async ({ context }) => {
  // Strip heavy entities from combat_test.toml (asteroid fields / planet /
  // sun pull in ~150 MB of GLBs) so the world load stays fast, same as
  // combat-test-scenario.spec.ts. assets/scenarios.demo.toml itself is the
  // real, tiny, committed manifest — no need to stub it.
  await context.route('**/assets/worlds/combat_test.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: stripHeavyEntities(COMBAT_TEST_TOML) }),
  );
  const hullRequests = allianceHullRequests(context);

  const serverPage = await context.newPage();
  await serverPage.goto('/?manifest=assets/scenarios.demo.toml');
  await serverPage.bringToFront();

  // Scenario stage: exactly one option, combat_test. `.world-btn` alone would
  // also match the always-present #mod-pack-btn, so scope to the
  // scenario-only data attribute.
  const scenarioButtons = serverPage.locator('#world-list .world-btn[data-scenario-id]');
  await scenarioButtons.first().waitFor({ state: 'visible', timeout: 30_000 });
  await expect(scenarioButtons).toHaveCount(1);
  await expect(scenarioButtons.first()).toHaveAttribute('data-scenario-id', 'combat_test');

  await scenarioButtons.first().click();

  // Ship stage never appears: the demo manifest curates combat_test to one
  // hull, so the pre-load arbiter auto-resolves it (server.html
  // renderScenarioLockState) instead of showing <ph-ship-picker>.
  await expect(serverPage.locator('#scenario-panel ph-ship-picker')).toHaveCount(0);
  await expect(serverPage.locator('#scenario-panel')).toBeHidden({ timeout: 30_000 });

  await waitForWasmReady(serverPage);
  const hostId = await readHostPeerId(serverPage);
  expect(hostId).toBeTruthy();

  // Only the destroyer's template TOML was ever fetched — the cruiser,
  // battleship, and courier hulls combat_test.toml also authors are never
  // requested, confirming the curation actually narrowed the playable hull
  // and didn't just narrow what the UI happens to display.
  await expect
    .poll(() => hullRequests.some((u) => u.endsWith('/assets/entities/alliance_destroyer.toml')))
    .toBe(true);
  for (const other of ['alliance_cruiser.toml', 'alliance_battleship.toml', 'alliance_courier.toml']) {
    expect(hullRequests.some((u) => u.endsWith(`/assets/entities/${other}`)), `unexpected request for ${other}`).toBe(false);
  }
});

test('dev/default (no ?manifest param): host still offers the full base catalogue', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.goto('/');
  await serverPage.bringToFront();

  const scenarioButtons = serverPage.locator('#world-list .world-btn[data-scenario-id]');
  await scenarioButtons.first().waitFor({ state: 'visible', timeout: 30_000 });

  // assets/scenarios.toml ships three selectable roots today (default,
  // combat_test, before_the_fire) — asserted by the Rust
  // shipped_manifest_parses_and_validates test too. Checking the ids rather
  // than a bare count also confirms combat_test is NOT curated down when the
  // demo manifest isn't selected.
  const ids = await scenarioButtons.evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.scenarioId));
  expect(ids.sort()).toEqual(['before_the_fire', 'combat_test', 'default']);
});
