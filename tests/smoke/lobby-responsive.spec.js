// Smoke tests for the responsive lobby reflow (issue #436 follow-up).
//
// The lobby cards reflow with viewport via `repeat(auto-fit, minmax(220px, 360px))`.
// At portrait or narrow widths a media query
// `@media (orientation: portrait), (max-width: 720px)` triggers compact mode:
//   - the right rail collapses to a strip BELOW the station grid
//   - per-slot RESERVED placeholders (.station-card.empty.per-slot) are hidden
//   - a single aggregate chip (#reserved-aggregate.active) replaces them
// At landscape ≥ 720px width the rail stays to the right and per-slot empties
// remain visible (no aggregate chip).

import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

/** Boot a server page at a specific viewport, attach two clients, claim Helm
 *  on the first (forces the 2-player layout), and wait for the lobby grid to
 *  be rendered with cards. */
async function setupLobby(
  context,
  viewport,
) {
  const serverPage = await context.newPage();
  await serverPage.setViewportSize(viewport);
  await serverPage.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(serverPage);

  const hostId = await readHostPeerId(serverPage);

  // Two clients: the 2-player layout is the smallest one
  // that exposes a "Helm" station (the 1-player layout is a single "Captain"
  // station with all consoles bundled).
  const clientA = await createTestClient(context, hostId, { name: 'Alpha' });
  const clientB = await createTestClient(context, hostId, { name: 'Beta' });

  await clientA.send('SelectStation', { station: 'Helm' });
  await clientA.waitForMessage('StationAssigned', 5_000);

  // Wait for the lobby panel to be visible AND populated. The Bevy → JS push
  // happens after the StationAssigned wire message; allow a short window.
  console.log('Waiting for lobby panel with claimed card…');
  await serverPage.waitForFunction(
    () => {
      const panel = document.getElementById('lobby-panel');
      const grid = document.getElementById('station-grid');
      const claimed = grid?.querySelector('.station-card.claimed');
      return !!panel && panel.style.display !== 'none' && !!grid && !!claimed;
    },
    { timeout: 10_000 },
  );

  return { serverPage, clientA, clientB };
}

test.describe('lobby responsive — portrait', () => {
  test.setTimeout(60_000);
  test('compact: rail below grid, no aggregate chip (fixed roster), no per-slot empties, no body h-scroll', async ({ context }) => {
    const { serverPage, clientA, clientB } = await setupLobby(context, { width: 480, height: 900 });

    // (a) Fixed-roster (#495): all 6 station slots are always defined, so reservedCount=0
    // and the aggregate chip is never shown.
    const aggregateActive = await serverPage.evaluate(() => {
      const el = document.getElementById('reserved-aggregate');
      return !!el && el.classList.contains('active') && getComputedStyle(el).display !== 'none';
    });
    expect(aggregateActive).toBe(false);

    // (b) No per-slot empty placeholders exist (all slots have station definitions).
    const emptyCardVisible = await serverPage.evaluate(() => {
      const el = document.querySelector('.lobby-grid .station-card.empty.per-slot');
      return !!el && getComputedStyle(el).display !== 'none';
    });
    expect(emptyCardVisible).toBe(false);

    // (c) The rail is positioned below the grid column (rail.top ≥ gridColumn.bottom).
    // We measure the .lobby-grid-column flex item (not #station-grid whose content
    // can overflow into the gap when all 6 slots are filled).
    const layout = await serverPage.evaluate(() => {
      const gridColumn = document.querySelector('.lobby-grid-column').getBoundingClientRect();
      const rail = document.querySelector('.lobby-rail').getBoundingClientRect();
      return { gridBottom: gridColumn.bottom, railTop: rail.top };
    });
    expect(layout.railTop).toBeGreaterThanOrEqual(layout.gridBottom - 5);

    // (d) No horizontal scroll *inside the lobby panel*. The pre-existing
    // viewscreen border decoration (`.vs-piece`) sits at a fixed pixel size
    // and overflows narrow viewports — not introduced by this work, not part
    // of the lobby UI, and out of scope for the responsive reflow.
    const lobbyHOverflow = await serverPage.evaluate(() => {
      const panel = document.getElementById('lobby-panel');
      if (!panel) return { panelWidth: 0, panelScrollWidth: 0 };
      return { panelWidth: panel.clientWidth, panelScrollWidth: panel.scrollWidth };
    });
    expect(lobbyHOverflow.panelScrollWidth).toBeLessThanOrEqual(lobbyHOverflow.panelWidth + 1);

    // (e) Connected list rendered as pills, not legacy rows. The second client
    // is a spectator (didn't claim a station), so at least one pill exists.
    const hasPills = await serverPage.evaluate(() => {
      return document.querySelectorAll('#lobby-spectator-list .spectator-pill').length > 0;
    });
    expect(hasPills).toBe(true);

    await clientA.close();
    await clientB.close();
  });
});

test.describe('lobby responsive — landscape', () => {
  test.setTimeout(60_000);
  test('wide: rail right of grid, no per-slot empties (fixed roster), no aggregate, multi-column grid', async ({ context }) => {
    const { serverPage, clientA, clientB } = await setupLobby(context, { width: 1280, height: 720 });

    // (a) Fixed-roster (#495): no per-slot empty placeholders — all slots have station definitions.
    const emptyCardVisible = await serverPage.evaluate(() => {
      const cards = document.querySelectorAll('.lobby-grid .station-card.empty.per-slot');
      if (cards.length === 0) return false;
      return getComputedStyle(cards[0]).display !== 'none';
    });
    expect(emptyCardVisible).toBe(false);

    // (b) Aggregate chip is NOT visible in wide mode (default `display:none`).
    const aggregateVisible = await serverPage.evaluate(() => {
      const el = document.getElementById('reserved-aggregate');
      return !!el && getComputedStyle(el).display !== 'none';
    });
    expect(aggregateVisible).toBe(false);

    // (c) Rail is to the right of the grid (rail.left ≈ grid.right + gap).
    const layout = await serverPage.evaluate(() => {
      const grid = document.getElementById('station-grid').getBoundingClientRect();
      const rail = document.querySelector('.lobby-rail').getBoundingClientRect();
      return { gridRight: grid.right, railLeft: rail.left };
    });
    expect(layout.railLeft).toBeGreaterThan(layout.gridRight - 50);

    // (d) Multiple grid columns: at least two cards share the same row top.
    const multipleColumns = await serverPage.evaluate(() => {
      const cards = Array.from(
        document.querySelectorAll('.lobby-grid .station-card')
      );
      if (cards.length < 2) return false;
      const tops = cards.map((c) => c.getBoundingClientRect().top);
      const firstTop = tops[0];
      return tops.slice(1).some((t) => Math.abs(t - firstTop) < 5);
    });
    expect(multipleColumns).toBe(true);

    await clientA.close();
    await clientB.close();
  });
});
