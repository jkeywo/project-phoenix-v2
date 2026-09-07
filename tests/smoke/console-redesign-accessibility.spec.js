import { test, expect } from '@playwright/test';

// Exercise the shipped renderer and shell stylesheet with a deterministic
// direct + overlay + visiting model. Host/iframe routing lives in console-tabs.
for (const viewport of [{ width: 390, height: 844 }, { width: 844, height: 390 }]) {
  test(`Station Bar roving and presentation ${viewport.width}x${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    // Keep the shipped markup and CSS but isolate the renderer from the
    // disconnected lobby's periodic updates, which would hide this bar.
    // The separate accessibility-* specs cover the full settings write path.
    await page.route('**/client/', async route => {
      const response = await route.fetch();
      const html = (await response.text()).replace(/<script\b[^>]*>[\s\S]*?<\/script>/gi, '');
      await route.fulfill({ response, body: html });
    });
    await page.goto('/client/');
    await page.evaluate(async () => {
      const { applyEffectsToRoot, resolveEffects } = await import('/client/gui/accessibility-profile.js');
      const presentation = {};
      window.__setPresentation = (effect, value) => {
        presentation[effect] = value;
        applyEffectsToRoot(document.documentElement, resolveEffects({ presentation }));
      };
      const { heroBarModel, renderHeroBarDom } = await import('/client/gui/hero-bar.js');
      const bar = document.getElementById('station-hero');
      bar.hidden = false;
      bar.style.display = 'flex';
      // The disconnected shell's full-screen join surfaces are outside this
      // renderer test; place its actual bar above them without changing CSS.
      bar.style.position = 'fixed';
      bar.style.inset = '0 auto auto 0';
      bar.style.zIndex = '10000';
      document.body.append(bar);
      window.__barActivations = [];
      let activeStation = 'tactical', activeOverlay = null;
      const draw = () => renderHeroBarDom({
        tabsEl: document.getElementById('station-hero-tabs'),
        titleEl: document.getElementById('station-hero-title'),
        ratingEl: document.getElementById('station-hero-rating'),
        aiEl: document.getElementById('station-hero-ai'),
        translate: key => key,
        labelMode: 'code',
        model: heroBarModel({ directStation: 'tactical', activeStation, activeOverlay,
          stations: [
            { id: 'tactical', name: 'Tactical', short_code: 'TAC' },
            { id: 'helm', name: 'Helm', short_code: 'HELM', human_seeking: true },
          ],
          stationHosts: { helm: { host: 'tactical' } },
          stationHealth: { tactical: 0.5, helm: 1 },
          consoleTabs: [{ id: 'intel', name: 'Intel', code: 'INT', badge: 2 }],
        }),
        onActivate: (id, kind) => {
          window.__barActivations.push([id, kind]);
          activeOverlay = kind === 'overlay' ? id : null;
          if (kind === 'station') activeStation = id;
          draw();
        },
      });
      draw();
    });
    const tabs = page.locator('#station-hero-tabs [role=tab]');
    await expect(tabs).toHaveCount(3);
    await tabs.first().focus();
    for (const [key, selected] of [['ArrowRight', 'intel'], ['ArrowDown', 'helm'], ['Home', 'tactical'], ['End', 'helm'], ['ArrowRight', 'tactical']]) {
      await page.keyboard.press(key);
      const current = page.locator(`#station-hero-tabs [data-tab-id="${selected}"]`);
      await expect(current, `${key}: ${JSON.stringify(await page.evaluate(() => window.__barActivations))}`).toBeFocused();
      await expect(current).toHaveAttribute('aria-selected', 'true');
      await expect(page.locator('#station-hero-tabs [tabindex="0"]')).toHaveCount(1);
    }
    expect(await page.evaluate(() => window.__barActivations)).toEqual([
      ['intel', 'overlay'], ['helm', 'station'], ['tactical', 'station'], ['helm', 'station'], ['tactical', 'station'],
    ]);
    const palette = () => tabs.first().evaluate(el => {
      const style = getComputedStyle(el);
      return { color: style.color, border: style.borderTopColor };
    });
    await page.evaluate(() => window.__setPresentation('contrast', 'off'));
    const normal = await palette();
    await page.evaluate(() => window.__setPresentation('contrast', 'on'));
    const contrast = await palette();
    expect(contrast.color).not.toBe(normal.color);
    expect(contrast.border).not.toBe(normal.border);
    // Read the real Red Alert bezel, not a synthetic animation probe.
    await page.evaluate(() => {
      window.__setPresentation('reducedMotion', 'off');
      document.getElementById('phone-bezel').classList.add('alert-on');
    });
    const motion = () => page.locator('#phone-bezel').evaluate(el => {
      const s = getComputedStyle(el);
      return { name: s.animationName, count: s.animationIterationCount };
    });
    expect((await motion()).name).toBe('bezel-pulse');
    await page.evaluate(() => window.__setPresentation('reducedMotion', 'on'));
    expect((await motion()).count).not.toBe('infinite');
  });
}
