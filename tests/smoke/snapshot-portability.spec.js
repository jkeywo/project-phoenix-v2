// Issue #866 — portable saves: export a file from a running host, import it
// into a fresh one, and refuse the two bad files differently.
//
// Drives the REAL host page (server.html): the settings cog's export command,
// the scenario panel's import chooser, and the version gate that runs between
// the world load and `wasm_init`. What the vitest suite
// (tests/client/snapshot-transfer.test.js) can only assert about the
// CLASSIFICATION of a refusal, this file asserts about the rendered page.
//
// The incompatible-file case leans on the property the issue asks to lean on:
// the artifact is RON TEXT, so this test makes an incompatible save by editing
// the exported one's format number. A test that could not read the artifact
// would have to fabricate an incompatible save some other way, and would then
// be proving something about the fabrication.
//
// Needs a built dist (`TRUNK_BUILD_RELEASE=true trunk build --release` +
// `node scripts/build-client.mjs`) served by the smoke suite's webServer, per
// AGENTS.md — the same convention as mod-pack.spec.js.

import {
  test,
  expect,
  readHostPeerId,
  createTestClient,
  waitForWasmReady,
} from './fixtures';
import { ts } from './strings';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// The world every other smoke spec boots, and the one the `context` fixture
// routes to its minimal stand-in — so the save exported here names a world the
// importing page can also load.
const WORLD = 'assets/worlds/default.toml';

/** Scenario buttons only — `.world-btn` alone also matches the static
 *  #mod-pack-btn and #snapshot-import-btn, so scope to the scenario-only data
 *  attribute (as mod-pack.spec.js does). */
const scenarioButtons = (page) => page.locator('#world-list .world-btn[data-scenario-id]');

/**
 * The fixed half of a string-table sentence that carries a `{detail}`.
 *
 * Derived from the table rather than pasted, so a copy edit moves the assertion
 * with it and only a RENAMED id fails — the same contract `ts` exists for. The
 * detail itself is Rust's composed sentence and is asserted separately where it
 * matters.
 */
function frameOf(id) {
  const MARK = '<<detail>>';
  return ts(id, { detail: MARK }).split(MARK)[0].replace(/^\[/, '').trim();
}

/** Write `text` to a scratch file the file chooser can be pointed at. */
function scratchFile(name, text) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'phoenix-save-'));
  const file = path.join(dir, name);
  fs.writeFileSync(file, text, 'utf-8');
  return file;
}

/** Bring a host up on `WORLD` and run it to a started game, so there is a
 *  session with a roster to export. A save taken in the lobby is refused by
 *  design (issue #934), so the export half of this file needs a real run. */
async function startedHost(context) {
  const page = await context.newPage();
  await page.goto(`/?scenario=${WORLD}`);
  await waitForWasmReady(page);

  const hostId = await readHostPeerId(page);
  const helm = await createTestClient(context, hostId, { name: 'Helm' });
  await helm.send('SelectStation', { station: 'Helm' });
  await helm.page.waitForFunction(
    (t) => window.__messages?.some((m) => m.type === 'StationAssigned' && m.data.token === t),
    helm.token,
    { timeout: 10_000 },
  );
  await helm.send('SetReady', { ready: true });
  await helm.waitForMessage('GameStarted', 20_000);
  return page;
}

/** Open a host at the QR-first scenario stage — no `?scenario=` bypass and no
 *  world load — which is where an import is offered. */
async function scenarioStage(context) {
  const page = await context.newPage();
  await page.goto('/');
  await page.bringToFront();
  await scenarioButtons(page).first().waitFor({ state: 'visible', timeout: 30_000 });
  return page;
}

/** Click the host's export command and return the downloaded file's text. */
async function exportSave(page) {
  const pending = page.waitForEvent('download', { timeout: 30_000 });
  await page.evaluate(() => window.__hostExportSnapshot());
  const download = await pending;
  const at = await download.path();
  return { text: fs.readFileSync(at, 'utf-8'), suggested: download.suggestedFilename() };
}

/** Whether the scenario panel is still on screen — i.e. no world load started.
 *
 *  Computed `display`, not `offsetParent`: the panel is positioned, so
 *  `offsetParent` is null whether it is showing or not, and a check written on
 *  it answers "gone" every time — including for the wait that is supposed to
 *  block until a world load actually starts. */
const stillPicking = (page) =>
  page.evaluate(() => {
    const el = document.getElementById('scenario-panel');
    return !!el && getComputedStyle(el).display !== 'none';
  });

/** Whether a save passed the gate and is staged for the boot in progress. */
const staged = (page) =>
  page.evaluate(() => !!(window.wasm_resume_pending && window.wasm_resume_pending()));

// ── The round trip ───────────────────────────────────────────────────────────

test('a save exported from a running host imports into a fresh one and is staged for restore', async ({
  context,
}) => {
  const host = await startedHost(context);
  const { text, suggested } = await exportSave(host);

  // RON text, not opaque bytes — the issue's own framing, and the reason the
  // incompatible case below can edit the artifact at all.
  expect(suggested).toBe('phoenix-save.ron');
  expect(text).toContain(WORLD);
  expect(text.trimStart().startsWith('(')).toBe(true);

  // A genuinely fresh page: no `?scenario=`, nothing loaded, the imported file
  // is the only thing that has told it anything.
  const fresh = await scenarioStage(context);
  expect(await stillPicking(fresh)).toBe(true);
  await fresh
    .locator('#snapshot-import-file')
    .setInputFiles(scratchFile('phoenix-save.ron', text));

  // The file named its own world, so the page left the picker without being
  // told which one — the first half of the gate's ordering.
  await fresh.waitForFunction(
    () => {
      const el = document.getElementById('scenario-panel');
      return !el || getComputedStyle(el).display === 'none';
    },
    null,
    { timeout: 30_000 },
  );

  // …and the second half: the version gate passed and the save is staged,
  // waiting for the roster to bootstrap. `wasm_resume_pending` is the same
  // question a local-slot resume answers.
  await fresh.waitForFunction(
    () => !!(window.wasm_resume_pending && window.wasm_resume_pending()),
    null,
    { timeout: 60_000 },
  );

  // Nothing was refused on the way in.
  const chooser = (await fresh.locator('#snapshot-import-status').textContent()) || '';
  expect(chooser).not.toContain(frameOf('server.import_snapshot_damaged'));
  const session = (await fresh.locator('#snapshot-status').textContent()) || '';
  expect(session).not.toContain(frameOf('server.import_snapshot_incompatible'));
});

// ── The two refusals ─────────────────────────────────────────────────────────

test('a damaged file is refused as damaged, before any world is loaded on its behalf', async ({
  context,
}) => {
  const fresh = await scenarioStage(context);

  await fresh
    .locator('#snapshot-import-file')
    .setInputFiles(scratchFile('broken.ron', '(scenario: "assets/worlds/default.toml", trunc'));

  await expect(fresh.locator('#snapshot-import-status')).toContainText(
    frameOf('server.import_snapshot_damaged'),
    { timeout: 10_000 },
  );

  // And the scenario panel is still up: a file that could not be read must not
  // cost the host a world load, which is the whole reason the parse happens
  // before the gate rather than inside it.
  expect(await stillPicking(fresh)).toBe(true);
  expect(await staged(fresh)).toBe(false);
});

test('an intact save this build cannot honour is refused as incompatible, naming the dimension', async ({
  context,
}) => {
  const host = await startedHost(context);
  const { text } = await exportSave(host);

  // Edit the artifact — possible only because it is text — so it is a
  // WELL-FORMED save from a build whose payload shape moved. `format` is
  // `vellum_save::Versions`' own field, and moving it is exactly the case AC5
  // separates from a damaged file.
  const moved = text.replace(/format:\s*(\d+)/, (_, n) => `format:${Number(n) - 1}`);
  expect(moved).not.toBe(text);

  const fresh = await scenarioStage(context);
  await fresh.locator('#snapshot-import-file').setInputFiles(scratchFile('older.ron', moved));

  // It PARSES, so the page loads the world it names; the refusal comes from the
  // version gate afterwards rather than from the chooser. That ordering is the
  // difference between the two classes made visible.
  await fresh.waitForFunction(
    () => {
      const el = document.getElementById('scenario-panel');
      return !el || getComputedStyle(el).display === 'none';
    },
    null,
    { timeout: 30_000 },
  );

  await expect(fresh.locator('#snapshot-status')).toContainText(
    frameOf('server.import_snapshot_incompatible'),
    { timeout: 60_000 },
  );
  // Carrying `vellum_save::Moved`'s own sentence inside the frame — which names
  // WHICH dimension moved, and is the only part of the message worth reading.
  await expect(fresh.locator('#snapshot-status')).toContainText('format', { timeout: 5_000 });

  // Refused means nothing was staged: the page is playing a fresh session, not
  // a half-adopted one.
  expect(await staged(fresh)).toBe(false);
});
