import { test, expect } from '@playwright/test';

// Real shared native shell in Chromium; production native PCM/device tests live
// in native_host/audio/tests.rs. The fixture only answers the native UI bridge.
const PAGE = `<!doctype html><html><head>
<link rel="stylesheet" href="/gui/tokens.css"><link rel="stylesheet" href="/gui/native-settings.css">
<link rel="stylesheet" href="/gui/audio-settings.css"></head><body><script type="module">
import '/gui/strings-boot.js';
import { mountNativeSettings } from '/gui/native-settings.js';
import { createNativeAudio } from '/gui/native-audio.js';
import { defaultAudioMix } from '/gui/audio-mix.js';
window.requests = [];
window.state = { room:true, mix:defaultAudioMix(), categories:['music','ambience','alerts'], status:'playing',
  test:'idle', persistence:'saved', hardware_persistence:'saved', output:null,
  devices:[{id:'output:Bridge',label:'Bridge speakers',available:true}], detail:'', asset_failures:[] };
window.publish = () => window.__phoenixNativeAudioApply(JSON.stringify(window.state));
const audio = createNativeAudio({win:window, send: record => {
  window.requests.push(record);
  if(record.kind==='set_audio_bus') window.state.mix[record.bus] = {level:record.level_percent/100,muted:record.muted};
  if(record.kind==='select_audio_output') window.state.output = record.output;
  if(record.kind==='test_audio_output') window.state.test = 'playing';
  window.publish();
}});
window.shell = mountNativeSettings(document, {audio});
</script></body></html>`;

test('native Audio shell retains keyboard access, output status and large-text controls', { tag: '@core' }, async ({ page }) => {
  await page.route('**/native-audio-probe', route => route.fulfill({ contentType: 'text/html', body: PAGE }));
  const errors = []; page.on('pageerror', error => errors.push(String(error)));
  await page.goto('/native-audio-probe');
  await page.evaluate(() => window.shell.open());
  const master = page.locator('[data-audio-bus="master"] input');
  await expect(master).toBeVisible(); await master.focus(); await page.keyboard.press('ArrowLeft');
  await expect(master).toHaveValue('0.99');
  await page.locator('[data-audio-test]').click();
  await expect.poll(() => page.evaluate(() => requests.filter(r => r.kind === 'test_audio_output').length)).toBe(1);
  const output = page.locator('.audio-settings select'); await output.selectOption('output:Bridge');
  await page.evaluate(() => { state.devices = []; state.detail = 'settings.audio.selected_missing'; state.status = 'failed'; publish(); });
  await expect(output).toHaveValue('output:Bridge');
  await expect(page.locator('.audio-settings [role="status"]').first()).toContainText('disconnected');
  await page.locator('[data-audio-bus="master"] button').click();
  await expect(page.locator('[data-audio-bus="master"] button')).toHaveAttribute('aria-pressed', 'true');
  await page.emulateMedia({ forcedColors: 'active' });
  await page.evaluate(() => { document.documentElement.style.fontSize = '200%'; });
  await master.focus();
  await page.keyboard.press('ArrowLeft');
  const box = await master.boundingBox(); expect(box.width).toBeGreaterThan(40);
  expect(await master.evaluate(el => getComputedStyle(el).outlineStyle)).not.toBe('none');
  await page.keyboard.press('Escape');
  await expect(page.locator('#native-settings-overlay')).toBeHidden();
  expect(errors).toEqual([]);
});
