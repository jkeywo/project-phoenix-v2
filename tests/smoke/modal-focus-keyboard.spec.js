// Issue #1174 — Keyboard-only smoke for the modal focus contract.
//
// The end-to-end claim of PRD #1168's modal slice: a host with no pointing
// device can OPEN the settings panel, MOVE around inside it, and DISMISS it,
// and focus never escapes the modal while it is open nor is stranded when it
// closes. This drives the real host page's settings cog (the same one
// server-settings-cog.spec.js measures) and asserts:
//
//   - opening moves focus INTO the panel;
//   - Tab / Shift+Tab keep focus inside the panel, never on the page behind it;
//   - the background is `inert` while the panel is open;
//   - Escape closes the panel and RESTORES focus to the cog that opened it.
//
// As in tactical-keyboard.spec.js, the one rule is that every interaction is a
// keyboard.* / locator.press() call — never click/tap/hover/mouse — and a
// pointer-event guard counts any mouse/pointer/touch event, which must stay 0.

import { test, expect } from './fixtures';

const COG = '#server-settings-btn';
const OVERLAY = '#server-settings-overlay';

/** The active element's identity, and whether it sits inside the overlay. */
async function focusState(page) {
  return page.evaluate((overlaySel) => {
    const el = document.activeElement;
    const overlay = document.querySelector(overlaySel);
    return {
      id: el && el.id ? '#' + el.id : (el ? el.tagName : null),
      inOverlay: !!(overlay && el && overlay.contains(el)),
    };
  }, OVERLAY);
}

test('settings panel: opens, traps, and dismisses from the keyboard alone', async ({ context }) => {
  const page = await context.newPage();
  await page.goto('/');

  // The cog mounts on the first screen; wait for it before touching anything.
  await page.locator(COG).waitFor({ state: 'attached', timeout: 30_000 });

  // Prove no pointer path is taken: a keyboard-activated button emits a
  // synthetic `click`, but never a mousedown/pointerdown/touchstart — so those
  // are the honest "a pointer was used" signal to count.
  await page.evaluate(() => {
    window.__pointerEvents = 0;
    for (const type of ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'touchstart', 'pointermove']) {
      window.addEventListener(type, () => { window.__pointerEvents += 1; }, { capture: true });
    }
  });

  // ── Open from the keyboard: focus the cog (no pointer) and press Enter ───────
  // locator.press() focuses the element and dispatches a keydown — it does not
  // use the mouse. A native button activates on Enter, opening the panel.
  await page.locator(COG).press('Enter');

  // Opening moved focus INTO the panel (to its first control).
  await expect.poll(async () => (await focusState(page)).inOverlay).toBe(true);

  // The page behind the panel is inert for the duration.
  expect(await page.evaluate(() => {
    const overlay = document.getElementById('server-settings-overlay');
    const roots = Array.from(document.body.children).filter((el) => el !== overlay && el.id !== 'server-settings-btn');
    // At least one background root, and every one of them inert.
    return roots.length > 0 && roots.every((el) => el.hasAttribute('inert'));
  })).toBe(true);

  // ── Tab and Shift+Tab keep focus inside the panel ───────────────────────────
  for (let i = 0; i < 6; i += 1) {
    await page.keyboard.press('Tab');
    expect((await focusState(page)).inOverlay, `Tab #${i + 1} let focus escape the modal`).toBe(true);
  }
  for (let i = 0; i < 6; i += 1) {
    await page.keyboard.press('Shift+Tab');
    expect((await focusState(page)).inOverlay, `Shift+Tab #${i + 1} let focus escape the modal`).toBe(true);
  }

  // ── Escape closes the panel and restores focus to the cog ───────────────────
  await page.keyboard.press('Escape');
  await expect.poll(() => page.evaluate(() => document.getElementById('server-settings-overlay').hidden)).toBe(true);
  expect((await focusState(page)).id).toBe(COG);

  // The background is live again once the modal is gone.
  expect(await page.evaluate(() => {
    const roots = Array.from(document.body.children);
    return roots.every((el) => !el.hasAttribute('inert'));
  })).toBe(true);

  // Not one pointer event was used to open, operate, or dismiss the panel.
  expect(await page.evaluate(() => window.__pointerEvents)).toBe(0);
});
