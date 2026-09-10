import { test, expect } from '@playwright/test';
import {
  DEVICE_MATRIX,
  TEXT_SCALES,
  BROWSER_ZOOMS,
} from '../fixtures/device-matrix.mjs';

/**
 * tests/smoke/text-scale-comms-captain-workflow.spec.js — issue #1426 (PRD
 * #1418 stories 1, 2, 4, 5, 6, 7).
 *
 * TWO complete console workflows — reading and answering a Comms hail on the
 * battleship's Comms seat, and the Captain's Objective-boost workflow —
 * carried through 100%, 150% and 200% text, and separately under browser zoom
 * and forced colours. A third scenario exercises the courier's Captain seat,
 * the one hull where Captain is a genuine COMPOSITE surface: Shields, Power,
 * Repair, Navigation and Comms all absorbed into one Station, with Navigation
 * and Comms toggled as overlays over the same objectives list.
 *
 * PRD #1418: "ordinary messages enlarge" — unlike the phone-Viewscreen
 * exemption for level-3 AI-to-AI System Coordination (out of scope here:
 * that is a different message class on a different surface), a Comms hail on
 * a player console carries no exemption at all. The long body and both its
 * responses (one importantly irreversible, requiring the existing two-click
 * confirm, one ordinary) below are REAL String Table rows from the shipped
 * `falling_skyway` world (`rigger_hails` -> `on_rigger_ask` ->
 * `rigger_account`), not placeholder text, so this spec exercises the
 * longest scripted Comms body in the table (322 characters) exactly as a
 * player would read it.
 *
 * ── Which bundle ──────────────────────────────────────────────────────────
 * Everything below is served from `/client/...` (`node scripts/build-client.mjs`
 * output). Comms and Captain are pure HTML + `gui/*` modules with no WASM.
 */

const CARRIED_ON = [
  'phone-390x844-portrait',
  'phone-844x390-landscape',
  'tablet-1280x720-interim-landscape',
  'native-split-pane-floor',
];

const device = (id) => {
  const found = DEVICE_MATRIX.find((d) => d.id === id);
  if (!found) throw new Error(`device-matrix.mjs has no row '${id}'`);
  return found;
};

/** Same split-pane floor scaling rule as text-scale-power-workflow.spec.js:
 *  MIN_CONSOLE_LOGICAL_WIDTH_PX scales linearly with the text multiplier. */
function viewportFor(entry, scale) {
  if (entry.kind !== 'split-pane') return { width: entry.width, height: entry.height };
  return { width: Math.round(entry.width * scale), height: entry.height };
}

/**
 * Apply a text scale through the SHIPPED profile modules, not by poking CSS —
 * then let it actually settle. `.resp-btn` (ph-comms-current-message.js)
 * declares `transition: all 0.15s ease`, so `getComputedStyle` immediately
 * after the change can read a MID-TRANSITION font-size instead of the target
 * one; measuring is deliberately deferred past that window rather than
 * asserting on an animation frame.
 */
async function applyTextScale(page, scale) {
  await page.evaluate(async (value) => {
    const { applyEffectsToRoot, resolveEffects } = await import(
      '/client/gui/accessibility-profile.js');
    applyEffectsToRoot(
      document.documentElement,
      resolveEffects({ presentation: { textScale: value } }),
    );
  }, scale);
  await page.waitForTimeout(200);
}

const COMMS_URL = '/client/gui/battleship/comms.html';
const CAPTAIN_URL = '/client/gui/battleship/captain.html';
const COURIER_CAPTAIN_URL = '/client/gui/courier/captain.html';

/**
 * Real scripted `falling_skyway` content: Rigger Tacket's corroboration
 * thread. The first message is already answered (the operator picked "ask");
 * the second — the longest Comms body in the String Table — is the live one,
 * carrying one important response (`rigger_protect`, a commitment that needs
 * the existing two-click confirm) and one ordinary response
 * (`rigger_no_promise`, submits immediately).
 */
async function commsThreadPayload(page) {
  return page.evaluate(async () => {
    const { t } = await import('/client/gui/strings.js');
    const sender = t('world.falling_skyway.entity.rigger_tacket.name');
    return {
      messages: [
        {
          id: 'msg-hail', thread_id: 'thread-tacket', sender_name: sender,
          body: t('world.falling_skyway.comms.rigger_hails'),
          responses: [], selected_response: 0, is_read: true,
        },
        {
          id: 'msg-account', thread_id: 'thread-tacket', sender_name: sender,
          body: t('world.falling_skyway.comms.rigger_account'),
          responses: [
            { text: t('world.falling_skyway.comms.rigger_protect'), important: true, available: true },
            { text: t('world.falling_skyway.comms.rigger_no_promise'), important: false, available: true },
          ],
          selected_response: null, is_read: false,
        },
      ],
      contacts: [
        { uuid: 'tacket', name: sender, stance: 'neutral', in_range: true },
      ],
      comms_auto: false,
    };
  });
}

/**
 * Open the Comms console and push the real thread, then open it the real way
 * a phone operator does: tap its HAILS row. Issue #1380 — proven by the
 * existing `comms-console.spec.js` — makes the open thread an OVERLAY on a
 * portrait phone that starts `display:none` until a row activates it; a fresh
 * push alone never reveals it there (a landscape/desktop layout keeps it on
 * screen the whole time regardless, so the click is a harmless no-op there).
 * The row's own click runs through the REAL semantic-action registry
 * `initConsole` wired up, so `window.activateSemanticAction` is stubbed only
 * AFTER this — capturing just the response-button commands under test.
 */
async function openCommsConsole(page) {
  await page.goto(COMMS_URL);
  await page.waitForFunction(() =>
    typeof window.__updateConsole === 'function'
    && !!document.querySelector('ph-comms-current-message')?.shadowRoot);
  const payload = await commsThreadPayload(page);
  await page.evaluate((state) => window.__updateConsole('comms', JSON.stringify(state)), payload);
  await page.locator('ph-comms-hail-list .row').first().click();
  await page.evaluate(() => {
    window.__commands = [];
    window.activateSemanticAction = (actionId, opts) => {
      window.__commands.push({ actionId, ...opts });
      return true;
    };
  });
  await page.evaluate(() => document.fonts.ready);
  return payload;
}

/** Every rendered string the workflow depends on, plus reachability. */
async function commsWorkflowRead(page) {
  return page.evaluate(() => {
    const shadow = document.querySelector('ph-comms-current-message').shadowRoot;
    const boxed = (el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };
    const bodyEls = Array.from(shadow.querySelectorAll('.msg .text'));
    const respButtons = Array.from(shadow.querySelectorAll('.resp-btn'));
    const doc = document.documentElement;
    return {
      messageCount: bodyEls.length,
      longBody: bodyEls.at(-1)?.textContent || '',
      longBodyBoxed: boxed(bodyEls.at(-1)),
      longBodyFontSize: bodyEls.at(-1) ? parseFloat(getComputedStyle(bodyEls.at(-1)).fontSize) : null,
      responses: respButtons.length,
      everyResponseBoxed: respButtons.every(boxed),
      responseFontSize: respButtons[0] ? parseFloat(getComputedStyle(respButtons[0]).fontSize) : null,
      // Panels may wrap/stack/scroll; nothing may go sideways off the page.
      horizontalOverflow: doc.scrollWidth - doc.clientWidth,
    };
  });
}

// ── 1. The long Comms thread stays readable and its responses reachable ────

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`a long Comms message and its responses are reachable and readable at every text scale on ${id}`, async ({ page }) => {
    const baseline = {};

    for (const scale of TEXT_SCALES) {
      const viewport = viewportFor(entry, scale);
      await page.setViewportSize(viewport);
      await openCommsConsole(page);
      await applyTextScale(page, scale);
      await page.evaluate(() => document.fonts.ready);

      const where = `${id} @ ${scale}x (${viewport.width}x${viewport.height})`;
      const read = await commsWorkflowRead(page);

      // The whole conversation and both live responses are on screen.
      expect(read.messageCount, `${where}: both messages in the thread`).toBe(2);
      expect(read.longBody.length, `${where}: the long body is not truncated`)
        .toBeGreaterThan(300);
      expect(read.longBodyBoxed, `${where}: long body laid out`).toBe(true);
      expect(read.responses, `${where}: both responses present`).toBe(2);
      expect(read.everyResponseBoxed, `${where}: every response laid out`).toBe(true);

      // Reflow, never sideways clipping.
      expect(read.horizontalOverflow, `${where}: horizontal overflow`).toBeLessThanOrEqual(1);

      // Nothing shrinks as the scale rises.
      if (scale === 1) {
        baseline.body = read.longBodyFontSize;
        baseline.resp = read.responseFontSize;
      } else {
        expect(read.longBodyFontSize, `${where}: body vs 100% (${baseline.body}px)`)
          .toBeGreaterThanOrEqual(baseline.body);
        expect(read.responseFontSize, `${where}: response vs 100% (${baseline.resp}px)`)
          .toBeGreaterThanOrEqual(baseline.resp);
      }
    }
    // And the top scale genuinely grew, not merely held steady.
    const top = await commsWorkflowRead(page);
    expect(top.longBodyFontSize).toBeGreaterThan(baseline.body);
    expect(top.responseFontSize).toBeGreaterThan(baseline.resp);
  });
}

// ── 2. The two-step confirm and the ordinary response, from the keyboard ───

test('the important-response two-click confirm and the ordinary one-click response stay keyboard-operable at 200% text', async ({ page }) => {
  const entry = device('phone-390x844-portrait');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await openCommsConsole(page);
  await applyTextScale(page, 2);
  await page.evaluate(() => document.fonts.ready);

  const respButtons = () => page.evaluate(() => Array.from(
    document.querySelector('ph-comms-current-message').shadowRoot.querySelectorAll('.resp-btn'),
  ).map((b) => ({
    text: b.textContent.trim(), important: b.dataset.important, disabled: b.disabled,
  })));

  const confirmLabel = await page.evaluate(async () =>
    (await import('/client/gui/strings.js')).t('component.comms_message.confirm_important'));

  // ── The ordinary response submits on one click, unchanged ────────────────
  await page.evaluate(() => document.querySelector('ph-comms-current-message')
    .shadowRoot.querySelectorAll('.resp-btn')[1].focus());
  const focusedIsOrdinary = await page.evaluate(() => {
    const btn = document.querySelector('ph-comms-current-message').shadowRoot.activeElement;
    return btn && btn.dataset.important === 'false';
  });
  expect(focusedIsOrdinary, 'the ordinary response is a real keyboard tab stop').toBe(true);
  await page.keyboard.press('Enter');
  expect(await page.evaluate(() => window.__commands)).toEqual([
    {
      actionId: 'comms.respond', source: 'control',
      detail: { message_id: 'msg-account', response_index: 1, confirmed: false },
    },
  ]);

  // ── Fresh state: the important response ARMS on the first activation ─────
  await openCommsConsole(page);
  await applyTextScale(page, 2);
  await page.evaluate(() => document.querySelector('ph-comms-current-message')
    .shadowRoot.querySelectorAll('.resp-btn')[0].focus());
  await page.keyboard.press('Enter');
  expect(await page.evaluate(() => window.__commands), 'nothing sent while only armed').toEqual([]);
  let armed = await respButtons();
  expect(armed[0].text, '200%: the confirm prompt is the localized string').toBe(confirmLabel);

  // The SAME control keeps keyboard focus across the re-render an arm triggers
  // — a composite surface's focus/status must survive its own repaint.
  const stillFocused = await page.evaluate(() => {
    const btn = document.querySelector('ph-comms-current-message').shadowRoot.activeElement;
    return btn && btn.classList.contains('resp-btn') && btn.dataset.important === 'true';
  });
  expect(stillFocused, '200%: focus stays on the armed control').toBe(true);

  // ── …and SENDS on the second activation, confirmed ────────────────────────
  await page.keyboard.press('Enter');
  expect(await page.evaluate(() => window.__commands)).toEqual([
    {
      actionId: 'comms.respond', source: 'control',
      detail: { message_id: 'msg-account', response_index: 0, confirmed: true },
    },
  ]);
});

// ── 2b. The unavailable/refused path stays readable at 200% ────────────────
//
// Acceptance: "Behavioral tests cover this complete path and its refusal/
// unavailable states." tests/smoke/comms-console.spec.js already proves the
// mechanics (disabled control, forced submission, Refused feedback) at 100%;
// this is that same real path carried to the text-scale ceiling — an
// out-of-range sender's response stays visibly boxed rather than disappearing,
// and the error feedback banner (PRD #1418 story 4: "errors ... enlarge as
// reliably as the normal interface") is not exempt from growing either.

test('an unavailable response and its Refused feedback stay readable at 200% text', async ({ page }) => {
  const entry = device('phone-390x844-portrait');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await page.goto(COMMS_URL);
  await page.waitForFunction(() =>
    typeof window.__updateConsole === 'function'
    && !!document.querySelector('ph-comms-current-message')?.shadowRoot);

  const payload = await page.evaluate(async () => {
    const { t } = await import('/client/gui/strings.js');
    const sender = t('world.falling_skyway.entity.rigger_tacket.name');
    return {
      messages: [{
        id: 'msg-out-of-range', thread_id: 'thread-tacket', sender_name: sender,
        body: t('world.falling_skyway.comms.rigger_account'),
        responses: [
          { text: t('world.falling_skyway.comms.rigger_protect'), important: true, available: true },
          { text: t('world.falling_skyway.comms.rigger_no_promise'), important: false, available: false },
        ],
        selected_response: null, is_read: false,
      }],
      contacts: [{
        uuid: 'tacket', name: sender, stance: 'neutral', in_range: false,
      }],
    };
  });
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(JSON.parse(json));
  });
  await page.evaluate((state) => window.__updateConsole('comms', JSON.stringify(state)), payload);
  await page.locator('ph-comms-hail-list .row').first().click();
  await applyTextScale(page, 2);
  await page.evaluate(() => document.fonts.ready);

  const before = await page.evaluate(() => {
    const shadow = document.querySelector('ph-comms-current-message').shadowRoot;
    const btns = Array.from(shadow.querySelectorAll('.resp-btn'));
    const boxed = (el) => {
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };
    const unavailable = btns[1];
    return {
      count: btns.length,
      boxed: boxed(unavailable),
      disabled: unavailable.disabled,
      title: unavailable.title,
      fontSize: parseFloat(getComputedStyle(unavailable).fontSize),
    };
  });
  const expectedTitle = await page.evaluate(async () =>
    (await import('/client/gui/strings.js')).t('component.comms_message.unavailable'));
  expect(before.count, 'both responses present').toBe(2);
  expect(before.boxed, 'the unavailable response stays visible, not hidden').toBe(true);
  // Not colour-alone: `disabled` (assistive tech) and a real tooltip string.
  expect(before.disabled, 'unavailable: disabled, not merely tinted').toBe(true);
  expect(before.title, 'unavailable: the real tooltip string').toBe(expectedTitle);
  expect(before.fontSize, 'unavailable: text grew past the 11px floor').toBeGreaterThan(11);

  // A forced submission (bypassing the disabled control — a stale/forged
  // activation, exactly like the existing comms-console.spec.js coverage)
  // still reaches the host and is genuinely refused.
  const correlation = await page.evaluate(() => {
    window.activateSemanticAction('comms.respond', {
      source: 'control',
      detail: { message_id: 'msg-out-of-range', response_index: 1 },
    });
    return window.__sent.at(-1)?.correlation;
  });
  expect(correlation, 'the forced submission reached the host').toEqual(expect.any(String));
  await page.evaluate((c) => window.__updateActionFeedback({ correlation: c, state: 'Refused' }), correlation);

  const feedback = page.locator('.semantic-action-feedback__item[data-action-id="comms.respond"]');
  await expect(feedback).toHaveAttribute('data-state', 'Refused');
  const feedbackFontSize = await feedback.evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
  expect(feedbackFontSize, 'the Refused banner text also grew — errors are not exempt').toBeGreaterThan(11);
});

// ── 3. Captain: the Objective-boost workflow, keyboard-operable at 200% ────

async function objectivesPayload(page) {
  return page.evaluate(async () => {
    const { t } = await import('/client/gui/strings.js');
    return {
      objectives: [
        { id: 'obj-survey', text: t('world.falling_skyway.objective.survey.text'), done: false },
        { id: 'obj-corridor', text: t('world.falling_skyway.objective.corridor.text'), done: true },
      ],
      boosted_objective_id: 'obj-survey',
      red_alert: false,
      view_direction: 'camera_fore',
      camera_views: ['camera_fore'],
    };
  });
}

test('the Objective list stays readable, boostable and reachable from the keyboard at 200% text', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await page.goto(CAPTAIN_URL);
  await page.waitForFunction(() =>
    typeof window.__updateConsole === 'function'
    && !!document.querySelector('ph-objective-list')?.shadowRoot);
  await page.evaluate(() => {
    window.__commands = [];
    window.activateSemanticAction = (actionId, payload) => {
      window.__commands.push({ actionId, ...payload });
      return true;
    };
  });
  const payload = await objectivesPayload(page);
  await page.evaluate((state) => window.__updateConsole('captain', JSON.stringify(state)), payload);
  await applyTextScale(page, 2);
  await page.evaluate(() => document.fonts.ready);

  const read = () => page.evaluate(() => {
    const shadow = document.querySelector('ph-objective-list').shadowRoot;
    const boxed = (el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };
    const rows = Array.from(shadow.querySelectorAll('.row'));
    return {
      rows: rows.length,
      everyRowBoxed: rows.every(boxed),
      texts: rows.map((r) => r.querySelector('.text').textContent),
      // The done/pending distinction is never colour-alone: a checkmark glyph
      // plus a strikethrough survive contrast and forced colours together.
      doneMark: getComputedStyle(rows[1].querySelector('.indicator'), '::after').content,
      pendingMark: getComputedStyle(rows[0].querySelector('.indicator'), '::after').content,
      doneStrike: getComputedStyle(rows[1].querySelector('.text')).textDecorationLine,
      pendingStrike: getComputedStyle(rows[0].querySelector('.text')).textDecorationLine,
      boostedRow: rows.findIndex((r) => r.getAttribute('aria-selected') === 'true'),
      horizontalOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
    };
  });

  const before = await read();
  expect(before.rows, 'both objectives laid out').toBe(2);
  expect(before.everyRowBoxed, 'every objective row boxed').toBe(true);
  expect(before.texts, 'long objective text unshortened').toEqual(payload.objectives.map((o) => o.text));
  expect(before.doneMark, 'done glyph, not colour alone')
    .not.toBe(before.pendingMark);
  expect(before.doneMark).not.toBe('none');
  expect(before.doneStrike).toContain('line-through');
  expect(before.pendingStrike).toBe('none');
  expect(before.boostedRow, 'the authored boosted objective is the selected option').toBe(0);
  expect(before.horizontalOverflow, 'no sideways overflow').toBeLessThanOrEqual(1);

  // Activate the second (pending → done) row from the keyboard and confirm the
  // semantic action names it, unaffected by text scale.
  await page.evaluate(() => document.querySelector('ph-objective-list')
    .shadowRoot.querySelectorAll('.row')[1].focus());
  await page.keyboard.press('Enter');
  expect(await page.evaluate(() => window.__commands)).toEqual([
    {
      actionId: 'captain.objective-priority',
      context: 'captain',
      source: 'control',
      detail: { id: 'obj-corridor' },
    },
  ]);
});

// ── 4. The courier's composite Captain: five systems, focus/status survive
//      the console's own ~10Hz repaint ──────────────────────────────────────

async function courierPayload(page) {
  const comms = await commsThreadPayload(page);
  const captainCore = await objectivesPayload(page);
  return {
    system_ids: ['captain', 'shields', 'power', 'repair', 'navigation', 'comms'],
    system_families: {
      captain: 'captain', shields: 'shields', power: 'power',
      repair: 'repair', navigation: 'navigation', comms: 'comms',
    },
    systems: {
      captain: captainCore,
      shields: { facings: [], focused_facing: null, shields_auto: false, threat_bearing: null },
      power: { consoles: [], power_auto: false, battery_online: true, charging: false, battery_charge: 80, battery_max: 100 },
      repair: {
        overall_hull: { pct: 1, destroyed_pct: 0 }, teams: [], repair_auto: false,
        dispatch_targets: [], damaged_systems: [], external_dispatch: null,
      },
      navigation: {
        blips: [], regions: [], radar_range: 800, ship_x: 0, ship_z: 0,
        ship_heading: 0, waypoint: null, navigation_auto: false,
      },
      comms,
    },
  };
}

test('the courier composite Captain seat keeps focus and the boosted objective through its own repaint at 200% text', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await page.goto(COURIER_CAPTAIN_URL);
  await page.waitForFunction(() =>
    typeof window.__updateConsole === 'function'
    && !!document.querySelector('ph-objective-list')?.shadowRoot);

  const payload = await courierPayload(page);
  await page.evaluate((state) => window.__updateConsole('captain', JSON.stringify(state)), payload);
  await applyTextScale(page, 2);
  await page.evaluate(() => document.fonts.ready);

  // Focus the pending objective row and hold it there.
  await page.evaluate(() => document.querySelector('ph-objective-list')
    .shadowRoot.querySelectorAll('.row')[1].focus());
  const focusedBefore = await page.evaluate(() => {
    const el = document.querySelector('ph-objective-list').shadowRoot.activeElement;
    return el && el.querySelector('.text').textContent;
  });
  expect(focusedBefore).toBe(payload.systems.captain.objectives[1].text);

  // The shell pushes console state ~10x a second (console-core.js). Three
  // repeated pushes of the SAME payload must not tear down and recreate the
  // focused row, and must not silently drop the boosted objective.
  for (let i = 0; i < 3; i += 1) {
    // eslint-disable-next-line no-await-in-loop
    await page.evaluate((state) => window.__updateConsole('captain', JSON.stringify(state)), payload);
  }
  const stillFocused = await page.evaluate(() => {
    const el = document.querySelector('ph-objective-list').shadowRoot.activeElement;
    return el && el.querySelector('.text').textContent;
  });
  expect(stillFocused, 'focus survives repeated console repaints').toBe(focusedBefore);
  const boostedRow = await page.evaluate(() => Array.from(
    document.querySelector('ph-objective-list').shadowRoot.querySelectorAll('.row'),
  ).findIndex((r) => r.getAttribute('aria-selected') === 'true'));
  expect(boostedRow, 'the boosted objective is still marked after repeated repaints').toBe(0);

  // ── The absorbed Comms overlay: same long body, same two responses ───────
  await page.click('#comms-toggle');
  const overlayOpen = await page.evaluate(() =>
    document.getElementById('comms-overlay').classList.contains('open'));
  expect(overlayOpen, 'the Comms overlay opened').toBe(true);
  const commsRead = await commsWorkflowRead(page);
  // The compact courier ships no HAILS list at all (courier/captain.html has
  // no <ph-comms-hail-list>) — its tail hands ph-comms-current-message only
  // the single active message, not the whole conversation, so this composite
  // surface legitimately shows ONE message rather than the dedicated Comms
  // seat's two. What the criterion is actually about — the long body and both
  // its responses staying readable/reachable — still applies in full.
  expect(commsRead.messageCount, 'the composite seat renders the active message').toBe(1);
  expect(commsRead.longBody.length).toBeGreaterThan(300);
  expect(commsRead.responses).toBe(2);
  expect(commsRead.everyResponseBoxed).toBe(true);
  expect(commsRead.horizontalOverflow, 'composite overlay: no sideways overflow').toBeLessThanOrEqual(1);

  // Back, then the Navigation overlay — its own toggle button lives in the
  // Command column BEHIND whichever overlay is open (both overlays are
  // `position: absolute; inset: 0`), so it is reachable only once Comms is
  // closed; this is the real, pre-existing interaction shape (unrelated to
  // text scale — the overlay covers it at 100% too), not a workaround.
  await page.click('#comms-back');
  await page.click('#nav-toggle');
  const navOpen = await page.evaluate(() =>
    document.getElementById('nav-overlay').classList.contains('open'));
  const commsStillOpen = await page.evaluate(() =>
    document.getElementById('comms-overlay').classList.contains('open'));
  expect(navOpen).toBe(true);
  expect(commsStillOpen, 'only one overlay is open at a time').toBe(false);

  await page.click('#nav-back');
  const boostedAfterOverlayRound = await page.evaluate(() => Array.from(
    document.querySelector('ph-objective-list').shadowRoot.querySelectorAll('.row'),
  ).findIndex((r) => r.getAttribute('aria-selected') === 'true'));
  expect(boostedAfterOverlayRound, 'the boosted objective survives an overlay round trip')
    .toBe(0);

  const overallOverflow = await page.evaluate(() =>
    document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overallOverflow, 'the whole composite body: no sideways overflow at 200%').toBeLessThanOrEqual(1);
});

// ── 5. Forced colours: the confirm/unavailable cues are never colour-alone ─

test('under forced colours the Comms focus ring is visible and the confirm cue is not colour-alone', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await page.emulateMedia({ forcedColors: 'active' });
  await page.goto(COMMS_URL);
  await page.waitForFunction(() =>
    typeof window.__updateConsole === 'function'
    && !!document.querySelector('ph-comms-current-message')?.shadowRoot);
  const payload = await commsThreadPayload(page);
  await page.evaluate((state) => window.__updateConsole('comms', JSON.stringify(state)), payload);
  // Keyboard-only, deliberately: COMMS_SELECT_MESSAGE_ACTION's shipped 'm'
  // binding opens the thread exactly as the HAILS-row click would, but a real
  // prior POINTER click flips Chromium's next-focus input modality and
  // suppresses `:focus-visible` on the button focused right after — which
  // would make the assertion below about a mouse artefact of this test, not
  // about the shipped ring.
  await page.keyboard.press('m');

  await page.evaluate(() => document.querySelector('ph-comms-current-message')
    .shadowRoot.querySelectorAll('.resp-btn')[0].focus());
  const ring = await page.evaluate(() => {
    const btn = document.querySelector('ph-comms-current-message').shadowRoot.activeElement;
    const s = getComputedStyle(btn);
    return {
      focusVisible: btn.matches(':focus-visible'),
      outline: s.outlineStyle, outlineColor: s.outlineColor, boxShadow: s.boxShadow,
    };
  });
  expect(ring.focusVisible, 'the focused response is :focus-visible').toBe(true);
  // A forced palette repaints whichever colour property is actually drawn;
  // the control must still show SOME visible focus indication, not none.
  expect(ring.outline !== 'none' || ring.boxShadow !== 'none', 'a focus indicator is drawn')
    .toBe(true);
  expect(ring.outlineColor, 'the ring is a real, non-transparent colour')
    .not.toBe('rgba(0, 0, 0, 0)');

  await page.keyboard.press('Enter'); // arm the important response
  const confirmLabel = await page.evaluate(async () =>
    (await import('/client/gui/strings.js')).t('component.comms_message.confirm_important'));
  const armedText = await page.evaluate(() => document.querySelector('ph-comms-current-message')
    .shadowRoot.querySelectorAll('.resp-btn')[0].textContent.trim());
  // The confirm cue is the STRING, not a colour — it survives forced colours
  // unchanged, which is the point: no forced palette can hide a word.
  expect(armedText, 'forced colours: the confirm prompt is still the real string').toBe(confirmLabel);
});

// ── 6. Browser zoom, tested separately from the Phoenix text setting ───────

test('browser zoom is usable alongside the Phoenix text setting on Comms', async ({ browser }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  for (const zoom of BROWSER_ZOOMS) {
    const context = await browser.newContext({
      viewport: {
        width: Math.round(entry.width / zoom),
        height: Math.round(entry.height / zoom),
      },
      deviceScaleFactor: zoom,
    });
    const page = await context.newPage();
    try {
      await openCommsConsole(page);
      const where = `browser zoom ${zoom * 100}%`;
      const read = await commsWorkflowRead(page);
      expect(read.messageCount, where).toBe(2);
      expect(read.responses, where).toBe(2);
      expect(read.everyResponseBoxed, where).toBe(true);
      expect(read.horizontalOverflow, where).toBeLessThanOrEqual(1);

      // Additive with Phoenix's own ceiling, not exclusive with it.
      const before = read.longBodyFontSize;
      await applyTextScale(page, 2);
      const after = await commsWorkflowRead(page);
      expect(after.longBodyFontSize, `${where}: 200% text on top of zoom`).toBeGreaterThan(before);
      expect(after.everyResponseBoxed, `${where} + 200%`).toBe(true);
      expect(after.horizontalOverflow, `${where} + 200%`).toBeLessThanOrEqual(1);
    } finally {
      await context.close();
    }
  }
});

// ── 7. Every supported hull's Captain/Comms document, at 200% text ─────────
//
// "Complete ... Objective and Captain workflows at enlarged text ...
// including supported hull variants" (issue #1426). Tests 1-6 above carry the
// full workflow on the reference (battleship) and composite (courier) hulls;
// this sweeps the remaining shipped Captain/Comms documents for the same
// baseline: the Objective list (or Comms thread) is readable, reachable and
// never pushed sideways off the page at the text-scale ceiling.

const CAPTAIN_HULLS = [
  { hull: 'battleship', url: CAPTAIN_URL, flat: true },
  { hull: 'cruiser', url: '/client/gui/cruiser/captain.html', flat: true },
  { hull: 'destroyer', url: '/client/gui/destroyer/captain.html', flat: false },
];

for (const { hull, url, flat } of CAPTAIN_HULLS) {
  test(`${hull} Captain: the Objective list is readable and reachable at 200% text`, async ({ page }) => {
    const entry = device('tablet-1280x720-interim-landscape');
    await page.setViewportSize({ width: entry.width, height: entry.height });
    await page.goto(url);
    await page.waitForFunction(() =>
      typeof window.__updateConsole === 'function'
      && !!document.querySelector('ph-objective-list')?.shadowRoot);
    const core = await objectivesPayload(page);
    const state = flat
      ? { ...core, deadlines: [] }
      : {
        system_ids: ['captain', 'sensors'],
        system_families: { captain: 'captain', sensors: 'sensors' },
        systems: {
          captain: core,
          sensors: {
            blips: [], target_uuid: null, scan: {}, deadlines: [],
            target_name: null,
          },
        },
      };
    await page.evaluate((s) => window.__updateConsole('captain', JSON.stringify(s)), state);
    await applyTextScale(page, 2);
    await page.evaluate(() => document.fonts.ready);

    const read = await page.evaluate(() => {
      const shadow = document.querySelector('ph-objective-list').shadowRoot;
      const rows = Array.from(shadow.querySelectorAll('.row'));
      const boxed = (el) => {
        if (!el) return false;
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      };
      return {
        rows: rows.length,
        everyRowBoxed: rows.every(boxed),
        horizontalOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      };
    });
    expect(read.rows, `${hull}: both objectives`).toBe(2);
    expect(read.everyRowBoxed, `${hull}: every row laid out`).toBe(true);
    expect(read.horizontalOverflow, `${hull}: no sideways overflow at 200%`).toBeLessThanOrEqual(1);
  });
}

for (const hull of ['battleship', 'cruiser']) {
  test(`${hull} Comms: the long thread is readable and reachable at 200% text`, async ({ page }) => {
    const entry = device('tablet-1280x720-interim-landscape');
    await page.setViewportSize({ width: entry.width, height: entry.height });
    await page.goto(`/client/gui/${hull}/comms.html`);
    await page.waitForFunction(() =>
      typeof window.__updateConsole === 'function'
      && !!document.querySelector('ph-comms-current-message')?.shadowRoot);
    const core = await commsThreadPayload(page);
    // The cruiser's comms.html shares one document with its auxiliary
    // Navigation Station (gui/cruiser/comms.console.js's `resolveFamilyView`):
    // a single-owned-system payload stays FLAT but still needs
    // `system_ids`/`system_families` naming "comms" as the owned family, or
    // the Comms view resolves empty and the Navigation view takes the screen.
    const payload = hull === 'cruiser'
      ? { ...core, system_ids: ['comms'], system_families: { comms: 'comms' } }
      : core;
    await page.evaluate((state) => window.__updateConsole('comms', JSON.stringify(state)), payload);
    await page.locator('ph-comms-hail-list .row').first().click();
    await applyTextScale(page, 2);
    await page.evaluate(() => document.fonts.ready);

    const read = await commsWorkflowRead(page);
    expect(read.messageCount, `${hull}: both messages`).toBe(2);
    expect(read.longBody.length, `${hull}: long body intact`).toBeGreaterThan(300);
    expect(read.longBodyBoxed, `${hull}: long body laid out`).toBe(true);
    expect(read.responses, `${hull}: both responses`).toBe(2);
    expect(read.everyResponseBoxed, `${hull}: every response laid out`).toBe(true);
    expect(read.horizontalOverflow, `${hull}: no sideways overflow at 200%`).toBeLessThanOrEqual(1);
  });
}
