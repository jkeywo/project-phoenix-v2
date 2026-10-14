import { test, expect, readHostPeerId, createTestClient, waitForWasmReady, expectFixtureWorld } from './fixtures';
import { dismissTutorialCards } from './dock-helpers.js';

for (const hull of [
  { file: 'alliance_destroyer', station: 'Engineering', primary: 'engineering', comms: 'comms', navigation: 'navigation', seats: 4 },
  { file: 'alliance_courier', station: 'Captain', primary: 'captain', comms: 'comms-service', navigation: 'navigation-service', seats: 2 },
]) test(`${hull.seats}-seat ${hull.file}: Energy and human-seeking Consoles retain their own working view`, { tag: '@core' }, async ({ context }) => {
  test.setTimeout(180000);
  const world = `[global]\nseed=808\ntitle="Energy crew ${hull.file}"\n[[available_ships]]\ntemplate_path="assets/entities/${hull.file}.toml"\n[[entity]]\ntemplate_path="assets/entities/${hull.file}.toml"\nid="player-ship"\nspawn_on="game_start"\n`;
  await context.route('**/assets/worlds/default.toml', route => route.fulfill({ contentType: 'text/plain', body: world }));
  const host = await context.newPage(); await host.goto('/?scenario=assets/worlds/default.toml'); await waitForWasmReady(host);
  const hostId = await readHostPeerId(host), phone = await context.newPage();
  await phone.setViewportSize({ width: 1280, height: 800 }); await phone.goto(`/client/#${hostId}`);
  await phone.locator('#station-list .station-row').filter({ hasText: hull.station }).locator('button.claim-btn').click();
  await phone.locator('#ready-btn').click();
  await expect(phone.locator('#station-hero')).toHaveAttribute('aria-hidden', 'false');
  const observer = await createTestClient(context, hostId, { name: 'Fixture observer' });
  expectFixtureWorld(await observer.waitForMessage('WorldSetup', 10000), world);
  const tabs = phone.locator('#station-hero-tabs');
  await expect(tabs.locator(`[data-tab-id="${hull.comms}"]`)).toBeVisible();
  await expect(tabs.locator(`[data-tab-id="${hull.navigation}"]`)).toBeVisible();
  expect(await phone.evaluate(() => Object.keys(window.simState.stationSystems).length)).toBe(hull.seats);
  const selector = `#${hull.primary}-iframe`, frame = phone.frameLocator(selector);
  await dismissTutorialCards(phone, selector);
  await expect(frame.locator('ph-energy-controls')).toBeVisible();
  const depth = frame.locator('ph-energy-controls #depth'); await depth.selectOption('mid');
  const flow = frame.locator('ph-energy-controls #groups > fieldset input[type="range"]').first();
  await expect(flow).toBeEnabled();
  const requested = await flow.evaluate(input => { const value = Number(input.value), max = Number(input.max);
    const next = value < max ? Math.min(max, value + 1) : Math.max(0, value - 1);
    input.value = String(next); input.dispatchEvent(new Event('change', { bubbles: true })); return { group: input.closest('[data-energy-group]').dataset.energyGroup, value: next }; });
  await expect.poll(() => frame.locator('ph-energy-controls').evaluate((widget, id) => widget.state.energy.groups.find(group => group.id === id).desired_flow_per_sec, requested.group)).toBeCloseTo(requested.value);
  await depth.selectOption('min');
  await phone.waitForFunction(id => window.simState.consoleViews?.[id]?.energy_depth === 'min', hull.primary);
  await tabs.locator(`[data-tab-id="${hull.comms}"]`).click();
  await expect(phone.locator(`#${hull.comms}-ui`)).toHaveClass(/\bactive\b/);
  await tabs.locator(`[data-tab-id="${hull.primary}"]`).click(); await expect(depth).toHaveValue('min');
  await frame.locator('html').evaluate(node => { node.dataset.oldEnergyDocument = 'true'; });
  await phone.evaluate(id => { document.getElementById(id).contentWindow.location.reload(); }, `${hull.primary}-iframe`);
  await expect(frame.locator('html')).not.toHaveAttribute('data-old-energy-document', 'true');
  await expect(depth).toHaveValue('min');
  await observer.close(); await phone.close(); await host.close();
});
