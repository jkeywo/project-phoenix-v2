import { test, expect } from '@playwright/test';
import {
  DEVICE_MATRIX,
  TEXT_SCALES,
  BROWSER_ZOOMS,
} from '../fixtures/device-matrix.mjs';

/**
 * tests/smoke/text-scale-power-workflow.spec.js — issue #1422 (PRD #1418
 * stories 1, 2, 4, 9, 10, 16, 17).
 *
 * ONE complete console workflow — Power allocation on the battleship's Power
 * seat — carried through the shell, its console iframe and the allocation
 * controls at 100%, 150% and 200% text, and separately under browser zoom and
 * forced colours. This is the half `tests/client/accessibility-200-percent.js`
 * cannot do: only a real engine lays out `max(px, rem)` type ramps, resolves
 * `clamp()` root sizes against a real viewport, honours `clip-path` when
 * deciding whether a focus ring survives, and repaints a page in a forced
 * palette.
 *
 * Viewports, scales and zoom steps come from `tests/fixtures/device-matrix.mjs`
 * (issue #1421) rather than from literals here, so this spec and the human
 * acceptance kit in `docs/acceptance/1421-device-matrix.md` exercise the same
 * matrix.
 *
 * ── Which bundle ──────────────────────────────────────────────────────────
 * Everything below is served from `/client/...`, i.e. `node
 * scripts/build-client.mjs` output. The Power console is pure HTML + `gui/*`
 * modules with no WASM, so this spec never needs the Trunk host bundle that
 * `/gui/...`-rooted specs depend on.
 */

// The devices this workflow is carried on. Deliberately a NAMED subset of
// DEVICE_MATRIX rather than all of it: PRD #1418's own starting regression
// cases (390x844 phone, 1280x720 landscape) plus the landscape phone and the
// native split-pane floor, which is the geometry the reflow contract in
// src/native_host/setup_accessibility.rs is actually about. Running all
// fourteen rows x three scales in a workers:1 suite would buy repetition, not
// coverage.
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

/**
 * The viewport to exercise `entry` at, for text scale `scale`.
 *
 * A split-pane row is not a device: it is a per-pane FLOOR quoted at text
 * scale 1.0, and `MIN_CONSOLE_LOGICAL_WIDTH_PX` scales linearly with the text
 * multiplier (the height floor does not — consoles scroll). So the supported
 * pane at 200% is 640 logical px wide, not 320, and testing 320x320 at 200%
 * would be asserting a case the Rust contract explicitly says is TOO SMALL.
 * Every other row is a real viewport and is used as authored.
 */
function viewportFor(entry, scale) {
  if (entry.kind !== 'split-pane') return { width: entry.width, height: entry.height };
  return { width: Math.round(entry.width * scale), height: entry.height };
}

/** A realistic allocation: one group held below its commanded level by the
 *  battery floor, one switched off, one ordinary. */
const POWER_PAYLOAD = {
  groups: [
    { id: 'helm', label: 'PROPULSION', level: 2, commanded_level: 2, min_level: 1, max_level: 4 },
    { id: 'weapons', label: 'WEAPONS', level: 1, commanded_level: 3, min_level: 0, max_level: 4 },
    { id: 'shields', label: 'SHIELDS', level: 0, commanded_level: 0, min_level: 0, max_level: 4 },
  ],
  power_auto: false,
  battery_online: true,
  charging: true,
  battery_charge: 72,
  battery_max: 100,
  total: 3,
  total_max: 12,
  draining: false,
};

const POWER_URL = '/client/gui/battleship/power.html';

/** Open the Power console, capture the commands it would send, and push a
 *  realistic allocation into it. Returns nothing; the page is ready to probe. */
async function openPowerConsole(page) {
  await page.goto(POWER_URL);
  await page.waitForFunction(() =>
    typeof window.__updateConsole === 'function'
    && !!document.querySelector('ph-power-controls')?.shadowRoot);
  // The console's outbound path, captured rather than stubbed away: this is the
  // real `activateSemanticAction` seam every visible Engineering control uses.
  await page.evaluate(() => {
    window.__commands = [];
    window.activateSemanticAction = (actionId, payload) => {
      window.__commands.push({ actionId, ...payload });
      return true;
    };
  });
  await page.evaluate(
    (state) => window.__updateConsole('power', JSON.stringify(state)),
    POWER_PAYLOAD,
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

/** Every rendered string this workflow depends on, with its computed size. */
async function textSizes(page) {
  return page.evaluate(() => {
    const px = (el) => (el ? parseFloat(getComputedStyle(el).fontSize) : null);
    const shadow = document.querySelector('ph-power-controls').shadowRoot;
    return {
      root: px(document.documentElement),
      batteryLabel: px(document.getElementById('bat-val')),
      batteryCaption: px(document.querySelector('.readout [data-i18n]')),
      groupLabel: px(shadow.querySelector('.group-label')),
      levelText: px(shadow.querySelector('.level-text')),
      stepper: px(shadow.querySelector('.mini-btn')),
      coldTag: px(shadow.querySelector('.cold-tag')),
    };
  });
}

/** Is every control and status of the workflow actually on the page and
 *  reachable — laid out with a real box, and no sideways scroll to reach it? */
async function reachability(page) {
  return page.evaluate(() => {
    const shadow = document.querySelector('ph-power-controls').shadowRoot;
    const boxed = (el) => {
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };
    const steppers = Array.from(shadow.querySelectorAll('.mini-btn'));
    const pips = Array.from(shadow.querySelectorAll('.pip'));
    const doc = document.documentElement;
    return {
      groups: shadow.querySelectorAll('.group').length,
      steppers: steppers.length,
      pips: pips.length,
      everyStepperBoxed: steppers.every(boxed),
      everyPipBoxed: pips.every(boxed),
      // One tab stop for the panel, arrows between the steppers (roving
      // tabindex) — enlargement must not turn one stop into six or none.
      tabStops: steppers.filter((b) => b.getAttribute('tabindex') === '0').length,
      // Status, in words: the running level, the held gap, the cold group and
      // the battery percentage.
      levelTexts: Array.from(shadow.querySelectorAll('.level-text'))
        .map((el) => el.textContent.trim()).filter(Boolean).length,
      // The switched-off group's COLD tag: shown, and laid out. Its two
      // neighbours keep theirs hidden, which is why this reads the shields row
      // rather than any `.cold-tag`.
      coldVisible: boxed(shadow.querySelector('[data-group-id="shields"] .cold-tag'))
        && !shadow.querySelector('[data-group-id="shields"] .cold-tag').hidden
        && shadow.querySelectorAll('.cold-tag:not([hidden])').length === 1,
      battery: (document.getElementById('bat-val') || {}).textContent,
      // Panels may wrap, stack and scroll; what they may not do is put content
      // sideways off the page where no scroll reaches it.
      horizontalOverflow: doc.scrollWidth - doc.clientWidth,
      verticalScrollAvailable: doc.scrollHeight >= doc.clientHeight,
    };
  });
}

// ── 1. The workflow at 100% / 150% / 200% on each carried surface ──────────

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`Power allocation is reachable and readable at every text scale on ${id}`, async ({ page }) => {
    const baseline = {};

    for (const scale of TEXT_SCALES) {
      const viewport = viewportFor(entry, scale);
      await page.setViewportSize(viewport);
      await openPowerConsole(page);
      await applyTextScale(page, scale);
      await page.evaluate(() => document.fonts.ready);

      const where = `${id} @ ${scale}x (${viewport.width}x${viewport.height})`;

      // ── Controls and status are all there and all laid out ──────────────
      const reach = await reachability(page);
      expect(reach.groups, `${where}: groups`).toBe(3);
      expect(reach.steppers, `${where}: steppers`).toBe(6);
      expect(reach.pips, `${where}: pips`).toBe(12);
      expect(reach.everyStepperBoxed, `${where}: every stepper laid out`).toBe(true);
      expect(reach.everyPipBoxed, `${where}: every rung laid out`).toBe(true);
      expect(reach.tabStops, `${where}: one tab stop into the panel`).toBe(1);
      expect(reach.levelTexts, `${where}: a level readout per group`).toBe(3);
      expect(reach.coldVisible, `${where}: the cold group still says so`).toBe(true);
      expect(reach.battery, `${where}: battery status`).toBe('72%');

      // Reflow, do not clip: vertical growth is absorbed by the console's own
      // scroll, and nothing is pushed sideways out of reach.
      expect(reach.horizontalOverflow, `${where}: horizontal overflow`).toBeLessThanOrEqual(1);
      expect(reach.verticalScrollAvailable, `${where}: vertical scroll`).toBe(true);

      // ── Nothing shrinks ─────────────────────────────────────────────────
      const sizes = await textSizes(page);
      for (const [key, value] of Object.entries(sizes)) {
        expect(value, `${where}: ${key} rendered`).toBeGreaterThan(0);
      }
      if (scale === 1) {
        Object.assign(baseline, sizes);
      } else {
        for (const [key, value] of Object.entries(sizes)) {
          // PRD #1418: "Do not silently shrink text to preserve the original
          // composition." Every string is at least as large as it was at 100%.
          expect(value, `${where}: ${key} vs 100% (${baseline[key]}px)`)
            .toBeGreaterThanOrEqual(baseline[key]);
        }
        // And the multiplier is genuinely applied, not merely not-shrunk.
        expect(sizes.root, `${where}: root font-size grew`)
          .toBeGreaterThan(baseline.root);
      }
    }
  });
}

// ── 2. The keyboard path through the workflow at the ceiling ───────────────

test('the whole allocation workflow is operable from the keyboard at 200% text', async ({ page }) => {
  const entry = device('phone-390x844-portrait');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await openPowerConsole(page);
  await applyTextScale(page, 2);

  // One Tab reaches the panel; arrows rove between the steppers.
  await page.evaluate(() => document.querySelector('ph-power-controls')
    .shadowRoot.querySelector('.mini-btn').focus());
  const focused = () => page.evaluate(() => {
    const el = document.querySelector('ph-power-controls').shadowRoot.activeElement;
    return el && {
      group: el.closest('[data-group-id]').dataset.groupId,
      action: el.dataset.action,
      // The ring is drawn on the recessed body, so a visible focus indicator
      // means that body actually carries one.
      ringed: getComputedStyle(el.querySelector('.mini-bg')).boxShadow !== 'none',
    };
  });
  expect(await focused()).toMatchObject({ group: 'helm', action: 'decr' });

  await page.keyboard.press('ArrowDown');
  expect(await focused()).toMatchObject({ group: 'helm', action: 'incr' });
  await page.keyboard.press('ArrowDown');
  expect(await focused()).toMatchObject({ group: 'weapons', action: 'decr' });

  // Activate it: the authoritative order goes out unchanged by the text size.
  await page.keyboard.press('Enter');
  expect(await page.evaluate(() => window.__commands)).toEqual([
    { actionId: 'power.decrease-allocation', source: 'control', detail: { target: 'weapons', level: 2 } },
  ]);
});

// ── 3. Browser zoom, tested separately from Phoenix text scale ─────────────

test('browser zoom is usable alongside the Phoenix text setting', async ({ browser }) => {
  // PRD #1418: "Verify browser zoom separately; do not assume a universally
  // available browser query for the Windows text-size percentage." Browser
  // zoom is emulated the way the browser itself does it — the CSS viewport
  // shrinks by the zoom factor while the device pixel ratio grows — rather than
  // by writing a CSS `zoom`, which is a page-authored effect and not the same
  // thing at all.
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
      await openPowerConsole(page);
      // Phoenix's own setting stays at its default here: the point is that the
      // browser's zoom works on its own, without Phoenix compensating for it.
      const reach = await reachability(page);
      const where = `browser zoom ${zoom * 100}%`;
      expect(reach.groups, where).toBe(3);
      expect(reach.steppers, where).toBe(6);
      expect(reach.everyStepperBoxed, where).toBe(true);
      expect(reach.coldVisible, where).toBe(true);
      expect(reach.battery, where).toBe('72%');
      expect(reach.horizontalOverflow, where).toBeLessThanOrEqual(1);

      // And the two are additive rather than exclusive: the Phoenix ceiling on
      // top of browser zoom still enlarges, and still reaches everything.
      const before = (await textSizes(page)).root;
      await applyTextScale(page, 2);
      const after = await textSizes(page);
      expect(after.root, `${where}: 200% text on top of zoom`).toBeGreaterThan(before);
      const zoomed = await reachability(page);
      expect(zoomed.steppers, `${where} + 200%`).toBe(6);
      expect(zoomed.everyStepperBoxed, `${where} + 200%`).toBe(true);
      expect(zoomed.horizontalOverflow, `${where} + 200%`).toBeLessThanOrEqual(1);
    } finally {
      await context.close();
    }
  }
});

// ── 4. Contrast and forced colours ─────────────────────────────────────────

test('the Phoenix contrast choice changes the palette; forced colours overrule it', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await openPowerConsole(page);

  const setContrast = (value) => page.evaluate(async (v) => {
    const { applyEffectsToRoot, resolveEffects } = await import(
      '/client/gui/accessibility-profile.js');
    applyEffectsToRoot(document.documentElement, resolveEffects({ presentation: { contrast: v } }));
  }, value);

  const ringColour = () => page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue('--focus-ring').trim());
  const panelEdge = () => page.evaluate(() =>
    getComputedStyle(document.querySelector('ph-power-controls').shadowRoot
      .querySelector('.group')).borderTopColor);

  // ── Phoenix's own contrast setting, with no forced palette in play ──────
  await setContrast('off');
  const standardRing = await ringColour();
  const standardEdge = await panelEdge();
  await setContrast('on');
  expect(await ringColour(), 'the high-contrast ring differs').not.toBe(standardRing);
  expect(await panelEdge(), 'the high-contrast edge differs').not.toBe(standardEdge);

  // ── Forced colours: the browser's palette, not Phoenix's ────────────────
  // PRD #1418: "A Phoenix contrast selection is not permission to defeat
  // browser-enforced colours." So under forced colours the ring must be the
  // SAME whichever Phoenix contrast the operator picked — the setting stops
  // having a say.
  await page.emulateMedia({ forcedColors: 'active' });
  await setContrast('on');
  const forcedWithContrast = await ringColour();
  await setContrast('off');
  const forcedWithout = await ringColour();
  expect(forcedWithout, 'forced colours are not overridden by the Phoenix setting')
    .toBe(forcedWithContrast);
  expect(forcedWithout, 'the forced ring is a real colour').not.toBe('');
  expect(forcedWithout, 'the forced ring is not the authored one').not.toBe(standardRing);

  // Borders survive: a group's edge is drawn, not collapsed away with the
  // background ramp.
  const forcedEdge = await panelEdge();
  expect(forcedEdge).not.toBe('rgba(0, 0, 0, 0)');
  expect(forcedEdge).not.toBe('transparent');
});

test('under forced colours the focus ring and the lit rungs are still visible', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await page.emulateMedia({ forcedColors: 'active' });
  await openPowerConsole(page);

  // A forced-colours mode DROPS box-shadow, and the chamfered control family
  // draws its focus ring as an inset shadow after setting `outline: none` —
  // which is why gui/components/ph-console-styles.js redraws it as a border in
  // that mode. Without that, this stepper would show no keyboard focus at all.
  await page.evaluate(() => document.querySelector('ph-power-controls')
    .shadowRoot.querySelector('.mini-btn').focus());
  const ring = await page.evaluate(() => {
    const btn = document.querySelector('ph-power-controls').shadowRoot
      .querySelector('.mini-btn');
    const bg = getComputedStyle(btn.querySelector('.mini-bg'));
    return { width: parseFloat(bg.borderTopWidth), colour: bg.borderTopColor };
  });
  expect(ring.width, 'the focus border is drawn').toBeGreaterThan(0);
  expect(ring.colour).not.toBe('rgba(0, 0, 0, 0)');

  // The allocation readout must not collapse to twelve identical circles: a lit
  // rung and an unlit one still differ.
  const pips = await page.evaluate(() => {
    const shadow = document.querySelector('ph-power-controls').shadowRoot;
    const of = (sel) => {
      const s = getComputedStyle(shadow.querySelector(sel));
      return { background: s.backgroundColor, borderStyle: s.borderTopStyle };
    };
    return {
      active: of('[data-group-id="helm"] .pip[data-level="1"]'),
      inactive: of('[data-group-id="helm"] .pip[data-level="4"]'),
      held: of('[data-group-id="weapons"] .pip[data-level="3"]'),
    };
  });
  expect(pips.active.background, 'a lit rung differs from an unlit one')
    .not.toBe(pips.inactive.background);
  // The held rung — commanded but refused by the battery floor — is told apart
  // by its border STYLE, which a forced palette keeps when it flattens colour.
  expect(pips.held.borderStyle).toBe('dashed');
  expect(pips.inactive.borderStyle).not.toBe('dashed');
});

// ── 5. Shell -> console iframe: one choice, both sides of the boundary ─────

test('the text choice reaches the shell and the console inside its iframe', async ({ page }) => {
  const entry = device('phone-390x844-portrait');
  await page.setViewportSize({ width: entry.width, height: entry.height });

  // The real shell document with its scripts stripped, so the disconnected
  // lobby does not run — the seam under test is the mount + the profile push,
  // both of which are the shipped modules. (Same isolation
  // console-redesign-accessibility.spec.js uses.)
  await page.route('**/client/', async (route) => {
    const response = await route.fetch();
    const html = (await response.text()).replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, '');
    await route.fulfill({ response, body: html });
  });
  await page.goto('/client/');

  await page.evaluate(async () => {
    const { mountConsoles, applyConsoleVisibility } = await import('/client/gui/console-mount.js');
    const ship = { stations: [{ id: 'power', name: 'Power', console: 'gui/battleship/power.html' }] };
    let container = document.getElementById('console-container');
    if (!container) {
      container = document.createElement('div');
      container.id = 'console-container';
      document.body.appendChild(container);
    }
    mountConsoles(document, container, ship);
    applyConsoleVisibility(document, 'power', true, ['power']);
    // The mount points the iframe at a RELATIVE console path; this page is
    // /client/, so it resolves to the built console next to it.
    const frame = document.getElementById('power-iframe');
    frame.style.width = '100%';
    frame.style.height = '100%';
  });

  const frame = page.frameLocator('#power-iframe');
  await expect(frame.locator('ph-power-controls')).toBeAttached();
  await page.waitForFunction(() =>
    typeof document.getElementById('power-iframe').contentWindow.__updateConsole === 'function');
  await page.evaluate((state) => document.getElementById('power-iframe')
    .contentWindow.__updateConsole('power', JSON.stringify(state)), POWER_PAYLOAD);

  const sizes = async () => page.evaluate(() => ({
    shell: parseFloat(getComputedStyle(document.documentElement).fontSize),
    // Shell CHROME, not just the shell root: the Station Bar title and the
    // connection label are the "settings, errors and overlays" half of PRD
    // #1418 story 4, and they live on this page, outside every console iframe.
    shellTitle: parseFloat(getComputedStyle(
      document.getElementById('station-hero-title')).fontSize),
    shellStatus: parseFloat(getComputedStyle(
      document.getElementById('conn-label')).fontSize),
    iframe: parseFloat(getComputedStyle(
      document.getElementById('power-iframe').contentDocument.documentElement).fontSize),
    control: parseFloat(getComputedStyle(
      document.getElementById('power-iframe').contentDocument
        .querySelector('ph-power-controls').shadowRoot
        .querySelector('.group-label')).fontSize),
  }));

  const applied = [];
  for (const scale of TEXT_SCALES) {
    // The SHELL-side entry point: one call, and it must reach the shell root
    // and every mounted same-origin console iframe root.
    const effects = await page.evaluate(async (value) => {
      const mod = await import('/client/gui/accessibility-profile.js');
      return mod.applyAccessibilityProfile({ presentation: { textScale: value } });
    }, scale);
    expect(effects.textScale).toBeCloseTo(scale);
    applied.push(await sizes());
  }

  for (let i = 1; i < applied.length; i += 1) {
    const where = `${TEXT_SCALES[i]}x vs ${TEXT_SCALES[i - 1]}x`;
    expect(applied[i].shell, `${where}: shell`).toBeGreaterThan(applied[i - 1].shell);
    expect(applied[i].shellTitle, `${where}: Station Bar title`)
      .toBeGreaterThan(applied[i - 1].shellTitle);
    expect(applied[i].shellStatus, `${where}: connection label`)
      .toBeGreaterThan(applied[i - 1].shellStatus);
    expect(applied[i].iframe, `${where}: console iframe`).toBeGreaterThan(applied[i - 1].iframe);
    // And through the shadow boundary to the allocation control itself.
    expect(applied[i].control, `${where}: allocation control`)
      .toBeGreaterThanOrEqual(applied[i - 1].control);
  }
  expect(applied[applied.length - 1].control).toBeGreaterThan(applied[0].control);
});

// ── 6. The Station Bar's own labels, in the landscape rail ─────────────────

/**
 * The shell root scales now, and the landscape rail is the one place in the
 * shell whose width is a fixed pixel number (132px) holding text that grows.
 * The strip above it escapes because HERO_BAR_CODE_QUERY switches a phone-sized
 * bar to the hull's three-letter short codes; a landscape TABLET or desktop is
 * over that 500px height threshold and shows full Station names in a ~84px
 * label box. So this is the shell-side half of acceptance criterion 1: the
 * controls that switch Station stay readable at the ceiling, not just the
 * console's own.
 *
 * Real hull names, taken from assets/entities/alliance_battleship.toml, because
 * "Navigation" (the longest the battleship authors) is precisely the label that
 * outgrew its box: 107px of text in 84px of room at 200%.
 */
const BATTLESHIP_TABS = [
  { id: 'power', name: 'Power', short_code: 'PWR' },
  { id: 'repair', name: 'Repair', short_code: 'ENG' },
  { id: 'navigation', name: 'Navigation', short_code: 'NAV' },
  { id: 'sensors', name: 'Sensors', short_code: 'SCI' },
];

test('Station Bar tab labels stay whole in the landscape rail at every text scale', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });

  // Same isolation as the shell test above: the real shell document, its
  // scripts stripped so the disconnected lobby does not run, with the Station
  // Bar rendered by the shipped gui/hero-bar.js module.
  await page.route('**/client/', async (route) => {
    const response = await route.fetch();
    const html = (await response.text()).replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, '');
    await route.fulfill({ response, body: html });
  });
  await page.goto('/client/');

  const labelMode = await page.evaluate(async (stations) => {
    const { heroBarModel, renderHeroBarDom, heroBarLabelMode } = await import(
      '/client/gui/hero-bar.js');
    // Every seat human-seeking and hosted on the direct Station, so the rail
    // carries the visiting tabs too — one tab is not a rail.
    const model = heroBarModel({
      directStation: 'power',
      stations: stations.map((st) => ({ ...st, human_seeking: true })),
      stationHosts: Object.fromEntries(stations.map((st) => [st.id, { host: 'power' }])),
      activeStation: 'power',
    });
    const mode = heroBarLabelMode(window);
    renderHeroBarDom({
      tabsEl: document.getElementById('station-hero-tabs'),
      titleEl: document.getElementById('station-hero-title'),
      ratingEl: document.getElementById('station-hero-rating'),
      aiEl: document.getElementById('station-hero-ai'),
      model,
      translate: (id) => id,
      onActivate: () => {},
      labelMode: mode,
    });
    document.getElementById('station-hero').setAttribute('aria-hidden', 'false');
    document.getElementById('console-container').classList.add('station-hero-visible');
    return mode;
  }, BATTLESHIP_TABS);

  // The premise: above the phone threshold the rail shows NAMES. If this ever
  // flips to 'code' the measurements below stop meaning anything.
  expect(labelMode, 'a landscape tablet shows full Station names').toBe('name');
  await page.evaluate(() => document.fonts.ready);

  const readTabs = () => page.evaluate(() => {
    const tabsEl = document.getElementById('station-hero-tabs');
    return {
      railDirection: getComputedStyle(tabsEl).flexDirection,
      tabs: [...tabsEl.querySelectorAll('button[data-tab-id]')].map((button) => {
        const label = button.children[0];
        const box = label.getBoundingClientRect();
        return {
          id: button.dataset.tabId,
          text: label.textContent.trim(),
          // The label's own line box against the room it is given. A label that
          // still says `nowrap` overruns this; a wrapped one never can.
          scrollWidth: label.scrollWidth,
          width: box.width,
          // …and the tab actually grew to hold the wrapped lines rather than
          // hiding them under its own `overflow: hidden`.
          labelBottom: box.bottom,
          tabBottom: button.getBoundingClientRect().bottom,
          fontSize: parseFloat(getComputedStyle(label).fontSize),
        };
      }),
      // The rail scrolls DOWN when tabs grow; it must never need to scroll
      // sideways, which is the direction it has no scrollbar for.
      sidewaysOverflow: tabsEl.scrollWidth - tabsEl.clientWidth,
    };
  });

  let previous = null;
  for (const scale of TEXT_SCALES) {
    await applyTextScale(page, scale);
    await page.evaluate(() => document.fonts.ready);
    const read = await readTabs();
    const where = `rail @ ${scale}x`;

    expect(read.railDirection, `${where}: a column rail`).toBe('column');
    expect(read.tabs.length, `${where}: every seat has a tab`).toBe(BATTLESHIP_TABS.length);
    expect(read.tabs.map((t) => t.text), `${where}: full names, untruncated`)
      .toEqual(BATTLESHIP_TABS.map((st) => st.name));

    for (const tab of read.tabs) {
      // The finding this pins: at 200% "Navigation" measured 107px of text in
      // an 84px box with `overflow-x: hidden` above it and no ellipsis.
      expect(tab.scrollWidth, `${where}: ${tab.id} label fits its box`)
        .toBeLessThanOrEqual(Math.ceil(tab.width));
      expect(tab.labelBottom, `${where}: ${tab.id} label inside its tab`)
        .toBeLessThanOrEqual(Math.ceil(tab.tabBottom));
      expect(tab.width, `${where}: ${tab.id} laid out`).toBeGreaterThan(0);
    }
    expect(read.sidewaysOverflow, `${where}: no sideways scroll`).toBeLessThanOrEqual(1);

    if (previous) {
      for (const [i, tab] of read.tabs.entries()) {
        expect(tab.fontSize, `${where}: ${tab.id} did not shrink`)
          .toBeGreaterThan(previous.tabs[i].fontSize);
      }
    }
    previous = read;
  }
});
