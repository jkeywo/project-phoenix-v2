import { test, expect } from '@playwright/test';

test('legacy editor bookmark migrates into Workshop authoring', async ({ page }) => {
  await page.goto('/editor.html?file=assets/worlds/combat_test.toml');
  await expect(page).toHaveURL(/workshop\.html#.*panel=files.*file=assets%2Fworlds%2Fcombat_test\.toml/);
  await expect(page.locator('h1')).toContainText('Workshop');
});

test('legacy viewer bookmark migrates into Workshop model preview', async ({ page }) => {
  await page.goto('/viewer.html?model=assets/models/ships/alliance_cruiser.glb&lighting=ambient&gizmos=0');
  await expect(page).toHaveURL(/workshop\.html#.*panel=model-preview.*model=assets%2Fmodels%2Fships%2Falliance_cruiser\.glb/);
  await expect(page.locator('h1')).toContainText('Workshop');
});
