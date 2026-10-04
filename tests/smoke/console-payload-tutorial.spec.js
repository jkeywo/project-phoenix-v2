import { test, expect } from '@playwright/test';

test('a visiting-System tutorial reaches the shipped Station through the JSON adapter', async ({ page }) => {
  await page.goto('/gui/battleship/helm.html');
  await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  const payload = await page.evaluate(async () => {
    await import('/gui/console-state.js');
    const state = {
      stationSystems: { helm: ['drive'], liaison: ['radio'] },
      systemConsoleFamilies: { drive: 'helm', radio: 'comms' },
      blackboardKinds: { radio: 'Comms' },
      blackboards: { radio: { host_station: 'helm' } },
      stationTutorials: { helm: [{ id: 'visiting-radio', title: 'component.tutorial.heading', text: 'component.tutorial.dismiss', trigger: { kind: 'state', path: 'systems.radio.comms_auto', op: 'falsy' } }] },
      tutorialProgress: { dismissed: {}, used: {} },
    };
    const json = window.buildConsoleState('helm', state);
    window.__updateConsole('helm', json);
    return JSON.parse(json);
  });
  expect(payload.hosted_systems).toEqual(['drive', 'radio']);
  expect(payload.tutorial.active.id).toBe('visiting-radio');
  await expect(page.locator('ph-tutorial-overlay')).toBeVisible();
  await expect(page.locator('ph-tutorial-overlay').locator('#title')).not.toBeEmpty();
});
