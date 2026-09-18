import { expect } from './fixtures';

export async function revealGmPanel(page, panel) {
  const settings = page.locator('#server-settings-overlay');
  if (await settings.isVisible()) {
    await page.keyboard.press('Escape');
    await settings.waitFor({ state: 'hidden' });
  }
  await page.locator(`#gm-live-layout .workshop-panel-switcher [data-layout-panel="${panel}"]`)
    .evaluate(control => control.click());
  // The tab is pressed; the panel is what the caller is about to touch. A
  // scroll or press that lands while the dock is still bringing it forward
  // races the repaint, so wait for the panel to actually be on screen.
  await page.locator(`#gm-live-layout .workshop-dock-panel[data-panel="${panel}"]`)
    .waitFor({ state: 'visible' });
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

/** Scroll a docked console frame to the bottom edge of its panel, the way an
 *  operator reaches a control near the foot of a console: the dock canvas
 *  scrolls, the frame is not resized. */
export async function bringConsoleIntoView(page, frameSelector) {
  await page.locator(frameSelector).evaluate(node => node.scrollIntoView({ block: 'end' }));
}

/** Dismiss every tutorial card a console shows on first use, one press each,
 *  so the control under test is the one under the pointer. */
export async function dismissTutorialCards(page, frameSelector) {
  const frame = page.frameLocator(frameSelector);
  const tutorial = frame.locator('ph-tutorial-overlay');
  for (let dismissed = 0; dismissed < 16 && await tutorial.isVisible(); dismissed++) {
    await bringConsoleIntoView(page, frameSelector);
    const activeId = await tutorial.evaluate(element => element.state?.active?.id ?? null);
    await tutorial.locator('#dismiss').click();
    await expect.poll(() => tutorial.evaluate(element => element.state?.active?.id ?? null))
      .not.toBe(activeId);
  }
  await expect(tutorial).toBeHidden();
}
