// Issue #939 — the settings cog must not paint over the panels underneath it.
//
// The cog is `position: fixed` at top/left 10px, 34px square, z-index 210. It
// wins the stacking fight against every full-viewport panel on purpose (so the
// host can always reach master volume, which is the only control that matters
// while the menu music is the only thing playing) — and that is exactly what
// lets it land on top of whatever those panels put in the same corner. Because
// it is `fixed` it contributes no layout, so nothing below it can push it
// aside: each panel has to reserve the corner itself.
//
// Two corners regressed and are guarded here:
//
//   - `#world-list-label` ("SELECT A WORLD") sat at y 24 with the cog covering
//     y 10–44, so the host read "⚙ CT A WORLD" on the first screen of every
//     launch and every return-to-lobby. Set by `#world-list`'s padding alone,
//     so it was width-independent, not a narrow-viewport edge case.
//   - `#lobby-title` sat at the `.lobby-panel-wrap` clamp's left padding,
//     which only clears 44px above roughly 1100px of viewport width.
//
// Issue #1360 moved the first screen. The landing is what a cold `/` paints
// now, at z-index 205 over the picker, and the cog's corner lands inside its
// `.landing-rail` — so the FIRST test below measures the cog against what is
// actually in that corner today (`.landing-lockup`, `#landing-rail-stamp`)
// before it measures the picker at all. Without that, this file would still
// pass while guarding a corner nothing sits in: the picker only reaches the
// screen docked in the landing's middle column, well clear of the cog.
//
// The vitest suite (tests/client/server-settings.test.js) can only assert that
// the keep-out padding is *declared* — jsdom computes no layout. These are the
// assertions that measure the rendered rectangles.

import { test, expect, waitForWasmReady, openWorldPicker } from './fixtures';

/** Bounding boxes of the cog and `selector`, plus whether they intersect.
 *
 *  Resolves to `{ cog, target, intersects }`, where each box is
 *  `{ x, y, right, bottom, w, h }` — `w`/`h` are the rect's width/height.
 */
async function overlap(page, selector) {
  return page.evaluate((sel) => {
    const box = (el) => {
      const b = el.getBoundingClientRect();
      return { x: b.x, y: b.y, right: b.right, bottom: b.bottom, w: b.width, h: b.height };
    };
    const btn = document.getElementById('server-settings-btn');
    if (!btn) throw new Error('#server-settings-btn is not mounted');
    const el = document.querySelector(sel);
    if (!el) throw new Error(`${sel} not found`);
    const cog = box(btn);
    const target = box(el);
    const intersects = !(
      cog.right <= target.x || target.right <= cog.x ||
      cog.bottom <= target.y || target.bottom <= cog.y
    );
    return { cog, target, intersects };
  }, selector);
}

test('landing, then picker: the cog clears whatever is in the corner', async ({ context }) => {
  const serverPage = await context.newPage();
  // No `?scenario=` bypass: a cold `/` is the literal first screen the host
  // sees, and the one `hostReturnToLobby` comes back to. Since issue #1360
  // that screen is the landing, not the picker.
  await serverPage.goto('/');
  await serverPage.bringToFront();

  // ── The landing, before anything is clicked ──────────────────────────────────────
  //
  // The cog is mounted by gui/server-settings.js and the menu is drawn by the
  // landing's module island, so wait for both before measuring: an unmounted
  // cog throws in `overlap`, and a menu that has not been filled would let a
  // clash appear a frame later.
  await serverPage
    .locator('#server-settings-btn')
    .waitFor({ state: 'visible', timeout: 30_000 });
  await serverPage
    .locator('#landing-menu [data-landing-entry]')
    .first()
    .waitFor({ state: 'visible', timeout: 30_000 });

  // The cog sits INSIDE `.landing-rail` (64px wide, full height) rather than
  // beside it, so nothing about that rail's own box can be the guard. What has
  // to hold is that the two things a host reads in that corner stay out from
  // under it: the rail's vertical stamp, which the rail keeps at its far end,
  // and the identity lockup — logo, `#landing-title`, tagline — which starts
  // below the cog because `.landing-stage` opens at `top: 88px`.
  const stamp = await overlap(serverPage, '#landing-rail-stamp');
  expect(stamp.cog.w, 'cog should be its authored 34px square').toBeGreaterThan(0);
  expect(
    stamp.intersects,
    `cog ${JSON.stringify(stamp.cog)} overlaps #landing-rail-stamp ${JSON.stringify(stamp.target)}`,
  ).toBe(false);

  const lockup = await overlap(serverPage, '.landing-lockup');
  expect(
    lockup.intersects,
    `cog ${JSON.stringify(lockup.cog)} overlaps .landing-lockup ${JSON.stringify(lockup.target)}`,
  ).toBe(false);

  // ── ...and then the World picker, docked (issue #1360) ─────────────────────
  await openWorldPicker(serverPage);

  // `.world-btn` alone would also match the always-present #mod-pack-btn, so
  // scope to the scenario-only data attribute (same selector demo-manifest
  // .spec.js uses). Its presence means renderScenarioLockState() has run and
  // the panel is in the state a host actually reads.
  await serverPage
    .locator('#world-list .world-btn[data-scenario-id]')
    .first()
    .waitFor({ state: 'visible', timeout: 30_000 });

  // HONEST NOTE: docked, `#world-list` is a child of `#landing-mid` roughly
  // 500px in and 88px down, so this pair cannot intersect and
  // gui/host-landing.css drops the very padding the original regression
  // needed (`#scenario-panel.landing-docked #world-list { padding-top: 16px }`).
  // It is kept because the undocked, full-viewport picker is still the shape
  // `#world-list`'s own keep-out padding exists for, and this is the assertion
  // that would catch its removal the day a surface shows the picker that way
  // again. The corner that is live TODAY is the landing, above.
  const label = await overlap(serverPage, '#world-list-label');
  expect(
    label.intersects,
    `cog ${JSON.stringify(label.cog)} overlaps #world-list-label ${JSON.stringify(label.target)}`,
  ).toBe(false);

  // The first scenario button is the next thing down the column; the keep-out
  // must not have merely pushed the clash onto it.
  const first = await overlap(serverPage, '#world-list .world-btn[data-scenario-id]');
  expect(first.intersects, 'cog overlaps the first scenario button').toBe(false);

  // The reserve must not have introduced a scrollbar in the list column.
  const listOverflow = await serverPage.evaluate(() => {
    const el = document.getElementById('world-list');
    return { clientWidth: el.clientWidth, scrollWidth: el.scrollWidth };
  });
  expect(listOverflow.scrollWidth).toBeLessThanOrEqual(listOverflow.clientWidth + 1);
});

// 900x700 is inside the band where `.lobby-panel-wrap`'s
// `clamp(1rem, 4vw, 3rem)` left padding resolves below the cog's 44px extent
// (4vw = 36px here), i.e. the width at which the old CSS clipped the title.
// A 1280-wide check would have passed against the unfixed page.
test('lobby: the cog clears the lobby title at a width where the clamp does not', async ({ context }) => {
  const serverPage = await context.newPage();
  await serverPage.setViewportSize({ width: 900, height: 700 });
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  // Phase reaches Lobby once the world is loaded; server.html then unhides
  // #lobby-panel and fills #lobby-title. No client needs to connect — the
  // corner is painted before anyone claims a station.
  await serverPage.waitForFunction(
    () => {
      const panel = document.getElementById('lobby-panel');
      const title = document.getElementById('lobby-title');
      return !!panel && panel.style.display !== 'none'
        && !!title && (title.textContent || '').trim().length > 0;
    },
    { timeout: 30_000 },
  );

  const title = await overlap(serverPage, '#lobby-title');
  expect(
    title.intersects,
    `cog ${JSON.stringify(title.cog)} overlaps #lobby-title ${JSON.stringify(title.target)}`,
  ).toBe(false);

  // Same guard lobby-responsive.spec.js makes: the extra left padding must not
  // push the station grid into a horizontal scroll.
  const overflowX = await serverPage.evaluate(() => {
    const panel = document.getElementById('lobby-panel');
    return { clientWidth: panel.clientWidth, scrollWidth: panel.scrollWidth };
  });
  expect(overflowX.scrollWidth).toBeLessThanOrEqual(overflowX.clientWidth + 1);
});
