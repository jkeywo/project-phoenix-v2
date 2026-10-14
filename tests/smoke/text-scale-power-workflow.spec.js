import { test, expect } from '@playwright/test';
import { DEVICE_MATRIX, TEXT_SCALES, BROWSER_ZOOMS } from '../fixtures/device-matrix.mjs';
import { energyConsolePayload } from '../fixtures/energy-console.mjs';
import { ts } from './strings';

// Carry the actual Energy workflow through text scaling, browser zoom, native
// controls, forced colours and the shell/Console boundary.
const CARRIED_ON = ['phone-390x844-portrait', 'phone-844x390-landscape', 'tablet-1280x720-interim-landscape', 'native-split-pane-floor'];
const device = id => { const found = DEVICE_MATRIX.find(row => row.id === id); if (!found) throw new Error(id); return found; };
const viewportFor = (entry, scale) => ({ width: entry.kind === 'split-pane' ? Math.round(entry.width * scale) : entry.width, height: entry.height });
const POWER_PAYLOAD = energyConsolePayload();
const POWER_URL = '/client/gui/battleship/power.html';
async function openPowerConsole(page) {
  await page.goto(POWER_URL); await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  await page.evaluate(() => { window.__commands = []; window.activateSemanticAction = (actionId, payload) => { window.__commands.push({ actionId, ...payload }); return true; }; });
  await page.evaluate(state => window.__updateConsole('power', JSON.stringify(state)), POWER_PAYLOAD);
  await expect(page.locator('ph-energy-controls')).toBeVisible(); await page.evaluate(() => document.fonts.ready);
}
async function applyTextScale(page, scale) {
  await page.evaluate(async value => { const { applyEffectsToRoot, resolveEffects } = await import('/client/gui/accessibility-profile.js');
    applyEffectsToRoot(document.documentElement, resolveEffects({ presentation: { textScale: value } })); }, scale);
}
async function textSizes(page) {
  return page.evaluate(() => {
    const shadow = document.querySelector('ph-power-controls').shadowRoot.querySelector('ph-energy-controls').shadowRoot;
    const size = element => element ? parseFloat(getComputedStyle(element).fontSize) : null;
    return { root: size(document.documentElement), battery: size(document.getElementById('bat-val')), group: size(shadow.querySelector('[data-energy-group] legend')),
      status: size(shadow.querySelector('[data-energy-group] .stats')), selector: size(shadow.getElementById('depth')), value: size(shadow.querySelector('output')) };
  });
}
async function reachability(page) {
  return page.evaluate(() => {
    const shadow = document.querySelector('ph-power-controls').shadowRoot.querySelector('ph-energy-controls').shadowRoot;
    const boxed = element => { const box = element.getBoundingClientRect(); return box.width > 0 && box.height > 0; };
    const sliders = [...shadow.querySelectorAll('input[type="range"]')].filter(boxed);
    return { groups: [...shadow.getElementById('groups').children].filter(boxed).length, sliders: sliders.length,
      nativeControls: [...shadow.querySelectorAll('select,input,button')].filter(boxed).every(element => !element.disabled),
      statuses: [...shadow.querySelectorAll('[data-energy-group] .stats')].filter(boxed).map(node => node.textContent),
      battery: document.getElementById('bat-val').textContent,
      horizontalOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth };
  });
}
for (const id of CARRIED_ON) test(`Energy allocation stays readable and reachable at every text scale on ${id}`, async ({ page }) => {
  const entry = device(id); let baseline;
  for (const scale of TEXT_SCALES) {
    await page.setViewportSize(viewportFor(entry, scale)); await openPowerConsole(page); await applyTextScale(page, scale); await page.evaluate(() => document.fonts.ready);
    const reach = await reachability(page), sizes = await textSizes(page), where = `${id} @ ${scale}x`;
    expect(reach.groups, where).toBe(3); expect(reach.sliders, where).toBe(8); expect(reach.nativeControls, where).toBe(true);
    expect(reach.statuses, where).toHaveLength(3); expect(reach.statuses.join(' '), where).toContain(ts('energy.controls.flow_cut'));
    expect(reach.battery, where).toBe('72%'); expect(reach.horizontalOverflow, where).toBeLessThanOrEqual(1);
    for (const [key, value] of Object.entries(sizes)) { expect(value, `${where}:${key}`).toBeGreaterThan(0); if (baseline) expect(value, `${where}:${key}`).toBeGreaterThanOrEqual(baseline[key]); }
    if (!baseline) baseline = sizes; else expect(sizes.root, where).toBeGreaterThan(baseline.root);
  }
});

test('Energy native flow controls remain keyboard operable at 200% text', async ({ page }) => {
  await page.setViewportSize(viewportFor(device('phone-390x844-portrait'), 2)); await openPowerConsole(page); await applyTextScale(page, 2);
  await page.locator('ph-energy-controls #flow-helm').focus(); await page.keyboard.press('ArrowRight');
  expect(await page.evaluate(() => window.__commands)).toEqual([{ actionId: 'power.energy-group', source: 'control', detail: { group: 'helm', flow_per_sec: 5, cooling_per_sec: 1 } }]);
  await page.locator('ph-energy-controls #depth').selectOption('min');
  await page.locator('ph-energy-controls [data-energy-group="helm"] .row button:last-child').focus(); await page.keyboard.press('Enter');
  expect(await page.evaluate(() => window.__commands.at(-1))).toMatchObject({ actionId: 'power.energy-priority', detail: { group: 'helm', priority: 1 } });
});

test('browser zoom and Phoenix text scaling independently retain Energy controls', async ({ browser }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  for (const zoom of BROWSER_ZOOMS) {
    const context = await browser.newContext({ viewport: { width: Math.round(entry.width / zoom), height: Math.round(entry.height / zoom) }, deviceScaleFactor: zoom });
    try { const page = await context.newPage(); await openPowerConsole(page); const before = await textSizes(page);
      for (const scale of [1, 2]) { await applyTextScale(page, scale); const reach = await reachability(page);
        expect(reach.sliders).toBe(8); expect(reach.nativeControls).toBe(true); expect(reach.horizontalOverflow).toBeLessThanOrEqual(1); }
      expect((await textSizes(page)).root).toBeGreaterThan(before.root);
    } finally { await context.close(); }
  }
});

test('forced colours retain native focus, group boundaries and textual intake status', async ({ page }) => {
  await page.setViewportSize(viewportFor(device('tablet-1280x720-interim-landscape'), 1)); await openPowerConsole(page);
  await page.evaluate(async () => { const { applyEffectsToRoot, resolveEffects } = await import('/client/gui/accessibility-profile.js');
    applyEffectsToRoot(document.documentElement, resolveEffects({ presentation: { contrast: 'on' } })); });
  await page.emulateMedia({ forcedColors: 'active' }); await page.locator('ph-energy-controls #depth').focus(); await page.keyboard.press('Tab');
  const styles = await page.evaluate(() => { const shadow = document.querySelector('ph-power-controls').shadowRoot.querySelector('ph-energy-controls').shadowRoot;
    const focused = getComputedStyle(shadow.activeElement), group = getComputedStyle(shadow.querySelector('[data-energy-group]'));
    return { outline: focused.outlineStyle, outlineWidth: parseFloat(focused.outlineWidth), border: parseFloat(group.borderTopWidth), colour: group.borderTopColor }; });
  expect(styles.outline).not.toBe('none'); expect(styles.outlineWidth).toBeGreaterThan(0); expect(styles.border).toBeGreaterThan(0); expect(styles.colour).not.toBe('rgba(0, 0, 0, 0)');
  expect((await reachability(page)).statuses.join(' ')).toContain(ts('energy.controls.flow_cut'));
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
    const ship = { stations: [{ id: 'power', name: 'Power' }], consoles: [{ id: 'power', name: 'Power', home_station: 'power', screen: 'gui/battleship/power.html' }] };
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
        .querySelector('ph-energy-controls').shadowRoot.querySelector('[data-energy-group] legend')).fontSize),
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
      stations,
      consoles: stations.map((st) => ({ ...st, home_station: st.id, screen: 'fixture.html', human_seeking: true })),
      consoleHosts: Object.fromEntries(stations.map((st) => [st.id, { host: 'power' }])),
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
