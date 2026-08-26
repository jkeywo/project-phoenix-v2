import { test, expect, createServerPage, readHostPeerId } from './fixtures';

test('client Hero Bar is horizontal in portrait and a left rail in landscape', async ({ context }) => {
  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);
  const captain = await context.newPage();

  await captain.setViewportSize({ width: 480, height: 900 });
  await captain.goto(`/client/#${hostId}`);
  await captain.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await captain.click('#station-list .station-row:has-text("Captain") button.claim-btn');
  await captain.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await captain.click('#ready-btn');
  await captain.waitForSelector('#station-hero[aria-hidden="false"]', { timeout: 10_000 });

  const portrait = await captain.evaluate(() => {
    const hero = document.getElementById('station-hero').getBoundingClientRect();
    const consoleSection = document.querySelector('.console-section.active').getBoundingClientRect();
    const tabs = getComputedStyle(document.getElementById('station-hero-tabs'));
    return { hero, consoleSection, flexDirection: tabs.flexDirection };
  });
  expect(portrait.flexDirection).toBe('row');
  expect(portrait.hero.bottom).toBeLessThanOrEqual(portrait.consoleSection.top + 2);
  expect(portrait.hero.width).toBeGreaterThan(450);

  await captain.setViewportSize({ width: 900, height: 480 });
  await expect.poll(() => captain.evaluate(
    () => getComputedStyle(document.getElementById('station-hero-tabs')).flexDirection,
  )).toBe('column');

  const landscape = await captain.evaluate(() => {
    const container = document.getElementById('console-container');
    const hero = document.getElementById('station-hero').getBoundingClientRect();
    const consoleSection = document.querySelector('.console-section.active').getBoundingClientRect();
    const button = document.querySelector('#station-hero-tabs button');
    return {
      containerDirection: getComputedStyle(container).flexDirection,
      hero,
      consoleSection,
      titleWritingMode: getComputedStyle(document.getElementById('station-hero-details')).writingMode,
      buttonWritingMode: getComputedStyle(button).writingMode,
      overflow: container.scrollWidth - container.clientWidth,
    };
  });
  expect(landscape.containerDirection).toBe('row');
  expect(landscape.hero.left).toBeLessThanOrEqual(landscape.consoleSection.left);
  expect(landscape.hero.right).toBeLessThanOrEqual(landscape.consoleSection.left + 2);
  expect(landscape.hero.height).toBeGreaterThan(470);
  expect(landscape.titleWritingMode).toBe('vertical-rl');
  expect(landscape.buttonWritingMode).toBe('horizontal-tb');
  expect(landscape.overflow).toBeLessThanOrEqual(1);

  await captain.close();
});
