import { test, expect, createServerPage, readHostPeerId } from './fixtures';

/**
 * The Station bar is the phone's only header (PRD #1371, issue #1372): the
 * settings cog leads it, a help button closes it, and the tabs between them
 * shrink to the hull's authored short codes when the bar is phone-sized.
 *
 * Three viewports, because the bar has three jobs: a 52px strip across a phone
 * held upright, a narrow rail down the side of one held sideways, and a wider
 * rail with room for full Station names on a desktop. This measures the real
 * boxes — tests/client/hero-bar.test.js pins the rules that produce them.
 */
test('the Station bar is a phone strip, a phone rail and a desktop rail', async ({ context }) => {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);
  const captain = await context.newPage();

  await captain.setViewportSize({ width: 390, height: 844 });
  await captain.goto(`/client/#${hostId}`);
  await captain.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await captain.click('#station-list .station-row:has-text("Captain") button.claim-btn');
  await captain.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await captain.click('#ready-btn');
  await captain.waitForSelector('#station-hero[aria-hidden="false"]', { timeout: 10_000 });
  // The cog is re-parented on the render that raises the bar, so wait for the
  // move rather than for the bar alone.
  await captain.waitForSelector('#station-hero > #settings-btn', { timeout: 10_000 });

  const readBar = () => captain.evaluate(() => {
    const bar = document.getElementById('station-hero');
    const tabsEl = document.getElementById('station-hero-tabs');
    const strip = document.querySelector('.station-tab-health');
    const ai = document.getElementById('station-hero-ai');
    const tabs = [...tabsEl.querySelectorAll('button[data-station]')];
    return {
      hero: bar.getBoundingClientRect(),
      consoleSection: document.querySelector('.console-section.active').getBoundingClientRect(),
      containerDirection: getComputedStyle(document.getElementById('console-container')).flexDirection,
      tabsDirection: getComputedStyle(tabsEl).flexDirection,
      firstChildId: bar.firstElementChild.id,
      lastChildId: bar.lastElementChild.id,
      // What the tabs actually read, and whether they fit without a scroller.
      labels: tabs.map((b) => b.children[0].textContent.trim()),
      names: tabs.map((b) => b.title),
      tabsOverflow: tabsEl.scrollWidth - tabsEl.clientWidth,
      pageOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      cog: document.getElementById('settings-btn').getBoundingClientRect(),
      help: document.getElementById('help-btn').getBoundingClientRect(),
      // A 44px box is worth nothing if something else is drawn over it. The
      // page's other fixed chrome (#top-bar, z-index 20) outranks the bar's 16
      // and shares its corner, so ask the browser who actually receives a tap
      // in the middle of each button. `closest` because the target the finger
      // meets is the ::after that widens the ink to the touch floor.
      hitIds: ['settings-btn', 'help-btn'].map((id) => {
        const el = document.getElementById(id);
        const box = el.getBoundingClientRect();
        const at = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
        if (!at) return 'nothing';
        const chrome = at.closest('.settings-btn');
        return chrome ? chrome.id : (at.id || at.tagName.toLowerCase());
      }),
      healthStripHeight: strip ? getComputedStyle(strip).height : null,
      // The AI roll-call stays in the tree for a screen reader even where the
      // title block is dropped for room.
      aiLive: ai ? ai.getAttribute('aria-live') : null,
      aiClipped: ai ? ai.getBoundingClientRect().height <= 1 : null,
      titleShown: getComputedStyle(document.getElementById('station-hero-title')).display,
    };
  });

  // ── Phone, upright: a strip across the top, tabs as short codes ───────────
  const portrait = await readBar();
  expect(portrait.tabsDirection).toBe('row');
  expect(portrait.hero.width).toBeGreaterThan(380);
  // 53px of strip: the 44px touch floor its tallest child carries, 4px above
  // and below it, and the bottom border — the bar counts its own padding and
  // border, so the 52px floor in the stylesheet is a floor and not a base to
  // pile them onto.
  expect(portrait.hero.height).toBeGreaterThanOrEqual(52);
  expect(portrait.hero.height).toBeLessThanOrEqual(56);
  expect(portrait.hero.bottom).toBeLessThanOrEqual(portrait.consoleSection.top + 2);
  // Cog first, help last, both inside the bar.
  expect(portrait.firstChildId).toBe('settings-btn');
  expect(portrait.lastChildId).toBe('help-btn');
  for (const chrome of [portrait.cog, portrait.help]) {
    expect(chrome.height).toBeGreaterThanOrEqual(44);
    expect(chrome.width).toBeGreaterThanOrEqual(40);
  }
  // …and a tap in the middle of each one reaches it. The strip runs the full
  // width of the page, so the fixed #top-bar corner has to give way to the
  // help button that now closes the bar rather than sit on top of it.
  expect(portrait.hitIds).toEqual(['settings-btn', 'help-btn']);
  // Short codes, and every one of them on screen at once.
  expect(portrait.labels.length).toBeGreaterThan(0);
  for (const label of portrait.labels) expect(label).toMatch(/^[A-Z]{2,4}$/);
  expect(portrait.tabsOverflow).toBeLessThanOrEqual(1);
  expect(portrait.pageOverflow).toBeLessThanOrEqual(1);
  // The tab still carries the 5px health strip and the full name for AT.
  expect(portrait.healthStripHeight).toBe('5px');
  expect(portrait.names.every((name) => name.length > 0)).toBe(true);
  // The title block is dropped for room; the live region is not.
  expect(portrait.titleShown).toBe('none');
  expect(portrait.aiLive).toBe('polite');
  expect(portrait.aiClipped).toBe(true);

  // ── Phone, sideways: a narrow left rail ───────────────────────────────────
  await captain.setViewportSize({ width: 844, height: 390 });
  await expect.poll(() => captain.evaluate(
    () => getComputedStyle(document.getElementById('station-hero-tabs')).flexDirection,
  )).toBe('column');

  const landscapePhone = await readBar();
  expect(landscapePhone.containerDirection).toBe('row');
  expect(landscapePhone.hero.width).toBe(96);
  expect(landscapePhone.hero.left).toBeLessThanOrEqual(landscapePhone.consoleSection.left);
  expect(landscapePhone.hero.right).toBeLessThanOrEqual(landscapePhone.consoleSection.left + 2);
  expect(landscapePhone.hero.height).toBeGreaterThan(380);
  expect(landscapePhone.firstChildId).toBe('settings-btn');
  expect(landscapePhone.lastChildId).toBe('help-btn');
  expect(landscapePhone.hitIds).toEqual(['settings-btn', 'help-btn']);
  for (const label of landscapePhone.labels) expect(label).toMatch(/^[A-Z]{2,4}$/);
  expect(landscapePhone.pageOverflow).toBeLessThanOrEqual(1);

  // ── Desktop: the wider rail, with room for the names ──────────────────────
  await captain.setViewportSize({ width: 1440, height: 900 });
  await expect.poll(() => captain.evaluate(
    () => document.getElementById('station-hero').getBoundingClientRect().width,
  )).toBe(132);

  const desktop = await readBar();
  expect(desktop.containerDirection).toBe('row');
  expect(desktop.tabsDirection).toBe('column');
  // Full Station names, not codes — the rail has the room for them here.
  expect(desktop.labels).toEqual(desktop.names);
  expect(desktop.labels.some((label) => /[a-z]/.test(label))).toBe(true);
  expect(desktop.hitIds).toEqual(['settings-btn', 'help-btn']);
  expect(desktop.pageOverflow).toBeLessThanOrEqual(1);

  await captain.close();
});
