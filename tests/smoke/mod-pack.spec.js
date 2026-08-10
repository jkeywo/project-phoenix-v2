// Issue #991 — Mod packs 6/6: browser drop / apply / precedence smoke.
//
// Drives the REAL host page (server.html) through the pre-scenario mod-pack
// upload UI (#760/#986–#990): `setInputFiles` on the host's #mod-pack-file,
// then asserts the rendered status, the applied-pack list, the validation
// findings, whether the catalog gains the mod scenario, two-pack precedence by
// which world the host actually resolves, that a connected phone receives the
// active-pack list, and that an upload after world load is ignored.
//
// Every committed fixture under tests/fixtures/mod-packs/ is exercised here AND
// by a Rust `validate_mod_pack` test over the SAME bytes (src/world/mod_pack.rs),
// so the two consumers cannot drift (issue #991 AC). Expectations are read out
// of each pack's own manifest (readModPackManifest + tomlString/tableArrayValues)
// rather than pinned — the derive-don't-pin rule from fixtures.js.
//
// Needs a built dist (`TRUNK_BUILD_RELEASE=true trunk build --release` +
// `node scripts/build-client.mjs`) served by the smoke suite's webServer, per
// AGENTS.md — exactly like demo-manifest.spec.js. This spec is written to that
// convention; a full smoke run requires the built dist.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  readModPackManifest,
  modPackFixturePath,
  tomlString,
  tableArrayValues,
  stripHeavyEntities,
} from './fixtures';
import { ts } from './strings';
import fs from 'fs';
import path from 'path';

const asset = (rel) => fs.readFileSync(path.join(__dirname, '../..', rel), 'utf-8');
const COMBAT_TEST_TOML = asset('assets/worlds/combat_test.toml');

// Scenario buttons only — `.world-btn` alone also matches the static
// #mod-pack-btn, so scope to the scenario-only data attribute (as
// demo-manifest.spec.js does).
const scenarioButtons = (page) => page.locator('#world-list .world-btn[data-scenario-id]');
const scenarioButton = (page, id) =>
  page.locator(`#world-list .world-btn[data-scenario-id="${id}"]`);

// Track requests to a specific mod world path — the analog of
// demo-manifest.spec.js's hullTemplateRequests. A mod pack's worlds live only in
// the in-memory session overlay and must NEVER be fetched over the network,
// least of all a REJECTED pack's. Returns the live array of matching URLs.
function worldRequests(context, worldPath) {
  const requested = [];
  const suffix = worldPath.startsWith('/') ? worldPath : `/${worldPath}`;
  context.on('request', (req) => {
    const url = req.url().split('?')[0];
    if (url.endsWith(suffix)) requested.push(url);
  });
  return requested;
}

// Open the host at the QR-first scenario stage — no `?scenario` bypass, no world
// load — and wait until the scenario buttons render. Their presence means the
// #754 catalog built, which means WASM is instantiated (`wasmReady`), so a
// mod-pack upload will be honoured.
async function openScenarioStage(context) {
  const page = await context.newPage();
  await page.goto('/');
  await page.bringToFront();
  await scenarioButtons(page).first().waitFor({ state: 'visible', timeout: 30_000 });
  return page;
}

const uploadPack = (page, name) =>
  page.locator('#mod-pack-file').setInputFiles(modPackFixturePath(name));

// ── Valid pack: host UI + connected phone + #990 provenance ───────────────────

test('valid pack: host shows the applied pack, its scenario joins the catalog, and a connected phone gets the active-pack list + mod origin', async ({
  context,
}) => {
  const manifest = readModPackManifest('valid-v1');
  const packId = tomlString(manifest, 'pack', 'id');
  const packName = tomlString(manifest, 'pack', 'name');
  const packVersion = tomlString(manifest, 'pack', 'version');
  const scenarioId = tableArrayValues(manifest, 'scenario', 'id')[0];

  const page = await openScenarioStage(context);
  const hostId = await readHostPeerId(page);
  // A phone that connects DURING the scenario stage never gets a Welcome (Bevy
  // is not running); it gets the synthesized ScenarioCatalog instead.
  const phone = await createTestClient(context, hostId, {
    name: 'Phone',
    waitFor: 'ScenarioCatalog',
  });

  const before = await scenarioButtons(page).count();
  await uploadPack(page, 'valid-v1');

  // Status + applied-pack row carrying the pack's name and version.
  await expect(page.locator('#mod-pack-status')).toContainText(ts('server.mod_pack_applied'));
  await expect(page.locator('#mod-pack-list')).toContainText(packName);
  await expect(page.locator('#mod-pack-list')).toContainText(packVersion);
  await expect(page.locator('#mod-pack-findings .mod-pack-finding.error')).toHaveCount(0);

  // The mod scenario joins the catalog with its own button.
  await expect(scenarioButton(page, scenarioId)).toHaveCount(1);
  await expect(scenarioButtons(page)).toHaveCount(before + 1);

  // The connected phone receives the updated catalog: the active-pack list and
  // the mod scenario's provenance — the data the #990 origin badge renders from.
  await phone.page.waitForFunction(
    (id) =>
      window.__messages?.some(
        (m) => m.type === 'ScenarioCatalog' && (m.data.active_packs || []).some((p) => p.id === id),
      ),
    packId,
    { timeout: 15_000 },
  );
  const cat = await phone.lastMessage('ScenarioCatalog');
  const active = (cat.data.active_packs || []).find((p) => p.id === packId);
  expect(active).toMatchObject({ id: packId, name: packName, version: packVersion });
  const modScenario = (cat.data.scenarios || []).find((s) => s.id === scenarioId);
  expect(modScenario, 'the mod scenario reached the phone catalog').toBeTruthy();
  expect(modScenario.source).toBe(packId);

  await phone.close();
});

// ── Every other accepted fixture also lands its scenario ──────────────────────

for (const name of ['script-valid', 'editor-round-trip']) {
  test(`accepted pack (${name}): status applied, no error findings, scenario joins the catalog`, async ({
    context,
  }) => {
    const manifest = readModPackManifest(name);
    const scenarioId = tableArrayValues(manifest, 'scenario', 'id')[0];

    const page = await openScenarioStage(context);
    const before = await scenarioButtons(page).count();
    await uploadPack(page, name);

    await expect(page.locator('#mod-pack-status')).toContainText(ts('server.mod_pack_applied'));
    await expect(page.locator('#mod-pack-findings .mod-pack-finding.error')).toHaveCount(0);
    await expect(scenarioButton(page, scenarioId)).toHaveCount(1);
    await expect(scenarioButtons(page)).toHaveCount(before + 1);
  });
}

// ── Every rejected fixture: findings in the DOM, catalog unchanged, no fetch ───

const REJECTED = [
  { name: 'format-too-new', category: 'unsupported-pack-format' },
  { name: 'content-epoch-mismatch', category: 'pack-content-mismatch' },
  { name: 'disallowed-path', category: 'disallowed-path' },
  { name: 'corrupt-crc', category: 'invalid-archive' },
  { name: 'schema-invalid-world', category: 'unparseable-scenario-world' },
  { name: 'unresolved-manifest-world', category: 'missing-scenario-world' },
  { name: 'script-denied-capability', category: 'denied-script-capability' },
];

for (const { name, category } of REJECTED) {
  test(`rejected pack (${name}): findings rendered, catalog count unchanged, mod world never fetched`, async ({
    context,
  }) => {
    const manifest = readModPackManifest(name);
    const modWorld = tableArrayValues(manifest, 'scenario', 'world')[0];
    // Attach the network watcher before any navigation, so a stray fetch of the
    // mod world at any point in the flow is caught.
    const requests = worldRequests(context, modWorld);

    const page = await openScenarioStage(context);
    const before = await scenarioButtons(page).count();
    await uploadPack(page, name);

    // The validator's finding category is rendered into #mod-pack-findings, and
    // the pack is rejected (atomic) — nothing applied.
    await expect(page.locator('#mod-pack-findings')).toContainText(category);
    await expect(page.locator('#mod-pack-findings .mod-pack-finding.error')).not.toHaveCount(0);
    await expect(page.locator('#mod-pack-status')).toContainText(ts('server.mod_pack_rejected'));

    // Catalog count unchanged, and the mod world was never requested over the
    // network (same technique as hullTemplateRequests).
    await expect(scenarioButtons(page)).toHaveCount(before);
    expect(requests, `mod world ${modWorld} must never be fetched for a rejected pack`).toHaveLength(
      0,
    );
  });
}

// ── Two-pack precedence + reorder ─────────────────────────────────────────────

test('two-pack precedence: the later pack wins the shared world path, and reordering in the host UI flips it', async ({
  context,
}) => {
  const aManifest = readModPackManifest('overlap-a');
  const bManifest = readModPackManifest('overlap-b');
  const aPackId = tomlString(aManifest, 'pack', 'id');
  const bPackId = tomlString(bManifest, 'pack', 'id');
  const aScenarioId = tableArrayValues(aManifest, 'scenario', 'id')[0];
  const sharedWorld = tableArrayValues(aManifest, 'scenario', 'world')[0];

  // The shared world's [global].title differs per pack — read from each fixture's
  // SOURCE so the assertion is "the resolved world is pack X's", derived not
  // pinned. Both scenarios point at the same path, so the button that renders it
  // shows the WINNING pack's title.
  const aTitle = tomlString(asset(`tests/fixtures/mod-packs/src/overlap-a/${sharedWorld}`), 'global', 'title');
  const bTitle = tomlString(asset(`tests/fixtures/mod-packs/src/overlap-b/${sharedWorld}`), 'global', 'title');
  expect(aTitle, 'the overlap fixtures must differ, or precedence is unobservable').not.toBe(bTitle);

  const page = await openScenarioStage(context);

  await uploadPack(page, 'overlap-a');
  await expect(scenarioButton(page, aScenarioId)).toHaveCount(1);
  await uploadPack(page, 'overlap-b');

  // overlap-b loaded last → it wins the shared path, so overlap_a's button (whose
  // label is that world's title) renders pack B's title.
  const aBtn = scenarioButton(page, aScenarioId);
  await expect(aBtn).toHaveText(bTitle);
  // The host UI names the conflict winner + shadowed loser.
  await expect(page.locator('#mod-pack-conflicts')).toContainText(
    ts('server.mod_pack_conflict_entry', { path: sharedWorld, winner: bPackId, losers: aPackId }),
  );

  // Reorder in the host UI: move pack A (row 0, oldest) DOWN so it becomes newest
  // and wins the shared path. The rendered world must flip to pack A's title.
  const firstRow = page.locator('#mod-pack-list .mod-pack-row').first();
  await firstRow.getByRole('button', { name: ts('server.mod_pack_move_down'), exact: true }).click();

  await expect(aBtn).toHaveText(aTitle);
  await expect(page.locator('#mod-pack-conflicts')).toContainText(
    ts('server.mod_pack_conflict_entry', { path: sharedWorld, winner: aPackId, losers: bPackId }),
  );
});

// ── Upload after world load is ignored ────────────────────────────────────────

test('upload after world load is ignored: the panel is hidden and no pack is applied', async ({
  context,
}) => {
  // The curated demo manifest resolves to a single scenario + single hull, so
  // clicking the scenario drives straight into world load (demo-manifest.spec.js
  // pattern). Strip combat_test's heavy entities so the load stays cheap.
  await context.route('**/assets/worlds/combat_test.toml', (route) =>
    route.fulfill({ contentType: 'text/plain', body: stripHeavyEntities(COMBAT_TEST_TOML) }),
  );

  const page = await context.newPage();
  await page.goto('/?manifest=assets/scenarios.demo.toml');
  await page.bringToFront();
  const buttons = scenarioButtons(page);
  await buttons.first().waitFor({ state: 'visible', timeout: 30_000 });
  await buttons.first().click();

  // World load latched (_worldLoadStarted): the scenario panel — which HOSTS the
  // mod-pack upload controls — is hidden.
  await expect(page.locator('#scenario-panel')).toBeHidden({ timeout: 30_000 });

  // Attempt an upload anyway. The change handler's _worldLoadStarted guard drops
  // it before any wasm_add_mod_pack call, so nothing changes.
  await uploadPack(page, 'valid-v1');
  await expect(page.locator('#mod-pack-status')).toBeEmpty();
  const packCount = await page.evaluate(() => {
    try {
      const r = window.wasmBindings.wasm_active_pack_manifest();
      return r && r.packs ? Array.from(r.packs).length : 0;
    } catch (_) {
      return -1;
    }
  });
  expect(packCount, 'no pack may be applied once the world has loaded').toBe(0);
});
