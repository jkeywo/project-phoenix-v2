import { test, expect, waitForWasmReady } from './fixtures';
import fs from 'node:fs';
import path from 'node:path';
import { DEVICE_MATRIX, TEXT_SCALES } from '../fixtures/device-matrix.mjs';

// The GM console's smallest supported landscape surface and the top of the
// enlargement range, both taken from #1421's shared matrix rather than
// restated here (PRD #1418 acceptance-matrix decision).
const GM_VIEWPORT = DEVICE_MATRIX.find((row) => row.id === 'desktop-1280x720-gm');
const MAX_TEXT_SCALE = Math.max(...TEXT_SCALES);

// Single-simulation-peer live restore at the GM console's smallest supported
// landscape viewport with text at 200% (issues #1446 / #1418).
//
// A REAL restore, on the real page: a real bookmark through the ordinary save
// machinery, then the typed `request_live_restore` action through real GM
// admission, the real recovery capture, the real gated load and the real
// digest check — with the phase read back off the real `gm_health` projection.
// jsdom can prove the sentences; only a browser can prove that the whole path
// reaches a held, restored world with a Resume the operator can actually press.
//
// The scale is set on the document root directly, exactly as the presentation
// profile does (`accessibility-profile.js` TEXT_SCALE_VAR); exposing 200% in
// the Settings slider is #1422's deliverable.
//
// DELIBERATELY UNTAGGED. `@core` runs on every PR and push, and this spec has
// not yet been run green once: building the bundle it needs (`trunk build` +
// `node scripts/build-client.mjs`) was out of reach in the batch that wrote it,
// and an unverified spec must not gate everybody else's work. Untagged, it
// still runs in the nightly full suite, which is where its first real verdict
// comes from. Add `{ tag: '@core' }` back the first time it passes locally.
test('a GM restores a checkpoint and resumes it at 200% text on 1280x720',
  async ({ context }, testInfo) => {
    test.setTimeout(180000);
    const world = fs.readFileSync(
      path.resolve(__dirname, '../fixtures/worlds/gm_npc_doctrine.toml'), 'utf8');
    await context.route('**/assets/worlds/default.toml',
      route => route.fulfill({ contentType: 'text/plain', body: world }));
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.setViewportSize({ width: GM_VIEWPORT.width, height: GM_VIEWPORT.height });
    await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
    await waitForWasmReady(page);
    await page.evaluate(() => window.__hostFleetOpen());
    await page.waitForFunction(() => window.__hostGmStartState?.().localValidation);
    await page.evaluate(() => document.getElementById('gm-ready-btn').click());
    await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');

    await page.evaluate((scale) => document.documentElement.style
      .setProperty('--a11y-text-scale', String(scale)), MAX_TEXT_SCALE);

    // A real checkpoint to restore, through the ordinary bookmark path.
    const name = 'Before the ambush';
    await page.locator('#gm-checkpoint-name').fill(name);
    await page.locator('#gm-checkpoint-bookmark').click();
    await page.waitForFunction(() => {
      const state = window.__hostGmCheckpointState?.();
      return state && !state.pending && state.statusTone === 'ok';
    }, null, { timeout: 60_000 });
    const slotId = await page.evaluate((label) => window.__hostGmCheckpointState()
      .rows.find((row) => row.displayName === label).slotId, name);

    const restore = page.locator('#gm-restore');
    await expect(restore).toBeVisible();

    // Nothing selected yet: the control asks for a selection in WORDS rather
    // than presenting a dead button (PRD #1418 story 27).
    await expect(page.locator('#gm-restore-summary')).not.toBeEmpty();
    await expect(page.locator('#gm-restore-apply')).toBeDisabled();

    // Touch/pointer reach at this size.
    const row = page.locator(`#gm-checkpoint-list .gm-checkpoint-row[data-checkpoint-slot-id="${slotId}"]`);
    await row.click();
    const apply = page.locator('#gm-restore-apply');
    await expect(apply).toBeEnabled();
    expect((await apply.boundingBox()).height).toBeGreaterThanOrEqual(44);
    expect((await page.locator('#gm-restore-resume').boundingBox()).height)
      .toBeGreaterThanOrEqual(44);
    await expect(page.locator('#gm-restore-summary')).toContainText(name);

    // A restore is the one decision that opens a confirmation, and the preview
    // names what a crew has already witnessed as well as the technical change.
    await apply.click();
    const dialog = page.locator('#gm-action-confirmation');
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(name);
    await expect(dialog).toContainText('discarded');
    await dialog.locator('[data-confirmation-accept]').click();

    // The world really rewinds, and stops.
    await page.waitForFunction(() => {
      const state = window.__hostGmRestoreState?.();
      return state && !state.pending && ['restored', 'rolled-back', 'failed'].includes(state.phase);
    }, null, { timeout: 90_000 });
    const settled = await page.evaluate(() => window.__hostGmRestoreState());
    expect(settled.phase).toBe('restored');
    expect(settled.paused).toBe(true);

    // Non-colour status: the phase is a sentence, not only `data-phase`.
    await expect(restore).toHaveAttribute('data-phase', 'restored');
    const statusText = await page.locator('#gm-restore-status').textContent();
    expect(statusText.trim().length).toBeGreaterThan(0);

    // The same fact reaches the unfilterable technical banner (#1437), so a GM
    // who did not press it still sees that the world was replaced.
    await expect(page.locator('#gm-attention-banners .gm-health-banner'
      + '[data-kind="live_restore_settled"]')).toBeVisible();

    // Nothing resumed on its own; the GM's own Resume is reachable and works.
    const resume = page.locator('#gm-restore-resume');
    await expect(resume).toBeEnabled();
    await resume.focus();
    expect(await page.evaluate(() => document.activeElement?.id)).toBe('gm-restore-resume');
    await resume.click();
    await page.waitForFunction(() => window.__hostGmRestoreState?.().phase === 'idle',
      null, { timeout: 60_000 });

    // Panels wrap, stack and scroll; they never scroll sideways or shrink text.
    const geometry = await restore.evaluate(el => ({
      clientWidth: el.clientWidth,
      scrollWidth: el.scrollWidth,
      fontSize: parseFloat(getComputedStyle(el).fontSize),
    }));
    expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth + 1);
    expect(geometry.fontSize).toBeGreaterThanOrEqual(14 * MAX_TEXT_SCALE);

    const screenshot = testInfo.outputPath('gm-restore-200.png');
    await page.screenshot({ path: screenshot, fullPage: false });
    await testInfo.attach('GM live restore 1280×720 @200%', { path: screenshot, contentType: 'image/png' });
    expect(errors).toEqual([]);
  });
