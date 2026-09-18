import { test, expect } from '@playwright/test';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST } from '../fixtures/workshop-pack.js';
import { ts } from './strings';
import { revealWorkshopPanel } from './dock-helpers.js';

// Issue #1475, criteria 3 and 4: a composition edit the runtime refuses leaves
// the source untouched, and the composition it accepts is what a disposable Test
// runs. The draft imports two child worlds nothing composes yet — one that would
// close a cycle, one that spawns a hull — and the panel composes them through the
// REAL wasm_workshop_compose over the exact source. Unit tests of this panel mock
// that runtime; this is the run that does not.
const WORLD = 'assets/worlds/workshop.toml';
const CHILD = 'assets/worlds/workshop_child.toml';
const LOOP = 'assets/worlds/workshop_loop.toml';
const SHIP = 'assets/entities/alliance_cruiser.toml';
// A hull the child world spawns, named with a literal the String Table has never
// heard of, so the GM roster can only be showing what this draft authored.
const PATROL = 'Workshop Child Patrol';
const isTestFrame = frame => /\/workshop-test(?:\.html)?$/.test(new URL(frame.url() || 'about:blank').pathname);
const ROOT_TEXT = `# Keep this comment on the root world
[global]
title = "Composed in the Workshop"
seed = 5
[[available_ships]]
template_path = "${SHIP}"
[[entity]]
template_path = "${SHIP}"
id = "player-ship"
spawn_on = "game_start"
`;
// A layer's entities spawn with the layer, so the child authors no `spawn_on`:
// an extra world is applied through apply_loaded_layer, which spawns only the
// Immediate ones, and `game_start` is read from the ROOT config alone.
const CHILD_TEXT = `# The child world, composed only by the draft
[global]
title = "Workshop child"
[[entity]]
template_path = "${SHIP}"
id = "child-patrol"
name = "${PATROL}"
transform = { position = [400.0, 0.0, -600.0] }
`;
// Listing the root back makes this one a cycle the moment the root lists it.
// `extra_worlds` is a ROOT-table key, so it has to precede the first table
// header: written under `[global]` it belongs to that table and the loader never
// reads it — which is exactly how an earlier version of this fixture composed
// without closing a cycle at all.
const LOOP_TEXT = `extra_worlds = ["${WORLD}"]
[global]
title = "Workshop loop"
`;
const sourcePack = () => createStoreZip([
  { path: 'scenarios.toml', text: WORKSHOP_MANIFEST },
  { path: WORLD, text: ROOT_TEXT },
  { path: CHILD, text: CHILD_TEXT },
  { path: LOOP, text: LOOP_TEXT },
]);
const byId = (page, id) => page.locator(`#workshop-composition-${id}`);
// The words a rule has to carry, taken from the String Table rather than
// transcribed. The sentence interpolates the runtime's own detail in the middle,
// so only the part BEFORE it is a contiguous substring of what the panel shows:
// a sentinel detail marks where to cut. It is ordinary text on purpose: a control
// byte in a source file makes git treat the whole spec as binary.
const DETAIL_SENTINEL = '<<<detail>>>';
const refusalWords = id => ts(id, { detail: DETAIL_SENTINEL }).split(DETAIL_SENTINEL)[0].trim();

/** The exact source of one member. `#workshop-files` lives in the files panel,
 * which shares its dock group with `composition` — so the files tab has to be
 * brought back to the front before the select is reachable at all. */
async function sourceOf(page, member) {
  await revealWorkshopPanel(page, 'files');
  await page.locator('#workshop-files').selectOption(member);
  await revealWorkshopPanel(page, 'source');
  return page.locator('#workshop-source').inputValue();
}

test('the composition panel composes a child world through the real runtime and the Test runs it',
  { tag: '@core' }, async ({ page }, testInfo) => {
    test.setTimeout(240_000);
    const errors = [], messages = [];
    page.on('console', message => { if (messages.length < 300) messages.push(`${message.type()}: ${message.text()}`); });
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      Object.defineProperty(navigator, 'webdriver', { get: () => false });
    });
    // A small declared base fixture keeps the shipped hull and shaders, as the
    // other Test render specs do. Both cruisers draw on the same assets.
    await page.route('**/workshop-base.json', async route => {
      const response = await route.fetch(), source = await response.json();
      const all = source.base_asset_manifest;
      const selected = new Set(Object.keys(all).filter(path => path.endsWith('.wgsl')
        || /^assets\/(pfx|radar_icons|skybox)\//.test(path)
        || path.includes('alliance_cruiser_recreated')));
      for (const path of selected) for (const dependency of all[path]?.requires || []) selected.add(dependency);
      source.base_asset_manifest = Object.fromEntries([...selected].map(path => [path, all[path]]));
      await route.fulfill({ response, json: source });
    });

    await page.goto('/workshop.html');
    const chooser = page.waitForEvent('filechooser');
    await page.locator('#workshop-import').click();
    await (await chooser).setFiles({ name: 'composition.zip', mimeType: 'application/zip', buffer: Buffer.from(sourcePack()) });

    // A reading from the runtime: both children are members nothing composes,
    // and the root carries no extra worlds yet.
    await revealWorkshopPanel(page, 'composition');
    await byId(page, 'refresh').click();
    await expect(byId(page, 'members')).toContainText(CHILD);
    await expect(byId(page, 'members')).toContainText(LOOP);
    await byId(page, 'world').selectOption(WORLD);
    await expect(byId(page, 'extra-worlds')).not.toContainText(CHILD);

    // Criterion 3, through the real runtime: the loop world already lists the
    // root, so composing it closes a cycle. The refusal names the rule and the
    // root's source is exactly what it was.
    await byId(page, 'add-extra').selectOption(LOOP);
    await byId(page, 'add-extra-button').click();
    await byId(page, 'apply-world').click();
    await expect(byId(page, 'status')).toHaveAttribute('role', 'alert');
    await expect(byId(page, 'status')).toContainText(refusalWords('workshop.composition.refused.cycle'));
    // And the runtime's own sentence reaches the author, cycle and all, rather
    // than being replaced by the panel's summary of it.
    await expect(byId(page, 'status')).toContainText('composes a cycle');
    await expect(byId(page, 'status')).toContainText(LOOP);
    expect(await sourceOf(page, WORLD)).toBe(ROOT_TEXT);

    // Criterion 4's composition: ONE runtime edit of the root's exact source.
    await revealWorkshopPanel(page, 'composition');
    await byId(page, 'refresh').click();
    await byId(page, 'world').selectOption(WORLD);
    await byId(page, 'add-extra').selectOption(CHILD);
    await byId(page, 'add-extra-button').click();
    await byId(page, 'apply-world').click();
    await expect.poll(() => sourceOf(page, WORLD)).toContain(CHILD);
    const composed = await sourceOf(page, WORLD);
    // Every authored line survived, and one press undoes the whole edit.
    expect(composed).toContain('# Keep this comment on the root world');
    expect(composed).toContain('title = "Composed in the Workshop"');
    expect(composed).not.toContain(LOOP);
    await page.locator('#workshop-undo').click();
    await expect.poll(() => sourceOf(page, WORLD)).toBe(ROOT_TEXT);
    await page.locator('#workshop-redo').click();
    await expect.poll(() => sourceOf(page, WORLD)).toBe(composed);

    // Now run it. The child world is composed only by this unsaved draft and
    // exists nowhere else, so a hull it spawns is the exact composition living
    // in the simulation rather than in a reading of the source.
    await page.locator('#workshop-test-world').selectOption(WORLD);
    await page.locator('#workshop-test-ship').selectOption(SHIP);
    await page.locator('#workshop-test-seed').fill('5');
    await expect(page.locator('#workshop-test-start')).toBeEnabled();
    await page.locator('#workshop-test-start').click();
    try {
      await expect.poll(async () => await page.locator('#workshop-test-pause').isEnabled()
        ? 'running' : await page.locator('#workshop-test-status').getAttribute('role') === 'alert'
          ? await page.locator('#workshop-test-status').textContent() : 'starting', { timeout: 120_000 }).toBe('running');
    } catch (error) {
      await testInfo.attach('browser-diagnostics', { contentType: 'application/json', body: JSON.stringify({ errors, messages }, null, 2) });
      throw error;
    }
    const iframe = page.frames().find(isTestFrame);
    expect(iframe).toBeTruthy();

    // The omniscient view's roster is the GM's own projection of every hull the
    // run has. Read through the frame so a roster nobody can see cannot pass:
    // the omniscient view has to be on screen and the row has to be laid out.
    await page.locator('#workshop-test-view').selectOption('game-master');
    await expect.poll(async () => await iframe.evaluate(() => {
      const gm = document.getElementById('test-gm'), roster = document.getElementById('gm-roster-ships');
      if (gm?.hidden || !roster || roster.offsetParent === null) return null;
      return roster.textContent;
    }), { timeout: 60_000, message: 'the composed child world spawned its hull in the running Test' }).toContain(PATROL);

    await page.locator('#workshop-test-stop').click();
    await expect(page.locator('.workshop-test-viewscreen')).toHaveCount(0);
    expect(errors).toEqual([]);
  });
