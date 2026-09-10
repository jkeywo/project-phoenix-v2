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
//
// The one exception is the hull cards' own detail, which the second test below
// guards because a real page is the ONLY thing that can: the host has to
// deliver each hull's template to Rust before it reads the catalogue back, and
// nothing cheaper ever sees that ordering.

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
    // slices each ADD a row or activate one, so a spec that pinned "five
    // entries" — or "one live entry" — would fail on the next one for no
    // reason. What must hold is that New Game is offered and live, and that the
    // rows still WAITING for a slice are the ones that say so. On THIS host,
    // before a world has booted, that is now none of them: Load Game got its
    // stage in #1363, both join routes got theirs in #1364, and Load mod pack
    // is native-only — a shelf is a scanned folder, and the browser's own
    // mod-pack door is the live upload control inside the picker.
    await expect(page.locator(`${MENU} [data-landing-entry]`).first())
      .toBeVisible({ timeout: 30_000 });
    // `toContainText`, not `toHaveText`: the button also carries its ordinal
    // and its description line. And a plain string, not a RegExp — the copy is
    // still in its `[bracketed]` draft phase, and `[New Game]` read as a
    // pattern is a character class that matches nearly anything.
    await expect(page.locator(NEW_GAME)).toContainText(ts('server.landing.new_game'));
    await expect(page.locator(`${NEW_GAME}[aria-disabled="true"]`)).toHaveCount(0);
    expect(await page.locator(`${MENU} [data-landing-entry][aria-disabled="true"]`)
      .evaluateAll((els) => els.map((el) => el.getAttribute('data-landing-entry'))))
      .toEqual([]);

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

// Tagged @core deliberately, and kept short enough to earn its place there:
// this is the ONLY test that can catch the enrichment regressing. `class`,
// `hull_id`, `mass` and `power_rating` are read by
// `delivery::payload::ship_payload` out of a CACHED entity template, and
// `buildScenarioCatalog` used to read the catalogue without ever delivering
// one — so every card badged `component.ship_picker.class.unknown` and drew no
// stats. The vitest suite hands `<ph-ship-picker>` a row that already carries
// the fields, and the `?scenario=` dev bypass reads its hulls back AFTER the
// preload, so a revert would ship green past both. In the nightly-only tier
// this guard would be worth nothing to the PR that broke it.
//
// combat_test BY NAME, not "whichever World is first": a single-hull scenario
// auto-resolves (issue #917) and never draws a card at all, so reading the
// first row would turn this into a silent skip the day the manifest is
// reordered. And the wait is unconditional — no picker here is a FAILURE.
test('the picker\'s hull cards know which hull they are',
  { tag: '@core' }, async ({ context }) => {
    const page = await context.newPage();
    await page.goto('/');
    await page.bringToFront();

    await page.locator(NEW_GAME).waitFor({ state: 'visible', timeout: 30_000 });
    await page.click(NEW_GAME);
    await page.locator('#world-list .world-btn[data-scenario-id="combat_test"]')
      .click({ timeout: 30_000 });

    const shipCard = page.locator('ph-ship-picker .ship-card').first();
    await shipCard.waitFor({ state: 'visible', timeout: 60_000 });

    // Asserted structurally rather than against `#AEV-0741`/`70`: the failure
    // this guards is TOTAL (no enrichment at all), not a wrong number, and the
    // numbers themselves are the world file's business to change.
    await expect(shipCard.locator('.ship-badge')).not.toHaveClass(/(^|\s)unknown(\s|$)/);
    await expect(shipCard.locator('.ship-stat-value')).not.toHaveCount(0);
    await expect(shipCard.locator('.ship-stat-value').first()).not.toBeEmpty();
    // Nothing is clicked: the walk past this point is the next test's, and this
    // one has no reason to pay for a world load.
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
    // The picker mounts in `#landing-ship` since issue #1362 — a column of the
    // landing's own beside the World list — so this is no longer scoped to
    // `#scenario-panel`. When it IS the hull stage, the two things that slice
    // claimed are checked on the way past: the World rows are still standing,
    // and the landing has slid one column further rather than swapped one.
    //
    // RACE, not a poll: `isVisible()` does NOT wait — it answers about this
    // instant and ignores a `timeout` passed to it. Asking it straight after the
    // world click therefore always said "no picker", the ship was never chosen,
    // and the assertions below then failed on a landing that was right to still
    // be up. So wait for whichever of the two outcomes this world produces, and
    // click only if it was the picker.
    const shipCard = page.locator('ph-ship-picker .ship-card').first();
    const landingGone = page.locator('#landing-panel');
    await Promise.race([
      shipCard.waitFor({ state: 'visible', timeout: 60_000 }),
      landingGone.waitFor({ state: 'hidden', timeout: 60_000 }),
    ]);
    if (await shipCard.isVisible()) {
      await expect(page.locator('#landing-ship ph-ship-picker')).toBeVisible();
      await expect(page.locator(SCENARIO_BUTTONS).first()).toBeVisible();
      await expect(page.locator('#landing-panel')).toHaveClass(/is-deep/);
      // What the card SAYS is the @core test's business, not this one's: it
      // names combat_test so it cannot be reordered into skipping, whereas
      // everything in this branch is conditional on which World came first.
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

    // ── Round two: the landing that comes BACK (issue #1364) ─────────────
    //
    // Return to Lobby re-shows this same landing over a world that is ALREADY
    // loaded into the running engine, and that is the one state the join routes
    // cannot be offered in: `is_browser_gm` is read once, inside `wasm_init`,
    // so Join as Peer here would compose the document as a Game Master page
    // over an app Bevy built as a `BrowserHost` — blanking the viewscreen and
    // removing the crew QR for the rest of the session, with no way back but a
    // reload. Only a real page can show that the second landing is the first
    // one again minus those two rows; the vitest suite proves the decision, not
    // the round trip.
    await page.evaluate(() => window.__hostReturnToLobby());
    await expect(page.locator('#landing-panel')).toBeVisible({ timeout: 30_000 });
    // The viewscreen is still a viewscreen, and this page is still a host.
    await expect(page.locator('#canvas')).toBeVisible();
    expect(await page.evaluate(
      () => document.documentElement.classList.contains('phoenix-gm-page'),
    )).toBe(false);
    // ...and the rows that would have changed that are inert IN PLACE, rather
    // than gone from the menu.
    expect(await page.locator(`${MENU} [data-landing-entry][aria-disabled="true"]`)
      .evaluateAll((els) => els.map((el) => el.getAttribute('data-landing-entry'))))
      .toEqual(['host_gm', 'join_peer', 'connect_host']);
    await expect(page.locator(`${NEW_GAME}[aria-disabled="true"]`)).toHaveCount(0);

    // Round two's hull cards are NOT asserted here, and the omission is
    // deliberate rather than an oversight: every hull combat_test offers is in
    // its own `[[available_ships]]`, so round one's #917 curation preloaded all
    // four and the second round's cards would read out of the preload's cache
    // whether the catalogue store were re-delivered or not — an assertion that
    // cannot fail. The round-two enrichment is guarded where it CAN fail, by
    // uploading a pack whose hulls the first round never saw
    // (mod-pack.spec.js, 'a pack uploaded in a SECOND lobby round').
  });
