import { test, expect } from '@playwright/test';
import { ts } from './strings';

const probe = `<!doctype html><html><head>
<link rel="stylesheet" href="/client/gui/tokens.css"><link rel="stylesheet" href="/client/gui/gm-workspace.css">
<style>html{font-size:32px}body{margin:0}#gm-console{padding:1rem;box-sizing:border-box}button,input,select,textarea{font:inherit}</style>
</head><body><main id="gm-console" class="gm-desk"><section id="gm-mission-panel"></section></main>
<script type="module">
import '/client/gui/strings-boot.js';
import { t } from '/client/gui/strings.js';
import { createGmPresentationPanel } from '/client/gui/gm-presentation-panel.js';
window.projection={entities:[{kind:'player_ship',entity_id:'alpha',name:'Alliance Horizon'}],
 presentation_cameras:{alpha:['camera_fore','camera_aft']},
 presentation_messages:[{message:'message-one',sender:'Lyra Station',ship:'alpha'}]};
window.panel=createGmPresentationPanel({t,getOperator:()=>({id:'gm'}),submit:request=>{window.request=request;return true;}});
panel.update(projection);
</script></body></html>`;

test('GM presentation controls remain keyboard reachable at 200% with stable selections', { tag: '@core' }, async ({ page }, testInfo) => {
  await page.route('**/presentation-control-probe', route => route.fulfill({ contentType: 'text/html', body: probe }));
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto('/presentation-control-probe');
  await expect(page.locator('#gm-presentation-camera')).toBeVisible();
  await page.locator('#gm-presentation-camera').selectOption('camera_aft');
  await page.locator('#gm-presentation-duration').fill('120');
  const force = page.getByRole('button', { name: ts('server.gm.presentation.force'), exact: true });
  await force.focus(); await page.keyboard.press('Enter');
  await expect.poll(() => page.evaluate(() => window.request?.cue)).toEqual({ force_view: { view: { camera: 'camera_aft' }, duration_ticks: 120 } });
  await page.evaluate(() => panel.update({ ...projection, presentation_results: [{ ...window.request, outcome: 'applied' }] }));
  await expect(page.locator('#gm-presentation-panel [role=status]')).toHaveText(ts('server.gm.presentation.applied'));
  await page.emulateMedia({ forcedColors: 'active' });
  const release = page.getByRole('button', { name: ts('server.gm.presentation.release'), exact: true });
  await release.focus(); await expect(release).toBeFocused();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('gm-presentation-200.png'), fullPage: true });
});

test('native Viewscreen HUD renders and clears the shared title/Comms card at chosen scale', { tag: '@core' }, async ({ page }, testInfo) => {
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto('/client/gui/viewscreen-hud.html');
  await page.waitForFunction(() => !!window.__phoenixHud?.render);
  await page.evaluate(() => {
    document.documentElement.style.fontSize = '32px';
    window.__updateHud(JSON.stringify({ heading: 90, hull_pct: 100, condition: 'Nominal',
      presentation_card: { kind: 'title', title: 'Arrival at Lyra', body: 'Awaiting the crew', literal_title: true, literal_body: true } }));
  });
  const card = page.locator('.vs-presentation-card');
  await expect(card).toBeVisible(); await expect(card).toContainText('Arrival at Lyra');
  expect(await card.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
  await card.focus(); await expect(card).toBeFocused();
  await page.screenshot({ path: testInfo.outputPath('native-presentation-200.png') });
  await page.evaluate(() => window.__updateHud(JSON.stringify({ presentation_card: { kind: 'incoming', title: 'Lyra', body: 'Hold position', literal_body: true } })));
  await expect(card).toContainText('Hold position');
  await page.evaluate(() => window.__updateHud(JSON.stringify({ presentation_card: {
    kind: 'incoming', title: 'Lyra', body: Array.from({ length: 60 }, (_, i) => `Message line ${i + 1}`).join('\n'), literal_body: true,
  } })));
  expect(await card.locator('h1').evaluate(el => el.getBoundingClientRect().top >= el.parentElement.getBoundingClientRect().top)).toBe(true);
  await card.focus(); await page.keyboard.press('End');
  await expect.poll(() => card.evaluate(el => el.scrollTop)).toBeGreaterThan(0);
  await page.evaluate(() => window.__updateHud(JSON.stringify({ presentation_card: {
    kind: 'title', title: 'The next scene', body: 'Ready', literal_title: true, literal_body: true,
  } })));
  expect(await card.evaluate(el => el.scrollTop)).toBe(0);
  await expect(card).toContainText('The next scene');
  await page.evaluate(() => window.__updateHud(JSON.stringify({ presentation_card: null })));
  await expect(card).toBeHidden(); expect(errors).toEqual([]);
});
