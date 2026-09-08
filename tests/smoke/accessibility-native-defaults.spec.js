import { test, expect } from '@playwright/test';

// Render the ordinary client with the native document's documented default
// seam. This covers consumers and private persistence, not the WinRT getter
// or Ultralight itself (those have separate native checks).
test('native defaults survive override, reload and reset independently for two players', async ({ browser, baseURL }) => {
  const contexts = await Promise.all([0, 1].map(() => browser.newContext({ baseURL })));
  try {
    for (const context of contexts) {
      await context.addInitScript(() => {
        window.PhoenixOsAccessibilityDefaults = {
          textScale: 1.5, contrast: true, reducedMotion: true,
          availability: { textScale: true, contrast: true, reducedMotion: true },
        };
      });
    }
    const pages = await Promise.all(contexts.map(context => context.newPage()));
    async function effects(page) {
      return page.evaluate(() => {
        const root = document.documentElement;
        return {
          scale: getComputedStyle(root).getPropertyValue('--a11y-text-scale').trim(),
          contrast: root.getAttribute('data-contrast'),
          motion: root.getAttribute('data-reduced-motion'),
        };
      });
    }
    const defaults = { scale: '1.5', contrast: 'more', motion: 'reduce' };
    for (const page of pages) {
      await page.goto('/client/');
      await page.waitForFunction(() => typeof window.setAccessibilityPresentation === 'function');
      await expect.poll(() => effects(page)).toEqual(defaults);
    }
    await pages[0].evaluate(() => {
      window.setAccessibilityPresentation('textScale', 1);
      window.setAccessibilityPresentation('contrast', 'off');
      window.setAccessibilityPresentation('reducedMotion', 'off');
    });
    const overridden = await effects(pages[0]);
    expect(overridden.scale).toBe('1');
    expect(overridden.contrast).toBe('standard');
    expect(overridden.motion).not.toBe('reduce');
    await pages[0].reload();
    await expect.poll(() => effects(pages[0])).toEqual(overridden);
    expect(await effects(pages[1])).toEqual(defaults);
    await pages[0].evaluate(() => {
      for (const effect of ['textScale', 'contrast', 'reducedMotion']) {
        window.setAccessibilityPresentation(effect, 'default');
      }
    });
    await expect.poll(() => effects(pages[0])).toEqual(defaults);
    await pages[0].reload();
    await expect.poll(() => effects(pages[0])).toEqual(defaults);
    expect(await effects(pages[1])).toEqual(defaults);
  } finally {
    await Promise.all(contexts.map(context => context.close()));
  }
});
