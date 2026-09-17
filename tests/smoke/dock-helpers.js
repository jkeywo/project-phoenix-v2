export async function revealGmPanel(page, panel) {
  const settings = page.locator('#server-settings-overlay');
  if (await settings.isVisible()) {
    await page.keyboard.press('Escape');
    await settings.waitFor({ state: 'hidden' });
  }
  await page.locator(`#gm-live-layout .workshop-panel-switcher [data-layout-panel="${panel}"]`)
    .evaluate(control => control.click());
}

/** Open a complex-action draft and keep it open across presses.
 *
 * A draft closes itself when the world takes the press it carried (issues
 * #1506/#1511), which is right for an operator and wrong for a spec that sends
 * several in a row — so this ticks the panel's own Keep open, the same control
 * an operator uses for exactly that.
 */
export async function openGmDraft(page, panel, keepOpenId) {
  await revealGmPanel(page, panel);
  // The operator's own control, pressed the way an operator presses it: a
  // Keep open that is not reachable is not a Keep open.
  await page.locator(`#${keepOpenId}`).check();
}

export async function clickGmControl(page, controlId, panel = 'readiness') {
  await revealGmPanel(page, panel);
  await page.locator(`#${controlId}`).evaluate(control => control.click());
}

export async function revealWorkshopPanel(page, panel) {
  await page.locator(`.workshop-layout .workshop-panel-switcher [data-layout-panel="${panel}"]`).click();
}
