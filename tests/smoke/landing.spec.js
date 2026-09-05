// Issue #1360 — the landing screen is the host's first paint, and New Game is
// the one thing on it that works.
//
// The vitest suites beside this one prove the decision (which entries, which
// one is open) and the writes (into a document, including an incomplete one).
// Neither can prove the thing this slice is actually claiming: that on a REAL
// cold page load, before anything is clicked, the landing is what an operator
// sees — ahead of `#scenario-panel`, which is CSS-visible at z-index 200 before
// a line of script runs and was the first paint until this slice. jsdom
// computes no layout and no stacking, so only a browser can answer that.
//
// The walk is deliberately the whole route and nothing more: first paint ->
// the menu -> New Game -> the World picker -> New Game again closes it. What
// happens AFTER a world is chosen is already covered several times over
// (lobby.spec.js, demo-manifest.spec.js, mod-pack.spec.js), and this spec
// asserts only that the pick still reaches the same place.

import { test, expect, waitForWasmReady } from './fixtures';
import { ts } from './strings';

const MENU = '#landing-menu';
const ENTRY = (id) => `${MENU} [data-landing-entry="${id}"]`;
const NEW_GAME = ENTRY('new_game');
const SCENARIO_BUTTONS = '#world-list .world-btn[data-scenario-id]';

/** The rendered stacking answer: what is actually on top at the screen centre. */
async function topmostAt(page, x, y) {
  return page.evaluate(([px, py]) => {
    const el = document.elementFromPoint(px, py);
    // Walk up to the nearest full-surface panel, which is the thing whose
    // identity the question is really about.
    const surface = el && el.closest('#landing-panel, #scenario-panel, #lobby-panel');
    return surface ? surface.id : (el ? el.id || el.tagName : null);
  }, [x, y]);
}

test('the landing is the first paint, and New Game opens the World picker',
  { tag: '@core' }, async ({ context }) => {
    const page = await context.newPage();
    // No `?scenario=` — that dev bypass names the world in the URL and
    // dismisses the landing at parse time, which is the case this spec must
    // NOT take. A cold `/` is what an operator opens.
    await page.goto('/');
    await page.bringToFront();

    // ── First paint ──────────────────────────────────────────────────────
    //
    // The landing's chrome is markup and CSS, so it is there before any module
    // has loaded — that is what "first paint" means here.
    const landing = page.locator('#landing-panel');
    await expect(landing).toBeVisible({ timeout: 30_000 });
    await expect(landing).toHaveClass(/is-idle/);

    // ...and it is ON TOP of the picker, not merely present beside it. The
    // centre of the viewport is where #scenario-panel paints its logo.
    const box = await landing.boundingBox();
    expect(await topmostAt(page, box.x + box.width / 2, box.y + box.height / 2))
      .toBe('landing-panel');

    // The identity the AC asks for: the logo, the game name, and the build.
    await expect(page.locator('#landing-logo')).toBeVisible();
    await expect(page.locator('#landing-title')).toHaveText(ts('server.landing.title'));
    await expect(page.locator('#landing-status-build')).not.toBeEmpty();

    // ── The menu ─────────────────────────────────────────────────────────
    //
    // Read as ids, not as English, and not as a pinned count: the six sibling
    // slices each ADD a row, so a spec that pinned "five entries" would fail on
    // the next one for no reason. What must hold is that New Game is offered
    // and is the only entry that is not inert.
    await expect(page.locator(`${MENU} [data-landing-entry]`).first())
      .toBeVisible({ timeout: 30_000 });
    // `toContainText`, not `toHaveText`: the button also carries its ordinal
    // and its description line. And a plain string, not a RegExp — the copy is
    // still in its `[bracketed]` draft phase, and `[New Game]` read as a
    // pattern is a character class that matches nearly anything.
    await expect(page.locator(NEW_GAME)).toContainText(ts('server.landing.new_game'));
    await expect(page.locator(`${MENU} [data-landing-entry]:not([aria-disabled="true"])`))
      .toHaveCount(1);

    // ── New Game ─────────────────────────────────────────────────────────
    //
    // The picker exists all along (it is the layer underneath, and Playwright
    // calls its buttons "visible" because occlusion is not visibility). What
    // New Game changes is WHERE it is — the middle column — and therefore that
    // it is on top and can be clicked, which the stacking check above is the
    // honest test of.
    expect(await page.evaluate(
      () => document.getElementById('scenario-panel').parentElement === document.body,
    )).toBe(true);
    await page.click(NEW_GAME);
    await expect(landing).toHaveClass(/is-open/);
    await expect(page.locator(NEW_GAME)).toHaveAttribute('aria-expanded', 'true');

    const picker = page.locator('#landing-mid #scenario-panel');
    await expect(picker).toBeVisible();
    await expect(page.locator(SCENARIO_BUTTONS).first())
      .toBeVisible({ timeout: 30_000 });

    // ── ...and again closes it ───────────────────────────────────────────
    await page.click(NEW_GAME);
    await expect(landing).toHaveClass(/is-idle/);
    await expect(page.locator(NEW_GAME)).toHaveAttribute('aria-expanded', 'false');
    await expect(page.locator('#landing-mid #scenario-panel')).toHaveCount(0);
    // ...and the landing owns the centre of the screen again.
    expect(await topmostAt(page, box.x + box.width / 2, box.y + box.height / 2))
      .toBe('landing-panel');
  });

test('choosing a world from the landing reaches the lobby it always reached',
  async ({ context }) => {
    const page = await context.newPage();
    await page.goto('/');
    await page.bringToFront();

    await page.locator(NEW_GAME).waitFor({ state: 'visible', timeout: 30_000 });
    await page.click(NEW_GAME);

    // The scenario buttons rendering at all means the #754 catalog built, which
    // means WASM instantiated. Take the first one — which world it is does not
    // matter to this assertion, only that the pick still runs driveWorldLoad().
    const first = page.locator(SCENARIO_BUTTONS).first();
    await first.waitFor({ state: 'visible', timeout: 30_000 });
    await first.click();

    // Single-hull scenarios auto-resolve (issue #917); multi-hull ones offer a
    // ph-ship-picker. Either way the landing goes and the lobby arrives, which
    // is the whole of "exactly the lobby it reaches today".
    //
    // RACE, not a poll: `isVisible()` does NOT wait — it answers about this
    // instant and ignores a `timeout` passed to it. Asking it straight after the
    // world click therefore always said "no picker", the ship was never chosen,
    // and the assertions below then failed on a landing that was right to still
    // be up. So wait for whichever of the two outcomes this world produces, and
    // click only if it was the picker.
    const shipCard = page.locator('#scenario-panel ph-ship-picker .ship-card').first();
    const landingGone = page.locator('#landing-panel');
    await Promise.race([
      shipCard.waitFor({ state: 'visible', timeout: 60_000 }),
      landingGone.waitFor({ state: 'hidden', timeout: 60_000 }),
    ]);
    if (await shipCard.isVisible()) {
      await shipCard.click();
    }

    await expect(page.locator('#landing-panel')).toBeHidden({ timeout: 60_000 });
    await expect(page.locator('#lobby-panel')).toBeVisible({ timeout: 60_000 });
    await waitForWasmReady(page);

    // The picker went back to being a body-level layer on the way out, which is
    // what lets a later Return to Lobby show it again (issue #756).
    expect(await page.evaluate(
      () => document.getElementById('scenario-panel').parentElement === document.body,
    )).toBe(true);
  });
