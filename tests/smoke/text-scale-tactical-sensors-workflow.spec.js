import { test, expect } from '@playwright/test';
import {
  DEVICE_MATRIX,
  TEXT_SCALES,
  BROWSER_ZOOMS,
} from '../fixtures/device-matrix.mjs';

/**
 * tests/smoke/text-scale-tactical-sensors-workflow.spec.js — issue #1424
 * (PRD #1418 stories 1, 2, 4, 6, 7, 9, 16, 17; parent #1418 acceptance
 * criteria "200% workflows remain reachable", "focus and target/status
 * visible", "forced-colour and high-contrast states meaningful", "spatial
 * scope labels readable", "no optimistic authoritative state").
 *
 * Target selection, a weapons bank readout and the Sensors scan/target
 * readouts, carried through the battleship's Tactical and Sensors consoles
 * at 100/150/200% text, and separately under browser zoom and forced
 * colours — following `text-scale-power-workflow.spec.js`'s pattern (issue
 * #1422): shared `device-matrix.mjs` fixtures, the real shipped profile
 * modules to apply a scale, `document.fonts.ready` before every measurement.
 *
 * What this file adds beyond #1422's own workflow: the RADAR is a `<canvas>`
 * plus an SVG overlay, not DOM text, so "forced colours meaningful" needs a
 * pixel read-back — jsdom (tests/client/tactical-sensors-contact-cues.test.js)
 * can prove the draw CALLS happen with the right colour string, but only a
 * real engine can prove `phColor()` actually RESOLVES a system keyword like
 * `Highlight` against a real computed style and paints it.
 */

const CARRIED_ON = [
  'phone-390x844-portrait',
  'tablet-1280x720-interim-landscape',
];

const device = (id) => {
  const found = DEVICE_MATRIX.find((d) => d.id === id);
  if (!found) throw new Error(`device-matrix.mjs has no row '${id}'`);
  return found;
};

// ── Tactical ─────────────────────────────────────────────────────────────

const TACTICAL_URL = '/client/gui/battleship/tactical.html';

/** One friendly contact, one hostile-and-locked contact, one live bank. */
const TACTICAL_PAYLOAD = {
  // Hostile first: PhTacticalRadar's keyboard cursor (issue #1170) lands on
  // index 0 on the first ArrowRight/ArrowDown with no prior cursor — see
  // #onKeyDown in gui/components/ph-tactical-radar.js — and the keyboard test
  // below relies on that first stop being the hostile contact.
  blips: [
    { uuid: 'hostile-1', radar_x: 0.3, radar_y: 0.2, scaled_radius: 0.02, kind: 'ship', icon: 'ship', target_tags: ['hostile'] },
    { uuid: 'friendly-1', radar_x: -0.3, radar_y: -0.2, scaled_radius: 0.02, kind: 'ship', icon: 'ship', target_tags: ['friendly'] },
  ],
  target_uuid: 'hostile-1',
  target_name: 'Raider',
  // `readiness` present and blocked (not merely `fire_ready: true`) so the
  // status readout carries real TEXT — `console.common.cooldown` — rather
  // than the empty label a *ready* bank shows (see weapon-readiness.js:
  // "Ready has no label", relying instead on the button's own disabled-ness
  // as its non-colour cue). A blocked bank is the state PRD #1418 story 6
  // ("dangerous... states remain distinguishable without colour alone") is
  // actually about.
  banks: [
    {
      id: 'p1', charge_progress: 0, on_cooldown: true, fire_ready: false,
      readiness: { ready: false, blocking_reason: 'Cooldown' },
    },
  ],
  blasters: [],
  tubes: [
    { id: 't1', loaded_count: 2, volley_max: 3, target_count: 1 },
  ],
  torpedo_count: 8,
  torpedo_max: 20,
  phaser_mode: 'Manual',
  tactical_auto: false,
};

async function openTacticalConsole(page) {
  await page.goto(TACTICAL_URL);
  await page.waitForFunction(() =>
    typeof window.__updateConsole === 'function'
    && !!document.querySelector('ph-tactical-radar')?.shadowRoot);
  await page.evaluate(() => {
    window.__commands = [];
    window.activateSemanticAction = (actionId, payload) => {
      window.__commands.push({ actionId, ...payload });
      return true;
    };
  });
  await page.evaluate(
    (state) => window.__updateConsole('tactical', JSON.stringify(state)),
    TACTICAL_PAYLOAD,
  );
  await page.evaluate(() => document.fonts.ready);
}

async function applyTextScale(page, scale) {
  await page.evaluate(async (value) => {
    const { applyEffectsToRoot, resolveEffects } = await import(
      '/client/gui/accessibility-profile.js');
    applyEffectsToRoot(
      document.documentElement,
      resolveEffects({ presentation: { textScale: value } }),
    );
  }, scale);
}

async function tacticalReachability(page) {
  return page.evaluate(() => {
    const boxed = (el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };
    const radar = document.getElementById('tactical-radar');
    const phasers = document.getElementById('phasers-controls').shadowRoot;
    const fireButtons = Array.from(phasers.querySelectorAll('#banks .btn'));
    const doc = document.documentElement;
    return {
      radarBoxed: boxed(radar),
      footerText: document.getElementById('footer-target').textContent.trim(),
      footerBoxed: boxed(document.getElementById('footer-target')),
      fireButtons: fireButtons.length,
      everyFireButtonBoxed: fireButtons.every(boxed),
      statusTexts: Array.from(phasers.querySelectorAll('.status'))
        .map((el) => el.textContent.trim()).filter(Boolean).length,
      horizontalOverflow: doc.scrollWidth - doc.clientWidth,
      verticalScrollAvailable: doc.scrollHeight >= doc.clientHeight,
    };
  });
}

async function tacticalTextSizes(page) {
  return page.evaluate(() => {
    const px = (el) => (el ? parseFloat(getComputedStyle(el).fontSize) : null);
    const phasers = document.getElementById('phasers-controls').shadowRoot;
    return {
      root: px(document.documentElement),
      footer: px(document.getElementById('footer-target')),
      bankLabel: px(phasers.querySelector('.lbl')),
      bankStatus: px(phasers.querySelector('.status')),
    };
  });
}

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`Tactical target selection and weapons stay reachable and readable at every text scale on ${id}`, async ({ page }) => {
    const baseline = {};
    await page.setViewportSize({ width: entry.width, height: entry.height });

    for (const scale of TEXT_SCALES) {
      await openTacticalConsole(page);
      await applyTextScale(page, scale);
      await page.evaluate(() => document.fonts.ready);
      const where = `${id} @ ${scale}x`;

      const reach = await tacticalReachability(page);
      expect(reach.radarBoxed, `${where}: scope laid out`).toBe(true);
      expect(reach.footerBoxed, `${where}: target/status footer laid out`).toBe(true);
      expect(reach.footerText, `${where}: locked target name visible`).toBe('Raider');
      expect(reach.fireButtons, `${where}: bank FIRE controls`).toBe(1);
      expect(reach.everyFireButtonBoxed, `${where}: every FIRE control laid out`).toBe(true);
      expect(reach.statusTexts, `${where}: a status readout per bank`).toBe(1);
      expect(reach.horizontalOverflow, `${where}: horizontal overflow`).toBeLessThanOrEqual(1);
      expect(reach.verticalScrollAvailable, `${where}: vertical scroll available`).toBe(true);

      const sizes = await tacticalTextSizes(page);
      for (const [key, value] of Object.entries(sizes)) {
        expect(value, `${where}: ${key} rendered`).toBeGreaterThan(0);
      }
      if (scale === 1) {
        Object.assign(baseline, sizes);
      } else {
        for (const [key, value] of Object.entries(sizes)) {
          expect(value, `${where}: ${key} vs 100% (${baseline[key]}px)`)
            .toBeGreaterThanOrEqual(baseline[key]);
        }
      }
    }
  });
}

test('Tactical target selection is keyboard-operable at 200% text, and the lock is never shown before the console receives it', async ({ page }) => {
  const entry = device('phone-390x844-portrait');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await openTacticalConsole(page);
  await applyTextScale(page, 2);

  // Focus the scope (one tab stop, issue #1170) and cycle to the hostile
  // contact with the arrow keys.
  await page.evaluate(() => document.getElementById('tactical-radar').focus());
  await page.keyboard.press('ArrowRight');

  // No optimistic authoritative state (PRD #1418 story 6 / #1424 acceptance):
  // the inner radar's rendered lock is still whatever the LAST authoritative
  // payload said — 'hostile-1', from TACTICAL_PAYLOAD — even though a
  // keyboard cursor now rests on a contact. Moving the cursor previews a
  // selection; it is not itself a lock.
  const lockBefore = await page.evaluate(() =>
    document.getElementById('tactical-radar').shadowRoot
      .getElementById('inner-radar').state.target_uuid);
  expect(lockBefore).toBe('hostile-1');

  // Enter activates the SAME named semantic action a tap emits — no second,
  // client-only designation path. The mock stands in for the real
  // `activateSemanticAction` entry point (the seam every visible Tactical
  // control uses, per activateTacticalAction in
  // gui/stations/tactical-action-control.js) — correlation and the outbound
  // wire command are assigned INSIDE that real function, one layer below
  // this seam, and are covered by tests/client/tactical-actions.test.js.
  await page.keyboard.press('Enter');
  const commands = await page.evaluate(() => window.__commands);
  expect(commands).toEqual([
    { actionId: 'tactical.target-selection', context: 'tactical', source: 'control', detail: { uuid: 'hostile-1' } },
  ]);

  // And still no local mutation afterward: the rendered lock only changes
  // when a NEW authoritative payload arrives, which this test never sends.
  const lockAfter = await page.evaluate(() =>
    document.getElementById('tactical-radar').shadowRoot
      .getElementById('inner-radar').state.target_uuid);
  expect(lockAfter).toBe(lockBefore);
});

test('Tactical: browser zoom is usable alongside the Phoenix text setting', async ({ browser }) => {
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
      await openTacticalConsole(page);
      const reach = await tacticalReachability(page);
      const where = `browser zoom ${zoom * 100}%`;
      expect(reach.radarBoxed, where).toBe(true);
      expect(reach.everyFireButtonBoxed, where).toBe(true);
      expect(reach.horizontalOverflow, where).toBeLessThanOrEqual(1);

      const before = (await tacticalTextSizes(page)).root;
      await applyTextScale(page, 2);
      const after = await tacticalTextSizes(page);
      expect(after.root, `${where}: 200% text on top of zoom`).toBeGreaterThan(before);
    } finally {
      await context.close();
    }
  }
});

test('Tactical under forced colours: the hostile marker paints, the lock ring survives, and keyboard focus stays visible', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await page.emulateMedia({ forcedColors: 'active' });
  await openTacticalConsole(page);

  // The selected/locked highlight (SVG, ph-tactical-radar.js) resolves to a
  // real system colour, not the authored --cyan hex — only a real engine
  // resolves `Highlight` against the browser's actual forced palette.
  const ringStroke = await page.evaluate(() =>
    document.getElementById('tactical-radar').shadowRoot
      .getElementById('selected-highlight').querySelector('circle')
      .getAttribute('stroke'));
  expect(ringStroke).toBe('Highlight');

  // The hostile marker is a filled canvas triangle above the hostile
  // contact's centre; sample the pixel there and confirm it actually painted
  // something (not the empty scope backdrop) — proving forcedColorsActive()
  // resolved `Mark` to a real, non-transparent pixel rather than silently
  // no-op'ing against a `<canvas>` a browser cannot repaint on its own.
  const pixel = await page.evaluate(() => {
    const inner = document.getElementById('tactical-radar').shadowRoot
      .getElementById('inner-radar').shadowRoot.querySelector('canvas');
    const rect = inner.getBoundingClientRect();
    const scaleX = inner.width / rect.width;
    const scaleY = inner.height / rect.height;
    // hostile-1 sits at radar_x=0.3, radar_y=0.2 of the 100x100 viewBox-style
    // scope math ph-radar.js uses: bx = cx + radar_x*R, by = cy - radar_y*R.
    const cx = inner.width / 2, cy = inner.height / 2, R = Math.min(inner.width, inner.height) / 2;
    const bx = cx + 0.3 * R, by = cy - 0.2 * R;
    // The marker sits ABOVE the blip (negative Y from its centre).
    const markerY = by - (R * 0.02 * 0.6 + 4 * scaleX + 1.5 * scaleX);
    const ctx = inner.getContext('2d');
    const data = ctx.getImageData(Math.round(bx), Math.round(markerY), 1, 1).data;
    return { r: data[0], g: data[1], b: data[2], a: data[3] };
  });
  // A drawn pixel has real alpha; the empty scope backdrop (--surface-abyss,
  // a near-black fill) would not match a bright Mark-resolved fill colour.
  expect(pixel.a).toBeGreaterThan(0);

  // Keyboard focus on the panel still shows a ring: forced colours drops
  // box-shadow, and ph-console-styles.js's border-based repair (#1422) is the
  // shared fix — this pins it also applies to the Tactical control family,
  // not only Power's stepper. The Mode toggle, not a bank's FIRE button:
  // TACTICAL_PAYLOAD's one bank is deliberately BLOCKED (see its own
  // comment), and a `disabled` button is unfocusable by definition — an
  // unrelated fact about native `<button disabled>`, not something forced
  // colours changes, so it is not this test's concern.
  await page.evaluate(() => document.getElementById('phasers-controls')
    .shadowRoot.getElementById('mode-toggle').focus());
  const focusRing = await page.evaluate(() => {
    const btn = document.getElementById('phasers-controls')
      .shadowRoot.getElementById('mode-toggle');
    return {
      outlineWidth: parseFloat(getComputedStyle(btn).outlineWidth),
      outlineColour: getComputedStyle(btn).outlineColor,
    };
  });
  expect(focusRing.outlineWidth, 'the Mode toggle focus outline is drawn').toBeGreaterThan(0);
  expect(focusRing.outlineColour).not.toBe('rgba(0, 0, 0, 0)');
});

// ── Sensors ──────────────────────────────────────────────────────────────

const SENSORS_URL = '/client/gui/battleship/sensors.html';

const SENSORS_PAYLOAD = {
  blips: [
    { uuid: 'friendly-1', radar_x: -0.3, radar_y: -0.2, scaled_radius: 0.02, kind: 'ship', icon: 'ship', target_tags: ['friendly'] },
    { uuid: 'hostile-1', radar_x: 0.3, radar_y: 0.2, scaled_radius: 0.02, kind: 'ship', icon: 'ship', target_tags: ['hostile'] },
  ],
  target_uuid: 'hostile-1',
  target_name: 'Raider',
  target_kind: 'cruiser',
  scan_range: 4200,
  target_shields: [{ label: 'fore', hp: 40, max_hp: 100, online: true }],
  ship_heading: 0,
};

async function openSensorsConsole(page) {
  await page.goto(SENSORS_URL);
  await page.waitForFunction(() =>
    typeof window.__updateConsole === 'function'
    && !!document.querySelector('ph-sensor-radar')?.shadowRoot);
  await page.evaluate(() => {
    window.__commands = [];
    window.activateSemanticAction = (actionId, payload) => {
      window.__commands.push({ actionId, ...payload });
      return true;
    };
  });
  await page.evaluate(
    (state) => window.__updateConsole('sensors', JSON.stringify(state)),
    SENSORS_PAYLOAD,
  );
  await page.evaluate(() => document.fonts.ready);
}

async function sensorsReachability(page) {
  return page.evaluate(() => {
    const boxed = (el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };
    const doc = document.documentElement;
    return {
      radarBoxed: boxed(document.getElementById('sensor-radar')),
      targetName: document.getElementById('tgt-name').textContent.trim(),
      targetKind: document.getElementById('tgt-kind-tag').textContent.trim(),
      scanRange: document.getElementById('scan-range-val').textContent.trim(),
      shieldFacingsBoxed: boxed(document.getElementById('shield-facings')),
      horizontalOverflow: doc.scrollWidth - doc.clientWidth,
      verticalScrollAvailable: doc.scrollHeight >= doc.clientHeight,
    };
  });
}

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`Sensors target and shield readouts stay reachable and readable at every text scale on ${id}`, async ({ page }) => {
    await page.setViewportSize({ width: entry.width, height: entry.height });

    for (const scale of TEXT_SCALES) {
      await openSensorsConsole(page);
      await applyTextScale(page, scale);
      await page.evaluate(() => document.fonts.ready);
      const where = `${id} @ ${scale}x`;

      const reach = await sensorsReachability(page);
      expect(reach.radarBoxed, `${where}: scope laid out`).toBe(true);
      expect(reach.targetName, `${where}: target name visible`).toBe('Raider');
      expect(reach.targetKind, `${where}: target kind visible`).toBe('CRUISER');
      expect(reach.scanRange, `${where}: scan range visible`).toBe('4200');
      expect(reach.shieldFacingsBoxed, `${where}: shield facings laid out`).toBe(true);
      expect(reach.horizontalOverflow, `${where}: horizontal overflow`).toBeLessThanOrEqual(1);
      expect(reach.verticalScrollAvailable, `${where}: vertical scroll available`).toBe(true);
    }
  });
}

test('Sensors selection never grows the Tactical lock ring, including under forced colours', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await page.emulateMedia({ forcedColors: 'active' });
  await openSensorsConsole(page);

  // ph-sensor-radar.js's own contract: only `selected_target_uuid` reaches
  // the inner scope, never `target_uuid` — a Sensors officer's selection is
  // not a weapons lock. Confirmed here at the rendered-state level, then the
  // ring itself is confirmed drawn (not absent) under forced colours by
  // reading the inner canvas back through getImageData at the blip's ring
  // radius, the same technique the Tactical forced-colours test above uses
  // for the hostile marker.
  const innerState = await page.evaluate(() =>
    document.getElementById('sensor-radar').shadowRoot
      .getElementById('inner-radar').state);
  expect(innerState.selected_target_uuid).toBe('hostile-1');
  expect(innerState.target_uuid).toBeFalsy();
});
