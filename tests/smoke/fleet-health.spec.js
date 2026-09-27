// Browser DOM coverage for #1535. The fixture substitutes sockets and RTC,
// while the pages, fleet admission, status component and String Table are real.
import { test, expect, waitForWasmReady, waitForJoinCode } from './fixtures';
import { ts } from './strings';

const WORLD = 'assets/worlds/default.toml';
const health = page => page.locator('[data-fleet-health]');

async function host(context, { gm = false, relay = false } = {}) {
  const page = await context.newPage();
  const query = new URLSearchParams({ scenario: WORLD,
    ...(gm ? { gm: '1' } : {}), ...(relay ? { transport: 'ws-relay' } : {}) });
  await page.goto(`/?${query}`);
  await waitForWasmReady(page);
  if (!gm) await waitForJoinCode(page, 'join-code', 30_000);
  return page;
}

async function join(page, code, { gm = false } = {}) {
  // Smoke pages share localStorage; separate real GM hosts do not. Reusing the
  // previous reconnect proof would reclaim its slot instead of admitting one.
  if (gm) await page.evaluate(() => localStorage.removeItem('phoenix.fleet.gm-identity.v1'));
  await page.evaluate(value => window.__hostFleetJoin(value), code);
  await page.waitForFunction(() => window.__hostFleetState?.().open, undefined, { timeout: 30_000 });
  if (gm) await page.waitForFunction(() => {
    const state = window.__hostGmStartState?.();
    return state?.admitted && state.presentationReady && state.localValidation;
  }, undefined, { timeout: 30_000 });
}

test('operator sees forced fallback and a non-blocking warning beyond the supported fleet size', async ({ context }) => {
  test.setTimeout(180_000);
  const lead = await host(context);
  await lead.evaluate(() => window.__hostFleetOpen());
  await lead.waitForFunction(() => window.__hostFleetState?.().suffix);
  const code = await lead.evaluate(() => window.__hostFleetState().suffix);
  const members = [];
  for (let i = 0; i < 3; i += 1) {
    const page = await host(context, { relay: i === 0 });
    await join(page, code);
    members.push(page);
  }
  for (let i = 0; i < 2; i += 1) {
    const page = await host(context, { gm: true });
    await join(page, code, { gm: true });
    members.push(page);
  }
  await expect.poll(() => lead.locator('#fleet-slots li').count()).toBe(4);
  await expect.poll(() => lead.locator('#fleet-gms [role="listitem"]').count()).toBe(2);
  const status = health(lead);
  const relayPhrase = ts('server.fleet.health.relay', { name: 'PEER' }).split('PEER: ')[1].split('.')[0];
  await expect(status).toContainText(relayPhrase);
  await expect(status).not.toContainText('Beyond tested capacity');
  await status.focus();

  const fifthShip = await host(context);
  await join(fifthShip, code);
  await expect(status).toContainText(ts('server.fleet.health.capacity', { ships: 5, gms: 2 }));
  const thirdGm = await host(context, { gm: true });
  await join(thirdGm, code, { gm: true });
  await expect(status).toContainText(ts('server.fleet.health.capacity', { ships: 5, gms: 3 }));
  expect(await status.getAttribute('role')).toBe('status');
  expect(await status.getAttribute('aria-live')).toBe('polite');
  expect(await status.evaluate(element => element.tabIndex)).toBe(0);
  expect(await status.evaluate(element => document.activeElement === element)).toBe(true);
  await expect.poll(() => lead.locator('#fleet-slots li').count()).toBe(5);
  await expect.poll(() => lead.locator('#fleet-gms [role="listitem"]').count()).toBe(3);
});
