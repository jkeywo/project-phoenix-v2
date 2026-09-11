import { test, expect, waitForWasmReady } from './fixtures';
import fs from 'node:fs';
import path from 'node:path';
import { DEVICE_MATRIX, TEXT_SCALES, BROWSER_ZOOMS } from '../fixtures/device-matrix.mjs';

// Issue #1430: the GM's smallest supported landscape surface and the top of
// the enlargement range, from #1421's shared matrix (PRD #1418) — the same
// pair `gm-undo-200.spec.js` / `gm-journal-200.spec.js` / `gm-checkpoint-200.spec.js`
// use, so this file and its M4/M5 siblings exercise one matrix rather than
// four copies of viewport/scale literals.
const GM_VIEWPORT = DEVICE_MATRIX.find((row) => row.id === 'desktop-1280x720-gm');
const MAX_TEXT_SCALE = Math.max(...TEXT_SCALES);

async function joinAsReadyGm(page, world) {
  await page.route('**/assets/worlds/default.toml', route => route.fulfill({ contentType: 'text/plain', body: world }));
  await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await page.evaluate(() => window.__hostFleetOpen());
  await page.waitForFunction(() => window.__hostGmStartState?.().localValidation);
  await page.evaluate(() => document.getElementById('gm-ready-btn').click());
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');
}

test('GM desktop layout is usable at both host viewport sizes', async ({ context }, testInfo) => {
  test.setTimeout(90000);
  const world = fs.readFileSync(path.resolve(__dirname, '../fixtures/worlds/gm_npc_doctrine.toml'), 'utf8');
  await context.route('**/assets/worlds/default.toml', route => route.fulfill({contentType:'text/plain',body:world}));
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/?gm=1&scenario=assets/worlds/default.toml');
  await waitForWasmReady(page);
  await page.evaluate(() => window.__hostFleetOpen());
  await page.waitForFunction(() => window.__hostGmStartState?.().localValidation);
  await page.evaluate(() => document.getElementById('gm-ready-btn').click());
  await page.waitForFunction(() => window.__saveSlotsPhase === 'InProgress');
  await expect(page.locator('#gm-roster-ships button').first()).toBeVisible();
  await page.locator('#gm-roster-ships button').first().click();
  // The post-M5 screen (PRD #930, canvas artboard "After M5 - facilitation +
  // operations"): six regions in reading order, three columns over two rows.
  expect(await page.locator('#gm-workspace > *').evaluateAll(nodes => nodes.map(node => node.id)))
    .toEqual(['gm-desk-brief', 'gm-map-panel', 'gm-desk-detail',
      'gm-mission-panel', 'gm-desk-log', 'gm-health-panel']);
  for (const [width,height] of [[1440,900],[1280,720]]) {
    await page.setViewportSize({width,height});
    await page.locator('#gm-console').evaluate(el=>el.scrollTop=0);
    await expect(page.locator('#gm-workspace')).toBeVisible();
    // Three columns over two rows, measured rather than declared: each column
    // shares a left edge and each row shares a top edge, at BOTH viewports.
    const boxes = {};
    for (const id of ['gm-desk-brief', 'gm-map-panel', 'gm-desk-detail',
      'gm-mission-panel', 'gm-desk-log', 'gm-health-panel']) {
      await expect(page.locator(`#${id}`)).toBeVisible();
      boxes[id] = await page.locator(`#${id}`).boundingBox();
    }
    expect(Math.round(boxes['gm-desk-brief'].x), `${width}: left column`)
      .toBe(Math.round(boxes['gm-mission-panel'].x));
    expect(Math.round(boxes['gm-map-panel'].x), `${width}: centre column`)
      .toBe(Math.round(boxes['gm-desk-log'].x));
    expect(Math.round(boxes['gm-desk-detail'].x), `${width}: right column`)
      .toBe(Math.round(boxes['gm-health-panel'].x));
    expect(Math.round(boxes['gm-desk-brief'].y), `${width}: top row`)
      .toBe(Math.round(boxes['gm-desk-detail'].y));
    expect(boxes['gm-mission-panel'].y, `${width}: second row is below the first`)
      .toBeGreaterThan(boxes['gm-desk-brief'].y);
    const geometry = await page.locator('#gm-workspace').evaluate(el=>({
      width:el.clientWidth, scroll:el.scrollWidth,
      height:el.clientHeight, scrollHeight:el.scrollHeight,
    }));
    expect(geometry.scroll).toBeLessThanOrEqual(geometry.width);
    // At ordinary text the whole desk fits ONE screen: every panel row is on
    // it, and each section scrolls its own overflow. The desk's height must be
    // DEFINITE for that — with `min-height` the grid's block size is
    // indefinite, `1fr` resolves against max-content, and the desk silently
    // becomes three viewports tall with only the roster/map/inspector row
    // above the fold while the horizontal contract above still passes.
    expect(geometry.height).toBeLessThanOrEqual(height);
    expect(geometry.scrollHeight).toBeLessThanOrEqual(geometry.height + 1);
    // Comms, the activity feed and the saved action journal share the centre
    // region behind one tab strip, at both viewports.
    await expect(page.locator('#gm-comms-panel')).toBeVisible();
    await expect(page.locator('#gm-journal')).toBeHidden();
    await page.locator('#gm-log-tab-journal').click();
    await expect(page.locator('#gm-journal')).toBeVisible();
    await expect(page.locator('#gm-comms-panel')).toBeHidden();
    // The switch is `data-log-view` on the region, never `hidden` on a panel:
    // that attribute belongs to the role preset (GM_ROLE_PRESET_PANEL_IDS), and
    // two writers on one attribute is exactly the race this avoids.
    expect(await page.locator('#gm-comms-panel').evaluate(el => el.hasAttribute('hidden'))).toBe(false);
    await page.locator('#gm-log-tab-comms').click();
    await expect(page.locator('#gm-comms-panel')).toBeVisible();
    const screenshot = testInfo.outputPath(`gm-screen-${width}.png`);
    await page.screenshot({path:screenshot});
    await testInfo.attach(`GM ${width}×${height}`, {path:screenshot,contentType:'image/png'});
  }
  // PRD #1418: the desk stays usable at 200% text on the smaller supported
  // landscape viewport. The variable is set DIRECTLY here rather than through
  // the settings slider, whose exposed ceiling is issue #1422's to lift; this
  // asserts the layout, not the control. The attention queue (issue #1433) is
  // included by name because it is the newest region and the one whose rows
  // wrap most: panels must scroll their own overflow rather than push the desk
  // sideways.
  await page.setViewportSize({width:1280,height:720});
  await page.evaluate(() => document.documentElement.style.setProperty('--a11y-text-scale', '2'));
  await expect(page.locator('#gm-attention-panel')).toBeVisible();
  await expect(page.locator('#gm-attention-heading')).toBeVisible();
  await expect(page.locator('#gm-attention-filter-band')).toBeVisible();
  // The technical health panel (issue #1437) is on the same landscape desk and
  // holds the same contract: it reads at 200% and its rows wrap rather than
  // shrink. Its warning region is rendered into #gm-attention-banners above,
  // which is asserted here to be reachable rather than clipped away.
  await expect(page.locator('#gm-health-panel')).toBeVisible();
  await expect(page.locator('#gm-health-heading')).toBeVisible();
  await expect(page.locator('#gm-health-summary')).toBeVisible();
  const health = await page.locator('#gm-health-panel').evaluate(el => ({
    scroll: el.scrollWidth,
    width: el.clientWidth,
    fontPx: parseFloat(getComputedStyle(el).fontSize),
  }));
  expect(health.scroll).toBeLessThanOrEqual(health.width + 1);
  expect(health.fontPx).toBeGreaterThanOrEqual(28);
  const scaled = await page.locator('#gm-workspace').evaluate(el => ({
    width: el.clientWidth,
    scroll: el.scrollWidth,
    body: document.documentElement.scrollWidth,
    viewport: document.documentElement.clientWidth,
    fontPx: parseFloat(getComputedStyle(document.getElementById('gm-console')).fontSize),
  }));
  expect(scaled.scroll).toBeLessThanOrEqual(scaled.width);
  expect(scaled.body).toBeLessThanOrEqual(scaled.viewport);
  expect(scaled.fontPx).toBeGreaterThanOrEqual(28);
  // An eligible authored beat (issue #1434) is a row in that same queue, so it
  // meets the same contract: fed through the REAL host channel, its band reads
  // as a word, its reason is a sentence, and both of its verbs are still on
  // screen and pressable at 200% without the desk scrolling sideways.
  await page.evaluate(() => window.__hostChannel('gm_attention', JSON.stringify({
    occurrences: [{
      id: 'event:base-world::smoke#1',
      category: 'eligible_beat',
      band: 'background',
      first_seen_tick: 1,
      age_ms: 1000,
      reason: {
        id: 'server.gm.attention.reason.eligible_beat_manual',
        params: { beat: 'server.gm.mission.heading' },
      },
      target: {
        event: {
          id: 'base-world::smoke', label: 'server.gm.mission.heading',
          fire: true, pause: false, skip: false,
        },
      },
    }],
  })));
  const beatRow = page.locator('#gm-attention-list li[data-occurrence-id="event:base-world::smoke#1"]');
  await expect(beatRow).toBeVisible();
  await expect(beatRow.locator('.gm-attention-band')).toHaveText(/\S/);
  await expect(beatRow.locator('.gm-attention-reason')).toHaveText(/\S/);
  await expect(beatRow.locator('button[data-action="open"]')).toBeVisible();
  await expect(beatRow.locator('button[data-action="snooze"]')).toBeVisible();
  const withBeat = await page.locator('#gm-workspace').evaluate(el => ({
    scroll: el.scrollWidth,
    width: el.clientWidth,
    body: document.documentElement.scrollWidth,
    viewport: document.documentElement.clientWidth,
  }));
  expect(withBeat.scroll).toBeLessThanOrEqual(withBeat.width);
  expect(withBeat.body).toBeLessThanOrEqual(withBeat.viewport);
  // The keyboard reaches the row's verbs, and reading one holds the list.
  await beatRow.locator('button[data-action="open"]').focus();
  expect(await page.evaluate(() => document.activeElement?.dataset?.action)).toBe('open');
  // Reading a row HOLDS the queue (issue #1433): a list an operator is
  // standing in must not rebuild under them, so the projection pushed below
  // would land behind the held beat row and nothing after it would ever be
  // drawn. The next case wants the live list, so it returns to live through
  // the panel's own control, exactly as an operator would.
  // The quiet-time advisory (issue #1436) is the third row family in the same
  // queue, and the one with no destination: it must read as a full sentence and
  // offer its ONE verb, with no dead Open button beside it, still without the
  // desk scrolling sideways at 200%.
  await page.evaluate(() => window.__hostChannel('gm_attention', JSON.stringify({
    occurrences: [{
      id: 'quiet:1',
      category: 'quiet_time',
      band: 'background',
      first_seen_tick: 1,
      age_ms: 125000,
      reason: {
        id: 'server.gm.attention.reason.quiet_time',
        params: { seconds: '120' },
      },
      target: {},
    }],
  })));
  // Focus is still standing on the beat row above, so that push did NOT
  // rebuild the list under the operator — issue #1433's reading hold, which is
  // the behaviour PRD #1418 story 4 asks for, and which every case after this
  // one depends on being released deliberately rather than silently.
  await page.waitForFunction(() => window.__hostGmAttentionState().held === true);
  await expect(page.locator('#gm-attention-list li[data-occurrence-id="quiet:1"]')).toHaveCount(0);
  // The operator's own word that the presentation may catch up.
  await page.locator('#gm-attention-live').click();
  await page.waitForFunction(() => window.__hostGmAttentionState().held === false);
  const quietRow = page.locator('#gm-attention-list li[data-occurrence-id="quiet:1"]');
  await expect(quietRow).toBeVisible();
  // …and the bar states it as a pill, from that same occurrence's own age.
  // No pill is invented: the desk draws one only for a fact a live payload
  // carries, which is why there is no rate and no wall-clock checkpoint age.
  const quietPill = page.locator('#gm-health-pills span[data-pill="quiet"]');
  await expect(quietPill).toBeVisible();
  // Read the pill and the row it summarises together: they are the same live
  // clock, because the QUEUE ages both. Rust does not republish on age alone,
  // and a lull keeps one occurrence id for its whole duration, so a pill
  // painted from the payload's own `age_ms` would sit frozen at the sampled
  // 2:05 while the row beside it counted on.
  const readClocks = () => page.evaluate(() => {
    const seconds = (node) => {
      const match = node && node.textContent.match(/(\d+):(\d\d)/);
      return match ? Number(match[1]) * 60 + Number(match[2]) : null;
    };
    return {
      pill: seconds(document.querySelector('#gm-health-pills span[data-pill="quiet"]')),
      row: seconds(document.querySelector(
        '#gm-attention-list li[data-occurrence-id="quiet:1"] .gm-attention-age')),
    };
  });
  const clocks = await readClocks();
  expect(clocks.row).toBeGreaterThanOrEqual(125);
  expect(Math.abs(clocks.pill - clocks.row)).toBeLessThanOrEqual(1);
  // And it keeps counting with no further payload at all.
  await expect.poll(async () => (await readClocks()).pill, { timeout: 8000 })
    .toBeGreaterThan(clocks.pill);
  await expect(quietRow.locator('.gm-attention-band')).toHaveText(/\S/);
  await expect(quietRow.locator('.gm-attention-reason')).toHaveText(/\S/);
  await expect(quietRow.locator('button[data-action="snooze"]')).toBeVisible();
  await expect(quietRow.locator('button[data-action="open"]')).toHaveCount(0);
  const withQuiet = await page.locator('#gm-workspace').evaluate(el => ({
    scroll: el.scrollWidth,
    width: el.clientWidth,
    body: document.documentElement.scrollWidth,
    viewport: document.documentElement.clientWidth,
  }));
  expect(withQuiet.scroll).toBeLessThanOrEqual(withQuiet.width);
  expect(withQuiet.body).toBeLessThanOrEqual(withQuiet.viewport);
  await quietRow.locator('button[data-action="snooze"]').focus();
  expect(await page.evaluate(() => document.activeElement?.dataset?.action)).toBe('snooze');
  // Routine attention stays in the queue: no dialog opened and no panel moved.
  expect(await page.locator('#gm-action-confirmation[open]').count()).toBe(0);
  // The Station-workload advisory (issue #1438) meets the same contract in the
  // same column: fed through the REAL host channel, its level reads as a word,
  // its expander is reachable by keyboard at 200%, and its evidence appears
  // without the desk scrolling sideways.
  await page.evaluate(() => window.__hostChannel('gm_workload', JSON.stringify({
    stations: [{
      ship: { entity_id: 'smoke-hull', name: 'server.gm.roster.heading' },
      station_id: 'comms',
      station_name: 'server.gm.mission.heading',
      level: 'overloaded',
      count: 3,
      sustained_secs: 31,
      overload_count: 3,
      overload_secs: 30,
      demands: [
        { key: 'comms:a', source: 'pending_comms',
          reason: { id: 'server.gm.workload.reason.pending_comms', params: { sender: 'Cordon Control' } } },
        { key: 'nav:smoke-hull#2', source: 'navigation_clearance',
          reason: { id: 'server.gm.workload.reason.navigation_clearance', params: { x: '400', z: '-900' } } },
        { key: 'repair:smoke-hull/weapons', source: 'repair_dispatch',
          reason: { id: 'server.gm.workload.reason.repair_dispatch', params: { station: 'Tactical', tier: 'server.gm.workload.tier.damaged' } } },
      ],
    }],
  })));
  const workloadRow = page.locator('#gm-workload-list li[data-station-key="smoke-hull/comms"]');
  await expect(workloadRow).toBeVisible();
  await expect(workloadRow.locator('.gm-workload-state')).toHaveText(/\S/);
  await expect(workloadRow.locator('.gm-workload-count')).toHaveText(/\S/);
  await workloadRow.locator('summary').focus();
  expect(await page.evaluate(() => document.activeElement?.tagName)).toBe('SUMMARY');
  await workloadRow.locator('summary').click();
  await expect(workloadRow.locator('.gm-workload-demands li')).toHaveCount(3);
  const withWorkload = await page.locator('#gm-workspace').evaluate(el => ({
    scroll: el.scrollWidth,
    width: el.clientWidth,
    body: document.documentElement.scrollWidth,
    viewport: document.documentElement.clientWidth,
  }));
  expect(withWorkload.scroll).toBeLessThanOrEqual(withWorkload.width);
  expect(withWorkload.body).toBeLessThanOrEqual(withWorkload.viewport);
  // The typed world-authored widget region (issue #1439) is composed from the
  // preset the operator selects, so it is driven the way an operator drives it:
  // the authored list goes in through the SAME `wasm_get_gm_role_presets`
  // payload seam the page uses, and the role is chosen from the real selector.
  // The composition is the shipped probe world's, shared with
  // tests/gm_widgets.rs and tests/client/gm-widgets-panel.test.js.
  const authoredPresets = fs.readFileSync(
    path.resolve(__dirname, '../fixtures/gm-widgets-presets.json'), 'utf8');
  await page.evaluate((payload) => window.__hostGmRolePresetsSetAvailable(payload), authoredPresets);
  await page.locator('#gm-role-preset-select').selectOption('tactical');
  await expect(page.locator('#gm-widgets')).toBeVisible();
  for (const id of ['urgent-traffic', 'seats', 'session-levers', 'brief']) {
    await expect(page.locator(`#gm-widgets-list li[data-widget-id="${id}"]`)).toBeVisible();
  }
  // The note is TEXT: it reads as a sentence and the card grows no elements.
  const note = page.locator('#gm-widgets-list li[data-widget-id="brief"] .gm-widget-note');
  await expect(note).toHaveText(/\S/);
  expect(await note.evaluate(el => el.children.length)).toBe(0);
  // An authored action button is the shipped control, pressed from here: it is
  // keyboard-reachable at 200% and hands the press to #gm-session-pause.
  const lever = page.locator('#gm-widgets-list button[data-widget-action="gm-session-pause"]');
  await expect(lever).toBeVisible();
  await lever.focus();
  expect(await page.evaluate(() => document.activeElement?.dataset?.widgetAction))
    .toBe('gm-session-pause');
  const hit = await lever.evaluate(el => el.getBoundingClientRect().height);
  expect(hit).toBeGreaterThanOrEqual(44);
  // And the desk still never scrolls sideways with the region on it.
  const withWidgets = await page.locator('#gm-workspace').evaluate(el => ({
    scroll: el.scrollWidth,
    width: el.clientWidth,
    body: document.documentElement.scrollWidth,
    viewport: document.documentElement.clientWidth,
  }));
  expect(withWidgets.scroll).toBeLessThanOrEqual(withWidgets.width);
  expect(withWidgets.body).toBeLessThanOrEqual(withWidgets.viewport);
  // The bar's tick-health pill, from a REAL gm_health payload on the same
  // channel the health panel reads — one reader more, never a second channel.
  await page.evaluate(() => window.__hostChannel('gm_health', JSON.stringify({
    tick: 4821,
    paused: false,
    peers: [{ id: 'peer:1', state: 'stale', behind_ticks: 4, operators: [] }],
    stations: [],
    operators: [],
    alerts: [],
  })));
  const tickPill = page.locator('#gm-health-pills span[data-pill="tick"]');
  await expect(tickPill).toBeVisible();
  // A WORD, not a colour: the pill says which condition the tick is in.
  await expect(tickPill).toHaveText(/\S/);
  expect(await tickPill.getAttribute('data-state')).toBe('stale');
  // The whole bar is still one bar at 200%: it wraps, it does not scroll.
  const bar = await page.locator('#gm-console > header').evaluate(el => ({
    scroll: el.scrollWidth, width: el.clientWidth,
  }));
  expect(bar.scroll).toBeLessThanOrEqual(bar.width + 1);
  // The checkpoint column and its restore control are on the right-hand region
  // at 200%, which is where the artboard puts them.
  await expect(page.locator('#gm-desk-detail > #gm-checkpoint')).toBeVisible();
  await expect(page.locator('#gm-checkpoint-bookmark')).toBeVisible();
  expect(await page.locator('#gm-checkpoint-bookmark').evaluate(el =>
    el.getBoundingClientRect().height)).toBeGreaterThanOrEqual(44);
  // Every tab in the centre strip stays pressable at 200%, and each view
  // scrolls inside its own region rather than widening the desk.
  for (const view of ['comms', 'activity', 'journal']) {
    const tab = page.locator(`#gm-log-tab-${view}`);
    await expect(tab).toBeVisible();
    expect(await tab.evaluate(el => el.getBoundingClientRect().height)).toBeGreaterThanOrEqual(44);
    await tab.click();
    const region = await page.locator('#gm-desk-log').evaluate(el => ({
      scroll: el.scrollWidth, width: el.clientWidth,
    }));
    expect(region.scroll, `${view}: centre region width at 200%`)
      .toBeLessThanOrEqual(region.width + 1);
  }
  await page.locator('#gm-log-tab-comms').click();
  const doubled = testInfo.outputPath('gm-screen-1280-200pc.png');
  await page.screenshot({path:doubled});
  await testInfo.attach('GM 1280×720 at 200% text', {path:doubled,contentType:'image/png'});
  await page.evaluate(() => document.documentElement.style.removeProperty('--a11y-text-scale'));
  expect(errors).toEqual([]);
});

// Issue #1430: the T2 "directing" (#1301-#1316) and "performing" (#1317-#1320)
// panels above are the desk's ORIGINAL controls/status/confirmations — the
// mission log and the Objective list are the two that grow into genuinely
// dense reading surfaces during a live session, and #1421's own
// `DENSE_CONTENT.gmLists` fixture names real long rows from exactly these two
// panels without anything ever having driven them through the real reducer at
// 200%. Injected via the same real Host Channel seam
// (`window.__hostChannel('gm_mission', …)`) the existing test above already
// uses for `gm_attention` — both panels share that one channel
// (`GmMissionProjection`), so one push exercises both.
test('GM directing panels stay reachable with a dense mission log and Objective list at 100%, 150% and 200% text',
  async ({ context }, testInfo) => {
    test.setTimeout(90000);
    const world = fs.readFileSync(path.resolve(__dirname, '../fixtures/worlds/gm_npc_doctrine.toml'), 'utf8');
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.setViewportSize({ width: GM_VIEWPORT.width, height: GM_VIEWPORT.height });
    await joinAsReadyGm(page, world);

    const EVENT_IDS = ['base-world::breach_alarm', 'base-world::relief', 'base-world::skyhook_loss'];
    await page.evaluate(({ eventIds }) => {
      const event = (id) => ({ id, label: 'server.gm.mission.heading', fire: true, pause: true,
        skip: true, repeatable: true, spent: false, armed: false, paused: false, skip_armed: false });
      const outcomes = ['applied', 'no-op', 'refused'];
      const results = [];
      for (let i = 0; i < 9; i += 1) {
        results.push({ operator_id: 'gm-a', correlation: `dense-fire-${i}`, outcome: outcomes[i % 3],
          tick: 100 + i, target: eventIds[i % 3], verb: 'fire', requested_active: true,
          ...(outcomes[i % 3] === 'refused' ? { reason: 'unknown-gm-event' } : {}) });
      }
      // #1421's own dense-fixture id, driven for real: the `skip_result`
      // family, three occurrences across applied/no-op/refused.
      for (let i = 0; i < 3; i += 1) {
        results.push({ operator_id: 'gm-b', correlation: `dense-skip-${i}`, outcome: outcomes[i],
          tick: 200 + i, target: eventIds[i], lever: 'skip-next', requested_active: true,
          ...(outcomes[i] === 'refused' ? { reason: 'unknown-gm-event' } : {}) });
      }
      const objectives = Array.from({ length: 12 }, (_, i) => ({
        id: `dense-objective-${i}`,
        text: `Escort the Directive courier past the Ladder ${i} inspection line and confirm the transfer window stays open for every crew still aboard (objective ${i}).`,
        text_params: {}, recipients: [], available: true, status: 'Active',
      }));
      window.__hostChannel('gm_mission', JSON.stringify({
        events: eventIds.map(event), results,
        objective_palette: [], objectives, objective_results: [],
      }));
    }, { eventIds: EVENT_IDS });

    // The mission log: twelve rows, none shrunk sideways off the desk.
    const missionRows = page.locator('#gm-mission-log .gm-mission-log-entry');
    await expect(missionRows).toHaveCount(12);
    for (const id of EVENT_IDS) {
      await expect(page.locator(`button[data-role="fire"][data-event-id="${id}"]`)).toBeVisible();
      await expect(page.locator(`button[data-role="skip"][data-event-id="${id}"]`)).toBeVisible();
    }

    // The Objective list: twelve dense rows, each with its full ~180-char
    // description and both verb buttons — PRD #1418's "long text and dense
    // states", not a shortened stand-in.
    const objectiveRows = page.locator('#gm-objective-list .gm-objective-row');
    await expect(objectiveRows).toHaveCount(12);
    const firstText = await objectiveRows.first().locator('p').first().textContent();
    expect(firstText?.length).toBeGreaterThan(120);
    await expect(objectiveRows.first().locator('button[data-verb="complete"]')).toBeVisible();
    await expect(objectiveRows.first().locator('button[data-verb="fail"]')).toBeVisible();

    // PRD #1418 Testing Decisions: "exercise 100%, 150% and 200% with
    // realistic long text and dense states." Content is injected once above;
    // only the scale changes on each pass, so this stays one boot.
    let previousFont = 0;
    for (const scale of TEXT_SCALES) {
      const where = `@ ${scale}x`;
      await page.evaluate((value) => document.documentElement.style
        .setProperty('--a11y-text-scale', String(value)), scale);
      await expect(missionRows).toHaveCount(12);
      await expect(objectiveRows).toHaveCount(12);

      // Neither dense panel — nor the desk around them — grows a sideways
      // scrollbar to hold it, at any of the three scales.
      const geometry = await page.locator('#gm-workspace').evaluate(el => ({
        scroll: el.scrollWidth, width: el.clientWidth,
        body: document.documentElement.scrollWidth, viewport: document.documentElement.clientWidth,
        fontPx: parseFloat(getComputedStyle(document.getElementById('gm-console')).fontSize),
      }));
      expect(geometry.scroll, `${where}: mission/objective desk width`).toBeLessThanOrEqual(geometry.width);
      expect(geometry.body, `${where}: no sideways body scroll`).toBeLessThanOrEqual(geometry.viewport);
      // Enlargement is genuinely applied, never a silent shrink back down.
      expect(geometry.fontPx, `${where}: desk root grew`).toBeGreaterThan(previousFont);
      previousFont = geometry.fontPx;

      // Every control PRD #1418 story 1 asks to stay reachable is still
      // there and still pressable — not merely present in the DOM.
      await expect(objectiveRows.first().locator('button[data-verb="complete"]')).toBeVisible();
      await expect(objectiveRows.first().locator('button[data-verb="fail"]')).toBeVisible();
      const firstBox = await objectiveRows.first().locator('button[data-verb="complete"]').boundingBox();
      expect(firstBox?.height, `${where}: verb button hit height`).toBeGreaterThanOrEqual(44);
    }

    // Keyboard reach into the dense Objective list's own verb button, at the
    // ceiling this loop finished on (200%).
    await objectiveRows.first().locator('button[data-verb="complete"]').focus();
    expect(await page.evaluate(() => document.activeElement?.dataset?.verb)).toBe('complete');

    await page.evaluate(() => document.documentElement.style.removeProperty('--a11y-text-scale'));
    expect(errors).toEqual([]);
  });

// Issue #1430 acceptance: "the shell does not clone [the already-corrected
// Station family] contents or defer their readability checks." The puppet
// mounts the REAL authored `StationConfig.console` URL (`gm-station-puppet.js`
// — never a second/simplified Station UI), and this proves the desk's own
// text scale reaches THROUGH that iframe rather than leaving the puppeted
// document at its own 100% default (the fix `gui/gm-station-puppet.js`
// carries for this issue).
test('the authentic Station puppet mounts the real per-hull console and reads the desk\'s own text scale',
  async ({ context }, testInfo) => {
    test.setTimeout(90000);
    const world = fs.readFileSync(path.resolve(__dirname, '../fixtures/worlds/gm_npc_doctrine.toml'), 'utf8');
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.setViewportSize({ width: GM_VIEWPORT.width, height: GM_VIEWPORT.height });
    await joinAsReadyGm(page, world);

    // The fixture's player ship is an Alliance Cruiser (#1424 corrected its
    // Tactical console at 200%). A real Ship/Station row is selected as soon
    // as the projection has one — no takeover click required to READ it.
    await page.waitForFunction(() =>
      (window.__hostGmStationState?.().projection?.ships || []).length > 0);
    const consoleUrl = await page.evaluate(() =>
      window.__hostGmStationState().selectedRow?.station?.console);
    // A real authored `StationConfig.console` path (`assets/entities/*.toml`),
    // never a second/synthetic GM-only document.
    expect(consoleUrl).toMatch(/^gui\/[a-z-]+(\/[a-z-]+)?\.html$/);
    await expect(page.locator('#gm-station-frame')).toHaveAttribute('src', consoleUrl);

    const frameFontPx = () => page.locator('#gm-station-frame').evaluate(el =>
      parseFloat(getComputedStyle(el.contentDocument.documentElement).fontSize));
    const before = await frameFontPx();
    expect(before).toBeGreaterThan(0);

    await page.evaluate((scale) => document.documentElement.style
      .setProperty('--a11y-text-scale', String(scale)), MAX_TEXT_SCALE);
    // The puppet's own tick cadence (`gm_station` arriving again) is what
    // carries the change in — no reload, matching the production comment in
    // `gui/gm-station-puppet.js`.
    await page.waitForFunction((prev) => {
      const el = document.getElementById('gm-station-frame');
      const doc = el && el.contentDocument;
      return !!doc && parseFloat(getComputedStyle(doc.documentElement).fontSize) > prev;
    }, before, { timeout: 10000 });
    const after = await frameFontPx();
    expect(after).toBeGreaterThan(before);

    await page.evaluate(() => document.documentElement.style.removeProperty('--a11y-text-scale'));
    expect(errors).toEqual([]);
  });

// Issue #1430: browser zoom, tested separately from the Phoenix text setting
// (PRD #1418: "Verify browser zoom separately; do not assume a universally
// available browser query for the Windows text-size percentage") — a gap
// across every GM spec until now. Emulated the way the browser itself does
// it (a shrunk CSS viewport plus a grown device pixel ratio), matching
// `tests/smoke/text-scale-power-workflow.spec.js`. One representative step
// rather than the full `BROWSER_ZOOMS` sweep: unlike a static console
// document, a GM page pays a full deterministic-simulation boot per browser
// context, and this file already carries that cost four times over.
test('browser zoom on the GM desk works alongside the Phoenix text setting', async ({ browser }, testInfo) => {
  test.setTimeout(90000);
  const world = fs.readFileSync(path.resolve(__dirname, '../fixtures/worlds/gm_npc_doctrine.toml'), 'utf8');
  const zoom = BROWSER_ZOOMS[BROWSER_ZOOMS.length - 2]; // 1.5, bracketing 150%/200% text without the 4x cost
  const zoomContext = await browser.newContext({
    viewport: {
      width: Math.round(GM_VIEWPORT.width / zoom),
      height: Math.round(GM_VIEWPORT.height / zoom),
    },
    deviceScaleFactor: zoom,
  });
  try {
    const page = await zoomContext.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await joinAsReadyGm(page, world);

    await expect(page.locator('#gm-workspace')).toBeVisible();
    await expect(page.locator('#gm-mission-panel')).toBeVisible();
    const before = await page.evaluate(() =>
      parseFloat(getComputedStyle(document.getElementById('gm-console')).fontSize));

    // Additive, not exclusive: the Phoenix ceiling on top of browser zoom
    // still enlarges, and the desk still fits without a sideways scrollbar.
    await page.evaluate((scale) => document.documentElement.style
      .setProperty('--a11y-text-scale', String(scale)), MAX_TEXT_SCALE);
    const after = await page.evaluate(() =>
      parseFloat(getComputedStyle(document.getElementById('gm-console')).fontSize));
    expect(after).toBeGreaterThan(before);
    const geometry = await page.locator('#gm-workspace').evaluate(el => ({
      scroll: el.scrollWidth, width: el.clientWidth,
      body: document.documentElement.scrollWidth, viewport: document.documentElement.clientWidth,
    }));
    expect(geometry.scroll).toBeLessThanOrEqual(geometry.width);
    expect(geometry.body).toBeLessThanOrEqual(geometry.viewport);
    expect(errors).toEqual([]);
  } finally {
    await zoomContext.close();
  }
});

// Issue #1430: forced colours, tested as the browser's own behaviour (PRD
// #1418: "A Phoenix contrast selection is not permission to defeat
// browser-enforced colours") — also a gap across every GM spec until now.
// Covers the general focus-ring/edge repair `gui/tokens.css` already carries
// for every endpoint (issue #1422) AND the GM-specific redundant border this
// issue adds for the roster's selected entity, since a forced palette is
// free to flatten `background: var(--gold)` on the pressed button to the
// system's own button colour.
test('forced colours keep the GM desk\'s focus ring, edges and selected entity visible',
  async ({ context }, testInfo) => {
    test.setTimeout(90000);
    const world = fs.readFileSync(path.resolve(__dirname, '../fixtures/worlds/gm_npc_doctrine.toml'), 'utf8');
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.setViewportSize({ width: GM_VIEWPORT.width, height: GM_VIEWPORT.height });
    await page.emulateMedia({ forcedColors: 'active' });
    await joinAsReadyGm(page, world);

    await expect(page.locator('#gm-roster-ships button').first()).toBeVisible();
    await page.locator('#gm-roster-ships button').first().click();
    const row = page.locator('.gm-roster-row', { has: page.locator('button[aria-pressed="true"]') });
    await expect(row).toHaveCount(1);
    const edge = await row.evaluate(el => getComputedStyle(el).borderLeftColor);
    expect(edge, 'the selected row draws a real forced-colours border').not.toBe('rgba(0, 0, 0, 0)');

    // Under forced colours the ring redraws as a BORDER on the chamfered
    // `.btn-bg` body (`ph-console-styles.js`), not an outline on the button
    // itself — the box-shadow ring `forced-colors` drops entirely.
    await row.locator('button').focus();
    const ring = await row.locator('button .btn-bg').evaluate(el => {
      const s = getComputedStyle(el);
      return { width: parseFloat(s.borderTopWidth), colour: s.borderTopColor };
    });
    expect(ring.width, 'the forced-colours focus border is drawn').toBeGreaterThan(0);
    expect(ring.colour, 'the forced-colours focus ring is a real colour').not.toBe('rgba(0, 0, 0, 0)');

    expect(errors).toEqual([]);
  });

// Issue #1430 review, blocking finding: the "performing" surface (Comms
// Studio #1317, Knowledge Compare #1318, role presets #1319) is named
// alongside the directing panels above as in scope for this issue, but
// nothing here had touched it at 200% text. #1421's own `DENSE_CONTENT
// .gmLists` fixture names two rows the Knowledge Compare panel renders
// (`server.gm.knowledge.hint`, `server.gm.knowledge.summary`); those are now
// driven through the real controller at the jsdom level
// (`tests/client/gm-directing-performing-dense-content.test.js`) — this is
// the companion Playwright half, proving the real running page never clips
// them. It also stresses the one pre-existing panel-specific CSS rule on
// this surface the issue's own audit missed (`#gm-role-preset-label`'s
// `max-width`, gui/gm-workspace.css — present before this issue's diff, not
// added by it) with a maximally long preset id: that rule constrains the
// LABEL box only (no `overflow:hidden`/`text-overflow`/`white-space:nowrap`
// on it), so the real risk it could pose is pushing the desk into sideways
// scroll, not silently clipping text — the same "no sideways scrollbar"
// contract every other case in this file already checks.
test('GM performing panels (Comms, Knowledge Compare, role presets) stay reachable and unclipped at 200% text',
  async ({ context }, testInfo) => {
    test.setTimeout(90000);
    const world = fs.readFileSync(path.resolve(__dirname, '../fixtures/worlds/gm_npc_doctrine.toml'), 'utf8');
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.setViewportSize({ width: GM_VIEWPORT.width, height: GM_VIEWPORT.height });
    await joinAsReadyGm(page, world);

    // Knowledge Compare: a real ship's Truth/Crew comparison, waited for
    // exactly as the #1318 exit tests do (tests/smoke/gm-page.spec.js).
    await page.waitForFunction(() => !!window.__hostGmKnowledgeState?.().selectedShipId);
    await page.waitForFunction(() => {
      const rows = document.getElementById('gm-knowledge-contacts-rows');
      return !!rows && rows.children.length > 0;
    });

    // A maximally long preset id, injected through the real controller's own
    // exposed global (`gui/gm-workspace.js`) rather than fabricated markup —
    // an empty `label` falls back to rendering the id itself
    // (`gm-role-presets.js`'s `paintOptions`), so this needs no new
    // String-Table row to carry 80 real characters into the option text.
    const longPresetId = 'x'.repeat(80);
    await page.evaluate((id) => window.__hostGmRolePresetsSetAvailable(JSON.stringify([
      { id, label: '', panels: [], quick_actions: [], contacts: [] },
    ])), longPresetId);
    await page.selectOption('#gm-role-preset-select', longPresetId);

    await page.evaluate(() => document.documentElement.style.setProperty('--a11y-text-scale', '2'));

    await expect(page.locator('#gm-knowledge-panel')).toBeVisible();
    const hint = await page.locator('#gm-knowledge-hint').textContent();
    expect(hint?.length).toBeGreaterThan(100);
    await expect(page.locator('#gm-knowledge-hint')).toBeVisible();
    const summary = await page.locator('#gm-knowledge-contacts-summary').textContent();
    expect(summary?.trim().length).toBeGreaterThan(0);

    await expect(page.locator('#gm-comms-panel')).toBeVisible();
    await expect(page.locator('#gm-role-preset-label')).toBeVisible();
    await expect(page.locator('#gm-role-preset-select')).toHaveValue(longPresetId);

    const geometry = await page.locator('#gm-workspace').evaluate(el => ({
      scroll: el.scrollWidth, width: el.clientWidth,
      body: document.documentElement.scrollWidth, viewport: document.documentElement.clientWidth,
    }));
    expect(geometry.scroll, 'no sideways scroll inside the desk').toBeLessThanOrEqual(geometry.width);
    expect(geometry.body, 'no sideways scroll on the page').toBeLessThanOrEqual(geometry.viewport);

    await page.evaluate(() => document.documentElement.style.removeProperty('--a11y-text-scale'));
    expect(errors).toEqual([]);
  });
