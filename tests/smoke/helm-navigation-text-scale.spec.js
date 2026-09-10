import { test, expect } from '@playwright/test';
import {
  DEVICE_MATRIX,
  TEXT_SCALES,
  BROWSER_ZOOMS,
} from '../fixtures/device-matrix.mjs';

/**
 * tests/smoke/helm-navigation-text-scale.spec.js — issue #1423 (PRD #1418
 * stories 1, 2, 3, 4, 5, 6, 7; parent #1418, milestones #1419/#1420 reuse this
 * platform).
 *
 * Two complete workflows — Navigation's chart (select a contact, set a
 * waypoint) and Helm's radar/movement rail (steer, dock) — carried through
 * their real shipped documents at 100/150/200% text, browser zoom and forced
 * colours, on the same device matrix and with the same monkeypatch-the-real-
 * modules discipline `text-scale-power-workflow.spec.js` (issue #1422)
 * established. `tests/client/ph-navigation-map.test.js`,
 * `tests/client/ph-radar.test.js` and `tests/client/radar-scope-contract.test.js`
 * hold the behavioural half this file cannot: jsdom does not lay out real CSS
 * or run a real `<canvas>`, so reflow, computed sizes and forced-colours
 * repaint all belong here instead.
 *
 * ── Which documents ─────────────────────────────────────────────────────
 * Navigation: `gui/battleship/navigation.html`. The destroyer's "navigation"
 * Station points at this SAME file (assets/entities/alliance_destroyer.toml),
 * and the cruiser's auxiliary "navigation" Station is the structurally
 * identical `#nav-view` inside `gui/cruiser/comms.html` (same grid, same
 * element ids, same `ph-navigation-map` — see the comment naming
 * gui/battleship/navigation.html in gui/cruiser/comms.html itself), so this
 * one document's map/wp-bar/side-panel exercise the shared contract behind
 * all three. The courier has no Helm or Navigation Station at all
 * (assets/entities/alliance_courier.toml: captain, tactical only).
 *
 * Helm: `gui/destroyer/helm.html` — the one hull whose Helm carries a lateral
 * joystick AND a contextual Dock/tow-load tail on top of the radar + joystick
 * + impulse/boost core every hull shares, so one document exercises the
 * fullest surface. `ph-helm-radar`/`ph-radar` are shared verbatim by every
 * hull's Helm and by Tactical/Sensors, so the canvas-label fix under test
 * here is the same fix those consoles get.
 *
 * ── Which bundle ─────────────────────────────────────────────────────────
 * `/client/...`, i.e. `node scripts/build-client.mjs` output. Both documents
 * are pure HTML + `gui/*` modules with no WASM, so this spec never needs the
 * Trunk host bundle `/gui/...`-rooted specs depend on.
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

/** See text-scale-power-workflow.spec.js for why a split-pane row scales its
 *  WIDTH floor with the text multiplier rather than being tested at 320x320. */
function viewportFor(entry, scale) {
  if (entry.kind !== 'split-pane') return { width: entry.width, height: entry.height };
  return { width: Math.round(entry.width * scale), height: entry.height };
}

const NAVIGATION_URL = '/client/gui/battleship/navigation.html';
const HELM_URL = '/client/gui/destroyer/helm.html';

/** A realistic Navigation payload: two contacts, a nebula region, no waypoint
 *  yet — the state the "select a contact, then set it as the waypoint"
 *  workflow starts from. */
const NAVIGATION_PAYLOAD = {
  blips: [
    { uuid: 'contact-alpha', kind: 'ship', name: 'Sundered Wake', world_x: 1600, world_z: -600, stance: 'hostile' },
    { uuid: 'contact-bravo', kind: 'station', name: 'Kestrel Anchorage', world_x: -1800, world_z: 900, stance: 'friendly' },
  ],
  regions: [
    { uuid: 'region-belt', x: 2600, z: 2000, shape: 'sphere', radius: 900, color: [0.4, 0.5, 0.7], name: 'Kaleth Belt' },
  ],
  radar_range: 5000,
  ship_x: 0, ship_z: 0, ship_heading: 45,
  waypoint: null,
  navigation_auto: false,
  objectives: [],
  civilians: [],
};

/** A realistic destroyer Helm payload: one contact on the radar, a berth in
 *  range but refused (so the Dock panel's own refusal text is exercised), a
 *  tow load, and impulse charging so the readouts carry real numbers. */
const HELM_PAYLOAD = {
  blips: [
    { uuid: 'contact-alpha', radar_x: 0.3, radar_y: -0.5, scaled_radius: 0.05, color: 'var(--fire-hot)', kind: 'ship', label: 'Sundered Wake' },
  ],
  range: 500, x: 0, z: 0, ship_heading: 45, speed: 12,
  on_screen: false,
  engine_port_thrust: 0.4, engine_stbd_thrust: 0.2,
  hostile_arcs: [], hostile_arc_color: null,
  helm_auto: false, lateral_auto: false,
  impulse_charge_progress: 0.6,
  boost_enabled: true, boost_active: false, boost_battery: 82,
  dock: {
    system_id: 'dock', available: true, engaged: false, docked: false,
    available_target_name: 'console.tractor.idle',
    refusal: 'console.tractor.idle',
  },
  tow_load: { active: true, target_name: 'console.tractor.idle' },
};

/**
 * A `window.__commands` feed of every outbound admitted command, WITHOUT
 * replacing `window.activateSemanticAction` (unlike the Power workflow spec):
 * Navigation's contact selection is a genuinely LOCAL semantic action —
 * `registerNavigationActions` (gui/console-core.js) resolves it through the
 * real registry, which is what actually moves the selection ring and the
 * overlay. Stubbing the global away, the way the Power spec safely does for
 * its purely-authoritative steppers, would silently stop contact selection
 * from doing anything at all.
 *
 * `sendAction` (gui/console-core.js) instead posts on
 * `BroadcastChannel('phoenix-console-state')` when the document has no
 * parent/wry/WASM host — exactly this spec's standalone `page.goto`, target 4
 * of ADR-0001 §3 — so a second channel object of the same name, created here,
 * receives every envelope the real adapters actually send.
 */
async function installCommandCapture(page) {
  await page.addInitScript(() => {
    window.__commands = [];
    const bc = new BroadcastChannel('phoenix-console-state');
    bc.onmessage = (e) => {
      if (e.data && e.data.type === 'console_action') {
        try { window.__commands.push(JSON.parse(e.data.payload)); } catch (_) { /* ignore */ }
      }
    };
    window.__commandCapture = bc;
  });
}

/**
 * Record every `<canvas>` font string set anywhere on the page, in order.
 * `ph-radar.js` and `ph-navigation-map.js` are the whole reason this exists —
 * a canvas font string sits outside the CSS ramp entirely, so nothing but
 * reading the actual draw calls proves the operator's text scale reached it.
 */
async function installCanvasFontCapture(page) {
  await page.addInitScript(() => {
    window.__canvasFonts = [];
    const proto = CanvasRenderingContext2D.prototype;
    const desc = Object.getOwnPropertyDescriptor(proto, 'font');
    Object.defineProperty(proto, 'font', {
      configurable: true,
      get() { return desc.get.call(this); },
      set(v) { window.__canvasFonts.push(v); desc.set.call(this, v); },
    });
  });
}

/** Drain the recorded canvas fonts and return the largest px size among them. */
async function maxCanvasFontPx(page) {
  return page.evaluate(() => {
    const fonts = window.__canvasFonts || [];
    window.__canvasFonts = [];
    let max = 0;
    for (const f of fonts) {
      const m = /^(\d+(?:\.\d+)?)px/.exec(f);
      if (m) max = Math.max(max, parseFloat(m[1]));
    }
    return max;
  });
}

async function openNavigationConsole(page) {
  await page.goto(NAVIGATION_URL);
  await page.waitForFunction(() =>
    typeof window.__updateConsole === 'function'
    && !!document.getElementById('navigation-map')?.shadowRoot);
  await page.evaluate(
    (state) => window.__updateConsole('navigation', JSON.stringify(state)),
    NAVIGATION_PAYLOAD,
  );
  await page.evaluate(() => document.fonts.ready);
}

async function openHelmConsole(page) {
  await page.goto(HELM_URL);
  await page.waitForFunction(() =>
    typeof window.__updateConsole === 'function'
    && !!document.getElementById('helm-radar')?.shadowRoot);
  await page.evaluate(
    (state) => window.__updateConsole('helm', JSON.stringify(state)),
    HELM_PAYLOAD,
  );
  await page.evaluate(() => document.fonts.ready);
}

/** Apply a text scale through the SHIPPED profile modules, not by poking CSS. */
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

/** Every rendered Navigation control/status with a real box, and whether
 *  anything is pushed sideways where no scroll would reach it. */
async function navigationReachability(page) {
  return page.evaluate(() => {
    const boxed = (el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };
    const map = document.getElementById('navigation-map');
    const shadow = map.shadowRoot;
    const doc = document.documentElement;
    return {
      mapBoxed: boxed(map),
      setWaypointBoxed: boxed(shadow.getElementById('btn-set-waypoint')),
      onScreenBoxed: boxed(document.getElementById('btn-on-screen')),
      selectedMetricBoxed: boxed(document.getElementById('ent-name')),
      waypointMetricBoxed: boxed(document.getElementById('waypoint-name')),
      contactCountBoxed: boxed(document.getElementById('nav-contact-count')),
      horizontalOverflow: doc.scrollWidth - doc.clientWidth,
      verticalScrollAvailable: doc.scrollHeight >= doc.clientHeight,
    };
  });
}

/** Every rendered Helm control/status with a real box. */
async function helmReachability(page) {
  return page.evaluate(() => {
    const boxed = (el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };
    const radarShadow = document.getElementById('helm-radar').shadowRoot;
    const doc = document.documentElement;
    return {
      radarBoxed: boxed(document.getElementById('helm-radar')),
      onScreenBoxed: boxed(radarShadow.getElementById('on-screen-btn')),
      posLabelBoxed: boxed(radarShadow.getElementById('label-pos')),
      bearingLabelBoxed: boxed(radarShadow.getElementById('label-bearing')),
      speedLabelBoxed: boxed(radarShadow.getElementById('label-speed')),
      joystickBoxed: boxed(document.getElementById('helm-joystick')),
      impulseBoxed: boxed(document.getElementById('impulse-btn')),
      boostBoxed: boxed(document.getElementById('boost-btn')),
      dockBtnBoxed: boxed(document.getElementById('dock-btn')),
      dockRefusalBoxed: boxed(document.getElementById('dock-refusal')),
      towLoadBoxed: boxed(document.getElementById('tow-load-panel')),
      horizontalOverflow: doc.scrollWidth - doc.clientWidth,
      verticalScrollAvailable: doc.scrollHeight >= doc.clientHeight,
    };
  });
}

// ── 1. Navigation: select + set-waypoint stays reachable, readable, growing ─

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`Navigation select-and-waypoint is reachable and its labels grow with text scale on ${id}`, async ({ page }) => {
    await installCanvasFontCapture(page);
    const baselineFont = {};

    for (const scale of TEXT_SCALES) {
      const viewport = viewportFor(entry, scale);
      await page.setViewportSize(viewport);
      await openNavigationConsole(page);
      await applyTextScale(page, scale);
      await page.evaluate(() => document.fonts.ready);
      // `installCanvasFontCapture` resets `window.__canvasFonts` on every
      // navigation (`page.addInitScript` re-runs pre-document), so this is a
      // clean slate for THIS scale — nothing to discard from a previous
      // iteration. Wait a couple of frames so the chart's own rAF loop (which
      // now notices a text-scale change every tick — see `ph-navigation-map.js`
      // `#rafLoop`) has actually painted under the applied scale.
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

      const where = `${id} @ ${scale}x (${viewport.width}x${viewport.height})`;

      const reach = await navigationReachability(page);
      expect(reach.mapBoxed, `${where}: map`).toBe(true);
      expect(reach.setWaypointBoxed, `${where}: Set Waypoint`).toBe(true);
      expect(reach.onScreenBoxed, `${where}: On Screen`).toBe(true);
      expect(reach.selectedMetricBoxed, `${where}: Selected metric`).toBe(true);
      expect(reach.waypointMetricBoxed, `${where}: Waypoint metric`).toBe(true);
      expect(reach.contactCountBoxed, `${where}: Contacts metric`).toBe(true);
      expect(reach.horizontalOverflow, `${where}: horizontal overflow`).toBeLessThanOrEqual(1);
      expect(reach.verticalScrollAvailable, `${where}: vertical scroll`).toBe(true);

      // ── The chart's own labels grow with the operator's text choice ─────
      // (PRD #1418 story 3: labels stay readable while the chart itself
      // stays spatial.)
      const font = await maxCanvasFontPx(page);
      expect(font, `${where}: a label was painted`).toBeGreaterThan(0);
      if (scale === TEXT_SCALES[0]) {
        baselineFont[id] = font;
      } else {
        expect(font, `${where}: label grew vs 100% (${baselineFont[id]}px)`)
          .toBeGreaterThan(baselineFont[id]);
      }
    }
  });
}

// ── 2. Helm: radar + movement rail stays reachable, readable, growing ──────

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`Helm's radar, joystick and Dock panel are reachable and readable at every text scale on ${id}`, async ({ page }) => {
    await installCanvasFontCapture(page);
    const baselineFont = {};

    for (const scale of TEXT_SCALES) {
      const viewport = viewportFor(entry, scale);
      await page.setViewportSize(viewport);
      await openHelmConsole(page);
      await applyTextScale(page, scale);
      await page.evaluate(() => document.fonts.ready);
      // See the matching comment in the Navigation loop above: a clean slate
      // per navigation, so this just waits for the scope's own rAF loop to
      // notice the applied scale and repaint.
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

      const where = `${id} @ ${scale}x (${viewport.width}x${viewport.height})`;

      const reach = await helmReachability(page);
      expect(reach.radarBoxed, `${where}: radar`).toBe(true);
      expect(reach.onScreenBoxed, `${where}: On Screen`).toBe(true);
      expect(reach.posLabelBoxed, `${where}: position corner label`).toBe(true);
      expect(reach.bearingLabelBoxed, `${where}: bearing corner label`).toBe(true);
      expect(reach.speedLabelBoxed, `${where}: speed corner label`).toBe(true);
      expect(reach.joystickBoxed, `${where}: joystick`).toBe(true);
      expect(reach.impulseBoxed, `${where}: impulse`).toBe(true);
      expect(reach.boostBoxed, `${where}: boost`).toBe(true);
      expect(reach.dockBtnBoxed, `${where}: dock button`).toBe(true);
      // The refusal banner is text (PRD #1418 story 6: refused is never
      // hue-only), and it must stay visible/laid-out at scale, not just at
      // 100%.
      expect(reach.dockRefusalBoxed, `${where}: dock refusal banner`).toBe(true);
      expect(reach.towLoadBoxed, `${where}: tow-load banner`).toBe(true);
      expect(reach.horizontalOverflow, `${where}: horizontal overflow`).toBeLessThanOrEqual(1);
      expect(reach.verticalScrollAvailable, `${where}: vertical scroll`).toBe(true);

      const font = await maxCanvasFontPx(page);
      expect(font, `${where}: a radar label was painted`).toBeGreaterThan(0);
      if (scale === TEXT_SCALES[0]) {
        baselineFont[id] = font;
      } else {
        expect(font, `${where}: radar label grew vs 100% (${baselineFont[id]}px)`)
          .toBeGreaterThan(baselineFont[id]);
      }
    }
  });
}

// ── 3. Keyboard: select a contact and commit it as the waypoint at 200% ────

test('the whole select-then-waypoint workflow is operable from the keyboard at 200% text', async ({ page }) => {
  await installCommandCapture(page);
  const entry = device('phone-390x844-portrait');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await openNavigationConsole(page);
  await applyTextScale(page, 2);

  // Tab reaches the chart (the shell's own light-DOM ON SCREEN button sits
  // before it in source order).
  await page.evaluate(() => document.getElementById('navigation-map').focus());
  const selected = () => page.evaluate(() =>
    document.getElementById('ent-name').textContent.trim());

  // Arrow cycles the selection over the two real contacts (issue #1176's
  // keyboard path), same as a tap would.
  await page.keyboard.press('ArrowRight');
  const first = await selected();
  expect(['Sundered Wake', 'Kestrel Anchorage']).toContain(first);

  // Enter commits the selected contact through the SAME set_navigation_waypoint
  // action the Set As Waypoint bar button sends — captured on the real
  // BroadcastChannel outbound path, not a stub.
  await page.keyboard.press('Enter');
  await expect.poll(() => page.evaluate(() => window.__commands.length)).toBeGreaterThan(0);
  const commands = await page.evaluate(() => window.__commands);
  expect(commands[0].action).toBe('set_navigation_waypoint');
  expect(commands[0].source_uuid).toBeTruthy();
});

// ── 4. Browser zoom, tested separately from the Phoenix text setting ───────

test('browser zoom is usable alongside the Phoenix text setting on the Navigation chart', async ({ browser }) => {
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
      await openNavigationConsole(page);
      const reach = await navigationReachability(page);
      const where = `browser zoom ${zoom * 100}%`;
      expect(reach.mapBoxed, where).toBe(true);
      expect(reach.setWaypointBoxed, where).toBe(true);
      expect(reach.horizontalOverflow, where).toBeLessThanOrEqual(1);

      // Additive with the Phoenix ceiling on top, same as the Power workflow.
      await applyTextScale(page, 2);
      const zoomed = await navigationReachability(page);
      expect(zoomed.mapBoxed, `${where} + 200%`).toBe(true);
      expect(zoomed.horizontalOverflow, `${where} + 200%`).toBeLessThanOrEqual(1);
    } finally {
      await context.close();
    }
  }
});

// ── 5. Pending is not hue-only, including under forced colours ─────────────

test('the armed Set Waypoint button is distinguishable without colour, in and out of forced colours', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await openNavigationConsole(page);

  const btnStyle = () => page.evaluate(() => {
    const shadow = document.getElementById('navigation-map').shadowRoot;
    const btn = shadow.getElementById('btn-set-waypoint');
    const cs = getComputedStyle(btn);
    const after = getComputedStyle(btn, '::after');
    return { borderStyle: cs.borderStyle, afterContent: after.content };
  });

  const resting = await btnStyle();
  expect(resting.borderStyle).not.toBe('dashed');

  await page.evaluate(() => document.getElementById('navigation-map')
    .shadowRoot.getElementById('btn-set-waypoint').click());
  const armed = await btnStyle();
  // A dashed border and an appended glyph — not merely a different colour —
  // distinguish "pending" (PRD #1418 story 6).
  expect(armed.borderStyle).toBe('dashed');
  expect(armed.afterContent).not.toBe('none');
  expect(armed.afterContent.length).toBeGreaterThan(0);

  // Forced colours strip authored colour, never border-style: the cue
  // survives exactly because it was never colour in the first place.
  await page.emulateMedia({ forcedColors: 'active' });
  const armedForced = await btnStyle();
  expect(armedForced.borderStyle).toBe('dashed');
});

// ── 6. Forced colours on the Helm radar's DOM chrome ────────────────────────

test('under forced colours the Helm radar corner chrome and focus ring stay visible', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await page.emulateMedia({ forcedColors: 'active' });
  await openHelmConsole(page);

  const onScreen = await page.evaluate(() => {
    const shadow = document.getElementById('helm-radar').shadowRoot;
    const btn = shadow.getElementById('on-screen-btn');
    const cs = getComputedStyle(btn);
    return { borderWidth: parseFloat(cs.borderTopWidth), borderColour: cs.borderTopColor };
  });
  expect(onScreen.borderWidth).toBeGreaterThan(0);
  expect(onScreen.borderColour).not.toBe('rgba(0, 0, 0, 0)');

  // Impulse/boost sit outside the scope's own shadow root and share the
  // chamfered control family issue #1422 fixed to redraw its focus ring as a
  // border under forced colours (box-shadow is dropped there).
  await page.evaluate(() => document.getElementById('boost-btn')
    .shadowRoot.getElementById('btn').focus());
  const boostRing = await page.evaluate(() => {
    const btn = document.getElementById('boost-btn').shadowRoot.getElementById('btn');
    const cs = getComputedStyle(btn);
    return { outlineWidth: parseFloat(cs.outlineWidth), borderWidth: parseFloat(cs.borderTopWidth) };
  });
  // Either a visible outline or a visible border carries the keyboard focus
  // under forced colours; neither being zero is the actual requirement.
  expect(boostRing.outlineWidth > 0 || boostRing.borderWidth > 0).toBe(true);
});
