// Regression test for a manual bug report: "when selecting a station mid-game,
// the ready and leave buttons don't work (or at least leave is intermittent)".
//
// Drives the REAL client.html DOM (not the raw createTestClient JS shim) for
// a second player who joins after GameStarted, claims a vacant station while
// SimState/BlackboardUpdate are flowing at ~10Hz from the solo captain, and
// exercises both Leave (while the claim is still pending) and Ready (which
// hands the station over from Backfill AI and transitions into the console
// view, hiding the lobby panel — that transition IS the expected outcome of
// a working Ready click).

import { test, expect, readHostPeerId, createServerPage } from './fixtures';

test('mid-game station claim: Leave and Ready both register via real DOM clicks', async ({ context }) => {
  test.setTimeout(60_000);

  const serverPage = await createServerPage(context);
  const hostId = await readHostPeerId(serverPage);

  // Player 1: real client.html, claims Captain, readies solo -> auto-starts.
  const p1 = await context.newPage();
  await p1.goto(`/client/#${hostId}`);
  await p1.waitForSelector('#station-list .station-row', { timeout: 15_000 });
  await p1.click('#station-list .station-row:has-text("Captain") button.claim-btn');
  await p1.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await p1.click('#ready-btn');

  // Ready starts a server-authoritative 5 s countdown before the game
  // actually transitions to InProgress (src/lobby/handler.rs). Wait for
  // that transition on p1's page before bringing p2 in, or p2's mid-game
  // claim flow below races a server that's still in Lobby.
  await p1.waitForSelector('#lobby-ui.active', { state: 'detached', timeout: 10_000 });

  // Player 2: joins after the game has started.
  const p2 = await context.newPage();
  await p2.goto(`/client/#${hostId}`);
  await p2.waitForSelector('#station-list .station-row', { timeout: 15_000 });

  // Let a few SimState/BlackboardUpdate ticks land before claiming, to catch
  // the render-guard race as it exists during normal play.
  await p2.waitForTimeout(500);

  await p2.click('#station-list .station-row:has-text("Helm") button.claim-btn');
  await p2.waitForSelector('.detail-release-btn', { timeout: 5_000 });

  // Soak through several more ticks before clicking Leave, so the click lands
  // in the middle of the ongoing 10Hz stream (the original failure mode).
  await p2.waitForTimeout(800);
  // #771 AC3: mid-round release is a two-step arm→confirm. The first click
  // arms the control (swapping its label to the confirm string) without
  // sending; the second click sends ReleaseStation.
  await p2.click('.detail-release-btn');
  await expect(p2.locator('.detail-release-btn')).toHaveText('[CONFIRM RELEASE?]', { timeout: 5_000 });
  await p2.click('.detail-release-btn');
  await p2.waitForSelector('.detail-station-name:has-text("No station assigned")', { timeout: 5_000 });

  // Re-claim Helm and this time press Take Station (the in-progress relabel of
  // the Ready hand-off, #771 AC1). It still sends SetReady{ ready: true }.
  await p2.click('#station-list .station-row:has-text("Helm") button.claim-btn');
  await p2.waitForSelector('#ready-btn:not([style*="display: none"])', { timeout: 5_000 });
  await expect(p2.locator('#ready-btn')).toHaveText('[TAKE STATION]', { timeout: 5_000 });
  await p2.waitForTimeout(800);
  await p2.click('#ready-btn');

  // A successful Take Station hands the station over from Backfill AI and
  // transitions the client into the console view (lobby panel hides, the
  // Helm console section becomes active).
  await expect(p2.locator('#helm-ui')).toHaveClass(/active/, { timeout: 5_000 });
  await expect(p2.locator('#lobby-ui')).not.toHaveClass(/active/);

  await p1.close();
  await p2.close();
});
