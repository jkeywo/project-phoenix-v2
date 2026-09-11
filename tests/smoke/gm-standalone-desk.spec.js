// A standalone game master's desk can actually act.
//
// John, 2026-09-11: "the previous version of the screen had most of the buttons
// disabled permanently." Reproduced on a built bundle: the landing's Host as GM
// route to a started mission left 42 of 80 visible `#gm-console` buttons
// disabled, because a fleetless peer had no bound GM operator and every panel
// gates its controls on exactly that identity.
//
// This is the assertion that report would have caught, and it has to be a real
// browser: the whole failure lived in the seam between the page's admission
// question and the simulation's answer to it. jsdom can compile the page's
// half; only this can run both and find them agreeing. So the spec walks the
// route an operator walks — landing entry, scenario, hull, Start — and then
// presses a real verb and reads the authoritative outcome back.

import { test, expect, waitForWasmReady } from './fixtures';
import { ts } from './strings';
import { DEVICE_MATRIX, TEXT_SCALES } from '../fixtures/device-matrix.mjs';

// The GM console's smallest supported landscape surface and the top of the
// enlargement range, from #1421's shared matrix (PRD #1418).
const GM_VIEWPORT = DEVICE_MATRIX.find((row) => row.id === 'desktop-1280x720-gm');
const MAX_TEXT_SCALE = Math.max(...TEXT_SCALES);

const MENU = '#landing-menu';
const HOST_GM = `${MENU} [data-landing-entry="host_gm"]`;
const SCENARIO_BUTTONS = '#world-list .world-btn[data-scenario-id]';

test('the Host as GM route reaches a live desk, not a disabled one',
  { tag: '@core' },
  async ({ context }) => {
    test.setTimeout(180000);
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.setViewportSize({ width: GM_VIEWPORT.width, height: GM_VIEWPORT.height });
    await page.goto('/');
    await page.bringToFront();

    // ── The route ────────────────────────────────────────────────────────
    await page.locator(HOST_GM).waitFor({ state: 'visible', timeout: 30_000 });
    await page.click(HOST_GM);
    const first = page.locator(SCENARIO_BUTTONS).first();
    await first.waitFor({ state: 'visible', timeout: 60_000 });
    await first.click();

    // Multi-hull scenarios offer a picker; single-hull ones resolve themselves.
    const shipCard = page.locator('ph-ship-picker .ship-card').first();
    await Promise.race([
      shipCard.waitFor({ state: 'visible', timeout: 60_000 }),
      page.locator('#landing-panel').waitFor({ state: 'hidden', timeout: 60_000 }),
    ]);
    if (await shipCard.isVisible()) await shipCard.click();
    await expect(page.locator('#landing-panel')).toBeHidden({ timeout: 60_000 });
    await waitForWasmReady(page);

    // This IS the standalone route: the game master with no fleet behind it.
    expect(await page.evaluate(() => window.__hostGmStandalone())).toBe(true);

    // ── The identity the desk acts as ────────────────────────────────────
    //
    // Bound by the simulation and read back by the page, so a live control and
    // an accepted action are one fact rather than two hopes.
    await page.waitForFunction(() => !!window.__hostLocalGm?.(), null, { timeout: 60_000 });
    const operator = await page.evaluate(() => window.__hostLocalGm());
    expect(operator.id).toBe('gm-1');
    expect(await page.evaluate(() => JSON.parse(window.wasm_local_gm_operator() || 'null')))
      .toMatchObject({ id: 'gm-1', connected: true });

    // ── Start the mission through the desk's own control ─────────────────
    const start = page.locator('#gm-session-start');
    await expect(start).toBeVisible({ timeout: 60_000 });
    await start.click();
    await page.locator('#gm-action-confirmation [data-confirmation-accept]').click();
    await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress', null,
      { timeout: 120_000 });

    // ── The reported symptom, measured ───────────────────────────────────
    //
    // Every GM panel gates its controls on one question — is there a local
    // operator — and records the answer on its own region. All four must say
    // yes; that is the defect, stated as the page states it.
    const admitted = await page.locator('#gm-console [data-admitted]')
      .evaluateAll((els) => els.map((el) => [el.id, el.dataset.admitted]));
    expect(admitted.length).toBeGreaterThanOrEqual(4);
    expect(admitted.filter(([, value]) => value !== 'true')).toEqual([]);

    // The verbs named in the report, individually, so a regression says which.
    // These are the ones gated on ADMISSION alone: a control that also needs a
    // selection or a typed field is a different sentence and stays disabled
    // until the GM has chosen something, which is not what was reported.
    for (const id of ['gm-session-pause', 'gm-session-resume']) {
      await expect(page.locator(`#${id}`)).toBeEnabled();
    }

    // And the whole console, counted the way the report counted it. The
    // remaining disabled controls must be a small selection-dependent minority,
    // not most of the desk.
    const tally = await page.evaluate(() => {
      const visible = [...document.getElementById('gm-console').querySelectorAll('button')]
        .filter((button) => !!button.offsetParent);
      return { visible: visible.length, disabled: visible.filter((b) => b.disabled).length };
    });
    expect(tally.visible).toBeGreaterThan(40);
    expect(tally.disabled).toBeLessThan(tally.visible / 3);

    // The operator strip a fleet GM gets from its roster publish. A session
    // with no fleet has an honest equivalent rather than an empty column.
    await expect(page.locator('#gm-roster-operators'))
      .toContainText(ts('server.gm.operator.standalone'));

    // ── And a verb that really runs ──────────────────────────────────────
    //
    // The defect's second layer: the page could be fixed alone and every press
    // would still be ingested and then refused. Pause the session and read the
    // AUTHORITATIVE answer back off the session projection.
    await page.locator('#gm-session-pause').click();
    const accept = page.locator('#gm-action-confirmation [data-confirmation-accept]');
    if (await accept.isVisible()) await accept.click();
    await page.waitForFunction(
      () => window.__hostGmSessionState?.().paused === true, null, { timeout: 60_000 });
    const paused = await page.evaluate(() => window.__hostGmSessionState());
    expect(paused.paused).toBe(true);
    expect(paused.authoritative).toBeGreaterThan(0);

    // ── The same desk at 200% text (PRD #1418) ───────────────────────────
    //
    // This slice's own new surface is the operator strip and the verbs the
    // binding switched on, so they are what is checked at the top of the
    // enlargement range: the session verbs keep a thumb-sized target, the
    // operator's name is still readable text rather than a truncated id, and
    // the console wraps and stacks instead of scrolling sideways.
    await page.evaluate((scale) => document.documentElement.style
      .setProperty('--a11y-text-scale', String(scale)), MAX_TEXT_SCALE);

    for (const id of ['gm-session-pause', 'gm-session-resume']) {
      const button = page.locator(`#${id}`);
      await expect(button).toBeEnabled();
      expect((await button.boundingBox()).height).toBeGreaterThanOrEqual(44);
    }
    const operators = page.locator('#gm-roster-operators');
    await expect(operators).toContainText(ts('server.gm.operator.standalone'));
    // Scoped to this slice's own two surfaces. The console shell's overall
    // enlargement is #1430's contract and is asserted by its own specs; what
    // is new here is the session controls the binding switched on and the
    // operator strip a standalone session now fills.
    for (const id of ['gm-session-controls', 'gm-roster-operators']) {
      const geometry = await page.locator(`#${id}`).evaluate((el) => ({
        clientWidth: el.clientWidth,
        scrollWidth: el.scrollWidth,
        fontSize: parseFloat(getComputedStyle(el).fontSize),
      }));
      expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth + 1);
      expect(geometry.fontSize).toBeGreaterThanOrEqual(14 * MAX_TEXT_SCALE);
    }

    expect(errors).toEqual([]);
  });
