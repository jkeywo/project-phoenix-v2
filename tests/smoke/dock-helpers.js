export async function revealGmPanel(page, panel) {
  const settings = page.locator('#server-settings-overlay');
  if (await settings.isVisible()) {
    await page.keyboard.press('Escape');
    await settings.waitFor({ state: 'hidden' });
  }
  await page.locator(`#gm-live-layout .workshop-panel-switcher [data-layout-panel="${panel}"]`)
    .evaluate(control => control.click());
}

export async function clickGmControl(page, controlId, panel = 'readiness') {
  await revealGmPanel(page, panel);
  await page.locator(`#${controlId}`).evaluate(control => control.click());
}

export async function revealWorkshopPanel(page, panel) {
  await page.locator(`.workshop-layout .workshop-panel-switcher [data-layout-panel="${panel}"]`).click();
}
