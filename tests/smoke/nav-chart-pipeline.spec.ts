// Issue #819 — nav-chart pipeline smoke.
//
// client.html's hand-maintained sim mirror is gone: window.simState is the
// only store the console builders read. The failure mode the deleted mirror's
// comment warned about — forgetting to carry `nav_chart_shows` through, which
// makes the Navigation builder's outer filter silently drop every
// non-objective entity — can now only regress inside gui/sim-state.js /
// gui/console-state.js. This spec drives the REAL client.html page (not the
// createTestClient JS shim): a real Welcome + world flows host →
// connection-manager → simState.apply, and the navigation console state is
// built from window.simState through the same `buildConsoleState` entry point
// the iframe push uses. Asserts a non-objective entity ("Starbase Alpha", a
// plain station from MINIMAL_DEFAULT_WORLD) reaches the nav-chart blip list.

import { test, expect, readHostPeerId, createServerPage } from './fixtures';

test('nav chart shows non-objective entities through the real page pipeline', async ({ context }) => {
  test.setTimeout(60_000);

  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  // Solo crew: Captain covers every console at 1P, so a single player can
  // ready up and the game moves into InProgress (same flow as
  // reconnect-midgame-sever.spec.ts).
  const client = await context.newPage();
  await client.goto(`/client/#${hostId}`);
  await client.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await client.click('#station-list .station-row:has-text("Captain") button.claim-btn');
  await client.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await client.click('#ready-btn');
  await expect(client.locator('#captain-ui')).toHaveClass(/active/, { timeout: 10_000 });

  // The ship_config's nav-chart filter lists must survive the Welcome →
  // simState path: an empty `navChartShows` blanks the chart's outer filter.
  await client.waitForFunction(
    () => (((window as any).simState?.navChartShows) ?? []).length > 0,
    undefined,
    { timeout: 10_000 },
  );

  // Build the navigation console state from the single store, through the
  // same entry point client.html's iframe push uses, and wait until the
  // world snapshot has delivered the starbase.
  await client.waitForFunction(
    () => {
      const sim = (window as any).simState;
      const build = (window as any).buildConsoleState;
      if (!sim || typeof build !== 'function') return false;
      try {
        const nav = JSON.parse(build('navigation', sim));
        return (nav.blips ?? []).some(
          (b: any) => b.name === 'Starbase Alpha' && !b.objective_target,
        );
      } catch {
        return false;
      }
    },
    undefined,
    { timeout: 15_000 },
  );

  const blip = await client.evaluate(() => {
    const nav = JSON.parse(
      (window as any).buildConsoleState('navigation', (window as any).simState),
    );
    return nav.blips.find((b: any) => b.name === 'Starbase Alpha');
  });
  expect(blip).toBeTruthy();
  expect(blip.objective_target).toBeFalsy();
  expect(blip.kind).toBe('station');

  await client.close();
});
